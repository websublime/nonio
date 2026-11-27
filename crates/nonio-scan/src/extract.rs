//! Facts for one TS/JS file (or Vue SFC, see [`crate::vue`]): one oxc parse, one walk. Ported from `extract` in `facts/extract.ts` of illuminate (MIT,
//! Copyright (c) 2026 johnz4021).

use std::rc::Rc;
use std::sync::LazyLock;

use fancy_regex::Regex;
use indexmap::IndexMap;
use nonio_facts::{matches_any, EntryFact, EntryKind, FileFacts, ImportFact, RouteFact, WriteSite};
use oxc_allocator::Allocator;
use oxc_semantic::SemanticBuilder;

use crate::adapters::{mask_comments, Hit};
use crate::entries::PackageEntry;
use crate::hash::hash_text;
use crate::re::{at, each, end, js, test, try_js};
use crate::record::{
    clean, comment_spans, from_oxc, from_regex, is_minified, parse, ExportTarget, Mode, ModuleRecord, BULK_MIN,
};
use crate::routes::{next_routes, route_fact};
use crate::tables::{find_reads, sql_facts};
use crate::vue::parse_sfc;
use crate::walk::{Context, Walker};

/// A route pattern confirmed for a repo: `file` is a glob; `pattern` a JavaScript regex source matched against that
/// file's text (comments masked), with named groups `method` and `path` (or groups 1 and 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutePattern {
    pub file: String,
    pub pattern: String,
}

/// What extraction may ask about the repo around the file.
pub struct ExtractContext<'c> {
    pub resolve: &'c dyn Fn(&str, &str) -> Option<String>,
    pub record_of: &'c dyn Fn(&str) -> Option<Rc<ModuleRecord>>,
    /// package.json entries that run this file. A `script` whose file reads its arguments becomes a `cli` entry.
    pub entries: &'c [PackageEntry],
    pub routes: &'c [RoutePattern],
}

/// Reads CLI arguments (`process.argv.slice(2)`, `argv[2]`, `parseArgs(…)`), not just the `argv[1]` is-main check.
static READS_ARGS: LazyLock<Regex> = LazyLock::new(|| {
    js(r"\bprocess\s*\.\s*argv\s*(?:\.\s*(?:slice|includes|indexOf|filter|find)\b|\[\s*[2-9])|\bparseArgs\s*\(")
});

/// Facts for `path` (calls unsettled; see [`nonio_facts::settle_calls`]).
pub fn extract_facts(path: &str, source: &str, ctx: &ExtractContext<'_>) -> FileFacts {
    extract(path, source, ctx).0
}

