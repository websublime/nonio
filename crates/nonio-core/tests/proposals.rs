//! Proposals: what a map suggests adding, changing or removing, drawn on top of the code as it is. The map says what
//! is proposed; where each proposal stands is computed from the repo, like every other status.
//! Ported from `packages/core/test/proposals.test.ts` of illuminate (MIT, Copyright (c) 2026 johnz4021).

mod common;

use std::fs;

use common::*;
use nonio_core::check::{ProposalCheck, ProposalCount, ProposalState, Summary, TourCheck, TourStatus};
use nonio_core::format::{parse_map, ProposalKind};
use nonio_core::screens::{ScreenShow, ScreenStatus};
use nonio_core::{settled_proposals, ClaimCheck, Proof, Status};
use serde_json::{json, Value};

/// The care-app map plus a proposal: listen for the announcement, and stop relying on the nightly job.
fn proposal(edit: impl FnOnce(&mut Value)) -> Value {
    let mut m = map_json();
    m["nodes"].as_array_mut().unwrap().push(json!({
        "id": "pl", "area": "automations", "name": "Listen for conversions",
        "symbol": "src/automations/listeners.ts#onConverted", "proposed": { "kind": "add", "note": "Run the rules as soon as a patient converts." },
    }));
    m["edges"].as_array_mut().unwrap().push(json!({ "id": "p1", "from": "s4", "to": "pl", "kind": "then", "label": "is heard by", "proposed": { "kind": "add" } }));
    m["edges"].as_array_mut().unwrap().push(json!({ "id": "p2", "from": "pl", "to": "u1", "kind": "calls", "label": "runs the rules", "proposed": { "kind": "add" } }));
    item(&mut m, "nodes", "j1")["proposed"] =
        json!({ "kind": "remove", "note": "No longer needed once the listener exists." });
    item(&mut m, "edges", "ej")["proposed"] = json!({ "kind": "remove" });
    edit(&mut m);
    m
}

const LISTENER: &str = "import { bus } from '../events/bus';\nimport { evaluateRules } from './evaluate';\n\nexport function onConverted({ patientId }: { patientId: string }) {\n  return evaluateRules({ patientId });\n}\nbus.on('patient.converted', onConverted);\n";

fn is(c: &ClaimCheck, status: Status, kind: ProposalKind, state: ProposalState) {
    assert_eq!((c.status, c.proposal), (status, Some(ProposalCheck { kind, state })), "{c:?}");
}

#[test]
fn a_proposed_addition_may_cite_nothing_and_nothing_else_may() {
    assert!(parse_map(&proposal(|_| {})).is_ok());
    let r = parse_map(&proposal(|m| item(m, "nodes", "j1")["evidence"] = json!([])));
    assert_eq!(
        r.unwrap_err(),
        vec!["node \"j1\": cite at least one line of code in `evidence` (only a proposed addition goes without)"]
    );
}

#[test]
fn an_arrow_to_or_from_a_proposed_box_is_a_proposal_too() {
    let r = parse_map(&proposal(|m| {
        let p2 = item(m, "edges", "p2");
        p2.as_object_mut().unwrap().remove("proposed");
        p2["evidence"] = json!([{ "file": "src/db.ts", "line": 1, "text": "x" }]);
    }));
    let p = r.unwrap_err();
    assert!(
        p[0].contains("edge \"p2\": box \"pl\" is a proposed addition, so an arrow to or from it must be one too"),
        "{p:?}"
    );
}

#[test]
fn refuses_a_proposed_box_that_connects_to_nothing_in_the_code() {
    let r = parse_map(&proposal(|m| m["edges"].as_array_mut().unwrap().retain(|e| e["id"] != "p1" && e["id"] != "p2")));
    assert_eq!(
        r.unwrap_err(),
        vec!["node \"pl\": a proposed box must connect, through arrows, to a box that exists in the code"]
    );
    // Two proposed boxes that only connect to each other aren't anchored either.
    let island = parse_map(&proposal(|m| {
        m["edges"].as_array_mut().unwrap().retain(|e| e["id"] != "p1" && e["id"] != "p2");
        m["nodes"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "id": "pm", "area": "patients", "name": "Another new step", "proposed": { "kind": "add" } }));
        m["edges"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "id": "p3", "from": "pl", "to": "pm", "label": "then", "proposed": { "kind": "add" } }));
    }));
    assert!(island.is_err());
}

