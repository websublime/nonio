//! Every prompt nonio sends. The reader is semi-technical: they can follow a diagram but don't hold the system in
//! their head. Plain English first.
//!
//! Ported from `packages/server/src/prompts.ts` (MIT, Copyright (c) 2026 johnz4021); see NOTICE. The text is the
//! original's, with "illuminate" read as "nonio"; `tests/prompts.rs` compares every prompt with the original's output
//! on the same input (recorded by `tools/record-prompts`).

use nonio_core::check::Status;
use nonio_core::format::View;
use nonio_core::{settled_proposals, CheckReport, CodeMap};

pub const EVIDENCE_RULES: &[&str] = &[
    r#"## Evidence rules (a program checks every claim; anything it can't confirm is shown as unconfirmed)"#,
    r#"- Every box and arrow that is in the code cites at least one line: `file` (repo-relative), `line` (1-based), and `text` copied exactly from that line. A distinctive fragment of 10–80 characters works best."#,
    r#"- A box that is a function or method gets `symbol`: "<file>#<name>", named the way the checker names it: methods as Class.method, object members as obj.member ("src/db.ts#db.update"), a function inside another function as outer.inner, and a function in the object a factory returns as factory.member. When a list of functions by file is given, copy the name from it. If you aren't sure of the name, leave `symbol` out: the box is still checked by its cited line."#,
    r#"- Arrow `kind`: `calls` when a function in the from box literally calls the function in the to box; `request` when an HTTP call in the from box hits a route handled by the to box; `imports`; `then` for anything else (events, ordering). The checker verifies calls, request and imports against the code, so use them only when literally true."#,
    r#"- Never cite or read secret files (.env, keys)."#,
];

pub const MAP_RULES: &[&str] = &[
    r#"## What a good map is"#,
    r#"- It follows what happens, in order, from where it starts (a button, a route, a job, a command) to where it ends (a table write, an email, an outside API), or to where it stops."#,
    r#"- 2–6 `areas` named in product language (Patients, Billing, Background jobs), not folder names."#,
    r#"- Aim for 6–8 boxes (`nodes`); 12 is a hard ceiling, reached only when the question asks about several things at once. A map is an answer, not an inventory: someone should be able to hold it in their head. Each box is one step a person would recognize and that the answer turns on."#,
    r#"- Leave plumbing off the map unless the question is about it: a request arriving, a job being started, a result being saved, an update being sent to the screen. Fold each into the box it serves and say it in that box's `explain` or on the arrow."#,
    r#"- When several steps in a row just hand on to the next (a route that calls a handler that calls a service), they are one box, cited at the step that matters. If one of them fills in or changes what it passes along (who the owner is, a mode turned on or off, a value worked out from the caller), say that in the box's `explain` and cite its line too."#,
    r#"- When what they ask about can happen more than one way (two ways to sign up, a request through the server and a direct call, two modes), draw each way to where they meet, unless they asked about one of them. Someone new to the code needs to know the others exist. Each way can be a box or two."#,
    r#"- When the answer is a list of like things (five kinds of rule, four checks), that is one box naming the group, or at most the two or three that differ in a way the question cares about; put the rest in `explain`."#,
    r#"- Before returning, go through the boxes one at a time: if the answer would be the same without it, drop or merge it."#,
    r#"- Box `name`: plain English, at most 30 characters ("Patient marked enrolled", not "markConverted()")."#,
    r#"- `kind`: entry (a way in), step, store (table, queue, cache), external (outside service), job (scheduled or background work)."#,
    r#"- `explain` for every box: just `concept`, 1–2 plain sentences, no jargon, saying what this step does. The box already shows its function and file, so leave those out."#,
    r#"- Arrow `label`: a few words saying how one step leads to the next ("sends request", "calls", "announces", "every night"). People read an arrow as a sentence, "<from box> <label> <to box>", so the label's subject is the box it leaves. When someone else does the next step, say who or what sets it off ("then Alice claims", "once paid"), never only what the next box does."#,
    r#"- Code that exists but isn't reached from the start can still be boxes. Don't draw arrows that don't exist."#,
    r#"- An `absence` is a break in the flow you were asked about: a step on that path should lead to the next one and doesn't (something is announced that nothing listens for, a job that nothing ever starts). Most maps have none, and a map without one is complete. Don't go looking for one, and don't use one for a design choice or guarantee ("X never calls Y", "nothing here edits files"): say that in a box's explanation or the walkthrough instead. Don't record small mismatches beside the question (a command the docs don't mention, an unused option)."#,
    r#"- When there is a break, record it: the `claim` in plain words, the two boxes it should connect (`between`), and the exact `search` that proves nothing is there (`pattern`, `regex`, optional `paths` globs covering every file where the missing piece could live), with `matches` set to the count that search returns (usually 0). Run that search yourself with Grep first."#,
];

