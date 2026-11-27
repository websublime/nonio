//! An answer as a chat-only host posts it: the checked reply, the code its notes point at, and what the map has to
//! act on. Ported from `packages/server/test/reply.test.ts`.

mod common;

use common::*;
use nonio_core::code::CodeNote;
use nonio_server::explainer::{Answer, Moves, RefState, RefStatus, Selection, Source};
use nonio_server::fixes::{FixProposal, RawFix};
use nonio_server::reply::chat_reply;

fn answer() -> Answer {
    Answer {
        id: "a1".into(),
        question: "q".into(),
        selection: Selection::default(),
        concept: "It announces the conversion.".into(),
        refs: vec![],
        moves: Moves::default(),
        added: vec![],
        proposed: vec![],
        screens: vec![],
        notes: vec![],
        problems: vec![],
        fix: None,
        source: Source::Map,
        offer: None,
        cost: 0.0,
    }
}

fn note(line: usize, text: &str, note: &str) -> CodeNote {
    CodeNote { file: "src/patients/service.ts".into(), line, text: text.into(), note: note.into() }
}

#[test]
fn is_just_the_answer_when_it_points_at_no_code() {
    let dir = copy_care_app();
    assert_eq!(chat_reply(dir.path(), &answer()), "It announces the conversion.");
}

#[test]
fn shows_the_lines_a_note_points_at_with_a_few_around_them_and_the_note_under_them() {
    let dir = copy_care_app();
    let a = Answer { notes: vec![note(5, "await db.update('patients'", "Saved as enrolled.")], ..answer() };
    let text = chat_reply(dir.path(), &a);
    assert!(text.contains("`src/patients/service.ts` · markConverted (lines 4–7)"), "{text}");
    assert!(text.contains("5    await db.update('patients'"), "{text}");
    assert!(text.contains("- Line 5: Saved as enrolled."), "{text}");
}

#[test]
fn says_what_is_unconfirmed_what_is_gone_and_that_a_fix_waits_on_the_map() {
    let dir = copy_care_app();
    // Any proposal will do: the reply only says that one is waiting.
    let fix: FixProposal = nonio_server::fixes::propose_fix(
        dir.path(),
        &care_map(),
        &serde_json::from_value::<RawFix>(listener_fix()).unwrap(),
    )
    .unwrap();
    let a = Answer {
        refs: vec![RefStatus { id: "miss".into(), status: RefState::Unconfirmed }],
        notes: vec![note(400, "nowhere", "Gone.")],
        fix: Some(fix),
        ..answer()
    };
    let text = chat_reply(dir.path(), &a);
    assert!(text.contains("Not confirmed against the code: miss"), "{text}");
    assert!(text.contains("aren't in the code any more"), "{text}");
    assert!(text.contains("A fix is proposed on the map: approve or reject it there."), "{text}");
}