#[test]
fn keeps_proposed_screen_elements_to_boxes_that_carry_a_proposal() {
    let r = parse_map(&proposal(|m| {
        item(m, "nodes", "s2")["screen"] = json!({ "kind": "sketch", "file": "src/ui/PatientHeader.tsx", "elements": [{ "kind": "button", "text": "Archive", "proposed": true }] });
    }));
    assert_eq!(
        r.unwrap_err(),
        vec!["node \"s2\": only a box that carries `proposed` can have proposed screen elements"]
    );
}

#[test]
fn is_pending_while_the_code_is_as_it_was() {
    let report = run(&load(&proposal(|_| {})), &app());
    use ProposalKind::*;
    use ProposalState::*;
    is(&report.nodes["pl"], Status::Proposed, Add, Pending);
    is(&report.edges["p1"], Status::Proposed, Add, Pending);
    is(&report.edges["p2"], Status::Proposed, Add, Pending);
    // What would be removed is still real code, and is checked as such.
    is(&report.nodes["j1"], Status::Confirmed, Remove, Pending);
    assert_eq!(report.nodes["j1"].proof, Some(Proof::Fact));
    is(&report.edges["ej"], Status::Confirmed, Remove, Pending);
    assert_eq!(
        report.summary,
        Summary {
            claims: 15,
            confirmed: 15,
            unconfirmed: 0,
            proposals: ProposalCount { pending: 5, done: 0 },
            changes: None
        }
    );
    assert!(settled_proposals(&report).is_empty());
}

#[test]
fn is_done_once_the_code_has_the_addition_and_no_longer_has_what_was_to_go() {
    let root = copy_app();
    write(root.path(), "src/automations/listeners.ts", LISTENER);
    fs::remove_file(root.path().join("src/jobs/nightly.ts")).unwrap();
    let report = run(&load(&proposal(|_| {})), root.path());
    use ProposalKind::*;
    use ProposalState::*;
    is(&report.nodes["pl"], Status::Confirmed, Add, Done);
    assert_eq!(report.nodes["pl"].proof, Some(Proof::Fact));
    is(&report.edges["p2"], Status::Confirmed, Add, Done);
    assert_eq!(report.edges["p2"].proof, Some(Proof::Fact));
    // "is heard by" isn't a call, request or import, so the facts can't show it was built: it stays proposed.
    is(&report.edges["p1"], Status::Proposed, Add, Pending);
    is(&report.nodes["j1"], Status::Proposed, Remove, Done);
    is(&report.edges["ej"], Status::Proposed, Remove, Done);
    assert_eq!(report.summary.proposals, ProposalCount { pending: 1, done: 4 });
    // The gap the proposal was for is no longer a gap: its search now finds the listener.
    assert_eq!((report.absences["miss"].status, report.absences["miss"].found), (Status::Unconfirmed, 1));
    assert_eq!(report.summary.unconfirmed, 1);
}

#[test]
fn reports_a_proposal_the_code_already_settles() {
    let report = run(
        &load(&proposal(|m| {
            item(m, "nodes", "pl")["symbol"] = json!("src/automations/evaluate.ts#evaluateRules"); // already there
            item(m, "nodes", "j1")["evidence"][0]["text"] = json!("function runWeekly(");
            // not there
        })),
        &app(),
    );
    let settled: Vec<String> =
        settled_proposals(&report).iter().map(|p| format!("{} {} {}", p.what, p.id, p.kind.as_str())).collect();
    assert_eq!(settled, vec!["box j1 remove", "box pl add"]);
}