pub const BRANCH_RULES: &[&str] = &[
    r#"## Ways through (`branch`, on any kind of map)"#,
    r#"- When the answer has more than one way through and the difference matters (two ways in that meet later, an error or cancel path, the options of a change they asked to compare), give every box and arrow that is on only one of them a `"branch"`: a short name for that way in plain words ("Wallet sign-up", "Payment fails", "A: prompt rule"), the same name on every box and arrow of it. What the ways share carries no branch. Each way is drawn in its own colour, with a key."#,
    r#"- Keep to the ways that matter, at most 5. A map with one way through has no branches."#,
];

pub const TIMELINE_RULES: &[&str] = &[
    r#"## When the answer is parties taking turns (a timeline)"#,
    r#"- Set `"view": "timeline"` when the answer is two or more parties or places handing off to each other, back and forth, and the order between them is the point: a protocol or handshake, a login through an outside service, a payment and its webhook, a client and server over a socket, a swap across two chains. It is drawn with one lane per area and time running left to right, so make each area one party or place ("Browser", "Our server", "Google"; "Chain A", "Chain B"), and list the boxes in the order things happen."#,
    r#"- Not for a flow that passes through each place once (the usual map already reads in order), not for how things are organised or what depends on what, and not for what a person sees (that is a journey)."#,
];

pub const JOURNEY_RULES: &[&str] = &[
    r#"## When the question is about what a person sees or does (a journey, a user flow)"#,
    r#"- Set `"view": "journey"` on the map: it is then drawn as a user flow, left to right, with no code on the boxes (the code is one click away)."#,
    r#"- `kind` here is one of: `screen` (something the person looks at), `action` (something they do that is a step in itself: "Fills in the form", "Pays"), `choice` (a point where they pick between ways forward, named as the question they face: "Buy now or add to bag?"; each arrow out of it is one option, labeled with what they pick). Use a `choice` wherever the path forks on what the person decides, rather than two arrows out of a screen."#,
    r#"- Draw only what the person sees and does. Every box is a screen they look at or something they do on one, in order, named the way they would say it ("Sign-up form", "Email confirmed"). Areas are the stages of the journey as the person lives them ("Choosing", "Paying", "Waiting for it"). Arrows say what the person does or what happens to them next ("taps Sign up", "is sent to")."#,
    r#"- Leave out what goes on behind the screen: no boxes for routes, requests, functions, jobs, database writes or other services. When several of those sit between two screens, they are one arrow ("pays, and lands on"), and that arrow cites the code that does it. A step that happens outside the app is a box only if the person sees it (a payment page, an email they get), cited at the line that sends them there or sends it."#,
    r#"- A wait is part of the journey: when the next thing the person sees comes later (an email after a nightly job, an order shipped after approval), say so on the arrow in their terms ("a day later, gets"), and don't draw the job."#,
    r#"- Each box and arrow still cites the code that makes it so: the component, the handler, the redirect."#,
    r#"- Show where each step happens: define the pages the journey goes through in `pages`, one for each state of a screen the person sees (see Screens), and give each screen box a wireframe of its page with its own part marked. Steps in the same state share one page. A box whose page can't be drawn usefully (nothing recognisable on it, or it isn't in this repo) gets no `screen`: no picture is better than a thin one."#,
    r#"- Where the journey stops short (a button that leads nowhere, a result with no screen, nothing ever telling the person what happened), that is a break: record it as an absence."#,
    r#"- One main path, with branches (see Ways through): the main path is what most people do from start to finish, and its boxes and arrows carry no `branch`; each way off it is a branch named in the person's words ("Saves for later", "Payment fails"). The main path is drawn on one straight line, left to right, and each branch below the step it leaves from. An arrow that leads from a branch back to the main path carries that branch too."#,
    r#"- A branch leaves from a `choice`, or from any step where things can go another way (an error, a cancel, a link elsewhere). Keep to the branches that matter, at most 5. A journey with only one way through has no branches."#,
    r#"- The walkthrough is told as the person's experience too: down the main path first, one stop per screen, then one stop for each branch."#,
];

