//! What differs from a commit, for the head start of a map about a change: which files, and in code which functions
//! are new, different or gone. Deterministic (git and the code facts), so the model doesn't have to work the diff out
//! itself.
//!
//! Ported from `packages/server/src/changes.ts` and `diffSymbols` in `packages/facts/src/facts/diff.ts` (MIT,
//! Copyright (c) 2026 johnz4021); see NOTICE. "Code" here is every file a nonio backend reads, so Vue, Java and
//! Kotlin count too.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::{Command, Stdio};
use std::rc::Rc;

use nonio_core::base::{commit_info, BaseReader};
use nonio_core::repo::{is_secret_path, list_repo_files, Reader, RepoReader, DEFAULT_IGNORE, STATE_DIR};
use nonio_facts::{Extractor, RepoView, SymbolFact, SymbolKind};
use nonio_scan::Scanner;

use crate::scan::has_facts;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum What {
    New,
    Changed,
    Deleted,
}

impl What {
    pub fn as_str(self) -> &'static str {
        match self {
            What::New => "new",
            What::Changed => "changed",
            What::Deleted => "deleted",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub file: String,
    pub what: What,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changes {
    pub commit: String,
    pub subject: Option<String>,
    pub files: Vec<FileChange>,
}

/// Symbols added, removed, with a different signature or body, and renamed (same kind and body, new id).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SymbolDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub sig: Vec<String>,
    pub body: Vec<String>,
    pub renamed: Vec<(String, String)>,
}

/// Classify symbols as added, removed, signature-changed or body-changed by comparing hashes. A removed and an added
/// symbol of the same kind with the same `body_hash` is a rename, listed only under `renamed`.
pub fn diff_symbols(before: &[SymbolFact], after: &[SymbolFact]) -> SymbolDiff {
    let old: HashMap<&str, &SymbolFact> = before.iter().map(|s| (s.id.as_str(), s)).collect();
    let now: HashSet<&str> = after.iter().map(|s| s.id.as_str()).collect();
    let mut d = SymbolDiff::default();
    for s in after {
        let Some(o) = old.get(s.id.as_str()) else { continue };
        if o.sig_hash != s.sig_hash {
            d.sig.push(s.id.clone());
        } else if o.body_hash != s.body_hash {
            d.body.push(s.id.clone());
        }
    }
    let removed: Vec<&SymbolFact> = before.iter().filter(|s| !now.contains(s.id.as_str())).collect();
    let added: Vec<&SymbolFact> = after.iter().filter(|s| !old.contains_key(s.id.as_str())).collect();
    let mut taken: HashSet<&str> = HashSet::new();
    for r in &removed {
        let Some(a) =
            added.iter().find(|x| !taken.contains(x.id.as_str()) && x.kind == r.kind && x.body_hash == r.body_hash)
        else {
            continue;
        };
        taken.insert(&a.id);
        taken.insert(&r.id);
        d.renamed.push((r.id.clone(), a.id.clone()));
    }
    d.added = added.iter().filter(|s| !taken.contains(s.id.as_str())).map(|s| s.id.clone()).collect();
    d.removed = removed.iter().filter(|s| !taken.contains(s.id.as_str())).map(|s| s.id.clone()).collect();
    d
}

const MAX_NAMES: usize = 8;

fn git_list(root: &Path, args: &[&str]) -> Vec<String> {
    let Ok(out) = Command::new("git").args(args).current_dir(root).stdin(Stdio::null()).stderr(Stdio::null()).output()
    else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&out.stdout).split('\0').filter(|s| !s.is_empty()).map(str::to_string).collect()
}

struct View<'a>(&'a dyn Reader);

impl RepoView for View<'_> {
    fn exists(&self, path: &str) -> bool {
        self.0.has(path)
    }
    fn text(&self, path: &str) -> Option<Rc<str>> {
        self.0.text(path)
    }
}

fn is_code(file: &str) -> bool {
    has_facts(file) && !file.ends_with(".sql")
}

