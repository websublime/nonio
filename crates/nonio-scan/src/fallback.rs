//! The regex reader for source that doesn't parse (a file mid-edit). Ported from `facts/regex.ts` of illuminate
//! (MIT, Copyright (c) 2026 johnz4021).

use std::sync::LazyLock;

use fancy_regex::Regex;

use crate::re::{at, each, group, js};

/// An import specifier and where it was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecSite {
    pub spec: String,
    pub type_only: bool,
    pub at: usize,
}

const REGEX_PRECEDERS: &[u8] = b"(,=:[!&|?{};+-*%<>~^";
const REGEX_KEYWORDS: [&[u8]; 13] = [
    b"return", b"typeof", b"case", b"do", b"else", b"in", b"of", b"void", b"yield", b"await", b"delete", b"throw",
    b"new",
];

/// Whether `before` ends with a keyword after which `/` starts a regex (`return /x/`), as a whole word.
fn ends_with_keyword(before: &[u8]) -> bool {
    REGEX_KEYWORDS.iter().any(|k| {
        before.ends_with(k) && {
            let at = before.len() - k.len();
            at == 0 || !(before[at - 1].is_ascii_alphanumeric() || before[at - 1] == b'_' || before[at - 1] == b'$')
        }
    })
}

/// `src` with `//` and `/* */` comments replaced by spaces (newlines kept), leaving strings, template literals and
/// regex literals as they are, so byte offsets and line numbers still match.
pub fn strip_comments(src: &str) -> String {
    let mut s = Strip { src: src.as_bytes(), out: Vec::with_capacity(src.len()), i: 0 };
    s.code(false);
    // Only ASCII bytes are ever replaced, and only inside comments (whole chars), so this is still UTF-8.
    String::from_utf8(s.out).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

struct Strip<'a> {
    src: &'a [u8],
    out: Vec<u8>,
    i: usize,
}

impl Strip<'_> {
    fn n(&self) -> usize {
        self.src.len()
    }

    fn copy(&mut self) {
        self.out.push(self.src[self.i]);
        self.i += 1;
    }

    fn copy_two(&mut self) {
        let stop = (self.i + 2).min(self.n());
        self.out.extend_from_slice(&self.src[self.i..stop]);
        self.i = stop;
    }

    fn quoted(&mut self, quote: u8) {
        self.copy();
        while self.i < self.n() && self.src[self.i] != quote && self.src[self.i] != b'\n' {
            if self.src[self.i] == b'\\' {
                self.copy();
            }
            if self.i < self.n() {
                self.copy();
            }
        }
        if self.i < self.n() && self.src[self.i] == quote {
            self.copy();
        }
    }

    fn regex(&mut self) {
        let mut class = false;
        self.copy();
        while self.i < self.n() && self.src[self.i] != b'\n' {
            let c = self.src[self.i];
            if c == b'\\' {
                self.copy_two();
                continue;
            }
            self.copy();
            if c == b'[' {
                class = true;
            } else if c == b']' {
                class = false;
            } else if c == b'/' && !class {
                break;
            }
        }
    }

    fn template(&mut self) {
        self.copy();
        while self.i < self.n() && self.src[self.i] != b'`' {
            if self.src[self.i] == b'\\' {
                self.copy_two();
            } else if self.src[self.i] == b'$' && self.src.get(self.i + 1) == Some(&b'{') {
                self.copy_two();
                self.code(true);
            } else {
                self.copy();
            }
        }
        if self.i < self.n() {
            self.copy();
        }
    }

    /// Code; when `in_template`, stop after the `}` that closes the `${`.
    fn code(&mut self, in_template: bool) {
        let mut depth: i64 = 0;
        while self.i < self.n() {
            let c = self.src[self.i];
            let next = self.src.get(self.i + 1).copied();
            if c == b'/' && next == Some(b'/') {
                while self.i < self.n() && self.src[self.i] != b'\n' {
                    self.out.push(b' ');
                    self.i += 1;
                }
            } else if c == b'/' && next == Some(b'*') {
                let stop = find(self.src, b"*/", self.i + 2).map_or(self.n(), |e| e + 2);
                for &b in &self.src[self.i..stop] {
                    self.out.push(if b == b'\n' { b'\n' } else { b' ' });
                }
                self.i = stop;
            } else if c == b'"' || c == b'\'' {
                self.quoted(c);
            } else if c == b'`' {
                self.template();
            } else if c == b'/' {
                let before = trim_end(&self.out);
                let prev = before.last().copied();
                if prev.is_none_or(|p| REGEX_PRECEDERS.contains(&p)) || ends_with_keyword(before) {
                    self.regex();
                } else {
                    self.copy();
                }
            } else {
                if c == b'{' {
                    depth += 1;
                } else if c == b'}' {
                    if in_template && depth == 0 {
                        self.copy();
                        return;
                    }
                    depth -= 1;
                }
                self.copy();
            }
        }
    }
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

fn trim_end(b: &[u8]) -> &[u8] {
    let mut n = b.len();
    while n > 0 && b[n - 1].is_ascii_whitespace() {
        n -= 1;
    }
    &b[..n]
}

static STATIC_FROM: LazyLock<Regex> =
    LazyLock::new(|| js(r#"(?<![\w$.])(?:import|export)\s+(type\s+)?[\w$\s{},*]*?\bfrom\s*(['"])([^'"\n]+)\2"#));
static SIDE_EFFECT: LazyLock<Regex> = LazyLock::new(|| js(r#"(?<![\w$.])import\s*(['"])([^'"\n]+)\1"#));
static DYNAMIC: LazyLock<Regex> = LazyLock::new(|| js(r#"(?<![\w$.])import\s*\(\s*(['"`])([^'"`$\n]+)\1\s*\)"#));
pub static REQUIRE: LazyLock<Regex> = LazyLock::new(|| js(r#"(?<![\w$.])require\s*\(\s*(['"])([^'"\n]+)\1\s*\)"#));

/// `require('x')` calls in comment-free code.
pub fn requires(code: &str, out: &mut Vec<SpecSite>) {
    for m in each(&REQUIRE, code) {
        out.push(SpecSite { spec: group(&m, 2).unwrap_or("").into(), type_only: false, at: at(&m) });
    }
}

/// Import specifiers in comment-stripped code, in source order.
pub fn regex_imports(code: &str) -> Vec<SpecSite> {
    let mut found = Vec::new();
    for m in each(&STATIC_FROM, code) {
        found.push(SpecSite { spec: group(&m, 3).unwrap_or("").into(), type_only: m.get(1).is_some(), at: at(&m) });
    }
    for re in [&*SIDE_EFFECT, &*DYNAMIC, &*REQUIRE] {
        for m in each(re, code) {
            found.push(SpecSite { spec: group(&m, 2).unwrap_or("").into(), type_only: false, at: at(&m) });
        }
    }
    found.sort_by_key(|s| s.at);
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_offsets_and_newlines_when_stripping_comments() {
        let src = "a // x\n/* y\nz */ b";
        let out = strip_comments(src);
        assert_eq!(out.len(), src.len());
        assert_eq!(out.split('\n').count(), 3);
        assert_eq!(crate::hash::squash(&out), "a b");
    }
}