pub const PROPOSAL_RULES: &[&str] = &[
    r#"## Proposals (`proposed` on a box or arrow; only when a change is asked for)"#,
    r#"- When they ask how something could or should change ("how would we add…", "plan…", "what would it take to…", "propose…"), draw the code as it is, cited and checked as usual, and the change on top of it. A question about how things work today gets no proposals."#,
    r#"- `"proposed": { "kind": "add", "note": "…" }`: a box or arrow that isn't in the code yet. It cites nothing, so leave `evidence` out. Give a new box the `symbol` it would have ("<file>#<name>"; the file may be new) and a new arrow its real `kind` (`calls`, `request`, `imports`): that is how the check can tell, later, that it has been built. An arrow to or from a new box is itself an addition."#,
    r#"- `"kind": "change"` or `"remove"`: code that is there today and would be changed or deleted. Cite it like any other claim."#,
    r#"- `note`: one plain sentence saying what would change and why."#,
    r#"- Every new box must connect, through arrows, to a box that exists. Keep it small: only the boxes and arrows the change needs."#,
    r#"- When they ask to compare ways of making a change, each option is a way through (see Ways through) named for it ("A: …", "B: …"), drawn as far as where its effect shows (what the person sees, or what the code does differently), so the options can be told apart at a glance."#,
    r#"- On a screen: a box with `proposed` may have a sketch with elements marked `"proposed": true` for what would be added to it, and a new screen is a new box whose sketch is all new."#,
    r#"- A program checks proposals against the code too. Proposing to add what the code already has, or to change or remove what it doesn't have, is sent back."#,
];

pub const CHANGE_RULES: &[&str] = &[
    r#"## When the question is about a change (what changed, what a commit or a task did, "what did you just change")"#,
    r#"- Set `"since"` on the map to the commit to compare with: "HEAD" for work that isn't committed yet, "HEAD~1" for the last commit, or the branch or commit they name. A question about how things work today gets no `since`."#,
    r#"- Draw the flow the change belongs to as it works now, the map you would draw if they had asked how it works: start where it starts and follow it through, steps that didn't change included. The person reads the change against those: a map where every box is new or changed shows no before, and can't be told from a map of something else. As a guide, at least a third of the boxes are ones that didn't change (the steps the change plugs into, what calls the new code, what it hands on to). Leave out helpers the change added that aren't steps of the flow."#,
    r#"- Cite everything as usual. Don't mark what is new or different: a program compares every box and arrow with that commit and marks them itself. It compares a box that names its function (`symbol`) as that function, so give new and changed boxes their `symbol`; for anything else, cite a line that is new."#,
    r#"- Code that was deleted is a box or arrow with `"removed": true`. It cites the code as it was at that commit: `file`, `line` and `text` from the old version (`git show <since>:<file>` shows it, when you can run git). An arrow to or from a removed box is removed too. The program checks that it was there then and is gone now, so use it only for code that is really gone, not for code that moved or was renamed."#,
    r#"- In the explanation of each box that changed, say what is different now, in plain words ("Now also tells the listener"), not as a diff."#,
    r#"- If you made the change yourself in this conversation, draw what you changed and what it touches. Other differences from that commit that aren't part of it stay off the map."#,
    r#"- The walkthrough says what changed first, then follows the changed path in the order things happen."#,
];

