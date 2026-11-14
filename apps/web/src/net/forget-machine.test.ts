import { describe, expect, it } from 'vitest';
import { forgetMachineReasonKey } from './forget';
import { applyEvent, fromSnapshot, isOld, machines, OLD_AFTER_MS, projectChoices, shown, tiles } from './view';

// ADR-0073: forgotten machines, old machines.
const snapshot = () => fromSnapshot({
  nodes: [{ nodeId: 'old', hostname: 'Old box', online: false, lastEventMs: 1 }, { nodeId: 'here', hostname: 'Laptop', online: true, lastEventMs: 5 }],
  checkouts: [{ checkoutId: 'old/c', nodeId: 'old', projectId: 'p', path: '/repo' }, { checkoutId: 'here/c', nodeId: 'here', projectId: 'p', path: '/repo' }],
  agents: [
    { agentId: 'old/closed', nodeId: 'old', kind: 'shell', running: false },
    { agentId: 'old/running', nodeId: 'old', kind: 'shell', running: true },
    { agentId: 'here/a', nodeId: 'here', kind: 'shell', running: true },
  ],
});

describe('a forgotten machine', () => {
  it('leaves what people see, with its folders and agents, and comes back with what still runs when it connects', () => {
    const view = snapshot();
    applyEvent(view, 10, { producerId: 'hub', body: 'nodeForgotten', nodeForgotten: { nodeId: 'old' } } as never);
    const seen = shown(view);
    expect(machines(seen).map((m) => m.id)).toEqual(['here']);
    expect(tiles(seen).map((a) => a.id)).toEqual(['here/a']);
    expect([...seen.checkouts.keys()]).toEqual(['here/c']);
    // It connects again: shown again, with the agent it still runs; the closed one stays forgotten.
    applyEvent(view, 11, { producerId: 'hub', body: 'nodeLink', nodeLink: { nodeId: 'old', up: true, generation: 2 } } as never);
    const back = shown(view);
    expect(machines(back).map((m) => m.id).sort()).toEqual(['here', 'old']);
    expect(tiles(back).map((a) => a.id).sort()).toEqual(['here/a', 'old/running']);
    expect(projectChoices(back).some((c) => c.node === 'old')).toBe(true);
  });

  it('is only forgotten by the hub, never by a machine saying so', () => {
    const view = snapshot();
    applyEvent(view, 10, { producerId: 'here', body: 'nodeForgotten', nodeForgotten: { nodeId: 'old' } } as never);
    expect(machines(shown(view))).toHaveLength(2);
  });
});

describe('an old machine', () => {
  it('is offline and not seen for a day', () => {
    const now = 10 * OLD_AFTER_MS;
    expect(isOld({ online: false, lastSeenMs: now - OLD_AFTER_MS }, now)).toBe(true);
    expect(isOld({ online: false, lastSeenMs: now - OLD_AFTER_MS + 1 }, now)).toBe(false);
    expect(isOld({ online: true, lastSeenMs: 0 }, now)).toBe(false);
  });
});

describe('why a machine was not forgotten', () => {
  it('has words for each answer the hub gives', () => {
    expect(forgetMachineReasonKey('node.online')).toBe('forget-machine-reason-online');
    expect(forgetMachineReasonKey('auth.forbidden')).toBe('forget-machine-reason-forbidden');
    expect(forgetMachineReasonKey('node.unknown')).toBe('forget-machine-reason-gone');
    expect(forgetMachineReasonKey('anything')).toBe('forget-machine-reason-other');
  });
});
