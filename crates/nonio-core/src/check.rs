//! The checker: decides which claims in a map the code actually backs. It reads the repo and never writes. One
//! function, used everywhere a map reaches a person. No model calls.
//!
//! Ported from `packages/core/src/check.ts` of illuminate (MIT, Copyright (c) 2026 johnz4021). Facts come from an
//! [`Extractor`] (D12); the checker never learns which backend produced them.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;

use indexmap::IndexMap;
use nonio_facts::{
    find_fetch_sites, match_route, matches_any, settle_calls, squash, CallFact, Extractor, FileFacts, RepoView,
    SourceFile, SymbolFact,
};
use regex::Regex;
use serde::Serialize;

use crate::base::{commit_info, line_diff, BaseReader};
use crate::format::{CodeMap, EdgeKind, Evidence, MapAbsence, MapEdge, MapNode, ProposalKind};
use crate::repo::{is_secret_path, list_repo_files, Reader, RepoReader, DEFAULT_IGNORE};
use crate::screens::{check_page, check_screen, PageCheck, ScreenCheck};

/// Whether the code backs a claim. `Proposed` is only for a box or arrow that carries a proposal: it isn't in the
/// code, and the proposal says it shouldn't be (an addition not built yet, or a change or removal already made).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Confirmed,
    Unconfirmed,
    Proposed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProposalState {
    Pending,
    Done,
}

/// Where a proposal stands, computed from the repo like everything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ProposalCheck {
    pub kind: ProposalKind,
    pub state: ProposalState,
}

/// `Fact`: the extracted code facts back the claim. `Text`: the cited lines say what the claim says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Proof {
    Fact,
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceCheck {
    pub file: String,
    pub line: usize,
    pub ok: bool,
    pub found_at: Option<usize>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Added,
    Changed,
    Removed,
}

/// On a map of a change: how this box or arrow differs from that commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChangeCheck {
    pub kind: ChangeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClaimCheck {
    pub status: Status,
    pub proof: Option<Proof>,
    pub evidence: Vec<EvidenceCheck>,
    pub reasons: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposal: Option<ProposalCheck>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change: Option<ChangeCheck>,
}

/// What a map of a change is compared with. `commit` is `None` when `ref` names no commit in this repo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SinceCheck {
    #[serde(rename = "ref")]
    pub reference: String,
    pub commit: Option<String>,
    pub date: Option<String>,
    pub subject: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FoundAt {
    pub file: String,
    pub line: usize,
}

