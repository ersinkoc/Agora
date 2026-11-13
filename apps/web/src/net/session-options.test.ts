import { describe, expect, it } from 'vitest';
import { agora } from '../gen/agora.js';
import { applyEvent, emptyView, fromSnapshot, tiles } from './view';
import { choices, contextPercent, needsConfirm, ownCost } from '../components/sessionChoice';

const ev = (producer: string, seq: number, body: Record<string, unknown>) => agora.v1.Event.create({ producerId: producer, seq, ...body });
const opt = (id: string, skips = false) => ({ id, name: id.toUpperCase(), description: '', skipsPermissions: skips });

// ADR-0058: what a chat agent reports about its session reaches the window, live and after a reload.
describe('session options, context and own cost', () => {
  it('follows the events', () => {
    const v = emptyView();
    applyEvent(v, 1, ev('n1', 1, { agentStarted: { agentId: 'a1', nodeId: 'n1', kind: 'acp' } }));
    applyEvent(v, 2, ev('n1', 2, { agentSessionOptions: { agentId: 'a1', modes: [opt('build'), opt('yolo', true)], models: [{ id: 'm1', skipsPermissions: true }], currentMode: 'build', currentModel: 'm1' } }));
    applyEvent(v, 3, ev('n1', 3, { agentUsageReported: { agentId: 'a1', costMicros: 10_000, currency: 'USD', contextTokens: 250, contextWindow: 1000 } }));
    applyEvent(v, 4, ev('n1', 4, { agentUsageReported: { agentId: 'a1', costMicros: 5_000, currency: 'USD' } }));
    const s = tiles(v)[0]?.session;
    expect(s?.modes.map((m) => [m.id, m.skipsPermissions])).toEqual([['build', false], ['yolo', true]]);
    expect(s?.models[0]?.skipsPermissions).toBe(false); // only a mode can skip permissions
    expect([s?.currentMode, s?.currentModel, s?.contextTokens, s?.costMicros]).toEqual(['build', 'm1', 250, 15_000]);
    expect(contextPercent(s)).toBe(25);
    expect(ownCost(s, 'en-US')).toBe('$0.02');
  });
  it('comes back from the snapshot', () => {
    const v = fromSnapshot({ agents: [{ agentId: 'n1/a1', nodeId: 'n1', modes: [opt('plan')], currentMode: 'plan', contextTokens: 5, contextWindow: 10 }] });
    const s = tiles(v)[0]?.session;
    expect([s?.currentMode, contextPercent(s), ownCost(s)]).toEqual(['plan', 50, undefined]);
    expect(tiles(fromSnapshot({ agents: [{ agentId: 'n1/a2', nodeId: 'n1' }] }))[0]?.session).toBeUndefined();
  });
  it('asks before a mode that does not ask, and only then', () => {
    expect(needsConfirm('mode', opt('yolo', true))).toBe(true);
    expect(needsConfirm('mode', opt('plan'))).toBe(false);
    expect(needsConfirm('model', opt('m', true))).toBe(false);
    expect(needsConfirm('mode', undefined)).toBe(false);
    expect(choices([opt('a')], 'gone').map((o) => o.id)).toEqual(['gone', 'a']);
    expect(contextPercent({ modes: [], models: [], currentMode: '', currentModel: '', contextTokens: 9, contextWindow: 0 })).toBeUndefined();
  });
});
