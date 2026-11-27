//! nonio as an MCP server the coding agent starts over stdio (`nonio mcp`, D29), so the person's own agent does the
//! reading and writing, on their own sign-in, and nonio checks it (D06).
//!
//! The page server runs inside this process with the bridge as its agent (bridge.rs). Work reaches the model in tool
//! results:
//! - `nonio_map` and `nonio_submit` wait for the next request, so making a map (draft, check, repair) runs within one
//!   turn;
//! - where the host can run a call in the background and wake the session (Claude Code, `wake` in the host profile),
//!   the model keeps one `nonio_listen` call open between turns. When the person asks on the map, it returns with the
//!   request and the host wakes the session. Claude Code drops a call that has been quiet for 30 minutes even with
//!   progress notifications, so a listening call returns on its own before then and the model starts another; after
//!   enough quiet ones it stops. A session there also gets a `notifications/claude/channel` message when work arrives
//!   and nothing is listening (then the model calls `nonio_next`).
//! - everywhere else (Codex, Cursor, any other client: the base protocol, D07) there is no listening: a question
//!   about the map is asked in the agent's chat and comes in through `nonio_ask`, no call waits past the profile's
//!   `max_wait`, `nonio_next` is how the model carries on waiting, and a request longer than the profile's
//!   `max_output` comes in parts (`nonio_more`).
//!
//! Only the person's browser can start work, and the page needs nonio's token.
//!
//! Ported from `packages/server/src/channel.ts` (MIT, Copyright (c) 2026 johnz4021); see NOTICE. What differs: the
//! original tells only Codex from Claude Code; nonio asks the host profile, and a client it doesn't know gets the base
//! protocol. The instructions are written for the client that connected (it names itself in `initialize`, before
//! they are sent). Update notices wait for nonio's distribution to be settled.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use base64::Engine;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, CustomNotification, Implementation,
    InitializeRequestParams, InitializeResult, ListToolsResult, PaginatedRequestParams, ProgressNotificationParam,
    ProgressToken, ServerCapabilities, ServerNotification, Tool, ToolAnnotations,
};
use rmcp::service::{Peer, RequestContext};
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::{json, Map, Value};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::bridge::{Bridge, BridgeOptions, BridgeRequest};
use crate::explainer::{Answer, OfferStatus};
use crate::host::{host_of, HostProfile, HOST_ENV};
use crate::reply::chat_reply;
use crate::request::{parts_of, request_text};
use crate::screens::{capture, list_screens, CaptureTarget, ScreenScan, Shooter};
use crate::server::{start_server, AutoStop, JobKind, JobStatus, NonioServer, ServerEvent, ServerOptions};
use crate::store::Store;
use crate::tools::{ASK, CAPTURE, FAIL, LISTEN, MAP, MORE, NEXT, OPEN, SCREENS, SUBMIT};

/// The port a page server asks for first. A steady port means a browser asks to allow it only once.
pub const PORT: u16 = 4317;
const VERSION: &str = env!("CARGO_PKG_VERSION");

pub type OpenFn = Arc<dyn Fn(&str) + Send + Sync>;

pub struct ChannelOptions {
    /// The repo maps are about unless a tool call names another.
    pub root: PathBuf,
    /// How a link is opened in the browser (tests pass a stand-in).
    pub open: Option<OpenFn>,
    /// How long `nonio_map` and `nonio_submit` wait for what comes next (default: the host profile's `max_wait`).
    pub wait: Option<Duration>,
    /// The longest text one tool result carries before a request is split into parts (default: the profile's).
    pub part_chars: Option<usize>,
    /// How a page is screenshotted (tests pass a stand-in for the browser).
    pub shoot: Option<Shooter>,
    /// How often a waiting call sends progress.
    pub heartbeat: Duration,
    /// How long one `nonio_listen` call waits before returning (default: the profile's `listen`).
    pub listen: Option<Duration>,
    /// How many quiet listening calls in a row before nonio stops listening (default 5, about two hours).
    pub quiet_listens: u32,
    /// How long after the last nonio tab closes the page's server stops (enough for a reload).
    pub after_last_tab: Duration,
    /// Port for the page server (default 4317, or the next free one; 0 for any).
    pub port: u16,
    /// The built viewer, for the page server.
    pub viewer: Option<crate::server::Viewer>,
    /// The profile to use whatever the client says (`NONIO_HOST`).
    pub forced_host: Option<String>,
}

impl ChannelOptions {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        ChannelOptions {
            root: root.into(),
            open: None,
            wait: None,
            part_chars: None,
            shoot: None,
            heartbeat: Duration::from_secs(30),
            listen: None,
            quiet_listens: 5,
            after_last_tab: Duration::from_secs(5),
            port: PORT,
            viewer: None,
            forced_host: std::env::var(HOST_ENV).ok(),
        }
    }
}

