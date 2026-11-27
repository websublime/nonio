//! Q11 spike: does `vue_oxlint_jsx` (vue-oxlint-toolkit) give the checker what it needs from a Vue SFC?
//!
//! For each `.vue` file of the fixture it parses the SFC into one oxc `Program`, builds oxc_semantic over it, and
//! prints every call and every template reference with: where it is in the `.vue` file (byte span, line:col), whether
//! the span slices back to the same text in the original file, and what oxc_semantic resolves its root to (an import,
//! a binding of `<script setup>`, a template-local such as a `v-for` variable, or nothing). Then it scores the
//! hand-written expectations in `expected.txt`.
//!
//! Run: `cargo run --manifest-path tools/spike-vue/Cargo.toml -- tools/spike-vue/fixture`

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    CallExpression, Expression, IdentifierReference, ImportDeclaration, ImportDeclarationSpecifier, JSXElementName,
    JSXExpressionContainer, Program,
};
use oxc_ast_visit::{walk, Visit};
use oxc_semantic::{Scoping, SemanticBuilder, SymbolFlags};
use oxc_span::{GetSpan, Span};
use vue_oxlint_jsx::VueJsxParser;

/// One observation: a call (`call`), or a name the template uses (`ref`, `component`).
#[derive(Debug)]
struct Seen {
    kind: &'static str,
    /// Callee or name, as written.
    text: String,
    span: Span,
    /// Which block of the SFC the span falls in.
    block: &'static str,
    /// `source[span] == text`, i.e. the span points into the original `.vue` text.
    faithful: bool,
    /// What the root name resolves to.
    target: String,
}

