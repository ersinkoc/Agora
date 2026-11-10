import { AgentTile } from './components/AgentTile';
import { AttentionStrip } from './components/AttentionStrip';
import { StateBadge } from './components/StateBadge';
import type { MachineModel, MessageModel, ProjectCost } from './net/view';
import type { Device } from './net/approval';
import { money, type UsageSummary } from './net/usage';
import { demoAgents } from './fixtures';
import { demoSource, type TerminalSource } from './terminal/source';
import { Suspense, lazy, useEffect, useMemo, useState } from 'react';
import { actionKey, outcomeKind, type AuditResult } from './net/audit';

// The terminal emulator is the heaviest part of the client; load it only when a Focus view needs it.
const TerminalView = lazy(() => import('./terminal/TerminalView').then((m) => ({ default: m.TerminalView })));
import { t } from './i18n';
import { type AgentTileModel, type UiState, needsAttention, sortFleet } from './model';

const demoMessages: MessageModel[] = [
  { from: 'claude · amber', to: 'codex · teal', body: 'The login fix is on branch login-fix, ready for review.', atMs: 2, crossMachine: false },
  { from: 'claude · amber', to: 'laptop/codex · violet', body: 'Can you run the tests on your machine?', atMs: 1, crossMachine: true },
];

function Messages({ messages }: { messages: readonly MessageModel[] }) {
  if (messages.length === 0) return null;
  return (
        <section aria-label={t('messages-title')}>
        <h2>{t('messages-title')}</h2>
        <ul className="messages">
          {messages.map((m, i) => (
            <li key={`${m.atMs}-${i}`}>
              <strong>{m.from}</strong> → <strong>{m.to}</strong>{m.crossMachine ? <span className="muted"> · {t('message-cross')}</span> : null}
              <div className="muted">{m.body}</div>
            </li>
          ))}
        </ul>
      </section>
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
      <Messages messages={demoMessages} />
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
export function Focus({ agent, source: live, costMicroUsd, onBack }: {
  agent: AgentTileModel | undefined;
  costMicroUsd?: number;
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
      {agent.tokens ? <span className="muted">{t('agent-tokens', { count: agent.tokens })}{costMicroUsd !== undefined ? ` · ${t('agent-cost', { amount: money(costMicroUsd) })}` : ''}</span> : null}
      <h2>{t('focus-terminal')}</h2>
      <Suspense fallback={<div className="terminal" aria-busy="true" />}>
        <TerminalView source={source} label={t('focus-terminal')} />
      </Suspense>
      {!live && <p className="muted">{t('focus-terminal-placeholder')}</p>}
    </main>
  );
}

// How an owner makes a machine trust this browser: the key is public, the command is for a terminal.
export function DevicePage({ device, onBack, onAddPasskey }: { device: Device | undefined; onBack: () => void; onAddPasskey?: () => Promise<boolean> }) {
  const [passkey, setPasskey] = useState<'added' | 'failed' | undefined>();
  return (
    <main className="page">
      <button type="button" className="link" onClick={onBack}>{t('focus-back')}</button>
      <h1>{t('device-title')}</h1>
      <p>{t('device-intro')}</p>
      {device && (
        <section className="stack">
          <p><span className="muted">{t('device-name')}</span> <span className="mono">{device.id}</span></p>
          <p className="mono codebox">agora node trust {device.id} {device.publicKeyHex}</p>
        </section>
      )}
      {onAddPasskey && (
        <section className="stack">
          <p>{t('device-passkey-intro')}</p>
          <button type="button" className="button" onClick={() => { void onAddPasskey().then((ok) => setPasskey(ok ? 'added' : 'failed')); }}>{t('device-passkey-add')}</button>
          {passkey && <p role="status">{t(passkey === 'added' ? 'device-passkey-added' : 'device-passkey-failed')}</p>}
        </section>
      )}
    </main>
  );
}

// Who did what on this hub, newest first. The hub shows it to its owner only.
export function AuditPage({ load, onBack }: { load: (who: string) => Promise<AuditResult>; onBack: () => void }) {
  const [who, setWho] = useState('');
  const [result, setResult] = useState<AuditResult | undefined>();
  useEffect(() => {
    let live = true;
    const timer = setTimeout(() => { void load(who.trim()).then((r) => { if (live) setResult(r); }); }, who ? 250 : 0);
    return () => { live = false; clearTimeout(timer); };
  }, [who, load]);
  return (
    <main className="page">
      <button type="button" className="link" onClick={onBack}>{t('focus-back')}</button>
      <h1>{t('audit-title')}</h1>
      <p className="muted">{t('audit-intro')}</p>
      <label>
        <span className="muted">{t('audit-person')}</span>{' '}
        <input value={who} onChange={(e) => setWho(e.target.value)} autoComplete="off" spellCheck={false} />
      </label>
      {result === undefined ? null : !result.ok ? (
        <p role="alert">{t(`audit-${result.reason}`)}</p>
      ) : result.rows.length === 0 ? (
        <p className="muted">{t('audit-empty')}</p>
      ) : (
        <ul className="machines">
          {result.rows.map((r) => (
            <li key={r.id} className="machine">
              <span className="muted">{new Date(r.atMs).toLocaleString()}</span>
              <strong>{r.who}</strong>
              <span>{actionLabel(r.action)}</span>
              {r.node ? <span className="muted mono">{r.node}</span> : null}
              {/* A change to the team carries the role in its outcome, which is a detail and not a result. */}
              {r.action.startsWith('user.') && r.outcome !== 'ok' ? <span className="muted">{r.outcome}</span> : (
                <span className={outcomeKind(r.outcome) === 'problem' ? 'mono' : 'muted'}>
                  {outcomeKind(r.outcome) === 'problem' ? r.outcome : t(`audit-outcome-${outcomeKind(r.outcome)}`)}
                </span>
              )}
            </li>
          ))}
        </ul>
      )}
    </main>
  );
}

function actionLabel(action: string): string {
  const key = actionKey(action);
  const text = t(key);
  // A command this build has no words for is shown as the hub named it.
  return text === key ? action : text;
}

export function SignInPage({ onSubmit, onPasskey, notice }: { onSubmit: (code: string) => void; onPasskey?: () => void; notice?: string }) {
  const [code, setCode] = useState('');
  return (
    <main className="page">
      <h1>{t('signin-title')}</h1>
      <p>{t('signin-intro')}</p>
      <form onSubmit={(e) => { e.preventDefault(); if (code.trim()) onSubmit(code); }}>
        <label>
          <span className="muted">{t('signin-code')}</span>{' '}
          <input value={code} onChange={(e) => setCode(e.target.value)} autoComplete="off" autoCapitalize="characters" spellCheck={false} />
        </label>{' '}
        <button type="submit" className="button button--primary">{t('signin-action')}</button>
      </form>
      {onPasskey && <p><button type="button" className="button" onClick={onPasskey}>{t('signin-passkey')}</button></p>}
      {notice && <p role="alert">{notice}</p>}
    </main>
  );
}

export function MachinesPage({ machines, usage, projects = [], onBack }: { machines: readonly MachineModel[]; usage?: UsageSummary; projects?: readonly ProjectCost[]; onBack: () => void }) {
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
              {usage?.perNode.has(m.id) ? <span className="muted">{t('machine-cost', { amount: money(usage.perNode.get(m.id) ?? 0) })}</span> : null}
            </li>
          ))}
        </ul>
      )}
      {projects.length > 0 && (
        <section className="stack" aria-label={t('usage-projects')}>
          <h2>{t('usage-projects')}</h2>
          <ul className="machines">
            {projects.map((p) => <li key={p.label} className="machine"><strong>{p.label}</strong><span className="muted">{t('machine-cost', { amount: money(p.microUsd) })}</span></li>)}
          </ul>
        </section>
      )}
      {usage && usage.totalMicroUsd > 0 && (
        <p className="muted">
          {t('usage-total', { amount: money(usage.totalMicroUsd) })} · {usage.pricesAtMs ? t('usage-prices-date', { date: new Date(usage.pricesAtMs).toLocaleDateString() }) : t('usage-prices-bundled')}
        </p>
      )}
    </main>
  );
}
