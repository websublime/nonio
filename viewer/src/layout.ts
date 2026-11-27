// Laying the map out with ELK, in two passes: each area's steps stack top to bottom in the order the arrows go, then
// the areas themselves run left to right along the arrows that cross between them. A timeline is laid out apart
// (`layoutLanes`): a lane per area, time left to right. A journey too (`layoutJourney`, `layoutBranches`). Missing connections and proposed
// fixes take part like arrows do. Arrows attach to the sides of the boxes that face each other.
// What answers added is laid out around the map as it was first drawn, so the boxes someone already knows never move:
// additions to an existing area go in rows below its steps, new areas go to the right.
import type { ElkExtendedEdge, ElkNode } from 'elkjs/lib/elk.bundled.js';
import type { CodeMap } from './types';

export const NODE_W = 232;
export const NODE_H = 96;
export type Box = { x: number; y: number; w: number; h: number };
/** Area boxes are absolute; node boxes are relative to their area. `flat` (a journey): no area boxes, node boxes absolute. */
export type Layout = {
  areas: Record<string, Box>;
  nodes: Record<string, Box>;
  width: number;
  height: number;
  flat?: boolean;
  /** A wrapped journey: where each row starts, top to bottom. */
  rowTops?: number[];
  /** A journey with branches: the path each arrow takes, by link id (`e:`, `a:` or `x:` and its id). */
  routes?: Record<string, Route>;
  /** The ways through the map names (`branch`), in the order their colours are given out (see `branchNames`). */
  branches?: string[];
  /** A timeline: each area a lane across the map, time left to right (`layoutLanes`). */
  lanes?: boolean;
};

export type Point = [number, number];
/** An arrow drawn through these points; its words sit on the middle of segment `label` (from `points[label]`). */
export type Route = { points: Point[]; label: number };

/** A journey drawn as one main path with branches below it: any box or arrow on it names a branch. */
export const hasBranches = (map: CodeMap): boolean => map.view === 'journey' && [...map.nodes, ...map.edges].some((x) => !!x.branch);

/** Which row of a wrapped journey a box is on (0 when it isn't wrapped). */
export function rowOf(layout: Layout, id: string): number {
  const y = layout.nodes[id]?.y ?? 0;
  const tops = layout.rowTops ?? [];
  let row = 0;
  for (const [i, top] of tops.entries()) if (y >= top - 1) row = i;
  return row;
}

/** Room a journey box gives the picture of its screen, drawn above its name. */
export const PICTURE_H = 132;

/** A journey's boxes are sized by what they are; a choice is a diamond, so it needs room around its words. */
export function journeySize(kind: string, pictured = false): { w: number; h: number } {
  if (kind === 'choice') return { w: 196, h: 132 };
  if (kind === 'action') return { w: 196, h: 60 };
  return pictured ? { w: 232, h: 64 + PICTURE_H } : { w: 196, h: 72 };
}
export type ExtraEdge = { id: string; from: string; to: string };
export type Side = 't' | 'r' | 'b' | 'l';

// ELK is most of the bundle; load it on first layout so the page shell shows up first.
let elk: Promise<{ layout(graph: ElkNode): Promise<ElkNode> }> | null = null;
const getElk = () => (elk ??= import('elkjs/lib/elk.bundled.js').then((m) => new m.default()));

const LAYERED = {
  'elk.algorithm': 'layered',
  'elk.layered.nodePlacement.strategy': 'NETWORK_SIMPLEX',
  'elk.layered.considerModelOrder.strategy': 'NODES_AND_EDGES',
};

/** Every connection on the map, as from → to pairs between boxes. */
function links(map: CodeMap, extra: readonly ExtraEdge[]): { id: string; from: string; to: string }[] {
  const ids = new Set(map.nodes.map((n) => n.id));
  return [
    ...map.edges.map((e) => ({ id: `e:${e.id}`, from: e.from, to: e.to })),
    ...map.absences.flatMap((a) => (a.between ? [{ id: `a:${a.id}`, from: a.between.from, to: a.between.to }] : [])),
    ...extra.map((x) => ({ id: `x:${x.id}`, from: x.from, to: x.to })),
  ].filter((l) => ids.has(l.from) && ids.has(l.to) && l.from !== l.to);
}

type Engine = Awaited<ReturnType<typeof getElk>>;
const PAD = { top: 52, side: 26, bottom: 26 };
const ROW_GAP = 64;
const COL_GAP = 44;
const AREA_GAP = 56;
const COLUMN_GAP = 140;

