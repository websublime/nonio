//! Making a map: a head start from the code facts, the host's model explores and drafts, the checker re-reads the
//! code, and the model gets up to `repairs` chances to fix what didn't check out. A repair returns only the changes
//! (`patch_map`), not the whole map again, since writing the map is most of the time a map takes. The best attempt
//! wins; nothing unconfirmed is hidden.
//!
//! Ported from `packages/server/src/maker.ts` (MIT, Copyright (c) 2026 johnz4021); see NOTICE.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nonio_core::base::resolve_commit;
use nonio_core::edit::{patch_map, MapPatch};
use nonio_core::repo::{list_repo_files, DEFAULT_IGNORE};
use nonio_core::{check, parse_map, settled_proposals, CheckOptions, CheckReport, CodeMap, ParseResult};
use nonio_scan::Scanner;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::bridge::{Agent, AgentError, AgentRequest, Progress, ProgressFn};
use crate::hints::{head_start, symbols_by_file};
use crate::prompts::{evidence_repair_prompt, maker_prompt, patch_repair_prompt, shape_repair_prompt};
use crate::schemas::{Schema, MAP, MAP_PATCH};

pub struct MakeOptions {
    pub root: PathBuf,
    pub question: String,
    pub agent: Arc<dyn Agent>,
    /// Repair rounds after the first draft (default 2).
    pub repairs: usize,
    /// Who does the drafting, as the progress names them ("Claude Code", "Cursor").
    pub agent_name: String,
    pub on_progress: Option<ProgressFn>,
    pub cancel: Option<CancellationToken>,
}

