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
    for id in ["vue-java", "tsx-kotlin", "nuxt", "next", "through-client", "four-hops", "bff", "client-instance"] {
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

#[test]
fn a_request_arrow_follows_call_facts_until_a_boundary() {
    let r = report();
    let status = |id: &str| r.edges[id].status;
    // OrderDetail.vue#open → api/orders.ts#getOrder → fetch(`/api/orders/${id}`).
    assert_eq!(status("through-client"), Status::Confirmed);
    // chain.ts#h1 → h2 → h3 → h4 → h5 fetches: no fixed number of calls.
    assert_eq!(status("four-hops"), Status::Confirmed);
    // A route handler that fetches another API (a backend for the frontend), from its own box.
    assert_eq!(status("bff"), Status::Confirmed);
    // Far.vue#far → chain.ts#h1, but h1 is a box of the map: the arrow should go through it.
    assert_eq!(status("through-a-box"), Status::Unconfirmed);
    // warm.ts#warm → GET and Shell.tsx#Shell → PayButton: both callees are boxes of the map.
    assert_eq!(status("into-entry"), Status::Unconfirmed);
    assert_eq!(status("into-component"), Status::Unconfirmed);
}

/// The shop map without some boxes (and their arrows), checked.
fn without(ids: &[&str]) -> nonio_core::CheckReport {
    let mut m = json("shop.map.json");
    m["nodes"].as_array_mut().unwrap().retain(|n| !ids.contains(&n["id"].as_str().unwrap()));
    m["edges"]
        .as_array_mut()
        .unwrap()
        .retain(|e| !ids.contains(&e["from"].as_str().unwrap()) && !ids.contains(&e["to"].as_str().unwrap()));
    let root = shop();
    let scanner = nonio_scan::Scanner::new(&root, files(&root));
    nonio_core::check(&load(&m), &root, &scanner, &nonio_core::CheckOptions::default()).unwrap()
}

#[test]
fn another_component_or_entry_is_a_boundary_even_when_it_is_not_a_box() {
    let r = without(&["pay", "sapi"]);
    // warm.ts#warm → the Next route handler GET, which fetches: a route handler is another entry of the app.
    assert_eq!(r.edges["into-entry"].status, Status::Unconfirmed);
    // Shell.tsx#Shell → PayButton.tsx#PayButton, which posts: another component.
    assert_eq!(r.edges["into-component"].status, Status::Unconfirmed);
    assert_eq!(
        r.edges["into-component"].reasons,
        vec!["no request from web/src/components/Shell.tsx#Shell matches a route handled by backend/src/main/kotlin/com/shop/billing/InvoiceController.kt#InvoiceController.pay"]
    );
}

#[test]
fn without_the_box_in_between_the_chain_is_followed_to_its_end() {
    // Far.vue#far → h1 → h2 → h3 → h4 → h5: five calls.
    assert_eq!(without(&["h1"]).edges["through-a-box"].status, Status::Confirmed);
}

#[test]
fn http_clients_are_linked_by_binding_not_by_name() {
    let r = report();
    // `api` is imported from http.ts, where it is `axios.create({ baseURL: '/api' })`: GET /api/invoices.
    assert_eq!(r.edges["client-instance"].status, Status::Confirmed);
    // `external`'s base URL comes from the environment: the request proves nothing.
    assert_eq!(r.edges["unknown-prefix"].status, Status::Unconfirmed);
    // A local object named `axios` is not the library.
    assert_eq!(r.edges["not-axios"].status, Status::Unconfirmed);
}