pub const SCREEN_RULES: &[&str] = &[
    r#"## Screens (`pages` on the map, `screen` on a box; optional, and your call)"#,
    r#"- A picture of where a step happens on the screen can ground a map for people who don't read code. Add one where the flow touches the screen and seeing it helps: usually the box where it starts (the button or form), sometimes where a result appears. For backend-only questions, add none."#,
    r#"- The usual picture is a wireframe of the whole page, with the part the box is about marked. A page is what the person sees at one moment, defined in the map's `pages`: `{ "id": "…", "name": "Patient page", "file": "<the page's own component or template>", "regions": [ … ] }`."#,
    r#"- When the same screen changes as they go (loading, then ready; a panel, dialog or bubble opening; an error; a form step), each state they see is its own page, named for it ("Map page, loading", "Map page, with an answer"), listing only what is on screen then. Never put things that are never on screen together in one page: a loading message beside the result it was waiting for, a dialog beside the screen it replaces. Steps in the same state share its page."#,
    r#"- A region is one visible part of the page: `{ "id": "…", "slot": "top" | "left" | "main" | "right" | "bottom", "label": "Top bar", "file": "<the file it comes from>", "elements": [{ "kind": "heading|button|field|link|text|image", "text": "…" }] }`. Give the 3–6 regions a person would recognise the page by, not only the one the step uses. Put each in the slot where it appears on screen, going by the markup and the styles (a bar across the top is `top`, a sidebar is `left` or `right`, the content is `main`). `elements`: up to 6 of the most recognisable things in the region, in the order they appear, each `text` copied exactly from that region's file; a region may have none."#,
    r#"- Then on a box: `"screen": { "kind": "wireframe", "page": "<page id>", "region": "<region id>", "highlight": "<text of one element in that region>" }`. Boxes that happen on the same page in the same state use the same page, each marking its own region and element."#,
    r#"- A program checks every page: a region's `file` must be the page's `file` or one it imports (directly or through other files), every element's text must be in its region's file, and the box must cite the page's code or the code the marked element is in. A region that fails is left off; where regions sit can't be checked, so read the layout from the code and don't invent parts."#,
    r#"- `{ "kind": "capture", "capture": "<id>", "highlight": "<label>" }`: a real screenshot, in place of a wireframe, when a page is already running on this machine. Only if you have the nonio_capture tool: it screenshots a running page or an HTML file in the repo, and returns the id. Look at the image first, and don't use one that shows a login page, an error or the wrong state. Never start the app yourself."#,
    r#"- `{ "kind": "image", "file": "<path>" }`: an image file in the repo (an icon, a sprite) that the box's cited code names."#,
    r#"- `{ "kind": "sketch", "file": "<markup file>", "elements": [ … ], "highlight": "<one element's text>" }`: the visible things in one component, as a list. Only for a proposal (see Proposals): it can show elements that aren't there yet."#,
];

pub const TOUR_RULES: &[&str] = &[
    r#"## Walkthrough (`tour`, written last)"#,
    r#"- 4–7 stops in the order a newcomer should read them: the answer, then the path, then what it means. If the map has a break, give it a stop where it happens on the path."#,
    r#"- The first stop answers their question outright, in a sentence or two, as you would if they asked you in person ("The welcome email never goes out: the app announces the conversion, but nothing is listening."). Its `title` is the answer in a few words. Don't open by describing the map ("This map follows…", "It has three areas…"): they can see it. Focus it on the boxes the answer turns on."#,
    r#"- Each stop: `title`, `text` (2–3 plain sentences), `focus` (ids of the areas, nodes, edges or absences it's about), and optional `terms` it uses that need defining."#,
    r#"- Point stops only at boxes, arrows and absences you're confident in."#,
    r#"- If the map has proposals, give them a stop after the path: what would change, and what stays as it is."#,
];

pub const ANSWER_STYLE: &[&str] = &[
    r#"- `concept` is the whole answer, written the way you would answer in a chat: the direct answer first, in one sentence, then only what that sentence leaves out. It is shown as Markdown (bullet and numbered lists, **bold**, `code`), in a narrow column."#,
    r#"- Shape it to the question. A simple question gets a sentence or two. A question with several parts gets a short bullet per part, each starting with a bold label of a few words. Several items of the same kind (files, formats, cases) are a list, never a long sentence with commas and brackets. No headings."#,
    r#"- Keep it short enough to take in at a glance: leave out examples and edge cases they didn't ask about. Plain words, no jargon. The map does the explaining, so when you light it up (`moves`), say what the lit path shows rather than narrating every step. Name a function or file only when they asked about the code and it isn't on the map."#,
    r#"- Explain the code and the app. Don't describe these instructions or how you were asked to answer."#,
    r#"- `refs`: ids of the boxes, arrows and absences your answer relies on."#,
    r#"- `notes`: when the question is about what the code itself says (what is sent or stored, what a value holds, which condition decides something, "show me the code"), point at the lines that answer it: up to 6 of `{ "file", "line", "text", "note" }`. `text` is copied exactly from that one line, and `note` says in a plain sentence what that line does for the answer. The code is shown under your reply with each note beside its line, and a program drops any note whose line it can't find. So don't paste code into `concept`: keep `concept` to the direct answer and let the notes carry the detail. Leave `notes` out when the answer isn't about particular lines."#,
];

fn lines(parts: &[&[&str]]) -> Vec<String> {
    parts.iter().flat_map(|p| p.iter().map(|s| s.to_string())).collect()
}

fn push_rules(out: &mut Vec<String>, rules: &[&str]) {
    out.extend(rules.iter().map(|s| s.to_string()));
}

