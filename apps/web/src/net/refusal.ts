// What a refused change of an agent says (ADR-0090). A command that did not go through carries a stable code; the window says it in plain
// words from its locale files, and falls back to its general sentence for a code it does not know.

import { markSkipCapable } from './skipApproval';

/** The machine neither applied nor refused the choice: it waits for the owner's signed approval (ADR-0090 Amendment A, `agent.approval_needed`).
 *  `approvalNeeded` is the hub-wide key `<node>/<approval id>` of the card that is raised for it. */
export interface ApprovalNeeded { approvalNeeded: string }

/** `true`: it went through. `false`: it did not, for a reason not known. A string: the refusal's stable code. An object: it waits for approval. */
export type ActionResult = boolean | string | ApprovalNeeded;

/** What a failed command's error says: `{approvalNeeded}` for `agent.approval_needed` with its id, else the stable code (or `false`). */
export function failure(node: string, e: unknown): ActionResult {
  const err = e as { code?: string; params?: Record<string, string> };
  if (err.code === 'agent.approval_needed') {
    if (!err.params?.approval_id) return false;
    markSkipCapable(node); // only a node that offers `approval.skip_permissions` answers so (see skipApproval.ts)
    return { approvalNeeded: `${node}/${err.params.approval_id}` };
  }
  // The one limit that has its own words: too many requests waiting for this agent (the others read as the machine's general limit).
  if (err.code === 'node.resource_limit' && err.params?.resource === 'skip_approvals_agent') return 'node.resource_limit.skip_approvals_agent';
  return err.code ?? false;
}

const KEYS: Record<string, string> = {
  'agent.option_unconfirmed': 'refusal-unconfirmed',
  'agent.option_forbidden': 'refusal-forbidden',
  'agent.option_unknown': 'refusal-unknown-option',
  'agent.queue_full': 'prompt-queue-full',
  'protocol.capability_missing': 'refusal-unsupported',
  'node.resource_limit': 'refusal-limit',
  // `node.resource_limit` with resource `skip_approvals_agent` (see `failure`).
  'node.resource_limit.skip_approvals_agent': 'refusal-skip-agent-limit',
  'agent.not_found': 'refusal-agent-gone',
  // ADR-0095: giving a board task to an agent (net/give.ts adds the refusals whose words depend on
  // their parameters or that only giving a task gives, such as `mesh.recipient_unknown`).
  'task.not_open': 'refusal-task-taken',
  'task.blocked': 'refusal-task-blocked',
  'task.not_found': 'refusal-task-gone',
  'board.unavailable': 'refusal-board-unavailable',
};

/** The locale key to say for a refusal code; undefined for none known (the caller uses its own general sentence). */
export const refusalKey = (code: string): string | undefined => KEYS[code];

/** Every locale key `refusalKey` can return, for tests. */
export const REFUSAL_KEYS: readonly string[] = [...new Set(Object.values(KEYS))];
