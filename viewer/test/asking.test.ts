// Where questions are asked: in the map's own box, or (Codex) in the agent's chat beside the page.
import { describe, expect, it } from 'vitest';
import { DEFAULT_AGENT, answersOnPage, asksInChat, chatHint, needsPage, selectionReport } from '../src/asking';
import { emptySel, type FixState } from '../src/state';
import type { Answer } from '../src/types';

const codex = { name: 'Codex', asks: 'chat' as const };

describe('asking about the map', () => {
  it('takes questions on the page unless the agent asks in its chat', () => {
    expect(asksInChat(DEFAULT_AGENT)).toBe(false);
    expect(asksInChat(codex)).toBe(true);
  });

  it('points to the chat, and says what the agent will see', () => {
    expect(chatHint(codex, 0, null)).toEqual({ title: 'Ask in the Codex chat beside this page', detail: 'Codex will see what you select here.' });
    expect(chatHint(codex, 2, null).detail).toBe('Codex will see what you selected here.');
    expect(chatHint(codex, 0, 'The announcement').detail).toBe('Codex will know you\'re on the step “The announcement”.');
  });

  it('reports the selection only when questions are asked in the chat', () => {
    const sel = { ...emptySel(), nodes: ['s4'] };
    expect(selectionReport(DEFAULT_AGENT, sel, null)).toBeNull();
    expect(selectionReport(codex, sel, 'Step')).toEqual({ selection: sel, step: 'Step' });
  });
});

describe('the conversation beside the map, when the reply is in the chat', () => {
  const base = { id: 'a1', question: 'q', selection: {}, concept: 'c', refs: [], moves: { focus: null, highlight: [] }, added: [], problems: [], fix: null, source: 'map', cost: 0 } as unknown as Answer;
  const fix = { id: 'f1' } as Answer['fix'];
  const state = (status: FixState['status'], undoable = true): Record<string, FixState> => ({ f1: { proposal: fix!, answerId: 'a2', status, problems: [], edgeId: null, closed: [], undoable } });

  it('keeps only answers with something to do on the page', () => {
    const plain = base;
    const withFix = { ...base, id: 'a2', fix };
    const withOffer = { ...base, id: 'a3', offer: { question: 'q2', existing: null, status: 'pending', mapId: null } } as Answer;
    expect(answersOnPage(codex, [plain, withFix, withOffer], state('proposed')).map((a) => a.id)).toEqual(['a2', 'a3']);
    expect(needsPage(withFix, state('rejected'))).toBe(false);
    expect(needsPage(withFix, state('applied'))).toBe(true); // it can still be undone
    expect(needsPage(withFix, state('applied', false))).toBe(false);
  });

  it('keeps them all when questions are asked on the page', () => {
    expect(answersOnPage({ name: 'Claude', asks: 'page' }, [base], {})).toHaveLength(1);
  });
});
