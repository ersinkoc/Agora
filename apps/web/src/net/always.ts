// ADR-0101: "Allow always", an answer the agent remembers. Pure: what a request offers, what an answer may be sent as, and which words say it.

// `agora.v1.ApprovalScope`: 0 (unset) and 1 are a single answer, 2 is one the agent remembers. Any other value is not read as either.
export const SCOPE_UNSPECIFIED = 0;
export const SCOPE_ONCE = 1;
export const SCOPE_ALWAYS = 2;

/** What a request offers beyond a single answer: the agent's own name for its "always" option (untrusted text, shown quoted, may be
 * empty), the exact id of that option (signed, never shown), and whether it offers ONLY that (no single answer: `only`). */
export interface AlwaysOffer { label: string; optionId: string; only: boolean }

/** The offer a request makes, as the machine announced it (its `scopes`, after the machine's policy). Never for a request the machine itself
 * makes (`agent.start`, `agent.skip_permissions`, any `agent.` action): those have signed grants of their own and never carry it. An offer
 * without the option's id cannot be signed, so it is no offer. */
export function alwaysOffer(action: string, scopes: readonly number[], label: string, optionId: string): AlwaysOffer | undefined {
  if (action.startsWith('agent.') || !scopes.includes(SCOPE_ALWAYS) || !optionId) return undefined;
  return { label, optionId, only: !scopes.some((s) => s === SCOPE_ONCE || s === SCOPE_UNSPECIFIED) };
}

/** The node's own id of the asking agent (the signed `agent_id`): the part of the hub-wide key after `<node>/`. Undefined when the key does
 * not belong to that node, so nothing is signed for an agent the window cannot name exactly. */
export function localAgentId(a: { node: string; agentId: string }): string | undefined {
  const prefix = `${a.node}/`;
  return a.node && a.agentId.startsWith(prefix) && a.agentId.length > prefix.length ? a.agentId.slice(prefix.length) : undefined;
}

/** How an answer goes out: a single answer (v1 bytes, no scope), an "always" one (its own bytes and scope ALWAYS), or not at all.
 * ALWAYS only with Allow, only on a request that offers it, never on a start or a switch of mode; a plain Allow is never sent for a
 * request that offers only "always" (the agent would remember it). A Deny is always a single answer, also on such a request. */
export type AnswerPlan = { scope: 'once' } | { scope: 'always'; agentId: string; offer: AlwaysOffer } | { scope: 'none' };
export function answerPlan(
  a: { node: string; agentId: string; always?: AlwaysOffer; start?: unknown; skip?: unknown; skipUnknown?: boolean },
  approved: boolean,
  always: boolean,
): AnswerPlan {
  if (!always) return approved && a.always?.only ? { scope: 'none' } : { scope: 'once' };
  const agentId = localAgentId(a);
  if (!approved || !a.always || a.start || a.skip || a.skipUnknown || !agentId) return { scope: 'none' };
  return { scope: 'always', agentId, offer: a.always };
}

// Codex's review of ADR-0101: "for this session" and "until it ends" are said only for a kind known to drop the answer when its session ends.
// ACP lets the agent decide what the answer covers and for how long, and no machine reports a kind as checked yet (no manifest or protocol
// field says so), so every kind gets the words that say the agent keeps it. The ADR's session words stay under their own ids for the day a
// kind can be marked as checked.
const KEPT = {
  'approval-allow-always': 'approval-allow-always-kept',
  'approval-allow-always-note': 'approval-allow-always-note-kept',
  'approval-always-only': 'approval-always-only-kept',
  'agent-always-allowed-explain': 'agent-always-allowed-explain-kept',
  'permission-approved-always': 'permission-approved-always-kept',
} as const;
export type AlwaysTextId = keyof typeof KEPT;
export const alwaysText = (id: AlwaysTextId, sessionLimited = false): string => (sessionLimited ? id : KEPT[id]);

/** The most actions the "Asks less" chip lists (the hub keeps 16). */
export const ALWAYS_LIST_MAX = 16;
