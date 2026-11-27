// The built-in panels (ADR-0068 rung 3): the registry's facts (panels.ts) with each panel's drawing. A drawing sees only the context
// the window gives its pane and acts only through that context's actions; the window draws the pane bar around it.
import { Fragment, Suspense, lazy, type ReactNode } from 'react';
import { visible } from '../net/hidden';
import { t } from '../i18n';
import { type AgentTileModel, endedWithError } from '../model';
import { asksUnderHeader } from '../net/chatView';
import { AgentAddress } from '../components/AgentAddress';
import { AsksLessChip } from '../components/AllowAlways';
import { RunsInsideChip } from '../components/RunsInside';
import { ApprovalBanners } from '../components/ApprovalBanner';
import { CloseAgentContext, CloseAgentMenu, CloseConfirm } from '../components/CloseAgent';
import { ChatActionsContext } from '../components/chatActions';
import { GiveContext } from '../components/GiveTask';
import { ClosedSummary } from '../components/ClosedSummary';
import { FleetConsole } from '../components/FleetConsole';
import { HelperGrants } from '../components/HelperGrants';
import { Icon } from '../components/Icon';
import { Details, type DetailsTab } from '../components/Inspector';
import { LiveSummary } from '../components/LiveSummary';
import { NoAskBadge } from '../components/NoAskBadge';
import { PromptBox } from '../components/PromptBox';
import { StateBadge } from '../components/StateBadge';
import { ChangesPane, FilesPane, ManagerPane, TerminalPane, ToolChooser } from '../components/ToolPanes';
import { MAX_SIZE, MIN_SIZE, setTerminalSize, terminalSize } from '../terminal/prefs';
import { TerminalSettings } from '../terminal/TerminalSettings';
import { ThemeEditor } from '../themes/ThemeEditor';
import { Menu, type MenuItem } from './Menu';
import { Note } from './Note';
import { BUILTIN_PANELS, PANEL_INFO, TOOL_PANELS, type BuiltinPanel, type PanelActions, type PanelContext, type PanelDef, type PanelRender, type PanelState, type ToolPanel, panelRegistry } from './panels';

const TerminalView = lazy(() => import('../terminal/TerminalView').then((m) => ({ default: m.TerminalView })));

/** The window's actions before they are given to one pane (which adds its own `rebind`). */
export type HostActions = Omit<PanelActions, 'rebind'>;

// Home: the fleet console. It is what an empty window shows (its welcome) and what the Fleet panel is. A tile opens its agent as the tree does.
export function homeView(state: PanelState, actions: HostActions): ReactNode {
  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <FleetConsole fleet={state.fleet} machines={state.machines} approvals={state.approvals} usage={state.usage} link={state.link} now={state.now} onOpen={actions.openFromHome} onStart={actions.start} onResolve={actions.resolve}
        onReview={() => actions.showNeeds()} onRetry={actions.retry} pairing={actions.pairing} guest={state.guest} watching={state.watching} />
    </div>
  );
}

// Needs you, shared tasks, team chat and usage: the side panel's views, as a pane.
const details = (tab: DetailsTab): PanelRender => ({ state, sources, actions }) => (
  <div className="min-h-0 flex-1 overflow-y-auto p-3">
    <Details tab={tab} fleet={state.fleet} now={state.now} pairing={actions.pairing} attention={state.attention} approvals={state.approvals} notice={state.notice} changed={state.changed} boards={state.boards} messages={state.messages} usage={state.usage} projects={state.projects} onOpen={actions.openAgent}
      onResolve={actions.resolve} stale={state.link !== 'live'} onReview={sources.loadDiff ? actions.review : undefined} />
  </div>
);

