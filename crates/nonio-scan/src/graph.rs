//! Facts for Java, Kotlin and Python from `code2graph`'s precise tier (D14), behind the facts contract (D12).
//!
//! `code2graph` (Apache-2.0, a dependency, nothing copied) parses each file with tree-sitter and resolves
//! references across the whole set of files with [`LayeredResolver::default_scoped`]: lexical scopes, imports,
//! qualified paths, inherited members and receivers whose local type is written. That tier never matches by name
//! alone across the repo, but three of its rules still do, inside a smaller set; this adapter checks every edge it
//! keeps again, so a call in the facts is one a reader of the code would also resolve (D16):
//!
//! - **A qualified call is a path lookup.** `client.send()` resolves to `send` in a module or type whose path ends
//!   in `client`, even when `client` is a parameter. Kept only when the receiver is an import, a type, or a name
//!   nothing in the file binds (a package path); dropped when it is a local, a parameter, a field, a constructor
//!   property, or `this`/`self`.
//! - **An unqualified call falls back to the package.** A name the file doesn't bind resolves to the one definition
//!   with that name in the same package: right for a class or a Kotlin top-level function, a name match for a method
//!   of another class (`this.repo.save()` in Java is read without its receiver). A call to a member is kept only
//!   when that member belongs to a type enclosing the call (or one it extends), or comes through a static import.
//! - **Types are matched by their bare name.** The receiver-typed and inherited-member passes look a type up by its
//!   simple name across all packages, so two `Mailer` classes are one. Their edges are kept only when the type name,
//!   read where it is written, resolves to the type that owns the member: same file, then an explicit import, then
//!   the same package.
//!
//! Everything else stays out of the facts and the checker falls back to the text the claim cites.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::LazyLock;

use code2graph::resolve::{LayeredResolver, Resolver};
use code2graph::{
    Binding, BindingKind, BindingTarget, CodeGraph, Edge, Provenance, RefRole, Reference, Symbol, SymbolId,
    SymbolKind as GraphKind, TypeRefContext, Visibility,
};
use nonio_facts::{CallConfidence, CallFact, Extractor, FileFacts, ImportFact, RepoView, Span, SymbolFact, SymbolKind};
use regex::Regex;

use crate::hash::{hash_text, squash};

/// The extensions read through `code2graph`.
pub const GRAPH_EXTENSIONS: [&str; 4] = [".java", ".kt", ".kts", ".py"];

pub fn is_graph_file(path: &str) -> bool {
    GRAPH_EXTENSIONS.iter().any(|e| path.ends_with(e))
}

/// Facts for the Java, Kotlin and Python files of one repo.
///
/// Calls cross files, so the extractor reads every such file it was given (through the [`RepoView`], so a commit is
/// read as that commit) and resolves them together; the result is kept until one of those texts changes.
pub struct GraphExtractor {
    files: Vec<String>,
    built: RefCell<Option<Built>>,
}

struct Built {
    texts: Vec<(String, Rc<str>)>,
    facts: HashMap<String, FileFacts>,
}

impl GraphExtractor {
    /// `files` are the repo's files (any extension; the others are ignored), e.g. from `list_repo_files`.
    pub fn new<I, S>(files: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut files: Vec<String> = files.into_iter().map(Into::into).filter(|f| is_graph_file(f)).collect();
        files.sort();
        files.dedup();
        Self { files, built: RefCell::new(None) }
    }

    fn texts(&self, repo: &dyn RepoView, path: &str, source: &str) -> Vec<(String, Rc<str>)> {
        let mut texts: Vec<(String, Rc<str>)> =
            self.files.iter().filter(|f| f.as_str() != path).filter_map(|f| Some((f.clone(), repo.text(f)?))).collect();
        texts.push((path.to_string(), Rc::from(source)));
        texts.sort_by(|a, b| a.0.cmp(&b.0));
        texts
    }
}

fn same_texts(a: &[(String, Rc<str>)], b: &[(String, Rc<str>)]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.0 == y.0 && (Rc::ptr_eq(&x.1, &y.1) || x.1 == y.1))
}

