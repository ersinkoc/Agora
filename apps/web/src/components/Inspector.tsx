import { useEffect, useRef, useState } from 'react';
import { t } from '../i18n';
import type { AgentTileModel } from '../model';
import { money, type UsageSummary } from '../net/usage';
import type { ApprovalModel, BoardModel, MessageModel, ProjectCost } from '../net/view';
import { ApprovalCard } from './ApprovalCard';
import type { StartChoice } from './StartApproval';
import { Icon } from './Icon';
import { ProjectBoard } from './ProjectBoard';
import { StateBadge } from './StateBadge';

type Tab = 'needs' | 'changes' | 'board' | 'messages' | 'cost';

// The right panel: one subject at a time, in tabs (at most two panels are ever visible, visual-language.md).
export function Inspector({ attention, approvals, notice, changed, boards, messages, usage, projects, onOpen, onResolve, onReview, onCollapse }: {
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
  onCollapse: () => void;
}) {
  const needs = attention.length + approvals.filter((x) => !attention.some((a) => a.id === x.agentId)).length;
  const tabs: { id: Tab; label: string; badge?: number }[] = [
    { id: 'needs', label: t('attention-title'), badge: needs },
    { id: 'changes', label: t('ide-tab-changes'), badge: changed.length },
    ...(boards.length > 0 ? [{ id: 'board' as const, label: t('board-title') }] : []),
    { id: 'messages', label: t('ide-tab-messages'), badge: messages.length },
    ...(usage && usage.totalMicroUsd > 0 ? [{ id: 'cost' as const, label: t('ide-tab-cost') }] : []),
  ];
  const [tab, setTab] = useState<Tab>('needs');
  // Something new that needs the user wins over a tab that is quietly open; one that goes away does not pull the user back.
  const seen = useRef(0);
  useEffect(() => { if (needs > seen.current) setTab('needs'); seen.current = needs; }, [needs]);
  const active = tabs.some(x => x.id === tab) ? tab : 'needs';

  return <aside className="flex min-h-0 flex-col border-line bg-surface-1 lg:border-s" aria-label={t('console-activity')}>
    <div className="flex h-9 flex-none items-stretch border-b border-line">
      <div role="tablist" aria-label={t('ide-details')} className="flex min-w-0 flex-1 overflow-x-auto quiet-scroll">
        {tabs.map(x => <button key={x.id} type="button" role="tab" aria-selected={active === x.id} onClick={() => setTab(x.id)}
          className="ide-tab max-w-none border-e-0 px-2.5 text-[12px] whitespace-nowrap">
          {x.label}{!!x.badge && <span className={`rounded-full px-1.5 text-[11px] ${x.id === 'needs' ? 'bg-needs-you text-on-accent' : 'bg-surface-2 text-muted'}`}>{x.badge}</span>}
        </button>)}
      </div>
      <button type="button" className="ide-iconbtn m-1" onClick={onCollapse} aria-label={t('ide-hide-panel')} title={t('ide-hide-panel')}><Icon name="panelRight" /></button>
    </div>
    <div className="min-h-0 flex-1 overflow-y-auto p-3" role="tabpanel" aria-label={tabs.find(x => x.id === active)?.label}>
      {active === 'needs' && <div className="grid gap-2">
        {needs === 0 && <p className="muted">{t('attention-empty')}</p>}
        {attention.map(a => <button key={a.id} type="button" className="ide-row justify-between border border-needs-you/60 px-3 py-2" onClick={() => onOpen(a.id)}><span className="truncate font-medium">{a.name}</span><StateBadge state={a.state} /></button>)}
        {notice && <p role="alert" className="text-needs-you">{notice}</p>}
        {approvals.map(a => <ApprovalCard key={a.id} approval={a} onResolve={(ok, choice) => onResolve(a, ok, choice)} />)}
      </div>}
      {active === 'changes' && (changed.length === 0 ? <p className="muted">{t('changes-none')}</p> : <ul className="m-0 grid list-none gap-1 p-0">
        {changed.map(a => <li key={a.id}>
          <button type="button" className="ide-row justify-between gap-3 py-1.5" onClick={() => (onReview ? onReview(a.id) : onOpen(a.id))}>
            <span className="min-w-0"><span className="block truncate font-medium">{a.shared ? a.workspace : a.name}</span><span className="block truncate text-xs text-muted">{a.shared ? t('workspace-shared') : a.machine} · {a.workspace}</span></span>
            <span className="flex-none text-xs text-needs-you">{t('changes-files', { count: a.filesChanged ?? 0 })}</span>
          </button>
        </li>)}
      </ul>)}
      {active === 'board' && <div className="grid gap-2">{boards.map(b => <ProjectBoard key={b.projectId} board={b} />)}</div>}
      {active === 'messages' && (messages.length === 0 ? <p className="muted">{t('console-no-messages')}</p> : <ul className="m-0 grid list-none gap-2 p-0">
        {messages.map((m, i) => <li key={`${m.atMs}-${i}`} className="rounded-xs border border-line bg-surface-0 p-2.5">
          <div className="text-xs"><strong>{m.from}</strong> <span className="inline-block rtl:-scale-x-100" aria-hidden="true">→</span> <strong>{m.to}</strong>{m.crossMachine ? <span className="ide-chip ms-1.5">{t('message-cross')}</span> : null}</div>
          <div className="mt-1 break-words text-muted">{m.body}</div>
        </li>)}
      </ul>)}
      {active === 'cost' && usage && <div className="grid gap-2">
        <p className="m-0 text-2xl font-semibold tracking-tight">{money(usage.totalMicroUsd)}</p>
        <ul className="m-0 grid list-none gap-1 p-0">{projects.map(p => <li key={p.label} className="flex justify-between gap-2"><span className="truncate">{p.label}</span><span className="text-muted">{money(p.microUsd)}</span></li>)}</ul>
        <p className="muted">{usage.pricesAtMs ? t('usage-prices-date', { date: new Date(usage.pricesAtMs).toLocaleDateString() }) : t('usage-prices-bundled')}</p>
      </div>}
    </div>
  </aside>;
}
