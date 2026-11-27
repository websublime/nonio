// The maps the viewer's tests use, read and checked by nonio itself (the Rust `parse_map` and checker, through the
// `viewer-maps` example that `test/build-nonio.ts` builds), so the tests run on the JSON the server sends the page.
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import type { CheckReport, CodeMap } from '../src/types';

const TOOL = fileURLToPath(new URL(`../../target/debug/examples/viewer-maps${process.platform === 'win32' ? '.exe' : ''}`, import.meta.url));

/** The care-app fixtures nonio-core's own tests use. */
export const fixture = (name: string) => fileURLToPath(new URL(`../../crates/nonio-core/tests/fixtures/${name}`, import.meta.url));

const run = (args: string[], input: unknown) => JSON.parse(execFileSync(TOOL, args, { input: JSON.stringify(input), encoding: 'utf8' }));

export function parseMap(input: unknown): { ok: true; map: CodeMap } | { ok: false; problems: string[] } {
  const r = run(['parse'], input);
  return r.problems ? { ok: false, problems: r.problems } : { ok: true, map: r.map };
}

export function check(map: CodeMap, root: string): CheckReport {
  return run(['check', root], map).report;
}
