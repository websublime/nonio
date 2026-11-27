//! Import specifiers and relative resolution. Ported from `packages/facts/test/imports.test.ts` of illuminate (MIT,
//! Copyright (c) 2026 johnz4021).

use std::collections::HashSet;

use nonio_scan::{module_record, resolve_specifier, strip_comments};

fn parse_imports(src: &str) -> Vec<String> {
    module_record("source.ts", src).specs.into_iter().map(|s| s.spec).collect()
}

#[test]
fn reads_every_import_syntax_in_source_order() {
    let src = [
        r#"import a from "./a";"#,
        "import type { T } from './types';",
        "import {",
        "  b,",
        "  c as d,",
        r#"} from "../b";"#,
        "import * as ns from './ns';",
        "import x, { y } from './xy';",
        "import './side-effect';",
        r#"export { e } from "./e";"#,
        "export * from './star';",
        "export type { F } from './f';",
        "const m = await import('./dynamic');",
        "const t = await import(`./tpl`);",
        r#"import { readFile } from "node:fs";"#,
        "import { z } from 'zod';",
    ]
    .join("\n");
    assert_eq!(
        parse_imports(&src),
        [
            "./a",
            "./types",
            "../b",
            "./ns",
            "./xy",
            "./side-effect",
            "./e",
            "./star",
            "./f",
            "./dynamic",
            "./tpl",
            "node:fs",
            "zod"
        ]
    );
}

#[test]
fn ignores_commented_out_imports_but_not_strings_that_contain_slashes() {
    let src = [
        r#"// import { chat } from "../../vendor/llm-openai/index";"#,
        "/* import x from './block';",
        "   import y from './block2'; */",
        r#"const url = "https://example.com/x"; import { ok } from './ok';"#,
        r"const re = /\/\/ not a comment/; import './after-regex';",
        r#"const tpl = `// ${"x" + `${1}`} still a string`; import "./after-template";"#,
        "const notDynamic = foo.import('./nope');",
        "const computed = import(`./${name}`);",
    ]
    .join("\n");
    assert_eq!(parse_imports(&src), ["./ok", "./after-regex", "./after-template"]);
}

#[test]
fn keeps_offsets_and_newlines_when_stripping_comments() {
    let src = "a // x\n/* y\nz */ b";
    let out = strip_comments(src);
    assert_eq!(out.len(), src.len());
    assert_eq!(out.split('\n').count(), 3);
    assert_eq!(nonio_scan::squash(&out), "a b");
}

fn files() -> HashSet<&'static str> {
    HashSet::from([
        "src/gateway/complete.ts",
        "src/tools/index.ts",
        "src/ui/App.tsx",
        "src/ui/widgets/index.tsx",
        "src/esm/mod.mts",
        "src/plain.js",
        "vendor/llm-openai/index.ts",
    ])
}

/// `resolveImport` of the original: a repo file, never a package.
fn resolve_import(from: &str, spec: &str) -> Option<String> {
    let files = files();
    resolve_specifier(from, spec, &|p| files.contains(p), None).filter(|t| !t.starts_with("vendor:"))
}

#[test]
fn tries_as_written_then_ts_tsx_index_ts_index_tsx() {
    assert_eq!(resolve_import("src/tools/a.ts", "../gateway/complete").as_deref(), Some("src/gateway/complete.ts"));
    assert_eq!(resolve_import("src/tools/a.ts", "../gateway/complete.ts").as_deref(), Some("src/gateway/complete.ts"));
    assert_eq!(resolve_import("src/main.ts", "./ui/App").as_deref(), Some("src/ui/App.tsx"));
    assert_eq!(resolve_import("src/orchestrator/r.ts", "../tools").as_deref(), Some("src/tools/index.ts"));
    assert_eq!(resolve_import("src/ui/App.tsx", "./widgets").as_deref(), Some("src/ui/widgets/index.tsx"));
    assert_eq!(
        resolve_import("src/tools/x/y.ts", "../../../vendor/llm-openai/index").as_deref(),
        Some("vendor/llm-openai/index.ts")
    );
    assert_eq!(resolve_import("src/main.ts", "./plain.js").as_deref(), Some("src/plain.js"));
}

#[test]
fn maps_js_jsx_mjs_specifiers_to_ts_tsx_mts() {
    assert_eq!(resolve_import("src/tools/a.ts", "../gateway/complete.js").as_deref(), Some("src/gateway/complete.ts"));
    assert_eq!(resolve_import("src/main.ts", "./ui/App.js").as_deref(), Some("src/ui/App.tsx"));
    assert_eq!(resolve_import("src/main.ts", "./ui/App.jsx").as_deref(), Some("src/ui/App.tsx"));
    assert_eq!(resolve_import("src/main.ts", "./esm/mod.mjs").as_deref(), Some("src/esm/mod.mts"));
}

#[test]
fn ignores_bare_and_node_specifiers_missing_files_and_paths_above_the_root() {
    assert_eq!(resolve_import("src/a.ts", "zod"), None);
    assert_eq!(resolve_import("src/a.ts", "node:fs"), None);
    assert_eq!(resolve_import("src/a.ts", "./missing"), None);
    assert_eq!(resolve_import("src/a.ts", "../../outside"), None);
}
