// ELK layout of the care-app map: steps stack down inside their area, areas run left to right along the arrows, every
// box stays inside its area, and arrows attach to the sides that face each other.
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { fixture, parseMap } from './nonio';
import { branchNames, hasBranches, layoutKey, layoutMap, rowOf, sides, type Box } from '../src/layout';

const map = (() => {
  const r = parseMap(JSON.parse(readFileSync(fixture('care-app.map.json'), 'utf8')));
  if (!r.ok) throw new Error(r.problems.join('\n'));
  return r.map;
})();

describe('layoutMap', () => {
  it('places every box inside its area, and every area', async () => {
    const l = await layoutMap(map);
    expect(Object.keys(l.areas).sort()).toEqual(['automations', 'events', 'jobs', 'patients']);
    for (const n of map.nodes) {
      const box = l.nodes[n.id]!;
      const area = l.areas[n.area]!;
      expect(box.x).toBeGreaterThanOrEqual(0);
      expect(box.y).toBeGreaterThanOrEqual(0);
      expect(box.x + box.w).toBeLessThanOrEqual(area.w);
      expect(box.y + box.h).toBeLessThanOrEqual(area.h);
    }
  });

  it('stacks steps down inside an area, and runs areas left to right along the path', async () => {
    const l = await layoutMap(map);
    expect(l.nodes.s1!.y).toBeLessThan(l.nodes.s2!.y);
    expect(l.nodes.s2!.y).toBeLessThan(l.nodes.s3!.y);
    expect(l.nodes.u1!.y).toBeLessThan(l.nodes.u3!.y);
    expect(l.areas.patients!.x).toBeLessThan(l.areas.events!.x);
    expect(l.areas.events!.x).toBeLessThan(l.areas.automations!.x); // the missing connection still pulls Automations right
    expect(l.areas.jobs!.x).toBeLessThan(l.areas.automations!.x);
  });

  it('attaches arrows to facing sides: down within an area, across between areas', async () => {
    const l = await layoutMap(map);
    expect(sides(map, l, 's1', 's2')).toEqual(['b', 't']);
    expect(sides(map, l, 's3', 's4')).toEqual(['r', 'l']);
    expect(sides(map, l, 'j1', 'u1')).toEqual(['r', 'l']);
  });

  it('keeps the original boxes in place when answers add to the map', async () => {
    const grown = {
      ...map,
      areas: [...map.areas, { id: 'mail', label: 'Email' }],
      nodes: [...map.nodes, { ...map.nodes.find((n) => n.area === 'jobs')!, id: 'n9' }, { ...map.nodes[0]!, id: 'n10', area: 'mail' }],
      edges: [...map.edges, { ...map.edges[0]!, id: 'e9', from: 'j1', to: 'n9' }, { ...map.edges[0]!, id: 'e10', from: 'n9', to: 'n10' }],
    };
    const before = await layoutMap(map);
    const after = await layoutMap(grown, [], new Set(['mail', 'n9', 'n10', 'e9', 'e10']));
    for (const n of map.nodes) expect(after.nodes[n.id]).toEqual(before.nodes[n.id]); // same place inside its area
    for (const a of map.areas) {
      expect(after.areas[a.id]!.x).toBe(before.areas[a.id]!.x);
      expect(after.areas[a.id]!.y).toBeGreaterThanOrEqual(before.areas[a.id]!.y); // only ever slides down
    }
    const jobs = after.areas.jobs!;
    expect(after.nodes.n9!.y + after.nodes.n9!.h).toBeLessThanOrEqual(jobs.h); // inside the area, which grew
    expect(after.areas.mail!.x).toBeGreaterThan(Math.max(...map.areas.map((a) => before.areas[a.id]!.x + before.areas[a.id]!.w)));
    const boxes = Object.values(after.areas);
    for (const [i, a] of boxes.entries()) {
      for (const b of boxes.slice(i + 1)) expect(a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h).toBe(false);
    }
  });

  it('changes its key only when the structure changes', () => {
    const renamed = { ...map, nodes: map.nodes.map((n) => (n.id === 's1' ? { ...n, name: 'Different words' } : n)) };
    expect(layoutKey(renamed)).toBe(layoutKey(map));
    expect(layoutKey(map, [{ id: 'fix-1', from: 's4', to: 'u1' }])).not.toBe(layoutKey(map));
  });
});

