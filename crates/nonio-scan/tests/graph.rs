//! The phase 2 fixture: the spike in `05-spike-crates.md` (Spring in Java and Kotlin, FastAPI, TS and Vue), rebuilt
//! because the original harness isn't available. 20 hand-written calls between its symbols, and decoys: methods of
//! the same name in other classes (`create`, `save`, `send`, `quote`, `record`, `issue`, `compute`, `print`,
//! `check`, `load`), declared before and after the real ones, and a Python parameter named like a module
//! (`client.send()` next to `app/client.py#send`).
//!
//! Every call the scanner gives between symbols of the fixture must be a real one (zero wrong); the score of the
//! 20 is pinned so a change that loses one shows.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use nonio_facts::{settle_calls, symbol_names, Extractor, FileFacts, RepoView, SymbolKind};
use nonio_scan::{graph_facts, GraphExtractor, Scanner};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/spike05")
}

struct Disk(PathBuf);

impl RepoView for Disk {
    fn exists(&self, path: &str) -> bool {
        self.0.join(path).is_file()
    }
    fn text(&self, path: &str) -> Option<Rc<str>> {
        fs::read_to_string(self.0.join(path)).ok().map(Rc::from)
    }
}

fn files(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out.sort();
    out
}

const J: &str = "backend/src/main/java/com/acme/orders/";
const K: &str = "backend/src/main/kotlin/com/acme/billing/";

fn j(rest: &str) -> String {
    format!("{J}{rest}")
}
fn k(rest: &str) -> String {
    format!("{K}{rest}")
}

/// The 20 calls of the spike, with the case each one stands for.
fn expected() -> Vec<(&'static str, String, String)> {
    vec![
        (
            "java: injected field",
            j("OrderController.java#OrderController.create"),
            j("OrderService.java#OrderService.create"),
        ),
        ("java: static call", j("OrderController.java#OrderController.create"), j("Validation.java#Validation.check")),
        (
            "java: final field set in the constructor",
            j("OrderService.java#OrderService.create"),
            j("PriceService.java#PriceService.quote"),
        ),
        ("java: constructor", j("OrderService.java#OrderService.create"), j("Order.java#Order")),
        (
            "java: @Autowired field",
            j("OrderService.java#OrderService.create"),
            j("OrderRepository.java#OrderRepository.save"),
        ),
        (
            "java: @Autowired field",
            j("OrderService.java#OrderService.create"),
            j("NotificationService.java#NotificationService.send"),
        ),
        ("java: typed parameter", j("OrderService.java#OrderService.audit"), j("AuditLog.java#AuditLog.record")),
        (
            "kotlin: constructor property",
            k("InvoiceController.kt#InvoiceController.issue"),
            k("InvoiceService.kt#InvoiceService.issue"),
        ),
        ("kotlin: constructor", k("InvoiceService.kt#InvoiceService.issue"), k("Taxes.kt#TaxCalculator")),
        (
            "kotlin: local it constructs",
            k("InvoiceService.kt#InvoiceService.issue"),
            k("Taxes.kt#TaxCalculator.compute"),
        ),
        ("kotlin: constructor property", k("InvoiceService.kt#InvoiceService.issue"), k("Mailer.kt#Mailer.send")),
        ("kotlin: top-level function", k("InvoiceService.kt#InvoiceService.issue"), k("Taxes.kt#formatAmount")),
        ("kotlin: typed parameter", k("InvoiceService.kt#InvoiceService.bill"), k("Mailer.kt#Printer.print")),
        ("python: imported function", "py/app/main.py#post_user".into(), "py/app/services.py#create_user".into()),
        ("python: constructor", "py/app/main.py#get_user".into(), "py/app/repo.py#UserRepo".into()),
        ("python: local it constructs", "py/app/main.py#get_user".into(), "py/app/repo.py#UserRepo.load".into()),
        (
            "ts: member of an imported object",
            "web/src/api/client.ts#getOrders".into(),
            "web/src/api/http.ts#http.get".into(),
        ),
        (
            "ts: imported function",
            "web/src/composables/useOrders.ts#useOrders.load".into(),
            "web/src/api/client.ts#getOrders".into(),
        ),
        (
            "vue: top of <script setup>",
            "web/src/components/OrderList.vue".into(),
            "web/src/composables/useOrders.ts#useOrders".into(),
        ),
        (
            "vue: inside a <script setup> function",
            "web/src/components/OrderList.vue#refresh".into(),
            "web/src/api/client.ts#getOrders".into(),
        ),
    ]
}

/// Calls the fixture makes that are not among the 20 (still right when found).
fn also_true() -> Vec<(String, String)> {
    vec![("web/src/components/OrderList.vue#<template>".into(), "web/src/components/OrderList.vue#refresh".into())]
}

