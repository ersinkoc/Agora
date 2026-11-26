import { Suspense, lazy, useCallback, useRef, type KeyboardEvent, type ReactNode } from 'react';
import { t } from '../i18n';
import { endedWithError, type AgentTileModel } from '../model';
import type { DiffSource } from '../net/diff';
import type { WorkspaceFiles } from '../net/files';
import { folderChoices, folderKey, type ToolPanel } from '../net/paneTools';
import type { TerminalSource } from '../terminal/source';
import { MAX_SIZE, MIN_SIZE, setTerminalSize, terminalSize } from '../terminal/prefs';
import { TerminalSettings } from '../terminal/TerminalSettings';
import { Menu, type MenuItem } from '../layout/Menu';
import { ChangesView } from './ChangesView';
import { EmptyState } from './EmptyState';
import { FileManager } from './FileManager';
import { FilesView } from './FilesView';
import { Icon } from './Icon';
import { StateBadge } from './StateBadge';
import { visible } from '../net/hidden';

// The tool panes of the workspace (ADR-0068, docs/ux/main-window.md, "Panes"): Changes, Files, File manager and Terminal, each one a
// pane of its own that sits beside an agent. Every one shows one agent's folder; the bar on top switches to another folder (another
// agent's, or the same project on another machine) without leaving the pane. Read-only: nothing here writes to a machine.

const TerminalView = lazy(() => import('../terminal/TerminalView').then((m) => ({ default: m.TerminalView })));

const TITLE: Record<ToolPanel, string> = { terminal: 'focus-terminal', changes: 'ide-tab-changes', files: 'console-files', manager: 'layout-panel-manager' };
const ICON = { terminal: 'terminal', changes: 'diff', files: 'files', manager: 'folder' } as const;

// The bar on top of a pane: which folder it shows, a menu to point it at another, and what the pane adds (its own controls).
export function FolderBar({ agent, fleet, panel, onPick, children }: { agent: AgentTileModel; fleet: readonly AgentTileModel[]; panel: ToolPanel; onPick: (agentId: string) => void; children?: ReactNode }) {
  const items = (): MenuItem[] => folderChoices(fleet, panel === 'changes').flatMap((g, gi) => g.folders.map((f, i) => ({
    label: `${visible(g.project)} · ${visible(f.agent.machine)}`, checked: f.key === folderKey(agent), divider: gi > 0 && i === 0,
    hint: f.changed > 0 ? t('changes-files', { count: f.changed }) : undefined, run: () => onPick(f.agent.id),
  })));
  const name = agent.project || agent.workspace;
  return (
    <div className="ide-bar flex-none flex-wrap gap-x-2 gap-y-1 border-b py-1" data-folder-bar>
      <Menu label={t('pane-switch-folder')} items={items} align="start" className="inline-flex min-h-7 min-w-0 max-w-full items-center gap-1.5 rounded-xs px-1.5 text-start text-[0.8125rem] hover:bg-surface-2">
        <Icon name="folder" className="size-3.5 text-muted" />
        <span className="min-w-0 truncate font-medium">{visible(name)}</span>
        <span className="hidden min-w-0 truncate text-xs text-muted sm:inline">{visible(agent.machine)}</span>
        {agent.branch && <span className="hidden min-w-0 truncate font-mono text-xs text-muted md:inline">{visible(agent.branch)}</span>}
        <Icon name="chevronDown" className="size-3 text-muted" />
      </Menu>
      <span className="flex-1" />
      {children}
    </div>
  );
}

