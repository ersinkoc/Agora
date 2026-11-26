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
import { useCloseAsk } from '../net/closeAgent';
import { NoAskBadge } from '../components/NoAskBadge';
import { FinishedMark } from '../components/FinishedMark';
import { AlwaysNote, AlwaysOnly, AsksLessChip, alwaysNoteId } from '../components/AllowAlways';
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
import { kindOf } from '../components/AgentTile';
import { AnswerButtons, ApprovalRequest, ASK_ICON, askKind, askTitle } from '../components/ApprovalRequest';
import { AgentAvatar, Button, Callout, Pill, StatusPill, Tag } from '../components/kit';
import { ATTENTION_DWELL_MS } from '../net/attention';
import { requestShown, useAnswerGuard, type AnswerGuard } from '../components/useAnswerGuard';
import { PHONE_DETAILS, PHONE_QUERY, byMachine, dockOf, needsYou, type Dock, type NeedsItem, type PhoneDetailsTab } from './nav';
import { Details } from '../components/Inspector';
import { Menu, type MenuItem } from '../layout/Menu';
import type { BoardModel, MessageModel } from '../net/view';
import { type FleetFilter, askLine, emptyKind, fleetCounts, matchesFilter } from '../net/fleet';
import { FleetEmpty, FleetSummary } from '../components/FleetConsole';
import { PairCards, PairRecentList, PairWindowControl, type PairingUi } from '../components/Pairing';
import './phone.css';

const TerminalView = lazy(() => import('../terminal/TerminalView').then((m) => ({ default: m.TerminalView })));

const subscribePhone = (cb: () => void) => { const m = typeof window !== 'undefined' ? window.matchMedia?.(PHONE_QUERY) : undefined; m?.addEventListener?.('change', cb); return () => m?.removeEventListener?.('change', cb); };
const isPhone = () => typeof window !== 'undefined' && !!window.matchMedia?.(PHONE_QUERY).matches;
export const usePhone = () => useSyncExternalStore(subscribePhone, isPhone, () => false);

type Resolve = (approval: ApprovalModel, approved: boolean, choice?: StartChoice, always?: boolean) => unknown;

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

// T44 on the phone (#1048 item 4): its list of requests and its bar for what needs you are surfaces like Home's queue and the side panel,
// with the same guard (`useAnswerGuard`) and the window's holds. Allow and Deny do nothing for a short dwell after anything the surface
// shows changed, after the person came back and after the link came back; a request is answered once on every surface (held while it
// goes, "Answer sent" once it went, answerable again after a fresh dwell when it failed); nothing is sent while the link is down. A start
// goes through the same guard; a switch keeps its own guards (its Allow dwell, its one-click lock, its Deny always available).
// What a list of request cards draws, for its guard's layout: a change in any of it can move a card or a button.
// `machine`: the machine's name as the card draws it (the agent's, else the request's, else the machine list's).
// `folded`: drawn as one line (an agent's screen while the bar at the bottom holds a request); a fold or an unfold moves the buttons,
// so it is part of what the guard watches and the dwell starts again with it.
type PhoneAsk = { approval: ApprovalModel; agent?: AgentTileModel; machine: string; docked: boolean; folded?: boolean };
const asksShown = (asks: readonly PhoneAsk[], status: (id: string) => string, around: unknown) =>
  JSON.stringify([around, asks.map((a) => [...requestShown(a.approval, a.agent), a.machine, a.docked, !!a.folded, status(a.approval.id)])]);