const branched = (() => {
  const r = parseMap(JSON.parse(readFileSync(fixture('care-app.journey.json'), 'utf8')));
  if (!r.ok) throw new Error(r.problems.join('\n'));
  return r.map;
})();

describe('layoutMap on a journey', () => {
  // The journey as it was drawn before branches: the main path alone.
  const journey = {
    ...branched,
    nodes: branched.nodes.filter((n) => !n.branch),
    edges: branched.edges.filter((e) => !e.branch),
    tour: branched.tour.filter((t) => !(t.focus.nodes ?? []).some((id) => branched.nodes.find((n) => n.id === id)?.branch)),
  };

  it('lays a journey with no branches out as before: no paths of its own for the arrows, no key', async () => {
    expect(hasBranches(journey)).toBe(false);
    const l = await layoutMap(journey);
    expect(l.routes).toBeUndefined();
    expect(l.branches).toBeUndefined();
  });

  it('lays the boxes out in one line, left to right, with no area boxes', async () => {
    const l = await layoutMap(journey);
    expect(l.flat).toBe(true);
    expect(l.areas).toEqual({});
    expect(l.nodes.j1!.x).toBeLessThan(l.nodes.j2!.x);
    expect(l.nodes.j2!.x).toBeLessThan(l.nodes.j3!.x);
    expect(sides(journey, l, 'j1', 'j2')).toEqual(['r', 'l']);
  });

  it('gives a choice more room than a screen, and lays out again when a box changes kind', async () => {
    const forked = { ...journey, nodes: journey.nodes.map((n) => (n.id === 'j2' ? { ...n, kind: 'choice' as const } : n)) };
    const l = await layoutMap(forked);
    expect(l.nodes.j2!.h).toBeGreaterThan(l.nodes.j1!.h);
    expect(layoutKey(forked)).not.toBe(layoutKey(journey));
  });

  it('wraps a long journey onto rows, and joins the rows from the bottom of one to the top of the next', async () => {
    const base = journey.nodes[0]!;
    const ids = Array.from({ length: 10 }, (_, i) => `k${i}`);
    const long = {
      ...journey,
      nodes: ids.map((id) => ({ ...base, id, screen: undefined })),
      edges: ids.slice(1).map((id, i) => ({ ...journey.edges[0]!, id: `ke${i}`, from: ids[i]!, to: id })),
      tour: [],
    };
    const l = await layoutMap(long);
    const rows = new Set(ids.map((id) => Math.round(l.nodes[id]!.y)));
    expect(rows.size).toBeGreaterThan(1);
    expect(l.width).toBeLessThan(1900);
    const turn = ids.findIndex((id, i) => i > 0 && l.nodes[id]!.x < l.nodes[ids[i - 1]!]!.x);
    expect(turn).toBeGreaterThan(0);
    expect(sides(long, l, ids[turn - 1]!, ids[turn]!)).toEqual(['b', 't']);
  });

  it('gives a screen box with a picture more room, and still wraps a long journey of them', async () => {
    const base = journey.nodes[0]!;
    const ids = Array.from({ length: 8 }, (_, i) => `p${i}`);
    const long = { ...journey, nodes: ids.map((id) => ({ ...base, id })), edges: ids.slice(1).map((id, i) => ({ ...journey.edges[0]!, id: `pe${i}`, from: ids[i]!, to: id })), tour: [] };
    const plain = await layoutMap(long);
    const l = await layoutMap(long, [], new Set(), new Set(ids));
    expect(l.nodes.p0!.h).toBeGreaterThan(plain.nodes.p0!.h);
    expect(l.width).toBeLessThan(1900);
    expect(layoutKey(long, [], [], ids)).not.toBe(layoutKey(long));
  });

  it('starts a journey that loops back where the person starts, not in the middle of the loop', async () => {
    const base = journey.nodes[0]!;
    const ids = ['a', 'b', 'c', 'd', 'e', 'f'];
    const arrows = [['a', 'b'], ['b', 'c'], ['c', 'd'], ['d', 'e'], ['e', 'f'], ['f', 'b']];
    const loop = { ...journey, nodes: ids.map((id) => ({ ...base, id })), edges: arrows.map(([from, to], i) => ({ ...journey.edges[0]!, id: `le${i}`, from: from!, to: to! })), tour: [] };
    const l = await layoutMap(loop);
    const rank = (id: string) => rowOf(l, id) * 100_000 + l.nodes[id]!.x; // reading order: row, then column
    for (let i = 1; i < ids.length; i++) expect(rank(ids[i - 1]!)).toBeLessThan(rank(ids[i]!));
  });
});

