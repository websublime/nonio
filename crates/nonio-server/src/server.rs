//! The local nonio server: stores maps, runs the maker and the explainer, applies approved fixes, serves the viewer,
//! and streams progress over a WebSocket. It binds to 127.0.0.1 only. Every API call needs the random token from the
//! page URL, and requests whose Host or Origin isn't this server are refused, so other sites in the browser can't
//! drive it. With `auto_stop` it stops itself once no nonio tab is open and the agent isn't working, so it doesn't
//! linger. Each map page tells it what is selected there (kept in memory only), so a question asked in the agent's
//! own chat (hosts that ask in the chat, see host.rs) can be about what the person is pointing at.
//!
//! Ported from `packages/server/src/server.ts` (MIT, Copyright (c) 2026 johnz4021); see NOTICE. What differs: the
//! token header is `x-nonio-token`, maps live in `.nonio/` (D29), and the viewer comes from a folder or from files the
//! binary embeds (D09, see [`Viewer`]).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::{to_bytes, Body};
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{FromRequestParts, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use nonio_core::base::resolve_commit;
use nonio_core::code::{code_for_notes, code_of, CodeNote, When};
use nonio_core::format::QUESTION_MAX;
use nonio_core::repo::{list_repo_files, DEFAULT_IGNORE};
use nonio_core::screens::{capture_paths, read_capture, repo_image};
use nonio_core::{CheckReport, CodeMap};
use nonio_scan::Scanner;
use serde::Serialize;
use serde_json::{json, Map, Value};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::bridge::{Agent, Progress, ProgressKind};
use crate::explainer::{answer, Answer, AnswerOptions, OfferStatus, Selection};
use crate::fixes::{apply_fix, connect_after_fix, undo_fix, AppliedFix, FixProposal};
use crate::host::{Asks, PageAgent};
use crate::maker::{check_map, make_map, MakeOptions};
use crate::prompts::{EarlierTurn, OtherMap};
use crate::step::ask_about_step;
use crate::store::{FixStatus, MapMeta, ParentMap, SavedFix, Store, StoredMap};

/// WebSocket close code that tells open tabs nonio stopped on purpose, so they say so instead of reconnecting.
pub const STOPPED_CLOSE_CODE: u16 = 4000;
/// The header every API call carries the page's token in.
pub const TOKEN_HEADER: &str = "x-nonio-token";
const MAX_BODY: usize = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JobKind {
    Make,
    Ask,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Started,
    Done,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FixEvent {
    Applied,
    Rejected,
    Undone,
    Failed,
}

/// What the open tabs are told, and `on_event` with them.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ServerEvent {
    Job {
        map_id: String,
        job: JobKind,
        status: JobStatus,
        question: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    Progress {
        map_id: String,
        job: JobKind,
        kind: &'static str,
        text: String,
    },
    Map {
        map_id: String,
        meta: MapMeta,
        map: Box<CodeMap>,
        report: Box<CheckReport>,
    },
    Answer {
        map_id: String,
        answer: Answer,
    },
    /// An answer already sent changed: its offer of a new map was taken up or turned down.
    AnswerUpdate {
        map_id: String,
        answer: Answer,
    },
    Fix {
        map_id: String,
        fix_id: String,
        status: FixEvent,
        #[serde(skip_serializing_if = "Option::is_none")]
        problems: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        edge_id: Option<Option<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        closed: Option<Vec<String>>,
    },
}

/// Why the server stopped itself: nobody was looking (`auto_stop`), or the page's Stop button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    Idle,
    Requested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutoStop {
    /// Once a tab has connected: how long after the last one closes (long enough for a reload).
    pub after_last_tab: Duration,
    /// If no tab ever connects.
    pub before_first_tab: Duration,
}

impl Default for AutoStop {
    fn default() -> Self {
        AutoStop { after_last_tab: Duration::from_secs(30), before_first_tab: Duration::from_secs(600) }
    }
}

pub type EventFn = Arc<dyn Fn(&ServerEvent) + Send + Sync>;
pub type StopFn = Arc<dyn Fn(StopReason) + Send + Sync>;

pub struct ServerOptions {
    /// The repo to map.
    pub root: PathBuf,
    /// The port asked for first; a taken one moves on to the next ten, then any free one. 0 picks a free port.
    pub port: u16,
    pub agent: Arc<dyn Agent>,
    pub token: Option<String>,
    /// The built viewer. Without one, or one with no `index.html`, the page says the viewer isn't built.
    pub viewer: Option<Viewer>,
    /// Stop on its own when nobody is looking: no tab open and no job running. Off unless set.
    pub auto_stop: Option<AutoStop>,
    /// Called after the server stopped itself.
    pub on_stop: Option<StopFn>,
    /// Every event sent to the open tabs, also handed here (the MCP channel uses it to know when work is done).
    pub on_event: Option<EventFn>,
    /// Who does the model work, for the page.
    pub page: PageAgent,
}

impl ServerOptions {
    pub fn new(root: impl Into<PathBuf>, agent: Arc<dyn Agent>) -> Self {
        ServerOptions {
            root: root.into(),
            port: 0,
            agent,
            token: None,
            viewer: None,
            auto_stop: None,
            on_stop: None,
            on_event: None,
            page: PageAgent { name: "Claude", asks: Asks::Page },
        }
    }
}

/// An error with the HTTP status it is answered with.
#[derive(Debug, Clone, PartialEq)]
pub struct HttpError {
    pub status: StatusCode,
    pub message: String,
    pub extra: Map<String, Value>,
}

impl HttpError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        HttpError { status, message: message.into(), extra: Map::new() }
    }
    fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }
    fn with_problems(mut self, problems: &[String]) -> Self {
        self.extra.insert("problems".into(), json!(problems));
        self
    }
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for HttpError {}

/// A question asked in the agent's chat, as it was started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskedFromChat {
    pub map_id: String,
    pub question: String,
    /// What is selected on the page, by name.
    pub selected: Vec<String>,
}

/// The map whose page last connected or reported its selection, and when (milliseconds since the epoch).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    pub map_id: String,
    pub at: u128,
}

