#!/usr/bin/env node
// Builds nonio's npm packages from dist's release archives (D24): one package per platform with the binary inside,
// and `@websublime/nonio`, which has them as optionalDependencies and a `nonio` command that runs the one npm
// installed. Nothing is downloaded when the package is installed or run, so it works from any registry that holds the
// packages (public npm, or an internal one), with `ignore-scripts`, and without reaching GitHub.
//
//   node tools/npm-packages/build.mjs --archives target/distrib --out target/npm [--version 0.0.1] [--scope @websublime]
//
// Every archive found for a known target becomes a package; a target with no archive is left out of
// optionalDependencies (useful for a local build of one platform). `npm pack` or `npm publish` each folder in --out.
import { execFileSync } from 'node:child_process';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');

/** dist's target triples, and the platform and CPU npm knows them by. */
export const TARGETS = {
  'aarch64-apple-darwin': { os: 'darwin', cpu: 'arm64' },
  'x86_64-apple-darwin': { os: 'darwin', cpu: 'x64' },
  'aarch64-unknown-linux-gnu': { os: 'linux', cpu: 'arm64', libc: 'glibc' },
  'x86_64-unknown-linux-gnu': { os: 'linux', cpu: 'x64', libc: 'glibc' },
  'x86_64-pc-windows-msvc': { os: 'win32', cpu: 'x64' },
};

function args(argv) {
  const a = { scope: '@websublime' };
  for (let i = 0; i < argv.length; i += 2) {
    const k = argv[i];
    const v = argv[i + 1];
    if (!k.startsWith('--') || v === undefined) throw new Error(`Expected --name value, got ${k}`);
    a[k.slice(2)] = v;
  }
  if (!a.archives || !a.out) throw new Error('Usage: build.mjs --archives <dir> --out <dir> [--version <v>] [--scope <@scope>]');
  if (!a.scope.startsWith('@')) throw new Error(`A scope starts with @: ${a.scope}`);
  a.version ??= workspaceVersion();
  return a;
}

function workspaceVersion() {
  const toml = fs.readFileSync(path.join(root, 'Cargo.toml'), 'utf8');
  const m = toml.match(/\[workspace\.package\][^[]*?\nversion\s*=\s*"([^"]+)"/);
  if (!m) throw new Error('No version in [workspace.package]');
  return m[1];
}

const common = (version) => ({
  version,
  license: 'MIT',
  homepage: 'https://github.com/websublime/nonio#readme',
  repository: { type: 'git', url: 'git+https://github.com/websublime/nonio.git' },
});

const write = (file, text, mode) => {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, text);
  if (mode) fs.chmodSync(file, mode);
};
const json = (v) => `${JSON.stringify(v, null, 2)}\n`;

function extract(archive, into) {
  fs.mkdirSync(into, { recursive: true });
  if (archive.endsWith('.zip')) execFileSync('unzip', ['-q', '-o', archive, '-d', into]);
  else execFileSync('tar', ['-xf', archive, '-C', into]);
  // dist's .tar.xz archives hold one folder named after the archive; its zips hold the files at the top.
  const entries = fs.readdirSync(into);
  return entries.length === 1 && fs.statSync(path.join(into, entries[0])).isDirectory() ? path.join(into, entries[0]) : into;
}

