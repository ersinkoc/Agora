import { AttentionSettings } from './components/AttentionSettings';
import { StartApproval, type StartChoice } from './components/StartApproval';
import { HelperGrants } from './components/HelperGrants';
import { DeviceRequest } from './components/DeviceRequest';
import { PairCardView, PairRequest } from './components/PairRequest';
import { SignInWaiting } from './components/SignInWaiting';
import type { SignInApprovalModel } from './components/pairChoice';
import { ApprovalBanner } from './components/ApprovalBanner';
import { SkipApproval } from './components/SkipApproval';
import { AgentAddress } from './components/AgentAddress';
import { LanguagePicker } from './components/LanguagePicker';
import { ThemePicker } from './components/ThemePicker';
import { themeStore } from './themes/store';
import { themeName } from './themes/messages';
import { PromptBox } from './components/PromptBox';
import type { PromptSend } from './components/chatActions';
import type { ActionResult } from './net/refusal';
import type { PasskeyResult } from './net/passkey';
import type { ChatSource } from './net/chat';
import { demoConversation, demoRichConversation } from './demoChat';
import { ChatActionsContext } from './components/chatActions';
import { AgentTile } from './components/AgentTile';
import { ApprovalCard } from './components/ApprovalCard';
import { StartAgent, type KindChoice } from './components/StartAgent';
import { AttentionStrip } from './components/AttentionStrip';
import { HeaderLinks } from './components/HeaderLinks';
import { Icon } from './components/Icon';
import { CloseAgentMenu, CloseConfirm } from './components/CloseAgent';
import { ago } from './net/time';
import { NetworkView } from './components/NetworkView';
import { StateBadge } from './components/StateBadge';
import { KitGallery } from './components/kit/KitGallery';
import { NoAskBadge } from './components/NoAskBadge';
import { AsksLessChip } from './components/AllowAlways';
import type { ApprovalModel, MachineModel, MessageModel, ProjectCost } from './net/view';
import type { Device } from './net/approval';
import { deviceLabel } from './net/devices';
import { AgentsSettings } from './components/AgentsSettings';
import type { UsageSummary } from './net/usage';
import { demoAgents, demoAlwaysApproval, demoAlwaysOnlyApproval, demoApprovals, demoAsksLess, demoHelper, demoSkipApproval, demoStartApproval, demoStartSettingsApproval, demoStartNoAskApproval } from './fixtures';
import { forgetSkipCapable, markSkipCapable } from './net/skipApproval';
import { SessionBar } from './components/SessionBar';
import { demoSource, type TerminalSource } from './terminal/source';
import { Suspense, lazy, useEffect, useMemo, useState, useSyncExternalStore, type ReactNode } from 'react';
import { actionKey, outcomeKind, type AuditResult } from './net/audit';

// The terminal emulator is the heaviest part of the client; load it only when a Focus view needs it.
const TerminalView = lazy(() => import('./terminal/TerminalView').then((m) => ({ default: m.TerminalView })));
import { t } from './i18n';
import { type AgentTileModel, type UiState, needsAttention, sortFleet } from './model';
import { GALLERY_NOW, PanelGallery } from './components/PanelGallery';

// The team chat in the gallery: a long message cut to two lines, a helper's first prompt with line breaks, one between machines and one
// only another machine recorded. Times are before the gallery's fixed clock.
const minAgo = (n: number) => GALLERY_NOW - n * 60_000;
const demoMessages: MessageModel[] = [
  { from: 'codex · teal', fromId: 'a2', to: 'claude · amber', atMs: minAgo(1), crossMachine: false,
    body: 'Fine. While you are there: the expiry check compares against SystemTime::now() inside is_expired, which makes it impossible to test deterministically, so I would pass the clock in. Also the token lifetime is hardcoded as 3600 seconds in issue(), but the spec says fifteen minutes.' },
  { from: 'claude · amber', fromId: 'a1', to: 'codex · rose', atMs: minAgo(2), crossMachine: false,
    body: 'Write unit tests for src/auth.rs.\n\nCover these cases:\n1. A token exactly at its expiry is still valid.\n2. A token one second past its expiry is refused.' },
  { from: 'claude · amber', fromId: 'a1', to: 'wrongstack · rose', toMachine: 'Windows PC', atMs: minAgo(3), crossMachine: true, body: 'I am taking src/auth.rs. Please stay out of it until I am done.' },
  // A copy only another machine than the sender's recorded (a receiver's channel copy, or one a node wrote for another): it says who.
  { from: 'desktop/claude · rose', to: 'codex · teal', toMachine: 'Build server', atMs: minAgo(4), crossMachine: true, via: 'laptop', body: '[#ops] Deploy is done.' },
];

