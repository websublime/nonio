//! A question about a different part of the app, over HTTP: the offer, a new map linked back to this one, or an answer
//! in words that draws nothing here. Ported from `packages/server/test/offers.test.ts`.

mod common;

use common::web::*;
use common::*;
use nonio_server::schemas::ANSWER;
use nonio_server::server::{start_server, ServerOptions};
use serde_json::{json, Value};

fn elsewhere() -> Reply {
    reply(json!({ "route": "elsewhere", "newQuestion": "How do invoices get sent?" }))
}

fn answers(tab: &Tab) -> Vec<Value> {
    tab.events().into_iter().filter(|e| e["type"] == "answer").map(|e| e["answer"].clone()).collect()
}

fn made(tab: &Tab, id: &str) -> Option<()> {
    tab.events()
        .iter()
        .any(|e| e["type"] == "job" && e["job"] == "make" && e["mapId"] == id && e["status"] == "done")
        .then_some(())
}

#[tokio::test]
async fn offers_a_new_map_and_makes_one_linked_back_or_answers_in_words_drawing_nothing_here() {
    let agent = ScriptedAgent::new(vec![
        reply(care_map_json()),                                             // the first map
        elsewhere(),                                                        // "and invoices?" on it
        reply(care_map_json()),                                             // the new map they said yes to
        elsewhere(),                                                        // asked again
        reply(json!({ "concept": "Nothing in this app sends invoices." })), // …and this time answered in words
    ]);
    let dir = copy_care_app();
    let server = start_server(ServerOptions::new(dir.path(), agent.clone())).await.unwrap();
    let tab = open_tab(server.port, &format!("t={}", server.token)).await.unwrap();

    // Offers a new map instead of answering, and makes one linked back when they say yes.
    let map_id =
        api(&server, "POST", "/api/maps", Some(json!({ "question": "What happens when a patient converts?" }))).await.1
            ["id"]
            .as_str()
            .unwrap()
            .to_string();
    wait_for(|| made(&tab, &map_id)).await;
    let ask = json!({ "question": "and invoices?", "selection": {} });
    api(&server, "POST", &format!("/api/maps/{map_id}/ask"), Some(ask.clone())).await;
    let offered = wait_for(|| answers(&tab).first().cloned()).await;
    assert_eq!(
        offered["offer"],
        json!({ "question": "How do invoices get sent?", "existing": null, "status": "pending", "mapId": null })
    );
    let offer = format!("/api/maps/{map_id}/answers/{}/offer", offered["id"].as_str().unwrap());
    assert_eq!(api(&server, "POST", &offer, Some(json!({ "choice": "maybe" }))).await.0, 400);
    let (status, yes) = api(&server, "POST", &offer, Some(json!({ "choice": "map" }))).await;
    assert_eq!(status, 200);
    let new_id = yes["id"].as_str().unwrap().to_string();
    wait_for(|| made(&tab, &new_id)).await;
    let child = api(&server, "GET", &format!("/api/maps/{new_id}"), None).await.1;
    assert_eq!(child["meta"]["question"], "How do invoices get sent?");
    assert_eq!(
        child["meta"]["parent"],
        json!({ "mapId": map_id, "question": "What happens when a patient converts?" })
    );
    let parent = api(&server, "GET", &format!("/api/maps/{map_id}"), None).await.1;
    // Saved, so a reload shows it.
    assert_eq!(
        (parent["answers"][0]["offer"]["status"].clone(), parent["answers"][0]["offer"]["mapId"].clone()),
        (json!("mapped"), json!(new_id))
    );
    assert!(tab
        .events()
        .iter()
        .any(|e| e["type"] == "answerUpdate" && e["mapId"] == map_id && e["answer"]["id"] == offered["id"]));
    // Only once.
    assert_eq!(api(&server, "POST", &offer, Some(json!({ "choice": "map" }))).await.0, 404);

    // Answers in words from the code when they say no, drawing nothing on this map.
    let before = api(&server, "GET", &format!("/api/maps/{map_id}"), None).await.1["map"].clone();
    api(&server, "POST", &format!("/api/maps/{map_id}/ask"), Some(ask)).await;
    let offered = wait_for(|| answers(&tab).get(1).cloned()).await;
    let offer = format!("/api/maps/{map_id}/answers/{}/offer", offered["id"].as_str().unwrap());
    assert_eq!(api(&server, "POST", &offer, Some(json!({ "choice": "words" }))).await.0, 200);
    let words = wait_for(|| answers(&tab).get(2).cloned()).await;
    assert_eq!(
        (
            words["question"].clone(),
            words["source"].clone(),
            words["concept"].clone(),
            words["added"].clone(),
            words["offer"].clone()
        ),
        (json!("and invoices?"), json!("code"), json!("Nothing in this app sends invoices."), json!([]), json!(null))
    );
    let last = agent.call(agent.count() - 1);
    assert!(std::sync::Arc::ptr_eq(&last.schema, &ANSWER.json));
    assert!(last.prompt.contains("different part of the app"));
    let after = api(&server, "GET", &format!("/api/maps/{map_id}"), None).await.1;
    assert_eq!(after["map"], before);
    let statuses: Vec<Value> =
        after["answers"].as_array().unwrap().iter().map(|a| a["offer"]["status"].clone()).collect();
    assert_eq!(statuses, vec![json!("mapped"), json!("answered"), json!(null)]);

    tab.close();
    server.close().await;
}
