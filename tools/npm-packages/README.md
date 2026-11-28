# npm-packages

nonio's npm packages (D24), built from dist's release archives:

- `@websublime/nonio-<os>-<cpu>`, one per target, with the `nonio` binary inside and npm's `os`, `cpu` (and `libc`
  on Linux) fields, so npm installs only the one for the machine;
- `@websublime/nonio`, with those as `optionalDependencies` and a `nonio` command (`bin/nonio.js`) that runs the
  installed binary with the same arguments and stdio.

Nothing is downloaded at install or run time and there are no install scripts, so the packages work from any
registry that holds them (public npm, or an internal one), with `ignore-scripts` on, and without reaching GitHub.
The plugin starts nonio with `npx -y @websublime/nonio@<version> mcp`; which registry npm uses is the machine's npm
configuration (`@websublime:registry=<url>` in `.npmrc` for an internal one).

| Script | What it does |
|---|---|
| `build.mjs --archives <dir> --out <dir> [--version v] [--scope @s]` | Builds the packages from the `nonio-cli-<target>` archives in `<dir>` |
| `smoke.sh <dir>` | Publishes them to a throwaway Verdaccio with no uplink and starts nonio from it with `npx`, scripts off (CI's `npm` job) |
| `publish.sh <tag> <registry> [--dry-run]` | Downloads a GitHub release's archives, checks their checksums, builds and publishes the packages (platform packages first) |

The `npm publish` workflow runs `publish.sh` by hand for a tag and a registry, with the `NPM_TOKEN` secret, dry run
by default. An internal registry that GitHub's runners can't reach is published from inside its network with
`publish.sh`, after `npm login --registry <url>`.

The platform packages are about 10 MB packed and 34 MB unpacked; a registry with an upload limit below that refuses
them (Verdaccio's default is 10 MB, which the smoke test raises).
