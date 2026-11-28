# nonio://

Interactive maps of how code works, where every box and every arrow is checked against the code by a deterministic
program before it is shown. Written in Rust, inspired by [illuminate](https://github.com/johnz4021/illuminate).

Status: phase 3. The map format, the checker, the TS/JS extraction backend (oxc), the page server and the MCP channel
and the web viewer are ported, and the `nonio` binary runs it all as an MCP server (`nonio mcp`) with the viewer
built in.

## Workspace

| Crate | What it holds |
|---|---|
| `nonio-facts` | The facts contract (`FileFacts`, `Extractor`), and the pure functions over facts the checker needs: settling calls, `fetch` sites, route matching, globs |
| `nonio-core` | The map format (`parse_map`), the checker (`check`), map edits, the working-tree and git readers, screens and code views |
| `nonio-scan` | The TS/JS extraction backend on oxc (`OxcExtractor`); see [its README](crates/nonio-scan/README.md) for where it differs from illuminate |
| `nonio-server` | Host profiles (D07), the bridge that hands work to the host's model in tool results (D06), the request text and its parts, the map store in `.nonio/maps`, the output schemas and prompts (the same text as illuminate's, see [`tools/record-prompts`](tools/record-prompts/README.md)), the head start, the maker, the explainer (answers on a map), fixes (proposed, applied on approval, undone), the chat reply, and the page server on 127.0.0.1 (HTTP API and WebSocket, token, Host and Origin checks, stops itself when nobody is looking), and screens: what a repo can show, and captures from the installed Chrome over the DevTools protocol (`NONIO_CHROME` names one), and the MCP channel (rmcp) with the `nonio_*` tools, shaped per host: Claude Code listens on the map, Codex and Cursor ask from the chat |
| `nonio-cli` | The `nonio` binary: `nonio mcp` runs the MCP channel over stdio, with `viewer/dist` built in (rust-embed); a build without the viewer still runs and its pages say how to build it |

The checker knows facts only through the `Extractor` trait. `nonio-scan` implements it for TS/JS and Vue on oxc, and
for Java, Kotlin and Python through the `code2graph` adapter and the field resolver (D14, D15).

## Viewer

`viewer/` is the web page that shows a map (React, React Flow and ELK, from illuminate). The page server serves its
built `dist/`.

```sh
cd viewer
npm ci
npm test         # builds the viewer-maps example: the tests read their maps through nonio's own parser and checker
npm run build
cargo run -p nonio-server --example preview   # the care-app map in the real server, no model; open the printed link
```

## Running it

```sh
(cd viewer && npm ci && npm run build)   # first, so the binary carries the viewer
cargo build --release -p nonio-cli
claude mcp add --scope user nonio -- /path/to/target/release/nonio mcp
```

`--repo <dir>` picks the repo (default: the host's project folder, else the current one); `--port <n>` the page
server's port (default 4317). `NONIO_HOST` forces a host profile (`claude-code`, `codex`, `cursor`).

## Plugin

`plugin/` is nonio as an [Agent Plugin](https://agent-plugins.org/) (spec 1.0.0, D04): `plugin.json`, `mcp.json` and
the `nonio` skill, plus `.cursor-plugin/plugin.json` for Cursor. `mcp.json` starts the server with
`npx -y @websublime/nonio@<version> mcp`, so the plugin needs Node and npm, and npm decides where the package comes
from: the public npm registry, or an internal one when the `@websublime` scope points there. The repo root's
`.cursor-plugin/marketplace.json` lists the plugin, so this repo (or a mirror of it) can be imported as a Cursor team
marketplace. To try it locally in Cursor, copy `plugin/` to `~/.cursor/plugins/local/nonio` and reload the window.

Releases are built by `dist` from a `v*` tag (`dist-workspace.toml`, `.github/workflows/release.yml`): a GitHub
Release with archives for macOS, Linux and Windows, shell and PowerShell installers, and a Homebrew formula. The npm
packages are built from those archives by [`tools/npm-packages`](tools/npm-packages/README.md): one per platform with
the binary inside, so `npx` needs no download from GitHub and no install script, from public npm or an internal
registry.

The versions in `plugin/` follow the workspace version; `cargo test -p nonio-cli --test plugin` fails when they drift.

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
