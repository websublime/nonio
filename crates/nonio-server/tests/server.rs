//! The page server end to end over HTTP and WebSocket, with a scripted model: make a map, ask, approve a fix, undo.
//! Ported from `packages/server/test/server.test.ts`.

mod common;

use std::sync::Arc;

use common::web::*;
use common::*;
use nonio_server::server::{start_server, NonioServer, ServerOptions, TOKEN_HEADER};
use serde_json::{json, Value};

fn agent() -> Arc<ScriptedAgent> {
    ScriptedAgent::new(vec![
        reply(care_map_json()),
        // First question: the map-only try says it needs the code, then the model answers with it.
        needs_code("Who listens for the announcement."),
        reply(json!({
            "concept": "Nothing listens for the announcement, so the rules never run from here.",
            "refs": ["miss"],
            "moves": { "focus": "s4", "highlight": ["s4", "miss"] },
            "notes": [
                { "file": "src/patients/service.ts", "line": 6, "text": "bus.emit('patient.converted'", "note": "The announcement goes out here, with the patient id." },
                { "file": "src/patients/service.ts", "line": 2, "text": "bus.on(", "note": "Made up: no line says this." },
            ],
        })),
        // Second: a change request always needs the code.
        needs_code("The change itself."),
        reply(json!({ "concept": "Add a listener.", "refs": ["miss"], "fix": listener_fix() })),
    ])
}

async fn start(root: &std::path::Path, agent: Arc<ScriptedAgent>) -> NonioServer {
    start_server(ServerOptions::new(root, agent)).await.unwrap()
}

fn find(events: &[Value], f: impl Fn(&Value) -> bool) -> Option<Value> {
    events.iter().find(|e| f(e)).cloned()
}