#[test]
fn never_counts_a_tour_stop_about_a_proposal_as_unconfirmed() {
    let map = load(&proposal(|m| {
        m["tour"].as_array_mut().unwrap().push(json!({ "id": "tp", "title": "The proposal", "text": "Listen instead of waiting for the night.", "focus": { "nodes": ["pl", "j1"], "edges": ["p1"] } }));
    }));
    assert_eq!(run(&map, &app()).tour["tp"], TourCheck { status: TourStatus::Ok, unconfirmed: vec![] });
}

const HEADER: &str = "src/ui/PatientHeader.tsx";

fn with_sketch(m: &mut Value) {
    let s1 = item(m, "nodes", "s1");
    s1["proposed"] = json!({ "kind": "change", "note": "Add a way to undo a conversion." });
    s1["screen"] = json!({ "kind": "sketch", "file": HEADER, "elements": [{ "kind": "button", "text": "Mark as converted" }, { "kind": "button", "text": "Undo conversion", "proposed": true }] });
}

fn sketch_elements(show: &Option<ScreenShow>) -> Vec<(String, Option<bool>)> {
    match show {
        Some(ScreenShow::Sketch { elements, .. }) => elements.iter().map(|e| (e.text.clone(), e.proposed)).collect(),
        other => panic!("not a sketch: {other:?}"),
    }
}

#[test]
fn a_proposed_screen_shows_what_would_be_added_next_to_what_is_there() {
    let report = run(&load(&proposal(with_sketch)), &app());
    assert_eq!(report.screens["s1"].status, ScreenStatus::Confirmed);
    assert_eq!(
        sketch_elements(&report.screens["s1"].show),
        vec![("Mark as converted".into(), None), ("Undo conversion".into(), Some(true))]
    );
    let invented = run(
        &load(&proposal(|m| {
            with_sketch(m);
            item(m, "nodes", "s1")["screen"]["elements"]
                .as_array_mut()
                .unwrap()
                .push(json!({ "kind": "link", "text": "Help" }));
        })),
        &app(),
    );
    assert_eq!(invented.screens["s1"].status, ScreenStatus::Unconfirmed);
    assert_eq!(invented.screens["s1"].reasons, vec![format!("\"Help\" isn't in {HEADER}")]);
}

#[test]
fn stops_calling_an_element_proposed_once_the_file_has_it() {
    let root = copy_app();
    write(root.path(), HEADER, &format!("{}\n// <button>Undo conversion</button>\n", read(root.path(), HEADER)));
    let show = run(&load(&proposal(with_sketch)), root.path()).screens["s1"].show.clone();
    assert_eq!(sketch_elements(&show), vec![("Mark as converted".into(), None), ("Undo conversion".into(), None)]);
    assert!(!serde_json::to_string(&show).unwrap().contains("proposed"));
}

#[test]
fn draws_a_new_screen_for_a_proposed_box_whose_file_does_not_exist_yet() {
    let report = run(
        &load(&proposal(|m| {
            item(m, "nodes", "pl")["screen"] = json!({ "kind": "sketch", "file": "src/ui/Welcome.tsx", "elements": [{ "kind": "heading", "text": "Welcome aboard" }] });
        })),
        &app(),
    );
    assert_eq!(report.screens["pl"].status, ScreenStatus::Confirmed);
    assert_eq!(sketch_elements(&report.screens["pl"].show), vec![("Welcome aboard".into(), Some(true))]);
}

#[test]
fn a_missing_connection_says_how_many_files_its_search_covered() {
    let miss = &run(&load(&map_json()), &app()).absences["miss"];
    assert_eq!((miss.status, miss.found, miss.searched), (Status::Confirmed, 0, 12));
    let mut narrow = map_json();
    narrow["absences"][0]["search"]["paths"] = json!(["src/nowhere/**"]);
    let r = &run(&load(&narrow), &app()).absences["miss"];
    assert_eq!((r.status, r.searched), (Status::Unconfirmed, 0));
    assert_eq!(r.reasons, vec!["no files match src/nowhere/**, so nothing was searched"]);
}