/// `searched` is how many files the search covered, so a person can judge whether it looked widely enough.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AbsenceCheck {
    pub status: Status,
    pub found: u64,
    pub searched: usize,
    pub locations: Vec<FoundAt>,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum TourStatus {
    #[serde(rename = "ok")]
    Ok,
    #[serde(rename = "mentions-unconfirmed")]
    MentionsUnconfirmed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TourCheck {
    pub status: TourStatus,
    pub unconfirmed: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ProposalCount {
    pub pending: usize,
    pub done: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ChangeCount {
    pub added: usize,
    pub changed: usize,
    pub removed: usize,
}

/// `claims` counts what the map says about the code as it is; what is only proposed is under `proposals`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub claims: usize,
    pub confirmed: usize,
    pub unconfirmed: usize,
    pub proposals: ProposalCount,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changes: Option<ChangeCount>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CheckReport {
    pub nodes: IndexMap<String, ClaimCheck>,
    pub edges: IndexMap<String, ClaimCheck>,
    pub absences: IndexMap<String, AbsenceCheck>,
    pub tour: IndexMap<String, TourCheck>,
    /// By box id, for boxes that carry a screen. Not counted as claims: a screen that fails is simply not shown.
    pub screens: IndexMap<String, ScreenCheck>,
    pub summary: Summary,
    /// Only on a map of a change.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<SinceCheck>,
}

/// How far a cited line may have moved and still count, unless a caller says otherwise.
pub const DEFAULT_LINE_WINDOW: usize = 3;

#[derive(Debug, Clone)]
pub struct CheckOptions {
    /// Extra globs to leave out of searches (build output and nonio's own folder are always left out).
    pub ignore: Vec<String>,
    /// How far a cited line may have moved and still count.
    pub line_window: usize,
}

impl Default for CheckOptions {
    fn default() -> Self {
        Self { ignore: Vec::new(), line_window: DEFAULT_LINE_WINDOW }
    }
}

const MAX_LOCATIONS: usize = 20;

fn file_of(symbol: &str) -> &str {
    &symbol[..symbol.find('#').unwrap_or(0)]
}

fn within(from: &str, symbol: &str) -> bool {
    from == symbol || from.strip_prefix(symbol).is_some_and(|rest| rest.starts_with('.'))
}

fn short(text: &str) -> String {
    if text.encode_utf16().count() > 60 {
        format!("{}…", text.chars().take(57).collect::<String>())
    } else {
        text.to_string()
    }
}

/// The repo as an extractor sees it: one version of the files, and the working tree for files that version
/// doesn't decide (so a commit's imports resolve the way the original's `carryResolver` does).
struct View<'a> {
    repo: &'a dyn Reader,
    fallback: Option<&'a dyn Reader>,
}

impl RepoView for View<'_> {
    fn exists(&self, path: &str) -> bool {
        self.repo.has(path) || self.fallback.is_some_and(|f| f.has(path))
    }
    fn text(&self, path: &str) -> Option<Rc<str>> {
        self.repo.text(path)
    }
}

/// Facts per file, extracted on demand and cached for one check.
pub struct Facts<'a> {
    view: View<'a>,
    extractor: &'a dyn Extractor,
    cache: RefCell<HashMap<String, Option<Rc<FileFacts>>>>,
}

impl<'a> Facts<'a> {
    fn new(repo: &'a dyn Reader, fallback: Option<&'a dyn Reader>, extractor: &'a dyn Extractor) -> Self {
        Self { view: View { repo, fallback }, extractor, cache: RefCell::default() }
    }

    /// Facts for a file this version of the code can read; a file the extractor has nothing for has empty facts.
    pub fn get(&self, file: &str) -> Option<Rc<FileFacts>> {
        if let Some(hit) = self.cache.borrow().get(file) {
            return hit.clone();
        }
        let facts = self.view.repo.text(file).map(|text| {
            Rc::new(self.extractor.extract(&self.view, file, &text).unwrap_or_else(|| FileFacts::empty(file)))
        });
        self.cache.borrow_mut().insert(file.to_string(), facts.clone());
        facts
    }

    /// Calls made in `file`, settled against the symbols of `file` and `others`.
    fn calls(&self, file: &str, others: &[&str]) -> Vec<CallFact> {
        let Some(own) = self.get(file) else {
            return Vec::new();
        };
        let mut names: HashMap<String, HashSet<String>> = HashMap::new();
        for p in std::iter::once(file).chain(others.iter().copied()) {
            if let Some(f) = self.get(p) {
                names.insert(p.to_string(), f.symbols.iter().map(|s| s.name.clone()).collect());
            }
        }
        settle_calls(&own.calls, |f| names.get(f))
    }

    fn symbol(&self, symbol: &str) -> Option<SymbolFact> {
        self.get(file_of(symbol))?.symbols.iter().find(|s| s.id == symbol).cloned()
    }
}

/// One version of the code: the working tree, or the commit a map of a change is compared with.
pub struct Side<'a> {
    pub repo: &'a dyn Reader,
    pub facts: Facts<'a>,
}

/// The readers for both versions of the code a map is read against. Build once, then take [`Sides::now`] and
/// [`Sides::base`].
pub struct Readers {
    pub now: RepoReader,
    /// Set when the map has `since` and it names a commit here.
    pub base: Option<BaseReader>,
}

impl Readers {
    pub fn new(map_since: Option<&str>, root: &Path) -> std::io::Result<Self> {
        let now = RepoReader::new(root)?;
        let base = map_since.and_then(|since| BaseReader::new(now.root(), since));
        Ok(Self { now, base })
    }

