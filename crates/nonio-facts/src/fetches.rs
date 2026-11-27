//! UI → API links: `fetch` call sites (direct, Nuxt's `$fetch`, `useFetch` and `useLazyFetch`, through a small
//! wrapper such as `request(path)`) with their URL literal, matched to route facts. Pure: it scans
//! source text, using the file's symbol spans only to name the enclosing symbol.
//!
//! Ported from `packages/facts/src/journeys/fetches.ts` of illuminate (MIT, Copyright (c) 2026 johnz4021).
//! Offsets are UTF-8 byte offsets, like [`crate::Span`].

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;

use crate::contract::{ClientFact, FileFacts, RouteFact, SymbolFact, SymbolKind};

/// A call that sends an HTTP request to a URL known (at least in part) from the source text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchSite {
    pub file: String,
    /// Enclosing symbol id, or the file path for top-level code.
    pub from: String,
    /// Upper case; GET when the call doesn't say.
    pub method: String,
    /// Path only, query dropped, `${…}` → `{}`.
    pub url: String,
    /// The whole URL is literal text at the call (no template holes, no wrapper prefix).
    pub exact: bool,
    /// Wrapper or client the call goes through (`request`, `vendor:axios`, `src/http.ts#api`), `None` for a direct
    /// fetch.
    pub via: Option<String>,
}

/// A source file with its symbols, as [`find_fetch_sites`] reads it.
#[derive(Debug, Clone, Copy)]
pub struct SourceFile<'a> {
    pub path: &'a str,
    pub source: &'a str,
    pub symbols: &'a [SymbolFact],
}

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("valid regex"))
}

fn client_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r"(fetch|\$fetch|ofetch|useFetch|useLazyFetch)\s*(?:<[^()]*?>)?\s*\(")
}

const fn is_quote(b: u8) -> bool {
    matches!(b, b'"' | b'\'' | b'`')
}

/// Whether the byte before `at` is one the original's lookbehind `(?<![chars])` rejects.
fn preceded_by(src: &[u8], at: usize, also_dot: bool) -> bool {
    if at == 0 {
        return false;
    }
    let b = src[at - 1];
    b.is_ascii_alphanumeric()
        || b == b'_'
        || b == b'$'
        || (also_dot && b == b'.')
        || (b >= 0x80 && word_char_before(src, at))
}

fn word_char_before(src: &[u8], at: usize) -> bool {
    let s = std::str::from_utf8(&src[..at]).ok();
    s.and_then(|s| s.chars().next_back()).is_some_and(char::is_alphanumeric)
}

struct Str {
    value: String,
    end: usize,
    holes: usize,
    tail: String,
}

/// Read a string or template literal starting at `i` (the quote). Template holes become `{}`; `holes` counts them.
fn read_string(src: &[u8], i: usize) -> Option<Str> {
    let q = *src.get(i)?;
    if !is_quote(q) {
        return None;
    }
    let mut value: Vec<u8> = Vec::new();
    let mut tail: Vec<u8> = Vec::new();
    let mut holes = 0;
    let mut j = i + 1;
    while j < src.len() {
        let c = src[j];
        if c == b'\\' {
            if let Some(&n) = src.get(j + 1) {
                value.push(n);
                tail.push(n);
            }
            j += 2;
            continue;
        }
        if c == q {
            let value = String::from_utf8_lossy(&value).into_owned();
            let tail = if holes > 0 { String::from_utf8_lossy(&tail).into_owned() } else { value.clone() };
            return Some(Str { value, end: j + 1, holes, tail });
        }
        if q != b'`' && c == b'\n' {
            return None;
        }
        if q == b'`' && c == b'$' && src.get(j + 1) == Some(&b'{') {
            let mut depth = 1;
            j += 2;
            while j < src.len() && depth > 0 {
                let d = src[j];
                if d == b'{' {
                    depth += 1;
                } else if d == b'}' {
                    depth -= 1;
                } else if is_quote(d) {
                    if let Some(s) = read_string(src, j) {
                        j = s.end - 1;
                    }
                }
                j += 1;
            }
            value.extend_from_slice(b"{}");
            holes += 1;
            tail.clear();
            continue;
        }
        value.push(c);
        tail.push(c);
        j += 1;
    }
    None
}

