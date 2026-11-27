//! Vue single-file components (D28): `<script>` and `<script setup>` read in place, the template lowered into the
//! same oxc `Program`.
//!
//! The first test ports the Q11 spike harness (`tools/spike-vue/src/main.rs`) and its 16 hand-written expectations
//! (`fixtures/vue-app/expected.txt`) from `vue_oxlint_jsx` 0.15 to nonio's own lowering: for every call and template
//! reference it checks the text as written in the `.vue`, the line, that `source[span]` slices back to that text, and
//! what oxc_semantic resolves the root name to. The rest check the facts the extractor gives.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use nonio_facts::{settle_calls, symbol_names, Extractor, FileFacts, RepoView};
use nonio_scan::{extract_facts, parse_sfc, ExtractContext, OxcExtractor};
use oxc_allocator::Allocator;
use oxc_ast::ast::{
    CallExpression, Expression, IdentifierReference, ImportDeclaration, ImportDeclarationSpecifier, JSXElementName,
    JSXExpressionContainer,
};
use oxc_ast_visit::{walk, Visit};
use oxc_semantic::{Scoping, SemanticBuilder, SymbolFlags};
use oxc_span::{GetSpan, Span};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vue-app")
}

// ---- the spike harness, ported

/// One observation: a call (`call`), or a name the template uses (`ref`, `component`).
struct Seen {
    kind: &'static str,
    text: String,
    span: Span,
    faithful: bool,
    target: String,
}

/// Byte ranges of the top-level blocks, read with a plain scan (independent of the backend).
fn blocks(source: &str) -> Vec<(&'static str, usize, usize)> {
    let mut out = Vec::new();
    for tag in ["script", "template"] {
        let mut from = 0;
        while let Some(i) = source[from..].find(&format!("<{tag}")) {
            let open = from + i;
            let head_end = open + source[open..].find('>').unwrap() + 1;
            let close = if tag == "script" {
                head_end + source[head_end..].find("</script>").unwrap()
            } else {
                head_end + source[head_end..].rfind("</template>").unwrap()
            };
            let setup = source[open..head_end].contains("setup");
            out.push((if setup { "script setup" } else { tag }, head_end, close));
            from = close;
            if tag == "template" {
                break;
            }
        }
    }
    out
}

