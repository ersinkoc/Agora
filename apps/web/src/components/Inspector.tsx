import { useContext, useEffect, useRef, type KeyboardEvent as ReactKeyboardEvent } from 'react';
import { needsPanel } from '../net/needsPanel';
import { t } from '../i18n';
import type { AgentTileModel } from '../model';
import { visible } from '../net/hidden';
import type { UsageSummary } from '../net/usage';
import { hasUsage } from '../net/usageView';
import { UsagePanel } from './UsagePanel';
import type { ApprovalModel, BoardModel, MessageModel, ProjectCost } from '../net/view';
import { ApprovalCard } from './ApprovalCard';
import { requestShown, useAnswerGuard, type AnswerResult } from './useAnswerGuard';
import type { StartChoice } from './StartApproval';
import { EmptyState } from './EmptyState';
import { Icon } from './Icon';
import { SharedBoards } from './SharedBoards';
import { StateBadge } from './StateBadge';
import { ChannelNotices, TeamChat } from './TeamChat';

import { PairCards, type PairingUi } from './Pairing';
import { pathParts } from '../net/paneTools';
import { badgeText, railItems, railLabel, resolveTab, type RailTab } from './rail';
import { useCommands } from '../keys/registry';
import { ariaKey, shownKey, useKeymap } from '../keys/view';

type Tab = RailTab;