function platformPackage({ scope, version, out }, target, archive) {
  const { os, cpu, libc } = TARGETS[target];
  const name = `${scope}/nonio-${os}-${cpu}`;
  const dir = path.join(out, `nonio-${os}-${cpu}`);
  fs.rmSync(dir, { recursive: true, force: true });
  const tmp = fs.mkdtempSync(path.join(out, '.extract-'));
  try {
    const from = extract(archive, tmp);
    const exe = os === 'win32' ? 'nonio.exe' : 'nonio';
    if (!fs.existsSync(path.join(from, exe))) throw new Error(`${archive} holds no ${exe}`);
    fs.mkdirSync(path.join(dir, 'bin'), { recursive: true });
    fs.copyFileSync(path.join(from, exe), path.join(dir, 'bin', exe));
    fs.chmodSync(path.join(dir, 'bin', exe), 0o755);
    for (const f of ['LICENSE', 'NOTICE']) fs.copyFileSync(path.join(from, f), path.join(dir, f));
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
  write(path.join(dir, 'README.md'), `# ${name}\n\nThe \`nonio\` binary for ${os} ${cpu}. Install \`${scope}/nonio\` instead: it picks this package for you.\n`);
  write(
    path.join(dir, 'package.json'),
    json({
      name,
      ...common(version),
      description: `The nonio binary for ${os} ${cpu}; installed by ${scope}/nonio.`,
      os: [os],
      cpu: [cpu],
      ...(libc ? { libc: [libc] } : {}),
      files: ['bin', 'LICENSE', 'NOTICE'],
      preferUnplugged: true,
    }),
  );
  return name;
}

function mainPackage({ scope, version, out }, platforms) {
  const dir = path.join(out, 'nonio');
  fs.rmSync(dir, { recursive: true, force: true });
  write(path.join(dir, 'bin', 'nonio.js'), launcher(scope), 0o755);
  for (const f of ['LICENSE', 'NOTICE']) fs.copyFileSync(path.join(root, f), path.join(dir, f));
  write(path.join(dir, 'README.md'), readme(scope));
  write(
    path.join(dir, 'package.json'),
    json({
      name: `${scope}/nonio`,
      ...common(version),
      description: 'nonio://: interactive maps of how your code works, checked against the code. `nonio mcp` runs it as an MCP server.',
      keywords: ['mcp', 'code-map', 'architecture', 'diagram'],
      bin: { nonio: 'bin/nonio.js' },
      files: ['bin', 'LICENSE', 'NOTICE'],
      engines: { node: '>=18' },
      optionalDependencies: Object.fromEntries(platforms.sort().map((p) => [p, version])),
    }),
  );
}

/** The `nonio` command: find the binary npm installed for this machine and run it with the same arguments and stdio
 * (stdio carries MCP). It never downloads anything (D25). */
const launcher = (scope) => `#!/usr/bin/env node
'use strict';
const { spawn } = require('node:child_process');
const path = require('node:path');

const pkg = \`${scope}/nonio-\${process.platform}-\${process.arch}\`;
const exe = process.platform === 'win32' ? 'nonio.exe' : 'nonio';
let bin;
try {
  bin = path.join(path.dirname(require.resolve(\`\${pkg}/package.json\`)), 'bin', exe);
} catch {
  console.error(
    \`nonio: \${pkg} isn't installed, so there is no nonio binary for this machine (\${process.platform} \${process.arch}).\\n\` +
      'npm installs it with ${scope}/nonio unless optional dependencies are turned off (--omit=optional, --no-optional)\\n' +
      "or the registry doesn't hold it. Install again with optional dependencies, or ask whoever runs your registry.",
  );
  process.exit(1);
}
const child = spawn(bin, process.argv.slice(2), { stdio: 'inherit', windowsHide: true });
for (const s of ['SIGINT', 'SIGTERM', 'SIGHUP']) process.on(s, () => child.kill(s));
child.on('error', (e) => {
  console.error(\`nonio: couldn't start \${bin}: \${e.message}\`);
  process.exit(1);
});
child.on('exit', (code, signal) => {
  if (signal) process.kill(process.pid, signal);
  else process.exit(code ?? 1);
});
`;

const readme = (scope) => `# ${scope}/nonio

nonio:// draws interactive maps of how your code works, and checks every box and arrow against the code before it is
shown. Your coding agent starts it as an MCP server:

\`\`\`sh
npx -y ${scope}/nonio mcp
\`\`\`

The binary for your platform comes in one of the \`${scope}/nonio-<os>-<cpu>\` packages, which npm installs as an
optional dependency. Nothing is downloaded at install or run time. See https://github.com/websublime/nonio.
`;

function main() {
  const a = args(process.argv.slice(2));
  a.out = path.resolve(a.out);
  fs.mkdirSync(a.out, { recursive: true });
  const files = fs.readdirSync(a.archives);
  const platforms = [];
  for (const target of Object.keys(TARGETS)) {
    const archive = files.find((f) => f === `nonio-cli-${target}.tar.xz` || f === `nonio-cli-${target}.zip`);
    if (!archive) continue;
    platforms.push(platformPackage(a, target, path.join(a.archives, archive)));
  }
  if (platforms.length === 0) throw new Error(`No nonio-cli-<target> archive in ${a.archives}`);
  mainPackage(a, platforms);
  console.log(`${a.scope}/nonio ${a.version} with ${platforms.join(', ')} in ${a.out}`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
