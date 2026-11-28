# Releasing nonio

Releases are automatic once a change is described; nothing is tagged or published by hand.

1. **Describe the change.** In the PR that makes it, add a change file: run `knope document-change` (or write
   `.changeset/<name>.md` by hand) and pick `major`, `minor` or `patch`. While nonio is at 0.x, `major` moves to the
   next 0.x and `minor` and `patch` to the next patch.
2. **Release PR.** On every push to main with change files, `prepare-release.yml` runs `knope prepare-release`: on
   the `release` branch it bumps the version wherever it is written (`Cargo.toml`, `Cargo.lock`, `plugin/plugin.json`,
   `plugin/.cursor-plugin/plugin.json`, `plugin/mcp.json`, the skill), moves the change files into `CHANGELOG.md`, and
   opens or updates the PR "Release <version>". `cargo test -p nonio-cli --test plugin` checks that every place agrees.
3. **Merge it.** `release-tag.yml` tags `v<version>` on main.
4. **The tag releases.** `release.yml` (dist) builds every target, creates the GitHub Release with the archives and
   the shell and PowerShell installers, publishes the Homebrew formula to `websublime/homebrew-tap`, and runs
   `publish-npm.yml`, which publishes the npm packages (D24) to public npm.
5. **Internal registry.** From inside its network: `tools/npm-packages/publish.sh v<version> <registry url>`, or the
   hand-run "npm publish" workflow when GitHub's runners can reach it.

## Secrets (repository settings)

| Secret | Used by | Needs |
|---|---|---|
| `RELEASE_TOKEN` | `prepare-release.yml`, `release-tag.yml` | A fine-grained token on this repo with Contents and Pull requests read and write. The default token can't do it: its PRs get no CI and its tags start no workflow |
| `HOMEBREW_TAP_TOKEN` | `release.yml` | Contents write on `websublime/homebrew-tap` |
| `NPM_TOKEN` | `publish-npm.yml`, "npm publish" | An npm automation token that can publish under `@websublime` |

Without `RELEASE_TOKEN` or `NPM_TOKEN` those steps warn and skip; without `HOMEBREW_TAP_TOKEN` the Homebrew job fails
after the GitHub Release is out.

`dist` is pinned to 0.33.1: 0.34.0 writes `uses: $/.github/workflows/…` for custom jobs, which GitHub can't run.
