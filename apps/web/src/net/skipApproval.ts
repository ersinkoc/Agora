import { useSyncExternalStore } from 'react';
import { toHex, type Device } from './approval';

// ADR-0090 Amendment A: a running agent switched to a mode or option that skips permission questions is an approval of the kind
// `agent.skip_permissions`, raised by the node and decided over the domain `agora-approval-skip-v1`
// (docs/specs/protocol.md, vectors in schema/fixtures/signing/approval-skip-v1.json, crates/agora-node/src/approvals.rs `skip_signed_message`).

export const SKIP_ACTION = 'agent.skip_permissions';
const DOMAIN = 'agora-approval-skip-v1';

/** What the node's `detail` says, every field as the node gave it. `optionName` and `valueName` are the agent's own words (untrusted):
 *  shown as text, never interpreted. `option` and `value` are the ids the decision is signed over. */
export interface SkipRequest {
  node: string; agent: string; agentKind: string; folder: string;
  optionType: 'mode' | 'config'; option: string; optionName: string; value: string; valueName: string;
}
// `option` and `value` are signed exactly as the node gave them (never trimmed, cleaned or re-encoded here); the node already bounded and
// cleaned every other string.

/** The detail read strictly: every field a string, the type one the window knows, a mode's option the literal `mode`, the scope `session`.
 *  Anything else (a missing field, another type, another scope) is `undefined`: the window shows "update your window" and signs nothing. */
export function parseSkipDetail(detail: string): SkipRequest | undefined {
  let d: unknown;
  try { d = JSON.parse(detail); } catch { return undefined; }
  if (typeof d !== 'object' || d === null || Array.isArray(d)) return undefined;
  const o = d as Record<string, unknown>;
  const s = (k: string): string | undefined => (typeof o[k] === 'string' ? (o[k] as string) : undefined);
  const node = s('node'), agent = s('agent'), agentKind = s('agent_kind'), folder = s('folder'), type = s('option_type');
  const option = s('option'), optionName = s('option_name'), value = s('value'), valueName = s('value_name');
  if (node === undefined || !agent || agentKind === undefined || folder === undefined || option === undefined || optionName === undefined || value === undefined || valueName === undefined) return undefined;
  if (type !== 'mode' && type !== 'config') return undefined;
  // `skips: true` is the node's own judgement that the choice makes the agent act without asking; a card without it is not what is expected.
  if (o.skips !== true) return undefined;
  if (s('scope') !== 'session' || option === '' || (type === 'mode' && option !== 'mode')) return undefined;
  return { node, agent, agentKind, folder, optionType: type, option, optionName, value, valueName };
}

/** The request of an approval as the window may act on it: `ok` when it is the shape the window expects AND names this node and agent;
 *  `unknown` when the action is the skip kind but the card is not what the window expects; `undefined` for any other approval. */
export function skipOf(action: string, detail: string, node: string, agentId: string): { ok: SkipRequest } | { unknown: true } | undefined {
  if (action !== SKIP_ACTION) return undefined;
  const r = parseSkipDetail(detail);
  // The card must be about the machine and the agent the hub filed it under; otherwise it is not what the window expects.
  if (!r || r.node !== node || (agentId !== r.agent && agentId !== `${node}/${r.agent}`)) return { unknown: true };
  return { ok: r };
}

/** `F(x)`: the decimal byte length of x, `:`, then x. */
const frame = (x: string): Uint8Array => {
  const b = new TextEncoder().encode(x);
  return new Uint8Array([...new TextEncoder().encode(`${b.length}:`), ...b]);
};

/** The bytes a device signs: the domain, one NUL, then F(node_id) F(approval_id) F(verdict) F(agent_id) F(option_id) F(value) F(sha256(detail)). */
export async function skipSignedMessage(
  nodeId: string, approvalId: string, approved: boolean, agentId: string, optionId: string, value: string, detail: string,
  subtle: SubtleCrypto = crypto.subtle,
): Promise<Uint8Array> {
  const hash = toHex(new Uint8Array(await subtle.digest('SHA-256', new TextEncoder().encode(detail) as BufferSource)));
  const parts = [new TextEncoder().encode(DOMAIN), new Uint8Array([0]), ...[nodeId, approvalId, approved ? '1' : '0', agentId, optionId, value, hash].map(frame)];
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let at = 0;
  for (const p of parts) { out.set(p, at); at += p.length; }
  return out;
}

/** The signature of a decision on a skip request; `agent_id`, `option_id` and `value` are the detail's own `agent`, `option` and `value`. */
export async function signSkip(device: Device, nodeId: string, approvalId: string, approved: boolean, req: SkipRequest, detail: string, subtle: SubtleCrypto = crypto.subtle): Promise<Uint8Array> {
  const message = await skipSignedMessage(nodeId, approvalId, approved, req.agent, req.option, req.value, detail, subtle);
  return new Uint8Array(await subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, device.keys.privateKey, message as BufferSource));
}

// The window cannot read a node's capabilities (the snapshot carries none). A node that has `approval.skip_permissions` is the only one
// that answers a switch with `agent.approval_needed` (any other refuses with `agent.option_unconfirmed`), so that answer is the evidence:
// the window shows an answerable card, and signs, only for a node it has heard say so.
const capable = new Set<string>();
const listeners = new Set<() => void>();
let version = 0;
const changed = () => { version++; listeners.forEach((l) => l()); };
export function markSkipCapable(node: string): void {
  if (!node || capable.has(node)) return;
  capable.add(node);
  changed();
}
export const isSkipCapable = (node: string): boolean => capable.has(node);
/** Forgets what was heard: of one machine, or (no argument) of all, and every answer given here. A new connection or another hub starts again; tests. */
export function forgetSkipCapable(node?: string): void {
  if (node === undefined) { capable.clear(); answers.clear(); } else capable.delete(node);
  changed();
}

// What THIS window answered in this session, by the card's hub-wide key `<node>/<approval id>`: so the window says "with your approval" only
// for a value it saw approved, and tells an approved switch (the report may come later) from one that was denied.
interface Answer { approved: boolean; agent: string; option: string; value: string }
const answers = new Map<string, Answer>();
export function recordSkipAnswer(key: string, a: Answer): void { answers.set(key, a); changed(); }
export function forgetSkipAnswer(key: string): void { answers.delete(key); changed(); }
export const skipAnswerOf = (key: string): 'approved' | 'denied' | undefined => { const a = answers.get(key); return a ? (a.approved ? 'approved' : 'denied') : undefined; };
/** Whether this window approved exactly this value of this option for this agent (`<node>/<agent>`) in this session. */
export function approvedHere(agentKey: string, option: string, value: string): boolean {
  for (const a of answers.values()) if (a.approved && a.agent === agentKey && a.option === option && a.value === value) return true;
  return false;
}

// An unknown-shape card is dismissed locally only: nothing is sent, because a decision in a shape the window does not understand is never signed.
const dismissedUnknown = new Set<string>();
export function dismissUnknown(id: string): void { dismissedUnknown.add(id); changed(); }
export const isDismissedUnknown = (id: string): boolean => dismissedUnknown.has(id);

/** Re-renders on any change of what is remembered here; the value is whether the machine was heard to offer it. */
export function useSkipCapable(node: string): boolean {
  useSyncExternalStore((l) => { listeners.add(l); return () => { listeners.delete(l); }; }, () => version, () => version);
  return capable.has(node);
}
