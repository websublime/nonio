# record-facts

`crates/nonio-core/tests/fixtures/recorded-facts.json` holds the facts illuminate's own extractor produced for every
file its core tests read, keyed by path and the SHA-256 of the file's text. The ported checker tests read facts from
there (`Recorded` in `crates/nonio-core/tests/common/mod.rs`), so they test the checker alone, against exactly the
facts the original checker saw. A file the recording doesn't have fails the test.

The tests now take their facts from nonio's own TS/JS backend (`crates/nonio-scan`) and assert that each file's facts
equal the recorded ones.

## Regenerate

```sh
git clone https://github.com/johnz4021/illuminate && cd illuminate
git checkout 7a88df6491ba8b50055e8dc657e126dbd0a53125
git apply /path/to/nonio/tools/record-facts/record.patch
npm ci --ignore-scripts
cd packages/core && NONIO_RECORD_FACTS=/tmp/facts npx vitest run
python3 /path/to/nonio/tools/record-facts/merge.py /tmp/facts > /path/to/nonio/crates/nonio-core/tests/fixtures/recorded-facts.json
```

The patch makes `createFactsIndex` write each extraction to a file, with symbol spans converted from UTF-16 to
UTF-8 byte offsets (nonio's spans are byte offsets).
