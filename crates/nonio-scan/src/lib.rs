//! Extraction backends of nonio: facts from source code, behind the [`Extractor`] trait of `nonio-facts` (D12).
//!
//! [`OxcExtractor`] reads TS, JS, JSX and TSX with oxc (D13), and `.sql` schema files, the way illuminate's extractor
//! does (MIT, Copyright (c) 2026 johnz4021): the same symbol ids, hashes, calls, routes, entries, table reads and
//! writes. Calls are resolved by scope and import only (`syntactic`); a call resolved by name alone is never made
//! (D16).
//!
//! Vue single-file components are read with the same backend (D28): the `<script>` blocks in place and the template
//! lowered to JSX in the same program (see [`parse_sfc`]); template calls belong to the symbol `<path>#<template>`.
//!
//! Java, Kotlin and Python are read with `code2graph`'s precise tier behind [`GraphExtractor`] (D14), whose edges
//! are checked again so none rests on a name match (D16), and whose gaps on typed receivers (fields, constructor
//! properties, typed locals) are filled from the declared types (D15). Spring MVC routes come from the annotations of
//! Java and Kotlin controllers, literal paths only ([`spring_routes`]). [`Scanner`] sends each file to its backend.
//! Go gets no facts yet.

mod adapters;
mod entries;
mod extract;
mod fallback;
mod graph;
mod hash;
mod re;
mod record;
mod resolve;
mod routes;
mod spring;
mod tables;
mod vue;
mod walk;

use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use nonio_facts::{Extractor, FileFacts, RepoView};

pub use entries::{package_entries, PackageEntry};
pub use extract::{extract_facts, ExtractContext, RoutePattern};
pub use fallback::strip_comments;
pub use graph::{graph_facts, is_graph_file, GraphExtractor, GRAPH_EXTENSIONS};
pub use hash::{hash_text, squash};
pub use record::{module_record, ExportTarget, ModuleRecord};
pub use resolve::{normalize_path, package_name, resolve_path, resolve_specifier, workspace_dirs, RepoResolver};
pub use routes::normalize_route_path;
pub use spring::spring_routes;
pub use tables::{find_reads, parse_sql_tables};
pub use vue::{parse_sfc, Sfc};

use record::ModuleRecord as Record;

/// The extensions the oxc backend reads as code (and whose records calls follow through).
pub const CODE_EXTENSIONS: [&str; 7] = [".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".mts"];

pub fn is_code_file(path: &str) -> bool {
    CODE_EXTENSIONS.iter().any(|e| path.ends_with(e))
}

/// Files another backend reads: Java, Kotlin and Python ([`GraphExtractor`]), and Go.
fn other_backend(path: &str) -> bool {
    is_graph_file(path) || path.ends_with(".go")
}

/// Facts for TS/JS files (and `.sql` schemas) of one repo, read with oxc. Java, Kotlin, Python and Go files get
/// `None` (another backend reads them); any other file is read as TS/JS and, when it doesn't parse, through the
/// regex fallback, as in the original.
///
/// Module resolution reads the repo on disk at `root` (tsconfig, package.json, workspaces), as the original does for
/// both the working tree and an earlier commit; files the [`RepoView`] has but the disk doesn't resolve through
/// relative paths and tsconfig `paths`.
pub struct OxcExtractor {
    root: PathBuf,
    routes: Vec<RoutePattern>,
    resolver: OnceCell<RepoResolver>,
    entries: OnceCell<HashMap<String, Vec<PackageEntry>>>,
    /// The last two records per path, so following imports through a file parses it once.
    records: RefCell<HashMap<String, Vec<Cached>>>,
}

/// A file's text and the record built from it.
type Cached = (Rc<str>, Rc<Record>);

impl OxcExtractor {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
            routes: Vec::new(),
            resolver: OnceCell::new(),
            entries: OnceCell::new(),
            records: RefCell::default(),
        }
    }

    /// With route patterns confirmed for this repo.
    pub fn with_routes(mut self, routes: Vec<RoutePattern>) -> Self {
        self.routes = routes;
        self
    }

    fn resolver(&self) -> &RepoResolver {
        self.resolver.get_or_init(|| RepoResolver::new(&self.root))
    }

    fn record(&self, path: &str, text: &Rc<str>) -> Rc<Record> {
        if let Some(hit) = self.records.borrow().get(path).and_then(|l| l.iter().find(|(t, _)| t == text)) {
            return hit.1.clone();
        }
        let rec = Rc::new(module_record(path, text));
        self.remember(path, text.clone(), rec.clone());
        rec
    }

    fn remember(&self, path: &str, text: Rc<str>, rec: Rc<Record>) {
        let mut records = self.records.borrow_mut();
        let list = records.entry(path.to_string()).or_default();
        list.retain(|(t, _)| *t != text);
        list.insert(0, (text, rec));
        list.truncate(2);
    }
}

impl Extractor for OxcExtractor {
    fn extract(&self, repo: &dyn RepoView, path: &str, source: &str) -> Option<FileFacts> {
        if other_backend(path) {
            return None;
        }
        let resolver = self.resolver();
        let exists = |p: &str| repo.exists(p);
        let resolve = |from: &str, spec: &str| resolve_specifier(from, spec, &exists, Some(resolver));
        let record_of = |file: &str| {
            if !is_code_file(file) && !file.ends_with(".vue") {
                return None;
            }
            let text = repo.text(file)?;
            Some(self.record(file, &text))
        };
        let entries = self.entries.get_or_init(|| {
            let root = resolver.root().to_path_buf();
            package_entries(&root, &|p: &str| root.join(p).is_file())
        });
        let ctx = ExtractContext {
            resolve: &resolve,
            record_of: &record_of,
            entries: entries.get(path).map_or(&[], Vec::as_slice),
            routes: &self.routes,
        };
        let (facts, record, _) = extract::extract(path, source, &ctx);
        if !path.ends_with(".sql") {
            self.remember(path, Rc::from(source), Rc::new(record));
        }
        Some(facts)
    }
}

/// Every backend of one repo: Java, Kotlin and Python files go to [`GraphExtractor`], the rest to [`OxcExtractor`].
pub struct Scanner {
    oxc: OxcExtractor,
    graph: GraphExtractor,
}

impl Scanner {
    /// `files` are the repo's files (e.g. from `list_repo_files`); the graph backend resolves calls across them.
    pub fn new<I, S>(root: impl AsRef<Path>, files: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self { oxc: OxcExtractor::new(root), graph: GraphExtractor::new(files) }
    }

    /// With route patterns confirmed for this repo (TS/JS only for now).
    pub fn with_routes(mut self, routes: Vec<RoutePattern>) -> Self {
        self.oxc = self.oxc.with_routes(routes);
        self
    }
}

impl Extractor for Scanner {
    fn extract(&self, repo: &dyn RepoView, path: &str, source: &str) -> Option<FileFacts> {
        if is_graph_file(path) {
            self.graph.extract(repo, path, source)
        } else {
            self.oxc.extract(repo, path, source)
        }
    }
}
