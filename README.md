# nonio://

Interactive maps of how code works, where every box and every arrow is checked against the code by a deterministic
program before it is shown. Written in Rust, inspired by [illuminate](https://github.com/johnz4021/illuminate).

Status: phase 1. The map format and the checker are ported; there is no scanner backend, server or viewer yet.

## Workspace

| Crate | What it holds |
|---|---|
| `nonio-facts` | The facts contract (`FileFacts`, `Extractor`), and the pure functions over facts the checker needs: settling calls, `fetch` sites, route matching, globs |
| `nonio-core` | The map format (`parse_map`), the checker (`check`), map edits, the working-tree and git readers, screens and code views |

The checker knows facts only through the `Extractor` trait. Extraction backends (oxc for TS/JS/Vue, `code2graph` for
Java/Kotlin/Python) come in phase 2.

## Tests

```sh
cargo test
```

The tests in `crates/nonio-core/tests` are ported from illuminate's core tests and run against facts recorded from
illuminate's own extractor (see [`tools/record-facts`](tools/record-facts/README.md)).

## License

MIT. Parts are carried over from illuminate (MIT, Copyright (c) 2026 johnz4021); see [NOTICE](NOTICE).
