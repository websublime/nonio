//! The map format: shape validation, defaults, and references between areas, boxes, arrows, absences and tour stops.
//! Ported from `packages/core/test/format.test.ts` of illuminate (MIT, Copyright (c) 2026 johnz4021).

mod common;

use common::*;
use nonio_core::format::{is_safe_relative_path, parse_map, NodeKind, View};
use serde_json::{json, Value};

fn minimal() -> Value {
    json!({
        "version": 1,
        "question": "What happens when staff convert a patient?",
        "areas": [{ "id": "patients", "label": "Patients" }],
        "nodes": [
            { "id": "a", "area": "patients", "name": "Click", "evidence": [{ "file": "src/ui.tsx", "line": 3, "text": "onClick" }] },
            { "id": "b", "area": "patients", "name": "Save", "evidence": [{ "file": "src/save.ts", "line": 1, "text": "save(" }] },
        ],
    })
}

#[test]
fn accepts_a_minimal_map_and_fills_defaults() {
    let map = parse_map(&minimal()).unwrap();
    assert_eq!(map.nodes[0].kind, NodeKind::Step);
    assert!(map.edges.is_empty() && map.absences.is_empty() && map.tour.is_empty());
}

#[test]
fn reports_shape_problems_with_their_paths() {
    let mut m = minimal();
    m["version"] = json!(2);
    m["nodes"][0]["evidence"] = json!([]);
    m["nodes"][1]["symbol"] = json!("no-hash");
    let problems = parse_map(&m).unwrap_err();
    assert!(problems.iter().any(|p| p.starts_with("version: ")), "{problems:?}");
    assert!(problems.contains(&"nodes.1.symbol: symbol is <file>#<name>".to_string()), "{problems:?}");
}

#[test]
fn reports_broken_references_and_duplicate_ids() {
    let mut m = minimal();
    m["nodes"].as_array_mut().unwrap().push(json!({ "id": "a", "area": "billing", "name": "Dup", "evidence": [{ "file": "x.ts", "line": 1, "text": "x" }] }));
    m["edges"] = json!([{ "id": "e1", "from": "a", "to": "zzz", "label": "calls", "evidence": [{ "file": "x.ts", "line": 1, "text": "x" }] }]);
    m["absences"] = json!([{ "id": "g", "claim": "none", "between": { "from": "b", "to": "nope" }, "search": { "pattern": "x" }, "matches": 0 }]);
    m["tour"] =
        json!([{ "id": "t", "title": "Start", "text": "Hi", "focus": { "edges": ["e9"], "areas": ["patients"] } }]);
    assert_eq!(
        parse_map(&m).unwrap_err(),
        vec![
            "node id \"a\" is used twice",
            "node \"a\": unknown area \"billing\"",
            "edge \"e1\": unknown node \"zzz\"",
            "absence \"g\": unknown node \"nope\"",
            "tour stop \"t\": unknown edge \"e9\"",
        ]
    );
}

#[test]
fn safe_relative_paths_cannot_leave_the_repo() {
    assert!(["src/a.ts", "a.ts", ".github/workflows/ci.yml"].iter().all(|p| is_safe_relative_path(p)));
    assert!(!["/etc/passwd", "../x.ts", "src/../../x", "C:/x", "src\\x.ts", "src//x.ts", ""]
        .iter()
        .any(|p| is_safe_relative_path(p)));
}

fn branches_of(nodes: &[nonio_core::format::MapNode]) -> Vec<Option<&str>> {
    nodes.iter().map(|n| n.branch.as_deref()).collect()
}

#[test]
fn a_journey_is_the_same_format_with_a_view_and_its_own_box_kinds() {
    let map = load(&journey_json());
    assert_eq!(map.view, Some(View::Journey));
    use NodeKind::*;
    assert_eq!(
        map.nodes.iter().map(|n| n.kind).collect::<Vec<_>>(),
        vec![Screen, Action, External, Action, Action, External]
    );
}