fn block_of(bl: &[(&'static str, usize, usize)], at: u32) -> &'static str {
    let at = at as usize;
    bl.iter().find(|(_, s, e)| *s <= at && at < *e).map_or("outside", |b| b.0)
}

fn line_col(source: &str, at: u32) -> (usize, usize) {
    let before = &source[..at as usize];
    let line = before.matches('\n').count() + 1;
    let col = before.chars().count() - before.rfind('\n').map_or(0, |i| before[..=i].chars().count()) + 1;
    (line, col)
}

struct Collector<'s> {
    source: &'s str,
    scoping: &'s Scoping,
    blocks: Vec<(&'static str, usize, usize)>,
    imports: BTreeMap<String, String>,
    seen: Vec<Seen>,
    /// Every identifier reference with a span in the template: name, span.
    names: Vec<(String, Span)>,
}

impl<'s> Collector<'s> {
    fn resolve(&self, id: &IdentifierReference<'_>) -> String {
        let Some(rid) = id.reference_id.get() else { return "no-reference-id".into() };
        let Some(sym) = self.scoping.get_reference(rid).symbol_id() else {
            return "unresolved".into();
        };
        let name = self.scoping.symbol_name(sym).to_string();
        let decl = self.scoping.symbol_span(sym);
        let depth = self.scoping.scope_ancestors(self.scoping.symbol_scope_id(sym)).count() - 1;
        if self.scoping.symbol_flags(sym).contains(SymbolFlags::Import) {
            let spec = self.imports.get(&name).cloned().unwrap_or_default();
            return format!("import {name} from '{spec}'");
        }
        let (l, c) = line_col(self.source, decl.start);
        let where_ = block_of(&self.blocks, decl.start);
        format!("binding {name} declared in <{where_}> at {l}:{c}, scope depth {depth}")
    }

    fn push(&mut self, kind: &'static str, text: String, span: Span, target: String) {
        let faithful = self.source.get(span.start as usize..span.end as usize) == Some(text.as_str());
        self.seen.push(Seen { kind, text, span, faithful, target });
    }
}

fn callee_root<'n, 'a>(e: &'n Expression<'a>) -> Option<&'n IdentifierReference<'a>> {
    match e {
        Expression::Identifier(i) => Some(i),
        Expression::StaticMemberExpression(m) => callee_root(&m.object),
        Expression::ComputedMemberExpression(m) => callee_root(&m.object),
        Expression::CallExpression(c) => callee_root(&c.callee),
        _ => None,
    }
}

impl<'a> Visit<'a> for Collector<'_> {
    fn visit_import_declaration(&mut self, it: &ImportDeclaration<'a>) {
        for s in it.specifiers.iter().flatten() {
            let local = match s {
                ImportDeclarationSpecifier::ImportSpecifier(s) => &s.local,
                ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => &s.local,
                ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => &s.local,
            };
            self.imports.insert(local.name.to_string(), it.source.value.to_string());
        }
        walk::walk_import_declaration(self, it);
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        let span = it.callee.span();
        let text = self.source.get(span.start as usize..span.end as usize).unwrap_or("<out of range>").to_string();
        let target = callee_root(&it.callee).map_or_else(|| "no root identifier".into(), |r| self.resolve(r));
        // A handler given by reference (`@click="refresh"`) is lowered to a call with an empty span, because Vue
        // calls it on the event: it is still a reference as written.
        let kind = if it.span.is_empty() { "ref" } else { "call" };
        self.push(kind, text, span, target);
        walk::walk_call_expression(self, it);
    }

    fn visit_jsx_expression_container(&mut self, it: &JSXExpressionContainer<'a>) {
        if let Some(Expression::Identifier(i)) = it.expression.as_expression() {
            let target = self.resolve(i);
            self.push("ref", i.name.to_string(), i.span, target);
        }
        walk::walk_jsx_expression_container(self, it);
    }

    fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
        if block_of(&self.blocks, it.span.start) == "template" && !it.span.is_empty() {
            self.names.push((it.name.to_string(), it.span));
        }
    }

    fn visit_jsx_element_name(&mut self, it: &JSXElementName<'a>) {
        if let JSXElementName::IdentifierReference(i) = it {
            let target = self.resolve(i);
            self.push("component", i.name.to_string(), i.span, target);
        }
        walk::walk_jsx_element_name(self, it);
    }
}

fn vue_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            vue_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "vue") {
            out.push(p);
        }
    }
}

#[test]
fn the_spike_expectations_hold_with_every_position_in_the_vue_file() {
    let root = fixture();
    let mut files = Vec::new();
    vue_files(&root, &mut files);
    files.sort();
    assert_eq!(files.len(), 3);

    // (file, line, observation)
    let mut all: Vec<(String, usize, Seen)> = Vec::new();
    for f in &files {
        let rel = f.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
        let source = fs::read_to_string(f).unwrap();
        let allocator = Allocator::default();
        let sfc = parse_sfc(&allocator, &source).expect("the SFC parses");
        assert!(!sfc.ret.fatal_error && !sfc.ret.diagnostics.has_errors(), "{rel}: script parse errors");
        let sem = SemanticBuilder::new().build(&sfc.ret.program);
        assert!(!sem.diagnostics.has_errors(), "{rel}: semantic errors");
        let scoping = sem.semantic.into_scoping();
        let mut c = Collector {
            source: &source,
            scoping: &scoping,
            blocks: blocks(&source),
            imports: BTreeMap::new(),
            seen: Vec::new(),
            names: Vec::new(),
        };
        c.visit_program(&sfc.ret.program);
        // Every name the template uses has a span that slices back to it in the `.vue` file.
        for (name, span) in &c.names {
            assert_eq!(&source[span.start as usize..span.end as usize], name, "{rel}: {name} at {span:?}");
        }
        for s in c.seen {
            assert!(s.faithful, "{rel}: {} {} at {:?} does not slice back", s.kind, s.text, s.span);
            let line = line_col(&source, s.span.start).0;
            all.push((rel.clone(), line, s));
        }
    }

    let expected = fs::read_to_string(root.join("expected.txt")).unwrap();
    let mut failed = Vec::new();
    let mut total = 0;
    for line in expected.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let p: Vec<&str> = line.split('|').map(str::trim).collect();
        let [file, kind, text, at, want] = p[..] else { panic!("bad line: {line}") };
        let at: usize = at.parse().unwrap();
        total += 1;
        let hit = all.iter().find(|(f, l, s)| f == file && *l == at && s.kind == kind && s.text == text);
        match hit {
            // Without `<script setup>` the template is not lowered at all (it sees the component instance, not the
            // module), so nothing in it can resolve.
            None if want == "not lowered" => {}
            Some((_, _, s)) if want == "not lowered" => {
                failed.push(format!("{file}:{at} {kind} {text}: want it not lowered, got '{}'", s.target))
            }
            Some((_, _, s)) if s.target.contains(want) => {}
            Some((_, _, s)) => failed.push(format!("{file}:{at} {kind} {text}: want '{want}', got '{}'", s.target)),
            None => failed.push(format!("{file}:{at} {kind} {text}: not seen")),
        }
    }
    assert_eq!(total, 16);
    assert!(failed.is_empty(), "{} of {total} failed:\n{}", failed.len(), failed.join("\n"));
}

