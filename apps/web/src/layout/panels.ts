// The panel registry (ADR-0068 decision 3, rung 3; docs/ux/layout.md, "Panels"). Every panel a pane can show is registered here with
// a stable id and a small contract: what it may see (a read-only context) and what it may do (a typed set of actions the window gives
// it, the same paths as before). A layout names panels by id; an id nobody registered is drawn as the "unknown panel" placeholder.
//
// This half is plain data and types, so the layout model and the stream plan read it without drawing anything; the built-in panels'
// render functions are attached in `builtinPanels.tsx`.
import type { ReactNode } from 'react';
import type { AgentTileModel, ResumeAction } from '../model';
import type { ActionResult } from '../net/refusal';
import type { ChatSource } from '../net/chat';
import type { CommitItem } from '../net/history';
import type { DiffSource } from '../net/diff';
import type { WorkspaceFiles } from '../net/files';
import type { UsageSummary } from '../net/usage';
import type { ApprovalModel, BoardModel, MachineModel, MessageModel, ProjectCost } from '../net/view';
import type { TerminalSource } from '../terminal/source';
import type { ChatActions, PromptSend } from '../components/chatActions';
import type { GiveApi } from '../components/GiveTask';
import type { HelperGrant } from '../components/HelperGrants';
import type { PairingUi } from '../components/Pairing';
import type { StartChoice } from '../components/StartApproval';
import type { AnswerGuard, AnswerResult } from '../components/useAnswerGuard';
import type { Bind } from './tree';

/** Where a panel is offered and what it is about:
 *  - `global`: about no one agent (Needs you, Usage); listed in Show… and the empty pane's chooser.
 *  - `tool`: one agent's folder or terminal (Changes, Files); listed too, and when bound to nothing it draws its own chooser.
 *  - `agent`: one agent's own view; reached by showing an agent, never listed; bound to nothing, the window draws the chooser.
 *  - `chooser`: the empty pane; the window draws it from this registry. */
export type PanelKind = 'global' | 'tool' | 'agent' | 'chooser';

// Host-set facts: `kind` (above all `chooser`), `streams`, `lone` and `label` are set by the window, for built-ins here. A plugin bridge
// (ADR-0114) sets them itself and never takes them from a plugin's manifest: `lone` would drop the host's pane bar and the plugin
// marker, `streams` would hand out a terminal, and a plugin's `label` (a Fluent key) could pose as "Needs you".
export interface PanelInfo {
  readonly id: string;
  /** The Fluent key of the panel's name (UI vocabulary). */
  readonly label: string;
  readonly kind: PanelKind;
  /** What a pane of it is bound to: nothing, or an agent (by key, following attention or following the selection). */
  readonly binds: 'nothing' | 'agent';
  /** Draws a live terminal. Only a pane on screen draws, so only it subscribes (ADR-0068 §7); only such a panel is given one. */
  readonly streams: boolean;
  /** Alone in the window it is drawn without a pane bar (Home, the empty pane). Every other panel always has the host's bar. */
  readonly lone?: boolean;
}

// The built-in panels, in the order the window lists them: Show… lists the global ones then the tools, the empty pane's chooser the
// tools then the global ones. `docs/ux/layout.md` has the same table.
export const BUILTIN_PANELS = [
  { id: 'fleet', label: 'fleet-panel', kind: 'global', binds: 'nothing', streams: false, lone: true },
  { id: 'approvals', label: 'attention-title', kind: 'global', binds: 'nothing', streams: false },
  { id: 'tasks', label: 'board-title', kind: 'global', binds: 'nothing', streams: false },
  { id: 'messages', label: 'ide-tab-messages', kind: 'global', binds: 'nothing', streams: false },
  { id: 'usage', label: 'ide-tab-cost', kind: 'global', binds: 'nothing', streams: false },
  { id: 'theme', label: 'theme-editor-title', kind: 'global', binds: 'nothing', streams: false },
  { id: 'terminal', label: 'focus-terminal', kind: 'tool', binds: 'agent', streams: true },
  { id: 'changes', label: 'ide-tab-changes', kind: 'tool', binds: 'agent', streams: false },
  { id: 'files', label: 'console-files', kind: 'tool', binds: 'agent', streams: false },
  { id: 'manager', label: 'layout-panel-manager', kind: 'tool', binds: 'agent', streams: false },
  { id: 'agent', label: 'layout-panel-agent', kind: 'agent', binds: 'agent', streams: true },
  { id: 'conversation', label: 'chat-log', kind: 'agent', binds: 'agent', streams: false },
  { id: 'empty', label: 'layout-panel-empty', kind: 'chooser', binds: 'nothing', streams: false, lone: true },
] as const satisfies readonly PanelInfo[];

