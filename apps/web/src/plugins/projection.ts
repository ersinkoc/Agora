// What a plugin is shown (ADR-0114 decision 3; docs/ux/plugins.md, "Capabilities"). The bridge builds a view of its own from the window's
// state for each plugin pane (`viewOf`), with only the facts the rung-1 capabilities can answer, and each capability's answer is a plain
// projection of that view (`project`). A plugin pane never receives the built-in panels' context, state or actions (layout.md, "Not for
// plugins as it is"): no requests or answers to them, no terminal, no preview of one, no prompts, no starting or stopping, nothing signed.
// Every answer is plain JSON data (no functions), with its lists and texts bounded.
import type { AgentTileModel, UiState } from '../model';
import type { ChatLine, ChatSource } from '../net/chat';
import { readDiff, type DiffSource } from '../net/diff';
import type { BoardModel } from '../net/view';
import { activeMs } from '../net/usageView';
import { contextPercent } from '../components/sessionChoice';
import type { Capability } from './capabilities';

export const MAX_AGENTS = 500;
export const MAX_TASKS = 500;
export const MAX_ENTRIES = 200;
export const MAX_DIFF_LINES = 5000;
export const MAX_TEXT = 8192;

const cut = (s: string | undefined, max = MAX_TEXT) => (s === undefined ? undefined : s.length > max ? `${s.slice(0, max)}…` : s);

/** An agent as a plugin may know it (`agents.read`): who it is, what it is doing, where. */
export interface AgentFacts { readonly id: string; readonly name: string; readonly kind: string; readonly state: UiState; readonly machine: string; readonly project: string; readonly closed: boolean }
export interface UsageFacts { readonly id: string; readonly name: string; readonly tokens: number; readonly contextPercent?: number; readonly activeMs?: number }
export interface TaskFacts { readonly id: string; readonly title: string; readonly status: 'open' | 'claimed' | 'done'; readonly owner: string }

/** The bridge's own view for one plugin pane. Built from the window's state by `viewOf`; nothing else reaches a plugin's frame. */
export interface PluginView {
  readonly pane: { readonly id: string; readonly kind: 'global' | 'tool'; readonly follows?: 'attention' | 'selection'; readonly agent?: { readonly id: string; readonly name: string } };
  readonly agents: readonly AgentFacts[];
  readonly usage: readonly UsageFacts[];
  readonly boards: readonly { readonly project: string; readonly tasks: readonly TaskFacts[] }[];
  /** The bound agent's conversation as the window shows it (already redacted, ADR-0089), and its changes; absent when the pane is
   *  bound to no agent or this window cannot read them. Reads only. */
  readonly chat?: ChatSource;
  readonly diff?: () => Promise<DiffSource>;
  /** What the bound agent's changes are now, so a watch reads them again when they move. */
  readonly changesKey?: string;
}

const facts = (a: AgentTileModel): AgentFacts => Object.freeze({
  id: a.id, name: a.name, kind: a.kind || '', state: a.state, machine: a.machine, project: a.project || a.workspace || '', closed: !!a.closed,
});

/** The inputs `viewOf` reads: the slices of the window's state the rung-1 capabilities answer from, and the bound agent's two reads. */
export interface ViewInputs {
  readonly pane: string;
  readonly kind: 'global' | 'tool';
  readonly follows?: 'attention' | 'selection';
  readonly agent?: AgentTileModel;
  readonly fleet: readonly AgentTileModel[];
  readonly boards: readonly BoardModel[];
  readonly now: number;
  readonly chatOf?: (a: AgentTileModel) => ChatSource | undefined;
  readonly loadDiff?: (a: AgentTileModel, path?: string) => Promise<DiffSource>;
}

export function viewOf(i: ViewInputs): PluginView {
  const a = i.agent;
  const fleet = i.fleet.slice(0, MAX_AGENTS);
  let tasks = 0;
  const chat = a ? i.chatOf?.(a) : undefined;
  const load = a ? i.loadDiff : undefined;
  return Object.freeze({
    pane: Object.freeze({ id: i.pane, kind: i.kind, ...(i.follows ? { follows: i.follows } : {}), ...(a ? { agent: Object.freeze({ id: a.id, name: a.name }) } : {}) }),
    agents: Object.freeze(fleet.map(facts)),
    usage: Object.freeze(fleet.map((x): UsageFacts => {
      const pct = contextPercent(x.session);
      const active = activeMs(x, i.now);
      return Object.freeze({ id: x.id, name: x.name, tokens: x.tokens ?? 0, ...(pct !== undefined ? { contextPercent: pct } : {}), ...(active !== undefined ? { activeMs: active } : {}) });
    })),
    boards: Object.freeze(i.boards.map((b) => Object.freeze({
      project: b.project,
      tasks: Object.freeze(b.tasks.filter(() => tasks++ < MAX_TASKS).map((t): TaskFacts => Object.freeze({ id: t.id, title: cut(t.title, 500) as string, status: t.status === 'review' ? 'claimed' : t.status, owner: t.owner }))),
    }))),
    ...(chat ? { chat: { connect: chat.connect.bind(chat) } } : {}),
    ...(a && load ? { diff: () => load(a), changesKey: `${a.id}:${a.filesChanged ?? 0}:${a.state}` } : {}),
  });
}

