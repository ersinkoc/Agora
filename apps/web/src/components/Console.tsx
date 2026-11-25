import type { ActionResult } from '../net/refusal';
import { asksUnderHeader } from '../net/chatView';
import { ApprovalBanners } from './ApprovalBanner';
import { Arrangement, requestShown, useAnswerGuard, type AnswerResult } from './useAnswerGuard';
import { Suspense, lazy, useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore, type ReactNode } from 'react';
import { t } from '../i18n';
import { type AgentTileModel, endedWithError, machineKey, needsAttention, sortFleet, type ResumeAction } from '../model';
import type { UsageSummary } from '../net/usage';
import type { ApprovalModel, BoardModel, MachineModel, MessageModel, ProjectCost } from '../net/view';
import type { TerminalSource } from '../terminal/source';
import { DiffDialog } from './DiffDialog';
import { PromptBox } from './PromptBox';
import type { PromptSend } from './chatActions';
import type { ChatSource } from '../net/chat';
import { FilesDialog } from './FilesDialog';
import { HistoryDialog } from './HistoryDialog';
import type { CommitItem } from '../net/history';
import type { DiffSource } from '../net/diff';
import type { WorkspaceFiles } from '../net/files';
import { AgentAddress } from './AgentAddress';
import { NoAskBadge } from './NoAskBadge';
import { AsksLessChip } from './AllowAlways';
import { LiveSummary } from './LiveSummary';
import { Explorer } from './Explorer';
import { FleetConsole } from './FleetConsole';
import type { PairingUi } from './Pairing';
import { HeaderLinks } from './HeaderLinks';
import { Icon } from './Icon';
import { CloseAgentMenu, CloseConfirm } from './CloseAgent';
import { Inspector } from './Inspector';
import { saveGroupBy, useGroupBy } from '../net/fleetPrefs';
import { projectSpread } from '../net/fleet';
import { loadRailTab, saveRailTab, type RailTab } from './rail';
import { LanguagePicker } from './LanguagePicker';
import { ThemePicker } from './ThemePicker';
import { MAX_SIZE, MIN_SIZE, setTerminalSize, terminalSize } from '../terminal/prefs';
import { TerminalSettings } from '../terminal/TerminalSettings';
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
import { LAYOUT_VERSION, bindOf, followOf, panesOf } from '../layout/tree';
import { Workspace } from '../layout/Workspace';
import { arrangementKey } from '../layout/geometry';
import { LayoutMenu } from '../layout/LayoutMenu';
import { subscribePanelRequests, subscribeProjectRequests, subscribeToolRequests, takePanelRequest, takeProjectRequest, takeToolRequests, type ToolRequest } from '../layout/requests';
import { type ToolPanel, changedFolders, isToolPanel, soleTarget, toolTarget } from '../net/paneTools';
import { Menu, type MenuItem } from '../layout/Menu';
import { ChangesPane, FilesPane, ManagerPane, TerminalPane, ToolChooser } from './ToolPanes';
import { ThemeEditor } from '../themes/ThemeEditor';

const TerminalView = lazy(() => import('../terminal/TerminalView').then((m) => ({ default: m.TerminalView })));

const MAX_AUTO_PINNED = 4;
// The icon rail of the right panel (rem, so it grows with the text size like the rail itself, `w-11`), always there; the panel's own width comes on top when it is open.
const RAIL_WIDTH = '2.75rem';
// The search shortcut as the keyboard spells it: ⌘ on Apple machines (the desktop app's WebKit included), Ctrl elsewhere.
const commandKey = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent) ? '⌘' : 'Ctrl';
const homeWorkspace = () => workspaceOf({ version: LAYOUT_VERSION, root: { type: 'pane', id: 'p1', panel: 'fleet' } });
const wide = () => typeof window === 'undefined' || !window.matchMedia || window.matchMedia('(min-width: 1024px)').matches;
// Below this width the panes are tabs of one group (no tiling on a phone; ADR-0068).
const NARROW = '(max-width: 699px)';
const subscribeNarrow = (cb: () => void) => { const m = typeof window !== 'undefined' ? window.matchMedia?.(NARROW) : undefined; m?.addEventListener?.('change', cb); return () => m?.removeEventListener?.('change', cb); };
const isNarrow = () => typeof window !== 'undefined' && !!window.matchMedia?.(NARROW).matches;
// The layout's global panels and the details tab that draws each.
// The layout's first panel is Home itself: the fleet console.
const PANEL_TAB: Record<string, DetailsTab> = { approvals: 'needs', tasks: 'board', messages: 'messages', usage: 'usage' };

