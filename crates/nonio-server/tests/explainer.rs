//! Answering on a map, with a scripted model against a copy of the care app. Ported from the `answer` tests in
//! `packages/server/test/workflows.test.ts` and "a proposal in an answer" in `proposals.test.ts`.

mod common;

use std::sync::Arc;

use common::*;
use nonio_server::explainer::{
    answer, cited_excerpts, AnswerOptions, OfferStatus, RefState, RefStatus, Selection, Source,
};
use nonio_server::prompts::{EarlierTurn, OtherMap};
use nonio_server::schemas::{ANSWER, MAP, MAP_PATCH, QUICK_ANSWER};
use serde_json::{json, Value};

fn ask(root: &std::path::Path, question: &str, agent: &Arc<ScriptedAgent>) -> AnswerOptions {
    AnswerOptions::new(root, care_map(), question, agent.clone())
}

fn nodes(ids: &[&str]) -> Selection {
    Selection { nodes: ids.iter().map(|s| s.to_string()).collect(), ..Default::default() }
}

fn refs(list: &[(&str, RefState)]) -> Vec<RefStatus> {
    list.iter().map(|(id, status)| RefStatus { id: id.to_string(), status: *status }).collect()
}

#[tokio::test]
async fn answers_from_the_map_alone_when_it_can() {
    let root = copy_care_app();
    let agent = ScriptedAgent::new(vec![reply(json!({
        "route": "answer",
        "concept": "This is where the app tells the rest of the system a patient enrolled. Nothing is listening.",
        "refs": ["s4", "miss", "invented"],
        "moves": { "focus": "s4", "highlight": ["s4", "miss", "u1", "nope"] },
    }))]);
    let mut o = ask(root.path(), "What does this do?", &agent);
    o.selection = nodes(&["s4"]);
    o.session = Some("session-0".into());
    o.earlier = vec![EarlierTurn { question: "What starts this?".into(), concept: "Staff press Convert.".into() }];
    let r = answer(o).await.unwrap();
    assert_eq!(
        r.answer.refs,
        refs(&[("s4", RefState::Confirmed), ("miss", RefState::Confirmed), ("invented", RefState::Unknown)])
    );
    assert_eq!(r.answer.moves.focus.as_deref(), Some("s4"));
    assert_eq!(r.answer.moves.highlight, ["s4", "miss", "u1"]);
    assert_eq!((r.answer.source, r.answer.added.len(), r.answer.fix.is_none()), (Source::Map, 0, true));
    assert_eq!(agent.count(), 1);
    let quick = agent.call(0);
    assert!(Arc::ptr_eq(&quick.schema, &QUICK_ANSWER.json));
    assert_eq!((quick.tools.as_deref(), quick.resume.as_deref()), (Some(&[][..]), None));
    assert!(quick.prompt.contains("- s4: box \"Announces “converted”\""));
    assert!(quick.prompt.contains("(cited by s4)"));
    assert!(quick.prompt.contains("- \"What starts this?\": Staff press Convert."));
    assert_eq!(r.session_id.as_deref(), Some("session-0"));
    assert_eq!(r.map, care_map());
}

#[tokio::test]
async fn gives_both_steps_the_conversation_so_far() {
    let root = copy_care_app();
    let agent = ScriptedAgent::new(vec![
        needs_code("The listener code."),
        reply(json!({ "concept": "It announces it.", "refs": ["s4"] })),
    ]);
    let mut o = ask(root.path(), "Show me that in the code", &agent);
    o.earlier = vec![EarlierTurn {
        question: "Why is the email late?".into(),
        concept: "Nothing listens for the announcement.".into(),
    }];
    answer(o).await.unwrap();
    assert_eq!(agent.count(), 2);
    for i in 0..2 {
        let p = agent.call(i).prompt;
        assert!(p.contains("## The conversation so far on this map (oldest first)"));
        assert!(p.contains("- \"Why is the email late?\": Nothing listens for the announcement."));
    }
}

