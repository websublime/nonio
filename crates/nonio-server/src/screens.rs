//! Taking screens for a map: what this repo can show (`list_screens`) and a screenshot of one page (`capture`).
//! Captures come from the person's installed Chrome, headless, in a throwaway profile (so no logins), and only from
//! this machine: a page already served on localhost, or an HTML file in the repo. nonio never starts the app. Each
//! capture is saved under `.nonio/screens/` with a record of what was visible, which the checker reads.
//!
//! Ported from `packages/server/src/screens.ts` (MIT, Copyright (c) 2026 johnz4021); see NOTICE. The browser is
//! driven over the DevTools protocol, as the original does; `NONIO_CHROME` names one instead of looking for it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use nonio_core::format::is_safe_relative_path;
use nonio_core::repo::{is_secret_path, list_repo_files, DEFAULT_IGNORE};
use nonio_core::screens::{capture_paths, screens_dir, Capture, CaptureSource, CapturedElement, IMAGE_TYPES};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;

use crate::bridge::BoxFuture;
use crate::ids::random_id;
use crate::tools::CAPTURE;

pub const CAPTURE_WIDTH: u32 = 1280;
pub const CAPTURE_HEIGHT: u32 = 800;
/// The variable that names the browser to capture with.
pub const CHROME_ENV: &str = "NONIO_CHROME";
const COMMON_PORTS: [u16; 10] = [3000, 3001, 4200, 4321, 5000, 5173, 5174, 8000, 8080, 8888];

const CHROME_PATHS: [&str; 12] = [
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
    "/usr/bin/google-chrome",
    "/usr/bin/google-chrome-stable",
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
    "/snap/bin/chromium",
    r"C:\Program Files\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
];

