import { useMemo, useState } from 'react';
import { t } from '../i18n';
import { NoAskBadge } from './NoAskBadge';
import { LimitChip } from './Quota';
import { forgetReasonKey } from '../net/forget';
import { resumeReasonKey } from '../net/directory';
import { agentKind, endedWithError, helperTree, machineKey, sharesName, type AgentTileModel, type ResumeAction } from '../model';
import { bytesLabel, type MachineModel } from '../net/view';
import { ago, agoShort } from '../net/time';
import { Icon } from './Icon';
import { CloseAgentMenu, CloseConfirm } from './CloseAgent';
import { FinishedMark } from './FinishedMark';
import { AgentAvatar, Button, StatusMark } from './kit';
import { GroupByControl, groupTitle } from './GroupByControl';
import { ExplorerRows } from './ExplorerRows';
import { ProjectChip, ProjectHint } from './ProjectMark';
import { groupFleet, sharedAcross, sortByUrgency, type GroupBy, type ProjectSpread } from '../net/fleet';

const LARGE_FLEET = 120;
const agentId = (agent: AgentTileModel) => agent.id;

// The left panel: machine → folder → agent, like a file explorer. Opening an agent puts it in a tab.
export function Explorer({ by, onBy, spread, fleet, machines, open, current, projects, project, onProject, onToggle, onStart, onCollapse, onDismiss, everyone, forgetError, onResume, resuming, resumeError, watching }: {
  // How the list is grouped (one choice with the fleet console) and where each project is open (net/fleet.ts `projectSpread`).
  by: GroupBy;
  onBy: (by: GroupBy) => void;
  spread: ReadonlyMap<string, ProjectSpread>;
  fleet: readonly AgentTileModel[];
  machines: readonly MachineModel[];
  open: readonly string[];
  current?: string;
  projects: readonly (readonly [string, { name: string; machines: ReadonlySet<string> }])[];
  project: string;
  onProject: (id: string) => void;
  onToggle: (id: string) => void;
  onStart?: (machine?: string) => void;
  onCollapse: () => void;
  onDismiss: (ids: string[]) => void;
  // Removing a closed agent here asks the hub to forget it for every client (ADR-0053), not only this browser.
  everyone?: boolean;
  forgetError?: string;
  // ADR-0060: agents that were running when their machine stopped are listed under it with Resume (or Start again).
  onResume?: (agent: AgentTileModel, action: ResumeAction) => void;
  resuming?: string;
  resumeError?: { id: string; code: string };
  // ADR-0106: a viewer's window watches: no Start button, and the empty list does not ask them to start one.
  watching?: boolean;
}) {
  const [filter, setFilter] = useState('');
  const [viewport, setViewport] = useState<HTMLElement | null>(null);
  const windowed = fleet.length >= LARGE_FLEET && typeof IntersectionObserver === 'function' && typeof ResizeObserver === 'function';
  const rowWindow = { viewport, current, windowed };
  const [folded, setFolded] = useState<ReadonlySet<string>>(new Set());
  const fold = (key: string) => setFolded(cur => { const next = new Set(cur); if (!next.delete(key)) next.add(key); return next; });
  const tree = useMemo(() => {
    const q = filter.trim().toLowerCase();
    const groups = new Map<string, Map<string, AgentTileModel[]>>();
    for (const m of machines) if (!q) groups.set(m.id, new Map());
    for (const a of fleet) {
      if (a.closed && a.interrupted && !groups.has(machineKey(a))) groups.set(machineKey(a), new Map());
      if (a.closed) continue;
      if (q && ![a.name, a.id, a.machine, machineKey(a), a.project ?? '', a.workspace, a.branch ?? ''].some(x => x.toLowerCase().includes(q))) continue;
      const folders = groups.get(machineKey(a)) ?? new Map<string, AgentTileModel[]>();
      const key = a.projectId ?? a.project ?? a.workspace;
      folders.set(key, [...(folders.get(key) ?? []), a]);
      groups.set(machineKey(a), folders);
    }
    return groups;
  }, [fleet, machines, filter]);
  // Closed agents wait in their own section until the owner clears them.
  const closed = useMemo(() => {
    const q = filter.trim().toLowerCase();
    return fleet.filter((a) => a.closed && !a.interrupted && (!q || [a.name, a.machine, a.project ?? '', a.workspace].some((x) => x.toLowerCase().includes(q))))
      .sort((a, b) => (b.endedMs ?? 0) - (a.endedMs ?? 0));
  }, [fleet, filter]);
  // ADR-0060: per machine, the agents its restart interrupted (shown under it, not with the closed ones).
  const interrupted = useMemo(() => {
    const q = filter.trim().toLowerCase();
    const by = new Map<string, AgentTileModel[]>();
    for (const a of fleet) {
      if (!a.closed || !a.interrupted) continue;
      if (q && ![a.name, a.machine, a.project ?? '', a.workspace].some((x) => x.toLowerCase().includes(q))) continue;
      by.set(machineKey(a), [...(by.get(machineKey(a)) ?? []), a]);
    }
    return by;
  }, [fleet, filter]);
  // Grouped by project, agent or state: flat groups of live agents, most urgent first; the machine moves into each row.
  const flat = useMemo(() => {
    if (by === 'machine') return [];
    const q = filter.trim().toLowerCase();
    const live = fleet.filter((a) => (!a.closed || !!a.interrupted) && (!q || [a.name, a.id, a.machine, machineKey(a), a.project ?? '', a.workspace, a.branch ?? ''].some((x) => x.toLowerCase().includes(q))));
    return groupFleet(sortByUrgency(live), [], by);
  }, [fleet, filter, by]);
  const now = Date.now();
  // ADR-0060: an agent its machine's restart interrupted, with Resume or Start again; shown in every grouping.
  const stoppedRow = (a: AgentTileModel) => <li key={a.id}>
                <div className="flex items-center">
                  <button type="button" className="ide-row flex-1" aria-current={current === a.id ? 'true' : undefined} aria-pressed={open.includes(a.id)} onClick={() => onToggle(a.id)} title={`${a.machine} · ${a.workspace}`}>
                    <span className="opacity-60"><AgentAvatar accent={a.accent} kind={agentKind(a)} size="s" /></span>
                    <span className="min-w-0 flex-1 truncate text-muted">{a.name}</span>
                    <StatusMark state="finished" />
                  </button>
                  {onResume && <button type="button" className="ide-chip me-1 flex-none hover:text-ink" disabled={resuming === a.id} onClick={() => onResume(a, a.interrupted?.stillRunning ? 'stop' : a.interrupted?.resumable ? 'resume' : 'again')}>
                    {t(a.interrupted?.stillRunning ? 'action-stop-old' : a.interrupted?.resumable ? 'action-resume' : 'action-start-again')}
                  </button>}
                </div>
                {resumeError?.id === a.id && <p role="alert" className="m-0 px-2 pb-1 text-xs text-danger">{t(resumeReasonKey(resumeError.code))}</p>}
              </li>;
  // #945: a row keeps the agent's name readable, and the bolt of an agent that acts without asking whole. The name has at least 40 % of the row and gives way only after "started by …"
  // and the machine; when the name and the badges do not fit on one line the badges wrap below it. While the row is narrower than 16rem the
  // limit chip says only "82 %" and "Shared" hides (it stays in the row's tooltip).
  // Redesign PR 7: who it is leads (the avatar: its colour and kind), the facts trail (the changed files as a neutral ±N, the bolt bare in
  // red, the state's shape last); the row in front is an accent tint, never a bar on its edge.
  const agentRow = (a: AgentTileModel, depth: number, withMachine: boolean) => <li key={a.id} className={depth === 0 ? undefined : depth === 1 ? 'ms-3' : 'ms-6'}>
    <div className="group flex items-center">
    <button type="button" data-agent-row={a.id} className="ide-row @container flex-1 flex-wrap gap-y-1 overflow-hidden" aria-current={current === a.id ? 'true' : undefined} aria-pressed={open.includes(a.id)} onClick={() => onToggle(a.id)} title={`${a.name} · ${a.machine} · ${a.workspace}${a.branch ? ` · ${a.branch}` : ''}${a.parentName ? ` · ${t('helper-started-by', { name: a.parentName })}` : ''}${a.shared ? ` · ${t('ide-shared')}` : ''}${a.skipsPermissions ? ` · ${t('agent-does-not-ask')}` : ''}`}>
      <AgentAvatar accent={a.accent} kind={agentKind(a)} size="s" needsYou={a.state === 'needs-you'} />
      <span className="flex min-w-[40%] flex-1 items-baseline gap-1.5 overflow-hidden" data-agent-name-cell>
        <span className="max-w-full flex-none truncate" data-agent-name>{a.name}</span>
        {a.parentName && <span className="min-w-0 truncate text-xs text-muted">{t('helper-started-by', { name: a.parentName })}</span>}
        {withMachine && <span className="min-w-0 truncate text-xs text-muted">{a.machine}</span>}
      </span>
      <LimitChip agent={a} compact fit />
      {(by === 'kind' || by === 'state') && a.projectId && sharedAcross(spread.get(a.projectId)) && <ProjectChip spread={spread.get(a.projectId) as ProjectSpread} compact />}
      {a.shared && <span className="hidden flex-none text-xs text-muted @3xs:inline" title={t('workspace-shared')}>{t('ide-shared')}</span>}
      {(a.filesChanged ?? 0) > 0 && <span className="flex-none font-mono text-xs font-normal text-muted" data-files-changed><span aria-hidden>±{a.filesChanged}</span><span className="sr-only">{t('changes-files', { count: a.filesChanged ?? 0 })}</span></span>}
      <NoAskBadge agent={a} nested bare />
      <FinishedMark agentId={a.id} compact />
      <StatusMark state={a.state} />
    </button>
    {/* Close agent: the target is this row's agent. Shown on hover or focus; always on a touch screen. */}
    <span className="flex-none opacity-0 focus-within:opacity-100 group-hover:opacity-100 [@media(hover:none)]:opacity-100"><CloseAgentMenu agent={a} where="row" /></span>
    </div>
    <CloseConfirm agent={a} where="row" />
  </li>;

  // Under 1024 px the list stacks above the panes (Console.tsx): there it keeps a height of its own (up to about a third of the window,
  // never less than its tools and a few rows) and its agents scroll inside it, so the panes' tab bar never slides over its rows (#1069).
  return <aside data-region="sidebar" className="flex min-h-0 flex-col border-sidebar-line bg-sidebar text-sidebar-ink max-lg:max-h-[max(35dvh,16rem)] max-lg:flex-none max-lg:border-b lg:border-e" aria-label={t('console-agents')}>
    <div className="flex h-9 flex-none items-center gap-1 border-b border-line ps-3 pe-1">
      <h2 className="m-0 flex-1 text-xs">{t('console-agents')}</h2>
      {!watching && <button type="button" className="ide-iconbtn" disabled={!onStart} onClick={() => onStart?.()} aria-label={t('action-start-agent')} title={t('action-start-agent')}><Icon name="plus" /></button>}
      <button type="button" className="ide-iconbtn" onClick={onCollapse} aria-label={t('ide-hide-panel')} title={t('ide-hide-panel')}><Icon name="panelLeft" /></button>
    </div>
    <div className="flex-none p-2">
      <label className="flex items-center gap-2 rounded-xs border border-line bg-surface-0 px-2 text-muted focus-within:border-accent">
        <Icon name="search" className="size-3.5" />
        <input type="search" value={filter} onChange={e => setFilter(e.target.value)} placeholder={t('filter-placeholder')} aria-label={t('filter-placeholder')}
          className="min-w-0 flex-1 border-0 bg-transparent px-0 py-1.5 text-[0.8125rem] focus-visible:outline-none" />
      </label>
    </div>
    {projects.length > 0 && <label className="flex-none px-2 pb-2 text-xs text-muted"><span className="sr-only">{t('filter-project')}</span>
      <select value={project} onChange={e => onProject(e.target.value)} className="w-full rounded-xs py-1 text-[0.8125rem]"><option value="">{t('filter-all-projects')}</option>{projects.map(([id, p]) => <option key={id} value={id}>{t('filter-project-machines', { name: p.name, count: p.machines.size })}</option>)}</select>
    </label>}
    <div className="@container flex-none px-2 pb-2"><GroupByControl by={by} onChange={onBy} /></div>
    <nav ref={setViewport} data-agent-count={fleet.length} className="min-h-0 flex-1 overflow-y-auto px-1 pb-3 [--explorer-row-size:2rem] [@media(pointer:coarse)]:[--explorer-row-size:var(--tap)]">
      {machines.length === 0 && fleet.length === 0 && <p className="muted px-3 py-2">{t('machines-empty')}</p>}
      {by !== 'machine' && flat.map((g) => {
        const key = `~${by}:${g.key}`;
        const sp = by === 'project' ? spread.get(g.key) : undefined;
        const title = groupTitle(g, by);
        return <section key={key} data-group={g.key} className="mb-0.5">
          <button type="button" className="ide-row font-medium" aria-expanded={!folded.has(key)} onClick={() => fold(key)}>
            <Icon name="chevron" className={`size-3.5 text-muted transition-transform duration-150 ${folded.has(key) ? 'rtl:rotate-180' : 'rotate-90'}`} />
            <span className="min-w-0 flex-1 truncate">{title}</span>
            {sharedAcross(sp) && <ProjectChip spread={sp} compact />}
            <span className="text-xs font-normal text-muted">{g.agents.length}</span>
          </button>
          {!folded.has(key) && <div className="ms-3 border-s border-line ps-1">
            {sp && <div className="px-2 pb-0.5"><ProjectHint spread={sp} /></div>}
            <ExplorerRows {...rowWindow} items={g.agents.filter((a) => !a.closed)} idOf={agentId} renderRow={a => agentRow(a, 0, true)} />
            {g.agents.some((a) => a.closed) && <div role="group" aria-label={t('interrupted-section')}>
              <div className="px-2 pt-1.5 pb-0.5 text-xs text-muted" title={`${t('interrupted-section')}\n${t('interrupted-hint')}`}>{t('interrupted-section-short')}</div>
              <ExplorerRows {...rowWindow} items={g.agents.filter((a) => a.closed)} idOf={agentId} renderRow={stoppedRow} />
            </div>}
          </div>}
        </section>;
      })}
      {by === 'machine' && [...tree.entries()].map(([machine, folders]) => {
        const m = machines.find(x => x.id === machine) ?? machines.find(x => x.name === machine);
        const count = [...folders.values()].reduce((n, l) => n + l.length, 0);
        const stopped = interrupted.get(machine) ?? [];
        const closed = folded.has(machine);
        const name = m?.name ?? machine;
        // ADR-0070: an offline machine says when it was last heard from: "offline · 2 hr" on the row, the full words in its tooltip.
        const seen = m && !m.online ? t('machine-last-seen', { when: ago(m.lastSeenMs, now) }) : undefined;
        return <section key={machine} data-machine-id={machine} data-online={m?.online ? 'true' : 'false'} className="mb-0.5">
          <div className="group flex items-center">
            <button type="button" className="ide-row flex-1 font-medium" aria-expanded={!closed} aria-label={t('ide-toggle-machine', { name })} title={seen ? `${t('machine-offline')} · ${seen}` : undefined} onClick={() => fold(machine)}>
              <Icon name="chevron" className={`size-3.5 text-muted transition-transform duration-150 ${closed ? 'rtl:rotate-180' : 'rotate-90'}`} />
              <span aria-hidden className={`size-2 flex-none rounded-full ${m?.online ? 'bg-finished' : 'bg-unknown'}`} />
              <span className="sr-only">{t(m?.online ? 'machine-online' : 'machine-offline')}</span>
              <span className="min-w-0 flex-1 truncate">{name}{m && sharesName(m, machines) && <code className="ms-1.5 text-xs font-normal text-muted">{m.id}</code>}</span>
              {m && !m.online && <span className="flex-none truncate text-xs font-normal text-muted" data-last-seen>{t('explorer-offline-for', { when: agoShort(m.lastSeenMs, now) })}</span>}
              {!!m?.memoryBytes && <span className="font-mono text-xs font-normal text-muted" title={t('ide-memory')}>{bytesLabel(m.memoryBytes)}</span>}
              {(m?.online !== false || count > 0) && <span className="text-xs font-normal text-muted">{count}</span>}
            </button>
            {onStart && m?.online && <button type="button" className="ide-iconbtn size-6 opacity-0 focus-visible:opacity-100 group-hover:opacity-100 [@media(hover:none)]:opacity-100" onClick={() => onStart(machine)} aria-label={t('ide-add-here', { machine: name })} title={t('ide-add-here', { machine: name })}><Icon name="plus" className="size-3.5" /></button>}
          </div>
          {!closed && [...folders.entries()].map(([folder, list]) => <div key={folder} className="ms-3 border-s border-line ps-1">
            <div className="flex items-center gap-1.5 px-2 pt-1.5 pb-0.5 text-xs text-muted"><Icon name="folder" className="size-3.5" /><span className="truncate">{list[0]?.project || folder}</span>{(() => { const sp = list[0]?.projectId ? spread.get(list[0].projectId) : undefined; return sharedAcross(sp) ? <ProjectChip spread={sp} compact /> : null; })()}</div>
            <ExplorerRows {...rowWindow} items={helperTree(list)} idOf={row => row.agent.id}
              renderRow={({ agent: a, depth }) => agentRow(a, depth, false)} />
          </div>)}
          {!closed && stopped.length > 0 && <div className="ms-3 border-s border-line ps-1" role="group" aria-label={t('interrupted-section')}>
            <div className="px-2 pt-1.5 pb-0.5 text-xs text-muted" title={`${t('interrupted-section')}\n${t('interrupted-hint')}`}>{t('interrupted-section-short')}</div>
            <ExplorerRows {...rowWindow} items={stopped} idOf={agentId} renderRow={stoppedRow} />
          </div>}
        </section>;
      })}
      {closed.length > 0 && <section className="mt-2 border-t border-line pt-1" aria-label={t('closed-section')}>
        <div className="flex items-center">
          <button type="button" className="ide-row flex-1 font-medium" aria-expanded={!folded.has('~closed')} onClick={() => fold('~closed')}>
            <Icon name="chevron" className={`size-3.5 text-muted transition-transform duration-150 ${folded.has('~closed') ? 'rtl:rotate-180' : 'rotate-90'}`} />
            <span className="min-w-0 flex-1 truncate">{t('closed-section')}</span><span className="text-xs font-normal text-muted">{closed.length}</span>
          </button>
          <Button variant="ghost" size="sm" className="me-1" onClick={() => onDismiss(closed.map((a) => a.id))} aria-label={t('closed-clear')} title={everyone ? t('closed-clear-everyone') : undefined}>{t('closed-clear-short')}</Button>
        </div>
        {forgetError && <p role="alert" className="m-0 px-2 py-1 text-xs text-danger">{t('closed-forget-failed', { reason: t(forgetReasonKey(forgetError)) })}</p>}
        {!folded.has('~closed') && <ExplorerRows {...rowWindow} items={closed} idOf={agentId} renderRow={a => <li key={a.id} className="group flex items-center">
            <button type="button" className="ide-row flex-1" aria-current={current === a.id ? 'true' : undefined} aria-pressed={open.includes(a.id)} onClick={() => onToggle(a.id)} title={`${a.machine} · ${a.workspace}`}>
              <span className="opacity-60"><AgentAvatar accent={a.accent} kind={agentKind(a)} size="s" /></span>
              <span className="min-w-0 flex-1 truncate text-muted">{a.name}</span>
              <span className="flex-none text-xs font-normal text-muted">{a.endedMs ? agoShort(a.endedMs, now) : ''}</span>
              <StatusMark state="finished" failed={endedWithError(a)} />
            </button>
            <button type="button" className="ide-iconbtn size-6 opacity-0 focus-visible:opacity-100 group-hover:opacity-100" onClick={() => onDismiss([a.id])} aria-label={t(everyone ? 'closed-forget' : 'closed-dismiss')} title={t(everyone ? 'closed-forget' : 'closed-dismiss')}><Icon name="close" className="size-3.5" /></button>
          </li>} />}
      </section>}
      {fleet.length > 0 && (by === 'machine' ? tree.size === 0 : flat.length === 0) && closed.length === 0 && <p className="muted px-3 py-2">{t('filter-none')}</p>}
      {fleet.length === 0 && machines.length > 0 && <p className="muted px-3 py-2">{t(watching ? 'fleet-watch-empty' : 'fleet-empty')}</p>}
    </nav>
  </aside>;
}
