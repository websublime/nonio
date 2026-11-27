//! nonio as an MCP server, driven the way the agents drive it: an MCP client calls the tools, and the test plays the
//! model by submitting output. No model is called. Ported from "the channel" and "the channel in Codex" in
//! `packages/server/test/channel.test.ts` (the bridge and `partsOf` tests are in bridge.rs and request.rs). What
//! differs: the Claude Code stand-in names itself `claude-code`, since nonio gives a client it doesn't know the base
//! protocol (D07), and the update notice waits for nonio's distribution.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::web::{raw, wait_for};
use common::*;
use nonio_core::screens::CapturedElement;
use nonio_server::channel::{Channel, ChannelOptions};
use nonio_server::host::{CLAUDE_CODE_CLIENT, CODEX, CODEX_CLIENT};
use nonio_server::screens::{Shooter, Shot};
use nonio_server::server::TOKEN_HEADER;
use regex::Regex;
use rmcp::model::{
    CallToolRequestParams, CancelledNotificationParam, ClientCapabilities, ClientConfig, ClientNotification,
    ClientRequest, CustomNotification, Implementation, ProgressNotificationParam, Request, ServerResult,
};
use rmcp::service::{NotificationContext, PeerRequestOptions, RunningService};
use rmcp::{ClientHandler, RoleClient, ServiceExt};
use serde_json::{json, Map, Value};

#[derive(Clone, Default)]
struct Seen {
    pushed: Arc<Mutex<Vec<Value>>>,
    progress: Arc<Mutex<Vec<f64>>>,
}

struct StandIn {
    name: String,
    seen: Seen,
}

impl ClientHandler for StandIn {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::new(ClientCapabilities::default(), Implementation::new(self.name.clone(), "0"))
    }

    async fn on_custom_notification(&self, n: CustomNotification, _: NotificationContext<RoleClient>) {
        if n.method == "notifications/claude/channel" {
            self.seen.pushed.lock().unwrap().push(n.params.unwrap_or(Value::Null));
        }
    }

    async fn on_progress(&self, p: ProgressNotificationParam, _: NotificationContext<RoleClient>) {
        self.seen.progress.lock().unwrap().push(p.progress);
    }
}

struct Session {
    client: RunningService<RoleClient, StandIn>,
    channel: Channel,
    seen: Seen,
}

struct Called {
    text: String,
    is_error: bool,
    content: Vec<Value>,
}

impl Session {
    async fn start(o: ChannelOptions, name: &str) -> Session {
        let channel = Channel::new(o);
        let (a, b) = tokio::io::duplex(1 << 20);
        let server = channel.clone();
        tokio::spawn(async move {
            if let Ok(s) = server.serve(a).await {
                let _ = s.waiting().await;
            }
        });
        let seen = Seen::default();
        let client = StandIn { name: name.into(), seen: seen.clone() }.serve(b).await.unwrap();
        Session { client, channel, seen }
    }

    async fn call(&self, name: &str, args: Value) -> Called {
        let args: Map<String, Value> = match args {
            Value::Object(o) => o,
            _ => Map::new(),
        };
        let r = self
            .client
            .peer()
            .call_tool(CallToolRequestParams::new(name.to_string()).with_arguments(args))
            .await
            .unwrap();
        read(serde_json::to_value(&r).unwrap())
    }

    fn instructions(&self) -> String {
        self.client.peer().peer_info().and_then(|i| i.instructions.clone()).unwrap_or_default()
    }

    async fn tool_names(&self) -> Vec<String> {
        self.client.peer().list_all_tools().await.unwrap().iter().map(|t| t.name.to_string()).collect()
    }

    async fn close(self) {
        let _ = self.client.cancel().await;
        self.channel.close().await;
    }
}

fn read(v: Value) -> Called {
    let content = v["content"].as_array().cloned().unwrap_or_default();
    let text = content.iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n");
    Called { text, is_error: v["isError"].as_bool().unwrap_or(false), content }
}

