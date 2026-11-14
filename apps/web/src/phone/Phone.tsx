import { Suspense, lazy, useEffect, useMemo, useState, useSyncExternalStore, type ReactNode } from 'react';
import { t } from '../i18n';
import type { AgentTileModel } from '../model';
import { visible, type ApprovalModel, type MachineModel } from '../net/view';
import { demoSource, type TerminalSource } from '../terminal/source';
import type { ChatSource } from '../net/chat';
import { dismiss, dismissedIds, subscribeDismissed } from '../net/dismissed';
import { money, type UsageSummary } from '../net/usage';
import { Icon, type IconName } from '../components/Icon';
import { StateBadge, StateMark } from '../components/StateBadge';
import { NoAskBadge } from '../components/NoAskBadge';
import { ThemePicker } from '../components/ThemePicker';
import { PromptBox } from '../components/PromptBox';
import { ClosedSummary } from '../components/ClosedSummary';
import { DiffDialog } from '../components/DiffDialog';
import { FilesDialog } from '../components/FilesDialog';
import type { WorkspaceFiles } from '../net/files';
import { StartApproval, type StartChoice } from '../components/StartApproval';
import { ThemeEditor } from '../themes/ThemeEditor';
import { PHONE_QUERY, byMachine, needsYou } from './nav';
import './phone.css';

const TerminalView = lazy(() => import('../terminal/TerminalView').then((m) => ({ default: m.TerminalView })));

const subscribePhone = (cb: () => void) => { const m = typeof window !== 'undefined' ? window.matchMedia?.(PHONE_QUERY) : undefined; m?.addEventListener?.('change', cb); return () => m?.removeEventListener?.('change', cb); };
const isPhone = () => typeof window !== 'undefined' && !!window.matchMedia?.(PHONE_QUERY).matches;
export const usePhone = () => useSyncExternalStore(subscribePhone, isPhone, () => false);

type Resolve = (approval: ApprovalModel, approved: boolean, choice?: StartChoice) => unknown;

// The visible part of the page: with the touch keyboard open it is shorter than the window (iOS does not resize the page), and the
// bars must stay on screen above the keyboard.
function useVisibleHeight(): { height?: number; top?: number } {
  const [box, setBox] = useState<{ height?: number; top?: number }>({});
  useEffect(() => {
    const vv = window.visualViewport;
    if (!vv) return;
    const on = () => setBox({ height: vv.height, top: vv.offsetTop });
    on();
    vv.addEventListener('resize', on);
    vv.addEventListener('scroll', on);
    return () => { vv.removeEventListener('resize', on); vv.removeEventListener('scroll', on); };
  }, []);
  return box;
}

// The phone window: a bar at the top, the screen, and (on the list) a bar at the bottom, inside the safe area.
function PhoneShell({ top, bottom, children, label }: { top: ReactNode; bottom?: ReactNode; children: ReactNode; label: string }) {
  const { height, top: offset } = useVisibleHeight();
  return (
    <div className="phone" style={height ? { height, top: offset } : undefined}>
      <header className="phone__top">{top}</header>
      <main className="phone__main" aria-label={label}>{children}</main>
      {bottom && <nav className="phone__bottom" aria-label={t('phone-nav')}>{bottom}</nav>}
    </div>
  );
}

function NavButton({ icon, label, onClick, href, primary }: { icon: IconName; label: string; onClick?: () => void; href?: string; primary?: boolean }) {
  const body = <><Icon name={icon} className="size-5" /><span className="truncate">{label}</span></>;
  const cls = `phone__nav${primary ? ' phone__nav--primary' : ''}`;
  return href ? <a className={cls} href={href}>{body}</a> : <button type="button" className={cls} onClick={onClick} disabled={!onClick}>{body}</button>;
}