#[tokio::test]
async fn puts_a_picture_of_the_screen_on_an_existing_box_and_says_when_one_cant_be_shown() {
    let root = copy_care_app();
    let sketch = |text: &str| json!({ "kind": "sketch", "file": "src/ui/PatientHeader.tsx", "elements": [{ "kind": "button", "text": text }], "highlight": text });
    let agent = ScriptedAgent::new(vec![
        needs_code("What the button looks like."),
        reply(json!({ "concept": "It is this button.", "refs": ["s1"], "screens": [
            { "node": "s1", "screen": sketch("Mark as converted") },
            { "node": "s2", "screen": sketch("Archive patient") },
            { "node": "nope", "screen": sketch("Mark as converted") },
        ] })),
    ]);
    let mut o = ask(root.path(), "What does this look like?", &agent);
    o.selection = nodes(&["s1"]);
    let r = answer(o).await.unwrap();
    assert!(agent.call(1).prompt.contains("put `{ \"node\": \"<box id>\", \"screen\": { … } }` in `screens`"));
    assert_eq!((r.answer.screens.clone(), r.answer.added.len()), (vec!["s1".to_string()], 0));
    let s1 = r.map.nodes.iter().find(|n| n.id == "s1").unwrap().screen.clone().unwrap();
    assert_eq!((s1.kind.as_str(), s1.highlight.as_deref()), ("sketch", Some("Mark as converted")));
    assert!(r.map.nodes.iter().find(|n| n.id == "s2").unwrap().screen.is_none());
    assert_eq!(r.report.screens["s1"].status, nonio_core::screens::ScreenStatus::Confirmed);
    assert_eq!(r.answer.problems.len(), 2);
    assert_eq!(r.answer.problems[0], "No screen shown: there is no box \"nope\"");
    assert!(r.answer.problems[1].starts_with("No screen shown for \""), "{}", r.answer.problems[1]);
    assert!(
        r.answer.problems[1].ends_with("\"Archive patient\" isn't in src/ui/PatientHeader.tsx"),
        "{}",
        r.answer.problems[1]
    );
}

#[test]
fn asks_for_the_plain_answer_only_and_the_plain_explanation_only_for_boxes() {
    let props = |v: &Value| v["properties"].as_object().unwrap().keys().cloned().collect::<Vec<_>>();
    assert!(!props(&ANSWER.json).contains(&"how".to_string()));
    assert!(!props(&ANSWER.json).contains(&"code".to_string()));
    assert!(!props(&QUICK_ANSWER.json).contains(&"how".to_string()));
    for s in [&MAP, &MAP_PATCH] {
        assert_eq!(props(&s.json["properties"]["nodes"]["items"]["properties"]["explain"]), ["concept"]);
    }
}

#[tokio::test]
async fn says_when_a_question_is_about_a_different_part_of_the_app_and_points_at_a_saved_map() {
    let root = copy_care_app();
    let others = vec![OtherMap { id: "billing".into(), question: "How does billing work?".into() }];
    let agent = ScriptedAgent::new(vec![
        reply(json!({ "route": "elsewhere", "newQuestion": "How do invoices get sent?", "existingMap": "billing" })),
        reply(json!({ "route": "elsewhere", "newQuestion": "", "existingMap": "made-up" })),
    ]);
    let mut o = ask(root.path(), "and invoices?", &agent);
    o.session = Some("session-0".into());
    o.other_maps = others.clone();
    let r = answer(o).await.unwrap();
    let offer = r.answer.offer.clone().unwrap();
    assert_eq!((r.answer.concept.as_str(), r.answer.added.len(), r.answer.fix.is_none()), ("", 0, true));
    assert_eq!(offer.question, "How do invoices get sent?");
    assert_eq!(offer.existing.map(|e| (e.id, e.question)), Some(("billing".into(), "How does billing work?".into())));
    assert_eq!((offer.status, offer.map_id), (OfferStatus::Pending, None));
    assert_eq!(agent.count(), 1); // no search: they choose first
    assert!(agent.call(0).prompt.contains("- billing: \"How does billing work?\""));
    assert_eq!(r.session_id.as_deref(), Some("session-0"));
    assert_eq!(r.map, care_map());

    let mut o = ask(root.path(), "and invoices?", &agent);
    o.other_maps = others;
    let offer = answer(o).await.unwrap().answer.offer.unwrap();
    assert_eq!((offer.question.as_str(), offer.existing), ("and invoices?", None));
    // only maps that exist
}

#[tokio::test]
async fn treats_a_question_about_something_selected_as_about_this_map() {
    let root = copy_care_app();
    let agent = ScriptedAgent::new(vec![
        reply(json!({ "route": "elsewhere", "newQuestion": "x" })),
        reply(json!({ "concept": "It announces it.", "refs": ["s4"] })),
    ]);
    let mut o = ask(root.path(), "and this?", &agent);
    o.selection = nodes(&["s4"]);
    let r = answer(o).await.unwrap();
    assert_eq!(
        (r.answer.source, r.answer.offer.is_none(), r.answer.concept.as_str()),
        (Source::Code, true, "It announces it.")
    );
}

