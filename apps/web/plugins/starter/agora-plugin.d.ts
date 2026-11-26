// Types for writing an Agora plugin: the `agora` object the window's loader gives a plugin's frame, what each capability answers, and
// the messages the loader exchanges with the window (protocol version 1, rung 1 of ADR-0114; docs/ux/plugins.md).
//
// In your entry script (plain JavaScript is fine), put these two lines at the top and your editor checks your calls:
//
//   // @ts-check
//   /// <reference path="agora-plugin.d.ts" />
//
// This file is never packed into the package (as README.md is not), so it needs no hash in the manifest. `agora plugin new` puts it in
// every new plugin folder. It is checked against the window's own types (apps/web/plugins/tsconfig.json, `npm run typecheck`), so it
// says exactly what the window does.

declare namespace AgoraPlugin {
  /** What a plugin may ask to see. Each is read-only, scoped to what the person can already see, and granted by the person one by one. */
  type Capability = 'agents.read' | 'usage.read' | 'tasks.read' | 'changes.read' | 'chat.read' | 'theme.read' | 'pane.context';

  /** Why a read or a watch was refused: the `message` of the Error `agora.read` rejects with and `agora.watch` hands to `onError`.
   *  - `not-granted`: the person did not allow this capability (or has not looked at a new version's request yet);
   *  - `unknown-capability`: no such capability;
   *  - `unavailable`: nothing to answer (`changes.read` and `chat.read` in a pane bound to no agent, or a read that failed);
   *  - `too-many`: more than 16 watches at once, or a watch id used twice. */
  type ReadError = 'not-granted' | 'unknown-capability' | 'unavailable' | 'too-many';

  type AgentState = 'working' | 'needs-you' | 'finished' | 'idle' | 'unknown';

  /** An agent (`agents.read`): who it is, what it is doing, where. Never its terminal, its requests or whether it asks. */
  interface Agent {
    id: string;
    name: string;
    kind: string;
    state: AgentState;
    machine: string;
    project: string;
    closed: boolean;
  }

  /** An agent's usage (`usage.read`). Never money. */
  interface AgentUsage {
    id: string;
    name: string;
    tokens: number;
    contextPercent?: number;
    activeMs?: number;
  }

  /** A shared task (`tasks.read`). Never held files or leases. */
  interface Task {
    id: string;
    title: string;
    status: 'open' | 'claimed' | 'done';
    owner: string;
  }

  /** A project's shared tasks (`tasks.read`). */
  interface Board {
    project: string;
    tasks: Task[];
  }

  /** The pane the plugin is drawn in (`pane.context`): its kind, what it follows, and the agent it is bound to. */
  interface Pane {
    id: string;
    kind: 'global' | 'tool';
    follows?: 'attention' | 'selection';
    agent?: { id: string; name: string };
  }

  /** The window's colours as it draws them now (`theme.read`): `tokens` maps a design token (`surface-0`, `text`, `accent`, …) to a CSS
   *  colour. */
  interface Theme {
    scheme: 'dark' | 'light';
    tokens: { [token: string]: string };
  }

  /** One line of a changed file's diff (`changes.read`). */
  interface DiffLine {
    kind: 'add' | 'del' | 'ctx' | 'hunk';
    text: string;
    /** Its number in the old file. */
    old?: number;
    /** Its number in the new file. */
    new?: number;
  }

  /** A changed file of the bound agent (`changes.read`). A file the machine keeps secret has its counts only (`secret`, no lines). */
  interface ChangedFile {
    path: string;
    oldPath?: string;
    status: 'added' | 'deleted' | 'modified' | 'renamed';
    binary: boolean;
    added: number;
    removed: number;
    secret?: boolean;
    /** Some of its lines were left out (the diff was cut, or the 5,000-line limit was reached). */
    partial?: boolean;
    lines: DiffLine[];
  }

  /** An entry of the bound agent's conversation as the window shows it, already redacted (`chat.read`). Never a request for approval. */
  interface ChatEntry {
    id: number;
    atMs: number;
    role: 'agent' | 'user';
    kind: 'text' | 'tool' | 'plan' | 'diff' | 'notice' | 'thought';
    text: string;
    path?: string;
    /** A notice says what happened as a code and its parameters, not as text. */
    code?: string;
    params?: { [name: string]: string };
    callKind?: 'read' | 'edit' | 'delete' | 'move' | 'search' | 'run' | 'think' | 'fetch' | 'other';
    status?: 'pending' | 'in_progress' | 'completed' | 'failed';
    output?: string;
    steps?: { text: string; status?: 'pending' | 'in_progress' | 'completed' }[];
  }

  /** What each capability answers. Lists and texts are bounded: 500 agents, 500 tasks, 200 entries, 5,000 diff lines, 8,192
   *  characters a text. */
  interface Answers {
    'agents.read': { agents: Agent[] };
    'usage.read': { agents: AgentUsage[]; totalTokens: number };
    'tasks.read': { boards: Board[] };
    'changes.read': { agent: string; files: ChangedFile[] };
    'chat.read': { agent: string; entries: ChatEntry[] };
    'theme.read': Theme;
    'pane.context': { pane: Pane };
  }

  /** The global `agora`, ready before the entry script runs. */
  interface Api {
    /** The message set's version. */
    readonly version: 1;
    /** This plugin, as its manifest names it. */
    readonly plugin: { readonly id: string; readonly name: string; readonly version: string };
    /** The id of the panel this frame draws (one of the manifest's `panels`). */
    readonly panel: string;
    /** The window's colour scheme when the frame started (watch `theme.read` to follow changes). */
    readonly scheme: 'dark' | 'light';
    /** The capabilities the person allowed this plugin. */
    readonly granted: readonly Capability[];
    /** One answer. Rejects with an Error whose message is a `ReadError`. */
    read<C extends Capability>(cap: C): Promise<Answers[C]>;
    /** An answer now and one on each change, until the returned function is called. `onError` gets an Error whose message is a
     *  `ReadError`; after it, the watch has ended. At most 16 at once. */
    watch<C extends Capability>(cap: C, onData: (data: Answers[C]) => void, onError?: (error: Error) => void): () => void;
    /** A `blob:` address of one of the plugin's own images or fonts (by its file name), or undefined. */
    file(name: string): string | undefined;
  }

  // The messages under `agora` (a plugin does not send these itself; the loader does). Every one is `{ agora: 1, type, … }`. The window
  // takes at most 20 a second (a burst of 40) and stops the frame at a message of any other shape.

  /** From the frame to the window. */
  type FrameMessage =
    | { agora: 1; type: 'boot' }
    | { agora: 1; type: 'read'; id: number; cap: string }
    | { agora: 1; type: 'watch'; id: number; cap: string }
    | { agora: 1; type: 'unwatch'; id: number };

  /** From the window to the frame. */
  type HostMessage =
    | {
        agora: 1;
        type: 'load';
        plugin: { id: string; name: string; version: string };
        panel: string;
        granted: Capability[];
        entry: string;
        scheme: 'dark' | 'light';
        lang: string;
        dir: 'ltr' | 'rtl';
        files: { name: string; type: string; bytes: Uint8Array }[];
      }
    | { agora: 1; type: 'result'; id: number; ok: true; data: unknown }
    | { agora: 1; type: 'result'; id: number; ok: false; error: ReadError }
    | { agora: 1; type: 'update'; id: number; data: unknown };
}

declare const agora: AgoraPlugin.Api;