/// What every agent is told, and then what differs where it runs.
pub fn instructions(host: &HostProfile) -> String {
    let mut lines = vec![
        "nonio draws checked, interactive maps of how this codebase works, in the person's browser. You do the reading and writing; nonio checks every claim against the code before it shows anything.".to_string(),
        format!("- When the person asks how something works across files (\"what happens when…\", \"how does X reach Y\", \"what did this change do\"), what someone sees or goes through in the app (a user journey), or how a change would look before it's made (\"how would we add…\", \"plan…\"), or asks for a map, diagram or walkthrough, call {MAP} with their question in their own words."),
        "- A map can hold proposals: boxes and arrows that aren't in the code yet, or that would change or go, drawn on top of the code as it is. The request explains how. nonio works out from the code which proposals are done, so after you build a change you proposed on a map, the person can reopen that map to see it.".to_string(),
        "- A map can show a change that was made: \"what did you just change?\", \"what did the last commit do?\". The request explains how. You draw the code as it is now and name the commit to compare with; nonio works out from the two versions what is new, different or gone. Use it when the person wants to see what a task changed.".to_string(),
        format!("- nonio gives you work as requests in tool results. Each request says what to do and the JSON to return. Explore the code with your read-only tools (read, search, list files), then call {SUBMIT} with the request_id and your output. If the result holds another request, do that one too. If you can't finish one, call {FAIL}."),
        format!("- A picture of the screen can ground a map for people who don't read code. Use your judgement: when a flow starts from something the person sees (a button, a form, a page) a screen on that box helps; for backend-only questions leave screens out. {SCREENS} lists what this repo can show; {CAPTURE} screenshots a page already running on this machine, or an HTML file in the repo, and shows you the result. Look at it before using it: if it shows a login page, an error or the wrong state, don't use it. Never start the app yourself to get a screenshot. When nothing is running, a map can still show a wireframe of the page drawn from the code; the request explains how."),
        "- While working on a nonio request, don't edit files or run commands that change anything, and never read secret files (.env, keys). A fix is only proposed; nonio applies it after the person approves it on the map.".to_string(),
        "- Answers are shown on the map. In the chat or terminal, say at most a sentence or two.".to_string(),
    ];
    if host.wake {
        lines.push(format!("- Once a map is ready or an answer is shown, call {LISTEN} and leave it running. It moves to the background, so the person keeps their terminal. When they ask something on the map it returns with a request: do it, submit it, then call {LISTEN} again. If it returns saying nothing was asked yet, call it again without telling the person. Only one is needed at a time."));
        lines.push(format!("- If a nonio channel message arrives instead, call {NEXT} to get the request."));
    } else {
        if host.repo_from_env.is_none() {
            lines.push(format!("- Pass `repo`, the absolute path of the project, to {MAP} and {OPEN}."));
        }
        lines.push(if host.inline_browser {
            "- Give the person the map's link, and if you have a built-in browser, open it there so the map sits beside the chat.".to_string()
        } else {
            "- The map opens in the person's browser; give them the link too.".to_string()
        });
        lines.push(format!("- While a map is open and the person asks about it in the chat (the map, a box or arrow, what they selected, a walkthrough step), call {ASK} with their question and the page's current URL if you know it. nonio adds what they selected on the page."));
        lines.push(format!("- A long request comes in parts: read every part with {MORE} before you start. If a result says nonio is still working, or the person says to carry on with nonio, call {NEXT}."));
    }
    lines.join("\n")
}

struct ToolDef {
    name: &'static str,
    title: &'static str,
    read_only: bool,
    description: String,
    schema: Value,
}

