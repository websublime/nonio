// The start page: maps already made in this repo, and a box to ask a new question.
import { useEffect, useState } from 'react';
import { api, connect, pageUrl } from './api';
import { DEFAULT_AGENT, asksInChat } from './asking';
import { navigate } from './nav';
import { StopButton } from './stop';
import type { MapMeta } from './types';

export function Home({ onStopped }: { onStopped: () => void }) {
  const [repo, setRepo] = useState('');
  const [maps, setMaps] = useState<MapMeta[] | null>(null);
  const [making, setMaking] = useState<{ id: string; question: string }[]>([]);
  const [question, setQuestion] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [agent, setAgent] = useState(DEFAULT_AGENT);

  // Stay connected while this page is open: it keeps nonio running, and hears when it stops.
  useEffect(() => connect(() => {}, () => {}, onStopped), [onStopped]);
  useEffect(() => {
    api.list().then((r) => { setRepo(r.repo); setMaps(r.maps); setMaking(r.making); if (r.agent) setAgent(r.agent); }, (e: Error) => setError(e.message));
  }, []);

  const submit = async () => {
    if (!question.trim()) return;
    setBusy(true);
    try {
      const { id } = await api.make(question.trim());
      navigate(pageUrl(id));
    } catch (e) {
      setError((e as Error).message);
      setBusy(false);
    }
  };

  return (
    <main className="home">
      <div className="home-brand"><i className="brand-mark" aria-hidden="true" />nonio://{repo && <span className="muted">{repo}</span>}</div>
      <h1>Ask how your code works</h1>
      <p className="muted lede">Every box and arrow is checked against the code.</p>
      {asksInChat(agent) ? (
        // The agent can't be woken from the page, so a new map is asked for in its chat.
        <p className="home-ask chat-hint"><strong>Ask for a new map in the {agent.name} chat beside this page</strong> <span className="muted">for example: “Map what happens when a user signs up.”</span></p>
      ) : (
        <form className="home-ask" onSubmit={(e) => { e.preventDefault(); void submit(); }}>
          <input id="question" value={question} onChange={(e) => setQuestion(e.target.value)} placeholder="What happens when a user signs up?" aria-label="Your question" autoFocus />
          <button className="btn primary" disabled={busy || !question.trim()}>{busy ? 'Starting…' : 'Map it'}</button>
        </form>
      )}
      {error && <div className="banner banner-dead" role="alert">{error}</div>}
      {making.length > 0 && (
        <section>
          <h2 className="section-title">Being made now</h2>
          <ul className="maplist">{making.map((m) => <li key={m.id}><a href={pageUrl(m.id)}>{m.question}</a><span className="muted small">{agent.name} is exploring…</span></li>)}</ul>
        </section>
      )}
      <section>
        <h2 className="section-title">Maps in this repo</h2>
        {maps === null ? <p className="muted">Loading…</p> : maps.length === 0 ? <p className="muted">None yet.</p> : (
          <ul className="maplist">
            {maps.map((m) => (
              <li key={m.id}>
                <a href={pageUrl(m.id)}>{m.question}</a>
                <span className="muted small">{new Date(m.updatedAt).toLocaleString()}{m.commit ? ` · ${m.commit.slice(0, 7)}` : ''}</span>
              </li>
            ))}
          </ul>
        )}
      </section>
      <footer className="home-foot">
        <StopButton busy={making.length > 0} agentName={agent.name} onStopped={onStopped} onError={setError} />
      </footer>
    </main>
  );
}