/** Pass 1 for one area: its steps top to bottom, with room between rows for arrow labels. */
async function layoutArea(engine: Engine, areaId: string, members: readonly { id: string }[], inside: readonly { id: string; from: string; to: string }[]) {
  const out = await engine.layout({
    id: areaId,
    layoutOptions: {
      ...LAYERED,
      'elk.direction': 'DOWN',
      'elk.padding': `[top=${PAD.top},left=${PAD.side},bottom=${PAD.bottom},right=${PAD.side}]`,
      'elk.layered.spacing.nodeNodeBetweenLayers': String(ROW_GAP),
      'elk.spacing.nodeNode': String(COL_GAP),
    },
    children: members.map((n) => ({ id: n.id, width: NODE_W, height: NODE_H })),
    edges: inside.map((l): ElkExtendedEdge => ({ id: l.id, sources: [l.from], targets: [l.to] })),
  });
  const nodes: Record<string, Box> = {};
  for (const c of out.children ?? []) nodes[c.id] = { x: c.x ?? 0, y: c.y ?? 0, w: NODE_W, h: NODE_H };
  return { nodes, size: { w: out.width ?? NODE_W + 2 * PAD.side, h: out.height ?? NODE_H + PAD.top + PAD.bottom } };
}

/** The map without the given ids, and without arrows and gaps that lose an end. */
function withoutIds(map: CodeMap, ids: ReadonlySet<string>): CodeMap {
  const nodes = map.nodes.filter((n) => !ids.has(n.id));
  const kept = new Set(nodes.map((n) => n.id));
  return {
    ...map,
    areas: map.areas.filter((a) => !ids.has(a.id)),
    nodes,
    edges: map.edges.filter((e) => !ids.has(e.id) && kept.has(e.from) && kept.has(e.to)),
    absences: map.absences.filter((a) => !ids.has(a.id) && (!a.between || (kept.has(a.between.from) && kept.has(a.between.to)))),
  };
}

/**
 * Lay the map out. `added` names what answers added since it was first drawn: the rest is laid out exactly as it would
 * be without them, and the additions are fitted around it.
 */
export async function layoutMap(map: CodeMap, extra: readonly ExtraEdge[] = [], added: ReadonlySet<string> = new Set(), pictured: ReadonlySet<string> = new Set()): Promise<Layout> {
  // A journey is one line of boxes, so anything added is simply laid out with the rest. Its layout places its branches
  // and names them itself.
  if (map.view === 'journey') return hasBranches(map) ? layoutBranches(map, extra, pictured) : layoutJourney(map, extra, pictured);
  // Any other map only colours its ways through, so the layout just names them for the key.
  const named = branchNames(map);
  const l = await arrange(map, extra, added);
  return named.length ? { ...l, branches: named } : l;
}

/** The ways through a map names (`branch`), in the order they first appear among its boxes, then its arrows. */
export function branchNames(map: CodeMap): string[] {
  const names: string[] = [];
  for (const x of [...map.nodes, ...map.edges]) if (x.branch && !names.includes(x.branch)) names.push(x.branch);
  return names;
}

async function arrange(map: CodeMap, extra: readonly ExtraEdge[], added: ReadonlySet<string>): Promise<Layout> {
  // A timeline puts everything in its place in time, what answers added included (so known boxes can move).
  if (map.view === 'timeline') return layoutLanes(map, extra);
  const addedNodes = map.nodes.filter((n) => added.has(n.id));
  if (!addedNodes.length) return layoutWhole(map, extra);

  const base = withoutIds(map, added);
  const baseIds = new Set(base.nodes.map((n) => n.id));
  const l = await layoutWhole(base, extra.filter((x) => baseIds.has(x.from) && baseIds.has(x.to)));
  const engine = await getElk();
  const areas: Record<string, Box> = Object.fromEntries(Object.entries(l.areas).map(([id, b]) => [id, { ...b }]));
  const nodes: Record<string, Box> = { ...l.nodes };

  // Additions to areas that are already drawn: rows below the steps already there. The area grows downward.
  const growth: Record<string, number> = {};
  const fresh: Record<string, typeof addedNodes> = {};
  for (const n of addedNodes) (fresh[n.area] ??= []).push(n);
  for (const [areaId, members] of Object.entries(fresh)) {
    const box = areas[areaId];
    if (!box) continue;
    const perRow = Math.max(1, Math.floor((box.w - 2 * PAD.side + COL_GAP) / (NODE_W + COL_GAP)));
    const top = Math.max(PAD.top - ROW_GAP, ...base.nodes.filter((n) => n.area === areaId).map((n) => (nodes[n.id]?.y ?? 0) + NODE_H)) + ROW_GAP;
    members.forEach((n, i) => {
      nodes[n.id] = { x: PAD.side + (i % perRow) * (NODE_W + COL_GAP), y: top + Math.floor(i / perRow) * (NODE_H + ROW_GAP), w: NODE_W, h: NODE_H };
    });
    const h = Math.max(box.h, top + Math.ceil(members.length / perRow) * (NODE_H + ROW_GAP) - ROW_GAP + PAD.bottom);
    growth[areaId] = h - box.h;
    box.h = h;
  }

  // Areas below one that grew slide down by as much (and so on down the column). Nothing moves sideways.
  const original = Object.fromEntries(Object.entries(l.areas).map(([id, b]) => [id, b]));
  const order = Object.keys(original).sort((a, b) => original[a]!.y - original[b]!.y);
  const shift: Record<string, number> = {};
  for (const b of order) {
    const B = original[b]!;
    shift[b] = Math.max(0, ...order
      .filter((a) => a !== b && shift[a] !== undefined)
      .filter((a) => {
        const A = original[a]!;
        return A.y + A.h <= B.y + 1 && A.x < B.x + B.w && B.x < A.x + A.w;
      })
      .map((a) => shift[a]! + (growth[a] ?? 0)));
    areas[b]!.y += shift[b]!;
  }

  // New areas: each laid out on its own, stacked in a column to the right of everything.
  const all = links(map, extra);
  const areaOf = new Map(map.nodes.map((n) => [n.id, n.area]));
  const x = Math.max(0, ...Object.values(areas).map((b) => b.x + b.w)) + COLUMN_GAP;
  let y = 0;
  for (const area of map.areas) {
    if (areas[area.id]) continue;
    const members = map.nodes.filter((n) => n.area === area.id);
    if (!members.length) continue;
    const inside = all.filter((k) => areaOf.get(k.from) === area.id && areaOf.get(k.to) === area.id);
    const out = await layoutArea(engine, area.id, members, inside);
    Object.assign(nodes, out.nodes);
    areas[area.id] = { x, y, w: out.size.w, h: out.size.h };
    y += out.size.h + AREA_GAP;
  }

  const boxes = Object.values(areas);
  return { areas, nodes, width: Math.max(0, ...boxes.map((b) => b.x + b.w)), height: Math.max(0, ...boxes.map((b) => b.y + b.h)) };
}

