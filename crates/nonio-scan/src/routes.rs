//! Route paths and file-based routes (Next.js). Ported from `facts/routes.ts` of illuminate (MIT, Copyright (c) 2026
//! johnz4021).

use std::sync::LazyLock;

use fancy_regex::Regex;
use nonio_facts::RouteFact;

use crate::re::{group, js, test};

pub const HTTP_METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

fn replace(re: &Regex, text: &str, with: &str) -> String {
    re.replace_all(text, with).into_owned()
}

static ESCAPED_SLASH: LazyLock<Regex> = LazyLock::new(|| js(r"\\/"));
static GROUP: LazyLock<Regex> = LazyLock::new(|| js(r"\((?:[^()\\]|\\.)*\)\??"));
static BRACKET: LazyLock<Regex> = LazyLock::new(|| js(r"\[\[?\.{0,3}[^\]/]*\]?\]"));
static COLON: LazyLock<Regex> = LazyLock::new(|| js(r":[A-Za-z_$][\w$]*\??"));
static BRACE: LazyLock<Regex> = LazyLock::new(|| js(r"\{[^}/]+\}"));
static BRACE_REPEAT: LazyLock<Regex> = LazyLock::new(|| js(r"\{\}[+*?]+"));
static STAR: LazyLock<Regex> = LazyLock::new(|| js(r"(^|/)\*+(?=/|$)"));
static ESCAPE: LazyLock<Regex> = LazyLock::new(|| js(r"\\(.)"));
static SLASHES: LazyLock<Regex> = LazyLock::new(|| js(r"/+"));

/// Normalize a route path so ids are stable: params (`:id`, `{id}`, `[id]`, `[...x]`, regex groups `([\w-]+)`)
/// become `{}`, regex anchors and escapes are dropped, and a leading `/` is ensured.
pub fn normalize_route_path(raw: &str) -> String {
    let mut p = raw.trim();
    p = p.strip_prefix('^').unwrap_or(p);
    p = p.strip_suffix('$').unwrap_or(p);
    let mut p = replace(&ESCAPED_SLASH, p, "/");
    p = replace(&GROUP, &p, "{}");
    p = replace(&BRACKET, &p, "{}");
    p = replace(&COLON, &p, "{}");
    p = replace(&BRACE, &p, "{}");
    p = replace(&BRACE_REPEAT, &p, "{}");
    p = replace(&STAR, &p, "${1}{}");
    p = replace(&ESCAPE, &p, "${1}");
    p = replace(&SLASHES, &p, "/");
    if !p.starts_with('/') {
        p.insert(0, '/');
    }
    if p.len() > 1 && p.ends_with('/') {
        p.pop();
    }
    p
}

pub fn route_id(method: &str, path: &str) -> String {
    format!("route:{method} {path}")
}

pub fn route_fact(method: &str, raw_path: &str, file: &str, handler: Option<String>) -> RouteFact {
    let up = method.to_uppercase();
    let m = if up == "ALL" { "ANY".to_string() } else { up };
    let path = normalize_route_path(raw_path);
    RouteFact { id: route_id(&m, &path), method: m, path, file: file.to_string(), handler }
}

static NEXT_APP_ROUTE: LazyLock<Regex> = LazyLock::new(|| js(r"(?:^|/)app/((?:.*/)?)route\.[cm]?[jt]sx?$"));
static NEXT_APP_PAGE: LazyLock<Regex> = LazyLock::new(|| js(r"(?:^|/)app/((?:.*/)?)page\.[cm]?[jt]sx?$"));
static NEXT_PAGES: LazyLock<Regex> = LazyLock::new(|| js(r"(?:^|/)pages/(.*)\.[cm]?[jt]sx?$"));
static INDEX: LazyLock<Regex> = LazyLock::new(|| js(r"(?:^|/)index$"));
static API: LazyLock<Regex> = LazyLock::new(|| js(r"^api(?:/|$)"));
static SPECIAL: LazyLock<Regex> = LazyLock::new(|| js(r"(?:^|/)_(?:app|document|error)$"));

/// URL path for a Next.js segment list: route groups `(x)` and slots `@x` drop out.
fn segments_to_path(dir: &str) -> String {
    let segs: Vec<&str> = dir
        .split('/')
        .filter(|s| !s.is_empty() && !(s.starts_with('(') && s.ends_with(')') && s.len() >= 2) && !s.starts_with('@'))
        .collect();
    normalize_route_path(&format!("/{}", segs.join("/")))
}

/// A file-based route: method, URL path and the name of the export that handles it.
pub struct NextRoute {
    pub method: String,
    pub path: String,
    pub symbol: String,
}

/// Next.js file-based routes and the page for `path`, given its export names.
pub fn next_routes(path: &str, export_names: &[&str]) -> (Vec<NextRoute>, Option<String>) {
    let cap = |re: &Regex| re.captures(path).ok().flatten().and_then(|c| group(&c, 1).map(str::to_string));
    if let Some(dir) = cap(&NEXT_APP_ROUTE) {
        let url = segments_to_path(&dir);
        let routes = export_names
            .iter()
            .filter(|n| HTTP_METHODS.contains(n))
            .map(|n| NextRoute { method: n.to_string(), path: url.clone(), symbol: n.to_string() })
            .collect();
        return (routes, None);
    }
    if let Some(dir) = cap(&NEXT_APP_PAGE) {
        return (Vec::new(), Some(segments_to_path(&dir)));
    }
    if let Some(rest) = cap(&NEXT_PAGES) {
        let rest = INDEX.replace(&rest, "").into_owned();
        if test(&API, &rest) {
            let routes = if export_names.contains(&"default") {
                vec![NextRoute { method: "ANY".into(), path: segments_to_path(&rest), symbol: "default".into() }]
            } else {
                Vec::new()
            };
            return (routes, None);
        }
        if test(&SPECIAL, &rest) || rest.starts_with('_') {
            return (Vec::new(), None);
        }
        return (Vec::new(), Some(segments_to_path(&rest)));
    }
    (Vec::new(), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_route_params() {
        let got: Vec<String> = ["/a/:id", "/a/{id}/b", "/a/[id]", "/a/[...slug]", r"^\/a\/([\w-]+)$", "/a/*", "a/"]
            .iter()
            .map(|p| normalize_route_path(p))
            .collect();
        assert_eq!(got, ["/a/{}", "/a/{}/b", "/a/{}", "/a/{}", "/a/{}", "/a/{}", "/a"]);
    }
}
