// Code in the conversation: the function behind a selected box or arrow with its cited lines marked, and the lines an
// answer points at with its notes beside them. The server reads and checks it each time (core/src/code.ts); nothing
// here decides what is real. Each block links out to the person's own editor at that line.
import { useEffect, useMemo, useRef, useState } from 'react';
import { api } from './api';
import { EDITORS, editorUrl, type EditorId } from './editor';
import { highlightLines, languageOf } from './highlight';
import type { CodeBlock, CodeView } from './types';

const cls = (...xs: (string | false | null | undefined)[]) => xs.filter(Boolean).join(' ');

export const stored = (key: string): string | null => {
  try { return localStorage.getItem(key); } catch { return null; }
};
export const store = (key: string, value: string) => {
  try { localStorage.setItem(key, value); } catch { /* private mode: the choice lasts for this page */ }
};

function useEditor(): [EditorId, (id: EditorId) => void] {
  const [editor, setEditor] = useState<EditorId>(() => EDITORS.find((e) => e.id === stored('nonio-editor'))?.id ?? 'vscode');
  return [editor, (id) => { store('nonio-editor', id); setEditor(id); }];
}

type Loaded = (CodeView & { root: string }) | null;
/** `undefined` while loading, `null` when there is nothing to show. `stamp` changes when the code may have. */
function useCode(load: () => Promise<Loaded>, deps: readonly unknown[]): Loaded | undefined {
  const [view, setView] = useState<Loaded | undefined>(undefined);
  useEffect(() => {
    let live = true;
    load().then((v) => live && setView(v), () => live && setView(null));
    return () => { live = false; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);
  return view;
}

const at = (file: string, line: number) => `${file}:${line}`;

function Block({ block, root, editor, before }: { block: CodeBlock; root: string; editor: EditorId; before: boolean }) {
  const scroller = useRef<HTMLDivElement>(null);
  // Start with the first cited line in view.
  useEffect(() => {
    const el = scroller.current;
    const first = el?.querySelector<HTMLElement>('.code-line.cited');
    // Only when it would otherwise be out of sight: a short block stays at its top.
    if (el && first && first.offsetTop - el.offsetTop + first.offsetHeight > el.clientHeight) el.scrollTop = Math.max(0, first.offsetTop - el.offsetTop - 48);
  }, [block.file, block.start, block.cited.join(',')]);
  const pieces = useMemo(() => highlightLines(block.lines, languageOf(block.file)), [block.file, block.lines.join('\n')]);
  const cited = new Set(block.cited);
  const notes = new Map(block.notes.map((n) => [n.line, n.note]));
  const open = block.cited[0] ?? block.start;
  const editorName = EDITORS.find((e) => e.id === editor)!.name;
  return (
    <figure className="code-block">
      <figcaption className="code-cap">
        <span className="code-where">
          {block.name && <strong className="code-name">{block.name}</strong>}
          <span className="code-file">{block.file}{!block.whole && `:${block.start}`}</span>
        </span>
        {!before && <a className="code-open" href={editorUrl(editor, root, block.file, open)} title={`Open ${block.file} at line ${open} in ${editorName}`}>Open in {editorName}</a>}
      </figcaption>
      <div className="code-lines" ref={scroller} tabIndex={0} aria-label={`Code from ${block.file}`}>
        <div className="code-inner">
          {block.lines.map((text, i) => {
            const n = block.start + i;
            return (
              <div key={n} data-at={at(block.file, n)} className={cls('code-line', cited.has(n) && 'cited', notes.has(n) && 'noted')}>
                <span className="code-n">{n}</span>
                <span className="code-t">{text ? (pieces[i] ?? []).map((p, j) => (p.kind ? <span key={j} className={p.kind}>{p.text}</span> : p.text)) : ' '}</span>
                {notes.has(n) && <span className="code-note">{notes.get(n)}</span>}
              </div>
            );
          })}
        </div>
      </div>
    </figure>
  );
}

function Blocks({ view, jump, before = false, what }: { view: CodeView & { root: string }; jump?: { at: string; n: number } | null; before?: boolean; what: string }) {
  const [editor, setEditor] = useEditor();
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!jump) return;
    const el = [...(box.current?.querySelectorAll<HTMLElement>('.code-line') ?? [])].find((x) => x.dataset.at === jump.at);
    if (!el) return;
    el.scrollIntoView({ block: 'center', behavior: 'smooth' });
    el.classList.add('flash');
    const t = setTimeout(() => el.classList.remove('flash'), 1200);
    return () => clearTimeout(t);
  }, [jump?.at, jump?.n, view]);
  return (
    <div className="code-view" ref={box}>
      {view.blocks.map((b) => <Block key={`${b.file}:${b.start}`} block={b} root={view.root} editor={editor} before={before} />)}
      {view.missing > 0 && (
        <p className="warn small code-missing">
          {view.missing === 1 ? `1 ${what} isn’t` : `${view.missing} ${what}s aren’t`} in {before ? 'that version of the code' : 'the code any more'}, so {view.missing === 1 ? 'it isn’t' : 'they aren’t'} shown.
        </p>
      )}
      {!before && view.blocks.length > 0 && (
        <label className="code-editor">
          Links open in{' '}
          <select value={editor} onChange={(e) => setEditor(e.target.value as EditorId)} aria-label="Which editor code links open in">
            {EDITORS.map((e) => <option key={e.id} value={e.id}>{e.name}</option>)}
          </select>
        </label>
      )}
    </div>
  );
}

