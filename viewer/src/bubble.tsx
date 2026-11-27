// What a selected box or arrow is, in a bubble on the map beside it, so the side panel stays the conversation. It is
// placed in screen space next to the box (or the arrow's label), on whichever side has room, and follows it as the map
// pans and zooms while staying the same size to read. If it can't fit beside the box, the camera moves over once.
// A grip on its far bottom corner resizes it (width, and the height it may grow to before it scrolls); the size is
// remembered for every bubble, and a double-click on the grip goes back to the default.
import { useLayoutEffect, useRef, useState, type CSSProperties, type PointerEvent as ReactPointerEvent } from 'react';
import { useReactFlow, useStore } from '@xyflow/react';
import { store, stored } from './code';
import { Selected } from './panel';
import type { Sel } from './state';
import type { CheckReport, CodeMap } from './types';

const GAP = 14; // between the box and the bubble
const EDGE = 12; // kept clear at the edges of the map area

/** The element on the map a selection points at: the box, or the arrow's or gap's label. */
function anchorOf(canvas: HTMLElement, sel: Sel): Element | null {
  const last = (xs: string[]) => xs[xs.length - 1];
  const node = last(sel.nodes);
  const edge = last(sel.edges);
  const gap = last(sel.absences);
  const q = node !== undefined ? `.react-flow__node[data-id="${CSS.escape(node)}"]`
    : edge !== undefined ? `.link-label[data-id="${CSS.escape(`e:${edge}`)}"]`
    : gap !== undefined ? `.link-label[data-id="${CSS.escape(`a:${gap}`)}"]`
    : null;
  return q ? canvas.querySelector(q) : null;
}

const SIZE_KEY = 'nonio-bubble-size';
const MIN_W = 300;
const MIN_H = 180;
type Size = { w: number; h: number };

function storedSize(): Size | null {
  try {
    const v: unknown = JSON.parse(stored(SIZE_KEY) ?? 'null');
    if (v && typeof v === 'object' && 'w' in v && 'h' in v && typeof v.w === 'number' && typeof v.h === 'number') {
      return { w: Math.max(MIN_W, v.w), h: Math.max(MIN_H, v.h) };
    }
  } catch { /* not ours, or broken: use the default */ }
  return null;
}

type Place = { left: number; top: number; side: 'left' | 'right'; pointer: number; maxHeight: number };