#[tokio::test]
async fn answers_in_words_from_the_code_drawing_nothing_and_proposing_nothing() {
    let root = copy_care_app();
    let agent = ScriptedAgent::new(vec![reply(json!({
        "concept": "Invoices aren't in this app.",
        "add": { "nodes": [{ "id": "n-filter", "area": "jobs", "name": "Only yesterday's patients", "evidence": [{ "file": "src/jobs/nightly.ts", "line": 9, "text": "createdAfter: yesterday" }] }] },
        "fix": listener_fix(),
    }))]);
    let mut o = ask(root.path(), "How do invoices get sent?", &agent);
    o.words_only = true;
    let r = answer(o).await.unwrap();
    assert_eq!(agent.count(), 1); // straight to the code
    assert!(Arc::ptr_eq(&agent.call(0).schema, &ANSWER.json));
    assert!(agent.call(0).prompt.contains("different part of the app"));
    assert_eq!(
        (r.answer.source, r.answer.concept.as_str(), r.answer.added.len(), r.answer.fix.is_none()),
        (Source::Code, "Invoices aren't in this app.", 0, true)
    );
    assert_eq!(r.map, care_map());
}

#[test]
fn reads_the_selections_code_first_and_never_secret_files() {
    let root = copy_care_app();
    let mut map = care_map();
    map.nodes[0].evidence.insert(0, nonio_core::format::Evidence { file: ".env".into(), line: 1, text: "KEY".into() });
    let text = cited_excerpts(root.path(), &map, &nodes(&["s4"]));
    assert!(text.split('\n').next().unwrap().ends_with("(cited by s4)"), "{text}");
    assert!(!text.contains(".env"));
}

#[tokio::test]
async fn goes_to_the_code_when_the_map_isnt_enough_if_the_quick_try_fails_or_when_asked_not_to_try() {
    let root = copy_care_app();
    let answer_reply = || reply(json!({ "concept": "The rules only run in a nightly job.", "refs": ["j1"] }));
    let agent = ScriptedAgent::new(vec![
        needs_code("How the nightly job picks patients."),
        answer_reply(),
        reply(json!({ "nonsense": true })),
        answer_reply(),
        answer_reply(),
    ]);
    let mut o = ask(root.path(), "Where do the rules run?", &agent);
    o.session = Some("session-0".into());
    let r = answer(o).await.unwrap();
    assert_eq!((r.answer.source, r.answer.concept.as_str()), (Source::Code, "The rules only run in a nightly job."));
    assert!((r.answer.cost - 0.02).abs() < 1e-9);
    let second = agent.call(1);
    assert!(Arc::ptr_eq(&second.schema, &ANSWER.json));
    assert_eq!(second.resume.as_deref(), Some("session-0"));
    assert!(second.tools.is_none()); // Read, Grep and Glob
    assert!(second.prompt.contains("## Why the map alone wasn't enough\nHow the nightly job picks patients."));

    assert_eq!(answer(ask(root.path(), "Where?", &agent)).await.unwrap().answer.source, Source::Code);
    assert!(Arc::ptr_eq(&agent.call(3).schema, &ANSWER.json));
    let mut o = ask(root.path(), "Where?", &agent);
    o.quick = false;
    answer(o).await.unwrap();
    let quick: Vec<bool> = (0..5).map(|i| Arc::ptr_eq(&agent.call(i).schema, &QUICK_ANSWER.json)).collect();
    assert_eq!(quick, [true, false, true, false, false]);
}

#[tokio::test]
async fn draws_what_it_found_when_the_checker_confirms_it_and_drops_what_it_cant_confirm() {
    let root = copy_care_app();
    let agent = ScriptedAgent::new(vec![
        needs_code("How the nightly job picks patients."),
        reply(json!({
            "concept": "The rules only run in a nightly job.",
            "refs": ["j1", "n-filter"],
            "add": {
                "nodes": [
                    { "id": "n-filter", "area": "jobs", "name": "Only yesterday's patients", "evidence": [{ "file": "src/jobs/nightly.ts", "line": 9, "text": "createdAfter: yesterday" }] },
                    { "id": "n-wrong", "area": "jobs", "name": "Retries failed emails", "evidence": [{ "file": "src/jobs/nightly.ts", "line": 3, "text": "retryFailed(" }] },
                ],
                "edges": [{ "id": "e-filter", "from": "j1", "to": "n-filter", "kind": "then", "label": "filters", "evidence": [{ "file": "src/jobs/nightly.ts", "line": 9, "text": "db.where('patients'" }] }],
            },
        })),
    ]);
    let mut o = ask(root.path(), "Where do the rules run?", &agent);
    o.session = Some("session-0".into());
    let r = answer(o).await.unwrap();
    let mut added = r.answer.added.clone();
    added.sort();
    assert_eq!(added, ["e-filter", "n-filter"]);
    assert!(r.map.nodes.iter().any(|n| n.id == "n-filter"));
    assert!(!r.map.nodes.iter().any(|n| n.id == "n-wrong"));
    assert_eq!(
        r.answer.problems,
        ["Not drawn, couldn't confirm n-wrong: src/jobs/nightly.ts:3: \"retryFailed(\" isn't on line 3 or near it"]
    );
    assert_eq!(r.report.summary.unconfirmed, 0);
    assert_eq!(agent.call(1).resume.as_deref(), Some("session-0"));
}

