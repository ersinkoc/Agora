import { AgentTile } from './components/AgentTile';
import { AttentionStrip } from './components/AttentionStrip';
import { StateBadge } from './components/StateBadge';
import { demoAgents } from './fixtures';
import { t } from './i18n';
import { type AgentTileModel, type UiState, needsAttention, sortFleet } from './model';

export function Home({ onOpen }: { onOpen?: (id: string) => void }) {
  const fleet = sortFleet(demoAgents);
  const machines = new Set(demoAgents.map((a) => a.machine)).size;
  return (
    <main className="page">
      <header className="topbar">
        <h1>{t('home-title')}</h1>
        <span className="muted">{t('machines-online', { count: machines })}</span>
      </header>
      <AttentionStrip agents={needsAttention(fleet)} onOpen={onOpen} />
      <h2>{t('fleet-title')}</h2>
      {fleet.length === 0 ? (
        <p className="muted">{t('fleet-empty')}</p>
      ) : (
        <div className="fleet">{fleet.map((a) => <AgentTile key={a.id} agent={a} onOpen={onOpen} />)}</div>
      )}
      <p className="muted">{t('palette-hint')}</p>
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

// Focus: one agent, full size. The terminal arrives with the hub connection; until then a placeholder.
export function Focus({ agent, onBack }: { agent: AgentTileModel | undefined; onBack: () => void }) {
  if (!agent) {
    return (
      <main className="page">
        <p>{t('focus-not-found')}</p>
        <button type="button" className="link" onClick={onBack}>{t('focus-back')}</button>
      </main>
    );
  }
  const why = agent.reportedBy === 'agent' ? t('focus-why-agent') : t('focus-why-screen');
  return (
    <main className="page focus">
      <button type="button" className="link" onClick={onBack}>{t('focus-back')}</button>
      <header className="topbar">
        <h1>{agent.name}</h1>
        <span title={`${t('focus-why')} ${why}`}><StateBadge state={agent.state} /></span>
      </header>
      <span className="muted">
        {t('agent-on-machine', { machine: agent.machine })} · {t('agent-workspace', { name: agent.workspace })}
      </span>
      <h2>{t('focus-terminal')}</h2>
      <div className="focus__terminal" role="region" aria-label={t('focus-terminal')}>
        {agent.preview}
        <p className="muted">{t('focus-terminal-placeholder')}</p>
      </div>
    </main>
  );
}