// What an unbound tool pane shows: the folders (or terminals) it could show, one tap each. Arrows move between them.
export function ToolChooser({ panel, fleet, onPick, onNewTerminal }: { panel: ToolPanel; fleet: readonly AgentTileModel[]; onPick: (agentId: string) => void; onNewTerminal?: () => void }) {
  const box = useRef<HTMLDivElement>(null);
  const groups = panel === 'terminal'
    ? [{ project: '', folders: fleet.filter((a) => !a.closed && a.terminal !== false).map((a) => ({ key: a.id, agent: a, changed: 0 })) }].filter((g) => g.folders.length > 0)
    : folderChoices(fleet, panel === 'changes');
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const all = [...(box.current?.querySelectorAll<HTMLElement>('button[data-choice]') ?? [])];
    const at = all.indexOf(document.activeElement as HTMLElement);
    const to = e.key === 'ArrowDown' ? at + 1 : e.key === 'ArrowUp' ? at - 1 : e.key === 'Home' ? 0 : e.key === 'End' ? all.length - 1 : undefined;
    if (to === undefined || all.length === 0) return;
    e.preventDefault();
    all[Math.max(0, Math.min(all.length - 1, to))]?.focus();
  };
  return (
    <div ref={box} className="min-h-0 flex-1 overflow-y-auto p-3" onKeyDown={onKey}>
      <div className="mx-auto grid w-full max-w-md gap-3">
        <h2 className="m-0 flex items-center gap-2"><Icon name={ICON[panel]} className="size-4 text-muted" />{t(panel === 'terminal' ? 'pane-choose-terminal' : 'pane-choose-folder', { name: t(TITLE[panel]) })}</h2>
        {groups.length === 0 && <EmptyState icon={ICON[panel]} title={t(panel === 'terminal' ? 'pane-no-terminals-title' : 'pane-no-folders-title')} hint={t(panel === 'terminal' ? 'pane-no-terminals-hint' : 'pane-no-folders-hint')} />}
        {groups.map((g) => (
          <section key={g.project} className="grid gap-0.5" aria-label={g.project ? visible(g.project) : undefined}>
            {g.project && <h3 className="m-0 truncate text-xs font-semibold text-muted">{visible(g.project)}</h3>}
            <ul className="m-0 grid list-none gap-0.5 p-0">
              {g.folders.map((f) => (
                <li key={f.key}>
                  <button type="button" data-choice className="ide-row py-1.5" onClick={() => onPick(f.agent.id)}>
                    <Icon name={panel === 'terminal' ? 'terminal' : 'folder'} className="size-3.5 text-muted" />
                    <span className="min-w-0 flex-1 truncate">{visible(panel === 'terminal' ? f.agent.name : f.agent.workspace)}</span>
                    <span className="truncate text-xs text-muted">{visible(f.agent.machine)}</span>
                    {f.changed > 0 && <span className="kit-tag flex-none">{t('changes-files', { count: f.changed })}</span>}
                  </button>
                </li>
              ))}
            </ul>
          </section>
        ))}
        {panel === 'terminal' && onNewTerminal && <button type="button" className="button justify-self-start" onClick={onNewTerminal}><Icon name="plus" />{t('pane-new-terminal')}</button>}
      </div>
    </div>
  );
}

const Offline = ({ panel }: { panel: ToolPanel }) => <EmptyState icon={ICON[panel]} title={t('pane-offline-title', { name: t(TITLE[panel]) })} hint={t('pane-offline-hint')} />;
const NoFolder = ({ panel }: { panel: ToolPanel }) => <EmptyState icon={ICON[panel]} title={t('pane-no-folder-title')} hint={t('pane-no-folder-hint')} />;

// Changes: the files an agent changed and the diff of the one chosen, side by side or unified, for the agent's folder.
export function ChangesPane({ agent, fleet, loadDiff, filesOf, onPick }: { agent: AgentTileModel; fleet: readonly AgentTileModel[]; loadDiff?: (a: AgentTileModel, path?: string) => Promise<DiffSource>; filesOf?: (a: AgentTileModel) => WorkspaceFiles | undefined; onPick: (id: string) => void }) {
  // The diff of this folder, a stable function per folder: the view reads again when it changes, not on each update of the fleet.
  const latest = useRef({ loadDiff, agent });
  latest.current = { loadDiff, agent };
  const load = useCallback((path?: string): Promise<DiffSource> => latest.current.loadDiff?.(latest.current.agent, path) ?? Promise.resolve(''), [folderKey(agent)]); // eslint-disable-line react-hooks/exhaustive-deps
  if (!agent.workspaceRef) return <NoFolder panel="changes" />;
  if (!loadDiff) return <Offline panel="changes" />;
  const files = filesOf?.(agent);
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <FolderBar agent={agent} fleet={fleet} panel="changes" onPick={onPick} />
      <div className="min-h-0 flex-1 bg-surface-0"><ChangesView key={folderKey(agent)} load={load} read={files?.read} changed={agent.filesChanged} /></div>
    </div>
  );
}