#[test]
fn a_journey_keeps_the_branch_each_box_and_arrow_is_on() {
    let map = load(&journey_json());
    assert_eq!(
        branches_of(&map.nodes),
        vec![None, None, None, Some("Writes a note instead"), Some("Goes back to the lists"), Some("No welcome rule")]
    );
    assert_eq!(map.edges.iter().find(|e| e.id == "jd").unwrap().branch.as_deref(), Some("Writes a note instead"));
    assert_eq!(map.edges.iter().find(|e| e.id == "ja").unwrap().branch, None);
    // Parsing what was parsed gives the same map back.
    assert_eq!(parse_map(&map.to_value()).unwrap(), map);
}

#[test]
fn any_map_can_name_its_ways_through_a_proposals_options_included() {
    let cites = json!([{ "file": "src/a.ts", "line": 1, "text": "x" }]);
    let map = load(&json!({
        "version": 1,
        "question": "How would (a) or (b) change it?",
        "areas": [{ "id": "x", "label": "Map rules" }],
        "nodes": [
            { "id": "rules", "area": "x", "name": "Rules", "evidence": cites },
            { "id": "a", "area": "x", "name": "Wrapper rule", "branch": "A: prompt rule", "proposed": { "kind": "add" } },
            { "id": "b", "area": "x", "name": "Called from list", "branch": "B: computed list", "proposed": { "kind": "add" } },
        ],
        "edges": [
            { "id": "ea", "from": "rules", "to": "a", "kind": "then", "label": "gains", "branch": "A: prompt rule", "proposed": { "kind": "add" } },
            { "id": "eb", "from": "rules", "to": "b", "kind": "then", "label": "feeds", "branch": "B: computed list", "proposed": { "kind": "add" } },
        ],
    }));
    assert_eq!(map.view, None);
    assert_eq!(branches_of(&map.nodes), vec![None, Some("A: prompt rule"), Some("B: computed list")]);
    assert_eq!(
        map.edges.iter().map(|e| e.branch.as_deref()).collect::<Vec<_>>(),
        vec![Some("A: prompt rule"), Some("B: computed list")]
    );
}

#[test]
fn a_branch_is_named_in_a_few_words() {
    let raw = journey_json();
    let named = |branch: Value| {
        let mut m = raw.clone();
        item(&mut m, "nodes", "j4")["branch"] = branch;
        parse_map(&m)
    };
    assert!(named(json!("Saves for later")).is_ok());
    for bad in [json!(""), json!("   "), json!("x".repeat(61)), json!(3)] {
        assert!(named(bad.clone()).is_err(), "{bad}");
    }
    let mut long = raw.clone();
    item(&mut long, "edges", "jc")["branch"] = json!("y".repeat(61));
    let problems = parse_map(&long).unwrap_err();
    assert_eq!(problems.len(), 1);
    assert!(problems[0].starts_with("edges.2.branch: "), "{problems:?}");
}

#[test]
fn still_reads_a_journey_written_before_branches() {
    let mut old = journey_json();
    let strip = |v: &mut Value| {
        v.as_object_mut().unwrap().remove("branch");
    };
    old["nodes"].as_array_mut().unwrap().truncate(3);
    old["nodes"].as_array_mut().unwrap().iter_mut().for_each(strip);
    old["edges"].as_array_mut().unwrap().truncate(2);
    old["tour"].as_array_mut().unwrap().truncate(2);
    let map = load(&old);
    assert!(map.nodes.iter().all(|n| n.branch.is_none()) && map.edges.iter().all(|e| e.branch.is_none()));
}

#[test]
fn rejects_a_view_it_does_not_know() {
    let mut m = journey_json();
    m["view"] = json!("poster");
    assert!(parse_map(&m).is_err());
}

#[test]
fn a_timeline_is_the_same_format_and_no_other_view_is_accepted() {
    let mut m = minimal();
    m["view"] = json!("timeline");
    assert_eq!(parse_map(&m).unwrap().view, Some(View::Timeline));
    m["view"] = json!("swimlanes");
    assert!(parse_map(&m).is_err());
}

#[test]
fn reads_the_original_care_app_map() {
    // D22: a single-root map is the original's format.
    let map = load(&map_json());
    assert_eq!(map.nodes.len(), 8);
    assert_eq!(parse_map(&map.to_value()).unwrap(), map);
}
