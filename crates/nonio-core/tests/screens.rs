//! Screens on boxes: the checker shows a screenshot, an image or a sketch only when it can tie it to the repo.
//! Ported from `packages/core/test/screens.test.ts` of illuminate (MIT, Copyright (c) 2026 johnz4021).

mod common;

use std::fs;
use std::path::Path;

use common::*;
use nonio_core::check::{ProposalCount, Summary};
use nonio_core::format::{parse_map, CodeMap, ParseResult};
use nonio_core::screens::{capture_paths, ScreenBox, ScreenShow, ScreenStatus};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// The fixture map with `screen` on its first box (the Convert button, cited in src/ui/PatientHeader.tsx).
fn with_screen(screen: Value) -> CodeMap {
    variant(|m| m["nodes"][0]["screen"] = screen)
}

const BOX: &str = "s1";
const PNG: &[u8] = b"not really a png, but the checker only hashes it";

fn save_capture(root: &Path) {
    let sha: String = Sha256::digest(PNG).iter().map(|b| format!("{b:02x}")).collect();
    let record = json!({
        "id": "scr-test1", "source": { "kind": "page", "url": "http://localhost:3000/patients" }, "title": "Patients", "width": 1000, "height": 500,
        "sha256": sha, "takenAt": "2026-11-12T00:00:00.000Z",
        "elements": [{ "tag": "a", "text": "Care journeys", "x": 0, "y": 0, "w": 200, "h": 40 }, { "tag": "button", "text": "Mark as converted", "x": 500, "y": 100, "w": 100, "h": 50 }],
    });
    let (rec, img) = capture_paths(root, "scr-test1");
    fs::create_dir_all(img.parent().unwrap()).unwrap();
    fs::write(&img, PNG).unwrap();
    fs::write(rec, record.to_string()).unwrap();
}

#[test]
fn the_format_reads_maps_with_and_without_screens_and_says_what_a_screen_is_missing() {
    assert!(parse_map(&map_json()).is_ok());
    let mut bad = map_json();
    bad["nodes"][0]["screen"] = json!({ "kind": "capture" });
    bad["nodes"][1]["screen"] = json!({ "kind": "sketch", "file": "../outside.tsx" });
    let p = parse_map(&bad).unwrap_err().join("\n");
    let a = p.find("needs the `capture` id").expect(&p);
    assert!(p[a..].contains("needs `file`, a path inside the repo"), "{p}");
}

#[test]
fn a_screenshot_is_shown_with_its_highlighted_label_marked() {
    let root = copy_app();
    save_capture(root.path());
    let report = run(
        &with_screen(json!({ "kind": "capture", "capture": "scr-test1", "highlight": "Mark as converted" })),
        root.path(),
    );
    let sc = &report.screens[BOX];
    assert_eq!(sc.status, ScreenStatus::Confirmed);
    match &sc.show {
        Some(ScreenShow::Capture { capture, from, area, .. }) => {
            assert_eq!((capture.as_str(), from.as_str()), ("scr-test1", "http://localhost:3000/patients"));
            assert_eq!(*area, Some(ScreenBox { x: 0.5, y: 0.2, w: 0.1, h: 0.1 }));
        }
        other => panic!("{other:?}"),
    }
    // Screens aren't claims: the count is the map's own.
    assert_eq!(
        report.summary,
        Summary { claims: 15, confirmed: 15, unconfirmed: 0, proposals: ProposalCount::default(), changes: None }
    );
}

#[test]
fn a_screenshot_is_left_out_when_nonio_took_no_such_capture_or_its_image_was_swapped() {
    let root = copy_app();
    let none = &run(&with_screen(json!({ "kind": "capture", "capture": "scr-none" })), root.path()).screens[BOX];
    assert_eq!((none.status, none.show.is_none()), (ScreenStatus::Unconfirmed, true));
    save_capture(root.path());
    fs::write(capture_paths(root.path(), "scr-test1").1, "a picture Claude drew").unwrap();
    let swapped = &run(&with_screen(json!({ "kind": "capture", "capture": "scr-test1" })), root.path()).screens[BOX];
    assert_eq!(swapped.status, ScreenStatus::Unconfirmed);
    assert!(swapped.reasons[0].contains("isn't the one nonio took"), "{:?}", swapped.reasons);
}

#[test]
fn a_screenshot_is_left_out_when_the_highlight_is_not_on_it_or_not_in_the_cited_code() {
    let root = copy_app();
    save_capture(root.path());
    let off = run(
        &with_screen(json!({ "kind": "capture", "capture": "scr-test1", "highlight": "Delete patient" })),
        root.path(),
    );
    assert!(off.screens[BOX].reasons[0].contains("isn't on the captured screen"));
    // "Care journeys" is on the screen, but the box's code (the header component) never says it.
    let elsewhere = run(
        &with_screen(json!({ "kind": "capture", "capture": "scr-test1", "highlight": "Care journeys" })),
        root.path(),
    );
    assert!(elsewhere.screens[BOX].reasons[0].contains("isn't in the code this box cites"));
}