// Files: the agent's folder as a tree, a file with syntax colours and the places it changed, search by name. Read-only.
export function FilesPane({ agent, fleet, loadDiff, filesOf, onPick }: { agent: AgentTileModel; fleet: readonly AgentTileModel[]; loadDiff?: (a: AgentTileModel, path?: string) => Promise<DiffSource>; filesOf?: (a: AgentTileModel) => WorkspaceFiles | undefined; onPick: (id: string) => void }) {
  const files = filesOf?.(agent);
  const latest = useRef({ loadDiff, agent });
  latest.current = { loadDiff, agent };
  const load = useCallback((path?: string): Promise<DiffSource> => latest.current.loadDiff?.(latest.current.agent, path) ?? Promise.resolve(''), [folderKey(agent)]); // eslint-disable-line react-hooks/exhaustive-deps
  if (!agent.workspaceRef) return <NoFolder panel="files" />;
  if (!files) return <Offline panel="files" />;
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <FolderBar agent={agent} fleet={fleet} panel="files" onPick={onPick} />
      <div className="min-h-0 flex-1"><FilesView key={folderKey(agent)} files={files} load={loadDiff && agent.git !== false ? load : undefined} changed={agent.filesChanged} compact /></div>
    </div>
  );
}

// File manager: the folder as a list (name, kind, size), a folder tree, a preview. Read-only for now; the actions that change files are
// shown switched off with the reason.
export function ManagerPane({ agent, fleet, filesOf, onPick, onTerminal }: { agent: AgentTileModel; fleet: readonly AgentTileModel[]; filesOf?: (a: AgentTileModel) => WorkspaceFiles | undefined; onPick: (id: string) => void; onTerminal?: (a: AgentTileModel) => void }) {
  const files = filesOf?.(agent);
  if (!agent.workspaceRef) return <NoFolder panel="manager" />;
  if (!files) return <Offline panel="manager" />;
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <FolderBar agent={agent} fleet={fleet} panel="manager" onPick={onPick} />
      <div className="min-h-0 flex-1"><FileManager key={folderKey(agent)} files={files} rootName={agent.project || agent.workspace}
        onTerminal={onTerminal && agent.terminal !== false && !agent.closed ? () => onTerminal(agent) : undefined}
        terminalWhy={agent.closed ? 'fm-terminal-closed' : agent.terminal === false ? 'fm-terminal-chat' : undefined} /></div>
    </div>
  );
}

// Terminal: one agent's terminal as a tile of its own, with only what belongs to the terminal on top.
export function TerminalPane({ agent, source }: { agent: AgentTileModel; source: TerminalSource }) {
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <div className="ide-bar flex-none flex-wrap gap-x-2 gap-y-1 border-b py-1 text-xs text-muted" title={`${visible(agent.machine)} · ${visible(agent.workspace)}`}>
        <Icon name="machine" className="size-3.5" /><span className="min-w-0 truncate">{visible(agent.machine)}</span>
        <Icon name="chevron" className="size-3 rtl:rotate-180" /><Icon name="folder" className="size-3.5" /><span className="min-w-0 truncate">{visible(agent.project || agent.workspace)}</span>
        {agent.branch && <><Icon name="branch" className="size-3.5" /><span className="min-w-0 truncate font-mono">{visible(agent.branch)}</span></>}
        <span className="flex-1" />
        <StateBadge state={agent.state} failed={endedWithError(agent)} />
        <span className="flex items-center" role="group" aria-label={t('terminal-size')}>
          <button type="button" className="ide-iconbtn w-6 text-xs font-semibold" onClick={() => setTerminalSize(terminalSize() - 1)} disabled={terminalSize() <= MIN_SIZE} aria-label={t('terminal-smaller')} title={t('terminal-smaller')}>A−</button>
          <button type="button" className="ide-iconbtn w-6 text-[0.8125rem] font-semibold" onClick={() => setTerminalSize(terminalSize() + 1)} disabled={terminalSize() >= MAX_SIZE} aria-label={t('terminal-larger')} title={t('terminal-larger')}>A+</button>
        </span>
        <TerminalSettings />
      </div>
      <div className="pane min-h-0 flex-1 p-1.5">
        <Suspense fallback={<div className="terminal-host" aria-busy="true" />}>
          <TerminalView source={source} label={`${t('focus-terminal')} · ${visible(agent.name)}`} />
        </Suspense>
      </div>
    </div>
  );
}
