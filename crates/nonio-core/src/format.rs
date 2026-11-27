//! The map format: what the host's model writes, the checker verifies and the viewer draws.
//!
//! Ported from `packages/core/src/format.ts` of illuminate (MIT, Copyright (c) 2026 johnz4021). A single-root map is
//! the same JSON as the original's `CodeMap` version 1 (D22).
//!
//! This is the one-way door. Statuses are never part of it: the checker computes them from the repo every time. That
//! goes for proposals too, and for a map of a change (`since`): the map only says which boxes and arrows are code that
//! has gone (`removed`), since those cite the code as it was.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The longest question a map holds. Checked when a map is asked for, so a long one is refused up front.
pub const QUESTION_MAX: usize = 300;

/// A line of code that backs a claim. `text` must appear on that line (whitespace-insensitive) or within a few lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    pub file: String,
    pub line: usize,
    pub text: String,
}

macro_rules! string_enum {
    ($(#[$m:meta])* $name:ident { $($variant:ident = $s:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum $name { $(#[serde(rename = $s)] $variant),+ }
        impl $name {
            pub const ALL: &'static [&'static str] = &[$($s),+];
            pub fn as_str(self) -> &'static str { match self { $(Self::$variant => $s),+ } }
            fn parse(s: &str) -> Option<Self> { match s { $($s => Some(Self::$variant),)+ _ => None } }
        }
    };
}

string_enum!(
    /// How a box is drawn. On a journey: a screen, something the person does, a point where they choose.
    NodeKind { Entry = "entry", Step = "step", Store = "store", External = "external", Job = "job", Screen = "screen", Action = "action", Choice = "choice" }
);
string_enum!(
    /// What an arrow claims. `calls`, `request` (fetch → route) and `imports` are also proven from code facts; `then`
    /// is proven by its evidence text alone.
    EdgeKind { Calls = "calls", Request = "request", Imports = "imports", Then = "then" }
);
string_enum!(ElementKind { Heading = "heading", Button = "button", Field = "field", Link = "link", Text = "text", Image = "image" });
string_enum!(
    /// What the person sees at a box: a capture nonio took, an image in the repo, a sketch of a markup file, or a
    /// wireframe of one of the map's pages.
    ScreenKind { Capture = "capture", Image = "image", Sketch = "sketch", Wireframe = "wireframe" }
);
string_enum!(
    /// Where a region sits on its page. Five fixed places, so a page can be drawn without the map describing a layout.
    Slot { Top = "top", Left = "left", Main = "main", Right = "right", Bottom = "bottom" }
);
string_enum!(ProposalKind { Add = "add", Change = "change", Remove = "remove" });
string_enum!(View { Journey = "journey", Timeline = "timeline" });

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenElement {
    pub kind: ElementKind,
    pub text: String,
    /// Not on the screen yet: something a proposal would put there. Only on a box that carries `proposed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposed: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Screen {
    pub kind: ScreenKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elements: Option<Vec<ScreenElement>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highlight: Option<String>,
}

/// One visible part of a page, from one file. Every element's text must be in that file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageRegion {
    pub id: String,
    pub slot: Slot,
    pub label: String,
    pub file: String,
    pub elements: Vec<ScreenElement>,
}

/// A page of the app, defined once on the map and drawn as a wireframe on every box that happens on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Page {
    pub id: String,
    pub name: String,
    pub file: String,
    pub regions: Vec<PageRegion>,
}

