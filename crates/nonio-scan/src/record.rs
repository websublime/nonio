//! The import/export record and table writes of one file. Ported from `facts/record.ts` of illuminate (MIT,
//! Copyright (c) 2026 johnz4021); the record is built from oxc's module record the way `oxc-parser`'s JavaScript
//! binding presents it (`staticImports`, `staticExports`, `dynamicImports`).

use std::collections::HashMap;

use indexmap::IndexMap;
use oxc_allocator::Allocator;
use oxc_parser::{Parser, ParserReturn};
use oxc_span::SourceType;
use oxc_syntax::module_record::{
    ExportExportName, ExportImportName, ExportLocalName, ImportImportName, ModuleRecord as OxcRecord,
};

use crate::adapters::{find_writes, mask_comments, Hit};
use crate::fallback::{regex_imports, requires, strip_comments, SpecSite};

/// Above this size (or when minified) a file gets its import/export record only: no symbols, calls or routes.
pub const BULK_MIN: usize = 500_000;

/// What an export name points at: a local binding, or `name` (`*` = namespace) of the module `spec`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportTarget {
    Local(String),
    From { spec: String, name: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Oxc,
    Regex,
}

#[derive(Debug, Clone)]
pub struct ModuleRecord {
    /// Imports, re-exports, dynamic imports and `require()`, in source order.
    pub specs: Vec<SpecSite>,
    pub exports: IndexMap<String, ExportTarget>,
    /// `export * from` specs.
    pub stars: Vec<String>,
    pub writes: Vec<Hit>,
    pub mode: Mode,
}

fn has_code_ext(path: &str) -> bool {
    [".js", ".mjs", ".cjs", ".jsx", ".ts", ".mts", ".cts", ".tsx"].iter().any(|e| path.ends_with(e))
}

/// The file name oxc picks the language from (`.ts` when the path has no code extension).
pub fn parse_name(path: &str) -> String {
    if has_code_ext(path) {
        path.to_string()
    } else {
        format!("{}.ts", if path.is_empty() { "source" } else { path })
    }
}

/// Long average line length means generated or minified code.
pub fn is_minified(source: &str) -> bool {
    if source.len() < 20_000 {
        return false;
    }
    let mut sample = source.len().min(100_000);
    while !source.is_char_boundary(sample) {
        sample -= 1;
    }
    let lines = 1 + source[..sample].bytes().filter(|&b| b == b'\n').count();
    sample / lines > 400
}

/// `'x'`, `"x"` or `` `x` `` (no substitutions) as its value.
fn literal(text: &str) -> Option<&str> {
    let b = text.as_bytes();
    if b.len() < 2 {
        return None;
    }
    let q = b[0];
    if !matches!(q, b'\'' | b'"' | b'`') || b[b.len() - 1] != q {
        return None;
    }
    // Both ends are ASCII quotes, so slicing them off stays on char boundaries.
    let inner = &text[1..text.len() - 1];
    let ok = if q == b'`' { !inner.contains(['`', '$', '\n']) } else { !inner.contains(['\'', '"', '\n']) };
    ok.then_some(inner)
}

/// Parse `source` with oxc as the original does (`preserveParens`, language from the file name).
pub fn parse<'a>(allocator: &'a Allocator, path: &str, source: &'a str) -> ParserReturn<'a> {
    let source_type = SourceType::from_path(parse_name(path)).unwrap_or_default();
    Parser::new(allocator, source, source_type).parse()
}

/// Whether the parse is clean: the original treats any diagnostic as a syntax error.
pub fn clean(ret: &ParserReturn<'_>) -> bool {
    !ret.panicked() && !ret.diagnostics.has_errors() && !ret.diagnostics.has_warnings()
}

trait Panicked {
    fn panicked(&self) -> bool;
}

impl Panicked for ParserReturn<'_> {
    fn panicked(&self) -> bool {
        self.fatal_error
    }
}

/// The comment spans of a parse.
pub fn comment_spans(ret: &ParserReturn<'_>) -> Vec<(usize, usize)> {
    ret.program.comments.iter().map(|c| (c.span.start as usize, c.span.end as usize)).collect()
}