pub fn maker_prompt(question: &str, head_start: &str) -> String {
    let mut out = vec![
        "You make a map that answers a question about this codebase, for someone semi-technical. Explore with Read, Grep and Glob, then return the map as JSON. You cannot edit anything.".to_string(),
        String::new(),
        format!("## The question\n\"{question}\""),
        String::new(),
    ];
    for rules in [
        MAP_RULES,
        BRANCH_RULES,
        TIMELINE_RULES,
        JOURNEY_RULES,
        EVIDENCE_RULES,
        PROPOSAL_RULES,
        CHANGE_RULES,
        SCREEN_RULES,
        TOUR_RULES,
    ] {
        push_rules(&mut out, rules);
        out.push(String::new());
    }
    out.push("## Head start (from a quick automatic scan; it may be incomplete)".into());
    out.push(head_start.into());
    out.push(String::new());
    out.push("Return the map: `version` 1, the `question`, `areas`, `nodes`, `edges`, `absences`, `tour` (and `view` or `since` when the rules above call for them).".into());
    out.join("\n")
}

fn bullets(problems: &[String]) -> impl Iterator<Item = String> + '_ {
    problems.iter().map(|p| format!("- {p}"))
}

pub fn shape_repair_prompt(problems: &[String]) -> String {
    let mut out = vec!["The map you returned has problems that stop it from being drawn:".to_string()];
    out.extend(bullets(problems));
    out.extend([String::new(), "Return the complete corrected map.".into()]);
    out.join("\n")
}

pub fn patch_repair_prompt(problems: &[String]) -> String {
    let mut out = vec!["Your changes couldn't be applied to the map:".to_string()];
    out.extend(bullets(problems));
    out.extend([
        String::new(),
        "Return the corrected changes, in the same form: only what changes, each item complete.".into(),
    ]);
    out.join("\n")
}

/// What a repair prompt needs to suggest real names: the function names the code facts know in a file.
pub type SymbolsIn<'a> = &'a dyn Fn(&str) -> Vec<String>;

/// "the code has no symbol a.ts#save" → the same reason, plus the names in a.ts that end in `save` (or a few names).
fn with_suggestion(reason: &str, symbols_in: Option<SymbolsIn<'_>>) -> String {
    let (Some(rest), Some(symbols_in)) = (reason.strip_prefix("the code has no symbol "), symbols_in) else {
        return reason.to_string();
    };
    // `(.+)#(.+)` is greedy: the file is everything up to the last `#`.
    let Some((file, name)) = rest.rsplit_once('#').filter(|(f, n)| !f.is_empty() && !n.is_empty()) else {
        return reason.to_string();
    };
    let last = name.rsplit('.').next().unwrap_or(name);
    let names = symbols_in(file);
    let close: Vec<&String> = names.iter().filter(|n| *n == last || n.ends_with(&format!(".{last}"))).collect();
    if !close.is_empty() {
        let list: Vec<String> = close.iter().take(3).map(|n| format!("{file}#{n}")).collect();
        return format!("{reason} (in that file it's {})", list.join(" or "));
    }
    if names.is_empty() {
        reason.to_string()
    } else {
        format!("{reason} (that file has: {})", names.iter().take(12).cloned().collect::<Vec<_>>().join(", "))
    }
}

pub fn evidence_repair_prompt(report: &CheckReport, symbols_in: Option<SymbolsIn<'_>>) -> String {
    let why =
        |reasons: &[String]| reasons.iter().map(|r| with_suggestion(r, symbols_in)).collect::<Vec<_>>().join("; ");
    let mut found: Vec<String> = Vec::new();
    for (id, c) in &report.nodes {
        if c.status == Status::Unconfirmed {
            found.push(format!("- box {id}: {}", why(&c.reasons)));
        }
    }
    for (id, c) in &report.edges {
        if c.status == Status::Unconfirmed {
            found.push(format!("- arrow {id}: {}", why(&c.reasons)));
        }
    }
    for p in settled_proposals(report) {
        let kind = p.kind.as_str();
        found.push(if kind == "add" {
            format!("- {} {}: you propose adding it, but the code already has it. Draw it as code that exists (cite its line, no `proposed`), or drop it.", p.what, p.id)
        } else {
            format!("- {} {}: you propose to {kind} it, so it must cite code that is there now: {}", p.what, p.id, why(&p.reasons))
        });
    }
    for (id, c) in &report.absences {
        if c.status != Status::Confirmed {
            let at = if c.locations.is_empty() {
                String::new()
            } else {
                let l: Vec<String> = c.locations.iter().take(5).map(|l| format!("{}:{}", l.file, l.line)).collect();
                format!(" (found at {})", l.join(", "))
            };
            found.push(format!("- absence {id}: {}{at}", c.reasons.join("; ")));
        }
    }
    let screens: Vec<String> = report
        .screens
        .iter()
        .filter(|(_, c)| c.status != nonio_core::screens::ScreenStatus::Confirmed)
        .map(|(id, c)| format!("- screen on box {id}: {}", c.reasons.join("; ")))
        .collect();
    let mut out = vec!["The checker re-read the code and could not confirm these claims:".to_string()];
    out.extend(found);
    if !screens.is_empty() {
        out.push(String::new());
        out.push(
            "These screens can't be shown either (fix the `screen` on the box, or return the box without it):".into(),
        );
        out.extend(screens);
    }
    out.extend(lines(&[&[
        "",
        "Fix only these. For each one, correct it (exact line text, right line number, a real symbol name, the right arrow kind) or drop it if it isn't true. If an absence was wrong, the code is connected after all: drop the absence and add that arrow.",
        "",
        "Return only the changes, not the whole map. Put each corrected box, arrow or absence (complete, with all its fields and the same id) and anything new in `nodes`, `edges`, `absences` or `areas`, and the ids of claims to drop in `remove`. Everything you leave out stays as it is.",
    ]]));
    out.join("\n")
}

