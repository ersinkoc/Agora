import { AttentionSettings } from './components/AttentionSettings';
import { SoundSettings } from './components/SoundSettings';
import { AppNotifySettings } from './components/AppNotifySettings';
import { useAppNotify } from './components/useAppNotify';
import { KeyboardSettings } from './keys/KeyboardSettings';
import { StartApproval, type StartChoice } from './components/StartApproval';
import { HelperGrants, type HelperGrant } from './components/HelperGrants';
import { DeviceRequest } from './components/DeviceRequest';
import { PairCardView, PairRequest } from './components/PairRequest';
import { SignInWaiting } from './components/SignInWaiting';
import type { SignInApprovalModel } from './components/pairChoice';
import { ApprovalBanner, ApprovalBanners } from './components/ApprovalBanner';
import { requestShown, useAnswerGuard, type AnswerResult } from './components/useAnswerGuard';
import { SkipApproval } from './components/SkipApproval';
import { AgentAddress } from './components/AgentAddress';
import { LanguagePicker } from './components/LanguagePicker';
import { ThemePicker } from './components/ThemePicker';
import { type Mode, themeStore } from './themes/store';
import { themeName } from './themes/messages';
import { ThemeSyncSetting } from './themes/SyncPanels';
import { PromptBox } from './components/PromptBox';
import type { PromptSend } from './components/chatActions';
import type { ActionResult } from './net/refusal';
import type { PasskeyResult } from './net/passkey';
import type { ChatSource } from './net/chat';
import { demoConversation, demoRichConversation } from './demoChat';
import { ChatActionsContext } from './components/chatActions';
import { AgentTile } from './components/AgentTile';
import { FinishedMark } from './components/FinishedMark';
import { finishedMarks } from './net/finished';
import { FleetEmpty } from './components/FleetConsole';
import { ApprovalCard } from './components/ApprovalCard';
import { StartAgent, type KindChoice } from './components/StartAgent';
import { AttentionStrip } from './components/AttentionStrip';
import { HeaderLinks } from './components/HeaderLinks';
import { Icon, type IconName } from './components/Icon';
import { CloseAgentMenu, CloseConfirm } from './components/CloseAgent';
import { ago, when } from './net/time';
import { NetworkView } from './components/NetworkView';
import type { PairingUi } from './components/Pairing';
import { Mark, StateBadge } from './components/StateBadge';
import { KitGallery } from './components/kit/KitGallery';
import { PolishGallery } from './components/PolishGallery';
import { AgentAvatar, Button, Callout, ConfirmDialog, CopyChip, Pill, SegmentedControl, SettingRow, StatusMark, Tag } from './components/kit';
import { SettingsBlock, SettingsCard, SystemTile } from './components/SettingsCard';
import { isolated, peerText, systemName } from './components/pairChoice';
import { NoAskBadge } from './components/NoAskBadge';
import { AsksLessChip } from './components/AllowAlways';
import { RunsInsideChip } from './components/RunsInside';
import type { ApprovalModel, MachineModel, MessageModel } from './net/view';
import type { Device } from './net/approval';
import { deviceLabel } from './net/devices';
import { AgentsSettings } from './components/AgentsSettings';
import { demoSettingsDevices, demoSettingsKinds, demoSettingsMachines, demoSettingsNow, demoSettingsTrustedOn } from './fixtures';
import { ScreenPasswordSettings } from './components/Locks';
import type { IdleMinutes } from './net/idle';
import { GALLERY_NOW as MACHINES_NOW, galleryMachineAgents, galleryMachines, galleryPairing, demoAgents, demoAlwaysApproval, demoAlwaysOnlyApproval, demoApprovals, demoAsksLess, demoHelper, demoSkipApproval, demoRemoteStartApproval, demoStartApproval, demoStartLongApproval, demoStartSettingsApproval, demoStartNoAskApproval } from './fixtures';
import { forgetSkipCapable, markSkipCapable } from './net/skipApproval';
import { SessionBar } from './components/SessionBar';
import { demoSource, type TerminalSource } from './terminal/source';
import { Suspense, lazy, useEffect, useId, useMemo, useState, useSyncExternalStore, type ReactNode } from 'react';
import { actionKey, outcomeKind, type AuditResult } from './net/audit';
import { visible } from './net/hidden';

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
  { kind: 'codex', name: 'Codex', installed: true, level: 2, options: [{ id: 'skip-permissions', label: 'option-skip-permissions', skipsPermissions: true }],
    settings: [{ id: 'model', label: 'setting-model', values: [], defaultValue: '' }, { id: 'effort', label: 'setting-effort', values: ['low', 'medium', 'high', 'xhigh', 'max'], defaultValue: 'medium' }] },
  // A chat agent (ACP) and a plain terminal beside it, so the cards show both tags.
  { kind: 'claude', name: 'Claude Code', installed: true, level: 3 },
  { kind: 'shell', name: 'Shell', installed: true, level: 1 },
  { kind: 'gemini', name: 'Gemini CLI', installed: false, level: 2, program: 'gemini' },
  // ADR-0092: an ACP registry agent the machine can start (grouped as untested) and one it cannot find (left to Settings → Agents).
  { kind: 'registry-qwen-code', name: 'Qwen Code', installed: true, level: 3, program: 'npx' },
  { kind: 'registry-goose', name: 'goose', installed: false, level: 3, program: 'goose' },
];
function DemoStart({ onClose }: { onClose: () => void }) {
  return <StartAgent projects={[{ node: 'laptop', projectId: 'p', label: 'login-fix', path: '/home/dev/code/login-fix' }]}
    machines={[{ id: 'laptop', name: 'laptop', os: 'linux', online: true, agentsRunning: 1, lastSeenMs: 0 }, { id: 'mac', name: 'mac', os: 'macos', online: true, agentsRunning: 0, lastSeenMs: 0 }, { id: 'win', name: 'win', os: 'windows', online: true, agentsRunning: 0, lastSeenMs: 0 }]}
    loadKinds={async () => demoKinds}
    inspect={async (_node, path) => ({ path, projectId: 'p', gitRoot: path, branch: 'main', portable: false })}
    onStart={async () => { onClose(); return undefined; }} onClose={onClose} />;
}

