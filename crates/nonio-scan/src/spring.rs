//! Spring MVC routes in Java and Kotlin, read from the annotations (D17, engine "annotation or decorator").
//!
//! A route is made only from what the annotations say in literal text, and only on a type Spring would register as
//! a handler:
//!
//! - the class carries `@RestController` or `@Controller` (a class with only `@RequestMapping` is not a handler
//!   since Spring 6; custom stereotypes that wrap `@RestController` are not followed);
//! - the class `@RequestMapping` gives the prefixes (none: `""`), each method `@GetMapping`, `@PostMapping`,
//!   `@PutMapping`, `@DeleteMapping`, `@PatchMapping` or `@RequestMapping` the paths (none: `""`) and the methods
//!   (`@RequestMapping` without `method` is `ANY`); every prefix is joined to every path;
//! - a path is a string literal, an array of them, or literals joined with `+`. Anything else (a constant, a
//!   template with `$`, `arrayOf(…)`) makes no route at all, on the class or the method where it is written: a
//!   route that may be wrong is worse than a missing one;
//! - mappings inherited from an interface or a superclass are not followed.
//!
//! The handler is the method's symbol, the innermost [`SymbolFact`] around its name.

use nonio_facts::{RouteFact, SymbolFact, SymbolKind};
use tree_sitter::{Node, Parser};

use crate::routes::route_fact;

/// A value written in an annotation argument.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Value {
    /// One or more string literals (an array of them, or literals joined with `+`).
    Strings(Vec<String>),
    /// Enum constants by their last name (`RequestMethod.POST` → `POST`).
    Names(Vec<String>),
    /// Anything else.
    Unknown,
}

struct Annotation {
    name: String,
    /// `(key, value)`; `None` for a positional argument.
    args: Vec<(Option<String>, Value)>,
}

impl Annotation {
    fn arg(&self, keys: &[&str], positional: bool) -> Option<&Value> {
        self.args
            .iter()
            .find(|(k, _)| match k {
                Some(k) => keys.contains(&k.as_str()),
                None => positional,
            })
            .map(|(_, v)| v)
    }

