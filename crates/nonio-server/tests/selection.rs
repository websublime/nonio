//! What a map's page reports about itself (what is selected, which map is open), and a question asked in the agent's
//! chat that leans on it. Memory only; a scripted model answers. Ported from `packages/server/test/selection.test.ts`
//! (the comparison with the viewer's own wording of a step waits for the viewer).

mod common;

use std::sync::{Arc, Mutex};

use common::web::*;
use common::*;
use nonio_server::host::{Asks, PageAgent};
use nonio_server::server::{start_server, NonioServer, ServerEvent, ServerOptions, TOKEN_HEADER};
use nonio_server::step::ask_about_step;
use serde_json::{json, Value};

const CODEX: PageAgent = PageAgent { name: "Codex", asks: Asks::Chat };

async fn codex_server(
    agent: Arc<dyn nonio_server::Agent>,
    events: Arc<Mutex<Vec<Value>>>,
) -> (tempfile::TempDir, NonioServer) {
    let dir = copy_care_app();
    let mut o = ServerOptions::new(dir.path(), agent);
    o.page = CODEX;
    o.on_event = Some(Arc::new(move |e: &ServerEvent| events.lock().unwrap().push(serde_json::to_value(e).unwrap())));
    (dir, start_server(o).await.unwrap())
}

fn done(events: &Mutex<Vec<Value>>, n: usize) -> Option<()> {
    (events.lock().unwrap().iter().filter(|e| e["type"] == "job" && e["status"] == "done").count() >= n).then_some(())
}

#[tokio::test]
async fn a_map_page_reporting_what_is_selected() {
    let agent = ScriptedAgent::new(vec![
        reply(care_map_json()),
        reply(
            json!({ "route": "answer", "concept": "It tells the rest of the app the patient converted.", "refs": ["s4"] }),
        ),
        reply(
            json!({ "route": "answer", "concept": "This step is where the welcome email should start.", "refs": [] }),
        ),
    ]);
    let events = Arc::new(Mutex::new(Vec::new()));
    let (_dir, server) = codex_server(agent.clone(), events.clone()).await;
    server.start_map("What happens when a patient converts?").unwrap();
    wait_for(|| done(&events, 1)).await;

    // Tells the page who does the work and where questions are asked.
    assert_eq!(
        api(&server, "GET", "/api/maps/patient-converts", None).await.1["agent"],
        json!({ "name": "Codex", "asks": "chat" })
    );
    assert_eq!(api(&server, "GET", "/api/maps", None).await.1["agent"], json!({ "name": "Codex", "asks": "chat" }));

    // Knows which map was open last: the one a page connected to.
    assert_eq!(server.last_seen(), None);
    let tab = open_tab(server.port, &format!("t={}&m=patient-converts", server.token)).await.unwrap();
    wait_for(|| server.last_seen()).await;
    assert_eq!(server.last_seen().unwrap().map_id, "patient-converts");
    tab.close();

    // Keeps the selection, only with the token and for a map that exists.
    let put = |headers: Vec<(&'static str, String)>, path: &'static str, body: Value| {
        let port = server.port;
        async move {
            let h: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
            raw(port, "PUT", path, &h, Some(&body)).await
        }
    };
    let token = server.token.clone();
    let sel = json!({ "selection": { "nodes": ["s4"] } });
    assert_eq!(
        put(vec![(TOKEN_HEADER, "nope".into())], "/api/maps/patient-converts/selection", sel.clone()).await.status,
        401
    );
    assert_eq!(
        put(
            vec![(TOKEN_HEADER, token.clone()), ("origin", "http://evil.example".into())],
            "/api/maps/patient-converts/selection",
            sel
        )
        .await
        .status,
        403
    );
    assert_eq!(
        put(vec![(TOKEN_HEADER, token.clone())], "/api/maps/no-such-map/selection", json!({ "selection": {} }))
            .await
            .status,
        404
    );
    let kept = put(
        vec![(TOKEN_HEADER, token.clone())],
        "/api/maps/patient-converts/selection",
        json!({ "selection": { "nodes": ["s4", 7] } }),
    )
    .await;
    assert_eq!(kept.json(), json!({ "ok": true }));

    // Answers a question asked in the chat about what is selected on the page.
    let asked = server.ask_from_chat("What does this do?", None).unwrap();
    assert_eq!((asked.map_id.as_str(), asked.question.as_str()), ("patient-converts", "What does this do?"));
    assert_eq!(asked.selected.len(), 1);
    wait_for(|| done(&events, 2)).await;
    let call = agent.call(1);
    assert!(call.prompt.contains("What does this do?"));
    assert!(call.prompt.contains(&format!("box \"{}\"", asked.selected[0])));
    let saved = api(&server, "GET", "/api/maps/patient-converts", None).await.1;
    let last = saved["answers"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(
        (last["question"].clone(), last["selection"].clone()),
        (json!("What does this do?"), json!({ "nodes": ["s4"] }))
    );
    // Who is working is named on the page's progress and errors.
    assert!(events
        .lock()
        .unwrap()
        .iter()
        .any(|e| e["type"] == "progress" && e["text"] == "Codex is exploring the code"));

    // Asks about the walkthrough stop on screen when nothing is selected.
    put(
        vec![(TOKEN_HEADER, token.clone())],
        "/api/maps/patient-converts/selection",
        json!({ "selection": {}, "step": "The announcement" }),
    )
    .await;
    let asked = server.ask_from_chat("Why is this here?", Some("patient-converts")).unwrap();
    assert_eq!(asked.question, ask_about_step("The announcement", "Why is this here?"));
    assert_eq!(asked.question, "About the walkthrough step “The announcement”: Why is this here?");
    wait_for(|| done(&events, 3)).await;

    // Says so when there is no such map.
    let err = server.ask_from_chat("Why?", Some("no-such-map")).unwrap_err();
    assert!(err.message.contains("There is no map \"no-such-map\""), "{err}");

    server.close().await;
}

#[tokio::test]
async fn tells_the_page_who_asks_while_the_map_is_still_being_made() {
    let (_dir, making) = codex_server(Arc::new(Silent), Arc::new(Mutex::new(Vec::new()))).await;
    let id = making.start_map("Where does the email go?").unwrap();
    let body = api(&making, "GET", &format!("/api/maps/{id}"), None).await.1;
    assert_eq!(body["map"], json!(null));
    assert_ne!(body["job"], json!(null));
    assert_eq!(body["agent"], json!({ "name": "Codex", "asks": "chat" }));
    making.close().await;
}

#[tokio::test]
async fn moves_to_the_next_port_when_the_one_it_wants_is_taken() {
    let taken = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = taken.local_addr().unwrap().port();
    let dir = copy_care_app();
    let mut o = ServerOptions::new(dir.path(), ScriptedAgent::new(vec![]));
    o.port = port;
    let s = start_server(o).await.unwrap();
    assert!(s.port > port && s.port <= port + 10, "{} after {port}", s.port);
    s.close().await;
}