const overlaps = (a: Box, b: Box) => a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
const middle = (b: Box) => b.y + b.h / 2;

describe('layoutMap on a journey with branches', () => {
  it('puts the main path on one straight line, in order, and each branch below the step it leaves from', async () => {
    const l = await layoutMap(branched);
    const { j1, j2, j3, j4, j5, j6 } = l.nodes as Record<string, Box>;
    expect(new Set([j1, j2, j3].map((b) => middle(b!))).size).toBe(1);
    expect(j1!.x).toBeLessThan(j2!.x);
    expect(j2!.x).toBeLessThan(j3!.x);
    const line = Math.max(...[j1, j2, j3].map((b) => b!.y + b!.h));
    for (const b of [j4, j5, j6]) expect(b!.y).toBeGreaterThan(line);
    // A branch starts between the step it leaves from and the next one.
    for (const b of [j4, j5]) expect(middle(b!) > 0 && b!.x + b!.w / 2 > j1!.x + j1!.w / 2 && b!.x + b!.w / 2 < j2!.x + j2!.w / 2).toBe(true);
    expect(j6!.x + j6!.w / 2).toBeGreaterThan(j2!.x + j2!.w / 2);
    // Two branches leave the patient page: the one that comes back to it goes below the one that doesn't.
    expect(j4!.y).toBeGreaterThan(j5!.y + j5!.h);
    // Branches that don't overlap share a line.
    expect(middle(j5!)).toBe(middle(j6!));
  });

  it('gives every arrow a path from one box to the other, and colours in the order branches leave the main path', async () => {
    const l = await layoutMap(branched);
    expect(l.branches).toEqual(['Writes a note instead', 'Goes back to the lists', 'No welcome rule']);
    for (const e of branched.edges) {
      const r = l.routes?.[`e:${e.id}`];
      expect(r, e.id).toBeDefined();
      const [first, last] = [r!.points[0]!, r!.points.at(-1)!];
      const touches = (b: Box, [x, y]: [number, number]) => x >= b.x - 1 && x <= b.x + b.w + 1 && y >= b.y - 1 && y <= b.y + b.h + 1;
      expect(touches(l.nodes[e.from]!, first), e.id).toBe(true);
      expect(touches(l.nodes[e.to]!, last), e.id).toBe(true);
    }
    // The arrow back to the patient page comes up beside the ones going down from it, not on top of them.
    const xs = ['jc', 'jd', 'je'].map((id) => l.routes![`e:${id}`]!.points[id === 'jd' ? 3 : 0]![0]);
    expect(new Set(xs).size).toBe(3);
  });

  it('never wraps the main path, and stacks branches from the same and nearby steps without overlapping', async () => {
    const base = branched.nodes[1]!;
    const step = (id: string, branch?: string) => ({ ...base, id, ...(branch ? { branch } : {}) });
    const arrow = (id: string, from: string, to: string, branch?: string) => ({ ...branched.edges[0]!, id, from, to, ...(branch ? { branch } : {}) });
    const ids = Array.from({ length: 10 }, (_, i) => `m${i}`);
    const long = {
      ...branched,
      nodes: [...ids.map((id) => step(id)), step('a1', 'A'), step('a2', 'A'), step('b1', 'B'), step('c1', 'C'), step('d1', 'D'), step('d2', 'D')],
      edges: [
        ...ids.slice(1).map((id, i) => arrow(`me${i}`, ids[i]!, id)),
        arrow('xa', 'm2', 'a1', 'A'), arrow('xa2', 'a1', 'a2'), arrow('ya', 'a2', 'm5', 'A'), // comes back further along
        arrow('xb', 'm2', 'b1', 'B'), // ends on its own
        arrow('xc', 'm3', 'c1', 'C'), arrow('yc', 'c1', 'm1', 'C'), // goes back
        arrow('xd', 'c1', 'd1', 'D'), arrow('xd2', 'd1', 'd2', 'D'), // leaves from a branch
        arrow('back', 'm8', 'm0'), // back along the main path
      ],
      tour: [],
    };
    const l = await layoutMap(long);
    expect(l.width).toBeGreaterThan(1900);
    expect(new Set(ids.map((id) => middle(l.nodes[id]!))).size).toBe(1);
    for (let i = 1; i < ids.length; i++) expect(l.nodes[ids[i]!]!.x).toBeGreaterThan(l.nodes[ids[i - 1]!]!.x);
    const boxes = Object.entries(l.nodes);
    for (const [i, [a, A]] of boxes.entries()) for (const [b, B] of boxes.slice(i + 1)) expect(overlaps(A, B), `${a} and ${b}`).toBe(false);
    expect(l.nodes.b1!.y).toBeGreaterThan(l.nodes.a1!.y); // of two from the same step, the one that comes back is nearer
    expect(l.nodes.d1!.y).toBeGreaterThan(l.nodes.c1!.y); // a branch off a branch goes below it
    // The arrow back along the main path runs above it.
    expect(Math.min(...l.routes!['e:back']!.points.map((p) => p[1]))).toBeLessThan(Math.min(...ids.map((id) => l.nodes[id]!.y)));
    // No upright stretch of an arrow runs through a box it doesn't start or end at.
    for (const e of long.edges) {
      const pts = l.routes![`e:${e.id}`]!.points;
      for (let i = 1; i < pts.length; i++) {
        const [[x1, y1], [x2, y2]] = [pts[i - 1]!, pts[i]!];
        if (x1 !== x2) continue;
        for (const [id, b] of boxes) {
          if (id === e.from || id === e.to) continue;
          expect(x1 > b.x && x1 < b.x + b.w && Math.min(y1, y2) < b.y + b.h && Math.max(y1, y2) > b.y, `${e.id} through ${id}`).toBe(false);
        }
      }
    }
    expect(layoutKey({ ...long, nodes: long.nodes.map((n) => (n.id === 'b1' ? { ...n, branch: 'A' } : n)) })).not.toBe(layoutKey(long));
  });
});

