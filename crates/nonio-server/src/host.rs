//! Which coding agent does nonio's model work, and what that changes (03-arquitetura, "Perfil de capacidades do
//! host"). The original (`packages/server/src/host.ts`) tells only Codex from "everything else", and treats everything
//! else as Claude Code; nonio keeps a profile of capabilities per host and puts an unknown client on the base protocol
//! (D07): short calls, carrying on with `nonio_next`, questions asked in the chat, long requests in parts.
//!
//! What each value rests on:
//! - Claude Code: background calls that wake the session, 90 s per call, `listen` up to 25 min, no cut-off on output
//!   (the original's code). It names itself `claude-code` in `initialize` (read in the Claude Code 2.1.332 binary, not
//!   yet seen on the wire).
//! - Codex: no wake, 60 s tool timeout so nothing waits past 40 s, output cut near 10k tokens so requests come in parts
//!   of 28 000 characters, the start folder isn't always the project's (the original's code). It names itself
//!   `codex-mcp-client`.
//! - Cursor: not measured yet (Q06, phase 0). Until it is, it gets the base protocol's values; the name it sends is not
//!   known, so it is recognised only when forced with `NONIO_HOST=cursor`.

use std::time::Duration;

/// The variable that forces a profile, whatever the client says it is.
pub const HOST_ENV: &str = "NONIO_HOST";

/// What Codex's MCP client calls itself in `initialize` (openai/codex, `codex-rs`).
pub const CODEX_CLIENT: &str = "codex-mcp-client";
/// What Claude Code's MCP client calls itself in `initialize`.
pub const CLAUDE_CODE_CLIENT: &str = "claude-code";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostId {
    ClaudeCode,
    Codex,
    Cursor,
    /// A client nonio doesn't know: the base protocol (D07).
    Base,
}

impl HostId {
    /// The name `NONIO_HOST` takes for this profile.
    pub fn key(self) -> &'static str {
        match self {
            HostId::ClaudeCode => "claude-code",
            HostId::Codex => "codex",
            HostId::Cursor => "cursor",
            HostId::Base => "base",
        }
    }

    pub fn from_key(key: &str) -> Option<HostId> {
        [HostId::ClaudeCode, HostId::Codex, HostId::Cursor, HostId::Base].into_iter().find(|h| h.key() == key.trim())
    }
}

/// Where a question about the map is asked: in the page's own box (only where the session can be woken), or in the
/// agent's chat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asks {
    Page,
    Chat,
}

/// What the map page is told: who is working, and where questions are asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageAgent {
    pub name: &'static str,
    pub asks: Asks,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostProfile {
    pub id: HostId,
    /// The agent's name, as the person knows it.
    pub name: &'static str,
    pub page: PageAgent,
    /// A call can run in the background and wake the session when it returns: turns on `nonio_listen` and the page's
    /// question box (D10).
    pub wake: bool,
    /// How long a tool call waits for what comes next before handing the turn back.
    pub max_wait: Duration,
    /// How long one `nonio_listen` call waits before returning so another can start (only with `wake`).
    pub listen: Duration,
    /// The longest text one tool result carries; a longer request comes in parts, read with `nonio_more`.
    pub max_output: Option<usize>,
    /// A variable the host sets to the project's root. Without one, the model passes `repo` on every call.
    pub repo_from_env: Option<&'static str>,
    /// The host shows MCP Apps UI (D11: not used before v1).
    pub embedded_view: bool,
    /// The host has a browser beside the chat to open the map in, instead of the system browser.
    pub inline_browser: bool,
    /// Progress on the map once the agent has picked a request up.
    pub claimed: &'static str,
    /// Progress on the map while a request waits for the agent.
    pub waiting: &'static str,
    /// Said on the map when a request has waited `nudge` with nobody picking it up.
    pub not_picked_up: &'static str,
    /// The error once nobody picked a request up at all.
    pub never_picked_up: &'static str,
    /// How long a request waits unclaimed before the map says `not_picked_up`.
    pub nudge: Duration,
}

/// Base protocol: how long a call waits, and how much text a tool result carries (Codex's limits, the tightest known).
const BASE_WAIT: Duration = Duration::from_secs(40);
const BASE_OUTPUT: usize = 28_000;