impl Extractor for GraphExtractor {
    fn extract(&self, repo: &dyn RepoView, path: &str, source: &str) -> Option<FileFacts> {
        if !is_graph_file(path) {
            return None;
        }
        let texts = self.texts(repo, path, source);
        if let Some(b) = self.built.borrow().as_ref() {
            if same_texts(&b.texts, &texts) {
                return b.facts.get(path).cloned();
            }
        }
        let facts = build(&texts);
        let out = facts.get(path).cloned();
        *self.built.borrow_mut() = Some(Built { texts, facts });
        out
    }
}

/// Facts for every file in `texts`, resolved together.
pub fn graph_facts(texts: &[(String, Rc<str>)]) -> HashMap<String, FileFacts> {
    build(texts)
}

fn build(texts: &[(String, Rc<str>)]) -> HashMap<String, FileFacts> {
    let parsed: Vec<code2graph::FileFacts> =
        texts.iter().filter_map(|(path, text)| code2graph::extract_path(path, text).ok()).collect();
    let graph = LayeredResolver::default_scoped()
        .resolve(&parsed)
        .unwrap_or(CodeGraph { symbols: Vec::new(), edges: Vec::new() });
    let ix = Index::new(&parsed, &graph, texts);
    texts
        .iter()
        .map(|(path, text)| {
            let facts = match ix.files.get(path.as_str()) {
                Some(f) => ix.file_facts(f, text),
                None => FileFacts { hash: hash_text(text), ..FileFacts::empty(path) },
            };
            (path.clone(), facts)
        })
        .collect()
}

fn is_type(kind: GraphKind) -> bool {
    matches!(
        kind,
        GraphKind::Class
            | GraphKind::Struct
            | GraphKind::Enum
            | GraphKind::Interface
            | GraphKind::Trait
            | GraphKind::TypeAlias
    )
}

/// The names of a symbol's path: namespaces, then types and members (`com, acme, OrderService, create`).
fn names(id: &SymbolId) -> Vec<String> {
    id.descriptor_names_iter().map(str::to_string).collect()
}

fn namespace_len(id: &SymbolId) -> usize {
    id.namespaces_iter().count()
}