// One request in the phone's list: the one request component in its row density (redesign PR 13), with the text across the card and the
// buttons under it, each as wide as it can be. The ask is whole: the action as one chip and the agent's full detail, any hidden character
// written out (what you allow is what you see). A plain request the bar answers says so instead of repeating the buttons; a request to start
// an agent and a switch are read whole in their own cards (they carry a folder, a prompt, settings) and never answered from the bar.
// `onFold`: on an agent's screen, a request folds and unfolds in place (visual pass 2026-11, finding 16). Folded it is one line, the ask's
// title and nothing to answer with: Deny and Allow are only ever beside the ask read whole.
function PhoneRequest({ ask, guard, onResolve, onOpen, onFold }: { ask: PhoneAsk; guard: AnswerGuard; onResolve?: Resolve; onOpen?: () => void; onFold?: (folded: boolean) => void }) {
  const { approval: x, agent, machine, docked } = ask;
  if (ask.folded && onFold) {
    const title = askTitle(x, visible(agent?.name || approvalName(x)));
    return <div id={`phone-ask-${x.id}`} tabIndex={-1} className="phone-fold">
      <button type="button" className="phone-fold__btn" aria-expanded={false} onClick={() => onFold(false)}>
        <span aria-hidden className="phone-fold__chip"><Icon name={ASK_ICON[askKind(x)]} className="size-4" /></span>
        <span className="min-w-0 flex-1 truncate">{title}</span>
        <Icon name="chevronDown" className="size-4 flex-none text-muted" />
      </button>
    </div>;
  }
  const fold = onFold && <button type="button" className="phone-iconbtn phone-req__open" aria-expanded onClick={() => onFold(true)} aria-label={t('phone-ask-fold')} title={t('phone-ask-fold')}><Icon name="chevronDown" className="size-4 rotate-180" /></button>;
  const send = (go: () => unknown) => guard.answer(x.id, go);
  const stale = guard.stale;
  const staleWhy = stale && <p className="phone-card__why m-0">{t('fleet-stale-no-answer')}</p>;
  if (x.start) return <div id={`phone-ask-${x.id}`} tabIndex={-1} className="phone-card phone-card--slab">{fold && <div className="phone-fold__back">{fold}</div>}<StartApproval approval={x} start={x.start} busy={!guard.canAnswer(x.id)} onResolve={onResolve ? (ok, choice) => send(() => onResolve(x, ok, choice)) : undefined} />{onResolve && staleWhy}</div>;
  // ADR-0090 Amendment A: a switch to a mode that skips permissions is read in the node's own rendering, never as raw text. Its answer
  // returns how it went, so the window's holds free it again when it failed.
  if (x.skip || x.skipUnknown) return <div id={`phone-ask-${x.id}`} tabIndex={-1} className="phone-card phone-card--slab">{fold && <div className="phone-fold__back">{fold}</div>}<SkipApproval approval={x} busy={stale} onResolve={onResolve ? (ok) => onResolve(x, ok) : undefined} />{onResolve && staleWhy}</div>;
  const name = agent?.name || approvalName(x);
  const why = `phone-why-${x.id}`;
  const sent = guard.status(x.id) === 'sent';
  const off = { disabled: stale, 'aria-disabled': !guard.canAnswer(x.id) || undefined, 'aria-describedby': stale ? why : undefined };
  return (
    <div id={`phone-ask-${x.id}`} tabIndex={-1} className="phone-req">
      <ApprovalRequest density="row" stacked icon={ASK_ICON[askKind(x)]}
        avatar={<AgentAvatar accent={agent?.accent || 'var(--agent-slate)'} kind={kindOf({ kind: agent?.kind, name })} size="s" />}
        title={askTitle(x, visible(name))} where={<bdi>{visible(machine)}</bdi>}
        titleEnd={fold || onOpen && <button type="button" className="phone-iconbtn phone-req__open" onClick={onOpen} aria-label={t('approval-open-agent', { name })} title={t('approval-open-agent', { name })}><Icon name="chevron" className="size-4 rtl:rotate-180" /></button>}
        actions={onResolve && !docked && <AnswerButtons approval={x} off={off} alwaysNote={alwaysNoteId(x, 'phone')}
          onDeny={() => send(() => onResolve(x, false))} onAllow={() => send(() => onResolve(x, true))} onAlways={() => send(() => onResolve(x, true, undefined, true))} />}
        foot={onResolve && (docked || x.always || stale || sent) ? <>
          {docked && <p className="phone-card__why m-0">{t('phone-answer-below')}</p>}
          {!docked && x.always && <AlwaysNote approval={x} surface="phone" />}
          {/* A docked card's reason is the bar's, beside its disabled buttons. */}
          {stale && !docked && <p id={why} className="phone-card__why m-0">{t('fleet-stale-no-answer')}</p>}
          {sent && <p role="status" className="phone-card__why m-0">{t('approval-sent')}</p>}
        </> : undefined}>
        <p className="approval-req__what"><span className="approval-req__cmd" dir="auto">{visible(x.action)}</span></p>
        {x.detail && <pre className="phone-card__detail">{visible(x.detail)}</pre>}
        {/* ADR-0101: "Allow always" only when the request offers it; in the place of Allow (the primary) when it offers nothing else. */}
        {!docked && <AlwaysOnly approval={x} className="mt-1" />}
      </ApprovalRequest>
    </div>
  );
}

