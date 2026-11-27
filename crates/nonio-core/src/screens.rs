//! Screens: what the person sees at a box. A screen is shown only if the checker can tie it to the repo:
//! - a capture must be one nonio saved under [`screens_dir`] (its record and image still matching), and a
//!   highlighted label must be both on the captured screen and in the code the box cites;
//! - an image must be a file in the repo that the cited code names;
//! - a sketch's elements must each appear in the markup file it was drawn from, unless the box carries a proposal
//!   and the element is part of it;
//! - a wireframe draws one of the map's pages: each region's file must be the page's file or one it brings in, and
//!   each element's text must be in its region's file. The layout is not checked.
//!
//! Ported from `packages/core/src/screens.ts` of illuminate (MIT, Copyright (c) 2026 johnz4021).

use std::collections::{HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use indexmap::IndexMap;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::format::{is_safe_relative_path, ElementKind, MapNode, Page, ScreenKind, Slot};
use crate::repo::{is_secret_path, Reader, STATE_DIR};

/// Where captures live, inside the repo.
pub fn screens_dir() -> String {
    format!("{STATE_DIR}/screens")
}

pub const IMAGE_TYPES: &[(&str, &str)] = &[
    (".png", "image/png"),
    (".jpg", "image/jpeg"),
    (".jpeg", "image/jpeg"),
    (".gif", "image/gif"),
    (".webp", "image/webp"),
    (".avif", "image/avif"),
    (".svg", "image/svg+xml"),
];
pub const MAX_IMAGE_BYTES: u64 = 5_000_000;

fn capture_id_ok(id: &str) -> bool {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^[A-Za-z0-9_-]{1,64}$").expect("valid regex")).is_match(id)
}

/// Something visible on a captured screen: its text and where it is, in CSS pixels of the capture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapturedElement {
    pub tag: String,
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// A page on this machine, or an HTML file in the repo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum CaptureSource {
    Page { url: String },
    File { file: String },
}

/// The record saved beside a capture's image. `sha256` is of the image file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capture {
    pub id: String,
    pub source: CaptureSource,
    pub title: String,
    pub width: f64,
    pub height: f64,
    pub sha256: String,
    pub taken_at: String,
    pub elements: Vec<CapturedElement>,
}

/// A box on the capture, as fractions of its width and height.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ScreenBox {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShownElement {
    pub kind: ElementKind,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposed: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RegionShow {
    pub id: String,
    pub slot: Slot,
    pub label: String,
    pub file: String,
    pub elements: Vec<ShownElement>,
}

/// A page as it can be drawn: the regions that checked out, each with the elements found in its file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PageShow {
    pub id: String,
    pub name: String,
    pub file: String,
    pub regions: Vec<RegionShow>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ScreenShow {
    Capture {
        capture: String,
        title: String,
        width: f64,
        height: f64,
        from: String,
        #[serde(rename = "box")]
        area: Option<ScreenBox>,
    },
    Image {
        file: String,
    },
    Sketch {
        file: String,
        elements: Vec<ShownElement>,
        highlight: Option<String>,
    },
    Wireframe {
        page: PageShow,
        region: Option<String>,
        highlight: Option<String>,
    },
}