/// Every settled call between symbols of the fixture (not vendor, not to a whole file).
fn scanned() -> BTreeSet<(String, String)> {
    let root = root();
    let all = files(&root);
    let scanner = Scanner::new(&root, all.iter().cloned());
    let repo = Disk(root.clone());
    let facts: std::collections::HashMap<String, FileFacts> = all
        .iter()
        .filter_map(|f| {
            let text = fs::read_to_string(root.join(f)).ok()?;
            Some((f.clone(), scanner.extract(&repo, f, &text)?))
        })
        .collect();
    let names = symbol_names(&facts);
    facts
        .values()
        .flat_map(|f| settle_calls(&f.calls, |p| names.get(p)))
        .filter(|c| c.to.contains('#') && !c.to.starts_with("vendor:"))
        .map(|c| (c.from, c.to))
        .collect()
}

#[test]
fn spike_fixture_scores_with_zero_wrong() {
    let got = scanned();
    let expected = expected();
    let mut report = String::new();
    let mut correct = Vec::new();
    for (case, from, to) in &expected {
        let hit = got.contains(&(from.clone(), to.clone()));
        report.push_str(&format!("{} {case}: {from} -> {to}\n", if hit { "ok  " } else { "miss" }));
        if hit {
            correct.push(*case);
        }
    }
    let truth: BTreeSet<(String, String)> =
        expected.iter().map(|(_, f, t)| (f.clone(), t.clone())).chain(also_true()).collect();
    let wrong: Vec<&(String, String)> = got.iter().filter(|c| !truth.contains(c)).collect();
    for (from, to) in &wrong {
        report.push_str(&format!("WRONG {from} -> {to}\n"));
    }
    println!("{report}{} of 20, {} wrong", correct.len(), wrong.len());
    assert!(wrong.is_empty(), "{report}");
    assert_eq!(correct.len(), 12, "{report}");
}

#[test]
fn a_parameter_named_like_a_module_is_not_a_path() {
    // code2graph's precise tier resolves `client.send()` to app/client.py#send although `client` is a parameter.
    let root = root();
    let texts: Vec<(String, Rc<str>)> = ["py/app/main.py", "py/app/client.py", "py/app/services.py", "py/app/repo.py"]
        .iter()
        .map(|p| (p.to_string(), Rc::from(fs::read_to_string(root.join(p)).unwrap())))
        .collect();
    let facts = graph_facts(&texts);
    let main = &facts["py/app/main.py"];
    assert!(main.calls.iter().all(|c| !c.to.starts_with("py/app/client.py")), "{:?}", main.calls);
    assert!(main
        .calls
        .iter()
        .any(|c| c.from == "py/app/main.py#post_user" && c.to == "py/app/services.py#create_user"));
}

#[test]
fn symbols_and_imports() {
    let root = root();
    let all = files(&root);
    let ex = GraphExtractor::new(all.iter().cloned());
    let repo = Disk(root.clone());
    let path = j("OrderService.java");
    let text = fs::read_to_string(root.join(&path)).unwrap();
    let f = ex.extract(&repo, &path, &text).unwrap();
    let names: Vec<(&str, SymbolKind, bool, Option<&str>)> =
        f.symbols.iter().map(|s| (s.name.as_str(), s.kind, s.exported, s.parent.as_deref())).collect();
    let parent = j("OrderService.java#OrderService");
    assert_eq!(
        names,
        vec![
            ("OrderService", SymbolKind::Class, true, None),
            ("OrderService.repository", SymbolKind::Property, false, Some(parent.as_str())),
            ("OrderService.notifications", SymbolKind::Property, false, Some(parent.as_str())),
            ("OrderService.pricing", SymbolKind::Property, false, Some(parent.as_str())),
            ("OrderService.OrderService", SymbolKind::Method, true, Some(parent.as_str())),
            ("OrderService.create", SymbolKind::Method, true, Some(parent.as_str())),
            ("OrderService.audit", SymbolKind::Method, true, Some(parent.as_str())),
        ]
    );
    for s in &f.symbols {
        assert!(text[s.span.start..s.span.end].contains(s.name.rsplit('.').next().unwrap()), "{}", s.name);
    }
    let imports: Vec<&str> = f.imports.iter().map(|i| i.to.as_str()).collect();
    assert_eq!(
        imports,
        ["vendor:org.springframework.beans.factory.annotation", "vendor:org.springframework.stereotype"]
    );

    let main = ex.extract(&repo, "py/app/main.py", &fs::read_to_string(root.join("py/app/main.py")).unwrap()).unwrap();
    let imports: Vec<&str> = main.imports.iter().map(|i| i.to.as_str()).collect();
    assert_eq!(imports, ["vendor:fastapi", "py/app/services.py", "py/app/repo.py"]);
    // Files of other languages are not this backend's.
    assert!(ex.extract(&repo, "web/src/api/client.ts", "").is_none());
}