/// A box or arrow the map suggests adding, changing or removing. Where it stands is computed, never stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    pub kind: ProposalKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Area {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Explain {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concept: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub how: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapNode {
    pub id: String,
    pub area: String,
    pub kind: NodeKind,
    pub name: String,
    /// `<file>#<symbol>` as the facts name it, e.g. `src/patients/service.ts#markConverted`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explain: Option<Explain>,
    /// At least one line, except on a proposed addition.
    pub evidence: Vec<Evidence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screen: Option<Screen>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposed: Option<Proposal>,
    /// Only on a map with `since`: this code was there at that commit and is gone now.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub removed: Option<bool>,
    /// The way through this box is on; none for what the ways share.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapEdge {
    pub id: String,
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    pub label: String,
    pub evidence: Vec<Evidence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposed: Option<Proposal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub removed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Between {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Search {
    pub pattern: String,
    pub regex: bool,
    /// Globs to search; the whole repo when omitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paths: Option<Vec<String>>,
}

/// A claim that something is missing, proven by a search the checker runs again: `matches` is the count claimed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapAbsence {
    pub id: String,
    pub claim: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub between: Option<Between>,
    pub search: Search,
    pub matches: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Focus {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub areas: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nodes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edges: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub absences: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TourStop {
    pub id: String,
    pub title: String,
    pub text: String,
    pub focus: Focus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terms: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ask: Option<Vec<String>>,
}

/// Most roots a map may name in this version. Several roots come with the `<root>:<path>` paths, per-root `since`
/// and the rule that links requests across roots (Q12), none of which exist yet.
pub const MAX_ROOTS: usize = 1;

/// One code root of a map: a repo or checkout, and the commit it was read at, when known.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Root {
    pub id: String,
    /// `Some(None)` is an explicit `null`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<Option<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeMap {
    pub version: u8,
    pub question: String,
    /// The commit the map was made from, when known. `Some(None)` is an explicit `null`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<Option<String>>,
    /// The code roots the map is drawn from (D19). Absent on a map of one implicit root, which is then the same
    /// JSON as illuminate's. This version accepts exactly one root; paths are still plain repo-relative paths.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roots: Option<Vec<Root>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<View>,
    /// Set on a map of a change: the commit the code is compared with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pages: Option<Vec<Page>>,
    pub areas: Vec<Area>,
    pub nodes: Vec<MapNode>,
    pub edges: Vec<MapEdge>,
    pub absences: Vec<MapAbsence>,
    pub tour: Vec<TourStop>,
}

impl CodeMap {
    /// The map as JSON, the way it is stored and handed to [`parse_map`] again.
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("a map serializes")
    }
}

/// A map, or every problem that keeps it from being one.
pub type ParseResult = Result<CodeMap, Vec<String>>;

/// A box or arrow the map proposes adding: the only kind of claim that cites no code.
pub fn is_addition(proposed: Option<&Proposal>) -> bool {
    proposed.is_some_and(|p| p.kind == ProposalKind::Add)
}

impl MapNode {
    pub fn is_addition(&self) -> bool {
        is_addition(self.proposed.as_ref())
    }
    pub fn is_removed(&self) -> bool {
        self.removed == Some(true)
    }
}

impl MapEdge {
    pub fn is_addition(&self) -> bool {
        is_addition(self.proposed.as_ref())
    }
    pub fn is_removed(&self) -> bool {
        self.removed == Some(true)
    }
}

/// A repo-relative path that can't climb out of the repo: no absolute paths, no `..`, no backslashes.
pub fn is_safe_relative_path(path: &str) -> bool {
    let b = path.as_bytes();
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
    {
        return false;
    }
    path.split('/').all(|seg| seg != ".." && !seg.is_empty())
}

fn rx(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("valid regex"))
}

/// A commit, branch or tag name: it is handed to git, so it can't start with `-` or hold anything else.
pub fn is_git_ref(s: &str) -> bool {
    static R: OnceLock<Regex> = OnceLock::new();
    rx(&R, r"^[A-Za-z0-9_][A-Za-z0-9_./~^-]{0,99}$").is_match(s)
}

// ---------------------------------------------------------------------------------------------------------------
// Shape validation. Every problem is reported as `<path>: <message>`, the path joined with dots (`nodes.1.symbol`).

struct V {
    problems: Vec<String>,
}

/// Length as the original counts it (UTF-16 code units), so limits agree with maps made for illuminate.
fn len16(s: &str) -> usize {
    s.encode_utf16().count()
}

fn kind_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn join(path: &str, key: impl std::fmt::Display) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

#[derive(Clone, Copy)]
struct Text {
    min: usize,
    max: usize,
}

const ANY: Text = Text { min: 0, max: usize::MAX };
const NONEMPTY: Text = Text { min: 1, max: usize::MAX };
const fn upto(max: usize) -> Text {
    Text { min: 1, max }
}

impl V {
    fn push(&mut self, path: &str, message: impl Into<String>) {
        let p = if path.is_empty() { "(map)" } else { path };
        self.problems.push(format!("{p}: {}", message.into()));
    }

    fn missing(&mut self, path: &str, expected: &str) {
        self.push(path, format!("Invalid input: expected {expected}, received undefined"));
    }

    fn wrong(&mut self, path: &str, expected: &str, got: &Value) {
        self.push(path, format!("Invalid input: expected {expected}, received {}", kind_of(got)));
    }

