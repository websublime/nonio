//! An [`Agent`] that is the person's own coding agent session, reached through MCP tool results (D06: no API key, no
//! sampling). Each request waits until the host's model submits output for it, or gives up. The maker, explainer and
//! checker don't know the difference: they get output back as from any other agent, and everything still goes
//! through the checker.
//!
//! A request reaches the model one of two ways: handed to a tool call that is waiting for the next piece of work (so
//! the model carries straight on within one turn), or, when none is waiting, announced, so the server can tell the
//! host if it has a way to (a channel message in Claude Code).
//!
//! A request can go unheard: no listening call is open and the announcement went nowhere. So a request nobody has
//! picked up says so on the map after the host's `nudge`, and gives up after `give_up` rather than wait forever.
//!
//! Ported from `packages/server/src/bridge.ts` (MIT, Copyright (c) 2026 johnz4021); see NOTICE. Two changes: output is
//! validated by a function the request carries (the original looks the schema up in a table), and the host is a
//! [`HostProfile`] rather than "Claude Code unless Codex".

use std::fmt;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::host::{HostProfile, BASE};

/// What the agent is doing, for the progress shown on the map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressKind {
    Read,
    Search,
    List,
    Status,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub kind: ProgressKind,
    pub text: String,
}

impl Progress {
    pub fn status(text: impl Into<String>) -> Self {
        Progress { kind: ProgressKind::Status, text: text.into() }
    }
}

pub type ProgressFn = Arc<dyn Fn(Progress) + Send + Sync>;
/// Checks output against what a request asked for; the problems, in plain words, when it doesn't fit.
pub type Validator = Arc<dyn Fn(&Value) -> Result<(), Vec<String>> + Send + Sync>;
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// The tools a request may use when it doesn't say: read-only, always.
pub const READ_ONLY_TOOLS: [&str; 3] = ["Read", "Grep", "Glob"];

#[derive(Clone)]
pub struct AgentRequest {
    pub prompt: String,
    /// What this is for, in a few plain words ("Making the map for …"). Shown when the request is handed over.
    pub label: Option<String>,
    /// The output's JSON Schema, shown to the model.
    pub schema: Arc<Value>,
    /// How output is checked before it is accepted. Without one, any JSON object is.
    pub validate: Option<Validator>,
    pub cwd: PathBuf,
    /// Continue an earlier session (follow-up questions keep what was already read).
    pub resume: Option<String>,
    /// Tools the model may use: a subset of Read, Grep and Glob (default all three; empty to answer from the prompt).
    pub tools: Option<Vec<String>>,
    pub on_progress: Option<ProgressFn>,
    pub cancel: Option<CancellationToken>,
}

