// What the person sees at a box: a screenshot nonio took (with the thing the box is about marked), an image file
// from the repo, a wireframe of the whole page with the box's part of it marked, or a rough sketch of one component's
// visible parts. Only screens the checker confirmed get here.
import { useLayoutEffect, useRef, useState } from 'react';
import { screenUrl } from './api';
import type { CheckReport, ScreenShow } from './types';

const cls = (...xs: (string | false | null | undefined)[]) => xs.filter(Boolean).join(' ');
const pct = (n: number) => `${Math.max(0, Math.min(1, n)) * 100}%`;

/** The confirmed screen of a box, if it has one. */
export function screenOf(report: CheckReport | null, id: string): ScreenShow | null {
  const c = report?.screens?.[id];
  return c?.status === 'confirmed' ? c.show : null;
}

type WireframeShow = Extract<ScreenShow, { kind: 'wireframe' }>;
type WireRegion = WireframeShow['page']['regions'][number];

/**
 * A page as a wireframe: its regions in five fixed places (top, left, main, right, bottom), with the region and the
 * element a box is about marked. `small` is the size that sits on a journey box: region names only, plus the marked
 * element, since the point there is where on the page the step happens.
 */
function Wireframe({ show, small = false }: { show: WireframeShow; small?: boolean }) {
  const at = (slot: WireRegion['slot']) => show.page.regions.filter((r) => r.slot === slot);
  const hl = show.highlight?.trim().toLowerCase();
  const region = (r: WireRegion) => {
    const on = r.id === show.region;
    const marked = (e: { text: string }) => on && e.text.trim().toLowerCase() === hl;
    const elements = small ? r.elements.filter(marked) : r.elements;
    return (
      <div key={r.id} className={cls('wf-region', on && 'wf-on')}>
        <span className="wf-label">{r.label}</span>
        <span className="wf-els">
          {elements.map((e, i) => (
            <span key={i} className={cls('sk', `sk-${e.kind}`, marked(e) && 'sk-mark')} title={e.text}>{e.kind === 'image' ? `▧ ${e.text}` : e.text}</span>
          ))}
        </span>
      </div>
    );
  };
  const slot = (name: WireRegion['slot']) => (at(name).length ? <div className={`wf-slot wf-${name}`}>{at(name).map(region)}</div> : null);
  const sides = at('left').length + at('right').length;
  // The middle row: the main area takes most of the width, unless the page has only side regions.
  const columns = [at('left').length && 'minmax(0, 1fr)', (at('main').length || !sides) && 'minmax(0, 2.6fr)', at('right').length && 'minmax(0, 1fr)'].filter(Boolean).join(' ');
  return (
    <div className={cls('wf', small && 'wf-small')}>
      {slot('top')}
      {(sides > 0 || at('main').length > 0) && <div className="wf-mid" style={{ gridTemplateColumns: columns }}>{slot('left')}{slot('main')}{slot('right')}</div>}
      {slot('bottom')}
    </div>
  );
}