#[test]
fn a_sketch_is_shown_when_every_element_is_in_the_markup_file() {
    let file = "src/ui/PatientHeader.tsx";
    let ok = &run(&with_screen(json!({ "kind": "sketch", "file": file, "elements": [{ "kind": "button", "text": "Mark as converted" }], "highlight": "Mark as converted" })), &app()).screens[BOX];
    assert_eq!(ok.status, ScreenStatus::Confirmed);
    assert!(
        matches!(&ok.show, Some(ScreenShow::Sketch { file: f, highlight: Some(h), .. }) if f == file && h == "Mark as converted")
    );
    let invented = &run(
        &with_screen(json!({ "kind": "sketch", "file": file, "elements": [{ "kind": "button", "text": "Mark as converted" }, { "kind": "button", "text": "Archive" }] })),
        &app(),
    )
    .screens[BOX];
    assert_eq!(
        (invented.status, invented.reasons.clone()),
        (ScreenStatus::Unconfirmed, vec![format!("\"Archive\" isn't in {file}")])
    );
}

#[test]
fn an_image_file_is_shown_when_the_cited_code_names_it() {
    let root = copy_app();
    write(root.path(), "src/ui/convert.svg", "<svg xmlns=\"http://www.w3.org/2000/svg\"/>");
    let image = || with_screen(json!({ "kind": "image", "file": "src/ui/convert.svg" }));
    assert!(run(&image(), root.path()).screens[BOX].reasons[0].contains("doesn't mention convert.svg"));
    let header = "src/ui/PatientHeader.tsx";
    write(root.path(), header, &format!("{}\nexport const icon = './convert.svg';\n", read(root.path(), header)));
    let shown = &run(&image(), root.path()).screens[BOX];
    assert_eq!(shown.status, ScreenStatus::Confirmed);
    assert!(matches!(&shown.show, Some(ScreenShow::Image { file }) if file == "src/ui/convert.svg"));
    assert_eq!(
        run(&with_screen(json!({ "kind": "image", "file": ".env" })), root.path()).screens[BOX].status,
        ScreenStatus::Unconfirmed
    );
    assert!(run(&with_screen(json!({ "kind": "image", "file": header })), root.path()).screens[BOX].reasons[0]
        .contains("not an image file"));
}

fn page() -> Value {
    json!({
        "id": "patient",
        "name": "Patient page",
        "file": "src/ui/PatientPage.tsx",
        "regions": [
            { "id": "header", "slot": "top", "label": "Header", "file": "src/ui/PatientHeader.tsx", "elements": [{ "kind": "button", "text": "Mark as converted" }] },
            { "id": "links", "slot": "left", "label": "Links", "file": "src/ui/PatientSidebar.tsx", "elements": [{ "kind": "link", "text": "All patients" }, { "kind": "link", "text": "Leads" }] },
            { "id": "notes", "slot": "main", "label": "Notes", "file": "src/ui/PatientNotes.tsx", "elements": [{ "kind": "heading", "text": "Notes" }, { "kind": "button", "text": "Save note" }] },
        ],
    })
}

fn mark() -> Value {
    json!({ "kind": "wireframe", "page": "patient", "region": "header", "highlight": "Mark as converted" })
}

/// The fixture map with a page, and a wireframe of it on the given boxes (the first box cites PatientHeader.tsx).
fn with_page(edit: impl FnOnce(&mut Value), screens: &[(&str, Value)]) -> ParseResult {
    let mut m = map_json();
    let mut p = page();
    edit(&mut p);
    m["pages"] = json!([p]);
    for n in m["nodes"].as_array_mut().unwrap() {
        if let Some((_, sc)) = screens.iter().find(|(id, _)| n["id"] == *id) {
            n["screen"] = sc.clone();
        }
    }
    parse_map(&m)
}

fn checked(edit: impl FnOnce(&mut Value), screens: &[(&str, Value)]) -> nonio_core::CheckReport {
    run(&with_page(edit, screens).unwrap_or_else(|p| panic!("{p:?}")), &app())
}

fn region_ids(show: &Option<ScreenShow>) -> Vec<String> {
    match show {
        Some(ScreenShow::Wireframe { page, .. }) => page.regions.iter().map(|r| r.id.clone()).collect(),
        other => panic!("not a wireframe: {other:?}"),
    }
}

