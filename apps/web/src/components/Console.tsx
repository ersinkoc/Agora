import { ApprovalBanner } from './ApprovalBanner';
import { Suspense, lazy, useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore, type ReactNode } from 'react';
import { t } from '../i18n';
import { type AgentTileModel, machineKey, needsAttention, sortFleet, type ResumeAction } from '../model';
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
import { NoAskBadge } from './NoAskBadge';
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
import { forgetReasonKey } from '../net/forget';
import { Resizer } from './Resizer';
import type { StartChoice } from './StartApproval';
import { HelperGrants, type HelperGrant } from './HelperGrants';
import { Details, type DetailsTab } from './Inspector';
import { type Action, findPane, reduce, tabsOf, visiblePanes, workspaceOf } from '../layout/engine';
import { loadWorkspace, saveWorkspace } from '../layout/store';
import { agentOf, assignAttention } from '../layout/streams';
import { bindOf, defaultLayout, followOf, panesOf } from '../layout/tree';
import { Workspace } from '../layout/Workspace';
import { LayoutMenu } from '../layout/LayoutMenu';

const TerminalView = lazy(() => import('../terminal/TerminalView').then((m) => ({ default: m.TerminalView })));

const MAX_AUTO_PINNED = 4;
// The search shortcut as the keyboard spells it: ⌘ on Apple machines (the desktop app's WebKit included), Ctrl elsewhere.
const commandKey = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent) ? '⌘' : 'Ctrl';
const wide = () => typeof window === 'undefined' || !window.matchMedia || window.matchMedia('(min-width: 1024px)').matches;
// Below this width the panes are tabs of one group (no tiling on a phone; ADR-0068).
const NARROW = '(max-width: 699px)';
const subscribeNarrow = (cb: () => void) => { const m = typeof window !== 'undefined' ? window.matchMedia?.(NARROW) : undefined; m?.addEventListener?.('change', cb); return () => m?.removeEventListener?.('change', cb); };
const isNarrow = () => typeof window !== 'undefined' && !!window.matchMedia?.(NARROW).matches;
// The layout's global panels and the details tab that draws each.
const PANEL_TAB: Record<string, DetailsTab> = { approvals: 'needs', changes: 'changes', tasks: 'board', messages: 'messages', usage: 'cost' };

