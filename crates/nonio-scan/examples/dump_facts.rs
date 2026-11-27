//! Print the facts of the given files of a repo as JSON (`{path: facts}`), calls unsettled. Used by
//! tools/compare-facts to compare nonio's oxc backend with illuminate's extractor.
//!
//! cargo run -p nonio-scan --example dump_facts -- <root> <file>...

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;

use nonio_facts::{Extractor, RepoView};
use nonio_scan::OxcExtractor;

struct Disk(PathBuf);

impl RepoView for Disk {
    fn exists(&self, path: &str) -> bool {
        self.0.join(path).is_file()
    }
    fn text(&self, path: &str) -> Option<Rc<str>> {
        fs::read_to_string(self.0.join(path)).ok().map(Rc::from)
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(args.next().expect("usage: dump_facts <root> <file>..."));
    let view = Disk(root.clone());
    let oxc = OxcExtractor::new(&root);
    let mut out = BTreeMap::new();
    for file in args {
        if let Some(text) = view.text(&file) {
            if let Some(facts) = oxc.extract(&view, &file, &text) {
                out.insert(file, facts);
            }
        }
    }
    println!("{}", serde_json::to_string(&out).unwrap());
}