// ---- facts

struct Disk(PathBuf);

impl RepoView for Disk {
    fn exists(&self, path: &str) -> bool {
        self.0.join(path).is_file()
    }
    fn text(&self, path: &str) -> Option<Rc<str>> {
        fs::read_to_string(self.0.join(path)).ok().map(Rc::from)
    }
}

fn scan_fixture() -> HashMap<String, FileFacts> {
    let root = fixture();
    let view = Disk(root.clone());
    let oxc = OxcExtractor::new(&root);
    let files = [
        "src/components/OrderList.vue",
        "src/components/OrderRow.vue",
        "src/components/LegacyCounter.vue",
        "src/composables/useOrders.ts",
        "src/api/client.ts",
        "src/utils/format.ts",
    ];
    let mut facts = HashMap::new();
    for f in files {
        let text = view.text(f).unwrap();
        facts.insert(f.to_string(), oxc.extract(&view, f, &text).unwrap());
    }
    let names = symbol_names(&facts);
    for f in facts.values_mut() {
        f.calls = settle_calls(&f.calls, |file| names.get(file));
    }
    facts
}

fn calls(f: &FileFacts) -> Vec<String> {
    let mut out: Vec<String> = f.calls.iter().map(|c| format!("{} -> {}", short(&c.from), short(&c.to))).collect();
    out.sort();
    out
}

fn short(id: &str) -> &str {
    id.strip_prefix("src/").unwrap_or(id)
}

fn symbols(f: &FileFacts) -> Vec<String> {
    f.symbols.iter().map(|s| short(&s.id).to_string()).collect()
}

#[test]
fn script_setup_calls_and_template_calls_link_to_what_they_resolve_to() {
    let facts = scan_fixture();
    let list = &facts["src/components/OrderList.vue"];
    let source = fs::read_to_string(fixture().join("src/components/OrderList.vue")).unwrap();

    assert_eq!(
        list.imports.iter().map(|i| short(&i.to)).collect::<Vec<_>>(),
        ["vendor:vue", "components/OrderRow.vue", "composables/useOrders.ts", "utils/format.ts", "api/client.ts"]
    );
    assert_eq!(
        symbols(list),
        [
            "components/OrderList.vue#<cb1>",
            "components/OrderList.vue#refresh",
            "components/OrderList.vue#pick",
            "components/OrderList.vue#<cb2>",
            "components/OrderList.vue#<template>",
        ]
    );
    // `load` (destructured from `useOrders()`) and `o.load` (the `v-for` variable) link to nothing: neither the
    // composable's `load` nor `utils/format.ts#load` (D16).
    assert_eq!(
        calls(list),
        [
            "components/OrderList.vue -> composables/useOrders.ts#useOrders",
            "components/OrderList.vue -> vendor:vue",
            "components/OrderList.vue#<cb2> -> components/OrderList.vue#refresh",
            "components/OrderList.vue#<template> -> components/OrderList.vue#pick",
            "components/OrderList.vue#<template> -> components/OrderList.vue#refresh",
            "components/OrderList.vue#<template> -> utils/format.ts#formatTotal",
            "components/OrderList.vue#refresh -> api/client.ts#http.get",
        ]
    );
    let count =
        |from: &str, to: &str| list.calls.iter().find(|c| c.from.ends_with(from) && c.to.ends_with(to)).unwrap().count;
    assert_eq!(count("vue", "vendor:vue"), 2);

    // Spans are offsets in the `.vue` file.
    let span_text = |name: &str| {
        let s = list.symbols.iter().find(|s| s.name == name).unwrap();
        &source[s.span.start..s.span.end]
    };
    assert!(span_text("refresh").starts_with("function refresh() {"));
    assert!(span_text("<template>").starts_with("<template>\n  <section>"));
    assert!(span_text("<template>").ends_with("</section>\n</template>"));
}

