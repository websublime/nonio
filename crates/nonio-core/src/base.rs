//! The code as it was at a commit, read through git: what a map of a change (`since`) is compared with. Read-only,
//! the same rules as the working tree, and nothing is checked out or written.
//!
//! Ported from `packages/core/src/base.ts` of illuminate (MIT, Copyright (c) 2026 johnz4021).

use std::cell::{OnceCell, RefCell};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;

use crate::format::{is_git_ref, is_safe_relative_path};
use crate::repo::{is_secret_path, ReadResult, Reader, MAX_FILE_BYTES};

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let out =
        Command::new("git").args(args).current_dir(root).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The full hash `reference` names in the repo at `root`, or `None` when it names no commit (or this isn't a git repo).
pub fn resolve_commit(root: &Path, reference: &str) -> Option<String> {
    if !is_git_ref(reference) {
        return None;
    }
    let spec = format!("{reference}^{{commit}}");
    let out = git(root, &["rev-parse", "--verify", "--quiet", "--end-of-options", &spec])?;
    let out = out.trim();
    (out.len() >= 40 && out.len() <= 64 && out.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()))
        .then(|| out.to_string())
}

/// When a commit was made and its first line, for saying what a map is compared with.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct CommitInfo {
    pub date: Option<String>,
    pub subject: Option<String>,
}

pub fn commit_info(root: &Path, commit: &str) -> CommitInfo {
    let Some(out) = git(root, &["show", "-s", "--format=%cI%x00%s", "--end-of-options", commit]) else {
        return CommitInfo::default();
    };
    let out = out.trim();
    let mut parts = out.splitn(2, '\0');
    let some = |s: Option<&str>| s.filter(|s| !s.is_empty()).map(str::to_string);
    CommitInfo { date: some(parts.next()), subject: some(parts.next()) }
}

/// A reader over the repo as it was at one commit.
#[derive(Debug)]
pub struct BaseReader {
    root: PathBuf,
    commit: String,
    files: OnceCell<HashSet<String>>,
    cache: RefCell<HashMap<String, ReadResult>>,
}

impl BaseReader {
    /// A reader over the repo at `root` as it was at `reference`, or `None` when `reference` names no commit there.
    pub fn new(root: impl AsRef<Path>, reference: &str) -> Option<Self> {
        let root = fs::canonicalize(root).ok()?;
        let commit = resolve_commit(&root, reference)?;
        Some(Self { root, commit, files: OnceCell::new(), cache: RefCell::default() })
    }

    pub fn commit(&self) -> &str {
        &self.commit
    }

    fn files(&self) -> &HashSet<String> {
        // Run from `root`, git lists and reads paths relative to it, so a repo inside a bigger checkout works the same.
        self.files.get_or_init(|| {
            git(&self.root, &["ls-tree", "-r", "--name-only", "-z", &self.commit])
                .unwrap_or_default()
                .split('\0')
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
    }

    fn read_uncached(&self, rel: &str) -> ReadResult {
        if !is_safe_relative_path(rel) {
            return Err("path is outside the repo".into());
        }
        if is_secret_path(rel) {
            return Err("secret files are never read".into());
        }
        if !self.has(rel) {
            return Err("file not found".into());
        }
        let Some(text) = git(&self.root, &["show", &format!("{}:./{rel}", self.commit)]) else {
            return Err("file not found".into());
        };
        if text.len() as u64 > MAX_FILE_BYTES {
            return Err("file is too large to check".into());
        }
        if text.chars().take(8000).any(|c| c == '\0') {
            return Err("binary file".into());
        }
        Ok(Rc::from(text.as_str()))
    }
}

impl Reader for BaseReader {
    fn root(&self) -> &Path {
        &self.root
    }

    fn read(&self, rel: &str) -> ReadResult {
        if let Some(hit) = self.cache.borrow().get(rel) {
            return hit.clone();
        }
        let r = self.read_uncached(rel);
        self.cache.borrow_mut().insert(rel.to_string(), r.clone());
        r
    }

    fn has(&self, rel: &str) -> bool {
        self.files().contains(rel)
    }
}

/// What changed between two versions of some code, as lines: `+` added, `-` removed, two spaces for a line that
/// stayed (a couple are kept around each change), and `…` where unchanged lines were left out. `None` when the texts
/// are too long to compare line by line.
pub fn line_diff(before: &str, after: &str, context: usize, max: usize) -> Option<Vec<String>> {
    let a: Vec<&str> = before.split('\n').collect();
    let b: Vec<&str> = after.split('\n').collect();
    let mut start = 0;
    while start < a.len() && start < b.len() && a[start] == b[start] {
        start += 1;
    }
    let (mut end_a, mut end_b) = (a.len(), b.len());
    while end_a > start && end_b > start && a[end_a - 1] == b[end_b - 1] {
        end_a -= 1;
        end_b -= 1;
    }
    let (n, m) = (end_a - start, end_b - start);
    if n * m > 1_000_000 {
        return None;
    }
    // Longest common subsequence of the middle, then walk it back into kept, removed and added lines.
    let w = m + 1;
    let mut lcs = vec![0u32; (n + 1) * w];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i * w + j] = if a[start + i] == b[start + j] {
                lcs[(i + 1) * w + j + 1] + 1
            } else {
                lcs[(i + 1) * w + j].max(lcs[i * w + j + 1])
            };
        }
    }
    let mut ops: Vec<(char, &str)> = a[..start].iter().map(|l| (' ', *l)).collect();
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && a[start + i] == b[start + j] {
            ops.push((' ', a[start + i]));
            i += 1;
            j += 1;
        } else if i < n && (j == m || lcs[(i + 1) * w + j] >= lcs[i * w + j + 1]) {
            ops.push(('-', a[start + i]));
            i += 1;
        } else {
            ops.push(('+', b[start + j]));
            j += 1;
        }
    }
    ops.extend(a[end_a..].iter().map(|l| (' ', *l)));
    let mut keep = vec![false; ops.len()];
    for (i, (op, _)) in ops.iter().enumerate() {
        if *op == ' ' {
            continue;
        }
        let last = (i + context).min(ops.len() - 1);
        keep[i.saturating_sub(context)..=last].fill(true);
    }
    let mut out: Vec<String> = Vec::new();
    let mut skipped = false;
    for (i, (op, line)) in ops.iter().enumerate() {
        if !keep[i] {
            skipped = true;
            continue;
        }
        if skipped && !out.is_empty() {
            out.push("…".into());
        }
        skipped = false;
        if out.len() >= max {
            out.push("…".into());
            break;
        }
        out.push(if *op == ' ' { format!("  {line}") } else { format!("{op} {line}") });
    }
    Some(out)
}