impl MakeOptions {
    pub fn new(root: impl Into<PathBuf>, question: impl Into<String>, agent: Arc<dyn Agent>) -> Self {
        MakeOptions {
            root: root.into(),
            question: question.into(),
            agent,
            repairs: 2,
            agent_name: "your agent".into(),
            on_progress: None,
            cancel: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MakeResult {
    pub map: CodeMap,
    pub report: CheckReport,
    pub session_id: Option<String>,
    pub cost: f64,
    pub rounds: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MakeError {
    Agent(AgentError),
    /// No reply could be read as a map, with the last problems found.
    Unreadable(String),
    Io(String),
}

impl fmt::Display for MakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MakeError::Agent(e) => e.fmt(f),
            MakeError::Unreadable(m) | MakeError::Io(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for MakeError {}

/// The model's map with the person's question and the format's version, whatever the model wrote there.
fn with_question(output: &Value, question: &str) -> Value {
    let mut v = output.clone();
    if let Value::Object(o) = &mut v {
        o.insert("version".into(), 1.into());
        o.insert("question".into(), question.into());
    }
    v
}

/// The model's changes applied to the map they're for, or what stops them from applying.
fn apply_patch(map: &CodeMap, output: &Value) -> ParseResult {
    let problems = MAP_PATCH.problems(output);
    if !problems.is_empty() {
        return Err(problems.into_iter().map(|p| p.replacen("(output)", "(changes)", 1)).collect());
    }
    let list = |k: &str| output.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
    let patch = MapPatch {
        areas: list("areas"),
        nodes: list("nodes"),
        edges: list("edges"),
        absences: list("absences"),
        remove: list("remove").into_iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
    };
    patch_map(map, &patch)
}

/// A map of a change names what it is compared with (`since`: "HEAD", a branch). That is pinned to the commit it
/// means right now, so the map shows the same change after the work is committed. A name that is no commit is sent
/// back.
fn pinned(parsed: ParseResult, root: &Path) -> ParseResult {
    let mut map = parsed?;
    let Some(since) = map.since.clone() else { return Ok(map) };
    match resolve_commit(root, &since) {
        Some(commit) => {
            map.since = Some(commit);
            Ok(map)
        }
        None => Err(vec![format!("since: \"{since}\" isn't a commit in this repo. Use \"HEAD\" for work that isn't committed yet, \"HEAD~1\" for the last commit, or leave `since` out if the map isn't about a change.")]),
    }
}

/// The checker's report, with facts from every backend.
pub fn check_map(map: &CodeMap, root: &Path) -> Result<CheckReport, MakeError> {
    let scanner = Scanner::new(root, list_repo_files(root, DEFAULT_IGNORE));
    check(map, root, &scanner, &CheckOptions::default()).map_err(|e| MakeError::Io(e.to_string()))
}

/// What the check sends back: claims it couldn't confirm, and proposals the code shows were misread.
fn off(r: &CheckReport) -> usize {
    r.summary.unconfirmed + settled_proposals(r).len()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Reply {
    Map,
    Patch,
}

pub async fn make_map(o: MakeOptions) -> Result<MakeResult, MakeError> {
    let status = |text: String| {
        if let Some(f) = &o.on_progress {
            f(Progress::status(text));
        }
    };
    let who = o.agent_name.as_str();
    status("Scanning the code for ways in, routes and tables".into());
    let (hints, symbols): (String, HashMap<String, Vec<String>>) = {
        let h = head_start(&o.root);
        let symbols = symbols_by_file(h.scan.as_ref());
        (h.text, symbols)
    };
    let symbols_in = move |file: &str| symbols.get(file).cloned().unwrap_or_default();
    status(format!("{who} is exploring the code"));
    let run = |prompt: String, resume: Option<String>, schema: &'static Schema| {
        let mut req = AgentRequest::new(prompt, schema.json.clone(), o.root.clone());
        req.label = Some(format!("Making the map for \"{}\"", o.question));
        req.validate = Some(schema.validator());
        req.resume = resume;
        req.on_progress = o.on_progress.clone();
        req.cancel = o.cancel.clone();
        o.agent.ask(req)
    };

    let mut res = run(maker_prompt(&o.question, &hints), None, &MAP).await.map_err(MakeError::Agent)?;
    let mut cost = res.cost.unwrap_or(0.0);
    let mut rounds = 1;
    let mut best: Option<(CodeMap, CheckReport)> = None;
    let mut last_problems: Vec<String> = Vec::new();
    // What the last reply is: a whole map, or changes to `current`, the last map that could be read.
    let mut reply = Reply::Map;
    let mut current: Option<CodeMap> = None;

    let mut attempt = 0;
    loop {
        let parsed = match (&current, reply) {
            (Some(map), Reply::Patch) => apply_patch(map, &res.output),
            _ => parse_map(&with_question(&res.output, &o.question)),
        };
        let parsed = pinned(parsed, &o.root);
        let parsed_ok = parsed.is_ok();
        let repair: Option<(String, Reply)> = match parsed {
            Err(problems) => {
                let r = if current.is_some() {
                    (patch_repair_prompt(&problems), Reply::Patch)
                } else {
                    (shape_repair_prompt(&problems), Reply::Map)
                };
                last_problems = problems;
                Some(r)
            }
            Ok(map) => {
                status("Checking every claim against the code".into());
                let report = check_map(&map, &o.root)?;
                status(format!("{} of {} claims confirmed", report.summary.confirmed, report.summary.claims));
                let repair =
                    (off(&report) > 0).then(|| (evidence_repair_prompt(&report, Some(&symbols_in)), Reply::Patch));
                if best.as_ref().is_none_or(|(_, b)| off(&report) < off(b)) {
                    best = Some((map.clone(), report));
                }
                current = Some(map);
                repair
            }
        };
        let Some((prompt, next)) = repair else { break };
        if attempt >= o.repairs {
            break;
        }
        status(if parsed_ok {
            format!("Asking {who} to fix the claims that didn't check out")
        } else if current.is_some() {
            format!("Asking {who} to fix its changes")
        } else {
            format!("Asking {who} to fix the map's structure")
        });
        res = run(prompt, res.session_id.clone(), if next == Reply::Map { &MAP } else { &MAP_PATCH })
            .await
            .map_err(MakeError::Agent)?;
        reply = next;
        cost += res.cost.unwrap_or(0.0);
        rounds += 1;
        attempt += 1;
    }
    let Some((map, report)) = best else {
        let first: Vec<String> = last_problems.into_iter().take(5).collect();
        return Err(MakeError::Unreadable(format!("{who}'s map couldn't be read: {}", first.join("; "))));
    };
    Ok(MakeResult { map, report, session_id: res.session_id, cost, rounds })
}