/// A Java or Kotlin import of a whole package: `import com.acme.model.*`.
static WILDCARD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*import\s+([A-Za-z_][\w]*(?:\.[A-Za-z_][\w]*)*)\.\*\s*;?\s*$").unwrap());
/// A Kotlin constructor property just before its type: `private val mailer: ` in `class S(private val mailer: M)`.
/// The text just before a call's name in `this.repo.save()`, which Java gives without its receiver.
static THIS_FIELD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bthis\s*\.\s*([A-Za-z_]\w*)\s*\.\s*$").unwrap());
/// The text just before a class name in `new Order(…)`.
static NEW: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bnew\s+$").unwrap());
/// A local bound once to a constructor call: `repo = UserRepo(` (Python gives such a local no type).
static CONSTRUCTED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z_]\w*\s*=\s*([A-Za-z_]\w*)\s*\(").unwrap());
static CTOR_PROPERTY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(?:val|var)\s+([A-Za-z_]\w*)\s*:\s*$").unwrap());

/// Lookups over one resolved set of files.
struct Index<'g> {
    files: HashMap<&'g str, &'g code2graph::FileFacts>,
    texts: HashMap<&'g str, &'g str>,
    /// Graph symbol → nonio symbol id, per file (a Java package symbol is shared by its files).
    ids: HashMap<(&'g str, &'g SymbolId), String>,
    /// Graph symbol → the files that define it.
    defined_in: HashMap<&'g SymbolId, Vec<&'g Symbol>>,
    /// A type's path names → the type.
    types_by_names: HashMap<Vec<String>, &'g SymbolId>,
    /// `(package namespaces, name)` → the top-level types with that name in that package.
    types_by_package: HashMap<(Vec<String>, String), Vec<&'g SymbolId>>,
    /// `(owner type's path names, name)` → the methods of that name the type declares.
    methods: HashMap<(Vec<String>, String), Vec<&'g SymbolId>>,
    /// Edges by `(file, byte, role)` of the reference they resolve.
    edges_at: HashMap<(&'g str, usize, RefRole), Vec<&'g Edge>>,
    /// A type → the types it extends or implements, resolved where they are written.
    supers: HashMap<&'g SymbolId, Vec<&'g SymbolId>>,
}

impl<'g> Index<'g> {
    fn new(parsed: &'g [code2graph::FileFacts], graph: &'g CodeGraph, texts: &'g [(String, Rc<str>)]) -> Self {
        let mut ix = Index {
            files: parsed.iter().map(|f| (f.file.as_str(), f)).collect(),
            texts: texts.iter().map(|(p, t)| (p.as_str(), &**t)).collect(),
            ids: HashMap::new(),
            defined_in: HashMap::new(),
            types_by_names: HashMap::new(),
            types_by_package: HashMap::new(),
            methods: HashMap::new(),
            edges_at: HashMap::new(),
            supers: HashMap::new(),
        };
        for f in parsed {
            let mut taken: HashMap<String, usize> = HashMap::new();
            for s in &f.symbols {
                if s.kind == GraphKind::Module {
                    continue;
                }
                let all = names(&s.id);
                let ns = namespace_len(&s.id);
                let qual = all[ns.min(all.len())..].join(".");
                if qual.is_empty() {
                    continue;
                }
                let n = taken.entry(qual.clone()).or_insert(0);
                *n += 1;
                let name = if *n == 1 { qual } else { format!("{qual}~{n}") };
                ix.ids.insert((f.file.as_str(), &s.id), format!("{}#{name}", f.file));
                ix.defined_in.entry(&s.id).or_default().push(s);
                if matches!(s.kind, GraphKind::Method | GraphKind::Function) && all.len() > ns + 1 {
                    let list =
                        ix.methods.entry((all[..all.len() - 1].to_vec(), all[all.len() - 1].clone())).or_default();
                    if !list.contains(&&s.id) {
                        list.push(&s.id);
                    }
                }
                if is_type(s.kind) {
                    ix.types_by_names.insert(all.clone(), &s.id);
                    if all.len() == ns + 1 {
                        ix.types_by_package.entry((all[..ns].to_vec(), all[ns].clone())).or_default().push(&s.id);
                    }
                }
            }
        }
        for e in &graph.edges {
            ix.edges_at.entry((e.occ.file.as_str(), e.occ.byte, e.role)).or_default().push(e);
        }
        let mut supers: HashMap<&'g SymbolId, Vec<&'g SymbolId>> = HashMap::new();
        for f in parsed {
            for r in f.references.iter().filter(|r| r.role == RefRole::IsImplementation) {
                let subject = match r.qualifier.as_deref() {
                    Some(q) => ix.type_named(f, q),
                    None => ix.innermost_type(f, r.occ.byte),
                };
                if let (Some(sub), Some(sup)) = (subject, ix.type_named(f, &r.name)) {
                    supers.entry(sub).or_default().push(sup);
                }
            }
        }
        ix.supers = supers;
        ix
    }

    /// The type a name written in `f` stands for: a type of the same file, else the one an explicit import names
    /// (an import nothing here defines stops the search), else the one top-level type of that name in the package.
    /// Never a match by name across packages.
    fn type_named(&self, f: &'g code2graph::FileFacts, name: &str) -> Option<&'g SymbolId> {
        let name = name.trim_end_matches('?');
        if name.is_empty() || name.contains(['.', '<', '[']) {
            return None;
        }
        let local: Vec<&SymbolId> =
            f.symbols.iter().filter(|s| is_type(s.kind) && s.id.leaf_name() == Some(name)).map(|s| &s.id).collect();
        match local.as_slice() {
            [one] => return Some(one),
            [] => {}
            _ => return None,
        }
        let imports: Vec<&Reference> =
            f.references.iter().filter(|r| r.role == RefRole::Import && r.name == name).collect();
        if !imports.is_empty() {
            let mut found: HashSet<&SymbolId> = HashSet::new();
            for r in imports {
                for e in self.edges_at.get(&(f.file.as_str(), r.occ.byte, RefRole::Import)).into_iter().flatten() {
                    found.insert(&e.to);
                }
            }
            let mut found = found.into_iter().filter(|id| self.is_type_id(id));
            return match (found.next(), found.next()) {
                (Some(one), None) => self.types_by_names.get(&names(one)).copied(),
                (None, _) => self.imported_type(f, name),
                _ => None,
            };
        }
        let package = f.symbols.iter().find(|s| s.id.namespaces_iter().next().is_some())?;
        let ns: Vec<String> = package.id.namespaces_iter().map(str::to_string).collect();
        match self.types_by_package.get(&(ns, name.to_string())).map(Vec::as_slice) {
            Some([one]) => return Some(one),
            Some([]) | None => {}
            Some(_) => return None,
        }
        // Last, the packages imported whole (`import a.b.*`): only when exactly one of them has the name, as
        // Java and Kotlin reject an ambiguous one.
        let text = self.texts.get(f.file.as_str()).copied().unwrap_or("");
        let mut found: Vec<&SymbolId> = Vec::new();
        for m in WILDCARD.captures_iter(text) {
            let pkg: Vec<String> = m[1].split('.').map(str::to_string).collect();
            if let Some(ids) = self.types_by_package.get(&(pkg, name.to_string())) {
                found.extend(ids.iter().copied());
            }
        }
        match found.as_slice() {
            [one] => Some(one),
            _ => None,
        }
    }

    /// The type an import names by its full path, when code2graph left the import unresolved: in Java and Kotlin a
    /// class with a constructor has two definitions of that name (the class and the constructor), and code2graph
    /// links no import to an ambiguous name.
    fn imported_type(&self, f: &'g code2graph::FileFacts, name: &str) -> Option<&'g SymbolId> {
        let mut found: HashSet<&SymbolId> = HashSet::new();
        for r in f.references.iter().filter(|r| r.role == RefRole::Import && r.name == name) {
            let path = r.from_path.as_deref().filter(|p| !p.is_empty() && !p.starts_with('.'))?;
            let segs: Vec<&str> = path.split(['.', '/']).collect();
            let leaf = r.imported_name.as_deref().unwrap_or(&r.name);
            for ((ns, n), ids) in &self.types_by_package {
                if n == leaf
                    && ns.len() >= segs.len()
                    && ns[ns.len() - segs.len()..].iter().zip(&segs).all(|(a, b)| a == b)
                {
                    found.extend(ids.iter().copied());
                }
            }
        }
        let mut found = found.into_iter();
        match (found.next(), found.next()) {
            (Some(one), None) => Some(one),
            _ => None,
        }
    }

    fn is_type_id(&self, id: &SymbolId) -> bool {
        self.defined_in.get(id).is_some_and(|d| d.iter().any(|s| is_type(s.kind)))
    }

    /// The type that declares a member, or `None` for a top-level function or a type.
    fn owner(&self, id: &SymbolId) -> Option<&'g SymbolId> {
        let all = names(id);
        if all.len() <= namespace_len(id) + 1 {
            return None;
        }
        self.types_by_names.get(&all[..all.len() - 1]).copied()
    }

    /// The types whose bodies contain `at`, innermost first.
    fn enclosing_types(&self, f: &'g code2graph::FileFacts, at: usize) -> Vec<&'g SymbolId> {
        let mut found: Vec<&Symbol> =
            f.symbols.iter().filter(|s| is_type(s.kind) && s.span.start <= at && at < s.span.end).collect();
        found.sort_by_key(|s| s.span.end - s.span.start);
        found.into_iter().map(|s| &s.id).collect()
    }

    fn innermost_type(&self, f: &'g code2graph::FileFacts, at: usize) -> Option<&'g SymbolId> {
        self.enclosing_types(f, at).into_iter().next()
    }

    /// The types enclosing `at` and everything they extend or implement.
    fn types_around(&self, f: &'g code2graph::FileFacts, at: usize) -> HashSet<&'g SymbolId> {
        let mut seen: HashSet<&SymbolId> = HashSet::new();
        let mut stack = self.enclosing_types(f, at);
        while let Some(t) = stack.pop() {
            if seen.insert(t) {
                stack.extend(self.supers.get(t).into_iter().flatten().copied());
            }
        }
        seen
    }

    /// The binding a name has at `at`, walking out from the reference's scope.
    fn binding(
        &self,
        f: &'g code2graph::FileFacts,
        name: &str,
        at: usize,
        scope: Option<usize>,
    ) -> Option<&'g Binding> {
        let mut scope = scope;
        while let Some(s) = scope {
            let best = f
                .bindings
                .iter()
                .filter(|b| b.scope == s && b.name == name && (b.kind != BindingKind::Local || b.intro <= at))
                .max_by_key(|b| b.intro);
            if best.is_some() {
                return best;
            }
            scope = f.scopes.get(s).and_then(|x| x.parent);
        }
        None
    }

    /// Kotlin constructor properties of the types enclosing `at`: name → declared type as written.
    fn ctor_properties(&self, f: &'g code2graph::FileFacts, at: usize) -> HashMap<String, String> {
        let around: HashSet<&SymbolId> = self.enclosing_types(f, at).into_iter().collect();
        let text = self.texts.get(f.file.as_str()).copied().unwrap_or("");
        let mut out = HashMap::new();
        for r in &f.references {
            if r.role != RefRole::TypeRef || r.type_ref_ctx != Some(TypeRefContext::ParameterType) {
                continue;
            }
            // A constructor parameter belongs to the class itself, not to one of its methods.
            let owner = f
                .symbols
                .iter()
                .filter(|s| s.kind != GraphKind::Module && s.span.start <= r.occ.byte && r.occ.byte < s.span.end)
                .min_by_key(|s| s.span.end - s.span.start);
            let Some(owner) = owner.filter(|s| is_type(s.kind) && around.contains(&s.id)) else {
                continue;
            };
            let before = &text[owner.span.start..r.occ.byte.min(text.len())];
            if let Some(m) = CTOR_PROPERTY.captures(before) {
                let ty = match &r.qualifier {
                    Some(q) => format!("{q}.{}", r.name),
                    None => r.name.clone(),
                };
                out.entry(m[1].to_string()).or_insert(ty);
            }
        }
        out
    }

    /// Whether a ScopeGraph call edge is resolution a reader would make (see the module docs).
    fn keep_scoped(&self, f: &'g code2graph::FileFacts, r: &Reference, e: &Edge) -> bool {
        let at = r.occ.byte;
        if r.self_receiver {
            return self.owner(&e.to).is_some_and(|o| self.types_around(f, at).contains(o));
        }
        if let Some(q) = r.qualifier.as_deref() {
            let root = q.split('.').next().unwrap_or(q);
            if matches!(root, "this" | "self" | "super" | "cls") || self.ctor_properties(f, at).contains_key(root) {
                return false;
            }
            return match self.binding(f, root, at, r.scope) {
                None => true,
                Some(b) => match (&b.kind, &b.target) {
                    (BindingKind::Import, _) => true,
                    (BindingKind::Definition, BindingTarget::Def(id)) => self.is_type_id(id),
                    _ => false,
                },
            };
        }
        let Some(owner) = self.owner(&e.to) else {
            return true; // a type (constructor call) or a top-level function
        };
        if self.binding(f, &r.name, at, r.scope).is_some_and(|b| b.kind == BindingKind::Import) {
            return true; // static import
        }
        self.types_around(f, at).contains(owner)
    }

    /// Whether a receiver-typed edge (`x.m()`, `x` a local or parameter with a written type) is to a member of the
    /// type that name resolves to here.
    fn keep_local_type(&self, f: &'g code2graph::FileFacts, r: &Reference, e: &Edge) -> bool {
        let Some(q) = r.qualifier.as_deref() else {
            return false;
        };
        let Some(b) = self.binding(f, q, r.occ.byte, r.scope) else {
            return false;
        };
        if !matches!(b.kind, BindingKind::Local | BindingKind::Param) {
            return false;
        }
        let (Some(ty), Some(owner)) = (b.type_name.as_deref().and_then(|t| self.type_named(f, t)), self.owner(&e.to))
        else {
            return false;
        };
        ty == owner
    }

    /// Whether an inherited-member edge is to a type the enclosing type really extends.
    fn keep_inherited(&self, f: &'g code2graph::FileFacts, r: &Reference, e: &Edge) -> bool {
        if r.qualifier.as_deref().is_some_and(|q| !matches!(q, "this" | "self" | "super")) {
            return false;
        }
        self.owner(&e.to).is_some_and(|o| self.types_around(f, r.occ.byte).contains(o))
    }

    // ---- typed receivers and constructors (D15): what code2graph leaves unresolved, read from declared types

    /// The callee of a call code2graph gave no kept edge for, when the receiver's type is written in the code: a
    /// field or Kotlin constructor property of an enclosing type, a local or parameter with a declared type (or a
    /// local bound once to a constructor call), or the class of a Java `new`. The type name is resolved where it is
    /// written ([`Self::type_named`]) and the method looked up on that type and what it extends; any doubt gives
    /// nothing.
    fn typed_call(&self, f: &'g code2graph::FileFacts, r: &Reference) -> Option<&'g SymbolId> {
        let at = r.occ.byte;
        let text = self.texts.get(f.file.as_str()).copied().unwrap_or("");
        let before = tail(text, at, 160);
        if r.qualifier.is_none() && NEW.is_match(before) {
            return self.type_named(f, &r.name);
        }
        let ty = self.receiver_type(f, r, before)?;
        self.method_of(ty, &r.name)
    }

    fn receiver_type(&self, f: &'g code2graph::FileFacts, r: &Reference, before: &str) -> Option<&'g SymbolId> {
        let at = r.occ.byte;
        let (name, field_only) = match r.qualifier.as_deref() {
            Some(q) => match q.strip_prefix("this.") {
                Some(field) => (field.to_string(), true),
                None => (q.to_string(), false),
            },
            None => (THIS_FIELD.captures(before)?[1].to_string(), true),
        };
        if name.contains('.') || matches!(name.as_str(), "this" | "self" | "super") {
            return None;
        }
        if !field_only {
            match self.binding(f, &name, at, r.scope) {
                Some(b) if matches!(b.kind, BindingKind::Local | BindingKind::Param) => return self.local_type(f, b),
                Some(b) if b.kind == BindingKind::Definition => {
                    let BindingTarget::Def(id) = &b.target else {
                        return None;
                    };
                    let owner = self.owner(id)?;
                    return self.enclosing_types(f, at).contains(&owner).then(|| self.field_type(f, owner, &name))?;
                }
                Some(_) => return None,
                None => {}
            }
        }
        for t in self.enclosing_types(f, at) {
            if let Some(ty) = self.field_type(f, t, &name) {
                return Some(ty);
            }
        }
        let written = self.ctor_properties(f, at).remove(&name)?;
        self.type_named(f, &written)
    }

    fn local_type(&self, f: &'g code2graph::FileFacts, b: &Binding) -> Option<&'g SymbolId> {
        if let Some(t) = b.type_name.as_deref() {
            return self.type_named(f, t);
        }
        let once = f.bindings.iter().filter(|x| x.scope == b.scope && x.name == b.name).count() == 1;
        let text = self.texts.get(f.file.as_str()).copied().unwrap_or("");
        let class = CONSTRUCTED.captures(text.get(b.intro..)?)?;
        if !once || b.kind != BindingKind::Local {
            return None;
        }
        self.type_named(f, &class[1])
    }

    /// The declared type of field `name` of `owner`, from the one type written in the field's declaration.
    fn field_type(&self, f: &'g code2graph::FileFacts, owner: &SymbolId, name: &str) -> Option<&'g SymbolId> {
        let mut path = names(owner);
        path.push(name.to_string());
        let field = f.symbols.iter().find(|s| {
            matches!(s.kind, GraphKind::Field | GraphKind::Static | GraphKind::Const) && names(&s.id) == path
        })?;
        let mut written = f.references.iter().filter(|r| {
            r.role == RefRole::TypeRef
                && r.type_ref_ctx == Some(TypeRefContext::Field)
                && field.span.start <= r.occ.byte
                && r.occ.byte < field.span.end
        });
        match (written.next(), written.next()) {
            (Some(t), None) if t.qualifier.is_none() => self.type_named(f, &t.name),
            _ => None,
        }
    }

    /// The one method `name` of `ty`, or of the nearest types it extends that declare one.
    fn method_of(&self, ty: &'g SymbolId, name: &str) -> Option<&'g SymbolId> {
        let mut level = vec![ty];
        let mut seen: HashSet<&SymbolId> = HashSet::new();
        while !level.is_empty() {
            let mut found: Vec<&SymbolId> = Vec::new();
            let mut next = Vec::new();
            for t in level {
                if !seen.insert(t) {
                    continue;
                }
                if let Some(m) = self.methods.get(&(names(t), name.to_string())) {
                    found.extend(m.iter().copied());
                }
                next.extend(self.supers.get(t).into_iter().flatten().copied());
            }
            found.dedup();
            match found.as_slice() {
                [one] => return Some(one),
                [] => level = next,
                _ => return None,
            }
        }
        None
    }

    fn target(&self, id: &SymbolId) -> Option<String> {
        match self.defined_in.get(id)?.as_slice() {
            [one] => self.ids.get(&(one.file.as_str(), id)).cloned(),
            _ => None,
        }
    }

    fn file_facts(&self, f: &'g code2graph::FileFacts, text: &str) -> FileFacts {
        let mut facts = FileFacts { hash: hash_text(text), ..FileFacts::empty(&f.file) };
        facts.symbols = self.symbols(f, text);
        facts.imports = self.imports(f);
        facts.calls = self.calls(f);
        facts
    }

    fn symbols(&self, f: &'g code2graph::FileFacts, text: &str) -> Vec<SymbolFact> {
        let mut out = Vec::new();
        for s in &f.symbols {
            let Some(id) = self.ids.get(&(f.file.as_str(), &s.id)) else {
                continue;
            };
            let all = names(&s.id);
            let ns = namespace_len(&s.id);
            let member = all.len() > ns + 1;
            let kind = match s.kind {
                k if is_type(k) && k != GraphKind::TypeAlias => SymbolKind::Class,
                GraphKind::TypeAlias => SymbolKind::Type,
                GraphKind::Method if member => SymbolKind::Method,
                GraphKind::Method | GraphKind::Function => SymbolKind::Function,
                GraphKind::Field | GraphKind::Static | GraphKind::Const | GraphKind::Variant if member => {
                    SymbolKind::Property
                }
                GraphKind::Static | GraphKind::Const => SymbolKind::Const,
                _ => continue,
            };
            let exported = match s.visibility {
                Visibility::Public => true,
                Visibility::Unknown => !s.name.starts_with('_'),
                _ => false,
            };
            let parent = (1..all.len().saturating_sub(ns))
                .rev()
                .map(|n| &all[..ns + n])
                .find_map(|prefix| {
                    f.symbols
                        .iter()
                        .find(|p| p.kind != GraphKind::Module && names(&p.id) == prefix)
                        .and_then(|p| self.ids.get(&(f.file.as_str(), &p.id)))
                })
                .cloned();
            let body = text.get(s.span.start..s.span.end).unwrap_or("");
            out.push(SymbolFact {
                id: id.clone(),
                file: f.file.clone(),
                name: id[f.file.len() + 1..].to_string(),
                kind,
                exported,
                parent,
                span: Span { start: s.span.start, end: s.span.end },
                sig_hash: hash_text(&format!("{}|{}", s.signature, if exported { "export" } else { "" })),
                body_hash: hash_text(&squash(body)),
            });
        }
        out
    }

    fn imports(&self, f: &'g code2graph::FileFacts) -> Vec<ImportFact> {
        let mut out: Vec<ImportFact> = Vec::new();
        for r in f.references.iter().filter(|r| r.role == RefRole::Import) {
            let resolved: HashSet<&str> = self
                .edges_at
                .get(&(f.file.as_str(), r.occ.byte, RefRole::Import))
                .into_iter()
                .flatten()
                .filter_map(|e| match self.defined_in.get(&e.to)?.as_slice() {
                    [one] => Some(one.file.as_str()),
                    _ => None,
                })
                .collect();
            let mut resolved: Vec<&str> = resolved.into_iter().collect();
            if resolved.is_empty() {
                let by_path = self.imported_type(f, &r.name).and_then(|t| self.defined_in.get(t));
                if let Some([one]) = by_path.map(Vec::as_slice) {
                    resolved.push(one.file.as_str());
                }
            }
            let to = match resolved.as_slice() {
                [one] if *one != f.file => one.to_string(),
                [_] => continue,
                _ => match r.from_path.as_deref() {
                    Some(p) if !p.starts_with('.') && !p.is_empty() => format!("vendor:{p}"),
                    Some(_) => continue,
                    None => format!("vendor:{}", r.name),
                },
            };
            if !out.iter().any(|i| i.to == to) {
                out.push(ImportFact { to, type_only: false });
            }
        }
        out
    }

    fn calls(&self, f: &'g code2graph::FileFacts) -> Vec<CallFact> {
        let file = f.file.as_str();
        let mut out: Vec<CallFact> = Vec::new();
        for r in f.references.iter().filter(|r| r.role == RefRole::Call) {
            let edges = self.edges_at.get(&(file, r.occ.byte, RefRole::Call)).map(Vec::as_slice).unwrap_or(&[]);
            let mut kept: Vec<(&Edge, CallConfidence)> = Vec::new();
            for e in edges {
                let confidence = match e.provenance {
                    Provenance::ScopeGraph if self.keep_scoped(f, r, e) => CallConfidence::Syntactic,
                    Provenance::LocalType if self.keep_local_type(f, r, e) => CallConfidence::Typed,
                    Provenance::Conformance if self.keep_inherited(f, r, e) => CallConfidence::Dispatch,
                    _ => continue,
                };
                kept.push((e, confidence));
            }
            // One call site stands for one callee; two different ones mean nothing here is sure.
            let targets: HashSet<&SymbolId> = kept.iter().map(|(e, _)| &e.to).collect();
            let (to, from, confidence) = match targets.len() {
                1 => {
                    let (e, confidence) = kept[0];
                    (self.target(&e.to), self.caller(f, e), confidence)
                }
                0 => match self.typed_call(f, r) {
                    Some(to) => (self.target(to), self.caller_at(f, r.occ.byte), CallConfidence::Typed),
                    None => continue,
                },
                _ => continue,
            };
            let (Some(to), Some(from)) = (to, from) else {
                continue;
            };
            push_call(&mut out, from, to, confidence);
        }
        out
    }

    /// The innermost symbol around `at`, or the file for top-level code.
    fn caller_at(&self, f: &'g code2graph::FileFacts, at: usize) -> Option<String> {
        let file = f.file.as_str();
        let inner = f
            .symbols
            .iter()
            .filter(|s| s.kind != GraphKind::Module && s.span.start <= at && at < s.span.end)
            .min_by_key(|s| s.span.end - s.span.start);
        match inner {
            Some(s) => self.ids.get(&(file, &s.id)).cloned(),
            None => Some(file.to_string()),
        }
    }

    fn caller(&self, f: &'g code2graph::FileFacts, e: &Edge) -> Option<String> {
        let file = f.file.as_str();
        match self.ids.get(&(file, &e.from)) {
            Some(id) => Some(id.clone()),
            None => f.symbols.iter().any(|s| s.kind == GraphKind::Module && s.id == e.from).then(|| file.to_string()),
        }
    }
}

/// Up to `max` bytes of `text` before `at`, cut at a char boundary.
fn tail(text: &str, at: usize, max: usize) -> &str {
    let at = at.min(text.len());
    let mut start = at.saturating_sub(max);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..at]
}

fn push_call(out: &mut Vec<CallFact>, from: String, to: String, confidence: CallConfidence) {
    if from == to {
        return;
    }
    if let Some(c) = out.iter_mut().find(|c| c.from == from && c.to == to) {
        c.count += 1;
    } else {
        out.push(CallFact { from, to, confidence, count: 1 });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receiver_patterns() {
        assert_eq!(&THIS_FIELD.captures("  this.repo.").unwrap()[1], "repo");
        assert!(THIS_FIELD.captures("repo.").is_none());
        assert!(NEW.is_match("return new "));
        assert_eq!(&CONSTRUCTED.captures("repo = UserRepo()\n").unwrap()[1], "UserRepo");
        assert_eq!(tail("aé", 3, 1), "");
        assert_eq!(&CTOR_PROPERTY.captures("class S(private val mailer: ").unwrap()[1], "mailer");
        assert!(CTOR_PROPERTY.captures("class S(mailer: ").is_none());
    }
}