/// What the checker made of a page. `dropped` says, by region id, why a region isn't drawn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PageCheck {
    pub show: Option<PageShow>,
    pub reason: Option<String>,
    pub dropped: IndexMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ScreenStatus {
    Confirmed,
    Unconfirmed,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScreenCheck {
    pub status: ScreenStatus,
    pub reasons: Vec<String>,
    pub show: Option<ScreenShow>,
}

fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Where a capture's record and image are kept under `root`.
pub fn capture_paths(root: &Path, id: &str) -> (PathBuf, PathBuf) {
    let dir = root.join(screens_dir());
    (dir.join(format!("{id}.json")), dir.join(format!("{id}.png")))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A saved capture whose image is still the one that was taken, or why it can't be used.
pub fn read_capture(root: &Path, id: &str) -> Result<Capture, String> {
    if !capture_id_ok(id) {
        return Err(format!("\"{id}\" is not a capture id"));
    }
    let (record, image) = capture_paths(root, id);
    // Only files nonio wrote itself: real files inside the repo, never a link to somewhere else.
    let own = |p: &Path| {
        let real_root = fs::canonicalize(root).ok();
        fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_file())
            && fs::canonicalize(p).ok().zip(real_root).is_some_and(|(real, root)| real.starts_with(root))
    };
    if record.exists() && !own(&record) || image.exists() && !own(&image) {
        return Err(format!("capture {id} isn't a file nonio saved in this repo"));
    }
    let Ok(text) = fs::read_to_string(&record) else {
        return Err(format!("there is no capture {id}; take one with the capture tool"));
    };
    let capture: Capture = match serde_json::from_str::<Capture>(&text) {
        Ok(c) if c.id == id && capture_id_ok(&c.id) && c.width > 0.0 && c.height > 0.0 => c,
        _ => return Err(format!("the record of capture {id} can't be read")),
    };
    let Ok(bytes) = fs::read(&image) else {
        return Err(format!("the image of capture {id} is missing"));
    };
    if hex(&Sha256::digest(&bytes)) != capture.sha256 {
        return Err(format!("the image of capture {id} isn't the one nonio took"));
    }
    Ok(capture)
}

fn extension(rel: &str) -> String {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    match name.rfind('.') {
        Some(i) if i > 0 => name[i..].to_lowercase(),
        _ => String::new(),
    }
}

fn basename(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

/// An image file inside the repo that may be shown, as its real path and media type, or why not.
pub fn repo_image(root: &Path, rel: &str) -> Result<(PathBuf, &'static str), String> {
    if !is_safe_relative_path(rel) {
        return Err(format!("{rel} is outside the repo"));
    }
    if is_secret_path(rel) {
        return Err("secret files are never shown".into());
    }
    let ext = extension(rel);
    let Some(&(_, kind)) = IMAGE_TYPES.iter().find(|(e, _)| *e == ext) else {
        return Err(format!("{rel} is not an image file"));
    };
    let real_root = fs::canonicalize(root).map_err(|_| format!("{rel} doesn't exist"))?;
    let real = fs::canonicalize(real_root.join(rel)).map_err(|_| format!("{rel} doesn't exist"))?;
    if !real.starts_with(&real_root) || real == real_root {
        return Err(format!("{rel} is outside the repo"));
    }
    let meta = fs::metadata(&real).map_err(|_| format!("{rel} doesn't exist"))?;
    if !meta.is_file() {
        return Err(format!("{rel} is not a file"));
    }
    if meta.len() > MAX_IMAGE_BYTES {
        return Err(format!("{rel} is too large to show"));
    }
    Ok((real, kind))
}

/// The element on a capture that a label points at: the one whose text is the label, else the smallest that holds it.
pub fn find_on_capture<'c>(capture: &'c Capture, label: &str) -> Option<&'c CapturedElement> {
    let want = norm(label);
    if let Some(exact) = capture.elements.iter().find(|e| norm(&e.text) == want) {
        return Some(exact);
    }
    capture
        .elements
        .iter()
        .filter(|e| norm(&e.text).contains(&want))
        .min_by(|a, b| (a.w * a.h).partial_cmp(&(b.w * b.h)).unwrap_or(std::cmp::Ordering::Equal))
}

/// How many files a page may bring in, directly or through others, before the search for a region's file gives up.
const MAX_PAGE_FILES: usize = 600;