impl AgentRequest {
    pub fn new(prompt: impl Into<String>, schema: Arc<Value>, cwd: impl Into<PathBuf>) -> Self {
        AgentRequest {
            prompt: prompt.into(),
            label: None,
            schema,
            validate: None,
            cwd: cwd.into(),
            resume: None,
            tools: None,
            on_progress: None,
            cancel: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentResponse {
    pub output: Value,
    pub session_id: Option<String>,
    pub cost: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentError {
    /// The work was stopped (the person pressed Stop, or the caller cancelled).
    Stopped,
    /// The agent gave up, saying why.
    Failed(String),
    /// Nobody ever picked the request up.
    Unheard(String),
}

impl fmt::Display for AgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentError::Stopped => f.write_str("Stopped."),
            AgentError::Failed(m) | AgentError::Unheard(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for AgentError {}

pub type AgentResult = Result<AgentResponse, AgentError>;

/// The one way nonio's workflows reach a model. Tests swap in a scripted one.
pub trait Agent: Send + Sync {
    fn ask(&self, req: AgentRequest) -> BoxFuture<AgentResult>;
}

/// The session id a bridged answer carries: the host keeps its own conversation, so there is nothing to resume by id.
pub const HOST_SESSION: &str = "host";

/// A request as the host's model reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeRequest {
    pub id: String,
    pub label: String,
    pub prompt: String,
    pub schema: Arc<Value>,
    /// Tools the request may use; empty means answer from the request alone, without reading the code.
    pub tools: Vec<String>,
    /// It follows on from earlier work in the same conversation (a repair round, a follow-up question).
    pub continues: bool,
}

pub type Announce = Arc<dyn Fn(&BridgeRequest) + Send + Sync>;
pub type HostFn = Arc<dyn Fn() -> &'static HostProfile + Send + Sync>;

pub struct BridgeOptions {
    pub announce: Announce,
    /// Which agent picks requests up, for what the map says. Asked per request: the server only knows once the MCP
    /// client has connected. Default: the base protocol.
    pub host: HostFn,
    /// After this long unclaimed, say so on the map (default: the host's `nudge`).
    pub nudge: Option<Duration>,
    /// After this long unclaimed, give up with an error (default 5 min).
    pub give_up: Duration,
}

impl BridgeOptions {
    pub fn new(announce: impl Fn(&BridgeRequest) + Send + Sync + 'static) -> Self {
        BridgeOptions {
            announce: Arc::new(announce),
            host: Arc::new(|| &BASE),
            nudge: None,
            give_up: Duration::from_secs(5 * 60),
        }
    }
}

struct Entry {
    view: BridgeRequest,
    validate: Option<Validator>,
    handed: bool,
    claimed: &'static str,
    on_progress: Option<ProgressFn>,
    reply: Option<oneshot::Sender<AgentResult>>,
    tasks: Vec<JoinHandle<()>>,
}

#[derive(Default)]
struct State {
    entries: Vec<Entry>,
    waiters: Vec<(u64, oneshot::Sender<BridgeRequest>)>,
    made: u64,
    waiter_ids: u64,
}

struct Inner {
    opts: BridgeOptions,
    state: Mutex<State>,
}

/// A progress line to send once the lock is let go: callbacks are the caller's code and may call back in.
type Said = Option<(ProgressFn, Progress)>;

fn say(said: Said) {
    if let Some((f, p)) = said {
        f(p);
    }
}

#[derive(Clone)]
pub struct Bridge {
    inner: Arc<Inner>,
}

impl Bridge {
    pub fn new(opts: BridgeOptions) -> Self {
        Bridge { inner: Arc::new(Inner { opts, state: Mutex::new(State::default()) }) }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.inner.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn host(&self) -> &'static HostProfile {
        (self.inner.opts.host)()
    }

    /// Off the list, its timers stopped, answered with `result`. Nothing happens if it was settled already.
    fn settle(&self, id: &str, result: AgentResult) -> bool {
        let entry = {
            let mut s = self.state();
            let Some(i) = s.entries.iter().position(|e| e.view.id == id) else { return false };
            s.entries.remove(i)
        };
        for t in &entry.tasks {
            t.abort();
        }
        if let Some(tx) = entry.reply {
            let _ = tx.send(result);
        }
        true
    }

    /// Marks the oldest request not handed over yet as handed; it and what to say on the map.
    fn take(s: &mut State) -> Option<(BridgeRequest, Said)> {
        let e = s.entries.iter_mut().find(|e| !e.handed)?;
        e.handed = true;
        let said = e.on_progress.clone().map(|f| (f, Progress::status(e.claimed)));
        Some((e.view.clone(), said))
    }

    /// The newest waiting call that is still there gets `view`. False when none is.
    fn hand_to_waiter(s: &mut State, view: &BridgeRequest) -> bool {
        // The newest: one the model has open right now (making a map, handing in) beats an older listener left
        // running in the background, which keeps waiting for the next request.
        while let Some((_, tx)) = s.waiters.pop() {
            if tx.send(view.clone()).is_ok() {
                return true;
            }
        }
        false
    }

    /// Ask the host's model. The request is queued (or handed to a waiting call) before this returns, so whatever
    /// follows sees it; the future resolves once output is submitted, or the request fails.
    pub fn ask(&self, req: AgentRequest) -> BoxFuture<AgentResult> {
        if req.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return Box::pin(async { Err(AgentError::Stopped) });
        }
        let h = self.host();
        let (tx, rx) = oneshot::channel();
        let mut said: Vec<Said> = Vec::new();
        let (view, handed) = {
            let mut s = self.state();
            s.made += 1;
            let view = BridgeRequest {
                id: format!("r{}", s.made),
                label: req.label.clone().unwrap_or_else(|| "Work for nonio".into()),
                prompt: req.prompt.clone(),
                schema: req.schema.clone(),
                tools: req.tools.clone().unwrap_or_else(|| READ_ONLY_TOOLS.iter().map(|t| t.to_string()).collect()),
                continues: req.resume.is_some(),
            };
            let handed = Self::hand_to_waiter(&mut s, &view);
            let progress = |text: &'static str| req.on_progress.clone().map(|f| (f, Progress::status(text)));
            said.push(progress(if handed { h.claimed } else { h.waiting }));
            s.entries.push(Entry {
                view: view.clone(),
                validate: req.validate.clone(),
                handed,
                claimed: h.claimed,
                on_progress: req.on_progress.clone(),
                reply: Some(tx),
                tasks: Vec::new(),
            });
            (view, handed)
        };
        said.into_iter().for_each(say);

        let mut tasks = Vec::new();
        if let Some(cancel) = req.cancel.clone() {
            let me = self.clone();
            let id = view.id.clone();
            tasks.push(tokio::spawn(async move {
                cancel.cancelled().await;
                me.settle(&id, Err(AgentError::Stopped));
            }));
        }
        if !handed {
            let unclaimed = |me: &Bridge, id: &str| me.state().entries.iter().any(|e| e.view.id == id && !e.handed);
            let (me, id, after) = (self.clone(), view.id.clone(), self.inner.opts.nudge.unwrap_or(h.nudge));
            let on_progress = req.on_progress.clone();
            tasks.push(tokio::spawn(async move {
                tokio::time::sleep(after).await;
                if unclaimed(&me, &id) {
                    if let Some(f) = on_progress {
                        f(Progress::status(h.not_picked_up));
                    }
                }
            }));
            let (me, id, after) = (self.clone(), view.id.clone(), self.inner.opts.give_up);
            tasks.push(tokio::spawn(async move {
                tokio::time::sleep(after).await;
                if unclaimed(&me, &id) {
                    me.settle(&id, Err(AgentError::Unheard(h.never_picked_up.into())));
                }
            }));
        }
        {
            let mut s = self.state();
            match s.entries.iter_mut().find(|e| e.view.id == view.id) {
                Some(e) => e.tasks.extend(tasks),
                // Settled already (cancelled in between): nothing left to time.
                None => tasks.iter().for_each(|t| t.abort()),
            }
        }
        if !handed {
            (self.inner.opts.announce)(&view);
        }
        Box::pin(async move { rx.await.unwrap_or(Err(AgentError::Stopped)) })
    }

    /// The oldest request not handed over yet, or else the oldest still waiting for output. Marks it handed.
    pub fn next(&self) -> Option<BridgeRequest> {
        let (got, said) = {
            let mut s = self.state();
            match Self::take(&mut s) {
                Some((v, said)) => (Some(v), said),
                None => (s.entries.first().map(|e| e.view.clone()), None),
            }
        };
        say(said);
        got
    }

    /// The next request to hand over: now if one is waiting, else the next one made, or `None` once `cancel` fires.
    pub async fn wait_for_next(&self, cancel: &CancellationToken) -> Option<BridgeRequest> {
        let (id, mut rx) = {
            let mut s = self.state();
            if let Some((v, said)) = Self::take(&mut s) {
                drop(s);
                say(said);
                return Some(v);
            }
            if cancel.is_cancelled() {
                return None;
            }
            let (tx, rx) = oneshot::channel();
            s.waiter_ids += 1;
            let id = s.waiter_ids;
            s.waiters.push((id, tx));
            (id, rx)
        };
        tokio::select! {
            got = &mut rx => got.ok(),
            _ = cancel.cancelled() => {
                self.state().waiters.retain(|(w, _)| *w != id);
                // Handed something just as it stopped: take it rather than lose it.
                rx.try_recv().ok()
            }
        }
    }

    /// Hand in output for a request. A string is read as JSON. The problems come back when it doesn't fit.
    pub fn submit(&self, id: &str, output: Value) -> Result<(), Vec<String>> {
        let validate = {
            let s = self.state();
            let Some(e) = s.entries.iter().find(|e| e.view.id == id) else {
                return Err(vec![format!(
                    "No request \"{id}\" is waiting. It may have been answered already, or stopped."
                )]);
            };
            e.validate.clone()
        };
        let value = match output {
            Value::String(text) => match serde_json::from_str::<Value>(&text) {
                Ok(v) => v,
                Err(_) => return Err(vec!["`output` must be a JSON object matching the schema, not text.".into()]),
            },
            v => v,
        };
        match &validate {
            Some(check) => check(&value).map_err(|p| p.into_iter().take(12).collect::<Vec<_>>())?,
            None if !value.is_object() => {
                return Err(vec!["`output` must be a JSON object matching the schema.".into()])
            }
            None => {}
        }
        // The raw value: callers parse it themselves.
        self.settle(id, Ok(AgentResponse { output: value, session_id: Some(HOST_SESSION.into()), cost: None }));
        Ok(())
    }

    /// Give up on a request, saying why. False when no such request is waiting.
    pub fn fail(&self, id: &str, reason: &str) -> bool {
        let message = format!("{} couldn't finish this: {reason}", self.host().name);
        self.settle(id, Err(AgentError::Failed(message)))
    }

    /// Requests still waiting for output.
    pub fn outstanding(&self) -> Vec<BridgeRequest> {
        self.state().entries.iter().map(|e| e.view.clone()).collect()
    }
}

impl Agent for Bridge {
    fn ask(&self, req: AgentRequest) -> BoxFuture<AgentResult> {
        Bridge::ask(self, req)
    }
}