#[test]
fn compiler_macros_and_template_names_that_are_not_symbols_give_no_calls() {
    let facts = scan_fixture();
    let row = &facts["src/components/OrderRow.vue"];
    // `defineProps`/`defineEmits` are compiler macros, and `emit` is a plain const: no call.
    assert!(row.imports.is_empty());
    assert_eq!(symbols(row), ["components/OrderRow.vue#<template>"]);
    assert!(calls(row).is_empty(), "{:?}", calls(row));
}

#[test]
fn options_api_script_is_read_and_its_template_links_nothing_by_name() {
    let facts = scan_fixture();
    let legacy = &facts["src/components/LegacyCounter.vue"];
    assert_eq!(
        symbols(legacy),
        [
            "components/LegacyCounter.vue#load",
            "components/LegacyCounter.vue#default",
            "components/LegacyCounter.vue#<template>"
        ]
    );
    // `{{ show() }}` is a method of the component instance, not a binding of the module: no call from the template.
    assert_eq!(
        calls(legacy),
        [
            "components/LegacyCounter.vue#default -> components/LegacyCounter.vue#load",
            "components/LegacyCounter.vue#default -> utils/format.ts#formatTotal",
            "components/LegacyCounter.vue#default -> vendor:vue",
        ]
    );
}

// ---- single files

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
    let p = dir.join("/");
    Some(if p.ends_with(".vue") || p.ends_with(".ts") { p } else { format!("{p}.ts") })
}

fn facts(src: &str) -> FileFacts {
    let ctx = ExtractContext { resolve: &resolve, record_of: &|_| None, entries: &[], routes: &[] };
    extract_facts("src/C.vue", src, &ctx)
}

fn file_calls(src: &str) -> Vec<String> {
    let f = facts(src);
    let mut out: Vec<String> = f
        .calls
        .iter()
        .map(|c| format!("{} -> {}", c.from.trim_start_matches("src/C.vue"), c.to.trim_start_matches("src/C.vue")))
        .collect();
    out.sort();
    out
}

#[test]
fn slot_props_and_v_for_variables_shadow_setup_names() {
    let got = file_calls(
        r#"<script setup lang="ts">
import { item, row } from './names'
function open(x: unknown) { return x }
</script>

<template>
  <List v-slot="{ item }">
    <span @click="item.open()">{{ open(item) }}</span>
  </List>
  <Table>
    <template #row="{ row: r }">{{ r.id }}</template>
  </Table>
  <li v-for="(row, i) in rows" :key="i" @click="row()">{{ item() }}</li>
</template>
"#,
    );
    // `item.open()` and `row()` are the slot prop and the v-for variable; `item()` outside the slot is the import.
    assert_eq!(got, ["#<template> -> #open", "#<template> -> src/names.ts#item"]);
}

#[test]
fn v_if_is_outside_the_v_for_scope_and_handlers_may_be_statements() {
    let got = file_calls(
        r#"<script setup>
import { ok, save, close, track } from './actions'
const x = 1
</script>
<template>
  <div v-for="ok in list" v-if="ok()" @click="save(ok); close()">x</div>
  <button @click="() => track('b')" @keyup.enter="track">go</button>
</template>
"#,
    );
    // `v-if` is evaluated before `v-for` in Vue 3, so its `ok` is the import; inside the element `ok` is the variable.
    assert_eq!(
        got,
        [
            "#<template> -> src/actions.ts#close",
            "#<template> -> src/actions.ts#ok",
            "#<template> -> src/actions.ts#save",
            "#<template> -> src/actions.ts#track",
        ]
    );
    let f = facts(
        r#"<script setup>
import { track } from './actions'
</script>
<template><a @click="() => track('b')" @keyup.enter="track">go</a></template>
"#,
    );
    assert_eq!(f.calls.len(), 1);
    assert_eq!(f.calls[0].count, 2);
}

#[test]
fn script_and_script_setup_are_both_visible_to_the_template() {
    let f = facts(
        r#"<script lang="ts">
export function helper() { return 1 }
export default { name: 'C' }
</script>
<script setup lang="ts">
import { api } from './api'
const run = () => api.get(helper())
</script>
<template><b @click="run">{{ helper() }}</b></template>
"#,
    );
    let mut got: Vec<String> = f.calls.iter().map(|c| format!("{} -> {}", c.from, c.to)).collect();
    got.sort();
    assert_eq!(
        got,
        [
            "src/C.vue#<template> -> src/C.vue#helper",
            "src/C.vue#<template> -> src/C.vue#run",
            "src/C.vue#run -> src/C.vue#helper",
            "src/C.vue#run -> src/api.ts#api.get",
        ]
    );
    assert!(f.symbols.iter().any(|s| s.id == "src/C.vue#helper" && s.exported));
}