/// The tools as this agent sees them. Codex asks the person before each MCP tool call unless a tool says it only
/// reads, or that it neither destroys anything nor reaches outside this machine (openai/codex `mcp_tool_call.rs`);
/// nonio's tools write only under `.nonio/` and talk only to this machine, so they say so, and `readOnlyHint` only
/// where it's true.
pub fn tools_for(host: &HostProfile) -> Vec<Tool> {
    let repo = if host.repo_from_env.is_some() {
        json!({ "type": "string", "description": "The repo root, if not the project the agent was started in." })
    } else {
        json!({ "type": "string", "description": "The absolute path of the repo you are working in. Pass it every time." })
    };
    let chat = !host.wake;
    let map_description = if !chat {
        "Make a checked map that answers a question about how this codebase works, and open it in the person's browser. Returns the first piece of work for you to do.".to_string()
    } else if host.inline_browser {
        "Make a checked map that answers a question about how this codebase works. Returns the map page's link to give the person (open it in your built-in browser if you have one) and the first piece of work for you to do.".to_string()
    } else {
        "Make a checked map that answers a question about how this codebase works, and open it in the person's browser. Returns the map page's link to give the person and the first piece of work for you to do.".to_string()
    };
    let mut defs = vec![
        ToolDef {
            name: MAP,
            title: "Make a map",
            read_only: false,
            description: map_description,
            schema: json!({
                "type": "object",
                "properties": {
                    "question": { "type": "string", "maxLength": 300, "description": "The person's question, in their own words (300 characters at most)." },
                    "repo": repo,
                },
                "required": ["question"],
            }),
        },
        ToolDef {
            name: NEXT,
            title: "Get the next request",
            read_only: true,
            description: if chat {
                format!("Get the next piece of work nonio has for you, waiting up to {} seconds for it. Call it when a result says nonio is still working, or when the person says to carry on with nonio.", host.max_wait.as_secs())
            } else {
                "Get the next piece of work nonio has for you (after a nonio channel message).".into()
            },
            schema: json!({ "type": "object", "properties": {} }),
        },
        ToolDef {
            name: SUBMIT,
            title: "Hand in a request",
            read_only: false,
            description: "Hand in your output for a nonio request. It is checked; if it can't be read you get the problems back to fix. Returns the next piece of work, if there is one.".into(),
            schema: json!({
                "type": "object",
                "properties": {
                    "request_id": { "type": "string" },
                    "output": { "type": "object", "description": "JSON matching the request's schema." },
                },
                "required": ["request_id", "output"],
            }),
        },
        ToolDef {
            name: FAIL,
            title: "Give up on a request",
            read_only: false,
            description: "Give up on a nonio request, saying why in plain words. The person sees it on the map.".into(),
            schema: json!({
                "type": "object",
                "properties": { "request_id": { "type": "string" }, "reason": { "type": "string" } },
                "required": ["request_id", "reason"],
            }),
        },
    ];
    if chat {
        defs.push(ToolDef {
            name: ASK,
            title: "Ask about the open map",
            read_only: false,
            description: "Ask nonio a question the person asked in this chat about a nonio map that is open in the browser. nonio adds what they have selected on the page and starts the answer; returns the first piece of work for you to do. The answer is shown on the map.".into(),
            schema: json!({
                "type": "object",
                "properties": {
                    "question": { "type": "string", "maxLength": 2000, "description": "The person's question, in their own words." },
                    "url": { "type": "string", "description": "The map page's current URL in the browser (http://127.0.0.1:…/m/…), if you know it." },
                },
                "required": ["question"],
            }),
        });
        defs.push(ToolDef {
            name: MORE,
            title: "Read the rest of a request",
            read_only: true,
            description: "Read the next part of a nonio request that came in parts. Read every part before you start on it.".into(),
            schema: json!({
                "type": "object",
                "properties": {
                    "request_id": { "type": "string" },
                    "part": { "type": "number", "description": "Which part (2 for the second). Leave out for the next one." },
                },
                "required": ["request_id"],
            }),
        });
    } else {
        defs.push(ToolDef {
            name: LISTEN,
            title: "Listen to the map",
            read_only: true,
            description: "Wait for the next question the person asks on the map (or a new map they start there). Keep one running whenever you have no nonio work: it moves to the background, and returns with a request when they ask. Do the request, submit it, then call this again.".into(),
            schema: json!({ "type": "object", "properties": {} }),
        });
    }
    defs.push(ToolDef {
        name: SCREENS,
        title: "List screens",
        read_only: true,
        description: "List what this repo can show of its screens: local pages being served right now, HTML files, preview setups and image files. Costs nothing; call it when a picture of the screen might help a map or an answer.".into(),
        schema: json!({ "type": "object", "properties": { "repo": repo } }),
    });
    defs.push(ToolDef {
        name: CAPTURE,
        title: "Screenshot a local page",
        read_only: false,
        description: "Screenshot one page so it can be a box's `screen` on a map: a `url` served on this machine (http://localhost:…), or an HTML `file` in the repo. Returns the image for you to look at, the labelled things visible on it, and the capture id to put in the box's `screen`. Runs in a clean browser profile, so pages behind a login show their login page.".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "A page on this machine, e.g. http://localhost:3000/patients" },
                "file": { "type": "string", "description": "Or: a repo-relative .html file" },
                "repo": repo,
            },
        }),
    });
    defs.push(ToolDef {
        name: OPEN,
        title: "Open saved maps",
        read_only: false,
        description: if host.inline_browser {
            "Get the link to saved maps: one map, or the list of all of them, to open in your built-in browser or give the person. Returns the maps saved in the repo.".into()
        } else {
            "Open saved maps in the person's browser: one map, or the list of all of them. Returns the maps saved in the repo.".into()
        },
        schema: json!({
            "type": "object",
            "properties": {
                "map_id": { "type": "string", "description": "A saved map's id; leave out for the list." },
                "repo": repo,
            },
        }),
    });
    defs.into_iter()
        .map(|d| {
            let schema = match d.schema {
                Value::Object(o) => o,
                _ => Map::new(),
            };
            let mut t = Tool::new(d.name, d.description, Arc::new(schema));
            t.annotations = Some(
                ToolAnnotations::with_title(d.title)
                    .read_only(d.read_only)
                    .destructive(false)
                    .idempotent(false)
                    .open_world(false),
            );
            t
        })
        .collect()
}

fn text(t: impl Into<String>) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(t)])
}

fn error(t: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(t)])
}

fn closed_text() -> String {
    format!("The person closed nonio, so there is nothing to listen to. Don't call {LISTEN} again unless they ask for a map or ask you to listen. Say nothing to the person about this.")
}

struct Report {
    confirmed: usize,
    claims: usize,
    hidden: Vec<String>,
}