#[derive(Debug, Clone, Serialize)]
struct JobState {
    kind: JobKind,
    question: String,
    progress: Vec<String>,
}

#[derive(Debug, Clone)]
struct PageSelection {
    selection: Selection,
    step: Option<String>,
}

struct Applied {
    map_id: String,
    applied: AppliedFix,
    map_before: CodeMap,
}

enum ToTab {
    Event(String),
    Stopped,
}

#[derive(Default)]
struct Shared {
    jobs: HashMap<String, JobState>,
    failures: HashMap<String, (String, String)>,
    /// Only what Undo needs (the files as they were) stays in memory, so after a restart an applied fix shows as
    /// applied, without Undo.
    applied: HashMap<String, Applied>,
    tabs: HashMap<u64, mpsc::UnboundedSender<ToTab>>,
    selections: HashMap<String, PageSelection>,
    seen: Option<Seen>,
    visited: bool,
    idle: Option<JoinHandle<()>>,
    closing: bool,
}

struct Inner {
    root: PathBuf,
    agent: Arc<dyn Agent>,
    token: String,
    store: Store,
    page: PageAgent,
    port: u16,
    viewer: Option<Viewer>,
    auto_stop: Option<AutoStop>,
    on_stop: Option<StopFn>,
    on_event: Option<EventFn>,
    shared: Mutex<Shared>,
    next_tab: AtomicU64,
    /// Fires when closing starts: tabs and the HTTP server end.
    shutdown: CancellationToken,
    /// Fires once everything is closed.
    closed: CancellationToken,
    serve: Mutex<Option<JoinHandle<()>>>,
}

/// A running server. Cloning it gives another handle to the same server.
#[derive(Clone)]
pub struct NonioServer {
    inner: Arc<Inner>,
    pub url: String,
    pub port: u16,
    pub token: String,
}

fn now_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

/// The time now as `2026-11-27T19:25:35.451Z`.
pub(crate) fn now_iso() -> String {
    let ms = now_ms() as i64;
    let (days, rem) = (ms.div_euclid(86_400_000), ms.rem_euclid(86_400_000));
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let (h, m, s, milli) = (rem / 3_600_000, rem / 60_000 % 60, rem / 1000 % 60, rem % 1000);
    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}.{milli:03}Z")
}

