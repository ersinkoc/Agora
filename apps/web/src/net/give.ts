// ADR-0095 step 2, the window's side: "Give to an agent" hands a Backlog task of a project to one of that project's running agents
// (`TaskDispatch`, capability `task.dispatch`). The node claims the task and sends the agent one prompt built from it; nothing here is an
// approval, and the agent keeps its own settings. Pure except for the small store of what this window heard machines answer.
import { useSyncExternalStore } from 'react';
import type { agora } from '../gen/agora.js';
import { helperTree, type AgentTileModel } from '../model';
import type { CommandBody } from './live';
import { refusalKey } from './refusal';
import { actsFreely } from '../components/sessionChoice';

/** The task a "Give to an agent" is about. `key` is the board's own key (`<project>/<task>`), `taskId` the id the node knows. */
export interface GiveTask { key: string; projectId: string; taskId: string; project: string; title: string }

/** What the window says once it tried: given now, given and waiting for the turn to end, or the locale key of the refusal. `grant`: the
 *  machine asked for the owner's permission first; it settles with the answer to the command that is sent again once it is allowed. */
export type GiveResult = { ok: true; queued: boolean } | { ok: false; key: string; args?: Record<string, string>; grant?: Promise<GiveResult> };

// ADR-0132: a hub that publishes them says which machines' current links speak `task.dispatch` (`noteCapabilities`); a machine known
// not to is not offered, and one not known (an older hub, a link that just opened) is offered as before, the window learning from the
// answer. A machine that does not take tasks answers in one of two ways: `protocol.capability_missing`
// for `task.dispatch` (the hub, for a node that does not offer it), or `protocol.invalid_message` for the field `command` (a node too old to
// decode the command; a hub too old to know it answers the same, and the window cannot tell the two apart). Either way only that machine is
// no longer offered. What was heard of a machine is forgotten when it goes offline (an update restarts it) and on every new connection.
const lacking = new Set<string>();
let version = 0;
const listeners = new Set<() => void>();
const changed = () => { version++; listeners.forEach((l) => l()); };

/** Remembers what a refused dispatch says about the machine. The refusal itself is still shown. */
export function noteRefusal(node: string, code: string, params: Record<string, string> = {}): void {
  const cannot = (code === 'protocol.capability_missing' && (params.capability ?? 'task.dispatch') === 'task.dispatch')
    || (code === 'protocol.invalid_message' && params.field === 'command');
  if (cannot && node && !lacking.has(node)) {
    lacking.add(node);
    changed();
  }
}
// ADR-0132: machines whose current link is known not to speak `task.dispatch`, from the hub's published capabilities.
let knownLacking = new Set<string>();
/** Takes what the hub publishes of each machine's link: a machine with a known list without `task.dispatch` is known not to take tasks;
 *  a machine without one (not known) is offered as before. */
export function noteCapabilities(list: readonly { id: string; capabilities?: readonly string[] }[]): void {
  const next = new Set(list.filter((m) => m.capabilities && !m.capabilities.includes('task.dispatch')).map((m) => m.id));
  if (next.size === knownLacking.size && [...next].every((id) => knownLacking.has(id))) return;
  knownLacking = next;
  changed();
}
/** Whether the window still offers giving tasks to agents on this machine. */
export const offersGive = (node: string): boolean => !lacking.has(node) && !knownLacking.has(node);
/** Forgets what was heard (a new connection or another hub starts again; tests). */
export function forgetGiveSupport(): void { if (lacking.size > 0 || knownLacking.size > 0) { lacking.clear(); knownLacking = new Set(); changed(); } }
/** Forgets what was heard of machines that are not connected now: one that comes back may have been updated. */
export function forgetOffline(online: ReadonlySet<string>): void {
  let any = false;
  for (const node of lacking) if (!online.has(node)) { lacking.delete(node); any = true; }
  if (any) changed();
}
/** Re-renders when what was heard changes. */
export function useGiveSupport(): number {
  return useSyncExternalStore((l) => { listeners.add(l); return () => { listeners.delete(l); }; }, () => version, () => version);
}

/** The agents a task can be given to: running agents of the task's project on a connected machine that may take it, helpers right under
 *  the agent that started them (ADR-0057). Plain terminals are not agents and agents of other projects are never offered. */
export function giveCandidates(agents: readonly AgentTileModel[], projectId: string, online?: ReadonlySet<string>): { agent: AgentTileModel; depth: number }[] {
  if (!projectId) return [];
  return helperTree(agents.filter((a) => canTake(a, projectId, online)));
}

/** Whether this agent may be given a task of `projectId` now. By its lifecycle, not its state word: a running agent whose last turn is
 *  Finished is between turns and takes it; a closed or interrupted one does not. */
export function canTake(a: AgentTileModel, projectId: string, online?: ReadonlySet<string>): boolean {
  return !!projectId && a.projectId === projectId && !a.closed && !a.interrupted && !!a.machineId && (a.hostKind ?? a.kind) !== 'shell'
    && (!online || online.has(a.machineId)) && offersGive(a.machineId);
}

