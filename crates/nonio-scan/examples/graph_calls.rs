//! Q10 harness: every call the Java/Kotlin/Python backend gives for a repo, with the lines that make it, and the
//! raw `code2graph` precise-tier call edges the adapter dropped. `cargo run --release -p nonio-scan --example
//! graph_calls -- <repo> [kept|dropped|summary]`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::rc::Rc;

use code2graph::resolve::{LayeredResolver, Resolver};
use code2graph::RefRole;
use nonio_scan::{graph_facts, is_graph_file};

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
    for e in fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if p.is_dir() {
            if !matches!(name.as_str(), ".git" | "node_modules" | "target" | "build" | ".venv") {
                walk(root, &p, out);
            }
        } else {
            let rel = p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            if is_graph_file(&rel) {
                out.push(rel);
            }
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = Path::new(&args[1]);
    let mode = args.get(2).map_or("summary", String::as_str);
    let mut files = Vec::new();
    walk(root, root, &mut files);
    files.sort();
    let texts: Vec<(String, Rc<str>)> =
        files.iter().filter_map(|f| Some((f.clone(), Rc::from(fs::read_to_string(root.join(f)).ok()?)))).collect();
    let start = std::time::Instant::now();
    let facts = graph_facts(&texts);
    let took = start.elapsed();
    let text_of: HashMap<&str, &str> = texts.iter().map(|(p, t)| (p.as_str(), &**t)).collect();

    // Raw precise tier, for comparison.
    let parsed: Vec<_> = texts.iter().filter_map(|(p, t)| code2graph::extract_path(p, t).ok()).collect();
    let graph = LayeredResolver::default_scoped().resolve(&parsed).unwrap();
    let raw: Vec<_> =
        graph.edges.iter().filter(|e| e.role == RefRole::Call && !e.to.to_scip_string().starts_with("local")).collect();
    let calls: Vec<_> = facts.values().flat_map(|f| f.calls.iter()).collect();
    let mut by_conf: HashMap<String, usize> = HashMap::new();
    for c in &calls {
        *by_conf.entry(format!("{:?}", c.confidence)).or_default() += c.count as usize;
    }
    let call_refs: usize = parsed.iter().map(|f| f.references.iter().filter(|r| r.role == RefRole::Call).count()).sum();
    match mode {
        "kept" => {
            for f in facts.values() {
                let syms: HashMap<&str, _> = f.symbols.iter().map(|s| (s.id.as_str(), s)).collect();
                for c in &f.calls {
                    let leaf = c.to.rsplit(['#', '.']).next().unwrap_or("");
                    let body = syms
                        .get(c.from.as_str())
                        .map_or(text_of[f.path.as_str()], |s| &text_of[f.path.as_str()][s.span.start..s.span.end]);
                    let lines: Vec<&str> =
                        body.lines().filter(|l| l.contains(&format!("{leaf}("))).map(str::trim).collect();
                    println!("{:?}\t{}\t{}\t{}", c.confidence, c.from, c.to, lines.join(" | "));
                }
            }
        }
        "dropped" => {
            // A raw edge is kept when some kept call goes from the same file to the same leaf name.
            let kept: HashSet<(String, String)> = facts
                .values()
                .flat_map(|f| {
                    f.calls
                        .iter()
                        .map(move |c| (f.path.clone(), c.to.rsplit(['#', '.']).next().unwrap_or("").to_string()))
                })
                .collect();
            for e in &raw {
                let leaf = e.to.leaf_name().unwrap_or("").to_string();
                if !kept.contains(&(e.occ.file.clone(), leaf)) {
                    let line = text_of[e.occ.file.as_str()].lines().nth(e.occ.line as usize - 1).unwrap_or("").trim();
                    println!("{:?}\t{}:{}\t{}\t{}", e.provenance, e.occ.file, e.occ.line, e.to.to_scip_string(), line);
                }
            }
        }
        _ => println!(
            "files {} | call sites {} | raw precise call edges {} | kept calls {} ({:?}) | {:?}",
            texts.len(),
            call_refs,
            raw.len(),
            calls.iter().map(|c| c.count as usize).sum::<usize>(),
            by_conf,
            took
        ),
    }
}