export function SelectionBubble({ mapId, map, report, selection, onClose }: {
  mapId: string;
  map: CodeMap;
  report: CheckReport | null;
  selection: Sel;
  onClose: () => void;
}) {
  const ids = [...selection.nodes, ...selection.edges, ...selection.absences];
  const key = ids.join(' ');
  const ref = useRef<HTMLDivElement>(null);
  const [place, setPlace] = useState<Place | null>(null);
  const nudged = useRef<string | null>(null);
  const rf = useReactFlow();
  // Re-place whenever the camera moves.
  const transform = useStore((st) => st.transform.join(','));
  const [size, setSize] = useState('');
  const [chosen, setChosen] = useState<Size | null>(storedSize);

  // Resizing from the far bottom corner: the side facing the box stays where it is.
  const onGrip = (e: ReactPointerEvent<HTMLElement>) => {
    const el = ref.current;
    const card = el?.querySelector<HTMLElement>('.card-selected');
    const canvas = el?.parentElement;
    if (!el || !card || !canvas || !place) return;
    e.preventDefault();
    const x0 = e.clientX;
    const y0 = e.clientY;
    const w0 = el.offsetWidth;
    const h0 = card.offsetHeight;
    const maxW = canvas.clientWidth - 2 * EDGE;
    const grows = place.side === 'right' ? 1 : -1;
    let last: Size = { w: w0, h: h0 };
    const move = (ev: PointerEvent) => {
      last = {
        w: Math.round(Math.max(MIN_W, Math.min(maxW, w0 + grows * (ev.clientX - x0)))),
        h: Math.round(Math.max(MIN_H, h0 + ev.clientY - y0)),
      };
      setChosen(last);
    };
    const up = () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
      document.body.classList.remove('resizing-bubble');
      store(SIZE_KEY, JSON.stringify(last));
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
    document.body.classList.add('resizing-bubble');
  };
  const resetSize = () => {
    try { localStorage.removeItem(SIZE_KEY); } catch { /* nothing kept */ }
    setChosen(null);
  };

  // The bubble's own size changes when its code opens or loads.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setSize(`${el.offsetWidth}x${el.offsetHeight}`));
    ro.observe(el);
    return () => ro.disconnect();
  }, [key]);

  useLayoutEffect(() => {
    const el = ref.current;
    const canvas = el?.parentElement;
    if (!el || !canvas) return;
    const anchor = anchorOf(canvas, selection);
    if (!anchor) { setPlace(null); return; }
    const c = canvas.getBoundingClientRect();
    const a = anchor.getBoundingClientRect();
    const r = { left: a.left - c.left, right: a.right - c.left, top: a.top - c.top, bottom: a.bottom - c.top };
    // Keep clear of the dock along the bottom (the question box is how you ask about what's selected).
    const dock = canvas.querySelector('.dock');
    const floor = dock ? dock.getBoundingClientRect().top - c.top - EDGE : c.height - EDGE;
    const maxHeight = Math.max(160, floor - EDGE);
    const w = el.offsetWidth;
    const h = Math.min(el.offsetHeight, maxHeight);
    const roomRight = c.width - EDGE - r.right - GAP;
    const roomLeft = r.left - GAP - EDGE;
    const side: Place['side'] = roomRight >= w || roomRight >= roomLeft ? 'right' : 'left';
    const want = side === 'right' ? r.right + GAP : r.left - GAP - w;
    const left = Math.max(EDGE, Math.min(want, c.width - EDGE - w));
    const top = Math.max(EDGE, Math.min(r.top, floor - h));
    const pointer = Math.max(16, Math.min((r.top + r.bottom) / 2 - top, h - 16));
    setPlace({ left, top, side, pointer, maxHeight });

    // Once per selection: if the box is cut off, or the bubble can't sit beside it without covering it, move the
    // camera so the box and its bubble are both in view (or at least the box, if they can't both fit).
    if (nudged.current === key) return;
    nudged.current = key;
    const from = side === 'right' ? r.left : r.left - GAP - w;
    const to = side === 'right' ? r.right + GAP + w : r.right;
    const fits = to - from <= c.width - 2 * EDGE;
    const lo = fits ? from : r.left;
    const hi = fits ? to : r.right;
    const dx = lo < EDGE ? EDGE - lo : hi > c.width - EDGE ? c.width - EDGE - hi : 0;
    const dy = r.top < EDGE ? EDGE - r.top : r.bottom > floor ? Math.max(floor - r.bottom, EDGE - r.top) : 0;
    if (dx || dy) {
      const vp = rf.getViewport();
      void rf.setViewport({ ...vp, x: vp.x + dx, y: vp.y + dy }, { duration: 300 });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, transform, size]);

  if (!ids.length) return null;
  return (
    <div
      ref={ref}
      className={`box-bubble box-bubble-${place?.side ?? 'right'}`}
      style={{
        ...(place ? { left: place.left, top: place.top, '--bubble-max': `${place.maxHeight}px` } : { visibility: 'hidden', left: 0, top: 0 }),
        ...(chosen ? { width: chosen.w, '--bubble-cap': `${chosen.h}px` } : {}),
      } as CSSProperties}
      role="dialog"
      aria-label="What you selected"
    >
      {place && <i className="box-bubble-pointer" style={{ top: place.pointer }} aria-hidden="true" />}
      <Selected key={key} mapId={mapId} map={map} report={report} ids={ids} onClose={onClose} />
      <span className="box-bubble-grip" onPointerDown={onGrip} onDoubleClick={resetSize} title="Drag to resize. Double-click for the usual size." aria-hidden="true" />
    </div>
  );
}