export type BuiltinPanel = (typeof BUILTIN_PANELS)[number]['id'];
export type ToolPanel = Extract<(typeof BUILTIN_PANELS)[number], { kind: 'tool' }>['id'];
/** The tool panes (Terminal, Changes, Files, File manager), in the order the window lists them. */
export const TOOL_PANELS: readonly ToolPanel[] = BUILTIN_PANELS.flatMap((p) => (p.kind === 'tool' ? [p.id] : []));
export const isToolPanel = (id: string): id is ToolPanel => (TOOL_PANELS as readonly string[]).includes(id);

/** What every panel may see: read-only slices of the window's state. A panel never gets the connection, the store or the signing key. */
export interface PanelState {
  readonly fleet: readonly AgentTileModel[];
  readonly machines: readonly MachineModel[];
  readonly approvals: readonly ApprovalModel[];
  /** The agents that need you (the window's order), and the folders with uncommitted changes (the chosen project's). */
  readonly attention: readonly AgentTileModel[];
  readonly changed: readonly AgentTileModel[];
  readonly messages: readonly MessageModel[];
  readonly boards: readonly BoardModel[];
  readonly usage?: UsageSummary;
  readonly projects: readonly ProjectCost[];
  readonly now: number;
  readonly link: 'connecting' | 'live' | 'offline';
  readonly notice?: string;
  readonly guest?: boolean;
  readonly watching?: boolean;
  /** How the last Remove, Resume or Start again went, for the agent it was about. */
  readonly forgetError?: string;
  readonly resumeError?: { readonly id: string; readonly code: string };
  readonly resuming?: string;
}

/** Reading what a node holds for an agent, each undefined when this window cannot (no hub, an older one). Reads only. */
export interface PanelSources {
  readonly chatOf?: (agent: AgentTileModel) => ChatSource | undefined;
  readonly filesOf?: (agent: AgentTileModel) => WorkspaceFiles | undefined;
  readonly loadDiff?: (agent: AgentTileModel, path?: string) => Promise<DiffSource>;
  readonly loadHistory?: (agent: AgentTileModel) => Promise<CommitItem[]>;
  readonly helperGrants?: (agent: AgentTileModel) => Promise<HelperGrant[]>;
}

/** What a panel may do: the window's own paths, each undefined where the window cannot do it. Every command a panel sends goes
 *  through these; anything signed (an answer, a prompt) is signed by the window behind the function, never by the panel. */
export interface PanelActions {
  // This pane.
  /** Point this pane's panel at another agent (a tool's folder chooser). */
  readonly rebind: (agentId: string) => void;
  // The window.
  readonly openAgent: (agentId: string) => void;
  /** Open an agent from Home: in the pane that follows the selection, else as the tree does. */
  readonly openFromHome: (agentId: string) => void;
  readonly openTool: (panel: ToolPanel, agentId?: string) => void;
  readonly review: (agentId: string) => void;
  readonly history: (agentId: string) => void;
  readonly browse: (agentId: string) => void;
  /** Bring the side panel's Needs you, or its Changes, into view. */
  readonly showNeeds: () => void;
  readonly showChanges: () => void;
  readonly dismiss: (agentIds: string[]) => void;
  readonly forget?: (agentIds: string[]) => void;
  readonly resume?: (agent: AgentTileModel, action: ResumeAction) => void;
  readonly start?: (machine?: string) => void;
  readonly newTerminal?: () => void;
  readonly retry?: () => void;
  readonly resolve?: (a: ApprovalModel, approved: boolean, choice?: StartChoice, always?: boolean) => AnswerResult;
  readonly sendPrompt?: PromptSend;
  readonly chooseOption?: (agentKey: string, kind: 'mode' | 'model', id: string) => Promise<ActionResult>;
  readonly revokeGrant?: (agent: AgentTileModel, id: string) => Promise<void>;
  /** The window's one guard on answering requests (T44): every card in a pane answers through it. */
  readonly answers: AnswerGuard;
  /** Machines asking to join, with their answers (ADR-0065), for an owner's session. */
  readonly pairing?: PairingUi;
  /** Close an agent (`AgentStop`); undefined where this window cannot. */
  readonly closeAgent?: (agent: AgentTileModel) => Promise<string | undefined>;
  /** An agent's own settings and ending its turn, from its composer. */
  readonly chat: ChatActions;
  /** Giving a shared task to an agent (the board). */
  readonly give?: GiveApi;
}

