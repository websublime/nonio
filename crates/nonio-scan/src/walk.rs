//! The one walk over a TS/JS file that names its symbols and finds its calls and framework routes. Ported from the
//! `Walker` class in `facts/extract.ts` of illuminate (MIT, Copyright (c) 2026 johnz4021), which walks oxc's ESTree
//! output; this walks oxc's Rust AST, the tree that output is made from.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use indexmap::IndexMap;
use nonio_facts::{CallConfidence, CallFact, RouteFact, SymbolFact, SymbolKind};
use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_span::{GetSpan, Span};
use oxc_syntax::scope::ScopeFlags;

use crate::hash::{hash_text, squash};
use crate::record::{ExportTarget, ModuleRecord};
use crate::resolve::package_name;
use crate::routes::route_fact;

/// What the walk may ask about the rest of the repo.
pub struct Context<'c> {
    /// `(from, spec)` → repo path, `vendor:<pkg>`, or `None`.
    pub resolve: &'c dyn Fn(&str, &str) -> Option<String>,
    /// The import/export record of another repo file, so calls link through barrels and default exports.
    pub record_of: &'c dyn Fn(&str) -> Option<Rc<ModuleRecord>>,
}

const ROUTE_METHODS: [&str; 8] = ["get", "post", "put", "patch", "delete", "options", "head", "all"];
const FRAMEWORKS: [&str; 6] = ["express", "hono", "fastify", "@koa/router", "koa-router", "@hono/zod-openapi"];

/// An expression with the wrappers that don't change what it is (`as`, `satisfies`, `!`, parentheses, `<T>x`,
/// instantiation, optional chains) taken off. A chain unwraps to the member or call inside it.
#[derive(Clone, Copy)]
enum U<'b, 'a> {
    E(&'b Expression<'a>),
    M(&'b MemberExpression<'a>),
    C(&'b CallExpression<'a>),
}

fn unwrap<'b, 'a>(e: &'b Expression<'a>) -> U<'b, 'a> {
    let mut x = e;
    loop {
        x = match x {
            Expression::TSAsExpression(t) => &t.expression,
            Expression::TSSatisfiesExpression(t) => &t.expression,
            Expression::TSNonNullExpression(t) => &t.expression,
            Expression::ParenthesizedExpression(t) => &t.expression,
            Expression::TSTypeAssertion(t) => &t.expression,
            Expression::TSInstantiationExpression(t) => &t.expression,
            Expression::ChainExpression(c) => {
                return match &c.expression {
                    ChainElement::CallExpression(call) => U::C(call),
                    ChainElement::TSNonNullExpression(t) => unwrap(&t.expression),
                    m => match m.as_member_expression() {
                        Some(m) => U::M(m),
                        None => U::E(x),
                    },
                }
            }
            other => {
                return match other.as_member_expression() {
                    Some(m) => U::M(m),
                    None => U::E(other),
                }
            }
        };
    }
}

impl<'b, 'a> U<'b, 'a> {
    fn expr(self) -> Option<&'b Expression<'a>> {
        match self {
            U::E(e) => Some(e),
            _ => None,
        }
    }
    fn object(self) -> Option<&'b ObjectExpression<'a>> {
        match self.expr()? {
            Expression::ObjectExpression(o) => Some(o),
            _ => None,
        }
    }
    fn func(self) -> Option<F<'b, 'a>> {
        match self.expr()? {
            Expression::FunctionExpression(f) => Some(F::Fun(f)),
            Expression::ArrowFunctionExpression(f) => Some(F::Arrow(f)),
            _ => None,
        }
    }
    fn class(self) -> Option<&'b Class<'a>> {
        match self.expr()? {
            Expression::ClassExpression(c) => Some(c),
            _ => None,
        }
    }
    fn ident(self) -> Option<&'a str> {
        match self.expr()? {
            Expression::Identifier(i) => Some(i.name.as_str()),
            _ => None,
        }
    }
    fn span(self) -> Span {
        match self {
            U::E(e) => e.span(),
            U::M(m) => m.span(),
            U::C(c) => c.span,
        }
    }
}

fn unwrap_opt<'b, 'a>(e: Option<&'b Expression<'a>>) -> Option<U<'b, 'a>> {
    e.map(unwrap)
}

fn arg_expr<'b, 'a>(a: Option<&'b Argument<'a>>) -> Option<&'b Expression<'a>> {
    a.and_then(Argument::as_expression)
}

/// A string literal, or a template literal with no substitutions, as its value.
fn str_value(e: Option<&Expression<'_>>) -> Option<String> {
    match unwrap(e?).expr()? {
        Expression::StringLiteral(s) => Some(s.value.to_string()),
        Expression::TemplateLiteral(t) if t.expressions.is_empty() => {
            t.quasis.first().and_then(|q| q.value.cooked.as_ref()).map(|c| c.to_string())
        }
        _ => None,
    }
}

fn key_name(key: &PropertyKey<'_>, computed: bool) -> Option<String> {
    match key {
        PropertyKey::StaticIdentifier(i) if !computed => Some(i.name.to_string()),
        PropertyKey::PrivateIdentifier(p) => Some(format!("#{}", p.name)),
        k => str_value(k.as_expression()),
    }
}

/// A function node: a function (declaration or expression) or an arrow.
#[derive(Clone, Copy)]
enum F<'b, 'a> {
    Fun(&'b Function<'a>),
    Arrow(&'b ArrowFunctionExpression<'a>),
}

