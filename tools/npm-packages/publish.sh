#!/usr/bin/env bash
# Publishes nonio's npm packages for a GitHub release to one registry (D24): downloads that release's archives, builds
# the packages with build.mjs and publishes the platform packages first, then @websublime/nonio. Run it where the
# registry is reachable (an internal one is often only reachable from inside the company network), signed in to it
# (npm login --registry <url>, or a token in .npmrc).
#   tools/npm-packages/publish.sh v0.0.1 https://npm.example.internal/ [--dry-run]
set -euo pipefail
tag=${1:?usage: publish.sh <tag> <registry url> [--dry-run]}
registry=${2:?usage: publish.sh <tag> <registry url> [--dry-run]}
dry=${3:-}
here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

gh release download "$tag" --repo websublime/nonio --dir "$work/archives" \
  --pattern 'nonio-cli-*.tar.xz' --pattern 'nonio-cli-*.zip' --pattern 'sha256.sum'
(cd "$work/archives" && grep -E 'nonio-cli-.*\.(tar\.xz|zip)$' sha256.sum | shasum -a 256 -c -)
node "$here/build.mjs" --archives "$work/archives" --out "$work/npm" --version "${tag#v}"
for p in "$work"/npm/nonio-*/ "$work"/npm/nonio/; do
  (cd "$p" && npm publish --registry "$registry" --access public $dry)
done