/** A journey wider than this is wrapped onto rows, read like lines of text, so it uses the height of the screen too. */
const JOURNEY_WRAP_OVER = 1900;
/** Room between one row of a wrapped journey and the next: enough for the arrow that carries on to the next row. */
export const JOURNEY_ROW_GAP = 130;

/**
 * A journey: every box in one graph, left to right in the order the arrows go, with room between for arrow labels.
 * A long one is cut into rows here (not by ELK, whose wrapping scatters a journey that loops back or has two starts):
 * the columns stay in the order ELK gave them, and each row starts again from the left, like a line of text.
 */
async function layoutJourney(map: CodeMap, extra: readonly ExtraEdge[], pictured: ReadonlySet<string>): Promise<Layout> {
  const engine = await getElk();
  const out = await engine.layout({
    id: 'root',
    // A journey that loops back (a screen that leads to an earlier one) is opened at the arrow going against the order
    // the boxes were written in, so it still starts where the person starts.
    layoutOptions: { ...LAYERED, 'elk.direction': 'RIGHT', 'elk.layered.cycleBreaking.strategy': 'MODEL_ORDER', 'elk.layered.spacing.nodeNodeBetweenLayers': '176', 'elk.spacing.nodeNode': '56', 'elk.spacing.componentComponent': '72' },
    children: map.nodes.map((n) => ({ id: n.id, width: journeySize(n.kind, pictured.has(n.id)).w, height: journeySize(n.kind, pictured.has(n.id)).h })),
    edges: links(map, extra).map((l): ElkExtendedEdge => ({ id: l.id, sources: [l.from], targets: [l.to] })),
  });
  const nodes: Record<string, Box> = {};
  for (const c of out.children ?? []) nodes[c.id] = { x: c.x ?? 0, y: c.y ?? 0, w: c.width ?? 0, h: c.height ?? 0 };
  const width = out.width ?? 0;
  if (width <= JOURNEY_WRAP_OVER) return { areas: {}, nodes, width, height: out.height ?? 0, flat: true };

  // Columns: boxes whose middles line up. Rows: runs of columns that fit the width, about the same number in each.
  const columns: { left: number; right: number; ids: string[] }[] = [];
  for (const [id, b] of Object.entries(nodes).sort((a, b) => a[1].x + a[1].w / 2 - (b[1].x + b[1].w / 2))) {
    const last = columns[columns.length - 1];
    if (last && b.x + b.w / 2 < last.right) {
      last.ids.push(id);
      last.left = Math.min(last.left, b.x);
      last.right = Math.max(last.right, b.x + b.w);
    } else columns.push({ left: b.x, right: b.x + b.w, ids: [id] });
  }
  const rowCount = Math.ceil(width / JOURNEY_WRAP_OVER);
  const perRow = Math.ceil(columns.length / rowCount);
  let top = 0;
  let widest = 0;
  const rowTops: number[] = [];
  for (let r = 0; r * perRow < columns.length; r++) {
    const row = columns.slice(r * perRow, (r + 1) * perRow);
    const ids = row.flatMap((c) => c.ids);
    const left = row[0]!.left;
    const up = Math.min(...ids.map((id) => nodes[id]!.y));
    rowTops.push(top);
    for (const id of ids) nodes[id] = { ...nodes[id]!, x: nodes[id]!.x - left, y: nodes[id]!.y - up + top };
    top = Math.max(...ids.map((id) => nodes[id]!.y + nodes[id]!.h)) + JOURNEY_ROW_GAP;
    widest = Math.max(widest, row[row.length - 1]!.right - left);
  }
  return { areas: {}, nodes, width: widest, height: top - JOURNEY_ROW_GAP, flat: true, rowTops };
}

