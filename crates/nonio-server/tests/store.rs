//! The map store. The original's only store tests are about moving its older `.codemap/` folder, which nonio has no
//! counterpart for; these cover what nonio's store does instead, and that it leaves the original's folder alone.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use nonio_core::parse_map;
use nonio_server::store::{FixStatus, MapMeta, SavedFix, Store, StoredMap};
use serde_json::{json, Value};

fn care_map() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../nonio-core/tests/fixtures/care-app.map.json");
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn meta(id: &str, updated: &str) -> MapMeta {
    MapMeta {
        id: id.into(),
        question: format!("{id}?"),
        created_at: "2026-11-27T10:00:00Z".into(),
        updated_at: updated.into(),
        commit: None,
        maker_session: None,
        explainer_session: None,
        cost: 0.0,
        parent: None,
    }
}

fn stored(id: &str, updated: &str) -> StoredMap {
    StoredMap {
        meta: meta(id, updated),
        map: parse_map(&care_map()).unwrap(),
        answers: vec![json!({ "id": "a1", "question": "Why?", "added": [] })],
        fixes: [("f1".to_string(), SavedFix { status: FixStatus::Proposed, edge_id: Some("e9".into()), closed: None })]
            .into_iter()
            .collect(),
    }
}

#[test]
fn saves_under_dot_nonio_and_reads_back_what_it_saved() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path());
    store.save(&stored("welcome-email", "2026-11-27T11:00:00Z")).unwrap();
    assert!(root.path().join(".nonio/maps/welcome-email.json").exists());
    assert_eq!(fs::read_to_string(root.path().join(".nonio/.gitignore")).unwrap(), "fixes/\nws-modules/\n");
    assert_eq!(store.get("welcome-email"), Some(stored("welcome-email", "2026-11-27T11:00:00Z")));
    assert!(!root.path().join(".nonio/maps/welcome-email.json.tmp").exists());
}

#[test]
fn lists_the_newest_first_and_skips_files_that_are_not_maps() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path());
    assert!(store.list().is_empty());
    store.save(&stored("older", "2026-11-27T11:00:00Z")).unwrap();
    store.save(&stored("newer", "2026-11-27T12:00:00Z")).unwrap();
    fs::write(store.dir().join("broken.json"), "{").unwrap();
    fs::write(store.dir().join("notes.txt"), "x").unwrap();
    assert_eq!(store.list().iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["newer", "older"]);
}

#[test]
fn loses_only_what_does_not_look_right_in_a_hand_edited_file() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path());
    fs::create_dir_all(store.dir()).unwrap();
    let raw = json!({
        "meta": meta("edited", "x"),
        "map": care_map(),
        "answers": [{ "id": "a1", "question": "Why?", "added": [] }, { "id": 3 }],
        "fixes": { "f1": { "status": "applied" }, "f2": { "status": "maybe" } },
    });
    fs::write(store.dir().join("edited.json"), raw.to_string()).unwrap();
    let got = store.get("edited").unwrap();
    assert_eq!(got.answers.len(), 1);
    assert_eq!(got.fixes.keys().collect::<Vec<_>>(), ["f1"]);

    // A map saved before answers were kept has neither.
    fs::write(store.dir().join("old.json"), json!({ "meta": meta("old", "x"), "map": care_map() }).to_string())
        .unwrap();
    let old = store.get("old").unwrap();
    assert!(old.answers.is_empty() && old.fixes.is_empty());

    // A map that doesn't parse isn't a map.
    fs::write(store.dir().join("bad.json"), json!({ "meta": meta("bad", "x"), "map": {} }).to_string()).unwrap();
    assert_eq!(store.get("bad"), None);
}

#[test]
fn refuses_ids_that_are_not_map_ids() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path());
    assert_eq!(store.get("../secret"), None);
    assert!(store.save(&stored("../x", "x")).is_err());
    assert!(store.save(&stored(&"a".repeat(81), "x")).is_err());
}

#[test]
fn hands_out_ids_unique_among_saved_maps_and_ids_already_taken() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path());
    let q = "What happens when a patient converts?";
    assert_eq!(store.new_id(q, &HashSet::new()), "patient-converts");
    store.save(&stored("patient-converts", "x")).unwrap();
    let taken: HashSet<String> = ["patient-converts-2".to_string()].into();
    assert_eq!(store.new_id(q, &taken), "patient-converts-3");
}

#[test]
fn leaves_the_original_tools_folder_alone() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join(".illuminate/maps")).unwrap();
    fs::write(root.path().join(".illuminate/maps/old.json"), "{}").unwrap();
    let store = Store::new(root.path());
    store.save(&stored("new", "x")).unwrap();
    assert!(root.path().join(".illuminate/maps/old.json").exists());
    assert_eq!(store.list().iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["new"]);
}