// The right panel: an icon rail that is always there (it stays visible at every width) and the subject it opens beside it, one at a
// time (at most two panels are ever visible, visual-language.md). On a wide window the subject sits in the layout; on a narrow one it
// is a drawer over the content, so the rail never clips and nothing is pushed out of view.
export function Inspector({ fleet = [], now = Date.now(), attention, approvals, notice, changed, boards, messages, usage, projects, onOpen, onResolve, stale, onReview, onPane, open, onOpenChange, tab, onTab, pairing }: {
  fleet?: readonly AgentTileModel[];
  now?: number;
  attention: readonly AgentTileModel[];
  approvals: readonly ApprovalModel[];
  notice?: string;
  changed: readonly AgentTileModel[];
  boards: readonly BoardModel[];
  messages: readonly MessageModel[];
  usage?: UsageSummary;
  projects?: readonly ProjectCost[];
  onOpen: (id: string) => void;
  // Left out in a viewer's window (ADR-0106): the requests are shown read-only.
  onResolve?: (a: ApprovalModel, approved: boolean, choice?: StartChoice, always?: boolean) => AnswerResult;
  // The link is not live: no request can be answered here (T44: the dwell starts again when it is back).
  stale?: boolean;
  onReview?: (id: string) => void;
  // Open an agent's changes as a pane of the window, beside what is there.
  onPane?: (id: string) => void;
  // Whether the subject is shown beside the rail (remembered per device by the window), and which one.
  open: boolean;
  onOpenChange: (open: boolean) => void;
  tab: RailTab;
  // `remember` false: the window moved there by itself (something needs you), so the subject this device chose stays the saved one.
  onTab: (tab: RailTab, remember?: boolean) => void;
  // ADR-0065: machines asking to join, answered in their full cards here.
  pairing?: PairingUi;
}) {
  const needs = attention.length + approvals.filter((x) => !attention.some((a) => a.id === x.agentId)).length + (pairing?.cards.length ?? 0);
  const items = railItems({ needs, changed: changed.length, messages: messages.length, hasBoard: boards.length > 0, hasUsage: hasUsage(fleet) });
  const active = resolveTab(tab, items);
  const label = t(items.find((x) => x.id === active)?.labelKey ?? 'attention-title');
  // Closing returns focus to the rail icon (the drawer is where focus went when it opened); opening by the person puts focus in the panel.
  const panel = useRef<HTMLElement>(null);
  const focusPanel = useRef(false);
  const close = () => { onOpenChange(false); document.getElementById(`rail-${active}`)?.focus(); };
  const press = (id: RailTab) => { if (open && id === active) close(); else { focusPanel.current = true; onTab(id); onOpenChange(true); } };
  useEffect(() => { if (open && focusPanel.current) panel.current?.focus(); focusPanel.current = false; }, [open]);
  // Something new that needs the user wins over a subject that is quietly open; one that goes away does not pull the user back.
  const seen = useRef(needs);
  useEffect(() => { if (needs > seen.current) onTab('needs', false); seen.current = needs; }, [needs]); // eslint-disable-line react-hooks/exhaustive-deps
  const rail = useRef<HTMLElement>(null);
  // The side panel's commands (docs/ux/keybindings.md): Alt+Shift+1 to 5 by default open the subject at that place of the rail; the
  // same key again closes the panel.
  useCommands(Object.fromEntries(Array.from({ length: 5 }, (_, i) => {
    const item = items[i];
    return [`side.place-${i + 1}`, { run: () => { if (item) press(item.id); }, available: !!item, label: item ? t('keys-cmd-side-named', { name: t(item.labelKey) }) : undefined }];
  })));
  const keymap = useKeymap();
  // The arrow keys walk the rail (a vertical tab list: Up and Down, Home and End); Escape closes the drawer and returns to its icon.
  const railKey = (e: ReactKeyboardEvent<HTMLElement>) => {
    const tabs = [...(rail.current?.querySelectorAll<HTMLElement>('[role=tab]') ?? [])];
    const at = tabs.indexOf(document.activeElement as HTMLElement);
    if (e.key === 'Escape' && open) { e.preventDefault(); e.stopPropagation(); close(); return; }
    const to = e.key === 'ArrowDown' ? (at + 1) % tabs.length : e.key === 'ArrowUp' ? (at - 1 + tabs.length) % tabs.length : e.key === 'Home' ? 0 : e.key === 'End' ? tabs.length - 1 : undefined;
    if (to !== undefined) { e.preventDefault(); tabs[to]?.focus(); }
  };

  return <aside className="inspector" aria-label={t('console-activity')} data-open={open}>
    {open && <section ref={panel} tabIndex={-1} className="inspector__panel" id="inspector-panel" role="tabpanel" aria-labelledby={`rail-${active}`}
      onKeyDown={(e) => { if (e.key === 'Escape') { e.stopPropagation(); close(); } }}>
      <div className="flex h-9 flex-none items-center justify-between gap-2 border-b border-line ps-3 pe-1">
        <h2 className="panel-title">{label}</h2>
        <button type="button" className="ide-iconbtn" onClick={close} aria-label={t('ide-hide-panel')} title={t('ide-hide-panel')}><Icon name="close" /></button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto p-3">
        <Details tab={active} fleet={fleet} now={now} pairing={pairing} attention={attention} approvals={approvals} notice={notice} changed={changed} boards={boards} messages={messages} usage={usage} projects={projects} onOpen={onOpen} onResolve={onResolve} stale={stale} onReview={onReview} onPane={onPane} />
      </div>
    </section>}
    <nav ref={rail} className="rail" role="tablist" aria-orientation="vertical" aria-label={t('ide-details')} onKeyDown={railKey}>
      {items.map((x, i) => {
        const text = t(x.labelKey);
        const selected = open && active === x.id;
        const count = x.badge ? ` (${x.badge})` : '';
        const key = shownKey(keymap, `side.place-${i + 1}`);
        return <button key={x.id} id={`rail-${x.id}`} type="button" role="tab" aria-selected={selected} aria-controls={selected ? 'inspector-panel' : undefined} aria-label={railLabel(text, x.badge)}
          aria-keyshortcuts={ariaKey(keymap, `side.place-${i + 1}`)} tabIndex={selected || (!open && active === x.id) ? 0 : -1} data-tip={key ? `${text}${count} · ${key}` : `${text}${count}`}
          className="rail__btn" onClick={() => press(x.id)}>
          <Icon name={x.icon} className="size-[1.125rem]" />
          {!!x.badge && <span aria-hidden className={`rail__badge${x.alert ? ' rail__badge--alert' : ''}`}>{badgeText(x.badge)}</span>}
        </button>;
      })}
      <span className="flex-1" />
      <button type="button" className="rail__btn" onClick={() => onOpenChange(!open)} aria-expanded={open} aria-controls={open ? 'inspector-panel' : undefined}
        aria-label={t(open ? 'ide-hide-panel' : 'ide-show-details')} data-tip={t(open ? 'ide-hide-panel' : 'ide-show-details')}>
        <Icon name="panelRight" className="size-[1.125rem]" />
      </button>
    </nav>
  </aside>;
}