pub const CLAUDE_CODE: HostProfile = HostProfile {
    id: HostId::ClaudeCode,
    name: "Claude Code",
    page: PageAgent { name: "Claude", asks: Asks::Page },
    wake: true,
    max_wait: Duration::from_secs(90),
    // Claude Code drops a call that has been quiet for 30 minutes, even with progress notifications.
    listen: Duration::from_secs(25 * 60),
    max_output: None,
    repo_from_env: Some("CLAUDE_PROJECT_DIR"),
    embedded_view: false,
    inline_browser: false,
    claimed: "Claude Code is working on this in your terminal",
    waiting: "Waiting for Claude Code to pick this up",
    not_picked_up: "Claude Code isn't listening to the map right now. In your Claude Code session, say \"listen to nonio\", and it will pick this up.",
    never_picked_up: "Claude Code never picked this up, so nothing was answered. Its session may have closed. In Claude Code, say \"listen to nonio\", then ask again.",
    nudge: Duration::from_secs(30),
};

pub const CODEX: HostProfile = HostProfile {
    id: HostId::Codex,
    name: "Codex",
    page: PageAgent { name: "Codex", asks: Asks::Chat },
    wake: false,
    // Codex's documented default tool timeout is 60 s; progress doesn't extend it.
    max_wait: BASE_WAIT,
    listen: Duration::ZERO,
    max_output: Some(BASE_OUTPUT),
    repo_from_env: None,
    embedded_view: false,
    inline_browser: true,
    claimed: "Codex is working on this in the Codex chat",
    waiting: "Waiting for Codex to pick this up",
    // Codex can't be woken from the page, so this is said as soon as it's clear nobody has it.
    not_picked_up: "Codex can't see work started on this page. In the Codex chat, say \"carry on with nonio\", and it will pick this up.",
    never_picked_up: "Codex never picked this up, so nothing was answered. Ask in the Codex chat instead.",
    nudge: Duration::from_millis(1_500),
};

/// Cursor, on the base protocol's values until phase 0 measures it (Q06).
pub const CURSOR: HostProfile = HostProfile {
    id: HostId::Cursor,
    name: "Cursor",
    page: PageAgent { name: "Cursor", asks: Asks::Chat },
    wake: false,
    max_wait: BASE_WAIT,
    listen: Duration::ZERO,
    max_output: Some(BASE_OUTPUT),
    repo_from_env: None,
    embedded_view: false,
    inline_browser: false,
    claimed: "Cursor is working on this in the Cursor chat",
    waiting: "Waiting for Cursor to pick this up",
    not_picked_up: "Cursor can't see work started on this page. In the Cursor chat, say \"carry on with nonio\", and it will pick this up.",
    never_picked_up: "Cursor never picked this up, so nothing was answered. Ask in the Cursor chat instead.",
    nudge: Duration::from_millis(1_500),
};

/// Any other client: the base protocol (D07).
pub const BASE: HostProfile = HostProfile {
    id: HostId::Base,
    name: "your agent",
    page: PageAgent { name: "your agent", asks: Asks::Chat },
    wake: false,
    max_wait: BASE_WAIT,
    listen: Duration::ZERO,
    max_output: Some(BASE_OUTPUT),
    repo_from_env: None,
    embedded_view: false,
    inline_browser: false,
    claimed: "Your agent is working on this in its chat",
    waiting: "Waiting for your agent to pick this up",
    not_picked_up: "Your agent can't see work started on this page. In its chat, say \"carry on with nonio\", and it will pick this up.",
    never_picked_up: "Nobody picked this up, so nothing was answered. Ask in your agent's chat instead.",
    nudge: Duration::from_millis(1_500),
};

pub fn profile(id: HostId) -> &'static HostProfile {
    match id {
        HostId::ClaudeCode => &CLAUDE_CODE,
        HostId::Codex => &CODEX,
        HostId::Cursor => &CURSOR,
        HostId::Base => &BASE,
    }
}

/// The agent behind an MCP client: `forced` (the value of `NONIO_HOST`) when it names a profile, else the client's
/// name from `initialize`, else the base protocol.
pub fn host_of(client_name: Option<&str>, forced: Option<&str>) -> &'static HostProfile {
    if let Some(id) = forced.and_then(HostId::from_key) {
        return profile(id);
    }
    match client_name {
        Some(CLAUDE_CODE_CLIENT) => &CLAUDE_CODE,
        Some(CODEX_CLIENT) => &CODEX,
        _ => &BASE,
    }
}

/// `host_of` with `NONIO_HOST` read from the environment.
pub fn host_from_env(client_name: Option<&str>) -> &'static HostProfile {
    host_of(client_name, std::env::var(HOST_ENV).ok().as_deref())
}