/// A page of the map, checked region by region: the region's file must be the page's own file or one it brings in
/// (through imports, however many steps), and every element's text must be in that file. A region that fails is
/// left off the drawing; the rest of the page stays.
pub fn check_page(page: &Page, repo: &dyn Reader, imports_of: &dyn Fn(&str) -> Vec<String>) -> PageCheck {
    if let Err(reason) = repo.read(&page.file) {
        return PageCheck { show: None, reason: Some(format!("{}: {reason}", page.file)), dropped: IndexMap::new() };
    }
    let mut reached: HashSet<String> = HashSet::from([page.file.clone()]);
    let mut queue: VecDeque<String> = VecDeque::from([page.file.clone()]);
    while reached.len() < MAX_PAGE_FILES {
        let Some(file) = queue.pop_front() else { break };
        for to in imports_of(&file) {
            if reached.insert(to.clone()) {
                queue.push_back(to);
            }
        }
    }
    let mut dropped = IndexMap::new();
    let mut regions = Vec::new();
    for r in &page.regions {
        match repo.read(&r.file) {
            Err(reason) => {
                dropped.insert(r.id.clone(), format!("{}: {reason}", r.file));
            }
            Ok(_) if !reached.contains(&r.file) => {
                dropped.insert(
                    r.id.clone(),
                    format!("{} isn't part of this page: {} doesn't bring it in", r.file, page.file),
                );
            }
            Ok(text) => {
                let text = norm(&text);
                let missing: Vec<String> = r
                    .elements
                    .iter()
                    .filter(|e| !text.contains(&norm(&e.text)))
                    .map(|e| format!("\"{}\" isn't in {}", e.text, r.file))
                    .collect();
                if missing.is_empty() {
                    let elements = r
                        .elements
                        .iter()
                        .map(|e| ShownElement { kind: e.kind, text: e.text.clone(), proposed: None })
                        .collect();
                    regions.push(RegionShow {
                        id: r.id.clone(),
                        slot: r.slot,
                        label: r.label.clone(),
                        file: r.file.clone(),
                        elements,
                    });
                } else {
                    dropped.insert(r.id.clone(), missing.join("; "));
                }
            }
        }
    }
    if regions.is_empty() {
        let why = dropped.values().cloned().collect::<Vec<_>>().join("; ");
        return PageCheck {
            show: None,
            reason: Some(format!("no region of page \"{}\" checked out: {why}", page.id)),
            dropped,
        };
    }
    PageCheck {
        show: Some(PageShow { id: page.id.clone(), name: page.name.clone(), file: page.file.clone(), regions }),
        reason: None,
        dropped,
    }
}

fn fail(reasons: Vec<String>) -> ScreenCheck {
    ScreenCheck { status: ScreenStatus::Unconfirmed, reasons, show: None }
}

fn shown(show: ScreenShow) -> ScreenCheck {
    ScreenCheck { status: ScreenStatus::Confirmed, reasons: Vec::new(), show: Some(show) }
}