/**
 * The code behind the selected box or arrow. `change` is how it differs on a map of a change: a changed one can be
 * read as it was, and a removed one only as it was. `jump` scrolls to a cited line (a click on its chip).
 */
export function ItemCode({ mapId, itemId, change, since, stamp, jump }: { mapId: string; itemId: string; change: 'added' | 'changed' | 'removed' | null; since: string; stamp: unknown; jump: { at: string; n: number } | null }) {
  const [when, setWhen] = useState<'now' | 'before'>(change === 'removed' ? 'before' : 'now');
  const view = useCode(() => api.code(mapId, itemId, when), [mapId, itemId, when, stamp]);
  const toggle = change === 'changed' && (
    <div className="seg code-when" role="group" aria-label="Which version of the code to show">
      <button className={cls('seg-btn', when === 'before' && 'on')} aria-pressed={when === 'before'} onClick={() => setWhen('before')}>As it was at {since}</button>
      <button className={cls('seg-btn', when === 'now' && 'on')} aria-pressed={when === 'now'} onClick={() => setWhen('now')}>Now</button>
    </div>
  );
  if (view === undefined) return toggle || null;
  if (!view || (!view.blocks.length && !view.missing)) return toggle ? <>{toggle}<p className="muted small">No code to show for that version.</p></> : null;
  return <>{toggle}<Blocks view={view} jump={jump} before={when === 'before'} what="cited line" /></>;
}

/** The lines an answer points at, with its notes beside them. Nothing when it has none. */
export function AnswerCode({ mapId, answerId, stamp }: { mapId: string; answerId: string; stamp: unknown }) {
  const view = useCode(() => api.answerCode(mapId, answerId), [mapId, answerId, stamp]);
  if (!view || (!view.blocks.length && !view.missing)) return null;
  return <Blocks view={view} what="note" />;
}

/** Whether code is shown under a selected box: off until the person opens it, which keeps the bubble small (remembered). */
export const useCodeShown = () => useShown('nonio-code');
/** Whether a changed box's line-by-line diff is shown in its bubble: off until opened, like the code (remembered). */
export const useDiffShown = () => useShown('nonio-diff');

function useShown(key: string): [boolean, (on: boolean) => void] {
  const [on, setOn] = useState(() => stored(key) === 'show');
  return [on, (v) => { store(key, v ? 'show' : 'hide'); setOn(v); }];
}
