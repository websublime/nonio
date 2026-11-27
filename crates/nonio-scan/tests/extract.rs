//! Fact extraction for TS/JS: symbols, calls, routes and entries, write adapters, large and broken files.
//! Ported from `packages/facts/test/extract.test.ts` of illuminate (MIT, Copyright (c) 2026 johnz4021).

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use nonio_facts::{settle_calls, EntryKind, FileFacts};
use nonio_scan::{
    extract_facts, module_record, normalize_route_path, ExtractContext, ModuleRecord, PackageEntry, RoutePattern,
};

fn files() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("src/lib/a.ts", "export function a() {}\nexport default function main() {}\n"),
        (
            "src/lib/index.ts",
            "export { a as alpha } from \"./a\";\nexport * from \"./b\";\nexport * as ns from \"./a\";\n",
        ),
        ("src/lib/b.ts", "export const b = () => 1;\n"),
    ])
}

fn resolve(from: &str, spec: &str) -> Option<String> {
    if !spec.starts_with('.') {
        return Some(format!("vendor:{}", spec.split('/').next().unwrap()));
    }
    let mut dir: Vec<&str> = from.split('/').collect();
    dir.pop();
    for seg in spec.split('/') {
        match seg {
            ".." => {
                dir.pop();
            }
            "." => {}
            s => dir.push(s),
        }
    }
    let base = dir.join("/");
    let files = files();
    [base.clone(), format!("{base}.ts"), format!("{base}/index.ts")]
        .into_iter()
        .find(|p| files.contains_key(p.as_str()) || p == "src/x.ts")
}

fn record_of(f: &str) -> Option<Rc<ModuleRecord>> {
    files().get(f).map(|t| Rc::new(module_record(f, t)))
}

struct Ctx {
    entries: Vec<PackageEntry>,
    routes: Vec<RoutePattern>,
    resolve: fn(&str, &str) -> Option<String>,
}

impl Default for Ctx {
    fn default() -> Self {
        Self { entries: Vec::new(), routes: Vec::new(), resolve }
    }
}

fn facts_with(src: &str, path: &str, c: &Ctx) -> FileFacts {
    let ctx = ExtractContext { resolve: &c.resolve, record_of: &record_of, entries: &c.entries, routes: &c.routes };
    extract_facts(path, src, &ctx)
}

fn facts_at(src: &str, path: &str) -> FileFacts {
    facts_with(src, path, &Ctx::default())
}

fn facts(src: &str) -> FileFacts {
    facts_at(src, "src/x.ts")
}

fn after_hash(id: &str) -> &str {
    id.split_once('#').map_or(id, |(_, n)| n)
}

fn kind(k: nonio_facts::SymbolKind) -> String {
    serde_json::to_value(k).unwrap().as_str().unwrap().to_string()
}