    pub fn sides<'a>(&'a self, extractor: &'a dyn Extractor) -> (Side<'a>, Option<Side<'a>>) {
        let now = Side { repo: &self.now, facts: Facts::new(&self.now, None, extractor) };
        let base = self
            .base
            .as_ref()
            .map(|b| Side { repo: b as &dyn Reader, facts: Facts::new(b, Some(&self.now), extractor) });
        (now, base)
    }
}

struct Compare<'s, 'a> {
    reference: &'s str,
    base: Option<&'s Side<'a>>,
}

/// Check every claim of `map` against the repo at `root`, with facts from `extractor`.
pub fn check(
    map: &CodeMap,
    root: &Path,
    extractor: &dyn Extractor,
    options: &CheckOptions,
) -> std::io::Result<CheckReport> {
    let readers = Readers::new(map.since.as_deref(), root)?;
    let (now, base) = readers.sides(extractor);
    let window = options.line_window;
    let cmp = map.since.as_deref().map(|reference| Compare { reference, base: base.as_ref() });
    let by_id: HashMap<&str, &MapNode> = map.nodes.iter().map(|n| (n.id.as_str(), n)).collect();

    let mut nodes = IndexMap::new();
    for n in &map.nodes {
        nodes.insert(n.id.clone(), check_node(n, &now, cmp.as_ref(), window));
    }
    let mut edges = IndexMap::new();
    for e in &map.edges {
        edges.insert(e.id.clone(), check_edge(e, &by_id, &now, cmp.as_ref(), window));
    }
    let mut absences = IndexMap::new();
    if !map.absences.is_empty() {
        let ignore: Vec<String> =
            DEFAULT_IGNORE.iter().map(|s| s.to_string()).chain(options.ignore.iter().cloned()).collect();
        let files: Vec<String> =
            list_repo_files(now.repo.root(), &ignore).into_iter().filter(|f| !is_secret_path(f)).collect();
        for a in &map.absences {
            absences.insert(a.id.clone(), check_absence(a, now.repo, &files));
        }
    }

    // A page is checked once, however many boxes show it.
    let page_checks: RefCell<HashMap<String, Option<PageCheck>>> = RefCell::default();
    let imports_of = |file: &str| -> Vec<String> {
        now.facts
            .get(file)
            .map(|f| f.imports.iter().map(|i| i.to.clone()).filter(|to| !to.starts_with("vendor:")).collect())
            .unwrap_or_default()
    };
    let page_of = |id: &str| -> Option<PageCheck> {
        if let Some(hit) = page_checks.borrow().get(id) {
            return hit.clone();
        }
        let checked = map.pages.iter().flatten().find(|p| p.id == id).map(|p| check_page(p, now.repo, &imports_of));
        page_checks.borrow_mut().insert(id.to_string(), checked.clone());
        checked
    };
    let mut screens = IndexMap::new();
    for n in &map.nodes {
        if let Some(sc) = check_screen(n, now.repo, &page_of) {
            screens.insert(n.id.clone(), sc);
        }
    }

    let mut tour = IndexMap::new();
    for t in &map.tour {
        let unconfirmed: Vec<String> = t
            .focus
            .nodes
            .iter()
            .flatten()
            .filter(|id| nodes.get(*id).is_some_and(|c: &ClaimCheck| c.status == Status::Unconfirmed))
            .chain(
                t.focus
                    .edges
                    .iter()
                    .flatten()
                    .filter(|id| edges.get(*id).is_some_and(|c: &ClaimCheck| c.status == Status::Unconfirmed)),
            )
            .chain(
                t.focus
                    .absences
                    .iter()
                    .flatten()
                    .filter(|id| absences.get(*id).is_none_or(|c: &AbsenceCheck| c.status != Status::Confirmed)),
            )
            .cloned()
            .collect();
        let status = if unconfirmed.is_empty() { TourStatus::Ok } else { TourStatus::MentionsUnconfirmed };
        tour.insert(t.id.clone(), TourCheck { status, unconfirmed });
    }

    let statuses: Vec<Status> = nodes
        .values()
        .map(|c| c.status)
        .chain(edges.values().map(|c| c.status))
        .chain(absences.values().map(|c| c.status))
        .collect();
    let count = |s: Status| statuses.iter().filter(|x| **x == s).count();
    let claims = || nodes.values().chain(edges.values());
    let mut proposals = ProposalCount::default();
    for p in claims().filter_map(|c| c.proposal) {
        match p.state {
            ProposalState::Pending => proposals.pending += 1,
            ProposalState::Done => proposals.done += 1,
        }
    }
    let changes = cmp.as_ref().map(|_| {
        let mut c = ChangeCount::default();
        for k in claims().filter_map(|c| c.change.as_ref().map(|x| x.kind)) {
            match k {
                ChangeKind::Added => c.added += 1,
                ChangeKind::Changed => c.changed += 1,
                ChangeKind::Removed => c.removed += 1,
            }
        }
        c
    });
    let (confirmed, unconfirmed) = (count(Status::Confirmed), count(Status::Unconfirmed));
    let since = cmp.as_ref().map(|c| {
        let commit = readers.base.as_ref().map(|b| b.commit().to_string());
        let info = commit.as_deref().map(|commit| commit_info(readers.now.root(), commit)).unwrap_or_default();
        SinceCheck { reference: c.reference.to_string(), commit, date: info.date, subject: info.subject }
    });
    Ok(CheckReport {
        nodes,
        edges,
        absences,
        tour,
        screens,
        summary: Summary { claims: confirmed + unconfirmed, confirmed, unconfirmed, proposals, changes },
        since,
    })
}

