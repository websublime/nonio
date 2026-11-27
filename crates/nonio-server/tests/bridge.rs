//! The bridge, ported from the "the bridge" tests in `packages/server/test/channel.test.ts`. No model is called: the
//! test plays the host's model by submitting output.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nonio_core::parse_map;
use nonio_server::bridge::{AgentError, AgentRequest, Bridge, BridgeOptions, HOST_SESSION};
use nonio_server::host::{HostProfile, CLAUDE_CODE};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

fn care_map() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../nonio-core/tests/fixtures/care-app.map.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// A request for a map, checked the way the maker checks one.
fn map_request(prompt: &str) -> AgentRequest {
    let mut r = AgentRequest::new(prompt, Arc::new(json!({ "type": "object" })), "/");
    r.validate = Some(Arc::new(|v: &Value| parse_map(v).map(|_| ())));
    r
}

fn recorder() -> (Arc<Mutex<Vec<String>>>, impl Fn(&nonio_server::BridgeRequest) + Send + Sync + 'static) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = seen.clone();
    (seen, move |r: &nonio_server::BridgeRequest| s.lock().unwrap().push(r.id.clone()))
}

/// The bridge as Claude Code sees it, so the progress lines are Claude Code's.
fn claude_code(o: BridgeOptions) -> Bridge {
    Bridge::new(BridgeOptions { host: Arc::new(|| -> &'static HostProfile { &CLAUDE_CODE }), ..o })
}

fn said() -> (Arc<Mutex<Vec<String>>>, nonio_server::bridge::ProgressFn) {
    let said = Arc::new(Mutex::new(Vec::new()));
    let s = said.clone();
    (said, Arc::new(move |p| s.lock().unwrap().push(p.text)))
}

#[tokio::test]
async fn announces_a_request_when_nothing_is_waiting_and_hands_back_output_once_it_reads() {
    let (announced, announce) = recorder();
    let bridge = claude_code(BridgeOptions::new(announce));
    let mut req = map_request("Draw it");
    req.label = Some("Making a map".into());
    let reply = bridge.ask(req);
    assert_eq!(*announced.lock().unwrap(), ["r1"]);
    let next = bridge.next().unwrap();
    assert_eq!((next.id.as_str(), next.label.as_str(), next.continues), ("r1", "Making a map", false));

    assert!(bridge.submit("r1", json!({ "version": 1 })).is_err());
    assert_eq!(bridge.submit("r1", Value::String(care_map().to_string())), Ok(()));
    let got = reply.await.unwrap();
    assert_eq!(got.output, care_map());
    assert_eq!(got.session_id.as_deref(), Some(HOST_SESSION));
    assert_eq!(got.cost, None);
    assert!(bridge.outstanding().is_empty());
    assert!(bridge.submit("r1", care_map()).is_err());
}

#[tokio::test]
async fn hands_a_request_to_a_waiting_tool_call_instead_of_announcing_it_and_lets_the_model_give_up() {
    let (announced, announce) = recorder();
    let bridge = claude_code(BridgeOptions::new(announce));
    let cancel = CancellationToken::new();
    let waiting = {
        let (b, c) = (bridge.clone(), cancel.clone());
        tokio::spawn(async move { b.wait_for_next(&c).await })
    };
    tokio::task::yield_now().await;
    let mut req = map_request("Answer");
    req.tools = Some(vec![]);
    req.resume = Some(HOST_SESSION.into());
    let reply = bridge.ask(req);
    let handed = waiting.await.unwrap().unwrap();
    assert_eq!((handed.id.as_str(), handed.tools.is_empty(), handed.continues), ("r1", true, true));
    assert!(announced.lock().unwrap().is_empty());
    assert!(bridge.fail("r1", "the code is not there"));
    let err = reply.await.unwrap_err();
    assert!(err.to_string().contains("the code is not there"), "{err}");
}

#[tokio::test(start_paused = true)]
async fn says_on_the_map_when_no_one_picks_a_request_up_then_gives_up_instead_of_waiting_forever() {
    let (said, on_progress) = said();
    let bridge = claude_code(BridgeOptions {
        nudge: Some(Duration::from_millis(20)),
        give_up: Duration::from_millis(60),
        ..BridgeOptions::new(|_| {})
    });
    let mut req = map_request("x");
    req.on_progress = Some(on_progress);
    let reply = bridge.ask(req);
    assert_eq!(*said.lock().unwrap(), ["Waiting for Claude Code to pick this up"]);
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(said.lock().unwrap()[1], CLAUDE_CODE.not_picked_up);
    assert_eq!(reply.await, Err(AgentError::Unheard(CLAUDE_CODE.never_picked_up.into())));
    assert!(bridge.outstanding().is_empty());
}

#[tokio::test(start_paused = true)]
async fn keeps_waiting_once_the_model_has_picked_a_request_up() {
    let (said, on_progress) = said();
    let bridge = claude_code(BridgeOptions {
        nudge: Some(Duration::from_millis(20)),
        give_up: Duration::from_millis(40),
        ..BridgeOptions::new(|_| {})
    });
    let mut req = map_request("x");
    req.on_progress = Some(on_progress);
    let reply = bridge.ask(req);
    assert_eq!(bridge.next().unwrap().id, "r1");
    assert_eq!(
        *said.lock().unwrap(),
        ["Waiting for Claude Code to pick this up", "Claude Code is working on this in your terminal"]
    );
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(said.lock().unwrap().len(), 2);
    assert_eq!(bridge.submit("r1", care_map()), Ok(()));
    assert_eq!(reply.await.unwrap().output, care_map());
}

#[tokio::test]
async fn hands_a_request_to_the_newest_waiting_call_leaving_an_older_listener_waiting() {
    let bridge = claude_code(BridgeOptions::new(|_| {}));
    let (older, newer) = (CancellationToken::new(), CancellationToken::new());
    let listener = {
        let b = bridge.clone();
        tokio::spawn(async move { b.wait_for_next(&older).await })
    };
    tokio::task::yield_now().await;
    let current = {
        let b = bridge.clone();
        tokio::spawn(async move { b.wait_for_next(&newer).await })
    };
    tokio::task::yield_now().await;
    let _first = bridge.ask(map_request("x"));
    assert_eq!(current.await.unwrap().unwrap().id, "r1");
    let _second = bridge.ask(map_request("y"));
    assert_eq!(listener.await.unwrap().unwrap().id, "r2");
}

#[tokio::test]
async fn drops_a_waiting_tool_call_once_it_stops_listening() {
    let (announced, announce) = recorder();
    let bridge = claude_code(BridgeOptions::new(announce));
    let cancel = CancellationToken::new();
    let waiting = {
        let (b, c) = (bridge.clone(), cancel.clone());
        tokio::spawn(async move { b.wait_for_next(&c).await })
    };
    tokio::task::yield_now().await;
    cancel.cancel();
    assert_eq!(waiting.await.unwrap(), None);
    let _reply = bridge.ask(map_request("x"));
    assert_eq!(*announced.lock().unwrap(), ["r1"]);
}

// Not in the original: what the Rust bridge adds.

#[tokio::test]
async fn stops_a_request_when_its_caller_cancels_it() {
    let bridge = Bridge::new(BridgeOptions::new(|_| {}));
    let cancel = CancellationToken::new();
    let mut req = map_request("x");
    req.cancel = Some(cancel.clone());
    let reply = bridge.ask(req);
    cancel.cancel();
    assert_eq!(reply.await, Err(AgentError::Stopped));
    assert!(bridge.outstanding().is_empty());
}

#[tokio::test]
async fn takes_any_json_object_when_a_request_has_no_validator_and_never_text() {
    let bridge = Bridge::new(BridgeOptions::new(|_| {}));
    let reply = bridge.ask(AgentRequest::new("x", Arc::new(json!({})), "/"));
    assert!(bridge.submit("r1", json!([1])).is_err());
    assert_eq!(
        bridge.submit("r1", Value::String("not json".into())),
        Err(vec!["`output` must be a JSON object matching the schema, not text.".into()])
    );
    assert_eq!(bridge.submit("r1", json!({ "a": 1 })), Ok(()));
    assert_eq!(reply.await.unwrap().output, json!({ "a": 1 }));
}

#[tokio::test]
async fn names_the_host_when_the_model_gives_up_and_says_so_for_no_such_request() {
    let bridge = Bridge::new(BridgeOptions::new(|_| {}));
    let reply = bridge.ask(map_request("x"));
    assert!(!bridge.fail("r9", "no"));
    assert!(bridge.fail("r1", "no code"));
    assert_eq!(reply.await, Err(AgentError::Failed("your agent couldn't finish this: no code".into())));
}
