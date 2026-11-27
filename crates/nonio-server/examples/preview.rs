//! Shows the viewer on a saved map without any model: the real server, on a throwaway copy of the care-app fixture,
//! with the care-app map saved in it. For UI work: build the viewer (`npm run build` in `viewer/`), then
//! `cargo run -p nonio-server --example preview` and open the printed link. Questions asked on the page wait forever.
//! `PORT` picks the port (any free one by default).

use std::path::Path;
use std::sync::Arc;

use nonio_core::parse_map;
use nonio_server::bridge::{Agent, AgentRequest, AgentResult, BoxFuture};
use nonio_server::server::{start_server, ServerOptions};
use nonio_server::store::{MapMeta, Store, StoredMap};

struct Nobody;

impl Agent for Nobody {
    fn ask(&self, _: AgentRequest) -> BoxFuture<AgentResult> {
        Box::pin(std::future::pending())
    }
}

fn copy(from: &Path, to: &Path) {
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let target = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            std::fs::create_dir_all(&target).unwrap();
            copy(&e.path(), &target);
        } else {
            std::fs::copy(e.path(), target).unwrap();
        }
    }
}

#[tokio::main]
async fn main() {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixtures = here.join("../nonio-core/tests/fixtures");
    let root = tempfile::Builder::new().prefix("nonio-preview-").tempdir().unwrap().keep();
    copy(&fixtures.join("care-app"), &root);
    let raw = std::fs::read_to_string(fixtures.join("care-app.map.json")).unwrap();
    let map = parse_map(&serde_json::from_str(&raw).unwrap()).expect("the care-app map");
    let now = "2026-11-27T00:00:00.000Z".to_string();
    let meta = MapMeta {
        id: "patient-converts".into(),
        question: map.question.clone(),
        created_at: now.clone(),
        updated_at: now,
        commit: None,
        maker_session: None,
        explainer_session: None,
        cost: 0.0,
        parent: None,
    };
    Store::new(&root).save(&StoredMap { meta, map, answers: Vec::new(), fixes: Default::default() }).unwrap();

    let mut o = ServerOptions::new(&root, Arc::new(Nobody));
    o.port = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(0);
    o.viewer = Some(here.join("../../viewer/dist"));
    let server = start_server(o).await.unwrap();
    println!("{}", server.page_url(Some("patient-converts")));
    // Runs until Ctrl+C, or until the last tab closes and the server stops itself.
    let _keep = server;
    std::future::pending::<()>().await;
}
