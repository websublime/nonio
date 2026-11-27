//! Regular expressions written the way the original writes them (JavaScript syntax, with lookbehind, lookahead and
//! backreferences), compiled with `fancy-regex`. `\w` is made ASCII, as it is in JavaScript. `\b` stays Unicode-aware
//! (fancy-regex can't turn Unicode off inline, and spelling it as lookarounds would make every pattern backtrack), so
//! next to a non-ASCII letter (`ñdb.get`) a word boundary can differ from the original's.

use fancy_regex::{Captures, Regex, RegexBuilder};

/// `src` with every `\w` spelled as the ASCII class JavaScript means by it.
fn ascii_words(src: &str) -> String {
    let mut out = String::with_capacity(src.len() + 16);
    let mut class = false;
    let mut chars = src.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('w') => out.push_str(if class { "0-9A-Za-z_" } else { "[0-9A-Za-z_]" }),
                Some(n) => {
                    out.push('\\');
                    out.push(n);
                }
                None => out.push('\\'),
            },
            '[' if !class => {
                class = true;
                out.push(c);
            }
            ']' if class => {
                class = false;
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// A pattern of the original. Panics on a pattern that doesn't compile: they are all constants.
pub fn js(src: &str) -> Regex {
    try_js(src, false).unwrap_or_else(|| panic!("bad pattern {src}"))
}

/// Case-insensitive [`js`] (the `i` flag).
pub fn js_i(src: &str) -> Regex {
    try_js(src, true).unwrap_or_else(|| panic!("bad pattern {src}"))
}

/// A JavaScript pattern from configuration; `None` when it doesn't compile.
pub fn try_js(src: &str, case_insensitive: bool) -> Option<Regex> {
    RegexBuilder::new(&ascii_words(src)).case_insensitive(case_insensitive).build().ok()
}

/// Every match of `re` in `text`, like `text.matchAll(re)`. A match the engine gives up on (backtrack limit) ends the
/// scan.
pub fn each<'r, 't>(re: &'r Regex, text: &'t str) -> impl Iterator<Item = Captures<'t, str>> + use<'r, 't> {
    re.captures_iter(text).map_while(Result::ok)
}

/// Whether `re` matches anywhere in `text`.
pub fn test(re: &Regex, text: &str) -> bool {
    re.is_match(text).unwrap_or(false)
}

/// Group `i` of `c` as a string, if it took part in the match.
pub fn group<'t>(c: &Captures<'t, str>, i: usize) -> Option<&'t str> {
    c.get(i).map(|m| m.as_str())
}

/// Start of the whole match.
pub fn at(c: &Captures<'_, str>) -> usize {
    c.get(0).map_or(0, |m| m.start())
}

/// End of the whole match.
pub fn end(c: &Captures<'_, str>) -> usize {
    c.get(0).map_or(0, |m| m.end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn w_is_ascii() {
        assert_eq!(ascii_words(r"a\w[\w.]\\w"), r"a[0-9A-Za-z_][0-9A-Za-z_.]\\w");
        assert!(!test(&js(r"^\w$"), "é"));
        assert!(test(&js(r"(?<![\w$])db"), " db"));
        assert!(!test(&js(r"(?<![\w$])db"), "xdb"));
    }
}
