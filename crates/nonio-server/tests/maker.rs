//! Making a map with a scripted model against a copy of the care app. Ported from the `makeMap` tests in
//! `packages/server/test/workflows.test.ts`, "a map with a proposal" in `proposals.test.ts`, and the journey tests in
//! `journeys.test.ts`.

mod common;

use std::sync::{Arc, Mutex};

use common::*;
use nonio_core::check::Summary;
use nonio_core::check::{ProposalState, Status};
use nonio_server::maker::{make_map, MakeOptions};
use nonio_server::prompts::explainer_prompt;
use nonio_server::schemas::{ANSWER, MAP, MAP_PATCH};
use serde_json::{json, Value};

fn opts(root: &std::path::Path, question: &str, agent: Arc<ScriptedAgent>) -> MakeOptions {
    MakeOptions::new(root, question, agent)
}

fn summary(claims: usize, confirmed: usize, pending: usize) -> Summary {
    Summary {
        claims,
        confirmed,
        unconfirmed: claims - confirmed,
        proposals: nonio_core::check::ProposalCount { pending, done: 0 },
        changes: None,
    }
}

fn node<'a>(m: &'a mut Value, id: &str) -> &'a mut Value {
    m["nodes"].as_array_mut().unwrap().iter_mut().find(|n| n["id"] == id).unwrap()
}

#[tokio::test]
async fn gives_the_model_a_head_start_from_the_code_facts_then_keeps_the_checked_map() {
    let root = copy_care_app();
    let agent = ScriptedAgent::new(vec![reply(care_map_json())]);
    let progress = Arc::new(Mutex::new(Vec::<String>::new()));
    let p = progress.clone();
    let mut o = opts(root.path(), "What happens when a patient converts?", agent.clone());
    o.on_progress = Some(Arc::new(move |x| p.lock().unwrap().push(x.text)));
    let r = make_map(o).await.unwrap();
    assert_eq!(r.report.summary, summary(15, 15, 0));
    assert_eq!(r.map.question, "What happens when a patient converts?"); // the person's words, not the model's
    assert_eq!(r.rounds, 1);
    let call = agent.call(0);
    for want in [
        "POST /api/patients/{}/convert → src/server/routes.ts#convertPatient",
        "src/ui/PatientHeader.tsx#PatientHeader.onConvert sends POST /api/patients/{}/convert",
        "Functions by file (use these exact names for `symbol`):",
        "  - src/ui/PatientHeader.tsx: PatientHeader, PatientHeader.onConvert",
        "  - src/db.ts: db.update, db.where",
        "patients ← src/patients/service.ts#markConverted",
    ] {
        assert!(call.prompt.contains(want), "{want}");
    }
    assert!(Arc::ptr_eq(&call.schema, &MAP.json));
    let progress = progress.lock().unwrap();
    assert!(progress.contains(&"Reading src/patients/service.ts".to_string()));
    assert!(progress.contains(&"15 of 15 claims confirmed".to_string()));
}

#[tokio::test]
async fn sends_what_failed_back_in_the_same_session_and_takes_back_only_the_changes() {
    let root = copy_care_app();
    let mut broken = care_map_json();
    node(&mut broken, "u2")["evidence"][0]["text"] = json!("function enqueue(");
    let fixed = node(&mut care_map_json(), "u2").clone();
    let agent = ScriptedAgent::new(vec![reply(broken), reply(json!({ "nodes": [fixed] }))]);
    let r = make_map(opts(root.path(), "q", agent.clone())).await.unwrap();
    assert_eq!(r.rounds, 2);
    assert_eq!(r.report.summary, summary(15, 15, 0));
    let ids = |m: &[nonio_core::format::MapNode]| m.iter().map(|n| n.id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(&r.map.nodes), ids(&care_map().nodes)); // u2 replaced in place, the rest kept
    assert_eq!(r.map.tour, care_map().tour);
    let second = agent.call(1);
    assert_eq!(second.resume.as_deref(), Some("session-1"));
    assert!(Arc::ptr_eq(&second.schema, &MAP_PATCH.json));
    assert!(second.prompt.contains("box u2: src/jobs/queue.ts:3: \"function enqueue(\" isn't on line 3 or near it"));
    assert!(second.prompt.contains("Return only the changes, not the whole map."));
}

#[tokio::test]
async fn suggests_the_real_name_when_a_box_cites_a_function_the_code_calls_something_else() {
    let root = copy_care_app();
    let mut broken = care_map_json();
    node(&mut broken, "s1")["symbol"] = json!("src/ui/PatientHeader.tsx#onConvert");
    let mut fixed = node(&mut broken, "s1").clone();
    fixed["symbol"] = json!("src/ui/PatientHeader.tsx#PatientHeader.onConvert");
    let agent = ScriptedAgent::new(vec![reply(broken), reply(json!({ "nodes": [fixed] }))]);
    let r = make_map(opts(root.path(), "q", agent.clone())).await.unwrap();
    assert!(agent.call(1).prompt.contains(
        "box s1: the code has no symbol src/ui/PatientHeader.tsx#onConvert (in that file it's src/ui/PatientHeader.tsx#PatientHeader.onConvert)"
    ));
    assert_eq!(r.report.nodes["s1"].status, Status::Confirmed);
    assert_eq!(r.report.nodes["s1"].proof, Some(nonio_core::Proof::Fact));
}

