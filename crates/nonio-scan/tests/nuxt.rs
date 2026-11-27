//! Nuxt routes need a `nuxt.config` above the file: the extractor finds it on disk.

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

#[test]
fn server_routes_only_below_a_nuxt_config() {
    let dir = tempfile::tempdir().unwrap();
    let write = |p: &str, t: &str| {
        let p = dir.path().join(p);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, t).unwrap();
    };
    let handler = "export default defineEventHandler(() => []);\n";
    write("web/nuxt.config.ts", "export default defineNuxtConfig({});\n");
    write("web/server/api/orders.get.ts", handler);
    write("api/server/api/orders.get.ts", handler);
    let repo = Disk(dir.path().to_path_buf());
    let ex = OxcExtractor::new(dir.path());
    let ids = |p: &str| -> Vec<String> {
        ex.extract(&repo, p, &repo.text(p).unwrap()).unwrap().routes.into_iter().map(|r| r.id).collect()
    };
    assert_eq!(ids("web/server/api/orders.get.ts"), ["route:GET /api/orders"]);
    assert!(ids("api/server/api/orders.get.ts").is_empty());
}
