// Where each arrow's label goes, so no label covers a box or another label. A label stays at the middle of its arrow
// when that spot is clear; otherwise it slides along its own arrow to the first clear spot; if there is none, it
// becomes a small "⋯" pill on the arrow, placed clear of boxes too where it can be, that shows its words when pointed at
// or selected. Pure geometry, in the map's own coordinates.

export type Rect = { x: number; y: number; w: number; h: number };
export type Point = { x: number; y: number };
/** A label that didn't fit, shown as a small pill at this spot on its arrow. */
export type Dot = Point & { dot: true };
/** Where a label goes: a spot on its arrow, a pill there (`Dot`), or a pill where its arrow put it (`'dot'`). */
export type LabelPlace = Point | Dot | 'dot';

export type LabelInput = {
  id: string;
  /** The label's size. */
  w: number;
  h: number;
  /** Where the arrow puts it on its own (the middle of the arrow, or the spot a routed arrow chose). */
  at: Point;
  /** The point a fraction of the way along the arrow, or null when the arrow can't be measured. */
  along: (t: number) => Point | null;
  /** Never made a dot (a missing connection: the map's most important words); placed first, kept where it is if nothing is clear. */
  keep?: boolean;
};

/** Fractions along the arrow to try, nearest the middle first, keeping clear of the arrow's ends. */
export const SPOTS = [0.5, 0.42, 0.58, 0.34, 0.66, 0.26, 0.74, 0.2, 0.8];
/** A pill is small enough to sit nearer an arrow's ends too, where an arrow that runs under a box comes out. */
const PILL_SPOTS = [...SPOTS, 0.86, 0.14, 0.9, 0.1];
/** Room kept around boxes and between labels. */
const GAP = 4;
/** The "⋯" pill a label becomes when its words don't fit (`.link-label.dot` in styles.css). */
export const DOT = { w: 26, h: 18 };

const overlaps = (a: Rect, b: Rect): boolean =>
  a.x < b.x + b.w + GAP && b.x < a.x + a.w + GAP && a.y < b.y + b.h + GAP && b.y < a.y + a.h + GAP;

const around = (p: Point, w: number, h: number): Rect => ({ x: p.x - w / 2, y: p.y - h / 2, w, h });

/**
 * A place for every label: labels that must stay readable first, then the rest in order; each avoids the boxes and
 * the labels placed before it. `soft` things (area titles) are avoided when possible, but a label would rather cross
 * one than become a dot. A label whose own spot is clear is left out of the result (it stays where its arrow put it).
 */
export function placeLabels(labels: readonly LabelInput[], boxes: readonly Rect[], soft: readonly Rect[] = []): Record<string, LabelPlace> {
  const taken: Rect[] = [];
  const out: Record<string, LabelPlace> = {};
  const clearOf = (r: Rect, avoidSoft: boolean) =>
    !boxes.some((b) => overlaps(r, b)) && !taken.some((t) => overlaps(r, t)) && (!avoidSoft || !soft.some((s) => overlaps(r, s)));
  const ordered = [...labels.filter((l) => l.keep), ...labels.filter((l) => !l.keep)];
  /** The first clear spot for something this size: where the arrow put it, then along the arrow. */
  const find = (l: LabelInput, w: number, h: number, spots: readonly number[] = SPOTS): Point | null => {
    for (const avoidSoft of [true, false]) {
      if (clearOf(around(l.at, w, h), avoidSoft)) return l.at;
      for (const t of spots) {
        const p = l.along(t);
        if (p && clearOf(around(p, w, h), avoidSoft)) return p;
      }
    }
    return null;
  };
  for (const l of ordered) {
    const spot = find(l, l.w, l.h) ?? (l.keep ? l.at : null);
    if (!spot) {
      // No room for the words: a small pill instead, kept off boxes and other labels when there is room even for that.
      const pill = find(l, DOT.w, DOT.h, PILL_SPOTS);
      if (pill) {
        taken.push(around(pill, DOT.w, DOT.h));
        out[l.id] = { ...pill, dot: true };
      } else out[l.id] = 'dot';
      continue;
    }
    taken.push(around(spot, l.w, l.h));
    if (spot !== l.at) out[l.id] = spot;
  }
  return out;
}
