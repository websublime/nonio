//! The checker against the care-app fixture: a patient converts, the app announces it, and nothing listens, so the
//! automation rules (and the welcome email) only run from a nightly job.
//! Ported from `packages/core/test/check.test.ts` of illuminate (MIT, Copyright (c) 2026 johnz4021).

mod common;

use common::*;
use nonio_core::check::{ProposalCount, Summary, TourCheck, TourStatus};
use nonio_core::format::parse_map;
use nonio_core::screens::ScreenStatus;
use nonio_core::{Proof, Status};
use serde_json::json;

fn ok_tour() -> TourCheck {
    TourCheck { status: TourStatus::Ok, unconfirmed: vec![] }
}

#[test]
fn the_hand_written_map_is_confirmed_claim_by_claim() {
    let report = run(&load(&map_json()), &app());
    // 8 boxes, 6 arrows, 1 missing connection.
    assert_eq!(
        report.summary,
        Summary { claims: 15, confirmed: 15, unconfirmed: 0, proposals: ProposalCount::default(), changes: None }
    );
    // Boxes are proven with symbols, and arrows with call and request facts, not just line text.
    assert!(report.nodes.values().all(|c| c.proof == Some(Proof::Fact)));
    assert_eq!(report.edges["e12"].proof, Some(Proof::Fact)); // the button's fetch matches the route handled by convertPatient
    assert_eq!(report.edges["e23"].proof, Some(Proof::Fact));
    assert_eq!(report.edges["ej"].proof, Some(Proof::Fact));
    assert_eq!(report.edges["e34"].proof, Some(Proof::Text)); // events aren't facts yet, so the cited line is the proof
                                                              // The missing listener is proven by re-running its search.
    let miss = &report.absences["miss"];
    assert_eq!((miss.status, miss.found, miss.locations.len(), miss.reasons.len()), (Status::Confirmed, 0, 0, 0));
    assert_eq!(report.tour["t1"], ok_tour());
    assert_eq!(report.tour["t5"], ok_tour());
}

#[test]
fn a_call_that_does_not_exist_fails_on_the_facts_even_when_the_cited_line_is_real() {
    // evaluateRules calls queueEmail, not sendEmail; the cited line exists, the claimed call doesn't.
    let map = variant(|m| item(m, "edges", "eu12")["to"] = json!("u3"));
    let r = &run(&map, &app()).edges["eu12"];
    assert_eq!(r.status, Status::Unconfirmed);
    assert!(r.evidence[0].ok);
    assert_eq!(
        r.reasons,
        vec!["the code has no call from src/automations/evaluate.ts#evaluateRules to src/email/send.ts#sendEmail"]
    );
}

#[test]
fn a_request_that_matches_no_route_fails() {
    let map = variant(|m| item(m, "edges", "e12")["to"] = json!("s3"));
    assert_eq!(
        run(&map, &app()).edges["e12"].reasons,
        vec!["no request from src/ui/PatientHeader.tsx#PatientHeader matches a route handled by src/patients/service.ts#markConverted"]
    );
}

#[test]
fn cited_text_not_in_the_file_fails_and_text_a_few_lines_away_still_counts() {
    let missing = variant(|m| item(m, "nodes", "u3")["evidence"][0]["text"] = json!("sendSms("));
    assert_eq!(
        run(&missing, &app()).nodes["u3"].reasons,
        vec!["src/email/send.ts:1: \"sendSms(\" isn't on line 1 or near it"]
    );
    let moved = variant(|m| item(m, "edges", "ej")["evidence"][0]["line"] = json!(8));
    let e = &run(&moved, &app()).edges["ej"].evidence[0];
    assert!(e.ok);
    assert_eq!(e.found_at, Some(10));
}

#[test]
fn follows_code_that_moved_far_as_long_as_the_text_is_on_exactly_one_line() {
    let repo = copy_app();
    let file = "src/patients/service.ts";
    write(repo.path(), file, &format!("{}{}", "// padding\n".repeat(20), read(repo.path(), file)));
    let e = &run(&load(&map_json()), repo.path()).nodes["s4"].evidence[0];
    assert!(e.ok);
    assert_eq!(e.found_at, Some(26));
    let ambiguous = variant(|m| {
        item(m, "edges", "e23")["evidence"][0] =
            json!({ "file": "src/server/routes.ts", "line": 1, "text": "convertPatient" })
    });
    assert_eq!(
        run(&ambiguous, &app()).edges["e23"].reasons,
        vec!["src/server/routes.ts:1: \"convertPatient\" isn't on line 1 or near it, and appears 2 times elsewhere in the file"]
    );
}

#[test]
fn a_symbol_that_does_not_exist_fails() {
    let map = variant(|m| item(m, "nodes", "u2")["symbol"] = json!("src/jobs/queue.ts#enqueue"));
    assert_eq!(run(&map, &app()).nodes["u2"].reasons, vec!["the code has no symbol src/jobs/queue.ts#enqueue"]);
}

#[test]
fn a_wrong_nothing_does_x_claim_fails_and_says_where_the_matches_are() {
    let map = variant(|m| {
        m["absences"].as_array_mut().unwrap().push(json!({ "id": "none", "claim": "Nothing calls sendEmail", "search": { "pattern": "sendEmail(" }, "matches": 0 }))
    });
    let r = &run(&map, &app()).absences["none"];
    assert_eq!((r.status, r.found), (Status::Unconfirmed, 2));
    assert_eq!(r.reasons, vec!["the search finds 2 matches; the map says 0"]);
    let at: Vec<(&str, usize)> = r.locations.iter().map(|l| (l.file.as_str(), l.line)).collect();
    assert_eq!(at, vec![("src/email/send.ts", 1), ("src/jobs/queue.ts", 4)]);
}