fn random_token() -> String {
    let mut bytes = [0u8; 24];
    getrandom::fill(&mut bytes).expect("the system's random source");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn valid_map_id(id: &str) -> bool {
    (1..=80).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn kind_name(k: ProgressKind) -> &'static str {
    match k {
        ProgressKind::Read => "read",
        ProgressKind::Search => "search",
        ProgressKind::List => "list",
        ProgressKind::Status => "status",
    }
}

fn page_json(p: &PageAgent) -> Value {
    json!({ "name": p.name, "asks": match p.asks { Asks::Page => "page", Asks::Chat => "chat" } })
}

fn answers_of(stored: &StoredMap) -> Vec<Answer> {
    stored.answers.iter().filter_map(|a| serde_json::from_value(a.clone()).ok()).collect()
}

fn to_value<T: Serialize>(v: &T) -> Value {
    serde_json::to_value(v).expect("serializes")
}

/// Starts the server on 127.0.0.1.
pub async fn start_server(o: ServerOptions) -> io::Result<NonioServer> {
    let root = fs::canonicalize(&o.root)?;
    let listener = bind(o.port).await?;
    let port = listener.local_addr()?.port();
    let token = o.token.unwrap_or_else(random_token);
    let inner = Arc::new(Inner {
        store: Store::new(&root),
        root,
        agent: o.agent,
        token: token.clone(),
        page: o.page,
        port,
        viewer: o.viewer,
        auto_stop: o.auto_stop,
        on_stop: o.on_stop,
        on_event: o.on_event,
        shared: Mutex::new(Shared::default()),
        next_tab: AtomicU64::new(0),
        shutdown: CancellationToken::new(),
        closed: CancellationToken::new(),
        serve: Mutex::new(None),
    });
    let app = Router::new().fallback(handle).with_state(inner.clone());
    let stop = inner.shutdown.clone();
    let serve = tokio::spawn(async move {
        let _ = axum::serve(listener, app).with_graceful_shutdown(async move { stop.cancelled().await }).await;
    });
    *inner.serve.lock().unwrap() = Some(serve);
    arm_idle(&inner);
    Ok(NonioServer { url: format!("http://127.0.0.1:{port}"), port, token, inner })
}

/// A port that is taken moves on to the next few (so the page's address, and a browser's permission for it, stays as
/// steady as it can), then to any free one.
async fn bind(port: u16) -> io::Result<TcpListener> {
    if port == 0 {
        return TcpListener::bind(("127.0.0.1", 0)).await;
    }
    for p in (0..=10u16).filter_map(|i| port.checked_add(i)) {
        match TcpListener::bind(("127.0.0.1", p)).await {
            Ok(l) => return Ok(l),
            Err(e) if e.kind() == io::ErrorKind::AddrInUse => continue,
            Err(e) => return Err(e),
        }
    }
    TcpListener::bind(("127.0.0.1", 0)).await
}

impl NonioServer {
    /// The page's link, with the token: one map, or the list of maps.
    pub fn page_url(&self, map_id: Option<&str>) -> String {
        match map_id {
            Some(id) => format!("{}/m/{id}?t={}", self.url, self.token),
            None => format!("{}/?t={}", self.url, self.token),
        }
    }

    /// Starts making a map for `question`; returns its id. The map is made in the background.
    pub fn start_map(&self, question: &str) -> Result<String, HttpError> {
        start_map(&self.inner, question, None)
    }

    /// A question asked in the agent's chat about a map open in the browser: the map named, else the one whose page
    /// was used last. It is asked about what is selected on that page, as if typed into the map's question box.
    pub fn ask_from_chat(&self, question: &str, map_id: Option<&str>) -> Result<AskedFromChat, HttpError> {
        ask_from_chat(&self.inner, question, map_id)
    }

    pub fn last_seen(&self) -> Option<Seen> {
        self.inner.lock().seen.clone()
    }

    /// Whether a map is being made or a question answered right now.
    pub fn working(&self) -> bool {
        !self.inner.lock().jobs.is_empty()
    }

    /// Tells open tabs nonio stopped (they show that instead of reconnecting), then shuts down. Safe to call twice.
    pub async fn close(&self) {
        close(&self.inner).await
    }
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn busy(&self) -> HttpError {
        HttpError::new(
            StatusCode::CONFLICT,
            format!("{} is still working on this map. Try again when it finishes.", self.page.name),
        )
    }

    fn broadcast(&self, event: ServerEvent) {
        if let Some(f) = &self.on_event {
            f(&event);
        }
        let data = serde_json::to_string(&event).expect("serializes");
        for tab in self.lock().tabs.values() {
            let _ = tab.send(ToTab::Event(data.clone()));
        }
    }

    fn progress(self: &Arc<Self>, map_id: &str, job: JobKind) -> crate::bridge::ProgressFn {
        let inner = self.clone();
        let map_id = map_id.to_string();
        Arc::new(move |p: Progress| {
            if let Some(j) = inner.lock().jobs.get_mut(&map_id) {
                j.progress.push(p.text.clone());
            }
            inner.broadcast(ServerEvent::Progress {
                map_id: map_id.clone(),
                job,
                kind: kind_name(p.kind),
                text: p.text,
            });
        })
    }

    fn publish_map(&self, stored: &StoredMap) -> Result<CheckReport, HttpError> {
        let report = check_map(&stored.map, &self.root).map_err(internal)?;
        self.broadcast(ServerEvent::Map {
            map_id: stored.meta.id.clone(),
            meta: stored.meta.clone(),
            map: Box::new(stored.map.clone()),
            report: Box::new(report.clone()),
        });
        Ok(report)
    }

    fn save(&self, stored: &StoredMap) -> Result<(), HttpError> {
        self.store.save(stored).map_err(internal)
    }

    fn end_job(self: &Arc<Self>, map_id: &str) {
        self.lock().jobs.remove(map_id);
        arm_idle(self);
    }

    fn saw_page(&self, map_id: &str) {
        self.lock().seen = Some(Seen { map_id: map_id.to_string(), at: now_ms() });
    }

    /// Where each fix on a map stands (fixes not listed were rejected), so a reloaded page shows Approve or Undo.
    fn fix_states(&self, stored: &StoredMap) -> Value {
        let shared = self.lock();
        let mut out = Map::new();
        for (id, f) in &stored.fixes {
            match f.status {
                FixStatus::Proposed => {
                    out.insert(id.clone(), json!({ "status": "proposed" }));
                }
                FixStatus::Applied => {
                    let undoable = shared.applied.get(id).is_some_and(|a| a.map_id == stored.meta.id);
                    out.insert(
                        id.clone(),
                        json!({ "status": "applied", "edgeId": f.edge_id, "closed": f.closed.clone().unwrap_or_default(), "undoable": undoable }),
                    );
                }
                FixStatus::Rejected => {}
            }
        }
        Value::Object(out)
    }
}

fn internal(e: impl std::fmt::Display) -> HttpError {
    HttpError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

/// The proposal behind a fix that is still waiting for approval.
fn pending(stored: &StoredMap, fix_id: &str) -> Option<FixProposal> {
    if stored.fixes.get(fix_id).map(|f| f.status) != Some(FixStatus::Proposed) {
        return None;
    }
    answers_of(stored).into_iter().find_map(|a| a.fix.filter(|f| f.id == fix_id))
}

fn with_fix(stored: &StoredMap, fix_id: &str, state: SavedFix) -> BTreeMap<String, SavedFix> {
    let mut fixes = stored.fixes.clone();
    fixes.insert(fix_id.to_string(), state);
    fixes
}

// ---------------------------------------------------------------- jobs

fn start_map(inner: &Arc<Inner>, question: &str, parent: Option<ParentMap>) -> Result<String, HttpError> {
    let q = question.trim().to_string();
    if q.is_empty() {
        return Err(HttpError::new(StatusCode::BAD_REQUEST, "Ask a question to map."));
    }
    // A map holds a question of up to QUESTION_MAX characters; a longer one would only fail once the map is drawn.
    let len = q.encode_utf16().count();
    if len > QUESTION_MAX {
        return Err(HttpError::new(
            StatusCode::BAD_REQUEST,
            format!("That question is {len} characters long. Ask it in {QUESTION_MAX} or fewer."),
        ));
    }
    let id = {
        let mut shared = inner.lock();
        let taken: HashSet<String> = shared.jobs.keys().chain(shared.failures.keys()).cloned().collect();
        let id = inner.store.new_id(&q, &taken);
        shared.jobs.insert(id.clone(), JobState { kind: JobKind::Make, question: q.clone(), progress: Vec::new() });
        id
    };
    inner.broadcast(ServerEvent::Job {
        map_id: id.clone(),
        job: JobKind::Make,
        status: JobStatus::Started,
        question: q.clone(),
        error: None,
    });
    let inner2 = inner.clone();
    let map_id = id.clone();
    tokio::spawn(async move {
        let inner = inner2;
        let mut o = MakeOptions::new(&inner.root, q.clone(), inner.agent.clone());
        o.agent_name = inner.page.name.to_string();
        o.on_progress = Some(inner.progress(&map_id, JobKind::Make));
        let made = make_map(o).await.map_err(|e| e.to_string()).and_then(|r| {
            let now = now_iso();
            let stored = StoredMap {
                meta: MapMeta {
                    id: map_id.clone(),
                    question: q.clone(),
                    created_at: now.clone(),
                    updated_at: now,
                    commit: resolve_commit(&inner.root, "HEAD"),
                    maker_session: r.session_id,
                    explainer_session: None,
                    cost: r.cost,
                    parent,
                },
                map: r.map,
                answers: Vec::new(),
                fixes: BTreeMap::new(),
            };
            inner.store.save(&stored).map_err(|e| e.to_string())?;
            Ok(stored)
        });
        inner.end_job(&map_id);
        match made {
            Ok(stored) => {
                let _ = inner.publish_map(&stored);
                inner.broadcast(ServerEvent::Job {
                    map_id,
                    job: JobKind::Make,
                    status: JobStatus::Done,
                    question: q,
                    error: None,
                });
            }
            Err(error) => {
                inner.lock().failures.insert(map_id.clone(), (q.clone(), error.clone()));
                inner.broadcast(ServerEvent::Job {
                    map_id,
                    job: JobKind::Make,
                    status: JobStatus::Failed,
                    question: q,
                    error: Some(error),
                });
            }
        }
    });
    Ok(id)
}

fn start_ask(
    inner: &Arc<Inner>,
    map_id: &str,
    question: &str,
    selection: Selection,
    words_only: bool,
) -> Result<(), HttpError> {
    let stored = inner.store.get(map_id).ok_or_else(|| HttpError::not_found("No such map."))?;
    let q = question.trim().to_string();
    {
        let mut shared = inner.lock();
        if shared.jobs.contains_key(map_id) {
            return Err(inner.busy());
        }
        if q.is_empty() {
            return Err(HttpError::new(StatusCode::BAD_REQUEST, "Ask a question."));
        }
        shared
            .jobs
            .insert(map_id.to_string(), JobState { kind: JobKind::Ask, question: q.clone(), progress: Vec::new() });
    }
    inner.broadcast(ServerEvent::Job {
        map_id: map_id.into(),
        job: JobKind::Ask,
        status: JobStatus::Started,
        question: q.clone(),
        error: None,
    });
    let inner2 = inner.clone();
    let map_id = map_id.to_string();
    tokio::spawn(async move {
        let inner = inner2;
        // The conversation so far (the last few turns), so a follow-up can lean on what was just said.
        let mut earlier: Vec<EarlierTurn> = answers_of(&stored)
            .into_iter()
            .map(|a| {
                let concept = if !a.concept.is_empty() {
                    a.concept
                } else if let Some(o) = &a.offer {
                    format!("(A different part of the app: offered a map for \"{}\".)", o.question)
                } else {
                    String::new()
                };
                EarlierTurn { question: a.question, concept }
            })
            .filter(|t| !t.concept.is_empty())
            .collect();
        earlier.drain(..earlier.len().saturating_sub(6));
        let other_maps = inner
            .store
            .list()
            .into_iter()
            .filter(|m| m.id != map_id)
            .take(30)
            .map(|m| OtherMap { id: m.id, question: m.question })
            .collect();
        let mut o = AnswerOptions::new(&inner.root, stored.map.clone(), q.clone(), inner.agent.clone());
        o.selection = selection;
        o.session = stored.meta.explainer_session.clone();
        o.earlier = earlier;
        o.other_maps = other_maps;
        o.words_only = words_only;
        o.on_progress = Some(inner.progress(&map_id, JobKind::Ask));
        let done = answer(o).await.map_err(|e| e.to_string()).and_then(|r| {
            let latest = inner.store.get(&map_id).unwrap_or_else(|| stored.clone());
            let drew = !r.answer.added.is_empty() || !r.answer.screens.is_empty() || !r.answer.proposed.is_empty();
            let mut meta = latest.meta.clone();
            meta.explainer_session = r.session_id.clone();
            meta.cost += r.answer.cost;
            meta.updated_at = now_iso();
            let mut answers = latest.answers.clone();
            answers.push(to_value(&r.answer));
            let fixes = match &r.answer.fix {
                Some(f) => {
                    with_fix(&latest, &f.id, SavedFix { status: FixStatus::Proposed, edge_id: None, closed: None })
                }
                None => latest.fixes.clone(),
            };
            let next = StoredMap { meta, map: if drew { r.map.clone() } else { latest.map.clone() }, answers, fixes };
            inner.store.save(&next).map_err(|e| e.to_string())?;
            Ok((next, r.answer, drew))
        });
        inner.end_job(&map_id);
        match done {
            Ok((next, a, drew)) => {
                if drew {
                    let _ = inner.publish_map(&next);
                }
                inner.broadcast(ServerEvent::Answer { map_id: map_id.clone(), answer: a });
                inner.broadcast(ServerEvent::Job {
                    map_id,
                    job: JobKind::Ask,
                    status: JobStatus::Done,
                    question: q,
                    error: None,
                });
            }
            Err(error) => inner.broadcast(ServerEvent::Job {
                map_id,
                job: JobKind::Ask,
                status: JobStatus::Failed,
                question: q,
                error: Some(error),
            }),
        }
    });
    Ok(())
}

fn ask_from_chat(inner: &Arc<Inner>, question: &str, named: Option<&str>) -> Result<AskedFromChat, HttpError> {
    let (map_id, got) = {
        let shared = inner.lock();
        let map_id = named.map(str::to_string).or_else(|| shared.seen.as_ref().map(|s| s.map_id.clone()));
        let got = map_id.as_ref().and_then(|id| shared.selections.get(id).cloned());
        (map_id, got)
    };
    let stored = map_id.as_deref().and_then(|id| inner.store.get(id));
    let (Some(map_id), Some(stored)) = (map_id, stored) else {
        return Err(HttpError::not_found(match named {
            Some(n) => format!("There is no map \"{n}\" in this repo."),
            None => "No nonio map has been open in the browser yet.".to_string(),
        }));
    };
    let selection = got.as_ref().map(|g| g.selection.clone()).unwrap_or_default();
    let map = &stored.map;
    fn name(map: &CodeMap, id: &str) -> String {
        if let Some(n) = map.nodes.iter().find(|n| n.id == id) {
            return n.name.clone();
        }
        if let Some(e) = map.edges.iter().find(|e| e.id == id) {
            return format!("{} → {}", name(map, &e.from), name(map, &e.to));
        }
        map.absences.iter().find(|a| a.id == id).map(|a| a.claim.clone()).unwrap_or_else(|| id.to_string())
    }
    let picked: Vec<&String> = selection.nodes.iter().chain(&selection.edges).chain(&selection.absences).collect();
    // Like the page's own box: a walkthrough stop on screen, with nothing picked, is what the question is about.
    let q = match got.as_ref().and_then(|g| g.step.as_deref()) {
        Some(step) if picked.is_empty() && !question.trim().is_empty() => ask_about_step(step, question.trim()),
        _ => question.to_string(),
    };
    let selected = picked.iter().map(|id| name(map, id)).collect();
    start_ask(inner, &map_id, &q, selection, false)?;
    Ok(AskedFromChat { map_id, question: q.trim().to_string(), selected })
}

/// Their answer to "this looks like a different part of the app": `map` starts a new map for the question (linked
/// back to this one), `words` answers it from the code without drawing anything here.
fn take_offer(inner: &Arc<Inner>, map_id: &str, answer_id: &str, choice: Option<&str>) -> Result<Value, HttpError> {
    let Some(choice) = choice.filter(|c| *c == "map" || *c == "words") else {
        return Err(HttpError::new(StatusCode::BAD_REQUEST, "Choose a new map or an answer in words."));
    };
    let none = || HttpError::not_found("No such question waiting for a choice.");
    let stored = inner.store.get(map_id).ok_or_else(none)?;
    let a = answers_of(&stored).into_iter().find(|a| a.id == answer_id).ok_or_else(none)?;
    let offer = a.offer.clone().filter(|o| o.status == OfferStatus::Pending).ok_or_else(none)?;
    if choice == "words" && inner.lock().jobs.contains_key(map_id) {
        return Err(inner.busy());
    }
    let new_id = if choice == "map" {
        Some(start_map(
            inner,
            &offer.question,
            Some(ParentMap { map_id: map_id.into(), question: stored.map.question.clone() }),
        )?)
    } else {
        None
    };
    let mut updated = a.clone();
    updated.offer = Some(crate::explainer::Offer {
        status: if choice == "map" { OfferStatus::Mapped } else { OfferStatus::Answered },
        map_id: new_id.clone(),
        ..offer
    });
    let answers = stored
        .answers
        .iter()
        .map(|x| if x.get("id").and_then(Value::as_str) == Some(answer_id) { to_value(&updated) } else { x.clone() })
        .collect();
    inner.save(&StoredMap { answers, ..stored })?;
    inner.broadcast(ServerEvent::AnswerUpdate { map_id: map_id.into(), answer: updated });
    if choice == "words" {
        start_ask(inner, map_id, &a.question, Selection::default(), true)?;
    }
    Ok(match new_id {
        Some(id) => json!({ "id": id, "ok": true }),
        None => json!({ "ok": true }),
    })
}

fn approve_fix(inner: &Arc<Inner>, map_id: &str, fix_id: &str) -> Result<Value, HttpError> {
    let none = || HttpError::not_found("No such fix waiting for approval.");
    let stored = inner.store.get(map_id).ok_or_else(none)?;
    let proposal = pending(&stored, fix_id).ok_or_else(none)?;
    if inner.lock().jobs.contains_key(map_id) {
        return Err(inner.busy());
    }
    let applied = match apply_fix(&inner.root, &proposal) {
        Ok(a) => a,
        Err(problems) => {
            inner.broadcast(ServerEvent::Fix {
                map_id: map_id.into(),
                fix_id: fix_id.into(),
                status: FixEvent::Failed,
                problems: Some(problems.clone()),
                edge_id: None,
                closed: None,
            });
            return Err(HttpError::new(StatusCode::CONFLICT, "The fix could not be applied.").with_problems(&problems));
        }
    };
    let c = connect_after_fix(&inner.root, &stored.map, &proposal);
    let files: Vec<String> = applied.written.iter().map(|w| w.file.clone()).collect();
    inner
        .lock()
        .applied
        .insert(fix_id.into(), Applied { map_id: map_id.into(), applied, map_before: stored.map.clone() });
    let mut meta = stored.meta.clone();
    meta.updated_at = now_iso();
    let fixes = with_fix(
        &stored,
        fix_id,
        SavedFix { status: FixStatus::Applied, edge_id: c.edge_id.clone(), closed: Some(c.closed.clone()) },
    );
    let next = StoredMap { meta, map: c.map.clone(), answers: stored.answers.clone(), fixes };
    inner.save(&next)?;
    let report = inner.publish_map(&next)?;
    inner.broadcast(ServerEvent::Fix {
        map_id: map_id.into(),
        fix_id: fix_id.into(),
        status: FixEvent::Applied,
        problems: None,
        edge_id: Some(c.edge_id.clone()),
        closed: Some(c.closed.clone()),
    });
    Ok(json!({ "edgeId": c.edge_id, "closed": c.closed, "files": files, "summary": report.summary }))
}

fn undo(inner: &Arc<Inner>, map_id: &str, fix_id: &str) -> Result<(), HttpError> {
    let a = {
        let mut shared = inner.lock();
        match shared.applied.remove(fix_id) {
            Some(a) if a.map_id == map_id => a,
            Some(a) => {
                shared.applied.insert(fix_id.into(), a);
                return Err(HttpError::not_found("No applied fix to undo."));
            }
            None => return Err(HttpError::not_found("No applied fix to undo.")),
        }
    };
    if let Err(problems) = undo_fix(&inner.root, &a.applied) {
        inner.lock().applied.insert(fix_id.into(), a);
        return Err(HttpError::new(StatusCode::CONFLICT, "The fix could not be undone.").with_problems(&problems));
    }
    if let Some(stored) = inner.store.get(map_id) {
        let mut meta = stored.meta.clone();
        meta.updated_at = now_iso();
        let fixes = with_fix(&stored, fix_id, SavedFix { status: FixStatus::Proposed, edge_id: None, closed: None });
        let next = StoredMap { meta, map: a.map_before, answers: stored.answers, fixes };
        inner.save(&next)?;
        inner.publish_map(&next)?;
    }
    inner.broadcast(ServerEvent::Fix {
        map_id: map_id.into(),
        fix_id: fix_id.into(),
        status: FixEvent::Undone,
        problems: None,
        edge_id: None,
        closed: None,
    });
    Ok(())
}

// ---------------------------------------------------------------- stopping

/// (Re)starts the countdown to stopping, if nothing is keeping the server busy.
fn arm_idle(inner: &Arc<Inner>) {
    let mut shared = inner.lock();
    if let Some(t) = shared.idle.take() {
        t.abort();
    }
    let Some(auto) = inner.auto_stop else { return };
    if shared.closing || !shared.tabs.is_empty() || !shared.jobs.is_empty() {
        return;
    }
    let wait = if shared.visited { auto.after_last_tab } else { auto.before_first_tab };
    let inner2 = inner.clone();
    shared.idle = Some(tokio::spawn(async move {
        tokio::time::sleep(wait).await;
        let quiet = {
            let s = inner2.lock();
            s.tabs.is_empty() && s.jobs.is_empty()
        };
        // Stopping aborts this timer, so it runs on its own.
        if quiet {
            tokio::spawn(stop_self(inner2, StopReason::Idle));
        }
    }));
}

async fn stop_self(inner: Arc<Inner>, reason: StopReason) {
    let closing = inner.lock().closing;
    if closing {
        return;
    }
    close(&inner).await;
    if let Some(f) = &inner.on_stop {
        f(reason);
    }
}

async fn close(inner: &Arc<Inner>) {
    let tabs: Option<Vec<mpsc::UnboundedSender<ToTab>>> = {
        let mut shared = inner.lock();
        if shared.closing {
            None
        } else {
            shared.closing = true;
            if let Some(t) = shared.idle.take() {
                t.abort();
            }
            Some(shared.tabs.values().cloned().collect())
        }
    };
    let Some(tabs) = tabs else {
        inner.closed.cancelled().await;
        return;
    };
    for tab in &tabs {
        let _ = tab.send(ToTab::Stopped);
    }
    // Give the tabs a moment to hear it and close.
    let until = tokio::time::Instant::now() + Duration::from_millis(500);
    while !inner.lock().tabs.is_empty() && tokio::time::Instant::now() < until {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    inner.shutdown.cancel();
    let serve = inner.serve.lock().unwrap().take();
    if let Some(serve) = serve {
        let _ = serve.await;
    }
    inner.closed.cancel();
}

// ---------------------------------------------------------------- HTTP

fn json_response(status: StatusCode, body: &Value) -> Response {
    let mut res = (status, body.to_string()).into_response();
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}

fn error_response(e: &HttpError) -> Response {
    let mut body = Map::new();
    body.insert("error".into(), json!(e.message));
    body.extend(e.extra.clone());
    json_response(e.status, &Value::Object(body))
}

fn query(uri: &axum::http::Uri) -> HashMap<String, String> {
    form_urlencoded::parse(uri.query().unwrap_or("").as_bytes()).into_owned().collect()
}

impl Inner {
    fn hosts(&self) -> [String; 2] {
        [format!("127.0.0.1:{}", self.port), format!("localhost:{}", self.port)]
    }
    fn host_ok(&self, h: &HeaderMap) -> bool {
        let host = h.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("");
        self.hosts().iter().any(|x| x == host)
    }
    fn origin_ok(&self, h: &HeaderMap) -> bool {
        match h.get(header::ORIGIN) {
            None => true,
            Some(o) => self.hosts().iter().any(|x| o.to_str().ok() == Some(&format!("http://{x}"))),
        }
    }
    fn token_ok(&self, given: Option<&str>) -> bool {
        given.is_some_and(|g| same(g.as_bytes(), self.token.as_bytes()))
    }
}

async fn handle(State(inner): State<Arc<Inner>>, req: Request) -> Response {
    let (mut parts, body) = req.into_parts();
    if !inner.host_ok(&parts.headers) {
        return error_response(&HttpError::new(StatusCode::FORBIDDEN, "Wrong host."));
    }
    let path = parts.uri.path().to_string();
    let q = query(&parts.uri);
    if path == "/ws" {
        if !inner.origin_ok(&parts.headers) || !inner.token_ok(q.get("t").map(String::as_str)) {
            return StatusCode::FORBIDDEN.into_response();
        }
        return match WebSocketUpgrade::from_request_parts(&mut parts, &inner).await {
            Ok(up) => {
                let on_map = q.get("m").cloned();
                let inner = inner.clone();
                up.on_upgrade(move |ws| tab(inner, ws, on_map))
            }
            Err(e) => e.into_response(),
        };
    }
    if path.starts_with("/api/") {
        // Pictures are loaded by <img>, which can't send a header, so these two read-only routes also take the token
        // in the URL.
        let picture = parts.method == Method::GET
            && (path.starts_with("/api/screens/") || path == "/api/repo-image")
            && inner.token_ok(q.get("t").map(String::as_str));
        let header_token = parts.headers.get(TOKEN_HEADER).and_then(|v| v.to_str().ok());
        if !picture && !inner.token_ok(header_token) {
            return error_response(&HttpError::new(
                StatusCode::UNAUTHORIZED,
                "Missing or wrong token. Open nonio from the link it printed.",
            ));
        }
        if parts.method != Method::GET && !inner.origin_ok(&parts.headers) {
            return error_response(&HttpError::new(StatusCode::FORBIDDEN, "Wrong origin."));
        }
        return match api(&inner, &parts.method, &path, &q, body).await {
            Ok(r) => r,
            Err(e) => error_response(&e),
        };
    }
    if parts.method != Method::GET {
        return error_response(&HttpError::new(StatusCode::METHOD_NOT_ALLOWED, "Method not allowed."));
    }
    serve_static(&inner, &path)
}

async fn read_body(body: Body) -> Result<Value, HttpError> {
    let bytes = to_bytes(body, MAX_BODY)
        .await
        .map_err(|_| HttpError::new(StatusCode::PAYLOAD_TOO_LARGE, "Request too large."))?;
    if bytes.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_slice(&bytes).map_err(|_| HttpError::new(StatusCode::BAD_REQUEST, "Request body must be JSON."))
}

fn as_selection(v: Option<&Value>) -> Selection {
    let ids = |k: &str| -> Vec<String> {
        v.and_then(|s| s.get(k))
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).take(30).collect())
            .unwrap_or_default()
    };
    Selection { nodes: ids("nodes"), edges: ids("edges"), absences: ids("absences") }
}

fn str_of<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

async fn api(
    inner: &Arc<Inner>,
    m: &Method,
    path: &str,
    q: &HashMap<String, String>,
    body: Body,
) -> Result<Response, HttpError> {
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).skip(1).collect(); // after "api"
    let ok = |v: Value| Ok(json_response(StatusCode::OK, &v));
    let none = || Err(HttpError::not_found("Not found."));
    match (parts.as_slice(), m) {
        (["stop"], &Method::POST) => {
            if !inner.lock().jobs.is_empty() {
                return Err(HttpError::new(
                    StatusCode::CONFLICT,
                    format!(
                        "{} is still working on a map. Stop nonio once it finishes, so the map gets saved.",
                        inner.page.name
                    ),
                ));
            }
            // The server stops gracefully, so this answer still goes out.
            tokio::spawn(stop_self(inner.clone(), StopReason::Requested));
            return ok(json!({ "ok": true }));
        }
        (["screens", id], &Method::GET) => {
            let id = id.strip_suffix(".png").unwrap_or(id);
            read_capture(&inner.root, id).map_err(|_| HttpError::not_found("No such capture."))?;
            return send_file(&capture_paths(&inner.root, id).1, "image/png");
        }
        (["repo-image"], &Method::GET) => {
            let (file, kind) = repo_image(&inner.root, q.get("file").map(String::as_str).unwrap_or(""))
                .map_err(|_| HttpError::not_found("No such image."))?;
            return send_file(&file, kind);
        }
        (["maps"], &Method::GET) => {
            let making: Vec<Value> = inner
                .lock()
                .jobs
                .iter()
                .filter(|(_, j)| j.kind == JobKind::Make)
                .map(|(id, j)| json!({ "id": id, "question": j.question }))
                .collect();
            let repo = inner.root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            return ok(
                json!({ "repo": repo, "maps": inner.store.list(), "making": making, "agent": page_json(&inner.page) }),
            );
        }
        (["maps"], &Method::POST) => {
            let b = read_body(body).await?;
            let parent = b
                .get("parent")
                .and_then(Value::as_str)
                .filter(|p| valid_map_id(p))
                .and_then(|p| inner.store.get(p))
                .map(|s| ParentMap { map_id: s.meta.id, question: s.map.question });
            let id = start_map(inner, str_of(&b, "question"), parent)?;
            return Ok(json_response(StatusCode::ACCEPTED, &json!({ "id": id })));
        }
        _ => {}
    }
    if parts.first() != Some(&"maps") || parts.len() < 2 {
        return none();
    }
    let map_id = parts[1];
    if !valid_map_id(map_id) {
        return Err(HttpError::not_found("No such map."));
    }
    match (&parts[2..], m) {
        ([], &Method::GET) => {
            let job = inner.lock().jobs.get(map_id).cloned();
            let Some(stored) = inner.store.get(map_id) else {
                // The page also learns who asks here while the map is still being made, or it shows the wrong
                // question box.
                let failure = inner.lock().failures.get(map_id).cloned();
                if job.is_some() || failure.is_some() {
                    return ok(json!({
                        "meta": null, "map": null, "report": null, "job": job,
                        "failure": failure.map(|(question, error)| json!({ "question": question, "error": error })),
                        "answers": [], "agent": page_json(&inner.page),
                    }));
                }
                return Err(HttpError::not_found("No such map."));
            };
            let report = check_map(&stored.map, &inner.root).map_err(internal)?;
            return ok(json!({
                "meta": stored.meta, "map": stored.map, "answers": stored.answers, "fixes": inner.fix_states(&stored),
                "report": report, "job": job, "failure": null, "agent": page_json(&inner.page),
            }));
        }
        // The code behind a box or arrow, or behind the notes on an answer: read and checked now, never stored.
        (["code"], &Method::GET) => {
            let stored = inner.store.get(map_id).ok_or_else(|| HttpError::not_found("No code to show for that."))?;
            let when = if q.get("when").map(String::as_str) == Some("before") { When::Before } else { When::Now };
            let item = q.get("item").map(String::as_str).unwrap_or("");
            let scanner = Scanner::new(&inner.root, list_repo_files(&inner.root, DEFAULT_IGNORE));
            let view = code_of(&stored.map, &inner.root, &scanner, item, when)
                .map_err(internal)?
                .ok_or_else(|| HttpError::not_found("No code to show for that."))?;
            return ok(with_root(to_value(&view), &inner.root));
        }
        (["answers", answer_id, "code"], &Method::GET) => {
            let notes: Vec<CodeNote> = inner
                .store
                .get(map_id)
                .and_then(|s| answers_of(&s).into_iter().find(|a| a.id == *answer_id))
                .map(|a| a.notes)
                .unwrap_or_default();
            if notes.is_empty() {
                return Err(HttpError::not_found("No code to show for that."));
            }
            let scanner = Scanner::new(&inner.root, list_repo_files(&inner.root, DEFAULT_IGNORE));
            let view = code_for_notes(&inner.root, &scanner, &notes).map_err(internal)?;
            return ok(with_root(to_value(&view), &inner.root));
        }
        (["answers", answer_id, "offer"], &Method::POST) => {
            let b = read_body(body).await?;
            return ok(take_offer(inner, map_id, answer_id, b.get("choice").and_then(Value::as_str))?);
        }
        // What is selected on a map's page, so a question asked in the agent's chat is about it. Memory only.
        (["selection"], &Method::PUT) => {
            if inner.store.get(map_id).is_none() {
                return Err(HttpError::not_found("No such map."));
            }
            let b = read_body(body).await?;
            let step = b
                .get("step")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.chars().take(200).collect::<String>());
            let selection = as_selection(b.get("selection"));
            inner.lock().selections.insert(map_id.into(), PageSelection { selection, step });
            inner.saw_page(map_id);
            return ok(json!({ "ok": true }));
        }
        (["ask"], &Method::POST) => {
            let b = read_body(body).await?;
            start_ask(inner, map_id, str_of(&b, "question"), as_selection(b.get("selection")), false)?;
            return Ok(json_response(StatusCode::ACCEPTED, &json!({ "ok": true })));
        }
        (["recheck"], &Method::POST) => {
            let stored = inner.store.get(map_id).ok_or_else(|| HttpError::not_found("No such map."))?;
            return ok(json!({ "report": inner.publish_map(&stored)? }));
        }
        (["fixes", fix_id, "approve"], &Method::POST) => return ok(approve_fix(inner, map_id, fix_id)?),
        (["fixes", fix_id, "undo"], &Method::POST) => {
            undo(inner, map_id, fix_id)?;
            return ok(json!({ "ok": true }));
        }
        (["fixes", fix_id, "reject"], &Method::POST) => {
            let none = || HttpError::not_found("No such fix waiting for approval.");
            let stored = inner.store.get(map_id).ok_or_else(none)?;
            pending(&stored, fix_id).ok_or_else(none)?;
            let fixes =
                with_fix(&stored, fix_id, SavedFix { status: FixStatus::Rejected, edge_id: None, closed: None });
            inner.save(&StoredMap { fixes, ..stored })?;
            inner.broadcast(ServerEvent::Fix {
                map_id: map_id.into(),
                fix_id: (*fix_id).into(),
                status: FixEvent::Rejected,
                problems: None,
                edge_id: None,
                closed: None,
            });
            return ok(json!({ "ok": true }));
        }
        _ => {}
    }
    none()
}