// The phone's request cards for one screen, under one guard (one dwell for the list, the window's holds). `around`: whatever above the
// cards can move them (a notice, the trust callout, the link row), so a change there restarts the dwell too.
function useAskGuard(asks: readonly PhoneAsk[], stale: boolean, around: unknown): AnswerGuard {
  return useAnswerGuard(asks.map((a) => a.approval.id), (status) => asksShown(asks, status, around), stale);
}

// The machines that have said they do not trust this device, among those that ask (ADR-0061), named once at the top of the list instead
// of on every card; the machine still decides either way.
function TrustCallout({ machines }: { machines: readonly string[] }) {
  if (machines.length === 0) return null;
  const list = new Intl.ListFormat(document.documentElement.lang || undefined, { type: 'conjunction' }).format(machines.map(visible));
  return (
    <Callout tone="info" icon="lock" title={t('phone-untrusted-title')}>
      <p className="m-0">{t('phone-untrusted-machines', { machines: list, count: machines.length })}</p>
      <a className="kit-btn kit-btn--sm mt-2" href="#/device">{t('device-link')}</a>
    </Callout>
  );
}

const bring = (id: string) => document.getElementById(id)?.scrollIntoView?.({ block: 'center', behavior: 'smooth' });

// How long a newly shown request keeps Allow and Deny off: the window's one dwell (T44), as every other surface.
export const DOCK_DWELL_MS = ATTENTION_DWELL_MS;