    fn object<'a>(&mut self, v: &'a Value, path: &str) -> Option<&'a Map<String, Value>> {
        match v {
            Value::Object(o) => Some(o),
            other => {
                self.wrong(path, "object", other);
                None
            }
        }
    }

    fn string(&mut self, v: &Value, path: &str, t: Text) -> Option<String> {
        let Value::String(s) = v else {
            self.wrong(path, "string", v);
            return None;
        };
        let n = len16(s);
        if n < t.min {
            self.push(path, format!("Too small: expected string to have >={} characters", t.min));
            return None;
        }
        if n > t.max {
            self.push(path, format!("Too big: expected string to have <={} characters", t.max));
            return None;
        }
        Some(s.clone())
    }

    fn id(&mut self, v: &Value, path: &str) -> Option<String> {
        static ID: OnceLock<Regex> = OnceLock::new();
        let s = self.string(v, path, ANY)?;
        if rx(&ID, r"^[A-Za-z0-9_.:-]{1,64}$").is_match(&s) {
            Some(s)
        } else {
            self.push(path, "ids use letters, digits, _ . : - (at most 64)");
            None
        }
    }

    fn enumerated<T>(&mut self, v: &Value, path: &str, all: &[&str], parse: fn(&str) -> Option<T>) -> Option<T> {
        let found = v.as_str().and_then(parse);
        if found.is_none() {
            let options = all.iter().map(|s| format!("\"{s}\"")).collect::<Vec<_>>().join("|");
            self.push(path, format!("Invalid option: expected one of {options}"));
        }
        found
    }

    fn int(&mut self, v: &Value, path: &str, min: i64) -> Option<u64> {
        let Value::Number(n) = v else {
            self.wrong(path, "number", v);
            return None;
        };
        let Some(i) =
            n.as_i64().or_else(|| n.as_f64().filter(|f| f.fract() == 0.0 && f.abs() < 9e15).map(|f| f as i64))
        else {
            self.push(path, "Invalid input: expected int, received number");
            return None;
        };
        if i < min {
            self.push(path, format!("Too small: expected number to be >={min}"));
            return None;
        }
        u64::try_from(i).ok()
    }

    fn boolean(&mut self, v: &Value, path: &str) -> Option<bool> {
        match v {
            Value::Bool(b) => Some(*b),
            other => {
                self.wrong(path, "boolean", other);
                None
            }
        }
    }

    /// An array with a length range, each item read by `item`. `None` if the array or any item is wrong.
    fn array<T>(
        &mut self,
        v: &Value,
        path: &str,
        min: usize,
        max: usize,
        mut item: impl FnMut(&mut Self, &Value, &str) -> Option<T>,
    ) -> Option<Vec<T>> {
        let Value::Array(items) = v else {
            self.wrong(path, "array", v);
            return None;
        };
        let mut out = Vec::with_capacity(items.len());
        let mut ok = true;
        if items.len() < min {
            self.push(path, format!("Too small: expected array to have >={min} items"));
            ok = false;
        }
        if items.len() > max {
            self.push(path, format!("Too big: expected array to have <={max} items"));
            ok = false;
        }
        for (i, x) in items.iter().enumerate() {
            match item(self, x, &join(path, i)) {
                Some(t) => out.push(t),
                None => ok = false,
            }
        }
        ok.then_some(out)
    }
}

/// Reads one field: required, optional, or with a default.
macro_rules! field {
    ($v:ident, $o:ident, $path:ident, $key:literal, required $expected:literal, |$x:ident, $p:ident| $read:expr) => {{
        let $p = join($path, $key);
        match $o.get($key) {
            None => {
                $v.missing(&$p, $expected);
                None
            }
            Some($x) => $read,
        }
    }};
    ($v:ident, $o:ident, $path:ident, $key:literal, optional, |$x:ident, $p:ident| $read:expr) => {{
        let $p = join($path, $key);
        match $o.get($key) {
            None => Some(None),
            Some($x) => ($read).map(Some),
        }
    }};
    ($v:ident, $o:ident, $path:ident, $key:literal, default $def:expr, |$x:ident, $p:ident| $read:expr) => {{
        let $p = join($path, $key);
        match $o.get($key) {
            None => Some($def),
            Some($x) => $read,
        }
    }};
}

fn evidence(v: &mut V, x: &Value, path: &str) -> Option<Evidence> {
    let o = v.object(x, path)?;
    let file = field!(v, o, path, "file", required "string", |x, p| v.string(x, &p, NONEMPTY));
    let line = field!(v, o, path, "line", required "number", |x, p| v.int(x, &p, 1));
    let text = field!(v, o, path, "text", required "string", |x, p| v.string(x, &p, upto(400)));
    Some(Evidence { file: file?, line: usize::try_from(line?).ok()?, text: text? })
}

fn element(v: &mut V, x: &Value, path: &str) -> Option<ScreenElement> {
    let o = v.object(x, path)?;
    let kind =
        field!(v, o, path, "kind", required "string", |x, p| v.enumerated(x, &p, ElementKind::ALL, ElementKind::parse));
    let text = field!(v, o, path, "text", required "string", |x, p| v.string(x, &p, upto(80)));
    let proposed = field!(v, o, path, "proposed", optional, |x, p| v.boolean(x, &p));
    Some(ScreenElement { kind: kind?, text: text?, proposed: proposed? })
}

