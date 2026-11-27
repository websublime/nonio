//! A map of a change, on the server side: the model is told what differs from the last commit, and the commit a map
//! names is pinned. Ported from `packages/server/test/changes.test.ts` (the parts about the head start and making the
//! map; answering on such a map comes with the explainer).

mod common;

use std::fs;
use std::path::Path;

use common::*;
use nonio_server::changes::{changes_since, recent_changes, FileChange, What};
use nonio_server::hints::head_start;
use nonio_server::maker::{make_map, MakeOptions};
use nonio_server::prompts::maker_prompt;
use serde_json::{json, Value};

/// The care app committed, then changed: a listener added, the service edited, the nightly job deleted.
fn changed_app() -> (tempfile::TempDir, String) {
    let dir = copy_care_app();
    let root = dir.path();
    git(root, &["init", "-q"]);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "the care app"]);
    let first = git(root, &["rev-parse", "HEAD"]);
    fs::write(root.join("src/automations/listeners.ts"), "import { bus } from '../events/bus';\nimport { evaluateRules } from './evaluate';\n\nexport function listenForConversions() {\n  bus.on('patient.converted', ({ patientId }) => evaluateRules({ patientId }));\n}\n").unwrap();
    let service = root.join("src/patients/service.ts");
    let text = fs::read_to_string(&service).unwrap().replace(
        "  bus.emit('patient.converted', { patientId });",
        "  console.log('converted', patientId);\n  bus.emit('patient.converted', { patientId });",
    );
    fs::write(&service, text).unwrap();
    fs::remove_file(root.join("src/jobs/nightly.ts")).unwrap();
    fs::write(root.join("NOTES.md"), "a note\n").unwrap();
    (dir, first)
}

/// The care-app map, drawn as a map of that change.
fn change_map(since: &str) -> Value {
    let mut m = care_map_json();
    m["since"] = json!(since);
    m["absences"] = json!([]);
    m["tour"] = json!([]);
    m["nodes"].as_array_mut().unwrap().push(json!({ "id": "l1", "area": "automations", "name": "Hears the announcement", "symbol": "src/automations/listeners.ts#listenForConversions", "evidence": [{ "file": "src/automations/listeners.ts", "line": 5, "text": "bus.on('patient.converted'" }] }));
    m["edges"].as_array_mut().unwrap().push(json!({ "id": "el", "from": "l1", "to": "u1", "kind": "calls", "label": "runs the rules", "evidence": [{ "file": "src/automations/listeners.ts", "line": 5, "text": "evaluateRules({ patientId })" }] }));
    for e in m["edges"].as_array_mut().unwrap() {
        if e["id"] == "e34" {
            e["evidence"][0]["line"] = json!(7);
        }
        if e["id"] == "ej" {
            e["removed"] = json!(true);
        }
    }
    for n in m["nodes"].as_array_mut().unwrap() {
        if n["id"] == "j1" {
            n["removed"] = json!(true);
        }
    }
    m
}

fn change(file: &str, what: What, detail: &str) -> FileChange {
    FileChange { file: file.into(), what, detail: detail.into() }
}

#[test]
fn lists_the_files_code_first_with_the_functions_that_are_new_different_or_gone() {
    let (dir, first) = changed_app();
    let c = changes_since(dir.path(), "HEAD").unwrap();
    assert_eq!((c.commit.as_str(), c.subject.as_deref()), (first.as_str(), Some("the care app")));
    assert_eq!(
        c.files,
        [
            change("src/automations/listeners.ts", What::New, "new: listenForConversions"),
            change("src/jobs/nightly.ts", What::Deleted, "removed: schedule, runNightly"),
            change("src/patients/service.ts", What::Changed, "changed: markConverted"),
            change("NOTES.md", What::New, ""),
        ]
    );
}