/// The screen on a box, checked; `None` for a box without one.
pub fn check_screen(
    n: &MapNode,
    repo: &dyn Reader,
    page_of: &dyn Fn(&str) -> Option<PageCheck>,
) -> Option<ScreenCheck> {
    let sc = n.screen.as_ref()?;
    let mut cited: Vec<&str> = Vec::new();
    for e in &n.evidence {
        if !cited.contains(&e.file.as_str()) {
            cited.push(&e.file);
        }
    }
    let in_cited_code =
        |text: &str| cited.iter().any(|f| norm(&repo.text(f).unwrap_or_default()).contains(&norm(text)));
    let cited_list = cited.join(", ");

    Some(match sc.kind {
        ScreenKind::Wireframe => {
            let Some(page) = page_of(sc.page.as_deref().unwrap_or("")) else {
                return Some(fail(vec![format!("the map has no page \"{}\"", sc.page.as_deref().unwrap_or(""))]));
            };
            let Some(show) = page.show else {
                return Some(fail(vec![page.reason.unwrap_or_else(|| "the page could not be checked".into())]));
            };
            let region = sc.region.as_ref().and_then(|id| show.regions.iter().find(|r| &r.id == id));
            if let (Some(id), None) = (&sc.region, region) {
                let why = page
                    .dropped
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| format!("page \"{}\" has no region \"{id}\"", show.id));
                return Some(fail(vec![why]));
            }
            if let Some(h) = &sc.highlight {
                if !region.is_some_and(|r| r.elements.iter().any(|e| norm(&e.text) == norm(h))) {
                    return Some(fail(vec![format!(
                        "the highlight \"{h}\" isn't one of the elements of region \"{}\"",
                        sc.region.as_deref().unwrap_or("")
                    )]));
                }
            }
            // The box has to be about this page: it cites the page's code, or the thing it marks is in its cited code.
            let about = cited.contains(&show.file.as_str())
                || region.is_some_and(|r| cited.contains(&r.file.as_str()))
                || sc.highlight.as_deref().is_some_and(in_cited_code);
            if !about {
                return Some(fail(vec![format!(
                    "the code this box cites ({cited_list}) isn't part of what it marks on page \"{}\"",
                    show.id
                )]));
            }
            let region = region.map(|r| r.id.clone());
            shown(ScreenShow::Wireframe { page: show, region, highlight: sc.highlight.clone() })
        }
        ScreenKind::Capture => {
            let c = match read_capture(repo.root(), sc.capture.as_deref().unwrap_or("")) {
                Ok(c) => c,
                Err(reason) => return Some(fail(vec![reason])),
            };
            let mut area = None;
            if let Some(h) = &sc.highlight {
                let Some(el) = find_on_capture(&c, h) else {
                    return Some(fail(vec![format!("\"{h}\" isn't on the captured screen")]));
                };
                if !in_cited_code(h) {
                    return Some(fail(vec![format!("\"{h}\" isn't in the code this box cites ({cited_list})")]));
                }
                area = Some(ScreenBox { x: el.x / c.width, y: el.y / c.height, w: el.w / c.width, h: el.h / c.height });
            }
            let from = match &c.source {
                CaptureSource::Page { url } => url.clone(),
                CaptureSource::File { file } => file.clone(),
            };
            shown(ScreenShow::Capture { capture: c.id, title: c.title, width: c.width, height: c.height, from, area })
        }
        ScreenKind::Image => {
            let file = sc.file.as_deref().unwrap_or("");
            if let Err(reason) = repo_image(repo.root(), file) {
                return Some(fail(vec![reason]));
            }
            if !in_cited_code(basename(file)) {
                return Some(fail(vec![format!(
                    "the code this box cites ({cited_list}) doesn't mention {}",
                    basename(file)
                )]));
            }
            shown(ScreenShow::Image { file: file.to_string() })
        }
        ScreenKind::Sketch => {
            // A sketch shows what's in the markup file. On a box that carries a proposal it may also show what isn't
            // there yet. Whether an element is still only proposed is read from the file, not taken from the map.
            let file = sc.file.as_deref().unwrap_or("");
            let adding = n.is_addition();
            let read = repo.read(file);
            if let (Err(reason), false) = (&read, adding) {
                return Some(fail(vec![format!("{file}: {reason}")]));
            }
            let text = read.map(|t| norm(&t)).unwrap_or_default();
            let written = sc.elements.as_deref().unwrap_or_default();
            let missing: Vec<String> = written
                .iter()
                .filter(|e| !(adding || e.proposed == Some(true)) && !text.contains(&norm(&e.text)))
                .map(|e| format!("\"{}\" isn't in {file}", e.text))
                .collect();
            if !missing.is_empty() {
                return Some(fail(missing));
            }
            if let Some(h) = &sc.highlight {
                if !written.iter().any(|e| norm(&e.text) == norm(h)) {
                    return Some(fail(vec![format!("the highlight \"{h}\" isn't one of the sketch's elements")]));
                }
            }
            let elements = written
                .iter()
                .map(|e| ShownElement {
                    kind: e.kind,
                    text: e.text.clone(),
                    proposed: (!text.contains(&norm(&e.text))).then_some(true),
                })
                .collect();
            shown(ScreenShow::Sketch { file: file.to_string(), elements, highlight: sc.highlight.clone() })
        }
    })
}