fn request_id(text: &str) -> String {
    Regex::new(r"nonio request (r\d+)").unwrap().captures(text).map(|c| c[1].to_string()).unwrap_or_default()
}

fn opened() -> (Arc<Mutex<Vec<String>>>, nonio_server::channel::OpenFn) {
    let opened = Arc::new(Mutex::new(Vec::new()));
    let o = opened.clone();
    (opened, Arc::new(move |u: &str| o.lock().unwrap().push(u.to_string())))
}

fn options(root: &std::path::Path) -> ChannelOptions {
    let mut o = ChannelOptions::new(root);
    o.port = 0;
    o.forced_host = None;
    o.open = Some(Arc::new(|_: &str| {}));
    o
}

/// The page server's origin and token, from a map link.
fn page_of(url: &str) -> (u16, String, String) {
    let u = url::Url::parse(url).unwrap();
    let token = u.query_pairs().find(|(k, _)| k == "t").unwrap().1.to_string();
    let map_id = u.path().rsplit('/').next().unwrap().to_string();
    (u.port().unwrap(), token, map_id)
}

async fn ask_on_the_map(url: &str, question: &str) {
    let (port, token, map_id) = page_of(url);
    let r = raw(
        port,
        "POST",
        &format!("/api/maps/{map_id}/ask"),
        &[(TOKEN_HEADER, &token)],
        Some(&json!({ "question": question })),
    )
    .await;
    assert_eq!(r.status, 202, "{}", r.text);
}

async fn saved_map(url: &str) -> Value {
    let (port, token, map_id) = page_of(url);
    raw(port, "GET", &format!("/api/maps/{map_id}"), &[(TOKEN_HEADER, &token)], None).await.json()
}