/** The theme's colours as the window draws them now, so a plugin can match them (`theme.read`). */
export interface ThemeFacts { readonly scheme: 'dark' | 'light'; readonly tokens: Readonly<Record<string, string>> }

/** What each capability answers. The plugin author's types (`apps/web/plugins/starter/agora-plugin.d.ts`, `AgoraPlugin.Answers`) are
 *  checked against this (`apps/web/plugins/kit.check.ts`), so they cannot drift from what the bridge sends. */
export interface Answers {
  'agents.read': { agents: PluginView['agents'] };
  'usage.read': { agents: PluginView['usage']; totalTokens: number };
  'tasks.read': { boards: PluginView['boards'] };
  'changes.read': ReturnType<typeof projectChanges>;
  'chat.read': ReturnType<typeof projectChat>;
  'theme.read': ThemeFacts;
  'pane.context': { pane: PluginView['pane'] };
}
const answer = <C extends Capability>(_cap: C, data: Answers[C]): Answers[C] => data;

/** The answer to a capability that is answered from the view at once; undefined for the two that are read from the node (`changes.read`,
 *  `chat.read`) and for one with nothing to answer (`unavailable`). */
export function project(cap: Capability, v: PluginView, theme: () => ThemeFacts): unknown {
  switch (cap) {
    case 'agents.read': return answer(cap, { agents: v.agents });
    case 'usage.read': return answer(cap, { agents: v.usage, totalTokens: v.usage.reduce((n, a) => n + a.tokens, 0) });
    case 'tasks.read': return answer(cap, { boards: v.boards });
    case 'pane.context': return answer(cap, { pane: v.pane });
    case 'theme.read': return answer(cap, theme());
    case 'changes.read':
    case 'chat.read':
      return undefined;
  }
}

/** The bound agent's changes, as file lists and diffs (`changes.read`). A file the node keeps secret by its name has counts only. */
export function projectChanges(agentId: string, src: DiffSource) {
  let lines = 0;
  const files = readDiff(src).map((f) => {
    const take = f.secret || f.binary ? [] : f.lines.slice(0, Math.max(0, MAX_DIFF_LINES - lines));
    lines += take.length;
    return {
      path: f.path, ...(f.oldPath ? { oldPath: f.oldPath } : {}), status: f.status, binary: f.binary, added: f.added, removed: f.removed,
      ...(f.secret ? { secret: true } : {}), ...(f.partial || take.length < f.lines.length ? { partial: true } : {}),
      lines: take.map((l) => ({ kind: l.kind, text: cut(l.text) as string, ...(l.oldNo !== undefined ? { old: l.oldNo } : {}), ...(l.newNo !== undefined ? { new: l.newNo } : {}) })),
    };
  });
  return { agent: agentId, files };
}

/** The bound agent's conversation as the window shows it, already redacted (`chat.read`): the newest entries, without requests for
 *  approval (a permission entry, an approval's id) and without anything the window would act on. */
export function projectChat(agentId: string, lines: readonly ChatLine[]) {
  const entries = lines.filter((l): l is ChatLine & { kind: Exclude<ChatLine['kind'], 'permission'> } => l.kind !== 'permission').slice(-MAX_ENTRIES).map((l) => ({
    id: l.id, atMs: l.atMs, role: l.role, kind: l.kind, text: cut(l.text) as string,
    ...(l.path ? { path: l.path } : {}),
    ...(l.code ? { code: l.code, params: { ...(l.params ?? {}) } } : {}),
    ...(l.callKind ? { callKind: l.callKind } : {}),
    ...(l.status ? { status: l.status } : {}),
    ...(l.output !== undefined ? { output: cut(l.output) } : {}),
    ...(l.steps ? { steps: l.steps.map((s) => ({ text: cut(s.text, 500) as string, ...(s.status ? { status: s.status } : {}) })) } : {}),
  }));
  return { agent: agentId, entries };
}
