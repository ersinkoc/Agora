// The ordinary start command's answer, as the window says it: the new agent's id on its machine, or the refusal's locale key. A start a
// machine held back until the owner allows it (ADR-0041, `approval.grant_required`) also carries `grant`: it settles with the answer to
// the start the grant dialog sends again once allowed (or with `start-grant-declined` when it was not), so the start window never asks
// for a second start while the first may still go through. Pure except for the small map of starts that wait for the owner.
import type { agora } from '../gen/agora.js';
import type { CommandBody } from './live';

export type StartAnswer = { agentId: string } | { failure: string };
export type StartResult = { agentId: string } | { failure: string; grant?: Promise<StartAnswer> };
type Command = (node: string, body: CommandBody) => Promise<agora.v1.CommandOk.$Properties>;
type Launch = { ids: readonly string[]; settings: readonly unknown[] };

/** The locale key that says why the machine did not start the agent. */
export function startRefusal(code: string | undefined, params: Record<string, string> = {}, launch: Launch = { ids: [], settings: [] }): string {
  switch (code) {
    // ADR-0059: why the machine refused the chosen launch option (its `policy` parameter, a stable value).
    case 'agent.option_forbidden': return params.policy === 'confined_only' ? 'start-option-confined-only' : 'start-option-forbidden';
    case 'agent.option_unconfirmed': return 'start-option-unconfirmed';
    case 'agent.option_invalid': return 'start-option-invalid';
    case 'agent.setting_unknown': case 'agent.setting_invalid': return 'start-setting-refused';
    case 'protocol.capability_missing':
      return launch.settings.length > 0 ? 'start-settings-unsupported' : launch.ids.length > 0 ? 'start-options-unsupported' : 'start-directory-unsupported';
    case 'agent.mode_unsupported': return 'start-directory-agent-unsupported';
    case 'approval.signature_invalid': return 'approval-not-trusted';
    case 'approval.grant_required': return 'start-refused-grant';
    case 'agent.kind_unknown': return 'start-kind-unknown';
    case 'node.resource_limit': return 'start-limit';
    case 'auth.forbidden': return 'start-directory-forbidden';
    default: return 'start-failed';
  }
}

// Keyed by the exact command object: the connection hands the grant dialog the same object it was asked to send.
const waiting = new WeakMap<object, (r: StartAnswer) => void>();

async function run(command: Command, node: string, body: CommandBody, launch: Launch): Promise<StartResult> {
  try {
    const ok = await command(node, body);
    return { agentId: ok.agentLaunch?.agentId ?? '' };
  } catch (e) {
    const err = e as { code?: string; params?: Record<string, string> };
    const failure = startRefusal(err.code, err.params, launch);
    if (err.code !== 'approval.grant_required') return { failure };
    return { failure, grant: new Promise<StartAnswer>((resolve) => { waiting.set(body, resolve); }) };
  }
}

/** Sends a start (`agentLaunch`) and says how it went. */
export const sendStart = (command: Command, node: string, body: CommandBody, launch: Launch): Promise<StartResult> => run(command, node, body, launch);

/** Whether a command is a start (the grant dialog sends it again through `resendStart`, so its answer reaches the window). */
export const isStart = (body: CommandBody): boolean => !!body.agentLaunch;

/** The owner did not allow it: the waiting window says no agent was started. */
export function declineStart(body: CommandBody): void {
  const resolve = waiting.get(body);
  waiting.delete(body);
  resolve?.({ failure: 'start-grant-declined' });
}

/** Sends a start again after the owner allowed it, and hands the answer to the window that waits for it (if it is still open). */
export async function resendStart(command: Command, node: string, body: CommandBody): Promise<void> {
  const resolve = waiting.get(body);
  waiting.delete(body);
  const b = body.agentLaunch ?? {};
  const result = await run(command, node, body, { ids: b.launchOptions ?? [], settings: b.settings ?? [] });
  // A second permission request (unlikely) keeps the window waiting on the new one.
  if ('grant' in result && result.grant) void result.grant.then((r) => resolve?.(r));
  else resolve?.('agentId' in result ? { agentId: result.agentId } : { failure: result.failure });
}
