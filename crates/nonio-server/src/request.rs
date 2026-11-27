//! A request as the host's model reads it in a tool result, and how a long one is cut into parts for hosts that keep
//! only so much of a tool result (`max_output` in the host profile; read with `nonio_more`).
//!
//! Ported from `requestText` and `partsOf` in `packages/server/src/channel.ts` (MIT, Copyright (c) 2026 johnz4021);
//! see NOTICE.

use crate::bridge::BridgeRequest;
use crate::tools::SUBMIT;

/// What to do, how to answer, the prompt, and the output schema.
pub fn request_text(r: &BridgeRequest) -> String {
    let id = &r.id;
    let lines = [
        format!("## nonio request {id}: {}", r.label),
        String::new(),
        format!(
            "When you're done, call {SUBMIT} with request_id \"{id}\" and `output`: a JSON object matching the schema at the end. Don't edit any files."
        ),
        if r.tools.is_empty() { "Answer from what is in this request alone. Don't read the code for this one.".into() } else { String::new() },
        if r.continues { "This follows on from your earlier work for nonio in this conversation.".into() } else { String::new() },
        String::new(),
        "---".into(),
        r.prompt.clone(),
        "---".into(),
        String::new(),
        "## Output schema (JSON Schema)".into(),
        r.schema.to_string(),
    ];
    // No two blank lines in a row.
    let mut out: Vec<&str> = Vec::new();
    for line in &lines {
        if line.is_empty() && out.last().is_some_and(|l| l.is_empty()) {
            continue;
        }
        out.push(line);
    }
    out.join("\n")
}

/// A long text cut into parts of at most `max` characters, at line ends where it can be (a line longer than a part,
/// such as a big output schema, is cut where it must be). Joined back with newlines it is the text again, apart from
/// where a long line was cut.
pub fn parts_of(t: &str, max: usize) -> Vec<String> {
    assert!(max > 0, "a part holds at least one character");
    let mut parts: Vec<String> = Vec::new();
    let mut cur: Option<(String, usize)> = None;
    for line in t.split('\n') {
        let mut rest: Vec<char> = line.chars().collect();
        while rest.len() > max {
            if let Some((c, _)) = cur.take() {
                parts.push(c);
            }
            parts.push(rest[..max].iter().collect());
            rest.drain(..max);
        }
        let len = rest.len();
        let rest: String = rest.into_iter().collect();
        cur = match cur.take() {
            Some((c, n)) if n + 1 + len > max => {
                parts.push(c);
                Some((rest, len))
            }
            Some((c, n)) => Some((format!("{c}\n{rest}"), n + 1 + len)),
            None => Some((rest, len)),
        };
    }
    if let Some((c, _)) = cur {
        parts.push(c);
    }
    parts
}