fn screen(v: &mut V, x: &Value, path: &str) -> Option<Screen> {
    let o = v.object(x, path)?;
    let kind =
        field!(v, o, path, "kind", required "string", |x, p| v.enumerated(x, &p, ScreenKind::ALL, ScreenKind::parse));
    let capture = field!(v, o, path, "capture", optional, |x, p| v.id(x, &p));
    let page = field!(v, o, path, "page", optional, |x, p| v.id(x, &p));
    let region = field!(v, o, path, "region", optional, |x, p| v.id(x, &p));
    let file = field!(v, o, path, "file", optional, |x, p| v.string(x, &p, NONEMPTY));
    let elements = field!(v, o, path, "elements", optional, |x, p| v.array(x, &p, 1, 12, element));
    let highlight = field!(v, o, path, "highlight", optional, |x, p| v.string(x, &p, upto(80)));
    Some(Screen {
        kind: kind?,
        capture: capture?,
        page: page?,
        region: region?,
        file: file?,
        elements: elements?,
        highlight: highlight?,
    })
}

fn region(v: &mut V, x: &Value, path: &str) -> Option<PageRegion> {
    let o = v.object(x, path)?;
    let id = field!(v, o, path, "id", required "string", |x, p| v.id(x, &p));
    let slot = field!(v, o, path, "slot", required "string", |x, p| v.enumerated(x, &p, Slot::ALL, Slot::parse));
    let label = field!(v, o, path, "label", required "string", |x, p| v.string(x, &p, upto(40)));
    let file = field!(v, o, path, "file", required "string", |x, p| v.string(x, &p, NONEMPTY));
    let elements = field!(v, o, path, "elements", default Vec::new(), |x, p| v.array(x, &p, 0, 6, element));
    Some(PageRegion { id: id?, slot: slot?, label: label?, file: file?, elements: elements? })
}

fn page(v: &mut V, x: &Value, path: &str) -> Option<Page> {
    let o = v.object(x, path)?;
    let id = field!(v, o, path, "id", required "string", |x, p| v.id(x, &p));
    let name = field!(v, o, path, "name", required "string", |x, p| v.string(x, &p, upto(60)));
    let file = field!(v, o, path, "file", required "string", |x, p| v.string(x, &p, NONEMPTY));
    let regions = field!(v, o, path, "regions", required "array", |x, p| v.array(x, &p, 1, 8, region));
    Some(Page { id: id?, name: name?, file: file?, regions: regions? })
}

fn proposal(v: &mut V, x: &Value, path: &str) -> Option<Proposal> {
    let o = v.object(x, path)?;
    let kind = field!(v, o, path, "kind", required "string", |x, p| v.enumerated(x, &p, ProposalKind::ALL, ProposalKind::parse));
    let note = field!(v, o, path, "note", optional, |x, p| v.string(x, &p, upto(300)));
    Some(Proposal { kind: kind?, note: note? })
}

fn branch(v: &mut V, x: &Value, path: &str) -> Option<String> {
    let s = v.string(x, path, upto(60))?;
    if s.chars().any(|c| !c.is_whitespace()) {
        Some(s)
    } else {
        v.push(path, "a branch is named in words");
        None
    }
}

fn nullable_string(v: &mut V, x: &Value, path: &str) -> Option<Option<String>> {
    if x.is_null() {
        Some(None)
    } else {
        v.string(x, path, ANY).map(Some)
    }
}

fn root(v: &mut V, x: &Value, path: &str) -> Option<Root> {
    let o = v.object(x, path)?;
    let id = field!(v, o, path, "id", required "string", |x, p| v.id(x, &p));
    let commit = field!(v, o, path, "commit", optional, |x, p| nullable_string(v, x, &p));
    Some(Root { id: id?, commit: commit? })
}

fn roots(v: &mut V, x: &Value, path: &str) -> Option<Vec<Root>> {
    let list = v.array(x, path, 1, usize::MAX, root)?;
    if list.len() > MAX_ROOTS {
        v.push(path, format!("this version reads maps of at most {MAX_ROOTS} root"));
        return None;
    }
    Some(list)
}

fn area(v: &mut V, x: &Value, path: &str) -> Option<Area> {
    let o = v.object(x, path)?;
    let id = field!(v, o, path, "id", required "string", |x, p| v.id(x, &p));
    let label = field!(v, o, path, "label", required "string", |x, p| v.string(x, &p, upto(40)));
    Some(Area { id: id?, label: label? })
}

