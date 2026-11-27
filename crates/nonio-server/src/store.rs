//! Maps on disk, in the target repo: `.nonio/maps/<id>.json` (D29). Commit them to share; teammates re-check them
//! against their own copy of the code when they open them. Each file also keeps the questions asked on the map with
//! their answers, and where each proposed fix stands, so a reopened map still has its conversation.
//!
//! Ported from `packages/server/src/store.ts` (MIT, Copyright (c) 2026 johnz4021); see NOTICE. The original moves an
//! older `.codemap/` folder over once; nonio has no older folder, and leaves the original's `.illuminate/` alone, so
//! both tools can be used on one repo.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use nonio_core::edit::slugify;
use nonio_core::{parse_map, CodeMap};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// nonio's folder in a repo.
pub const STATE_DIR: &str = ".nonio";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapMeta {
    pub id: String,
    pub question: String,
    pub created_at: String,
    pub updated_at: String,
    pub commit: Option<String>,
    pub maker_session: Option<String>,
    pub explainer_session: Option<String>,
    pub cost: f64,
    /// The map this one was started from, when a question there turned out to be about something else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<ParentMap>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParentMap {
    pub map_id: String,
    pub question: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FixStatus {
    Proposed,
    Applied,
    Rejected,
}

/// Where a proposed fix stands. Undo isn't saved: it needs the files as they were, which only the running server has.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedFix {
    pub status: FixStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StoredMap {
    pub meta: MapMeta,
    pub map: CodeMap,
    /// The questions asked on the map and their answers, as the explainer writes them.
    pub answers: Vec<Value>,
    pub fixes: BTreeMap<String, SavedFix>,
}

fn valid_id(id: &str) -> bool {
    (1..=80).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// An answer as saved: anything else in the list is left out, so a hand-edited file loses only what doesn't look right.
fn looks_like_answer(a: &Value) -> bool {
    a.get("id").is_some_and(Value::is_string)
        && a.get("question").is_some_and(Value::is_string)
        && a.get("added").is_some_and(Value::is_array)
}

#[derive(Debug, Clone)]
pub struct Store {
    base: PathBuf,
    dir: PathBuf,
}

impl Store {
    pub fn new(root: &Path) -> Self {
        let base = root.join(STATE_DIR);
        let dir = base.join("maps");
        Store { base, dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn file(&self, id: &str) -> io::Result<PathBuf> {
        if !valid_id(id) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("Not a map id: {id}")));
        }
        Ok(self.dir.join(format!("{id}.json")))
    }

    fn ensure(&self) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let ignore = self.base.join(".gitignore");
        // Maps are worth committing; scratch output is not.
        if !ignore.exists() {
            fs::write(ignore, "fixes/\nws-modules/\n")?;
        }
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<StoredMap> {
        let raw: Value = serde_json::from_str(&fs::read_to_string(self.file(id).ok()?).ok()?).ok()?;
        let map = parse_map(raw.get("map")?).ok()?;
        let meta: MapMeta = serde_json::from_value(raw.get("meta")?.clone()).ok()?;
        // Maps saved before answers were kept have neither.
        let answers = match raw.get("answers") {
            Some(Value::Array(a)) => a.iter().filter(|a| looks_like_answer(a)).cloned().collect(),
            _ => Vec::new(),
        };
        let fixes = match raw.get("fixes") {
            Some(Value::Object(o)) => {
                o.iter().filter_map(|(k, v)| serde_json::from_value(v.clone()).ok().map(|f| (k.clone(), f))).collect()
            }
            _ => BTreeMap::new(),
        };
        Some(StoredMap { meta, map, answers, fixes })
    }

    /// Written to a temporary file first, then moved over, so a reader never sees half a map.
    pub fn save(&self, stored: &StoredMap) -> io::Result<()> {
        self.ensure()?;
        let target = self.file(&stored.meta.id)?;
        let tmp = target.with_extension("json.tmp");
        let mut text = serde_json::to_string_pretty(stored).map_err(io::Error::other)?;
        text.push('\n');
        fs::write(&tmp, text)?;
        fs::rename(tmp, target)
    }

    /// Every saved map's meta, most recently updated first.
    pub fn list(&self) -> Vec<MapMeta> {
        let Ok(dir) = fs::read_dir(&self.dir) else { return Vec::new() };
        let mut metas: Vec<MapMeta> = dir
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .filter_map(|p| {
                let raw: Value = serde_json::from_str(&fs::read_to_string(p).ok()?).ok()?;
                serde_json::from_value(raw.get("meta")?.clone()).ok()
            })
            .collect();
        metas.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        metas
    }

    /// A new id for a question, unique among saved maps and ids already handed out.
    pub fn new_id(&self, question: &str, taken: &HashSet<String>) -> String {
        let base = slugify(question);
        (1..)
            .map(|i| if i == 1 { base.clone() } else { format!("{base}-{i}") })
            .find(|id| !taken.contains(id) && !self.dir.join(format!("{id}.json")).exists())
            .expect("an unused id")
    }
}
