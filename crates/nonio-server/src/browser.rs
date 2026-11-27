//! Opens a link in the person's default browser.
//!
//! Ported from `packages/server/src/browser.ts` (MIT, Copyright (c) 2026 johnz4021); see NOTICE.

use std::process::{Command, Stdio};

pub fn open_browser(url: &str) {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(url);
        c
    } else if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/c", "start", "", url]);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    let _ = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
}
