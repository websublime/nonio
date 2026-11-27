# compare-facts

Runs illuminate's extractor and nonio's over the same files and lists the files whose facts differ. It is how the
oxc backend (`crates/nonio-scan`) is checked beyond the ported tests.

```sh
# illuminate's facts (from an illuminate checkout at 7a88df6, after npm ci)
npx tsx /path/to/nonio/tools/compare-facts/dump.ts <root> <file>... > illuminate.json
# nonio's facts
cargo run --release -p nonio-scan --example dump_facts -- <root> <file>... > nonio.json
python3 tools/compare-facts/compare.py illuminate.json nonio.json      # -v shows each difference
```

Both print `{path: facts}` with calls unsettled and symbol spans as UTF-8 byte offsets. `dump.ts` skips files
illuminate throws on (it does when a tsconfig `extends` can't be found).

Last run (2026-11-27): illuminate's own repo, 149 of 149 files identical. About 8,000 files of its `node_modules`:
the differences are the ones listed in [crates/nonio-scan/README.md](../../crates/nonio-scan/README.md#where-it-differs-from-illuminate).