#[tokio::test]
async fn turns_a_change_request_into_a_validated_fix_proposal_and_never_edits() {
    let root = copy_care_app();
    let agent = ScriptedAgent::new(vec![
        needs_code("The change itself."),
        reply(json!({ "concept": "Listen for the announcement.", "refs": ["miss"], "fix": listener_fix() })),
    ]);
    let mut o = ask(root.path(), "Make the email go out right away", &agent);
    o.selection = Selection { absences: vec!["miss".into()], ..Default::default() };
    let r = answer(o).await.unwrap();
    let fix = r.answer.fix.unwrap();
    assert_eq!(fix.summary, listener_fix()["summary"].as_str().unwrap());
    assert_eq!(fix.files.len(), 1);
    assert_eq!(
        (fix.files[0].file.as_str(), fix.files[0].change, fix.files[0].hash.clone()),
        ("src/automations/listeners.ts", nonio_server::fixes::Change::Create, None)
    );
    assert!(fix.diff.contains("+ bus.on('patient.converted', ({ patientId }) => evaluateRules({ patientId }));"));
    assert!(!root.path().join("src/automations/listeners.ts").exists());
    assert!(agent.call(1).prompt.contains("- miss: missing connection: Nothing listens for patient.converted"));
}

// Proposals in an answer.

fn listener_box() -> Value {
    json!({ "id": "pl", "area": "automations", "name": "Listen for conversions", "symbol": "src/automations/listeners.ts#onConverted", "proposed": { "kind": "add", "note": "Run the rules right away." } })
}

fn listener_arrows() -> Value {
    json!([
        { "id": "p1", "from": "s4", "to": "pl", "kind": "then", "label": "is heard by", "proposed": { "kind": "add" } },
        { "id": "p2", "from": "pl", "to": "u1", "kind": "calls", "label": "runs the rules", "proposed": { "kind": "add" } },
    ])
}

#[tokio::test]
async fn draws_new_boxes_as_proposed_and_marks_what_is_already_on_the_map_as_to_change_or_remove() {
    let root = copy_care_app();
    let agent = ScriptedAgent::new(vec![
        needs_code("What a listener would look like."),
        reply(json!({
            "concept": "Listen for the announcement and drop the nightly job.",
            "refs": ["pl", "j1"],
            "add": { "nodes": [listener_box()], "edges": listener_arrows() },
            "propose": [
                { "id": "j1", "proposed": { "kind": "remove", "note": "Not needed once the listener exists." } },
                { "id": "pl", "proposed": { "kind": "change" } },
                { "id": "nope", "proposed": { "kind": "remove" } },
            ],
        })),
    ]);
    let r = answer(ask(root.path(), "What would it take to send it right away?", &agent)).await.unwrap();
    assert_eq!(r.answer.added, ["pl", "p1", "p2"]);
    assert_eq!(r.answer.proposed, ["j1"]);
    let j1 = r.map.nodes.iter().find(|n| n.id == "j1").unwrap().proposed.clone().unwrap();
    assert_eq!((j1.kind.as_str(), j1.note.as_deref()), ("remove", Some("Not needed once the listener exists.")));
    assert_eq!(
        (r.report.summary.unconfirmed, r.report.summary.proposals.pending, r.report.summary.proposals.done),
        (0, 4, 0)
    );
    assert_eq!(r.answer.refs, refs(&[("pl", RefState::Proposed), ("j1", RefState::Confirmed)]));
    assert_eq!(
        r.answer.problems,
        [
            "Not marked as a proposal: \"Listen for conversions\" (new boxes and arrows carry their own `proposed`)",
            "Not marked as a proposal: \"nope\" isn't a confirmed box or arrow on this map",
        ]
    );
    assert!(agent.call(1).prompt.contains("draw it on the map as proposals"));
}

#[tokio::test]
async fn does_not_draw_a_box_proposed_as_new_when_the_code_already_has_it() {
    let root = copy_care_app();
    let mut exists = listener_box();
    exists["symbol"] = json!("src/automations/evaluate.ts#evaluateRules");
    let agent = ScriptedAgent::new(vec![
        needs_code("x"),
        reply(
            json!({ "concept": "Add a listener.", "refs": [], "add": { "nodes": [exists], "edges": listener_arrows() } }),
        ),
    ]);
    let r = answer(ask(root.path(), "q", &agent)).await.unwrap();
    assert!(r.answer.added.is_empty());
    assert_eq!(r.answer.problems[0], "Not drawn, pl was proposed as new but the code already has it");
    assert_eq!(r.map, care_map());
}

#[test]
fn answers_in_a_future_that_can_move_between_threads() {
    fn send<T: Send>(_: &T) {}
    let root = copy_care_app();
    let f = answer(ask(root.path(), "q", &ScriptedAgent::new(vec![])));
    send(&f);
}