// One request, answered with one tap. The answer goes through the window's own signing (the device key, checked on the machine);
// a second tap waits for the first. A machine that does not trust this device yet is named before the tap, never skipped.
function ApprovalCard({ approval: x, machine, untrusted: notTrusted, onResolve, onOpen }: { approval: ApprovalModel; machine: string; untrusted: boolean; onResolve?: Resolve; onOpen?: () => void }) {
  const [busy, setBusy] = useState(false);
  const answer = (ok: boolean, choice?: StartChoice) => {
    if (!onResolve || busy) return;
    setBusy(true);
    void Promise.resolve(onResolve(x, ok, choice)).finally(() => setBusy(false));
  };
  const untrusted = notTrusted && <div className="phone-card__hint"><p className="m-0">{t('phone-untrusted', { machine })}</p><a className="phone-chipbtn justify-self-start" href="#/device">{t('device-link')}</a></div>;
  if (x.start) return <div className="phone-card phone-card--needs">{untrusted}<StartApproval approval={x} start={x.start} busy={busy || !onResolve} onResolve={(ok, choice) => answer(ok, choice)} /></div>;
  return (
    <article className="phone-card phone-card--needs" aria-label={t('approval-title', { name: x.agentName })}>
      <button type="button" className="phone-card__head" onClick={onOpen} disabled={!onOpen}>
        <StateMark state="needs-you" />
        <span className="min-w-0 flex-1"><strong className="block truncate">{t('approval-title', { name: x.agentName })}</strong><span className="block truncate text-xs text-muted">{machine}</span></span>
        {onOpen && <Icon name="chevron" className="size-4 text-muted rtl:rotate-180" />}
      </button>
      <p className="m-0 break-words">{visible(x.action)}</p>
      {/* The agent's own text, whole, with any hidden character written out: what you allow is what you see. */}
      {x.detail && <pre className="phone-card__detail">{visible(x.detail)}</pre>}
      {untrusted}
      {onResolve && <div className="phone-card__actions">
        <button type="button" className="button button--primary" disabled={busy} onClick={() => answer(true)}>{t('approval-allow')}</button>
        <button type="button" className="button" disabled={busy} onClick={() => answer(false)}>{t('approval-deny')}</button>
      </div>}
    </article>
  );
}

function AgentRow({ agent: a, onOpen }: { agent: AgentTileModel; onOpen: (id: string) => void }) {
  return (
    <li>
      <button type="button" className={`phone-row${a.closed ? ' phone-row--closed' : ''}`} onClick={() => onOpen(a.id)}>
        <StateMark state={a.state} />
        <span className="min-w-0 flex-1">
          <span className="block truncate font-medium text-ink">{a.name}</span>
          <span className="block truncate text-xs text-muted">{a.project || a.workspace}{a.branch ? ` · ${a.branch}` : ''}</span>
        </span>
        <NoAskBadge agent={a} />
        <span className="flex-none text-xs"><StateBadge state={a.state} /></span>
        <Icon name="chevron" className="size-4 text-muted rtl:rotate-180" />
      </button>
    </li>
  );
}

export interface PhoneCommon {
  approvals: readonly ApprovalModel[];
  machines: readonly MachineModel[];
  onResolve: Resolve;
  // ADR-0061: whether a machine is known not to trust this browser's key yet (it reported whom it trusts, and not this one). Unknown
  // is not said: the machine decides either way.
  untrusted?: (node: string) => boolean;
  notice?: string;
}

const machineName = (machines: readonly MachineModel[], id: string) => machines.find((m) => m.id === id)?.name || id;