fn with_root(mut v: Value, root: &Path) -> Value {
    if let Value::Object(o) = &mut v {
        o.insert("root".into(), json!(root.to_string_lossy()));
    }
    v
}

/// A picture from the repo. Never run as a page: no sniffing, and no scripts even if an SVG is opened directly.
fn send_file(file: &Path, kind: &str) -> Result<Response, HttpError> {
    let bytes = fs::read(file).map_err(|_| HttpError::not_found("Not found."))?;
    let mut res = bytes.into_response();
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_str(kind).map_err(internal)?);
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; style-src 'unsafe-inline'"),
    );
    Ok(res)
}

/// Reads one of the built viewer's files, by its path inside the build (`index.html`, `assets/…`).
pub type ViewerFiles = Arc<dyn Fn(&str) -> Option<std::borrow::Cow<'static, [u8]>> + Send + Sync>;

/// Where the built viewer's files come from.
#[derive(Clone)]
pub enum Viewer {
    /// A folder on disk, such as `viewer/dist` while working on the viewer.
    Dir(PathBuf),
    /// Files built into the binary.
    Files(ViewerFiles),
}

impl Viewer {
    fn read(&self, rel: &str) -> Option<Vec<u8>> {
        match self {
            Viewer::Dir(dir) => {
                let file = dir.join(rel);
                if file.is_file() {
                    fs::read(file).ok()
                } else {
                    None
                }
            }
            Viewer::Files(f) => f(rel).map(|b| b.into_owned()),
        }
    }
}

