//! The explorer's head start: what a quick deterministic scan already knows (ways in, routes, which requests hit which
//! routes, which tables are written), so the model spends its reading on the question instead of on finding the
//! edges.
//!
//! Ported from `packages/server/src/hints.ts` (MIT, Copyright (c) 2026 johnz4021); see NOTICE.

use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use indexmap::{IndexMap, IndexSet};
use nonio_facts::{find_fetch_sites, match_route, EntryKind, SourceFile, SymbolKind};
use regex::Regex;

use crate::changes::recent_changes;
use crate::scan::{scan_repo, RepoScan};

const MAX_LINES: usize = 40;
/// The function index can be longer: it's what keeps citations exact, and input is cheap next to output.
const MAX_SYMBOL_FILES: usize = 150;
const MAX_SYMBOLS_PER_FILE: usize = 40;

/// Tests, fixtures and mocks: code that exists to exercise the app, which a map of the app shouldn't start from.
static TEST_FILE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(^|/)(test|tests|__tests__|__mocks__|e2e|fixtures?)/|\.(test|spec)\.[cm]?[jt]sx?$|_test\.(go|py)$|(^|/)test_[^/]*\.py$")
        .expect("valid regex")
});
static JS_FILE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.(tsx?|jsx?|mjs|cjs)$").expect("valid regex"));
static FETCHES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bfetch\(|\buse(?:Lazy)?Fetch\b|axios|ky\.").expect("valid regex"));

pub fn is_test_file(path: &str) -> bool {
    TEST_FILE.is_match(path)
}

fn citable(kind: SymbolKind) -> bool {
    matches!(kind, SymbolKind::Function | SymbolKind::Method | SymbolKind::Component | SymbolKind::Class)
}

fn section(title: &str, lines: &[String], max: usize) -> Vec<String> {
    if lines.is_empty() {
        return Vec::new();
    }
    let mut out = vec![format!("{title}:")];
    out.extend(lines.iter().take(max).cloned());
    if lines.len() > max {
        out.push(format!("  …and {} more", lines.len() - max));
    }
    out.push(String::new());
    out
}

fn names_in(facts: &nonio_facts::FileFacts) -> Vec<String> {
    facts.symbols.iter().filter(|s| citable(s.kind) && !s.name.contains('<')).map(|s| s.name.clone()).collect()
}

/// Every function a box can name as its `symbol`, by file, exactly as the checker knows them. Test files and
/// anonymous callbacks are left out.
pub fn symbol_index(scan: &RepoScan) -> Vec<String> {
    let mut files: Vec<(&str, Vec<String>)> = scan
        .facts
        .iter()
        .filter(|f| !is_test_file(&f.path))
        .map(|f| (f.path.as_str(), names_in(f)))
        .filter(|(_, n)| !n.is_empty())
        .collect();
    files.sort_by(|a, b| a.0.cmp(b.0));
    files
        .into_iter()
        .map(|(path, names)| {
            let more = if names.len() > MAX_SYMBOLS_PER_FILE {
                format!(", …and {} more", names.len() - MAX_SYMBOLS_PER_FILE)
            } else {
                String::new()
            };
            let shown: Vec<&str> = names.iter().take(MAX_SYMBOLS_PER_FILE).map(String::as_str).collect();
            format!("  - {path}: {}{more}", shown.join(", "))
        })
        .collect()
}

/// Function names by file, for suggesting the real name when a claim cites one the code doesn't have.
pub fn symbols_by_file(scan: Option<&RepoScan>) -> HashMap<String, Vec<String>> {
    scan.map(|s| s.facts.iter().map(|f| (f.path.clone(), names_in(f))).collect()).unwrap_or_default()
}

/// The function index, when it fits. In a big repo a cut-off list would name files the question isn't about, so it's
/// left out: the model names functions by the rules, and a repair round sends back the real names in the files it
/// cited.
fn function_section(index: &[String]) -> Vec<String> {
    if index.len() <= MAX_SYMBOL_FILES {
        return section("Functions by file (use these exact names for `symbol`)", index, MAX_SYMBOL_FILES);
    }
    vec![
        format!("Functions by file: not listed ({} files have functions, too many to list). Name each `symbol` by the rules above; if a name is off, the check sends back the real names in that file.", index.len()),
        String::new(),
    ]
}

fn entry_kind(k: EntryKind) -> &'static str {
    match k {
        EntryKind::Bin => "bin",
        EntryKind::Script => "script",
        EntryKind::Route => "route",
        EntryKind::Page => "page",
        EntryKind::Cli => "cli",
        EntryKind::Main => "main",
    }
}