/** Whether a new agent for a task of `projectId` can be started here (ADR-0095 step 3): the project has a folder on a connected machine
 *  that was not heard to lack `task.dispatch`, so the agent started there could be given the task. */
export function canStartFor(projects: readonly { node: string; projectId: string }[], machines: readonly { id: string; online: boolean }[], projectId: string): boolean {
  return !!projectId && projects.some((p) => p.projectId === projectId && offersGive(p.node) && machines.some((m) => m.id === p.node && m.online));
}

/** Why an agent cannot be given a task of `projectId` now, as the locale key the dialog shows: its machine is not connected, or was heard
 *  not to take tasks, or the agent stopped, was closed or interrupted, or left the project (or is gone from the list). */
export function cannotTakeKey(a: AgentTileModel | undefined, online?: ReadonlySet<string>): string {
  if (a?.machineId && !a.closed && !a.interrupted) {
    if (online && !online.has(a.machineId)) return 'give-refused-offline';
    if (!offersGive(a.machineId)) return 'give-machine-unsupported';
  }
  return 'give-target-gone';
}

/** Whether the agent acts without asking now: started so (a launch option, ADR-0059), or its current mode or setting is one that skips
 *  approvals (ADR-0058, ADR-0090 Amendment A). */
export const actsFreelyNow = (a: AgentTileModel): boolean => !!a.skipsPermissions || actsFreely(a.session);

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
    // Only giving a task names an agent of another project this way; other commands keep their general sentence.
    case 'mesh.recipient_unknown': return { key: 'give-refused-other-project' };
    case 'node.offline': return { key: 'give-refused-offline' };
    case 'auth.forbidden': return { key: 'give-refused-forbidden' };
    default: return { key: refusalKey(code) ?? 'give-failed' };
  }
}

type Command = (node: string, body: CommandBody) => Promise<agora.v1.CommandOk.$Properties>;

// A dispatch the machine held back until the owner allows it (ADR-0041): the window's grant dialog sends the same command again once
// allowed (`resendGive`), and its answer goes to the dialog that waits here. Keyed by the exact command.
const waiting = new Map<string, (r: GiveResult) => void>();
const waitKey = (node: string, body: CommandBody) => {
  const d = body.taskDispatch;
  return d ? [node, d.agentId ?? '', d.projectId ?? '', d.taskId ?? ''].join('\u0000') : '';
};

async function run(command: Command, node: string, body: CommandBody): Promise<GiveResult> {
  try {
    const ok = await command(node, body);
    return { ok: true, queued: !!ok.taskDispatch?.queued };
  } catch (e) {
    const err = e as { code?: string; params?: Record<string, string> };
    const code = err.code ?? '';
    noteRefusal(node, code, err.params);
    const said = giveRefusal(code, err.params);
    if (code !== 'approval.grant_required') return { ok: false, ...said };
    const key = waitKey(node, body);
    return { ok: false, ...said, grant: new Promise<GiveResult>((resolve) => { waiting.set(key, resolve); }) };
  }
}

/** Sends the dispatch and says how it went; a refusal also teaches the window what the machine can do (`noteRefusal`). */
export async function giveTask(command: Command, agent: AgentTileModel, task: GiveTask): Promise<GiveResult> {
  const { node, body } = giveRequest(agent, task);
  if (!node) return { ok: false, key: 'give-failed' };
  return run(command, node, body);
}

/** Whether a command is a dispatch (the grant dialog sends it again through `resendGive`, so its answer is shown). */
export const isGive = (body: CommandBody): boolean => !!body.taskDispatch;

/** The owner did not allow it (the grant dialog was dismissed): the waiting dialog says nothing was sent and offers another try. */
export function declineGive(node: string, body: CommandBody): void {
  const key = waitKey(node, body);
  const resolve = waiting.get(key);
  waiting.delete(key);
  resolve?.({ ok: false, key: 'give-grant-declined' });
}

/** Sends a dispatch again after the owner allowed it, and hands the answer to the dialog that waits for it (if it is still open). */
export async function resendGive(command: Command, node: string, body: CommandBody): Promise<void> {
  const key = waitKey(node, body);
  const resolve = waiting.get(key);
  waiting.delete(key);
  const result = await run(command, node, body);
  // A second permission request (unlikely) keeps the dialog waiting on the new one.
  if (!result.ok && result.grant) void result.grant.then((r) => resolve?.(r));
  else resolve?.(result);
}

/** The board's tasks a person can give to an agent now, for the command palette: Backlog, with at least one agent to take it, or any
 *  Backlog task of a project the window can start a new agent for (`canStart`, ADR-0095 step 3). */
export function givableTasks(
  boards: readonly { projectId: string; project: string; tasks: readonly { id: string; taskId?: string; title: string; status: string }[] }[],
  agents: readonly AgentTileModel[],
  online?: ReadonlySet<string>,
  max = 300,
  canStart?: (projectId: string) => boolean,
): GiveTask[] {
  const out: GiveTask[] = [];
  for (const b of boards) {
    if (!b.projectId || (!canStart?.(b.projectId) && giveCandidates(agents, b.projectId, online).length === 0)) continue;
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
