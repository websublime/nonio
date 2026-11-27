//! Entry points from package.json: `bin`, `main`, and `scripts` that run a source file. Ported from
//! `facts/entries.ts` of illuminate (MIT, Copyright (c) 2026 johnz4021).

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

use fancy_regex::Regex;
use indexmap::IndexMap;
use nonio_facts::EntryKind;
use serde::Deserialize;
use serde_json::Value;

use crate::re::{js, test};
use crate::resolve::{normalize_path, resolve_path, workspace_dirs};

/// A package.json entry whose target is the file (`bin`, `scripts`, `main`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageEntry {
    pub kind: EntryKind,
    pub label: String,
}

const RUNNERS: [&str; 13] = [
    "node",
    "tsx",
    "ts-node",
    "ts-node-esm",
    "bun",
    "deno",
    "esno",
    "esrun",
    "vite-node",
    "nodemon",
    "jiti",
    "sucrase-node",
    "babel-node",
];
static CODE: LazyLock<Regex> = LazyLock::new(|| js(r"\.[cm]?[jt]sx?$"));
static ENV: LazyLock<Regex> = LazyLock::new(|| js(r"^\w+="));
static SEP: LazyLock<Regex> = LazyLock::new(|| js(r"&&|\|\||[;|&]"));
static BUILT: LazyLock<Regex> = LazyLock::new(|| js(r"(^|/)(?:dist|build|lib|out)/"));

/// The parts of a package.json read here. Objects keep their order (the original walks them in insertion order).
#[derive(Deserialize)]
struct Pkg {
    name: Option<Value>,
    bin: Option<Bin>,
    main: Option<Value>,
    scripts: Option<Scripts>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Bin {
    One(String),
    Many(IndexMap<String, Value>),
    /// Any other value: present, and not an entry.
    Other(#[allow(dead_code)] Value),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Scripts {
    Many(IndexMap<String, Value>),
    /// Any other value: present, and not an entry.
    Other(#[allow(dead_code)] Value),
}

/// package.json entries (root and workspace packages) keyed by the repo file they run.
pub fn package_entries(root: &Path, exists: &dyn Fn(&str) -> bool) -> HashMap<String, Vec<PackageEntry>> {
    let mut out: HashMap<String, Vec<PackageEntry>> = HashMap::new();
    let dirs = std::iter::once(String::new()).chain(workspace_dirs(root));
    for dir in dirs {
        let file = if dir.is_empty() { "package.json".to_string() } else { format!("{dir}/package.json") };
        let Some(pkg) = fs::read_to_string(root.join(&file)).ok().and_then(|t| serde_json::from_str::<Pkg>(&t).ok())
        else {
            continue;
        };
        let target = |file: &str| -> Option<String> {
            let p = normalize_path(&if dir.is_empty() { file.to_string() } else { format!("{dir}/{file}") })?;
            // Built outputs (`dist/cli.js`) usually come from `src/cli.ts`.
            resolve_path(&p, exists).or_else(|| resolve_path(&BUILT.replace(&p, "${1}src/"), exists))
        };
        let mut add = |file: Option<&str>, entry: PackageEntry| {
            let Some(t) = file.and_then(target) else {
                return;
            };
            let list = out.entry(t).or_default();
            if !list.contains(&entry) {
                list.push(entry);
            }
        };
        let name = pkg.name.as_ref().and_then(Value::as_str).map(String::from).unwrap_or_else(|| {
            if dir.is_empty() {
                "root".into()
            } else {
                dir.clone()
            }
        });
        match &pkg.bin {
            Some(Bin::One(bin)) => {
                let label = match name.strip_prefix('@') {
                    Some(scoped) => scoped.split_once('/').map_or(name.clone(), |(_, n)| n.to_string()),
                    None => name.clone(),
                };
                add(Some(bin), PackageEntry { kind: EntryKind::Bin, label });
            }
            Some(Bin::Many(bins)) => {
                for (bin, file) in bins {
                    add(file.as_str(), PackageEntry { kind: EntryKind::Bin, label: bin.clone() });
                }
            }
            _ => {}
        }
        if let Some(main) = pkg.main.as_ref().and_then(Value::as_str).filter(|m| !m.is_empty()) {
            add(Some(main), PackageEntry { kind: EntryKind::Main, label: name.clone() });
        }
        if let Some(Scripts::Many(scripts)) = &pkg.scripts {
            for (script, cmd) in scripts {
                let Some(cmd) = cmd.as_str() else { continue };
                let label = if dir.is_empty() { script.clone() } else { format!("{name}:{script}") };
                for part in SEP.split(cmd).map_while(Result::ok) {
                    let tokens: Vec<&str> = part.split_whitespace().filter(|t| !test(&ENV, t)).collect();
                    let first = tokens.first().map(|t| if *t == "npx" { "" } else { *t }).unwrap_or("");
                    let runner = if first.is_empty() { tokens.get(1).copied() } else { Some(first) };
                    if !runner.is_some_and(|r| RUNNERS.contains(&r)) {
                        continue;
                    }
                    if let Some(file) = tokens.iter().find(|t| !t.starts_with('-') && test(&CODE, t)) {
                        add(Some(file), PackageEntry { kind: EntryKind::Script, label: label.clone() });
                    }
                }
            }
        }
    }
    out
}
