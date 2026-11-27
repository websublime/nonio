//! A whole repo's facts, for the head start and for suggesting real names in a repair round. The checker extracts
//! facts on demand, per file; this reads every file the backends know once.
//!
//! Ported from `scanRepoFacts` in `packages/facts/src/scan.ts` (MIT, Copyright (c) 2026 johnz4021); see NOTICE.
//! nonio's backends also read Vue, Java and Kotlin; Go has no facts yet.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use nonio_core::repo::{is_secret_path, list_repo_files, DEFAULT_IGNORE, MAX_FILE_BYTES};
use nonio_facts::{settle_calls, symbol_names, Extractor, FileFacts, RepoView};
use nonio_scan::{is_code_file, is_graph_file, Scanner};

/// What one scan of the repo found: every file it lists, and the facts of those the backends read.
#[derive(Debug, Clone, Default)]
pub struct RepoScan {
    pub files: Vec<String>,
    /// In the order of `files`.
    pub facts: Vec<FileFacts>,
}

impl RepoScan {
    pub fn get(&self, file: &str) -> Option<&FileFacts> {
        self.facts.iter().find(|f| f.path == file)
    }
}

/// Files a backend reads: TS/JS, Vue, `.sql` schemas, and Java, Kotlin and Python.
pub fn has_facts(path: &str) -> bool {
    path.ends_with(".sql") || path.ends_with(".vue") || is_code_file(path) || is_graph_file(path)
}

struct Disk<'a> {
    root: PathBuf,
    known: &'a HashSet<&'a str>,
}

impl RepoView for Disk<'_> {
    fn exists(&self, path: &str) -> bool {
        self.known.contains(path) || self.root.join(path).is_file()
    }
    fn text(&self, path: &str) -> Option<Rc<str>> {
        read(&self.root, path).map(Rc::from)
    }
}

fn read(root: &Path, path: &str) -> Option<String> {
    if is_secret_path(path) {
        return None;
    }
    let p = root.join(path);
    if fs::metadata(&p).ok()?.len() > MAX_FILE_BYTES {
        return None;
    }
    fs::read_to_string(p).ok()
}

/// Every file of the repo at `root`, and the facts of the ones a backend reads, with calls settled across the repo.
pub fn scan_repo(root: &Path) -> RepoScan {
    let files = list_repo_files(root, DEFAULT_IGNORE);
    let known: HashSet<&str> = files.iter().map(String::as_str).collect();
    let view = Disk { root: root.to_path_buf(), known: &known };
    let scanner = Scanner::new(root, files.iter().cloned());
    let mut facts: Vec<FileFacts> = files
        .iter()
        .filter(|f| has_facts(f))
        .filter_map(|f| {
            let text = read(root, f)?;
            scanner.extract(&view, f, &text)
        })
        .collect();
    let by_file: HashMap<String, FileFacts> = facts.iter().map(|f| (f.path.clone(), f.clone())).collect();
    let names = symbol_names(&by_file);
    for f in &mut facts {
        f.calls = settle_calls(&f.calls, |p| names.get(p));
    }
    drop(view);
    RepoScan { files, facts }
}