impl std::fmt::Debug for Viewer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Viewer::Dir(d) => f.debug_tuple("Dir").field(d).finish(),
            Viewer::Files(_) => f.write_str("Files"),
        }
    }
}

const NOT_BUILT: &str = "<!doctype html><meta charset=\"utf-8\"><title>nonio</title><p style=\"font:16px system-ui;margin:40px\">The viewer isn't built yet. Run <code>npm run build</code> in the nonio repo's <code>viewer/</code>, then reload.</p>";

fn content_type(file: &Path) -> &'static str {
    match file.extension().and_then(|x| x.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript",
        "css" => "text/css",
        "svg" => "image/svg+xml",
        "json" | "map" => "application/json",
        "woff2" => "font/woff2",
        "png" => "image/png",
        "ico" => "image/x-icon",
        _ => "application/octet-stream",
    }
}

fn serve_static(inner: &Inner, path: &str) -> Response {
    let index = inner.viewer.as_ref().and_then(|v| v.read("index.html"));
    let (Some(viewer), Some(index)) = (inner.viewer.as_ref(), index) else {
        let mut res = NOT_BUILT.into_response();
        res.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
        return res;
    };
    let rel = percent_encoding::percent_decode_str(path.trim_start_matches('/')).decode_utf8_lossy().to_string();
    let safe = rel.starts_with("assets/")
        && !rel.contains('\\')
        && rel.split('/').all(|s| !s.is_empty() && s != "." && s != "..");
    let asset = if safe { viewer.read(&rel).map(|b| (b, rel.as_str())) } else { None };
    let is_index = asset.is_none();
    let (bytes, name) = asset.unwrap_or((index, "index.html"));
    let mut res = bytes.into_response();
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type(Path::new(name))));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    if is_index {
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        h.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'self'; connect-src 'self' ws://127.0.0.1:* ws://localhost:*; img-src 'self' data:; style-src 'self' 'unsafe-inline'; font-src 'self'"),
        );
    } else {
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=31536000, immutable"));
    }
    res
}

