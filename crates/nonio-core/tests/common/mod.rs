//! Shared by the ported tests: the care-app fixture, recorded facts, temporary copies and git.
#![allow(dead_code)]

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use nonio_core::format::{parse_map, CodeMap};
use nonio_facts::{Extractor, FileFacts, RepoView};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// The care-app fixture repo.
pub fn app() -> PathBuf {
    fixtures().join("care-app")
}

pub fn json(name: &str) -> Value {
    serde_json::from_str(&fs::read_to_string(fixtures().join(name)).unwrap()).unwrap()
}

/// The hand-written care-app map, as JSON.
pub fn map_json() -> Value {
    json("care-app.map.json")
}

pub fn journey_json() -> Value {
    json("care-app.journey.json")
}

pub fn load(v: &Value) -> CodeMap {
    parse_map(v).unwrap_or_else(|p| panic!("{}", p.join("\n")))
}

/// A copy of the fixture map, changed by `edit`.
pub fn variant(edit: impl FnOnce(&mut Value)) -> CodeMap {
    let mut m = map_json();
    edit(&mut m);
    load(&m)
}

/// The item of `list` (`nodes`, `edges`, ...) with this id.
pub fn item<'a>(m: &'a mut Value, list: &str, id: &str) -> &'a mut Value {
    m[list].as_array_mut().unwrap().iter_mut().find(|x| x["id"] == id).unwrap_or_else(|| panic!("no {list} {id}"))
}

/// Facts recorded from illuminate's own extractor while its core tests ran (see tools/record-facts). Keyed by path
/// and the SHA-256 of the content, so a test that changes a file finds the facts of exactly that text. A file the
/// recording doesn't have fails the test: the Rust tests must read the same code the original tests read.
pub struct Recorded {
    by_file: HashMap<String, HashMap<String, FileFacts>>,
}

impl Recorded {
    pub fn load() -> Self {
        let text = fs::read_to_string(fixtures().join("recorded-facts.json")).unwrap();
        Self { by_file: serde_json::from_str(&text).unwrap() }
    }
}

impl Extractor for Recorded {
    fn extract(&self, _repo: &dyn RepoView, path: &str, source: &str) -> Option<FileFacts> {
        let hash: String = Sha256::digest(source.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
        let facts = self.by_file.get(path).and_then(|v| v.get(&hash));
        Some(facts.unwrap_or_else(|| panic!("no recorded facts for {path} at {hash}")).clone())
    }
}

pub fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// A fresh copy of the care app, outside any git repo. Removed when the guard drops.
pub fn copy_app() -> tempfile::TempDir {
    let dir = tempfile::Builder::new().prefix("nonio-care-").tempdir().unwrap();
    copy_dir(&app(), dir.path());
    dir
}

pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

pub fn write(dir: &Path, rel: &str, text: &str) {
    fs::write(dir.join(rel), text).unwrap();
}

pub fn read(dir: &Path, rel: &str) -> String {
    fs::read_to_string(dir.join(rel)).unwrap()
}

/// Check a map against `root` with the recorded facts.
pub fn run(map: &CodeMap, root: &Path) -> nonio_core::CheckReport {
    nonio_core::check(map, root, &Recorded::load(), &nonio_core::CheckOptions::default()).unwrap()
}