// An agent that finished while the person was away (net/finished.ts): the mark on its tile, and the small one of a row in the agent list
// and of a tab, until the agent is seen.
const galleryFinished: AgentTileModel = { ...(demoAgents[1] as AgentTileModel), id: 'gallery/finished', state: 'idle', preview: '', since: undefined, skipsPermissions: false };
function FinishedUnseenDemo() {
  useState(() => finishedMarks.apply({ finished: [galleryFinished.id] }));
  const a = galleryFinished;
  return (
    <div data-finished-gallery className="gallery__row items-start">
      <div className="fleet w-full max-w-sm"><AgentTile agent={a} /></div>
      <div className="flex w-64 flex-col gap-1 rounded-s border border-line bg-sidebar p-1">
        <span className="ide-row"><AgentAvatar accent={a.accent} kind={a.kind || a.name} size="s" /><span className="min-w-0 flex-1 truncate">{a.name}</span><FinishedMark agentId={a.id} compact /><StatusMark state={a.state} /></span>
      </div>
      <div className="flex border-b border-panebar-line bg-panebar text-panebar-ink"><span className="ide-tab"><AgentAvatar accent={a.accent} kind={a.kind || a.name} size="s" /><span className="truncate">{a.name}</span><FinishedMark agentId={a.id} compact /><StatusMark state={a.state} /></span></div>
    </div>
  );
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
      <div className="h-[44rem] max-w-2xl"><PromptBox agent={demoChatAgent} source={demoChat} send={async () => true} choose={async () => true} approvals={<ApprovalBanner approval={demoApprovals[0] as ApprovalModel} onResolve={() => {}} pinned />} /></div>
      <ChatActionsContext.Provider value={{ setConfig: async (_a, _c, v) => (v === 'yolo' ? { approvalNeeded: 'demo/9' } : true), cancelTurn: async () => true, approvals: new Set(['demo/9']) }}>
        <div className="h-[52rem] max-w-2xl"><PromptBox agent={demoRichAgent} source={demoRich} send={async () => true} choose={async () => true} approvals={<ApprovalBanner approval={demoApprovals[0] as ApprovalModel} onResolve={() => {}} pinned />} /></div>
      </ChatActionsContext.Provider>
      <PolishGallery />
      <div className="fleet"><AgentTile agent={demoHelper} /></div>
      <FinishedUnseenDemo />
      {/* ADR-0101: "Allow always" beside Allow and Deny, in the place of Allow when it is all the agent offers, and "Asks less". */}
      <div data-always className="grid max-w-3xl grid-cols-1 gap-3">
        <div className="approvals"><ApprovalCard approval={demoAlwaysApproval} onResolve={() => {}} /><ApprovalCard approval={demoAlwaysOnlyApproval} onResolve={() => {}} /></div>
        <ApprovalBanner approval={demoAlwaysApproval} onResolve={() => {}} pinned />
        <div className="fleet"><AgentTile agent={demoAsksLess} /></div>
      </div>
      {/* Redesign PR 4: the one request component as a card, and in a pane narrower than 360 px (the row density, the buttons under the text). */}
      <div data-approval-sizes className="gallery__row items-start">
        <div className="w-full max-w-[22rem]"><ApprovalCard approval={{ ...(demoApprovals[0] as ApprovalModel), machineName: 'laptop' }} agent={demoAgents[0]} onResolve={() => {}} onOpen={() => {}} /></div>
        <div data-approval-narrow className="w-[12.5rem]"><ApprovalBanner approval={{ ...demoAlwaysApproval, machineName: 'laptop' }} agent={demoAsksLess} onResolve={() => {}} pinned /></div>
      </div>
      <AgentAddress agent={{ ...demoHelper, machineId: 'laptop', id: 'laptop/a8' }} />
      {/* ADR-0090 Amendment A: the card for a switch to a mode that skips permissions (the demo machine "has been heard"), and the badge of an agent that now runs so. */}
      <div data-skip-badge><SessionBar part="controls" agentKey="demo/rich" session={{ ...(demoRichAgent.session as NonNullable<AgentTileModel['session']>), config: (demoRichAgent.session?.config ?? []).map((o) => (o.id === 'mode' ? { ...o, current: 'yolo' } : o)) }} setConfig={async () => true} /></div>
      {/* Redesign PR 5: a start and a switch to act without asking as short cards with Review (a long prompt, settings, a start that
          would not ask, a switch, a pane narrower than 360 px), and the whole request as Review shows it. */}
      <div data-start-cards className="grid max-w-3xl grid-cols-1 gap-3">
        <div className="approvals">
          <ApprovalCard approval={demoStartLongApproval as ApprovalModel} agent={demoAgents[0]} onResolve={() => {}} onOpen={() => {}} />
          <ApprovalCard approval={demoStartSettingsApproval as ApprovalModel} agent={demoAgents[0]} onResolve={() => {}} onOpen={() => {}} />
          <ApprovalCard approval={demoStartNoAskApproval as ApprovalModel} agent={demoAgents[0]} onResolve={() => {}} onOpen={() => {}} />
          <ApprovalCard approval={demoSkipApproval as ApprovalModel} agent={demoRichAgent} onResolve={() => {}} onOpen={() => {}} />
        </div>
        <ApprovalBanner approval={demoStartApproval as ApprovalModel} agent={demoAgents[0]} onResolve={() => {}} pinned />
        <div data-start-narrow className="w-[12.5rem]"><ApprovalBanner approval={demoStartNoAskApproval as ApprovalModel} agent={demoAgents[0]} onResolve={() => {}} pinned /></div>
        <div data-start-full className="max-w-xl"><StartApproval approval={demoStartLongApproval} start={demoStartLongApproval.start} density="full" onResolve={() => {}} /></div>
        {/* ADR-0119: a start another machine asks for, as Review shows it, with its standing-grant choice. */}
        <div data-start-remote className="max-w-xl"><StartApproval approval={demoRemoteStartApproval} start={demoRemoteStartApproval.start} density="full" onResolve={() => {}} /></div>
        <div data-skip-full className="max-w-xl"><SkipApproval approval={demoSkipApproval as ApprovalModel} density="full" onResolve={() => {}} /></div>
      </div>
      {/* ADR-0106 (#1140): the same requests in a viewer's window, read-only: a plain request and a start's card with no answers, and
          the whole start and switch as Review shows them, with no answers and no grant to choose. */}
      <div data-viewer-requests className="grid max-w-3xl grid-cols-1 gap-3">
        <div className="approvals">
          <ApprovalCard approval={{ ...(demoApprovals[0] as ApprovalModel), machineName: 'laptop' }} agent={demoAgents[0]} onOpen={() => {}} />
          <ApprovalCard approval={demoStartLongApproval as ApprovalModel} agent={demoAgents[0]} onOpen={() => {}} />
        </div>
        <div data-start-full-readonly className="max-w-xl"><StartApproval approval={demoStartLongApproval} start={demoStartLongApproval.start} density="full" /></div>
        <div data-skip-full-readonly className="max-w-xl"><SkipApproval approval={demoSkipApproval as ApprovalModel} density="full" /></div>
      </div>
      <div data-helper-grants><HelperGrants view="demo" list={async () => [{ id: 'ap-1', kind: 'codex', folder: '/home/dev/code/login-fix', remaining: 2 }, { id: 'ap-7', kind: 'codex', folder: '/home/dev/code/login-fix', remaining: 4, machine: 'beta', machineName: 'Beta', max: 6 }]} revoke={async () => {}} refresh={0} /></div>
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
      {/* My machines (redesign PR 9): a request to join, the strip, cards online and offline, Add a machine, the old ones. Forget opens
          the danger dialog; nothing is sent from the gallery. */}
      <div data-machines className="max-w-5xl">
        <NetworkView now={MACHINES_NOW} machines={galleryMachines} agents={galleryMachineAgents} hubName="localhost:7444" pairing={galleryPairing}
          heldGrants={async (m) => (m === galleryMachines[0]?.id ? [{ id: 'ap-7', kind: 'codex', folder: '/home/dev/code/login-fix', remaining: 4, machine: m, max: 6, from: 'agent-3 on Mac' }] : [])} revokeHeld={async () => {}}
          onForget={async (ids) => new Map(ids.map((id) => [id, undefined]))} onStart={() => {}} onNewTerminal={() => {}} />
      </div>
      {/* The sign-in card while it waits for the owner's approval and once refused (ADR-0065 decision 6), and Home's first run. */}
      <div className="gallery__row items-start" data-gallery="signin">
        <SignInCard onSubmit={() => {}} onPasskey={() => {}} approval={{ number: '731004', state: 'waiting' }} />
        <SignInCard onSubmit={() => {}} onPasskey={() => {}} approval={{ number: '731004', state: 'denied' }} />
      </div>
      <div className="fleet-console" data-gallery="home-first-run">
        <FleetEmpty kind="first-run" machines={[{ id: 'laptop', name: 'laptop', os: 'linux', online: true, agentsRunning: 0, lastSeenMs: 0 }]} onStart={onStart} />
      </div>
      {/* Home with Agora out of reach and nothing known. (Home with no machine is not here: its join command names this page's address.) */}
      <div className="fleet-console" data-gallery="home-offline"><FleetEmpty kind="offline" machines={[]} onRetry={() => {}} /></div>
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
      <h1>{t('catalog-gallery-title')}</h1>
      <Themed theme="dark" onStart={() => setStarting(true)} />
      <Themed theme="light" onStart={() => setStarting(true)} />
      {starting && <DemoStart onClose={() => setStarting(false)} />}
    </main>
  );
}

