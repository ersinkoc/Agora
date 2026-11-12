import { Suspense, lazy, useEffect, useMemo, useRef, useState, useSyncExternalStore, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from 'react';
import { t } from '../i18n';
import { type AgentTileModel, machineKey, needsAttention, sortFleet } from '../model';
import { money, type UsageSummary } from '../net/usage';
import type { ApprovalModel, BoardModel, MachineModel, MessageModel, ProjectCost } from '../net/view';
import type { TerminalSource } from '../terminal/source';
import { DiffDialog } from './DiffDialog';
import { PromptBox } from './PromptBox';
import type { ChatSource } from '../net/chat';
import { FilesDialog } from './FilesDialog';
import { HistoryDialog } from './HistoryDialog';
import type { CommitItem } from '../net/history';
import type { WorkspaceFiles } from '../net/files';
import { AgentAddress } from './AgentAddress';
import { Explorer } from './Explorer';
import { Icon } from './Icon';
import { Inspector } from './Inspector';
import { LanguagePicker } from './LanguagePicker';
import { ThemePicker } from './ThemePicker';
import { MAX_SIZE, MIN_SIZE, setTerminalSize, terminalSize } from '../terminal/prefs';
import { StateBadge, StateMark } from './StateBadge';
import { ClosedSummary } from './ClosedSummary';
import { dismiss, dismissedIds, subscribeDismissed } from '../net/dismissed';
import { loadLayout, saveLayout } from '../net/layout';
import { Resizer } from './Resizer';

const TerminalView = lazy(() => import('../terminal/TerminalView').then((m) => ({ default: m.TerminalView })));

const MAX_AUTO_PINNED = 4;
// The search shortcut as the keyboard spells it: ⌘ on Apple machines (the desktop app's WebKit included), Ctrl elsewhere.
const commandKey = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent) ? '⌘' : 'Ctrl';
const wide = () => typeof window === 'undefined' || !window.matchMedia || window.matchMedia('(min-width: 1024px)').matches;

