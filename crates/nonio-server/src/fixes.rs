//! Fixing from the map. The model proposes; this module validates the proposal, and after the person approves it
//! writes exactly the reviewed change: only if the files haven't changed since, never to secrets, nonio's own folder
//! or outside the repo, with the originals kept for undo. Then the map gets the new connection and drops the gap it
//! closed. The server enforces these rules itself (D25); nothing rests on asking the agent.
//!
//! Ported from `packages/server/src/fixes.ts` and `git.ts` (MIT, Copyright (c) 2026 johnz4021); see NOTICE.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use nonio_core::edit::{fresh_id, remove_absences};
use nonio_core::format::is_safe_relative_path;
use nonio_core::repo::{is_secret_path, Reader, RepoReader, STATE_DIR};
use nonio_core::{parse_map, CodeMap};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::ids::random_id;

/// An exact-text replacement: `find` is copied from the file and occurs exactly once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edit {
    pub file: String,
    pub find: String,
    pub replace: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Create {
    pub file: String,
    pub content: String,
}

/// The connection a fix creates between two boxes; `evidence_text` must appear in the new code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connects {
    pub from: String,
    pub to: String,
    pub label: String,
    pub evidence_file: String,
    pub evidence_text: String,
}

/// A fix as the model writes it (`fix` in an answer).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawFix {
    pub summary: String,
    #[serde(default)]
    pub edits: Vec<Edit>,
    #[serde(default)]
    pub creates: Vec<Create>,
    #[serde(default)]
    pub connects: Option<Connects>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Change {
    Edit,
    Create,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixFile {
    pub file: String,
    pub change: Change,
    /// SHA-256 of the file when the fix was proposed; `None` for a new file.
    pub hash: Option<String>,
    pub uncommitted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixProposal {
    pub id: String,
    pub summary: String,
    pub edits: Vec<Edit>,
    pub creates: Vec<Create>,
    pub connects: Option<Connects>,
    pub files: Vec<FixFile>,
    /// A readable before/after of every change, for the person approving it.
    pub diff: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Original {
    pub file: String,
    pub content: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Written {
    pub file: String,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedFix {
    pub proposal: FixProposal,
    pub originals: Vec<Original>,
    pub written: Vec<Written>,
}

fn sha(text: &str) -> String {
    Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn line_of(text: &str, index: usize) -> usize {
    text[..index].matches('\n').count() + 1
}

/// True when git reports the file as modified, added or untracked. False outside a git repo.
pub fn has_uncommitted_changes(root: &Path, file: &str) -> bool {
    Command::new("git")
        .args(["status", "--porcelain", "--", file])
        .current_dir(root)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .is_ok_and(|o| o.status.success() && !String::from_utf8_lossy(&o.stdout).trim().is_empty())
}

fn allowed(file: &str) -> Option<String> {
    if !is_safe_relative_path(file) {
        return Some(format!("{file}: must be a path inside the repo"));
    }
    if is_secret_path(file) || file == STATE_DIR || file.starts_with(&format!("{STATE_DIR}/")) {
        return Some(format!("{file}: nonio never changes this file"));
    }
    None
}

fn diff_lines<'a>(prefix: &'a str, text: &'a str) -> impl Iterator<Item = String> + 'a {
    text.split('\n').map(move |l| format!("{prefix} {l}"))
}

pub fn propose_fix(root: &Path, map: &CodeMap, raw: &RawFix) -> Result<FixProposal, Vec<String>> {
    let repo = RepoReader::new(root).map_err(|e| vec![e.to_string()])?;
    let mut problems: Vec<String> = Vec::new();
    // In the order they were first touched.
    let mut after: Vec<(String, String)> = Vec::new();
    let mut before: Vec<(String, String)> = Vec::new();
    let mut diff: Vec<String> = Vec::new();
    let get = |list: &Vec<(String, String)>, f: &str| list.iter().position(|(k, _)| k == f);

    if raw.edits.is_empty() && raw.creates.is_empty() {
        problems.push("the fix changes nothing".into());
    }
    for e in &raw.edits {
        if let Some(bad) = allowed(&e.file) {
            problems.push(bad);
            continue;
        }
        if get(&before, &e.file).is_none() {
            match repo.read(&e.file) {
                Ok(text) => {
                    before.push((e.file.clone(), text.to_string()));
                    after.push((e.file.clone(), text.to_string()));
                }
                Err(reason) => {
                    problems.push(format!("{}: {reason}", e.file));
                    continue;
                }
            }
        }
        let i = get(&after, &e.file).expect("read above");
        let current = &after[i].1;
        let n = current.matches(e.find.as_str()).count();
        if n != 1 {
            problems.push(format!("{}: the text to replace appears {n} times (it must appear exactly once)", e.file));
            continue;
        }
        let at = current.find(e.find.as_str()).expect("found once");
        diff.push(format!("{} · line {}", e.file, line_of(current, at)));
        diff.extend(diff_lines("-", &e.find));
        diff.extend(diff_lines("+", &e.replace));
        diff.push(String::new());
        let next = format!("{}{}{}", &current[..at], e.replace, &current[at + e.find.len()..]);
        after[i].1 = next;
    }
    for c in &raw.creates {
        if let Some(bad) = allowed(&c.file) {
            problems.push(bad);
            continue;
        }
        if root.join(&c.file).exists() || get(&after, &c.file).is_some() {
            problems.push(format!("{}: already exists", c.file));
            continue;
        }
        after.push((c.file.clone(), c.content.clone()));
        diff.push(format!("{} · new file", c.file));
        diff.extend(diff_lines("+", c.content.strip_suffix('\n').unwrap_or(&c.content)));
        diff.push(String::new());
    }

    if let Some(c) = &raw.connects {
        let ids: Vec<&str> = map.nodes.iter().map(|n| n.id.as_str()).collect();
        if !ids.contains(&c.from.as_str()) || !ids.contains(&c.to.as_str()) {
            problems.push(format!("the new connection must join two boxes on the map ({} → {})", c.from, c.to));
        }
        let text = get(&after, &c.evidence_file).map(|i| after[i].1.as_str()).unwrap_or("");
        if !text.contains(&c.evidence_text) {
            problems.push(format!(
                "the new connection's evidence (\"{}\") isn't in the changed {}",
                c.evidence_text, c.evidence_file
            ));
        }
    }
    if !problems.is_empty() {
        return Err(problems);
    }

    let files = after
        .iter()
        .map(|(file, _)| match get(&before, file) {
            Some(i) => FixFile {
                file: file.clone(),
                change: Change::Edit,
                hash: Some(sha(&before[i].1)),
                uncommitted: has_uncommitted_changes(root, file),
            },
            None => FixFile { file: file.clone(), change: Change::Create, hash: None, uncommitted: false },
        })
        .collect();
    Ok(FixProposal {
        id: format!("fix-{}", random_id()),
        summary: raw.summary.clone(),
        edits: raw.edits.clone(),
        creates: raw.creates.clone(),
        connects: raw.connects.clone(),
        files,
        diff: diff.join("\n").trim_end().to_string(),
    })
}

/// Writes the approved change. Refuses if any file changed since the proposal (someone, or the agent, edited it).
pub fn apply_fix(root: &Path, proposal: &FixProposal) -> Result<AppliedFix, Vec<String>> {
    let mut problems = Vec::new();
    let mut contents: Vec<(String, String)> = Vec::new();
    let mut originals = Vec::new();
    for f in &proposal.files {
        if allowed(&f.file).is_some() {
            problems.push(format!("{}: nonio never changes this file", f.file));
            continue;
        }
        let abs = root.join(&f.file);
        if f.change == Change::Create {
            if abs.exists() {
                problems.push(format!("{} was created since this fix was proposed", f.file));
            }
            originals.push(Original { file: f.file.clone(), content: None });
            continue;
        }
        match fs::read_to_string(&abs) {
            Ok(text) if Some(sha(&text)) == f.hash => {
                contents.push((f.file.clone(), text.clone()));
                originals.push(Original { file: f.file.clone(), content: Some(text) });
            }
            _ => problems.push(format!("{} changed since this fix was proposed; ask for the fix again", f.file)),
        }
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    for e in &proposal.edits {
        if let Some(slot) = contents.iter_mut().find(|(f, _)| *f == e.file) {
            slot.1 = slot.1.replacen(&e.find, &e.replace, 1);
        }
    }
    for c in &proposal.creates {
        contents.push((c.file.clone(), c.content.clone()));
    }

    let io = |e: std::io::Error| vec![e.to_string()];
    let backup = root.join(STATE_DIR).join("fixes").join(&proposal.id);
    fs::create_dir_all(&backup).map_err(io)?;
    let saved = json!({ "proposal": proposal, "originals": originals });
    fs::write(backup.join("originals.json"), serde_json::to_string_pretty(&saved).expect("serializes")).map_err(io)?;
    let mut written = Vec::new();
    for (file, text) in &contents {
        let abs = root.join(file);
        if let Some(dir) = abs.parent() {
            fs::create_dir_all(dir).map_err(io)?;
        }
        fs::write(&abs, text).map_err(io)?;
        written.push(Written { file: file.clone(), hash: sha(text) });
    }
    Ok(AppliedFix { proposal: proposal.clone(), originals, written })
}

/// Puts the files back, unless they were changed again after the fix (then undoing would lose that work).
pub fn undo_fix(root: &Path, applied: &AppliedFix) -> Result<(), Vec<String>> {
    let problems: Vec<String> = applied
        .written
        .iter()
        .filter(|w| fs::read_to_string(root.join(&w.file)).map(|t| sha(&t)).ok().as_ref() != Some(&w.hash))
        .map(|w| format!("{} changed after the fix; undo it by hand", w.file))
        .collect();
    if !problems.is_empty() {
        return Err(problems);
    }
    for o in &applied.originals {
        let abs = root.join(&o.file);
        let done = match &o.content {
            None => {
                fs::remove_file(&abs)
                    .or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) })
            }
            Some(text) => fs::write(&abs, text),
        };
        done.map_err(|e| vec![e.to_string()])?;
    }
    Ok(())
}

/// What a landed fix did to the map.
#[derive(Debug, Clone, PartialEq)]
pub struct Connected {
    pub map: CodeMap,
    pub edge_id: Option<String>,
    pub closed: Vec<String>,
}

/// After a fix lands: draw its new connection (evidence located in the new code), and drop the gap it closed.
pub fn connect_after_fix(root: &Path, map: &CodeMap, proposal: &FixProposal) -> Connected {
    let unchanged = || Connected { map: map.clone(), edge_id: None, closed: Vec::new() };
    let Some(c) = &proposal.connects else { return unchanged() };
    let text =
        RepoReader::new(root).ok().and_then(|r| r.text(&c.evidence_file)).map(|t| t.to_string()).unwrap_or_default();
    let Some(index) = text.find(&c.evidence_text) else { return unchanged() };
    let id = fresh_id(map, "fx");
    let mut v = map.to_value();
    if let Some(edges) = v.get_mut("edges").and_then(|e| e.as_array_mut()) {
        edges.push(json!({
            "id": id, "from": c.from, "to": c.to, "kind": "then", "label": c.label,
            "evidence": [{ "file": c.evidence_file, "line": line_of(&text, index), "text": c.evidence_text }],
        }));
    }
    let Ok(parsed) = parse_map(&v) else { return unchanged() };
    let closed: Vec<String> = parsed
        .absences
        .iter()
        .filter(|a| {
            a.between.as_ref().is_some_and(|b| (b.from == c.from && b.to == c.to) || (b.from == c.to && b.to == c.from))
        })
        .map(|a| a.id.clone())
        .collect();
    Connected { map: remove_absences(&parsed, &closed), edge_id: Some(id), closed }
}