/// Byte ranges of the top-level blocks, read from the original text with a plain scan (independent of the crate).
fn blocks(source: &str) -> Vec<(&'static str, usize, usize)> {
    let mut out = Vec::new();
    for (tag, name_plain, name_setup) in [("script", "script", "script setup"), ("template", "template", "template")] {
        let mut from = 0;
        while let Some(i) = source[from..].find(&format!("<{tag}")) {
            let open = from + i;
            let head_end = open + source[open..].find('>').unwrap() + 1;
            let close = source[head_end..].rfind(&format!("</{tag}>")).map(|j| head_end + j);
            let close =
                if tag == "script" { head_end + source[head_end..].find("</script>").unwrap() } else { close.unwrap() };
            let setup = source[open..head_end].contains("setup");
            out.push((if setup { name_setup } else { name_plain }, head_end, close));
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
    let col = before.len() - before.rfind('\n').map_or(0, |i| i + 1) + 1;
    (line, col)
}

struct Collector<'s, 'a> {
    source: &'s str,
    scoping: &'s Scoping,
    blocks: Vec<(&'static str, usize, usize)>,
    /// Local name → module specifier, for every import binding.
    imports: BTreeMap<String, String>,
    seen: Vec<Seen>,
    /// Every identifier reference in the template (or with an empty, synthetic span): name, span, target.
    names: Vec<(String, Span, String)>,
    _p: std::marker::PhantomData<&'a ()>,
}

impl<'s, 'a> Collector<'s, 'a> {
    fn resolve(&self, id: &IdentifierReference<'a>) -> String {
        let Some(rid) = id.reference_id.get() else { return "no-reference-id".into() };
        let Some(sym) = self.scoping.get_reference(rid).symbol_id() else {
            return "unresolved".into();
        };
        let name = self.scoping.symbol_name(sym).to_string();
        let flags = self.scoping.symbol_flags(sym);
        let decl = self.scoping.symbol_span(sym);
        let depth = self.scoping.scope_ancestors(self.scoping.symbol_scope_id(sym)).count() - 1;
        let where_ = block_of(&self.blocks, decl.start);
        if flags.contains(SymbolFlags::Import) {
            let spec = self.imports.get(&name).cloned().unwrap_or_default();
            return format!("import {name} from '{spec}'");
        }
        let (l, c) = line_col(self.source, decl.start);
        format!("binding {name} declared in <{where_}> at {l}:{c}, scope depth {depth}")
    }

    fn push(&mut self, kind: &'static str, text: String, span: Span, target: String) {
        let faithful = self.source.get(span.start as usize..span.end as usize) == Some(text.as_str());
        let block = block_of(&self.blocks, span.start);
        self.seen.push(Seen { kind, text, span, block, faithful, target });
    }
}

/// The root identifier of a callee (`a` in `a.b.c()`), when there is one.
fn callee_root<'n, 'a>(e: &'n Expression<'a>) -> Option<&'n IdentifierReference<'a>> {
    match e {
        Expression::Identifier(i) => Some(i),
        Expression::StaticMemberExpression(m) => callee_root(&m.object),
        Expression::ComputedMemberExpression(m) => callee_root(&m.object),
        Expression::CallExpression(c) => callee_root(&c.callee),
        _ => None,
    }
}

impl<'s, 'a> Visit<'a> for Collector<'s, 'a> {
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
        let text = self.source.get(span.start as usize..span.end as usize).unwrap_or("<span out of range>").to_string();
        let target = callee_root(&it.callee).map_or_else(|| "no root identifier".into(), |r| self.resolve(r));
        self.push("call", text, span, target);
        walk::walk_call_expression(self, it);
    }

    fn visit_jsx_expression_container(&mut self, it: &JSXExpressionContainer<'a>) {
        // A bare name in a template expression (`@click="refresh"`, `{{ total }}`) is a reference, not a call.
        if let Some(Expression::Identifier(i)) = it.expression.as_expression() {
            let target = self.resolve(i);
            self.push("ref", i.name.to_string(), i.span, target);
        }
        walk::walk_jsx_expression_container(self, it);
    }

    fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
        if block_of(&self.blocks, it.span.start) == "template" || it.span.is_empty() {
            let target = self.resolve(it);
            self.names.push((it.name.to_string(), it.span, target));
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

/// The shape of the Program the crate builds: top-level statement kinds, and where the `<script setup>` body ends up.
fn shape(program: &Program<'_>) -> String {
    let mut out = String::new();
    for s in &program.body {
        let span = s.span();
        let kind = format!("{s:?}");
        let kind = kind.split(['(', ' ']).next().unwrap_or("?").to_string();
        let _ = write!(out, "{kind}@{}..{} ", span.start, span.end);
    }
    out
}

fn main() {
    let root = std::env::args().nth(1).map(PathBuf::from).expect("usage: spike-vue <fixture dir>");
    let mut files: Vec<PathBuf> = Vec::new();
    collect(&root, &mut files);
    files.sort();

    let mut all: Vec<(String, Seen)> = Vec::new();
    for f in &files {
        let rel = f.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
        let source = std::fs::read_to_string(f).unwrap();
        let allocator = Allocator::default();
        let ret = VueJsxParser::new(&allocator, &source).parse();
        println!("== {rel}");
        println!("   panicked={} errors={}", ret.panicked, ret.errors.len());
        for e in &ret.errors {
            println!("   error: {e}");
        }
        println!("   program body: {}", shape(&ret.program));
        let sem = SemanticBuilder::new().build(&ret.program);
        println!("   semantic errors: {}", sem.errors.len());
        for e in &sem.errors {
            println!("   semantic error: {e}");
        }
        let scoping = sem.semantic.into_scoping();
        let mut c = Collector {
            source: &source,
            scoping: &scoping,
            blocks: blocks(&source),
            imports: BTreeMap::new(),
            seen: Vec::new(),
            names: Vec::new(),
            _p: std::marker::PhantomData,
        };
        c.visit_program(&ret.program);
        let synthetic = c.seen.iter().filter(|s| s.span.is_empty()).count();
        let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
        for (_, sp, t) in &c.names {
            let k = if sp.is_empty() {
                "synthetic span".to_string()
            } else {
                t.split(" declared").next().unwrap().split(' ').next().unwrap().to_string()
                    + &t.split(" declared in ")
                        .nth(1)
                        .map(|r| format!(" in {}", r.split(" at").next().unwrap()))
                        .unwrap_or_default()
            };
            *kinds.entry(k).or_default() += 1;
        }
        let unfaithful = c
            .names
            .iter()
            .filter(|(n, sp, _)| !sp.is_empty() && source.get(sp.start as usize..sp.end as usize) != Some(n.as_str()))
            .count();
        println!("   template identifier references: {} {:?}; span not slicing back to the name: {unfaithful}; calls with an empty span: {synthetic}", c.names.len(), kinds);
        for s in c.seen {
            let (l, col) = line_col(&source, s.span.start);
            println!(
                "   {:<9} {:<28} <{}> {}:{} [{}..{}] faithful={} -> {}",
                s.kind, s.text, s.block, l, col, s.span.start, s.span.end, s.faithful, s.target
            );
            all.push((rel.clone(), s));
        }
    }

    // Score the expectations: `file | kind | text | line | target substring`.
    let expected_path = root.join("expected.txt");
    let expected = std::fs::read_to_string(&expected_path).expect("expected.txt");
    let (mut ok, mut total) = (0, 0);
    println!("\n== expectations ({})", expected_path.display());
    for line in expected.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let p: Vec<&str> = line.split('|').map(str::trim).collect();
        let [file, kind, text, at, want] = p[..] else { panic!("bad line: {line}") };
        let at: usize = at.parse().unwrap();
        total += 1;
        let hit = all
            .iter()
            .find(|(f, s)| f == file && s.kind == kind && s.text == text && line_col_of(&root, f, s.span.start) == at);
        let pass = hit.is_some_and(|(_, s)| s.faithful && s.target.contains(want));
        ok += usize::from(pass);
        let got = hit.map_or("not seen".to_string(), |(_, s)| s.target.clone());
        println!("   {} {file}:{at} {kind} {text}: want '{want}', got '{got}'", if pass { "PASS" } else { "FAIL" });
    }
    println!("\n== score: {ok}/{total}");
}

fn line_col_of(root: &Path, rel: &str, at: u32) -> usize {
    let source = std::fs::read_to_string(root.join(rel)).unwrap();
    line_col(&source, at).0
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().is_some_and(|x| x == "vue") {
            out.push(p);
        }
    }
}
