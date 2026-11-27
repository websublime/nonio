# spike-vue

Q11 spike: does `vue-oxlint-toolkit` (crates `vue_oxlint_jsx` and `vue_oxlint_parser` 0.15.0) give the checker what
it needs from a Vue SFC? Results and conclusions: [`docs/spikes/q11-vue.md`](../../docs/spikes/q11-vue.md).

This crate is standalone (its own `[workspace]` and `Cargo.lock`): `vue_oxlint_jsx` only builds against oxc 0.129 to
0.135, while nonio is on oxc 0.153, so it stays out of nonio's workspace, lockfile, CI and MSRV.

```sh
# vue_oxlint_jsx + oxc_semantic 0.129 over the fixture, scored against fixture/expected.txt
cargo run --manifest-path tools/spike-vue/Cargo.toml --bin spike-vue -- tools/spike-vue/fixture

# nonio-scan (oxc 0.153) on the same files: `raw` (as today) or `blank` (only <script> contents, offsets kept)
cargo run --manifest-path tools/spike-vue/Cargo.toml --bin baseline -- tools/spike-vue/fixture blank \
  src/components/OrderList.vue src/components/OrderRow.vue src/components/LegacyCounter.vue
```

`results-*.txt` hold the output of the run recorded in the spike notes.
