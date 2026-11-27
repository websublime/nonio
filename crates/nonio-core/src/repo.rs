//! Read-only access to the repo for the checker: paths can't leave the repo (symlinks included), secrets are never
//! read, and big or binary files are skipped. The server enforces these rules itself (D25); they are not
//! instructions to the agent.
//!
//! Ported from `packages/core/src/repo.ts` of illuminate (MIT, Copyright (c) 2026 johnz4021).

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;

use nonio_facts::matches_any;

use crate::format::is_safe_relative_path;

/// Never read, never searched, never sent anywhere.
pub const SECRET_GLOBS: &[&str] = &[
    "**/.env",
    "**/.env.*",
    "**/*.pem",
    "**/*.key",
    "**/*.p12",
    "**/id_rsa*",
    "**/id_ed25519*",
    "**/.npmrc",
    "**/.netrc",
];

/// Files above this size are not read.
pub const MAX_FILE_BYTES: u64 = 2_000_000;

/// Where nonio keeps what it writes inside a repo (captures). Provisional until Q05 names the folder.
pub const STATE_DIR: &str = ".nonio";

/// Never scanned or searched: nonio's own output (and illuminate's), and common build output.
pub const DEFAULT_IGNORE: &[&str] = &[
    ".nonio/**",
    ".illuminate/**",
    ".codemap/**",
    "**/dist/**",
    "**/build/**",
    "**/coverage/**",
    "**/.next/**",
    "**/venv/**",
    "**/.venv/**",
    "**/site-packages/**",
    "**/__pycache__/**",
];

/// A file's text, or why it can't be read.
pub type ReadResult = Result<Rc<str>, String>;

pub fn is_secret_path(rel: &str) -> bool {
    matches_any(SECRET_GLOBS, rel) || matches_any(SECRET_GLOBS, &format!("/{rel}"))
}

/// One version of the repo's code, read under the rules above.
pub trait Reader {
    /// The repo root, with symlinks resolved.
    fn root(&self) -> &Path;
    fn read(&self, rel: &str) -> ReadResult;
    fn text(&self, rel: &str) -> Option<Rc<str>> {
        self.read(rel).ok()
    }
    /// Whether this version of the repo has the file (whether or not it may be read).
    fn has(&self, rel: &str) -> bool;
}

fn is_binary(text: &str) -> bool {
    text.chars().take(8000).any(|c| c == '\0')
}

/// The working tree.
#[derive(Debug)]
pub struct RepoReader {
    root: PathBuf,
    cache: RefCell<HashMap<String, ReadResult>>,
}

impl RepoReader {
    pub fn new(root: impl AsRef<Path>) -> std::io::Result<Self> {
        Ok(Self { root: fs::canonicalize(root)?, cache: RefCell::default() })
    }

    fn read_uncached(&self, rel: &str) -> ReadResult {
        if !is_safe_relative_path(rel) {
            return Err("path is outside the repo".into());
        }
        if is_secret_path(rel) {
            return Err("secret files are never read".into());
        }
        let Ok(real) = fs::canonicalize(self.root.join(rel)) else {
            return Err("file not found".into());
        };
        if real != self.root && !real.starts_with(&self.root) {
            return Err("path is outside the repo".into());
        }
        let meta = fs::metadata(&real).map_err(|_| "file not found".to_string())?;
        if !meta.is_file() {
            return Err("not a file".into());
        }
        if meta.len() > MAX_FILE_BYTES {
            return Err("file is too large to check".into());
        }
        let bytes = fs::read(&real).map_err(|_| "file not found".to_string())?;
        let text = String::from_utf8_lossy(&bytes);
        if is_binary(&text) {
            return Err("binary file".into());
        }
        Ok(Rc::from(text.as_ref()))
    }
}

impl Reader for RepoReader {
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
        is_safe_relative_path(rel) && self.root.join(rel).is_file()
    }
}

/// The files under `root` that git tracks or would track, or `None` when `root` isn't in a git work tree or git
/// lists nothing there.
fn git_files(root: &Path) -> Option<Vec<String>> {
    let out = Command::new("git")
        .args(["ls-files", "-z", "--cached", "--others", "--exclude-standard"])
        .current_dir(root)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let files: Vec<String> =
        String::from_utf8_lossy(&out.stdout).split('\0').filter(|s| !s.is_empty()).map(str::to_string).collect();
    (!files.is_empty()).then_some(files)
}

/// Every non-ignored file under `root`, sorted. In a git repo that is what git tracks or would track, so installed
/// packages and build output a `.gitignore` names are never read; elsewhere the folder is walked.
pub fn list_repo_files<S: AsRef<str>>(root: &Path, ignore: &[S]) -> Vec<String> {
    let ignored = |p: &str| ignore.iter().any(|g| matches_any(&[g.as_ref()], p));
    if let Some(listed) = git_files(root) {
        let mut files: Vec<String> = listed
            .into_iter()
            .filter(|p| {
                !p.split('/').any(|seg| seg == "node_modules" || seg == ".git")
                    && !ignored(p)
                    // Git also lists links, submodules and deleted files; only real files are read.
                    && fs::symlink_metadata(root.join(p)).is_ok_and(|m| m.is_file())
            })
            .collect();
        files.sort();
        return files;
    }
    let mut files = Vec::new();
    let mut stack = vec![String::new()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(if dir.is_empty() { root.to_path_buf() } else { root.join(&dir) }) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == "node_modules" || name == ".git" {
                continue;
            }
            let path = if dir.is_empty() { name } else { format!("{dir}/{name}") };
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if ignored(&path) || (kind.is_dir() && ignored(&format!("{path}/"))) {
                continue;
            }
            if kind.is_dir() {
                stack.push(path);
            } else if kind.is_file() {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}