/** Between boxes along a line (the main path, or a branch): room for the words on the arrow between them. */
const LINE_GAP = 176;
/** Between the main path and the first branch below it, and between one branch and the next. */
const LANE_GAP = 120;
/** How far above a branch's boxes the arrow into it runs across, and below them an arrow back. */
const LANE_RUN = 34;
/** Between arrows that meet the same side of a box. */
const PORT = 14;
/** How far above the main path an arrow that skips along it, or goes back along it, runs. */
const ARC = 58;

/** Reading order: each box after the boxes that lead to it, in the order they were written; a loop opens where it was written first. */
function readingOrder(ids: readonly string[], ls: readonly { from: string; to: string }[]): string[] {
  const into = new Map(ids.map((id) => [id, 0]));
  for (const l of ls) into.set(l.to, (into.get(l.to) ?? 0) + 1);
  const left = new Set(ids);
  const out: string[] = [];
  while (left.size) {
    const next = ids.find((id) => left.has(id) && !into.get(id)) ?? ids.find((id) => left.has(id))!;
    left.delete(next);
    out.push(next);
    for (const l of ls) if (l.from === next && left.has(l.to)) into.set(l.to, into.get(l.to)! - 1);
  }
  return out;
}

/**
 * A journey with branches. The main path (every box with no `branch`) runs along one straight line, left to right, in
 * reading order, and is never wrapped: a long one is panned along. Each branch is a line of its own below the step it
 * leaves from, starting halfway to the next step, so the arrows that go down from later steps, and back up to them,
 * pass between its boxes. Branches that would overlap stack downward; of those leaving the same step, the ones that
 * come back further along go nearest the main path and the ones that go back to an earlier step furthest from it, so
 * their arrows don't cross. Every arrow's path is worked out here (`routes`), so it can run around boxes.
 */
