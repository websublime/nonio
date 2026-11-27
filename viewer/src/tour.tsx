// The walkthrough, as a bubble on the map: the stop's text and the buttons to move through it sit together, next to
// the boxes it lights up. The camera leaves room for it (see `TOUR_ROOM` in graph.tsx). It shows no picture of a
// page: the boxes it lights up carry theirs on the map, and selecting one shows it full size in the box's bubble, so a
// picture here only covered the map.
import { Markdown } from './markdown';
import type { CodeMap } from './types';

const cls = (...xs: (string | false | null | undefined)[]) => xs.filter(Boolean).join(' ');

export type TourBubbleProps = {
  map: CodeMap;
  stop: number;
  onTour: (stop: number | null) => void;
  /** Put this stop in the question box, as a chip (left out when questions are asked in the agent's chat). */
  onAsk?: () => void;
};

export function TourBubble({ map, stop: i, onTour, onAsk }: TourBubbleProps) {
  const stop = map.tour[i];
  if (!stop) return null;
  const last = i === map.tour.length - 1;
  return (
    <section className="card card-focus tour-bubble" aria-label="Walkthrough" aria-live="polite">
      <button className="card-close" aria-label="Leave the walkthrough" title="Leave the walkthrough (Esc)" onClick={() => onTour(null)}>×</button>
      <div className="tour-head">
        <span className="eyebrow">Walkthrough, stop {i + 1} of {map.tour.length}</span>
        <span className="dots" aria-hidden="true">
          {map.tour.map((t, n) => <i key={t.id} className={cls(n === i && 'on', n < i && 'done')} />)}
        </span>
      </div>
      <h2 className="card-title">{stop.title}</h2>
      <Markdown text={stop.text} className="depth-text" />
      {stop.terms?.length ? <p className="muted small">Terms: {stop.terms.join(', ')}</p> : null}
      <div className="tour-nav">
        <button className="btn" disabled={i === 0} onClick={() => onTour(i - 1)}>‹ Back</button>
        {onAsk && <button className="btn" onClick={onAsk}>Ask about this step</button>}
        <span className="grow" />
        {last
          ? <button className="btn primary" onClick={() => onTour(null)}>Done</button>
          : <button className="btn primary" onClick={() => onTour(i + 1)}>Next ›</button>}
      </div>
      <div className="kbd">← → to step, Esc to leave</div>
    </section>
  );
}
