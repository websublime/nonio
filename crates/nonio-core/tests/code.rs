//! The code behind a box, an arrow or an answer's notes: which lines are shown, which are marked, and that it is
//! read by the same rules as the check.
//! Ported from `packages/core/test/code.test.ts` of illuminate (MIT, Copyright (c) 2026 johnz4021).

mod common;

use std::path::Path;

use common::*;
use nonio_core::code::{check_notes, code_for_notes, code_of, CodeNote, CodeView, LineNote, When};
use nonio_core::format::CodeMap;
use serde_json::json;

fn view(map: &CodeMap, root: &Path, id: &str, when: When) -> Option<CodeView> {
    code_of(map, root, &Recorded::load(), id, when).unwrap()
}

fn map() -> CodeMap {
    load(&map_json())
}

const SERVICE: &str = "src/patients/service.ts";

#[test]
fn shows_the_whole_function_a_box_names_with_its_cited_line_marked() {
    let v = view(&map(), &app(), "s3", When::Now).unwrap();
    assert_eq!((v.when, v.missing, v.blocks.len()), (When::Now, 0, 1));
    let b = &v.blocks[0];
    assert_eq!(
        (b.file.as_str(), b.name.as_deref(), b.whole, b.start, b.cited.clone()),
        (SERVICE, Some("markConverted"), true, 4, vec![5])
    );
    assert!(b.lines[0].contains("export async function markConverted"));
    assert_eq!(b.lines.last().unwrap(), "}");
}

#[test]
fn shows_the_function_an_arrows_line_is_in_though_an_arrow_names_none() {
    let b = view(&map(), &app(), "e23", When::Now).unwrap().blocks.remove(0);
    assert_eq!(b.file, "src/server/routes.ts");
    assert_eq!(b.cited, vec![9]);
    assert_eq!(b.name.as_deref(), Some("convertPatient"));
    assert!(b.lines[9 - b.start].contains("markConverted(req.params.id)"));
}

#[test]
fn follows_a_cited_line_that_moved_the_way_the_check_does() {
    let dir = copy_app();
    write(dir.path(), SERVICE, &format!("// a comment\n// another\n{}", read(dir.path(), SERVICE)));
    let b = view(&map(), dir.path(), "s3", When::Now).unwrap().blocks.remove(0);
    assert_eq!((b.start, b.cited), (6, vec![7]));
}

#[test]
fn counts_a_citation_the_code_no_longer_has_and_still_shows_the_function() {
    let dir = copy_app();
    write(dir.path(), SERVICE, &read(dir.path(), SERVICE).replacen("db.update('patients'", "db.save('people'", 1));
    let v = view(&map(), dir.path(), "s3", When::Now).unwrap();
    assert_eq!(v.missing, 1);
    assert_eq!((v.blocks[0].name.as_deref(), v.blocks[0].cited.clone()), (Some("markConverted"), vec![]));
}

#[test]
fn shows_a_long_function_only_around_its_cited_line() {
    let dir = copy_app();
    let filler = (0..200).map(|i| format!("  const v{i} = {i};")).collect::<Vec<_>>().join("\n");
    write(dir.path(), SERVICE, &format!("export async function markConverted(patientId: string) {{\n{filler}\n  await db.update('patients', {{ id: patientId }});\n}}\n"));
    let m = variant(|m| {
        item(m, "nodes", "s3")["evidence"] = json!([{ "file": SERVICE, "line": 202, "text": "db.update('patients'" }])
    });
    let b = view(&m, dir.path(), "s3", When::Now).unwrap().blocks.remove(0);
    assert_eq!((b.name.as_deref(), b.whole, b.cited.clone()), (Some("markConverted"), false, vec![202]));
    assert!(b.lines.len() < 20);
}

