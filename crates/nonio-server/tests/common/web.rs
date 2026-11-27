//! A bare HTTP/1.1 client and a WebSocket tab for the page server tests: every header is the test's to set, Host and
//! Origin included.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use nonio_server::server::{NonioServer, TOKEN_HEADER};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub struct Response {
    pub status: u16,
    pub headers: String,
    pub text: String,
}

impl Response {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.text).unwrap_or_else(|_| panic!("not JSON: {}", self.text))
    }
}

/// One request on its own connection. `Host` is the server's unless `headers` names one.
pub async fn raw(port: u16, method: &str, path: &str, headers: &[(&str, &str)], body: Option<&Value>) -> Response {
    let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    let mut req = format!("{method} {path} HTTP/1.1\r\nconnection: close\r\ncontent-length: {}\r\n", body.len());
    if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("host")) {
        req.push_str(&format!("host: 127.0.0.1:{port}\r\n"));
    }
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    if !body.is_empty() {
        req.push_str("content-type: application/json\r\n");
    }
    req.push_str("\r\n");
    req.push_str(&body);
    s.write_all(req.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    s.read_to_end(&mut out).await.unwrap();
    let text = String::from_utf8_lossy(&out).to_string();
    let (head, rest) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let body =
        if head.to_ascii_lowercase().contains("transfer-encoding: chunked") { unchunk(rest) } else { rest.to_string() };
    Response { status, headers: head.to_string(), text: body }
}

fn unchunk(mut s: &str) -> String {
    let mut out = String::new();
    while let Some((size, rest)) = s.split_once("\r\n") {
        let n = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
        if n == 0 {
            break;
        }
        out.push_str(&rest[..n]);
        s = &rest[n + 2..];
    }
    out
}

/// An API call with the server's token.
pub async fn api(server: &NonioServer, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
    let r = raw(server.port, method, path, &[(TOKEN_HEADER, &server.token)], body.as_ref()).await;
    let v = r.json();
    (r.status, v)
}

/// An open tab: every event the server sends it, and how it was closed.
pub struct Tab {
    pub events: Arc<Mutex<Vec<Value>>>,
    pub closed: Arc<Mutex<Option<u16>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Tab {
    pub fn events(&self) -> Vec<Value> {
        self.events.lock().unwrap().clone()
    }
    pub fn close(self) {
        self.task.abort();
    }
}

pub async fn open_tab(port: u16, query: &str) -> Result<Tab, String> {
    let (ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws?{query}"))
        .await
        .map_err(|e| e.to_string())?;
    let events = Arc::new(Mutex::new(Vec::new()));
    let closed = Arc::new(Mutex::new(None));
    let (e, c) = (events.clone(), closed.clone());
    let task = tokio::spawn(async move {
        let (_tx, mut rx) = ws.split();
        while let Some(m) = rx.next().await {
            match m {
                Ok(tokio_tungstenite::tungstenite::Message::Text(t)) => {
                    e.lock().unwrap().push(serde_json::from_str(&t).unwrap())
                }
                Ok(tokio_tungstenite::tungstenite::Message::Close(f)) => {
                    *c.lock().unwrap() = Some(f.map(|f| u16::from(f.code)).unwrap_or(0));
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        if c.lock().unwrap().is_none() {
            *c.lock().unwrap() = Some(0);
        }
    });
    Ok(Tab { events, closed, task })
}

/// Waits for `f` to give something, up to five seconds.
pub async fn wait_for<T>(mut f: impl FnMut() -> Option<T>) -> T {
    let until = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(tokio::time::Instant::now() < until, "timed out waiting");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// A stand-in for the model that never answers.
pub struct Silent;

impl nonio_server::bridge::Agent for Silent {
    fn ask(
        &self,
        _: nonio_server::bridge::AgentRequest,
    ) -> nonio_server::bridge::BoxFuture<nonio_server::bridge::AgentResult> {
        Box::pin(std::future::pending())
    }
}

/// A stand-in that answers with the care-app map only once `release` is called.
pub struct Held {
    gate: Arc<tokio::sync::Notify>,
    open: Arc<std::sync::atomic::AtomicBool>,
}

impl Held {
    pub fn new() -> Arc<Self> {
        Arc::new(Held { gate: Arc::new(tokio::sync::Notify::new()), open: Arc::new(false.into()) })
    }
    pub fn release(&self) {
        self.open.store(true, std::sync::atomic::Ordering::SeqCst);
        self.gate.notify_waiters();
    }
}

impl nonio_server::bridge::Agent for Held {
    fn ask(
        &self,
        _: nonio_server::bridge::AgentRequest,
    ) -> nonio_server::bridge::BoxFuture<nonio_server::bridge::AgentResult> {
        let (gate, open) = (self.gate.clone(), self.open.clone());
        Box::pin(async move {
            while !open.load(std::sync::atomic::Ordering::SeqCst) {
                let wait = gate.notified();
                if open.load(std::sync::atomic::Ordering::SeqCst) {
                    break;
                }
                wait.await;
            }
            Ok(nonio_server::bridge::AgentResponse {
                output: super::care_map_json(),
                session_id: Some("held".into()),
                cost: Some(0.0),
            })
        })
    }
}
