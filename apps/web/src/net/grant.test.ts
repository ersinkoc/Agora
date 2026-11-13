import { describe, expect, it } from 'vitest';
import { commandName, grantFor, signGrant, signedGrantMessage } from './grant';
import type { Device } from './approval';

describe('grants', () => {
  it('builds the exact bytes the node rebuilds', () => {
    // The same literal is asserted in crates/agora-node/src/grants.rs: the two sides must agree byte for byte.
    const m = signedGrantMessage('laptop', 'g1', 'agent:a-1', ['agent.prompt', 'terminal.input'], 1234);
    expect(new TextDecoder().decode(m)).toBe('agora-grant-v1\0laptop\0g1\0agent:a-1\0agent.prompt,terminal.input\x001234');
  });

  it('names commands the way the node does', () => {
    expect(commandName('agentSetMode')).toBe('agent.set_mode');
    expect(commandName('terminalInput')).toBe('terminal.input');
    expect(commandName('filesWrite')).toBe('files.write');
  });

  it('asks for one agent when typing, one project when starting, the machine otherwise', () => {
    expect(grantFor({ agentPrompt: { agentId: 'a-1', text: 'x' } })).toEqual({
      scope: 'agent:a-1', commands: ['agent.prompt', 'terminal.input'], kind: 'typing',
    });
    expect(grantFor({ terminalInput: { terminalId: 't-a-2', data: new Uint8Array() } })?.scope).toBe('agent:a-2');
    // ADR-0058: a model switch is about one agent, like a mode switch (it was asked machine-wide: `node`).
    expect(grantFor({ agentSetModel: { agentId: 'agent-1', model: 'm' } })).toEqual({
      scope: 'agent:agent-1', commands: ['agent.set_model', 'agent.prompt', 'terminal.input'], kind: 'typing',
    });
    expect(grantFor({ agentSetMode: { agentId: 'agent-1', mode: 'plan' } })?.scope).toBe('agent:agent-1');
    expect(grantFor({ agentStart: { projectId: 'p-1', kind: 'claude' } })).toEqual({ scope: 'project:p-1', commands: ['agent.start'], kind: 'start' });
    expect(grantFor({ filesWrite: { worktreeId: 'w', path: 'a' } })).toEqual({ scope: 'node', commands: ['files.write'], kind: 'other' });
    expect(grantFor({})).toBeUndefined();
  });

  it('signs with the device key and the signature verifies over that message', async () => {
    const keys = await crypto.subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, false, ['sign', 'verify']);
    const device: Device = { id: 'web-1', keys, publicKeyHex: '' };
    const ask = { scope: 'node', commands: ['agent.start'], kind: 'start' as const };
    const g = await signGrant(device, 'laptop', ask, 1000);
    expect(g.expiresAtMs).toBe(1000 + 3_600_000);
    expect(g.grantId).toMatch(/^gr-[0-9a-f]{16}$/);
    const ok = await crypto.subtle.verify({ name: 'ECDSA', hash: 'SHA-256' }, keys.publicKey,
      g.signature as BufferSource, signedGrantMessage('laptop', g.grantId, 'node', ['agent.start'], g.expiresAtMs) as BufferSource);
    expect(ok).toBe(true);
    const other = await crypto.subtle.verify({ name: 'ECDSA', hash: 'SHA-256' }, keys.publicKey,
      g.signature as BufferSource, signedGrantMessage('desktop', g.grantId, 'node', ['agent.start'], g.expiresAtMs) as BufferSource);
    expect(other).toBe(false);
  });
});
