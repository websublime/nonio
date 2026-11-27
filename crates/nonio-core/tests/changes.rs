//! A map of a change (`since`): the checker compares the code with a commit and says what is new, different or gone.
//! The care app is committed, then changed the way a fix would change it: a listener is added, the service is
//! edited, and the nightly job is deleted.
//! Ported from `packages/core/test/changes.test.ts` of illuminate (MIT, Copyright (c) 2026 johnz4021).

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use common::*;
use indexmap::IndexMap;
use nonio_core::base::{line_diff, resolve_commit, BaseReader};
use nonio_core::check::{ChangeCount, ClaimCheck, ProposalCount, SinceCheck, Summary};
use nonio_core::format::{parse_map, CodeMap};
use nonio_core::repo::Reader;
use nonio_core::{Proof, Status};
use serde_json::{json, Value};

/// The care app in a new git repo, committed as it is.
fn committed_app() -> (tempfile::TempDir, String) {
    let dir = copy_app();
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "the care app"]);
    let first = git(dir.path(), &["rev-parse", "HEAD"]);
    (dir, first)
}

/// The change: something now listens for the announcement, the service says so, and the nightly job is gone.
fn change(dir: &Path) {
    write(
        dir,
        "src/automations/listeners.ts",
        "import { bus } from '../events/bus';\nimport { evaluateRules } from './evaluate';\n\nexport function listenForConversions() {\n  bus.on('patient.converted', ({ patientId }) => evaluateRules({ patientId }));\n}\n",
    );
    let service = "src/patients/service.ts";
    let text = read(dir, service).replacen(
        "  bus.emit('patient.converted', { patientId });",
        "  console.log('converted', patientId);\n  bus.emit('patient.converted', { patientId });",
        1,
    );
    write(dir, service, &text);
    fs::remove_file(dir.join("src/jobs/nightly.ts")).unwrap();
}

/// The care-app map as a map of that change.
fn change_map(since: Option<&str>, edit: impl FnOnce(&mut Value)) -> CodeMap {
    let mut m = map_json();
    if let Some(s) = since {
        m["since"] = json!(s);
    }
    m["absences"] = json!([]);
    m["tour"] = json!([]);
    m["nodes"].as_array_mut().unwrap().push(json!({ "id": "l1", "area": "automations", "name": "Hears the announcement", "symbol": "src/automations/listeners.ts#listenForConversions", "evidence": [{ "file": "src/automations/listeners.ts", "line": 5, "text": "bus.on('patient.converted'" }] }));
    m["edges"].as_array_mut().unwrap().push(json!({ "id": "el", "from": "l1", "to": "u1", "kind": "calls", "label": "runs the rules", "evidence": [{ "file": "src/automations/listeners.ts", "line": 5, "text": "evaluateRules({ patientId })" }] }));
    item(&mut m, "edges", "e34")["evidence"][0]["line"] = json!(7);
    item(&mut m, "nodes", "j1")["removed"] = json!(true);
    item(&mut m, "edges", "ej")["removed"] = json!(true);
    edit(&mut m);
    load(&m)
}

fn kinds(group: &IndexMap<String, ClaimCheck>) -> BTreeMap<String, &'static str> {
    use nonio_core::check::ChangeKind::*;
    group
        .iter()
        .filter_map(|(id, c)| {
            c.change.as_ref().map(|x| {
                (
                    id.clone(),
                    match x.kind {
                        Added => "added",
                        Changed => "changed",
                        Removed => "removed",
                    },
                )
            })
        })
        .collect()
}

