//! Module resolution with oxc-resolver over a monorepo fixture: tsconfig paths (with `extends`), workspace packages and
//! their exports/imports maps, `.js` → `.ts`, vendor ids, files not on disk yet, and a scan through workspace barrels.
//! Ported from `packages/facts/test/resolve.test.ts` of illuminate (MIT, Copyright (c) 2026 johnz4021).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use nonio_facts::{settle_calls, symbol_names, Extractor, FileFacts, RepoView};
use nonio_scan::{resolve_specifier, OxcExtractor, RepoResolver};

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let target = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &target);
        } else {
            fs::copy(e.path(), target).unwrap();
        }
    }
}

/// A fresh copy of the fixture, so the test also shows nothing is written into it.
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::Builder::new().prefix("nonio-alias-").tempdir().unwrap();
    copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/alias-repo"), dir.path());
    dir
}

fn on_disk(root: &Path) -> impl Fn(&str) -> bool + '_ {
    move |p: &str| root.join(p).is_file()
}

#[test]
fn resolves_through_tsconfig_workspaces_package_maps_and_vendor_ids() {
    let dir = fixture();
    let root = dir.path();
    let repo = RepoResolver::new(root);
    let exists = on_disk(root);
    let r = |from: &str, spec: &str| resolve_specifier(from, spec, &exists, Some(&repo));

    // tsconfig paths, including through `extends`, and baseUrl paths.
    assert_eq!(r("apps/web/src/main.ts", "@shared/format").as_deref(), Some("packages/shared/src/format.ts"));
    assert_eq!(r("apps/api/src/server.ts", "~/handlers").as_deref(), Some("apps/api/src/handlers.ts"));

    // Workspace packages through package.json exports, and package imports.
    assert_eq!(r("apps/web/src/main.ts", "@acme/core").as_deref(), Some("packages/core/src/index.ts"));
    assert_eq!(r("apps/web/src/main.ts", "@acme/core/lib/util").as_deref(), Some("packages/core/src/lib/util.ts"));
    assert_eq!(r("packages/core/src/a.ts", "#internal/util").as_deref(), Some("packages/core/src/lib/util.ts"));
    // No node_modules, so the workspace links are needed; unlike the original, nothing is written into the repo.
    assert!(!root.join(".nonio").exists());

    // `.js` → `.ts`, and bare packages and builtins become vendor ids.
    assert_eq!(r("apps/web/src/main.ts", "./z.js").as_deref(), Some("apps/web/src/z.ts"));
    assert_eq!(r("apps/web/src/main.ts", "react").as_deref(), Some("vendor:react"));
    assert_eq!(r("apps/api/src/server.ts", "hono/jsx").as_deref(), Some("vendor:hono"));
    assert_eq!(r("apps/web/src/main.ts", "node:fs").as_deref(), Some("vendor:node:fs"));
    assert_eq!(r("apps/web/src/main.ts", "fs").as_deref(), Some("vendor:node:fs"));
    assert_eq!(r("apps/api/src/server.ts", "~/missing"), None); // an alias that matches nothing is not a package
}

#[test]
fn resolves_files_not_on_disk_through_relative_paths_and_tsconfig_paths() {
    let dir = fixture();
    let root = dir.path();
    let repo = RepoResolver::new(root);
    let planned: HashSet<&str> = ["packages/shared/src/slugify.ts", "apps/api/src/billing/index.ts"].into();
    let exists = |p: &str| planned.contains(p) || root.join(p).is_file();
    let r = |from: &str, spec: &str| resolve_specifier(from, spec, &exists, Some(&repo));
    assert_eq!(r("apps/web/src/main.ts", "@shared/slugify").as_deref(), Some("packages/shared/src/slugify.ts"));
    assert_eq!(r("apps/api/src/server.ts", "~/billing").as_deref(), Some("apps/api/src/billing/index.ts"));
    assert_eq!(r("apps/api/src/server.ts", "./billing").as_deref(), Some("apps/api/src/billing/index.ts"));
    let disk = on_disk(root);
    assert_eq!(resolve_specifier("apps/web/src/main.ts", "@shared/slugify", &disk, Some(&repo)), None);
}

/// The working tree, read from disk.
struct Disk(PathBuf);

impl RepoView for Disk {
    fn exists(&self, path: &str) -> bool {
        self.0.join(path).is_file()
    }
    fn text(&self, path: &str) -> Option<Rc<str>> {
        fs::read_to_string(self.0.join(path)).ok().map(Rc::from)
    }
}

fn code_files(root: &Path, dir: &str, out: &mut Vec<String>) {
    for e in fs::read_dir(root.join(dir)).unwrap() {
        let e = e.unwrap();
        let name = e.file_name().to_string_lossy().into_owned();
        let rel = if dir.is_empty() { name } else { format!("{dir}/{name}") };
        if e.file_type().unwrap().is_dir() {
            code_files(root, &rel, out);
        } else if nonio_scan::is_code_file(&rel) {
            out.push(rel);
        }
    }
}

#[test]
fn scans_the_repo_imports_calls_through_workspace_barrels_entries_and_hono_routes() {
    let dir = fixture();
    let root = dir.path();
    let view = Disk(root.to_path_buf());
    let oxc = OxcExtractor::new(root);
    let mut files = Vec::new();
    code_files(root, "", &mut files);
    let mut facts: HashMap<String, FileFacts> = HashMap::new();
    for f in files {
        let text = view.text(&f).unwrap();
        facts.insert(f.clone(), oxc.extract(&view, &f, &text).unwrap());
    }
    let names = symbol_names(&facts);
    for f in facts.values_mut() {
        f.calls = settle_calls(&f.calls, |file| names.get(file));
    }

    let main = &facts["apps/web/src/main.ts"];
    let imports: Vec<(&str, bool)> = main.imports.iter().map(|i| (i.to.as_str(), i.type_only)).collect();
    for want in [
        "packages/core/src/index.ts",
        "packages/core/src/lib/util.ts",
        "packages/shared/src/format.ts",
        "vendor:react",
        "vendor:node:fs",
    ] {
        assert!(imports.contains(&(want, false)), "{want} in {imports:?}");
    }
    let mut calls: Vec<String> = main
        .calls
        .iter()
        .map(|c| format!("{}{}", c.to, if c.count > 1 { format!(" x{}", c.count) } else { String::new() }))
        .collect();
    calls.sort();
    assert_eq!(
        calls,
        [
            "packages/core/src/lib/more.ts#more",      // through `export *`
            "packages/core/src/lib/util.ts#helper x3", // named via barrel, deep import, namespace member
            "packages/shared/src/format.ts#format",
            "vendor:node:fs",
        ]
    );
    let mut entries: Vec<&str> = facts["apps/web/src/cli.ts"].entries.iter().map(|e| e.id.as_str()).collect();
    entries.sort();
    assert_eq!(entries, ["entry:bin:webcli", "entry:cli:web:cli"]);
    let routes: Vec<(&str, &str, &str, &str, Option<&str>)> = facts["apps/api/src/server.ts"]
        .routes
        .iter()
        .map(|r| (r.id.as_str(), r.method.as_str(), r.path.as_str(), r.file.as_str(), r.handler.as_deref()))
        .collect();
    assert_eq!(
        routes,
        [
            (
                "route:GET /v1/users/{}",
                "GET",
                "/v1/users/{}",
                "apps/api/src/server.ts",
                Some("apps/api/src/handlers.ts#getUser")
            ),
            (
                "route:POST /v1/users",
                "POST",
                "/v1/users",
                "apps/api/src/server.ts",
                Some("apps/api/src/server.ts#<cb1>")
            ),
        ]
    );
}