const STATES: UiState[] = ['working', 'needs-you', 'finished', 'idle', 'unknown'];

// ADR-0058: a chat agent that reported its modes, models, context and cost, and a conversation with a notice in it.
const demoChatAgent: AgentTileModel = {
  ...(demoAgents[0] as AgentTileModel), id: 'demo/chat', terminal: false, state: 'working', tokens: 78_300, startedMs: Date.now() - 754_000,
  session: {
    modes: [{ id: 'build', name: 'Build', description: '', skipsPermissions: false }, { id: 'plan', name: 'Plan', description: 'No edits', skipsPermissions: false },
      { id: 'bypassPermissions', name: 'Bypass Permissions', description: '', skipsPermissions: true }],
    models: [{ id: 'provider/model-large', name: 'Model Large', description: '', skipsPermissions: false }, { id: 'provider/model-small', name: 'Model Small', description: '', skipsPermissions: false }],
    currentMode: 'build', currentModel: 'provider/model-large', contextTokens: 61_400, contextWindow: 200_000,
  },
};
const demoChat: ChatSource = {
  connect(onLines) {
    onLines(demoConversation(), true);
    return { close() {} };
  },
};
// ADR-0090: an agent whose node offers settings and slash commands, and a conversation that has thinking, calls with a status, a plan with
// step status, a request for permission and a prompt waiting behind the turn.
const choice = (id: string, name: string, skips = false) => ({ id, name, description: '', skipsPermissions: skips });
const demoRichAgent: AgentTileModel = {
  ...demoChatAgent, id: 'demo/rich',
  session: {
    ...(demoChatAgent.session as NonNullable<AgentTileModel['session']>), contextTokens: 62_000,
    config: [
      { id: 'mode', name: 'Mode', description: '', category: 'mode', type: 'select', current: 'plan', skipsPermissions: false, choices: [choice('plan', 'Plan'), choice('build', 'Build'), choice('yolo', 'Bypass Permissions', true)] },
      { id: 'model', name: 'Model', description: '', category: 'model', type: 'select', current: 'large', skipsPermissions: false, choices: [choice('large', 'Opus 5.5'), choice('small', 'Haiku 5')] },
      { id: 'effort', name: 'Effort', description: '', category: 'thought_level', type: 'select', current: 'high', skipsPermissions: false, choices: [choice('low', 'Low'), choice('medium', 'Medium'), choice('high', 'High')] },
      { id: 'compact', name: 'Auto compact', description: '', category: 'custom', type: 'boolean', current: 'true', skipsPermissions: false, choices: [] },
    ],
    commands: [{ name: 'review', description: 'Review the changes in this folder', inputHint: '' }, { name: 'compact', description: 'Summarise the conversation to free context', inputHint: 'what to keep' }, { name: 'init', description: 'Write a project guide', inputHint: '' }],
  },
};
const demoRich: ChatSource = {
  connect(onLines) {
    onLines(demoRichConversation(), true);
    return { close() {} };
  },
};