#[test]
fn is_none_for_a_name_that_is_no_commit_and_never_lists_secrets_or_nonios_own_files() {
    let (dir, _) = changed_app();
    assert_eq!(changes_since(dir.path(), "nope"), None);
    fs::write(dir.path().join(".env.local"), "KEY=1\n").unwrap();
    fs::create_dir_all(dir.path().join(".nonio/maps")).unwrap();
    fs::write(dir.path().join(".nonio/maps/m.json"), "{}").unwrap();
    let files: Vec<String> = changes_since(dir.path(), "HEAD").unwrap().files.into_iter().map(|f| f.file).collect();
    assert!(!files.iter().any(|f| f == ".env.local" || f.starts_with(".nonio/")), "{files:?}");
}

#[test]
fn gives_the_model_the_work_not_committed_yet_or_else_what_the_last_commit_did() {
    let (dir, _) = changed_app();
    let root = dir.path();
    assert!(recent_changes(root).unwrap().title.contains("a map of these sets `since`: \"HEAD\""));
    assert!(head_start(root).text.contains("  - changed src/patients/service.ts (changed: markConverted)"));
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "listen"]);
    let last = recent_changes(root).unwrap();
    assert!(last.title.contains("`since`: \"HEAD~1\""));
    assert!(last.lines.contains(&"  - new src/automations/listeners.ts (new: listenForConversions)".to_string()));
}

#[test]
fn says_nothing_outside_a_git_repo() {
    let root = copy_care_app();
    assert_eq!(recent_changes(root.path()), None);
    assert!(!head_start(root.path()).text.contains("Not committed yet"));
}

fn make(root: &Path, replies: Vec<Reply>) -> (MakeOptions, std::sync::Arc<ScriptedAgent>) {
    let agent = ScriptedAgent::new(replies);
    (MakeOptions::new(root, "What did that change do?", agent.clone()), agent)
}

#[tokio::test]
async fn pins_the_commit_the_map_names_so_it_shows_the_same_change_after_a_commit() {
    let (dir, first) = changed_app();
    let (o, _) = make(dir.path(), vec![reply(change_map("HEAD"))]);
    let r = make_map(o).await.unwrap();
    assert_eq!(r.map.since.as_deref(), Some(first.as_str()));
    assert_eq!(r.report.summary.unconfirmed, 0);
    let c = r.report.summary.changes.unwrap();
    assert_eq!((c.added, c.changed, c.removed), (2, 2, 2));
    assert_eq!(r.rounds, 1);
}

#[tokio::test]
async fn sends_back_a_starting_point_that_is_no_commit() {
    let (dir, first) = changed_app();
    let (o, agent) = make(dir.path(), vec![reply(change_map("the-old-branch")), reply(change_map("HEAD"))]);
    let r = make_map(o).await.unwrap();
    assert!(agent.call(1).prompt.contains("\"the-old-branch\" isn't a commit in this repo"));
    assert_eq!(r.map.since.as_deref(), Some(first.as_str()));
}

#[tokio::test]
async fn sends_back_code_marked_removed_that_is_still_there() {
    let (dir, _) = changed_app();
    let mut wrong = change_map("HEAD");
    let mut fixed_node = Value::Null;
    let mut fixed_edge = Value::Null;
    for n in wrong["nodes"].as_array_mut().unwrap() {
        if n["id"] == "u3" {
            fixed_node = n.clone();
            n["removed"] = json!(true);
        }
    }
    for e in wrong["edges"].as_array_mut().unwrap() {
        if e["id"] == "eu23" {
            fixed_edge = e.clone();
            e["removed"] = json!(true);
        }
    }
    fixed_node["removed"] = json!(false);
    fixed_edge["removed"] = json!(false);
    let (o, agent) =
        make(dir.path(), vec![reply(wrong), reply(json!({ "nodes": [fixed_node], "edges": [fixed_edge] }))]);
    let r = make_map(o).await.unwrap();
    assert!(agent.call(1).prompt.contains("still in the code"), "{}", agent.call(1).prompt);
    assert_eq!(r.report.summary.unconfirmed, 0);
}

#[test]
fn tells_the_model_how_to_draw_a_change() {
    let p = maker_prompt("What did you just change?", "(head start)");
    assert!(p.contains("## When the question is about a change"));
    assert!(p.contains("`\"removed\": true`"));
}