#[tokio::test]
async fn makes_a_map_answers_on_it_and_applies_and_undoes_a_fix() {
    let dir = copy_care_app();
    let root = dir.path();
    let agent = agent();
    let server = start(root, agent.clone()).await;
    let tab = open_tab(server.port, &format!("t={}", server.token)).await.unwrap();

    // Makes a map from a question and streams progress.
    let (status, body) =
        api(&server, "POST", "/api/maps", Some(json!({ "question": "What happens when a patient converts?" }))).await;
    assert_eq!((status, body), (202, json!({ "id": "patient-converts" })));
    let map_id = "patient-converts";
    wait_for(|| find(&tab.events(), |e| e["type"] == "job" && e["job"] == "make" && e["status"] == "done")).await;
    let progress: Vec<String> = tab
        .events()
        .iter()
        .filter(|e| e["type"] == "progress")
        .map(|e| e["text"].as_str().unwrap().to_string())
        .collect();
    for p in [
        "Scanning the code for ways in, routes and tables",
        "Reading src/patients/service.ts",
        "15 of 15 claims confirmed",
    ] {
        assert!(progress.iter().any(|x| x == p), "{p} in {progress:?}");
    }
    let (_, got) = api(&server, "GET", &format!("/api/maps/{map_id}"), None).await;
    assert_eq!(
        got["report"]["summary"],
        json!({ "claims": 15, "confirmed": 15, "unconfirmed": 0, "proposals": { "pending": 0, "done": 0 } })
    );
    assert!(root.join(".nonio/maps/patient-converts.json").exists());
    let (_, list) = api(&server, "GET", "/api/maps", None).await;
    assert_eq!(
        list["maps"].as_array().unwrap().iter().map(|m| m["id"].clone()).collect::<Vec<_>>(),
        vec![json!(map_id)]
    );

    // Refuses a question longer than a map can hold, before starting.
    let long = format!("Why {}?", "really ".repeat(60));
    let (status, body) = api(&server, "POST", "/api/maps", Some(json!({ "question": long }))).await;
    assert_eq!(status, 400);
    assert!(body["error"].as_str().unwrap().contains("300 or fewer"));
    assert_eq!(api(&server, "GET", "/api/maps", None).await.1["making"], json!([]));

    // Answers a question about a selection.
    let ask = json!({ "question": "What does this do?", "selection": { "nodes": ["s4"] } });
    assert_eq!(api(&server, "POST", &format!("/api/maps/{map_id}/ask"), Some(ask)).await.0, 202);
    let a = wait_for(|| find(&tab.events(), |e| e["type"] == "answer")).await;
    assert_eq!(a["mapId"], map_id);
    assert_eq!(a["answer"]["refs"], json!([{ "id": "miss", "status": "confirmed" }]));
    assert_eq!(a["answer"]["moves"]["focus"], "s4");
    // The first question that needs the code starts the explainer session…
    assert_eq!(agent.call(2).resume, None);

    // Shows the code behind a box, and behind the notes on an answer, checked as it is served.
    let (status, code) = api(&server, "GET", &format!("/api/maps/{map_id}/code?item=s3"), None).await;
    assert_eq!(status, 200);
    assert_eq!((code["when"].clone(), code["missing"].clone()), (json!("now"), json!(0)));
    assert_eq!(code["root"], json!(std::fs::canonicalize(root).unwrap().to_string_lossy()));
    let block = &code["blocks"][0];
    assert_eq!(
        (block["file"].clone(), block["name"].clone(), block["whole"].clone(), block["cited"].clone()),
        (json!("src/patients/service.ts"), json!("markConverted"), json!(true), json!([5]))
    );
    assert_eq!(api(&server, "GET", &format!("/api/maps/{map_id}/code?item=nope"), None).await.0, 404);
    // Not a map of a change.
    assert_eq!(api(&server, "GET", &format!("/api/maps/{map_id}/code?item=s3&when=before"), None).await.0, 404);
    let (_, got) = api(&server, "GET", &format!("/api/maps/{map_id}"), None).await;
    let first = &got["answers"][0];
    // The note whose line isn't in the code was dropped when the answer came in, and the answer says so.
    assert_eq!(
        first["notes"],
        json!([{ "file": "src/patients/service.ts", "line": 6, "text": "bus.emit('patient.converted'", "note": "The announcement goes out here, with the patient id." }])
    );
    let problems = first["problems"].as_array().unwrap();
    assert_eq!(problems.len(), 1);
    assert!(problems[0].as_str().unwrap().contains("A note on the code wasn't shown"), "{problems:?}");
    let answer_id = first["id"].as_str().unwrap();
    let (_, code) = api(&server, "GET", &format!("/api/maps/{map_id}/answers/{answer_id}/code"), None).await;
    let block = &code["blocks"][0];
    assert_eq!((block["name"].clone(), block["cited"].clone()), (json!("markConverted"), json!([6])));
    assert_eq!(block["notes"][0]["line"], 6);
    assert_eq!(block["notes"][0]["note"], "The announcement goes out here, with the patient id.");
    assert_eq!(api(&server, "GET", &format!("/api/maps/{map_id}/answers/ans-none/code"), None).await.0, 404);

    // Proposes a fix, applies it only on approval, re-checks the map, and undoes it.
    let ask = json!({ "question": "Make the email go out right away", "selection": { "absences": ["miss"] } });
    api(&server, "POST", &format!("/api/maps/{map_id}/ask"), Some(ask)).await;
    let a = wait_for(|| tab.events().into_iter().filter(|e| e["type"] == "answer").nth(1)).await;
    // …and later ones continue it.
    assert_eq!(agent.call(4).resume.as_deref(), Some("session-3"));
    let fix_id = a["answer"]["fix"]["id"].as_str().unwrap().to_string();
    let listener = root.join("src/automations/listeners.ts");
    assert!(!listener.exists());
    let (status, ok) = api(&server, "POST", &format!("/api/maps/{map_id}/fixes/{fix_id}/approve"), None).await;
    assert_eq!(status, 200, "{ok}");
    assert_eq!(
        (ok["edgeId"].clone(), ok["closed"].clone(), ok["files"].clone(), ok["summary"]["unconfirmed"].clone()),
        (json!("fx1"), json!(["miss"]), json!(["src/automations/listeners.ts"]), json!(0))
    );
    assert!(listener.exists());
    let (_, after) = api(&server, "GET", &format!("/api/maps/{map_id}"), None).await;
    assert!(after["map"]["edges"].as_array().unwrap().iter().any(|e| e["id"] == "fx1"));
    assert_eq!(after["map"]["absences"], json!([]));
    // A reload can still offer Undo.
    assert_eq!(
        after["fixes"],
        json!({ &fix_id: { "status": "applied", "edgeId": "fx1", "closed": ["miss"], "undoable": true } })
    );
    assert_eq!(api(&server, "POST", &format!("/api/maps/{map_id}/fixes/{fix_id}/undo"), None).await.0, 200);
    assert!(!listener.exists());
    let (_, undone) = api(&server, "GET", &format!("/api/maps/{map_id}"), None).await;
    assert_eq!(
        undone["map"]["absences"].as_array().unwrap().iter().map(|x| x["id"].clone()).collect::<Vec<_>>(),
        vec![json!("miss")]
    );
    assert_eq!(undone["report"]["absences"]["miss"]["status"], "confirmed");
    // Undone fixes can be approved again.
    assert_eq!(undone["fixes"], json!({ &fix_id: { "status": "proposed" } }));

    // Keeps the questions, answers and fixes across a restart; Undo only lasts while the server runs.
    let again = start(root, agent.clone()).await;
    let before = api(&again, "GET", &format!("/api/maps/{map_id}"), None).await.1;
    let asked: Vec<(Value, Value)> =
        before["answers"].as_array().unwrap().iter().map(|a| (a["question"].clone(), a["source"].clone())).collect();
    assert_eq!(
        asked,
        vec![(json!("What does this do?"), json!("code")), (json!("Make the email go out right away"), json!("code"))]
    );
    // It was undone before the restart, so it can be approved again.
    assert_eq!(before["fixes"], json!({ &fix_id: { "status": "proposed" } }));
    assert_eq!(api(&again, "POST", &format!("/api/maps/{map_id}/fixes/{fix_id}/approve"), None).await.0, 200);
    let now = api(&again, "GET", &format!("/api/maps/{map_id}"), None).await.1;
    assert_eq!(
        (now["fixes"][&fix_id]["status"].clone(), now["fixes"][&fix_id]["undoable"].clone()),
        (json!("applied"), json!(true))
    );
    again.close().await;
    let third = start(root, agent.clone()).await;
    let now = api(&third, "GET", &format!("/api/maps/{map_id}"), None).await.1;
    assert_eq!(
        (now["fixes"][&fix_id]["status"].clone(), now["fixes"][&fix_id]["undoable"].clone()),
        (json!("applied"), json!(false))
    );
    third.close().await;

    tab.close();
    server.close().await;
}