export function ScreenView({ show, name }: { show: ScreenShow; name: string }) {
  // A whole screen in a narrow card makes the marked thing tiny, so it starts zoomed in on the mark.
  const [whole, setWhole] = useState(false);
  if (show.kind === 'capture') {
    const box = show.box;
    const zoomed = !!box && !whole;
    const origin = box ? `${pct(box.x + box.w / 2)} ${pct(box.y + box.h / 2)}` : undefined;
    return (
      <figure className="screen">
        <div className="screen-shot" style={{ aspectRatio: `${show.width} / ${show.height}` }}>
          <div className="screen-zoom" style={zoomed ? { transform: 'scale(2.2)', transformOrigin: origin } : undefined}>
            <img src={screenUrl(show)} alt={`The screen at “${name}”${show.title ? `: ${show.title}` : ''}`} loading="lazy" />
            {box && <i className="screen-mark" aria-hidden="true" style={{ left: pct(box.x), top: pct(box.y), width: pct(box.w), height: pct(box.h) }} />}
          </div>
        </div>
        <figcaption className="muted small">
          Screenshot of {show.from}
          {box && <> · <button className="linklike" onClick={() => setWhole(!whole)}>{whole ? 'Zoom in' : 'Show the whole screen'}</button></>}
        </figcaption>
      </figure>
    );
  }
  if (show.kind === 'wireframe') {
    return (
      <figure className="screen">
        <div role="img" aria-label={`Wireframe of ${show.page.name}, the page at “${name}”`}><FitWireframe show={show} /></div>
        <figcaption className="muted small">Wireframe of {show.page.name}, from {show.page.file}. Where things sit is approximate; it isn’t a screenshot.</figcaption>
      </figure>
    );
  }
  if (show.kind === 'image') {
    return (
      <figure className="screen">
        <div className="screen-image"><img src={screenUrl(show)} alt={`Image used at “${name}”`} loading="lazy" /></div>
        <figcaption className="muted small">{show.file}</figcaption>
      </figure>
    );
  }
  const hl = show.highlight?.trim().toLowerCase();
  return (
    <figure className="screen">
      <div className="screen-sketch" role="img" aria-label={`Rough sketch of the screen at “${name}”`}>
        {show.elements.map((e, i) => (
          <span key={i} className={cls('sk', `sk-${e.kind}`, e.proposed && 'sk-proposed', hl === e.text.trim().toLowerCase() && 'sk-mark')} title={e.proposed ? 'Proposed: not on the screen yet' : undefined}>
            {e.kind === 'image' ? `▧ ${e.text}` : e.text}
            {e.proposed && <i className="sk-new">proposed</i>}
          </span>
        ))}
      </div>
      <figcaption className="muted small">
        {show.elements.every((e) => e.proposed) ? `Rough sketch of a proposed screen (${show.file})` : `Rough sketch from ${show.file}, not a screenshot`}
        {show.elements.some((e) => e.proposed) && !show.elements.every((e) => e.proposed) && ' · violet parts are proposed'}
      </figcaption>
    </figure>
  );
}

/** The same picture small enough to sit on a journey box: the page's wireframe, the top of a sketch, or a screenshot zoomed on its mark. */
export function ScreenThumb({ show, name }: { show: ScreenShow; name: string }) {
  if (show.kind === 'sketch') {
    const hl = show.highlight?.trim().toLowerCase();
    const shown = show.elements.slice(0, 4);
    return (
      <div className="thumb thumb-sketch" role="img" aria-label={`Rough sketch of the screen at “${name}”`}>
        {shown.map((e, i) => (
          <span key={i} className={cls('sk', `sk-${e.kind}`, e.proposed && 'sk-proposed', hl === e.text.trim().toLowerCase() && 'sk-mark')}>
            {e.kind === 'image' ? `▧ ${e.text}` : e.text}
          </span>
        ))}
        {show.elements.length > shown.length && <span className="thumb-more">+{show.elements.length - shown.length} more</span>}
      </div>
    );
  }
  if (show.kind === 'wireframe') {
    return <div className="thumb thumb-wire" role="img" aria-label={`Wireframe of ${show.page.name}, the page at “${name}”`}><Wireframe show={show} small /></div>;
  }
  if (show.kind === 'image') {
    return <div className="thumb thumb-image"><img src={screenUrl(show)} alt={`Image used at “${name}”`} loading="lazy" /></div>;
  }
  const box = show.box;
  return (
    <div className="thumb thumb-shot">
      <img
        src={screenUrl(show)}
        alt={`The screen at “${name}”`}
        loading="lazy"
        style={box ? { transform: 'scale(1.8)', transformOrigin: `${pct(box.x + box.w / 2)} ${pct(box.y + box.h / 2)}` } : undefined}
      />
    </div>
  );
}

/** The width a wireframe is laid out at, the shape of a page; it's then scaled to fit where it's shown. */
const WIREFRAME_W = 560;

/**
 * A wireframe laid out at a page's width and scaled down to fit its card, like a thumbnail of the page, rather than
 * drawn at full size: at full size it dwarfed the map beside it (the map is often zoomed out). Drag the bubble wider
 * and it grows, up to full size.
 */
function FitWireframe({ show }: { show: WireframeShow }) {
  const ref = useRef<HTMLDivElement>(null);
  const [scale, setScale] = useState(1);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const fit = () => setScale(Math.min(1, el.clientWidth / WIREFRAME_W));
    fit();
    const ro = new ResizeObserver(fit);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  return (
    <div ref={ref} className="wf-fit">
      <div className="wf-fit-page" style={{ width: WIREFRAME_W, zoom: scale }}><Wireframe show={show} /></div>
    </div>
  );
}