/// A box or arrow the person selected on the map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionLine {
    pub id: String,
    pub label: String,
}

/// A turn of the conversation on a map, as the next answer sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EarlierTurn {
    pub question: String,
    pub concept: String,
}

/// A saved map, as an answer may point at it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtherMap {
    pub id: String,
    pub question: String,
}

/// The map, what the checker couldn't confirm, what they selected and their question: shared by both answer prompts.
fn question_context(map: &CodeMap, report: &CheckReport, selection: &[SelectionLine], question: &str) -> Vec<String> {
    let mut unconfirmed: Vec<&str> = Vec::new();
    unconfirmed.extend(report.nodes.iter().filter(|(_, c)| c.status == Status::Unconfirmed).map(|(id, _)| id.as_str()));
    unconfirmed.extend(report.edges.iter().filter(|(_, c)| c.status == Status::Unconfirmed).map(|(id, _)| id.as_str()));
    unconfirmed
        .extend(report.absences.iter().filter(|(_, c)| c.status == Status::Unconfirmed).map(|(id, _)| id.as_str()));
    let proposals: Vec<String> = report
        .nodes
        .iter()
        .chain(report.edges.iter())
        .filter_map(|(id, c)| {
            c.proposal.map(|p| {
                let state = if p.state == nonio_core::check::ProposalState::Done {
                    "now done in the code"
                } else {
                    "not done yet"
                };
                format!("{id} ({}: {state})", p.kind.as_str())
            })
        })
        .collect();
    let mut out = vec![
        "## The map (JSON)".to_string(),
        serde_json::to_string(map).expect("a map serializes"),
        if unconfirmed.is_empty() {
            "\nThe checker confirmed every claim in the map about the code as it is.".to_string()
        } else {
            format!(
                "\nThe checker could not confirm: {}. Don't rely on those without checking.",
                unconfirmed.join(", ")
            )
        },
    ];
    if !proposals.is_empty() {
        out.push(format!(
            "These are proposals, not code as it is; the checker says where each stands: {}.",
            proposals.join(", ")
        ));
    }
    out.extend(changes_context(report));
    out.push(String::new());
    out.push(if selection.is_empty() {
        "## They selected nothing (the question is about the whole map)".to_string()
    } else {
        let s: Vec<String> = selection.iter().map(|s| format!("- {}: {}", s.id, s.label)).collect();
        format!("## They selected\n{}", s.join("\n"))
    });
    out.push(String::new());
    out.push(format!("## Their question\n\"{question}\""));
    out
}

