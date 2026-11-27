//! The plugin in `plugin/` (D04) as the hosts read it: the Agent Plugins 1.0.0 manifest and `mcp.json`, the Cursor
//! manifest and the marketplace that lists it, and the skill. The CI's `plugin` job also validates the two Agent
//! Plugins files against the official schemas; this test holds what a schema can't: that every file names the version
//! being built, and that the command the plugin runs is one this binary answers.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const PACKAGE: &str = "@websublime/nonio";
const PLUGIN_SCHEMA: &str = "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json";
const MCP_SCHEMA: &str = "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn json(rel: &str) -> Value {
    let path = repo().join(rel);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn keys(v: &Value) -> Vec<&str> {
    v.as_object().unwrap().keys().map(String::as_str).collect()
}

/// Agent Plugins 1.0.0 §5.5: 1 to 64 of a-z, 0-9, `-` and `.`, starting and ending alphanumeric, no `--` or `..`.
fn portable_name(name: &str) -> bool {
    let ok = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    (1..=64).contains(&name.len())
        && name.chars().all(|c| ok(c) || c == '-' || c == '.')
        && name.starts_with(ok)
        && name.ends_with(ok)
        && !name.contains("--")
        && !name.contains("..")
}

#[test]
fn the_manifest_is_agent_plugins_1_0_0_for_this_version() {
    let m = json("plugin/plugin.json");
    assert_eq!(m["$schema"], PLUGIN_SCHEMA);
    assert!(portable_name(m["name"].as_str().unwrap()));
    assert_eq!(m["name"], "nonio");
    assert_eq!(m["version"], VERSION);
    // The manifest is closed: anything else is reported and ignored, and the MCP servers may not be declared inline.
    let allowed = [
        "$schema",
        "name",
        "version",
        "description",
        "author",
        "homepage",
        "repository",
        "license",
        "keywords",
        "extensions",
    ];
    for k in keys(&m) {
        assert!(allowed.contains(&k), "plugin.json: {k} is not an Agent Plugins field");
    }
}

#[test]
fn mcp_json_starts_this_version_of_nonio_over_stdio() {
    let m = json("plugin/mcp.json");
    assert_eq!(m["$schema"], MCP_SCHEMA);
    assert_eq!(keys(&m), ["$schema", "mcpServers"]);
    let servers = m["mcpServers"].as_object().unwrap();
    assert_eq!(servers.keys().collect::<Vec<_>>(), ["nonio"]);
    let s = &servers["nonio"];
    for k in keys(s) {
        assert!(["type", "command", "args", "env", "cwd"].contains(&k), "mcp.json: {k} is not a stdio field");
    }
    assert_eq!(s["type"], "stdio");
    // One executable token, found on the PATH: npx brings the binary for this platform (D24).
    assert_eq!(s["command"], "npx");
    let args: Vec<&str> = s["args"].as_array().unwrap().iter().map(|a| a.as_str().unwrap()).collect();
    assert_eq!(args, ["-y", &format!("{PACKAGE}@{VERSION}"), "mcp"]);
}

#[test]
fn the_cursor_manifest_matches_and_the_marketplace_lists_it() {
    let base = json("plugin/plugin.json");
    let cursor = json("plugin/.cursor-plugin/plugin.json");
    for k in ["name", "version", "author", "homepage", "repository", "license", "keywords"] {
        assert_eq!(cursor[k], base[k], ".cursor-plugin/plugin.json: {k} differs from plugin.json");
    }
    // Cursor reads `mcp.json` at the plugin root for both formats; declaring servers here would override it.
    assert!(cursor.get("mcpServers").is_none());
    assert!(repo().join("plugin").join(cursor["logo"].as_str().unwrap()).is_file());

    let market = json(".cursor-plugin/marketplace.json");
    assert!(market["owner"]["name"].is_string());
    let plugins = market["plugins"].as_array().unwrap();
    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0]["name"], base["name"]);
    let source = plugins[0]["source"].as_str().unwrap();
    assert!(repo().join(source).join(".cursor-plugin/plugin.json").is_file());
    assert!(repo().join(source).join("plugin.json").is_file());
}

#[test]
fn the_skill_is_named_for_its_folder_and_names_this_version() {
    let text = std::fs::read_to_string(repo().join("plugin/skills/nonio/SKILL.md")).unwrap();
    let front = text.strip_prefix("---\n").and_then(|t| t.split_once("\n---\n")).map(|(f, _)| f).expect("frontmatter");
    let field = |k: &str| {
        front.lines().find_map(|l| l.strip_prefix(&format!("{k}: "))).unwrap_or_else(|| panic!("no {k}")).to_string()
    };
    assert_eq!(field("name"), "nonio");
    let description = field("description");
    assert!(!description.is_empty() && description.len() <= 1024);
    let mentions: Vec<&str> =
        text.match_indices(&format!("{PACKAGE}@")).map(|(i, _)| &text[i + PACKAGE.len() + 1..]).collect();
    assert!(!mentions.is_empty());
    for m in mentions {
        assert!(m.starts_with(&format!("{VERSION} ")), "SKILL.md names another version: {}", &m[..m.len().min(12)]);
    }
}

#[test]
fn the_version_the_skill_asks_for_is_what_the_binary_prints() {
    let out = Command::new(env!("CARGO_BIN_EXE_nonio")).arg("--version").output().unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8(out.stdout).unwrap().trim(), format!("nonio {VERSION}"));
}
