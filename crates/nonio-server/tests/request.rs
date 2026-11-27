//! The request text and its parts. `parts_of` is ported from "cutting a request into parts" in
//! `packages/server/test/channel.test.ts`; the rest is new.

use std::sync::Arc;

use nonio_server::bridge::BridgeRequest;
use nonio_server::request::{parts_of, request_text};
use serde_json::json;

#[test]
fn cuts_at_line_ends_and_cuts_a_line_only_when_it_is_longer_than_a_part() {
    let t = ["a".repeat(5), "b".repeat(5), "c".repeat(12), "d".into()].join("\n");
    let parts = parts_of(&t, 11);
    assert_eq!(parts, ["aaaaa\nbbbbb", "ccccccccccc", "c\nd"]);
    assert!(parts.iter().all(|p| p.chars().count() <= 11));
    assert_eq!(parts_of("short", 100), ["short"]);
}

#[test]
fn counts_characters_not_bytes() {
    assert_eq!(parts_of("ééé\nà", 3), ["ééé", "à"]);
}

fn request(tools: Vec<String>, continues: bool) -> BridgeRequest {
    BridgeRequest {
        id: "r2".into(),
        label: "Making the map for \"how\"".into(),
        prompt: "Draw the map.".into(),
        schema: Arc::new(json!({ "type": "object" })),
        tools,
        continues,
    }
}

#[test]
fn says_what_the_request_is_how_to_hand_it_in_the_prompt_and_the_schema() {
    let t = request_text(&request(vec!["Read".into()], false));
    assert_eq!(
        t,
        [
            "## nonio request r2: Making the map for \"how\"",
            "",
            "When you're done, call nonio_submit with request_id \"r2\" and `output`: a JSON object matching the schema at the end. Don't edit any files.",
            "",
            "---",
            "Draw the map.",
            "---",
            "",
            "## Output schema (JSON Schema)",
            "{\"type\":\"object\"}",
        ]
        .join("\n")
    );
}

#[test]
fn says_when_to_answer_without_the_code_and_when_it_follows_on() {
    let t = request_text(&request(vec![], true));
    assert!(t.contains("\nAnswer from what is in this request alone. Don't read the code for this one.\nThis follows on from your earlier work for nonio in this conversation.\n\n---"));
}
