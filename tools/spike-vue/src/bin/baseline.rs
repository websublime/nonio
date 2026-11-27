//! Baseline for Q11: nonio's own oxc 0.153 backend (`nonio-scan`) on the same `.vue` files, with no Vue crate.
//!
//! `raw` passes the `.vue` text as is (today's behaviour: it does not parse as TS, so only the regex fallback runs).
//! `blank` keeps the contents of the `<script>` blocks and turns every other byte into a space (newlines kept), so
//! the byte offsets stay those of the `.vue` file.
//!
//! Run: `cargo run --manifest-path tools/spike-vue/Cargo.toml --bin baseline -- tools/spike-vue/fixture blank <file>...`

use nonio_facts::{Extractor, RepoView};
use std::path::PathBuf;
use std::rc::Rc;
struct Disk(PathBuf, bool);
impl RepoView for Disk {
    fn exists(&self, p: &str) -> bool {
        self.0.join(p).is_file()
    }
    fn text(&self, p: &str) -> Option<Rc<str>> {
        std::fs::read_to_string(self.0.join(p))
            .ok()
            .map(|t| Rc::from(if self.1 && p.ends_with(".vue") { blank(&t) } else { t }))
    }
}
/// Keep only the contents of <script> blocks; every other byte becomes a space (newlines kept), so offsets are the .vue's.
fn blank(s: &str) -> String {
    let mut keep = vec![false; s.len()];
    let mut from = 0;
    while let Some(i) = s[from..].find("<script") {
        let o = from + i;
        let h = o + s[o..].find('>').unwrap() + 1;
        let c = h + s[h..].find("</script>").unwrap();
        keep[h..c].fill(true);
        from = c;
    }
    s.bytes().enumerate().map(|(i, b)| if keep[i] || b == b'\n' { b as char } else { ' ' }).collect()
}
fn main() {
    let mut a = std::env::args().skip(1);
    let root = PathBuf::from(a.next().unwrap());
    let mode = a.next().unwrap() == "blank";
    let v = Disk(root.clone(), mode);
    let ex = nonio_scan::OxcExtractor::new(&root);
    for f in a {
        let t = v.text(&f).unwrap();
        let facts = ex.extract(&v, &f, &t).unwrap();
        println!("== {f} ({})", if mode { "blanked" } else { "raw" });
        println!("imports: {:?}", facts.imports.iter().map(|i| &i.to).collect::<Vec<_>>());
        println!("symbols: {:?}", facts.symbols.iter().map(|s| (&s.id, s.span.start, s.span.end)).collect::<Vec<_>>());
        for c in &facts.calls {
            println!("call: {}", serde_json::to_string(c).unwrap());
        }
    }
}
