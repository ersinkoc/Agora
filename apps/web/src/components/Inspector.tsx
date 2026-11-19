import { useEffect, useRef, type KeyboardEvent as ReactKeyboardEvent } from 'react';
import { t } from '../i18n';
import type { AgentTileModel } from '../model';
import { money, type UsageSummary } from '../net/usage';
import type { ApprovalModel, BoardModel, MessageModel, ProjectCost } from '../net/view';
import { ApprovalCard } from './ApprovalCard';
import type { StartChoice } from './StartApproval';
import { Icon } from './Icon';
import { ProjectBoard } from './ProjectBoard';
import { StateBadge } from './StateBadge';

import { PairCards, type PairingUi } from './Pairing';
import { badgeText, railItems, resolveTab, shortcutIndex, shortcutLabel, type RailTab } from './rail';

type Tab = RailTab;

// The right panel: an icon rail that is always there (it stays visible at every width) and the subject it opens beside it, one at a
// time (at most two panels are ever visible, visual-language.md). On a wide window the subject sits in the layout; on a narrow one it
// is a drawer over the content, so the rail never clips and nothing is pushed out of view.
export function Inspector({ attention, approvals, notice, changed, boards, messages, usage, projects, onOpen, onResolve, onReview, open, onOpenChange, tab, onTab, pairing }: {
  attention: readonly AgentTileModel[];
  approvals: readonly ApprovalModel[];
  notice?: string;
  changed: readonly AgentTileModel[];
  boards: readonly BoardModel[];
  messages: readonly MessageModel[];
  usage?: UsageSummary;
  projects: readonly ProjectCost[];
  onOpen: (id: string) => void;
  onResolve: (a: ApprovalModel, approved: boolean, choice?: StartChoice) => void;
  onReview?: (id: string) => void;
  // Whether the subject is shown beside the rail (remembered per device by the window), and which one.
  open: boolean;
  onOpenChange: (open: boolean) => void;
  tab: RailTab;
  onTab: (tab: RailTab) => void;
  // ADR-0065: machines asking to join, answered in their full cards here.
  pairing?: PairingUi;
}) {
  const needs = attention.length + approvals.filter((x) => !attention.some((a) => a.id === x.agentId)).length + (pairing?.cards.length ?? 0);
  const items = railItems({ needs, changed: changed.length, messages: messages.length, hasBoard: boards.length > 0, hasCost: !!usage && usage.totalMicroUsd > 0 });
  const active = resolveTab(tab, items);
  const label = t(items.find((x) => x.id === active)?.labelKey ?? 'attention-title');
  const press = (id: RailTab) => { if (open && id === active) onOpenChange(false); else { onTab(id); onOpenChange(true); } };
  // Something new that needs the user wins over a subject that is quietly open; one that goes away does not pull the user back.
  const seen = useRef(needs);
  useEffect(() => { if (needs > seen.current) onTab('needs'); seen.current = needs; }, [needs]); // eslint-disable-line react-hooks/exhaustive-deps
  const rail = useRef<HTMLElement>(null);
  useEffect(() => {
    const on = (e: KeyboardEvent) => {
      const i = shortcutIndex(e);
      const item = i === undefined ? undefined : items[i];
      if (item) { e.preventDefault(); press(item.id); }
    };
    window.addEventListener('keydown', on);
    return () => window.removeEventListener('keydown', on);
  });
  // The arrow keys walk the rail (a vertical tab list: Up and Down, Home and End); Escape closes the drawer and returns to its icon.
  const railKey = (e: ReactKeyboardEvent<HTMLElement>) => {
    const tabs = [...(rail.current?.querySelectorAll<HTMLElement>('[role=tab]') ?? [])];
    const at = tabs.indexOf(document.activeElement as HTMLElement);
    const to = e.key === 'ArrowDown' ? (at + 1) % tabs.length : e.key === 'ArrowUp' ? (at - 1 + tabs.length) % tabs.length : e.key === 'Home' ? 0 : e.key === 'End' ? tabs.length - 1 : undefined;
    if (to !== undefined) { e.preventDefault(); tabs[to]?.focus(); }
  };

  return <aside className="inspector" aria-label={t('console-activity')} data-open={open}>
    {open && <section className="inspector__panel" id="inspector-panel" role="tabpanel" aria-labelledby={`rail-${active}`}
      onKeyDown={(e) => { if (e.key === 'Escape') { e.stopPropagation(); onOpenChange(false); document.getElementById(`rail-${active}`)?.focus(); } }}>
      <div className="flex h-9 flex-none items-center justify-between gap-2 border-b border-line ps-3 pe-1">
        <h2 className="m-0 truncate">{label}</h2>
        <button type="button" className="ide-iconbtn" onClick={() => onOpenChange(false)} aria-label={t('ide-hide-panel')} title={t('ide-hide-panel')}><Icon name="close" /></button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto p-3">
        <Details tab={active} pairing={pairing} attention={attention} approvals={approvals} notice={notice} changed={changed} boards={boards} messages={messages} usage={usage} projects={projects} onOpen={onOpen} onResolve={onResolve} onReview={onReview} />
      </div>
    </section>}
    <nav ref={rail} className="rail" role="tablist" aria-orientation="vertical" aria-label={t('ide-details')} onKeyDown={railKey}>
      {items.map((x, i) => {
        const text = t(x.labelKey);
        const selected = open && active === x.id;
        const count = x.badge ? ` (${x.badge})` : '';
        return <button key={x.id} id={`rail-${x.id}`} type="button" role="tab" aria-selected={selected} aria-controls={selected ? 'inspector-panel' : undefined} aria-label={text}
          aria-keyshortcuts={shortcutLabel(i)} tabIndex={selected || (!open && active === x.id) ? 0 : -1} data-tip={`${text}${count} · ${shortcutLabel(i)}`}
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
export function Details({ tab, attention, approvals, notice, changed, boards, messages, usage, projects, onOpen, onResolve, onReview, pairing }: {
  tab: Tab;
  attention: readonly AgentTileModel[];
  approvals: readonly ApprovalModel[];
  notice?: string;
  changed: readonly AgentTileModel[];
  boards: readonly BoardModel[];
  messages: readonly MessageModel[];
  usage?: UsageSummary;
  projects: readonly ProjectCost[];
  onOpen: (id: string) => void;
  onResolve: (a: ApprovalModel, approved: boolean, choice?: StartChoice) => void;
  onReview?: (id: string) => void;
  pairing?: PairingUi;
}) {
  // Every machine that has a folder on a board: a name is followed by its id only when two of them share it.
  const boardMachines = [...new Map(boards.flatMap((b) => b.folders).map((f) => [f.nodeId, { id: f.nodeId, name: f.machine }] as const)).values()];
  const needs = attention.length + approvals.filter((x) => !attention.some((a) => a.id === x.agentId)).length + (pairing?.cards.length ?? 0);
  return <>
    {tab === 'needs' && <div className="grid gap-2">
        {needs === 0 && <p className="muted">{t('attention-empty')}</p>}
        {attention.map(a => <button key={a.id} type="button" className="ide-row justify-between border border-needs-you/60 px-3 py-2" onClick={() => onOpen(a.id)}><span className="truncate font-medium">{a.name}</span><StateBadge state={a.state} /></button>)}
        {notice && <p role="alert" className="text-needs-you">{notice}</p>}
        {pairing && <PairCards ui={pairing} />}
        {approvals.map(a => <ApprovalCard key={a.id} approval={a} onResolve={(ok, choice) => onResolve(a, ok, choice)} />)}
      </div>}
    {tab === 'changes' && (changed.length === 0 ? <p className="muted">{t('changes-none')}</p> : <ul className="m-0 grid list-none gap-1 p-0">
        {changed.map(a => <li key={a.id}>
          <button type="button" className="ide-row justify-between gap-3 py-1.5" onClick={() => (onReview ? onReview(a.id) : onOpen(a.id))}>
            <span className="min-w-0"><span className="block truncate font-medium">{a.shared ? a.workspace : a.name}</span><span className="block truncate text-xs text-muted">{a.shared ? t('workspace-shared') : a.machine} · {a.workspace}</span></span>
            <span className="flex-none text-xs text-needs-you">{t('changes-files', { count: a.filesChanged ?? 0 })}</span>
          </button>
        </li>)}
      </ul>)}
    {tab === 'board' && <div className="grid gap-2">{boards.map(b => <ProjectBoard key={b.projectId} board={b} machines={boardMachines} />)}</div>}
    {tab === 'messages' && (messages.length === 0 ? <p className="muted">{t('console-no-messages')}</p> : <ul className="m-0 grid list-none gap-2 p-0">
        {messages.map((m, i) => <li key={`${m.atMs}-${i}`} className="rounded-xs border border-line bg-surface-0 p-2.5">
          <div className="text-xs"><strong>{m.from}</strong> <span className="inline-block rtl:-scale-x-100" aria-hidden="true">→</span> <strong>{m.to}</strong>{m.crossMachine ? <span className="ide-chip ms-1.5">{t('message-cross')}</span> : null}</div>
          <div className="mt-1 break-words text-muted">{m.body}</div>
        </li>)}
      </ul>)}
    {tab === 'cost' && usage && <div className="grid gap-2">
        <p className="m-0 text-2xl font-semibold tracking-tight">{money(usage.totalMicroUsd)}</p>
        <ul className="m-0 grid list-none gap-1 p-0">{projects.map(p => <li key={p.label} className="flex justify-between gap-2"><span className="truncate">{p.label}</span><span className="text-muted">{money(p.microUsd)}</span></li>)}</ul>
        <p className="muted">{usage.pricesAtMs ? t('usage-prices-date', { date: new Date(usage.pricesAtMs).toLocaleDateString() }) : t('usage-prices-bundled')}</p>
      </div>}
    {tab === 'board' && boards.length === 0 && <p className="muted">{t('layout-board-empty')}</p>}
    {tab === 'cost' && !usage && <p className="muted">{t('layout-cost-empty')}</p>}
  </>;
}