// The Settings page on demo data (#/gallery/settings[/section]): every section filled, nothing sent anywhere. The review surface for the
// Settings redesign and its visual-regression baseline; a production build serves it like the gallery.
const demoSettingsDevice = { id: 'web-10a3e055eea9e45f', publicKeyHex: `04${'5be1c09a7d3f42e8'.repeat(8)}`, keys: undefined as unknown as CryptoKeyPair };
export function SettingsDemo({ section, onBack }: { section?: string; onBack: () => void }) {
  const [idle, setIdle] = useState<IdleMinutes>(0);
  return (
    <DevicePage base="#/gallery/settings" section={section} onBack={onBack} now={demoSettingsNow} device={demoSettingsDevice} fingerprint="91c2-9a46-6de1-76e1-6bcf-2d47"
      machines={demoSettingsMachines} trustedOn={demoSettingsTrustedOn} others={demoSettingsDevices} onAsk={async () => true} onRemove={async () => true}
      onAddPasskey={async () => ({ ok: true })} loadKinds={async (node) => (node === 'mac' ? demoSettingsKinds.slice(0, 3) : demoSettingsKinds)}
      security={<ScreenPasswordSettings passwordSet={false} idle={idle} onIdle={setIdle} onSet={async () => ({ ok: true })} />} />
  );
}

