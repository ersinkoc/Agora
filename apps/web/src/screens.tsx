import { StartApproval, type StartChoice } from './components/StartApproval';
import { HelperGrants } from './components/HelperGrants';
import { DeviceRequest } from './components/DeviceRequest';
import { ApprovalBanner } from './components/ApprovalBanner';
import { AgentAddress } from './components/AgentAddress';
import { LanguagePicker } from './components/LanguagePicker';
import { ThemePicker } from './components/ThemePicker';
import { PromptBox } from './components/PromptBox';
import type { ChatSource } from './net/chat';
import { AgentTile } from './components/AgentTile';
import { StartAgent, type KindChoice } from './components/StartAgent';
import { AttentionStrip } from './components/AttentionStrip';
import { Icon } from './components/Icon';
import { NetworkView } from './components/NetworkView';
import { StateBadge } from './components/StateBadge';
import { NoAskBadge } from './components/NoAskBadge';
import type { ApprovalModel, MachineModel, MessageModel, ProjectCost } from './net/view';
import type { Device } from './net/approval';
import { deviceLabel } from './net/devices';
import { money, type UsageSummary } from './net/usage';
import { demoAgents, demoHelper, demoStartApproval } from './fixtures';
import { demoSource, type TerminalSource } from './terminal/source';
import { Suspense, lazy, useEffect, useMemo, useState, type ReactNode } from 'react';
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

// ADR-0058: a chat agent that reported its modes, models, context and cost, and a conversation with a notice in it.
const demoChatAgent: AgentTileModel = {
  ...(demoAgents[0] as AgentTileModel), id: 'demo/chat', terminal: false,
  session: {
    modes: [{ id: 'build', name: 'Build', description: '', skipsPermissions: false }, { id: 'plan', name: 'Plan', description: 'No edits', skipsPermissions: false },
      { id: 'bypassPermissions', name: 'Bypass Permissions', description: '', skipsPermissions: true }],
    models: [{ id: 'provider/model-large', name: 'Model Large', description: '', skipsPermissions: false }, { id: 'provider/model-small', name: 'Model Small', description: '', skipsPermissions: false }],
    currentMode: 'build', currentModel: 'provider/model-large', contextTokens: 61_400, contextWindow: 200_000, costMicros: 23_000, currency: 'USD',
  },
};
const demoChat: ChatSource = {
  connect(onLines) {
    onLines([
      { id: 1, atMs: 0, role: 'user', kind: 'text', text: 'Fix the login test.', path: '' },
      { id: 2, atMs: 0, role: 'agent', kind: 'notice', text: '', path: '', code: 'adapter.turn_empty', params: {} },
      { id: 3, atMs: 0, role: 'user', kind: 'text', text: 'Try again with the small model.', path: '' },
      { id: 4, atMs: 0, role: 'agent', kind: 'text', text: 'The test expected the old redirect; I updated it.', path: '' },
      { id: 5, atMs: 0, role: 'agent', kind: 'notice', text: '', path: '', code: 'adapter.agent_exited', params: { stage: 'idle', exit_code: '1', stderr_tail: 'error: free usage limit reached' } },
    ], true);
    return { close() {} };
  },
};

// ADR-0059: the start window with a kind that offers "Don't ask", for the gallery (nothing is sent anywhere).
const demoKinds: KindChoice[] = [{ kind: 'codex', name: 'Codex', installed: true, options: [{ id: 'skip-permissions', label: 'option-skip-permissions', skipsPermissions: true }] }];
function DemoStart({ onClose }: { onClose: () => void }) {
  return <StartAgent projects={[{ node: 'laptop', projectId: 'p', label: 'login-fix', path: '/home/dev/code/login-fix' }]}
    machines={[{ id: 'laptop', name: 'laptop', os: 'linux', online: true, agentsRunning: 1, lastSeenMs: 0 }]}
    loadKinds={async () => demoKinds}
    inspect={async (_node, path) => ({ path, projectId: 'p', gitRoot: path, branch: 'main', portable: false })}
    onStart={async () => { onClose(); return undefined; }} onClose={onClose} />;
}