describe('layoutMap on a timeline', () => {
  // Alice and Bob take turns across two chains: A, then B, then back to A.
  const swap = (() => {
    const cites = [{ file: 'src/swap.ts', line: 1, text: 'x' }];
    const r = parseMap({
      version: 1,
      question: 'How does the swap work?',
      view: 'timeline',
      areas: [{ id: 'a', label: 'Chain A' }, { id: 'b', label: 'Chain B' }],
      nodes: [
        { id: 'lockA', area: 'a', name: 'Alice locks on A', evidence: cites },
        { id: 'lockB', area: 'b', name: 'Bob locks on B', evidence: cites },
        { id: 'claimB', area: 'b', name: 'Alice claims on B', evidence: cites },
        { id: 'claimA', area: 'a', name: 'Bob claims on A', evidence: cites },
      ],
      edges: [
        { id: 'e1', from: 'lockA', to: 'lockB', kind: 'then', label: 'then Bob locks', evidence: cites },
        { id: 'e2', from: 'lockB', to: 'claimB', kind: 'then', label: 'then Alice claims', evidence: cites },
        { id: 'e3', from: 'claimB', to: 'claimA', kind: 'then', label: 'then Bob claims', evidence: cites },
      ],
    });
    if (!r.ok) throw new Error(r.problems.join('\n'));
    return r.map;
  })();

  it('gives each area a lane across the whole map, one under the other', async () => {
    const l = await layoutMap(swap);
    expect(l.lanes).toBe(true);
    expect(l.areas.a).toMatchObject({ x: 0, w: l.width });
    expect(l.areas.b).toMatchObject({ x: 0, w: l.width });
    expect(l.areas.a!.y + l.areas.a!.h).toBeLessThanOrEqual(l.areas.b!.y);
  });

  it('puts the steps left to right in the order they happen, even when the flow comes back to a lane', async () => {
    const l = await layoutMap(swap);
    const xs = ['lockA', 'lockB', 'claimB', 'claimA'].map((id) => l.nodes[id]!.x);
    expect(xs).toEqual([...xs].sort((p, q) => p - q));
    expect(new Set(xs).size).toBe(4);
  });

  it('joins steps in the same lane side to side, and steps in one column top to bottom', async () => {
    const l = await layoutMap(swap);
    expect(sides(swap, l, 'lockB', 'claimB')).toEqual(['r', 'l']);
    expect(sides(swap, l, 'claimB', 'claimA')).toEqual(['r', 'l']);
  });

  it('lays a loop out with only the arrow that goes back pointing left', async () => {
    const loop = { ...swap, edges: [...swap.edges, { ...swap.edges[0]!, id: 'back', from: 'claimA', to: 'lockA' }] };
    const l = await layoutMap(loop);
    expect(l.nodes.lockA!.x).toBeLessThan(l.nodes.claimA!.x);
  });

  it('lays out again when a map becomes a timeline', () => {
    expect(layoutKey(swap)).not.toBe(layoutKey({ ...swap, view: undefined }));
  });
});

