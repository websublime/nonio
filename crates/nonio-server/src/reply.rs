//! An answer as it is posted in the agent's chat. Where the person asks in the chat beside the map (every host
//! without `wake`), the reply belongs there too. The answer still goes through nonio first: its references and its
//! notes on lines of code are checked, and the map lights up what it is about. What the agent gets back to post is the
//! checked answer, with the code its notes point at read again now, as Markdown code blocks with line numbers. The
//! agent is told to post it unchanged; that is an instruction, not something nonio can enforce.
//!
//! Ported from `packages/server/src/reply.ts` (MIT, Copyright (c) 2026 johnz4021); see NOTICE.

use std::path::Path;

use nonio_core::code::{code_for_notes, CodeBlock};
use nonio_core::repo::{list_repo_files, DEFAULT_IGNORE};
use nonio_scan::Scanner;

use crate::explainer::{Answer, RefState};

/// Lines kept around the noted lines of a block, and the most lines one block shows in the chat.
const AROUND: usize = 3;
const MAX_LINES: usize = 30;
const MAX_BLOCKS: usize = 4;

fn lang(file: &str) -> &'static str {
    let ext = file.rsplit_once('.').map_or("", |(_, e)| e);
    match ext {
        "ts" => "ts",
        "tsx" => "tsx",
        "js" | "mjs" | "cjs" => "js",
        "jsx" => "jsx",
        "py" => "python",
        "go" => "go",
        "rs" => "rust",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "vue" => "vue",
        "rb" => "ruby",
        "sol" => "solidity",
        "css" => "css",
        "html" => "html",
        "json" => "json",
        "sql" => "sql",
        "sh" => "bash",
        "yml" | "yaml" => "yaml",
        "md" => "markdown",
        _ => "",
    }
}

/// The part of a block worth showing in a chat: the noted lines and a few around them, at most `MAX_LINES`.
fn excerpt(b: &CodeBlock) -> (usize, &[String]) {
    let marked: Vec<usize> =
        if b.notes.is_empty() { b.cited.clone() } else { b.notes.iter().map(|n| n.line).collect() };
    let end = b.start + b.lines.len().saturating_sub(1);
    let (from, to) = match (marked.iter().min(), marked.iter().max()) {
        (Some(&lo), Some(&hi)) => {
            let from = lo.saturating_sub(AROUND).max(b.start);
            let to = (hi + AROUND).min(end).min(from + MAX_LINES - 1);
            (from, to)
        }
        _ => (b.start, end.min(b.start + MAX_LINES - 1)),
    };
    (from, &b.lines[from - b.start..=(to - b.start).min(b.lines.len().saturating_sub(1))])
}

/// One block as Markdown: where it is, the lines with their numbers, then each note under the line it's about.
fn block_text(b: &CodeBlock) -> String {
    let (start, lines) = excerpt(b);
    let last = start + lines.len() - 1;
    let width = last.to_string().len();
    let numbered: Vec<String> = lines.iter().enumerate().map(|(i, l)| format!("{:>width$}  {l}", start + i)).collect();
    let name = b.name.as_ref().map(|n| format!(" · {n}")).unwrap_or_default();
    let mut out = vec![
        format!("`{}`{name} (lines {start}–{last})", b.file),
        format!("```{}", lang(&b.file)),
        numbered.join("\n"),
        "```".into(),
    ];
    out.extend(
        b.notes.iter().filter(|n| n.line >= start && n.line <= last).map(|n| format!("- Line {}: {}", n.line, n.note)),
    );
    out.join("\n")
}

/// The checked answer, ready for the chat: the reply, the code its notes point at, and what the map has to act on.
pub fn chat_reply(root: &Path, a: &Answer) -> String {
    let mut parts = vec![a.concept.trim().to_string()];
    let unconfirmed: Vec<&str> =
        a.refs.iter().filter(|r| r.status == RefState::Unconfirmed).map(|r| r.id.as_str()).collect();
    if !unconfirmed.is_empty() {
        parts.push(format!("_Not confirmed against the code: {}._", unconfirmed.join(", ")));
    }
    if !a.notes.is_empty() {
        let scanner = Scanner::new(root, list_repo_files(root, DEFAULT_IGNORE));
        if let Ok(view) = code_for_notes(root, &scanner, &a.notes) {
            let blocks: Vec<String> = view.blocks.iter().take(MAX_BLOCKS).map(block_text).collect();
            if !blocks.is_empty() {
                parts.push(blocks.join("\n\n"));
            }
            if view.missing > 0 {
                parts.push(format!(
                    "_{} of the lines it points at aren't in the code any more, so they're left out._",
                    view.missing
                ));
            }
        }
    }
    if a.fix.is_some() {
        parts.push("A fix is proposed on the map: approve or reject it there.".into());
    }
    parts.join("\n\n")
}