async function layoutBranches(map: CodeMap, extra: readonly ExtraEdge[], pictured: ReadonlySet<string>): Promise<Layout> {
  const all = links(map, extra);
  const size = new Map(map.nodes.map((n) => [n.id, journeySize(n.kind, pictured.has(n.id))]));
  const branchOf = new Map(map.nodes.map((n) => [n.id, n.branch ?? null]));
  const order = readingOrder(map.nodes.map((n) => n.id), all);
  const main = order.filter((id) => !branchOf.get(id));
  // Every box is on a branch: there is no main path to hang them from, so it is drawn as a journey without branches.
  if (!main.length) return layoutJourney(map, extra, pictured);
  const w = (id: string) => size.get(id)!.w;
  const h = (id: string) => size.get(id)!.h;

  // The main path, on lane 0.
  const left = new Map<string, number>();
  const lane = new Map<string, number>();
  let x = 0;
  for (const id of main) {
    left.set(id, x);
    lane.set(id, 0);
    x += w(id) + LINE_GAP;
  }
  const cx = (id: string) => left.get(id)! + w(id) / 2;
  const right = (id: string) => left.get(id)! + w(id);

  // Branches, each a line of boxes in reading order, named in the order they are first mentioned.
  const mention = [...new Set([...map.nodes, ...map.edges].flatMap((x) => (x.branch ? [x.branch] : [])))];
  const members = new Map<string, string[]>();
  for (const id of order) {
    const b = branchOf.get(id);
    if (b) members.set(b, [...(members.get(b) ?? []), id]);
  }
  type Placed = { name: string; lane: number; span: [number, number]; verticals: { x: number; from: number }[]; boxes: string[]; root: number };
  const placed: Placed[] = [];
  const rootOf = new Map<string, number>();
  const pending = new Set(members.keys());
  const plan = (name: string, force: boolean) => {
    const ids = members.get(name)!;
    const inside = new Set(ids);
    const into = all.filter((l) => inside.has(l.to) && !inside.has(l.from));
    // Not ready while it is only reached from a branch that isn't placed yet.
    if (!force && into.length && !into.some((l) => lane.has(l.from))) return null;
    const anchor = into.filter((l) => lane.has(l.from)).map((l) => l.from).sort((a, b) => lane.get(a)! - lane.get(b)! || cx(a) - cx(b))[0] ?? null;
    // The first box sits halfway between the step it leaves from and the step after that.
    const next = anchor && lane.get(anchor) === 0 ? main[main.indexOf(anchor) + 1] : undefined;
    const first = ids[0]!;
    const firstMid = anchor ? (next ? (cx(anchor) + cx(next)) / 2 : cx(anchor) + (w(anchor) / 2 + LINE_GAP + w(first) / 2) / 2) : w(first) / 2;
    const xs: number[] = [];
    let at = firstMid - w(first) / 2;
    for (const id of ids) {
      xs.push(at);
      at += w(id) + LINE_GAP;
    }
    const out = all.filter((l) => inside.has(l.from) && !inside.has(l.to) && lane.has(l.to));
    const ahead = out.filter((l) => cx(l.to) >= xs[0]!).map((l) => cx(l.to));
    const back = out.filter((l) => cx(l.to) < xs[0]!).map((l) => cx(l.to));
    return {
      name, ids, anchor, xs,
      /** 0: comes back to the main path further along; 1: ends on its own; 2: goes back to an earlier step. */
      kind: back.length ? 2 : ahead.length ? 0 : 1,
      reach: Math.max(0, ...ahead),
      span: [Math.min(anchor ? cx(anchor) : xs[0]!, ...back) - 30, Math.max(at - LINE_GAP, ...ahead) + 30] as [number, number],
      /** The arrows that run up or down from it, at what x, and from which lane. */
      verticals: [...(anchor ? [{ x: cx(anchor), from: lane.get(anchor)! }] : []), ...out.map((l) => ({ x: cx(l.to), from: lane.get(l.to)! }))],
      minLane: anchor ? lane.get(anchor)! + 1 : 1,
      root: anchor ? (lane.get(anchor) === 0 ? main.indexOf(anchor) : rootOf.get(branchOf.get(anchor)!) ?? main.length) : main.length,
    };
  };
  while (pending.size) {
    const ready = [...pending].flatMap((name) => plan(name, false) ?? []);
    // Branches that lead only into each other, with no way in from what is placed: the first one starts on its own.
    const p = ready.sort((a, b) => (a.anchor ? cx(a.anchor) : Infinity) - (b.anchor ? cx(b.anchor) : Infinity) || a.kind - b.kind || a.reach - b.reach || mention.indexOf(a.name) - mention.indexOf(b.name))[0]
      ?? plan(mention.find((n) => pending.has(n)) ?? [...pending][0]!, true)!;
    pending.delete(p.name);
    const fits = (k: number) => !placed.some((q) => q.lane === k && q.span[0] < p.span[1] && p.span[0] < q.span[1]);
    // Its arrows up and down mustn't run through a box on a lane in between, nor its boxes sit across the arrows of a
    // branch already placed further down.
    const clear = (k: number) =>
      p.verticals.every((v) => placed.every((q) => !(q.lane > v.from && q.lane < k) || q.boxes.every((id) => v.x < left.get(id)! - 12 || v.x > right(id) + 12))) &&
      placed.every((q) => q.lane <= k || q.verticals.every((v) => v.from >= k || p.xs.every((bx, i) => v.x < bx - 12 || v.x > bx + w(p.ids[i]!) + 12)));
    let k = p.minLane;
    while (k < p.minLane + 12 && !(fits(k) && clear(k))) k++;
    if (k >= p.minLane + 12) for (k = p.minLane; !fits(k); k++);
    p.ids.forEach((id, i) => {
      left.set(id, p.xs[i]!);
      lane.set(id, k);
    });
    placed.push({ name: p.name, lane: k, span: p.span, verticals: p.verticals, boxes: p.ids, root: p.root });
    rootOf.set(p.name, p.root);
  }

  // Lanes top to bottom, each as tall as its tallest box, the boxes on it centred on one line. Empty lanes close up.
  const used = [...new Set([...lane.values()])].sort((a, b) => a - b);
  const tops = new Map<number, number>();
  const bottoms = new Map<number, number>();
  let top = 0;
  for (const k of used) {
    const tall = Math.max(...[...lane].filter(([, l]) => l === k).map(([id]) => h(id)));
    tops.set(k, top);
    bottoms.set(k, top + tall);
    top += tall + LANE_GAP;
  }
  const nodes: Record<string, Box> = {};
  for (const n of map.nodes) {
    const k = lane.get(n.id)!;
    nodes[n.id] = { x: left.get(n.id)!, y: tops.get(k)! + (bottoms.get(k)! - tops.get(k)! - h(n.id)) / 2, w: w(n.id), h: h(n.id) };
  }

  // Where an arrow meets the top or bottom of a box: a choice is a diamond, so off its middle the edge is higher up.
  const kindOf = new Map(map.nodes.map((n) => [n.id, n.kind]));
  const edgeY = (id: string, px: number, side: 'top' | 'bottom') => {
    const b = nodes[id]!;
    const inset = kindOf.get(id) === 'choice' ? Math.min(b.h / 2, (Math.abs(px - (b.x + b.w / 2)) * b.h) / b.w) : 0;
    return side === 'top' ? b.y + inset : b.y + b.h - inset;
  };
  const mid = (id: string) => nodes[id]!.y + nodes[id]!.h / 2;
  const laneOf = (id: string) => lane.get(id)!;
  const lineOf = (k: number) => [...lane].filter(([, l]) => l === k).map(([id]) => id).sort((a, b) => left.get(a)! - left.get(b)!);

  // What each arrow is, and which ends meet which side of a box, so arrows meeting the same side get their own places.
  type Shape = 'straight' | 'arc' | 'under' | 'down' | 'rejoin' | 'up' | 'return';
  const shapes = new Map<string, Shape>();
  const ends = new Map<string, { link: string; key: number[] }[]>(); // `${node}:t` or `${node}:b`
  const end = (node: string, side: 't' | 'b', link: string, key: number[]) => ends.set(`${node}:${side}`, [...(ends.get(`${node}:${side}`) ?? []), { link, key }]);
  for (const l of all) {
    const la = laneOf(l.from);
    const lb = laneOf(l.to);
    let shape: Shape;
    if (la === lb) {
      const line = lineOf(la);
      shape = line.indexOf(l.to) === line.indexOf(l.from) + 1 ? 'straight' : la === 0 ? 'arc' : 'under';
    } else if (lb > la) shape = 'down';
    else shape = cx(l.to) > right(l.from) + 24 ? 'rejoin' : cx(l.to) < left.get(l.from)! - 24 ? 'return' : 'up';
    shapes.set(l.id, shape);
    if (shape === 'arc') {
      end(l.from, 't', l.id, [cx(l.to)]);
      end(l.to, 't', l.id, [cx(l.from)]);
    }
    // Along a box's bottom, left to right: arrows coming up from the left (from nearer lanes first), then straight up,
    // then coming back from the right (from further lanes first), then going down (to further lanes first).
    if (shape === 'rejoin' || shape === 'up') end(l.to, 'b', l.id, [0, la]);
    if (shape === 'return') end(l.to, 'b', l.id, [1, -la]);
    if (shape === 'down') end(l.from, 'b', l.id, [2, -lb]);
  }
  const portX = new Map<string, number>(); // `${link}:${node}:${side}`
  for (const [at, list] of ends) {
    const node = at.slice(0, at.lastIndexOf(':'));
    list.sort((a, b) => a.key.reduce((d, v, i) => d || v - (b.key[i] ?? 0), 0));
    list.forEach((e, i) => portX.set(`${e.link}:${at}`, cx(node) + (i - (list.length - 1) / 2) * PORT));
  }
  const port = (link: string, node: string, side: 't' | 'b') => portX.get(`${link}:${node}:${side}`) ?? cx(node);

  // Arrows that skip along the main path run above it, the shortest lowest; arrows within a branch that don't go to the
  // next box, and arrows back from a branch, run below it.
  const arcs = all.filter((l) => shapes.get(l.id) === 'arc').sort((a, b) => Math.abs(cx(a.to) - cx(a.from)) - Math.abs(cx(b.to) - cx(b.from)));
  const arcLevel = new Map(arcs.map((l, i) => [l.id, i]));
  const underCount = new Map<number, number>();
  const routes: Record<string, Route> = {};
  for (const l of all) {
    const a = nodes[l.from]!;
    const b = nodes[l.to]!;
    const la = laneOf(l.from);
    const shape = shapes.get(l.id)!;
    let route: Route;
    if (shape === 'straight') {
      route = { points: [[a.x + a.w, mid(l.from)], [b.x, mid(l.to)]], label: 0 };
    } else if (shape === 'arc') {
      const y = -ARC - 18 * arcLevel.get(l.id)!;
      const sx = port(l.id, l.from, 't');
      const tx = port(l.id, l.to, 't');
      route = { points: [[sx, edgeY(l.from, sx, 'top')], [sx, y], [tx, y], [tx, edgeY(l.to, tx, 'top')]], label: 1 };
    } else if (shape === 'under' || shape === 'return') {
      const n = underCount.get(la) ?? 0;
      underCount.set(la, n + 1);
      const y = bottoms.get(la)! + LANE_RUN + 16 * n;
      const sx = cx(l.from) + (shape === 'under' ? 6 : 0);
      const tx = shape === 'return' ? port(l.id, l.to, 'b') : cx(l.to) - 6;
      route = { points: [[sx, a.y + a.h], [sx, y], [tx, y], [tx, edgeY(l.to, tx, 'bottom')]], label: 1 };
    } else if (shape === 'down') {
      const sx = port(l.id, l.from, 'b');
      const tx = cx(l.to);
      const y = tops.get(laneOf(l.to))! - LANE_RUN;
      route = Math.abs(tx - sx) < 4
        ? { points: [[sx, edgeY(l.from, sx, 'bottom')], [sx, b.y]], label: 0 }
        : { points: [[sx, edgeY(l.from, sx, 'bottom')], [sx, y], [tx, y], [tx, b.y]], label: 1 };
    } else if (shape === 'rejoin') {
      const tx = port(l.id, l.to, 'b');
      // Across to below the step it comes back to, then up; over the top of its line if a box is in the way.
      const blocked = lineOf(la).some((id) => id !== l.from && left.get(id)! < tx && right(id) > a.x + a.w);
      const y = tops.get(la)! - LANE_GAP / 2;
      route = blocked
        ? { points: [[cx(l.from), a.y], [cx(l.from), y], [tx, y], [tx, edgeY(l.to, tx, 'bottom')]], label: 1 }
        : { points: [[a.x + a.w, mid(l.from)], [tx, mid(l.from)], [tx, edgeY(l.to, tx, 'bottom')]], label: tx - (a.x + a.w) >= 150 ? 0 : 1 };
    } else {
      const tx = Math.min(Math.max(port(l.id, l.to, 'b'), a.x + 12), a.x + a.w - 12);
      route = { points: [[tx, a.y], [tx, edgeY(l.to, tx, 'bottom')]], label: 0 };
    }
    routes[l.id] = route;
  }

  // Colours go to branches in the order they leave the main path, so the same journey is coloured the same way each time.
  const named = mention.map((name) => {
    const p = placed.find((q) => q.name === name);
    if (p) return { name, root: p.root };
    // A branch named only on arrows: where its first arrow leaves from.
    const from = map.edges.find((e) => e.branch === name)?.from;
    const fb = from ? branchOf.get(from) : null;
    return { name, root: from && lane.get(from) === 0 ? main.indexOf(from) : fb ? rootOf.get(fb) ?? main.length : main.length };
  });
  const branches = named.sort((a, b) => a.root - b.root || mention.indexOf(a.name) - mention.indexOf(b.name)).map((b) => b.name);

  const boxes = Object.values(nodes);
  const points = Object.values(routes).flatMap((r) => r.points);
  return {
    areas: {},
    nodes,
    width: Math.max(...boxes.map((b) => b.x + b.w), ...points.map((p) => p[0])),
    height: Math.max(...boxes.map((b) => b.y + b.h), ...points.map((p) => p[1])),
    flat: true,
    routes,
    branches,
  };
}