describe('ways through on any kind of map', () => {
  // Two ways to make an account that meet at the same record, and a proposal with two options.
  const cites = [{ file: 'src/accounts.ts', line: 1, text: 'x' }];
  const twoWays = (view?: 'timeline') => {
    const r = parseMap({
      version: 1,
      question: 'Where does creating an account happen?',
      ...(view ? { view } : {}),
      areas: [{ id: 'web', label: 'Dashboard' }, { id: 'chain', label: 'On chain' }],
      nodes: [
        { id: 'form', area: 'web', name: 'API-key form', evidence: cites, branch: 'API key' },
        { id: 'wallet', area: 'web', name: 'Wallet sign-up', evidence: cites, branch: 'Wallet' },
        { id: 'owner', area: 'chain', name: 'Wallet becomes owner', evidence: cites, branch: 'Wallet' },
        { id: 'record', area: 'chain', name: 'Account recorded', evidence: cites },
      ],
      edges: [
        { id: 'e1', from: 'form', to: 'record', kind: 'then', label: 'server writes', evidence: cites },
        { id: 'e2', from: 'wallet', to: 'owner', kind: 'then', label: 'wallet signs', evidence: cites },
        { id: 'e3', from: 'owner', to: 'record', kind: 'then', label: 'then creates', evidence: cites },
      ],
    });
    if (!r.ok) throw new Error(r.problems.join('\n'));
    return r.map;
  };

  it('names the ways for the key, in the order they first appear', () => {
    expect(branchNames(twoWays())).toEqual(['API key', 'Wallet']);
    expect(branchNames(map)).toEqual([]);
  });

  it('hands them back from the area layout and the timeline, so any map gets the colours and key', async () => {
    expect((await layoutMap(twoWays())).branches).toEqual(['API key', 'Wallet']);
    expect((await layoutMap(twoWays('timeline'))).branches).toEqual(['API key', 'Wallet']);
    expect((await layoutMap(map)).branches).toBeUndefined();
  });

  it('lays the map out the same with or without its ways named', async () => {
    const named = await layoutMap(twoWays());
    const plain = twoWays();
    for (const x of [...plain.nodes, ...plain.edges]) delete x.branch;
    expect((await layoutMap(plain)).nodes).toEqual(named.nodes);
  });

  it('draws it again when a way is renamed, since the key changes', () => {
    const renamed = twoWays();
    renamed.nodes[0]!.branch = 'Through the server';
    expect(layoutKey(renamed)).not.toEqual(layoutKey(twoWays()));
  });
});
