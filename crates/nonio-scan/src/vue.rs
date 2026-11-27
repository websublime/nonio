//! Vue single-file components (D28): one oxc `Program` per `.vue` file, with every position an offset in the `.vue`.
//!
//! - **Script, in place.** The text of the `.vue` with everything outside the `<script>` and `<script setup>` blocks
//!   turned into spaces (line breaks kept) is parsed as one module. Offsets need no mapping table, and the bindings of
//!   `<script setup>` stay in the module scope, so `refresh` is the symbol `C.vue#refresh` as in a TS file.
//! - **Template, lowered to JSX.** The template is read with `vue-compiler-core` and lowered to a JSX fragment in a
//!   statement appended to the same `Program`, so oxc_semantic resolves its names against the script. `v-for` and
//!   `v-slot` open arrow functions whose parameters are the loop variables and slot props, and `v-if` sits outside the
//!   `v-for` of its element, as Vue 3 evaluates it. This follows the lowering of `vue_oxlint_jsx` 0.15
//!   (vue-oxlint-toolkit, MIT, Copyright (c) 2026-present Liang Mi and contributors, Copyright (c) 2025 zhiyuanzmj; see
//!   NOTICE), ported to oxc 0.153.
//!
//! Where it differs from `vue_oxlint_jsx`:
//! - Each template expression is parsed on its own and its spans moved to where it is in the `.vue`, instead of
//!   padding a copy of the whole file for every expression.
//! - `vue-compiler-core` counts positions in chars; they are turned into byte offsets here.
//! - The template is lowered only when there is a `<script setup>`. Without one (Options API), the template sees the
//!   component instance, not the module's bindings, so it is left empty rather than linked by name (D16).
//! - A handler given by reference (`@click="refresh"`, `@x="a.b"`) is lowered to a call, as Vue calls it on the event;
//!   a handler with several statements becomes an arrow with those statements.
//! - A `v-for` or `v-slot` whose variables don't parse drops its element, instead of leaving names unscoped.
//! - `v-for` is lowered to `[source, (vars) => <el/>]` rather than a call, so it makes no synthetic call.

use std::cell::Cell;
use std::sync::LazyLock;

use oxc_allocator::{Allocator, ArenaBox, ArenaVec};
use oxc_ast::ast::*;
use oxc_ast::builder::AstBuilder;
use oxc_ast_visit::VisitMut;
use oxc_parser::{Parser, ParserReturn};
use oxc_span::{SourceType, Span, SPAN};
use regex::Regex;
use vue_compiler_core::error::{CompilationError, CompilationErrorKind as Kind, ErrorHandler};
use vue_compiler_core::parser::{
    AstNode, AstRoot, DirectiveArg, ElemProp, Element, ElementType, ParseOption, Parser as HtmlParser,
    WhitespaceStrategy,
};
use vue_compiler_core::scanner::{ScanOption, Scanner, TextMode};
use vue_compiler_core::SourceLocation;

/// A `.vue` file read as one module.
pub struct Sfc<'a> {
    /// The parse of the script blocks, with the lowered template appended to `program.body`.
    pub ret: ParserReturn<'a>,
    /// The text the script was parsed from: the `.vue` with everything outside the `<script>` blocks blanked.
    pub script: &'a str,
    /// The `<template>` block (tags included), when there is one; the lowered template is the statement with this
    /// span.
    pub template: Option<Span>,
    /// Whether there is a `<script setup>` (only then does the template see the script's bindings).
    pub setup: bool,
}

/// Parse a `.vue` file. `None` when its markup doesn't parse (an unclosed tag, comment or interpolation).
pub fn parse_sfc<'a>(allocator: &'a Allocator, source: &'a str) -> Option<Sfc<'a>> {
    let root = html(source)?;
    let pos = Offsets::new(source);
    let mut blank: Vec<u8> = source.bytes().map(|b| if b == b'\n' || b == b'\r' { b } else { b' ' }).collect();
    let mut lang = None;
    let mut setup = false;
    let mut template = None;
    for node in &root.children {
        let AstNode::Element(e) = node else { continue };
        match e.tag_name {
            "script" if attr(e, "src").is_none() => {
                lang = lang.or(attr(e, "lang").flatten());
                setup |= attr(e, "setup").is_some();
                if let Some(text) = e.children.first() {
                    let (start, end) = pos.span(text.get_location());
                    blank[start..end].copy_from_slice(&source.as_bytes()[start..end]);
                }
            }
            "template" if template.is_none() => template = Some(e),
            _ => {}
        }
    }
    // Only ASCII spaces replaced whole UTF-8 sequences outside the blocks, and the blocks are copied on char
    // boundaries, so this is valid UTF-8.
    let script = allocator.alloc_str(std::str::from_utf8(&blank).ok()?);
    let source_type = match lang {
        Some("ts") => SourceType::ts(),
        Some("tsx") => SourceType::tsx(),
        Some("jsx") => SourceType::jsx(),
        _ => SourceType::mjs(),
    };
    let mut ret = Parser::new(allocator, script, source_type).parse();
    let template = template.map(|t| {
        let (start, end) = pos.span(&t.location);
        let span = Span::new(start as u32, end as u32);
        let b = AstBuilder::new(allocator);
        // A template in another language (pug) or from another file isn't read; neither is one that only sees the
        // component instance (no `<script setup>`).
        let readable = attr(t, "src").is_none() && attr(t, "lang").flatten().is_none_or(|l| l == "html");
        let lower = Lower { alloc: allocator, b: &b, source, pos: &pos, source_type };
        let children = if readable && setup { lower.children(&t.children) } else { ArenaVec::new_in(&b) };
        let fragment = Expression::new_jsx_fragment(
            SPAN,
            JSXOpeningFragment::new(SPAN, &b),
            children,
            JSXClosingFragment::new(SPAN, &b),
            &b,
        );
        ret.program.body.push(Statement::new_expression_statement(span, fragment, &b));
        span
    });
    Some(Sfc { ret, script, template, setup })
}

