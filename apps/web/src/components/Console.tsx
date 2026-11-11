import { Suspense, lazy, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { t } from '../i18n';
import { type AgentTileModel, machineKey, needsAttention, sortFleet } from '../model';
import { money, type UsageSummary } from '../net/usage';
import type { ApprovalModel, BoardModel, MachineModel, MessageModel, ProjectCost } from '../net/view';
import type { TerminalSource } from '../terminal/source';
import { ApprovalCard } from './ApprovalCard';
import { ProjectBoard } from './ProjectBoard';
import { DiffDialog } from './DiffDialog';
import { PromptBox } from './PromptBox';
import type { ChatSource } from '../net/chat';
import { FilesDialog } from './FilesDialog';
import { HistoryDialog } from './HistoryDialog';
import type { CommitItem } from '../net/history';
import type { WorkspaceFiles } from '../net/files';
import { AgentAddress } from './AgentAddress';
import { StateBadge } from './StateBadge';

const TerminalView = lazy(() => import('../terminal/TerminalView').then((m) => ({ default: m.TerminalView })));

const MAX_AUTO_PINNED = 4;

// The one window: machines and agents on the left, the terminals of the agents you are watching in the middle (several at
// once), and what needs you, what agents say to each other and what it costs on the right. Everything is derived from props.
export function Console({ agents, machines, usage, projects = [], approvals, messages, boards = [], terminalFor, chatOf, sendPrompt, loadDiff, loadHistory, filesOf, onOpen, onStart, start, onResolve, notice, link }: {
  agents: readonly AgentTileModel[];
  machines: readonly MachineModel[];
  usage?: UsageSummary;
  projects?: readonly ProjectCost[];
  approvals: readonly ApprovalModel[];
  messages: readonly MessageModel[];
  boards?: readonly BoardModel[];
  terminalFor: (agentId: string) => TerminalSource;
  chatOf?: (agent: AgentTileModel) => ChatSource | undefined;
  sendPrompt?: (agent: AgentTileModel, text: string) => Promise<boolean>;
  loadDiff?: (agent: AgentTileModel) => Promise<string>;
  loadHistory?: (agent: AgentTileModel) => Promise<CommitItem[]>;
  filesOf?: (agent: AgentTileModel) => WorkspaceFiles | undefined;
  onOpen: (agentId: string) => void;
  onStart?: () => void;
  start?: ReactNode;
  onResolve: (a: ApprovalModel, approved: boolean) => void;
  notice?: string;
  link: 'connecting' | 'live' | 'offline';
}) {
  const fleet = useMemo(() => sortFleet(agents), [agents]);
  const [pinned, setPinned] = useState<string[]>([]);
  const touched = useRef(false);
  const [reviewing, setReviewing] = useState<string | undefined>();
  const [filter, setFilter] = useState('');
  const [projectFilter, setProjectFilter] = useState('');
  const projectOptions = useMemo(() => {
    const groups = new Map<string, { name: string; machines: Set<string> }>();
    for (const a of fleet) if (a.projectId) { const group = groups.get(a.projectId) ?? { name: a.project || a.projectId, machines: new Set<string>() }; group.machines.add(machineKey(a)); groups.set(a.projectId, group); }
    return [...groups.entries()];
  }, [fleet]);
  useEffect(() => {
    if (projectFilter && !projectOptions.some(([id]) => id === projectFilter)) setProjectFilter('');
  }, [projectFilter, projectOptions]);
  const chooseProject = (id: string) => {
    setProjectFilter(id);
    // The first view of a project gets real pinned panes, so Hide always removes the pane it displays.
    if (id) setPinned(cur => cur.some(key => fleet.some(a => a.id === key && a.projectId === id))
      ? cur : fleet.filter(a => a.projectId === id).slice(0, MAX_AUTO_PINNED).map(a => a.id));
  };
  const [browsing, setBrowsing] = useState<string | undefined>();
  const [history, setHistory] = useState<string | undefined>();

  // Until the user picks for themselves, watch the first few agents (those that need you come first). A new agent joins
  // while there is room; one that is gone leaves. Nothing is reordered, so a terminal never jumps under your hands.
  useEffect(() => {
    setPinned((cur) => {
      const alive = cur.filter((id) => fleet.some((a) => a.id === id));
      const room = touched.current ? [] : fleet.filter((a) => a.state !== 'finished' && a.state !== 'unknown' && !alive.includes(a.id)).slice(0, Math.max(0, MAX_AUTO_PINNED - alive.length)).map((a) => a.id);
      return alive.length === cur.length && room.length === 0 ? cur : [...alive, ...room];
    });
  }, [fleet]);
  // Panes are rearranged by dragging a pane's header onto another pane: the dragged pane takes that pane's place.
  const [dragging, setDragging] = useState<string | undefined>();
  const move = (from: string, to: string) => {
    touched.current = true;
    setPinned((cur) => {
      const a = cur.indexOf(from);
      const b = cur.indexOf(to);
      if (a < 0 || b < 0 || a === b) return cur;
      const next = [...cur];
      next.splice(a, 1);
      next.splice(b, 0, from);
      return next;
    });
  };
  const toggle = (id: string) => {
    touched.current = true;
    setPinned((cur) => (cur.includes(id) ? cur.filter((x) => x !== id) : [...cur, id]));
  };

  // machine → project → agents, after the search box (matches name, machine, project or workspace).
  const byMachine = useMemo(() => {
    const q = filter.trim().toLowerCase();
    const groups = new Map<string, Map<string, AgentTileModel[]>>();
    for (const a of fleet) {
      if (projectFilter && a.projectId !== projectFilter) continue;
      if (q && ![a.name, a.id, a.machine, machineKey(a), a.project ?? '', a.projectId ?? '', a.workspace].some((x) => x.toLowerCase().includes(q))) continue;
      const projects = groups.get(machineKey(a)) ?? new Map<string, AgentTileModel[]>();
      const key = a.projectId ?? a.project ?? '';
      projects.set(key, [...(projects.get(key) ?? []), a]);
      groups.set(machineKey(a), projects);
    }
    return groups;
  }, [fleet, filter, projectFilter]);
  const picked = pinned.map((id) => fleet.find((a) => a.id === id)).filter((a): a is AgentTileModel => !!a && (!projectFilter || a.projectId === projectFilter));
  const shown = picked;
  const attention = needsAttention(fleet);
  const changed = [...new Map(fleet.filter(a => (a.filesChanged ?? 0) > 0 && (!projectFilter || a.projectId === projectFilter))
    .map(a => [a.workspaceRef ? `${a.workspaceRef.node}/${a.workspaceRef.id}` : a.id, a])).values()]
    .sort((a, b) => (b.filesChanged ?? 0) - (a.filesChanged ?? 0));
  const cols = shown.length <= 1 ? 'lg:grid-cols-1' : 'lg:grid-cols-2';

  return (
    <div className="flex min-h-screen flex-col lg:h-screen">
      <header className="flex h-14 flex-none items-center justify-between gap-4 border-b border-line bg-surface-1 px-4">
        <div className="flex items-center gap-3">
          <span aria-hidden className="grid size-7 place-items-center rounded-s bg-accent text-sm font-bold text-on-accent">A</span>
          <h1 className="text-base">{t('home-title')}</h1>
          {link !== 'live' && <span role="status" className="rounded-full bg-surface-2 px-2.5 py-0.5 text-xs text-muted">{t(link === 'offline' ? 'link-offline' : 'link-connecting')}</span>}
        </div>
        <div className="flex items-center gap-2">
          <span className="hidden text-xs text-muted md:inline">{t('palette-hint')}</span>
          <a className="button whitespace-nowrap px-3 text-[13px]" href="#/machines">{t('machines-online', { count: machines.filter((m) => m.online).length })}</a>
          <a className="button whitespace-nowrap px-3 text-[13px]" href="#/device">{t('device-link')}</a>
        </div>
      </header>

      <div className="grid min-h-0 flex-1 grid-cols-1 lg:grid-cols-[17rem_minmax(0,1fr)_21rem]">
        <aside className="order-3 flex min-h-0 flex-col gap-5 overflow-y-auto border-line bg-surface-1 p-4 lg:order-1 lg:border-r" aria-label={t('console-agents')}>
          <button type="button" className="button button--primary w-full" disabled={!onStart} onClick={onStart}>{t('action-start-agent')}</button>
          {start}
          {projectOptions.length > 0 && <label className="grid gap-1 text-xs text-muted"><span>{t('filter-project')}</span><select value={projectFilter} onChange={e => chooseProject(e.target.value)}><option value="">{t('filter-all-projects')}</option>{projectOptions.map(([id, p]) => <option key={id} value={id}>{t('filter-project-machines', { name: p.name, count: p.machines.size })}</option>)}</select></label>}
          {fleet.length > 1 && <input type="search" value={filter} onChange={(e) => setFilter(e.target.value)} placeholder={t('filter-placeholder')} aria-label={t('filter-placeholder')} className="w-full" />}
          {machines.length === 0 && byMachine.size === 0 && !filter && <p className="muted">{t('machines-empty')}</p>}
          {[...byMachine.entries()].map(([machine, projects]) => {
            const m = machines.find((x) => x.id === machine) ?? machines.find((x) => x.name === machine);
            return (
              <section key={machine} data-machine-id={machine} className="grid min-w-0 gap-1.5">
                <h2 className="flex items-center gap-2">
                  <span aria-hidden className={`size-2 rounded-full ${m && !m.online ? 'bg-unknown' : 'bg-finished'}`} />
                  <span className="sr-only">{t(m && !m.online ? 'machine-offline' : 'machine-online')}</span>
                  <span className="min-w-0"><span className="block truncate">{m?.name ?? machine}</span>
                    {m && m.id !== m.name && <code className="block break-all text-xs font-normal normal-case text-muted">{m.id}</code>}
                  </span>
                </h2>
                {[...projects.entries()].map(([project, list]) => (
                  <div key={project} className="grid min-w-0 gap-1">
                    {project && <h3 className="m-0 px-1 text-[11px] font-medium tracking-wide text-muted">{list[0]?.project || project}</h3>}
                    <ul className="m-0 grid min-w-0 list-none gap-1 p-0">
                      {list.map((a) => (
                        <li key={a.id} className="min-w-0">
                          <button type="button" aria-pressed={pinned.includes(a.id)} onClick={() => toggle(a.id)}
                            className={`flex w-full items-center justify-between gap-2 rounded-s border px-3 py-2 text-left transition-colors hover:bg-surface-2 ${pinned.includes(a.id) ? 'border-accent bg-surface-2' : 'border-transparent'}`}>
                            <span className="min-w-0">
                              <span className="block truncate font-medium">{a.name}</span>
                              <span className="block truncate text-xs text-muted">{a.workspace}</span>{a.shared && <span className="block text-xs text-muted">{t('workspace-shared')}</span>}
                            </span>
                            <StateBadge state={a.state} />
                          </button>
                        </li>
                      ))}
                    </ul>
                  </div>
                ))}
              </section>
            );
          })}
          {fleet.length > 0 && byMachine.size === 0 && <p className="muted">{t('filter-none')}</p>}
          {fleet.length === 0 && <p className="muted">{t('fleet-empty')}</p>}
        </aside>

        <main className="order-2 min-h-0 bg-surface-0 p-3" aria-label={t('focus-terminal')}>
          {shown.length === 0 ? (
            <div className="grid h-full min-h-60 place-items-center rounded-l border border-dashed border-line p-8 text-center text-muted">{t('console-wall-empty')}</div>
          ) : (
            <div className={`grid h-full grid-cols-1 gap-3 ${cols}`} style={{ gridAutoRows: 'minmax(0, 1fr)' }}>
              {shown.map((a) => (
                <section key={a.id} onDragOver={(e) => { if (dragging) e.preventDefault(); }} onDrop={(e) => { e.preventDefault(); if (dragging) move(dragging, a.id); setDragging(undefined); }}
                  className={`flex h-[70vh] min-h-0 flex-col overflow-hidden rounded-l border bg-surface-1 shadow-card lg:h-auto ${a.state === 'needs-you' ? 'border-needs-you' : 'border-line'} ${dragging && dragging !== a.id ? 'ring-1 ring-accent/50' : ''}`}>
                  <header draggable onDragStart={(e) => { e.dataTransfer.effectAllowed = 'move'; e.dataTransfer.setData('text/plain', a.id); setDragging(a.id); }} onDragEnd={() => setDragging(undefined)}
                    title={t('console-drag')} className="flex flex-none cursor-grab flex-wrap items-center justify-between gap-2 border-b border-line px-3 py-2 active:cursor-grabbing">
                    <div className="flex min-w-0 flex-1 basis-40 items-center gap-2">
                      <span aria-hidden className="size-2.5 flex-none rounded-full" style={{ background: a.accent }} />
                      <strong className="min-w-0 flex-1 truncate" title={`${a.machine} · ${a.workspace}`}>{a.name}</strong>
                      {(a.filesChanged ?? 0) > 0 && (loadDiff
                        ? <button type="button" className="rounded-full bg-surface-2 px-2 py-0.5 text-xs whitespace-nowrap text-needs-you hover:brightness-125" onClick={() => setReviewing(a.id)}>{t('changes-files', { count: a.filesChanged ?? 0 })}</button>
                        : <span className="rounded-full bg-surface-2 px-2 py-0.5 text-xs whitespace-nowrap text-needs-you">{t('changes-files', { count: a.filesChanged ?? 0 })}</span>)}
                      {usage?.perAgent.has(a.id) && <span className="hidden text-xs text-muted xl:inline">{t('agent-cost', { amount: money(usage.perAgent.get(a.id) ?? 0) })}</span>}
                    </div>
                    <div className="ml-auto flex max-w-full flex-none flex-wrap items-center justify-end gap-2">
                      <StateBadge state={a.state} />
                      {loadHistory && a.git !== false && <button type="button" className="rounded-s px-2 py-1 text-xs text-muted hover:bg-surface-2 hover:text-ink" onClick={() => setHistory(a.id)}>{t('console-history')}</button>}
                      {filesOf && <button type="button" className="rounded-s px-2 py-1 text-xs text-muted hover:bg-surface-2 hover:text-ink" onClick={() => setBrowsing(a.id)}>{t('console-files')}</button>}
                      <button type="button" className="rounded-s px-2 py-1 text-xs text-muted hover:bg-surface-2 hover:text-ink" onClick={() => onOpen(a.id)}>{t('console-maximize')}</button>
                      <button type="button" aria-label={t('console-hide')} className="rounded-s px-2 py-1 text-xs text-muted hover:bg-surface-2 hover:text-ink" onClick={() => toggle(a.id)}>✕</button>
                    </div>
                  </header>
                  <AgentAddress agent={a} />
                  <div className="pane min-h-0 flex-1 p-1.5">
                    {a.terminal === false ? <PromptBox agent={a} send={sendPrompt} source={chatOf?.(a)} /> : (
                      <Suspense fallback={<div className="terminal" aria-busy="true" />}>
                        <TerminalView source={terminalFor(a.id)} label={`${t('focus-terminal')} · ${a.name}`} />
                      </Suspense>
                    )}
                  </div>
                </section>
              ))}
            </div>
          )}
        </main>

        <aside className="order-1 flex min-h-0 flex-col gap-6 overflow-y-auto border-line bg-surface-1 p-4 lg:order-3 lg:border-l" aria-label={t('console-activity')}>
          <section className="grid gap-2">
            <h2>{t('attention-title')}</h2>
            {attention.length === 0 && approvals.length === 0 ? <p className="muted">{t('attention-empty')}</p> : (
              <ul className="m-0 grid list-none gap-1.5 p-0">
                {attention.map((a) => (
                  <li key={a.id} className="min-w-0"><button type="button" className="flex w-full items-center justify-between gap-2 rounded-s border border-needs-you/60 px-3 py-2 text-left hover:bg-surface-2" onClick={() => onOpen(a.id)}><span className="truncate font-medium">{a.name}</span><StateBadge state={a.state} /></button></li>
                ))}
              </ul>
            )}
            {notice && <p role="alert" className="text-sm text-needs-you">{notice}</p>}
            {approvals.map((a) => <ApprovalCard key={a.id} approval={a} onResolve={(ok) => onResolve(a, ok)} />)}
          </section>

          <section className="grid gap-2" aria-label={t('changes-title')}>
            <h2>{t('changes-title')}</h2>
            {changed.length === 0 ? <p className="muted">{t('changes-none')}</p> : (
              <ul className="m-0 grid list-none gap-1.5 p-0">
                {changed.map((a) => (
                  <li key={a.id} className="min-w-0">
                    <button type="button" className="flex w-full items-center justify-between gap-2 rounded-s border border-line px-3 py-2 text-left hover:bg-surface-2" onClick={() => (loadDiff ? setReviewing(a.id) : onOpen(a.id))}>
                      <span className="min-w-0"><span className="block truncate font-medium">{a.shared ? a.workspace : a.name}</span><span className="block truncate text-xs text-muted">{a.shared ? t('workspace-shared') : a.machine} · {a.workspace}</span>{a.shared && <span className="block text-xs text-muted">{t('workspace-shared')}</span>}</span>
                      <span className="flex-none text-xs text-needs-you">{t('changes-files', { count: a.filesChanged ?? 0 })}</span>
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </section>

          {boards.length > 0 && (
            <section className="grid gap-2" aria-label={t('board-title')}>
              <h2>{t('board-title')}</h2>
              {boards.map(b => <ProjectBoard key={b.projectId} board={b} />)}
            </section>
          )}

          <section className="grid gap-2" aria-label={t('messages-title')}>
            <h2>{t('messages-title')}</h2>
            {messages.length === 0 ? <p className="muted">{t('console-no-messages')}</p> : (
              <ul className="m-0 grid list-none gap-2 p-0">
                {messages.map((m, i) => (
                  <li key={`${m.atMs}-${i}`} className="rounded-s border border-line bg-surface-0 p-2.5">
                    <div className="text-xs"><strong>{m.from}</strong> <span className="inline-block rtl:-scale-x-100" aria-hidden="true">→</span> <strong>{m.to}</strong>{m.crossMachine ? <span className="ms-1.5 rounded-full bg-surface-2 px-2 py-0.5 text-muted">{t('message-cross')}</span> : null}</div>
                    <div className="mt-1 break-words text-[13px] text-muted">{m.body}</div>
                  </li>
                ))}
              </ul>
            )}
          </section>

          {usage && usage.totalMicroUsd > 0 && (
            <section className="grid gap-2">
              <h2>{t('console-spent')}</h2>
              <p className="m-0 text-2xl font-semibold tracking-tight">{money(usage.totalMicroUsd)}</p>
              <ul className="m-0 grid list-none gap-1 p-0 text-[13px]">
                {projects.map((p) => <li key={p.label} className="flex justify-between gap-2"><span className="truncate">{p.label}</span><span className="text-muted">{money(p.microUsd)}</span></li>)}
              </ul>
              <p className="muted">{usage.pricesAtMs ? t('usage-prices-date', { date: new Date(usage.pricesAtMs).toLocaleDateString() }) : t('usage-prices-bundled')}</p>
            </section>
          )}
        </aside>
      </div>
      {history && loadHistory && fleet.find((a) => a.id === history) && (
        <HistoryDialog agent={fleet.find((a) => a.id === history) as AgentTileModel} load={loadHistory} onClose={() => setHistory(undefined)} />
      )}
      {browsing && filesOf && fleet.find((a) => a.id === browsing) && filesOf(fleet.find((a) => a.id === browsing) as AgentTileModel) && (
        <FilesDialog agent={fleet.find((a) => a.id === browsing) as AgentTileModel} files={filesOf(fleet.find((a) => a.id === browsing) as AgentTileModel) as WorkspaceFiles} onClose={() => setBrowsing(undefined)} />
      )}
      {reviewing && loadDiff && fleet.find((a) => a.id === reviewing) && (
        <DiffDialog agent={fleet.find((a) => a.id === reviewing) as AgentTileModel} load={loadDiff} onClose={() => setReviewing(undefined)} />
      )}
    </div>
  );
}