// What needs you, reachable with one thumb at the bottom of the screen (above the bar on the list). A plain yes or no that is short
// enough to be read whole here (the fleet console's rule, `decidableInline`) has Deny and Allow; anything else has one button that brings
// its full card into view or opens the agent that asks. The bar is a surface of its own under the shared guard (its dwell restarts with
// anything it shows, its answers are held with the window), and it answers one request at a time: while its answer goes, the next request
// it shows waits for it. Off with its reason while the link is down; the same signing answers.
function NeedsDock({ dock, count, machines, stale, onResolve, onOpen }: { dock: Dock; count: number; machines: readonly MachineModel[]; stale: boolean; onResolve?: Resolve; onOpen: (id: string) => void }) {
  const x = dock.kind === 'answer' ? dock.approval : undefined;
  const id = x?.id;
  const who = dock.kind === 'answer' || dock.kind === 'review' ? (dock.agent?.name ?? approvalName(dock.approval)) : dock.kind === 'agent' ? dock.agent.name : t('pair-section', { count: dock.count });
  const where = dock.kind === 'answer' || dock.kind === 'review' ? (dock.agent?.machine || machineName(machines, dock.approval.node)) : dock.kind === 'agent' ? dock.agent.machine : '';
  const line = dock.kind === 'answer' ? askLine(dock.approval.action, dock.approval.detail) : '';
  const guard = useAnswerGuard(id ? [id] : [], (status) => JSON.stringify([dock.kind, id ?? '', who, where, line, id ? status(id) : '']), stale);
  // One at a time: set at once by the press that sent (a ref, so two presses in one batch see it), cleared when that answer settles.
  const going = useRef(false);
  const [busy, setBusy] = useState(false);
  const answer = (ok: boolean) => {
    if (!x || !onResolve || going.current) return;
    guard.answer(x.id, () => {
      going.current = true;
      setBusy(true);
      const done = () => { going.current = false; setBusy(false); };
      let r: unknown;
      try { r = onResolve(x, ok); } catch (e) { done(); throw e; }
      Promise.resolve(r).then(done, done);
      return r;
    });
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
  const why = 'phone-dock-why';
  const sent = !!id && guard.status(id) === 'sent';
  const off = { disabled: stale || !onResolve, 'aria-disabled': busy || !(id && guard.canAnswer(id)) || undefined, 'aria-describedby': stale ? why : undefined };
  return (
    <section ref={box} tabIndex={-1} role="status" aria-live="polite" aria-atomic="true" className="phone-dock" aria-label={t('phone-needs', { count })}
      onFocus={() => { had.current = true; }} onBlur={(e) => { if (e.relatedTarget && !e.currentTarget.contains(e.relatedTarget as Node)) had.current = false; }}>
      <div className="phone-dock__head">
        <span className="phone-dock__title">{t('attention-title')}<Pill tone="needs"><span className="sr-only">· </span>{count}</Pill></span>
        {/* The request the bar answers says who and where in its own card; anything else says it here. */}
        {dock.kind !== 'answer' && <span className="min-w-0 flex-1 truncate text-caption text-muted">{visible(who)}{where ? ` · ${visible(where)}` : ''}</span>}
      </div>
      {/* The one request component in its row density, stacked (redesign PR 13): the ask whole, then Deny and Allow, 52 px. */}
      {dock.kind === 'answer' && <ApprovalRequest density="row" stacked className="phone-dock__req" icon={ASK_ICON[askKind(dock.approval)]}
        avatar={<AgentAvatar accent={dock.agent?.accent || 'var(--agent-slate)'} kind={kindOf({ kind: dock.agent?.kind, name: who })} size="s" />}
        title={askTitle(dock.approval, visible(who))} where={where ? <bdi>{visible(where)}</bdi> : undefined}
        actions={onResolve && <AnswerButtons approval={dock.approval} off={off} onDeny={() => answer(false)} onAllow={() => answer(true)} />}
        foot={!onResolve
          // ADR-0106: a viewer's bar shows the request with no answers (the hub would refuse them), and says why.
          ? <p className="phone-card__why m-0" data-watching>{t('approval-watching')}</p>
          : stale || sent ? <>
            {stale && <p id={why} className="phone-card__why m-0">{t('fleet-stale-no-answer')}</p>}
            {sent && <p className="phone-card__why m-0">{t('approval-sent')}</p>}
          </> : undefined}>
        <p className="approval-req__what"><span className="approval-req__cmd" dir="auto">{visible(line)}</span></p>
      </ApprovalRequest>}
      {dock.kind === 'review' && <Button className="phone-dock__btn" onClick={() => (dock.agent ? onOpen(dock.agent.id) : bring(`phone-ask-${dock.approval.id}`))}>{t('fleet-review')}</Button>}
      {dock.kind === 'agent' && <Button className="phone-dock__btn" onClick={() => onOpen(dock.agent.id)}>{t('action-open')}</Button>}
      {dock.kind === 'pair' && <Button className="phone-dock__btn" onClick={() => bring('phone-pair')}>{t('fleet-review')}</Button>}
    </section>
  );
}

// One agent in a list: its avatar (who it is), its name and where it works, then its state as a pill.
function AgentRow({ agent: a, onOpen }: { agent: AgentTileModel; onOpen: (id: string) => void }) {
  return (
    <li>
      <button type="button" className={`phone-row${a.closed ? ' phone-row--closed' : ''}`} onClick={() => onOpen(a.id)}>
        <AgentAvatar accent={a.accent || 'var(--agent-slate)'} kind={kindOf({ kind: a.kind, name: a.name })} needsYou={a.state === 'needs-you' && !a.closed} />
        <span className="min-w-0 flex-1">
          <span className="phone-row__name block truncate font-medium text-ink">{a.name}</span>
          <span className="block truncate text-caption text-muted">{a.project || a.workspace}{a.branch ? ` · ${a.branch}` : ''}</span>
        </span>
        <NoAskBadge agent={a} />
        <AsksLessChip agent={a} />
        <FinishedMark agentId={a.id} />
        <StatusPill state={a.state} failed={endedWithError(a)} />
        <Icon name="chevron" className="size-4 flex-none text-muted rtl:rotate-180" />
      </button>
    </li>
  );
}

export interface PhoneCommon {
  approvals: readonly ApprovalModel[];
  machines: readonly MachineModel[];
  // Left out in a viewer's window (ADR-0106): every request is shown read-only, with no answers.
  onResolve?: Resolve;
  // ADR-0061: whether a machine is known not to trust this browser's key yet (it reported whom it trusts, and not this one). Unknown
  // is not said: the machine decides either way.
  untrusted?: (node: string) => boolean;
  notice?: string;
}

const machineName = (machines: readonly MachineModel[], id: string) => machines.find((m) => m.id === id)?.name || id;

// The list: what needs you first, then every machine with its agents. Tapping an agent opens it.
export function PhoneHome({ agents, machines, approvals, onResolve, untrusted, notice, link, onRetry, onOpen, onStart, onCommand, onLock, start, pairing, guest, watching }: PhoneCommon & {
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
  // The hub said this session is not the owner's: an empty window says no project is theirs yet (#904).
  guest?: boolean;
  watching?: boolean;
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
  const docked = dock?.kind === 'answer' ? dock.approval.id : undefined;
  const whereOf = (x: ApprovalModel, agent?: AgentTileModel) => agent?.machine || x.machineName || machineName(machines, x.node);
  const asks = needs.flatMap((n): PhoneAsk[] => (n.kind === 'approval' ? [{ approval: n.approval, agent: n.agent, machine: whereOf(n.approval, n.agent), docked: n.approval.id === docked }] : []));
  const byId = new Map(asks.map((a) => [a.approval.id, a]));
  const distrusted = [...new Set(asks.filter((a) => untrusted?.(a.approval.node)).map((a) => a.machine))];
  // Everything above the cards that can move them: the notice, the link row, the counts' filter, requests to join, the trust callout, and
  // the agents that wait among the cards.
  const guard = useAskGuard(asks, stale, [notice ?? '', link, filter ?? '', pairing?.cards.length ?? 0, pairing?.recent.length ?? 0, !!pairing?.window?.open, empty, distrusted,
    needs.map((n) => (n.kind === 'agent' ? n.agent.id : n.approval.id))]);
  return (
    <PhoneShell label={t('home-title')}
      dock={dock && !empty && <NeedsDock dock={dock} count={count} machines={machines} stale={stale} onResolve={onResolve} onOpen={onOpen} />}
      top={<>
        <span aria-hidden className="grid size-7 flex-none place-items-center rounded-xs bg-accent text-xs font-bold text-on-accent">A</span>
        <h1 className="phone-title">{t('home-title')}</h1>
        <ThemePicker />
        {onLock && <button type="button" className="phone-iconbtn" onClick={onLock} aria-label={t('lock-now')} title={t('lock-now-hint')}><Icon name="lock" className="size-5" /></button>}
      </>}
      bottom={<>
        <NavButton icon="search" label={t('phone-search')} onClick={onCommand} />
        <NavButton icon="plus" label={t('phone-start')} onClick={onStart} primary />
        <NavButton icon="stack" label={t('phone-machines')} href="#/machines" />
        <NavButton icon="settings" label={t('device-link')} href="#/device" />
        {/* The deep subjects the side panel holds on a larger screen, each a screen of its own here. */}
        <Menu label={t('phone-more')} items={moreItems} className="phone__nav" align="end"><Icon name="more" className="size-5" /><span className="truncate">{t('phone-more')}</span></Menu>
      </>}>
      {notice && <div className="phone-notice"><Callout tone="danger" icon="warn" title={notice} alert /></div>}
      {/* The state of the link has a row of its own (in the title bar it pushed the buttons off a 390 px screen): one notice, what it means and Retry. */}
      {link !== 'live' && <div role="status" className="phone-link"><span className="min-w-0 flex-1">{t(link === 'offline' ? (empty ? 'link-offline' : 'fleet-offline-stale') : 'link-connecting')}</span>{link === 'offline' && onRetry && <Button size="sm" onClick={onRetry}>{t('link-retry')}</Button>}</div>}
      {!empty && <div className="phone-fleet"><FleetSummary counts={counts} filter={filter} onFilter={setFilter} /></div>}
      {pairing && pairing.cards.length > 0 && <section id="phone-pair" className="phone-section" aria-label={t('attention-title')}>
        <h2 className="phone-section__title">{t('pair-section', { count: pairing.cards.length })}</h2>
        <div className="grid gap-2"><PairCards ui={pairing} /></div>
      </section>}
      {empty && <FleetEmpty kind={empty} machines={machines} onStart={onStart ? () => onStart() : undefined} onRetry={onRetry} pairing={pairing && (empty === 'empty' || empty === 'first-run') && link === 'live' ? pairing : undefined} guest={guest} watching={watching} />}
      {pairing && <div className="px-3"><PairRecentList recent={pairing.recent} /></div>}
      {pairing?.window?.open && !empty && <div className="px-3"><PairWindowControl ui={pairing} /></div>}
      {needs.length > 0 && <section className="phone-section" aria-label={t('attention-title')}>
        <h2 className="phone-section__title">{t('attention-title')}<Pill tone="needs"><span className="sr-only">· </span>{needs.length}</Pill></h2>
        <TrustCallout machines={distrusted} />
        <div className="grid gap-2">
          {needs.map((n) => n.kind === 'approval'
            // `byId` is built from these same items, so every request listed here is in it.
            ? <PhoneRequest key={n.approval.id} ask={byId.get(n.approval.id)!} guard={guard}
              onResolve={onResolve} onOpen={n.agent ? () => onOpen(n.approval.agentId) : undefined} />
            : <ul key={n.agent.id} className="phone-list phone-list--needs"><AgentRow agent={n.agent} onOpen={onOpen} /></ul>)}
        </div>
      </section>}
      {groups.length === 0 && !empty && <p className="muted m-0 p-4">{t(filter ? 'fleet-filter-none' : 'ide-welcome-none')}</p>}
      {groups.map((g) => (
        <section key={g.id} className="phone-section" aria-label={g.name}>
          <h2 className="phone-section__title">
            <span aria-hidden className={`size-2 flex-none rounded-full ${g.online ? 'bg-finished' : 'bg-unknown'}`} />
            <span className="min-w-0 flex-1 truncate">{g.name}</span>
            {!g.online && <Tag>{t('phone-offline')}</Tag>}
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
  const docked = dock?.kind === 'answer' ? dock.approval.id : undefined;
  const whereOf = (x: ApprovalModel) => agent?.machine || x.machineName || machineName(machines, x.node);
  // While the bar at the bottom holds a request, the cards above fold to one line each, so the terminal or the conversation keeps its
  // room (visual pass 2026-11, finding 16): the bar answers a plain yes or no, and its Review and decide (or a tap on the line) unfolds
  // the card. Unfolded, a card can be folded again.
  const [unfolded, setUnfolded] = useState<ReadonlySet<string>>(() => new Set());
  const folds = !!dock && !!agent && !agent.closed;
  const asks = mine.map((approval): PhoneAsk => ({ approval, agent, machine: whereOf(approval), docked: approval.id === docked, folded: folds && !unfolded.has(approval.id) }));
  // Open or fold one card; what had focus (the line or the fold button, which go away) moves to the card itself.
  const unfold = (id: string | undefined, open: boolean) => {
    if (!id) return;
    setUnfolded((cur) => { const next = new Set(cur); if (open) next.add(id); else next.delete(id); return next; });
    window.setTimeout(() => { const el = document.getElementById(`phone-ask-${id}`); el?.focus({ preventScroll: true }); el?.scrollIntoView?.({ block: 'nearest', behavior: 'smooth' }); }, 0);
  };
  const distrusted = [...new Set(asks.filter((a) => untrusted?.(a.approval.node)).map((a) => a.machine))];
  // Everything above the cards that can move them: the notice, the close confirmation (open or not), the tools row and the trust callout.
  const closing = useCloseAsk();
  const guard = useAskGuard(asks, stale, [notice ?? '', !!agent?.closed, !!agent && closing?.id === agent.id && closing.where === 'pane', changed > 0 && !!loadDiff, !!files, distrusted]);
  return (
    <PhoneShell label={agent?.name ?? t('focus-not-found')}
      dock={agent && dock && !agent.closed && <NeedsDock dock={dock} count={mine.length} machines={machines} stale={stale} onResolve={onResolve} onOpen={(id) => (id === agent.id ? unfold(mine[0]?.id, true) : onOpen(id))} />}
      top={<>
        <button type="button" className="phone-iconbtn" onClick={onBack} aria-label={t('phone-back')} title={t('phone-back')}><Icon name="chevron" className="size-5 rotate-180 rtl:rotate-0" /></button>
        <div className="min-w-0 flex-1">
          <h1 className="phone-title">{agent?.name ?? t('focus-not-found')}</h1>
          {agent && <span className="flex min-w-0 items-center gap-2 text-caption text-muted"><StatusPill state={agent.state} failed={endedWithError(agent)} /><span className="truncate">{agent.machine}</span></span>}
        </div>
        {agent && <NoAskBadge agent={agent} />}
        {agent && <AsksLessChip agent={agent} />}
        {agent && <CloseAgentMenu agent={agent} />}
        {next && <Button className="phone-next" onClick={() => onOpen(next)}>{t('phone-next', { count: elsewhere.length })}</Button>}
      </>}>
      {notice && <div className="phone-notice"><Callout tone="danger" icon="warn" title={notice} alert /></div>}
      {!agent ? <div className="grid justify-items-start gap-3 p-4"><p className="m-0">{t('focus-not-found')}</p><button type="button" className="button" onClick={onBack}>{t('phone-back')}</button></div> : <>
        <CloseConfirm agent={agent} where="pane" />
        {!agent.closed && (changed > 0 && loadDiff || files) && (
          <div className="phone-tools">
            {changed > 0 && loadDiff && <Button size="sm" onClick={() => setShown('changes')}><Icon name="diff" className="size-4" />{t('changes-files', { count: changed })}</Button>}
            {files && <Button size="sm" onClick={() => setShown('files')}><Icon name="files" className="size-4" />{t('console-files')}</Button>}
          </div>
        )}
        {mine.length > 0 && <div className="phone-asks"><TrustCallout machines={distrusted} />{asks.map((a) => <PhoneRequest key={a.approval.id} ask={a} guard={guard} onResolve={onResolve} onFold={folds ? (folded) => unfold(a.approval.id, !folded) : undefined} />)}</div>}
        {agent.closed ? <div className="min-h-0 flex-1 overflow-y-auto"><ClosedSummary agent={agent} now={now} source={agent.terminal === false ? chatSource : undefined} onReview={loadDiff ? () => setShown('changes') : undefined} onFiles={files ? () => setShown('files') : undefined} onDismiss={() => { dismiss([agent.id]); onBack(); }} /></div>
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

// The side panel's deep subjects, reached from More in the bottom bar: the board, Team chat and Usage.
const DETAILS_LABEL: Record<PhoneDetailsTab, string> = { board: 'board-title', messages: 'ide-tab-messages', usage: 'ide-tab-usage' };
const DETAILS_ICON: Record<PhoneDetailsTab, IconName> = { board: 'board', messages: 'chat', usage: 'gauge' };
const moreItems = (): MenuItem[] => (Object.keys(PHONE_DETAILS) as PhoneDetailsTab[]).map((tab) => ({
  label: t(DETAILS_LABEL[tab]), lead: <Icon name={DETAILS_ICON[tab]} className="size-4 text-muted" />, run: () => { window.location.hash = PHONE_DETAILS[tab]; },
}));

// One of them on a phone: a screen with Back, as the theme editor. It draws what the side panel draws for that subject.
export function PhoneDetails({ tab, agents, boards, messages, onOpen, onBack }: {
  tab: PhoneDetailsTab;
  agents: readonly AgentTileModel[];
  boards: readonly BoardModel[];
  messages: readonly MessageModel[];
  onOpen: (id: string) => void;
  onBack: () => void;
}) {
  const title = t(DETAILS_LABEL[tab]);
  return (
    <PhoneShell label={title}
      top={<>
        <button type="button" className="phone-iconbtn" onClick={onBack} aria-label={t('phone-back')} title={t('phone-back')}><Icon name="chevron" className="size-5 rotate-180 rtl:rotate-0" /></button>
        <h1 className="phone-title">{title}</h1>
      </>}>
      <div className="phone-details"><Details tab={tab} fleet={agents} attention={[]} approvals={[]} changed={[]} boards={boards} messages={messages} onOpen={onOpen} /></div>
    </PhoneShell>
  );
}

// The theme editor (ADR-0068 rung 2) on a phone: a screen with Back, since the phone has no panels.
export function PhoneTheme({ onBack }: { onBack: () => void }) {
  return (
    <PhoneShell label={t('theme-editor-title')}
      top={<>
        <button type="button" className="phone-iconbtn" onClick={onBack} aria-label={t('phone-back')} title={t('phone-back')}><Icon name="chevron" className="size-5 rotate-180 rtl:rotate-0" /></button>
        <h1 className="phone-title">{t('theme-editor-title')}</h1>
      </>}>
      <ThemeEditor />
    </PhoneShell>
  );
}
