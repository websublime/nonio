# nonio://

Interactive maps of how code works, where every box and every arrow is checked against the code by a deterministic
program before it is shown. Written in Rust, inspired by [illuminate](https://github.com/johnz4021/illuminate).

Status: phase 2. The map format, the checker and the TS/JS extraction backend (oxc) are ported; there is no server or
viewer yet.

## Workspace

| Crate | What it holds |
|---|---|
| `nonio-facts` | The facts contract (`FileFacts`, `Extractor`), and the pure functions over facts the checker needs: settling calls, `fetch` sites, route matching, globs |
| `nonio-core` | The map format (`parse_map`), the checker (`check`), map edits, the working-tree and git readers, screens and code views |
| `nonio-scan` | The TS/JS extraction backend on oxc (`OxcExtractor`); see [its README](crates/nonio-scan/README.md) for where it differs from illuminate |
| `nonio-server` | Host profiles (D07), the bridge that hands work to the host's model in tool results (D06), the request text and its parts, and the map store in `.nonio/maps` |

The checker knows facts only through the `Extractor` trait. `nonio-scan` implements it for TS/JS and Vue on oxc, and
for Java, Kotlin and Python through the `code2graph` adapter and the field resolver (D14, D15).

## Tests

```sh
cargo test
```

The tests in `crates/nonio-core/tests` are ported from illuminate's core tests. They run the checker on facts from
`nonio-scan` and assert that those equal the facts recorded from illuminate's own extractor (see
[`tools/record-facts`](tools/record-facts/README.md)). [`tools/compare-facts`](tools/compare-facts/README.md) compares
both extractors over any repo.

## License

MIT. Parts are carried over from illuminate (MIT, Copyright (c) 2026 johnz4021); see [NOTICE](NOTICE).