#[derive(Default)]
struct State {
    client: Option<String>,
    peer: Option<Peer<RoleServer>>,
    servers: HashMap<PathBuf, NonioServer>,
    /// Listening calls in a row that ended with no question. Any other nonio call starts the count again.
    quiet: u32,
    /// Listening calls to end when the person closes nonio (Stop, or the last tab).
    closed_listeners: Vec<CancellationToken>,
    /// Waiting tool calls to tell when something finished.
    finished: Vec<(u64, mpsc::UnboundedSender<String>)>,
    next_waiter: u64,
    last_report: HashMap<String, Report>,
    /// The latest answer on each map, so a chat-only host can be told when it offered a new map.
    last_answer: HashMap<String, Answer>,
    /// Requests that came in parts, and the next part to hand over.
    parts: HashMap<String, (Vec<String>, usize)>,
}

struct Inner {
    o: ChannelOptions,
    state: Mutex<State>,
    /// One page server starts at a time.
    starting: tokio::sync::Mutex<()>,
    bridge: Bridge,
}

/// The MCP server. Serve it with rmcp, e.g. `channel.serve(rmcp::transport::stdio())`.
#[derive(Clone)]
pub struct Channel {
    inner: Arc<Inner>,
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Who is on the other end, from the name it sent when it connected (the base protocol until then).
    fn host(&self) -> &'static HostProfile {
        host_of(self.lock().client.as_deref(), self.o.forced_host.as_deref())
    }

    fn wait(&self) -> Duration {
        self.o.wait.unwrap_or(self.host().max_wait)
    }

    fn part_chars(&self) -> Option<usize> {
        self.o.part_chars.or(self.host().max_output)
    }

    fn open(&self, url: &str) {
        match &self.o.open {
            Some(f) => f(url),
            None => crate::browser::open_browser(url),
        }
    }
}

impl Channel {
    pub fn new(o: ChannelOptions) -> Self {
        let inner = Arc::new_cyclic(|weak: &std::sync::Weak<Inner>| {
            let (w1, w2) = (weak.clone(), weak.clone());
            let mut b = BridgeOptions::new(move |r: &BridgeRequest| {
                let Some(inner) = w1.upgrade() else { return };
                // Only a host that can be woken hears about work; the map page tells the person what to say elsewhere.
                if !inner.host().wake {
                    return;
                }
                let Some(peer) = inner.lock().peer.clone() else { return };
                let params = json!({
                    "content": format!("nonio has work for you: {}. Call {NEXT} to get it.", r.label),
                    "meta": { "request_id": r.id },
                });
                let n = ServerNotification::CustomNotification(CustomNotification::new(
                    "notifications/claude/channel",
                    Some(params),
                ));
                tokio::spawn(async move {
                    let _ = peer.send_notification(n).await;
                });
            });
            b.host = Arc::new(move || match w2.upgrade() {
                Some(inner) => inner.host(),
                None => &crate::host::BASE,
            });
            Inner {
                o,
                state: Mutex::new(State::default()),
                starting: tokio::sync::Mutex::new(()),
                bridge: Bridge::new(b),
            }
        });
        Channel { inner }
    }

    pub fn bridge(&self) -> &Bridge {
        &self.inner.bridge
    }

    /// Stops every page server.
    pub async fn close(&self) {
        let servers: Vec<NonioServer> = self.inner.lock().servers.drain().map(|(_, s)| s).collect();
        for s in servers {
            s.close().await;
        }
    }

    fn root_of(&self, repo: Option<&str>) -> Result<PathBuf, String> {
        let dir = match repo.map(str::trim).filter(|r| !r.is_empty()) {
            Some(r) => self.inner.o.root.join(r),
            None => self.inner.o.root.clone(),
        };
        if !dir.is_dir() {
            return Err(format!("No such folder: {}", dir.display()));
        }
        std::fs::canonicalize(&dir).map_err(|e| e.to_string())
    }

    async fn server_for(&self, root: &Path) -> Result<NonioServer, String> {
        let _one = self.inner.starting.lock().await;
        if let Some(s) = self.inner.lock().servers.get(root) {
            return Ok(s.clone());
        }
        let mut so = ServerOptions::new(root, Arc::new(self.inner.bridge.clone()));
        so.port = self.inner.o.port;
        so.page = self.inner.host().page.clone();
        so.viewer = self.inner.o.viewer.clone();
        // Stop, or closing the last tab, stops the page's server and ends listening. The MCP server itself stays
        // connected, so the next nonio tool call starts a new page server.
        so.auto_stop = Some(AutoStop { after_last_tab: self.inner.o.after_last_tab, ..AutoStop::default() });
        let (w1, w2) = (Arc::downgrade(&self.inner), Arc::downgrade(&self.inner));
        let (r1, r2) = (root.to_path_buf(), root.to_path_buf());
        so.on_stop = Some(Arc::new(move |_| {
            let Some(inner) = w1.upgrade() else { return };
            let mut s = inner.lock();
            s.servers.remove(&r1);
            if s.servers.is_empty() {
                for l in s.closed_listeners.drain(..) {
                    l.cancel();
                }
            }
        }));
        so.on_event = Some(Arc::new(move |e: &ServerEvent| {
            if let Some(inner) = w2.upgrade() {
                on_event(&inner, &r2, e);
            }
        }));
        let server = start_server(so).await.map_err(|e| e.to_string())?;
        self.inner.lock().servers.insert(root.to_path_buf(), server.clone());
        Ok(server)
    }