// The list: what needs you first, then every machine with its agents. Tapping an agent opens it.
export function PhoneHome({ agents, machines, approvals, onResolve, untrusted, notice, link, onRetry, onOpen, onStart, onCommand, onLock, start }: PhoneCommon & {
  agents: readonly AgentTileModel[];
  link: 'connecting' | 'live' | 'offline';
  onRetry?: () => void;
  onOpen: (id: string) => void;
  onStart?: () => void;
  onCommand?: () => void;
  onLock?: () => void;
  start?: ReactNode;
}) {
  const cleared = useSyncExternalStore(subscribeDismissed, dismissedIds, dismissedIds);
  const fleet = useMemo(() => agents.filter((a) => !(a.closed && cleared.has(a.id))), [agents, cleared]);
  const needs = needsYou(approvals, fleet);
  const groups = byMachine(fleet, machines);
  return (
    <PhoneShell label={t('home-title')}
      top={<>
        <span aria-hidden className="grid size-7 flex-none place-items-center rounded-xs bg-accent text-xs font-bold text-on-accent">A</span>
        <h1 className="m-0 min-w-0 flex-1 truncate text-base font-semibold tracking-normal">{t('home-title')}</h1>
        {link !== 'live' && <span role="status" className="ide-chip">{t(link === 'offline' ? 'link-offline' : 'link-connecting')}</span>}
        {link === 'offline' && onRetry && <button type="button" className="phone-chipbtn" onClick={onRetry}>{t('link-retry')}</button>}
        <ThemePicker />
        {onLock && <button type="button" className="phone-iconbtn" onClick={onLock} aria-label={t('lock-now')} title={t('lock-now-hint')}><Icon name="lock" className="size-5" /></button>}
      </>}
      bottom={<>
        <NavButton icon="search" label={t('phone-search')} onClick={onCommand} />
        <NavButton icon="plus" label={t('phone-start')} onClick={onStart} primary />
        <NavButton icon="machine" label={t('phone-machines')} href="#/machines" />
        <NavButton icon="lock" label={t('device-link')} href="#/device" />
      </>}>
      {notice && <p role="alert" className="phone-notice">{notice}</p>}
      {needs.length > 0 && <section className="phone-section" aria-label={t('attention-title')}>
        <h2 className="phone-section__title text-needs-you">{t('phone-needs', { count: needs.length })}</h2>
        <div className="grid gap-2">
          {needs.map((n) => n.kind === 'approval'
            ? <ApprovalCard key={n.approval.id} approval={n.approval} machine={machineName(machines, n.approval.node)} untrusted={!!untrusted?.(n.approval.node)}
              onResolve={onResolve} onOpen={n.agent ? () => onOpen(n.approval.agentId) : undefined} />
            : <ul key={n.agent.id} className="phone-list phone-card phone-card--needs p-0"><AgentRow agent={n.agent} onOpen={onOpen} /></ul>)}
        </div>
      </section>}
      {groups.length === 0 && <p className="muted m-0 p-4">{t('ide-welcome-none')}</p>}
      {groups.map((g) => (
        <section key={g.id} className="phone-section" aria-label={g.name}>
          <h2 className="phone-section__title">
            <span aria-hidden className={`size-2 flex-none rounded-full ${g.online ? 'bg-finished' : 'bg-unknown'}`} />
            <span className="min-w-0 flex-1 truncate">{g.name}</span>
            {!g.online && <span className="font-normal normal-case">{t('phone-offline')}</span>}
          </h2>
          {g.agents.length === 0 ? <p className="muted m-0 px-1">{t('phone-no-agents')}</p>
            : <ul className="phone-list">{g.agents.map((a) => <AgentRow key={a.id} agent={a} onOpen={onOpen} />)}</ul>}
        </section>
      ))}
      {start}
    </PhoneShell>
  );
}