/// On a map of a change: what the checker found new, different or gone, with the lines of each changed function.
fn changes_context(report: &CheckReport) -> Vec<String> {
    let Some(since) = &report.since else { return Vec::new() };
    let Some(commit) = &since.commit else {
        return vec![format!(
            "\nThis map is meant to show a change since \"{}\", but that isn't a commit in this repo, so nothing could be compared.",
            since.reference
        )];
    };
    let marked: Vec<(&String, &nonio_core::check::ChangeCheck)> = report
        .nodes
        .iter()
        .chain(report.edges.iter())
        .filter_map(|(id, c)| c.change.as_ref().map(|ch| (id, ch)))
        .collect();
    let short: String = commit.chars().take(12).collect();
    let subject = since.subject.as_ref().map(|s| format!(" (\"{s}\")")).unwrap_or_default();
    let mut out = vec![
        format!("\nThis map shows a change: the code as it is now, compared with commit {short}{subject}. The checker worked out what differs:"),
        if marked.is_empty() {
            "nothing on this map differs from that commit.".to_string()
        } else {
            let list: Vec<String> = marked.iter().map(|(id, c)| format!("{id} ({})", change_kind(c.kind))).collect();
            format!("{}. Everything else on the map is the same as it was.", list.join(", "))
        },
    ];
    for (id, c) in &marked {
        if let Some(diff) = c.diff.as_ref().filter(|d| !d.is_empty()) {
            out.push(format!("Lines of {id} that changed (- before, + now):"));
            out.extend(diff.iter().cloned());
        }
    }
    out
}

fn change_kind(k: nonio_core::check::ChangeKind) -> &'static str {
    match k {
        nonio_core::check::ChangeKind::Added => "added",
        nonio_core::check::ChangeKind::Changed => "changed",
        nonio_core::check::ChangeKind::Removed => "removed",
    }
}

/// The conversation so far, oldest first, so a follow-up ("show me that in the code") knows what "that" is.
fn conversation_so_far(earlier: &[EarlierTurn]) -> Vec<String> {
    if earlier.is_empty() {
        return Vec::new();
    }
    let mut out = vec![
        "## The conversation so far on this map (oldest first)".to_string(),
        "The new question may follow on from these: read \"that\", \"it\" or \"why?\" against them.".into(),
    ];
    out.extend(earlier.iter().map(|e| format!("- \"{}\": {}", e.question, e.concept)));
    out.push(String::new());
    out
}

/// The first, cheap try: answer from the map and the code around its citations, with no tools.
pub fn quick_answer_prompt(
    map: &CodeMap,
    report: &CheckReport,
    selection: &[SelectionLine],
    question: &str,
    excerpts: &str,
    earlier: &[EarlierTurn],
    other_maps: &[OtherMap],
) -> String {
    let mut out = vec![
        "You answer questions about a code map for someone semi-technical, using only the map and the code excerpts below. You have no tools: you can't read more of the repo.".to_string(),
        String::new(),
    ];
    out.extend(question_context(map, report, selection, question));
    out.push(String::new());
    out.extend(conversation_so_far(earlier));
    out.push("## Code around what the map cites".into());
    out.push(if excerpts.is_empty() { "(none)".into() } else { excerpts.into() });
    out.push(String::new());
    out.push("## Other maps in this repo".into());
    out.push(if other_maps.is_empty() {
        "(none)".into()
    } else {
        other_maps.iter().map(|m| format!("- {}: \"{}\"", m.id, m.question)).collect::<Vec<_>>().join("\n")
    });
    out.extend(lines(&[&[
        "",
        "## First decide how to handle it (`route`)",
        "- `answer`: the map and the excerpts fully answer it, without guessing about code you can't see. Then answer below.",
        "- `code`: it's about what this map shows (the same feature, or something its boxes lead to or come from), but answering needs code that isn't shown, or they ask for a change or fix, or a good answer would add boxes or arrows to this map. Say in `missing`, in one sentence, what you would need to look at, and leave the answer fields empty. Someone with the whole repo picks it up from there.",
        "- `elsewhere`: it's about a different part of the app that this map doesn't touch, so adding it here would bolt an unrelated area onto this map. Put in `newQuestion` their question in their own words, filling in only what it needs to make sense without this conversation (\"it\" becomes what it refers to). Don't add a route, function or detail they didn't ask about: the new map answers exactly what this says, so anything you add narrows it. If one of the other maps above already covers it, put that map's id in `existingMap`. Leave the answer fields empty. They'll be asked whether to make a new map. Only use this when the question really is about something else; when in doubt, use `code`.",
        "",
        "## How to answer (when `route` is `answer`)",
    ]]));
    push_rules(&mut out, ANSWER_STYLE);
    out.extend(lines(&[&[
        "- For `notes` you can only point at lines in the excerpts above: the number before `|` is the line. If the lines that answer it aren't there, that is a reason to route to `code`.",
        "- `moves`: `focus` on the most relevant id; `highlight` the ids that make up the path that answers the question.",
    ]]));
    out.join("\n")
}

