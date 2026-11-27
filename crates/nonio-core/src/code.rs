//! The code behind a box, an arrow or the notes on an answer, as blocks a person can read: the function a cited
//! line is in (whole when it is short, the lines around the citation when it is long), with the cited lines marked.
//! Read through the same readers and located with the same line check as everything else.
//!
//! Ported from `packages/core/src/code.ts` of illuminate (MIT, Copyright (c) 2026 johnz4021).

use std::path::Path;

use nonio_facts::{Extractor, SymbolKind};
use serde::Serialize;

use crate::check::{check_evidence, Readers, Side, DEFAULT_LINE_WINDOW};
use crate::format::{CodeMap, Evidence};

/// A line of code an answer points at, with what it says about that line. Checked like any citation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CodeNote {
    pub file: String,
    pub line: usize,
    pub text: String,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LineNote {
    pub line: usize,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CodeBlock {
    pub file: String,
    /// The function, class or type these lines are in, as the code facts name it.
    pub name: Option<String>,
    /// Whether the block is all of it, or only the lines around what is cited.
    pub whole: bool,
    /// The line number of the first line.
    pub start: usize,
    pub lines: Vec<String>,
    /// Line numbers the box, arrow or answer cites.
    pub cited: Vec<usize>,
    pub notes: Vec<LineNote>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum When {
    Now,
    /// The code as it was at the commit a map of a change is compared with.
    Before,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CodeView {
    pub when: When,
    pub blocks: Vec<CodeBlock>,
    /// Citations or notes whose line that version of the code doesn't have, so nothing is shown for them.
    pub missing: usize,
}

const WHOLE_NAMED: usize = 160;
const WHOLE_FOUND: usize = 60;
const CONTEXT: usize = 6;
const MAX_BLOCKS: usize = 8;
const MAX_LINES: usize = 500;
const MAX_LINE_CHARS: usize = 400;

struct Mark {
    file: String,
    line: usize,
    note: Option<String>,
}

#[derive(Clone)]
struct Range {
    from: usize,
    to: usize,
    name: Option<String>,
    whole: bool,
}

fn file_of(symbol: &str) -> &str {
    &symbol[..symbol.find('#').unwrap_or(0)]
}

/// Line numbers (from 1) for byte offsets into `text`.
fn line_finder(text: &str) -> impl Fn(usize) -> usize + '_ {
    let starts: Vec<usize> = std::iter::once(0).chain(text.match_indices('\n').map(|(i, _)| i + 1)).collect();
    move |offset| starts.partition_point(|s| *s <= offset).max(1)
}

fn join(a: Range, b: Range) -> Range {
    let from = a.from.min(b.from);
    let to = a.to.max(b.to);
    // One inside the other keeps the outer one's extent, and the name of whichever is a whole function. Two that
    // only overlap are just lines of the file.
    let a_outer = a.from == from && a.to == to;
    let b_outer = b.from == from && b.to == to;
    if a_outer || b_outer {
        let (outer, inner) = if a_outer { (a, b) } else { (b, a) };
        let name = if outer.whole {
            outer.name.clone()
        } else if inner.whole {
            inner.name.clone()
        } else {
            outer.name.clone().or(inner.name.clone())
        };
        return Range { name, ..outer };
    }
    let name = if a.name == b.name { a.name } else { a.name.or(b.name) };
    Range { from, to, name, whole: false }
}

fn clip(line: &str) -> String {
    if line.encode_utf16().count() > MAX_LINE_CHARS {
        format!("{}…", line.chars().take(MAX_LINE_CHARS).collect::<String>())
    } else {
        line.to_string()
    }
}

fn blocks_for(side: &Side<'_>, marks: &[Mark], symbol: Option<&str>) -> Vec<CodeBlock> {
    let mut files: Vec<String> = Vec::new();
    for f in symbol.map(file_of).into_iter().chain(marks.iter().map(|m| m.file.as_str())) {
        if !files.iter().any(|x| x == f) {
            files.push(f.to_string());
        }
    }
    let mut out = Vec::new();
    let mut total = 0;
    for file in &files {
        let Some(text) = side.repo.text(file) else {
            continue;
        };
        let lines: Vec<&str> = text.split('\n').collect();
        let line_of = line_finder(&text);
        struct Sym {
            id: String,
            name: String,
            from: usize,
            to: usize,
        }
        let symbols: Vec<Sym> = side
            .facts
            .get(file)
            .map(|f| {
                f.symbols
                    .iter()
                    .filter(|s| s.kind != SymbolKind::Property)
                    .map(|s| Sym {
                        id: s.id.clone(),
                        name: s.name.clone(),
                        from: line_of(s.span.start),
                        to: line_of(s.span.start.max(s.span.end.saturating_sub(1))),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let here: Vec<&Mark> =
            marks.iter().filter(|m| &m.file == file && m.line >= 1 && m.line <= lines.len()).collect();
        let around = |line: usize, name: Option<String>| Range {
            from: line.saturating_sub(CONTEXT).max(1),
            to: lines.len().min(line + CONTEXT),
            name,
            whole: false,
        };

        let mut ranges: Vec<Range> = Vec::new();
        let named = symbol.filter(|s| file_of(s) == file).and_then(|s| symbols.iter().find(|x| x.id == s));
        if let Some(named) = named {
            if named.to - named.from < WHOLE_NAMED {
                ranges.push(Range { from: named.from, to: named.to, name: Some(named.name.clone()), whole: true });
            } else if !here.iter().any(|m| m.line >= named.from && m.line <= named.to) {
                // Too long to show whole: its opening lines, unless a cited line inside it says where to look.
                ranges.push(Range {
                    from: named.from,
                    to: named.from + 2 * CONTEXT,
                    name: Some(named.name.clone()),
                    whole: false,
                });
            }
        }
        for m in &here {
            // The smallest function (or class, or type) the line is in. One-liners say nothing more than the line.
            let inside = symbols
                .iter()
                .filter(|s| s.from <= m.line && m.line <= s.to && s.to - s.from >= 2)
                .min_by_key(|s| s.to - s.from);
            match inside {
                Some(s) if s.to - s.from < WHOLE_FOUND => {
                    ranges.push(Range { from: s.from, to: s.to, name: Some(s.name.clone()), whole: true })
                }
                _ => ranges.push(around(m.line, inside.map(|s| s.name.clone()))),
            }
        }
        ranges.sort_by(|a, b| a.from.cmp(&b.from).then(b.to.cmp(&a.to)));
        let mut merged: Vec<Range> = Vec::new();
        for r in ranges {
            match merged.last_mut() {
                Some(last) if r.from <= last.to + 1 => *last = join(last.clone(), r),
                _ => merged.push(r),
            }
        }
        for r in merged {
            if out.len() >= MAX_BLOCKS || total >= MAX_LINES {
                return out;
            }
            let to = r.to.min(r.from + (MAX_LINES - total) - 1);
            let in_range: Vec<&&Mark> = here.iter().filter(|m| m.line >= r.from && m.line <= to).collect();
            let mut cited: Vec<usize> = in_range.iter().map(|m| m.line).collect();
            cited.sort_unstable();
            cited.dedup();
            out.push(CodeBlock {
                file: file.clone(),
                name: r.name.clone(),
                whole: r.whole && to == r.to,
                start: r.from,
                lines: lines[r.from - 1..to.min(lines.len())].iter().map(|l| clip(l)).collect(),
                cited,
                notes: in_range
                    .iter()
                    .filter_map(|m| m.note.as_ref().map(|n| LineNote { line: m.line, note: n.clone() }))
                    .collect(),
            });
            total += to - r.from + 1;
        }
    }
    out
}

/// Where each citation is in that version of the code. Ones it doesn't have are counted, not shown.
fn locate<'e>(side: &Side<'_>, evidence: impl Iterator<Item = (&'e Evidence, Option<String>)>) -> (Vec<Mark>, usize) {
    let mut found = Vec::new();
    let mut missing = 0;
    for (e, note) in evidence {
        let c = check_evidence(side.repo, e, DEFAULT_LINE_WINDOW);
        match c.found_at {
            Some(line) if c.ok => found.push(Mark { file: e.file.clone(), line, note }),
            _ => missing += 1,
        }
    }
    (found, missing)
}

/// The code behind a box or arrow of a map. `Before` reads the commit a map of a change is compared with. `None`
/// when the map has no such box or arrow, or `Before` is asked of a map that has nothing to compare with.
pub fn code_of(
    map: &CodeMap,
    root: &Path,
    extractor: &dyn Extractor,
    id: &str,
    when: When,
) -> std::io::Result<Option<CodeView>> {
    let (evidence, symbol) = if let Some(n) = map.nodes.iter().find(|n| n.id == id) {
        (&n.evidence, n.symbol.as_deref())
    } else if let Some(e) = map.edges.iter().find(|e| e.id == id) {
        (&e.evidence, None)
    } else {
        return Ok(None);
    };
    let readers = Readers::new(map.since.as_deref(), root)?;
    let (now, base) = readers.sides(extractor);
    let side = match when {
        When::Now => Some(&now),
        When::Before => base.as_ref(),
    };
    let Some(side) = side else { return Ok(None) };
    let (found, missing) = locate(side, evidence.iter().map(|e| (e, None)));
    Ok(Some(CodeView { when, blocks: blocks_for(side, &found, symbol), missing }))
}

fn as_evidence(n: &CodeNote) -> Evidence {
    Evidence { file: n.file.clone(), line: n.line, text: n.text.clone() }
}

/// The notes that hold against the code as it is, each on the line where its text was found, and why the others
/// don't.
pub fn check_notes(root: &Path, notes: &[CodeNote]) -> std::io::Result<(Vec<CodeNote>, Vec<String>)> {
    let readers = Readers::new(None, root)?;
    let mut kept: Vec<CodeNote> = Vec::new();
    let mut dropped = Vec::new();
    for n in notes {
        let c = check_evidence(&readers.now, &as_evidence(n), DEFAULT_LINE_WINDOW);
        match c.found_at {
            Some(line) if c.ok => {
                if !kept.iter().any(|k| k.file == n.file && k.line == line) {
                    kept.push(CodeNote { line, ..n.clone() });
                }
            }
            _ => dropped.push(format!("{}:{}: {}", n.file, n.line, c.reason.as_deref().unwrap_or("not found"))),
        }
    }
    Ok((kept, dropped))
}

/// The code an answer's notes point at, checked again against the code as it is now.
pub fn code_for_notes(root: &Path, extractor: &dyn Extractor, notes: &[CodeNote]) -> std::io::Result<CodeView> {
    let readers = Readers::new(None, root)?;
    let (now, _) = readers.sides(extractor);
    let evidence: Vec<(Evidence, String)> = notes.iter().map(|n| (as_evidence(n), n.note.clone())).collect();
    let (found, missing) = locate(&now, evidence.iter().map(|(e, n)| (e, Some(n.clone()))));
    Ok(CodeView { when: When::Now, blocks: blocks_for(&now, &found, None), missing })
}