/// Facts for `path`, its record, and whether it parsed (when it didn't, symbols and calls are empty, not "all removed"). Large
/// (> ~500 KB) or minified files get imports, writes, reads and file routes only; `.sql` files get the tables they
/// create. Calls come back unsettled.
pub fn extract(path: &str, source: &str, ctx: &ExtractContext<'_>) -> (FileFacts, ModuleRecord, bool) {
    if path.ends_with(".sql") {
        let record = from_regex("");
        return (sql_facts(path, source), record, true);
    }
    let allocator = Allocator::default();
    // A Vue SFC is read as its script blocks in place plus its template lowered into the same program (D28); `text`
    // is what the script was parsed from. Markup that doesn't parse goes the way of any file that doesn't.
    let sfc = path.ends_with(".vue").then(|| parse_sfc(&allocator, source)).flatten();
    let (ret, text, template) = match sfc {
        Some(s) => (s.ret, s.script, s.template),
        None => (parse(&allocator, path, source), source, None),
    };
    let parsed = clean(&ret);
    let record = if parsed { from_oxc(text, &ret) } else { from_regex(text) };
    let mut facts = FileFacts { hash: hash_text(source), ..FileFacts::empty(path) };

    let mut imports: IndexMap<String, bool> = IndexMap::new();
    for s in &record.specs {
        if let Some(to) = (ctx.resolve)(path, &s.spec).filter(|t| t != path) {
            let prev = imports.get(&to).copied().unwrap_or(true);
            imports.insert(to, prev && s.type_only);
        }
    }
    facts.imports = imports.into_iter().map(|(to, type_only)| ImportFact { to, type_only }).collect();

    let code = if parsed { mask_comments(text, &comment_spans(&ret)) } else { text.to_string() };
    let walk_ctx = Context { resolve: ctx.resolve, record_of: ctx.record_of };
    // The original counts the size in UTF-16 units.
    let walk = parsed && source.encode_utf16().count() <= BULK_MIN && !is_minified(source);
    let scoping = walk.then(|| SemanticBuilder::new().build(&ret.program).semantic.into_scoping());
    let mut w = Walker::new(path, source, &walk_ctx, &record, scoping.as_ref()).with_template(template);
    if walk {
        w.run(&ret.program);
    }
    w.finish();

    let mut routes: IndexMap<String, RouteFact> = IndexMap::new();
    for r in w.routes() {
        routes.entry(r.id.clone()).or_insert(r);
    }
    let export_names: Vec<&str> = record.exports.keys().map(String::as_str).collect();
    let (next, page) = next_routes(path, &export_names);
    let exported_symbol = |name: &str| {
        let local = match record.exports.get(name) {
            Some(ExportTarget::Local(l)) => l.as_str(),
            _ => name,
        };
        w.top_symbol(local)
    };
    for r in next {
        let f = route_fact(&r.method, &r.path, path, exported_symbol(&r.symbol));
        routes.entry(f.id.clone()).or_insert(f);
    }
    for r in custom_routes(path, &code, ctx.routes, &w) {
        routes.entry(r.id.clone()).or_insert(r);
    }
    facts.routes = routes.into_values().collect();

    let sites = |hits: &[Hit]| {
        let mut out: IndexMap<(String, Option<String>, &str), WriteSite> = IndexMap::new();
        for h in hits {
            let site = WriteSite { table: h.table.clone(), symbol: w.innermost(h.at), via: h.via.to_string() };
            out.insert((site.table.clone(), site.symbol.clone(), h.via), site);
        }
        out.into_values().collect::<Vec<_>>()
    };
    facts.writes = sites(&record.writes);
    facts.reads = sites(&find_reads(&code));

    let mut entries: IndexMap<String, EntryFact> = IndexMap::new();
    let mut entry = |kind: EntryKind, label: &str, symbol: Option<String>| {
        let id = format!("entry:{}:{label}", entry_kind(kind));
        entries.insert(id.clone(), EntryFact { id, kind, file: path.to_string(), symbol, label: label.to_string() });
    };
    for e in ctx.entries {
        let kind = if e.kind == EntryKind::Script && test(&READS_ARGS, &code) { EntryKind::Cli } else { e.kind };
        entry(kind, &e.label, None);
    }
    for r in &facts.routes {
        entry(EntryKind::Route, &format!("{} {}", r.method, r.path), r.handler.clone());
    }
    if let Some(page) = &page {
        entry(EntryKind::Page, page, exported_symbol("default"));
    }
    facts.entries = entries.into_values().collect();

    facts.symbols = std::mem::take(&mut w.symbols);
    facts.calls = std::mem::take(&mut w.calls).into_values().collect();
    let ok = record.mode != Mode::Regex;
    (facts, record, ok)
}

fn entry_kind(k: EntryKind) -> &'static str {
    match k {
        EntryKind::Bin => "bin",
        EntryKind::Script => "script",
        EntryKind::Route => "route",
        EntryKind::Page => "page",
        EntryKind::Cli => "cli",
        EntryKind::Main => "main",
    }
}

/// Routes from confirmed patterns. The handler is the first symbol defined after the match.
fn custom_routes(path: &str, code: &str, patterns: &[RoutePattern], w: &Walker<'_, '_>) -> Vec<RouteFact> {
    let mut out = Vec::new();
    for p in patterns {
        if !matches_any(&[p.file.as_str()], path) {
            continue;
        }
        let Some(re) = try_js(&p.pattern, false) else { continue };
        let matches: Vec<_> = each(&re, code).collect();
        for (i, m) in matches.iter().enumerate() {
            // Named groups `method` and `path` when the pattern names `path`; otherwise groups 1 and 2 (a lone group is
            // the path).
            let named = m.name("path").is_some();
            let groups = m.len();
            let method = if named {
                m.name("method").map(|x| x.as_str())
            } else if groups > 2 {
                m.get(1).map(|x| x.as_str())
            } else {
                None
            };
            let raw = if named {
                m.name("path")
            } else if groups > 2 {
                m.get(2)
            } else {
                m.get(1)
            };
            let Some(raw) = raw else { continue };
            let stop = matches.get(i + 1).map_or(code.len(), at);
            let handler = w.first_symbol_in(end(m), stop).or_else(|| w.innermost(at(m)));
            out.push(route_fact(method.unwrap_or("ANY"), raw.as_str(), path, handler));
        }
    }
    out
}