const LANE_COL_GAP = 132;
const LANE_ROW_GAP = 40;
const BAND_GAP = 36;

/**
 * Each box's column in time: how many steps after the start it is, following the arrows. Loops are broken where they
 * turn back (found walking the arrows in the map's own order), so an arrow that goes back to an earlier step is the
 * only kind that points left. A box no arrow reaches starts at column 0.
 */
export function timeColumns(ids: readonly string[], ls: readonly { from: string; to: string }[]): Map<string, number> {
  const out = new Map<string, string[]>();
  const into = new Set<string>();
  for (const l of ls) {
    (out.get(l.from) ?? out.set(l.from, []).get(l.from)!).push(l.to);
    into.add(l.to);
  }
  const state = new Map<string, 'open' | 'done'>();
  const finished: string[] = [];
  const visit = (id: string) => {
    state.set(id, 'open');
    for (const to of out.get(id) ?? []) if (!state.has(to)) visit(to);
    state.set(id, 'done');
    finished.push(id);
  };
  for (const id of [...ids.filter((i) => !into.has(i)), ...ids]) if (!state.has(id)) visit(id);
  const order = finished.reverse();
  const place = new Map(order.map((id, i) => [id, i]));
  const col = new Map(ids.map((id) => [id, 0]));
  for (const id of order) {
    for (const to of out.get(id) ?? []) {
      if ((place.get(to) ?? 0) > (place.get(id) ?? 0)) col.set(to, Math.max(col.get(to) ?? 0, (col.get(id) ?? 0) + 1));
    }
  }
  return col;
}

