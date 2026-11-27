//! What the model must return, as JSON Schema. The schemas are the original's own (`MAP_SCHEMA`, `MAP_PATCH_SCHEMA`,
//! `ANSWER_SCHEMA`, `QUICK_ANSWER_SCHEMA` in `packages/server/src/schemas.ts`, derived there from the map format's zod
//! schemas), written out by `tools/record-schemas` (MIT, Copyright (c) 2026 johnz4021; see NOTICE). Output handed in
//! through the bridge is checked against the schema the model was shown, before the workflow reads it again with
//! `parse_map` or `patch_map`.

use std::sync::{Arc, LazyLock};

use serde_json::Value;

use crate::bridge::Validator;

/// One output schema: what the model is shown, and the validator built from it.
pub struct Schema {
    pub json: Arc<Value>,
    validator: jsonschema::Validator,
}

impl Schema {
    fn load(text: &str) -> Self {
        let json: Value = serde_json::from_str(text).expect("an embedded schema is JSON");
        let validator = jsonschema::validator_for(&json).expect("an embedded schema is a valid JSON Schema");
        Schema { json: Arc::new(json), validator }
    }

    /// The problems with `output`, at most 12, as "path: message" (the path dotted, `(output)` for the whole).
    pub fn problems(&self, output: &Value) -> Vec<String> {
        self.validator
            .iter_errors(output)
            .take(12)
            .map(|e| {
                let at = e.instance_path().to_string();
                let path = at.trim_start_matches('/').replace('/', ".");
                let path = if path.is_empty() { "(output)".to_string() } else { path };
                format!("{path}: {}", e.masked())
            })
            .collect()
    }

    /// A validator for a bridge request.
    pub fn validator(&'static self) -> Validator {
        Arc::new(move |v: &Value| {
            let problems = self.problems(v);
            if problems.is_empty() {
                Ok(())
            } else {
                Err(problems)
            }
        })
    }
}

/// A whole map, as the model writes it (only the plain explanation on boxes).
pub static MAP: LazyLock<Schema> = LazyLock::new(|| Schema::load(include_str!("../schemas/map.schema.json")));
/// A repair round's changes: corrected or new items, and ids to drop (applied with `patch_map`).
pub static MAP_PATCH: LazyLock<Schema> =
    LazyLock::new(|| Schema::load(include_str!("../schemas/map-patch.schema.json")));
/// An answer to a question on a map.
pub static ANSWER: LazyLock<Schema> = LazyLock::new(|| Schema::load(include_str!("../schemas/answer.schema.json")));
/// The first, cheap try at an answer, from the map alone: answer, go to the code, or say it's elsewhere.
pub static QUICK_ANSWER: LazyLock<Schema> =
    LazyLock::new(|| Schema::load(include_str!("../schemas/quick-answer.schema.json")));