// ---- markup

/// Errors after which the tree can't be lowered (as `vue_oxlint_jsx` decides).
fn fatal(kind: &Kind) -> bool {
    matches!(
        kind,
        Kind::EofInTag
            | Kind::EofInComment
            | Kind::EofInCdata
            | Kind::EofBeforeTagName
            | Kind::EofInScriptHtmlCommentLikeText
            | Kind::MissingInterpolationEnd
            | Kind::MissingDynamicDirectiveArgumentEnd
            | Kind::MissingEndTag
            | Kind::UnexpectedNullCharacter
            | Kind::CDataInHtmlContent
    )
}

struct Errors<'f>(&'f Cell<bool>);

impl ErrorHandler for Errors<'_> {
    fn on_error(&self, e: CompilationError) {
        if fatal(&e.kind) {
            self.0.set(true);
        }
    }
}

fn text_mode(name: &str) -> TextMode {
    match name {
        "textarea" => TextMode::RcData,
        "iframe" | "xmp" | "noembed" | "noframes" | "noscript" | "script" | "style" => TextMode::RawText,
        _ => TextMode::Data,
    }
}

fn is_void_tag(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn html(source: &str) -> Option<AstRoot<'_>> {
    let broken = Cell::new(false);
    let scanner = Scanner::new(ScanOption { get_text_mode: text_mode, ..Default::default() });
    let parser = HtmlParser::new(ParseOption {
        whitespace: WhitespaceStrategy::Preserve,
        is_void_tag,
        get_text_mode: text_mode,
        ..Default::default()
    });
    let tokens = scanner.scan(source, Errors(&broken));
    let root = parser.parse(tokens, Errors(&broken));
    (!broken.get()).then_some(root)
}

/// A static attribute: `None` if absent, `Some(None)` if it has no value.
fn attr<'s>(e: &Element<'s>, name: &str) -> Option<Option<&'s str>> {
    e.properties.iter().find_map(|p| match p {
        ElemProp::Attr(a) if a.name == name => Some(a.value.as_ref().map(|v| v.content.raw)),
        _ => None,
    })
}

/// Char offsets (what `vue-compiler-core` counts) to byte offsets.
struct Offsets {
    /// Byte offset of each char, plus the length; empty when the text is ASCII (offsets are the same).
    bytes: Vec<usize>,
}

impl Offsets {
    fn new(source: &str) -> Self {
        let bytes = if source.is_ascii() {
            Vec::new()
        } else {
            source.char_indices().map(|(i, _)| i).chain(std::iter::once(source.len())).collect()
        };
        Self { bytes }
    }

    fn byte(&self, char_offset: usize) -> usize {
        if self.bytes.is_empty() {
            char_offset
        } else {
            self.bytes[char_offset.min(self.bytes.len() - 1)]
        }
    }

    fn span(&self, l: &SourceLocation) -> (usize, usize) {
        (self.byte(l.start.offset), self.byte(l.end.offset))
    }
}

// ---- template

/// `alias in source` / `alias of source`, as Vue's compiler splits a `v-for`.
static FOR_ALIAS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([\s\S]*?)\s+(?:in|of)\s+(\S[\s\S]*)").unwrap());

/// Moves every span of a fragment parsed on its own by `.0` bytes, to where the fragment is in the `.vue`.
struct Shift(i64);

impl<'a> VisitMut<'a> for Shift {
    fn visit_span(&mut self, it: &mut Span) {
        let at = |x: u32| (i64::from(x) + self.0).max(0) as u32;
        *it = Span::new(at(it.start), at(it.end));
    }
}

