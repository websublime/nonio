// Where a question about the map is asked. With Claude Code (and the CLI) it is the map's own question box. In Codex
// the person asks in the Codex chat beside this page, so the box says so, and the page tells nonio what is
// selected (and the walkthrough stop on screen) for the question to be about.
import type { FixState, Sel } from './state';
import type { Answer, PageAgent } from './types';

export const DEFAULT_AGENT: PageAgent = { name: 'Claude', asks: 'page' };

export const asksInChat = (agent: PageAgent): boolean => agent.asks === 'chat';

/** What the question box says in place of taking a question, when questions are asked in the agent's chat. */
export function chatHint(agent: PageAgent, selected: number, step: string | null): { title: string; detail: string } {
  return {
    title: `Ask in the ${agent.name} chat beside this page`,
    detail: selected
      ? `${agent.name} will see what you selected here.`
      : step
        ? `${agent.name} will know you're on the step “${step}”.`
        : `${agent.name} will see what you select here.`,
  };
}

/** What the page tells nonio about itself, or null when there's no need (questions are typed on the page). */
export function selectionReport(agent: PageAgent, selection: Sel, step: string | null): { selection: Sel; step: string | null } | null {
  return asksInChat(agent) ? { selection, step } : null;
}

/**
 * Whether an answer still needs the conversation beside the map when the reply is in the agent's chat (Codex): only
 * when there is something to do on the page, a fix to approve or undo, or the offer of a new map to take up.
 */
export function needsPage(a: Answer, fixes: Record<string, FixState>): boolean {
  if (a.offer?.status === 'pending') return true;
  const f = a.fix ? fixes[a.fix.id] : undefined;
  return !!f && (f.status === 'proposed' || f.status === 'applying' || f.status === 'failed' || (f.status === 'applied' && f.undoable));
}

/** The answers the conversation beside the map shows: all of them, or in the agent's chat only those needing the page. */
export function answersOnPage(agent: PageAgent, answers: Answer[], fixes: Record<string, FixState>): Answer[] {
  return asksInChat(agent) ? answers.filter((a) => needsPage(a, fixes)) : answers;
}