// The one window, laid out like an editor: the agents on the left as a tree, the terminals you are watching as tabs in the
// middle (or side by side), details on the right, and a status bar. Everything is derived from props.
export function Console({ agents, machines, usage, projects = [], approvals, messages, boards = [], terminalFor, chatOf, sendPrompt, chooseOption, loadDiff, loadHistory, filesOf, onOpen, onStart, onNewTerminal, onCommand, onForget, onResume, start, onResolve, helperGrants, revokeGrant, notice, link, onLock, onRetry, pairing, guest, watching }: {
  agents: readonly AgentTileModel[];
  machines: readonly MachineModel[];
  usage?: UsageSummary;
  projects?: readonly ProjectCost[];
  approvals: readonly ApprovalModel[];
  messages: readonly MessageModel[];
  boards?: readonly BoardModel[];
  terminalFor: (agentId: string) => TerminalSource;
  chatOf?: (agent: AgentTileModel) => ChatSource | undefined;
  sendPrompt?: PromptSend;
  chooseOption?: (agentKey: string, kind: 'mode' | 'model', id: string) => Promise<ActionResult>;
  loadDiff?: (agent: AgentTileModel, path?: string) => Promise<DiffSource>;
  loadHistory?: (agent: AgentTileModel) => Promise<CommitItem[]>;
  filesOf?: (agent: AgentTileModel) => WorkspaceFiles | undefined;
  onOpen: (agentId: string) => void;
  onStart?: (machine?: string) => void;
  // "New terminal": starts a plain terminal on a machine; it opens as a Terminal pane of this window.
  onNewTerminal?: (machine?: string) => void;
  onCommand?: () => void;
  // Ask the hub to forget a closed agent for every client (ADR-0053); false when it could not (an older hub), and it is only hidden here.
  onForget?: (agentId: string) => Promise<string | undefined>;
  // ADR-0060: resume an interrupted agent (or start it again in its folder); undefined when it did, else the error code.
  onResume?: (agent: AgentTileModel, action: ResumeAction) => Promise<string | undefined>;
  start?: ReactNode;
  // Left out in a viewer's window (ADR-0106): every request is shown read-only, with no answers.
  onResolve?: (a: ApprovalModel, approved: boolean, choice?: StartChoice, always?: boolean) => AnswerResult;
  // ADR-0057: what an agent may still start without asking, and taking that back.
  helperGrants?: (agent: AgentTileModel) => Promise<HelperGrant[]>;
  revokeGrant?: (agent: AgentTileModel, id: string) => Promise<void>;
  notice?: string;
  link: 'connecting' | 'live' | 'offline';
  // ADR-0067: sign this window out (its session ends at the hub); agents keep running.
  onLock?: () => void;
  // Try the hub now instead of after the wait, offered while it is unreachable.
  onRetry?: () => void;
  // ADR-0065: machines asking to join, for an owner's session.
  pairing?: PairingUi;
  // The hub said this session is not the owner's: an empty window says no project is theirs yet (#904).
  guest?: boolean;
  // ADR-0106: the hub said this person is a viewer; the window only watches.
  watching?: boolean;
}) {
  const cleared = useSyncExternalStore(subscribeDismissed, dismissedIds, dismissedIds);
  // What was cleared stays hidden only while the agent is closed: one that runs again (a new run under the same id) is back in the list.
  const fleet = useMemo(() => sortFleet(agents.filter((a) => !(a.closed && cleared.has(a.id)))), [agents, cleared]);
  const groupBy = useGroupBy();
  const spread = useMemo(() => projectSpread(fleet), [fleet]);
  const [now, setNow] = useState(Date.now());
  useEffect(() => { const id = setInterval(() => setNow(Date.now()), 15000); return () => clearInterval(id); }, []);
  const saved = useRef(loadLayout()).current;
  // The panes (ADR-0068): a tree of splits, tab groups and panes, kept for this visit and in this browser.
  const [stored, setStored] = useState(() => {
    // A window nobody has arranged opens on Home, the fleet console; agents opened for you wait in tabs beside it.
    const cur = loadWorkspace();
    return !cur.touched && panesOf(cur.ws.layout.root).every((p) => p.panel === 'empty') ? { ws: homeWorkspace(), touched: false } : cur;
  });
  const ws = stored.ws;
  // Every layout change goes through the reducer. One the user makes (not focusing or switching tabs) marks the window as
  // arranged by them: agents stop opening on their own.
  const dispatch = useCallback((a: Action, byUser = true) => setStored((cur) => {
    const next = reduce(cur.ws, a);
    const touched = cur.touched || (byUser && next !== cur.ws && a.type !== 'focus' && a.type !== 'activate');
    return next === cur.ws && touched === cur.touched ? cur : { ws: next, touched };
  }), []);
  useEffect(() => { saveWorkspace(stored, undefined, 200); }, [stored]);
  // A panel asked for from outside the window (the command palette, This device), also one asked for just before it opened.
  useEffect(() => {
    const take = () => { const panel = takePanelRequest(); if (panel) dispatch({ type: 'show', panel }); };
    take();
    return subscribePanelRequests(take);
  }, [dispatch]);
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
  // A project asked for from outside (the command palette): its agents are shown, as when it is chosen in the list.
  useEffect(() => {
    const take = () => { const id = takeProjectRequest(); if (id) { setShowAgents(true); chooseProject(id); } };
    take();
    return subscribeProjectRequests(take);
  });
  const [, textSize] = useState(0); // the size buttons follow the stored choice
  useEffect(() => { const on = () => textSize((n) => n + 1); window.addEventListener('agora-terminal-size', on); return () => window.removeEventListener('agora-terminal-size', on); }, []);
  const [showAgents, setShowAgents] = useState(saved?.showAgents ?? wide());
  const [showDetails, setShowDetails] = useState(saved?.showDetails ?? wide());
  // The right panel's rail: which subject it opens on, remembered in this browser (whether it is open is part of the saved layout).
  const [railTab, setRailTabState] = useState<RailTab>(() => loadRailTab() ?? 'needs');
  const setRailTab = useCallback((x: RailTab, remember = true) => { setRailTabState(x); if (remember) saveRailTab(x); }, []);
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
  // The tools (Terminal, Changes, Files, File manager) as panes beside the agent in focus. A pane that already shows that tool for that
  // agent comes to the front instead of a second one; a terminal for an agent that is already on screen brings that agent forward.
  const openTool = (panel: ToolPanel, agentId?: string) => {
    const agent = agentId ?? toolTarget(panel, current) ?? soleTarget(panel, fleet);
    const shown = panel === 'terminal' && agent ? allPanes.find((p) => bindOf(p) === agent && (p.panel === 'agent' || p.panel === 'terminal')) : undefined;
    if (shown) dispatch({ type: 'activate', pane: shown.id });
    else dispatch({ type: 'show', panel, ...(agent ? { bind: { agent } } : {}) });
  };
  // Requests from outside (the command palette, a terminal just started): one for an agent the fleet does not hold yet waits for it.
  const toolRef = useRef(openTool);
  toolRef.current = openTool;
  const fleetIds = useRef(new Set<string>());
  fleetIds.current = new Set(fleet.map((a) => a.id));
  const waitingTools = useRef<(ToolRequest & { at: number })[]>([]);
  const runTools = useCallback(() => {
    const all = [...waitingTools.current, ...takeToolRequests().map((r) => ({ ...r, at: Date.now() }))];
    waitingTools.current = [];
    for (const r of all) {
      if (r.wait && r.agent && !fleetIds.current.has(r.agent)) { if (Date.now() - r.at < 30_000) waitingTools.current.push(r); continue; }
      if (isToolPanel(r.panel)) toolRef.current(r.panel, r.agent);
    }
  }, []);
  useEffect(() => { runTools(); return subscribeToolRequests(runTools); }, [runTools]);
  useEffect(() => { if (waitingTools.current.length > 0) runTools(); }, [fleet, runTools]);
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
  const resetLayout = () => setStored({ ws: homeWorkspace(), touched: false });

  // Where the panes are drawn and what they show (#1048): a move, a split, a resize, another tab in front or a pane that shows another
  // agent restarts the dwell of every request card in this window (`Arrangement`), the side panel's and Home's included; never a pane
  // taking the focus.
  const arrangement = useMemo(() => arrangementKey(ws, narrow, [showAgents ? leftWidth : 0, showDetails ? rightWidth : 0], (p) => agentOf(p, bindings)), [ws, narrow, showAgents, leftWidth, showDetails, rightWidth, bindings]);
  // T44 (`useAnswerGuard`, as Home's queue): one guard for every card above a terminal or a composer in this window's panes. Its layout is
  // every request with what its card shows and where the panes are, so a card that comes, goes or changes in any pane, or a pane that moves,
  // restarts the dwell everywhere; a request is answered once, whichever pane (or other surface: the window's holds) it was answered in.
  const banners = useAnswerGuard(approvals.map((x) => x.id), (status) => JSON.stringify([arrangement, approvals.map((x) => { const a = fleet.find((f) => f.id === x.agentId); return [requestShown(x, a), a ? asksUnderHeader(a) : null, status(x.id)]; })]), link !== 'live');
  const attention = needsAttention(fleet);
  const changed = changedFolders(fleet, projectFilter);
  // An agent that is waiting and has an approval open is one thing that needs you, not two.
  const needs = attention.length + approvals.filter((x) => !attention.some((a) => a.id === x.agentId)).length + (pairing?.cards.length ?? 0);
  // Panel widths are what the user dragged them to; on a phone the panels stack and the widths do not apply.
  const cols = `${showAgents ? `${leftWidth}px ` : ''}minmax(0,1fr) ${showDetails ? `calc(${rightWidth}px + ${RAIL_WIDTH})` : RAIL_WIDTH}`
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

  // What a chat agent waits on, pinned above its composer (a terminal agent's requests are a bar under its header).
  const pinnedAsks = (agentId: string) => {
    const mine = approvals.filter((x) => x.agentId === agentId);
    const agent = fleet.find((f) => f.id === agentId);
    return mine.length ? <div className="grid gap-2"><ApprovalBanners approvals={mine} agent={agent} guard={banners} onResolve={onResolve} pinned /></div> : undefined;
  };
  // One agent as a pane draws it: where it runs and its tools, a request waiting for you, then its terminal, chat or summary.
  const agentBody = (a: AgentTileModel) => (
    <div className="agent-body flex min-h-0 min-w-0 flex-1 flex-col">
      {/* Redesign PR 7: in a narrow pane the machine and the branch leave this row (they stay in its tooltip) so the folder is read whole,
          not three single letters, and the tools wrap inside the pane instead of running into the next one (app.css `.agent-body`). */}
      <div className="flex min-h-9 flex-none flex-wrap items-center gap-x-3 gap-y-1 border-b border-line bg-surface-1 px-3 py-1">
        <div className="flex min-w-0 flex-1 basis-56 items-center gap-1.5 text-xs text-muted" title={`${a.machine} · ${a.workspace}${a.branch ? ` · ${a.branch}` : ''}`}>
          <span className="flex min-w-0 items-center gap-1.5" data-crumb="machine"><Icon name="machine" className="size-3.5 flex-none" /><span className="truncate">{a.machine}</span>
          <Icon name="chevron" className="size-3 flex-none rtl:rotate-180" /></span><Icon name="folder" className="size-3.5 flex-none" /><span className="min-w-[4ch] truncate">{a.project || a.workspace}</span>
          {a.branch && <span className="flex min-w-0 items-center gap-1.5" data-crumb="branch"><Icon name="branch" className="ms-1 size-3.5 flex-none" /><span className="truncate font-mono">{a.branch}</span></span>}
          {a.shared && <span className="ide-chip ms-1 flex-none" title={t('workspace-shared')}>{t('ide-shared')}</span>}
          <NoAskBadge agent={a} />
          <AsksLessChip agent={a} />
        </div>
        <div className="flex max-w-full flex-wrap items-center gap-1">
          {(a.filesChanged ?? 0) > 0 && (loadDiff
            ? <button type="button" className="ide-chip hover:text-ink" onClick={() => openTool('changes', a.id)} title={t('pane-open-changes')}>{t('changes-files', { count: a.filesChanged ?? 0 })}</button>
            : <span className="ide-chip">{t('changes-files', { count: a.filesChanged ?? 0 })}</span>)}
          {!(a.terminal === false && !a.closed) && <StateBadge state={a.state} failed={endedWithError(a)} />}
          <CloseAgentMenu agent={a} />
          {a.terminal !== false && !a.closed && <span className="flex items-center" role="group" aria-label={t('terminal-size')}>
            <button type="button" className="ide-iconbtn w-6 text-xs font-semibold" onClick={() => setTerminalSize(terminalSize() - 1)} disabled={terminalSize() <= MIN_SIZE} aria-label={t('terminal-smaller')} title={t('terminal-smaller')}>A−</button>
            <button type="button" className="ide-iconbtn w-6 text-[0.8125rem] font-semibold" onClick={() => setTerminalSize(terminalSize() + 1)} disabled={terminalSize() >= MAX_SIZE} aria-label={t('terminal-larger')} title={t('terminal-larger')}>A+</button>
          </span>}
          {a.terminal !== false && !a.closed && <TerminalSettings />}
          {loadHistory && a.git !== false && <button type="button" className="ide-iconbtn" onClick={() => setHistory(a.id)} aria-label={t('console-history')} title={t('console-history')}><Icon name="history" /></button>}
          {filesOf && <button type="button" className="ide-iconbtn" onClick={() => openTool('files', a.id)} aria-label={t('console-files')} title={t('console-files')}><Icon name="files" /></button>}
          {(filesOf || loadDiff) && !a.closed && <Menu label={t('pane-open-beside')} items={() => beside(a)}><Icon name="splitRight" /></Menu>}
          <button type="button" className="ide-iconbtn" onClick={() => onOpen(a.id)} aria-label={t('console-maximize')} title={t('console-maximize')}><Icon name="maximize" /></button>
        </div>
      </div>
      <CloseConfirm agent={a} where="pane" />
      {!asksUnderHeader(a) ? null : <ApprovalBanners approvals={approvals.filter((x) => x.agentId === a.id)} agent={a} guard={banners} onResolve={onResolve} />}
      <AgentAddress agent={a} />
      {helperGrants && revokeGrant && !a.closed && fleet.some((c) => c.parentId === a.id) &&
        <HelperGrants view={a.id} list={() => helperGrants(a)} revoke={(id) => revokeGrant(a, id)} refresh={fleet.filter((c) => c.parentId === a.id && !c.closed).length} />}
      {a.closed ? <ClosedSummary agent={a} now={now} source={a.terminal === false ? chatOf?.(a) : undefined}
        onReview={loadDiff ? () => setReviewing(a.id) : undefined} onHistory={loadHistory && a.git !== false ? () => setHistory(a.id) : undefined} onFiles={filesOf ? () => setBrowsing(a.id) : undefined}
        onStart={onStart ? () => onStart() : undefined} onDismiss={() => dismiss([a.id])}
        onForget={onForget ? () => removeClosed([a.id]) : undefined} forgetError={forgetError}
        onResume={resume && a.interrupted ? (action) => resume(a, action) : undefined} resuming={resuming === a.id}
        resumeError={resumeError?.id === a.id ? resumeError.code : undefined} /> : <>
        {a.terminal !== false && <LiveSummary agent={a} now={now} onChanges={() => { setRailTab('changes'); setShowDetails(true); }} />}
        <div className="pane min-h-0 flex-1 p-1.5">
        {a.terminal === false ? <PromptBox key={a.id} agent={a} send={sendPrompt} source={chatOf?.(a)} choose={chooseOption} onReview={loadDiff ? () => setReviewing(a.id) : undefined} approvals={pinnedAsks(a.id)} /> : (
          <Suspense fallback={<div className="terminal-host" aria-busy="true" />}>
            <TerminalView source={terminalFor(a.id)} label={`${t('focus-terminal')} · ${a.name}`} />
          </Suspense>
        )}
      </div></>}
    </div>
  );
  // "Open beside" in an agent's header: its terminal (unless it is this one), changes, files and file manager, each as a pane next to it.
  const beside = (a: AgentTileModel): MenuItem[] => [
    ...(['changes', 'files', 'manager'] as const).filter((p) => (p === 'changes' ? !!loadDiff : !!filesOf)).map((p) => ({ label: t({ changes: 'ide-tab-changes', files: 'console-files', manager: 'layout-panel-manager' }[p]), run: () => openTool(p, a.id) })),
  ];
  const renderTool = (tool: ToolPanel, a: AgentTileModel | undefined, rebind: (id: string) => void): ReactNode => {
    if (!a) return <ToolChooser panel={tool} fleet={fleet} onPick={rebind} onNewTerminal={onNewTerminal ? () => onNewTerminal() : undefined} />;
    if (tool === 'terminal') return <TerminalPane agent={a} source={terminalFor(a.id)} />;
    if (tool === 'changes') return <ChangesPane agent={a} fleet={fleet} loadDiff={loadDiff} filesOf={filesOf} onPick={rebind} />;
    if (tool === 'files') return <FilesPane agent={a} fleet={fleet} loadDiff={loadDiff} filesOf={filesOf} onPick={rebind} />;
    return <ManagerPane agent={a} fleet={fleet} filesOf={filesOf} onPick={rebind} onTerminal={(x) => openTool('terminal', x.id)} />;
  };
  // The + of a tab group: start something, or open a tool beside the agent in focus.
  const plusItems = (): MenuItem[] => [
    ...(onStart ? [{ label: t('action-start-agent'), run: () => onStart() }] : []),
    ...(onNewTerminal ? [{ label: t('pane-new-terminal'), run: () => onNewTerminal() }] : []),
    ...(['terminal', 'changes', 'files', 'manager'] as const).map((p, i) => ({ label: t(`palette-open-${p}`), divider: i === 0 && (!!onStart || !!onNewTerminal), run: () => openTool(p) })),
  ];
  const panel = (id: string) => id === 'fleet' ? home : PANEL_TAB[id] ? <div className="min-h-0 flex-1 overflow-y-auto p-3">
    <Details tab={PANEL_TAB[id] as DetailsTab} fleet={fleet} now={now} pairing={pairing} attention={attention} approvals={approvals} notice={notice} changed={changed} boards={boards} messages={messages} usage={usage} projects={projects} onOpen={onOpen}
      onResolve={onResolve} stale={link !== 'live'} onReview={loadDiff ? setReviewing : undefined} />
  </div> : id === 'theme' ? <div className="min-h-0 flex-1 overflow-y-auto"><ThemeEditor /></div> : undefined;

  // Home: the fleet console. It is what an empty window shows and what the Fleet tab is. A tile opens its agent like the tree does.
  const openFromFleet = (id: string) => {
    setSelection(id);
    if (allPanes.some((p) => followOf(p) === 'selection')) return;
    dispatch({ type: 'open', agent: id });
  };
  const home = (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <FleetConsole fleet={fleet} machines={machines} approvals={approvals} usage={usage} link={link} now={now} onOpen={openFromFleet} onStart={onStart} onResolve={onResolve} onReview={() => { setRailTab('needs'); setShowDetails(true); }} onRetry={onRetry} pairing={pairing} guest={guest} watching={watching} />
    </div>
  );
  const welcome = home;

  return (
    <Arrangement.Provider value={arrangement}>
    <div className="ide">
      <header className="ide-bar min-h-10 flex-wrap border-b py-1 sm:h-10 sm:flex-nowrap sm:py-0">
        <span aria-hidden className="grid size-6 flex-none place-items-center rounded-xs bg-accent text-xs font-bold text-on-accent">A</span>
        <h1 className="m-0 me-2 hidden text-[0.8125rem] font-semibold tracking-normal sm:block">{t('home-title')}</h1>
        {!showAgents && <button type="button" className="ide-iconbtn" onClick={() => setShowAgents(true)} aria-label={t('ide-show-agents')} title={t('ide-show-agents')}><Icon name="panelLeft" /></button>}
        <button type="button" onClick={onCommand} disabled={!onCommand} className="mx-auto flex h-7 min-w-0 max-w-md flex-1 items-center gap-2 rounded-xs border border-line bg-surface-0 px-2.5 text-start text-muted transition-colors duration-150 hover:border-accent hover:text-ink">
          <Icon name="search" className="size-3.5" /><span className="min-w-0 flex-1 truncate">{t('ide-command')}</span><kbd className="ide-kbd hidden sm:inline">{commandKey} K</kbd>
        </button>
        {link !== 'live' && <span role="status" className="ide-chip">{t(link === 'offline' ? 'link-offline' : 'link-connecting')}</span>}
        {link === 'offline' && onRetry && <button type="button" className="ide-chip whitespace-nowrap hover:border-accent hover:text-ink" onClick={onRetry}>{t('link-retry')}</button>}
        <LayoutMenu ws={ws} dispatch={dispatch} agents={presetAgents} onReset={resetLayout} narrow={narrow} />
        <span className="hidden sm:block"><LanguagePicker compact /></span>
        <ThemePicker />
        <HeaderLinks />
        {onLock && <button type="button" className="ide-chip whitespace-nowrap hover:text-ink" onClick={onLock} title={t('lock-now-hint')}><Icon name="lock" className="size-3.5" />{t('lock-now')}</button>}
      </header>

      <div className="relative grid min-h-0 grid-cols-[minmax(0,1fr)_auto] overflow-hidden lg:[grid-template-columns:var(--cols)]" style={{ ['--cols' as string]: cols }}>
        {showAgents && <div className="absolute inset-y-0 z-10 hidden lg:block" style={{ insetInlineStart: leftWidth }}><Resizer side="start" value={leftWidth} min={200} max={480} onChange={setLeftWidth} /></div>}
        {showDetails && <div className="absolute inset-y-0 z-10 hidden lg:block" style={{ insetInlineEnd: `calc(${rightWidth}px + ${RAIL_WIDTH})` }}><Resizer side="end" value={rightWidth} min={260} max={560} onChange={setRightWidth} /></div>}
        <div className="flex min-h-0 min-w-0 flex-col overflow-y-auto lg:contents">
        {showAgents && <Explorer watching={watching} by={groupBy} onBy={saveGroupBy} spread={spread} fleet={projectFilter ? fleet.filter((a) => a.projectId === projectFilter) : fleet} machines={machines} projects={projectOptions} project={projectFilter} onProject={chooseProject} open={shownAgents} current={current?.id} onToggle={select} onStart={onStart} onCollapse={() => setShowAgents(false)} onDismiss={removeClosed} everyone={!!onForget} forgetError={forgetError} onResume={resume} resuming={resuming} resumeError={resumeError} />}

        <main className="flex min-h-[55dvh] min-w-0 flex-1 flex-col lg:min-h-0" aria-label={t('focus-terminal')}>
          <Workspace ws={ws} dispatch={dispatch} fleet={fleet} bindings={bindings} narrow={narrow} renderAgent={agentBody} renderPanel={panel}
            renderTool={renderTool} plusItems={plusItems} welcome={welcome} />
        </main>
        </div>

        <Inspector fleet={fleet} now={now} pairing={pairing} attention={attention} approvals={approvals} notice={notice} changed={changed} boards={boards} messages={messages} usage={usage} projects={projects} onOpen={onOpen}
          onResolve={onResolve} stale={link !== 'live'} onReview={loadDiff ? setReviewing : undefined} onPane={loadDiff ? (id) => openTool('changes', id) : undefined} open={showDetails} onOpenChange={setShowDetails} tab={railTab} onTab={setRailTab} />
      </div>

      <footer className="ide-bar h-6 flex-wrap border-t border-statusline-line bg-statusline text-xs text-statusline-ink" data-region="status-line" aria-label={t('console-activity')}>
        <a className="inline-flex items-center gap-1.5 hover:text-ink" href="#/machines"><span aria-hidden className={`size-1.5 rounded-full ${link === 'live' ? 'bg-finished' : 'bg-idle'}`} />{t('machines-online', { count: machines.filter((m) => m.online).length })}</a>
        <span>{t('ide-status-agents', { count: fleet.filter((a) => !a.closed).length })}</span>
        {needs > 0 && <button type="button" className="text-needs-you hover:underline" onClick={() => { setRailTab('needs'); setShowDetails(true); }}>{t('ide-status-needs', { count: needs })}</button>}
        {forgetError && <span role="alert" className="truncate text-danger">{t('closed-forget-failed', { reason: t(forgetReasonKey(forgetError)) })}</span>}
        <span className="flex-1" />
      </footer>

      {start}
      {history && loadHistory && find(history) && <HistoryDialog agent={find(history) as AgentTileModel} load={loadHistory} onClose={() => setHistory(undefined)} />}
      {browsing && filesOf && find(browsing) && filesOf(find(browsing) as AgentTileModel) && <FilesDialog agent={find(browsing) as AgentTileModel} files={filesOf(find(browsing) as AgentTileModel) as WorkspaceFiles} load={loadDiff} onClose={() => setBrowsing(undefined)} />}
      {reviewing && loadDiff && find(reviewing) && <DiffDialog agent={find(reviewing) as AgentTileModel} load={loadDiff} read={filesOf?.(find(reviewing) as AgentTileModel)?.read} onClose={() => setReviewing(undefined)} />}
    </div>
    </Arrangement.Provider>
  );
}
