// A machine can ask for the owner's permission before it lets this window drive its agents (ADR-0041). The permission is a
// grant: ECDSA P-256 over the exact message the node rebuilds (crates/agora-node/src/grants.rs `signed_message`), for one
// scope and a short time. The private key never leaves the browser.
import type { Device } from './approval';

export const GRANT_TTL_MS = 60 * 60 * 1000;

export function signedGrantMessage(nodeId: string, grantId: string, scope: string, commands: string[], expiresAtMs: number): Uint8Array {
  const enc = new TextEncoder();
  const parts = ['agora-grant-v1', nodeId, grantId, scope, commands.join(','), String(expiresAtMs)];
  const out: number[] = [];
  parts.forEach((p, i) => {
    if (i > 0) out.push(0);
    out.push(...enc.encode(p));
  });
  return Uint8Array.from(out);
}

// `agentSetMode` is `agent.set_mode` on the wire.
export function commandName(key: string): string {
  const words = key.replace(/([A-Z])/g, ' $1').toLowerCase().split(' ');
  return `${words[0]}.${words.slice(1).join('_')}`;
}

export type GrantKind = 'typing' | 'start' | 'other';

export interface GrantAsk {
  scope: string;
  commands: string[];
  kind: GrantKind;
}

const AGENT_COMMANDS = ['agentPrompt', 'agentSetMode', 'agentSetModel', 'agentStop', 'terminalInput', 'terminalResize'];

// What to ask the owner for, given the command the node refused. One agent for typing, one project for starting, and the
// whole machine only for what names neither.
export function grantFor(body: Record<string, unknown>): GrantAsk | undefined {
  const key = Object.keys(body).find((k) => body[k] !== undefined && k !== 'idempotencyKey');
  if (!key) return undefined;
  const name = commandName(key);
  const inner = (body[key] ?? {}) as { agentId?: string; terminalId?: string; projectId?: string };
  if (AGENT_COMMANDS.includes(key)) {
    const agent = inner.agentId || (inner.terminalId?.startsWith('t-') ? inner.terminalId.slice(2) : '');
    const commands = [...new Set([name, 'agent.prompt', 'terminal.input'])];
    return { scope: agent ? `agent:${agent}` : 'node', commands, kind: 'typing' };
  }
  if (key === 'agentStart' && inner.projectId) return { scope: `project:${inner.projectId}`, commands: [name], kind: 'start' };
  return { scope: 'node', commands: [name], kind: 'other' };
}

export function newGrantId(): string {
  return `gr-${Array.from(crypto.getRandomValues(new Uint8Array(8)), (b) => b.toString(16).padStart(2, '0')).join('')}`;
}

export async function signGrant(
  device: Device,
  nodeId: string,
  ask: GrantAsk,
  now: number,
  subtle: SubtleCrypto = crypto.subtle,
): Promise<{ grantId: string; scope: string; commands: string[]; expiresAtMs: number; signerDeviceId: string; signature: Uint8Array }> {
  const grantId = newGrantId();
  const expiresAtMs = now + GRANT_TTL_MS;
  const sig = await subtle.sign(
    { name: 'ECDSA', hash: 'SHA-256' },
    device.keys.privateKey,
    signedGrantMessage(nodeId, grantId, ask.scope, ask.commands, expiresAtMs) as BufferSource,
  );
  return { grantId, scope: ask.scope, commands: ask.commands, expiresAtMs, signerDeviceId: device.id, signature: new Uint8Array(sig) };
}
