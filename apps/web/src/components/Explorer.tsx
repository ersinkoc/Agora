import { useMemo, useState } from 'react';
import { t } from '../i18n';
import { forgetReasonKey } from '../net/forget';
import { resumeReasonKey } from '../net/directory';
import { helperTree, machineKey, type AgentTileModel } from '../model';
import { bytesLabel, type MachineModel } from '../net/view';
import { ago } from '../net/time';
import { Icon } from './Icon';
import { StateMark } from './StateBadge';

// The left panel: machine → folder → agent, like a file explorer. Opening an agent puts it in a tab.
export function Explorer({ fleet, machines, open, current, projects, project, onProject, onToggle, onStart, onCollapse, onDismiss, everyone, forgetError, onResume, resuming, resumeError }: {
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
  onResume?: (agent: AgentTileModel, newSession: boolean) => void;
  resuming?: string;
  resumeError?: { id: string; code: string };
}) {
  const [filter, setFilter] = useState('');
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
  const now = Date.now();

  return <aside className="flex min-h-0 flex-col border-line bg-surface-1 lg:border-e" aria-label={t('console-agents')}>
    <div className="flex h-9 flex-none items-center gap-1 border-b border-line ps-3 pe-1">
      <h2 className="m-0 flex-1 text-[11px]">{t('console-agents')}</h2>
      <button type="button" className="ide-iconbtn" disabled={!onStart} onClick={() => onStart?.()} aria-label={t('action-start-agent')} title={t('action-start-agent')}><Icon name="plus" /></button>
      <button type="button" className="ide-iconbtn" onClick={onCollapse} aria-label={t('ide-hide-panel')} title={t('ide-hide-panel')}><Icon name="panelLeft" /></button>
    </div>
    <div className="flex-none p-2">
      <label className="flex items-center gap-2 rounded-xs border border-line bg-surface-0 px-2 text-muted focus-within:border-accent">
        <Icon name="search" className="size-3.5" />
        <input type="search" value={filter} onChange={e => setFilter(e.target.value)} placeholder={t('filter-placeholder')} aria-label={t('filter-placeholder')}
          className="min-w-0 flex-1 border-0 bg-transparent px-0 py-1.5 text-[13px] focus-visible:outline-none" />
      </label>
    </div>
    {projects.length > 0 && <label className="flex-none px-2 pb-2 text-xs text-muted"><span className="sr-only">{t('filter-project')}</span>
      <select value={project} onChange={e => onProject(e.target.value)} className="w-full rounded-xs py-1 text-[13px]"><option value="">{t('filter-all-projects')}</option>{projects.map(([id, p]) => <option key={id} value={id}>{t('filter-project-machines', { name: p.name, count: p.machines.size })}</option>)}</select>
    </label>}
    <nav className="min-h-0 flex-1 overflow-y-auto px-1 pb-3">
      {machines.length === 0 && fleet.length === 0 && <p className="muted px-3 py-2">{t('machines-empty')}</p>}
      {[...tree.entries()].map(([machine, folders]) => {
        const m = machines.find(x => x.id === machine) ?? machines.find(x => x.name === machine);
        const count = [...folders.values()].reduce((n, l) => n + l.length, 0);
        const stopped = interrupted.get(machine) ?? [];
        const closed = folded.has(machine);
        const name = m?.name ?? machine;
        return <section key={machine} data-machine-id={machine} className="mb-0.5">
          <div className="group flex items-center">
            <button type="button" className="ide-row flex-1 font-medium" aria-expanded={!closed} aria-label={t('ide-toggle-machine', { name })} onClick={() => fold(machine)}>
              <Icon name="chevron" className={`size-3.5 text-muted transition-transform duration-150 ${closed ? 'rtl:rotate-180' : 'rotate-90'}`} />
              <span aria-hidden className={`size-2 flex-none rounded-full ${m?.online ? 'bg-finished' : 'bg-unknown'}`} />
              <span className="sr-only">{t(m?.online ? 'machine-online' : 'machine-offline')}</span>
              <span className="min-w-0 flex-1 truncate">{name}{m && m.id !== m.name && <code className="ms-1.5 text-[11px] font-normal text-muted">{m.id}</code>}</span>
              {!!m?.memoryBytes && <span className="font-mono text-[11px] font-normal text-muted" title={t('ide-memory')}>{bytesLabel(m.memoryBytes)}</span>}
              <span className="text-[11px] font-normal text-muted">{count}</span>
            </button>
            {onStart && m?.online && <button type="button" className="ide-iconbtn size-6 opacity-0 focus-visible:opacity-100 group-hover:opacity-100 [@media(hover:none)]:opacity-100" onClick={() => onStart(machine)} aria-label={t('ide-add-here', { machine: name })} title={t('ide-add-here', { machine: name })}><Icon name="plus" className="size-3.5" /></button>}
          </div>
          {!closed && [...folders.entries()].map(([folder, list]) => <div key={folder} className="ms-3 border-s border-line ps-1">
            <div className="flex items-center gap-1.5 px-2 pt-1.5 pb-0.5 text-[11px] text-muted"><Icon name="folder" className="size-3.5" /><span className="truncate">{list[0]?.project || folder}</span></div>
            <ul className="m-0 list-none p-0">
              {helperTree(list).map(({ agent: a, depth }) => <li key={a.id} className={depth === 0 ? undefined : depth === 1 ? 'ms-3' : 'ms-6'}>
                <button type="button" className="ide-row" aria-current={current === a.id ? 'true' : undefined} aria-pressed={open.includes(a.id)} onClick={() => onToggle(a.id)} title={`${a.machine} · ${a.workspace}${a.branch ? ` · ${a.branch}` : ''}${a.parentName ? ` · ${t('helper-started-by', { name: a.parentName })}` : ''}`}>
                  <StateMark state={a.state} />
                  <span className="min-w-0 flex-1 truncate">{a.name}{a.parentName && <span className="ms-1.5 text-[11px] text-muted">{t('helper-started-by', { name: a.parentName })}</span>}</span>
                  {a.shared && <span className="flex-none text-[11px] text-muted" title={t('workspace-shared')}>{t('ide-shared')}</span>}
                  {(a.filesChanged ?? 0) > 0 && <span className="font-mono text-[11px] text-needs-you">{a.filesChanged}</span>}
                </button>
              </li>)}
            </ul>
          </div>)}
          {!closed && stopped.length > 0 && <div className="ms-3 border-s border-line ps-1" role="group" aria-label={t('interrupted-section')}>
            <div className="px-2 pt-1.5 pb-0.5 text-[11px] text-muted" title={t('interrupted-hint')}>{t('interrupted-section')}</div>
            <ul className="m-0 list-none p-0">
              {stopped.map((a) => <li key={a.id}>
                <div className="flex items-center">
                  <button type="button" className="ide-row flex-1 opacity-80" aria-current={current === a.id ? 'true' : undefined} aria-pressed={open.includes(a.id)} onClick={() => onToggle(a.id)} title={`${a.machine} · ${a.workspace}`}>
                    <span className="[&_.state__mark]:!animate-none"><StateMark state="finished" /></span>
                    <span className="min-w-0 flex-1 truncate">{a.name}</span>
                  </button>
                  {onResume && <button type="button" className="ide-chip me-1 flex-none hover:text-ink" disabled={resuming === a.id} onClick={() => onResume(a, !a.interrupted?.resumable)}>
                    {t(a.interrupted?.resumable ? 'action-resume' : 'action-start-again')}
                  </button>}
                </div>
                {resumeError?.id === a.id && <p role="alert" className="m-0 px-2 pb-1 text-xs text-needs-you">{t(resumeReasonKey(resumeError.code))}</p>}
              </li>)}
            </ul>
          </div>}
        </section>;
      })}
      {closed.length > 0 && <section className="mt-2 border-t border-line pt-1" aria-label={t('closed-section')}>
        <div className="flex items-center">
          <button type="button" className="ide-row flex-1 font-medium" aria-expanded={!folded.has('~closed')} onClick={() => fold('~closed')}>
            <Icon name="chevron" className={`size-3.5 text-muted transition-transform duration-150 ${folded.has('~closed') ? 'rtl:rotate-180' : 'rotate-90'}`} />
            <span className="min-w-0 flex-1 truncate">{t('closed-section')}</span><span className="text-[11px] font-normal text-muted">{closed.length}</span>
          </button>
          <button type="button" className="ide-chip me-1 hover:text-ink" onClick={() => onDismiss(closed.map((a) => a.id))} title={everyone ? t('closed-clear-everyone') : undefined}>{t('closed-clear')}</button>
        </div>
        {forgetError && <p role="alert" className="m-0 px-2 py-1 text-xs text-needs-you">{t('closed-forget-failed', { reason: t(forgetReasonKey(forgetError)) })}</p>}
        {!folded.has('~closed') && <ul className="m-0 list-none p-0">
          {closed.map((a) => <li key={a.id} className="group flex items-center">
            <button type="button" className="ide-row flex-1 opacity-80" aria-current={current === a.id ? 'true' : undefined} aria-pressed={open.includes(a.id)} onClick={() => onToggle(a.id)} title={`${a.machine} · ${a.workspace}`}>
              <span className="[&_.state__mark]:!animate-none"><StateMark state="finished" /></span>
              <span className="min-w-0 flex-1 truncate">{a.name}</span>
              <span className="flex-none text-[11px] text-muted">{a.endedMs ? ago(a.endedMs, now) : ''}</span>
            </button>
            <button type="button" className="ide-iconbtn size-6 opacity-0 focus-visible:opacity-100 group-hover:opacity-100" onClick={() => onDismiss([a.id])} aria-label={t(everyone ? 'closed-forget' : 'closed-dismiss')} title={t(everyone ? 'closed-forget' : 'closed-dismiss')}><Icon name="close" className="size-3.5" /></button>
          </li>)}
        </ul>}
      </section>}
      {fleet.length > 0 && tree.size === 0 && closed.length === 0 && <p className="muted px-3 py-2">{t('filter-none')}</p>}
      {fleet.length === 0 && machines.length > 0 && <p className="muted px-3 py-2">{t('fleet-empty')}</p>}
    </nav>
  </aside>;
}
