// The input bar floating over the map. What's selected rides along as chips, and so does the walkthrough stop on
// screen; Enter sends, Shift+Enter adds a line.
import { useEffect, useRef } from 'react';
import { chatHint } from './asking';
import { nameOf } from './panel';
import type { Sel } from './state';
import type { CodeMap, PageAgent } from './types';

export type ComposerProps = {
  map: CodeMap;
  selection: Sel;
  draft: string;
  busy: boolean;
  /** Who answers, named while they work. */
  agentName?: string;
  /** Focus the box when it appears (it moves into the conversation when that opens). */
  autoFocus?: boolean;
  onDraft: (text: string) => void;
  onRemove: (kind: keyof Sel, id: string) => void;
  /** The walkthrough stop the question will be about, if one is on screen and its chip wasn't removed. */
  step?: string | null;
  onRemoveStep?: () => void;
  onSend: () => void;
};

export function Composer({ map, selection, draft, busy, agentName = 'Claude', autoFocus, onDraft, onRemove, step, onRemoveStep, onSend }: ComposerProps) {
  const ref = useRef<HTMLTextAreaElement>(null);
  const chips = (['nodes', 'edges', 'absences'] as const).flatMap((kind) => selection[kind].map((id) => ({ kind, id })));

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = `${Math.min(el.scrollHeight, 140)}px`;
  }, [draft]);
  useEffect(() => {
    if (draft) ref.current?.focus();
  }, [draft === '']);
  useEffect(() => {
    if (autoFocus) ref.current?.focus({ preventScroll: true });
    // Only when it first appears.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <form
      className="composer"
      onSubmit={(e) => {
        e.preventDefault();
        if (!busy && draft.trim()) onSend();
      }}
    >
      {step && (
        <span className="sel-chip sel-step">
          <span className="sel-text" title={`Walkthrough step: ${step}`}>▸ {step}</span>
          <button type="button" aria-label={`Don't ask about the step “${step}”`} onClick={onRemoveStep}>×</button>
        </span>
      )}
      {chips.map((c) => (
        <span key={`${c.kind}:${c.id}`} className={`sel-chip sel-${c.kind}`}>
          <span className="sel-text" title={nameOf(map, c.id)}>
            {c.kind === 'absences' ? '✕ ' : '◉ '}
            {nameOf(map, c.id)}
          </span>
          <button type="button" aria-label={`Remove ${nameOf(map, c.id)}`} onClick={() => onRemove(c.kind, c.id)}>×</button>
        </span>
      ))}
      <textarea
        ref={ref}
        id="ask"
        rows={1}
        value={draft}
        placeholder={busy ? `${agentName} is working…` : chips.length === 1 && chips[0]!.kind === 'nodes' ? 'Ask, or try “what if it fails?”' : chips.length ? 'Ask about what you selected…' : step ? 'Ask about this step…' : 'Ask about this map…'}
        onChange={(e) => onDraft(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && !e.shiftKey) {
            e.preventDefault();
            if (!busy && draft.trim()) onSend();
          }
        }}
        aria-label="Ask a question about the map"
      />
      <button type="submit" className="send" disabled={busy || !draft.trim()} aria-label="Send">↑</button>
    </form>
  );
}

/**
 * In place of the question box when questions are asked in the agent's chat (Codex): what is selected, still
 * removable, and where to ask. The page tells nonio what is selected, so the agent's answer is about it.
 */
export function ChatAsk({ map, agent, selection, step, onRemove }: { map: CodeMap; agent: PageAgent; selection: Sel; step: string | null; onRemove: (kind: keyof Sel, id: string) => void }) {
  const chips = (['nodes', 'edges', 'absences'] as const).flatMap((kind) => selection[kind].map((id) => ({ kind, id })));
  const hint = chatHint(agent, chips.length, step);
  return (
    <div className="composer composer-chat" role="note" aria-label="Where to ask">
      {chips.map((c) => (
        <span key={`${c.kind}:${c.id}`} className={`sel-chip sel-${c.kind}`}>
          <span className="sel-text" title={nameOf(map, c.id)}>
            {c.kind === 'absences' ? '✕ ' : '◉ '}
            {nameOf(map, c.id)}
          </span>
          <button type="button" aria-label={`Remove ${nameOf(map, c.id)}`} onClick={() => onRemove(c.kind, c.id)}>×</button>
        </span>
      ))}
      <p className="chat-hint"><strong>{hint.title}</strong> <span className="muted">{hint.detail}</span></p>
    </div>
  );
}