#[test]
fn positions_stay_byte_offsets_after_non_ascii_text() {
    // vue-compiler-core counts positions in chars; the facts must carry byte offsets of the `.vue` file.
    let src = "<!-- Olá, ação → ✓ -->\n<script setup>\n// ção\nimport { f } from './f'\nfunction go() { f() }\n</script>\n<template><p title=\"é\">Olá {{ go() }} ✓ <i @click=\"go\">ü</i></p></template>\n";
    let f = facts(src);
    let go = f.symbols.iter().find(|s| s.name == "go").unwrap();
    assert_eq!(&src[go.span.start..go.span.end], "function go() { f() }");
    let t = f.symbols.iter().find(|s| s.name == "<template>").unwrap();
    assert!(src[t.span.start..t.span.end].starts_with("<template><p"));
    assert!(src[t.span.start..t.span.end].ends_with("</p></template>"));
    let mut got: Vec<String> = f.calls.iter().map(|c| format!("{} -> {} x{}", c.from, c.to, c.count)).collect();
    got.sort();
    assert_eq!(got, ["src/C.vue#<template> -> src/C.vue#go x2", "src/C.vue#go -> src/f.ts#f x1"]);

    let allocator = Allocator::default();
    let sfc = parse_sfc(&allocator, src).unwrap();
    let sem = SemanticBuilder::new().build(&sfc.ret.program);
    let scoping = sem.semantic.into_scoping();
    let mut c = Collector {
        source: src,
        scoping: &scoping,
        blocks: blocks(src),
        imports: BTreeMap::new(),
        seen: Vec::new(),
        names: Vec::new(),
    };
    c.visit_program(&sfc.ret.program);
    assert!(c.seen.iter().all(|s| s.faithful));
    assert_eq!(c.names.len(), 2);
}

#[test]
fn a_broken_template_expression_drops_that_expression_only() {
    let got = file_calls(
        r#"<script setup>
import { a, b } from './x'
</script>
<template><p>{{ a( }}</p><p>{{ b() }}</p></template>
"#,
    );
    assert_eq!(got, ["#<template> -> src/x.ts#b"]);
}

#[test]
fn a_scope_that_does_not_parse_drops_its_element_instead_of_linking_by_name() {
    let got = file_calls(
        r#"<script setup>
function o() {}
function keep() {}
</script>
<template>
  <p v-for="o in in list">{{ o() }}</p>
  <Comp v-slot="{ o">{{ o() }}</Comp>
  <i>{{ keep() }}</i>
</template>
"#,
    );
    assert_eq!(got, ["#<template> -> #keep"]);
}

#[test]
fn components_kebab_case_and_v_pre() {
    let src = r#"<script setup>
import OrderRow from './OrderRow.vue'
import { fmt } from './fmt'
</script>
<template>
  <order-row :value="fmt(1)" />
  <OrderRow />
  <div v-pre>{{ fmt(2) }}</div>
</template>
"#;
    let f = facts(src);
    let got: Vec<String> = f.calls.iter().map(|c| format!("{} -> {} x{}", c.from, c.to, c.count)).collect();
    // A component used in the template is not a call (as in TSX); the import already says it. `v-pre` is not
    // compiled.
    assert_eq!(got, ["src/C.vue#<template> -> src/fmt.ts#fmt x1"]);
    assert_eq!(f.imports.len(), 2);
}

#[test]
fn templates_vue_cannot_read_and_files_it_cannot_parse() {
    // A template in another language is left alone; the script is still read.
    let got = file_calls(
        "<script setup>\nimport { a } from './a'\na()\n</script>\n<template lang=\"pug\">\np {{ a() }}\n</template>\n",
    );
    assert_eq!(got, [" -> src/a.ts#a"]);

    // Unclosed markup: no symbols or calls, imports through the regex fallback, as for any file that doesn't parse.
    let f = facts("<script setup>\nimport { a } from './a'\na()\n</script>\n<template><p {{ a() }}\n");
    assert!(f.symbols.is_empty() && f.calls.is_empty());
    assert_eq!(f.imports.iter().map(|i| i.to.as_str()).collect::<Vec<_>>(), ["src/a.ts"]);
}
