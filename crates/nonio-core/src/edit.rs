//! Changing a map after it's made: what the explainer finds is added, a fix's new connection is drawn, a missing
//! connection that a fix closed is dropped, and a repair round's changes are patched in. Every change goes back
//! through [`parse_map`], so ids stay unique and references stay valid.
//!
//! Ported from `packages/core/src/edit.ts` of illuminate (MIT, Copyright (c) 2026 johnz4021).

use std::collections::HashSet;

use serde_json::Value;

use crate::format::{parse_map, CodeMap, ParseResult};

/// New areas, boxes, arrows and missing connections, as the model wrote them (validated on merge).
#[derive(Debug, Clone, Default)]
pub struct MapAdditions {
    pub areas: Vec<Value>,
    pub nodes: Vec<Value>,
    pub edges: Vec<Value>,
    pub absences: Vec<Value>,
}

/// Changes to a map, as a model returns them when fixing a few claims: items whose id already exists replace it,
/// new ids are added, and `remove` drops ids (with what points at them). Everything not mentioned stays.
#[derive(Debug, Clone, Default)]
pub struct MapPatch {
    pub areas: Vec<Value>,
    pub nodes: Vec<Value>,
    pub edges: Vec<Value>,
    pub absences: Vec<Value>,
    pub remove: Vec<String>,
}

const LISTS: [&str; 4] = ["areas", "nodes", "edges", "absences"];

fn lists(add: &MapAdditions) -> [&Vec<Value>; 4] {
    [&add.areas, &add.nodes, &add.edges, &add.absences]
}

pub fn add_to_map(map: &CodeMap, add: &MapAdditions) -> ParseResult {
    let mut v = map.to_value();
    for (key, items) in LISTS.iter().zip(lists(add)) {
        if let Some(Value::Array(list)) = v.get_mut(*key) {
            list.extend(items.iter().cloned());
        }
    }
    parse_map(&v)
}

/// The map without the given ids, and without arrows, missing connections or tour mentions that point at removed
/// boxes. The map is returned as it was if the result wouldn't parse.
pub fn remove_from_map(map: &CodeMap, ids: &HashSet<String>) -> CodeMap {
    if ids.is_empty() {
        return map.clone();
    }
    let mut m = map.clone();
    m.nodes.retain(|n| !ids.contains(&n.id));
    let kept: HashSet<&str> = m.nodes.iter().map(|n| n.id.as_str()).collect();
    m.edges.retain(|e| !ids.contains(&e.id) && kept.contains(e.from.as_str()) && kept.contains(e.to.as_str()));
    m.absences.retain(|a| {
        !ids.contains(&a.id)
            && a.between.as_ref().is_none_or(|b| kept.contains(b.from.as_str()) && kept.contains(b.to.as_str()))
    });
    let nodes = &m.nodes;
    m.areas.retain(|a| !ids.contains(&a.id) || nodes.iter().any(|n| n.area == a.id));
    let live: HashSet<String> = m
        .nodes
        .iter()
        .map(|x| x.id.clone())
        .chain(m.edges.iter().map(|x| x.id.clone()))
        .chain(m.absences.iter().map(|x| x.id.clone()))
        .chain(m.areas.iter().map(|x| x.id.clone()))
        .collect();
    for t in &mut m.tour {
        for list in
            [&mut t.focus.areas, &mut t.focus.nodes, &mut t.focus.edges, &mut t.focus.absences].into_iter().flatten()
        {
            list.retain(|id| live.contains(id));
        }
    }
    parse_map(&m.to_value()).unwrap_or_else(|_| map.clone())
}

pub fn patch_map(map: &CodeMap, patch: &MapPatch) -> ParseResult {
    let base = remove_from_map(map, &patch.remove.iter().cloned().collect());
    let mut v = base.to_value();
    for (key, items) in LISTS.iter().zip([&patch.areas, &patch.nodes, &patch.edges, &patch.absences]) {
        let Some(Value::Array(list)) = v.get_mut(*key) else {
            continue;
        };
        for item in items {
            let at = item
                .get("id")
                .and_then(Value::as_str)
                .and_then(|id| list.iter().position(|x| x.get("id").and_then(Value::as_str) == Some(id)));
            match at {
                Some(i) => list[i] = item.clone(),
                None => list.push(item.clone()),
            }
        }
    }
    parse_map(&v)
}

pub fn remove_absences(map: &CodeMap, ids: &[String]) -> CodeMap {
    let drop: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let mut m = map.clone();
    m.absences.retain(|a| !drop.contains(a.id.as_str()));
    for t in &mut m.tour {
        if let Some(list) = &mut t.focus.absences {
            list.retain(|id| !drop.contains(id.as_str()));
        }
    }
    m
}

/// Ids of everything in the map, for picking new ones.
pub fn map_ids(map: &CodeMap) -> HashSet<String> {
    map.areas
        .iter()
        .map(|x| x.id.clone())
        .chain(map.nodes.iter().map(|x| x.id.clone()))
        .chain(map.edges.iter().map(|x| x.id.clone()))
        .chain(map.absences.iter().map(|x| x.id.clone()))
        .chain(map.tour.iter().map(|x| x.id.clone()))
        .collect()
}

pub fn fresh_id(map: &CodeMap, prefix: &str) -> String {
    let taken = map_ids(map);
    (1..).map(|i| format!("{prefix}{i}")).find(|id| !taken.contains(id)).expect("an unused id")
}

const STOP_WORDS: &[&str] =
    &["what", "happens", "when", "the", "a", "an", "does", "do", "how", "is", "are", "it", "go", "out"];

/// A short, file-safe id for a map from its question.
pub fn slugify(question: &str) -> String {
    let lower = question.to_lowercase().replace(['\'', '’'], "");
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    for c in lower.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            cur.push(c);
        } else if !cur.is_empty() {
            words.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    let slug = words.into_iter().filter(|w| !STOP_WORDS.contains(&w.as_str())).take(6).collect::<Vec<_>>().join("-");
    if slug.is_empty() {
        "map".into()
    } else {
        slug
    }
}