fn explain(v: &mut V, x: &Value, path: &str) -> Option<Explain> {
    let o = v.object(x, path)?;
    let concept = field!(v, o, path, "concept", optional, |x, p| v.string(x, &p, ANY));
    let how = field!(v, o, path, "how", optional, |x, p| v.string(x, &p, ANY));
    let code = field!(v, o, path, "code", optional, |x, p| v.string(x, &p, ANY));
    Some(Explain { concept: concept?, how: how?, code: code? })
}

fn symbol(v: &mut V, x: &Value, path: &str) -> Option<String> {
    static SYM: OnceLock<Regex> = OnceLock::new();
    let s = v.string(x, path, ANY)?;
    if rx(&SYM, r"^[^#]+#.+$").is_match(&s) {
        Some(s)
    } else {
        v.push(path, "symbol is <file>#<name>");
        None
    }
}

fn node(v: &mut V, x: &Value, path: &str) -> Option<MapNode> {
    let o = v.object(x, path)?;
    let id = field!(v, o, path, "id", required "string", |x, p| v.id(x, &p));
    let area = field!(v, o, path, "area", required "string", |x, p| v.id(x, &p));
    let kind =
        field!(v, o, path, "kind", default NodeKind::Step, |x, p| v.enumerated(x, &p, NodeKind::ALL, NodeKind::parse));
    let name = field!(v, o, path, "name", required "string", |x, p| v.string(x, &p, upto(80)));
    let symbol = field!(v, o, path, "symbol", optional, |x, p| symbol(v, x, &p));
    let explain = field!(v, o, path, "explain", optional, |x, p| explain(v, x, &p));
    let ev = field!(v, o, path, "evidence", default Vec::new(), |x, p| v.array(x, &p, 0, usize::MAX, evidence));
    let screen = field!(v, o, path, "screen", optional, |x, p| screen(v, x, &p));
    let proposed = field!(v, o, path, "proposed", optional, |x, p| proposal(v, x, &p));
    let removed = field!(v, o, path, "removed", optional, |x, p| v.boolean(x, &p));
    let branch = field!(v, o, path, "branch", optional, |x, p| branch(v, x, &p));
    Some(MapNode {
        id: id?,
        area: area?,
        kind: kind?,
        name: name?,
        symbol: symbol?,
        explain: explain?,
        evidence: ev?,
        screen: screen?,
        proposed: proposed?,
        removed: removed?,
        branch: branch?,
    })
}

fn edge(v: &mut V, x: &Value, path: &str) -> Option<MapEdge> {
    let o = v.object(x, path)?;
    let id = field!(v, o, path, "id", required "string", |x, p| v.id(x, &p));
    let from = field!(v, o, path, "from", required "string", |x, p| v.id(x, &p));
    let to = field!(v, o, path, "to", required "string", |x, p| v.id(x, &p));
    let kind =
        field!(v, o, path, "kind", default EdgeKind::Then, |x, p| v.enumerated(x, &p, EdgeKind::ALL, EdgeKind::parse));
    let label = field!(v, o, path, "label", required "string", |x, p| v.string(x, &p, upto(60)));
    let ev = field!(v, o, path, "evidence", default Vec::new(), |x, p| v.array(x, &p, 0, usize::MAX, evidence));
    let proposed = field!(v, o, path, "proposed", optional, |x, p| proposal(v, x, &p));
    let removed = field!(v, o, path, "removed", optional, |x, p| v.boolean(x, &p));
    let branch = field!(v, o, path, "branch", optional, |x, p| branch(v, x, &p));
    Some(MapEdge {
        id: id?,
        from: from?,
        to: to?,
        kind: kind?,
        label: label?,
        evidence: ev?,
        proposed: proposed?,
        removed: removed?,
        branch: branch?,
    })
}

fn strings(v: &mut V, x: &Value, path: &str, t: Text) -> Option<Vec<String>> {
    v.array(x, path, 0, usize::MAX, |v, x, p| v.string(x, p, t))
}

fn ids(v: &mut V, x: &Value, path: &str) -> Option<Vec<String>> {
    v.array(x, path, 0, usize::MAX, |v, x, p| v.id(x, p))
}