fn kv(pairs: &[(&str, &'static str)]) -> BTreeMap<String, &'static str> {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

fn changed_app() -> (tempfile::TempDir, String) {
    let (dir, first) = committed_app();
    change(dir.path());
    (dir, first)
}

#[test]
fn says_what_it_is_compared_with() {
    let (dir, first) = changed_app();
    let since = run(&change_map(Some("HEAD"), |_| {}), dir.path()).since.unwrap();
    assert_eq!(
        (since.reference.as_str(), since.commit.as_deref(), since.subject.as_deref()),
        ("HEAD", Some(first.as_str()), Some("the care app"))
    );
    let date = since.date.unwrap();
    assert!(date.len() >= 10 && date.as_bytes()[4] == b'-' && date.as_bytes()[7] == b'-', "{date}");
}

#[test]
fn marks_what_is_new_different_and_gone_and_leaves_the_rest_alone() {
    let (dir, _) = changed_app();
    let report = run(&change_map(Some("HEAD"), |_| {}), dir.path());
    assert_eq!(kinds(&report.nodes), kv(&[("l1", "added"), ("s3", "changed"), ("s4", "changed"), ("j1", "removed")]));
    assert_eq!(kinds(&report.edges), kv(&[("el", "added"), ("ej", "removed")]));
    assert_eq!(
        report.summary,
        Summary {
            claims: 16,
            confirmed: 16,
            unconfirmed: 0,
            proposals: ProposalCount::default(),
            changes: Some(ChangeCount { added: 2, changed: 2, removed: 2 })
        }
    );
    // The lines of a changed function.
    let diff = report.nodes["s3"].change.as_ref().unwrap().diff.clone().unwrap();
    assert!(diff.contains(&"+   console.log('converted', patientId);".to_string()), "{diff:?}");
    assert!(!diff.iter().any(|l| l.starts_with("- ")));
    // Removed code is confirmed against the code as it was.
    let j1 = &report.nodes["j1"];
    assert_eq!(
        (j1.status, j1.proof, j1.evidence[0].ok, j1.evidence[0].found_at),
        (Status::Confirmed, Some(Proof::Fact), true, Some(5))
    );
    assert_eq!((report.edges["ej"].status, report.edges["ej"].proof), (Status::Confirmed, Some(Proof::Fact)));
}

#[test]
fn does_not_take_the_maps_word_that_something_was_removed() {
    let (dir, _) = changed_app();
    let r = run(
        &change_map(Some("HEAD"), |m| {
            item(m, "nodes", "u3")["removed"] = json!(true);
            item(m, "edges", "eu23")["removed"] = json!(true);
        }),
        dir.path(),
    );
    for c in [&r.nodes["u3"], &r.edges["eu23"]] {
        assert_eq!(c.status, Status::Unconfirmed);
        assert_eq!(c.reasons.len(), 1);
        assert!(c.reasons[0].contains("still in the code"), "{:?}", c.reasons);
    }
    assert!(r.nodes["u3"].change.is_none());
}

#[test]
fn does_not_confirm_removed_code_that_was_never_there() {
    let (dir, _) = changed_app();
    let r = run(
        &change_map(Some("HEAD"), |m| item(m, "nodes", "j1")["symbol"] = json!("src/jobs/nightly.ts#runHourly")),
        dir.path(),
    );
    assert_eq!(r.nodes["j1"].status, Status::Unconfirmed);
    assert!(
        r.nodes["j1"].reasons[0].contains("in the code as it was: the code has no symbol"),
        "{:?}",
        r.nodes["j1"].reasons
    );
}

#[test]
fn goes_by_cited_lines_for_a_box_that_names_no_function() {
    let (dir, _) = changed_app();
    let r = run(
        &change_map(Some("HEAD"), |m| {
            item(m, "nodes", "l1").as_object_mut().unwrap().remove("symbol"); // every cited file is new
            let s4 = item(m, "nodes", "s4");
            s4.as_object_mut().unwrap().remove("symbol");
            s4["evidence"] =
                json!([{ "file": "src/patients/service.ts", "line": 6, "text": "console.log('converted'" }]); // a new line in an old file
            item(m, "nodes", "u3").as_object_mut().unwrap().remove("symbol"); // untouched
            item(m, "edges", "el")["kind"] = json!("then");
        }),
        dir.path(),
    );
    let k = kinds(&r.nodes);
    assert_eq!((k["l1"], k["s4"]), ("added", "changed"));
    assert!(r.nodes["u3"].change.is_none());
    assert_eq!(r.edges["el"].change.as_ref().map(|c| c.kind), Some(nonio_core::check::ChangeKind::Added));
    assert!(r.edges["e34"].change.is_none()); // its line was there before
}

#[test]
fn keeps_comparing_with_the_same_commit_after_the_change_is_committed() {
    let (dir, first) = committed_app();
    change(dir.path());
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "listen for conversions"]);
    assert!(kinds(&run(&change_map(Some("HEAD"), |_| {}), dir.path()).nodes).is_empty()); // nothing has changed since the new commit
    let r = run(&change_map(Some(&first), |_| {}), dir.path());
    assert_eq!(kinds(&r.nodes), kv(&[("l1", "added"), ("s3", "changed"), ("s4", "changed"), ("j1", "removed")]));
    assert_eq!(run(&change_map(Some("HEAD~1"), |_| {}), dir.path()).since.unwrap().commit, Some(first));
}

#[test]
fn says_so_when_the_commit_cant_be_found_and_marks_nothing() {
    let (dir, _) = changed_app();
    let r = run(&change_map(Some("no-such-branch"), |_| {}), dir.path());
    assert_eq!(
        r.since,
        Some(SinceCheck { reference: "no-such-branch".into(), commit: None, date: None, subject: None })
    );
    assert_eq!(r.nodes["j1"].status, Status::Unconfirmed);
    assert!(r.nodes["j1"].reasons[0].contains("isn't a commit in this repo"));
    assert!(kinds(&r.nodes).is_empty());
    assert_eq!(r.nodes["l1"].status, Status::Confirmed);
}

#[test]
fn marks_nothing_on_a_map_that_isnt_about_a_change() {
    let (dir, _) = changed_app();
    let r = run(
        &change_map(None, |m| {
            for list in ["nodes", "edges"] {
                for x in m[list].as_array_mut().unwrap() {
                    x.as_object_mut().unwrap().remove("removed");
                }
            }
            m["nodes"].as_array_mut().unwrap().retain(|n| n["id"] != "j1");
            m["edges"].as_array_mut().unwrap().retain(|e| e["id"] != "ej");
        }),
        dir.path(),
    );
    assert!(r.since.is_none());
    assert!(r.summary.changes.is_none());
    assert!(kinds(&r.nodes).is_empty());
}

fn problems(edit: impl FnOnce(&mut Value)) -> Vec<String> {
    let mut m = map_json();
    edit(&mut m);
    parse_map(&m).err().unwrap_or_default()
}

fn one_containing(problems: &[String], text: &str) {
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains(text), "{problems:?}");
}

#[test]
fn the_format_needs_a_commit_for_anything_to_have_been_removed_after() {
    let p = problems(|m| {
        m["nodes"][7]["removed"] = json!(true);
        item(m, "edges", "ej")["removed"] = json!(true);
    });
    assert_eq!(p.len(), 2);
    assert!(p.iter().all(|x| x.contains("needs the map's `since`")), "{p:?}");
}

#[test]
fn the_format_removes_the_arrows_of_a_removed_box_and_keeps_gaps_about_the_code_as_it_is() {
    one_containing(
        &problems(|m| {
            m["since"] = json!("HEAD");
            item(m, "nodes", "j1")["removed"] = json!(true);
        }),
        "an arrow to or from it was removed too",
    );
    one_containing(
        &problems(|m| {
            m["since"] = json!("HEAD");
            item(m, "nodes", "s4")["removed"] = json!(true);
            item(m, "edges", "e34")["removed"] = json!(true);
        }),
        "an absence is about the code as it is",
    );
}

#[test]
fn the_format_only_takes_a_name_git_can_read_as_a_revision() {
    for since in ["HEAD", "main", "HEAD~2", "origin/main", "v1.2.0", "a1b2c3d"] {
        assert_eq!(problems(|m| m["since"] = json!(since)), Vec::<String>::new(), "{since}");
    }
    for since in ["--output=/tmp/x", "-n", "a b", "HEAD; rm -rf .", ""] {
        assert_eq!(problems(|m| m["since"] = json!(since)).len(), 1, "{since}");
    }
}

#[test]
fn the_format_cant_have_removed_and_proposed_at_once() {
    one_containing(
        &problems(|m| {
            m["since"] = json!("HEAD");
            let u3 = item(m, "nodes", "u3");
            u3["removed"] = json!(true);
            u3["proposed"] = json!({ "kind": "remove" });
            item(m, "edges", "eu23")["removed"] = json!(true);
        }),
        "both `removed`",
    );
}

#[test]
fn reads_a_commits_files_including_ones_deleted_since_and_not_ones_added_since() {
    let (dir, first) = changed_app();
    let base = BaseReader::new(dir.path(), "HEAD").unwrap();
    assert_eq!(base.commit(), first);
    assert!(base.text("src/jobs/nightly.ts").unwrap().contains("runNightly"));
    assert!(!base.text("src/patients/service.ts").unwrap().contains("console.log"));
    assert!(!base.has("src/automations/listeners.ts"));
    assert_eq!(base.read("src/automations/listeners.ts"), Err("file not found".into()));
}

#[test]
fn never_reads_secrets_or_paths_outside_the_repo_even_from_history() {
    let (dir, _) = changed_app();
    let base = BaseReader::new(dir.path(), "HEAD").unwrap();
    assert!(base.has(".env"));
    assert_eq!(base.read(".env"), Err("secret files are never read".into()));
    assert!(base.read("../outside.ts").is_err());
}

#[test]
fn resolves_only_real_commits_and_nothing_that_looks_like_an_option() {
    let (dir, first) = changed_app();
    assert_eq!(resolve_commit(dir.path(), "HEAD"), Some(first));
    assert_eq!(resolve_commit(dir.path(), "nope"), None);
    assert_eq!(resolve_commit(dir.path(), "--all"), None);
    let plain = tempfile::tempdir().unwrap();
    assert!(BaseReader::new(plain.path(), "HEAD").is_none()); // not a git repo
}

#[test]
fn line_diff_lists_added_and_removed_lines_with_a_little_of_what_surrounds_them() {
    let before = ["a", "b", "c", "d", "e", "f", "g", "h"].join("\n");
    let after = ["a", "b", "c", "D", "e", "f", "g", "h", "i"].join("\n");
    assert_eq!(line_diff(&before, &after, 1, 60).unwrap(), vec!["  c", "- d", "+ D", "  e", "…", "  h", "+ i"]);
}

#[test]
fn line_diff_is_empty_for_the_same_text_and_gives_up_on_texts_too_long_to_compare() {
    assert_eq!(line_diff("a\nb", "a\nb", 2, 60), Some(vec![]));
    let long = |seed: &str| (0..1100).map(|i| format!("{seed}{i}")).collect::<Vec<_>>().join("\n");
    assert_eq!(line_diff(&long("x"), &long("y"), 2, 60), None);
}