struct Lower<'a, 'b, 's> {
    alloc: &'a Allocator,
    b: &'b AstBuilder<'a>,
    source: &'s str,
    pos: &'b Offsets,
    source_type: SourceType,
}

impl<'a, 's> Lower<'a, '_, 's> {
    /// Byte offset of `text` in the `.vue`, when it is a slice of it.
    fn at(&self, text: &str) -> Option<i64> {
        let base = self.source.as_ptr() as usize;
        let p = text.as_ptr() as usize;
        (p >= base && p + text.len() <= base + self.source.len()).then(|| (p - base) as i64)
    }

    /// Parse `pre + text + post` as an expression, with spans placed so that `text` is where it is in the `.vue`.
    fn parse_expr(&self, text: &str, pre: &str, post: &str) -> Option<Expression<'a>> {
        let at = self.at(text)?;
        let src = self.alloc.alloc_str(&format!("{pre}{text}{post}"));
        let mut e = Parser::new(self.alloc, src, self.source_type).parse_expression().ok()?;
        Shift(at - pre.len() as i64).visit_expression(&mut e);
        Some(e)
    }

    fn expr(&self, text: &str) -> Option<Expression<'a>> {
        if text.trim().is_empty() {
            return None;
        }
        self.parse_expr(text, "", "")
    }

    /// The parameters of `(text) => 0`: `v-for` variables, slot props.
    fn params(&self, text: &str) -> Option<ArenaBox<'a, FormalParameters<'a>>> {
        let t = text.trim();
        let e = if t.starts_with('(') && t.ends_with(')') {
            self.parse_expr(text, "", "=>0")
        } else {
            self.parse_expr(text, "(", ")=>0")
        }?;
        match e {
            Expression::ArrowFunctionExpression(a) => Some(a.unbox().params),
            _ => None,
        }
    }

    fn no_params(&self) -> ArenaBox<'a, FormalParameters<'a>> {
        FormalParameters::boxed(
            SPAN,
            FormalParameterKind::ArrowFormalParameters,
            ArenaVec::new_in(self.b),
            None,
            self.b,
        )
    }

    /// `@event="…"`: a handler by reference is called; an inline expression is kept; several statements become the
    /// body of an arrow.
    fn handler(&self, text: &str) -> Option<Expression<'a>> {
        if text.trim().is_empty() {
            return None;
        }
        if let Some(e) = self.expr(text) {
            return Some(if is_path(&e) {
                Expression::new_call_expression(SPAN, e, None, ArenaVec::new_in(self.b), false, self.b)
            } else {
                e
            });
        }
        let at = self.at(text)?;
        let src = self.alloc.alloc_str(text);
        let ret = Parser::new(self.alloc, src, self.source_type).parse();
        if ret.fatal_error || ret.diagnostics.has_errors() {
            return None;
        }
        let mut body = ret.program.body;
        for s in body.iter_mut() {
            Shift(at).visit_statement(s);
        }
        let body = ArrowFunctionBody::new_function_body(SPAN, ArenaVec::new_in(self.b), body, self.b);
        Some(Expression::new_arrow_function_expression(SPAN, false, None, self.no_params(), None, body, self.b))
    }

    fn container(&self, e: Expression<'a>) -> JSXChild<'a> {
        JSXChild::new_expression_container(SPAN, JSXExpression::from(e), self.b)
    }

    fn children(&self, nodes: &[AstNode<'s>]) -> ArenaVec<'a, JSXChild<'a>> {
        let mut out = ArenaVec::new_in(self.b);
        for n in nodes {
            match n {
                AstNode::Element(e) => {
                    let (test, child) = self.element(e);
                    if let Some(t) = test {
                        out.push(self.container(t));
                    }
                    if let Some(c) = child {
                        out.push(c);
                    }
                }
                AstNode::Interpolation(i) => {
                    if let Some(e) = self.expr(i.source) {
                        out.push(self.container(e));
                    }
                }
                AstNode::Text(_) | AstNode::Comment(_) => {}
            }
        }
        out
    }

    /// An element: its `v-if`/`v-else-if` test (evaluated outside its `v-for`), and the element, unless a scope it
    /// opens doesn't parse.
    fn element(&self, el: &Element<'s>) -> (Option<Expression<'a>>, Option<JSXChild<'a>>) {
        let b = self.b;
        let name = el.tag_name;
        let name_span = self.at(name).map_or(SPAN, |s| Span::sized(s as u32, name.len() as u32));
        let reference = el.tag_type == ElementType::Component || (name.contains('-') && !name.contains('.'));
        let jsx_name = if reference {
            JSXElementName::new_identifier_reference(name_span, Ident::from(self.alloc.alloc_str(&pascal(name))), b)
        } else {
            JSXElementName::new_identifier(name_span, Str::from(self.alloc.alloc_str(name)), b)
        };

        let mut test = None;
        let mut v_for: Option<Option<(Expression<'a>, ArenaBox<'a, FormalParameters<'a>>)>> = None;
        let mut v_slot: Option<Option<ArenaBox<'a, FormalParameters<'a>>>> = None;
        let mut pre = false;
        let mut attrs = ArenaVec::new_in(b);
        for p in &el.properties {
            let ElemProp::Dir(d) = p else { continue };
            let value = d.expression.as_ref().map(|v| v.content.raw).filter(|t| !t.trim().is_empty());
            let mut exprs: Vec<Expression<'a>> = Vec::new();
            if let Some(DirectiveArg::Dynamic(arg)) = &d.argument {
                exprs.extend(self.expr(arg));
            }
            match d.name {
                "for" => v_for = Some(value.and_then(|v| self.v_for(v))),
                "slot" => v_slot = Some(value.map_or_else(|| Some(self.no_params()), |v| self.params(v))),
                "if" | "else-if" => test = value.and_then(|v| self.expr(v)),
                "pre" => pre = true,
                "on" => exprs.extend(value.and_then(|v| self.handler(v))),
                // `:msg-id` with no value is `:msg-id="msgId"` (Vue 3.4).
                "bind" if value.is_none() => {
                    if let Some(DirectiveArg::Static(arg)) = &d.argument {
                        let id = self.alloc.alloc_str(&camel(arg));
                        exprs.push(Expression::new_identifier(SPAN, Ident::from(id), b));
                    }
                }
                _ => exprs.extend(value.and_then(|v| self.expr(v))),
            }
            let (hs, he) = self.pos.span(&d.head_loc);
            let head = self.alloc.alloc_str(self.source.get(hs..he).unwrap_or(""));
            for e in exprs {
                let attr_name = JSXAttributeName::new_identifier(Span::new(hs as u32, he as u32), Str::from(head), b);
                let value = JSXAttributeValue::new_expression_container(SPAN, JSXExpression::from(e), b);
                attrs.push(JSXAttributeItem::new_attribute(SPAN, attr_name, Some(value), b));
            }
        }

        let mut children = if pre { ArenaVec::new_in(b) } else { self.children(&el.children) };
        match v_slot {
            Some(None) => return (test, None),
            Some(Some(params)) => {
                let body = ArrowFunctionBody::new_jsx_fragment(
                    SPAN,
                    JSXOpeningFragment::new(SPAN, b),
                    children,
                    JSXClosingFragment::new(SPAN, b),
                    b,
                );
                let arrow = Expression::new_arrow_function_expression(SPAN, false, None, params, None, body, b);
                children = ArenaVec::from_array_in([self.container(arrow)], b);
            }
            None => {}
        }
        let (s, e) = self.pos.span(&el.location);
        let opening = JSXOpeningElement::boxed(SPAN, jsx_name, None, attrs, b);
        let element = JSXElement::boxed(Span::new(s as u32, e as u32), opening, children, None, b);
        let child = match v_for {
            None => JSXChild::Element(element),
            Some(None) => return (test, None),
            Some(Some((source, params))) => {
                let body = ArrowFunctionBody::JSXElement(element);
                let arrow =
                    ArrayExpressionElement::new_arrow_function_expression(SPAN, false, None, params, None, body, b);
                let list = ArenaVec::from_array_in([ArrayExpressionElement::from(source), arrow], b);
                self.container(Expression::new_array_expression(SPAN, list, b))
            }
        };
        (test, Some(child))
    }

    /// `v-for="(item, i) in list"`: the list and the variables.
    fn v_for(&self, text: &'s str) -> Option<(Expression<'a>, ArenaBox<'a, FormalParameters<'a>>)> {
        let caps = FOR_ALIAS.captures(text)?;
        let source = self.expr(caps.get(2)?.as_str())?;
        let params = self.params(caps.get(1)?.as_str())?;
        Some((source, params))
    }
}

/// `a`, `a.b`, `a[0].c`: what Vue calls as a handler.
fn is_path(e: &Expression<'_>) -> bool {
    match e {
        Expression::Identifier(_) => true,
        Expression::StaticMemberExpression(m) => is_path(&m.object),
        Expression::ComputedMemberExpression(m) => is_path(&m.object),
        _ => false,
    }
}

/// `keep-alive` → `KeepAlive`.
fn pascal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut up = true;
    for c in s.chars() {
        if c == '-' {
            up = true;
        } else if up {
            out.extend(c.to_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// `msg-id` → `msgId`.
fn camel(s: &str) -> String {
    let p = pascal(s);
    let mut c = p.chars();
    c.next().map_or_else(String::new, |f| f.to_lowercase().chain(c).collect())
}
