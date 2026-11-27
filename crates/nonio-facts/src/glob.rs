//! Glob matching for repo-relative paths: `*` stays within a segment, `**` crosses them, and dotfiles match.
//! Compiled matchers are cached.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use globset::{GlobBuilder, GlobMatcher};

fn matcher(glob: &str) -> Option<GlobMatcher> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<GlobMatcher>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    cache
        .entry(glob.to_string())
        .or_insert_with(|| GlobBuilder::new(glob).literal_separator(true).build().ok().map(|g| g.compile_matcher()))
        .clone()
}

/// True if `path` matches any of `globs`. A glob that doesn't compile matches nothing.
pub fn matches_any<S: AsRef<str>>(globs: &[S], path: &str) -> bool {
    globs.iter().any(|g| matcher(g.as_ref()).is_some_and(|m| m.is_match(path)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_like_picomatch_with_dotfiles() {
        assert!(matches_any(&["src/**"], "src/a/b.ts"));
        assert!(matches_any(&["**/.env"], ".env"));
        assert!(matches_any(&["**/.env.*"], "app/.env.local"));
        assert!(matches_any(&["**/id_rsa*"], "home/.ssh/id_rsa.pub"));
        assert!(!matches_any(&["src/*.ts"], "src/a/b.ts"));
        assert!(!matches_any::<&str>(&[], "a"));
    }
}