function Themed({ theme, onStart }: { theme: 'dark' | 'light'; onStart?: () => void }) {
  return (
    <section className="theme-panel" data-theme={theme}>
      <h2>{t(`theme-${theme}`)}</h2>
      <div className="gallery__row">{STATES.map((s) => <StateBadge key={s} state={s} />)}</div>
      <AttentionStrip agents={needsAttention(demoAgents)} />
      <div className="fleet">{demoAgents.map((a) => <AgentTile key={a.id} agent={a} />)}</div>
      <Messages messages={demoMessages} />
      <div className="h-[26rem] max-w-2xl"><PromptBox agent={demoChatAgent} source={demoChat} choose={async () => true} /></div>
      <div className="fleet"><AgentTile agent={demoHelper} /></div>
      <AgentAddress agent={{ ...demoHelper, machineId: 'laptop', id: 'laptop/a8' }} />
      <StartApproval approval={demoStartApproval} start={demoStartApproval.start} onResolve={() => {}} />
      <HelperGrants list={async () => [{ id: 'ap-1', kind: 'codex', folder: '/home/dev/code/login-fix', remaining: 2 }]} revoke={async () => {}} refresh={0} />
      <DeviceRequest inline request={{ id: 'web-3fa9c1d24e5b6a70', name: 'Firefox · Linux', fingerprint: 'f947-3cb5-e304-81be-ecf2-64cd' }}
        machines={[{ id: 'laptop', name: 'laptop' }, { id: 'vps-1', name: 'vps-1' }]} onApprove={async () => true} onLater={() => {}} />
      <div className="gallery__row">
        <button type="button" className="button button--primary" onClick={onStart}>{t('action-start-agent')}</button>
        <button type="button" className="button">{t('action-open')}</button>
      </div>
    </section>
  );
}

// Every component in every state, both themes: the review surface for UI pull requests.
export function Gallery() {
  const [starting, setStarting] = useState(false);
  return (
    <main className="page gallery">
      <h1>{t('gallery-title')}</h1>
      <Themed theme="dark" onStart={() => setStarting(true)} />
      <Themed theme="light" onStart={() => setStarting(true)} />
      {starting && <DemoStart onClose={() => setStarting(false)} />}
    </main>
  );
}