#[test]
fn a_wireframe_draws_the_whole_page_with_the_boxs_region_and_element_marked() {
    let c = &checked(|_| {}, &[(BOX, mark())]).screens[BOX];
    assert_eq!(c.status, ScreenStatus::Confirmed);
    match &c.show {
        Some(ScreenShow::Wireframe { page, region, highlight }) => {
            assert_eq!((page.id.as_str(), page.name.as_str()), ("patient", "Patient page"));
            assert_eq!((region.as_deref(), highlight.as_deref()), (Some("header"), Some("Mark as converted")));
            let slots: Vec<(&str, &str)> = page.regions.iter().map(|r| (r.id.as_str(), r.slot.as_str())).collect();
            assert_eq!(slots, vec![("header", "top"), ("links", "left"), ("notes", "main")]);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_wireframe_leaves_off_a_region_whose_text_isnt_in_its_file_or_whose_file_the_page_doesnt_bring_in() {
    let made = checked(
        |p| p["regions"][1]["elements"].as_array_mut().unwrap().push(json!({ "kind": "link", "text": "Billing" })),
        &[(BOX, mark())],
    );
    assert_eq!(region_ids(&made.screens[BOX].show), vec!["header", "notes"]);
    let elsewhere = checked(
        |p| {
            p["regions"][2]["file"] = json!("src/email/send.ts");
            p["regions"][2]["elements"] = json!([]);
        },
        &[(BOX, mark())],
    );
    assert_eq!(region_ids(&elsewhere.screens[BOX].show), vec!["header", "links"]);
}

#[test]
fn a_wireframe_is_not_shown_when_the_boxs_own_region_fails_and_says_why() {
    let c = &checked(
        |p| {
            p["regions"][0]["file"] = json!("src/email/send.ts");
            p["regions"][0]["elements"] = json!([]);
        },
        &[(BOX, json!({ "kind": "wireframe", "page": "patient", "region": "header" }))],
    )
    .screens[BOX];
    assert_eq!((c.status, c.show.is_none()), (ScreenStatus::Unconfirmed, true));
    assert_eq!(
        c.reasons,
        vec!["src/email/send.ts isn't part of this page: src/ui/PatientPage.tsx doesn't bring it in"]
    );
}

#[test]
fn a_wireframe_follows_imports_through_other_files_to_find_a_regions_file() {
    let root = copy_app();
    write(root.path(), "src/ui/App.tsx", "import { PatientPage } from './PatientPage';\nexport function App() {\n  return <PatientPage patientId=\"1\" />;\n}\n");
    let map = with_page(|p| p["file"] = json!("src/ui/App.tsx"), &[(BOX, mark())]).unwrap();
    assert_eq!(region_ids(&run(&map, root.path()).screens[BOX].show).len(), 3);
}

#[test]
fn a_wireframe_needs_the_box_to_be_about_the_page_it_shows() {
    // The nightly job box has nothing to do with the patient page.
    let c = &checked(
        |_| {},
        &[("j1", json!({ "kind": "wireframe", "page": "patient", "region": "notes", "highlight": "Save note" }))],
    )
    .screens["j1"];
    assert_eq!(c.status, ScreenStatus::Unconfirmed);
    assert!(c.reasons[0].contains("isn't part of what it marks"));
}

#[test]
fn a_wireframe_marks_only_an_element_that_is_in_the_region() {
    let mut m = mark();
    m["highlight"] = json!("Save note");
    let c = &checked(|_| {}, &[(BOX, m)]).screens[BOX];
    assert_eq!(
        (c.status, c.reasons.clone()),
        (
            ScreenStatus::Unconfirmed,
            vec!["the highlight \"Save note\" isn't one of the elements of region \"header\"".to_string()]
        )
    );
}

#[test]
fn a_page_is_checked_once_and_shared_by_every_box_on_it() {
    // The second box cites src/server/routes.ts, so it has to mark something its code names.
    let r = checked(|_| {}, &[(BOX, mark()), ("s2", json!({ "kind": "wireframe", "page": "patient" }))]);
    assert_eq!(r.screens[BOX].status, ScreenStatus::Confirmed);
    assert_eq!(r.screens["s2"].status, ScreenStatus::Unconfirmed);
}

#[test]
fn the_format_refuses_a_missing_page_or_region_or_a_file_outside_the_repo() {
    let problems =
        |edit: &dyn Fn(&mut Value), screens: &[(&str, Value)]| with_page(edit, screens).err().unwrap_or_default();
    let one = |p: Vec<String>, text: &str| {
        assert_eq!(p.len(), 1, "{p:?}");
        assert!(p[0].contains(text), "{p:?}");
    };
    one(problems(&|_| {}, &[(BOX, json!({ "kind": "wireframe", "page": "nope" }))]), "there is no page \"nope\"");
    one(
        problems(&|_| {}, &[(BOX, json!({ "kind": "wireframe", "page": "patient", "region": "footer" }))]),
        "has no region \"footer\"",
    );
    one(
        problems(&|_| {}, &[(BOX, json!({ "kind": "wireframe", "page": "patient", "highlight": "Notes" }))]),
        "needs the `region`",
    );
    one(
        problems(&|p| p["regions"][0]["file"] = json!("../outside.tsx"), &[(BOX, mark())]),
        "must be a path inside the repo",
    );
    one(problems(&|p| p["regions"][1]["id"] = json!("header"), &[(BOX, mark())]), "region id \"header\" is used twice");
    assert_eq!(problems(&|p| p["regions"][0]["slot"] = json!("footer"), &[(BOX, mark())]).len(), 1);
}
