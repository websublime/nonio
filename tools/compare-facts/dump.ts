// Print illuminate's facts for the given files of a repo as JSON ({path: facts}), calls unsettled, symbol spans as
// UTF-8 byte offsets (nonio's spans). Run from an illuminate checkout:
//   npx tsx /path/to/nonio/tools/compare-facts/dump.ts <root> <file>...
import * as fs from 'node:fs';
import * as path from 'node:path';
import { createFactsIndex, fileExistsIn } from './packages/facts/src/index';

const [root, ...files] = process.argv.slice(2);
const read = (p: string) => {
  try {
    return fs.readFileSync(path.join(root!, p), 'utf8');
  } catch {
    return null;
  }
};
const index = createFactsIndex(root!, fileExistsIn(root!), read);
const out: Record<string, unknown> = {};
for (const file of files) {
  const source = read(file);
  if (source === null) continue;
  let facts;
  try {
    facts = index.extract(file, source).facts;
  } catch (e) {
    // The original throws on some repos (get-tsconfig on an `extends` it can't find); those files are left out.
    process.stderr.write(`${file}: ${(e as Error).message}\n`);
    continue;
  }
  const bytes = (off: number) => Buffer.byteLength(source.slice(0, off), 'utf8');
  out[file] = { ...facts, symbols: facts.symbols.map((s) => ({ ...s, span: { start: bytes(s.span.start), end: bytes(s.span.end) } })) };
}
process.stdout.write(JSON.stringify(out));