fn absence(v: &mut V, x: &Value, path: &str) -> Option<MapAbsence> {
    let o = v.object(x, path)?;
    let id = field!(v, o, path, "id", required "string", |x, p| v.id(x, &p));
    let claim = field!(v, o, path, "claim", required "string", |x, p| v.string(x, &p, upto(200)));
    let between = field!(v, o, path, "between", optional, |x, p| {
        let path = p.as_str();
        v.object(x, path).and_then(|o| {
            let from = field!(v, o, path, "from", required "string", |x, p| v.id(x, &p));
            let to = field!(v, o, path, "to", required "string", |x, p| v.id(x, &p));
            Some(Between { from: from?, to: to? })
        })
    });
    let search = field!(v, o, path, "search", required "object", |x, p| {
        let path = p.as_str();
        v.object(x, path).and_then(|o| {
            let pattern = field!(v, o, path, "pattern", required "string", |x, p| v.string(x, &p, upto(200)));
            let regex = field!(v, o, path, "regex", default false, |x, p| v.boolean(x, &p));
            let paths = field!(v, o, path, "paths", optional, |x, p| strings(v, x, &p, NONEMPTY));
            Some(Search { pattern: pattern?, regex: regex?, paths: paths? })
        })
    });
    let matches = field!(v, o, path, "matches", required "number", |x, p| v.int(x, &p, 0));
    Some(MapAbsence { id: id?, claim: claim?, between: between?, search: search?, matches: matches? })
}

fn tour_stop(v: &mut V, x: &Value, path: &str) -> Option<TourStop> {
    let o = v.object(x, path)?;
    let id = field!(v, o, path, "id", required "string", |x, p| v.id(x, &p));
    let title = field!(v, o, path, "title", required "string", |x, p| v.string(x, &p, upto(80)));
    let text = field!(v, o, path, "text", required "string", |x, p| v.string(x, &p, upto(1200)));
    let focus = field!(v, o, path, "focus", required "object", |x, p| {
        let path = p.as_str();
        v.object(x, path).and_then(|o| {
            let areas = field!(v, o, path, "areas", optional, |x, p| ids(v, x, &p));
            let nodes = field!(v, o, path, "nodes", optional, |x, p| ids(v, x, &p));
            let edges = field!(v, o, path, "edges", optional, |x, p| ids(v, x, &p));
            let absences = field!(v, o, path, "absences", optional, |x, p| ids(v, x, &p));
            Some(Focus { areas: areas?, nodes: nodes?, edges: edges?, absences: absences? })
        })
    });
    let terms = field!(v, o, path, "terms", optional, |x, p| strings(v, x, &p, ANY));
    let ask = field!(v, o, path, "ask", optional, |x, p| strings(v, x, &p, ANY));
    Some(TourStop { id: id?, title: title?, text: text?, focus: focus?, terms: terms?, ask: ask? })
}

fn shape(v: &mut V, input: &Value) -> Option<CodeMap> {
    let path = "";
    let o = v.object(input, path)?;
    let version = field!(v, o, path, "version", required "1", |x, p| {
        if x.as_u64() == Some(1) {
            Some(1u8)
        } else {
            v.push(&p, "Invalid input: expected 1");
            None
        }
    });
    let question = field!(v, o, path, "question", required "string", |x, p| v.string(x, &p, upto(QUESTION_MAX)));
    let commit = field!(v, o, path, "commit", optional, |x, p| nullable_string(v, x, &p));
    let roots = field!(v, o, path, "roots", optional, |x, p| roots(v, x, &p));
    let view = field!(v, o, path, "view", optional, |x, p| v.enumerated(x, &p, View::ALL, View::parse));
    let since = field!(v, o, path, "since", optional, |x, p| {
        let s = v.string(x, &p, ANY)?;
        if is_git_ref(&s) {
            Some(s)
        } else {
            v.push(&p, "a commit, branch or tag name, e.g. \"HEAD\"");
            None
        }
    });
    let pages = field!(v, o, path, "pages", optional, |x, p| v.array(x, &p, 0, usize::MAX, page));
    let areas = field!(v, o, path, "areas", required "array", |x, p| v.array(x, &p, 1, usize::MAX, area));
    let nodes = field!(v, o, path, "nodes", required "array", |x, p| v.array(x, &p, 1, usize::MAX, node));
    let edges = field!(v, o, path, "edges", default Vec::new(), |x, p| v.array(x, &p, 0, usize::MAX, edge));
    let absences = field!(v, o, path, "absences", default Vec::new(), |x, p| v.array(x, &p, 0, usize::MAX, absence));
    let tour = field!(v, o, path, "tour", default Vec::new(), |x, p| v.array(x, &p, 0, usize::MAX, tour_stop));
    Some(CodeMap {
        version: version?,
        question: question?,
        commit: commit?,
        roots: roots?,
        view: view?,
        since: since?,
        pages: pages?,
        areas: areas?,
        nodes: nodes?,
        edges: edges?,
        absences: absences?,
        tour: tour?,
    })
}

