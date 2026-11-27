// Seeing a proposal on its own. A map with proposals can be shown as the code is now, as it would be with the change,
// or both at once (the change drawn on top, as before). When the change has options (ways through, `branch`), each
// option can be shown on its own too. Like Before / After on a map of a change, nothing moves: what isn't part of the
// version shown stays on the map as a shadow, so flipping between versions shows what comes and goes.
import type { CodeMap } from './types';

/** Which version of a proposal is shown: `both` (the change on top), `now`, `proposed`, or one option (`option:<name>`). */
export type Plan = 'both' | 'now' | 'proposed' | `option:${string}`;
type Pending = { kind: 'add' | 'change' | 'remove'; state: 'pending' | 'done' } | null | undefined;
/** A shadow in a proposal's views: built only with the change, part of another option, or gone with the change. */
export type PlanGhost = 'unbuilt' | 'other' | 'dropped';

/**
 * The versions a map's proposal can be shown in, in the order the switch offers them. Only a map with something still
 * proposed (pending) has any. With two or more options (pending proposals on different ways through), each option is a
 * version of its own instead of one "with the change".
 */
export function planViews(map: CodeMap, pending: (kind: 'node' | 'edge', id: string) => boolean): Plan[] {
  const named: string[] = [];
  let any = false;
  for (const n of map.nodes) if (pending('node', n.id)) { any = true; if (n.branch && !named.includes(n.branch)) named.push(n.branch); }
  for (const e of map.edges) if (pending('edge', e.id)) { any = true; if (e.branch && !named.includes(e.branch)) named.push(e.branch); }
  if (!any) return [];
  return ['now', ...(named.length >= 2 ? named.map((n) => `option:${n}` as const) : ['proposed' as const]), 'both'];
}

/** Whether a proposal on this way through (none: shared by every option) is part of the version shown. */
function applies(plan: Plan, branch: string | null | undefined): boolean {
  if (plan === 'proposed') return true;
  if (plan.startsWith('option:')) return !branch || branch === plan.slice('option:'.length);
  return false;
}

/** Whether a box or arrow is a shadow in this version: an addition not built in it, or a removal it has made. */
export function planGhost(plan: Plan, proposal: Pending, branch: string | null | undefined): PlanGhost | null {
  if (plan === 'both' || !proposal || proposal.state !== 'pending') return null;
  if (proposal.kind === 'add') return applies(plan, branch) ? null : plan.startsWith('option:') ? 'other' : 'unbuilt';
  if (proposal.kind === 'remove') return applies(plan, branch) ? 'dropped' : null;
  return null;
}

/**
 * Whether a box or arrow still carries its proposal's look in this version. In the change (or its option) what is new
 * or different stays marked, so it can be told from what was there already; as the code is now, nothing is proposed.
 */
export function planMarks(plan: Plan, proposal: Pending, branch: string | null | undefined): boolean {
  if (!proposal) return false;
  if (plan === 'both' || proposal.state !== 'pending') return true;
  return applies(plan, branch) && !planGhost(plan, proposal, branch);
}

/** What the switch says for each version. */
export function planLabel(plan: Plan): { label: string; title: string; heading: string } {
  if (plan === 'now') return { label: 'Now', title: 'The code as it is; what is proposed is a shadow', heading: 'The code as it is' };
  if (plan === 'proposed') return { label: 'With the change', title: 'The code as it would be; what would go is a shadow', heading: 'With the change' };
  if (plan === 'both') return { label: 'Both', title: 'The code as it is, with the change drawn on top', heading: 'The change on top of the code' };
  const name = plan.slice('option:'.length);
  return { label: name, title: `The code as it would be with ${name}; the other options are shadows`, heading: `With ${name}` };
}
