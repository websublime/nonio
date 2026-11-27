//! A question about a walkthrough step starts by naming the step, so the model knows it and the page shows it as a
//! chip under the question. The page writes this for its own question box (the viewer's `step.ts`, which also reads
//! it back); this is the same wording for a question asked in the agent's chat while a step is on screen. Keep the two
//! the same.
//!
//! Ported from `packages/server/src/step.ts` (MIT, Copyright (c) 2026 johnz4021); see NOTICE.

pub fn ask_about_step(step: &str, question: &str) -> String {
    format!("About the walkthrough step “{step}”: {question}")
}