#[test]
fn a_changed_file_is_read_again() {
    let root = root();
    let all = files(&root);
    let ex = GraphExtractor::new(all.iter().cloned());
    let repo = Disk(root.clone());
    let path = k("InvoiceService.kt");
    let text = fs::read_to_string(root.join(&path)).unwrap();
    let before = ex.extract(&repo, &path, &text).unwrap();
    assert!(before.calls.iter().any(|c| c.to == k("Taxes.kt#formatAmount")));
    let edited = text.replace("return formatAmount(tax)", "return tax.toString()");
    let after = ex.extract(&repo, &path, &edited).unwrap();
    assert!(after.calls.iter().all(|c| c.to != k("Taxes.kt#formatAmount")));
    assert_ne!(before.hash, after.hash);
}

fn texts(files: &[(&str, &str)]) -> Vec<(String, Rc<str>)> {
    files.iter().map(|(p, t)| (p.to_string(), Rc::from(*t))).collect()
}

#[test]
fn a_java_receiver_it_does_not_capture_is_not_a_package_match() {
    // `this.store.persist()` reaches code2graph as an unqualified `persist`, which it links to the one `persist` in
    // the package. The field's type is a vendor class.
    let facts = graph_facts(&texts(&[
        (
            "q/J.java",
            "package q;\nimport org.other.Store;\npublic class J {\n    private Store store;\n    public void run() {\n        this.store.persist();\n    }\n}\n",
        ),
        ("q/Archive.java", "package q;\nclass Archive {\n    public void persist() {}\n}\n"),
    ]));
    assert!(facts["q/J.java"].calls.is_empty(), "{:?}", facts["q/J.java"].calls);
}

#[test]
fn a_kotlin_constructor_property_is_not_a_package_path() {
    // `billing.charge()` is a path lookup in code2graph, and `billing` is also a package with a `charge` function.
    let facts = graph_facts(&texts(&[
        (
            "k/T.kt",
            "package k\n\nclass T(private val billing: Foo) {\n    fun run() {\n        billing.charge()\n    }\n}\n",
        ),
        ("billing/Fns.kt", "package billing\n\nfun charge() {}\n"),
    ]));
    assert!(facts["k/T.kt"].calls.is_empty(), "{:?}", facts["k/T.kt"].calls);
}

#[test]
fn a_receiver_type_is_the_one_its_file_imports() {
    // code2graph keys member tables by the bare type name, so the first `Mailer` it reads wins: with `c` first it
    // links `m.dispatch()` to c/Mailer.kt although the file imports b.Mailer. Such an edge is dropped, never kept.
    let s = (
        "a/S.kt",
        "package a\n\nimport b.Mailer\n\nclass S {\n    fun run(m: Mailer) {\n        m.dispatch()\n    }\n}\n",
    );
    let b = ("b/Mailer.kt", "package b\n\nclass Mailer {\n    fun dispatch() {}\n}\n");
    let c = ("c/Mailer.kt", "package c\n\nclass Mailer {\n    fun dispatch() {}\n}\n");
    for order in [[s, b, c], [c, s, b], [s, c, b]] {
        let facts = graph_facts(&texts(&order));
        let calls: Vec<&str> = facts["a/S.kt"].calls.iter().map(|c| c.to.as_str()).collect();
        assert!(calls.iter().all(|c| c.starts_with("b/")), "{calls:?}");
    }
    let facts = graph_facts(&texts(&[s, b, c]));
    assert_eq!(facts["a/S.kt"].calls[0].to, "b/Mailer.kt#Mailer.dispatch");
}

#[test]
fn a_class_with_a_constructor_is_still_imported() {
    // The class and its constructor share the name, and code2graph resolves no import to an ambiguous name.
    let facts = graph_facts(&texts(&[
        (
            "a/S.java",
            "package a;\nimport b.Visit;\npublic class S {\n    public void run(Visit visit) {\n        visit.describe();\n    }\n}\n",
        ),
        ("b/Visit.java", "package b;\npublic class Visit {\n    public Visit() {}\n    public void describe() {}\n}\n"),
        ("c/Visit.java", "package c;\npublic class Visit {\n    public Visit() {}\n    public void describe() {}\n}\n"),
    ]));
    let s = &facts["a/S.java"];
    assert_eq!(s.imports.iter().map(|i| i.to.as_str()).collect::<Vec<_>>(), ["b/Visit.java"]);
}

#[test]
fn a_package_imported_whole_names_its_types() {
    let s = (
        "a/S.java",
        "package a;\nimport b.*;\nimport c.*;\npublic class S {\n    public void run(Visit visit, Only only) {\n        visit.describe();\n        only.describe();\n    }\n}\n",
    );
    let facts = graph_facts(&texts(&[
        s,
        ("b/Visit.java", "package b;\npublic class Visit {\n    public void describe() {}\n}\n"),
        ("c/Visit.java", "package c;\npublic class Visit {\n    public void describe() {}\n}\n"),
        ("c/Only.java", "package c;\npublic class Only {\n    public void describe() {}\n}\n"),
    ]));
    // `Visit` is in both packages (Java rejects it); `Only` is in one.
    let calls: Vec<&str> = facts["a/S.java"].calls.iter().map(|c| c.to.as_str()).collect();
    assert_eq!(calls, ["c/Only.java#Only.describe"]);
}
