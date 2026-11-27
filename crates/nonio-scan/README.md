# nonio-scan

The TS/JS extraction backend: `OxcExtractor` implements `nonio_facts::Extractor` with one oxc 0.153 parse and one
walk per file. It is a port of illuminate's `packages/facts` (extract, record, resolver, entries, adapters, tables,
routes, regex fallback) and gives the same facts for the same files, with the differences listed below.

```rust
let ex = nonio_scan::OxcExtractor::new(root);
let facts = ex.extract("src/app.ts", &source); // Some(FileFacts); None for files it does not handle
```

`.py` and `.go` files return `None` (they belong to the `code2graph` adapter). Vue single-file components are not
handled yet (open question Q11).

## How it is checked

- The `nonio-core` tests run the checker on facts from this crate and assert, file by file, that they equal the facts
  recorded from illuminate's own extractor (`crates/nonio-core/tests/common/mod.rs`, `Oxc`).
- `tests/` ports illuminate's facts tests (`extract`, `imports`, `resolve`, `tables`).
- [`tools/compare-facts`](../../tools/compare-facts/README.md) diffs both extractors over any set of files.

## Where it differs from illuminate

- **Parameters and local variables never link by name (D16).** illuminate links a call whose root is a parameter or a
  local variable to an import or a top-level symbol of the same name (`function f(url) { url.parse() }` next to
  `import url from 'node:url'` gives a call to `vendor:node:url`). nonio resolves the root with oxc_semantic: a
  binding below the module scope links only to the nested function or captured `import()`/`require()` it actually
  is, and otherwise gives no call. Over about 8,000 files of `node_modules` this drops 491 of 58,952 call edges
  (0.8%); every one sampled was a false name match.
- **Large and minified files go through oxc too.** For files over 128,000 characters, and minified ones, illuminate
  reads imports with es-module-lexer, which takes `require()` calls inside comments as imports, marks some type-only
  imports differently, and reads `export { a } from` re-exports as local names. nonio builds every record from oxc's
  module record (the walk itself still skips files over 500,000 UTF-16 units and minified files, as in the
  original), so imports are the ones the code has and calls follow re-exports to where the symbol is defined.
- **Uninstalled workspace packages resolve without writing.** illuminate links workspace packages into a folder on
  disk so its resolver can find them. nonio gives `oxc_resolver` a file system that shows those links virtually under
  `<root>/.nonio/ws-modules`; nothing is written to the repo (D25).