/// The Chrome (or Chromium, Edge, Brave) already on this machine; `NONIO_CHROME` overrides.
pub fn find_chrome() -> Option<PathBuf> {
    if let Some(given) = std::env::var_os(CHROME_ENV).filter(|v| !v.is_empty()) {
        let p = PathBuf::from(given);
        return p.exists().then_some(p);
    }
    CHROME_PATHS.iter().map(PathBuf::from).find(|p| p.exists())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureTarget {
    Url(String),
    File(String),
}

/// What a page showed: the picture, its title, its text, and the labelled things on it.
#[derive(Debug, Clone, PartialEq)]
pub struct Shot {
    pub png: Vec<u8>,
    pub title: String,
    pub text: String,
    pub elements: Vec<CapturedElement>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CaptureResult {
    pub capture: Capture,
    pub png: Vec<u8>,
    /// What a reader of the page would see, for judging it.
    pub text: String,
}

/// Opens a page and reports what's on it. Swapped out in tests, which have no browser.
pub type Shooter = Arc<dyn Fn(String) -> BoxFuture<Result<Shot, String>> + Send + Sync>;

/// The URL a target stands for and where it came from, or why it can't be captured.
pub fn resolve_target(root: &Path, target: &CaptureTarget) -> Result<(String, CaptureSource), String> {
    match target {
        CaptureTarget::Url(given) => {
            let u = url::Url::parse(given).map_err(|_| format!("\"{given}\" is not a URL."))?;
            let local = matches!(u.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
            if !local || !matches!(u.scheme(), "http" | "https") {
                return Err("Only pages served on this machine (http://localhost:…) can be captured.".into());
            }
            Ok((u.to_string(), CaptureSource::Page { url: u.to_string() }))
        }
        CaptureTarget::File(rel) => {
            let lower = rel.to_ascii_lowercase();
            if !is_safe_relative_path(rel)
                || is_secret_path(rel)
                || !(lower.ends_with(".html") || lower.ends_with(".htm"))
            {
                return Err("`file` must be an .html file inside the repo.".into());
            }
            let real_root = fs::canonicalize(root).map_err(|e| e.to_string())?;
            let real = fs::canonicalize(real_root.join(rel)).map_err(|_| format!("{rel} doesn't exist."))?;
            if !real.starts_with(&real_root) || real == real_root {
                return Err("`file` must be inside the repo.".into());
            }
            let url = url::Url::from_file_path(&real).map_err(|_| format!("{rel} can't be opened as a page."))?;
            Ok((url.to_string(), CaptureSource::File { file: rel.clone() }))
        }
    }
}

fn now_iso() -> String {
    crate::server::now_iso()
}

/// Screenshot a target and save it with its record. Without `shoot`, the browser on this machine takes it.
pub async fn capture(root: &Path, target: &CaptureTarget, shoot: Option<&Shooter>) -> Result<CaptureResult, String> {
    let (url, source) = resolve_target(root, target)?;
    let shot = match shoot {
        Some(s) => s(url).await?,
        None => shoot_with_chrome(url).await?,
    };
    let id = format!("scr-{}", random_id());
    let record = Capture {
        id: id.clone(),
        source,
        title: shot.title.chars().take(200).collect(),
        width: f64::from(CAPTURE_WIDTH),
        height: f64::from(CAPTURE_HEIGHT),
        sha256: Sha256::digest(&shot.png).iter().map(|b| format!("{b:02x}")).collect(),
        taken_at: now_iso(),
        elements: shot.elements,
    };
    let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    let (record_path, image) = capture_paths(&root, &id);
    if let Some(dir) = image.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    fs::write(&image, &shot.png).map_err(|e| e.to_string())?;
    let mut text = serde_json::to_string_pretty(&record).map_err(|e| e.to_string())?;
    text.push('\n');
    fs::write(record_path, text).map_err(|e| e.to_string())?;
    Ok(CaptureResult { capture: record, png: shot.png, text: shot.text })
}

/// Runs in the page: the visible, labelled things in the viewport, and the page's text.
const READ_PAGE: &str = r#"(() => {
  const squash = (s) => (s || '').replace(/\s+/g, ' ').trim();
  const out = [];
  const seen = new Set();
  for (const el of document.querySelectorAll('button, a, input, select, textarea, h1, h2, h3, h4, label, summary, [role=button], [role=tab], [role=link], img[alt]')) {
    const r = el.getBoundingClientRect();
    if (r.width < 4 || r.height < 4 || r.bottom < 0 || r.right < 0 || r.top > innerHeight || r.left > innerWidth) continue;
    const cs = getComputedStyle(el);
    if (cs.visibility === 'hidden' || cs.display === 'none' || Number(cs.opacity) === 0) continue;
    const text = squash(el.innerText || el.value || el.getAttribute('aria-label') || el.getAttribute('placeholder') || el.getAttribute('alt')).slice(0, 80);
    if (!text) continue;
    const key = el.tagName + '|' + text + '|' + Math.round(r.x) + '|' + Math.round(r.y);
    if (seen.has(key)) continue;
    seen.add(key);
    out.push({ tag: el.tagName.toLowerCase(), text, x: Math.round(r.x), y: Math.round(r.y), w: Math.round(r.width), h: Math.round(r.height) });
    if (out.length >= 150) break;
  }
  return JSON.stringify({ title: document.title, text: squash(document.body ? document.body.innerText : '').slice(0, 1500), elements: out });
})()"#;

type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>;

/// One DevTools connection: commands answered by id; events other than the one awaited are let go.
struct Devtools {
    socket: Socket,
    next: u64,
    loaded: bool,
}

impl Devtools {
    async fn send(&mut self, method: &str, params: Value, session: Option<&str>) -> Result<Value, String> {
        self.next += 1;
        let id = self.next;
        let mut msg = json!({ "id": id, "method": method, "params": params });
        if let Some(s) = session {
            msg["sessionId"] = json!(s);
        }
        self.socket.send(Message::Text(msg.to_string().into())).await.map_err(|e| e.to_string())?;
        loop {
            let m = self.read().await?;
            if m.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(e) = m.get("error") {
                    return Err(e.get("message").and_then(Value::as_str).unwrap_or("the browser refused").to_string());
                }
                return Ok(m.get("result").cloned().unwrap_or_else(|| json!({})));
            }
        }
    }

    async fn read(&mut self) -> Result<Value, String> {
        loop {
            match self.socket.next().await {
                Some(Ok(Message::Text(t))) => {
                    let v: Value = serde_json::from_str(&t).map_err(|e| e.to_string())?;
                    if v.get("method").and_then(Value::as_str) == Some("Page.loadEventFired") {
                        self.loaded = true;
                    }
                    return Ok(v);
                }
                Some(Ok(_)) => continue,
                Some(Err(e)) => return Err(e.to_string()),
                None => return Err("The browser closed before it could open the page.".into()),
            }
        }
    }

    async fn until_loaded(&mut self) {
        while !self.loaded {
            if self.read().await.is_err() {
                return;
            }
        }
    }
}