#[test]
fn gives_the_lines_around_a_citation_in_a_file_the_facts_cant_read_as_code() {
    let dir = copy_app();
    let sol = (0..30)
        .map(|i| {
            if i == 14 {
                "    requests[id].status = Status.Pending;".to_string()
            } else {
                format!("    // line {}", i + 1)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    write(dir.path(), "Vault.sol", &sol);
    let m = variant(|m| {
        m["nodes"].as_array_mut().unwrap().push(json!({ "id": "sol", "area": "patients", "name": "Marks it pending", "evidence": [{ "file": "Vault.sol", "line": 15, "text": "status = Status.Pending" }] }))
    });
    let b = view(&m, dir.path(), "sol", When::Now).unwrap().blocks.remove(0);
    assert_eq!(
        (b.file.as_str(), b.name.clone(), b.whole, b.start, b.cited.clone()),
        ("Vault.sol", None, false, 9, vec![15])
    );
    assert_eq!(b.lines.len(), 13);
}

#[test]
fn has_nothing_for_an_unknown_id_and_never_reads_a_secret_file() {
    assert!(view(&map(), &app(), "nope", When::Now).is_none());
    let m = variant(|m| {
        m["nodes"].as_array_mut().unwrap().push(json!({ "id": "env", "area": "patients", "name": "The key", "evidence": [{ "file": ".env", "line": 1, "text": "KEY" }] }))
    });
    assert_eq!(view(&m, &app(), "env", When::Now), Some(CodeView { when: When::Now, blocks: vec![], missing: 1 }));
}

#[test]
fn reads_a_changed_function_from_the_commit_the_map_is_compared_with() {
    let dir = copy_app();
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "the care app"]);
    write(
        dir.path(),
        SERVICE,
        &read(dir.path(), SERVICE).replacen(
            "  bus.emit('patient.converted', { patientId });",
            "  console.log('converted', patientId);\n  bus.emit('patient.converted', { patientId });",
            1,
        ),
    );
    let m = variant(|m| m["since"] = json!("HEAD"));
    let now = view(&m, dir.path(), "s4", When::Now).unwrap().blocks.remove(0);
    let was = view(&m, dir.path(), "s4", When::Before).unwrap();
    assert!(now.lines.join("\n").contains("console.log"));
    assert_eq!(now.cited, vec![7]);
    assert_eq!(was.when, When::Before);
    assert!(!was.blocks[0].lines.join("\n").contains("console.log"));
    assert_eq!(was.blocks[0].cited, vec![6]);
}

#[test]
fn has_no_earlier_version_on_a_map_that_compares_with_nothing() {
    assert!(view(&map(), &app(), "s3", When::Before).is_none());
}

fn note(line: usize, text: &str, file: &str) -> CodeNote {
    CodeNote { file: file.into(), line, text: text.into(), note: "What this line does.".into() }
}

#[test]
fn keeps_the_notes_whose_line_is_in_the_code_on_the_line_where_it_was_found() {
    let (notes, dropped) = check_notes(
        &app(),
        &[
            note(5, "db.update('patients'", SERVICE),
            note(1, "bus.emit('patient.converted'", SERVICE),
            note(3, "no such text", SERVICE),
        ],
    )
    .unwrap();
    assert_eq!(notes.iter().map(|n| n.line).collect::<Vec<_>>(), vec![5, 6]);
    assert_eq!(dropped.len(), 1);
    assert!(dropped[0].contains("src/patients/service.ts:3"));
}

#[test]
fn drops_a_second_note_on_the_same_line_and_a_note_on_a_secret_file() {
    let (notes, dropped) = check_notes(
        &app(),
        &[note(5, "db.update('patients'", SERVICE), note(5, "status:", SERVICE), note(1, "KEY", ".env")],
    )
    .unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(dropped, vec![".env:1: secret files are never read"]);
}

#[test]
fn shows_the_code_with_each_note_on_its_line_and_counts_notes_the_code_no_longer_has() {
    let v = code_for_notes(
        &app(),
        &Recorded::load(),
        &[note(6, "bus.emit('patient.converted'", SERVICE), note(2, "gone", SERVICE)],
    )
    .unwrap();
    assert_eq!((v.missing, v.blocks.len()), (1, 1));
    let b = &v.blocks[0];
    assert_eq!((b.name.as_deref(), b.cited.clone()), (Some("markConverted"), vec![6]));
    assert_eq!(b.notes, vec![LineNote { line: 6, note: "What this line does.".into() }]);
}

#[test]
fn keeps_the_functions_name_on_the_block_that_holds_both_it_and_a_line_outside_it() {
    // s2 names convertPatient (lines 8-11) and cites the route on line 6: one block, still named.
    let v = view(&map(), &app(), "s2", When::Now).unwrap();
    assert_eq!(v.blocks.len(), 1);
    let b = &v.blocks[0];
    assert_eq!((b.name.as_deref(), b.whole, b.cited.clone()), (Some("convertPatient"), false, vec![6]));
}