/** Everything one built-in pane's panel is given, frozen at the top level (TypeScript `readonly` keeps built-ins from changing what
 *  is inside). Not for plugins as it is: it carries requests and their answers, pairing, prompts, starting and resuming agents and
 *  terminals that can type. A plugin panel (ADR-0114) only ever gets a separate projection the bridge builds from granted capabilities. */
export interface PanelContext {
  /** The pane's id, and the panel id it shows. */
  readonly pane: string;
  readonly panel: string;
  /** What the layout binds the pane to, and the agent that resolves to now (undefined when it resolves to none). */
  readonly bind?: Readonly<Bind>;
  readonly agent?: AgentTileModel;
  readonly state: PanelState;
  readonly sources: PanelSources;
  readonly actions: PanelActions;
  /** The bound agent's terminal, only for a panel that `streams`. */
  readonly terminal?: () => TerminalSource;
}

/** A panel's drawing: undefined when this window cannot draw it (it shows the "unknown panel" placeholder). Called as a function
 *  in the pane's body, under the pane bar the window draws; it never draws the bar. */
export type PanelRender = (ctx: PanelContext) => ReactNode | undefined;
/** A registered panel: its facts and its drawing. The chooser (the empty pane) is drawn by the window from the registry itself. */
export type PanelDef =
  | (PanelInfo & { readonly kind: Exclude<PanelKind, 'chooser'>; readonly render: PanelRender })
  | (PanelInfo & { readonly kind: 'chooser'; readonly render?: undefined });

export interface PanelRegistry<T extends PanelInfo = PanelDef> {
  readonly ids: readonly string[];
  get(id: string): T | undefined;
  /** The panels Show… and the empty pane's chooser list, of one kind, in registration order. */
  offered(kind: 'global' | 'tool'): readonly T[];
  /** The Fluent key of a panel's name; undefined for an id nobody registered. */
  label(id: string): string | undefined;
  streams(id: string): boolean;
}

/** A registry from a list of panels. An id is registered once: a later panel with an id already taken is left out, so nothing
 *  registered after the built-ins can stand in for one of them. */
export function panelRegistry<T extends PanelInfo>(defs: readonly T[]): PanelRegistry<T> {
  const byId = new Map<string, T>();
  for (const d of defs) if (!byId.has(d.id)) byId.set(d.id, Object.freeze({ ...d }));
  const ids = Object.freeze([...byId.keys()]);
  const list = [...byId.values()];
  const offered = { global: Object.freeze(list.filter((d) => d.kind === 'global')), tool: Object.freeze(list.filter((d) => d.kind === 'tool')) };
  return Object.freeze({
    ids,
    get: (id: string) => byId.get(id),
    offered: (kind: 'global' | 'tool') => offered[kind],
    label: (id: string) => byId.get(id)?.label,
    streams: (id: string) => byId.get(id)?.streams ?? false,
  });
}

/** The built-in panels without their drawing: what the layout model and the stream plan read. */
export const PANEL_INFO: PanelRegistry<PanelInfo> = panelRegistry<PanelInfo>(BUILTIN_PANELS);
