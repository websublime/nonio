//! Answering a question about a map, in up to two steps. First the model answers from the map and the code around
//! its citations, with no tools: quick, and enough for many questions. If that isn't enough, it explores the repo.
//! Anything it adds must pass the checker or it isn't drawn; a requested change comes back as a validated fix
//! proposal, never an edit.
//!
//! Ported from `packages/server/src/explainer.ts` (MIT, Copyright (c) 2026 johnz4021); see NOTICE.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nonio_core::base::BaseReader;
use nonio_core::check::Status;
use nonio_core::code::{check_notes, CodeNote};
use nonio_core::edit::{add_to_map, map_ids, remove_from_map, MapAdditions};
use nonio_core::repo::{Reader, RepoReader};
use nonio_core::screens::ScreenStatus;
use nonio_core::{parse_map, settled_proposals, CheckReport, CodeMap};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::bridge::{Agent, AgentError, AgentRequest, Progress, ProgressFn};
use crate::fixes::{propose_fix, FixProposal, RawFix};
use crate::ids::random_id;
use crate::maker::{check_map, MakeError};
use crate::prompts::{explainer_prompt, quick_answer_prompt, EarlierTurn, OtherMap, SelectionLine};
use crate::schemas::{ANSWER, QUICK_ANSWER};

/// What the person selected on the map.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub absences: Vec<String>,
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.edges.is_empty() && self.absences.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RefState {
    Confirmed,
    Unconfirmed,
    Proposed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefStatus {
    pub id: String,
    pub status: RefState,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Moves {
    pub focus: Option<String>,
    #[serde(default)]
    pub highlight: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OfferStatus {
    Pending,
    Mapped,
    Answered,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedMapRef {
    pub id: String,
    pub question: String,
}

/// The offer of a new map, when a question is about a different part of the app than this map covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Offer {
    /// The question as a new map would ask it.
    pub question: String,
    /// A saved map that already covers it.
    pub existing: Option<SavedMapRef>,
    pub status: OfferStatus,
    /// The map made for it, once they said yes.
    pub map_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// Answered from the map alone.
    Map,
    /// The model looked in the code.
    Code,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Answer {
    pub id: String,
    pub question: String,
    pub selection: Selection,
    /// The answer, as short Markdown.
    pub concept: String,
    pub refs: Vec<RefStatus>,
    pub moves: Moves,
    /// Ids drawn on the map because of this answer (all confirmed, or proposals the checker placed).
    pub added: Vec<String>,
    /// Boxes and arrows already on the map that this answer marked as to change or remove.
    #[serde(default)]
    pub proposed: Vec<String>,
    /// Boxes whose picture of the screen this answer set (all confirmed).
    #[serde(default)]
    pub screens: Vec<String>,
    /// Lines of code the answer points at; only ones the checker found are kept.
    #[serde(default)]
    pub notes: Vec<CodeNote>,
    /// What was found but not drawn, and why; problems with a proposed fix.
    pub problems: Vec<String>,
    pub fix: Option<FixProposal>,
    pub source: Source,
    #[serde(default)]
    pub offer: Option<Offer>,
    pub cost: f64,
}

pub struct AnswerOptions {
    pub root: PathBuf,
    pub map: CodeMap,
    pub selection: Selection,
    pub question: String,
    pub agent: Arc<dyn Agent>,
    pub session: Option<String>,
    /// The conversation so far on this map, oldest first.
    pub earlier: Vec<EarlierTurn>,
    /// Try answering from the map alone first (default true).
    pub quick: bool,
    /// Other maps saved in this repo, so the first try can point at one that already covers the question.
    pub other_maps: Vec<OtherMap>,
    /// Answer in words from the code, drawing nothing: for a question about a different part of the app.
    pub words_only: bool,
    pub on_progress: Option<ProgressFn>,
    pub cancel: Option<CancellationToken>,
}

impl AnswerOptions {
    pub fn new(root: impl Into<PathBuf>, map: CodeMap, question: impl Into<String>, agent: Arc<dyn Agent>) -> Self {
        AnswerOptions {
            root: root.into(),
            map,
            selection: Selection::default(),
            question: question.into(),
            agent,
            session: None,
            earlier: Vec::new(),
            quick: true,
            other_maps: Vec::new(),
            words_only: false,
            on_progress: None,
            cancel: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Answered {
    pub answer: Answer,
    pub map: CodeMap,
    pub report: CheckReport,
    pub session_id: Option<String>,
}

const EXCERPT_RADIUS: usize = 4;
const MAX_EXCERPTS: usize = 40;
const MAX_EXCERPT_CHARS: usize = 30_000;

/// The code around each line the map cites, read through the secret-refusing reader: what the selection cites first,
/// then the rest of the map, until the budget runs out. Plain code, no model.
pub fn cited_excerpts(root: &Path, map: &CodeMap, selection: &Selection) -> String {
    let Ok(repo) = RepoReader::new(root) else { return String::new() };
    let picked: HashSet<&str> = selection.nodes.iter().chain(&selection.edges).map(String::as_str).collect();
    // Removed code is cited as it was at the commit the map is compared with, so that is where its lines are read.
    let any_removed =
        map.nodes.iter().any(|n| n.removed == Some(true)) || map.edges.iter().any(|e| e.removed == Some(true));
    let base = map.since.as_deref().filter(|_| any_removed).and_then(|s| BaseReader::new(root, s));
    let mut items: Vec<(&str, &[nonio_core::format::Evidence], bool)> = map
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n.evidence.as_slice(), n.removed == Some(true)))
        .chain(map.edges.iter().map(|e| (e.id.as_str(), e.evidence.as_slice(), e.removed == Some(true))))
        .collect();
    // Stable: what's selected first, the rest in map order.
    items.sort_by_key(|(id, _, _)| !picked.contains(id));
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    let mut size = 0;
    for (id, evidence, was) in items {
        for e in evidence {
            let key = format!("{}{}:{}", if was { "was:" } else { "" }, e.file, e.line);
            if seen.contains(&key) || out.len() >= MAX_EXCERPTS {
                continue;
            }
            seen.insert(key);
            let reader: &dyn Reader = match (was, &base) {
                (false, _) => &repo,
                (true, Some(b)) => b,
                (true, None) => continue,
            };
            let Ok(text) = reader.read(&e.file) else { continue };
            let lines: Vec<&str> = text.split('\n').collect();
            let from = e.line.saturating_sub(EXCERPT_RADIUS).max(1);
            let to = (e.line + EXCERPT_RADIUS).min(lines.len());
            let body: Vec<String> =
                (from..=to).map(|n| format!("{n:>5} | {}", lines.get(n - 1).unwrap_or(&""))).collect();
            let block = format!(
                "{}:{from}-{to} ({}cited by {id})\n{}",
                e.file,
                if was { "removed since; as it was, " } else { "" },
                body.join("\n")
            );
            let len = block.encode_utf16().count();
            if size + len > MAX_EXCERPT_CHARS {
                return out.join("\n\n");
            }
            out.push(block);
            size += len;
        }
    }
    out.join("\n\n")
}

fn selection_lines(map: &CodeMap, s: &Selection) -> Vec<SelectionLine> {
    let name = |id: &str| map.nodes.iter().find(|n| n.id == id).map_or(id.to_string(), |n| n.name.clone());
    let mut out = Vec::new();
    for id in &s.nodes {
        if map.nodes.iter().any(|n| &n.id == id) {
            out.push(SelectionLine { id: id.clone(), label: format!("box \"{}\"", name(id)) });
        }
    }
    for id in &s.edges {
        if let Some(e) = map.edges.iter().find(|e| &e.id == id) {
            let label = format!("arrow \"{}\" from \"{}\" to \"{}\"", e.label, name(&e.from), name(&e.to));
            out.push(SelectionLine { id: id.clone(), label });
        }
    }
    for id in &s.absences {
        if let Some(a) = map.absences.iter().find(|a| &a.id == id) {
            out.push(SelectionLine { id: id.clone(), label: format!("missing connection: {}", a.claim) });
        }
    }
    out
}

/// The notes an answer may show: the ones whose line the checker found. The rest are said, not shown.
fn notes_of(root: &Path, raw: &[CodeNote]) -> (Vec<CodeNote>, Vec<String>) {
    match check_notes(root, raw) {
        Ok((notes, dropped)) => (
            notes,
            dropped
                .into_iter()
                .map(|d| format!("A note on the code wasn't shown, its line isn't there: {d}"))
                .collect(),
        ),
        Err(e) => (Vec::new(), vec![format!("Notes on the code weren't checked: {e}")]),
    }
}

fn status_of(report: &CheckReport, id: &str) -> RefState {
    let s = report
        .nodes
        .get(id)
        .or_else(|| report.edges.get(id))
        .map(|c| c.status)
        .or_else(|| report.absences.get(id).map(|c| c.status));
    match s {
        Some(Status::Confirmed) => RefState::Confirmed,
        Some(Status::Unconfirmed) => RefState::Unconfirmed,
        Some(Status::Proposed) => RefState::Proposed,
        None => RefState::Unknown,
    }
}

fn reasons_of(report: &CheckReport, id: &str) -> Option<Vec<String>> {
    report
        .nodes
        .get(id)
        .or_else(|| report.edges.get(id))
        .map(|c| c.reasons.clone())
        .or_else(|| report.absences.get(id).map(|c| c.reasons.clone()))
}

fn unique(ids: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    ids.iter().filter(|id| seen.insert(id.as_str())).cloned().collect()
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMoves {
    focus: Option<String>,
    #[serde(default)]
    highlight: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Route {
    Answer,
    Code,
    Elsewhere,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawQuickAnswer {
    route: Route,
    #[serde(default)]
    missing: String,
    #[serde(default)]
    new_question: String,
    #[serde(default)]
    existing_map: String,
    #[serde(default)]
    concept: String,
    #[serde(default)]
    refs: Vec<String>,
    #[serde(default)]
    moves: Option<RawMoves>,
    #[serde(default)]
    notes: Vec<CodeNote>,
}

#[derive(Debug, Default, Deserialize)]
struct RawAdd {
    #[serde(default)]
    areas: Vec<Value>,
    #[serde(default)]
    nodes: Vec<Value>,
    #[serde(default)]
    edges: Vec<Value>,
    #[serde(default)]
    absences: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct RawScreen {
    node: String,
    screen: Value,
}

#[derive(Debug, Deserialize)]
struct RawPropose {
    id: String,
    proposed: Value,
}

#[derive(Debug, Deserialize)]
struct RawAnswer {
    concept: String,
    #[serde(default)]
    refs: Vec<String>,
    #[serde(default)]
    add: Option<RawAdd>,
    #[serde(default)]
    moves: Option<RawMoves>,
    #[serde(default)]
    notes: Vec<CodeNote>,
    #[serde(default)]
    screens: Vec<RawScreen>,
    #[serde(default)]
    propose: Vec<RawPropose>,
    #[serde(default)]
    fix: Option<RawFix>,
}

/// Output read against its schema first, then into the shape the workflow uses.
fn read<T: for<'de> Deserialize<'de>>(schema: &crate::schemas::Schema, output: &Value) -> Result<T, Vec<String>> {
    let problems = schema.problems(output);
    if !problems.is_empty() {
        return Err(problems);
    }
    serde_json::from_value(output.clone()).map_err(|e| vec![e.to_string()])
}

fn moves_in(raw: Option<RawMoves>, live: &HashSet<String>) -> Moves {
    let raw = raw.unwrap_or_default();
    Moves {
        focus: raw.focus.filter(|f| live.contains(f)),
        highlight: raw.highlight.into_iter().filter(|id| live.contains(id)).collect(),
    }
}

/// `map` with `field` set to `value` on the box or arrow `id`, read back as a map.
fn set_on(map: &CodeMap, lists: &[&str], id: &str, field: &str, value: &Value) -> Result<CodeMap, Vec<String>> {
    let mut v = map.to_value();
    for list in lists {
        if let Some(items) = v.get_mut(*list).and_then(Value::as_array_mut) {
            for x in items.iter_mut().filter(|x| x.get("id").and_then(Value::as_str) == Some(id)) {
                x[field] = value.clone();
            }
        }
    }
    parse_map(&v)
}

pub async fn answer(o: AnswerOptions) -> Result<Answered, MakeError> {
    let before = check_map(&o.map, &o.root)?;
    let selection = selection_lines(&o.map, &o.selection);
    let status = |text: &str| {
        if let Some(f) = &o.on_progress {
            f(Progress::status(text));
        }
    };
    let live0 = map_ids(&o.map);
    let selected = !o.selection.is_empty();
    let mut cost = 0.0;
    let mut missing = String::new();
    let new_id = || format!("ans-{}", random_id());

    if o.quick && !o.words_only {
        status("Answering from the map");
        let excerpts = cited_excerpts(&o.root, &o.map, &o.selection);
        let mut req = AgentRequest::new(
            quick_answer_prompt(&o.map, &before, &selection, &o.question, &excerpts, &o.earlier, &o.other_maps),
            QUICK_ANSWER.json.clone(),
            o.root.clone(),
        );
        req.label = Some(format!("Answering \"{}\" from the map", o.question));
        req.validate = Some(QUICK_ANSWER.validator());
        req.tools = Some(Vec::new());
        req.cancel = o.cancel.clone();
        match o.agent.ask(req).await {
            Ok(res) => {
                cost += res.cost.unwrap_or(0.0);
                match read::<RawQuickAnswer>(&QUICK_ANSWER, &res.output) {
                    // Something selected on this map means the question is about this map, whatever the first try
                    // thought.
                    Ok(q) if matches!(q.route, Route::Elsewhere) && !selected => {
                        let existing = o
                            .other_maps
                            .iter()
                            .find(|m| m.id == q.existing_map)
                            .map(|m| SavedMapRef { id: m.id.clone(), question: m.question.clone() });
                        let question = if q.new_question.trim().is_empty() {
                            o.question.clone()
                        } else {
                            q.new_question.trim().to_string()
                        };
                        let answer = Answer {
                            id: new_id(),
                            question: o.question.clone(),
                            selection: o.selection.clone(),
                            concept: String::new(),
                            refs: Vec::new(),
                            moves: Moves::default(),
                            added: Vec::new(),
                            proposed: Vec::new(),
                            screens: Vec::new(),
                            notes: Vec::new(),
                            problems: Vec::new(),
                            fix: None,
                            source: Source::Map,
                            offer: Some(Offer { question, existing, status: OfferStatus::Pending, map_id: None }),
                            cost,
                        };
                        return Ok(Answered { answer, map: o.map, report: before, session_id: o.session });
                    }
                    Ok(q) if matches!(q.route, Route::Answer) && !q.concept.trim().is_empty() => {
                        let (notes, problems) = notes_of(&o.root, &q.notes);
                        let answer = Answer {
                            id: new_id(),
                            question: o.question.clone(),
                            selection: o.selection.clone(),
                            concept: q.concept,
                            refs: unique(&q.refs)
                                .into_iter()
                                .map(|id| RefStatus { status: status_of(&before, &id), id })
                                .collect(),
                            moves: moves_in(q.moves, &live0),
                            added: Vec::new(),
                            proposed: Vec::new(),
                            screens: Vec::new(),
                            notes,
                            problems,
                            fix: None,
                            source: Source::Map,
                            offer: None,
                            cost,
                        };
                        // The map didn't change, and the explainer session carries on as it was.
                        return Ok(Answered { answer, map: o.map, report: before, session_id: o.session });
                    }
                    Ok(q) => missing = q.missing,
                    Err(_) => {}
                }
            }
            // Stopped, or no one is there to answer: asking again from the code would only wait as long again.
            Err(e @ (AgentError::Stopped | AgentError::Unheard(_))) => return Err(MakeError::Agent(e)),
            // A failed quick try isn't the end: the model with the code still gets the question.
            Err(AgentError::Failed(_)) => {}
        }
        status("Looking in the code");
    }

    let mut req = AgentRequest::new(
        explainer_prompt(&o.map, &before, &selection, &o.question, &missing, o.words_only, &o.earlier),
        ANSWER.json.clone(),
        o.root.clone(),
    );
    req.label = Some(format!("Answering \"{}\" from the code", o.question));
    req.validate = Some(ANSWER.validator());
    req.resume = o.session.clone();
    req.on_progress = o.on_progress.clone();
    req.cancel = o.cancel.clone();
    let res = o.agent.ask(req).await.map_err(MakeError::Agent)?;
    let mut raw: RawAnswer = read(&ANSWER, &res.output).map_err(|p| {
        MakeError::Unreadable(format!(
            "The answer couldn't be read: {}",
            p.into_iter().take(3).collect::<Vec<_>>().join("; ")
        ))
    })?;
    // An answer in words draws nothing and proposes nothing, whatever came back.
    if o.words_only {
        raw.add = None;
        raw.fix = None;
        raw.screens = Vec::new();
        raw.propose = Vec::new();
    }

    let mut problems: Vec<String> = Vec::new();
    let mut map = o.map.clone();
    let mut added: Vec<String> = Vec::new();
    if let Some(add) = raw.add.take() {
        let additions = MapAdditions { areas: add.areas, nodes: add.nodes, edges: add.edges, absences: add.absences };
        match add_to_map(&o.map, &additions) {
            Ok(merged) => {
                let old = map_ids(&o.map);
                added = ordered_ids(&merged).into_iter().filter(|id| !old.contains(id)).collect();
                map = merged;
            }
            Err(p) => problems.extend(p.into_iter().map(|p| format!("Not added to the map: {p}"))),
        }
    }

    // A new box may have come with its own screen; if that one didn't check out, say so (the box is still drawn).
    let mut report = check_map(&map, &o.root)?;
    for id in &added {
        if let Some(c) = report.screens.get(id).filter(|c| c.status != ScreenStatus::Confirmed) {
            problems.push(format!("No screen shown for \"{}\": {}", node_name(&map, id), c.reasons.join("; ")));
        }
    }
    // A proposal the code already settles was misread (adding what exists): like an unconfirmed claim, it isn't drawn.
    let misread: HashSet<String> = settled_proposals(&report).into_iter().map(|p| p.id).collect();
    let failed: Vec<String> = added
        .iter()
        .filter(|id| status_of(&report, id) == RefState::Unconfirmed || misread.contains(*id))
        .cloned()
        .collect();
    if !failed.is_empty() {
        for id in &failed {
            problems.push(if misread.contains(id) {
                format!("Not drawn, {id} was proposed as new but the code already has it")
            } else {
                let why = reasons_of(&report, id).map_or("unknown".to_string(), |r| r.join("; "));
                format!("Not drawn, couldn't confirm {id}: {why}")
            });
        }
        map = remove_from_map(&map, &failed.iter().cloned().collect());
        report = check_map(&map, &o.root)?;
        let live = map_ids(&map);
        added.retain(|id| live.contains(id));
    }

    // Marking what's already on the map as to change or remove. It only sticks on a claim the checker confirms: a
    // proposal about code that isn't there says nothing.
    let mut marked: Vec<String> = Vec::new();
    for p in &raw.propose {
        let name = node_name(&map, &p.id);
        if p.proposed.get("kind").and_then(Value::as_str) == Some("add") || added.contains(&p.id) {
            problems.push(format!(
                "Not marked as a proposal: \"{name}\" (new boxes and arrows carry their own `proposed`)"
            ));
            continue;
        }
        let on_map = report.nodes.contains_key(&p.id) || report.edges.contains_key(&p.id);
        if status_of(&report, &p.id) != RefState::Confirmed || !on_map {
            problems.push(format!("Not marked as a proposal: \"{name}\" isn't a confirmed box or arrow on this map"));
            continue;
        }
        match set_on(&map, &["nodes", "edges"], &p.id, "proposed", &p.proposed) {
            Ok(next) => {
                map = next;
                marked.push(p.id.clone());
            }
            Err(e) => problems.push(format!("Not marked as a proposal: {}", e.join("; "))),
        }
    }
    if !marked.is_empty() {
        report = check_map(&map, &o.root)?;
    }

    // Screens the answer puts on boxes, old or new. They come after proposals are marked, since a proposed screen
    // needs its box marked. One the checker can't confirm is taken back off (the box keeps whatever screen it had).
    let mut wanted = Vec::new();
    for s in &raw.screens {
        if map.nodes.iter().any(|n| n.id == s.node) {
            wanted.push(s);
        } else {
            problems.push(format!("No screen shown: there is no box \"{}\"", s.node));
        }
    }
    let mut screened: Vec<String> = Vec::new();
    for s in wanted {
        let why = match set_on(&map, &["nodes"], &s.node, "screen", &s.screen) {
            Ok(with_it) => {
                let r = check_map(&with_it, &o.root)?;
                match r.screens.get(&s.node) {
                    Some(c) if c.status == ScreenStatus::Confirmed => {
                        map = with_it;
                        screened.push(s.node.clone());
                        continue;
                    }
                    Some(c) => c.reasons.join("; "),
                    None => "unknown".into(),
                }
            }
            Err(p) => p.join("; "),
        };
        problems.push(format!("No screen shown for \"{}\": {why}", node_name(&map, &s.node)));
    }
    if !screened.is_empty() {
        report = check_map(&map, &o.root)?;
    }

    let mut fix: Option<FixProposal> = None;
    if let Some(raw_fix) = &raw.fix {
        match propose_fix(&o.root, &map, raw_fix) {
            Ok(p) => fix = Some(p),
            Err(p) => problems.extend(p.into_iter().map(|p| format!("The proposed fix can't be applied: {p}"))),
        }
    }

    let (notes, note_problems) = notes_of(&o.root, &raw.notes);
    problems.extend(note_problems);

    let live = map_ids(&map);
    let answer = Answer {
        id: new_id(),
        question: o.question.clone(),
        selection: o.selection.clone(),
        concept: raw.concept,
        refs: unique(&raw.refs).into_iter().map(|id| RefStatus { status: status_of(&report, &id), id }).collect(),
        moves: moves_in(raw.moves, &live),
        added,
        proposed: marked,
        screens: screened.into_iter().filter(|id| live.contains(id)).collect(),
        notes,
        problems,
        fix,
        source: Source::Code,
        offer: None,
        cost: cost + res.cost.unwrap_or(0.0),
    };
    Ok(Answered { answer, map, report, session_id: res.session_id })
}

fn node_name(map: &CodeMap, id: &str) -> String {
    map.nodes.iter().find(|n| n.id == id).map_or(id.to_string(), |n| n.name.clone())
}

/// Every id on the map in the order the map lists them (`map_ids` is a set).
fn ordered_ids(map: &CodeMap) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    out.extend(map.areas.iter().map(|a| a.id.clone()));
    out.extend(map.nodes.iter().map(|n| n.id.clone()));
    out.extend(map.edges.iter().map(|e| e.id.clone()));
    out.extend(map.absences.iter().map(|a| a.id.clone()));
    out.extend(map.tour.iter().map(|t| t.id.clone()));
    unique(&out)
}