/// Index just past the `)` that closes the call whose `(` is at `open - 1`, skipping strings; `src.len()` if unbalanced.
fn args_end(src: &[u8], open: usize) -> usize {
    let mut depth = 1;
    let mut j = open;
    while j < src.len() {
        let c = src[j];
        if is_quote(c) {
            if let Some(s) = read_string(src, j) {
                j = s.end;
                continue;
            }
        }
        if matches!(c, b'(' | b'[' | b'{') {
            depth += 1;
        } else if matches!(c, b')' | b']' | b'}') {
            depth -= 1;
            if depth == 0 {
                return j + 1;
            }
        }
        j += 1;
    }
    src.len()
}

fn skip_ws(src: &[u8], mut i: usize) -> usize {
    while i < src.len() && src[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// Path part of a URL: origin, query and hash dropped, slashes collapsed. `None` unless it is a path (starts with `/`).
pub fn url_path(raw: &str) -> Option<String> {
    static ORIGIN: OnceLock<Regex> = OnceLock::new();
    static QUERY: OnceLock<Regex> = OnceLock::new();
    static SLASHES: OnceLock<Regex> = OnceLock::new();
    let u = re(&ORIGIN, r"^https?://[^/]+").replace(raw.trim(), "");
    let u = re(&QUERY, r"[?#].*$").replace(&u, "");
    let u = re(&SLASHES, r"/{2,}").replace_all(&u, "/").into_owned();
    if !u.starts_with('/') {
        return None;
    }
    Some(if u.len() > 1 { u.strip_suffix('/').map(str::to_string).unwrap_or(u) } else { u })
}

fn enclosing(file: &SourceFile<'_>, at: usize) -> String {
    let mut best: Option<&SymbolFact> = None;
    for s in file.symbols {
        if s.kind == SymbolKind::Type || s.span.end <= s.span.start || at < s.span.start || at >= s.span.end {
            continue;
        }
        if best.is_none_or(|b| s.span.end - s.span.start < b.span.end - b.span.start) {
            best = Some(s);
        }
    }
    best.map_or_else(|| file.path.to_string(), |s| s.id.clone())
}

fn short_name(id: &str) -> Option<String> {
    let hash = id.find('#')?;
    id[hash + 1..].split('.').next_back().map(str::to_string)
}

fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Without symbol spans: the name of the nearest function-like definition before `at`.
fn defined_before(src: &str, at: usize) -> Option<String> {
    static DEF: OnceLock<Regex> = OnceLock::new();
    let head = &src[floor_char_boundary(src, at.saturating_sub(2000))..at];
    let def = re(
        &DEF,
        r"\bfunction\s*\*?\s*([A-Za-z_$][\w$]*)|([A-Za-z_$][\w$]*)\s*[:=]\s*(?:async\s*)?(?:function\b|\([^()]*\)\s*(?::[^=]*?)?=>|[A-Za-z_$][\w$]*\s*=>)",
    );
    let mut name = None;
    for m in def.captures_iter(head) {
        if let Some(n) = m.get(1).or_else(|| m.get(2)) {
            name = Some(n.as_str().to_string());
        }
    }
    name
}

fn method_in(src: &str, from: usize, to: usize) -> Option<String> {
    static METHOD: OnceLock<Regex> = OnceLock::new();
    let to = floor_char_boundary(src, to.min(src.len()));
    let from = floor_char_boundary(src, from.min(to));
    re(&METHOD, r#"\bmethod\s*:\s*["'`]([A-Za-z]+)["'`]"#).captures(&src[from..to]).map(|c| c[1].to_uppercase())
}

struct Wrapper {
    prefix: String,
    method: Option<String>,
    symbol: String,
}

/// Every fetch site in `files`:
/// 1. direct calls (`fetch`, `$fetch`, `ofetch`, `useFetch`, `useLazyFetch`) whose first argument is a URL literal;
/// 2. wrappers: a function whose `fetch` takes a non-literal URL (`fetch(path)`, ``fetch(`/api${path}`)``) is a
///    wrapper with that prefix; calls to it by name, anywhere, with a URL literal first argument are sites.
///
/// The method comes from a `method: "POST"` in the call's arguments; else GET. Client libraries (axios, ky, got,
/// `ofetch.create`) and their instances are not read here: the extractor resolves those calls ([`RequestFact`],
/// [`client_sites`]).
/// Sorted by file, then position.
pub fn find_fetch_sites(files: &[SourceFile<'_>]) -> Vec<FetchSite> {
    static HINT: OnceLock<Regex> = OnceLock::new();
    static VAR_ARG: OnceLock<Regex> = OnceLock::new();
    static DEF_BEFORE: OnceLock<Regex> = OnceLock::new();
    let mut wrappers: Vec<(String, Wrapper)> = Vec::new();
    let set_wrapper = |name: String, w: Wrapper, wrappers: &mut Vec<(String, Wrapper)>| {
        if let Some(slot) = wrappers.iter_mut().find(|(n, _)| *n == name) {
            slot.1 = w;
        } else {
            wrappers.push((name, w));
        }
    };
    let mut sites: Vec<(FetchSite, usize)> = Vec::new();

    for f in files {
        let src = f.source;
        let bytes = src.as_bytes();
        if !re(&HINT, r"fetch|Fetch").is_match(src) {
            continue;
        }
        for m in client_re().captures_iter(src) {
            let whole = m.get(0).expect("match");
            if preceded_by(bytes, whole.start(), true) {
                continue;
            }
            let open = whole.end();
            let end = args_end(bytes, open);
            let a = skip_ws(bytes, open);
            let client = m[1].to_string();
            let fixed: Option<String> = None;
            let from = enclosing(f, whole.start());
            if let Some(s) = read_string(bytes, a) {
                // `${…}` right after a literal prefix that doesn't end in `/` (e.g. `/api${path}`): a wrapper, not a site.
                if let Some(lead) = s.value.find("{}") {
                    if s.holes == 1 && !s.value[..lead].ends_with('/') && s.tail.is_empty() {
                        if let Some(name) = short_name(&from).or_else(|| defined_before(src, whole.start())) {
                            let method = fixed.clone().or_else(|| method_in(src, s.end, end));
                            set_wrapper(
                                name,
                                Wrapper { prefix: s.value[..lead].to_string(), method, symbol: from },
                                &mut wrappers,
                            );
                        }
                        continue;
                    }
                }
                if let Some(url) = url_path(&s.value) {
                    let method = fixed.or_else(|| method_in(src, s.end, end)).unwrap_or_else(|| "GET".into());
                    let via = (client != "fetch").then_some(client);
                    sites.push((
                        FetchSite { file: f.path.into(), from, method, url, exact: s.holes == 0, via },
                        whole.start(),
                    ));
                }
            } else {
                let probe = &src[a..floor_char_boundary(src, (a + 80).min(src.len()))];
                if re(&VAR_ARG, r"^[A-Za-z_$][\w$]*\s*[,)]").is_match(probe) {
                    if let Some(name) = short_name(&from).or_else(|| defined_before(src, whole.start())) {
                        let method = fixed.or_else(|| method_in(src, a, end));
                        set_wrapper(name, Wrapper { prefix: String::new(), method, symbol: from }, &mut wrappers);
                    }
                }
            }
        }
    }

    if !wrappers.is_empty() {
        let names = wrappers.iter().map(|(n, _)| regex::escape(n)).collect::<Vec<_>>().join("|");
        let call = Regex::new(&format!(r"({names})\s*(?:<[^()]*?>)?\s*\(")).expect("valid regex");
        let by_name: HashMap<&str, &Wrapper> = wrappers.iter().map(|(n, w)| (n.as_str(), w)).collect();
        for f in files {
            let src = f.source;
            let bytes = src.as_bytes();
            for m in call.captures_iter(src) {
                let whole = m.get(0).expect("match");
                if preceded_by(bytes, whole.start(), false) {
                    continue;
                }
                let before = &src[floor_char_boundary(src, whole.start().saturating_sub(12))..whole.start()];
                if re(&DEF_BEFORE, r"\bfunction\s*\*?\s*$").is_match(before) {
                    continue; // the definition
                }
                let w = by_name[&m[1]];
                let open = whole.end();
                let a = skip_ws(bytes, open);
                let Some(s) = read_string(bytes, a) else {
                    continue;
                };
                let Some(url) = url_path(&format!("{}{}", w.prefix, s.value)) else {
                    continue;
                };
                let from = enclosing(f, whole.start());
                if from == w.symbol && from.contains('#') {
                    continue; // a wrapper calling itself
                }
                let method = w
                    .method
                    .clone()
                    .or_else(|| method_in(src, s.end, args_end(bytes, open)))
                    .unwrap_or_else(|| "GET".into());
                let exact = s.holes == 0 && w.prefix.is_empty();
                sites.push((
                    FetchSite { file: f.path.into(), from, method, url, exact, via: Some(m[1].to_string()) },
                    whole.start(),
                ));
            }
        }
    }
    sites.sort_by(|(x, xa), (y, ya)| x.file.cmp(&y.file).then(xa.cmp(ya)));
    sites.into_iter().map(|(s, _)| s).collect()
}

/// The requests a file's facts hold, as fetch sites: a call on a client library as written, a call through an
/// instance with the instance's prefix in front. `client` finds a [`ClientFact`] by id (in whatever file defines it);
/// a request through an instance it doesn't find, or one whose prefix isn't known, is left out.
pub fn client_sites(facts: &FileFacts, client: impl Fn(&str) -> Option<ClientFact>) -> Vec<FetchSite> {
    let mut out = Vec::new();
    for r in &facts.requests {
        let (url, exact) = if r.via.starts_with("vendor:") {
            (Some(r.url.clone()), r.exact)
        } else {
            match client(&r.via).and_then(|c| c.prefix) {
                Some(p) => {
                    (url_path(&format!("{}/{}", p.trim_end_matches('/'), r.url.trim_start_matches('/'))), r.exact)
                }
                None => (None, false),
            }
        };
        if let Some(url) = url {
            out.push(FetchSite {
                file: facts.path.clone(),
                from: r.from.clone(),
                method: r.method.clone(),
                url,
                exact,
                via: Some(r.via.clone()),
            });
        }
    }
    out
}

/// How sure a route match is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchConfidence {
    /// The site's URL is exact and matched a route segment-for-segment with no wildcard on the site side.
    Syntactic,
    Inferred,
}

fn seg_re(s: &str) -> Regex {
    let body = s.split("{}").map(regex::escape).collect::<Vec<_>>().join("[^/]+");
    Regex::new(&format!("^{body}$")).expect("valid regex")
}

/// The route a site calls: same number of path segments, each equal or matched through a `{}` on either side, and
/// the same method (a route with method ANY matches any). Among matches the one with the fewest wildcards wins
/// (then id).
pub fn match_route<'r>(
    method: &str,
    url: &str,
    exact: bool,
    routes: &'r [RouteFact],
) -> Option<(&'r RouteFact, MatchConfidence)> {
    let segs = |p: &str| p.split('/').filter(|s| !s.is_empty()).map(str::to_string).collect::<Vec<_>>();
    let want = segs(url);
    let mut best: Option<(&RouteFact, usize, bool)> = None;
    for r in routes {
        if r.method != method && r.method != "ANY" {
            continue;
        }
        let have = segs(&r.path);
        if have.len() != want.len() {
            continue;
        }
        let (mut wild, mut site_wild, mut ok) = (0, false, true);
        for (h, w) in have.iter().zip(&want) {
            if h == w {
                continue;
            }
            if h == "{}" {
                wild += 1;
            } else if w.contains("{}") && seg_re(w).is_match(h) {
                wild += 1;
                site_wild = true;
            } else if h.contains("{}") && seg_re(h).is_match(w) {
                wild += 1;
            } else {
                ok = false;
                break;
            }
        }
        if !ok {
            continue;
        }
        if best.is_none_or(|(b, bw, _)| wild < bw || (wild == bw && r.id < b.id)) {
            best = Some((r, wild, site_wild));
        }
    }
    let (route, _, site_wild) = best?;
    let sure = exact && !site_wild && route.method != "ANY";
    Some((route, if sure { MatchConfidence::Syntactic } else { MatchConfidence::Inferred }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::Span;

    fn sym(id: &str, start: usize, end: usize) -> SymbolFact {
        SymbolFact {
            id: id.into(),
            file: id[..id.find('#').unwrap()].into(),
            name: id[id.find('#').unwrap() + 1..].into(),
            kind: SymbolKind::Function,
            exported: true,
            parent: None,
            span: Span { start, end },
            sig_hash: String::new(),
            body_hash: String::new(),
        }
    }

    #[test]
    fn finds_a_direct_fetch_with_a_template_url() {
        let src =
            "export function H({ id }) {\n  const go = () => fetch(`/api/p/${id}/convert`, { method: 'POST' });\n}\n";
        let symbols = [sym("a.tsx#H", 0, src.len())];
        let sites = find_fetch_sites(&[SourceFile { path: "a.tsx", source: src, symbols: &symbols }]);
        assert_eq!(
            sites,
            vec![FetchSite {
                file: "a.tsx".into(),
                from: "a.tsx#H".into(),
                method: "POST".into(),
                url: "/api/p/{}/convert".into(),
                exact: false,
                via: None
            }]
        );
    }

    #[test]
    fn follows_a_wrapper_and_ignores_member_fetch() {
        let src = "function request(path) { return fetch(`/api${path}`); }\nfunction load() { return request('/users'); }\nthis.fetch('/x');\n";
        let sites = find_fetch_sites(&[SourceFile { path: "c.ts", source: src, symbols: &[] }]);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].url, "/api/users");
        assert_eq!(sites[0].via.as_deref(), Some("request"));
        assert!(!sites[0].exact);
    }

    #[test]
    fn finds_nuxt_use_fetch_in_a_vue_sfc() {
        let src = "<script setup lang=\"ts\">\nconst { data } = await useFetch('/api/orders');\nasync function add() {\n  await useLazyFetch(`/api/orders/${id}`, { method: 'post' });\n}\n</script>\n";
        let symbols = [sym("o.vue#add", src.find("async function").unwrap(), src.find("</script>").unwrap())];
        let sites = find_fetch_sites(&[SourceFile { path: "o.vue", source: src, symbols: &symbols }]);
        let got: Vec<(&str, &str, &str, Option<&str>)> =
            sites.iter().map(|s| (s.from.as_str(), s.method.as_str(), s.url.as_str(), s.via.as_deref())).collect();
        assert_eq!(
            got,
            [
                ("o.vue", "GET", "/api/orders", Some("useFetch")),
                ("o.vue#add", "POST", "/api/orders/{}", Some("useLazyFetch"))
            ]
        );
    }

    #[test]
    fn client_sites_put_the_instance_prefix_in_front() {
        use crate::contract::RequestFact;
        let mut f = FileFacts::empty("ui.ts");
        let req = |url: &str, via: &str| RequestFact {
            from: "ui.ts#load".into(),
            method: "GET".into(),
            url: url.into(),
            exact: true,
            via: via.into(),
        };
        f.requests = vec![
            req("/orders", "http.ts#api"),
            req("/x", "http.ts#env"),
            req("/a", "vendor:axios"),
            req("/b", "ui.ts#nope"),
        ];
        let client = |id: &str| match id {
            "http.ts#api" => {
                Some(ClientFact { id: id.into(), lib: "ky".into(), prefix: Some("https://shop.test/api/".into()) })
            }
            "http.ts#env" => Some(ClientFact { id: id.into(), lib: "axios".into(), prefix: None }),
            _ => None,
        };
        let urls: Vec<String> = client_sites(&f, client).into_iter().map(|s| s.url).collect();
        assert_eq!(urls, ["/api/orders", "/a"]);
    }

    #[test]
    fn matches_routes_by_segments_and_method() {
        let route = |m: &str, p: &str| RouteFact {
            id: format!("route:{m} {p}"),
            method: m.into(),
            path: p.into(),
            file: "r.ts".into(),
            handler: None,
        };
        let routes = [route("POST", "/api/p/{}/convert"), route("GET", "/api/p/{}")];
        let (r, c) = match_route("POST", "/api/p/{}/convert", false, &routes).unwrap();
        assert_eq!(r.id, "route:POST /api/p/{}/convert");
        assert_eq!(c, MatchConfidence::Inferred);
        assert!(match_route("DELETE", "/api/p/1", true, &routes).is_none());
        assert_eq!(match_route("GET", "/api/p/1", true, &routes).unwrap().1, MatchConfidence::Syntactic);
        assert_eq!(url_path("https://x.io//api/a/?q=1").as_deref(), Some("/api/a"));
    }
}
