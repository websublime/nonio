// Arrow labels never cover a box or each other: they stay put when clear, slide along their arrow, or become a small
// "⋯" pill (a dot) that shows their words when pointed at.
import { describe, expect, it } from 'vitest';
import { DOT, placeLabels, type LabelInput } from '../src/labels';

// An arrow along y = 0 from x = 0 to x = 400.
const flat = (id: string, w = 60, h = 20, at = { x: 200, y: 0 }): LabelInput => ({ id, w, h, at, along: (t) => ({ x: 400 * t, y: 0 }) });

describe('placing arrow labels', () => {
  it('leaves a label where its arrow put it when that spot is clear', () => {
    expect(placeLabels([flat('a')], [])).toEqual({});
  });

  it('slides a label along its arrow, off a box', () => {
    const box = { x: 170, y: -30, w: 60, h: 60 }; // over the middle of the arrow
    const out = placeLabels([flat('a')], [box]);
    const p = out.a as { x: number; y: number };
    expect(p.y).toBe(0);
    expect(Math.abs(p.x - 200)).toBeGreaterThan(60);
  });

  it('keeps two labels at the same spot apart', () => {
    const out = placeLabels([flat('a'), flat('b')], []);
    expect(out.a).toBeUndefined(); // the first keeps its spot
    expect(out.b).not.toBe('dot');
    expect(Math.abs((out.b as { x: number }).x - 200)).toBeGreaterThan(60);
  });

  it('makes a label a dot when nowhere on its arrow is clear', () => {
    const wall = { x: -50, y: -40, w: 500, h: 80 };
    expect(placeLabels([flat('a')], [wall])).toEqual({ a: 'dot' });
  });

  it('puts the pill of a label too long to fit in a clear spot on its arrow, never on a box', () => {
    // Boxes over the whole arrow but a gap around x = 136 (a third of the way along): too narrow for the label, room
    // for the pill.
    const boxes = [{ x: -50, y: -30, w: 166, h: 60 }, { x: 156, y: -30, w: 294, h: 60 }];
    const out = placeLabels([flat('a', 200)], boxes);
    const p = out.a as { x: number; y: number; dot?: true };
    expect(p.dot).toBe(true);
    const pill = { x: p.x - DOT.w / 2, y: p.y - DOT.h / 2, w: DOT.w, h: DOT.h };
    for (const b of boxes) expect(pill.x < b.x + b.w && b.x < pill.x + pill.w && pill.y < b.y + b.h && b.y < pill.y + pill.h).toBe(false);
  });

  it("makes a label a dot when its arrow can't be measured and its own spot is taken", () => {
    const blind: LabelInput = { id: 'a', w: 60, h: 20, at: { x: 200, y: 0 }, along: () => null };
    expect(placeLabels([blind], [{ x: 190, y: -10, w: 20, h: 20 }])).toEqual({ a: 'dot' });
  });

  it("never makes a missing connection's label a dot, and places it before the others", () => {
    const wall = { x: -50, y: -40, w: 500, h: 80 };
    const gap = { ...flat('a:gap'), keep: true };
    expect(placeLabels([flat('e:x'), gap], [wall])).toEqual({ 'e:x': 'dot' });
  });

  it('crosses an area title rather than become a dot', () => {
    const title = { x: -50, y: -40, w: 500, h: 80 };
    expect(placeLabels([flat('a')], [], [title])).toEqual({});
  });
});