/// A proposal the code has already settled: an addition it has, or a change or removal whose cited code isn't there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settled {
    pub id: String,
    /// `box` or `arrow`.
    pub what: &'static str,
    pub kind: ProposalKind,
    pub reasons: Vec<String>,
}

/// Proposals the code has already settled. On a map made earlier that is the proposal carried out; in a map being
/// written it is a mistake (the code was misread), so the maker and the explainer send these back.
pub fn settled_proposals(report: &CheckReport) -> Vec<Settled> {
    let of = |what: &'static str, group: &IndexMap<String, ClaimCheck>| -> Vec<Settled> {
        group
            .iter()
            .filter_map(|(id, c)| {
                c.proposal.filter(|p| p.state == ProposalState::Done).map(|p| Settled {
                    id: id.clone(),
                    what,
                    kind: p.kind,
                    reasons: c.reasons.clone(),
                })
            })
            .collect()
    };
    let mut out = of("box", &report.nodes);
    out.extend(of("arrow", &report.edges));
    out
}

/// Whether a cited line is in that version of the code, and where: on its line, a few lines off, or the one line of
/// the file that has its text.
pub fn check_evidence(repo: &dyn Reader, e: &Evidence, window: usize) -> EvidenceCheck {
    let at = |ok, found_at, reason| EvidenceCheck { file: e.file.clone(), line: e.line, ok, found_at, reason };
    let text = match repo.read(&e.file) {
        Ok(t) => t,
        Err(reason) => return at(false, None, Some(reason)),
    };
    let lines: Vec<&str> = text.split('\n').collect();
    let want = squash(&e.text);
    let has = |n: usize| n >= 1 && lines.get(n - 1).is_some_and(|l| squash(l).contains(&want));
    for d in 0..=window {
        let tries: &[Option<usize>] = if d == 0 { &[Some(e.line)] } else { &[e.line.checked_sub(d), Some(e.line + d)] };
        for n in tries.iter().flatten() {
            if has(*n) {
                return at(true, Some(*n), None);
            }
        }
    }
    // Code moves (a fix above it, a new import). Text that appears on exactly one line of the file is still that line.
    let hits: Vec<usize> = (1..=lines.len()).filter(|n| has(*n)).collect();
    if hits.len() == 1 {
        return at(true, Some(hits[0]), None);
    }
    let reason = if hits.len() > 1 {
        format!(
            "\"{}\" isn't on line {} or near it, and appears {} times elsewhere in the file",
            short(&e.text),
            e.line,
            hits.len()
        )
    } else if e.line > lines.len() {
        format!("the file has only {} lines", lines.len())
    } else {
        format!("\"{}\" isn't on line {} or near it", short(&e.text), e.line)
    };
    at(false, None, Some(reason))
}