#[tokio::test]
async fn refuses_requests_without_the_token_from_other_hosts_or_origins_and_sockets_without_the_token() {
    let dir = copy_care_app();
    let server = start(dir.path(), ScriptedAgent::new(vec![])).await;
    assert_eq!(raw(server.port, "GET", "/api/maps", &[], None).await.status, 401);
    let evil = raw(
        server.port,
        "POST",
        "/api/maps",
        &[(TOKEN_HEADER, &server.token), ("origin", "https://evil.example")],
        Some(&json!({ "question": "x" })),
    )
    .await;
    assert_eq!(evil.status, 403);
    let wrong_host =
        raw(server.port, "GET", "/api/maps", &[("host", "evil.example"), (TOKEN_HEADER, &server.token)], None).await;
    assert_eq!(wrong_host.status, 403);
    assert!(open_tab(server.port, "t=wrong").await.is_err());
    server.close().await;
}

#[tokio::test]
async fn explains_how_to_build_the_viewer_when_it_is_missing() {
    let dir = copy_care_app();
    let server = start(dir.path(), ScriptedAgent::new(vec![])).await;
    let page = raw(server.port, "GET", &format!("/m/some-map?t={}", server.token), &[], None).await;
    assert!(page.text.contains("npm run build"), "{}", page.text);
    server.close().await;
}

#[tokio::test]
async fn serves_the_viewer_its_assets_and_index_for_everything_else() {
    let dir = copy_care_app();
    let viewer = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(viewer.path().join("assets")).unwrap();
    std::fs::write(viewer.path().join("index.html"), "<!doctype html>the viewer").unwrap();
    std::fs::write(viewer.path().join("assets/app.js"), "console.log(1)").unwrap();
    let mut o = ServerOptions::new(dir.path(), ScriptedAgent::new(vec![]));
    o.viewer = Some(viewer.path().to_path_buf());
    let server = start_server(o).await.unwrap();
    let page = raw(server.port, "GET", "/m/a-map", &[], None).await;
    assert!(page.text.contains("the viewer"));
    assert!(page.headers.to_ascii_lowercase().contains("content-security-policy: default-src 'self'"));
    let js = raw(server.port, "GET", "/assets/app.js", &[], None).await;
    assert_eq!(js.text, "console.log(1)");
    assert!(js.headers.contains("text/javascript"));
    // Nothing outside the viewer's assets is served: the page comes back instead.
    for path in ["/assets/../../etc/passwd", "/assets/%2e%2e/index.html", "/Cargo.toml"] {
        assert!(raw(server.port, "GET", path, &[], None).await.text.contains("the viewer"), "{path}");
    }
    assert_eq!(raw(server.port, "POST", "/m/a-map", &[], None).await.status, 405);
    server.close().await;
}