/// Files that differ between the commit `reference` names and the code as it is now (untracked files count as new).
/// `None` when `reference` names no commit.
pub fn changes_since(root: &Path, reference: &str) -> Option<Changes> {
    let base = BaseReader::new(root, reference)?;
    let now = RepoReader::new(root).ok()?;
    let at = base.root().to_path_buf();
    let mut found: Vec<(String, What)> = Vec::new();
    let mut put = |file: String, what: What| match found.iter_mut().find(|(f, _)| *f == file) {
        Some(slot) => slot.1 = what,
        None => found.push((file, what)),
    };
    let diff = git_list(&at, &["diff", "--name-status", "--no-renames", "--relative", "-z", base.commit(), "--", "."]);
    for pair in diff.as_chunks::<2>().0 {
        let what = match pair[0].as_str() {
            "A" => What::New,
            "D" => What::Deleted,
            _ => What::Changed,
        };
        put(pair[1].clone(), what);
    }
    for f in git_list(&at, &["ls-files", "--others", "--exclude-standard", "-z"]) {
        put(f, What::New);
    }

    let scanner = Scanner::new(&at, list_repo_files(&at, DEFAULT_IGNORE));
    // The fields of a type are left out: the type itself is listed when one of them changes.
    let symbols = |reader: &dyn Reader, file: &str| -> Vec<SymbolFact> {
        let Some(text) = reader.text(file) else { return Vec::new() };
        scanner
            .extract(&View(reader), file, &text)
            .map(|f| {
                f.symbols.into_iter().filter(|s| !s.name.contains('<') && s.kind != SymbolKind::Property).collect()
            })
            .unwrap_or_default()
    };
    let names = |label: &str, ids: &[String]| -> Option<String> {
        if ids.is_empty() {
            return None;
        }
        let shown: Vec<&str> =
            ids.iter().take(MAX_NAMES).map(|id| id.split_once('#').map_or(id.as_str(), |(_, n)| n)).collect();
        let more =
            if ids.len() > MAX_NAMES { format!(", …and {} more", ids.len() - MAX_NAMES) } else { String::new() };
        Some(format!("{label}: {}{more}", shown.join(", ")))
    };
    let own = format!("{STATE_DIR}/");
    let mut files: Vec<FileChange> = found
        .into_iter()
        .filter(|(file, _)| !file.starts_with(&own) && !is_secret_path(file))
        .map(|(file, what)| {
            if !is_code(&file) {
                return FileChange { file, what, detail: String::new() };
            }
            let after = if what == What::Deleted { Vec::new() } else { symbols(&now, &file) };
            let d = diff_symbols(&symbols(&base, &file), &after);
            let changed: Vec<String> = d.sig.iter().chain(&d.body).cloned().collect();
            let renamed: Vec<String> = d.renamed.iter().map(|(_, to)| to.clone()).collect();
            let detail = [
                names("new", &d.added),
                names("changed", &changed),
                names("removed", &d.removed),
                names("renamed", &renamed),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("; ");
            FileChange { file, what, detail }
        })
        .collect();
    // Code first: it is what a map is made of.
    files.sort_by(|a, b| is_code(&b.file).cmp(&is_code(&a.file)).then_with(|| a.file.cmp(&b.file)));
    Some(Changes { commit: base.commit().to_string(), subject: commit_info(&at, base.commit()).subject, files })
}

/// A section of the head start: its title and lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recent {
    pub title: String,
    pub lines: Vec<String>,
}

/// The head start's section on recent changes: work not committed yet, or (when there is none) what the last commit
/// did. Each says which `since` a map of it would set. `None` outside a git repo, or when nothing differs.
pub fn recent_changes(root: &Path) -> Option<Recent> {
    let line = |c: &FileChange| {
        let detail = if c.detail.is_empty() { String::new() } else { format!(" ({})", c.detail) };
        format!("  - {} {}{detail}", c.what.as_str(), c.file)
    };
    if let Some(open) = changes_since(root, "HEAD").filter(|c| !c.files.is_empty()) {
        let short: String = open.commit.chars().take(7).collect();
        return Some(Recent {
            title: format!(
                "Not committed yet (differences from the last commit, {short}; a map of these sets `since`: \"HEAD\")"
            ),
            lines: open.files.iter().map(line).collect(),
        });
    }
    let last = changes_since(root, "HEAD~1").filter(|c| !c.files.is_empty())?;
    Some(Recent {
        title: "Everything is committed. What the last commit changed (a map of it sets `since`: \"HEAD~1\")".into(),
        lines: last.files.iter().map(line).collect(),
    })
}