fn evidence_reasons(evidence: &[EvidenceCheck]) -> Vec<String> {
    evidence
        .iter()
        .filter(|x| !x.ok)
        .map(|x| format!("{}:{}: {}", x.file, x.line, x.reason.as_deref().unwrap_or("")))
        .collect()
}

fn result(evidence: Vec<EvidenceCheck>, reasons: Vec<String>, proof: Proof) -> ClaimCheck {
    let (status, proof) =
        if reasons.is_empty() { (Status::Confirmed, Some(proof)) } else { (Status::Unconfirmed, None) };
    ClaimCheck { status, proof, evidence, reasons, proposal: None, change: None }
}

fn fail(evidence: Vec<EvidenceCheck>, reasons: Vec<String>) -> ClaimCheck {
    ClaimCheck { status: Status::Unconfirmed, proof: None, evidence, reasons, proposal: None, change: None }
}

/// A box or arrow that carries a proposal, read from what the ordinary check found. An addition is in the code once
/// it checks out; a change or removal is pending while its cited code is still there.
fn as_proposal(kind: ProposalKind, mut c: ClaimCheck) -> ClaimCheck {
    let in_code = c.status == Status::Confirmed;
    // An addition is done once it is in the code; a change or removal once its cited code isn't.
    let done = (kind == ProposalKind::Add) == in_code;
    if !in_code {
        c.status = Status::Proposed;
    }
    c.proposal = Some(ProposalCheck { kind, state: if done { ProposalState::Done } else { ProposalState::Pending } });
    c
}

fn check_node(n: &MapNode, now: &Side<'_>, cmp: Option<&Compare<'_, '_>>, window: usize) -> ClaimCheck {
    if n.is_removed() {
        return check_gone(cmp, |side| check_node_now(n, side, window), || node_still_there(n, now));
    }
    let c = check_node_as_is(n, now, window);
    // Only what is in the code and isn't a proposal can have changed since.
    let Some(base) = cmp.and_then(|c| c.base) else {
        return c;
    };
    if n.proposed.is_some() || c.status != Status::Confirmed {
        return c;
    }
    match node_change(n, now, base) {
        Some(change) => ClaimCheck { change: Some(change), ..c },
        None => c,
    }
}

fn check_node_as_is(n: &MapNode, now: &Side<'_>, window: usize) -> ClaimCheck {
    let Some(p) = &n.proposed else {
        return check_node_now(n, now, window);
    };
    if p.kind != ProposalKind::Add {
        return as_proposal(p.kind, check_node_now(n, now, window));
    }
    // An addition is in the code once the function it names exists. Without a name there is nothing to look for.
    if n.symbol.is_none() {
        return as_proposal(
            ProposalKind::Add,
            fail(Vec::new(), vec!["it names no `symbol`, so the code can't show whether it has been built".into()]),
        );
    }
    let bare = MapNode { evidence: Vec::new(), ..n.clone() };
    as_proposal(ProposalKind::Add, check_node_now(&bare, now, window))
}

