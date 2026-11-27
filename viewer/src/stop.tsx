// Stopping nonio from the page, and what a tab shows once it has stopped (by the button, Ctrl+C, or on its own).
import { useEffect, useState } from 'react';
import { api } from './api';

// Stopping ends nonio for every tab, and Undo for applied fixes with it, so the button asks first. The question
// goes away by itself if it isn't answered, or on Escape.
const ASK_FOR_MS = 8000;

export function StopButton({ busy, agentName = 'Claude', onStopped, onError }: { busy: boolean; agentName?: string; onStopped: () => void; onError: (message: string) => void }) {
  const [asking, setAsking] = useState(false);

  useEffect(() => {
    if (!asking) return;
    const timer = setTimeout(() => setAsking(false), ASK_FOR_MS);
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') setAsking(false); };
    window.addEventListener('keydown', onKey);
    return () => { clearTimeout(timer); window.removeEventListener('keydown', onKey); };
  }, [asking]);

  if (asking && !busy) {
    return (
      <span className="stop-ask" role="group" aria-label="Stop nonio?">
        <span className="stop-ask-text">Stop nonio? Maps stay saved.</span>
        <button className="pill-btn stop-yes" autoFocus onClick={() => { setAsking(false); api.stop().then(onStopped, (e: Error) => onError(e.message)); }}>
          Stop
        </button>
        <button className="pill-btn" onClick={() => setAsking(false)}>Keep running</button>
      </span>
    );
  }

  return (
    <button
      className="pill-btn"
      disabled={busy}
      title={busy ? `${agentName} is still working` : 'Maps stay saved. Undo for applied fixes ends.'}
      onClick={() => setAsking(true)}
    >
      Stop nonio
    </button>
  );
}

export function Stopped() {
  return (
    <main className="home">
      <h1>nonio has stopped</h1>
      <p className="muted lede">You can close this tab.</p>
      <p className="muted lede">
        Maps are saved in <code>.nonio/maps</code>. Ask your agent to open nonio to see them again.
      </p>
    </main>
  );
}