pub struct HeadStart {
    pub text: String,
    pub scan: Option<RepoScan>,
}

pub fn head_start(root: &Path) -> HeadStart {
    let scan = scan_repo(root);
    // Tests and fixtures are left out of every section: their routes and tables aren't the app's.
    let facts: Vec<&nonio_facts::FileFacts> = scan.facts.iter().filter(|f| !is_test_file(&f.path)).collect();

    let entries: Vec<String> = facts
        .iter()
        .flat_map(|f| &f.entries)
        .map(|e| {
            let to = match &e.symbol {
                Some(s) => format!(" → {s}"),
                None => format!(" ({})", e.file),
            };
            format!("  - {} {}{to}", entry_kind(e.kind), e.label)
        })
        .collect();
    let routes: Vec<nonio_facts::RouteFact> = facts.iter().flat_map(|f| f.routes.iter().cloned()).collect();
    let route_lines: Vec<String> = routes
        .iter()
        .map(|r| format!("  - {} {} → {}", r.method, r.path, r.handler.as_deref().unwrap_or(&r.file)))
        .collect();

    let sources: Vec<(&nonio_facts::FileFacts, String)> = facts
        .iter()
        .filter(|f| JS_FILE.is_match(&f.path))
        .filter_map(|f| std::fs::read_to_string(root.join(&f.path)).ok().map(|s| (*f, s)))
        .filter(|(_, s)| FETCHES.is_match(s))
        .collect();
    let files: Vec<SourceFile<'_>> =
        sources.iter().map(|(f, s)| SourceFile { path: &f.path, source: s, symbols: &f.symbols }).collect();
    let requests: Vec<String> = find_fetch_sites(&files)
        .into_iter()
        .filter_map(|site| {
            let (route, _) = match_route(&site.method, &site.url, site.exact, &routes)?;
            Some(format!(
                "  - {} sends {} {} → {}",
                site.from,
                site.method,
                site.url,
                route.handler.as_deref().unwrap_or(&route.file)
            ))
        })
        .collect();

    let mut writers: IndexMap<&str, IndexSet<&str>> = IndexMap::new();
    for f in &facts {
        for w in &f.writes {
            writers.entry(&w.table).or_default().insert(w.symbol.as_deref().unwrap_or(&f.path));
        }
    }
    let tables: Vec<String> = writers
        .iter()
        .map(|(t, by)| format!("  - {t} ← {}", by.iter().take(4).copied().collect::<Vec<_>>().join(", ")))
        .collect();

    let mut folders: IndexMap<&str, usize> = IndexMap::new();
    for f in scan.files.iter().filter(|f| !is_test_file(f)) {
        let top = f.split_once('/').map_or("(root)", |(d, _)| d);
        *folders.entry(top).or_default() += 1;
    }
    let mut folders: Vec<(&str, usize)> = folders.into_iter().collect();
    // Stable, so folders with as many files keep the order they were first seen in.
    folders.sort_by_key(|f| std::cmp::Reverse(f.1));
    let folder_lines: Vec<String> = folders.iter().map(|(d, n)| format!("  - {d}/ ({n} files)")).collect();

    // What changed lately, for a question about a change. Never worth failing a map over.
    let recent = recent_changes(root);

    let mut out: Vec<String> = Vec::new();
    out.extend(section("Ways in (entry points)", &entries, MAX_LINES));
    out.extend(section("Routes and their handlers", &route_lines, MAX_LINES));
    out.extend(section("Requests from the code that match a route", &requests, MAX_LINES));
    out.extend(section("Tables and what writes them", &tables, MAX_LINES));
    out.extend(section("Top-level folders", &folder_lines, MAX_LINES));
    out.extend(function_section(&symbol_index(&scan)));
    if let Some(r) = recent {
        out.extend(section(&r.title, &r.lines, MAX_LINES));
    }
    let text = out.join("\n");
    let text = if text.is_empty() {
        "(The automatic scan found no entry points; explore the repo directly.)".into()
    } else {
        text
    };
    HeadStart { text, scan: Some(scan) }
}
