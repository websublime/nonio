//! The content and symbol hashes of illuminate (MIT, Copyright (c) 2026 johnz4021), `facts/hash.ts`: two FNV-1a
//! variants over the UTF-16 code units of the text, so the hashes equal the original's for the same text.

/// 16 hex chars, equal to illuminate's `hashText` for the same text.
pub fn hash_text(text: &str) -> String {
    let len = text.encode_utf16().count() as u32;
    let mut a: u32 = 0x811c_9dc5;
    let mut b: u32 = 0x0100_0193 ^ len;
    for c in text.encode_utf16() {
        let c = u32::from(c);
        a = (a ^ c).wrapping_mul(0x0100_0193);
        b = (b ^ c).wrapping_mul(0x5bd1_e995);
        b ^= b >> 15;
    }
    format!("{a:08x}{b:08x}")
}

/// Whitespace as JavaScript's `\s` sees it (which differs from Rust's `char::is_whitespace` at U+0085 and U+FEFF).
pub fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

/// Collapse whitespace runs to one space and trim, like the original's `squash`, so formatting-only edits keep the
/// same hash.
pub fn squash(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars() {
        if is_js_space(c) {
            space = true;
        } else {
            if space && !out.is_empty() {
                out.push(' ');
            }
            space = false;
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_like_the_original() {
        // Values computed with illuminate's hashText.
        assert_eq!(hash_text(""), "811c9dc501000193");
        assert_eq!(hash_text("export function f() {}"), "20ecbfcc3b810499");
        assert_eq!(hash_text("é😀x"), "269e8c9b1f1be86f");
    }

    #[test]
    fn squashes_js_whitespace() {
        assert_eq!(squash("  a \t b\n\u{feff} c  "), "a b c");
    }
}
