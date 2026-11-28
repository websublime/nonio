#!/usr/bin/env bash
# Publishes the packages in <dir> (from build.mjs) to a throwaway registry that stands for an internal one (Verdaccio,
# no uplink: it holds only what is published to it), then starts nonio the way the plugin does, from that registry
# only, with install scripts turned off:
#   npx -y @websublime/nonio@<version> --version, and `mcp` answering an MCP initialize.
#   tools/npm-packages/smoke.sh target/npm
set -euo pipefail
dir=$(cd "$1" && pwd)
scope=${SCOPE:-@websublime}
port=${PORT:-4873}
work=$(mktemp -d)
trap 'kill "$registry_pid" 2>/dev/null || true; rm -rf "$work"' EXIT

cat > "$work/config.yaml" <<YAML
storage: $work/storage
auth:
  htpasswd:
    file: $work/htpasswd
uplinks: {}
max_body_size: 100mb
packages:
  '$scope/*':
    access: \$all
    publish: \$authenticated
  '**':
    access: \$all
log: { type: stdout, format: pretty, level: warn }
YAML
npm install --prefix "$work/registry" --no-save --loglevel=error verdaccio@6.1.6 > /dev/null
node "$work/registry/node_modules/verdaccio/bin/verdaccio" --config "$work/config.yaml" --listen "127.0.0.1:$port" \
  > "$work/registry.log" 2>&1 &
registry_pid=$!
url="http://127.0.0.1:$port/"
for _ in $(seq 60); do curl -fs "${url}-/ping" > /dev/null && break; sleep 1; done
curl -fs "${url}-/ping" > /dev/null || { cat "$work/registry.log"; exit 1; }

# A clean npm: its own cache and config, this registry for the scope and nothing else, no install scripts.
export npm_config_cache="$work/cache" npm_config_userconfig="$work/npmrc" npm_config_ignore_scripts=true
export NPM_CONFIG_USERCONFIG="$work/npmrc" # an upper-case one already in the environment would win
token=$(curl -fs -X PUT -H 'content-type: application/json' -d '{"name":"smoke","password":"smoke-only"}' \
  "${url}-/user/org.couchdb.user:smoke" | node -p 'JSON.parse(require("fs").readFileSync(0)).token')
printf '%s:registry=%s\n//127.0.0.1:%s/:_authToken=%s\nregistry=%s\n' "$scope" "$url" "$port" "$token" "$url" > "$work/npmrc"
for p in "$dir"/nonio-*/ "$dir"/nonio/; do (cd "$p" && npm publish --registry "$url" --loglevel=error > /dev/null); done
version=$(node -p "require('$dir/nonio/package.json').version")

cd "$work"
got=$(npx -y "$scope/nonio@$version" --version)
[ "$got" = "nonio $version" ] || { echo "npx printed: $got"; exit 1; }
init='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"1"}}}'
reply=$(printf '%s\n' "$init" | npx -y "$scope/nonio@$version" mcp --repo "$work" --port 0 | head -n 1)
echo "$reply" | grep -q '"name":"nonio"' || { echo "mcp replied: $reply"; exit 1; }
echo "ok: $scope/nonio@$version from $url (no install scripts): $got, and mcp answers initialize"