impl<'b, 'a> F<'b, 'a> {
    fn span(self) -> Span {
        match self {
            F::Fun(f) => f.span,
            F::Arrow(f) => f.span,
        }
    }
    fn params(self) -> &'b FormalParameters<'a> {
        match self {
            F::Fun(f) => &f.params,
            F::Arrow(f) => &f.params,
        }
    }
    /// The span of the body as ESTree has it: the block, or the expression of an arrow.
    fn body_span(self) -> Option<Span> {
        match self {
            F::Fun(f) => f.body.as_ref().map(|b| b.span),
            F::Arrow(f) => Some(match &f.body {
                ArrowFunctionBody::FunctionBody(b) => b.span,
                e => e.as_expression().map_or(f.span, GetSpan::span),
            }),
        }
    }
    fn block(self) -> Option<&'b FunctionBody<'a>> {
        match self {
            F::Fun(f) => f.body.as_deref(),
            F::Arrow(f) => match &f.body {
                ArrowFunctionBody::FunctionBody(b) => Some(b),
                _ => None,
            },
        }
    }
}

/// What an imported name points at: the spec and the imported name (`default`, or `*` for a namespace or a
/// CommonJS module).
#[derive(Clone)]
struct Binding {
    spec: String,
    name: String,
}

/// A route handler as the route call names it.
enum Handler {
    Node(Span),
    Ident(String),
    Other,
}

struct PendingRoute {
    method: String,
    path: String,
    handler: Handler,
    enclosing: Option<String>,
}

pub struct Walker<'a, 'c> {
    path: &'c str,
    source: &'a str,
    ctx: &'c Context<'c>,
    record: &'c ModuleRecord,
    jsx: bool,
    pub symbols: Vec<SymbolFact>,
    pub calls: IndexMap<(String, String), CallFact>,
    sig_text: HashMap<String, String>,
    used: HashMap<String, u32>,
    cb_count: u32,
    bindings: HashMap<String, Binding>,
    top_names: HashSet<String>,
    /// Router local → route prefix.
    receivers: HashMap<String, String>,
    pending: Vec<PendingRoute>,
    node_sym: HashMap<(u32, u32), String>,
    current: Option<usize>,
    cls: Option<usize>,
    /// Named functions declared directly in each enclosing function's body (name → id), innermost last.
    locals: Vec<HashMap<String, String>>,
    resolved: HashMap<(String, String), Option<String>>,
}

impl<'a, 'c> Walker<'a, 'c> {
    pub fn new(path: &'c str, source: &'a str, ctx: &'c Context<'c>, record: &'c ModuleRecord) -> Self {
        Self {
            path,
            source,
            ctx,
            record,
            jsx: path.ends_with(".jsx") || path.ends_with(".tsx"),
            symbols: Vec::new(),
            calls: IndexMap::new(),
            sig_text: HashMap::new(),
            used: HashMap::new(),
            cb_count: 0,
            bindings: HashMap::new(),
            top_names: HashSet::new(),
            receivers: HashMap::new(),
            pending: Vec::new(),
            node_sym: HashMap::new(),
            current: None,
            cls: None,
            locals: Vec::new(),
            resolved: HashMap::new(),
        }
    }

    pub fn run(&mut self, program: &Program<'a>) {
        self.prepass(&program.body);
        for stmt in &program.body {
            self.statement(stmt);
        }
    }

    /// Mark symbols exported through `export { a, b as c }`, then hash signatures (which include the export flag).
    pub fn finish(&mut self) {
        for t in self.record.exports.values() {
            if let ExportTarget::Local(local) = t {
                if let Some(s) = self.symbols.iter_mut().find(|x| x.parent.is_none() && &x.name == local) {
                    s.exported = true;
                }
            }
        }
        for s in &mut self.symbols {
            let sig = self.sig_text.get(&s.id).map_or("", String::as_str);
            s.sig_hash = hash_text(&format!("{sig}|{}", if s.exported { "export" } else { "" }));
        }
    }

    /// Id of the innermost symbol whose span contains `at`.
    pub fn innermost(&self, at: usize) -> Option<String> {
        let mut best: Option<&SymbolFact> = None;
        for s in &self.symbols {
            if s.span.start <= at
                && at < s.span.end
                && best.is_none_or(|b| s.span.end - s.span.start < b.span.end - b.span.start)
            {
                best = Some(s);
            }
        }
        best.map(|s| s.id.clone())
    }

    /// First symbol starting in `[from, to)` at the same nesting as `from`.
    pub fn first_symbol_in(&self, from: usize, to: usize) -> Option<String> {
        let parent = self.innermost(from);
        self.symbols
            .iter()
            .find(|s| s.span.start >= from && s.span.start < to && s.parent == parent)
            .map(|s| s.id.clone())
    }

    /// The top-level symbol named `name`.
    pub fn top_symbol(&self, name: &str) -> Option<String> {
        self.symbols.iter().find(|s| s.parent.is_none() && s.name == name).map(|s| s.id.clone())
    }

    pub fn routes(&mut self) -> Vec<RouteFact> {
        let pending = std::mem::take(&mut self.pending);
        pending
            .into_iter()
            .map(|r| {
                let id = match &r.handler {
                    Handler::Node(span) => self.node_sym.get(&(span.start, span.end)).cloned(),
                    _ => None,
                };
                let id = id.or_else(|| match &r.handler {
                    Handler::Ident(name) => self.target_root(Root::Ident(name), Vec::new()),
                    _ => None,
                });
                route_fact(&r.method, &r.path, self.path, id.or(r.enclosing))
            })
            .collect()
    }

    // ---- declarations

