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
import type { DetailsSection, DetailsTab } from './detailsTabs';

// The right panel: one subject at a time, picked from the vertical menu at the window's end (at most two
// panels are ever visible, visual-language.md). Its top carries the name of what is open, not a row of tabs.
export function Inspector({ sections, tab, attention, approvals, notice, changed, boards, messages, usage, projects, onOpen, onResolve, onReview, onCollapse, pairing }: {
  sections: readonly [DetailsSection, ...DetailsSection[]];
  tab: DetailsTab;
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
  // ADR-0065: machines asking to join, answered in their full cards here.
  pairing?: PairingUi;
}) {
  // A section that is gone (no shared tasks any more, nothing spent) falls back to what needs you.
  const active = sections.find((x) => x.id === tab) ?? sections[0];

  return <aside className="flex min-h-0 flex-col border-line bg-surface-1 lg:border-s" aria-label={t('console-activity')}>
    <div className="flex h-9 flex-none items-center gap-2 border-b border-line ps-3 pe-1">
      <h2 className="m-0 min-w-0 flex-1 truncate text-xs">{active.label}</h2>
      {!!active.badge && <span className={`ide-chip ${active.id === 'needs' ? 'text-needs-you' : ''}`}>{active.badge}</span>}
      <button type="button" className="ide-iconbtn" onClick={onCollapse} aria-label={t('ide-hide-panel')} title={t('ide-hide-panel')}><Icon name="panelRight" /></button>
    </div>
    <div id="details-panel" className="min-h-0 flex-1 overflow-y-auto p-3" role="tabpanel" aria-label={active.label}>
      <Details tab={active.id} pairing={pairing} attention={attention} approvals={approvals} notice={notice} changed={changed} boards={boards} messages={messages} usage={usage} projects={projects} onOpen={onOpen} onResolve={onResolve} onReview={onReview} />
    </div>
  </aside>;
}

// One subject of the details, on its own: the Inspector's sections and the layout's panels (ADR-0068) draw the same thing.
export function Details({ tab, attention, approvals, notice, changed, boards, messages, usage, projects, onOpen, onResolve, onReview, pairing }: {
  tab: DetailsTab;
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
