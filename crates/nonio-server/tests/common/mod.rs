//! Shared by the server tests: the care-app fixture and throwaway copies of it.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use nonio_core::{check, parse_map, CheckOptions, CheckReport, CodeMap};
use nonio_scan::Scanner;
use serde_json::Value;

pub fn core_fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../nonio-core/tests/fixtures")
}

pub fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

pub fn care_map_json() -> Value {
    serde_json::from_str(&fs::read_to_string(core_fixtures().join("care-app.map.json")).unwrap()).unwrap()
}

pub fn care_map() -> CodeMap {
    parse_map(&care_map_json()).unwrap()
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap().flatten() {
        let target = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &target);
        } else {
            fs::copy(e.path(), target).unwrap();
        }
    }
}

/// A copy of the care app outside any git repo, so nothing nonio's own checkout holds leaks in.
pub fn copy_care_app() -> tempfile::TempDir {
    let dir = tempfile::Builder::new().prefix("nonio-server-").tempdir().unwrap();
    copy_dir(&core_fixtures().join("care-app"), dir.path());
    dir
}

/// The checker's report on `map`, with facts from nonio's own backends.
pub fn check_map(map: &CodeMap, root: &Path) -> CheckReport {
    let files = nonio_core::repo::list_repo_files(root, nonio_core::repo::DEFAULT_IGNORE);
    let scanner = Scanner::new(root, files);
    check(map, root, &scanner, &CheckOptions::default()).unwrap()
}

use std::sync::{Arc, Mutex};

use nonio_server::bridge::{
    Agent, AgentError, AgentRequest, AgentResponse, AgentResult, BoxFuture, Progress, ProgressKind,
};

pub type Reply = Box<dyn Fn(&AgentRequest) -> Value + Send + Sync>;

/// A stand-in for the host's model that replies from a script, in order, and records every request.
pub struct ScriptedAgent {
    replies: Vec<Reply>,
    pub calls: Mutex<Vec<AgentRequest>>,
}

impl ScriptedAgent {
    pub fn new(replies: Vec<Reply>) -> Arc<Self> {
        Arc::new(ScriptedAgent { replies, calls: Mutex::new(Vec::new()) })
    }
    pub fn call(&self, i: usize) -> AgentRequest {
        self.calls.lock().unwrap()[i].clone()
    }
    pub fn count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

/// A reply that is always `v`.
pub fn reply(v: Value) -> Reply {
    Box::new(move |_| v.clone())
}

impl Agent for ScriptedAgent {
    fn ask(&self, req: AgentRequest) -> BoxFuture<AgentResult> {
        let n = {
            let mut calls = self.calls.lock().unwrap();
            calls.push(req.clone());
            calls.len()
        };
        let result = match self.replies.get(n - 1) {
            None => Err(AgentError::Failed(format!("no scripted reply for call {n}"))),
            Some(r) => {
                if let Some(f) = &req.on_progress {
                    f(Progress { kind: ProgressKind::Read, text: "Reading src/patients/service.ts".into() });
                }
                Ok(AgentResponse { output: r(&req), session_id: Some(format!("session-{n}")), cost: Some(0.01) })
            }
        };
        Box::pin(async move { result })
    }
}

pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}
