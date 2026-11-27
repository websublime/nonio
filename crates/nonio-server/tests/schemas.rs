//! The output schemas, recorded from the original, and how output is checked against them.

use std::path::Path;

use nonio_server::schemas::{ANSWER, MAP, MAP_PATCH, QUICK_ANSWER};
use serde_json::{json, Value};

fn care_map() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../nonio-core/tests/fixtures/care-app.map.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn takes_the_care_app_map_and_says_what_is_wrong_with_one_that_is_not() {
    assert_eq!(MAP.problems(&care_map()), Vec::<String>::new());
    let mut broken = care_map();
    broken["nodes"][0].as_object_mut().unwrap().remove("name");
    broken["edges"][1]["kind"] = json!("maybe");
    let problems = MAP.problems(&broken);
    assert!(problems.iter().any(|p| p.starts_with("nodes.0: ")), "{problems:?}");
    assert!(problems.iter().any(|p| p.starts_with("edges.1.kind: ")), "{problems:?}");
    assert_eq!(MAP.problems(&json!("text")).len(), 1);
    assert!(MAP.problems(&json!("text"))[0].starts_with("(output): "));
}

#[test]
fn leaves_out_what_the_original_strips_so_structured_output_takes_them() {
    for s in [&MAP, &MAP_PATCH, &ANSWER, &QUICK_ANSWER] {
        let text = s.json.to_string();
        for k in ["\"$schema\"", "\"$id\"", "\"pattern\"", "\"default\"", "\"format\""] {
            assert!(!text.contains(&format!("{k}:")), "{k}");
        }
    }
}

#[test]
fn takes_an_empty_patch_and_answers_with_their_defaults_left_out() {
    assert!(MAP_PATCH.problems(&json!({})).is_empty());
    assert!(ANSWER.problems(&json!({ "concept": "It goes out at night." })).is_empty());
    assert!(!ANSWER.problems(&json!({ "concept": "" })).is_empty());
    assert!(QUICK_ANSWER.problems(&json!({ "route": "code", "missing": "the job" })).is_empty());
    assert!(!QUICK_ANSWER.problems(&json!({ "route": "maybe" })).is_empty());
}

#[test]
fn checks_output_handed_in_through_the_bridge() {
    let v = MAP.validator();
    assert!(v(&care_map()).is_ok());
    assert!(v(&json!({ "version": 1 })).is_err());
}