pub fn explainer_prompt(
    map: &CodeMap,
    report: &CheckReport,
    selection: &[SelectionLine],
    question: &str,
    missing: &str,
    words_only: bool,
    earlier: &[EarlierTurn],
) -> String {
    if words_only {
        let mut out = vec![
            "You answer questions about a codebase for someone semi-technical. You can Read, Grep and Glob the repo; you cannot edit anything.".to_string(),
            String::new(),
        ];
        out.extend(question_context(map, report, selection, question));
        out.push(String::new());
        out.extend(conversation_so_far(earlier));
        out.extend(lines(&[&[
            "## This question is about a different part of the app than the map shows",
            "They chose an answer in words rather than a new map. Explore the code to answer it, but leave out `add` and `fix`: nothing will be drawn on this map. Use `refs` and `moves` only for ids on this map that really matter to the answer (often none).",
            "",
            "## How to answer",
        ]]));
        push_rules(&mut out, ANSWER_STYLE);
        return out.join("\n");
    }
    let mut out = vec![
        "You answer questions about a code map for someone semi-technical. You can Read, Grep and Glob the repo; you cannot edit anything.".to_string(),
        String::new(),
    ];
    out.extend(question_context(map, report, selection, question));
    if map.view == Some(View::Journey) {
        out.extend(lines(&[&[
            "",
            "## This map is a journey",
            "It shows only what a person sees and does. Anything you add must be the same: `kind` is `screen`, `action` or `choice`, and no boxes for what goes on behind the screen. Say that part in words. A box or arrow you add off the main path carries the `branch` it is on (an existing branch's name, or a new short one in the person's words); one on the main path carries none.",
        ]]));
    }
    if let Some(since) = &map.since {
        let short: String = since.chars().take(12).collect();
        out.push(String::new());
        out.push("## This map shows a change".into());
        out.push(format!("It compares the code as it is now with commit {short}, and a program marks what is new or different since then. Anything you add is compared the same way, so don't mark it yourself. Only code that was deleted since that commit gets `\"removed\": true`, citing its lines as they were then (an arrow to or from a removed box is removed too)."));
    }
    if !missing.is_empty() {
        out.push(String::new());
        out.push(format!("## Why the map alone wasn't enough\n{missing}"));
    }
    out.push(String::new());
    out.extend(conversation_so_far(earlier));
    out.push("## How to answer".into());
    push_rules(&mut out, ANSWER_STYLE);
    out.extend(lines(&[&[
        "- If the map doesn't cover it, explore. Put what you found in `add` (new areas, nodes, edges or absences with evidence, same rules as the map; ids must be new) only when it belongs to the flow this map follows and seeing it drawn helps answer the question. If the answer is a fact you can say in words, or about something beside the flow, leave `add` empty. Connect new boxes to existing ones with arrows where the code really connects them.",
        "- `moves`: `focus` on the most relevant id; `highlight` the ids that make up the path that answers the question.",
        "- If they ask what a change would look like, how you would do something, or for a plan, without asking you to make it: draw it on the map as proposals (see Proposals). New boxes and arrows go in `add` with `proposed`; for a box or arrow already on the map that would change or go, put `{ \"id\": \"<its id>\", \"proposed\": { \"kind\": \"change\" | \"remove\", \"note\": \"…\" } }` in `propose`. Say in the answer what the proposal is; nothing is edited.",
        "- If they ask you to make a change, don't edit (you can't). Propose a `fix`: nonio shows it to them on the map, and if they approve, nonio applies exactly your edits and re-checks the map. So describe it as a proposal waiting for their approval, never as something for them to apply by hand.",
        "- The `fix`: `summary` in plain words; `edits` as exact replacements (`find` copied exactly from the file, occurring once; `replace` is the new text); `creates` for new files. If the fix creates a connection between two boxes, fill `connects` with `from`, `to`, a short `label`, and `evidenceFile` + `evidenceText` (text that will appear in the new code). Keep fixes small and in the style of the surrounding code.",
        "- When the question is about what the person sees at a step (\"what does this look like?\", \"where is that button?\"), answer with a picture: put `{ \"node\": \"<box id>\", \"screen\": { … } }` in `screens`, for a box already on the map or one in `add` (see Screens below). In an answer a wireframe can only use a page already in the map's `pages`; when the map has no page for it, a sketch of the component will do. Otherwise leave `screens` out: most questions don't need one.",
        "",
    ]]));
    for rules in [EVIDENCE_RULES, BRANCH_RULES, PROPOSAL_RULES] {
        push_rules(&mut out, rules);
        out.push(String::new());
    }
    push_rules(&mut out, SCREEN_RULES);
    out.join("\n")
}