// One agent as a pane draws it: where it runs and its tools, a request waiting for you, then its terminal, chat or summary (the `agent`
// panel, and `terminal` or `conversation` once the agent has ended). Moved here from the console unchanged (ADR-0068 rung 3).
function agentView(ctx: PanelContext, a: AgentTileModel): ReactNode {
  const { fleet, approvals, now, forgetError, resumeError, resuming } = ctx.state;
  const { loadDiff, loadHistory, filesOf, chatOf, helperGrants } = ctx.sources;
  const { answers: banners, resolve: onResolve, openTool, openAgent: onOpen, history, review, browse, showChanges, dismiss, forget, resume, start: onStart, sendPrompt, chooseOption, revokeGrant } = ctx.actions;
  const { terminal } = ctx;

  // What a chat agent waits on, pinned above its composer (a terminal agent's requests are a bar under its header).
  const pinnedAsks = (agentId: string) => {
    const mine = approvals.filter((x) => x.agentId === agentId);
    const agent = fleet.find((f) => f.id === agentId);
    return mine.length ? <div className="grid gap-2"><ApprovalBanners approvals={mine} agent={agent} guard={banners} onResolve={onResolve} pinned /></div> : undefined;
  };
  // "Open beside" in an agent's header: its terminal (unless it is this one), changes, files and file manager, each as a pane next to it.
  const beside = (): MenuItem[] => [
    ...TOOL_PANELS.filter((p) => p !== 'terminal' && (p === 'changes' ? !!loadDiff : !!filesOf)).map((p) => ({ label: t(PANEL_INFO.label(p) as string), run: () => openTool(p, a.id) })),
  ];
  return (
    <div className="agent-body flex min-h-0 min-w-0 flex-1 flex-col">
      {/* Redesign PR 7: in a narrow pane the machine and the branch leave this row (they stay in its tooltip) so the folder is read whole,
          not three single letters, and the tools wrap inside the pane instead of running into the next one (app.css `.agent-body`). */}
      <div className="flex min-h-9 flex-none flex-wrap items-center gap-x-3 gap-y-1 border-b border-line bg-surface-1 px-3 py-1">
        <div className="flex min-w-0 flex-1 basis-56 items-center gap-1.5 text-xs text-muted" title={`${visible(a.machine)} · ${visible(a.workspace)}${a.branch ? ` · ${visible(a.branch)}` : ''}`}>
          <span className="flex min-w-0 items-center gap-1.5" data-crumb="machine"><Icon name="machine" className="size-3.5 flex-none" /><span className="truncate">{visible(a.machine)}</span>
          <Icon name="chevron" className="size-3 flex-none rtl:rotate-180" /></span><Icon name="folder" className="size-3.5 flex-none" /><span className="min-w-[4ch] truncate">{visible(a.project || a.workspace)}</span>
          {a.branch && <span className="flex min-w-0 items-center gap-1.5" data-crumb="branch"><Icon name="branch" className="ms-1 size-3.5 flex-none" /><span className="truncate font-mono">{visible(a.branch)}</span></span>}
          {a.shared && <span className="ide-chip ms-1 flex-none" title={t('workspace-shared')}>{t('ide-shared')}</span>}
          <RunsInsideChip agent={a} />
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
          {loadHistory && a.git !== false && <button type="button" className="ide-iconbtn" onClick={() => history(a.id)} aria-label={t('console-history')} title={t('console-history')}><Icon name="history" /></button>}
          {filesOf && <button type="button" className="ide-iconbtn" onClick={() => openTool('files', a.id)} aria-label={t('console-files')} title={t('console-files')}><Icon name="files" /></button>}
          {(filesOf || loadDiff) && !a.closed && <Menu label={t('pane-open-beside')} items={beside}><Icon name="splitRight" /></Menu>}
          <button type="button" className="ide-iconbtn" onClick={() => onOpen(a.id)} aria-label={t('console-maximize')} title={t('console-maximize')}><Icon name="maximize" /></button>
        </div>
      </div>
      <CloseConfirm agent={a} where="pane" />
      {!asksUnderHeader(a) ? null : <ApprovalBanners approvals={approvals.filter((x) => x.agentId === a.id)} agent={a} guard={banners} onResolve={onResolve} />}
      <AgentAddress agent={a} />
      {helperGrants && revokeGrant && !a.closed && fleet.some((c) => c.parentId === a.id) &&
        <HelperGrants view={a.id} list={() => helperGrants(a)} revoke={(g) => revokeGrant(a, g)} refresh={fleet.filter((c) => c.parentId === a.id && !c.closed).length} />}
      {a.closed ? <ClosedSummary agent={a} now={now} source={a.terminal === false ? chatOf?.(a) : undefined}
        onReview={loadDiff ? () => review(a.id) : undefined} onHistory={loadHistory && a.git !== false ? () => history(a.id) : undefined} onFiles={filesOf ? () => browse(a.id) : undefined}
        onStart={onStart ? () => onStart() : undefined} onDismiss={() => dismiss([a.id])}
        onForget={forget ? () => forget([a.id]) : undefined} forgetError={forgetError}
        onResume={resume && a.interrupted ? (action) => resume(a, action) : undefined} resuming={resuming === a.id}
        resumeError={resumeError?.id === a.id ? resumeError.code : undefined} /> : <>
        {a.terminal !== false && <LiveSummary agent={a} now={now} onChanges={() => showChanges()} />}
        <div className="pane min-h-0 flex-1 p-1.5">
        {a.terminal === false ? <PromptBox key={a.id} agent={a} send={sendPrompt} source={chatOf?.(a)} choose={chooseOption} onReview={loadDiff ? () => review(a.id) : undefined} approvals={pinnedAsks(a.id)} /> : (
          terminal && <Suspense fallback={<div className="terminal-host" aria-busy="true" />}>
            <TerminalView source={terminal()} label={`${t('focus-terminal')} · ${visible(a.name)}`} />
          </Suspense>
        )}
      </div></>}
    </div>
  );
}