/**
 * Time left to right, places top to bottom: every area is a lane across the whole map, and a box sits in its area's
 * lane, in its column in time. A flow that comes back to a place it has been reads in order instead of zigzagging.
 */
async function layoutLanes(map: CodeMap, extra: readonly ExtraEdge[]): Promise<Layout> {
  const col = timeColumns(map.nodes.map((n) => n.id), links(map, extra));
  const cols = Math.max(0, ...col.values()) + 1;
  const width = 2 * PAD.side + cols * NODE_W + (cols - 1) * LANE_COL_GAP;
  const first = (areaId: string) => Math.min(...map.nodes.filter((n) => n.area === areaId).map((n) => col.get(n.id) ?? 0));
  const lanes = map.areas.filter((a) => map.nodes.some((n) => n.area === a.id)).sort((a, b) => first(a.id) - first(b.id));
  const areas: Record<string, Box> = {};
  const nodes: Record<string, Box> = {};
  let y = 0;
  for (const lane of lanes) {
    const stacked = new Map<number, number>();
    let rows = 1;
    for (const n of map.nodes.filter((m) => m.area === lane.id)) {
      const c = col.get(n.id) ?? 0;
      const row = stacked.get(c) ?? 0;
      stacked.set(c, row + 1);
      rows = Math.max(rows, row + 1);
      nodes[n.id] = { x: PAD.side + c * (NODE_W + LANE_COL_GAP), y: PAD.top + row * (NODE_H + LANE_ROW_GAP), w: NODE_W, h: NODE_H };
    }
    const h = PAD.top + rows * NODE_H + (rows - 1) * LANE_ROW_GAP + PAD.bottom;
    areas[lane.id] = { x: 0, y, w: width, h };
    y += h + BAND_GAP;
  }
  return { areas, nodes, width, height: Math.max(0, y - BAND_GAP), lanes: true };
}

