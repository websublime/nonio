//! Every prompt against the original's own output on the same input. The fixtures in `tests/fixtures/prompts` were
//! written by illuminate's prompt functions (see `tools/record-prompts`), with "illuminate" read as "nonio".

mod common;

use std::fs;

use common::*;
use nonio_core::format::View;
use nonio_core::parse_map;
use nonio_server::prompts::*;
use pretty_assertions::assert_eq;
use serde_json::Value;

fn golden(name: &str) -> String {
    fs::read_to_string(fixtures().join(format!("prompts/{name}.txt"))).unwrap()
}

fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn the_maker_prompt_is_the_originals() {
    let p =
        maker_prompt("What happens when a patient converts?", "Ways in (entry points):\n  - route POST /x → a.ts#b\n");
    assert_eq!(p, golden("maker"));
}

#[test]
fn the_repair_prompts_are_the_originals() {
    assert_eq!(
        shape_repair_prompt(&strings(&["nodes: Required", "edges.0.from: Expected string"])),
        golden("shape-repair")
    );
    assert_eq!(patch_repair_prompt(&strings(&["edge e9 goes to a box that does not exist"])), golden("patch-repair"));
}

fn broken() -> (nonio_core::CodeMap, nonio_core::CheckReport, tempfile::TempDir) {
    let raw: Value =
        serde_json::from_str(&fs::read_to_string(fixtures().join("prompts/broken.map.json")).unwrap()).unwrap();
    let map = parse_map(&raw).unwrap();
    let root = copy_care_app();
    let report = check_map(&map, root.path());
    (map, report, root)
}

#[test]
fn the_evidence_repair_prompt_is_the_originals_with_and_without_suggested_names() {
    let (_, report, _root) = broken();
    let symbols_in = |file: &str| -> Vec<String> {
        if file.ends_with("routes.ts") {
            strings(&["convertPatient", "listPatients", "router.convert"])
        } else {
            vec![]
        }
    };
    assert_eq!(evidence_repair_prompt(&report, Some(&symbols_in)), golden("evidence-repair"));
    assert_eq!(evidence_repair_prompt(&report, None), golden("evidence-repair-plain"));
}

#[test]
fn the_answer_prompts_are_the_originals() {
    let root = copy_care_app();
    let map = care_map();
    let report = check_map(&map, root.path());
    let selection = vec![SelectionLine { id: map.nodes[0].id.clone(), label: map.nodes[0].name.clone() }];
    let earlier = vec![EarlierTurn { question: "Why?".into(), concept: "Because nothing listens.".into() }];
    let others = vec![OtherMap { id: "other".into(), question: "How do invoices work?".into() }];
    assert_eq!(
        quick_answer_prompt(&map, &report, &selection, "Where is the email sent?", "1 | a\n2 | b", &earlier, &others),
        golden("quick-answer")
    );
    assert_eq!(quick_answer_prompt(&map, &report, &[], "Where?", "", &[], &[]), golden("quick-answer-bare"));
    assert_eq!(
        explainer_prompt(&map, &report, &selection, "Where is the email sent?", "The job code.", false, &earlier),
        golden("explainer")
    );
    assert_eq!(explainer_prompt(&map, &report, &[], "How do invoices work?", "", true, &[]), golden("explainer-words"));

    let mut journey = map.clone();
    journey.view = Some(View::Journey);
    let jr = check_map(&journey, root.path());
    assert_eq!(
        explainer_prompt(&journey, &jr, &[], "What does the person see?", "", false, &[]),
        golden("explainer-journey")
    );

    let (bmap, breport, _b) = broken();
    assert_eq!(explainer_prompt(&bmap, &breport, &[], "Is it right?", "", false, &[]), golden("explainer-broken"));
}

// The original's prompt tests (arrows, branches, routes, timeline, journeys), on the rules themselves.

#[test]
fn tells_the_maker_a_label_reads_from_the_box_it_leaves_and_to_say_who_does_the_next_step() {
    let rules = MAP_RULES.join("\n");
    assert!(rules.contains("\"<from box> <label> <to box>\""));
    assert!(rules.contains("say who or what sets it off"));
}

#[test]
fn ways_through_are_a_shared_rule_before_the_sections_for_particular_kinds_of_map() {
    let p = maker_prompt("q", "");
    let at = |s: &str| p.find(s).unwrap_or_else(|| panic!("{s}"));
    assert!(at("## Ways through") < at("## When the answer is parties taking turns"));
    assert!(at("## Ways through") < at("## When the question is about what a person sees"));
    let e = explainer_prompt(&care_map(), &empty_report(), &[], "q", "", false, &[]);
    assert!(e.contains("## Ways through"));
}

fn empty_report() -> nonio_core::CheckReport {
    let root = copy_care_app();
    check_map(&care_map(), root.path())
}
