// Golden prompts for nonio's port: each prompt function on fixed inputs, written as text files.
import * as fs from 'node:fs';
import { cpSync, mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { check, parseMap } from '@illuminate/core';
import { evidenceRepairPrompt, explainerPrompt, makerPrompt, patchRepairPrompt, quickAnswerPrompt, shapeRepairPrompt } from './src/prompts';
import { headStart } from './src/hints';

const out = process.argv[2]!;
const write = (name: string, text: string) => fs.writeFileSync(join(out, `${name}.txt`), text.replaceAll('illuminate', 'nonio'));
const careApp = new URL('../core/test/fixtures/care-app', import.meta.url).pathname;
const raw = JSON.parse(fs.readFileSync(new URL('../core/test/fixtures/care-app.map.json', import.meta.url), 'utf8'));
const parsed = parseMap(raw);
if (!parsed.ok) throw new Error('map');
const map = parsed.map;
const root = mkdtempSync(join(tmpdir(), 'golden-'));
cpSync(careApp, root, { recursive: true });

write('maker', makerPrompt('What happens when a patient converts?', 'Ways in (entry points):\n  - route POST /x → a.ts#b\n'));
write('shape-repair', shapeRepairPrompt(['nodes: Required', 'edges.0.from: Expected string']));
write('patch-repair', patchRepairPrompt(['edge e9 goes to a box that does not exist']));

// A map with claims that don't check out: a wrong symbol, a wrong line text, an absence that is found.
const broken = structuredClone(raw);
broken.nodes[1].symbol = broken.nodes[1].symbol.replace(/#.*/, '#convert');
broken.nodes[2].evidence[0].text = 'this text is nowhere';
broken.edges[0].kind = 'calls';
const bp = parseMap(broken);
if (!bp.ok) throw new Error(bp.problems.join('\n'));
const brokenReport = check(bp.map, root);
const symbolsIn = (file: string) => (file.endsWith('routes.ts') ? ['convertPatient', 'listPatients', 'router.convert'] : []);
write('evidence-repair', evidenceRepairPrompt(brokenReport, symbolsIn));
write('evidence-repair-plain', evidenceRepairPrompt(brokenReport));
fs.writeFileSync(join(out, 'broken.map.json'), JSON.stringify(broken, null, 2) + '\n');

const report = check(map, root);
const selection = [{ id: map.nodes[0]!.id, label: map.nodes[0]!.name }];
const earlier = [{ question: 'Why?', concept: 'Because nothing listens.' }];
write('quick-answer', quickAnswerPrompt(map, report, selection, 'Where is the email sent?', '1 | a\n2 | b', earlier, [{ id: 'other', question: 'How do invoices work?' }]));
write('quick-answer-bare', quickAnswerPrompt(map, report, [], 'Where?', '', []));
write('explainer', explainerPrompt(map, report, selection, 'Where is the email sent?', 'The job code.', false, earlier));
write('explainer-words', explainerPrompt(map, report, [], 'How do invoices work?', '', true, []));
const jp = parseMap({ ...raw, view: 'journey' }); if (!jp.ok) throw new Error('j'); const journey = jp.map;
write('explainer-journey', explainerPrompt(journey, check(journey, root), [], 'What does the person see?'));
write('explainer-broken', explainerPrompt(bp.map, brokenReport, [], 'Is it right?'));
write('head-start', (await headStart(root)).text);