// One subject of the details, on its own: the Inspector's tabs and the layout's panels (ADR-0068) draw the same thing.
export type DetailsTab = Tab;
export function Details({ tab, fleet = [], now = Date.now(), attention, approvals, notice, changed, boards, messages, usage, projects, onOpen, onResolve, stale, onReview, onPane, pairing }: {
  tab: Tab;
  fleet?: readonly AgentTileModel[];
  now?: number;
  attention: readonly AgentTileModel[];
  approvals: readonly ApprovalModel[];
  notice?: string;
  changed: readonly AgentTileModel[];
  boards: readonly BoardModel[];
  messages: readonly MessageModel[];
  usage?: UsageSummary;
  projects?: readonly ProjectCost[];
  onOpen: (id: string) => void;
  // Left out in a viewer's window (ADR-0106): the requests are shown read-only.
  onResolve?: (a: ApprovalModel, approved: boolean, choice?: StartChoice, always?: boolean) => AnswerResult;
  // The link is not live: no request can be answered here (T44: the dwell starts again when it is back).
  stale?: boolean;
  onReview?: (id: string) => void;
  onPane?: (id: string) => void;
  pairing?: PairingUi;
}) {
  const needs = attention.length + approvals.filter((x) => !attention.some((a) => a.id === x.agentId)).length + (pairing?.cards.length ?? 0);
  // While this list is on screen the attention toast is hidden: it would be a third copy of what is here.
  useEffect(() => (tab === 'needs' ? needsPanel.show() : undefined), [tab]);
  const notices = useContext(ChannelNotices);
  // An agent is listed once: by its request's card when it has one (the card opens the agent: a start's and a switch's too, redesign PR 5),
  // else by its row (it waits in its terminal). A switch's card of a shape this window does not understand can be dismissed here, so its
  // agent keeps its row.
  const carded = (id: string) => approvals.some((x) => x.agentId === id && !x.skipUnknown);
  const agentOf = (id: string) => fleet.find((a) => a.id === id) ?? attention.find((a) => a.id === id);
  // T44 (`useAnswerGuard`, as Home's queue): the cards' answers do nothing for a short dwell after anything on this list moved or changed
  // (the rows, the notice and the machines asking to join above the cards; a card's request, agent and status), after the person came
  // back and after the link came back; a request is answered once.
  const guard = useAnswerGuard(approvals.map((a) => a.id), (status) => JSON.stringify([tab, attention.filter((a) => !carded(a.id)).map((a) => [a.id, a.name, a.state]), notice ?? '',
    pairing?.cards.map((c) => c.id) ?? [], approvals.map((a) => [requestShown(a, agentOf(a.agentId)), status(a.id)])]), !!stale);
  return <>
    {tab === 'needs' && <div className="grid gap-2">
        {needs === 0 && <p className="muted">{t('attention-empty')}</p>}
        {attention.filter((a) => !carded(a.id)).map(a => <button key={a.id} type="button" className="ide-row justify-between border border-needs-you/60 px-3 py-2" onClick={() => onOpen(a.id)}><span className="truncate font-medium">{visible(a.name)}</span><StateBadge state={a.state} /></button>)}
        {notice && <p role="alert" className="text-danger">{notice}</p>}
        {pairing && <PairCards ui={pairing} />}
        {approvals.map(a => { const agent = agentOf(a.agentId); return <ApprovalCard key={a.id} approval={a} agent={agent} guard={guard} onOpen={agent ? () => onOpen(agent.id) : undefined} onResolve={onResolve ? (ok, choice, always) => onResolve(a, ok, choice, always) : undefined} />; })}
      </div>}
    {tab === 'changes' && (changed.length === 0 ? <EmptyState icon="check" title={t('empty-changes-title')} hint={t('empty-changes-hint')} /> : <ul className="m-0 grid grid-cols-1 list-none gap-1 p-0">
        {changed.map(a => <li key={a.id} className="flex items-center gap-1">
          <button type="button" className="ide-row min-w-0 flex-1 justify-between gap-3 py-1.5" onClick={() => (onReview ? onReview(a.id) : onOpen(a.id))} title={`${visible(a.workspace)} · ${visible(a.machine)}`}>
            <span className="min-w-0">{a.shared ? <span className="block font-medium"><FolderPath path={visible(a.workspace)} /></span> : <span className="block truncate font-medium">{visible(a.name)}</span>}<span className="block truncate text-xs text-muted">{a.shared ? `${visible(a.machine)} · ${t('workspace-shared')}` : `${visible(a.machine)} · ${visible(a.workspace)}`}</span></span>
            <span className="kit-tag flex-none">{t('changes-files', { count: a.filesChanged ?? 0 })}</span>
          </button>
          {onPane && <button type="button" className="ide-iconbtn flex-none" onClick={() => onPane(a.id)} aria-label={t('pane-open-beside-named', { name: visible(a.shared ? a.workspace : a.name) })} title={t('pane-open-changes')}><Icon name="splitRight" /></button>}
        </li>)}
      </ul>)}
    {tab === 'board' && boards.length > 0 && <SharedBoards boards={boards} />}
    {tab === 'messages' && (messages.length === 0 && notices.length === 0 ? <EmptyState icon="chat" title={t('empty-messages-title')} hint={t('empty-messages-hint')} /> : <TeamChat messages={messages} notices={notices} fleet={fleet} now={now} />)}
    {tab === 'usage' && <UsagePanel fleet={fleet} now={now} />}
    {tab === 'board' && boards.length === 0 && <EmptyState icon="board" title={t('empty-board-title')} hint={t('empty-board-hint')} />}
  </>;
}

// A shared folder's path in its row, cut in the middle: what leads to the folder gives way first, so the folder's name stays in view
// (the row's tooltip has the whole path, and assistive tech reads it whole: the text is all there). A path reads left to right, also
// in a right-to-left language, so its parts keep their order.
function FolderPath({ path }: { path: string }) {
  const { dir, name } = pathParts(path);
  return <span dir="ltr" className="flex min-w-0"><span className="min-w-0 truncate">{dir}</span><span className="max-w-full flex-none truncate">{name}</span></span>;
}