#[test]
fn flags_tour_stops_that_point_at_unconfirmed_claims() {
    let map = variant(|m| item(m, "nodes", "u1")["evidence"][0]["text"] = json!("export function evaluate("));
    assert_eq!(
        run(&map, &app()).tour["t5"],
        TourCheck { status: TourStatus::MentionsUnconfirmed, unconfirmed: vec!["u1".into()] }
    );
}

#[test]
fn after_a_fix_the_nothing_listens_claim_no_longer_holds() {
    let repo = copy_app();
    write(
        repo.path(),
        "src/automations/listeners.ts",
        &[
            "import { bus } from '../events/bus';",
            "import { evaluateRules } from './evaluate';",
            "",
            "bus.on('patient.converted', ({ patientId }) => evaluateRules({ patientId }));",
            "",
        ]
        .join("\n"),
    );
    let r = run(&load(&map_json()), repo.path());
    let miss = &r.absences["miss"];
    assert_eq!((miss.status, miss.found), (Status::Unconfirmed, 1));
    assert_eq!((miss.locations[0].file.as_str(), miss.locations[0].line), ("src/automations/listeners.ts", 4));
    assert_eq!(r.tour["t5"], TourCheck { status: TourStatus::MentionsUnconfirmed, unconfirmed: vec!["miss".into()] });
    assert_eq!(r.summary.confirmed, 14); // everything else still holds
}

#[test]
fn a_renamed_function_breaks_the_boxes_and_arrows_that_named_it_and_nothing_else() {
    let repo = copy_app();
    let file = "src/jobs/queue.ts";
    write(repo.path(), file, &read(repo.path(), file).replacen("queueEmail", "enqueueEmail", 1));
    let r = run(&load(&map_json()), repo.path());
    let mut broken: Vec<&str> = r
        .nodes
        .iter()
        .chain(r.edges.iter())
        .filter(|(_, c)| c.status == Status::Unconfirmed)
        .map(|(id, _)| id.as_str())
        .collect();
    broken.sort_unstable();
    assert_eq!(broken, vec!["eu12", "eu23", "u2"]); // the box, and the arrows into and out of it
}

#[test]
fn never_reads_secrets_or_files_outside_the_repo_even_when_a_map_points_at_them() {
    let map = variant(|m| {
        item(m, "nodes", "u3")["evidence"][0] = json!({ "file": ".env", "line": 1, "text": "EMAIL_API_KEY" })
    });
    assert_eq!(run(&map, &app()).nodes["u3"].reasons, vec![".env:1: secret files are never read"]);
    let mut bad = map_json();
    bad["nodes"][0]["evidence"] = json!([{ "file": "../outside.ts", "line": 1, "text": "x" }]);
    assert_eq!(
        parse_map(&bad).unwrap_err(),
        vec!["node \"s1\": evidence file \"../outside.ts\" must be a path inside the repo"]
    );
}

#[test]
fn leaves_secrets_out_of_searches() {
    let map = variant(|m| {
        m["absences"].as_array_mut().unwrap().push(json!({ "id": "key", "claim": "No API keys in the repo", "search": { "pattern": "EMAIL_API_KEY" }, "matches": 0 }))
    });
    let r = &run(&map, &app()).absences["key"];
    assert_eq!((r.status, r.found), (Status::Confirmed, 0));
}

#[test]
fn a_journey_is_checked_like_any_other_map() {
    let report = run(&load(&journey_json()), &app());
    assert_eq!((report.summary.claims, report.summary.confirmed, report.summary.unconfirmed), (12, 12, 0));
    assert_eq!(report.screens["j1"].status, ScreenStatus::Confirmed);
}

#[test]
fn branches_say_how_to_draw_the_map_not_anything_about_the_code() {
    let raw = journey_json();
    let map = load(&raw);
    let mut plain = raw.clone();
    for list in ["nodes", "edges"] {
        for x in plain[list].as_array_mut().unwrap() {
            x.as_object_mut().unwrap().remove("branch");
        }
    }
    let plain = load(&plain);
    assert_eq!(
        map.nodes.iter().filter(|n| n.branch.is_some()).map(|n| n.id.as_str()).collect::<Vec<_>>(),
        vec!["j4", "j5", "j6"]
    );
    assert_eq!(run(&map, &app()), run(&plain, &app()));
    assert_eq!(map.edges.iter().find(|e| e.id == "jd").unwrap().branch.as_deref(), Some("Writes a note instead"));
}

// Not in the original: guards found in review.

#[cfg(unix)]
#[test]
fn never_reads_a_secret_through_a_link_or_by_another_case() {
    let repo = copy_app();
    std::os::unix::fs::symlink("../.env", repo.path().join("src/config.txt")).unwrap();
    let map = variant(|m| {
        item(m, "nodes", "u3")["evidence"][0] = json!({ "file": "src/config.txt", "line": 1, "text": "EMAIL_API_KEY" })
    });
    assert_eq!(run(&map, repo.path()).nodes["u3"].reasons, vec!["src/config.txt:1: secret files are never read"]);
    let upper = variant(|m| {
        item(m, "nodes", "u3")["evidence"][0] = json!({ "file": ".ENV", "line": 1, "text": "EMAIL_API_KEY" })
    });
    assert_eq!(run(&upper, repo.path()).nodes["u3"].reasons, vec![".ENV:1: secret files are never read"]);
}

#[test]
fn blank_cited_text_proves_nothing() {
    let map = variant(|m| item(m, "nodes", "u3")["evidence"][0]["text"] = json!("   "));
    assert_eq!(run(&map, &app()).nodes["u3"].reasons, vec!["src/email/send.ts:1: the cited text is blank"]);
}