#[tokio::test]
async fn drops_claims_a_repair_removes_with_the_arrows_that_pointed_at_them() {
    let root = copy_care_app();
    let mut broken = care_map_json();
    node(&mut broken, "u3")["evidence"][0]["text"] = json!("export async function mailEverything");
    let agent = ScriptedAgent::new(vec![reply(broken), reply(json!({ "remove": ["u3"] }))]);
    let r = make_map(opts(root.path(), "q", agent)).await.unwrap();
    assert!(!r.map.nodes.iter().any(|n| n.id == "u3"));
    assert!(!r.map.edges.iter().any(|e| e.from == "u3" || e.to == "u3"));
    assert_eq!(r.report.summary.unconfirmed, 0);
}

#[tokio::test]
async fn asks_again_when_its_changes_cant_be_applied_and_keeps_the_last_good_map_meanwhile() {
    let root = copy_care_app();
    let mut broken = care_map_json();
    node(&mut broken, "u2")["evidence"][0]["text"] = json!("function enqueue(");
    let fixed = node(&mut care_map_json(), "u2").clone();
    let mut bad_edge = care_map_json()["edges"][0].clone();
    bad_edge["to"] = json!("nowhere");
    let agent = ScriptedAgent::new(vec![
        reply(broken),
        reply(json!({ "edges": [bad_edge] })),
        reply(json!({ "nodes": [fixed] })),
    ]);
    let r = make_map(opts(root.path(), "q", agent.clone())).await.unwrap();
    let third = agent.call(2);
    assert!(third.prompt.contains("Your changes couldn't be applied to the map:"));
    assert!(third.prompt.contains("unknown node \"nowhere\""), "{}", third.prompt);
    assert!(Arc::ptr_eq(&third.schema, &MAP_PATCH.json));
    assert_eq!(r.rounds, 3);
    assert_eq!(r.report.summary.unconfirmed, 0);
}

#[tokio::test]
async fn repairs_a_map_whose_structure_is_wrong_before_checking_it() {
    let root = copy_care_app();
    let mut wrong = care_map_json();
    let mut e = care_map_json()["edges"][0].clone();
    e["to"] = json!("nowhere");
    wrong["edges"] = json!([e]);
    let agent = ScriptedAgent::new(vec![reply(wrong), reply(care_map_json())]);
    let r = make_map(opts(root.path(), "q", agent.clone())).await.unwrap();
    assert!(agent.call(1).prompt.contains("edge \"e12\": unknown node \"nowhere\""), "{}", agent.call(1).prompt);
    assert_eq!(r.report.summary.unconfirmed, 0);
}

#[tokio::test]
async fn gives_up_with_a_readable_error_when_the_map_never_parses() {
    let root = copy_care_app();
    let agent = ScriptedAgent::new(vec![reply(json!({})), reply(json!({})), reply(json!({}))]);
    let err = make_map(opts(root.path(), "q", agent)).await.unwrap_err();
    assert!(err.to_string().starts_with("your agent's map couldn't be read"), "{err}");
}

#[test]
fn makes_a_map_in_a_future_that_can_move_between_threads() {
    fn send<T: Send>(_: &T) {}
    let root = copy_care_app();
    let f = make_map(opts(root.path(), "q", ScriptedAgent::new(vec![])));
    send(&f);
}

// Proposals.

fn listener_box() -> Value {
    let area = care_map_json()["nodes"].as_array().unwrap().iter().find(|n| n["id"] == "u1").unwrap()["area"].clone();
    json!({ "id": "pl", "area": area, "name": "Listen for conversions", "symbol": "src/automations/listeners.ts#onConverted", "proposed": { "kind": "add", "note": "Run the rules right away." } })
}

fn listener_arrows() -> Vec<Value> {
    vec![
        json!({ "id": "p1", "from": "s4", "to": "pl", "kind": "then", "label": "is heard by", "proposed": { "kind": "add" } }),
        json!({ "id": "p2", "from": "pl", "to": "u1", "kind": "calls", "label": "runs the rules", "proposed": { "kind": "add" } }),
    ]
}

fn with_listener(b: Value) -> Value {
    let mut m = care_map_json();
    m["nodes"].as_array_mut().unwrap().push(b);
    m["edges"].as_array_mut().unwrap().extend(listener_arrows());
    m
}