/// Code the map says was removed: it must check out against the commit the map is compared with, and no longer be
/// in the code now. Anything else is a claim that doesn't hold.
fn check_gone(
    cmp: Option<&Compare<'_, '_>>,
    at: impl Fn(&Side<'_>) -> ClaimCheck,
    still_there: impl Fn() -> bool,
) -> ClaimCheck {
    let Some(cmp) = cmp else {
        return fail(Vec::new(), vec!["the map names no commit (`since`) for this to have been removed after".into()]);
    };
    let Some(base) = cmp.base else {
        return fail(
            Vec::new(),
            vec![format!("\"{}\" isn't a commit in this repo, so there is nothing to compare with", cmp.reference)],
        );
    };
    let was = at(base);
    if was.status != Status::Confirmed {
        let reasons = was.reasons.iter().map(|r| format!("in the code as it was: {r}")).collect();
        return fail(was.evidence, reasons);
    }
    if still_there() {
        return fail(was.evidence, vec!["it is still in the code, so it wasn't removed (drop `removed`)".into()]);
    }
    ClaimCheck { change: Some(ChangeCheck { kind: ChangeKind::Removed, diff: None }), ..was }
}

/// Whether `text` is on some line of `file` in that version of the code.
fn has_line(side: &Side<'_>, file: &str, text: &str) -> bool {
    let Some(source) = side.repo.text(file) else {
        return false;
    };
    let want = squash(text);
    source.split('\n').any(|l| squash(l).contains(&want))
}

/// A removed box is still there while its function exists, or (when it names none) while any line it cites does.
fn node_still_there(n: &MapNode, now: &Side<'_>) -> bool {
    if let Some(sym) = &n.symbol {
        if now.facts.get(file_of(sym)).is_some() {
            return now.facts.symbol(sym).is_some();
        }
    }
    n.evidence.iter().any(|e| has_line(now, &e.file, &e.text))
}

fn span_text(text: &str, s: &SymbolFact) -> String {
    let end = s.span.end.min(text.len());
    text.get(s.span.start.min(end)..end).unwrap_or_default().to_string()
}

/// How a box that is in the code differs from the commit the map is compared with.
fn node_change(n: &MapNode, now: &Side<'_>, base: &Side<'_>) -> Option<ChangeCheck> {
    let added = Some(ChangeCheck { kind: ChangeKind::Added, diff: None });
    if let Some(sym) = &n.symbol {
        let file = file_of(sym);
        let cur = now.facts.symbol(sym);
        if cur.is_some() && !base.repo.has(file) {
            return added;
        }
        if let (Some(cur), Some(_)) = (&cur, base.facts.get(file)) {
            let Some(old) = base.facts.symbol(sym) else {
                return added;
            };
            if old.sig_hash == cur.sig_hash && old.body_hash == cur.body_hash {
                return None;
            }
            let before = span_text(&base.repo.text(file).unwrap_or_default(), &old);
            let after = span_text(&now.repo.text(file).unwrap_or_default(), cur);
            let diff = line_diff(&before, &after, 2, 60).filter(|d| !d.is_empty());
            return Some(ChangeCheck { kind: ChangeKind::Changed, diff });
        }
    }
    if n.evidence.is_empty() {
        return None;
    }
    if n.evidence.iter().all(|e| !base.repo.has(&e.file)) {
        return added;
    }
    n.evidence
        .iter()
        .any(|e| !has_line(base, &e.file, &e.text))
        .then_some(ChangeCheck { kind: ChangeKind::Changed, diff: None })
}

fn check_node_now(n: &MapNode, side: &Side<'_>, window: usize) -> ClaimCheck {
    let evidence: Vec<EvidenceCheck> = n.evidence.iter().map(|e| check_evidence(side.repo, e, window)).collect();
    let mut reasons = evidence_reasons(&evidence);
    let Some(sym) = n.symbol.as_ref().filter(|_| reasons.is_empty()) else {
        return result(evidence, reasons, Proof::Text);
    };
    if side.facts.symbol(sym).is_none() {
        reasons.push(format!("the code has no symbol {sym}"));
    }
    result(evidence, reasons, Proof::Fact)
}

/// Whether the code facts alone can show an arrow: a call, request or import between boxes that name their code.
fn fact_backed(e: &MapEdge, by_id: &HashMap<&str, &MapNode>) -> bool {
    let (Some(from), Some(to)) = (by_id.get(e.from.as_str()), by_id.get(e.to.as_str())) else {
        return false;
    };
    e.kind != EdgeKind::Then
        && (e.kind != EdgeKind::Calls || (from.symbol.is_some() && to.symbol.is_some()))
        && [from, to].iter().all(|n| n.symbol.is_some() || !n.evidence.is_empty())
}

fn check_edge(
    e: &MapEdge,
    by_id: &HashMap<&str, &MapNode>,
    now: &Side<'_>,
    cmp: Option<&Compare<'_, '_>>,
    window: usize,
) -> ClaimCheck {
    let bare = MapEdge { evidence: Vec::new(), ..e.clone() };
    let in_facts = |side: &Side<'_>| {
        fact_backed(e, by_id) && check_edge_now(&bare, by_id, side, window).status == Status::Confirmed
    };
    if e.is_removed() {
        // Gone when the facts no longer show it, or (for an arrow only its lines prove) when a line it cites has gone.
        let still_there = || {
            if fact_backed(e, by_id) {
                in_facts(now)
            } else {
                e.evidence.iter().all(|x| has_line(now, &x.file, &x.text))
            }
        };
        return check_gone(cmp, |side| check_edge_now(e, by_id, side, window), still_there);
    }
    let c = check_edge_as_is(e, by_id, now, window);
    let Some(base) = cmp.and_then(|c| c.base) else {
        return c;
    };
    if e.proposed.is_some() || c.status != Status::Confirmed {
        return c;
    }
    // An arrow is new, or it isn't: new when the facts didn't show it then, or when a line that proves it wasn't there.
    let added = if fact_backed(e, by_id) {
        !in_facts(base)
    } else {
        e.evidence.iter().any(|x| !has_line(base, &x.file, &x.text))
    };
    if added {
        ClaimCheck { change: Some(ChangeCheck { kind: ChangeKind::Added, diff: None }), ..c }
    } else {
        c
    }
}

