// The dock floating along the bottom of the map: the way into the walkthrough, and the question box while the
// conversation is closed (once it's open, the box sits at the bottom of the conversation instead). Zoom is the mouse
// wheel or trackpad. While the walkthrough runs, its controls are in the bubble beside its text (tour.tsx).
import { useEffect, useState, type ReactNode } from 'react';
import { store, stored } from './code';
import { parseToured, TOURED_KEY, withToured } from './toured';
import type { CodeMap } from './types';

export type DockProps = {
  map: CodeMap;
  /** Which map this is, to remember that its walkthrough was taken. */
  mapId: string;
  /** The walkthrough stop on screen, or null when the walkthrough isn't running. */
  tour: number | null;
  onTour: (stop: number | null) => void;
  /** How many boxes, arrows and gaps are selected (the hint changes once something is). */
  selected: number;
  /** The question box (the composer), when it belongs here rather than in the conversation. */
  children?: ReactNode;
};

// Whether the walkthrough of this map has been taken. People who missed it tried to read the map alone and found it
// confusing, so the button glows softly until they've started it once (however they started it), then stays quiet.
function useToured(mapId: string, touring: boolean): boolean {
  const [toured, setToured] = useState(() => parseToured(stored(TOURED_KEY)).includes(mapId));
  useEffect(() => {
    if (!touring || toured || !mapId) return;
    // Read the list again rather than keep it, so another tab's maps aren't lost.
    store(TOURED_KEY, JSON.stringify(withToured(parseToured(stored(TOURED_KEY)), mapId)));
    setToured(true);
  }, [mapId, touring, toured]);
  return toured;
}

export function Dock({ map, mapId, tour, onTour, selected, children }: DockProps) {
  // The dock is only on a map's page, so the address names the map.
  const toured = useToured(mapId, tour !== null);
  const stops = map.tour.length;
  const walk = stops > 0 && tour === null;
  if (!walk && !children) return null;
  return (
    <div className="dock">
      {walk && (
        <section className="dock-part dock-walk" aria-label="Walkthrough">
          <h2 className="dock-title">Walkthrough</h2>
          <button className={toured ? 'dock-start' : 'dock-start glow'} onClick={() => onTour(0)}>
            Take the walkthrough <span className="muted">{stops} {stops === 1 ? 'stop' : 'stops'}</span>
          </button>
        </section>
      )}
      {children && (
        <section className="dock-part dock-ask" aria-label="Ask about the map">
          <div className="dock-head">
            <h2 className="dock-title">Ask about the map</h2>
            <span className="dock-hint">{selected ? 'Shift-click to add more' : 'Click a box or arrow to ask about it'}</span>
          </div>
          {children}
        </section>
      )}
    </div>
  );
}