    /// A request as a tool result. One longer than the host keeps of a tool result comes in parts.
    fn deliver(&self, r: &BridgeRequest, after: &str) -> String {
        let whole = if after.is_empty() { request_text(r) } else { format!("{}\n\n{after}", request_text(r)) };
        let Some(max) = self.inner.part_chars() else { return whole };
        if whole.chars().count() <= max {
            return whole;
        }
        let all = parts_of(&whole, max);
        let first = part_text(&r.id, 0, &all);
        self.inner.lock().parts.insert(r.id.clone(), (all, 1));
        first
    }

    /// After handing something in: the next request, or what finished, or (after a while) that nothing is needed yet.
    async fn what_next(&self, ctx: &RequestContext<RoleServer>) -> CallToolResult {
        let stop = keep_alive(ctx, self.inner.o.heartbeat);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let id = {
            let mut s = self.inner.lock();
            s.next_waiter += 1;
            let id = s.next_waiter;
            s.finished.push((id, tx));
            id
        };
        let cancel = ctx.ct.child_token();
        let out = tokio::select! {
            r = self.inner.bridge.wait_for_next(&cancel) => r.map(|r| text(self.deliver(&r, ""))),
            m = rx.recv() => m.map(text),
            _ = tokio::time::sleep(self.inner.wait()) => None,
        };
        cancel.cancel();
        stop.cancel();
        self.inner.lock().finished.retain(|(w, _)| *w != id);
        out.unwrap_or_else(|| {
            if self.inner.host().wake {
                text(format!(
                    "nonio is still working on its side. Call {LISTEN}: it returns when there is something for you."
                ))
            } else {
                text(format!(
                    "nonio is still working on its side. Call {NEXT} to carry on: it waits for what comes next."
                ))
            }
        })
    }

    /// The page a question in the chat is about: the server and map its URL names, if it names one of nonio's; else
    /// the map whose page was used last, on any server running.
    fn page_for(&self, url: Option<&str>) -> Option<(NonioServer, Option<String>)> {
        let running: Vec<NonioServer> = self.inner.lock().servers.values().cloned().collect();
        if let Some(u) = url.and_then(|u| url::Url::parse(u).ok()) {
            let local = matches!(u.host_str(), Some("127.0.0.1" | "localhost"));
            if let Some(s) = running.iter().find(|s| local && u.port() == Some(s.port)) {
                let id = u.path().strip_prefix("/m/").map(|p| p.trim_end_matches('/')).filter(|p| {
                    (1..=80).contains(&p.len())
                        && p.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                });
                return Some((s.clone(), id.map(str::to_string)));
            }
        }
        running
            .into_iter()
            .filter_map(|s| s.last_seen().map(|seen| (seen.at, s)))
            .max_by_key(|(at, _)| *at)
            .map(|(_, s)| (s, None))
    }

    fn port_note(&self, s: &NonioServer) -> String {
        if !self.inner.host().wake && self.inner.o.port == PORT && s.port != PORT {
            format!("nonio is on port {} this time, since {PORT} was taken, so the browser may ask again whether to open it.", s.port)
        } else {
            String::new()
        }
    }