fn stand_in_shooter() -> Shooter {
    // No browser in tests: a stand-in that "sees" the convert button.
    Arc::new(|url: String| {
        Box::pin(async move {
            Ok(Shot {
                png: format!("shot of {url}").into_bytes(),
                title: "Patients".into(),
                text: "Patients Mark as converted".into(),
                elements: vec![CapturedElement {
                    tag: "button".into(),
                    text: "Mark as converted".into(),
                    x: 640.0,
                    y: 400.0,
                    w: 128.0,
                    h: 40.0,
                }],
            })
        })
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn the_channel_in_claude_code() {
    let dir = copy_care_app();
    let root = dir.path();
    let (opened, open) = opened();
    let mut o = options(root);
    o.open = Some(open);
    o.wait = Some(Duration::from_secs(5));
    o.heartbeat = Duration::from_millis(20);
    o.shoot = Some(stand_in_shooter());
    let s = Session::start(o, CLAUDE_CODE_CLIENT).await;

    // Tells Claude Code it is a channel with tools, and how to use them.
    let info = s.client.peer().peer_info().unwrap();
    assert!(info.capabilities.experimental.as_ref().is_some_and(|e| e.contains_key("claude/channel")));
    assert!(s.instructions().contains("nonio_submit"));
    assert!(s.instructions().contains("call nonio_listen and leave it running"));
    assert_eq!(
        s.tool_names().await,
        [
            "nonio_map",
            "nonio_next",
            "nonio_submit",
            "nonio_fail",
            "nonio_listen",
            "nonio_screens",
            "nonio_capture",
            "nonio_open"
        ]
    );

    // Makes a map in one turn: the first request comes back from nonio_map, the result from nonio_submit.
    let first = s.call("nonio_map", json!({ "question": "What happens when a patient converts?" })).await;
    assert!(!first.is_error, "{}", first.text);
    let map_url = {
        let o = opened.lock().unwrap();
        assert_eq!(o.len(), 1);
        o[0].clone()
    };
    assert!(first.text.contains(&map_url));
    assert!(first.text.contains("Making the map for \"What happens when a patient converts?\""), "{}", first.text);
    assert!(first.text.contains("## Output schema"));
    let id = request_id(&first.text);
    let wrong = s.call("nonio_submit", json!({ "request_id": id, "output": { "version": 1 } })).await;
    assert!(wrong.is_error);
    assert!(wrong.text.contains("Fix these"));
    let done = s.call("nonio_submit", json!({ "request_id": id, "output": care_map_json() })).await;
    assert!(done.text.contains("15 of 15 claims confirmed"), "{}", done.text);
    assert!(done.text.contains(map_url.split('?').next().unwrap()));
    assert!(done.text.contains("call nonio_listen"));
    assert!(s.seen.pushed.lock().unwrap().is_empty());

    // Hands a question asked on the map to the listening call, keeping it alive meanwhile, and the answer goes back.
    let peer = s.client.peer().clone();
    let listening = peer
        .send_cancellable_request(
            ClientRequest::CallToolRequest(Request::new(CallToolRequestParams::new("nonio_listen"))),
            PeerRequestOptions::no_options(),
        )
        .await
        .unwrap();
    wait_for(|| (s.seen.progress.lock().unwrap().len() >= 2).then_some(())).await;
    ask_on_the_map(&map_url, "Where does the welcome email start?").await;
    let ServerResult::CallToolResult(r) = listening.await_response().await.unwrap() else {
        panic!("not a tool result")
    };
    let req = read(serde_json::to_value(&r).unwrap()).text;
    assert!(req.contains("Answering \"Where does the welcome email start?\" from the map"), "{req}");
    assert!(req.contains("call nonio_listen again"));
    assert!(s.seen.pushed.lock().unwrap().is_empty());
    let done = s
        .call(
            "nonio_submit",
            json!({ "request_id": request_id(&req), "output": { "route": "answer", "concept": "It starts when a patient converts.", "refs": [] } }),
        )
        .await;
    assert!(done.text.contains("is shown on the map. Call nonio_listen again"), "{}", done.text);
    let saved = saved_map(&map_url).await;
    assert_eq!(saved["answers"].as_array().unwrap().last().unwrap()["concept"], "It starts when a patient converts.");

    // Stops listening when Claude Code cancels the call.
    let listening = peer
        .send_cancellable_request(
            ClientRequest::CallToolRequest(Request::new(CallToolRequestParams::new("nonio_listen"))),
            PeerRequestOptions::no_options(),
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    let cancelled = CancelledNotificationParam::new(Some(listening.id.clone()), Some("cancelled".into()));
    peer.send_notification(ClientNotification::CancelledNotification(rmcp::model::Notification::new(cancelled)))
        .await
        .unwrap();
    drop(listening);
    tokio::time::sleep(Duration::from_millis(30)).await;
    // Nothing is listening any more, so a question asked now is announced instead of vanishing into the cancelled call.
    ask_on_the_map(&map_url, "Who sends the reminder?").await;
    wait_for(|| (!s.seen.pushed.lock().unwrap().is_empty()).then_some(())).await;
    assert!(s.seen.pushed.lock().unwrap()[0]["content"].as_str().unwrap().contains("Who sends the reminder?"));
    let req = s.call("nonio_next", json!({})).await;
    let done = s
        .call("nonio_submit", json!({ "request_id": request_id(&req.text), "output": { "route": "answer", "concept": "The nightly job.", "refs": [] } }))
        .await;
    assert!(done.text.contains("is shown on the map"), "{}", done.text);
    s.seen.pushed.lock().unwrap().clear();

    // Pushes a question asked on the map into a channel session when nothing is listening, and the answer goes back.
    ask_on_the_map(&map_url, "Why does no email go out?").await;
    wait_for(|| (!s.seen.pushed.lock().unwrap().is_empty()).then_some(())).await;
    let pushed = s.seen.pushed.lock().unwrap()[0].clone();
    assert!(Regex::new(r"^r\d+$").unwrap().is_match(pushed["meta"]["request_id"].as_str().unwrap()));
    assert!(pushed["content"].as_str().unwrap().contains("Answering \"Why does no email go out?\" from the map"));
    let req = s.call("nonio_next", json!({})).await;
    assert!(req.text.contains("Answer from what is in this request alone"));
    let output = json!({ "route": "answer", "concept": "Nothing listens for the announcement, so the rules never run.", "refs": ["miss"] });
    let done = s.call("nonio_submit", json!({ "request_id": request_id(&req.text), "output": output })).await;
    assert!(done.text.contains("is shown on the map"));
    let saved = saved_map(&map_url).await;
    assert!(saved["answers"].as_array().unwrap().last().unwrap()["concept"]
        .as_str()
        .unwrap()
        .contains("Nothing listens"));

    // Lists what the repo can show, captures a page for the model to look at, and serves it to the map.
    std::fs::write(root.join("demo.html"), "<button>Mark as converted</button>").unwrap();
    assert!(s.call("nonio_screens", json!({})).await.text.contains("  - demo.html"));
    assert!(s
        .call("nonio_capture", json!({ "url": "https://example.com/" }))
        .await
        .text
        .contains("Only pages served on this machine"));
    assert!(s.call("nonio_capture", json!({ "file": ".env" })).await.is_error);
    let r = s.call("nonio_capture", json!({ "file": "demo.html" })).await;
    let id = Regex::new(r"Captured as (scr-[0-9a-f]+)").unwrap().captures(&r.text).map(|c| c[1].to_string()).unwrap();
    assert!(r.text.contains("button \"Mark as converted\""));
    assert_eq!((r.content[1]["type"].clone(), r.content[1]["mimeType"].clone()), (json!("image"), json!("image/png")));
    use base64::Engine;
    let png = base64::engine::general_purpose::STANDARD.decode(r.content[1]["data"].as_str().unwrap()).unwrap();
    assert!(String::from_utf8_lossy(&png).contains("shot of file://"));
    // The box whose code says "Mark as converted" may carry it; the checker works out where the button is.
    let mut map = care_map_json();
    map["nodes"][0]["screen"] = json!({ "kind": "capture", "capture": id, "highlight": "Mark as converted" });
    let parsed = nonio_core::parse_map(&map).unwrap();
    let first_node = map["nodes"][0]["id"].as_str().unwrap().to_string();
    let report = check_map(&parsed, root);
    let checked = serde_json::to_value(&report.screens[&first_node]).unwrap();
    assert_eq!(checked["status"], "confirmed", "{checked}");
    assert_eq!(checked["show"]["kind"], "capture");
    assert_eq!(checked["show"]["from"], "demo.html");
    assert_eq!((checked["show"]["box"]["x"].clone(), checked["show"]["box"]["y"].clone()), (json!(0.5), json!(0.5)));
    let (port, token, _) = page_of(&map_url);
    let img = raw(port, "GET", &format!("/api/screens/{id}.png?t={token}"), &[], None).await;
    assert_eq!(img.status, 200);

    // Opens saved maps and lists them.
    let r = s.call("nonio_open", json!({})).await;
    assert!(r.text.contains("patient-converts  What happens when a patient converts?"), "{}", r.text);
    assert_eq!(s.call("nonio_next", json!({})).await.text, "Nothing is waiting right now.");

    s.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn returns_from_a_quiet_listening_call_before_claude_code_drops_it_and_stops_after_enough_quiet_ones() {
    let dir = copy_care_app();
    let mut o = options(dir.path());
    o.listen = Some(Duration::from_millis(20));
    o.quiet_listens = 3;
    let s = Session::start(o, CLAUDE_CODE_CLIENT).await;
    s.call("nonio_open", json!({})).await;
    assert!(s
        .call("nonio_listen", json!({}))
        .await
        .text
        .contains("Nothing was asked on the map yet. Call nonio_listen again"));
    assert!(s.call("nonio_listen", json!({})).await.text.contains("Call nonio_listen again"));
    assert!(s.call("nonio_listen", json!({})).await.text.contains("stopped listening"));
    // Asked to listen again after that, it listens afresh.
    assert!(s.call("nonio_listen", json!({})).await.text.contains("Call nonio_listen again"));
    s.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn ends_listening_when_the_person_stops_nonio_and_doesnt_listen_while_it_is_closed() {
    let dir = copy_care_app();
    let s = Session::start(options(dir.path()), CLAUDE_CODE_CLIENT).await;
    assert!(s.call("nonio_listen", json!({})).await.text.contains("closed nonio"));
    let opened = s.call("nonio_open", json!({})).await.text;
    let url = Regex::new(r"Opened (\S+)").unwrap().captures(&opened).unwrap()[1].to_string();
    let peer = s.client.peer().clone();
    let listening = tokio::spawn(async move {
        peer.call_tool(CallToolRequestParams::new("nonio_listen"))
            .await
            .map(|r| read(serde_json::to_value(&r).unwrap()).text)
    });
    tokio::time::sleep(Duration::from_millis(30)).await;
    let (port, token, _) = page_of(&url);
    let stop = raw(port, "POST", "/api/stop", &[(TOKEN_HEADER, &token)], Some(&json!({}))).await;
    assert_eq!(stop.status, 200);
    assert!(listening.await.unwrap().unwrap().contains("Don't call nonio_listen again"));
    assert!(s.call("nonio_listen", json!({})).await.text.contains("closed nonio"));
    s.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_channel_in_codex() {
    let dir = copy_care_app();
    let root = dir.path().to_string_lossy().to_string();
    let (opened, open) = opened();
    let mut o = options(std::path::Path::new("/"));
    o.open = Some(open);
    // Parts kept small, so the map's request comes in several.
    o.part_chars = Some(6000);
    let s = Session::start(o, CODEX_CLIENT).await;

    // Gives Codex its tools: no listening, asking from the chat, nothing that needs approval.
    assert!(s.instructions().contains("nonio_ask"));
    assert!(!s.instructions().contains("call nonio_listen"));
    let tools = s.client.peer().list_all_tools().await.unwrap();
    assert_eq!(
        tools.iter().map(|t| t.name.to_string()).collect::<Vec<_>>(),
        [
            "nonio_map",
            "nonio_next",
            "nonio_submit",
            "nonio_fail",
            "nonio_ask",
            "nonio_more",
            "nonio_screens",
            "nonio_capture",
            "nonio_open"
        ]
    );
    for t in &tools {
        let a = t.annotations.as_ref().unwrap();
        assert_eq!((a.destructive_hint, a.open_world_hint), (Some(false), Some(false)), "{}", t.name);
    }
    let read_only: Vec<String> = tools
        .iter()
        .filter(|t| t.annotations.as_ref().unwrap().read_only_hint == Some(true))
        .map(|t| t.name.to_string())
        .collect();
    assert_eq!(read_only, ["nonio_next", "nonio_more", "nonio_screens"]);
    // Every call returns well inside Codex's tool timeout.
    assert!(CODEX.max_wait <= Duration::from_secs(45));

    // Makes a map without opening a browser, its request in parts, and says to open the link beside the chat.
    let first = s.call("nonio_map", json!({ "question": "What happens when a patient converts?", "repo": root })).await;
    assert!(opened.lock().unwrap().is_empty());
    let map_url = Regex::new(r"Give the person this link: (\S+)")
        .unwrap()
        .captures(&first.text)
        .map(|c| c[1].to_string())
        .unwrap();
    assert!(Regex::new(r"^http://127\.0\.0\.1:\d+/m/patient-converts\?t=").unwrap().is_match(&map_url), "{map_url}");
    assert!(first.text.contains("built-in browser"));
    let id = request_id(&first.text);
    let total: usize = Regex::new(r"part 1 of (\d+)").unwrap().captures(&first.text).unwrap()[1].parse().unwrap();
    assert!(total > 1);
    let mut pages = vec![first.text.clone()];
    for _ in 2..=total {
        pages.push(s.call("nonio_more", json!({ "request_id": id })).await.text);
    }
    assert!(pages.last().unwrap().contains("This is the last part."));
    assert!(pages.join("\n").contains("## Output schema"));
    assert!(pages.last().unwrap().trim_end().ends_with('}'));
    for p in &pages[1..] {
        assert!(p.chars().count() < 6200);
    }
    assert!(s.call("nonio_more", json!({ "request_id": id, "part": total + 1 })).await.is_error);
    let done = s.call("nonio_submit", json!({ "request_id": id, "output": care_map_json() })).await;
    assert!(done.text.contains("15 of 15 claims confirmed"), "{}", done.text);
    assert!(done.text.contains("nonio_ask"));
    assert!(!done.text.contains("nonio_listen"));
    assert_eq!(saved_map(&map_url).await["agent"], json!({ "name": "Codex", "asks": "chat" }));

    // Answers a question asked in the Codex chat about what is selected on the page, in the same turn.
    let (port, token, _) = page_of(&map_url);
    let put = raw(
        port,
        "PUT",
        "/api/maps/patient-converts/selection",
        &[(TOKEN_HEADER, &token)],
        Some(&json!({ "selection": { "nodes": ["s4"] } })),
    )
    .await;
    assert_eq!(put.status, 200);
    // No URL given: the map whose page reported last.
    let req = s.call("nonio_ask", json!({ "question": "Why does no email go out?" })).await;
    assert!(req.text.contains("Asked on the map \"patient-converts\" about"), "{}", req.text);
    assert!(req.text.contains("Answering \"Why does no email go out?\" from the map"));
    let output = json!({
        "route": "answer", "concept": "Nothing listens for the announcement.", "refs": ["miss"],
        "notes": [{ "file": "src/patients/service.ts", "line": 6, "text": "bus.emit('patient.converted'", "note": "The announcement goes out here." }],
    });
    let done = s.call("nonio_submit", json!({ "request_id": request_id(&req.text), "output": output })).await;
    // The checked answer comes back for Codex to post in its chat, with the line its note points at, read now.
    for want in [
        "Post the reply between the lines",
        "Nothing listens for the announcement.",
        "`src/patients/service.ts` · markConverted",
        "```ts\n",
        "6    bus.emit('patient.converted', { patientId });",
        "- Line 6: The announcement goes out here.",
    ] {
        assert!(done.text.contains(want), "{want} in {}", done.text);
    }
    assert!(!done.text.contains("nonio_listen"));
    let saved = saved_map(&map_url).await;
    let last = saved["answers"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(
        (last["concept"].clone(), last["selection"].clone()),
        (json!("Nothing listens for the announcement."), json!({ "nodes": ["s4"] }))
    );
    // With the page's URL, the map it names.
    let again = s.call("nonio_ask", json!({ "question": "And who sends the reminder?", "url": map_url })).await;
    assert!(again.text.contains("Asked on the map \"patient-converts\""));
    s.call("nonio_fail", json!({ "request_id": request_id(&again.text), "reason": "test" })).await;

    // Never listens, and says when nothing is waiting.
    assert!(s.call("nonio_listen", json!({})).await.is_error);
    // The question given up on above goes on to the code; giving that up too ends it.
    let next = s.call("nonio_next", json!({})).await;
    assert!(next.text.contains("from the code"), "{}", next.text);
    s.call("nonio_fail", json!({ "request_id": request_id(&next.text), "reason": "test" })).await;
    let until = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if s.call("nonio_next", json!({})).await.text == "Nothing is waiting right now." {
            break;
        }
        assert!(tokio::time::Instant::now() < until, "still waiting");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(s.seen.pushed.lock().unwrap().is_empty());
    s.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn puts_a_client_it_does_not_know_on_the_base_protocol_and_opens_the_map_in_the_browser() {
    let dir = copy_care_app();
    let (opened, open) = opened();
    let mut o = options(dir.path());
    o.open = Some(open);
    let s = Session::start(o, "some-other-agent").await;
    assert!(s.tool_names().await.contains(&"nonio_ask".to_string()));
    assert!(!s.tool_names().await.contains(&"nonio_listen".to_string()));
    let r = s.call("nonio_open", json!({})).await;
    assert_eq!(opened.lock().unwrap().len(), 1);
    assert!(r.text.contains("call nonio_ask"), "{}", r.text);
    s.close().await;
}
