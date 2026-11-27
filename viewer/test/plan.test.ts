import { describe, expect, it } from 'vitest';
import { fixture, parseMap } from './nonio';
import { planGhost, planMarks, planViews } from '../src/plan';

const add = { kind: 'add', state: 'pending' } as const;
const remove = { kind: 'remove', state: 'pending' } as const;
const change = { kind: 'change', state: 'pending' } as const;
const built = { kind: 'add', state: 'done' } as const;

describe('seeing a proposal on its own', () => {
  it('shows what would be added only with the change, as a shadow until then', () => {
    expect(planGhost('now', add, null)).toBe('unbuilt');
    expect(planGhost('proposed', add, null)).toBeNull();
    expect(planGhost('both', add, null)).toBeNull();
  });

  it('shows what would be removed as it is now, and as a shadow with the change', () => {
    expect(planGhost('now', remove, null)).toBeNull();
    expect(planGhost('proposed', remove, null)).toBe('dropped');
    expect(planGhost('both', remove, null)).toBeNull();
  });

  it('keeps a change\'s marks with the change and on both, and none as the code is now', () => {
    expect(planMarks('now', change, null)).toBe(false);
    expect(planMarks('now', remove, null)).toBe(false);
    expect(planMarks('proposed', add, null)).toBe(true);
    expect(planMarks('proposed', change, null)).toBe(true);
    expect(planMarks('proposed', remove, null)).toBe(false);
    expect(planMarks('both', remove, null)).toBe(true);
  });

  it('leaves what is already built alone in every version', () => {
    for (const v of ['now', 'proposed', 'both'] as const) {
      expect(planGhost(v, built, null)).toBeNull();
      expect(planMarks(v, built, null)).toBe(true);
    }
  });

  it('shows one option at a time, with what every option shares', () => {
    expect(planGhost('option:A: prompt rule', add, 'A: prompt rule')).toBeNull();
    expect(planGhost('option:A: prompt rule', add, 'B: Called from list')).toBe('other');
    expect(planGhost('option:A: prompt rule', add, null)).toBeNull();
    expect(planMarks('option:A: prompt rule', add, 'B: Called from list')).toBe(false);
  });
});

describe('the versions a map offers', () => {
  const cites = [{ file: 'src/a.ts', line: 1, text: 'x' }];
  // Each proposed box hangs off the one existing box, as a proposal must.
  const map = (nodes: { id: string; area: string; name: string; branch?: string; proposed?: unknown }[]) => {
    const edges = nodes.map((n) => ({ id: `to-${n.id}`, from: 'base', to: n.id, kind: 'then', label: 'leads to', ...(n.branch ? { branch: n.branch } : {}), proposed: { kind: 'add' } }));
    const r = parseMap({ version: 1, question: 'q', areas: [{ id: 'x', label: 'X' }], nodes: [{ id: 'base', area: 'x', name: 'Base', evidence: cites }, ...nodes], edges });
    if (!r.ok) throw new Error(r.problems.join('\n'));
    return r.map;
  };
  const pendingIn = (ids: string[]) => (_: 'node' | 'edge', id: string) => ids.includes(id);

  it('offers none without anything proposed', () => {
    expect(planViews(map([]), pendingIn([]))).toEqual([]);
  });

  it('offers now, with the change, and both for a single change', () => {
    const m = map([{ id: 'n', area: 'x', name: 'New', proposed: { kind: 'add' } }]);
    expect(planViews(m, pendingIn(['n']))).toEqual(['now', 'proposed', 'both']);
  });

  it('offers each option instead when the change has options', () => {
    const m = map([
      { id: 'a', area: 'x', name: 'A', branch: 'A: prompt rule', proposed: { kind: 'add' } },
      { id: 'b', area: 'x', name: 'B', branch: 'B: Called from list', proposed: { kind: 'add' } },
    ]);
    expect(planViews(m, pendingIn(['a', 'b']))).toEqual(['now', 'option:A: prompt rule', 'option:B: Called from list', 'both']);
  });
});