    async fn call(
        &self,
        name: &str,
        args: &Map<String, Value>,
        ctx: &RequestContext<RoleServer>,
    ) -> Result<CallToolResult, String> {
        let s = |k: &str| args.get(k).and_then(Value::as_str);
        if name != LISTEN {
            self.inner.lock().quiet = 0;
        }
        let host = self.inner.host();
        let chat = !host.wake;
        match name {
            MAP => {
                let question = s("question").unwrap_or("").trim().to_string();
                if question.is_empty() {
                    return Ok(error("Pass the person's question."));
                }
                let server = self.server_for(&self.root_of(s("repo"))?).await?;
                let id = server.start_map(&question).map_err(|e| e.message)?;
                let url = server.page_url(Some(&id));
                // A host with a browser beside the chat opens the map there itself.
                if !host.inline_browser {
                    self.inner.open(&url);
                }
                let first = self.what_next(ctx).await;
                let opening = if host.inline_browser {
                    [
                        format!(
                            "The map is being made. {} The person can watch it being drawn while you work.",
                            show_inline(&url)
                        ),
                        self.port_note(&server),
                    ]
                    .into_iter()
                    .filter(|l| !l.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n")
                } else {
                    format!("The map is opening in the browser: {url}\nTell the person, and give them the link in case the tab didn't appear.")
                };
                Ok(prepend(first, format!("{opening}\n")))
            }
            ASK if chat => {
                let question = s("question").unwrap_or("").trim().to_string();
                if question.is_empty() {
                    return Ok(error("Pass the person's question."));
                }
                let Some((server, map_id)) = self.page_for(s("url")) else {
                    return Ok(error(format!("No nonio map is open right now. To answer from a map, make one with {MAP}, or open a saved one with {OPEN}.")));
                };
                let asked = server.ask_from_chat(&question, map_id.as_deref()).map_err(|e| e.message)?;
                let first = self.what_next(ctx).await;
                let about = if asked.selected.is_empty() {
                    String::new()
                } else {
                    format!(
                        " about {} (selected on the page)",
                        asked.selected.iter().map(|n| format!("\"{n}\"")).collect::<Vec<_>>().join(", ")
                    )
                };
                Ok(prepend(first, format!("Asked on the map \"{}\"{about}.\n", asked.map_id)))
            }
            MORE if chat => {
                let id = s("request_id").unwrap_or("").to_string();
                let mut st = self.inner.lock();
                let Some((all, next)) = st.parts.get_mut(&id) else {
                    return Ok(error(format!("Request \"{id}\" didn't come in parts, or isn't waiting any more.")));
                };
                let want = match args.get("part").and_then(Value::as_f64) {
                    Some(p) => p.trunc() as i64 - 1,
                    None => *next as i64,
                };
                if want < 0 || want as usize >= all.len() {
                    return Ok(error(format!("Request {id} has parts 1 to {}.", all.len())));
                }
                let want = want as usize;
                *next = (want + 1).min(all.len() - 1);
                Ok(text(part_text(&id, want, all)))
            }
            LISTEN => {
                // Not offered where the host can't be woken: a waiting call there holds up the person's turn.
                if chat {
                    return Ok(error(format!(
                        "{LISTEN} isn't used here. When the person asks about the map in the chat, call {ASK}."
                    )));
                }
                let closed = CancellationToken::new();
                {
                    let mut st = self.inner.lock();
                    if st.servers.is_empty() {
                        return Ok(text(closed_text()));
                    }
                    st.closed_listeners.push(closed.clone());
                }
                let stop = keep_alive(ctx, self.inner.o.heartbeat);
                let cancel = ctx.ct.child_token();
                let listen = self.inner.o.listen.unwrap_or(host.listen);
                let got = tokio::select! {
                    r = self.inner.bridge.wait_for_next(&cancel) => r,
                    _ = tokio::time::sleep(listen) => None,
                    _ = closed.cancelled() => None,
                };
                cancel.cancel();
                stop.cancel();
                self.inner.lock().closed_listeners.retain(|c| !c.is_cancelled() && !std::ptr::eq(c, &closed));
                if let Some(r) = got {
                    self.inner.lock().quiet = 0;
                    return Ok(text(self.deliver(
                        &r,
                        &format!("After {SUBMIT}, call {LISTEN} again so questions on the map keep reaching you."),
                    )));
                }
                if closed.is_cancelled() {
                    return Ok(text(closed_text()));
                }
                if ctx.ct.is_cancelled() {
                    return Ok(text(format!(
                        "Stopped listening. Questions asked on the map wait until you call {LISTEN} again."
                    )));
                }
                let mut st = self.inner.lock();
                st.quiet += 1;
                if st.quiet >= self.inner.o.quiet_listens {
                    // The model is told not to listen again on its own; if the person asks it to, that call starts afresh.
                    st.quiet = 0;
                    return Ok(text(format!("Nobody has asked on the map for about two hours, so nonio stopped listening. Don't call {LISTEN} again unless the person asks you to listen, or asks for a map. Say nothing to the person about this.")));
                }
                Ok(text(format!("Nothing was asked on the map yet. Call {LISTEN} again to keep listening, and say nothing to the person about it.")))
            }
            NEXT => {
                if let Some(r) = self.inner.bridge.next() {
                    return Ok(text(self.deliver(&r, "")));
                }
                // Where calls can't run long, this is also how to keep waiting while nonio has work going.
                let busy = self.inner.lock().servers.values().any(NonioServer::working);
                if chat && busy {
                    return Ok(self.what_next(ctx).await);
                }
                Ok(text("Nothing is waiting right now."))
            }
            SUBMIT => {
                let id = s("request_id").unwrap_or("").to_string();
                let output = args.get("output").cloned().unwrap_or(Value::Null);
                match self.inner.bridge.submit(&id, output) {
                    Ok(()) => {
                        self.inner.lock().parts.remove(&id);
                        Ok(self.what_next(ctx).await)
                    }
                    Err(problems) => Ok(error(format!(
                        "That output for {id} can't be used yet. Fix these and call {SUBMIT} again:\n{}",
                        problems.iter().map(|p| format!("- {p}")).collect::<Vec<_>>().join("\n")
                    ))),
                }
            }
            FAIL => {
                let id = s("request_id").unwrap_or("");
                self.inner.lock().parts.remove(id);
                let ok = self.inner.bridge.fail(id, s("reason").unwrap_or("no reason given"));
                Ok(text(if ok { "Told the person on the map." } else { "No such request is waiting." }))
            }
            SCREENS => {
                let root = self.root_of(s("repo"))?;
                let port = self.inner.lock().servers.get(&root).map(|s| s.port);
                let scan = ScreenScan { skip_ports: port.into_iter().collect(), ports: None };
                Ok(text(list_screens(&root, &scan).await))
            }
            CAPTURE => {
                let root = self.root_of(s("repo"))?;
                let target = match (s("url").filter(|u| !u.is_empty()), s("file").filter(|f| !f.is_empty())) {
                    (Some(u), _) => CaptureTarget::Url(u.into()),
                    (None, Some(f)) => CaptureTarget::File(f.into()),
                    _ => {
                        return Ok(error("Pass `url` (a page on this machine) or `file` (an .html file in the repo)."))
                    }
                };
                let stop = keep_alive(ctx, self.inner.o.heartbeat);
                let got = capture(&root, &target, self.inner.o.shoot.as_ref()).await;
                stop.cancel();
                let r = match got {
                    Ok(r) => r,
                    Err(reason) => return Ok(error(reason)),
                };
                let c = &r.capture;
                let mut labels: Vec<String> = Vec::new();
                for e in &c.elements {
                    let l = format!("{} \"{}\"", e.tag, e.text);
                    if !labels.contains(&l) {
                        labels.push(l);
                    }
                }
                labels.truncate(40);
                // A host that keeps only so much of a tool result needs the image more than the page's text.
                let page_text: String =
                    if host.max_output.is_some() { r.text.chars().take(4000).collect() } else { r.text.clone() };
                let body = [
                    format!("Captured as {}{}. Look at the image below before using it.", c.id, if c.title.is_empty() { String::new() } else { format!(" (\"{}\")", c.title) }),
                    format!("To show it on a box: \"screen\": {{ \"kind\": \"capture\", \"capture\": \"{}\", \"highlight\": \"<label of the thing this box is about>\" }}. A highlight must be one of the labels below and also appear in the code the box cites; leave it out if nothing fits.", c.id),
                    String::new(),
                    if labels.is_empty() {
                        "Nothing labelled is visible on it.".to_string()
                    } else {
                        format!("Labelled things visible:\n{}", labels.iter().map(|l| format!("  - {l}")).collect::<Vec<_>>().join("\n"))
                    },
                    String::new(),
                    format!("Text on the page: {}", if page_text.is_empty() { "(none)".into() } else { page_text }),
                ]
                .join("\n");
                let png = base64::engine::general_purpose::STANDARD.encode(&r.png);
                Ok(CallToolResult::success(vec![ContentBlock::text(body), ContentBlock::image(png, "image/png")]))
            }
            OPEN => {
                let root = self.root_of(s("repo"))?;
                let server = self.server_for(&root).await?;
                let map_id = s("map_id").filter(|m| !m.is_empty());
                let url = server.page_url(map_id);
                if !host.inline_browser {
                    self.inner.open(&url);
                }
                let mut maps = Store::new(&root).list();
                if chat {
                    maps.truncate(200);
                }
                let mut lines: Vec<String> = if host.inline_browser {
                    vec![show_inline(&url), self.port_note(&server)]
                } else {
                    vec![format!("Opened {url}")]
                };
                lines.push(if chat {
                    format!("When the person asks about the map here in the chat, call {ASK}.")
                } else {
                    format!("Call {LISTEN} so questions asked on the map reach you.")
                });
                lines.retain(|l| !l.is_empty());
                lines.push(String::new());
                lines.push(if maps.is_empty() {
                    "No maps saved in this repo yet.".into()
                } else {
                    "Saved maps:".into()
                });
                lines.extend(maps.iter().map(|m| format!("{}  {}", m.id, m.question)));
                Ok(text(lines.join("\n")))
            }
            other => Ok(error(format!("Unknown tool: {other}"))),
        }
    }
}

/// How a host with a browser beside the chat shows the person a map.
fn show_inline(url: &str) -> String {
    format!("Give the person this link: {url}\nIf you have a built-in browser, open the link there, so the map sits beside this chat; don't open it in another browser.")
}

fn part_text(id: &str, i: usize, all: &[String]) -> String {
    let more = if i + 1 < all.len() {
        format!(" Call {MORE} with request_id \"{id}\" for part {} before you start; the output schema is at the end of the last part.", i + 2)
    } else {
        " This is the last part.".to_string()
    };
    format!("[nonio request {id}, part {} of {}.{more}]\n\n{}", i + 1, all.len(), all[i])
}

fn prepend(mut r: CallToolResult, t: String) -> CallToolResult {
    r.content.insert(0, ContentBlock::text(t));
    r
}

/// Progress notifications while a call waits, when it came with a progress token. Claude Code's docs say they count
/// as activity, but a backgrounded call was still dropped after 30 quiet minutes, so `nonio_listen` also returns on
/// its own. Cancel the returned token to stop them.
fn keep_alive(ctx: &RequestContext<RoleServer>, every: Duration) -> CancellationToken {
    let stop = CancellationToken::new();
    let Some(token) = ctx.meta.get_progress_token() else { return stop };
    let (peer, s) = (ctx.peer.clone(), stop.clone());
    tokio::spawn(async move {
        let mut n = 0.0;
        loop {
            tokio::select! {
                _ = s.cancelled() => return,
                _ = tokio::time::sleep(every) => {}
            }
            n += 1.0;
            let p = ProgressNotificationParam::new(ProgressToken::clone(&token), n).with_message("Waiting for the map");
            let _ = peer.notify_progress(p).await;
        }
    });
    stop
}

fn on_event(inner: &Arc<Inner>, root: &Path, e: &ServerEvent) {
    let host = inner.host();
    match e {
        ServerEvent::Map { map_id, report, .. } => {
            let hidden = report
                .screens
                .iter()
                .filter(|(_, c)| c.status != nonio_core::screens::ScreenStatus::Confirmed)
                .map(|(id, c)| format!("{id} ({})", c.reasons.join("; ")))
                .collect();
            inner.lock().last_report.insert(
                map_id.clone(),
                Report { confirmed: report.summary.confirmed, claims: report.summary.claims, hidden },
            );
            return;
        }
        ServerEvent::Answer { map_id, answer } => {
            inner.lock().last_answer.insert(map_id.clone(), answer.clone());
            return;
        }
        ServerEvent::Job { status: JobStatus::Started, .. } => return,
        ServerEvent::Job { .. } => {}
        _ => return,
    }
    let ServerEvent::Job { map_id, job, status, question, error } = e else { return };
    let chat = !host.wake;
    let listen =
        if chat { String::new() } else { format!("Then call {LISTEN} so questions they ask on the map reach you.") };
    let message = if *status == JobStatus::Failed {
        [format!("nonio couldn't finish \"{question}\": {}", error.as_deref().unwrap_or("unknown error")), listen]
            .into_iter()
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    } else if *job == JobKind::Make {
        let (server, r) = {
            let st = inner.lock();
            (
                st.servers.get(root).cloned(),
                st.last_report.get(map_id).map(|r| (r.confirmed, r.claims, r.hidden.clone())),
            )
        };
        let mut lines = vec![format!(
            "The map for \"{question}\" is ready{}.",
            r.as_ref().map(|(c, n, _)| format!(": {c} of {n} claims confirmed against the code")).unwrap_or_default()
        )];
        if let Some((_, _, hidden)) = &r {
            if !hidden.is_empty() {
                lines.push(format!("Screens left out because they didn't check out: {}.", hidden.join(", ")));
            }
        }
        if let Some(s) = server {
            let url = s.page_url(Some(map_id));
            lines.push(if host.inline_browser {
                show_inline(&url)
            } else {
                format!("It's open in the browser: {url}")
            });
        }
        lines.push(if chat {
            format!("Tell the person in a sentence; the map does the explaining. They can take the walkthrough, or click a box or arrow on the map and ask about it here in the chat (then call {ASK}).")
        } else {
            "Tell the person in a sentence; the map does the explaining. They can take the walkthrough, or select any box and ask about it there.".to_string()
        });
        if !listen.is_empty() {
            lines.push(listen);
        }
        lines.join("\n")
    } else if chat {
        let a = inner.lock().last_answer.get(map_id).cloned();
        match a {
            Some(a) if a.offer.as_ref().is_some_and(|o| o.status == OfferStatus::Pending) => {
                let offer = a.offer.expect("checked above");
                let existing = offer
                    .existing
                    .as_ref()
                    .map(|x| format!("the map that already covers it (\"{}\"), or ", x.question))
                    .unwrap_or_default();
                format!("That question is about a different part of the app than this map shows. The map offers {existing}a new map for \"{}\". Ask the person in a sentence whether they want a new map; if they do, call {MAP} with that question.", offer.question)
            }
            // The reply belongs in the chat where it was asked: hand back the checked answer to post there.
            Some(a) => [
                "nonio checked the answer and lit up what it is about on the map. Post the reply between the lines to the person as your answer, unchanged: keep the code blocks and line numbers exactly as they are, and add nothing about the code that isn't in it.".to_string(),
                "---".into(),
                chat_reply(root, &a),
                "---".into(),
            ]
            .join("\n"),
            None => format!("The answer to \"{question}\" is shown on the map. In the chat, say at most a sentence or two."),
        }
    } else {
        format!("The answer to \"{question}\" is shown on the map. Call {LISTEN} again for their next question.")
    };
    for (_, tx) in inner.lock().finished.iter() {
        let _ = tx.send(message.clone());
    }
}

impl ServerHandler for Channel {
    fn get_info(&self) -> InitializeResult {
        let mut experimental = std::collections::BTreeMap::new();
        experimental.insert("claude/channel".to_string(), Map::new());
        let caps = ServerCapabilities::builder().enable_experimental_with(experimental).enable_tools().build();
        InitializeResult::new(caps)
            .with_server_info(Implementation::new("nonio", VERSION))
            .with_instructions(instructions(self.inner.host()))
    }

    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, ErrorData> {
        {
            let mut st = self.inner.lock();
            st.client = Some(request.client_info.name.clone());
            st.peer = Some(context.peer.clone());
        }
        context.peer.set_peer_info(request.clone());
        self.negotiate_initialize(&request)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(tools_for(self.inner.host())))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let args = request.arguments.clone().unwrap_or_default();
        let r = match self.call(&request.name, &args, &context).await {
            Ok(r) => r,
            Err(message) => error(message),
        };
        Ok(r.into())
    }
}