async function layoutWhole(map: CodeMap, extra: readonly ExtraEdge[]): Promise<Layout> {
  const engine = await getElk();
  const areaOf = new Map(map.nodes.map((n) => [n.id, n.area]));
  const all = links(map, extra);

  // Pass 1: each area on its own.
  const nodes: Record<string, Box> = {};
  const sizes: Record<string, { w: number; h: number }> = {};
  for (const area of map.areas) {
    const members = map.nodes.filter((n) => n.area === area.id);
    if (!members.length) continue;
    const inside = all.filter((l) => areaOf.get(l.from) === area.id && areaOf.get(l.to) === area.id);
    const out = await layoutArea(engine, area.id, members, inside);
    Object.assign(nodes, out.nodes);
    sizes[area.id] = out.size;
  }

  // Pass 2: the areas, left to right, joined by the arrows that cross between them (one per pair is enough).
  const pairs = new Map<string, { from: string; to: string }>();
  for (const l of all) {
    const from = areaOf.get(l.from)!;
    const to = areaOf.get(l.to)!;
    if (from !== to && sizes[from] && sizes[to]) pairs.set(`${from}>${to}`, { from, to });
  }
  const out = await engine.layout({
    id: 'root',
    layoutOptions: { ...LAYERED, 'elk.direction': 'RIGHT', 'elk.layered.spacing.nodeNodeBetweenLayers': '120', 'elk.spacing.nodeNode': '48', 'elk.spacing.componentComponent': '64' },
    children: map.areas.filter((a) => sizes[a.id]).map((a) => ({ id: a.id, width: sizes[a.id]!.w, height: sizes[a.id]!.h })),
    edges: [...pairs.values()].map((p, i): ElkExtendedEdge => ({ id: `p${i}`, sources: [p.from], targets: [p.to] })),
  });
  const areas: Record<string, Box> = {};
  for (const c of out.children ?? []) areas[c.id] = { x: c.x ?? 0, y: c.y ?? 0, w: c.width ?? 0, h: c.height ?? 0 };
  return { areas, nodes, width: out.width ?? 0, height: out.height ?? 0 };
}

/** Which sides an arrow leaves and enters by: down within an area, across between areas side by side. */
export function sides(map: CodeMap, layout: Layout, from: string, to: string): [Side, Side] {
  if (layout.flat) {
    const mid = (id: string) => {
      const n = layout.nodes[id];
      return n ? { x: n.x + n.w / 2, y: n.y + n.h / 2 } : { x: 0, y: 0 };
    };
    const dx = mid(to).x - mid(from).x;
    const dy = mid(to).y - mid(from).y;
    // Across, the way a journey reads; up or down only between boxes stacked in the same column.
    if (Math.abs(dx) < 40) return dy >= 0 ? ['b', 't'] : ['t', 'b'];
    // Between rows of a wrapped journey: out of the bottom and in at the top, so the arrow can run along the gap
    // between the rows.
    const rows = rowOf(layout, to) - rowOf(layout, from);
    // Going back up it leaves by the side, since another box of its own row may sit right above it.
    if (rows) return rows > 0 ? ['b', 't'] : ['r', 'b'];
    return dx >= 0 ? ['r', 'l'] : ['l', 'r'];
  }
  const areaOf = (id: string) => map.nodes.find((n) => n.id === id)?.area ?? '';
  const abs = (id: string) => {
    const a = layout.areas[areaOf(id)];
    const n = layout.nodes[id];
    return a && n ? { x: a.x + n.x + n.w / 2, y: a.y + n.y + n.h / 2 } : { x: 0, y: 0 };
  };
  const p = abs(from);
  const q = abs(to);
  const dx = q.x - p.x;
  const dy = q.y - p.y;
  // In lanes, boxes of one area sit side by side in time, so only boxes in the same column are joined top to bottom.
  if ((!layout.lanes && areaOf(from) === areaOf(to)) || Math.abs(dx) < NODE_W * 0.75) return dy >= 0 ? ['b', 't'] : ['t', 'b'];
  return dx >= 0 ? ['r', 'l'] : ['l', 'r'];
}

/** A key that changes only when the layout would: which boxes exist, where they live, and what connects them. */
export function layoutKey(map: CodeMap, extra: readonly ExtraEdge[] = [], added: readonly string[] = [], pictured: readonly string[] = []): string {
  return JSON.stringify([
    map.view ?? null,
    map.view === 'journey' ? [map.nodes.map((n) => n.kind), [...pictured].sort()] : null,
    // Ways through: a journey places its branches by them, and every map's key names them.
    [...map.nodes, ...map.edges].map((x) => x.branch ?? ''),
    [...added].sort(),
    map.areas.map((a) => a.id),
    map.nodes.map((n) => `${n.id}@${n.area}`),
    map.edges.map((e) => `${e.from}>${e.to}`),
    map.absences.map((a) => (a.between ? `${a.between.from}~${a.between.to}` : '')),
    extra.map((x) => `${x.from}+${x.to}`),
  ]);
}