/// One page in headless Chrome over the DevTools protocol: open, wait for load, read, screenshot, close.
pub async fn shoot_with_chrome(url: String) -> Result<Shot, String> {
    let Some(chrome) = find_chrome() else {
        return Err("No Chrome, Chromium, Edge or Brave was found on this machine, so no screen can be captured. Use an image file or a sketch instead.".into());
    };
    let profile = tempfile::Builder::new().prefix("nonio-chrome-").tempdir().map_err(|e| e.to_string())?;
    let mut child = tokio::process::Command::new(&chrome)
        .args([
            "--headless=new",
            "--remote-debugging-port=0",
            &format!("--user-data-dir={}", profile.path().display()),
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-extensions",
            "--hide-scrollbars",
            "--mute-audio",
            &format!("--window-size={CAPTURE_WIDTH},{CAPTURE_HEIGHT}"),
            "about:blank",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("The browser couldn't start: {e}"))?;
    let result = async {
        let stderr = child.stderr.take().ok_or("The browser couldn't start.")?;
        let mut lines = BufReader::new(stderr).lines();
        let endpoint = tokio::time::timeout(Duration::from_secs(15), async {
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(at) = line.find("DevTools listening on ") {
                    return Some(line[at + 22..].trim().to_string());
                }
            }
            None
        })
        .await
        .map_err(|_| "The browser took too long to start.".to_string())?
        .ok_or("The browser closed before it could open the page.")?;
        // Chrome keeps writing to stderr; it must be drained or it can stall.
        tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
        let (socket, _) = tokio_tungstenite::connect_async(endpoint.as_str()).await.map_err(|e| e.to_string())?;
        let mut d = Devtools { socket, next: 0, loaded: false };
        let target = d.send("Target.createTarget", json!({ "url": "about:blank" }), None).await?;
        let attached =
            d.send("Target.attachToTarget", json!({ "targetId": target["targetId"], "flatten": true }), None).await?;
        let session = attached["sessionId"].as_str().ok_or("The browser gave no page to work in.")?.to_string();
        let s = Some(session.as_str());
        d.send("Page.enable", json!({}), s).await?;
        d.send(
            "Emulation.setDeviceMetricsOverride",
            json!({ "width": CAPTURE_WIDTH, "height": CAPTURE_HEIGHT, "deviceScaleFactor": 1, "mobile": false }),
            s,
        )
        .await?;
        d.loaded = false;
        let nav = d.send("Page.navigate", json!({ "url": url }), s).await?;
        if let Some(e) = nav.get("errorText").and_then(Value::as_str).filter(|e| !e.is_empty()) {
            return Err(format!("The page couldn't be opened ({e}). Is it running?"));
        }
        let _ = tokio::time::timeout(Duration::from_secs(15), d.until_loaded()).await;
        // Client-rendered apps draw after load.
        tokio::time::sleep(Duration::from_millis(700)).await;
        let read = d.send("Runtime.evaluate", json!({ "expression": READ_PAGE, "returnByValue": true }), s).await?;
        #[derive(Deserialize, Default)]
        struct Page {
            #[serde(default)]
            title: String,
            #[serde(default)]
            text: String,
            #[serde(default)]
            elements: Vec<CapturedElement>,
        }
        let page: Page =
            read["result"]["value"].as_str().and_then(|v| serde_json::from_str(v).ok()).unwrap_or_default();
        let shot = d.send("Page.captureScreenshot", json!({ "format": "png" }), s).await?;
        let png = base64::engine::general_purpose::STANDARD
            .decode(shot["data"].as_str().unwrap_or(""))
            .map_err(|_| "The browser sent no picture.".to_string())?;
        let _ = d.socket.close(None).await;
        Ok(Shot { png, title: page.title, text: page.text, elements: page.elements })
    }
    .await;
    let _ = child.kill().await;
    // A leftover temp folder is harmless if Chrome still holds the profile.
    let _ = profile.close();
    result
}

/// A local port that answers with a web page, and its title.
async fn probe(port: u16) -> Option<(String, String)> {
    let get = async {
        let mut s = TcpStream::connect(("127.0.0.1", port)).await.ok()?;
        let req = format!("GET / HTTP/1.1\r\nhost: 127.0.0.1:{port}\r\nconnection: close\r\naccept: text/html\r\n\r\n");
        s.write_all(req.as_bytes()).await.ok()?;
        let mut out = Vec::new();
        let _ = s.take(64_000).read_to_end(&mut out).await;
        Some(String::from_utf8_lossy(&out).to_string())
    };
    let text = tokio::time::timeout(Duration::from_millis(500), get).await.ok()??;
    let (head, body) = text.split_once("\r\n\r\n")?;
    let html = head
        .lines()
        .any(|l| l.to_ascii_lowercase().starts_with("content-type:") && l.to_ascii_lowercase().contains("text/html"));
    if !html {
        return None;
    }
    let body: String = body.chars().take(20_000).collect();
    let lower = body.to_ascii_lowercase();
    let title = lower
        .find("<title")
        .and_then(|i| lower[i..].find('>').map(|j| i + j + 1))
        .and_then(|start| lower[start..].find('<').map(|end| body[start..start + end].trim().to_string()))
        .unwrap_or_default();
    Some((format!("http://localhost:{port}/"), title))
}

