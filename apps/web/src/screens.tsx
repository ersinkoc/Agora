import { AgentTile } from './components/AgentTile';
import { AttentionStrip } from './components/AttentionStrip';
import { StateBadge } from './components/StateBadge';
import { demoAgents } from './fixtures';
import { t } from './i18n';
import { type UiState, needsAttention, sortFleet } from './model';

export function Home() {
  const fleet = sortFleet(demoAgents);
  const machines = new Set(demoAgents.map((a) => a.machine)).size;
  return (
    <main className="page">
      <header className="topbar">
        <h1>{t('home-title')}</h1>
        <span className="muted">{t('machines-online', { count: machines })}</span>
      </header>
      <AttentionStrip agents={needsAttention(fleet)} />
      <h2>{t('fleet-title')}</h2>
      {fleet.length === 0 ? (
        <p className="muted">{t('fleet-empty')}</p>
      ) : (
        <div className="fleet">{fleet.map((a) => <AgentTile key={a.id} agent={a} />)}</div>
      )}
      <div><button type="button" className="button button--primary">{t('action-start-agent')}</button></div>
    </main>
  );
}

const STATES: UiState[] = ['working', 'needs-you', 'finished', 'idle', 'unknown'];

function Themed({ theme }: { theme: 'dark' | 'light' }) {
  return (
    <section className="theme-panel" data-theme={theme}>
      <h2>{t(`theme-${theme}`)}</h2>
      <div className="gallery__row">{STATES.map((s) => <StateBadge key={s} state={s} />)}</div>
      <AttentionStrip agents={needsAttention(demoAgents)} />
      <div className="fleet">{demoAgents.map((a) => <AgentTile key={a.id} agent={a} />)}</div>
      <div className="gallery__row">
        <button type="button" className="button button--primary">{t('action-start-agent')}</button>
        <button type="button" className="button">{t('action-open')}</button>
      </div>
    </section>
  );
}

// Every component in every state, both themes: the review surface for UI pull requests.
export function Gallery() {
  return (
    <main className="page gallery">
      <h1>{t('gallery-title')}</h1>
      <Themed theme="dark" />
      <Themed theme="light" />
    </main>
  );
}