// The one window, laid out like an editor: the agents on the left as a tree, the terminals you are watching as tabs in the
// middle (or side by side), details on the right, and a status bar. Everything is derived from props.
export function Console({ agents, machines, usage, projects = [], approvals, messages, boards = [], terminalFor, chatOf, sendPrompt, chooseOption, loadDiff, loadHistory, filesOf, onOpen, onStart, onCommand, onForget, onResume, start, onResolve, helperGrants, revokeGrant, notice, link, onLock, onRetry }: {
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
  chooseOption?: (agentKey: string, kind: 'mode' | 'model', id: string) => Promise<boolean>;
  loadDiff?: (agent: AgentTileModel) => Promise<string>;
  loadHistory?: (agent: AgentTileModel) => Promise<CommitItem[]>;
  filesOf?: (agent: AgentTileModel) => WorkspaceFiles | undefined;
  onOpen: (agentId: string) => void;
  onStart?: (machine?: string) => void;
  onCommand?: () => void;
  // Ask the hub to forget a closed agent for every client (ADR-0053); false when it could not (an older hub), and it is only hidden here.
  onForget?: (agentId: string) => Promise<string | undefined>;
  // ADR-0060: resume an interrupted agent (or start it again in its folder); undefined when it did, else the error code.
  onResume?: (agent: AgentTileModel, action: ResumeAction) => Promise<string | undefined>;
  start?: ReactNode;
  onResolve: (a: ApprovalModel, approved: boolean, choice?: StartChoice) => void;
  // ADR-0057: what an agent may still start without asking, and taking that back.
  helperGrants?: (agent: AgentTileModel) => Promise<HelperGrant[]>;
  revokeGrant?: (agent: AgentTileModel, id: string) => Promise<void>;
  notice?: string;
  link: 'connecting' | 'live' | 'offline';
  // ADR-0067: sign this window out (its session ends at the hub); agents keep running.
  onLock?: () => void;
  // Try the hub now instead of after the wait, offered while it is unreachable.
  onRetry?: () => void;
}) {
  const cleared = useSyncExternalStore(subscribeDismissed, dismissedIds, dismissedIds);
  // What was cleared stays hidden only while the agent is closed: one that runs again (a new run under the same id) is back in the list.
  const fleet = useMemo(() => sortFleet(agents.filter((a) => !(a.closed && cleared.has(a.id)))), [agents, cleared]);
  const [now, setNow] = useState(Date.now());
  useEffect(() => { const id = setInterval(() => setNow(Date.now()), 15000); return () => clearInterval(id); }, []);
  const saved = useRef(loadLayout()).current;
  // The panes (ADR-0068): a tree of splits, tab groups and panes, kept for this visit and in this browser.
  const [stored, setStored] = useState(() => loadWorkspace());
  const ws = stored.ws;
  // Every layout change goes through the reducer. One the user makes (not focusing or switching tabs) marks the window as
  // arranged by them: agents stop opening on their own.
  const dispatch = useCallback((a: Action, byUser = true) => setStored((cur) => {
    const next = reduce(cur.ws, a);
    const touched = cur.touched || (byUser && next !== cur.ws && a.type !== 'focus' && a.type !== 'activate');
    return next === cur.ws && touched === cur.touched ? cur : { ws: next, touched };
  }), []);
  useEffect(() => { saveWorkspace(stored, undefined, 200); }, [stored]);
  const narrow = useSyncExternalStore(subscribeNarrow, isNarrow, () => false);
  const [selection, setSelection] = useState<string | undefined>();
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
    // The first view of a project opens its agents, so there is something of it to watch.
    if (id && !shownAgents.some((key) => fleet.some((a) => a.id === key && a.projectId === id))) {
      for (const a of fleet.filter((x) => x.projectId === id).slice(0, MAX_AUTO_PINNED)) dispatch({ type: 'open', agent: a.id, background: true });
    }
  };
  const [, textSize] = useState(0); // the size buttons follow the stored choice
  useEffect(() => { const on = () => textSize((n) => n + 1); window.addEventListener('agora-terminal-size', on); return () => window.removeEventListener('agora-terminal-size', on); }, []);
  const [showAgents, setShowAgents] = useState(saved?.showAgents ?? wide());
  const [showDetails, setShowDetails] = useState(saved?.showDetails ?? wide());
  const [leftWidth, setLeftWidth] = useState(saved?.leftWidth ?? 272);
  const [rightWidth, setRightWidth] = useState(saved?.rightWidth ?? 336);
  const [reviewing, setReviewing] = useState<string | undefined>();
  const [browsing, setBrowsing] = useState<string | undefined>();
  const [history, setHistory] = useState<string | undefined>();

  // Until the user arranges the window, watch the first few agents (those that need you come first). A new agent joins while there
  // is room; one that is gone leaves (its tab closes, a pane of its own stays empty). Nothing is reordered, so a terminal never jumps.
  useEffect(() => {
    // Before the first snapshot there is no fleet to compare with: keep what was open rather than dropping it.
    if (fleet.length === 0) return;
    setStored((cur) => {
      let next = reduce(cur.ws, { type: 'prune', alive: new Set(fleet.map((a) => a.id)) });
      if (!cur.touched) {
        const shown = new Set(panesOf(next.layout.root).map(bindOf).filter((x): x is string => !!x));
        const room = fleet.filter((a) => !a.closed && a.state !== 'finished' && a.state !== 'unknown' && !shown.has(a.id)).slice(0, Math.max(0, MAX_AUTO_PINNED - shown.size));
        for (const a of room) next = reduce(next, { type: 'open', agent: a.id, background: true });
      }
      return next === cur.ws ? cur : { ...cur, ws: next };
    });
  }, [fleet, stored.touched]);

  // Panes that follow attention each show a different agent that needs you, oldest first, and keep it while it still does.
  const attentionPanes = useMemo(() => panesOf(ws.layout.root).filter((p) => followOf(p) === 'attention').map((p) => p.id), [ws.layout]);
  const waiting = useMemo(() => needsAttention(fleet).map((a, i) => [a, i] as const).sort(([a, i], [b, j]) => (a.since ?? Infinity) - (b.since ?? Infinity) || i - j).map(([a]) => a.id), [fleet]);
  const lastAttention = useRef<ReadonlyMap<string, string>>(new Map());
  const attentionMap = useMemo(() => assignAttention(attentionPanes, lastAttention.current, waiting), [attentionPanes, waiting]);
  useEffect(() => { lastAttention.current = attentionMap; }, [attentionMap]);
  const bindings = useMemo(() => ({ selection, attention: attentionMap }), [selection, attentionMap]);
  const allPanes = panesOf(ws.layout.root);
  const shownAgents = [...new Set(allPanes.map((p) => agentOf(p, bindings)).filter((x): x is string => !!x))];
  const focusedPane = findPane(ws.layout, ws.focus);
  const current = focusedPane ? fleet.find((a) => a.id === agentOf(focusedPane, bindings)) : undefined;
  useEffect(() => { saveLayout({ pinned: shownAgents, active: current?.id, layout: 'single', touched: stored.touched, showAgents, showDetails, leftWidth, rightWidth }); },
    [shownAgents.join('\n'), current?.id, stored.touched, showAgents, showDetails, leftWidth, rightWidth]); // eslint-disable-line react-hooks/exhaustive-deps

  // The tree: selecting an agent shows it in a pane that follows the selection; else it opens in the pane in focus (as a tab) or
  // comes to the front where it is shown; selecting the one in focus closes it.
  const select = (id: string) => {
    setSelection(id);
    if (allPanes.some((p) => followOf(p) === 'selection')) return;
    if (focusedPane && bindOf(focusedPane) === id && visiblePanes(ws, narrow).some((p) => p.id === focusedPane.id)) dispatch({ type: 'close', pane: focusedPane.id });
    else dispatch({ type: 'open', agent: id });
  };
  const presetAgents = () => [...new Set([...shownAgents, ...fleet.filter((a) => !a.closed).map((a) => a.id)])];
  const resetLayout = () => setStored({ ws: workspaceOf(defaultLayout()), touched: false });

  const attention = needsAttention(fleet);
  const changed = [...new Map(fleet.filter((a) => (a.filesChanged ?? 0) > 0 && (!projectFilter || a.projectId === projectFilter))
    .map((a) => [a.workspaceRef ? `${a.workspaceRef.node}/${a.workspaceRef.id}` : a.id, a])).values()]
    .sort((a, b) => (b.filesChanged ?? 0) - (a.filesChanged ?? 0));
  // An agent that is waiting and has an approval open is one thing that needs you, not two.
  const needs = attention.length + approvals.filter((x) => !attention.some((a) => a.id === x.agentId)).length;
  // Panel widths are what the user dragged them to; on a phone the panels stack and the widths do not apply.
  const cols = `${showAgents ? `${leftWidth}px ` : ''}minmax(0,1fr)${showDetails ? ` ${rightWidth}px` : ''}`
  const find = (id?: string) => fleet.find((a) => a.id === id);

  // Editor habits: Alt+1…9 picks a tab of the group in focus (or a pane, when it has no tabs), Ctrl/⌘+B hides or shows the agent list.
  useEffect(() => {
    const on = (e: KeyboardEvent) => {
      if (e.altKey && !e.ctrlKey && !e.metaKey && /^[1-9]$/.test(e.key)) {
        const group = !narrow && !ws.zoom ? tabsOf(ws.layout, ws.focus) : undefined;
        const target = (group ?? (narrow ? panesOf(ws.layout.root) : visiblePanes(ws, narrow)))[Number(e.key) - 1];
        if (target) { e.preventDefault(); dispatch({ type: 'activate', pane: target.id }); }
      } else if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === 'b') { e.preventDefault(); setShowAgents((v) => !v); }
    };
    window.addEventListener('keydown', on);
    return () => window.removeEventListener('keydown', on);
  }, [ws, narrow, dispatch]);

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

  // ADR-0060: Resume or Start again, and why it did not happen, per agent.
  const [resumeError, setResumeError] = useState<{ id: string; code: string } | undefined>();
  const [resuming, setResuming] = useState<string | undefined>();
  const resume = onResume ? (a: AgentTileModel, action: ResumeAction) => {
    setResumeError(undefined);
    setResuming(a.id);
    void onResume(a, action).then((code) => { setResuming(undefined); if (code) setResumeError({ id: a.id, code }); });
  } : undefined;

  // One agent as a pane draws it: where it runs and its tools, a request waiting for you, then its terminal, chat or summary.
  const agentBody = (a: AgentTileModel) => (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <div className="flex min-h-9 flex-none flex-wrap items-center gap-x-3 gap-y-1 border-b border-line bg-surface-1 px-3 py-1">
        <div className="flex min-w-0 flex-1 basis-56 items-center gap-1.5 text-xs text-muted" title={`${a.machine} · ${a.workspace}`}>
          <Icon name="machine" className="size-3.5" /><span className="truncate">{a.machine}</span>
          <Icon name="chevron" className="size-3 rtl:rotate-180" /><Icon name="folder" className="size-3.5" /><span className="truncate">{a.project || a.workspace}</span>
          {a.branch && <><Icon name="branch" className="ms-1 size-3.5" /><span className="truncate font-mono">{a.branch}</span></>}
          {a.shared && <span className="ide-chip ms-1 flex-none" title={t('workspace-shared')}>{t('ide-shared')}</span>}
          <NoAskBadge agent={a} />
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
        </div>
      </div>
      {approvals.filter((x) => x.agentId === a.id).map((x) => <ApprovalBanner key={x.id} approval={x} onResolve={onResolve} />)}
      <AgentAddress agent={a} />
      {helperGrants && revokeGrant && !a.closed && fleet.some((c) => c.parentId === a.id) &&
        <HelperGrants list={() => helperGrants(a)} revoke={(id) => revokeGrant(a, id)} refresh={fleet.filter((c) => c.parentId === a.id && !c.closed).length} />}
      {a.closed ? <ClosedSummary agent={a} now={now} cost={usage?.perAgent.has(a.id) ? money(usage.perAgent.get(a.id) ?? 0) : undefined}
        onReview={loadDiff ? () => setReviewing(a.id) : undefined} onHistory={loadHistory && a.git !== false ? () => setHistory(a.id) : undefined} onFiles={filesOf ? () => setBrowsing(a.id) : undefined}
        onStart={onStart ? () => onStart() : undefined} onDismiss={() => dismiss([a.id])}
        onForget={onForget ? () => removeClosed([a.id]) : undefined} forgetError={forgetError}
        onResume={resume && a.interrupted ? (action) => resume(a, action) : undefined} resuming={resuming === a.id}
        resumeError={resumeError?.id === a.id ? resumeError.code : undefined} /> : <div className="pane min-h-0 flex-1 p-1.5">
        {a.terminal === false ? <PromptBox agent={a} send={sendPrompt} source={chatOf?.(a)} choose={chooseOption} /> : (
          <Suspense fallback={<div className="terminal" aria-busy="true" />}>
            <TerminalView source={terminalFor(a.id)} label={`${t('focus-terminal')} · ${a.name}`} />
          </Suspense>
        )}
      </div>}
    </div>
  );
  const panel = (id: string) => PANEL_TAB[id] ? <div className="min-h-0 flex-1 overflow-y-auto p-3">
    <Details tab={PANEL_TAB[id] as DetailsTab} attention={attention} approvals={approvals} notice={notice} changed={changed} boards={boards} messages={messages} usage={usage} projects={projects} onOpen={onOpen}
      onResolve={onResolve} onReview={loadDiff ? setReviewing : undefined} />
  </div> : undefined;

  // Nothing open: a start button, the agents that are running, and the two shortcuts.
  const welcome = (
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
        {link === 'offline' && onRetry && <button type="button" className="ide-chip whitespace-nowrap hover:border-accent hover:text-ink" onClick={onRetry}>{t('link-retry')}</button>}
        <LayoutMenu ws={ws} dispatch={dispatch} agents={presetAgents} onReset={resetLayout} narrow={narrow} />
        <span className="hidden sm:block"><LanguagePicker compact /></span>
        <ThemePicker />
        <a className="ide-chip hidden whitespace-nowrap hover:text-ink md:inline-flex" href="#/device">{t('device-link')}</a>
        {onLock && <button type="button" className="ide-chip whitespace-nowrap hover:text-ink" onClick={onLock} title={t('lock-now-hint')}><Icon name="lock" className="size-3.5" />{t('lock-now')}</button>}
        {!showDetails && <button type="button" className="ide-iconbtn" onClick={() => setShowDetails(true)} aria-label={t('ide-show-details')} title={t('ide-show-details')}><Icon name="panelRight" /></button>}
      </header>

      <div className="relative grid min-h-0 grid-cols-1 lg:[grid-template-columns:var(--cols)]" style={{ ['--cols' as string]: cols }}>
        {showAgents && <div className="absolute inset-y-0 z-10 hidden lg:block" style={{ insetInlineStart: leftWidth }}><Resizer side="start" value={leftWidth} min={200} max={480} onChange={setLeftWidth} /></div>}
        {showDetails && <div className="absolute inset-y-0 z-10 hidden lg:block" style={{ insetInlineEnd: rightWidth }}><Resizer side="end" value={rightWidth} min={260} max={560} onChange={setRightWidth} /></div>}
        {showAgents && <Explorer fleet={projectFilter ? fleet.filter((a) => a.projectId === projectFilter) : fleet} machines={machines} projects={projectOptions} project={projectFilter} onProject={chooseProject} open={shownAgents} current={current?.id} onToggle={select} onStart={onStart} onCollapse={() => setShowAgents(false)} onDismiss={removeClosed} everyone={!!onForget} forgetError={forgetError} onResume={resume} resuming={resuming} resumeError={resumeError} />}

        <main className="flex min-h-0 min-w-0 flex-col" aria-label={t('focus-terminal')}>
          <Workspace ws={ws} dispatch={dispatch} fleet={fleet} bindings={bindings} narrow={narrow} renderAgent={agentBody} renderPanel={panel}
            onFiles={filesOf ? (a) => setBrowsing(a.id) : undefined} onStart={onStart ? () => onStart() : undefined} welcome={welcome} />
        </main>

        {showDetails && <Inspector attention={attention} approvals={approvals} notice={notice} changed={changed} boards={boards} messages={messages} usage={usage} projects={projects} onOpen={onOpen}
          onResolve={onResolve} onReview={loadDiff ? setReviewing : undefined} onCollapse={() => setShowDetails(false)} />}
      </div>

      <footer className="ide-bar h-6 flex-wrap border-t text-[11px] text-muted" aria-label={t('console-activity')}>
        <a className="inline-flex items-center gap-1.5 hover:text-ink" href="#/machines"><span aria-hidden className={`size-1.5 rounded-full ${link === 'live' ? 'bg-finished' : 'bg-needs-you'}`} />{t('machines-online', { count: machines.filter((m) => m.online).length })}</a>
        <span>{t('ide-status-agents', { count: fleet.filter((a) => !a.closed).length })}</span>
        {needs > 0 && <button type="button" className="text-needs-you hover:underline" onClick={() => setShowDetails(true)}>{t('ide-status-needs', { count: needs })}</button>}
        {forgetError && <span role="alert" className="truncate text-needs-you">{t('closed-forget-failed', { reason: t(forgetReasonKey(forgetError)) })}</span>}
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
