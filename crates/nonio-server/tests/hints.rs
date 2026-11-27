//! The head start, ported from `packages/server/test/hints.test.ts`, and compared with the original's on the care
//! app (`tests/fixtures/prompts/head-start.txt`).

mod common;

use std::fs;

use common::*;
use nonio_server::hints::head_start;
use pretty_assertions::assert_eq;

#[test]
fn is_the_originals_on_the_care_app() {
    let root = copy_care_app();
    assert_eq!(head_start(root.path()).text, fs::read_to_string(fixtures().join("prompts/head-start.txt")).unwrap());
}

#[test]
fn leaves_tests_and_fixtures_out_of_every_section() {
    let root = copy_care_app();
    fs::create_dir_all(root.path().join("test/fixtures/copy")).unwrap();
    fs::copy(root.path().join("src/server/routes.ts"), root.path().join("test/fixtures/copy/routes.ts")).unwrap();
    let text = head_start(root.path()).text;
    assert!(text.contains("src/server/routes.ts#convertPatient"));
    assert!(!text.contains("test/fixtures"));
}

#[test]
fn lists_function_names_in_a_small_repo_and_says_why_it_does_not_in_a_big_one() {
    let root = copy_care_app();
    assert!(head_start(root.path()).text.contains("Functions by file (use these exact names"));
    fs::create_dir_all(root.path().join("src/many")).unwrap();
    for i in 0..160 {
        fs::write(root.path().join(format!("src/many/f{i}.ts")), format!("export function f{i}() {{ return {i}; }}\n"))
            .unwrap();
    }
    let text = head_start(root.path()).text;
    assert!(text.contains("Functions by file: not listed"));
    assert!(!text.contains("src/many/f1.ts"));
    assert!(text.contains("src/server/routes.ts#convertPatient"));
}
