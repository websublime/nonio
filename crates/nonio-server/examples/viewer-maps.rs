//! The viewer's tests ask nonio for maps the way the page gets them: `parse` reads a map as JSON on stdin and prints
//! it as `parse_map` reads it (defaults filled in), or `{"problems": [...]}`; `check <root>` prints the checker's
//! report for that map. The tests then run on exactly the JSON the Rust server sends.

use std::io::Read;
use std::path::Path;

use nonio_core::parse_map;
use serde_json::{json, Value};

fn main() {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_default();
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).expect("a map on stdin");
    let value: Value = serde_json::from_str(&input).expect("JSON on stdin");
    let out = match parse_map(&value) {
        Err(problems) => json!({ "problems": problems }),
        Ok(map) => match cmd.as_str() {
            "parse" => json!({ "map": map }),
            "check" => {
                let root = args.next().expect("check <root>");
                let report = nonio_server::maker::check_map(&map, Path::new(&root)).expect("checked");
                json!({ "map": map, "report": report })
            }
            other => panic!("unknown command {other:?}: parse or check <root>"),
        },
    };
    println!("{out}");
}