/// Which ports to look at for pages being served.
#[derive(Debug, Clone, Default)]
pub struct ScreenScan {
    /// Ports not to look at (the page server's own).
    pub skip_ports: Vec<u16>,
    /// Ports to look at instead of the usual dev-server ones.
    pub ports: Option<Vec<u16>>,
}

/// What this repo can show, in words for the model: running local pages, HTML files, preview setups, image files.
pub async fn list_screens(root: &Path, o: &ScreenScan) -> String {
    let files: Vec<String> = list_repo_files(root, DEFAULT_IGNORE).into_iter().filter(|f| !is_secret_path(f)).collect();
    let ext = |f: &str| f.rfind('.').map(|i| f[i..].to_ascii_lowercase()).unwrap_or_default();
    let html: Vec<String> = files.iter().filter(|f| matches!(ext(f).as_str(), ".html" | ".htm")).cloned().collect();
    let images: Vec<String> = files.iter().filter(|f| IMAGE_TYPES.iter().any(|(e, _)| *e == ext(f))).cloned().collect();
    let stories = files
        .iter()
        .filter(|f| {
            [".stories.js", ".stories.jsx", ".stories.ts", ".stories.tsx", ".stories.mdx"]
                .iter()
                .any(|s| f.ends_with(s))
        })
        .count();
    let scenes = files.iter().filter(|f| matches!(ext(f).as_str(), ".tscn" | ".unity" | ".prefab")).count();
    let ports: Vec<u16> =
        o.ports.clone().unwrap_or(COMMON_PORTS.to_vec()).into_iter().filter(|p| !o.skip_ports.contains(p)).collect();
    let pages: Vec<(String, String)> =
        futures_util::future::join_all(ports.into_iter().map(probe)).await.into_iter().flatten().collect();
    let scripts: serde_json::Map<String, Value> = fs::read_to_string(root.join("package.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.get("scripts").and_then(Value::as_object).cloned())
        .unwrap_or_default();
    let few = |list: &[String], max: usize| -> Vec<String> {
        let mut out: Vec<String> = list.iter().take(max).map(|f| format!("  - {f}")).collect();
        if list.len() > max {
            out.push(format!("  - …and {} more", list.len() - max));
        }
        out
    };
    let mut lines: Vec<String> = vec![
        if find_chrome().is_some() {
            format!("A browser is available, so {CAPTURE} can take screenshots.")
        } else {
            format!("No Chrome-like browser was found, so {CAPTURE} can't take screenshots here. Image files and sketches still work.")
        },
        String::new(),
        if pages.is_empty() {
            "No local web page is being served on the usual ports. nonio never starts the app itself.".into()
        } else {
            "Pages being served on this machine right now (check the title is this app before using one):".into()
        },
    ];
    lines
        .extend(pages.iter().map(|(u, t)| if t.is_empty() { format!("  - {u}") } else { format!("  - {u}  \"{t}\"") }));
    if pages.is_empty() && (scripts.contains_key("dev") || scripts.contains_key("start")) {
        let script = if scripts.contains_key("dev") { "dev" } else { "start" };
        lines.push(format!("  (The person could start it with `npm run {script}`; mention that only if a real screen would clearly help.)"));
    }
    lines.push(String::new());
    lines.push(if html.is_empty() {
        "No HTML files in the repo.".into()
    } else {
        "HTML files in the repo (capture with `file`):".into()
    });
    lines.extend(few(&html, 15));
    lines.push(String::new());
    if stories > 0 {
        lines.push(format!("Storybook stories: {stories} (not rendered by nonio yet; if Storybook is running it shows up as a page above)."));
    }
    if scenes > 0 {
        lines.push(format!(
            "Game scene files: {scenes} (not rendered by nonio; their sprites can be shown as image files)."
        ));
    }
    lines.push(if images.is_empty() {
        "No image files in the repo.".into()
    } else {
        format!("Image files in the repo ({}); one can be a box's screen when its cited code names it:", images.len())
    });
    lines.extend(few(&images, 15));
    lines.push(String::new());
    lines.push(format!("Captures are saved under {}/.", screens_dir()));
    let mut out: Vec<String> = Vec::new();
    for l in lines {
        if l.is_empty() && out.last().is_some_and(|p: &String| p.is_empty()) {
            continue;
        }
        out.push(l);
    }
    out.join("\n")
}