// The one window, laid out like an editor: the agents on the left as a tree, the terminals you are watching as tabs in the
// middle (or side by side), details on the right, and a status bar. Everything is derived from props.
export function Console({ agents, machines, usage, projects = [], approvals, messages, boards = [], terminalFor, chatOf, sendPrompt, loadDiff, loadHistory, filesOf, onOpen, onStart, onCommand, onForget, start, onResolve, notice, link }: {
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
  onStart?: (machine?: string) => void;
  onCommand?: () => void;
  // Ask the hub to forget a closed agent for every client (ADR-0053); false when it could not (an older hub), and it is only hidden here.
  onForget?: (agentId: string) => Promise<string | undefined>;
  start?: ReactNode;
  onResolve: (a: ApprovalModel, approved: boolean) => void;
  notice?: string;
  link: 'connecting' | 'live' | 'offline';
}) {
  const cleared = useSyncExternalStore(subscribeDismissed, dismissedIds);
  // What was cleared stays hidden only while the agent is closed: one that runs again (a new run under the same id) is back in the list.
  const fleet = useMemo(() => sortFleet(agents.filter((a) => !(a.closed && cleared.has(a.id)))), [agents, cleared]);
  const [now, setNow] = useState(Date.now());
  useEffect(() => { const id = setInterval(() => setNow(Date.now()), 15000); return () => clearInterval(id); }, []);
  const saved = useRef(loadLayout()).current;
  const [pinned, setPinned] = useState<string[]>(saved?.pinned ?? []);
  const [active, setActive] = useState<string | undefined>(saved?.active);
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
    // The first view of a project gets real tabs, so closing a tab always removes what it shows.
    if (id) setPinned(cur => cur.some(key => fleet.some(a => a.id === key && a.projectId === id)) ? cur : fleet.filter(a => a.projectId === id).slice(0, MAX_AUTO_PINNED).map(a => a.id));
  };
  const [layout, setLayout] = useState<'single' | 'grid'>(saved?.layout ?? 'single');
  const [, textSize] = useState(0); // the size buttons follow the stored choice
  useEffect(() => { const on = () => textSize((n) => n + 1); window.addEventListener('agora-terminal-size', on); return () => window.removeEventListener('agora-terminal-size', on); }, []);
  const [showAgents, setShowAgents] = useState(saved?.showAgents ?? wide());
  const [showDetails, setShowDetails] = useState(saved?.showDetails ?? wide());
  const [leftWidth, setLeftWidth] = useState(saved?.leftWidth ?? 272);
  const [rightWidth, setRightWidth] = useState(saved?.rightWidth ?? 336);
  const touched = useRef(saved?.touched ?? false);
  const [reviewing, setReviewing] = useState<string | undefined>();
  const [browsing, setBrowsing] = useState<string | undefined>();
  const [history, setHistory] = useState<string | undefined>();

  // Until the user picks for themselves, watch the first few agents (those that need you come first). A new agent joins
  // while there is room; one that is gone leaves. Nothing is reordered, so a terminal never jumps under your hands.
  useEffect(() => {
    setPinned((cur) => {
      // Before the first snapshot there is no fleet to compare with: keep what was open rather than dropping it.
      if (fleet.length === 0) return cur;
      const alive = cur.filter((id) => fleet.some((a) => a.id === id));
      const room = touched.current ? [] : fleet.filter((a) => !a.closed && a.state !== 'finished' && a.state !== 'unknown' && !alive.includes(a.id)).slice(0, Math.max(0, MAX_AUTO_PINNED - alive.length)).map((a) => a.id);
      return alive.length === cur.length && room.length === 0 ? cur : [...alive, ...room];
    });
  }, [fleet]);
  useEffect(() => { saveLayout({ pinned, active, layout, touched: touched.current, showAgents, showDetails, leftWidth, rightWidth }); }, [pinned, active, layout, showAgents, showDetails, leftWidth, rightWidth]);
  useEffect(() => {
    setActive((cur) => (cur && pinned.includes(cur) ? cur : pinned[0]));
  }, [pinned]);

  // Tabs are rearranged by dragging one onto another: the dragged tab takes that tab's place.
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
  const close = (id: string) => { touched.current = true; setPinned((cur) => cur.filter((x) => x !== id)); };
  // The tree: a closed agent opens as a tab and takes focus; the one in focus closes; an open one comes to the front.
  const select = (id: string) => {
    touched.current = true;
    if (!pinned.includes(id)) { setPinned((cur) => [...cur, id]); setActive(id); }
    else if (active === id) close(id);
    else setActive(id);
  };

  const tabs = pinned.map((id) => fleet.find((a) => a.id === id)).filter((a): a is AgentTileModel => !!a && (!projectFilter || a.projectId === projectFilter));
  // The tab row as a tab list: arrows (mirrored right to left) and Home/End move between tabs, Delete or Backspace closes the
  // focused one; only the selected tab is a stop for Tab, so the row is one stop on the way to the terminal.
  const tabKey = (e: ReactKeyboardEvent<HTMLElement>, id: string) => {
    const ids = tabs.map((x) => x.id);
    const at = ids.indexOf(id);
    const rtl = getComputedStyle(e.currentTarget).direction === 'rtl';
    const step = { ArrowRight: rtl ? -1 : 1, ArrowLeft: rtl ? 1 : -1 }[e.key];
    let to: string | undefined;
    if (e.key === 'Enter' || e.key === ' ') to = id;
    else if (step !== undefined) to = ids[(at + step + ids.length) % ids.length];
    else if (e.key === 'Home') to = ids[0];
    else if (e.key === 'End') to = ids[ids.length - 1];
    else if (e.key === 'Delete' || e.key === 'Backspace') {
      e.preventDefault();
      const next = ids[at + 1] ?? ids[at - 1];
      close(id);
      if (next) { setActive(next); requestAnimationFrame(() => document.getElementById(`tab-${next}`)?.focus()); }
      return;
    } else return;
    e.preventDefault();
    if (!to) return;
    setActive(to);
    document.getElementById(`tab-${to}`)?.focus();
  };
  const current = tabs.find((a) => a.id === active) ?? tabs[0];
  const shown = layout === 'grid' ? tabs.slice(0, MAX_AUTO_PINNED) : current ? [current] : [];
  const attention = needsAttention(fleet);
  const changed = [...new Map(fleet.filter((a) => (a.filesChanged ?? 0) > 0 && (!projectFilter || a.projectId === projectFilter))
    .map((a) => [a.workspaceRef ? `${a.workspaceRef.node}/${a.workspaceRef.id}` : a.id, a])).values()]
    .sort((a, b) => (b.filesChanged ?? 0) - (a.filesChanged ?? 0));
  // An agent that is waiting and has an approval open is one thing that needs you, not two.
  const needs = attention.length + approvals.filter((x) => !attention.some((a) => a.id === x.agentId)).length;
  // Panel widths are what the user dragged them to; on a phone the panels stack and the widths do not apply.
  const cols = `${showAgents ? `${leftWidth}px ` : ''}minmax(0,1fr)${showDetails ? ` ${rightWidth}px` : ''}`
  const find = (id?: string) => fleet.find((a) => a.id === id);

  // Editor habits: Alt+1…9 picks a tab, Ctrl/⌘+B hides or shows the agent list.
  useEffect(() => {
    const on = (e: KeyboardEvent) => {
      if (e.altKey && !e.ctrlKey && !e.metaKey && /^[1-9]$/.test(e.key)) {
        const target = tabs[Number(e.key) - 1];
        if (target) { e.preventDefault(); setActive(target.id); }
      } else if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === 'b') { e.preventDefault(); setShowAgents((v) => !v); }
    };
    window.addEventListener('keydown', on);
    return () => window.removeEventListener('keydown', on);
  }, [tabs]);

  // Removing closed agents from the list: for every client when the hub can forget them. A hub that does not know the command can only
  // hide them here; any other refusal (permission, the agent running again) is shown with its reason and nothing is hidden.
  const [forgetError, setForgetError] = useState<string | undefined>();
  const removeClosed = (ids: string[]) => {
    if (!onForget) { dismiss(ids); return; }
    setForgetError(undefined);
    for (const id of ids) {
      void onForget(id).then((code) => {
        if (!code) return;
        if (code === 'protocol.invalid_message' || code === 'protocol.capability_missing') dismiss([id]);
        else setForgetError(code);
      });
    }
  };

  const pane = (a: AgentTileModel) => (
    <section key={a.id} aria-label={a.name} onDragOver={(e) => { if (dragging) e.preventDefault(); }} onDrop={(e) => { e.preventDefault(); if (dragging) move(dragging, a.id); setDragging(undefined); }}
      className={`flex min-h-0 min-w-0 flex-col bg-surface-0 ${layout === 'grid' ? `border ${a.state === 'needs-you' ? 'border-needs-you' : 'border-line'}` : ''}`}>
      <div className="flex min-h-9 flex-none flex-wrap items-center gap-x-3 gap-y-1 border-b border-line bg-surface-1 px-3 py-1">
        <div className="flex min-w-0 flex-1 basis-56 items-center gap-1.5 text-xs text-muted" title={`${a.machine} · ${a.workspace}`}>
          <Icon name="machine" className="size-3.5" /><span className="truncate">{a.machine}</span>
          <Icon name="chevron" className="size-3 rtl:rotate-180" /><Icon name="folder" className="size-3.5" /><span className="truncate">{a.project || a.workspace}</span>
          {a.branch && <><Icon name="branch" className="ms-1 size-3.5" /><span className="truncate font-mono">{a.branch}</span></>}
          {a.shared && <span className="ide-chip ms-1 flex-none" title={t('workspace-shared')}>{t('ide-shared')}</span>}
        </div>
        <div className="flex flex-none items-center gap-1">
          {(a.filesChanged ?? 0) > 0 && (loadDiff
            ? <button type="button" className="ide-chip text-needs-you hover:brightness-125" onClick={() => setReviewing(a.id)}>{t('changes-files', { count: a.filesChanged ?? 0 })}</button>
            : <span className="ide-chip text-needs-you">{t('changes-files', { count: a.filesChanged ?? 0 })}</span>)}
          {usage?.perAgent.has(a.id) && <span className="ide-chip">{t('agent-cost', { amount: money(usage.perAgent.get(a.id) ?? 0) })}</span>}
          <StateBadge state={a.state} />
          {a.terminal !== false && !a.closed && <span className="flex items-center" role="group" aria-label={t('terminal-size')}>
            <button type="button" className="ide-iconbtn w-6 text-[11px] font-semibold" onClick={() => setTerminalSize(terminalSize() - 1)} disabled={terminalSize() <= MIN_SIZE} aria-label={t('terminal-smaller')} title={t('terminal-smaller')}>A−</button>
            <button type="button" className="ide-iconbtn w-6 text-[13px] font-semibold" onClick={() => setTerminalSize(terminalSize() + 1)} disabled={terminalSize() >= MAX_SIZE} aria-label={t('terminal-larger')} title={t('terminal-larger')}>A+</button>
          </span>}
          {loadHistory && a.git !== false && <button type="button" className="ide-iconbtn" onClick={() => setHistory(a.id)} aria-label={t('console-history')} title={t('console-history')}><Icon name="history" /></button>}
          {filesOf && <button type="button" className="ide-iconbtn" onClick={() => setBrowsing(a.id)} aria-label={t('console-files')} title={t('console-files')}><Icon name="files" /></button>}
          <button type="button" className="ide-iconbtn" onClick={() => onOpen(a.id)} aria-label={t('console-maximize')} title={t('console-maximize')}><Icon name="maximize" /></button>
          {layout === 'grid' && <button type="button" className="ide-iconbtn" onClick={() => close(a.id)} aria-label={t('console-hide')} title={t('console-hide')}><Icon name="close" /></button>}
        </div>
      </div>
      {approvals.filter((x) => x.agentId === a.id).map((x) => (
        <div key={x.id} role="alert" className="flex flex-wrap items-center gap-x-3 gap-y-1.5 border-b border-needs-you/60 bg-surface-2 px-3 py-2">
          <StateMark state="needs-you" />
          <div className="min-w-0 flex-1"><strong className="block truncate">{t('approval-title', { name: x.agentName })}</strong><span className="block truncate text-xs text-muted">{x.action}{x.detail ? ` · ${x.detail}` : ''}</span></div>
          <button type="button" className="button button--primary px-3 py-1 text-xs" onClick={() => onResolve(x, true)}>{t('approval-allow')}</button>
          <button type="button" className="button px-3 py-1 text-xs" onClick={() => onResolve(x, false)}>{t('approval-deny')}</button>
        </div>
      ))}
      <AgentAddress agent={a} />
      {a.closed ? <ClosedSummary agent={a} now={now} cost={usage?.perAgent.has(a.id) ? money(usage.perAgent.get(a.id) ?? 0) : undefined}
        onReview={loadDiff ? () => setReviewing(a.id) : undefined} onHistory={loadHistory && a.git !== false ? () => setHistory(a.id) : undefined} onFiles={filesOf ? () => setBrowsing(a.id) : undefined}
        onStart={onStart ? () => onStart() : undefined} onDismiss={() => dismiss([a.id])}
        onForget={onForget ? () => removeClosed([a.id]) : undefined} forgetError={forgetError} /> : <div className="pane min-h-0 flex-1 p-1.5">
        {a.terminal === false ? <PromptBox agent={a} send={sendPrompt} source={chatOf?.(a)} /> : (
          <Suspense fallback={<div className="terminal" aria-busy="true" />}>
            <TerminalView source={terminalFor(a.id)} label={`${t('focus-terminal')} · ${a.name}`} />
          </Suspense>
        )}
      </div>}
    </section>
  );

  return (
    <div className="ide">
      <header className="ide-bar h-10 border-b">
        <span aria-hidden className="grid size-6 flex-none place-items-center rounded-xs bg-accent text-xs font-bold text-on-accent">A</span>
        <h1 className="m-0 me-2 hidden text-[13px] font-semibold tracking-normal sm:block">{t('home-title')}</h1>
        {!showAgents && <button type="button" className="ide-iconbtn" onClick={() => setShowAgents(true)} aria-label={t('ide-show-agents')} title={t('ide-show-agents')}><Icon name="panelLeft" /></button>}
        <button type="button" onClick={onCommand} disabled={!onCommand} className="mx-auto flex h-7 min-w-0 max-w-md flex-1 items-center gap-2 rounded-xs border border-line bg-surface-0 px-2.5 text-start text-muted transition-colors duration-150 hover:border-accent hover:text-ink">
          <Icon name="search" className="size-3.5" /><span className="min-w-0 flex-1 truncate">{t('ide-command')}</span><kbd className="ide-kbd hidden sm:inline">{commandKey} K</kbd>
        </button>
        {link !== 'live' && <span role="status" className="ide-chip">{t(link === 'offline' ? 'link-offline' : 'link-connecting')}</span>}
        <span className="hidden sm:block"><LanguagePicker compact /></span>
        <ThemePicker />
        <a className="ide-chip hidden whitespace-nowrap hover:text-ink md:inline-flex" href="#/device">{t('device-link')}</a>
        {!showDetails && <button type="button" className="ide-iconbtn" onClick={() => setShowDetails(true)} aria-label={t('ide-show-details')} title={t('ide-show-details')}><Icon name="panelRight" /></button>}
      </header>

      <div className="relative grid min-h-0 grid-cols-1 lg:[grid-template-columns:var(--cols)]" style={{ ['--cols' as string]: cols }}>
        {showAgents && <div className="absolute inset-y-0 z-10 hidden lg:block" style={{ insetInlineStart: leftWidth }}><Resizer side="start" value={leftWidth} min={200} max={480} onChange={setLeftWidth} /></div>}
        {showDetails && <div className="absolute inset-y-0 z-10 hidden lg:block" style={{ insetInlineEnd: rightWidth }}><Resizer side="end" value={rightWidth} min={260} max={560} onChange={setRightWidth} /></div>}
        {showAgents && <Explorer fleet={projectFilter ? fleet.filter((a) => a.projectId === projectFilter) : fleet} machines={machines} projects={projectOptions} project={projectFilter} onProject={chooseProject} open={pinned} current={current?.id} onToggle={select} onStart={onStart} onCollapse={() => setShowAgents(false)} onDismiss={removeClosed} everyone={!!onForget} forgetError={forgetError} />}

        <main className="flex min-h-0 min-w-0 flex-col" aria-label={t('focus-terminal')}>
          <div className="flex h-9 flex-none items-stretch border-b border-line bg-surface-1">
            <div role="tablist" aria-label={t('console-agents')} className="flex min-w-0 flex-1 overflow-x-auto quiet-scroll">
              {tabs.map((a) => (
                <div key={a.id} id={`tab-${a.id}`} role="tab" aria-selected={current?.id === a.id} aria-keyshortcuts="Delete" tabIndex={current?.id === a.id ? 0 : -1} draggable className={`ide-tab group cursor-pointer ${dragging && dragging !== a.id ? 'ring-1 ring-inset ring-accent/50' : ''}`}
                  onClick={() => setActive(a.id)} onKeyDown={(e) => tabKey(e, a.id)}
                  onDragStart={(e) => { e.dataTransfer.effectAllowed = 'move'; e.dataTransfer.setData('text/plain', a.id); setDragging(a.id); }} onDragEnd={() => setDragging(undefined)}
                  onDragOver={(e) => { if (dragging) e.preventDefault(); }} onDrop={(e) => { e.preventDefault(); if (dragging) move(dragging, a.id); setDragging(undefined); }} title={t('console-drag')}>
                  <StateMark state={a.state} />
                  <span className="min-w-0 truncate">{a.name}</span>
                  <button type="button" tabIndex={-1} aria-hidden className="ide-iconbtn size-5 opacity-60 group-hover:opacity-100" onClick={(e) => { e.stopPropagation(); close(a.id); }} title={t('ide-close-tab', { name: a.name })}><Icon name="close" className="size-3" /></button>
                </div>
              ))}
              {onStart && <button type="button" className="ide-iconbtn m-1 flex-none" onClick={() => onStart()} aria-label={t('ide-new-tab')} title={t('ide-new-tab')}><Icon name="plus" /></button>}
            </div>
            {tabs.length > 1 && <div className="flex flex-none items-center gap-0.5 px-1" role="group" aria-label={t('ide-layout-single')}>
              <button type="button" className="ide-iconbtn" aria-pressed={layout === 'single'} onClick={() => setLayout('single')} aria-label={t('ide-layout-single')} title={t('ide-layout-single')}><Icon name="single" /></button>
              <button type="button" className="ide-iconbtn" aria-pressed={layout === 'grid'} onClick={() => setLayout('grid')} aria-label={t('ide-layout-grid')} title={t('ide-layout-grid')}><Icon name="grid" /></button>
            </div>}
          </div>
          {shown.length === 0 ? (
            <div className="grid min-h-0 flex-1 place-items-center overflow-y-auto p-6">
              <div className="grid w-full max-w-md gap-5">
                <div className="grid justify-items-start gap-1">
                  <Icon name="terminal" className="size-8 text-accent" />
                  <h2 className="m-0 mt-2 text-base normal-case tracking-normal text-ink">{t('ide-welcome-title')}</h2>
                  <p className="muted m-0">{t('console-wall-empty')}</p>
                </div>
                {onStart && <button type="button" className="button button--primary justify-self-start" onClick={() => onStart()}><Icon name="plus" />{t('action-start-agent')}</button>}
                <div className="grid gap-1">
                  <h2 className="m-0">{t('ide-welcome-running')}</h2>
                  {fleet.length === 0 ? <p className="muted m-0">{t('ide-welcome-none')}</p> : <ul className="m-0 grid list-none gap-0.5 p-0">
                    {fleet.slice(0, 6).map((a) => <li key={a.id}><button type="button" className="ide-row py-1.5" onClick={() => select(a.id)}><StateMark state={a.state} /><span className="min-w-0 flex-1 truncate">{a.name}</span><span className="truncate text-xs text-muted">{a.machine}</span></button></li>)}
                  </ul>}
                </div>
                <dl className="m-0 grid gap-1.5 text-xs text-muted">
                  <div className="flex items-center gap-2"><dt><kbd className="ide-kbd">{commandKey} K</kbd></dt><dd className="m-0">{t('ide-welcome-search')}</dd></div>
                  <div className="flex items-center gap-2"><dt><kbd className="ide-kbd">N</kbd></dt><dd className="m-0">{t('ide-welcome-next')}</dd></div>
                </dl>
              </div>
            </div>
          ) : (
            <div role={layout === 'single' ? 'tabpanel' : undefined} aria-labelledby={layout === 'single' && current ? `tab-${current.id}` : undefined}
              className={`grid min-h-0 flex-1 gap-px bg-line ${shown.length > 1 ? 'lg:grid-cols-2 lg:[&>*:last-child:nth-child(odd)]:col-span-2' : ''}`} style={{ gridAutoRows: 'minmax(0, 1fr)' }}>{shown.map(pane)}</div>
          )}
        </main>

        {showDetails && <Inspector attention={attention} approvals={approvals} notice={notice} changed={changed} boards={boards} messages={messages} usage={usage} projects={projects} onOpen={onOpen}
          onResolve={onResolve} onReview={loadDiff ? setReviewing : undefined} onCollapse={() => setShowDetails(false)} />}
      </div>

      <footer className="ide-bar h-6 flex-wrap border-t text-[11px] text-muted" aria-label={t('console-activity')}>
        <a className="inline-flex items-center gap-1.5 hover:text-ink" href="#/machines"><span aria-hidden className={`size-1.5 rounded-full ${link === 'live' ? 'bg-finished' : 'bg-needs-you'}`} />{t('machines-online', { count: machines.filter((m) => m.online).length })}</a>
        <span>{t('ide-status-agents', { count: fleet.filter((a) => !a.closed).length })}</span>
        {needs > 0 && <button type="button" className="text-needs-you hover:underline" onClick={() => setShowDetails(true)}>{t('ide-status-needs', { count: needs })}</button>}
        <span className="flex-1" />
        {usage && usage.totalMicroUsd > 0 && <span>{money(usage.totalMicroUsd)}</span>}
      </footer>

      {start}
      {history && loadHistory && find(history) && <HistoryDialog agent={find(history) as AgentTileModel} load={loadHistory} onClose={() => setHistory(undefined)} />}
      {browsing && filesOf && find(browsing) && filesOf(find(browsing) as AgentTileModel) && <FilesDialog agent={find(browsing) as AgentTileModel} files={filesOf(find(browsing) as AgentTileModel) as WorkspaceFiles} onClose={() => setBrowsing(undefined)} />}
      {reviewing && loadDiff && find(reviewing) && <DiffDialog agent={find(reviewing) as AgentTileModel} load={loadDiff} onClose={() => setReviewing(undefined)} />}
    </div>
  );
}
