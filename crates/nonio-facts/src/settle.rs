//! Settling calls against the symbols that exist. Ported from `settleCalls` and `symbolNames` in illuminate
//! (MIT, Copyright (c) 2026 johnz4021).

use std::collections::{HashMap, HashSet};

use crate::contract::{CallFact, FileFacts};

/// Symbol names per file, for [`settle_calls`].
pub fn symbol_names(facts: &HashMap<String, FileFacts>) -> HashMap<String, HashSet<String>> {
    facts.iter().map(|(file, f)| (file.clone(), f.symbols.iter().map(|s| s.name.clone()).collect())).collect()
}

/// Point each call at the deepest symbol that exists: `db.ts#db.all` stays if `db.all` is a symbol, `registry.get`
/// on a Map becomes `#registry`, and a member nothing defines becomes the file path. `symbols_of` gives a file's
/// symbol names (`None` = unknown, leave as is). Calls that collapse onto the same target are merged; self-calls drop.
pub fn settle_calls<'a>(calls: &[CallFact], symbols_of: impl Fn(&str) -> Option<&'a HashSet<String>>) -> Vec<CallFact> {
    let mut out: Vec<CallFact> = Vec::new();
    let mut at: HashMap<(String, String), usize> = HashMap::new();
    for c in calls {
        let mut to = c.to.clone();
        if !to.starts_with("vendor:") {
            if let Some(hash) = to.find('#') {
                let file = to[..hash].to_string();
                if let Some(names) = symbols_of(&file) {
                    let mut parts: Vec<&str> = to[hash + 1..].split('.').collect();
                    while !parts.is_empty() && !names.contains(&parts.join(".")) {
                        parts.pop();
                    }
                    to = if parts.is_empty() { file } else { format!("{file}#{}", parts.join(".")) };
                }
            }
        }
        if to == c.from || (!to.contains('#') && c.from.starts_with(&format!("{to}#"))) {
            continue;
        }
        let key = (c.from.clone(), to.clone());
        if let Some(&i) = at.get(&key) {
            out[i].count += c.count;
        } else {
            at.insert(key, out.len());
            out.push(CallFact { to, ..c.clone() });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::CallConfidence;

    fn call(from: &str, to: &str) -> CallFact {
        CallFact { from: from.into(), to: to.into(), confidence: CallConfidence::Syntactic, count: 1 }
    }

    #[test]
    fn settles_to_the_deepest_symbol_and_merges() {
        let names: HashSet<String> = ["db".to_string(), "db.all".to_string()].into();
        let out = settle_calls(
            &[
                call("a.ts#f", "db.ts#db.all"),
                call("a.ts#f", "db.ts#db.get.x"),
                call("a.ts#f", "db.ts#db.get"),
                call("db.ts#db", "db.ts#db"),
            ],
            |f| (f == "db.ts").then_some(&names),
        );
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].to, "db.ts#db.all");
        assert_eq!(out[1].to, "db.ts#db");
        assert_eq!(out[1].count, 2);
    }

    #[test]
    fn a_member_nothing_defines_becomes_the_file() {
        let names: HashSet<String> = HashSet::new();
        let out = settle_calls(&[call("a.ts#f", "b.ts#x.y"), call("b.ts#g", "b.ts#nope")], |_| Some(&names));
        assert_eq!(out, vec![call("a.ts#f", "b.ts")]);
    }
}
