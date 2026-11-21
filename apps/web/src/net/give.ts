// ADR-0095 step 2, the window's side: "Give to an agent" hands a Backlog task of a project to one of that project's running agents
// (`TaskDispatch`, capability `task.dispatch`). The node claims the task and sends the agent one prompt built from it; nothing here is an
// approval, and the agent keeps its own settings. Pure except for the small store of what this window heard machines answer.
import { useSyncExternalStore } from 'react';
import type { agora } from '../gen/agora.js';
import { helperTree, type AgentTileModel } from '../model';
import type { CommandBody } from './live';
import { refusalKey } from './refusal';

/** The task a "Give to an agent" is about. `key` is the board's own key (`<project>/<task>`), `taskId` the id the node knows. */
export interface GiveTask { key: string; projectId: string; taskId: string; project: string; title: string }

/** What the window says once it tried: given now, given and waiting for the turn to end, or the locale key of the refusal. */
export type GiveResult = { ok: true; queued: boolean } | { ok: false; key: string; args?: Record<string, string> };

// The window cannot read a machine's capabilities (the snapshot carries none, and a window does not negotiate them with the hub), so it
// offers the action and learns from the answer: a machine that refused `TaskDispatch` as `protocol.capability_missing` for `task.dispatch`
// (the hub says so for a node that does not offer it; an older node says so itself) is not offered again in this window. A hub too old to
// know the command at all answers `protocol.invalid_message` for the field `command`, and then no machine is offered.
const lacking = new Set<string>();
let hubLacks = false;
let version = 0;
const listeners = new Set<() => void>();
const changed = () => { version++; listeners.forEach((l) => l()); };

/** Remembers what a refused dispatch says about the machine (or the hub). The refusal itself is still shown. */
export function noteRefusal(node: string, code: string, params: Record<string, string> = {}): void {
  if (code === 'protocol.capability_missing' && (params.capability ?? 'task.dispatch') === 'task.dispatch' && node && !lacking.has(node)) {
    lacking.add(node);
    changed();
  } else if (code === 'protocol.invalid_message' && params.field === 'command' && !hubLacks) {
    hubLacks = true;
    changed();
  }
}
/** Whether the window still offers giving tasks to agents on this machine. */
export const offersGive = (node: string): boolean => !hubLacks && !lacking.has(node);
/** Forgets what was heard (a new connection or another hub starts again; tests). */
export function forgetGiveSupport(): void { lacking.clear(); hubLacks = false; changed(); }
/** Re-renders when what was heard changes. */
export function useGiveSupport(): number {
  return useSyncExternalStore((l) => { listeners.add(l); return () => { listeners.delete(l); }; }, () => version, () => version);
}

/** The agents a task can be given to: running agents of the task's project on a connected machine that may take it, helpers right under
 *  the agent that started them (ADR-0057). Plain terminals are not agents and agents of other projects are never offered. */
export function giveCandidates(agents: readonly AgentTileModel[], projectId: string, online?: ReadonlySet<string>): { agent: AgentTileModel; depth: number }[] {
  if (!projectId) return [];
  const list = agents.filter((a) => a.projectId === projectId && !a.closed && a.state !== 'finished' && !a.interrupted && !!a.machineId
    && a.kind !== 'shell' && (!online || online.has(a.machineId)) && offersGive(a.machineId));
  return helperTree(list);
}

/** The command for giving `task` to `agent`: to the agent's own machine, with `byUser` left empty (the hub writes who asked). */
export function giveRequest(agent: AgentTileModel, task: GiveTask): { node: string; body: CommandBody } {
  const node = agent.machineId ?? '';
  return { node, body: { taskDispatch: { projectId: task.projectId, taskId: task.taskId, agentId: agent.id.slice(agent.id.indexOf('/') + 1), byUser: '' } } };
}

/** The locale key (and its arguments) that says why a dispatch did not go through. */
export function giveRefusal(code: string, params: Record<string, string> = {}): { key: string; args?: Record<string, string> } {
  switch (code) {
    // A terminal program that cannot take a pasted prompt (the node pastes the task as one block so a line break never sends part of it).
    case 'protocol.capability_missing':
      return params.capability === 'terminal.bracketed_paste' ? { key: 'give-refused-paste' } : { key: refusalKey(code) ?? 'give-failed' };
    // A terminal agent holds one task at a time; a chat agent queues up to 8 messages behind its turn.
    case 'agent.queue_full': return params.limit === '1' ? { key: 'give-refused-waiting' } : { key: refusalKey(code) ?? 'give-failed' };
    case 'task.dispatch_limited': return { key: 'give-refused-limited', args: { limit: params.limit || '8' } };
    case 'approval.grant_required': return { key: 'give-refused-grant' };
    case 'node.offline': return { key: 'give-refused-offline' };
    case 'auth.forbidden': return { key: 'give-refused-forbidden' };
    default: return { key: refusalKey(code) ?? 'give-failed' };
  }
}

type Command = (node: string, body: CommandBody) => Promise<agora.v1.CommandOk.$Properties>;

/** Sends the dispatch and says how it went; a refusal also teaches the window what the machine can do (`noteRefusal`). */
export async function giveTask(command: Command, agent: AgentTileModel, task: GiveTask): Promise<GiveResult> {
  const { node, body } = giveRequest(agent, task);
  if (!node) return { ok: false, key: 'give-failed' };
  try {
    const ok = await command(node, body);
    return { ok: true, queued: !!ok.taskDispatch?.queued };
  } catch (e) {
    const err = e as { code?: string; params?: Record<string, string> };
    const code = err.code ?? '';
    noteRefusal(node, code, err.params);
    return { ok: false, ...giveRefusal(code, err.params) };
  }
}

/** The board's tasks a person can give to an agent now (Backlog, with at least one agent to take it), for the command palette. */
export function givableTasks(
  boards: readonly { projectId: string; project: string; tasks: readonly { id: string; taskId?: string; title: string; status: string }[] }[],
  agents: readonly AgentTileModel[],
  online?: ReadonlySet<string>,
  max = 300,
): GiveTask[] {
  const out: GiveTask[] = [];
  for (const b of boards) {
    if (giveCandidates(agents, b.projectId, online).length === 0) continue;
    for (const t of b.tasks) {
      if (t.status !== 'open') continue;
      out.push(giveTaskOf(b, t));
      if (out.length >= max) return out;
    }
  }
  return out;
}

/** A board card as the task to give. */
export function giveTaskOf(b: { projectId: string; project: string }, t: { id: string; taskId?: string; title: string }): GiveTask {
  return { key: t.id, projectId: b.projectId, taskId: t.taskId ?? t.id.slice(b.projectId.length + 1), project: b.project, title: t.title };
}

// Which task the "Give to an agent" dialog is open for. One at a time; the board's cards and the command palette open it.
let asking: GiveTask | undefined;
const askListeners = new Set<() => void>();
export const giveAsk = {
  ask(task: GiveTask) { asking = task; askListeners.forEach((l) => l()); },
  cancel() { if (asking) { asking = undefined; askListeners.forEach((l) => l()); } },
  get: () => asking,
  subscribe: (l: () => void) => { askListeners.add(l); return () => { askListeners.delete(l); }; },
};
export const useGiveAsk = (): GiveTask | undefined => useSyncExternalStore(giveAsk.subscribe, giveAsk.get, giveAsk.get);