    fn prepass(&mut self, body: &[Statement<'a>]) {
        let mut framework_locals = HashSet::new();
        for s in body {
            let Statement::ImportDeclaration(d) = s else { continue };
            if d.import_kind.is_type() {
                continue;
            }
            let spec = d.source.value.to_string();
            for sp in d.specifiers.iter().flatten() {
                let (local, name) = match sp {
                    ImportDeclarationSpecifier::ImportSpecifier(s) => {
                        if s.import_kind.is_type() {
                            continue;
                        }
                        (s.local.name.to_string(), s.imported.name().to_string())
                    }
                    ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => {
                        (s.local.name.to_string(), "default".into())
                    }
                    ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => (s.local.name.to_string(), "*".into()),
                };
                if package_name(&spec).is_some_and(|p| FRAMEWORKS.contains(&p.as_str())) {
                    framework_locals.insert(local.clone());
                }
                self.bindings.insert(local, Binding { spec: spec.clone(), name });
            }
        }
        for s0 in body {
            let (decl, exported): (Option<&Declaration<'a>>, bool) = match s0 {
                Statement::ExportDeclaration(e) => (Some(&e.declaration), true),
                Statement::ExportDefaultDeclaration(e) => {
                    match &e.declaration {
                        ExportDefaultDeclarationKind::FunctionDeclaration(f) => {
                            if let Some(id) = &f.id {
                                self.top_names.insert(id.name.to_string());
                            }
                        }
                        ExportDefaultDeclarationKind::ClassDeclaration(c) => {
                            if let Some(id) = &c.id {
                                self.top_names.insert(id.name.to_string());
                            }
                        }
                        _ => {}
                    }
                    continue;
                }
                s => (s.as_declaration(), false),
            };
            match decl {
                Some(Declaration::FunctionDeclaration(f)) if f.r#type == FunctionType::FunctionDeclaration => {
                    if let Some(id) = &f.id {
                        self.top_names.insert(id.name.to_string());
                    }
                }
                Some(Declaration::ClassDeclaration(c)) => {
                    if let Some(id) = &c.id {
                        self.top_names.insert(id.name.to_string());
                    }
                }
                Some(Declaration::VariableDeclaration(v)) => {
                    for d in &v.declarations {
                        let BindingPattern::BindingIdentifier(id) = &d.id else { continue };
                        if self.is_symbol_declarator(d, exported) {
                            self.top_names.insert(id.name.to_string());
                        }
                        if let Some(prefix) = self.framework_prefix(d.init.as_ref(), &framework_locals) {
                            self.receivers.insert(id.name.to_string(), prefix);
                        }
                    }
                }
                _ => {}
            }
        }
        for t in self.record.exports.values() {
            if let ExportTarget::Local(l) = t {
                self.top_names.insert(l.clone());
            }
        }
    }

    fn is_symbol_declarator(&self, d: &VariableDeclarator<'a>, exported: bool) -> bool {
        let init = unwrap_opt(d.init.as_ref());
        if init.is_some_and(|i| i.func().is_some() || i.class().is_some()) {
            return true;
        }
        if exported {
            return true;
        }
        // Object literals that are typed, or hold functions (a module-like namespace), name their members.
        init.and_then(U::object).is_some_and(|o| {
            d.type_annotation.is_some()
                || o.properties.iter().any(|p| match p {
                    ObjectPropertyKind::ObjectProperty(p) => p.method || unwrap(&p.value).func().is_some(),
                    ObjectPropertyKind::SpreadProperty(_) => false,
                })
        })
    }

    /// Route prefix if `init` creates a router from an imported framework: `express()`, `express.Router()`,
    /// `new Hono()`, `new Hono().basePath('/api')` (prefix `/api`), `Fastify()`, `new Router({ prefix: '/x' })`.
    fn framework_prefix(&self, init: Option<&Expression<'a>>, locals: &HashSet<String>) -> Option<String> {
        let mut x = unwrap(init?);
        let mut prefix = String::new();
        loop {
            let U::C(call) = (match x {
                U::E(Expression::CallExpression(c)) => U::C(c),
                other => other,
            }) else {
                break;
            };
            let U::M(m) = unwrap(&call.callee) else { break };
            let obj = unwrap(m.object());
            if obj.ident().is_some() {
                break;
            }
            if let MemberExpression::StaticMemberExpression(s) = m {
                if s.property.name == "basePath" {
                    prefix = str_value(arg_expr(call.arguments.first())).unwrap_or_default() + &prefix;
                }
            }
            x = obj;
        }
        let (callee, args) = match x {
            U::C(c) => (&c.callee, &c.arguments),
            U::E(Expression::CallExpression(c)) => (&c.callee, &c.arguments),
            U::E(Expression::NewExpression(n)) => (&n.callee, &n.arguments),
            _ => return None,
        };
        let ok = match unwrap(callee) {
            U::E(Expression::Identifier(i)) => locals.contains(i.name.as_str()),
            U::M(m) => matches!(m.object(), Expression::Identifier(i) if locals.contains(i.name.as_str())),
            _ => false,
        };
        if !ok {
            return None;
        }
        let opts = arg_expr(args.first()).map(unwrap).and_then(U::object);
        let p = opts.and_then(|o| {
            o.properties.iter().find_map(|q| match q {
                ObjectPropertyKind::ObjectProperty(q) if key_name(&q.key, q.computed).as_deref() == Some("prefix") => {
                    Some(q)
                }
                _ => None,
            })
        });
        Some(p.map(|p| str_value(Some(&p.value)).unwrap_or_default()).unwrap_or_default() + &prefix)
    }

    fn statement(&mut self, s: &Statement<'a>) {
        match s {
            Statement::ImportDeclaration(_)
            | Statement::ExportNamedDeclaration(_)
            | Statement::ExportFromDeclaration(_) => {}
            Statement::ExportDeclaration(e) => self.declaration(&e.declaration, true, e.span),
            Statement::ExportDefaultDeclaration(e) => self.export_default(e),
            Statement::FunctionDeclaration(_)
            | Statement::ClassDeclaration(_)
            | Statement::VariableDeclaration(_)
            | Statement::TSInterfaceDeclaration(_)
            | Statement::TSTypeAliasDeclaration(_)
            | Statement::TSEnumDeclaration(_) => {
                if let Some(d) = s.as_declaration() {
                    self.declaration(d, false, s.span());
                }
            }
            s => self.visit_statement(s),
        }
    }