/// The record of a clean parse.
pub fn from_oxc(source: &str, ret: &ParserReturn<'_>) -> ModuleRecord {
    let m: &OxcRecord<'_> = &ret.module_record;
    let mut specs = Vec::new();
    let mut exports: IndexMap<String, ExportTarget> = IndexMap::new();
    let mut stars = Vec::new();

    struct Import<'r, 'a> {
        start: u32,
        end: u32,
        spec: String,
        entries: Vec<&'r oxc_syntax::module_record::ImportEntry<'a>>,
    }
    let mut imports: Vec<Import<'_, '_>> = Vec::new();
    for (name, requested) in &m.requested_modules {
        for r in requested.iter().filter(|r| r.is_import) {
            let entries = m.import_entries.iter().filter(|e| e.statement_span == r.statement_span).collect();
            imports.push(Import {
                start: r.statement_span.start,
                end: r.statement_span.end,
                spec: name.to_string(),
                entries,
            });
        }
    }
    imports.sort_by_key(|i| i.start);
    for i in &imports {
        let type_only = !i.entries.is_empty() && i.entries.iter().all(|e| e.is_type);
        specs.push(SpecSite { spec: i.spec.clone(), type_only, at: i.start as usize });
    }

    let mut groups: Vec<(oxc_span::Span, Vec<&oxc_syntax::module_record::ExportEntry<'_>>)> = Vec::new();
    let mut at: HashMap<oxc_span::Span, usize> = HashMap::new();
    for e in m.local_export_entries.iter().chain(m.indirect_export_entries.iter()).chain(m.star_export_entries.iter()) {
        let i = *at.entry(e.statement_span).or_insert_with(|| {
            groups.push((e.statement_span, Vec::new()));
            groups.len() - 1
        });
        groups[i].1.push(e);
    }
    groups.sort_by_key(|g| g.0.start);
    for (span, entries) in &groups {
        if let Some(req) = entries.iter().find_map(|e| e.module_request.as_ref()) {
            specs.push(SpecSite {
                spec: req.name.to_string(),
                type_only: entries.iter().all(|e| e.is_type),
                at: span.start as usize,
            });
        }
        for e in entries {
            let key = match &e.export_name {
                ExportExportName::Default(_) => Some("default".to_string()),
                ExportExportName::Name(n) => Some(n.name.to_string()).filter(|k| !k.is_empty()),
                ExportExportName::Null => None,
            };
            if let Some(req) = &e.module_request {
                match &e.import_name {
                    ExportImportName::AllButDefault => stars.push(req.name.to_string()),
                    name => {
                        if let Some(key) = key {
                            let name = match name {
                                ExportImportName::All => "*".to_string(),
                                ExportImportName::Name(n) => n.name.to_string(),
                                _ => "default".to_string(),
                            };
                            exports.insert(key, ExportTarget::From { spec: req.name.to_string(), name });
                        }
                    }
                }
            } else if let Some(key) = key {
                // `import { a } from './x'; export { a }`: oxc points importName at the import specifier.
                let from_import = match &e.import_name {
                    ExportImportName::Name(n) => {
                        let at = n.span.start;
                        imports.iter().find(|s| s.start <= at && at < s.end).and_then(|s| {
                            s.entries
                                .iter()
                                .find(|x| {
                                    import_name_start(&x.import_name) == Some(at) || x.local_name.span.start == at
                                })
                                .map(|ie| (s, *ie))
                        })
                    }
                    _ => None,
                };
                if let Some((s, ie)) = from_import {
                    let name = match &ie.import_name {
                        ImportImportName::Default(_) => "default".to_string(),
                        ImportImportName::NamespaceObject => "*".to_string(),
                        ImportImportName::Name(n) => n.name.to_string(),
                    };
                    exports.insert(key, ExportTarget::From { spec: s.spec.clone(), name });
                } else {
                    let local = match &e.local_name {
                        ExportLocalName::Name(n) | ExportLocalName::Default(n) => n.name.to_string(),
                        ExportLocalName::Null => "default".to_string(),
                    };
                    exports.insert(key, ExportTarget::Local(local));
                }
            }
        }
    }
    for di in &m.dynamic_imports {
        let text = crate::adapters::slice(source, di.module_request.start as usize, di.module_request.end as usize);
        if let Some(spec) = literal(text) {
            specs.push(SpecSite { spec: spec.to_string(), type_only: false, at: di.span.start as usize });
        }
    }
    let code = mask_comments(source, &comment_spans(ret));
    requires(&code, &mut specs);
    specs.sort_by_key(|s| s.at);
    ModuleRecord { specs, exports, stars, writes: find_writes(&code), mode: Mode::Oxc }
}

fn import_name_start(n: &ImportImportName<'_>) -> Option<u32> {
    match n {
        ImportImportName::Name(n) => Some(n.span.start),
        ImportImportName::Default(s) => Some(s.start),
        ImportImportName::NamespaceObject => None,
    }
}

/// The record of source that doesn't parse.
pub fn from_regex(source: &str) -> ModuleRecord {
    let code = strip_comments(source);
    ModuleRecord {
        specs: regex_imports(&code),
        exports: IndexMap::new(),
        stars: Vec::new(),
        writes: find_writes(&code),
        mode: Mode::Regex,
    }
}

/// The record of `source`, parsed on its own.
pub fn module_record(path: &str, source: &str) -> ModuleRecord {
    let allocator = Allocator::default();
    let ret = parse(&allocator, path, source);
    if clean(&ret) {
        from_oxc(source, &ret)
    } else {
        from_regex(source)
    }
}
