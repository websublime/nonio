//! Phase 4 exit: arrows from a component to the controller (or handler) its request reaches, confirmed by the facts
//! of both sides: a fetch site in the component and a route whose handler is the controller method. The `shop`
//! fixture has a Vue SFC and a React component calling Spring controllers in Java and Kotlin, a Nuxt page calling a
//! Nitro handler and a Next client component calling an app route.

mod common;

use std::collections::HashMap;

use common::*;
use nonio_core::{Proof, Status};

fn shop() -> std::path::PathBuf {
    fixtures().join("shop")
}

fn files(root: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out
}

fn report() -> nonio_core::CheckReport {
    let root = shop();
    let scanner = nonio_scan::Scanner::new(&root, files(&root));
    nonio_core::check(&load(&json("shop.map.json")), &root, &scanner, &nonio_core::CheckOptions::default()).unwrap()
}

#[test]
fn component_to_controller_arrows_are_confirmed_by_facts() {
    let r = report();
    let got: HashMap<&str, (Status, Option<Proof>, Vec<String>)> =
        r.edges.iter().map(|(id, c)| (id.as_str(), (c.status, c.proof, c.reasons.clone()))).collect();
    for id in ["vue-java", "tsx-kotlin", "nuxt", "next"] {
        assert_eq!(got[id], (Status::Confirmed, Some(Proof::Fact), vec![]), "{id}");
    }
    assert!(r.nodes.values().all(|n| n.status == Status::Confirmed), "{:?}", r.nodes);
}

#[test]
fn a_request_to_another_handler_of_the_same_controller_fails() {
    let r = report();
    let e = &r.edges["wrong-route"];
    assert_eq!(e.status, Status::Unconfirmed);
    assert_eq!(
        e.reasons,
        vec!["no request from web/src/components/OrderList.vue#load matches a route handled by backend/src/main/java/com/shop/orders/OrderController.java#OrderController.get"]
    );
}
