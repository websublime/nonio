//! Changing a map after it's made: additions are validated like a new map; closed gaps drop out of the tour too.
//! Ported from `packages/core/test/edit.test.ts` of illuminate (MIT, Copyright (c) 2026 johnz4021).

mod common;

use std::collections::{BTreeMap, HashSet};

use common::*;
use nonio_core::edit::{
    add_to_map, fresh_id, patch_map, remove_absences, remove_from_map, slugify, MapAdditions, MapPatch,
};
use nonio_core::format::{parse_map, CodeMap};
use serde_json::{json, Value};

fn map() -> CodeMap {
    load(&map_json())
}

fn node_json(map: &CodeMap, id: &str) -> Value {
    serde_json::to_value(map.nodes.iter().find(|n| n.id == id).unwrap()).unwrap()
}

#[test]
fn adds_an_area_a_box_and_an_arrow_found_while_answering() {
    let r = add_to_map(
        &map(),
        &MapAdditions {
            nodes: vec![json!({ "id": "n9", "area": "jobs", "name": "Filter to yesterday", "evidence": [{ "file": "src/jobs/nightly.ts", "line": 9, "text": "createdAfter" }] })],
            edges: vec![json!({ "id": "e9", "from": "j1", "to": "n9", "label": "then", "evidence": [{ "file": "src/jobs/nightly.ts", "line": 9, "text": "db.where" }] })],
            ..Default::default()
        },
    )
    .unwrap();
    assert!(r.nodes.iter().any(|n| n.id == "n9"));
}

#[test]
fn rejects_additions_that_clash_with_existing_ids_or_point_at_nothing() {
    let r = add_to_map(
        &map(),
        &MapAdditions {
            nodes: vec![
                json!({ "id": "s1", "area": "patients", "name": "Again", "evidence": [{ "file": "a.ts", "line": 1, "text": "x" }] }),
            ],
            edges: vec![
                json!({ "id": "e10", "from": "u1", "to": "ghost", "label": "calls", "evidence": [{ "file": "a.ts", "line": 1, "text": "x" }] }),
            ],
            ..Default::default()
        },
    );
    assert_eq!(r.unwrap_err(), vec!["node id \"s1\" is used twice", "edge \"e10\": unknown node \"ghost\""]);
}

#[test]
fn drops_a_closed_gap_from_the_map_and_from_tour_stops_that_pointed_at_it() {
    let m = remove_absences(&map(), &["miss".to_string()]);
    assert!(m.absences.is_empty());
    assert_eq!(m.tour.iter().find(|t| t.id == "t5").unwrap().focus.absences, Some(vec![]));
    assert!(parse_map(&m.to_value()).is_ok());
}

#[test]
fn picks_the_next_free_id_and_slugs_questions() {
    assert_eq!(fresh_id(&map(), "e"), "e1");
    assert_eq!(fresh_id(&map(), "s"), "s5");
    assert_eq!(
        slugify("What happens when a patient converts? Does the welcome email go out?"),
        "patient-converts-welcome-email"
    );
    assert_eq!(slugify("???"), "map");
}

#[test]
fn replaces_items_by_id_in_place_adds_new_ones_and_leaves_the_rest_alone() {
    let base = map();
    let mut u2 = node_json(&base, "u2");
    u2["name"] = json!("Email waits in a queue");
    let r = patch_map(
        &base,
        &MapPatch { nodes: vec![u2, json!({ "id": "n9", "area": "jobs", "name": "New step", "evidence": [{ "file": "a.ts", "line": 1, "text": "x" }] })], ..Default::default() },
    )
    .unwrap();
    let mut ids: Vec<&str> = base.nodes.iter().map(|n| n.id.as_str()).collect();
    ids.push("n9");
    assert_eq!(r.nodes.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), ids);
    assert_eq!(r.nodes.iter().find(|n| n.id == "u2").unwrap().name, "Email waits in a queue");
    assert_eq!(CodeMap { nodes: vec![], ..r.clone() }, CodeMap { nodes: vec![], ..base });
}

#[test]
fn drops_removed_ids_with_the_arrows_and_tour_mentions_that_point_at_them() {
    let r = patch_map(&map(), &MapPatch { remove: vec!["u3".into()], ..Default::default() }).unwrap();
    assert!(!r.nodes.iter().any(|n| n.id == "u3"));
    assert!(!r.edges.iter().any(|e| e.from == "u3" || e.to == "u3"));
    assert!(r.tour.iter().all(|t| !t.focus.nodes.iter().flatten().any(|id| id == "u3")));
}

#[test]
fn refuses_changes_that_leave_the_map_broken() {
    let base = map();
    let mut first = serde_json::to_value(&base.edges[0]).unwrap();
    first["to"] = json!("nowhere");
    let r = patch_map(&base, &MapPatch { edges: vec![first], ..Default::default() });
    assert_eq!(r.unwrap_err(), vec![format!("edge \"{}\": unknown node \"nowhere\"", base.edges[0].id)]);
}

fn branches(m: &CodeMap) -> BTreeMap<String, String> {
    m.nodes
        .iter()
        .filter_map(|n| n.branch.clone().map(|b| (n.id.clone(), b)))
        .chain(m.edges.iter().filter_map(|e| e.branch.clone().map(|b| (e.id.clone(), b))))
        .collect()
}

#[test]
fn keeps_the_branches_of_a_journey_through_additions_patches_and_removals() {
    let journey = load(&journey_json());
    let added = add_to_map(
        &journey,
        &MapAdditions {
            nodes: vec![json!({ "id": "j7", "area": "page", "kind": "action", "name": "Opens settings", "branch": "Goes back to the lists", "evidence": [{ "file": "src/ui/PatientSidebar.tsx", "line": 7, "text": "Settings" }] })],
            edges: vec![json!({ "id": "jg", "from": "j1", "to": "j7", "label": "clicks Settings", "branch": "Goes back to the lists", "evidence": [{ "file": "src/ui/PatientSidebar.tsx", "line": 7, "text": "href=\"/settings\"" }] })],
            ..Default::default()
        },
    )
    .unwrap();
    let mut want = branches(&journey);
    want.insert("j7".into(), "Goes back to the lists".into());
    want.insert("jg".into(), "Goes back to the lists".into());
    assert_eq!(branches(&added), want);
    let mut j4 = node_json(&journey, "j4");
    j4["name"] = json!("Saves the note");
    let patched =
        patch_map(&added, &MapPatch { nodes: vec![j4], remove: vec!["j5".into()], ..Default::default() }).unwrap();
    let mut rest = branches(&added);
    rest.remove("j5");
    rest.remove("je");
    assert_eq!(branches(&patched), rest);
}

#[test]
fn changes_nothing_when_there_is_nothing_to_change() {
    let base = map();
    assert_eq!(patch_map(&base, &MapPatch::default()).unwrap(), base);
    assert_eq!(remove_from_map(&base, &HashSet::new()), base);
}