// One agent: Back, its name and state, what it asks, then its terminal (with the key bar) or its conversation.
export function PhoneAgent({ agent, agents, approvals, machines, onResolve, untrusted, notice, source, chatSource, sendPrompt, chooseOption, costMicroUsd, loadDiff, filesOf, onOpen, onBack }: PhoneCommon & {
  agent: AgentTileModel | undefined;
  agents: readonly AgentTileModel[];
  source?: TerminalSource;
  chatSource?: ChatSource;
  sendPrompt?: (agent: AgentTileModel, text: string) => Promise<boolean>;
  chooseOption?: (agentKey: string, kind: 'mode' | 'model', id: string) => Promise<boolean>;
  costMicroUsd?: number;
  /** What the agent changed (the review screen) and its files (the file browser), as on the desktop; absent without a hub. */
  loadDiff?: (agent: AgentTileModel, path?: string) => Promise<string>;
  filesOf?: (agent: AgentTileModel) => WorkspaceFiles | undefined;
  onOpen: (id: string) => void;
  onBack: () => void;
}) {
  const [now] = useState(() => Date.now());
  const [shown, setShown] = useState<'changes' | 'files' | undefined>();
  const files = agent && filesOf ? filesOf(agent) : undefined;
  const changed = agent?.filesChanged ?? 0;
  // Without a hub (development) the terminal is the local stand-in, as in the full-size view.
  const preview = agent?.preview ?? '';
  const demo = useMemo(() => demoSource(preview), [preview]);
  // What else needs you, and the first of it: one tap to go on answering.
  const elsewhere = needsYou(approvals, agents).filter((n) => (n.kind === 'approval' ? n.approval.agentId : n.agent.id) !== agent?.id);
  const next = elsewhere[0] && (elsewhere[0].kind === 'approval' ? elsewhere[0].approval.agentId : elsewhere[0].agent.id);
  const mine = agent ? approvals.filter((x) => x.agentId === agent.id) : [];
  return (
    <PhoneShell label={agent?.name ?? t('focus-not-found')}
      top={<>
        <button type="button" className="phone-iconbtn" onClick={onBack} aria-label={t('phone-back')} title={t('phone-back')}><Icon name="chevron" className="size-5 rotate-180 rtl:rotate-0" /></button>
        <div className="min-w-0 flex-1">
          <h1 className="m-0 truncate text-[15px] font-semibold tracking-normal">{agent?.name ?? t('focus-not-found')}</h1>
          {agent && <span className="flex min-w-0 items-center gap-2 text-xs text-muted"><StateBadge state={agent.state} /><span className="truncate">{agent.machine}</span>{costMicroUsd !== undefined && <span className="flex-none">{money(costMicroUsd)}</span>}</span>}
        </div>
        {agent && <NoAskBadge agent={agent} />}
        {next && <button type="button" className="phone-chipbtn text-needs-you" onClick={() => onOpen(next)}>{t('phone-next', { count: elsewhere.length })}</button>}
      </>}>
      {notice && <p role="alert" className="phone-notice">{notice}</p>}
      {!agent ? <div className="grid justify-items-start gap-3 p-4"><p className="m-0">{t('focus-not-found')}</p><button type="button" className="button" onClick={onBack}>{t('phone-back')}</button></div> : <>
        {!agent.closed && (changed > 0 && loadDiff || files) && (
          <div className="phone-tools">
            {changed > 0 && loadDiff && <button type="button" className="phone-chipbtn text-needs-you" onClick={() => setShown('changes')}>{t('changes-files', { count: changed })}</button>}
            {files && <button type="button" className="phone-chipbtn" onClick={() => setShown('files')}><Icon name="files" className="size-4" />{t('console-files')}</button>}
          </div>
        )}
        {mine.length > 0 && <div className="phone-asks">{mine.map((x) => <ApprovalCard key={x.id} approval={x} machine={machineName(machines, x.node)} untrusted={!!untrusted?.(x.node)} onResolve={onResolve} />)}</div>}
        {agent.closed ? <div className="min-h-0 flex-1 overflow-y-auto"><ClosedSummary agent={agent} now={now} onReview={loadDiff ? () => setShown('changes') : undefined} onFiles={files ? () => setShown('files') : undefined} onDismiss={() => { dismiss([agent.id]); onBack(); }} /></div>
          : <div className="phone-pane" aria-label={agent.terminal === false ? t('chat-log') : t('focus-terminal')}>
            {agent.terminal === false ? <PromptBox agent={agent} source={chatSource} send={sendPrompt} choose={chooseOption} /> : (
              <Suspense fallback={<div className="terminal" aria-busy="true" />}>
                <TerminalView source={source ?? demo} label={`${t('focus-terminal')} · ${agent.name}`} />
              </Suspense>
            )}
          </div>}
        {shown === 'changes' && loadDiff && <DiffDialog agent={agent} load={loadDiff} read={files?.read} onClose={() => setShown(undefined)} />}
        {shown === 'files' && files && <FilesDialog agent={agent} files={files} load={loadDiff} onClose={() => setShown(undefined)} />}
      </>}
    </PhoneShell>
  );
}

// The theme editor (ADR-0068 rung 2) on a phone: a screen with Back, since the phone has no panels.
export function PhoneTheme({ onBack }: { onBack: () => void }) {
  return (
    <PhoneShell label={t('theme-editor-title')}
      top={<>
        <button type="button" className="phone-iconbtn" onClick={onBack} aria-label={t('phone-back')} title={t('phone-back')}><Icon name="chevron" className="size-5 rotate-180 rtl:rotate-0" /></button>
        <h1 className="m-0 min-w-0 flex-1 truncate text-base font-semibold tracking-normal">{t('theme-editor-title')}</h1>
      </>}>
      <ThemeEditor />
    </PhoneShell>
  );
}
