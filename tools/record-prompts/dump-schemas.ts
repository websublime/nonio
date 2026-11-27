import { ANSWER_SCHEMA, MAP_PATCH_SCHEMA, MAP_SCHEMA, QUICK_ANSWER_SCHEMA } from './src/schemas';
import * as fs from 'node:fs';
const out = process.argv[2]!;
for (const [n, s] of Object.entries({ map: MAP_SCHEMA, 'map-patch': MAP_PATCH_SCHEMA, answer: ANSWER_SCHEMA, 'quick-answer': QUICK_ANSWER_SCHEMA })) fs.writeFileSync(`${out}/${n}.schema.json`, JSON.stringify(s, null, 2) + '\n');