#[test]
fn names_functions_classes_methods_exported_consts_and_object_literal_members_ids_carry_no_line_numbers() {
    let f = facts(
        &[
            r#"import type { Tool } from "./types";"#,
            "export function run(a: number): number { return a; }",
            "function helper() {}",
            "export class Store extends Base { constructor() { super(); } get size() { return 1; } static make() {} private go = () => 1; }",
            r#"export const tool: Tool = { name: "t", run: async () => 1, nested: { deep() {} } };"#,
            "const local = { run() {} };",
            "export const LIMIT = 3, chat = run;",
            "export interface Shape { run(x: number): void; size: number }",
            "function dup() {}",
            "function dup() {}",
            "export { helper };",
        ]
        .join("\n"),
    );
    let got: Vec<String> = f
        .symbols
        .iter()
        .map(|s| {
            format!(
                "{}:{}{}{}",
                s.name,
                kind(s.kind),
                if s.exported { "*" } else { "" },
                s.parent.as_deref().map(|p| format!("<{}", after_hash(p))).unwrap_or_default()
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            "run:function*",
            "helper:function*",
            "Store:class*",
            "Store.constructor:method<Store",
            "Store.size:method<Store",
            "Store.make:method<Store",
            "Store.go:method<Store",
            "tool:const*",
            "tool.run:method<tool",
            "tool.nested.deep:method<tool",
            "local:const",
            "local.run:method<local",
            "LIMIT:const*",
            "chat:function*",
            "Shape:type*",
            "Shape.run:method<Shape",
            "Shape.size:property<Shape",
            "dup:function",
            "dup~2:function",
        ]
    );
    assert!(f.symbols.iter().all(|s| {
        let name = s.id.strip_prefix("src/x.ts#").unwrap();
        !name.starts_with(|c: char| c.is_ascii_digit() || c == ':') && s.span.end > s.span.start
    }));
}

#[test]
fn gives_top_level_anonymous_callbacks_cb_symbols_and_folds_callbacks_inside_a_symbol_into_it() {
    let f = facts("describe(\"x\", () => { go(); });\nexport function go() { [1].map((x) => x + 1).forEach(() => go()); }\nsetTimeout(function () {}, 1);\n");
    assert_eq!(f.symbols.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["<cb1>", "go", "<cb2>"]);
    assert_eq!(f.calls.len(), 1);
    assert_eq!(
        (f.calls[0].from.as_str(), f.calls[0].to.as_str(), f.calls[0].count),
        ("src/x.ts#<cb1>", "src/x.ts#go", 1)
    );
    assert_eq!(f.calls[0].confidence, nonio_facts::CallConfidence::Syntactic);
}

#[test]
fn names_functions_declared_inside_functions_and_the_methods_a_factory_returns_as_outer_inner() {
    let f = facts(
        &[
            r#"import { save as importedSave } from "./lib/a";"#,
            "export function startServer() {",
            "  publish();",
            "  function startMap() { publish(); helper(); }",
            "  const publish = () => importedSave();",
            "  [1].forEach(() => publish());",
            "  return { startMap, pageUrl: (id) => id, close() { publish(); } };",
            "}",
            "export const createStore = (root) => ({ get() { return root; }, save: async () => {} });",
            "function helper() {}",
        ]
        .join("\n"),
    );
    let got: Vec<String> = f
        .symbols
        .iter()
        .map(|s| {
            format!(
                "{}:{}{}",
                s.name,
                kind(s.kind),
                s.parent.as_deref().map(|p| format!("<{}", after_hash(p))).unwrap_or_default()
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            "startServer:function",
            "startServer.startMap:function<startServer",
            "startServer.publish:function<startServer",
            "startServer.pageUrl:method<startServer",
            "startServer.close:method<startServer",
            "createStore:function",
            "createStore.get:method<createStore",
            "createStore.save:method<createStore",
            "helper:function",
        ]
    );
    // Calls belong to the innermost named function; a callback still folds into the function around it. Calls
    // between nested functions resolve, including before the declaration.
    let mut calls: Vec<String> =
        f.calls.iter().map(|c| format!("{} → {}", after_hash(&c.from), after_hash(&c.to))).collect();
    calls.sort();
    let mut want = vec![
        "startServer → startServer.publish",
        "startServer.close → startServer.publish",
        "startServer.publish → save", // the import's own name
        "startServer.startMap → helper",
        "startServer.startMap → startServer.publish",
    ];
    want.sort();
    assert_eq!(calls, want);
}

#[test]
fn lets_a_nested_function_shadow_a_top_level_one_and_leaves_a_name_declared_twice_unresolved() {
    let f = facts(
        &[
            "function go() {}",
            "export function outer() { go(); function go() {} }",
            "export function twice() { pick(); const pick = () => 1; function pick() {} }",
        ]
        .join("\n"),
    );
    let calls: Vec<String> =
        f.calls.iter().map(|c| format!("{} → {}", after_hash(&c.from), after_hash(&c.to))).collect();
    assert_eq!(calls, ["outer → outer.go"]);
}

/// D16: an edge found by name alone never backs a fact. A parameter or local variable that only shares its name with
/// an import or a top-level symbol is not that symbol (illuminate links these by name).
#[test]
fn never_links_a_parameter_or_local_variable_to_an_import_or_top_level_symbol_of_the_same_name() {
    let f = facts(
        &[
            r#"import { a } from "./lib/a";"#,
            "export type input<T> = T;",
            "export function output() {}",
            "export function f(input: any, a: any) {",
            "  input.data.trim(); a(); { const output = () => 1; output(); }",
            r#"  const { a: viaImport } = require("./lib/a");"#,
            "  viaImport();",
            "  function inner() { const viaImport = 1; viaImport(); }",
            "  inner();",
            "}",
            "export function g() { a(); output(); }",
        ]
        .join("\n"),
    );
    let mut calls: Vec<String> = f.calls.iter().map(|c| format!("{} → {}", after_hash(&c.from), c.to)).collect();
    calls.sort();
    assert_eq!(calls, ["f → src/lib/a.ts#a", "f → src/x.ts#f.inner", "g → src/lib/a.ts#a", "g → src/x.ts#output"]);
}

#[test]
fn marks_components_in_tsx_files() {
    let f = facts_at(
        "export function Page() { return <div/>; }\nexport const Row = () => <span/>;\nexport const useThing = () => 1;\n",
        "src/ui/Page.tsx",
    );
    let got: Vec<String> = f.symbols.iter().map(|s| format!("{}:{}", s.name, kind(s.kind))).collect();
    assert_eq!(got, ["Page:component", "Row:component", "useThing:function"]);
}

#[test]
fn hashes_signatures_and_bodies_apart_ignoring_whitespace() {
    let first = |src: &str| facts(src).symbols.remove(0);
    let a = first("export function f(a: number) { return a + 1; }");
    let b = first("export function f(a: number) {\n  return a  + 1;\n}");
    let c = first("export function f(a: number) { return a + 2; }");
    let d = first("export function f(a: string) { return a + 1; }");
    let e = first("function f(a: number) { return a + 1; }");
    assert_eq!((&b.sig_hash, &b.body_hash), (&a.sig_hash, &a.body_hash));
    assert_eq!((c.sig_hash == a.sig_hash, c.body_hash == a.body_hash), (true, false));
    assert_ne!(d.sig_hash, a.sig_hash);
    assert_ne!(e.sig_hash, a.sig_hash); // the export flag is part of the signature
}

#[test]
fn links_named_default_namespace_barrel_and_dynamic_import_bindings_packages_become_vendor_ids() {
    let f = facts(
        &[
            r#"import main, { a } from "./lib/a";"#,
            r#"import { alpha, b, ns } from "./lib";"#,
            r#"import * as lib from "./lib";"#,
            r#"import OpenAI from "openai";"#,
            "export async function go() {",
            "  a(); main(); alpha(); b(); ns.a(); lib.b(); lib.alpha();",
            "  new OpenAI().chat.completions.create({});",
            r#"  const { a: viaImport } = await import("./lib/a");"#,
            "  viaImport();",
            "}",
        ]
        .join("\n"),
    );
    let mut got: Vec<String> = f.calls.iter().map(|c| format!("{} x{}", c.to, c.count)).collect();
    got.sort();
    assert_eq!(got, ["src/lib/a.ts#a x5", "src/lib/a.ts#main x1", "src/lib/b.ts#b x2", "vendor:openai x1"]);
}

#[test]
fn links_this_m_in_classes_and_settles_members_to_the_deepest_existing_symbol() {
    let f = facts("import { db } from \"./lib/b\";\nexport class S { a() { this.b(); db.insert.x(); } b() {} }\n");
    let got: Vec<String> = f.calls.iter().map(|c| format!("{} → {}", after_hash(&c.from), c.to)).collect();
    assert_eq!(got, ["S.a → src/x.ts#S.b", "S.a → src/lib/b.ts#db.insert.x"]);
    let names: HashSet<String> = ["db".to_string(), "db.insert".to_string()].into();
    let settled = settle_calls(&f.calls, |file| (file == "src/lib/b.ts").then_some(&names));
    assert_eq!(settled.iter().map(|c| c.to.as_str()).collect::<Vec<_>>(), ["src/x.ts#S.b", "src/lib/b.ts#db.insert"]);
    let none: HashSet<String> = HashSet::new();
    // A call into its own file with no symbol drops.
    assert_eq!(
        settle_calls(&f.calls, |_| Some(&none)).iter().map(|c| c.to.as_str()).collect::<Vec<_>>(),
        ["src/lib/b.ts"]
    );
}

#[test]
fn finds_express_hono_and_fastify_routes_only_on_instances_of_those_frameworks() {
    let f = facts(
        &[
            r#"import express from "express";"#,
            r#"import Fastify from "fastify";"#,
            r#"import { list } from "./lib/a";"#,
            "const app = express();",
            "const router = express.Router();",
            "const fastify = Fastify();",
            "const cache = new Map();",
            r#"app.get("/runs", list);"#,
            "router.post(`/runs/:id/stop`, auth, (req, res) => res.end());",
            r#"fastify.route({ method: ["GET", "HEAD"], url: "/health", handler: async () => "ok" });"#,
            r#"cache.get("/not-a-route", 1);"#,
            "export function auth() {}",
        ]
        .join("\n"),
    );
    let got: Vec<String> =
        f.routes.iter().map(|r| format!("{} → {}", r.id, r.handler.as_deref().unwrap_or("null"))).collect();
    assert_eq!(
        got,
        [
            "route:GET /runs → src/lib/a.ts#list",
            "route:POST /runs/{}/stop → src/x.ts#<cb1>",
            "route:GET /health → src/x.ts#<cb2>",
            "route:HEAD /health → src/x.ts#<cb2>",
        ]
    );
    assert!(f.entries.iter().any(|e| e.id == "entry:route:GET /runs"));
}

#[test]
fn reads_next_js_app_and_pages_routes_from_the_file_path_and_exports() {
    let r = facts_at(
        "export async function GET() {}\nexport const POST = async () => {};\nexport function helper() {}\n",
        "web/app/(shop)/api/runs/[id]/route.ts",
    );
    let got: Vec<String> =
        r.routes.iter().map(|x| format!("{} → {}", x.id, x.handler.as_deref().unwrap_or("null"))).collect();
    assert_eq!(
        got,
        [
            "route:GET /api/runs/{} → web/app/(shop)/api/runs/[id]/route.ts#GET",
            "route:POST /api/runs/{} → web/app/(shop)/api/runs/[id]/route.ts#POST",
        ]
    );
    let api = facts_at("export default function handler() {}", "pages/api/users/index.ts");
    assert_eq!(api.routes.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), ["route:ANY /api/users"]);
    let page = facts_at("export default function Page() {}", "src/app/runs/[id]/page.tsx");
    let got: Vec<String> =
        page.entries.iter().map(|e| format!("{} {}", e.id, e.symbol.as_deref().unwrap_or("null"))).collect();
    assert_eq!(got, ["entry:page:/runs/{} src/app/runs/[id]/page.tsx#Page"]);
}

#[test]
fn applies_route_patterns_with_the_handler_defined_after_the_match() {
    let src = "const routes = [\n  [\"GET\", /^\\/api\\/runs\\/([\\w-]+)$/, ([id]) => getRun(id)],\n  [\"POST\", /^\\/api\\/runs$/, async () => start()],\n];\n";
    let c = Ctx {
        routes: vec![RoutePattern {
            file: "src/channels/*.ts".into(),
            pattern: r#"\[\s*"(?<method>[A-Z]+)",\s*/\^(?<path>(?:\\.|[^$])*)\$/"#.into(),
        }],
        ..Ctx::default()
    };
    let f = facts_with(src, "src/channels/routes.ts", &c);
    let got: Vec<String> =
        f.routes.iter().map(|r| format!("{} → {}", r.id, r.handler.as_deref().unwrap_or("null"))).collect();
    assert_eq!(
        got,
        [
            "route:GET /api/runs/{} → src/channels/routes.ts#<cb1>",
            "route:POST /api/runs → src/channels/routes.ts#<cb2>"
        ]
    );
}

#[test]
fn normalizes_route_params() {
    let got: Vec<String> = ["/a/:id", "/a/{id}/b", "/a/[id]", "/a/[...slug]", r"^\/a\/([\w-]+)$", "/a/*", "a/"]
        .iter()
        .map(|p| normalize_route_path(p))
        .collect();
    assert_eq!(got, ["/a/{}", "/a/{}/b", "/a/{}", "/a/{}", "/a/{}", "/a/{}", "/a"]);
}

#[test]
fn turns_package_json_entries_into_bin_script_and_cli_entries() {
    let c = Ctx {
        entries: vec![
            PackageEntry { kind: EntryKind::Script, label: "dev".into() },
            PackageEntry { kind: EntryKind::Bin, label: "x".into() },
        ],
        ..Ctx::default()
    };
    let e = |src: &str| facts_with(src, "src/x.ts", &c).entries.into_iter().map(|x| x.id).collect::<Vec<_>>();
    assert_eq!(e("main(process.argv.slice(2));"), ["entry:cli:dev", "entry:bin:x"]);
    assert_eq!(e("if (import.meta.url === process.argv[1]) start();"), ["entry:script:dev", "entry:bin:x"]);
}

#[test]
fn finds_prisma_drizzle_knex_kysely_and_raw_sql_writes_but_not_reads_or_prose() {
    let f = facts(
        &[
            "await prisma.user.create({ data });",
            "await tx.post.updateMany({});",
            "await db.insert(users).values({ id });",
            "await db.update(orders).set({ paid: true }).where(eq(orders.id, id));",
            r#"await knex("audit").where({ id }).update({ seen: true });"#,
            r#"await trx.table("events").insert(rows);"#,
            r#"await db.insertInto("person").values(v).execute();"#,
            "await sql`INSERT INTO runs (id) VALUES (${id})`;",
            r#"await pool.query("UPDATE contacts SET name = $1", [n]);"#,
            r#"await pool.query("DELETE FROM notes WHERE id = 1");"#,
            r#"await prisma.user.findMany(); await knex("audit").select("*"); await db.select().from(users);"#,
            r#"const label = "Delete from cart"; const t = "UPDATE your profile";"#,
            r#"// db.insert("runs", r);"#,
        ]
        .join("\n"),
    );
    let got: Vec<String> = f.writes.iter().map(|w| format!("{}:{}", w.via, w.table)).collect();
    assert_eq!(
        got,
        [
            "prisma:user",
            "prisma:post",
            "drizzle:users",
            "drizzle:orders",
            "knex:audit",
            "knex:events",
            "kysely:person",
            "sql:runs",
            "sql:contacts",
            "sql:notes",
        ]
    );
}

#[test]
fn records_only_imports_and_writes_for_files_over_500_kb_or_minified() {
    let big = format!(
        "import {{ a }} from \"./lib/a\";\nexport const blob = \"{}\";\nexport function f() {{ a(); }}\ndb.insert(\"runs\", r);\n",
        "A".repeat(600_000)
    );
    let f = facts(&big);
    assert_eq!(f.imports.len(), 1);
    assert_eq!((f.imports[0].to.as_str(), f.imports[0].type_only), ("src/lib/a.ts", false));
    assert_eq!(f.writes.iter().map(|w| w.table.as_str()).collect::<Vec<_>>(), ["runs"]);
    assert!(f.symbols.is_empty());
}

#[test]
fn falls_back_to_the_regex_reader_for_source_with_syntax_errors() {
    let f = facts("import { a } from \"./lib/a\";\nexport function broken( {\ndb.insert(\"runs\", r);\n");
    assert_eq!(f.imports.iter().map(|i| i.to.as_str()).collect::<Vec<_>>(), ["src/lib/a.ts"]);
    assert_eq!(f.writes.iter().map(|w| w.table.as_str()).collect::<Vec<_>>(), ["runs"]);
}