/// Validates shape, then references: unique ids, edges between real boxes, tour stops that point at real things.
///
/// Proposals have to hang off the code: everything cites a line except a proposed addition, an arrow to or from a
/// proposed box is itself proposed, and every proposed box connects, through arrows, to a box that exists. Removed
/// code needs something to have been removed since (`since`), and an arrow can't outlive a box at its end.
pub fn parse_map(input: &Value) -> ParseResult {
    let mut v = V { problems: Vec::new() };
    let map = shape(&mut v, input);
    let Some(map) = map.filter(|_| v.problems.is_empty()) else {
        return Err(v.problems);
    };
    let problems = references(&map);
    if problems.is_empty() {
        Ok(map)
    } else {
        Err(problems)
    }
}

/// A tour stop's list of ids of one kind, and the ids of that kind the map has.
type Ref<'a> = (&'a str, &'a Option<Vec<String>>, &'a HashSet<String>);

fn references(map: &CodeMap) -> Vec<String> {
    let mut problems: Vec<String> = Vec::new();
    let unique = |kind: &str, ids: &mut dyn Iterator<Item = &String>, problems: &mut Vec<String>| -> HashSet<String> {
        let mut seen = HashSet::new();
        for id in ids {
            if !seen.insert(id.clone()) {
                problems.push(format!("{kind} id \"{id}\" is used twice"));
            }
        }
        seen
    };
    let areas = unique("area", &mut map.areas.iter().map(|a| &a.id), &mut problems);
    let nodes = unique("node", &mut map.nodes.iter().map(|n| &n.id), &mut problems);
    let edges = unique("edge", &mut map.edges.iter().map(|e| &e.id), &mut problems);
    let absences = unique("absence", &mut map.absences.iter().map(|a| &a.id), &mut problems);
    unique("tour stop", &mut map.tour.iter().map(|t| &t.id), &mut problems);
    let page_list = map.pages.as_deref().unwrap_or_default();
    let pages: HashMap<&str, &Page> = page_list.iter().map(|p| (p.id.as_str(), p)).collect();
    unique("page", &mut page_list.iter().map(|p| &p.id), &mut problems);
    for p in page_list {
        unique(&format!("page \"{}\" region", p.id), &mut p.regions.iter().map(|r| &r.id), &mut problems);
        for f in std::iter::once(&p.file).chain(p.regions.iter().map(|r| &r.file)) {
            if !is_safe_relative_path(f) {
                problems.push(format!("page \"{}\": file \"{f}\" must be a path inside the repo", p.id));
            }
        }
        if p.regions.iter().any(|r| r.elements.iter().any(|e| e.proposed == Some(true))) {
            problems.push(format!(
                "page \"{}\": a page shows the screen as it is, so its elements can't be `proposed` (use a sketch on the proposed box for that)",
                p.id
            ));
        }
    }

    let cites = |where_: &str, ev: &[Evidence], addition: bool, problems: &mut Vec<String>| {
        if ev.is_empty() && !addition {
            problems.push(format!(
                "{where_}: cite at least one line of code in `evidence` (only a proposed addition goes without)"
            ));
        }
        for e in ev {
            if !is_safe_relative_path(&e.file) {
                problems.push(format!("{where_}: evidence file \"{}\" must be a path inside the repo", e.file));
            }
        }
    };
    let added: Vec<&str> = map.nodes.iter().filter(|n| n.is_addition()).map(|n| n.id.as_str()).collect();
    let added_set: HashSet<&str> = added.iter().copied().collect();
    let removed: HashSet<&str> = map.nodes.iter().filter(|n| n.is_removed()).map(|n| n.id.as_str()).collect();
    let gone = |where_: &str, is_removed: bool, proposed: bool, problems: &mut Vec<String>| {
        if !is_removed {
            return;
        }
        if map.since.is_none() {
            problems.push(format!("{where_}: `removed` needs the map's `since` (the commit it was removed after)"));
        }
        if proposed {
            problems.push(format!("{where_}: it can't be both `removed` (already gone) and `proposed` (not done yet)"));
        }
    };

    for n in &map.nodes {
        let at = format!("node \"{}\"", n.id);
        if !areas.contains(&n.area) {
            problems.push(format!("{at}: unknown area \"{}\"", n.area));
        }
        if let Some(sym) = &n.symbol {
            if !is_safe_relative_path(&sym[..sym.find('#').unwrap_or(0)]) {
                problems.push(format!("{at}: symbol file must be a path inside the repo"));
            }
        }
        cites(&at, &n.evidence, n.is_addition(), &mut problems);
        if let Some(sc) = &n.screen {
            if sc.kind == ScreenKind::Capture && sc.capture.is_none() {
                problems.push(format!("{at}: a capture screen needs the `capture` id the capture tool returned"));
            }
            if matches!(sc.kind, ScreenKind::Image | ScreenKind::Sketch)
                && !sc.file.as_deref().is_some_and(is_safe_relative_path)
            {
                problems.push(format!("{at}: a {} screen needs `file`, a path inside the repo", sc.kind.as_str()));
            }
            if sc.kind == ScreenKind::Wireframe {
                match pages.get(sc.page.as_deref().unwrap_or("")) {
                    None => {
                        let which =
                            sc.page.as_ref().map(|p| format!(" (there is no page \"{p}\")")).unwrap_or_default();
                        problems.push(format!(
                            "{at}: a wireframe screen needs `page`, the id of one of the map's `pages`{which}"
                        ));
                    }
                    Some(page) => {
                        if let Some(r) = &sc.region {
                            if !page.regions.iter().any(|x| &x.id == r) {
                                problems.push(format!("{at}: page \"{}\" has no region \"{r}\"", page.id));
                            }
                        }
                    }
                }
                if sc.highlight.is_some() && sc.region.is_none() {
                    problems.push(format!("{at}: a wireframe's `highlight` needs the `region` the element is in"));
                }
            }
            if sc.kind == ScreenKind::Sketch && sc.elements.is_none() {
                problems.push(format!("{at}: a sketch screen needs `elements`"));
            }
            if n.proposed.is_none()
                && sc.elements.as_ref().is_some_and(|es| es.iter().any(|e| e.proposed == Some(true)))
            {
                problems.push(format!("{at}: only a box that carries `proposed` can have proposed screen elements"));
            }
        }
        gone(&at, n.is_removed(), n.proposed.is_some(), &mut problems);
    }
    for e in &map.edges {
        let at = format!("edge \"{}\"", e.id);
        for end in [&e.from, &e.to] {
            if !nodes.contains(end) {
                problems.push(format!("{at}: unknown node \"{end}\""));
            }
        }
        cites(&at, &e.evidence, e.is_addition(), &mut problems);
        if let Some(end) = [&e.from, &e.to].into_iter().find(|id| added_set.contains(id.as_str())) {
            if !e.is_addition() {
                problems.push(format!(
                    "{at}: box \"{end}\" is a proposed addition, so an arrow to or from it must be one too (`proposed`: {{ \"kind\": \"add\" }})"
                ));
            }
        }
        gone(&at, e.is_removed(), e.proposed.is_some(), &mut problems);
        if let Some(was) = [&e.from, &e.to].into_iter().find(|id| removed.contains(id.as_str())) {
            if !e.is_removed() {
                problems.push(format!(
                    "{at}: box \"{was}\" was removed, so an arrow to or from it was removed too (`removed`: true)"
                ));
            }
        }
    }
    // A proposed box that touches nothing real is a drawing, not a change to this code.
    if !added.is_empty() {
        let mut anchored: HashSet<&str> =
            map.nodes.iter().map(|n| n.id.as_str()).filter(|id| !added_set.contains(id)).collect();
        let mut grew = true;
        while grew {
            grew = false;
            for e in &map.edges {
                if anchored.contains(e.from.as_str()) == anchored.contains(e.to.as_str()) {
                    continue;
                }
                anchored.insert(&e.from);
                anchored.insert(&e.to);
                grew = true;
            }
        }
        for id in &added {
            if nodes.contains(*id) && !anchored.contains(id) {
                problems.push(format!(
                    "node \"{id}\": a proposed box must connect, through arrows, to a box that exists in the code"
                ));
            }
        }
    }
    for a in &map.absences {
        let Some(b) = &a.between else { continue };
        let at = format!("absence \"{}\"", a.id);
        for end in [&b.from, &b.to] {
            if !nodes.contains(end) {
                problems.push(format!("{at}: unknown node \"{end}\""));
            }
        }
        if let Some(end) = [&b.from, &b.to].into_iter().find(|id| added_set.contains(id.as_str())) {
            problems.push(format!("{at}: box \"{end}\" is a proposed addition; an absence is about the code as it is"));
        }
        if let Some(was) = [&b.from, &b.to].into_iter().find(|id| removed.contains(id.as_str())) {
            problems.push(format!("{at}: box \"{was}\" was removed; an absence is about the code as it is"));
        }
    }
    for t in &map.tour {
        let refs: [Ref<'_>; 4] = [
            ("area", &t.focus.areas, &areas),
            ("node", &t.focus.nodes, &nodes),
            ("edge", &t.focus.edges, &edges),
            ("absence", &t.focus.absences, &absences),
        ];
        for (kind, list, known) in refs {
            for id in list.iter().flatten() {
                if !known.contains(id) {
                    problems.push(format!("tour stop \"{}\": unknown {kind} \"{id}\"", t.id));
                }
            }
        }
    }
    problems
}
