import { Suspense, lazy, useEffect, useMemo, useRef, useState, useSyncExternalStore, type ReactNode } from 'react';
import type { ActionResult } from '../net/refusal';
import { SkipApproval } from '../components/SkipApproval';
import { approvalName } from '../net/approvalText';
import { t } from '../i18n';
import { endedWithError, type AgentTileModel } from '../model';
import { visible, type ApprovalModel, type MachineModel } from '../net/view';
import { demoSource, type TerminalSource } from '../terminal/source';
import type { ChatSource } from '../net/chat';
import { dismiss, dismissedIds, subscribeDismissed } from '../net/dismissed';
import type { UsageSummary } from '../net/usage';
import { Icon, type IconName } from '../components/Icon';
import { CloseAgentMenu, CloseConfirm } from '../components/CloseAgent';
import { StateBadge, StateMark } from '../components/StateBadge';
import { NoAskBadge } from '../components/NoAskBadge';
import { ThemePicker } from '../components/ThemePicker';
import { PromptBox } from '../components/PromptBox';
import type { PromptSend } from '../components/chatActions';
import { ClosedSummary } from '../components/ClosedSummary';
import { DiffDialog } from '../components/DiffDialog';
import { FilesDialog } from '../components/FilesDialog';
import type { DiffSource } from '../net/diff';
import type { WorkspaceFiles } from '../net/files';
import { StartApproval, type StartChoice } from '../components/StartApproval';
import { ThemeEditor } from '../themes/ThemeEditor';
import { PHONE_QUERY, byMachine, dockOf, needsYou, type Dock, type NeedsItem } from './nav';
import { type FleetFilter, askLine, emptyKind, fleetCounts, matchesFilter } from '../net/fleet';
import { FleetEmpty, FleetSummary } from '../components/FleetConsole';
import { PairCards, PairRecentList, PairWindowControl, type PairingUi } from '../components/Pairing';
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
function PhoneShell({ top, bottom, dock, children, label }: { top: ReactNode; bottom?: ReactNode; dock?: ReactNode; children: ReactNode; label: string }) {
  const { height, top: offset } = useVisibleHeight();
  return (
    <div className="phone" style={height ? { height, top: offset } : undefined}>
      <header className="phone__top">{top}</header>
      <main className="phone__main" aria-label={label}>{children}</main>
      {(dock || bottom) && <div className="phone__foot">
        {dock}
        {bottom && <nav className="phone__bottom" aria-label={t('phone-nav')}>{bottom}</nav>}
      </div>}
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
// The one place a tap becomes an answer: a second tap waits for the first, and nothing is sent while the link is down.
function useAnswer(x: ApprovalModel, onResolve: Resolve | undefined, stale: boolean) {
  const [busy, setBusy] = useState(false);
  const answer = (ok: boolean, choice?: StartChoice) => {
    if (!onResolve || busy || stale) return;
    setBusy(true);
    void Promise.resolve(onResolve(x, ok, choice)).finally(() => setBusy(false));
  };
  return { busy, answer };
}

function ApprovalCard({ approval: x, machine, untrusted: notTrusted, onResolve, onOpen, stale = false, docked = false }: { approval: ApprovalModel; machine: string; untrusted: boolean; onResolve?: Resolve; onOpen?: () => void; stale?: boolean; docked?: boolean }) {
  const { busy, answer } = useAnswer(x, onResolve, stale);
  const untrusted = notTrusted && <div className="phone-card__hint"><p className="m-0">{t('phone-untrusted', { machine })}</p><a className="phone-chipbtn justify-self-start" href="#/device">{t('device-link')}</a></div>;
  // A request to start an agent is never answered from the bar: it is read whole in its card (it carries a folder, a prompt and settings).
  if (x.start) return <div id={`phone-ask-${x.id}`} className="phone-card phone-card--needs">{untrusted}<StartApproval approval={x} start={x.start} busy={busy || !onResolve || stale} onResolve={(ok, choice) => answer(ok, choice)} />{stale && <p className="phone-card__why m-0">{t('fleet-stale-no-answer')}</p>}</div>;
  // ADR-0090 Amendment A: a switch to a mode that skips permissions is read in the node's own rendering, never as raw text.
  if (x.skip || x.skipUnknown) return <div id={`phone-ask-${x.id}`} className="phone-card phone-card--needs">{untrusted}<SkipApproval approval={x} busy={busy || stale} onResolve={onResolve ? (ok) => answer(ok) : undefined} />{stale && <p className="phone-card__why m-0">{t('fleet-stale-no-answer')}</p>}</div>;
  return (
    <article id={`phone-ask-${x.id}`} className="phone-card phone-card--needs" aria-label={t('approval-title', { name: approvalName(x) })}>
      <button type="button" className="phone-card__head" onClick={onOpen} disabled={!onOpen}>
        <StateMark state="needs-you" />
        <span className="min-w-0 flex-1"><strong className="block truncate">{t('approval-title', { name: approvalName(x) })}</strong><span className="block truncate text-xs text-muted">{machine}</span></span>
        {onOpen && <Icon name="chevron" className="size-4 text-muted rtl:rotate-180" />}
      </button>
      <p className="m-0 break-words">{visible(x.action)}</p>
      {/* The agent's own text, whole, with any hidden character written out: what you allow is what you see. */}
      {x.detail && <pre className="phone-card__detail">{visible(x.detail)}</pre>}
      {untrusted}
      {onResolve && (docked ? <p className="phone-card__why m-0">{t('phone-answer-below')}</p> : <>
        <div className="phone-card__actions">
          <button type="button" className="button button--primary" disabled={busy || stale} aria-describedby={stale ? `phone-why-${x.id}` : undefined} onClick={() => answer(true)}>{t('approval-allow')}</button>
          <button type="button" className="button" disabled={busy || stale} aria-describedby={stale ? `phone-why-${x.id}` : undefined} onClick={() => answer(false)}>{t('approval-deny')}</button>
        </div>
        {stale && <p id={`phone-why-${x.id}`} className="phone-card__why m-0">{t('fleet-stale-no-answer')}</p>}
      </>)}
    </article>
  );
}

const bring = (id: string) => document.getElementById(id)?.scrollIntoView?.({ block: 'center', behavior: 'smooth' });

// How long a newly shown request keeps Allow and Deny disabled: a tap that was meant for the request that has just gone must not land
// on the one that replaced it, at the same place (as the pairing card's half second).
export const DOCK_DWELL_MS = 600;

// What needs you, reachable with one thumb at the bottom of the screen (above the bar on the list). A plain yes or no that is short
// enough to be read whole here (the fleet console's rule, `decidableInline`) has Allow and Deny; anything else has one button that brings its
// full card into view or opens the agent that asks. Disabled with its reason while the link is down; the same signing answers.
// The state (one answer at a time, the dwell) belongs to the bar, not to the request it shows: it is not remounted when the request changes.
function NeedsDock({ dock, count, machines, stale, onResolve, onOpen }: { dock: Dock; count: number; machines: readonly MachineModel[]; stale: boolean; onResolve?: Resolve; onOpen: (id: string) => void }) {
  const x = dock.kind === 'answer' ? dock.approval : undefined;
  const id = x?.id;
  const [busy, setBusy] = useState(false);
  // What is shown (a request, or any other kind of item) has a dwell of its own, and every change of what is shown clears it: a request that
  // comes back after any interlude, even a short one, waits again. Derived from what is shown, so no frame has the new one already ready.
  const shownKey = `${dock.kind}:${id ?? ''}`;
  const [readyFor, setReadyFor] = useState<string | undefined>();
  useEffect(() => {
    const h = setTimeout(() => setReadyFor(shownKey), DOCK_DWELL_MS);
    return () => { clearTimeout(h); setReadyFor(undefined); };
  }, [shownKey]);
  const shown = useRef(id);
  shown.current = id;
  const ready = !!id && readyFor === shownKey;
  const answer = (ok: boolean) => {
    // Only the request on screen, only once its dwell is over, one at a time, never while the link is down.
    if (!x || !onResolve || busy || stale || !ready || shown.current !== x.id) return;
    setBusy(true);
    void Promise.resolve(onResolve(x, ok)).finally(() => setBusy(false));
  };
  // A new request is announced politely (not as an alert) and takes nothing from where the user is; but a focus that was on a button
  // of the request that went moves to the bar instead of falling to the page.
  const box = useRef<HTMLElement>(null);
  const had = useRef(false);
  const first = useRef(true);
  useEffect(() => {
    if (first.current) { first.current = false; return; }
    if (had.current) box.current?.focus();
  }, [id, dock.kind]);
  const who = dock.kind === 'answer' || dock.kind === 'review' ? (dock.agent?.name ?? approvalName(dock.approval)) : dock.kind === 'agent' ? dock.agent.name : t('pair-section', { count: dock.count });
  const where = dock.kind === 'answer' || dock.kind === 'review' ? (dock.agent?.machine || machineName(machines, dock.approval.node)) : dock.kind === 'agent' ? dock.agent.machine : '';
  const why = 'phone-dock-why';
  return (
    <section ref={box} tabIndex={-1} role="status" aria-live="polite" aria-atomic="true" className="phone-dock" aria-label={t('phone-needs', { count })}
      onFocus={() => { had.current = true; }} onBlur={(e) => { if (e.relatedTarget && !e.currentTarget.contains(e.relatedTarget as Node)) had.current = false; }}>
      <div className="phone-dock__head">
        <StateMark state="needs-you" />
        <strong className="text-needs-you">{t('phone-needs', { count })}</strong>
        <span className="min-w-0 flex-1 truncate text-xs text-muted">{visible(who)}{where ? ` · ${visible(where)}` : ''}</span>
      </div>
      {dock.kind === 'answer' && <>
        <p className="phone-dock__ask m-0">{visible(askLine(dock.approval.action, dock.approval.detail))}</p>
        <div className="phone-card__actions">
          <button type="button" className="button button--primary phone-dock__btn" disabled={busy || stale || !ready || !onResolve} aria-describedby={stale ? why : undefined} onClick={() => answer(true)}>{t('approval-allow')}</button>
          <button type="button" className="button phone-dock__btn" disabled={busy || stale || !ready || !onResolve} aria-describedby={stale ? why : undefined} onClick={() => answer(false)}>{t('approval-deny')}</button>
        </div>
        {stale && <p id={why} className="phone-card__why m-0">{t('fleet-stale-no-answer')}</p>}
      </>}
      {dock.kind === 'review' && <button type="button" className="button phone-dock__btn" onClick={() => (dock.agent ? onOpen(dock.agent.id) : bring(`phone-ask-${dock.approval.id}`))}>{t('fleet-review')}</button>}
      {dock.kind === 'agent' && <button type="button" className="button phone-dock__btn" onClick={() => onOpen(dock.agent.id)}>{t('action-open')}</button>}
      {dock.kind === 'pair' && <button type="button" className="button phone-dock__btn" onClick={() => bring('phone-pair')}>{t('fleet-review')}</button>}
    </section>
  );
}

function AgentRow({ agent: a, onOpen }: { agent: AgentTileModel; onOpen: (id: string) => void }) {
  return (
    <li>
      <button type="button" className={`phone-row${a.closed ? ' phone-row--closed' : ''}`} onClick={() => onOpen(a.id)}>
        <StateMark state={a.state} failed={endedWithError(a)} />
        <span className="min-w-0 flex-1">
          <span className="block truncate font-medium text-ink">{a.name}</span>
          <span className="block truncate text-xs text-muted">{a.project || a.workspace}{a.branch ? ` · ${a.branch}` : ''}</span>
        </span>
        <NoAskBadge agent={a} />
        <span className="flex-none text-xs"><StateBadge state={a.state} failed={endedWithError(a)} /></span>
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
export function PhoneHome({ agents, machines, approvals, onResolve, untrusted, notice, link, onRetry, onOpen, onStart, onCommand, onLock, start, pairing }: PhoneCommon & {
  agents: readonly AgentTileModel[];
  link: 'connecting' | 'live' | 'offline';
  onRetry?: () => void;
  onOpen: (id: string) => void;
  onStart?: () => void;
  onCommand?: () => void;
  onLock?: () => void;
  start?: ReactNode;
  // ADR-0065: machines asking to join, for an owner's session.
  pairing?: PairingUi;
}) {
  const cleared = useSyncExternalStore(subscribeDismissed, dismissedIds, dismissedIds);
  const fleet = useMemo(() => agents.filter((a) => !(a.closed && cleared.has(a.id))), [agents, cleared]);
  const needs = needsYou(approvals, fleet);
  // The same five answers as the desktop's Home, as chips above the list; a press shows only those agents below.
  const counts = useMemo(() => fleetCounts(fleet, approvals, machines, pairing?.cards.length ?? 0), [fleet, approvals, machines, pairing?.cards.length]);
  const [filter, setFilter] = useState<FleetFilter | undefined>();
  const groups = useMemo(() => byMachine(fleet.filter((a) => matchesFilter(a, filter)), machines).filter((g) => !filter || g.agents.length > 0), [fleet, machines, filter]);
  const empty = emptyKind(link, fleet.length + approvals.length, machines.length);
  const { dock, count } = dockOf(needs, pairing?.cards.length ?? 0);
  const stale = link !== 'live';
  return (
    <PhoneShell label={t('home-title')}
      dock={dock && !empty && <NeedsDock dock={dock} count={count} machines={machines} stale={stale} onResolve={onResolve} onOpen={onOpen} />}
      top={<>
        <span aria-hidden className="grid size-7 flex-none place-items-center rounded-xs bg-accent text-xs font-bold text-on-accent">A</span>
        <h1 className="m-0 min-w-0 flex-1 truncate text-base font-semibold tracking-normal">{t('home-title')}</h1>
        <ThemePicker />
        {onLock && <button type="button" className="phone-iconbtn" onClick={onLock} aria-label={t('lock-now')} title={t('lock-now-hint')}><Icon name="lock" className="size-5" /></button>}
      </>}
      bottom={<>
        <NavButton icon="search" label={t('phone-search')} onClick={onCommand} />
        <NavButton icon="plus" label={t('phone-start')} onClick={onStart} primary />
        <NavButton icon="stack" label={t('phone-machines')} href="#/machines" />
        <NavButton icon="settings" label={t('device-link')} href="#/device" />
      </>}>
      {notice && <p role="alert" className="phone-notice">{notice}</p>}
      {/* The state of the link has a row of its own (in the title bar it pushed the buttons off a 390 px screen): one notice, what it means and Retry. */}
      {link !== 'live' && <div role="status" className="phone-link"><span className="min-w-0 flex-1">{t(link === 'offline' ? (empty ? 'link-offline' : 'fleet-offline-stale') : 'link-connecting')}</span>{link === 'offline' && onRetry && <button type="button" className="phone-chipbtn" onClick={onRetry}>{t('link-retry')}</button>}</div>}
      {!empty && <div className="phone-fleet"><FleetSummary counts={counts} filter={filter} onFilter={setFilter} /></div>}
      {pairing && pairing.cards.length > 0 && <section id="phone-pair" className="phone-section" aria-label={t('attention-title')}>
        <h2 className="phone-section__title text-needs-you">{t('pair-section', { count: pairing.cards.length })}</h2>
        <div className="grid gap-2"><PairCards ui={pairing} /></div>
      </section>}
      {empty && <FleetEmpty kind={empty} machines={machines} onStart={onStart ? () => onStart() : undefined} onRetry={onRetry} pairing={pairing && empty === 'empty' && link === 'live' ? pairing : undefined} />}
      {pairing && <div className="px-3"><PairRecentList recent={pairing.recent} /></div>}
      {pairing?.window?.open && !empty && <div className="px-3"><PairWindowControl ui={pairing} /></div>}
      {needs.length > 0 && <section className="phone-section" aria-label={t('attention-title')}>
        <h2 className="phone-section__title text-needs-you">{t('phone-needs', { count: needs.length })}</h2>
        <div className="grid gap-2">
          {needs.map((n) => n.kind === 'approval'
            ? <ApprovalCard key={n.approval.id} approval={n.approval} machine={n.agent?.machine || machineName(machines, n.approval.node)} untrusted={!!untrusted?.(n.approval.node)} stale={stale} docked={dock?.kind === 'answer' && dock.approval.id === n.approval.id}
              onResolve={onResolve} onOpen={n.agent ? () => onOpen(n.approval.agentId) : undefined} />
            : <ul key={n.agent.id} className="phone-list phone-card phone-card--needs p-0"><AgentRow agent={n.agent} onOpen={onOpen} /></ul>)}
        </div>
      </section>}
      {groups.length === 0 && !empty && <p className="muted m-0 p-4">{t(filter ? 'fleet-filter-none' : 'ide-welcome-none')}</p>}
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
export function PhoneAgent({ agent, agents, approvals, machines, onResolve, untrusted, notice, link = 'live', source, chatSource, sendPrompt, chooseOption, loadDiff, filesOf, onOpen, onBack }: PhoneCommon & {
  agent: AgentTileModel | undefined;
  agents: readonly AgentTileModel[];
  source?: TerminalSource;
  chatSource?: ChatSource;
  sendPrompt?: PromptSend;
  chooseOption?: (agentKey: string, kind: 'mode' | 'model', id: string) => Promise<ActionResult>;
  /** What the agent changed (the review screen) and its files (the file browser), as on the desktop; absent without a hub. */
  loadDiff?: (agent: AgentTileModel, path?: string) => Promise<DiffSource>;
  filesOf?: (agent: AgentTileModel) => WorkspaceFiles | undefined;
  link?: 'connecting' | 'live' | 'offline';
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
  const { dock } = dockOf(mine.map((approval): NeedsItem => ({ kind: 'approval', approval, agent })));
  const stale = link !== 'live';
  return (
    <PhoneShell label={agent?.name ?? t('focus-not-found')}
      dock={agent && dock && !agent.closed && <NeedsDock dock={dock} count={mine.length} machines={machines} stale={stale} onResolve={onResolve} onOpen={(id) => (id === agent.id ? bring(`phone-ask-${mine[0]?.id}`) : onOpen(id))} />}
      top={<>
        <button type="button" className="phone-iconbtn" onClick={onBack} aria-label={t('phone-back')} title={t('phone-back')}><Icon name="chevron" className="size-5 rotate-180 rtl:rotate-0" /></button>
        <div className="min-w-0 flex-1">
          <h1 className="m-0 truncate text-[15px] font-semibold tracking-normal">{agent?.name ?? t('focus-not-found')}</h1>
          {agent && <span className="flex min-w-0 items-center gap-2 text-xs text-muted"><StateBadge state={agent.state} /><span className="truncate">{agent.machine}</span></span>}
        </div>
        {agent && <NoAskBadge agent={agent} />}
        {agent && <CloseAgentMenu agent={agent} />}
        {next && <button type="button" className="phone-chipbtn text-needs-you" onClick={() => onOpen(next)}>{t('phone-next', { count: elsewhere.length })}</button>}
      </>}>
      {notice && <p role="alert" className="phone-notice">{notice}</p>}
      {!agent ? <div className="grid justify-items-start gap-3 p-4"><p className="m-0">{t('focus-not-found')}</p><button type="button" className="button" onClick={onBack}>{t('phone-back')}</button></div> : <>
        <CloseConfirm agent={agent} where="pane" />
        {!agent.closed && (changed > 0 && loadDiff || files) && (
          <div className="phone-tools">
            {changed > 0 && loadDiff && <button type="button" className="phone-chipbtn text-needs-you" onClick={() => setShown('changes')}>{t('changes-files', { count: changed })}</button>}
            {files && <button type="button" className="phone-chipbtn" onClick={() => setShown('files')}><Icon name="files" className="size-4" />{t('console-files')}</button>}
          </div>
        )}
        {mine.length > 0 && <div className="phone-asks">{mine.map((x) => <ApprovalCard key={x.id} approval={x} machine={agent?.machine || machineName(machines, x.node)} untrusted={!!untrusted?.(x.node)} stale={stale} docked={dock?.kind === 'answer' && dock.approval.id === x.id} onResolve={onResolve} />)}</div>}
        {agent.closed ? <div className="min-h-0 flex-1 overflow-y-auto"><ClosedSummary agent={agent} now={now} onReview={loadDiff ? () => setShown('changes') : undefined} onFiles={files ? () => setShown('files') : undefined} onDismiss={() => { dismiss([agent.id]); onBack(); }} /></div>
          : <div className="phone-pane" aria-label={agent.terminal === false ? t('chat-log') : t('focus-terminal')}>
            {agent.terminal === false ? <PromptBox key={agent.id} agent={agent} source={chatSource} send={sendPrompt} choose={chooseOption} /> : (
              <Suspense fallback={<div className="terminal-host" aria-busy="true" />}>
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
