//! The server stops itself when nobody is looking (no tab open, the agent not working) and from the page's Stop
//! button. Ported from `packages/server/test/stop.test.ts`.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::web::*;
use common::*;
use nonio_server::server::{
    start_server, AutoStop, NonioServer, ServerOptions, StopReason, STOPPED_CLOSE_CODE, TOKEN_HEADER,
};

struct Started {
    _dir: tempfile::TempDir,
    root: std::path::PathBuf,
    server: NonioServer,
    stops: Arc<Mutex<Vec<StopReason>>>,
}

async fn start(agent: Arc<dyn nonio_server::Agent>, auto: Option<(u64, u64)>) -> Started {
    let dir = copy_care_app();
    let stops = Arc::new(Mutex::new(Vec::new()));
    let mut o = ServerOptions::new(dir.path(), agent);
    o.auto_stop = auto.map(|(after, before)| AutoStop {
        after_last_tab: Duration::from_millis(after),
        before_first_tab: Duration::from_millis(before),
    });
    let s = stops.clone();
    o.on_stop = Some(Arc::new(move |r| s.lock().unwrap().push(r)));
    let server = start_server(o).await.unwrap();
    Started { root: dir.path().to_path_buf(), _dir: dir, server, stops }
}

fn stops(s: &Started) -> Vec<StopReason> {
    s.stops.lock().unwrap().clone()
}

async fn tab(s: &NonioServer) -> Tab {
    open_tab(s.port, &format!("t={}", s.token)).await.unwrap()
}

async fn stop_request(s: &NonioServer) -> Response {
    raw(s.port, "POST", "/api/stop", &[(TOKEN_HEADER, &s.token)], None).await
}

async fn sleep(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

fn none() -> Arc<dyn nonio_server::Agent> {
    ScriptedAgent::new(vec![])
}

#[tokio::test]
async fn stays_up_while_a_tab_is_open_and_stops_a_little_after_the_last_one_closes() {
    let s = start(none(), Some((50, 10_000))).await;
    let t = tab(&s.server).await;
    sleep(150).await;
    assert_eq!(stops(&s), vec![]);
    t.close();
    wait_for(|| (!stops(&s).is_empty()).then_some(())).await;
    assert_eq!(stops(&s), vec![StopReason::Idle]);
    assert!(tokio::net::TcpStream::connect(("127.0.0.1", s.server.port)).await.is_err());
}

#[tokio::test]
async fn keeps_going_while_a_reloaded_tab_reconnects_within_the_grace_period() {
    let s = start(none(), Some((150, 10_000))).await;
    let first = tab(&s.server).await;
    first.close();
    let _second = tab(&s.server).await;
    sleep(300).await;
    assert_eq!(stops(&s), vec![]);
    s.server.close().await;
}

#[tokio::test]
async fn never_stops_while_the_agent_is_working_even_with_no_tab_open() {
    let held = Held::new();
    let s = start(held.clone(), Some((20, 20))).await;
    let id = s.server.start_map("What happens when a patient converts?").unwrap();
    sleep(150).await;
    assert_eq!(stops(&s), vec![]);
    held.release();
    wait_for(|| (!stops(&s).is_empty()).then_some(())).await;
    assert_eq!(stops(&s), vec![StopReason::Idle]);
    // The map was saved before stopping.
    assert!(s.root.join(format!(".nonio/maps/{id}.json")).exists());
}

#[tokio::test]
async fn does_nothing_on_its_own_without_auto_stop() {
    let s = start(none(), None).await;
    let t = tab(&s.server).await;
    t.close();
    sleep(100).await;
    assert_eq!(stops(&s), vec![]);
    assert_eq!(raw(s.server.port, "GET", "/", &[], None).await.status, 200);
    s.server.close().await;
}

#[tokio::test]
async fn stops_from_the_page_refusing_while_the_agent_works() {
    let held = Held::new();
    let s = start(held.clone(), None).await;
    s.server.start_map("What happens when a patient converts?").unwrap();
    let busy = stop_request(&s.server).await;
    assert_eq!(busy.status, 409);
    assert!(busy.json()["error"].as_str().unwrap().contains("still working"));
    held.release();
    wait_for(|| (!s.server.working()).then_some(())).await;
    assert_eq!(stop_request(&s.server).await.status, 200);
    wait_for(|| (!stops(&s).is_empty()).then_some(())).await;
    assert_eq!(stops(&s), vec![StopReason::Requested]);
}

#[tokio::test]
async fn sends_open_tabs_the_stopped_close_code() {
    let s = start(none(), None).await;
    let t = tab(&s.server).await;
    assert_eq!(stop_request(&s.server).await.status, 200);
    let code = wait_for(|| *t.closed.lock().unwrap()).await;
    assert_eq!(code, STOPPED_CLOSE_CODE);
}

#[tokio::test]
async fn needs_the_token_to_stop() {
    let s = start(none(), None).await;
    assert_eq!(raw(s.server.port, "POST", "/api/stop", &[], None).await.status, 401);
    assert_eq!(stops(&s), vec![]);
    s.server.close().await;
}
