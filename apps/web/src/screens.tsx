import { AgentTile } from './components/AgentTile';
import { AttentionStrip } from './components/AttentionStrip';
import { ApprovalCard } from './components/ApprovalCard';
import { StateBadge } from './components/StateBadge';
import type { ApprovalModel, MachineModel } from './net/view';
import type { Device } from './net/approval';
import { demoAgents } from './fixtures';
import { demoSource, type TerminalSource } from './terminal/source';
import { Suspense, lazy, useMemo } from 'react';

// The terminal emulator is the heaviest part of the client; load it only when a Focus view needs it.
const TerminalView = lazy(() => import('./terminal/TerminalView').then((m) => ({ default: m.TerminalView })));
import { t } from './i18n';
import { type AgentTileModel, type UiState, needsAttention, sortFleet } from './model';

export function Home({ onOpen, agents = demoAgents, machines, link = 'live', approvals = [], onResolve, notice }: {
  onOpen?: (id: string) => void;
  agents?: readonly AgentTileModel[];
  machines?: number;
  link?: 'connecting' | 'live' | 'offline';
  approvals?: readonly ApprovalModel[];
  onResolve?: (a: ApprovalModel, approved: boolean) => void;
  notice?: string;
}) {
  const fleet = sortFleet(agents);
  const machineCount = machines ?? new Set(agents.map((a) => a.machine)).size;
  return (
    <main className="page">
      <header className="topbar">
        <h1>{t('home-title')}</h1>
        <a className="muted" href="#/machines">{t('machines-online', { count: machineCount })}</a>
      </header>
      {link !== 'live' && <p className="muted" role="status">{t(link === 'offline' ? 'link-offline' : 'link-connecting')}</p>}
      <AttentionStrip agents={needsAttention(fleet)} onOpen={onOpen} />
      {approvals.length > 0 && (
        <section aria-label={t('approvals-title')}>
          <h2>{t('approvals-title')}</h2>
          {notice && <p role="alert">{notice}</p>}
          <div className="approvals">
            {approvals.map((a) => <ApprovalCard key={a.id} approval={a} onResolve={(ok) => onResolve?.(a, ok)} />)}
          </div>
        </section>
      )}
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
export function Focus({ agent, source: live, onBack }: {
  agent: AgentTileModel | undefined;
  source?: TerminalSource;
  onBack: () => void;
}) {
  const preview = agent?.preview ?? '';
  const demo = useMemo(() => demoSource(preview), [preview]);
  const source = live ?? demo;
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
      <Suspense fallback={<div className="terminal" aria-busy="true" />}>
        <TerminalView source={source} label={t('focus-terminal')} />
      </Suspense>
      {!live && <p className="muted">{t('focus-terminal-placeholder')}</p>}
    </main>
  );
}

// How an owner makes a machine trust this browser: the key is public, the command is for a terminal.
export function DevicePage({ device, onBack }: { device: Device | undefined; onBack: () => void }) {
  return (
    <main className="page">
      <button type="button" className="link" onClick={onBack}>{t('focus-back')}</button>
      <h1>{t('device-title')}</h1>
      <p>{t('device-intro')}</p>
      {device && (
        <>
          <p><span className="muted">{t('device-name')}</span> <span className="mono">{device.id}</span></p>
          <p className="muted">{t('device-key')}</p>
          <p className="mono">{device.publicKeyHex}</p>
          <p className="mono">agora node trust {device.id} {device.publicKeyHex}</p>
        </>
      )}
    </main>
  );
}

export function MachinesPage({ machines, onBack }: { machines: readonly MachineModel[]; onBack: () => void }) {
  return (
    <main className="page">
      <button type="button" className="link" onClick={onBack}>{t('focus-back')}</button>
      <h1>{t('machines-title')}</h1>
      {machines.length === 0 ? (
        <p className="muted">{t('machines-empty')}</p>
      ) : (
        <ul className="machines">
          {machines.map((m) => (
            <li key={m.id} className="machine">
              <strong>{m.name}</strong>
              <span className="muted">{m.os}</span>
              <span>{t(m.online ? 'machine-online' : 'machine-offline')}</span>
              <span className="muted">{t('machine-agents', { count: m.agentsRunning })}</span>
            </li>
          ))}
        </ul>
      )}
    </main>
  );
}
