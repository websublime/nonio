//! The facts contract of nonio, and the pure functions over facts that the checker relies on.
//!
//! Extraction backends (oxc, `code2graph`, tree-sitter) live elsewhere and implement [`Extractor`]; the checker
//! knows only this crate (D12).

mod contract;
mod fetches;
mod glob;
mod settle;

pub use contract::*;
pub use fetches::{
    client_sites, find_fetch_sites, match_route, url_path, wrapper_called, FetchSite, MatchConfidence, SourceFile,
};
pub use glob::matches_any;
pub use settle::{settle_calls, symbol_names};

/// Collapse whitespace runs, so formatting-only differences compare equal.
pub fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    #[test]
    fn squash_collapses_whitespace() {
        assert_eq!(super::squash("  a \t b\n c  "), "a b c");
    }
}