#[tokio::test]
async fn a_proposal_on_confirmed_code_is_made_in_one_round() {
    let root = copy_care_app();
    let agent = ScriptedAgent::new(vec![reply(with_listener(listener_box()))]);
    let r =
        make_map(opts(root.path(), "How would we send the welcome email right away?", agent.clone())).await.unwrap();
    assert_eq!(r.rounds, 1);
    assert_eq!(r.report.summary, summary(15, 15, 3));
    let p = agent.call(0).prompt;
    assert!(p.contains("## Proposals (`proposed` on a box or arrow; only when a change is asked for)"));
    assert!(p.contains("## When the question is about what a person sees or does"));
}

#[tokio::test]
async fn sends_back_a_proposal_to_add_what_the_code_already_has() {
    let root = copy_care_app();
    let mut exists = listener_box();
    exists["symbol"] = json!("src/automations/evaluate.ts#evaluateRules");
    let agent = ScriptedAgent::new(vec![reply(with_listener(exists)), reply(json!({ "nodes": [listener_box()] }))]);
    let r = make_map(opts(root.path(), "q", agent.clone())).await.unwrap();
    assert_eq!(r.rounds, 2);
    assert!(agent.call(1).prompt.contains("- box pl: you propose adding it, but the code already has it."));
    let pl = &r.report.nodes["pl"];
    assert_eq!(pl.status, Status::Proposed);
    assert_eq!(pl.proposal.map(|p| (p.kind.as_str(), p.state)), Some(("add", ProposalState::Pending)));
}

#[tokio::test]
async fn a_proposal_turns_to_done_when_the_same_map_is_checked_after_the_change_is_built() {
    let root = copy_care_app();
    let agent = ScriptedAgent::new(vec![reply(with_listener(listener_box()))]);
    let r = make_map(opts(root.path(), "q", agent)).await.unwrap();
    std::fs::write(
        root.path().join("src/automations/listeners.ts"),
        "import { evaluateRules } from './evaluate';\nexport function onConverted({ patientId }: { patientId: string }) {\n  return evaluateRules({ patientId });\n}\n",
    )
    .unwrap();
    let after = check_map(&r.map, root.path());
    for c in [&after.nodes["pl"], &after.edges["p2"]] {
        assert_eq!(c.status, Status::Confirmed);
        assert_eq!(c.proposal.map(|p| (p.kind.as_str(), p.state)), Some(("add", ProposalState::Done)));
    }
}

// Journeys.

fn journey() -> Value {
    serde_json::from_str(&std::fs::read_to_string(core_fixtures().join("care-app.journey.json")).unwrap()).unwrap()
}

/// The properties a node or an edge may have in an output schema, found by walking it.
fn item_props(schema: &Value, list: &str) -> Vec<String> {
    let mut found = Vec::new();
    fn walk(v: &Value, list: &str, found: &mut Vec<String>) {
        if let Some(props) = v.pointer(&format!("/properties/{list}/items/properties")).and_then(Value::as_object) {
            found.extend(props.keys().cloned());
        }
        match v {
            Value::Object(o) => o.values().for_each(|x| walk(x, list, found)),
            Value::Array(a) => a.iter().for_each(|x| walk(x, list, found)),
            _ => {}
        }
    }
    walk(schema, list, &mut found);
    found
}

#[test]
fn lets_the_model_write_a_branch_on_boxes_and_arrows_in_a_map_a_repair_and_an_answer() {
    for s in [&MAP, &MAP_PATCH, &ANSWER] {
        assert!(item_props(&s.json, "nodes").contains(&"branch".to_string()));
        assert!(item_props(&s.json, "edges").contains(&"branch".to_string()));
    }
    assert!(MAP.problems(&journey()).is_empty());
}

#[tokio::test]
async fn keeps_the_branches_through_making_the_map_and_checks_every_part_of_it() {
    let root = copy_care_app();
    let j = journey();
    let q = j["question"].as_str().unwrap().to_string();
    let r = make_map(opts(root.path(), &q, ScriptedAgent::new(vec![reply(j)]))).await.unwrap();
    assert_eq!((r.report.summary.claims, r.report.summary.confirmed, r.report.summary.unconfirmed), (12, 12, 0));
    let branches: Vec<(String, String)> =
        r.map.edges.iter().filter_map(|e| e.branch.clone().map(|b| (e.id.clone(), b))).collect();
    let want = [
        ("jc", "Writes a note instead"),
        ("jd", "Writes a note instead"),
        ("je", "Goes back to the lists"),
        ("jf", "No welcome rule"),
    ];
    assert_eq!(branches, want.map(|(a, b)| (a.to_string(), b.to_string())));
    let p = explainer_prompt(&r.map, &r.report, &[], "And if they write a note?", "", false, &[]);
    assert!(p.contains("\"branch\":\"Writes a note instead\""));
    assert!(p.contains("carries the `branch` it is on"));
}