// ---------------------------------------------------------------- tabs

async fn tab(inner: Arc<Inner>, ws: WebSocket, on_map: Option<String>) {
    let (mut tx, mut rx) = ws.split();
    let stopped = || Message::Close(Some(CloseFrame { code: STOPPED_CLOSE_CODE, reason: "nonio stopped".into() }));
    let (send, mut outbox) = mpsc::unbounded_channel();
    let id = inner.next_tab.fetch_add(1, Ordering::Relaxed);
    let closing = {
        let mut shared = inner.lock();
        if !shared.closing {
            shared.tabs.insert(id, send);
            shared.visited = true;
        }
        shared.closing
    };
    if closing {
        let _ = tx.send(stopped()).await;
        return;
    }
    // A map's page names its map when it connects, so the agent's chat knows which map is open.
    if let Some(m) = on_map.filter(|m| valid_map_id(m) && inner.store.get(m).is_some()) {
        inner.saw_page(&m);
    }
    arm_idle(&inner);
    loop {
        tokio::select! {
            out = outbox.recv() => match out {
                Some(ToTab::Event(text)) => {
                    if tx.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                Some(ToTab::Stopped) => {
                    let _ = tx.send(stopped()).await;
                }
                None => break,
            },
            got = rx.next() => match got {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                Some(Ok(_)) => {}
            },
            _ = inner.shutdown.cancelled() => break,
        }
    }
    inner.lock().tabs.remove(&id);
    arm_idle(&inner);
}