// Focus: one agent, full size. The terminal arrives with the hub connection; until then a placeholder.
export function Focus({ agent, approvals = [], onResolve, stale = false, source: live, chatSource, sendPrompt, chooseOption, onBack }: {
  agent: AgentTileModel | undefined;
  // What this agent waits on, answered here as in the window (an agent shown full size must not be left waiting).
  approvals?: readonly ApprovalModel[];
  onResolve?: (approval: ApprovalModel, approved: boolean, choice?: StartChoice, always?: boolean) => AnswerResult;
  // The link is not live: its requests cannot be answered (T44: the dwell starts again when it is back).
  stale?: boolean;
  source?: TerminalSource;
  chatSource?: ChatSource;
  sendPrompt?: PromptSend;
  chooseOption?: (agentKey: string, kind: 'mode' | 'model', id: string) => Promise<ActionResult>;
  onBack: () => void;
}) {
  const preview = agent?.preview ?? '';
  const demo = useMemo(() => demoSource(preview), [preview]);
  const source = live ?? demo;
  // T44 (`useAnswerGuard`, as Home's queue): one dwell for this agent's cards, above its terminal or its composer.
  const mine = agent ? approvals.filter((x) => x.agentId === agent.id) : [];
  const guard = useAnswerGuard(mine.map((x) => x.id), (status) => JSON.stringify([agent?.terminal !== false, mine.map((x) => [requestShown(x, agent), status(x.id)])]), stale);
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
        <button type="button" className="ide-row w-auto flex-none text-[0.8125rem] text-muted hover:text-ink" onClick={onBack}><Icon name="chevron" className="size-3.5 rotate-180 rtl:rotate-0" />{t('focus-back')}</button>
        <h1 className="m-0 truncate text-[0.8125rem] font-semibold tracking-normal">{visible(agent.name)}</h1>
        {/* A live chat agent's state and tokens are said once, in its session bar (`PromptBox`); a terminal or a closed agent keeps them here. */}
        {!(agent.terminal === false && !agent.closed) && <span title={`${t('focus-why')} ${why}`}><StateBadge state={agent.state} /></span>}
        <RunsInsideChip agent={agent} />
        <NoAskBadge agent={agent} />
        <AsksLessChip agent={agent} />
        <span className="hidden min-w-0 items-center gap-1.5 truncate text-xs text-muted md:flex"><Icon name="machine" className="size-3.5" />{visible(agent.machine)}<Icon name="chevron" className="size-3 rtl:rotate-180" /><Icon name="folder" className="size-3.5" />{visible(agent.project || agent.workspace)}{agent.branch && <><Icon name="branch" className="ms-1 size-3.5" /><span className="font-mono">{visible(agent.branch)}</span></>}</span>
        <span className="flex-1" />
        {agent.tokens && agent.terminal !== false ? <span className="ide-chip">{t('agent-tokens', { count: agent.tokens })}</span> : null}
        <CloseAgentMenu agent={agent} />
        <ThemePicker />
      </header>
      <main className="flex min-h-0 min-w-0 flex-col">
        <CloseConfirm agent={agent} where="pane" />
        {agent.terminal === false ? null : <ApprovalBanners approvals={mine} agent={agent} guard={guard} onResolve={onResolve} />}
        <AgentAddress key={agent.id} agent={agent} />
        <div className="pane min-h-0 flex-1 bg-surface-0 p-1.5" aria-label={agent.terminal === false ? t('chat-log') : t('focus-terminal')}>
          {agent.terminal === false ? <PromptBox key={agent.id} agent={agent} source={chatSource} send={sendPrompt} choose={chooseOption} approvals={mine.length ? <div className="grid gap-2"><ApprovalBanners approvals={mine} agent={agent} guard={guard} onResolve={onResolve} pinned /></div> : undefined} /> : (
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
// `plainTitle`: the page draws its own heading (Settings), so the bar's title is not a second one.
function PageFrame({ title, onBack, children, wide, plainTitle }: { title: string; onBack: () => void; children: ReactNode; wide?: boolean; plainTitle?: boolean }) {
  return (
    <div className="min-h-screen bg-surface-0">
      <header className="ide-bar sticky top-0 z-10 h-10 border-b">
        <button type="button" className="ide-row w-auto flex-none text-[0.8125rem] text-muted hover:text-ink" onClick={onBack}><Icon name="chevron" className="size-3.5 rotate-180 rtl:rotate-0" />{t('focus-back')}</button>
        {plainTitle ? <span className="m-0 min-w-0 flex-1 truncate text-[0.8125rem] font-semibold tracking-normal">{title}</span>
          : <h1 className="m-0 min-w-0 flex-1 truncate text-[0.8125rem] font-semibold tracking-normal">{title}</h1>}
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
// Every section is a settings card (an icon, a title, one line and its state, then setting rows); a side index on a wide screen, a row
// of links above the content on a narrow one. Removing another device asks first, in a dialog whose safe answer comes first.
export function DevicePage({ device, onBack, onTheme, section, onAddPasskey, passkeyNeedsPassword = false, security, fingerprint, machines, trustedOn, others, asked, onAsk, onRemove, loadKinds, now: fixedNow, base = '#/device' }: {
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
  // The moment dates are written against (the demo page fixes it, so its screenshots do not move); now by default.
  now?: number;
  // Where the section links point: #/device, or the demo page's own address.
  base?: string;
}) {
  const theme = useSyncExternalStore(themeStore.subscribe, themeStore.state, themeStore.state); // the theme's name below follows a change
  const [passkeyNotice, setPasskeyNotice] = useState<string | undefined>();
  const [password, setPassword] = useState('');
  const [ask, setAsk] = useState<'asked' | 'failed' | undefined>();
  const [removing, setRemoving] = useState<{ id: string; failed?: boolean } | undefined>();
  const [confirming, setConfirming] = useState<MyDevice | undefined>();
  const name = (id: string) => visible(machines?.find((m) => m.id === id)?.name ?? id);
  const missing = (machines ?? []).filter((m) => !trustedOn?.includes(m.id));
  const canVouch = (trustedOn?.length ?? 0) > 0;
  const inApp = useAppNotify(); // the desktop app's own notifications have a card there only
  const sections: { group: string; items: { id: string; label: string; icon: IconName; show: boolean; count?: number }[] }[] = [
    { group: 'settings-group-device', items: [
      { id: 'device', label: 'settings-device', icon: 'monitor', show: !!device },
      { id: 'appearance', label: 'device-appearance', icon: 'palette', show: true },
      { id: 'keyboard', label: 'keys-title', icon: 'keyboard', show: true },
      { id: 'notifications', label: 'attention-settings-title', icon: 'bell', show: true },
      { id: 'sounds', label: 'sounds-title', icon: 'volume', show: true },
      { id: 'app-notifications', label: 'app-notify-title', icon: 'bell', show: inApp },
      { id: 'passkeys', label: 'settings-passkeys', icon: 'key', show: !!onAddPasskey },
      { id: 'lock', label: 'password-title', icon: 'lock', show: !!security },
      { id: 'devices', label: 'device-others', icon: 'globe', show: !!others && others.length > 0, count: others?.filter((d) => !d.removed).length },
    ] },
    { group: 'settings-group-machines', items: [
      { id: 'machines', label: 'settings-machines', icon: 'stack', show: true },
      { id: 'agents', label: 'settings-agents', icon: 'terminal', show: !!loadKinds },
    ] },
  ];
  // A deep link (#/device/passkeys) brings its section to the top; the page's own links change the hash, which lands here.
  useEffect(() => { if (section) document.getElementById(`settings-${section}`)?.scrollIntoView?.({ block: 'start' }); }, [section]);
  const now = fixedNow ?? Date.now();
  const anchor = (id: string) => ({ id: `settings-${id}`, className: 'set-anchor' });
  const addPasskey = () => {
    if (!onAddPasskey) return;
    void onAddPasskey(passkeyNeedsPassword ? password : undefined).then((result) => {
      if (!result.ok && result.reason === 'busy') { setPasskeyNotice('password-busy'); return; }
      setPasskeyNotice(result.ok ? 'device-passkey-added' : 'device-passkey-failed');
      setPassword('');
    });
  };
  const addButton = <Button disabled={passkeyNeedsPassword && !password} onClick={addPasskey}><Icon name="plus" className="size-3.5" />{t('device-passkey-add')}</Button>;
  const trusted = trustedOn?.length ?? 0;
  const trustPill = !machines?.length ? undefined : trusted > 0
    ? <Pill tone="finished" icon={<Icon name="check" className="size-3" />}>{t('device-trust-on', { count: trusted })}</Pill>
    : <Pill tone="plain">{t('device-trust-none')}</Pill>;
  const online = machines?.filter((m) => m.online).length ?? 0;
  const remove = () => {
    const d = confirming;
    setConfirming(undefined);
    if (!d || !onRemove) return;
    setRemoving({ id: d.id });
    void onRemove(d.id).then((ok) => setRemoving(ok ? undefined : { id: d.id, failed: true }));
  };
  return (
    <PageFrame wide title={t('settings-title')} onBack={onBack} plainTitle>
      <div className="set-page" data-settings="">
        <nav aria-label={t('settings-sections')} className="set-nav">
          {sections.map((g) => (
            <div key={g.group} className="set-nav__group">
              <span className="set-nav__overline">{t(g.group)}</span>
              {g.items.filter((i) => i.show).map((i) => (
                <a key={i.id} href={`${base}/${i.id}`} aria-current={section === i.id ? 'true' : undefined} className="kit-row set-nav__row">
                  <Icon name={i.icon} className="set-nav__icon size-4" />
                  <span className="kit-row__label">{t(i.label)}</span>
                  {i.count ? <span className="set-nav__count">{i.count}</span> : null}
                </a>
              ))}
            </div>
          ))}
        </nav>
        <div className="set-content">
          <div className="set-head">
            <h1 className="set-head__title">{t('settings-title')}</h1>
            <p className="set-head__line">{t('settings-subtitle')}</p>
          </div>
          <div className="set-group">
            <h2 className="set-group__title">{t('settings-group-device')}</h2>
            {device && (
              <SettingsCard id="device" icon="monitor" title={t('settings-device')} description={t('device-desc')} end={trustPill}>
                <SettingRow label={t('device-name')}>{() => <span className="set-value mono">{device.id}</span>}</SettingRow>
                {fingerprint && <SettingRow label={t('device-fingerprint')} hint={t('device-fingerprint-hint')}>{() => <span className="set-print mono">{fingerprint}</span>}</SettingRow>}
                {onAsk && missing.length > 0 && (
                  <SettingRow label={t('device-add-title')} hint={t('device-add-intro', { fingerprint: fingerprint ?? '' })}>
                    {() => <Button variant="primary" onClick={() => { void onAsk().then((ok) => setAsk(ok ? 'asked' : 'failed')); }}>{t('device-ask')}</Button>}
                  </SettingRow>
                )}
                {onAsk && missing.length > 0 && (ask === 'asked' || (asked && ask === undefined)) && <SettingsBlock><p role="status" className="m-0">{t('device-asked', { fingerprint: fingerprint ?? '' })}</p></SettingsBlock>}
                {onAsk && missing.length > 0 && ask === 'failed' && <SettingsBlock><p role="alert" className="m-0 text-danger">{t('device-ask-failed')}</p></SettingsBlock>}
                {(missing.length > 0 || !machines?.length) && (
                  <SettingsBlock>
                    <p className="muted m-0">{t('device-command-intro')}</p>
                    <p className="mono codebox m-0">agora node trust {device.id} {device.publicKeyHex}</p>
                  </SettingsBlock>
                )}
              </SettingsCard>
            )}
            <SettingsCard id="appearance" icon="palette" title={t('device-appearance')} description={t('device-appearance-intro', { name: themeName(themeStore.active()) })}>
              <SettingRow label={t('theme-label')}>
                {(labelId) => <>
                  <SegmentedControl labelledBy={labelId} value={theme.mode} onChange={(m) => themeStore.setMode(m)}
                    items={THEME_MODES.map((c) => ({ value: c.id, label: t(c.label), title: t(c.title), icon: <Icon name={c.icon} className="size-3.5" /> }))} />
                  {onTheme && <Button variant="ghost" size="sm" onClick={onTheme}>{t('theme-edit')}</Button>}
                  {onTheme && <a className="kit-btn kit-btn--ghost kit-btn--sm" href="#/catalog" data-gallery-link="">{t('catalog-gallery-open')}</a>}
                </>}
              </SettingRow>
              <SettingRow label={t('language-label')}>{() => <LanguagePicker compact />}</SettingRow>
              <ThemeSyncSetting />
            </SettingsCard>
            <KeyboardSettings />
            <div {...anchor('notifications')}><AttentionSettings /></div>
            <SoundSettings />
            {inApp && <AppNotifySettings />}
            {onAddPasskey && (
              <SettingsCard id="passkeys" icon="key" title={t('settings-passkeys')} description={t('device-passkey-intro')} end={passkeyNeedsPassword ? undefined : addButton}>
                {passkeyNeedsPassword && (
                  <form className="set-form" onSubmit={(e) => { e.preventDefault(); if (password) addPasskey(); }}>
                    <div className="set-form__row">
                      <label htmlFor="passkey-password" className="set-form__label">{t('signin-password')}</label>
                      <input id="passkey-password" type="password" name="passkey-password" value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="current-password" />
                    </div>
                    <div className="set-form__row"><span aria-hidden className="set-form__label" /><div className="set-form__actions">{addButton}</div></div>
                  </form>
                )}
                {passkeyNotice && <SettingsBlock><p role="status" className="m-0">{t(passkeyNotice)}</p></SettingsBlock>}
              </SettingsCard>
            )}
            {security && <div {...anchor('lock')}>{security}</div>}
            {others && others.length > 0 && (
              <SettingsCard id="devices" icon="globe" title={t('device-others')} description={t('device-others-desc')}>
                <ul className="set-list">
                  {others.map((d) => {
                    const label = deviceLabel(d.name, d.id);
                    return (
                      <li key={d.id} className="set-item">
                        <SystemTile system={d.name} />
                        <span className="set-item__main">
                          <span className="set-item__name"><bdi>{label}</bdi></span>
                          <span className="set-item__meta"><span className="set-fp">{d.fingerprint}</span>{d.lastUsedMs > 0 && <span>{t('device-last-used', { when: when(d.lastUsedMs, now) })}</span>}</span>
                        </span>
                        <span className="set-item__end">
                          {d.removed ? <Pill>{t('device-removed')}</Pill>
                            : d.waiting ? <Pill tone="needs" icon={<Mark state="needs-you" />}>{t('device-waiting')}</Pill>
                            : d.trustedOn.length ? <Pill tone="finished" icon={<Icon name="check" className="size-3" />}>{t('device-trusted-on', { machines: d.trustedOn.map(name).join(', ') })}</Pill>
                            : <Pill>{t('device-trusted-nowhere')}</Pill>}
                          {onRemove && canVouch && !d.removed && (
                            <Button variant="danger-ghost" size="sm" aria-label={t('device-remove-named', { name: label })} disabled={removing?.id === d.id && !removing.failed} onClick={() => setConfirming(d)}>
                              <Icon name="trash" className="size-3.5" />{t('device-remove')}
                            </Button>
                          )}
                        </span>
                        {removing?.id === d.id && removing.failed && <p role="alert" className="set-item__alert">{t('device-remove-failed')}</p>}
                      </li>
                    );
                  })}
                </ul>
              </SettingsCard>
            )}
          </div>
          <div className="set-group">
            <div className="set-group__head">
              <h2 className="set-group__title">{t('settings-group-machines')}</h2>
              <a className="kit-btn kit-btn--ghost kit-btn--sm" href="#/machines">{t('settings-machines-manage')}<Icon name="move" className="size-3.5 rtl:-scale-x-100" /></a>
            </div>
            <SettingsCard id="machines" icon="stack" title={t('settings-machines')} description={t('settings-machines-desc')}
              end={machines?.length ? <Pill>{t('settings-machines-online', { online, total: machines.length })}</Pill> : undefined}>
              {machines && machines.length > 0 ? (
                <ul className="set-list">
                  {machines.map((m) => (
                    <li key={m.id}>
                      <a className="set-item set-item--link" href={`#/machines/${encodeURIComponent(m.id)}`}>
                        <SystemTile system={m.os} />
                        <span className="set-item__main">
                          <span className="set-item__name"><bdi>{visible(m.name)}</bdi></span>
                          <span className="set-item__meta">
                            {systemName(m.os) && <span>{peerText(systemName(m.os))}</span>}
                            <span>{m.online ? t('network-heard', { when: ago(m.lastSeenMs, now) }) : t('network-last-seen', { when: ago(m.lastSeenMs, now) })}</span>
                            {trustedOn && <span className={trustedOn.includes(m.id) ? undefined : 'set-item__flag'}>{t(trustedOn.includes(m.id) ? 'device-trusted-here' : 'device-not-trusted-here')}</span>}
                          </span>
                        </span>
                        <span className="set-item__end">
                          {m.build && <span title={t('network-build')}><Tag mono>{visible(m.build)}</Tag></span>}
                          <Pill tone={m.online ? 'finished' : 'idle'} icon={<span aria-hidden className="set-dot" />}>{t(m.online ? 'machine-online' : 'machine-offline')}</Pill>
                        </span>
                      </a>
                    </li>
                  ))}
                </ul>
              ) : <SettingsBlock><p className="muted m-0">{t('machines-empty')}</p></SettingsBlock>}
            </SettingsCard>
            {loadKinds && <div {...anchor('agents')}><AgentsSettings machines={machines ?? []} loadKinds={loadKinds} /></div>}
          </div>
        </div>
      </div>
      <ConfirmDialog open={!!confirming} title={t('device-remove-title', { name: isolated(confirming ? deviceLabel(confirming.name, confirming.id) : '') })} body={t('device-remove-body')}
        cancel={t('device-remove-keep')} confirm={t('device-remove-confirm')} onCancel={() => setConfirming(undefined)} onConfirm={remove} />
    </PageFrame>
  );
}

// The title bar's three scheme buttons, as a labelled choice in Settings (short words; the whole name is the tooltip).
const THEME_MODES: { id: Mode; icon: IconName; label: string; title: string }[] = [
  { id: 'system', icon: 'monitor', label: 'theme-system-short', title: 'theme-system' },
  { id: 'dark', icon: 'moon', label: 'theme-dark', title: 'theme-dark' },
  { id: 'light', icon: 'sun', label: 'theme-light', title: 'theme-light' },
];

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
type SignInProps = { onSubmit: (code: string, password?: string) => void; onPasskey?: () => void; notice?: string; passwordRequired?: boolean; initialCode?: string; approval?: SignInApprovalModel; onAskAgain?: () => void };

export function SignInPage(props: SignInProps) {
  return <main className="signin"><SignInCard {...props} /></main>;
}

// The sign-in card (the gallery shows it in each state): the mark and the title, then the code form or the approval wait, an error as a
// danger callout, the passkey under a divider in every state, and the language in the footer (it works before signing in).
export function SignInCard({ onSubmit, onPasskey, notice, passwordRequired = false, initialCode = '', approval, onAskAgain }: SignInProps) {
  const [code, setCode] = useState(initialCode);
  const [password, setPassword] = useState('');
  const [byCode, setByCode] = useState(false);
  const id = useId();
  return (
    <section className="signin__card" aria-labelledby={`${id}-title`}>
      <header className="signin__head">
        <span aria-hidden className="signin__mark">{Array.from(t('app-name'))[0]}</span>
        <h1 id={`${id}-title`} className="signin__title">{t('signin-title')}</h1>
      </header>
      {approval && !byCode ? <SignInWaiting approval={approval} onAskAgain={() => onAskAgain?.()} onUseCode={() => setByCode(true)} /> : (
        <form className="signin__form" onSubmit={(e) => { e.preventDefault(); if (code.trim() && (!passwordRequired || password)) onSubmit(code, passwordRequired ? password : undefined); }}>
          <p className="signin__step">{t('signin-intro')}</p>
          <CopyChip text="agora hub login" />
          <label className="signin__field">
            <span className="signin__label">{t('signin-code')}</span>
            <input className="signin__code" value={code} onChange={(e) => setCode(e.target.value)} autoComplete="off" autoCapitalize="characters" spellCheck={false} />
          </label>
          {passwordRequired && <>
            <label className="signin__field">
              <span className="signin__label">{t('signin-password')}</span>
              <input type="password" name="password" value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="current-password" />
            </label>
            <p className="signin__hint">{t('signin-password-hint')}</p>
          </>}
          <Button type="submit" variant="primary" className="signin__wide">{t('signin-action')}</Button>
        </form>
      )}
      {notice && <Callout tone="danger" icon="warn" title={notice} alert />}
      {onPasskey && <>
        <div className="signin__or" aria-hidden>{t('signin-or')}</div>
        <Button className="signin__wide" onClick={onPasskey}><Icon name="lock" />{t('signin-passkey')}</Button>
      </>}
      <footer className="signin__foot"><Icon name="globe" className="size-3.5 text-muted" /><LanguagePicker compact /></footer>
    </section>
  );
}

// `pairing`: an owner's session (requests to join are shown here in full, not floating); `onStart` / `onNewTerminal`: a live link.
export function MachinesPage({ machines, agents = [], onBack, onForget, focus, pairing, onStart, onNewTerminal, heldGrants, revokeHeld }: { focus?: string; machines: readonly MachineModel[]; agents?: readonly AgentTileModel[]; onBack: () => void; onForget?: (ids: string[], refuseReturn: boolean) => Promise<Map<string, string | undefined>>; pairing?: PairingUi; onStart?: (node: string) => void; onNewTerminal?: (node: string) => void; heldGrants?: (machine: string) => Promise<HelperGrant[]>; revokeHeld?: (machine: string, grant: HelperGrant) => Promise<void> }) {
  return (
    <PageFrame wide title={t('machines-title')} onBack={onBack}>
      <NetworkView machines={machines} agents={agents} hubName={window.location.host} onForget={onForget} focus={focus} pairing={pairing} onStart={onStart} onNewTerminal={onNewTerminal} heldGrants={heldGrants} revokeHeld={revokeHeld} />
    </PageFrame>
  );
}
