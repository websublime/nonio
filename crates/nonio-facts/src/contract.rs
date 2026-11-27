//! Per-file facts shared by the extractors and everything downstream (the checker, explorer hints).
//!
//! Ported from `packages/facts/src/contracts/facts.ts` of illuminate (MIT, Copyright (c) 2026 johnz4021).
//! Ids never contain line numbers, so they stay stable across edits that shift code.
//!
//! One deliberate difference from the original: [`Span`] offsets are UTF-8 byte offsets into the file text
//! (what Rust strings and oxc use), not UTF-16 offsets.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SymbolKind {
    Function,
    Class,
    Method,
    Const,
    Type,
    Property,
    Component,
}

/// Byte offsets into the current text; never part of an id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

/// A definition in a file. `id` = `<path>#<qualified name>` (e.g. `src/tools/news.ts#news.run`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolFact {
    pub id: String,
    pub file: String,
    /// Qualified within the file: `news.run`; duplicates get `~2`, anonymous callbacks `<cb1>`.
    pub name: String,
    pub kind: SymbolKind,
    pub exported: bool,
    /// Enclosing symbol id, or `None` at top level.
    pub parent: Option<String>,
    pub span: Span,
    /// Normalized params, types, modifiers, export flag.
    pub sig_hash: String,
    /// Whitespace-normalized body text.
    pub body_hash: String,
}

/// How a call was resolved. Only resolution by scope, import or declared type may back a `fact` proof; a call
/// resolved by name alone never does (D16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CallConfidence {
    Syntactic,
    Typed,
    Dispatch,
}

/// A call (or `new`) from code in one file. `from` is a symbol id, or the file path for top-level code.
/// `to` is a symbol id, a file path (unresolved member), `vendor:<x>` or `table:<x>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallFact {
    pub from: String,
    pub to: String,
    pub confidence: CallConfidence,
    pub count: u32,
}

/// `id` = `route:<METHOD> <path>` with params normalized to `{}` (e.g. `route:POST /api/runs/{}`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteFact {
    pub id: String,
    pub method: String,
    pub path: String,
    pub file: String,
    pub handler: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    Bin,
    Script,
    Route,
    Page,
    Cli,
    Main,
}

/// `id` = `entry:<kind>:<label>` (e.g. `entry:bin:scout`, `entry:page:/runs/{}`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryFact {
    pub id: String,
    pub kind: EntryKind,
    pub file: String,
    pub symbol: Option<String>,
    pub label: String,
}

/// A table write or read found by an adapter (`db.insert`, Prisma, raw SQL, …).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteSite {
    pub table: String,
    pub symbol: Option<String>,
    pub via: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportFact {
    /// A resolved repo path, or `vendor:<pkg>` for a bare import.
    pub to: String,
    pub type_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableDef {
    pub name: String,
    pub columns: Vec<String>,
}

/// Everything an extractor knows about one file. Replaced wholesale when the file changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileFacts {
    pub path: String,
    /// Content hash of the source these facts came from.
    pub hash: String,
    pub imports: Vec<ImportFact>,
    pub writes: Vec<WriteSite>,
    #[serde(default)]
    pub reads: Vec<WriteSite>,
    #[serde(default)]
    pub tables: Vec<TableDef>,
    pub symbols: Vec<SymbolFact>,
    pub calls: Vec<CallFact>,
    pub routes: Vec<RouteFact>,
    pub entries: Vec<EntryFact>,
    /// HTTP clients this file creates with a known library (`axios.create`, `ky.create`, …).
    #[serde(default)]
    pub clients: Vec<ClientFact>,
    /// Requests sent through an HTTP client library or one of its instances, found by resolving the callee.
    #[serde(default)]
    pub requests: Vec<RequestFact>,
}

/// An HTTP client made from a library's factory and bound to a module-scope name: `export const api =
/// axios.create({ baseURL: '/api' })` is `{ id: "<file>#api", lib: "axios", prefix: Some("/api") }`. `prefix` is
/// `None` when the base URL isn't literal text (an env variable, a call): requests through it then prove nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientFact {
    pub id: String,
    pub lib: String,
    pub prefix: Option<String>,
}

/// A request sent with a URL literal through a client library (`axios.post('/x')`) or through an instance of one
/// (`api.get('/x')`). The callee is resolved by scope and import, never by name (D16).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestFact {
    /// Enclosing symbol id, or the file path for top-level code.
    pub from: String,
    /// Upper case.
    pub method: String,
    /// The URL written at the call: path only, query dropped, `${…}` → `{}`.
    pub url: String,
    /// The URL at the call has no template holes.
    pub exact: bool,
    /// `vendor:<lib>` for a call on the library itself, or the id of the [`ClientFact`] it goes through.
    pub via: String,
}

/// What an extractor may ask about the version of the repo a file is read from (the working tree, or a commit).
pub trait RepoView {
    /// Whether a repo-relative file exists in this version of the repo (for resolving imports).
    fn exists(&self, path: &str) -> bool;
    /// The text of a repo-relative file in this version of the repo, when it can be read.
    fn text(&self, path: &str) -> Option<std::rc::Rc<str>>;
}

/// Produces the facts of one file from its text. Calls come back unsettled (see [`crate::settle_calls`]).
///
/// This is the only thing the checker knows about extraction (D12): the oxc, `code2graph` and tree-sitter
/// backends each implement it, and tests stand in recorded facts.
pub trait Extractor {
    /// Facts for `path` with content `source`, or `None` when this extractor has nothing for that file.
    fn extract(&self, repo: &dyn RepoView, path: &str, source: &str) -> Option<FileFacts>;
}

impl<T: Extractor + ?Sized> Extractor for &T {
    fn extract(&self, repo: &dyn RepoView, path: &str, source: &str) -> Option<FileFacts> {
        (**self).extract(repo, path, source)
    }
}

/// An extractor that knows no files: every claim falls back to its cited text.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoFacts;

impl Extractor for NoFacts {
    fn extract(&self, _repo: &dyn RepoView, _path: &str, _source: &str) -> Option<FileFacts> {
        None
    }
}

impl FileFacts {
    /// Facts of a file an extractor has nothing for: it exists, and nothing is known about it.
    pub fn empty(path: &str) -> Self {
        Self {
            path: path.to_string(),
            hash: String::new(),
            imports: Vec::new(),
            writes: Vec::new(),
            reads: Vec::new(),
            tables: Vec::new(),
            symbols: Vec::new(),
            calls: Vec::new(),
            routes: Vec::new(),
            entries: Vec::new(),
            clients: Vec::new(),
            requests: Vec::new(),
        }
    }
}