    /// The paths: `value`, `path` or the positional argument. `None` when one is not literal.
    fn paths(&self) -> Option<Vec<String>> {
        match self.arg(&["value", "path"], true) {
            None => Some(vec![String::new()]),
            Some(Value::Strings(s)) if s.is_empty() => Some(vec![String::new()]),
            Some(Value::Strings(s)) => Some(s.clone()),
            Some(_) => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lang {
    Java,
    Kotlin,
}

/// Whether `path` is a file whose Spring routes [`spring_routes`] reads.
pub fn is_spring_file(path: &str) -> bool {
    path.ends_with(".java") || path.ends_with(".kt")
}

/// The Spring routes declared in one Java or Kotlin file, with handlers named from `symbols` (that file's facts).
pub fn spring_routes(path: &str, text: &str, symbols: &[SymbolFact]) -> Vec<RouteFact> {
    if !is_spring_file(path) || !text.contains("Controller") || !text.contains("Mapping") {
        return Vec::new();
    }
    let lang = if path.ends_with(".java") { Lang::Java } else { Lang::Kotlin };
    let mut parser = Parser::new();
    let language = match lang {
        Lang::Java => tree_sitter_java::LANGUAGE.into(),
        Lang::Kotlin => tree_sitter_kotlin_ng::LANGUAGE.into(),
    };
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(text, None) else {
        return Vec::new();
    };
    let r = Reader { lang, text, path, symbols };
    let mut out = Vec::new();
    r.walk(tree.root_node(), &mut out);
    let mut seen = std::collections::HashSet::new();
    out.retain(|f: &RouteFact| seen.insert((f.id.clone(), f.handler.clone())));
    out
}

struct Reader<'a> {
    lang: Lang,
    text: &'a str,
    path: &'a str,
    symbols: &'a [SymbolFact],
}

const MAPPINGS: [(&str, Option<&str>); 6] = [
    ("GetMapping", Some("GET")),
    ("PostMapping", Some("POST")),
    ("PutMapping", Some("PUT")),
    ("DeleteMapping", Some("DELETE")),
    ("PatchMapping", Some("PATCH")),
    ("RequestMapping", None),
];

const HTTP: [&str; 8] = ["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS", "TRACE"];

impl<'a> Reader<'a> {
    fn src(&self, n: Node<'_>) -> &'a str {
        &self.text[n.byte_range()]
    }

    fn children<'t>(n: Node<'t>) -> Vec<Node<'t>> {
        let mut c = n.walk();
        n.children(&mut c).collect()
    }

    fn walk(&self, n: Node<'_>, out: &mut Vec<RouteFact>) {
        if n.kind() == "class_declaration" {
            self.class(n, out);
        }
        for c in Self::children(n) {
            self.walk(c, out);
        }
    }

    fn annotations(&self, decl: Node<'_>) -> Vec<Annotation> {
        let Some(mods) = Self::children(decl).into_iter().find(|c| c.kind() == "modifiers") else {
            return Vec::new();
        };
        Self::children(mods).into_iter().filter_map(|a| self.annotation(a)).collect()
    }

    fn annotation(&self, a: Node<'_>) -> Option<Annotation> {
        match (self.lang, a.kind()) {
            (Lang::Java, "marker_annotation") => {
                Some(Annotation { name: last_name(self.src(a.child_by_field_name("name")?)), args: Vec::new() })
            }
            (Lang::Java, "annotation") => {
                let name = last_name(self.src(a.child_by_field_name("name")?));
                let mut args = Vec::new();
                if let Some(list) = a.child_by_field_name("arguments") {
                    for arg in Self::children(list).into_iter().filter(Node::is_named) {
                        if arg.kind() == "element_value_pair" {
                            let key = arg.child_by_field_name("key").map(|k| self.src(k).to_string());
                            let value = arg.child_by_field_name("value").map_or(Value::Unknown, |v| self.value(v));
                            args.push((key, value));
                        } else {
                            args.push((None, self.value(arg)));
                        }
                    }
                }
                Some(Annotation { name, args })
            }
            (Lang::Kotlin, "annotation") => {
                let inner = Self::children(a).into_iter().find(Node::is_named)?;
                match inner.kind() {
                    "user_type" => Some(Annotation { name: last_name(self.src(inner)), args: Vec::new() }),
                    "constructor_invocation" => {
                        let parts = Self::children(inner);
                        let ty = parts.iter().find(|c| c.kind() == "user_type")?;
                        let mut args = Vec::new();
                        if let Some(list) = parts.iter().find(|c| c.kind() == "value_arguments") {
                            for arg in Self::children(*list).into_iter().filter(|c| c.kind() == "value_argument") {
                                let named: Vec<Node<'_>> = Self::children(arg);
                                let has_key = named.len() >= 3 && named[1].kind() == "=";
                                let (key, value) = if has_key {
                                    (Some(self.src(named[0]).to_string()), named.get(2).copied())
                                } else {
                                    (None, named.iter().copied().find(Node::is_named))
                                };
                                if named.iter().any(|c| c.kind() == "*") {
                                    args.push((key, Value::Unknown)); // spread
                                    continue;
                                }
                                args.push((key, value.map_or(Value::Unknown, |v| self.value(v))));
                            }
                        }
                        Some(Annotation { name: last_name(self.src(*ty)), args })
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn value(&self, v: Node<'_>) -> Value {
        match v.kind() {
            "string_literal" => self.string(v).map_or(Value::Unknown, |s| Value::Strings(vec![s])),
            "binary_expression" | "additive_expression" => {
                self.concat(v).map_or(Value::Unknown, |s| Value::Strings(vec![s]))
            }
            "element_value_array_initializer" | "collection_literal" => {
                let items: Vec<Value> =
                    Self::children(v).into_iter().filter(Node::is_named).map(|c| self.value(c)).collect();
                let mut strings = Vec::new();
                let mut names = Vec::new();
                for i in items {
                    match i {
                        Value::Strings(s) => strings.extend(s),
                        Value::Names(n) => names.extend(n),
                        Value::Unknown => return Value::Unknown,
                    }
                }
                match (strings.is_empty(), names.is_empty()) {
                    (_, true) => Value::Strings(strings),
                    (true, false) => Value::Names(names),
                    _ => Value::Unknown,
                }
            }
            "field_access" | "navigation_expression" | "identifier" | "simple_identifier" => {
                Value::Names(vec![last_name(self.src(v))])
            }
            "parenthesized_expression" => {
                Self::children(v).into_iter().find(Node::is_named).map_or(Value::Unknown, |c| self.value(c))
            }
            _ => Value::Unknown,
        }
    }

    /// A string literal with no escapes or templates.
    fn string(&self, v: Node<'_>) -> Option<String> {
        let mut s = String::new();
        for c in Self::children(v) {
            match c.kind() {
                "\"" => {}
                "string_fragment" | "string_content" => s.push_str(self.src(c)),
                _ => return None, // escape sequences, `${…}`, text blocks
            }
        }
        // kotlin-ng keeps a bare `$name` template inside the string content.
        (self.lang == Lang::Java || !s.contains('$')).then_some(s)
    }

    /// Literals joined with `+`.
    fn concat(&self, v: Node<'_>) -> Option<String> {
        match v.kind() {
            "string_literal" => self.string(v),
            "parenthesized_expression" => self.concat(Self::children(v).into_iter().find(Node::is_named)?),
            "binary_expression" | "additive_expression" => {
                let parts = Self::children(v);
                let op = parts.iter().find(|c| !c.is_named())?;
                if self.src(*op) != "+" {
                    return None;
                }
                let operands: Vec<Node<'_>> = parts.into_iter().filter(Node::is_named).collect();
                let [l, r] = operands.as_slice() else { return None };
                Some(self.concat(*l)? + &self.concat(*r)?)
            }
            _ => None,
        }
    }

    fn class(&self, class: Node<'_>, out: &mut Vec<RouteFact>) {
        let anns = self.annotations(class);
        if !anns.iter().any(|a| a.name == "RestController" || a.name == "Controller") {
            return;
        }
        let prefixes = match anns.iter().find(|a| a.name == "RequestMapping") {
            Some(a) => match a.paths() {
                Some(p) => p,
                None => return,
            },
            None => vec![String::new()],
        };
        let Some(body) = Self::children(class).into_iter().find(|c| c.kind() == "class_body") else {
            return;
        };
        let method_kind = match self.lang {
            Lang::Java => "method_declaration",
            Lang::Kotlin => "function_declaration",
        };
        for m in Self::children(body).into_iter().filter(|c| c.kind() == method_kind) {
            let Some(name) = Self::children(m).into_iter().find(|c| c.kind() == "identifier") else {
                continue;
            };
            let handler = self.handler(name.start_byte());
            for a in self.annotations(m) {
                let Some((_, fixed)) = MAPPINGS.iter().find(|(n, _)| *n == a.name) else {
                    continue;
                };
                let Some(paths) = a.paths() else {
                    continue;
                };
                let methods: Vec<String> = match fixed {
                    Some(m) => vec![(*m).to_string()],
                    None => match a.arg(&["method"], false) {
                        None => vec!["ANY".into()],
                        Some(Value::Names(n)) if !n.is_empty() && n.iter().all(|x| HTTP.contains(&x.as_str())) => {
                            n.clone()
                        }
                        Some(_) => continue,
                    },
                };
                for p in &prefixes {
                    for q in &paths {
                        for method in &methods {
                            out.push(route_fact(method, &join(p, q), self.path, handler.clone()));
                        }
                    }
                }
            }
        }
    }

    /// The innermost method or function symbol around `at`.
    fn handler(&self, at: usize) -> Option<String> {
        self.symbols
            .iter()
            .filter(|s| matches!(s.kind, SymbolKind::Method | SymbolKind::Function))
            .filter(|s| s.span.start <= at && at < s.span.end)
            .min_by_key(|s| s.span.end - s.span.start)
            .map(|s| s.id.clone())
    }
}

/// The last dotted name: `org.x.GetMapping` → `GetMapping`.
fn last_name(s: &str) -> String {
    s.rsplit('.').next().unwrap_or(s).trim().to_string()
}

/// A class prefix and a method path, the way Spring joins them (a `/` between, never two).
fn join(prefix: &str, path: &str) -> String {
    match (prefix.is_empty(), path.is_empty()) {
        (true, true) => "/".into(),
        (true, false) => path.into(),
        (false, true) => prefix.into(),
        (false, false) => format!("{}/{}", prefix.trim_end_matches('/'), path.trim_start_matches('/')),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nonio_facts::Span;

    fn sym(file: &str, name: &str, text: &str, needle: &str) -> SymbolFact {
        let start = text.find(needle).unwrap();
        SymbolFact {
            id: format!("{file}#{name}"),
            file: file.into(),
            name: name.into(),
            kind: SymbolKind::Method,
            exported: true,
            parent: None,
            span: Span { start, end: start + needle.len() },
            sig_hash: String::new(),
            body_hash: String::new(),
        }
    }

    fn ids(routes: &[RouteFact]) -> Vec<(String, Option<String>)> {
        routes.iter().map(|r| (r.id.clone(), r.handler.clone())).collect()
    }

    #[test]
    fn reads_java_mappings() {
        let f = "A.java";
        let t = r#"
@RestController
@RequestMapping(value = {"/a", "/b/"})
class A {
  @GetMapping(path = "/x/{id}", produces = "json")
  public String x() { return ""; }
  @RequestMapping(value = "y", method = {RequestMethod.POST, RequestMethod.PUT})
  void y() {}
  @org.springframework.web.bind.annotation.DeleteMapping("/z" + "/{n:\\d+}") void z() {}
  @GetMapping(C.P) void w() {}
  @PostMapping void v() {}
  void plain() {}
}
"#;
        let symbols = [
            sym(f, "A.x", t, "public String x() { return \"\"; }"),
            sym(f, "A.y", t, "void y() {}"),
            sym(f, "A.z", t, "void z() {}"),
            sym(f, "A.w", t, "void w() {}"),
            sym(f, "A.v", t, "void v() {}"),
        ];
        let got = ids(&spring_routes(f, t, &symbols));
        let h = |s: &str| Some(format!("A.java#A.{s}"));
        assert_eq!(
            got,
            vec![
                ("route:GET /a/x/{}".into(), h("x")),
                ("route:GET /b/x/{}".into(), h("x")),
                ("route:POST /a/y".into(), h("y")),
                ("route:PUT /a/y".into(), h("y")),
                ("route:POST /b/y".into(), h("y")),
                ("route:PUT /b/y".into(), h("y")),
                ("route:POST /a".into(), h("v")),
                ("route:POST /b".into(), h("v")),
            ],
            "the `+` with an escape and the constant make no route"
        );
    }

    #[test]
    fn reads_kotlin_mappings() {
        let f = "A.kt";
        let t = r#"
@RestController
@RequestMapping(value = ["/a"])
class A(private val s: S) {
  @GetMapping(path = ["/x/{id}"], produces = ["json"])
  fun x(): String = ""
  @RequestMapping("/y", method = [RequestMethod.POST])
  fun y() {}
  @RequestMapping("/any") fun any() {}
  @DeleteMapping("/$base/z") fun z() {}
}
"#;
        let symbols = [
            sym(f, "A.x", t, "fun x(): String = \"\""),
            sym(f, "A.y", t, "fun y() {}"),
            sym(f, "A.any", t, "fun any() {}"),
        ];
        let got = ids(&spring_routes(f, t, &symbols));
        let h = |s: &str| Some(format!("A.kt#A.{s}"));
        assert_eq!(
            got,
            vec![
                ("route:GET /a/x/{}".into(), h("x")),
                ("route:POST /a/y".into(), h("y")),
                ("route:ANY /a/any".into(), h("any")),
            ]
        );
    }

    #[test]
    fn needs_a_controller_and_a_literal_prefix() {
        let t = "@RequestMapping(\"/a\")\nclass A { @GetMapping(\"/x\") void x() {} }\n";
        assert!(spring_routes("A.java", t, &[]).is_empty(), "no stereotype");
        let t = "@RestController\n@RequestMapping(Paths.A)\nclass A { @GetMapping(\"/x\") void x() {} }\n";
        assert!(spring_routes("A.java", t, &[]).is_empty(), "prefix is a constant");
        let t = "@Controller\nclass A { @GetMapping void x() {} }\n";
        assert_eq!(ids(&spring_routes("A.java", t, &[])), vec![("route:GET /".to_string(), None)]);
    }
}
