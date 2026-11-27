//! Fixes: proposed, applied only on approval, connected on the map, undone. Ported from the `fixes` tests in
//! `packages/server/test/workflows.test.ts`.

mod common;

use std::fs;

use common::*;
use nonio_server::fixes::{apply_fix, connect_after_fix, propose_fix, undo_fix, RawFix};
use serde_json::json;

fn raw(v: serde_json::Value) -> RawFix {
    serde_json::from_value(v).unwrap()
}

#[test]
fn applies_an_approved_fix_draws_the_new_connection_closes_the_gap_and_undoes_cleanly() {
    let dir = copy_care_app();
    let root = dir.path();
    let map = care_map();
    let p = propose_fix(root, &map, &raw(listener_fix())).unwrap_or_else(|p| panic!("{}", p.join("\n")));
    let a = apply_fix(root, &p).unwrap_or_else(|p| panic!("{}", p.join("\n")));
    let c = connect_after_fix(root, &map, &p);
    assert_eq!(c.edge_id.as_deref(), Some("fx1"));
    assert_eq!(c.closed, vec!["miss".to_string()]);
    let v = c.map.to_value();
    let edge = v["edges"].as_array().unwrap().iter().find(|e| e["id"] == "fx1").unwrap();
    assert_eq!(edge["from"], "s4");
    assert_eq!(edge["to"], "u1");
    assert_eq!(edge["evidence"][0]["file"], "src/automations/listeners.ts");
    assert_eq!(edge["evidence"][0]["line"], 4);
    assert_eq!(undo_fix(root, &a), Ok(()));
    assert!(!root.join("src/automations/listeners.ts").exists());
}

#[test]
fn edits_by_exact_replacement_and_refuses_if_the_file_changed_after_the_proposal() {
    let dir = copy_care_app();
    let root = dir.path();
    let fix = raw(json!({
        "summary": "Evaluate rules right after announcing.",
        "edits": [{ "file": "src/patients/service.ts", "find": "  bus.emit('patient.converted', { patientId });", "replace": "  bus.emit('patient.converted', { patientId });\n  await evaluateRules({ patientId });" }],
    }));
    let p = propose_fix(root, &care_map(), &fix).unwrap_or_else(|p| panic!("{}", p.join("\n")));
    assert!(p.diff.contains("src/patients/service.ts · line 6"), "{}", p.diff);
    let service = root.join("src/patients/service.ts");
    let text = fs::read_to_string(&service).unwrap();
    fs::write(&service, format!("{text}// touched\n")).unwrap();
    assert_eq!(
        apply_fix(root, &p).map(|_| ()),
        Err(vec!["src/patients/service.ts changed since this fix was proposed; ask for the fix again".to_string()])
    );
}

#[test]
fn rejects_fixes_that_touch_secrets_leave_the_repo_replace_ambiguous_text_or_overwrite_files() {
    let dir = copy_care_app();
    let fix = raw(json!({
        "summary": "bad",
        "edits": [
            { "file": ".env", "find": "EMAIL_API_KEY", "replace": "X" },
            { "file": "../outside.ts", "find": "a", "replace": "b" },
            { "file": "src/db.ts", "find": "async", "replace": "sync" },
        ],
        "creates": [{ "file": "src/db.ts", "content": "" }, { "file": ".nonio/maps/x.json", "content": "{}" }],
    }));
    assert_eq!(
        propose_fix(dir.path(), &care_map(), &fix),
        Err(vec![
            ".env: nonio never changes this file".to_string(),
            "../outside.ts: must be a path inside the repo".to_string(),
            "src/db.ts: the text to replace appears 2 times (it must appear exactly once)".to_string(),
            "src/db.ts: already exists".to_string(),
            ".nonio/maps/x.json: nonio never changes this file".to_string(),
        ])
    );
}

#[test]
fn refuses_a_connection_whose_evidence_isnt_in_the_new_code() {
    let dir = copy_care_app();
    let mut fix = listener_fix();
    fix["connects"]["evidenceText"] = json!("bus.on(\"something.else\"");
    assert!(propose_fix(dir.path(), &care_map(), &raw(fix)).is_err());
}
