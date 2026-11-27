//! What a repo can show, and a capture saved with its record. A stand-in shoots in most tests; one test uses the
//! browser on this machine when there is one (`NONIO_CHROME` names it), as the original's capture does.

mod common;

use std::fs;
use std::sync::Arc;

use common::*;
use nonio_core::screens::{read_capture, CaptureSource, CapturedElement};
use nonio_server::screens::{
    capture, find_chrome, list_screens, resolve_target, CaptureTarget, ScreenScan, Shooter, Shot,
};

fn stand_in() -> Shooter {
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

fn no_ports() -> ScreenScan {
    ScreenScan { skip_ports: vec![], ports: Some(vec![]) }
}

#[tokio::test]
async fn lists_html_and_image_files_but_never_secrets() {
    let dir = copy_care_app();
    fs::write(dir.path().join("demo.html"), "<button>Mark as converted</button>").unwrap();
    fs::write(dir.path().join("logo.png"), b"png").unwrap();
    let text = list_screens(dir.path(), &no_ports()).await;
    assert!(text.contains("HTML files in the repo (capture with `file`):\n  - demo.html"), "{text}");
    assert!(text.contains("  - logo.png"), "{text}");
    assert!(text.contains("No local web page is being served"), "{text}");
    assert!(text.contains("Captures are saved under .nonio/screens/."), "{text}");
    assert!(!text.contains(".env"));
}

#[tokio::test]
async fn finds_a_page_being_served_on_this_machine_and_its_title() {
    let dir = copy_care_app();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        while let Ok((mut s, _)) = listener.accept().await {
            let mut buf = [0u8; 1024];
            let _ = s.read(&mut buf).await;
            let body = "<html><head><title>Care app</title></head><body>hi</body></html>";
            let _ = s
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await;
        }
    });
    let text = list_screens(dir.path(), &ScreenScan { skip_ports: vec![], ports: Some(vec![port]) }).await;
    assert!(text.contains(&format!("  - http://localhost:{port}/  \"Care app\"")), "{text}");
    let skipped = list_screens(dir.path(), &ScreenScan { skip_ports: vec![port], ports: Some(vec![port]) }).await;
    assert!(!skipped.contains("Care app"));
}

#[test]
fn captures_only_pages_on_this_machine_and_html_files_in_the_repo() {
    let dir = copy_care_app();
    fs::write(dir.path().join("demo.html"), "<p>hi</p>").unwrap();
    let refuse = |t: CaptureTarget| resolve_target(dir.path(), &t).unwrap_err();
    assert!(refuse(CaptureTarget::Url("https://example.com/".into())).contains("Only pages served on this machine"));
    assert!(refuse(CaptureTarget::Url("file:///etc/passwd".into())).contains("Only pages served on this machine"));
    assert!(refuse(CaptureTarget::Url("not a url".into())).contains("is not a URL"));
    assert!(refuse(CaptureTarget::File(".env".into())).contains(".html file inside the repo"));
    assert!(refuse(CaptureTarget::File("../x.html".into())).contains(".html file inside the repo"));
    assert!(refuse(CaptureTarget::File("missing.html".into())).contains("doesn't exist"));
    let (url, source) =
        resolve_target(dir.path(), &CaptureTarget::Url("http://localhost:3000/patients".into())).unwrap();
    assert_eq!(url, "http://localhost:3000/patients");
    assert_eq!(source, CaptureSource::Page { url: url.clone() });
    let (url, source) = resolve_target(dir.path(), &CaptureTarget::File("demo.html".into())).unwrap();
    assert!(url.starts_with("file://") && url.ends_with("/demo.html"), "{url}");
    assert_eq!(source, CaptureSource::File { file: "demo.html".into() });
}

#[tokio::test]
async fn saves_a_capture_with_a_record_the_checker_reads() {
    let dir = copy_care_app();
    fs::write(dir.path().join("demo.html"), "<button>Mark as converted</button>").unwrap();
    let r = capture(dir.path(), &CaptureTarget::File("demo.html".into()), Some(&stand_in())).await.unwrap();
    assert!(r.capture.id.starts_with("scr-"));
    assert!(String::from_utf8_lossy(&r.png).starts_with("shot of file://"));
    let saved = read_capture(dir.path(), &r.capture.id).unwrap();
    assert_eq!(saved, r.capture);
    assert_eq!((saved.width, saved.height), (1280.0, 800.0));
    assert_eq!(saved.elements[0].text, "Mark as converted");
}

#[tokio::test]
async fn takes_a_real_screenshot_with_the_browser_on_this_machine() {
    if find_chrome().is_none() {
        eprintln!("no browser on this machine: nothing to measure");
        return;
    }
    let dir = copy_care_app();
    fs::write(dir.path().join("demo.html"), "<title>Demo</title><h1>Patients</h1><button>Mark as converted</button>")
        .unwrap();
    let r = capture(dir.path(), &CaptureTarget::File("demo.html".into()), None).await.unwrap();
    assert_eq!(&r.png[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(r.capture.title, "Demo");
    assert!(r.text.contains("Mark as converted"), "{}", r.text);
    assert!(
        r.capture.elements.iter().any(|e| e.tag == "button" && e.text == "Mark as converted"),
        "{:?}",
        r.capture.elements
    );
}