// A tool pane (Terminal, Changes, Files, File manager) for the agent the pane shows, or the tool's own chooser while it shows none.
// Keyed by agent and tool: a pane that changes agent mounts a new view; one that keeps it keeps its terminal.
function tool(panel: ToolPanel): PanelRender {
  return (ctx) => {
    const { agent: a, state, sources, actions } = ctx;
    if (!a) return <ToolChooser panel={panel} fleet={state.fleet} onPick={actions.rebind} onNewTerminal={actions.newTerminal ? () => actions.newTerminal?.() : undefined} />;
    if (panel === 'terminal') {
      if (a.terminal === false && !a.closed) return <Note>{t('layout-no-terminal')}</Note>;
      // A terminal that ended shows what the agent's own view shows (the summary), not a dead screen.
      if (a.closed) return <Fragment key={a.id}>{agentView(ctx, a)}</Fragment>;
      return ctx.terminal ? <Fragment key={`${a.id}:${panel}`}><TerminalPane agent={a} source={ctx.terminal()} /></Fragment> : undefined;
    }
    let view: ReactNode;
    if (panel === 'changes') view = <ChangesPane agent={a} fleet={state.fleet} loadDiff={sources.loadDiff} filesOf={sources.filesOf} onPick={actions.rebind} />;
    else if (panel === 'files') view = <FilesPane agent={a} fleet={state.fleet} loadDiff={sources.loadDiff} filesOf={sources.filesOf} onPick={actions.rebind} />;
    else view = <ManagerPane agent={a} fleet={state.fleet} filesOf={sources.filesOf} onPick={actions.rebind} onTerminal={(x) => actions.openTool('terminal', x.id)} />;
    return <Fragment key={`${a.id}:${panel}`}>{view}</Fragment>;
  };
}

// Each built-in panel's drawing. Every panel the registry lists has one (the empty pane's chooser is the window's own).
const RENDERS: Record<Exclude<BuiltinPanel, 'empty'>, PanelRender> = {
  fleet: ({ state, actions }) => homeView(state, actions),
  approvals: details('needs'),
  tasks: details('board'),
  messages: details('messages'),
  usage: details('usage'),
  theme: () => <div className="min-h-0 flex-1 overflow-y-auto"><ThemeEditor /></div>,
  terminal: tool('terminal'),
  changes: tool('changes'),
  files: tool('files'),
  manager: tool('manager'),
  // The agent's own view: terminal, chat or its summary. Keyed by the agent, so a pane that follows attention mounts a new view when
  // it moves on, and one that keeps its agent keeps its terminal.
  agent: (ctx) => (ctx.agent ? <Fragment key={ctx.agent.id}>{agentView(ctx, ctx.agent)}</Fragment> : undefined),
  conversation: (ctx) => {
    const a = ctx.agent;
    if (!a) return undefined;
    if (a.terminal !== false && !a.closed) return <Note>{t('layout-no-conversation')}</Note>;
    return <Fragment key={a.id}>{agentView(ctx, a)}</Fragment>;
  },
};

// The window's command paths that a built-in panel's components read from React context (closing an agent, an agent's settings, giving
// a task), provided again from the panel's own actions, so what a panel can send is exactly what its context gives it. Providers draw nothing.
export function withPaths(actions: PanelActions, view: ReactNode): ReactNode {
  return (
    <CloseAgentContext.Provider value={actions.closeAgent}>
      <ChatActionsContext.Provider value={actions.chat}>
        <GiveContext.Provider value={actions.give}>{view}</GiveContext.Provider>
      </ChatActionsContext.Provider>
    </CloseAgentContext.Provider>
  );
}

/** The window's panels. */
export const BUILTIN_REGISTRY = panelRegistry<PanelDef>(BUILTIN_PANELS.map((p): PanelDef => {
  if (p.kind === 'chooser') return p;
  const render = RENDERS[p.id];
  return { ...p, render: (ctx) => { const view = render(ctx); return view === undefined ? undefined : withPaths(ctx.actions, view); } };
}));