fn check_edge_as_is(e: &MapEdge, by_id: &HashMap<&str, &MapNode>, now: &Side<'_>, window: usize) -> ClaimCheck {
    let Some(p) = &e.proposed else {
        return check_edge_now(e, by_id, now, window);
    };
    if p.kind != ProposalKind::Add {
        return as_proposal(p.kind, check_edge_now(e, by_id, now, window));
    }
    // An added arrow is in the code once the facts show the call, request or import it stands for.
    if !fact_backed(e, by_id) {
        let why = "the code facts can't show whether this arrow has been built (it isn't a call, request or import between named functions)";
        return as_proposal(ProposalKind::Add, fail(Vec::new(), vec![why.into()]));
    }
    let bare = MapEdge { evidence: Vec::new(), ..e.clone() };
    as_proposal(ProposalKind::Add, check_edge_now(&bare, by_id, now, window))
}

fn check_edge_now(e: &MapEdge, by_id: &HashMap<&str, &MapNode>, side: &Side<'_>, window: usize) -> ClaimCheck {
    let evidence: Vec<EvidenceCheck> = e.evidence.iter().map(|x| check_evidence(side.repo, x, window)).collect();
    let mut reasons = evidence_reasons(&evidence);
    let (Some(from), Some(to)) = (by_id.get(e.from.as_str()), by_id.get(e.to.as_str())) else {
        return result(evidence, reasons, Proof::Text);
    };
    if !reasons.is_empty() || e.kind == EdgeKind::Then {
        return result(evidence, reasons, Proof::Text);
    }

    if e.kind == EdgeKind::Calls {
        let (Some(fs), Some(ts)) = (&from.symbol, &to.symbol) else {
            return result(evidence, reasons, Proof::Text);
        };
        let calls = side.facts.calls(file_of(fs), &[file_of(ts)]);
        if !calls.iter().any(|c| within(&c.from, fs) && &c.to == ts) {
            reasons.push(format!("the code has no call from {fs} to {ts}"));
        }
        return result(evidence, reasons, Proof::Fact);
    }

    let file_or_cited =
        |n: &MapNode| -> String { n.symbol.as_deref().map(file_of).unwrap_or_else(|| &n.evidence[0].file).to_string() };
    let from_file = file_or_cited(from);
    let to_file = file_or_cited(to);
    let Some(from_facts) = side.facts.get(&from_file) else {
        reasons.push(format!("{from_file} can't be read"));
        return result(evidence, reasons, Proof::Fact);
    };

    if e.kind == EdgeKind::Imports {
        if !from_facts.imports.iter().any(|i| i.to == to_file) {
            reasons.push(format!("{from_file} doesn't import {to_file}"));
        }
        return result(evidence, reasons, Proof::Fact);
    }

    // request: a fetch in the `from` box (direct, or through a wrapper it imports) that matches a route in the `to` box.
    let neighbours: Vec<String> = std::iter::once(from_file.clone())
        .chain(from_facts.imports.iter().filter(|i| !i.to.starts_with("vendor:")).map(|i| i.to.clone()))
        .collect();
    let loaded: Vec<(String, Rc<str>, Rc<FileFacts>)> =
        neighbours.iter().filter_map(|p| Some((p.clone(), side.repo.text(p)?, side.facts.get(p)?))).collect();
    let sources: Vec<SourceFile<'_>> =
        loaded.iter().map(|(path, source, f)| SourceFile { path, source, symbols: &f.symbols }).collect();
    let routes = side.facts.get(&to_file).map(|f| f.routes.clone()).unwrap_or_default();
    let matched = find_fetch_sites(&sources).iter().any(|site| {
        if site.file != from_file || from.symbol.as_ref().is_some_and(|s| !within(&site.from, s)) {
            return false;
        }
        match_route(&site.method, &site.url, site.exact, &routes).is_some_and(|(route, _)| match &to.symbol {
            Some(sym) => route.handler.as_deref() == Some(sym.as_str()),
            None => true,
        })
    });
    if !matched {
        let from_name = from.symbol.clone().unwrap_or_else(|| from_file.clone());
        let to_name = to.symbol.clone().unwrap_or_else(|| to_file.clone());
        reasons.push(format!("no request from {from_name} matches a route handled by {to_name}"));
    }
    result(evidence, reasons, Proof::Fact)
}

