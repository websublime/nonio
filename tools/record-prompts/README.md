# record-prompts

`crates/nonio-server/schemas/*.schema.json` are illuminate's own output schemas (`MAP_SCHEMA`, `MAP_PATCH_SCHEMA`,
`ANSWER_SCHEMA`, `QUICK_ANSWER_SCHEMA`), and `crates/nonio-server/tests/fixtures/prompts/*.txt` are illuminate's own
prompts on fixed inputs, with "illuminate" read as "nonio". `crates/nonio-server/tests/prompts.rs` and
`tests/hints.rs` assert that nonio's prompts and head start are the same text.

## Regenerate

```sh
git clone https://github.com/johnz4021/illuminate && cd illuminate
git checkout 7a88df6491ba8b50055e8dc657e126dbd0a53125
npm ci --ignore-scripts
cp /path/to/nonio/tools/record-prompts/dump-*.ts packages/server/
cd packages/server
npx tsx dump-schemas.ts /path/to/nonio/crates/nonio-server/schemas
npx tsx dump-prompts.ts /path/to/nonio/crates/nonio-server/tests/fixtures/prompts
```

`broken.map.json` (written by `dump-prompts.ts`) is the care-app map with claims that don't check out, so the repair
prompt has something to send back.