// ADR-0059: the start window with a kind that offers "Don't ask", for the gallery (nothing is sent anywhere).
// ADR-0069: and one whose program the machine cannot find, named in the explanation.
const demoKinds: KindChoice[] = [
  // ADR-0066: Codex takes a model (any name the tool knows) and an effort (a list, with the tool's own default shown).
  { kind: 'codex', name: 'Codex', installed: true, options: [{ id: 'skip-permissions', label: 'option-skip-permissions', skipsPermissions: true }],
    settings: [{ id: 'model', label: 'setting-model', values: [], defaultValue: '' }, { id: 'effort', label: 'setting-effort', values: ['low', 'medium', 'high', 'xhigh', 'max'], defaultValue: 'medium' }] },
  { kind: 'gemini', name: 'Gemini CLI', installed: false, program: 'gemini' },
];
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
      <KitGallery />
      <div className="gallery__row">{STATES.map((s) => <StateBadge key={s} state={s} />)}</div>
      <AttentionStrip agents={needsAttention(demoAgents)} />
      <div className="fleet">{demoAgents.map((a) => <AgentTile key={a.id} agent={a} />)}</div>
      <PanelGallery messages={demoMessages} fleet={demoAgents} />
      <div className="h-[44rem] max-w-2xl"><PromptBox agent={demoChatAgent} source={demoChat} choose={async () => true} approvals={<ApprovalBanner approval={demoApprovals[0] as ApprovalModel} onResolve={() => {}} pinned />} /></div>
      <ChatActionsContext.Provider value={{ setConfig: async (_a, _c, v) => (v === 'yolo' ? { approvalNeeded: 'demo/9' } : true), cancelTurn: async () => true, approvals: new Set(['demo/9']) }}>
        <div className="h-[52rem] max-w-2xl"><PromptBox agent={demoRichAgent} source={demoRich} send={async () => true} choose={async () => true} approvals={<ApprovalBanner approval={demoApprovals[0] as ApprovalModel} onResolve={() => {}} pinned />} /></div>
      </ChatActionsContext.Provider>
      <div className="fleet"><AgentTile agent={demoHelper} /></div>
      {/* ADR-0101: "Allow always" beside Allow and Deny, in the place of Allow when it is all the agent offers, and "Asks less". */}
      <div data-always className="grid max-w-3xl gap-3">
        <div className="approvals"><ApprovalCard approval={demoAlwaysApproval} onResolve={() => {}} /><ApprovalCard approval={demoAlwaysOnlyApproval} onResolve={() => {}} /></div>
        <ApprovalBanner approval={demoAlwaysApproval} onResolve={() => {}} pinned />
        <div className="fleet"><AgentTile agent={demoAsksLess} /></div>
      </div>
      <AgentAddress agent={{ ...demoHelper, machineId: 'laptop', id: 'laptop/a8' }} />
      {/* ADR-0090 Amendment A: the card for a switch to a mode that skips permissions (the demo machine "has been heard"), and the badge of an agent that now runs so. */}
      <SkipApproval approval={demoSkipApproval as ApprovalModel} onResolve={() => {}} />
      <div data-skip-badge><SessionBar part="controls" agentKey="demo/rich" session={{ ...(demoRichAgent.session as NonNullable<AgentTileModel['session']>), config: (demoRichAgent.session?.config ?? []).map((o) => (o.id === 'mode' ? { ...o, current: 'yolo' } : o)) }} setConfig={async () => true} /></div>
      <StartApproval approval={demoStartApproval} start={demoStartApproval.start} onResolve={() => {}} />
      <StartApproval approval={demoStartSettingsApproval} start={demoStartSettingsApproval.start} onResolve={() => {}} />
      <StartApproval approval={demoStartNoAskApproval} start={demoStartNoAskApproval.start} onResolve={() => {}} />
      <HelperGrants list={async () => [{ id: 'ap-1', kind: 'codex', folder: '/home/dev/code/login-fix', remaining: 2 }]} revoke={async () => {}} refresh={0} />
      <DeviceRequest inline request={{ id: 'web-3fa9c1d24e5b6a70', name: 'Firefox · Linux', fingerprint: 'f947-3cb5-e304-81be-ecf2-64cd' }}
        machines={[{ id: 'laptop', name: 'laptop' }, { id: 'vps-1', name: 'vps-1' }]} onApprove={async () => true} onLater={() => {}} />
      {/* ADR-0065: a machine's request to join as the owner sees it, in each state. The clock is fixed so the gallery does not change by itself. */}
      <div className="gallery__row items-start">
        <PairRequest inline now={0} request={{ id: 'pair-machine', kind: 'machine', name: 'laptop', os: 'windows', address: '192.168.1.50', number: '482193', expiresMs: 272_000 }} onAnswer={async () => 'ok'} />
        <PairCardView inline now={0} state={{ step: 'confirm', failed: false, replace: false, armed: true }} request={{ id: 'pair-confirm', kind: 'machine', name: 'laptop', os: 'windows', address: '192.168.1.50', number: '482193', expiresMs: 272_000 }}
          onApprove={() => {}} onBack={() => {}} onSend={() => {}} onReplace={() => {}} />
        <PairRequest inline now={0} request={{ id: 'pair-taken', kind: 'machine', name: 'laptop', os: 'macos', address: '192.168.1.51', number: '117305', expiresMs: 94_000, nameTaken: true }} onAnswer={async () => 'ok'} />
      </div>
      <div className="gallery__row items-start">
        <PairRequest inline now={0} offline request={{ id: 'pair-offline', kind: 'machine', name: 'desk', os: 'linux', address: '192.168.1.52', number: '640218', expiresMs: 200_000 }} onAnswer={async () => 'ok'} />
        <PairRequest inline now={300_000} request={{ id: 'pair-expired', kind: 'machine', name: 'desk', os: 'linux', address: '192.168.1.52', number: '640218', expiresMs: 272_000 }} onAnswer={async () => 'ok'} />
        <PairRequest inline request={{ id: 'pair-browser', kind: 'browser', name: 'Chrome · macOS', address: '192.168.1.57', number: '731004' }} onAnswer={async () => 'ok'} />
      </div>
      <div className="gallery__row">
        <SignInWaiting approval={{ number: '731004', state: 'waiting' }} onAskAgain={() => {}} onUseCode={() => {}} />
        <SignInWaiting approval={{ number: '731004', state: 'denied' }} onAskAgain={() => {}} onUseCode={() => {}} />
      </div>
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
  // The gallery's demo machine counts as having offered `approval.skip_permissions`, only while the gallery is shown (never at import).
  useEffect(() => { markSkipCapable('demo'); return () => forgetSkipCapable('demo'); }, []);
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
export function Focus({ agent, approvals = [], onResolve, source: live, chatSource, sendPrompt, chooseOption, onBack }: {
  agent: AgentTileModel | undefined;
  // What this agent waits on, answered here as in the window (an agent shown full size must not be left waiting).
  approvals?: readonly ApprovalModel[];
  onResolve?: (approval: ApprovalModel, approved: boolean, choice?: StartChoice, always?: boolean) => void;
  source?: TerminalSource;
  chatSource?: ChatSource;
  sendPrompt?: PromptSend;
  chooseOption?: (agentKey: string, kind: 'mode' | 'model', id: string) => Promise<ActionResult>;
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
  const asks = approvals.filter((x) => x.agentId === agent.id);
  // Full size: the terminal takes the whole window under one slim bar, like a maximized editor tab.
  return (
    <div className="ide" style={{ gridTemplateRows: 'auto minmax(0, 1fr)' }}>
      <header className="ide-bar min-h-10 flex-wrap border-b py-1">
        <button type="button" className="ide-row w-auto flex-none text-[0.8125rem] text-muted hover:text-ink" onClick={onBack}><Icon name="chevron" className="size-3.5 rotate-180 rtl:rotate-0" />{t('focus-back')}</button>
        <h1 className="m-0 truncate text-[0.8125rem] font-semibold tracking-normal">{agent.name}</h1>
        <span title={`${t('focus-why')} ${why}`}><StateBadge state={agent.state} /></span>
        <NoAskBadge agent={agent} />
        <AsksLessChip agent={agent} />
        <span className="hidden min-w-0 items-center gap-1.5 truncate text-xs text-muted md:flex"><Icon name="machine" className="size-3.5" />{agent.machine}<Icon name="chevron" className="size-3 rtl:rotate-180" /><Icon name="folder" className="size-3.5" />{agent.project || agent.workspace}{agent.branch && <><Icon name="branch" className="ms-1 size-3.5" /><span className="font-mono">{agent.branch}</span></>}</span>
        <span className="flex-1" />
        {agent.tokens ? <span className="ide-chip">{t('agent-tokens', { count: agent.tokens })}</span> : null}
        <CloseAgentMenu agent={agent} />
        <span className="hidden sm:block"><LanguagePicker compact /></span>
        <ThemePicker />
      </header>
      <main className="flex min-h-0 min-w-0 flex-col">
        <CloseConfirm agent={agent} where="pane" />
        {agent.terminal === false ? null : approvals.filter((x) => x.agentId === agent.id).map((x) => <ApprovalBanner key={x.id} approval={x} onResolve={onResolve} />)}
        <AgentAddress key={agent.id} agent={agent} />
        <div className="pane min-h-0 flex-1 bg-surface-0 p-1.5" aria-label={agent.terminal === false ? t('chat-log') : t('focus-terminal')}>
          {agent.terminal === false ? <PromptBox key={agent.id} agent={agent} source={chatSource} send={sendPrompt} choose={chooseOption} approvals={asks.length ? <div className="grid gap-2">{asks.map((x) => <ApprovalBanner key={x.id} approval={x} onResolve={onResolve} pinned />)}</div> : undefined} /> : (
            <Suspense fallback={<div className="terminal-host" aria-busy="true" />}>
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
        <button type="button" className="ide-row w-auto flex-none text-[0.8125rem] text-muted hover:text-ink" onClick={onBack}><Icon name="chevron" className="size-3.5 rotate-180 rtl:rotate-0" />{t('focus-back')}</button>
        <h1 className="m-0 min-w-0 flex-1 truncate text-[0.8125rem] font-semibold tracking-normal">{title}</h1>
        <span className="hidden sm:block"><LanguagePicker compact /></span>
        <ThemePicker />
        <HeaderLinks />
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
export function DevicePage({ device, onBack, onTheme, section, onAddPasskey, passkeyNeedsPassword = false, security, fingerprint, machines, trustedOn, others, asked, onAsk, onRemove, loadKinds }: {
  device: Device | undefined;
  onBack: () => void;
  // Opens the theme editor in the window (ADR-0068).
  onTheme?: () => void;
  // The section of the page to bring into view (#/device/<section>).
  section?: string;
  onAddPasskey?: (password?: string) => Promise<PasskeyResult>;
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
  // What a machine can start (Settings → Agents); without it (no live hub) the section is not offered.
  loadKinds?: (node: string) => Promise<KindChoice[]>;
}) {
  useSyncExternalStore(themeStore.subscribe, themeStore.state, themeStore.state); // the theme's name below follows a change
  const [passkeyNotice, setPasskeyNotice] = useState<string | undefined>();
  const [password, setPassword] = useState('');
  const [ask, setAsk] = useState<'asked' | 'failed' | undefined>();
  const [removing, setRemoving] = useState<{ id: string; failed?: boolean } | undefined>();
  const name = (id: string) => machines?.find((m) => m.id === id)?.name ?? id;
  const missing = (machines ?? []).filter((m) => !trustedOn?.includes(m.id));
  const canVouch = (trustedOn?.length ?? 0) > 0;
  const sections: { group: string; items: { id: string; label: string; show: boolean }[] }[] = [
    { group: 'settings-group-device', items: [
      { id: 'device', label: 'settings-device', show: !!device },
      { id: 'appearance', label: 'device-appearance', show: true },
      { id: 'notifications', label: 'attention-settings-title', show: true },
      { id: 'passkeys', label: 'settings-passkeys', show: !!onAddPasskey },
      { id: 'lock', label: 'password-title', show: !!security },
      { id: 'devices', label: 'device-others', show: !!others && others.length > 0 },
    ] },
    { group: 'settings-group-machines', items: [{ id: 'machines', label: 'settings-machines', show: true }, { id: 'agents', label: 'settings-agents', show: !!loadKinds }] },
  ];
  // A deep link (#/device/passkeys) brings its section to the top; the page's own links change the hash, which lands here.
  useEffect(() => { if (section) document.getElementById(`settings-${section}`)?.scrollIntoView?.({ block: 'start' }); }, [section]);
  const now = Date.now();
  const anchor = (id: string) => ({ id: `settings-${id}`, className: 'stack scroll-mt-24' });
  return (
    <PageFrame wide title={t('settings-title')} onBack={onBack}>
      <div className="grid items-start gap-6 md:grid-cols-[12rem_minmax(0,1fr)]">
        <nav aria-label={t('settings-sections')} className="sticky top-10 z-[5] -mx-4 flex gap-x-4 overflow-x-auto border-b border-line bg-surface-0 px-4 py-2 md:top-16 md:mx-0 md:grid md:gap-1 md:overflow-visible md:border-b-0 md:p-0">
          {sections.map((g) => (
            <div key={g.group} className="flex flex-none items-center gap-1 md:grid md:items-stretch">
              <span className="hidden px-2 pt-2 text-xs font-semibold uppercase tracking-wide text-muted md:block">{t(g.group)}</span>
              {g.items.filter((i) => i.show).map((i) => (
                <a key={i.id} href={`#/device/${i.id}`} aria-current={section === i.id ? 'true' : undefined} className="ide-row w-auto flex-none whitespace-nowrap text-[0.8125rem] text-muted hover:text-ink">{t(i.label)}</a>
              ))}
            </div>
          ))}
        </nav>
        <div className="grid min-w-0 max-w-3xl gap-8">
          <div className="grid gap-6">
            <h2 className="m-0 hidden text-base md:block">{t('settings-group-device')}</h2>
            {device && (
              <section {...anchor('device')}>
                <h2 className="m-0">{t('settings-device')}</h2>
                <p className="m-0">{t('device-intro')}</p>
                <p className="m-0"><span className="muted">{t('device-name')}</span> <span className="mono">{device.id}</span></p>
                {fingerprint && <p className="m-0"><span className="muted">{t('device-fingerprint')}</span> <span className="mono text-base tracking-wide">{fingerprint}</span></p>}
                {onAsk && missing.length > 0 && (
                  <div className="stack">
                    <h3 className="m-0">{t('device-add-title')}</h3>
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
            <section {...anchor('appearance')}>
              <h2 className="m-0">{t('device-appearance')}</h2>
              {onTheme && <p className="muted m-0">{t('device-appearance-intro', { name: themeName(themeStore.active()) })}</p>}
              <div className="flex flex-wrap items-center gap-3">
                <ThemePicker />
                <LanguagePicker />
                {onTheme && <button type="button" className="button" onClick={onTheme}>{t('theme-edit')}</button>}
              </div>
            </section>
            <div {...anchor('notifications')}><AttentionSettings /></div>
            {onAddPasskey && (
              <section {...anchor('passkeys')}>
                <h2 className="m-0">{t('settings-passkeys')}</h2>
                <p className="m-0">{t('device-passkey-intro')}</p>
                {passkeyNeedsPassword && (
                  <label>
                    <span className="muted">{t('signin-password')}</span>{' '}
                    <input type="password" name="passkey-password" value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="current-password" />
                  </label>
                )}
                <button type="button" className="button justify-self-start" disabled={passkeyNeedsPassword && !password} onClick={() => {
                  void onAddPasskey(passkeyNeedsPassword ? password : undefined).then((result) => {
                    if (!result.ok && result.reason === 'busy') { setPasskeyNotice('password-busy'); return; }
                    setPasskeyNotice(result.ok ? 'device-passkey-added' : 'device-passkey-failed');
                    setPassword('');
                  });
                }}>{t('device-passkey-add')}</button>
                {passkeyNotice && <p role="status" className="m-0">{t(passkeyNotice)}</p>}
              </section>
            )}
            {security && <div {...anchor('lock')}>{security}</div>}
            {others && others.length > 0 && (
              <section {...anchor('devices')}>
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
          </div>
          <div className="grid gap-6">
            <h2 className="m-0 hidden text-base md:block">{t('settings-group-machines')}</h2>
            <section {...anchor('machines')}>
              <h2 className="m-0">{t('settings-machines')}</h2>
              <p className="muted m-0">{t('settings-machines-intro')}</p>
              {machines && machines.length > 0 ? (
                <ul className="machines">
                  {machines.map((m) => (
                    <li key={m.id} className="machine">
                      <a className="font-semibold hover:underline" href={`#/machines/${encodeURIComponent(m.id)}`}>{m.name}</a>
                      <span className={m.online ? 'text-finished' : 'muted'}>{t(m.online ? 'machine-online' : 'machine-offline')}</span>
                      <span className="muted">{m.online ? t('network-heard', { when: ago(m.lastSeenMs, now) }) : t('network-last-seen', { when: ago(m.lastSeenMs, now) })}</span>
                      {m.build && <span className="muted font-mono" title={t('network-build')}>{m.build}</span>}
                      {trustedOn && <span className={trustedOn.includes(m.id) ? 'muted' : 'text-needs-you'}>{t(trustedOn.includes(m.id) ? 'device-trusted-here' : 'device-not-trusted-here')}</span>}
                    </li>
                  ))}
                </ul>
              ) : <p className="muted m-0">{t('machines-empty')}</p>}
              <p className="m-0"><a className="button" href="#/machines">{t('settings-machines-manage')}</a></p>
            </section>
            {loadKinds && (
              <section {...anchor('agents')}>
                <h2 className="m-0">{t('settings-agents')}</h2>
                <AgentsSettings machines={machines ?? []} loadKinds={loadKinds} />
              </section>
            )}
          </div>
        </div>
      </div>
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
// `approval`: this browser asked the owner to approve it (ADR-0065 decision 6); the page shows the number to compare instead of the
// code form, which stays one click away.
export function SignInPage({ onSubmit, onPasskey, notice, passwordRequired = false, initialCode = '', approval, onAskAgain }: { onSubmit: (code: string, password?: string) => void; onPasskey?: () => void; notice?: string; passwordRequired?: boolean; initialCode?: string; approval?: SignInApprovalModel; onAskAgain?: () => void }) {
  const [code, setCode] = useState(initialCode);
  const [password, setPassword] = useState('');
  const [byCode, setByCode] = useState(false);
  return (
    <main className="page">
      <div className="flex justify-end"><LanguagePicker /></div>
      <h1>{t('signin-title')}</h1>
      {approval && !byCode ? <SignInWaiting approval={approval} onAskAgain={() => onAskAgain?.()} onUseCode={() => setByCode(true)} /> : <>
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
      </>}
      {onPasskey && <p><button type="button" className="button" onClick={onPasskey}>{t('signin-passkey')}</button></p>}
      {notice && <p role="alert">{notice}</p>}
    </main>
  );
}

export function MachinesPage({ machines, agents = [], usage, projects = [], onBack, onForget, focus }: { focus?: string; machines: readonly MachineModel[]; agents?: readonly AgentTileModel[]; usage?: UsageSummary; projects?: readonly ProjectCost[]; onBack: () => void; onForget?: (ids: string[], refuseReturn: boolean) => Promise<Map<string, string | undefined>> }) {
  return (
    <PageFrame wide title={t('machines-title')} onBack={onBack}>
      <NetworkView machines={machines} agents={agents} usage={usage} hubName={window.location.host} onForget={onForget} focus={focus} />
    </PageFrame>
  );
}
