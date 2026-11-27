//! `nonio mcp` as an agent starts it: a child process speaking MCP over stdio. The test plays Codex (its host opens
//! no browser), asks for the saved maps, and loads the page the link points at from the server the binary started.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Command, Stdio};

use serde_json::{json, Value};

fn get(url: &str) -> String {
    let rest = url.strip_prefix("http://").unwrap();
    let (host, path) = rest.split_at(rest.find('/').unwrap());
    let mut s = TcpStream::connect(host).unwrap();
    write!(s, "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    out
}

#[test]
fn runs_as_an_mcp_server_over_stdio_and_serves_the_viewer_built_in() {
    let repo = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_nonio"))
        .args(["mcp", "--repo", repo.path().to_str().unwrap(), "--port", "0"])
        .env("NONIO_HOST", "codex")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut send = |v: Value| writeln!(stdin, "{v}").unwrap();
    let mut read = || -> Value {
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line:?}"))
    };

    send(json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "codex-mcp-client", "version": "1" } } }));
    let init = read();
    assert_eq!(init["result"]["serverInfo"]["name"], "nonio", "{init}");
    send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));

    send(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }));
    let tools = read();
    let names: Vec<&str> =
        tools["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"nonio_map") && names.contains(&"nonio_ask"), "{names:?}");

    send(
        json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "nonio_open", "arguments": {} } }),
    );
    let opened = read();
    let text = opened["result"]["content"][0]["text"].as_str().unwrap().to_string();
    assert!(text.contains("No maps saved in this repo yet."), "{text}");
    let url = text.split_whitespace().find(|w| w.starts_with("http://127.0.0.1:")).unwrap();
    let page = get(url);
    assert!(page.starts_with("HTTP/1.1 200"), "{page}");
    if std::env::var_os("NONIO_EXPECT_VIEWER").is_some() {
        // CI builds the viewer first, so the binary must carry it.
        assert!(page.contains("<title>nonio://</title>"), "{page}");
        assert!(page.contains("/assets/"), "{page}");
    } else {
        assert!(page.contains("<title>nonio://</title>") || page.contains("npm run build"), "{page}");
    }

    // The agent closing stdin ends nonio, and the page server with it.
    drop(stdin);
    let status = child.wait().unwrap();
    assert!(status.success(), "{status}");
    assert!(TcpStream::connect(url.strip_prefix("http://").unwrap().split('/').next().unwrap()).is_err());
}

/// A plugin's server starts in the plugin's own folder (Agent Plugins 1.0.0 §7.2.1), which is not the person's
/// project: there, a call has to name the repo.
#[test]
fn started_in_its_plugin_folder_it_asks_for_the_repo() {
    let plugin = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_nonio"))
        .args(["mcp", "--port", "0"])
        .current_dir(plugin.path())
        .env("PLUGIN_ROOT", plugin.path())
        .env_remove("CLAUDE_PROJECT_DIR")
        .env_remove("NONIO_HOST")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut send = |v: Value| writeln!(stdin, "{v}").unwrap();
    let mut read = || -> Value {
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line:?}"))
    };
    send(json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "some-agent", "version": "1" } } }));
    read();
    send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
    send(
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "nonio_open", "arguments": {} } }),
    );
    let r = read();
    assert_eq!(r["result"]["isError"], true, "{r}");
    assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("Pass `repo`"), "{r}");
    drop(stdin);
    assert!(child.wait().unwrap().success());
}