// Focus: one agent, full size. The terminal arrives with the hub connection; until then a placeholder.
export function Focus({ agent, approvals = [], onResolve, source: live, costMicroUsd, chatSource, sendPrompt, chooseOption, onBack }: {
  agent: AgentTileModel | undefined;
  // What this agent waits on, answered here as in the window (an agent shown full size must not be left waiting).
  approvals?: readonly ApprovalModel[];
  onResolve?: (approval: ApprovalModel, approved: boolean, choice?: StartChoice) => void;
  costMicroUsd?: number;
  source?: TerminalSource;
  chatSource?: ChatSource;
  sendPrompt?: (agent: AgentTileModel, text: string) => Promise<boolean>;
  chooseOption?: (agentKey: string, kind: 'mode' | 'model', id: string) => Promise<boolean>;
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
  // Full size: the terminal takes the whole window under one slim bar, like a maximized editor tab.
  return (
    <div className="ide" style={{ gridTemplateRows: 'auto minmax(0, 1fr)' }}>
      <header className="ide-bar min-h-10 flex-wrap border-b py-1">
        <button type="button" className="ide-row w-auto flex-none text-[13px] text-muted hover:text-ink" onClick={onBack}><Icon name="chevron" className="size-3.5 rotate-180 rtl:rotate-0" />{t('focus-back')}</button>
        <h1 className="m-0 truncate text-[13px] font-semibold tracking-normal">{agent.name}</h1>
        <span title={`${t('focus-why')} ${why}`}><StateBadge state={agent.state} /></span>
        <NoAskBadge agent={agent} />
        <span className="hidden min-w-0 items-center gap-1.5 truncate text-xs text-muted md:flex"><Icon name="machine" className="size-3.5" />{agent.machine}<Icon name="chevron" className="size-3 rtl:rotate-180" /><Icon name="folder" className="size-3.5" />{agent.project || agent.workspace}{agent.branch && <><Icon name="branch" className="ms-1 size-3.5" /><span className="font-mono">{agent.branch}</span></>}</span>
        <span className="flex-1" />
        {agent.tokens ? <span className="ide-chip">{t('agent-tokens', { count: agent.tokens })}{costMicroUsd !== undefined ? ` · ${t('agent-cost', { amount: money(costMicroUsd) })}` : ''}</span> : null}
        <span className="hidden sm:block"><LanguagePicker compact /></span>
        <ThemePicker />
      </header>
      <main className="flex min-h-0 min-w-0 flex-col">
        {approvals.filter((x) => x.agentId === agent.id).map((x) => <ApprovalBanner key={x.id} approval={x} onResolve={onResolve} />)}
        <AgentAddress key={agent.id} agent={agent} />
        <div className="pane min-h-0 flex-1 bg-surface-0 p-1.5" aria-label={agent.terminal === false ? t('chat-log') : t('focus-terminal')}>
          {agent.terminal === false ? <PromptBox agent={agent} source={chatSource} send={sendPrompt} choose={chooseOption} /> : (
            <Suspense fallback={<div className="terminal" aria-busy="true" />}>
              <TerminalView source={source} label={t('focus-terminal')} />
            </Suspense>
          )}
        </div>
        {agent.terminal !== false && !live && <p className="muted m-0 border-t border-line bg-surface-1 px-3 py-1.5">{t('focus-terminal-placeholder')}</p>}
      </main>
    </div>
  );
}

// Pages other than the window itself (machines, this device, audit): the same title bar, content in a readable column.
function PageFrame({ title, onBack, children, wide }: { title: string; onBack: () => void; children: ReactNode; wide?: boolean }) {
  return (
    <div className="min-h-screen bg-surface-0">
      <header className="ide-bar sticky top-0 z-10 h-10 border-b">
        <button type="button" className="ide-row w-auto flex-none text-[13px] text-muted hover:text-ink" onClick={onBack}><Icon name="chevron" className="size-3.5 rotate-180 rtl:rotate-0" />{t('focus-back')}</button>
        <h1 className="m-0 min-w-0 flex-1 truncate text-[13px] font-semibold tracking-normal">{title}</h1>
        <span className="hidden sm:block"><LanguagePicker compact /></span>
        <ThemePicker />
      </header>
      <main className={`mx-auto grid gap-4 px-4 py-6 ${wide ? 'max-w-5xl' : 'max-w-3xl'}`}>{children}</main>
    </div>
  );
}

// ADR-0061: one of the person's devices as "This device" lists it.
export interface MyDevice { id: string; name: string; fingerprint: string; lastUsedMs: number; trustedOn: string[]; waiting: boolean; removed: boolean }

// How an owner makes a machine trust this browser (ADR-0061): which machines trust it, asking another device of theirs to approve it,
// or the command to run on the machine. Fingerprints are what people compare, so every device is shown with its own. The screen
// password and the idle lock live here too (ADR-0067). `passkeyNeedsPassword`: this browser is on another machine and a screen
// password is set, so adding a passkey needs it (ADR-0067).
export function DevicePage({ device, onBack, onAddPasskey, passkeyNeedsPassword = false, security, fingerprint, machines, trustedOn, others, asked, onAsk, onRemove }: {
  device: Device | undefined;
  onBack: () => void;
  onAddPasskey?: (password?: string) => Promise<boolean>;
  passkeyNeedsPassword?: boolean;
  security?: ReactNode;
  fingerprint?: string;
  // Every machine, and the ids of the ones that trust this browser.
  machines?: readonly MachineModel[];
  trustedOn?: readonly string[];
  // The person's other devices.
  others?: readonly MyDevice[];
  asked?: boolean;
  onAsk?: () => Promise<boolean>;
  onRemove?: (id: string) => Promise<boolean>;
}) {
  const [passkey, setPasskey] = useState<'added' | 'failed' | undefined>();
  const [password, setPassword] = useState('');
  const [ask, setAsk] = useState<'asked' | 'failed' | undefined>();
  const [removing, setRemoving] = useState<{ id: string; failed?: boolean } | undefined>();
  const name = (id: string) => machines?.find((m) => m.id === id)?.name ?? id;
  const missing = (machines ?? []).filter((m) => !trustedOn?.includes(m.id));
  const canVouch = (trustedOn?.length ?? 0) > 0;
  return (
    <PageFrame title={t('device-title')} onBack={onBack}>
      <p>{t('device-intro')}</p>
      {device && (
        <section className="stack">
          <p><span className="muted">{t('device-name')}</span> <span className="mono">{device.id}</span></p>
          {fingerprint && <p><span className="muted">{t('device-fingerprint')}</span> <span className="mono text-base tracking-wide">{fingerprint}</span></p>}
          {machines && machines.length > 0 && (
            <ul className="machines">
              {machines.map((m) => (
                <li key={m.id} className="machine">
                  <strong>{m.name}</strong>
                  <span className={trustedOn?.includes(m.id) ? 'muted' : 'text-needs-you'}>{t(trustedOn?.includes(m.id) ? 'device-trusted-here' : 'device-not-trusted-here')}</span>
                </li>
              ))}
            </ul>
          )}
          {onAsk && missing.length > 0 && (
            <div className="stack">
              <h2 className="m-0">{t('device-add-title')}</h2>
              <p className="muted m-0">{t('device-add-intro', { fingerprint: fingerprint ?? '' })}</p>
              <p className="m-0"><button type="button" className="button button--primary" onClick={() => { void onAsk().then((ok) => setAsk(ok ? 'asked' : 'failed')); }}>{t('device-ask')}</button></p>
              {(ask === 'asked' || (asked && ask === undefined)) && <p role="status" className="m-0">{t('device-asked', { fingerprint: fingerprint ?? '' })}</p>}
              {ask === 'failed' && <p role="alert" className="m-0">{t('device-ask-failed')}</p>}
            </div>
          )}
          {(missing.length > 0 || !machines?.length) && <>
            <p className="m-0 muted">{t('device-command-intro')}</p>
            <p className="mono codebox">agora node trust {device.id} {device.publicKeyHex}</p>
          </>}
        </section>
      )}
      {others && others.length > 0 && (
        <section className="stack">
          <h2 className="m-0">{t('device-others')}</h2>
          <ul className="machines">
            {others.map((d) => (
              <li key={d.id} className="machine">
                <strong>{deviceLabel(d.name, d.id)}</strong>
                <span className="mono">{d.fingerprint}</span>
                <span className="muted">{d.removed ? t('device-removed') : d.waiting ? t('device-waiting') : d.trustedOn.length ? t('device-trusted-on', { machines: d.trustedOn.map(name).join(', ') }) : t('device-trusted-nowhere')}</span>
                {d.lastUsedMs > 0 && <span className="muted">{t('device-last-used', { when: new Date(d.lastUsedMs).toLocaleString() })}</span>}
                {onRemove && canVouch && !d.removed && (
                  <button type="button" className="button px-2 py-1 text-xs" disabled={removing?.id === d.id && !removing.failed}
                    onClick={() => { setRemoving({ id: d.id }); void onRemove(d.id).then((ok) => setRemoving(ok ? undefined : { id: d.id, failed: true })); }}>{t('device-remove')}</button>
                )}
                {removing?.id === d.id && removing.failed && <span role="alert">{t('device-remove-failed')}</span>}
              </li>
            ))}
          </ul>
        </section>
      )}
      {onAddPasskey && (
        <section className="stack">
          <p>{t('device-passkey-intro')}</p>
          {passkeyNeedsPassword && (
            <label>
              <span className="muted">{t('signin-password')}</span>{' '}
              <input type="password" name="passkey-password" value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="current-password" />
            </label>
          )}
          <button type="button" className="button" disabled={passkeyNeedsPassword && !password} onClick={() => { void onAddPasskey(passkeyNeedsPassword ? password : undefined).then((ok) => { setPasskey(ok ? 'added' : 'failed'); setPassword(''); }); }}>{t('device-passkey-add')}</button>
          {passkey && <p role="status">{t(passkey === 'added' ? 'device-passkey-added' : 'device-passkey-failed')}</p>}
        </section>
      )}
      {security}
    </PageFrame>
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
    <PageFrame title={t('audit-title')} onBack={onBack}>
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
              {r.session ? <span className="muted mono" title={t('audit-session-hint')}>{t('audit-session', { id: r.session })}</span> : null}
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
    </PageFrame>
  );
}

function actionLabel(action: string): string {
  const key = actionKey(action);
  const text = t(key);
  // A command this build has no words for is shown as the hub named it.
  return text === key ? action : text;
}

// `passwordRequired`: the hub asks this browser for the screen password too (ADR-0067; never on the hub's own machine).
export function SignInPage({ onSubmit, onPasskey, notice, passwordRequired = false, initialCode = '' }: { onSubmit: (code: string, password?: string) => void; onPasskey?: () => void; notice?: string; passwordRequired?: boolean; initialCode?: string }) {
  const [code, setCode] = useState(initialCode);
  const [password, setPassword] = useState('');
  return (
    <main className="page">
      <div className="flex justify-end"><LanguagePicker /></div>
      <h1>{t('signin-title')}</h1>
      <p>{t('signin-intro')}</p>
      <form onSubmit={(e) => { e.preventDefault(); if (code.trim() && (!passwordRequired || password)) onSubmit(code, passwordRequired ? password : undefined); }}>
        <label>
          <span className="muted">{t('signin-code')}</span>{' '}
          <input value={code} onChange={(e) => setCode(e.target.value)} autoComplete="off" autoCapitalize="characters" spellCheck={false} />
        </label>{' '}
        {passwordRequired && (
          <>
            <label>
              <span className="muted">{t('signin-password')}</span>{' '}
              <input type="password" name="password" value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="current-password" />
            </label>{' '}
          </>
        )}
        <button type="submit" className="button button--primary">{t('signin-action')}</button>
      </form>
      {passwordRequired && <p className="muted">{t('signin-password-hint')}</p>}
      {onPasskey && <p><button type="button" className="button" onClick={onPasskey}>{t('signin-passkey')}</button></p>}
      {notice && <p role="alert">{notice}</p>}
    </main>
  );
}

export function MachinesPage({ machines, agents = [], usage, projects = [], onBack }: { machines: readonly MachineModel[]; agents?: readonly AgentTileModel[]; usage?: UsageSummary; projects?: readonly ProjectCost[]; onBack: () => void }) {
  return (
    <PageFrame wide title={t('machines-title')} onBack={onBack}>
      <NetworkView machines={machines} agents={agents} usage={usage} hubName={window.location.host} />
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
    </PageFrame>
  );
}