    fn declaration(&mut self, d: &Declaration<'a>, exported: bool, outer: Span) {
        match d {
            Declaration::FunctionDeclaration(f) => {
                if let (Some(id), Some(_)) = (&f.id, &f.body) {
                    self.func(id.name.as_str(), F::Fun(f), exported, None, outer, None);
                }
            }
            Declaration::ClassDeclaration(c) => {
                let name = c.id.as_ref().map_or("default", |i| i.name.as_str());
                self.klass(name, c, exported, outer);
            }
            Declaration::VariableDeclaration(v) => {
                let one = v.declarations.len() == 1;
                for d in &v.declarations {
                    self.declarator(d, exported, if one { outer } else { d.span });
                }
            }
            Declaration::TSInterfaceDeclaration(_)
            | Declaration::TSTypeAliasDeclaration(_)
            | Declaration::TSEnumDeclaration(_) => {
                if exported {
                    self.type_decl(d, outer);
                }
            }
            d => self.visit_declaration(d),
        }
    }

    fn export_default(&mut self, s: &ExportDefaultDeclaration<'a>) {
        match &s.declaration {
            // `export default function f(): T;` (a declaration file) is a TSDeclareFunction in ESTree: neither a
            // function nor an expression the original names, so it becomes the `default` const below.
            ExportDefaultDeclarationKind::FunctionDeclaration(f) if f.r#type == FunctionType::TSDeclareFunction => {
                self.define("default", SymbolKind::Const, true, None, s.span, "const", self.text(f.span));
            }
            ExportDefaultDeclarationKind::FunctionDeclaration(f) => {
                let name = f.id.as_ref().map_or("default", |i| i.name.as_str());
                self.func(name, F::Fun(f), true, None, s.span, None);
            }
            ExportDefaultDeclarationKind::ClassDeclaration(c) => {
                let name = c.id.as_ref().map_or("default", |i| i.name.as_str());
                self.klass(name, c, true, s.span);
            }
            ExportDefaultDeclarationKind::TSInterfaceDeclaration(i) => self.type_decl_interface(i, s.span),
            d => {
                let Some(d) = d.as_expression() else { return };
                match d {
                    Expression::FunctionExpression(f) => {
                        let name = f.id.as_ref().map_or("default", |i| i.name.as_str());
                        self.func(name, F::Fun(f), true, None, s.span, None);
                    }
                    Expression::ArrowFunctionExpression(f) => {
                        self.func("default", F::Arrow(f), true, None, s.span, None)
                    }
                    Expression::ClassExpression(c) => {
                        let name = c.id.as_ref().map_or("default", |i| i.name.as_str());
                        self.klass(name, c, true, s.span);
                    }
                    Expression::Identifier(_) => {}
                    d => {
                        let sym =
                            self.define("default", SymbolKind::Const, true, None, s.span, "const", self.text(d.span()));
                        self.inside(sym, |w| match unwrap(d).object() {
                            Some(o) => w.members(o, sym, ""),
                            None => w.visit_expression(d),
                        });
                    }
                }
            }
        }
    }

    fn declarator(&mut self, d: &VariableDeclarator<'a>, exported: bool, span: Span) {
        self.capture(d);
        let BindingPattern::BindingIdentifier(id) = &d.id else {
            if let Some(init) = &d.init {
                self.visit_expression(init);
            }
            return;
        };
        let name = id.name.as_str();
        let init = unwrap_opt(d.init.as_ref());
        if let Some(f) = init.and_then(U::func) {
            return self.func(name, f, exported, None, span, None);
        }
        if let Some(c) = init.and_then(U::class) {
            return self.klass(name, c, exported, span);
        }
        if !self.is_symbol_declarator(d, exported) {
            if let Some(init) = &d.init {
                self.visit_expression(init);
            }
            return;
        }
        // `export const chat = call;` aliases a local function: it is one.
        let alias = init
            .and_then(U::ident)
            .and_then(|n| self.symbols.iter().find(|s| s.parent.is_none() && s.name == n).map(|s| s.kind));
        let kind = match alias {
            Some(k) if k != SymbolKind::Const && k != SymbolKind::Type => k,
            _ => SymbolKind::Const,
        };
        let sig = format!("const {}", self.text_opt(d.type_annotation.as_ref().map(|t| t.span)));
        let body = self.text_opt(d.init.as_ref().map(GetSpan::span));
        let sym = self.define(name, kind, exported, None, span, &sig, body);
        self.inside(sym, |w| match init.and_then(U::object) {
            Some(o) => w.members(o, sym, ""),
            None => {
                if let Some(init) = &d.init {
                    w.visit_expression(init);
                }
            }
        });
    }

    /// Methods of an object literal held by a symbol: `newsSummarize.run`, nested `api.runs.list`.
    fn members(&mut self, obj: &ObjectExpression<'a>, parent: usize, prefix: &str) {
        for p in &obj.properties {
            let p = match p {
                ObjectPropertyKind::ObjectProperty(p) => p,
                ObjectPropertyKind::SpreadProperty(s) => {
                    self.visit_spread_element(s);
                    continue;
                }
            };
            let key = key_name(&p.key, p.computed);
            let v = unwrap(&p.value);
            if p.computed {
                self.visit_property_key(&p.key);
            }
            match (key, v.func(), v.object()) {
                (Some(key), Some(f), _) => {
                    self.func(&format!("{prefix}{key}"), f, false, Some(parent), p.span, Some(SymbolKind::Method))
                }
                (Some(key), None, Some(o)) => self.members(o, parent, &format!("{prefix}{key}.")),
                _ => self.visit_expression(&p.value),
            }
        }
    }

    fn func(
        &mut self,
        name: &str,
        node: F<'_, 'a>,
        exported: bool,
        parent: Option<usize>,
        span: Span,
        kind: Option<SymbolKind>,
    ) {
        let kind = kind.unwrap_or(if self.jsx && name.starts_with(|c: char| c.is_ascii_uppercase()) {
            SymbolKind::Component
        } else {
            SymbolKind::Function
        });
        let sig = self.fn_sig(node);
        let body = self.text_opt(node.body_span());
        let sym = self.define(name, kind, exported, parent, span, &sig, body);
        let s = node.span();
        self.node_sym.insert((s.start, s.end), self.symbols[sym].id.clone());
        let locals = self.local_fns(node, sym);
        self.locals.push(locals);
        self.inside(sym, |w| w.fn_body(node));
        self.locals.pop();
    }

    /// The functions declared directly in `node`'s body, with the ids they'll get as nested symbols (`outer.a`), so
    /// calls between them resolve even before the declaration (hoisting). A name declared twice is left out.
    fn local_fns(&self, node: F<'_, 'a>, sym: usize) -> HashMap<String, String> {
        let mut out = HashMap::new();
        let mut twice = HashSet::new();
        let owner = &self.symbols[sym].name;
        let mut add = |name: &str| {
            if out.contains_key(name) || twice.contains(name) {
                out.remove(name);
                twice.insert(name.to_string());
            } else {
                out.insert(name.to_string(), format!("{}#{owner}.{name}", self.path));
            }
        };
        for st in node.block().map(|b| &b.statements[..]).unwrap_or(&[]) {
            match st {
                Statement::FunctionDeclaration(f)
                    if f.body.is_some() && f.r#type == FunctionType::FunctionDeclaration =>
                {
                    if let Some(id) = &f.id {
                        add(id.name.as_str());
                    }
                }
                Statement::VariableDeclaration(v) => {
                    for d in &v.declarations {
                        if let BindingPattern::BindingIdentifier(id) = &d.id {
                            if unwrap_opt(d.init.as_ref()).and_then(U::func).is_some() {
                                add(id.name.as_str());
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }

    fn klass(&mut self, name: &str, node: &Class<'a>, exported: bool, span: Span) {
        let implements: Vec<&str> = node.implements.iter().map(|i| self.text(i.span)).collect();
        let sig = format!(
            "class {} {} {}",
            self.text_opt(node.type_parameters.as_ref().map(|t| t.span)),
            self.text_opt(node.heritage.as_ref().map(|h| h.expression.span())),
            implements.join(",")
        );
        let sym = self.define(name, SymbolKind::Class, exported, None, span, &sig, self.text(node.body.span));
        self.node_sym.insert((node.span.start, node.span.end), self.symbols[sym].id.clone());
        let prev = self.cls;
        self.cls = Some(sym);
        self.inside(sym, |w| {
            for dec in &node.decorators {
                w.visit_decorator(dec);
            }
            if let Some(h) = &node.heritage {
                w.visit_expression(&h.expression);
            }
            for m in &node.body.body {
                w.class_element(m, sym);
            }
        });
        self.cls = prev;
    }

    fn class_element(&mut self, m: &ClassElement<'a>, sym: usize) {
        match m {
            ClassElement::MethodDefinition(d) => {
                let key = key_name(&d.key, d.computed);
                let abstract_ = d.r#type == MethodDefinitionType::TSAbstractMethodDefinition;
                match (key, &d.value.body) {
                    (Some(key), Some(_)) => {
                        for dec in &d.decorators {
                            self.visit_decorator(dec);
                        }
                        let name =
                            if d.kind == MethodDefinitionKind::Constructor { "constructor".to_string() } else { key };
                        self.func(&name, F::Fun(&d.value), false, Some(sym), d.span, Some(SymbolKind::Method));
                    }
                    // No body, but a symbol: calls through it are dispatch and subclasses implement it.
                    (Some(key), None) if abstract_ => {
                        let sig = format!("abstract {}", self.fn_sig(F::Fun(&d.value)));
                        self.define(&key, SymbolKind::Method, false, Some(sym), d.span, &sig, "");
                    }
                    _ => self.visit_class_element(m),
                }
            }
            ClassElement::PropertyDefinition(d) if d.r#type == PropertyDefinitionType::PropertyDefinition => {
                let key = key_name(&d.key, d.computed);
                match (key, unwrap_opt(d.value.as_ref()).and_then(U::func)) {
                    (Some(key), Some(f)) => self.func(&key, f, false, Some(sym), d.span, Some(SymbolKind::Method)),
                    _ => self.visit_class_element(m),
                }
            }
            m => self.visit_class_element(m),
        }
    }

    fn type_decl(&mut self, d: &Declaration<'a>, span: Span) {
        match d {
            Declaration::TSInterfaceDeclaration(i) => self.type_decl_interface(i, span),
            Declaration::TSTypeAliasDeclaration(t) => {
                let sym = self.define(
                    t.id.name.as_str(),
                    SymbolKind::Type,
                    true,
                    None,
                    span,
                    self.text(t.span),
                    self.text(t.span),
                );
                if let TSType::TSTypeLiteral(l) = &t.type_annotation {
                    self.type_members(&l.members, sym);
                }
            }
            Declaration::TSEnumDeclaration(e) => {
                self.define(
                    e.id.name.as_str(),
                    SymbolKind::Type,
                    true,
                    None,
                    span,
                    self.text(e.span),
                    self.text(e.span),
                );
            }
            _ => {}
        }
    }

    fn type_decl_interface(&mut self, i: &TSInterfaceDeclaration<'a>, span: Span) {
        let sym =
            self.define(i.id.name.as_str(), SymbolKind::Type, true, None, span, self.text(i.span), self.text(i.span));
        self.type_members(&i.body.body, sym);
    }

    fn type_members(&mut self, members: &[TSSignature<'a>], sym: usize) {
        for m in members {
            let (key, fn_type, span) = match m {
                TSSignature::TSPropertySignature(p) => (
                    key_name(&p.key, p.computed),
                    p.type_annotation.as_ref().is_some_and(|t| matches!(t.type_annotation, TSType::TSFunctionType(_))),
                    p.span,
                ),
                TSSignature::TSMethodSignature(p) => (key_name(&p.key, p.computed), true, p.span),
                _ => continue,
            };
            let Some(key) = key else { continue };
            let kind = if fn_type { SymbolKind::Method } else { SymbolKind::Property };
            self.define(&key, kind, false, Some(sym), span, self.text(span), self.text(span));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn define(
        &mut self,
        name: &str,
        kind: SymbolKind,
        exported: bool,
        parent: Option<usize>,
        span: Span,
        sig: &str,
        body: &str,
    ) -> usize {
        let qual = match parent {
            Some(p) => format!("{}.{name}", self.symbols[p].name),
            None => name.to_string(),
        };
        let n = self.used.entry(qual.clone()).or_insert(0);
        *n += 1;
        let full = if *n == 1 { qual } else { format!("{qual}~{n}") };
        let id = format!("{}#{full}", self.path);
        self.sig_text.insert(id.clone(), format!("{}|{}", kind_name(kind), squash(sig)));
        self.symbols.push(SymbolFact {
            id,
            file: self.path.to_string(),
            name: full,
            kind,
            exported,
            parent: parent.map(|p| self.symbols[p].id.clone()),
            span: nonio_facts::Span { start: span.start as usize, end: span.end as usize },
            sig_hash: String::new(),
            body_hash: hash_text(&squash(body)),
        });
        self.symbols.len() - 1
    }

    fn fn_sig(&self, f: F<'_, 'a>) -> String {
        let p = f.params();
        let this = match f {
            F::Fun(f) => f.this_param.as_ref().map(|t| t.span),
            F::Arrow(_) => None,
        };
        let spans: Vec<Span> =
            this.into_iter().chain(p.items.iter().map(|i| i.span)).chain(p.rest.as_ref().map(|r| r.span)).collect();
        let params = match (spans.first(), spans.last()) {
            (Some(a), Some(b)) => crate::adapters::slice(self.source, a.start as usize, b.end as usize),
            _ => "",
        };
        let (is_async, generator, tp, rt) = match f {
            F::Fun(f) => (
                f.r#async,
                f.generator,
                f.type_parameters.as_ref().map(|t| t.span),
                f.return_type.as_ref().map(|t| t.span),
            ),
            F::Arrow(f) => {
                (f.r#async, false, f.type_parameters.as_ref().map(|t| t.span), f.return_type.as_ref().map(|t| t.span))
            }
        };
        format!(
            "{}{}{}({params}):{}",
            if is_async { "async " } else { "" },
            if generator { "*" } else { "" },
            self.text_opt(tp),
            self.text_opt(rt)
        )
    }

    fn text(&self, span: Span) -> &'a str {
        crate::adapters::slice(self.source, span.start as usize, span.end as usize)
    }

    fn text_opt(&self, span: Option<Span>) -> &'a str {
        span.map_or("", |s| self.text(s))
    }

    fn inside(&mut self, sym: usize, f: impl FnOnce(&mut Self)) {
        let prev = self.current;
        self.current = Some(sym);
        f(self);
        self.current = prev;
    }

    fn fn_body(&mut self, f: F<'_, 'a>) {
        if let F::Fun(func) = f {
            if let Some(t) = &func.this_param {
                self.visit_ts_this_parameter(t);
            }
        }
        self.visit_formal_parameters(f.params());
        match f {
            F::Fun(func) => {
                if let Some(b) = &func.body {
                    self.visit_function_body(b);
                }
            }
            F::Arrow(a) => match &a.body {
                ArrowFunctionBody::FunctionBody(b) => self.visit_function_body(b),
                e => {
                    let Some(e) = e.as_expression() else { return };
                    // `() => ({ save() {} })`: a function that returns an object of functions names them like a return
                    // statement does.
                    match (unwrap(e).object(), self.current) {
                        (Some(o), Some(cur)) => self.members(o, cur, ""),
                        _ => self.visit_expression(e),
                    }
                }
            },
        }
    }

    /// Bindings from `const { a } = await import('x')`, `const m = require('x')`.
    fn capture(&mut self, d: &VariableDeclarator<'a>) {
        let mut init = unwrap_opt(d.init.as_ref());
        if let Some(U::E(Expression::AwaitExpression(a))) = init {
            init = Some(unwrap(&a.argument));
        }
        let spec = match init {
            Some(U::E(Expression::ImportExpression(i))) => str_value(Some(&i.source)),
            Some(U::E(Expression::CallExpression(c))) => require_spec(c),
            Some(U::C(c)) => require_spec(c),
            _ => None,
        };
        let Some(spec) = spec else { return };
        match &d.id {
            BindingPattern::BindingIdentifier(id) => {
                self.bindings.insert(id.name.to_string(), Binding { spec, name: "*".into() });
            }
            BindingPattern::ObjectPattern(o) => {
                for p in &o.properties {
                    let Some(key) = key_name(&p.key, p.computed) else { continue };
                    let local = match &p.value {
                        BindingPattern::AssignmentPattern(a) => &a.left,
                        v => v,
                    };
                    if let BindingPattern::BindingIdentifier(l) = local {
                        self.bindings.insert(l.name.to_string(), Binding { spec: spec.clone(), name: key });
                    }
                }
            }
            _ => {}
        }
    }

    fn call(&mut self, callee: &Expression<'a>, args: Option<&[Argument<'a>]>) {
        let callee = unwrap(callee);
        let to = self.target(callee);
        let from = self.current.map_or_else(|| self.path.to_string(), |c| self.symbols[c].id.clone());
        if let Some(to) = to.filter(|t| *t != from) {
            let key = (from.clone(), to.clone());
            match self.calls.get_mut(&key) {
                Some(c) => c.count += 1,
                None => {
                    self.calls.insert(key, CallFact { from, to, confidence: CallConfidence::Syntactic, count: 1 });
                }
            }
        }
        if let Some(args) = args {
            self.framework_route(args, callee);
        }
    }

    /// Callee → symbol id (`file#name.member`), `vendor:<pkg>`, or `None` when it isn't an import or a top-level
    /// symbol.
    fn target(&mut self, callee: U<'_, 'a>) -> Option<String> {
        let mut chain: Vec<String> = Vec::new();
        let mut root = callee;
        while let U::M(m) = root {
            let prop = match m {
                MemberExpression::ComputedMemberExpression(c) => str_value(Some(&c.expression)),
                MemberExpression::StaticMemberExpression(s) => Some(s.property.name.to_string()),
                MemberExpression::PrivateFieldExpression(_) => None,
            }?;
            chain.insert(0, prop);
            root = unwrap(m.object());
        }
        match root.expr()? {
            Expression::ThisExpression(_) => self.target_root(Root::This, chain),
            Expression::Identifier(i) => self.target_root(Root::Ident(i.name.as_str()), chain),
            _ => None,
        }
    }

    fn target_root(&mut self, root: Root<'_>, chain: Vec<String>) -> Option<String> {
        let name = match root {
            Root::This => {
                let cls = self.cls?;
                return (!chain.is_empty())
                    .then(|| format!("{}#{}.{}", self.path, self.symbols[cls].name, chain.join(".")));
            }
            Root::Ident(n) => n,
        };
        // A function declared in an enclosing function's body shadows imports and top-level names.
        for scope in self.locals.iter().rev() {
            if let Some(local) = scope.get(name) {
                return Some(std::iter::once(local.clone()).chain(chain).collect::<Vec<_>>().join("."));
            }
        }
        if let Some(b) = self.bindings.get(name).cloned() {
            return self.link_import(&b, chain);
        }
        self.top_names.contains(name).then(|| {
            format!("{}#{}", self.path, std::iter::once(name.to_string()).chain(chain).collect::<Vec<_>>().join("."))
        })
    }

    fn res(&mut self, from: &str, spec: &str) -> Option<String> {
        let key = (from.to_string(), spec.to_string());
        if let Some(hit) = self.resolved.get(&key) {
            return hit.clone();
        }
        let hit = (self.ctx.resolve)(from, spec);
        self.resolved.insert(key, hit.clone());
        hit
    }

    fn link_import(&mut self, b: &Binding, chain: Vec<String>) -> Option<String> {
        let path = self.path.to_string();
        let mut file = self.res(&path, &b.spec)?;
        if file.starts_with("vendor:") {
            return Some(file);
        }
        let mut name = b.name.clone();
        let mut rest = std::collections::VecDeque::from(chain);
        for _ in 0..4 {
            if name == "*" {
                match rest.pop_front() {
                    None => return Some(file),
                    Some(next) => name = next,
                }
            }
            let t = self.follow(&file, &name, 0);
            if t.0.starts_with("vendor:") {
                return Some(t.0);
            }
            file = t.0;
            name = t.1;
            if name != "*" {
                break;
            }
        }
        Some(format!("{file}#{}", std::iter::once(name).chain(rest).collect::<Vec<_>>().join(".")))
    }

    /// Follow `name` exported by `file` through re-exports and `export *` to where it is defined: (file, name,
    /// found).
    fn follow(&mut self, file: &str, name: &str, depth: usize) -> (String, String, bool) {
        let rec = if depth > 8 { None } else { (self.ctx.record_of)(file) };
        let Some(rec) = rec else {
            return (file.into(), name.into(), false);
        };
        if let Some(e) = rec.exports.get(name) {
            return match e {
                ExportTarget::Local(l) => (file.into(), l.clone(), true),
                ExportTarget::From { spec, name: en } => match self.res(file, spec) {
                    None => (file.into(), name.into(), true),
                    Some(to) if to.starts_with("vendor:") || en == "*" => (to, en.clone(), true),
                    Some(to) => self.follow(&to, en, depth + 1),
                },
            };
        }
        for star in &rec.stars {
            let Some(to) = self.res(file, star).filter(|t| !t.starts_with("vendor:")) else { continue };
            let t = self.follow(&to, name, depth + 1);
            if t.2 {
                return t;
            }
        }
        (file.into(), name.into(), false)
    }

    /// `app.get('/x', h)` on an Express/Hono/Fastify/Koa-router instance; Fastify's `app.route({ method, url, handler })`.
    fn framework_route(&mut self, args: &[Argument<'a>], callee: U<'_, 'a>) {
        let U::M(MemberExpression::StaticMemberExpression(m)) = callee else { return };
        let Expression::Identifier(obj) = &m.object else { return };
        let Some(prefix) = self.receivers.get(obj.name.as_str()).cloned() else { return };
        let method = m.property.name.as_str();
        let enclosing = self.current.map(|c| self.symbols[c].id.clone());
        let handler = |e: Option<&Expression<'a>>| match e.map(unwrap) {
            Some(u) if u.func().is_some() || u.class().is_some() => Handler::Node(u.span()),
            Some(u) => u.ident().map_or(Handler::Other, |n| Handler::Ident(n.to_string())),
            None => Handler::Other,
        };
        if ROUTE_METHODS.contains(&method) && args.len() >= 2 {
            if let Some(path) = str_value(arg_expr(args.first())) {
                self.pending.push(PendingRoute {
                    method: method.to_string(),
                    path: prefix + &path,
                    handler: handler(arg_expr(args.last())),
                    enclosing,
                });
            }
        } else if method == "route" {
            let Some(obj) = arg_expr(args.first()).map(unwrap).and_then(U::object) else { return };
            let mut props: IndexMap<String, &Expression<'a>> = IndexMap::new();
            for p in &obj.properties {
                if let ObjectPropertyKind::ObjectProperty(p) = p {
                    if let Some(k) = key_name(&p.key, p.computed) {
                        props.insert(k, &p.value);
                    }
                }
            }
            let url = str_value(props.get("url").or_else(|| props.get("path")).copied());
            let m = props.get("method").copied();
            let methods: Vec<Option<String>> = match m.map(unwrap).and_then(U::expr) {
                Some(Expression::ArrayExpression(a)) => {
                    a.elements.iter().map(|e| str_value(e.as_expression())).collect()
                }
                _ => vec![str_value(m)],
            };
            if let Some(url) = url {
                for x in methods.into_iter().flatten() {
                    self.pending.push(PendingRoute {
                        method: x,
                        path: format!("{prefix}{url}"),
                        handler: handler(props.get("handler").copied()),
                        enclosing: enclosing.clone(),
                    });
                }
            }
        }
    }

    fn cb(&mut self, f: F<'_, 'a>) {
        self.cb_count += 1;
        let name = format!("<cb{}>", self.cb_count);
        self.func(&name, f, false, None, f.span(), Some(SymbolKind::Function));
    }
}

/// The spec of `require('x')`.
fn require_spec(c: &CallExpression<'_>) -> Option<String> {
    match &c.callee {
        Expression::Identifier(i) if i.name == "require" && c.arguments.len() == 1 => {
            str_value(arg_expr(c.arguments.first()))
        }
        _ => None,
    }
}

enum Root<'n> {
    This,
    Ident(&'n str),
}

fn kind_name(k: SymbolKind) -> &'static str {
    match k {
        SymbolKind::Function => "function",
        SymbolKind::Class => "class",
        SymbolKind::Method => "method",
        SymbolKind::Const => "const",
        SymbolKind::Type => "type",
        SymbolKind::Property => "property",
        SymbolKind::Component => "component",
    }
}

const FN_KINDS: [SymbolKind; 3] = [SymbolKind::Function, SymbolKind::Method, SymbolKind::Component];

impl<'a> Visit<'a> for Walker<'a, '_> {
    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        self.call(&it.callee, Some(&it.arguments));
        walk::walk_call_expression(self, it);
    }

    fn visit_new_expression(&mut self, it: &NewExpression<'a>) {
        self.call(&it.callee, None);
        walk::walk_new_expression(self, it);
    }

    // Top-level anonymous callbacks (route handlers, `createServer(…)`, `describe(…)`) get their own symbol.
    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        if self.current.is_none() {
            return self.cb(F::Arrow(it));
        }
        walk::walk_arrow_function_expression(self, it);
    }

    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        match it.r#type {
            FunctionType::FunctionExpression if self.current.is_none() => return self.cb(F::Fun(it)),
            // Named functions inside a function are symbols too (`startServer.startMap`), so a map can cite them;
            // calls made in them belong to them. Anonymous callbacks still fold into the function around them.
            FunctionType::FunctionDeclaration if self.current.is_some() && it.body.is_some() => {
                if let Some(id) = &it.id {
                    return self.func(id.name.as_str(), F::Fun(it), false, self.current, it.span, None);
                }
            }
            FunctionType::TSDeclareFunction => return,
            _ => {}
        }
        walk::walk_function(self, it, flags);
    }

    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        self.capture(it);
        if let (Some(cur), BindingPattern::BindingIdentifier(id)) = (self.current, &it.id) {
            if let Some(f) = unwrap_opt(it.init.as_ref()).and_then(U::func) {
                return self.func(id.name.as_str(), f, false, Some(cur), it.span, None);
            }
        }
        walk::walk_variable_declarator(self, it);
    }

    // A function returning an object of functions (a factory: `return { get() {}, save: () => {} }`) names them
    // `createStore.save`.
    fn visit_return_statement(&mut self, it: &ReturnStatement<'a>) {
        if let (Some(cur), Some(o)) = (self.current, unwrap_opt(it.argument.as_ref()).and_then(U::object)) {
            if FN_KINDS.contains(&self.symbols[cur].kind) {
                return self.members(o, cur, "");
            }
        }
        walk::walk_return_statement(self, it);
    }

    fn visit_import_declaration(&mut self, _: &ImportDeclaration<'a>) {}
    fn visit_ts_interface_declaration(&mut self, _: &TSInterfaceDeclaration<'a>) {}
    fn visit_ts_type_alias_declaration(&mut self, _: &TSTypeAliasDeclaration<'a>) {}
    // Types never hold calls the original sees: it skips `typeAnnotation`, `returnType`, `typeParameters`,
    // `typeArguments`, `superTypeArguments` and `implements` wherever they are.
    fn visit_ts_type_annotation(&mut self, _: &TSTypeAnnotation<'a>) {}
    fn visit_ts_type(&mut self, _: &TSType<'a>) {}
    fn visit_ts_type_parameter_declaration(&mut self, _: &TSTypeParameterDeclaration<'a>) {}
    fn visit_ts_type_parameter_instantiation(&mut self, _: &TSTypeParameterInstantiation<'a>) {}
    fn visit_ts_class_implements(&mut self, _: &TSClassImplements<'a>) {}
}