fn check_absence(a: &MapAbsence, repo: &dyn Reader, files: &[String]) -> AbsenceCheck {
    let pattern = if a.search.regex { a.search.pattern.clone() } else { regex::escape(&a.search.pattern) };
    let Ok(pattern) = Regex::new(&pattern) else {
        return AbsenceCheck {
            status: Status::Unconfirmed,
            found: 0,
            searched: 0,
            locations: Vec::new(),
            reasons: vec!["the search pattern is not a valid regular expression".into()],
        };
    };
    let (mut found, mut searched) = (0u64, 0usize);
    let mut locations = Vec::new();
    for file in files {
        if a.search.paths.as_ref().is_some_and(|globs| !matches_any(globs, file)) {
            continue;
        }
        let Some(text) = repo.text(file) else {
            continue;
        };
        searched += 1;
        for (i, line) in text.split('\n').enumerate() {
            let n = pattern.find_iter(line).count() as u64;
            if n == 0 {
                continue;
            }
            found += n;
            if locations.len() < MAX_LOCATIONS {
                locations.push(FoundAt { file: file.clone(), line: i + 1 });
            }
        }
    }
    let mut reasons = Vec::new();
    if found != a.matches {
        reasons.push(format!(
            "the search finds {found} {}; the map says {}",
            if found == 1 { "match" } else { "matches" },
            a.matches
        ));
    }
    // A search that read no files proves nothing, whatever it claims to have found.
    if searched == 0 {
        reasons.push(match &a.search.paths {
            Some(p) => format!("no files match {}, so nothing was searched", p.join(", ")),
            None => "there were no files to search".into(),
        });
    }
    let status = if reasons.is_empty() { Status::Confirmed } else { Status::Unconfirmed };
    AbsenceCheck { status, found, searched, locations, reasons }
}
