import { expect, it } from 'vitest';
import { applyEvent, emptyView, fromSnapshot, tiles } from './view';
import { usageView } from './usageView';
import { agora } from '../gen/agora.js';

// ADR-0104 decision 2 (#930): an agent its node re-states as the same process (taken up from the holder after a node restart, or in
// a resync) keeps the tokens, context level, own cost and start time the window summed for it; a new start of the id starts again.
const started = (id: string, sameProcess?: boolean, nodeId = 'n') =>
  ({ producerId: nodeId, body: 'agentStarted' as const, agentStarted: { agentId: id, nodeId, projectId: 'p', kind: 'claude-code', worktreeId: '', ...(sameProcess === undefined ? {} : { sameProcess }) } });
const used = (id: string, input: number, output: number, cost: number, nodeId = 'n') =>
  ({ producerId: nodeId, body: 'agentUsageReported' as const, agentUsageReported: { agentId: id, model: 'm', inputTokens: input, outputTokens: output, costMicros: cost, currency: 'USD', contextTokens: 100, contextWindow: 1000 } });
const online = (nodeId = 'n') => ({ producerId: nodeId, body: 'nodeOnline' as const, nodeOnline: { nodeId, hostname: 'Laptop' } });

function before() {
  const v = emptyView();
  applyEvent(v, 1, { ...online(), atMs: 1000 });
  applyEvent(v, 2, { ...started('a'), atMs: 2000 });
  applyEvent(v, 3, { producerId: 'n', body: 'agentSessionOptions', agentSessionOptions: { agentId: 'a', currentMode: 'plan', modes: [{ id: 'plan' }] } });
  applyEvent(v, 4, used('a', 5, 7, 10));
  return v;
}

it('keeps the counters of a kept agent its restarted node takes up, and adds what it uses after', () => {
  const v = before();
  applyEvent(v, 5, { ...online(), atMs: 9000 });
  expect(tiles(v)[0]).toMatchObject({ closed: true });
  applyEvent(v, 6, { ...started('a', true), atMs: 9001 });
  const a = tiles(v)[0];
  expect(a).toMatchObject({ closed: false, tokens: 12, startedMs: 2000, session: { contextTokens: 100, contextWindow: 1000, costMicros: 10, currency: 'USD' } });
  // What the node re-states (modes, settings, commands) starts again as for any start.
  expect(a?.session?.currentMode).toBe('');
  applyEvent(v, 7, used('a', 1, 2, 3));
  expect(tiles(v)[0]).toMatchObject({ tokens: 15, session: { costMicros: 13 } });
  // The Usage panel sums the same numbers and measures time from the first start.
  expect(usageView(tiles(v), 10_000)).toMatchObject({ totalTokens: 15, totalActiveMs: 8000 });
});

it('keeps the counters on a resync re-statement of an agent that never stopped', () => {
  const v = before();
  applyEvent(v, 5, { ...started('a', true), atMs: 9001 });
  expect(tiles(v)[0]).toMatchObject({ closed: false, tokens: 12, startedMs: 2000, session: { costMicros: 10 } });
});

it('keeps counters from the snapshot of an agent already running, with no start time it never saw', () => {
  const v = fromSnapshot(agora.v1.Snapshot.create({ pos: 10, nodes: [{ nodeId: 'n', online: true }], agents: [{ agentId: 'n/a', nodeId: 'n', kind: 'claude-code', running: true, inputTokens: 5, outputTokens: 7, contextTokens: 100, contextWindow: 1000, costMicros: 10, currency: 'USD' }] }));
  applyEvent(v, 11, { ...online(), atMs: 9000 });
  applyEvent(v, 12, { ...started('a', true), atMs: 9001 });
  const a = tiles(v)[0];
  expect(a).toMatchObject({ closed: false, tokens: 12, session: { costMicros: 10, contextTokens: 100 } });
  expect(a?.startedMs).toBeUndefined();
});

it('starts the counters again for a new process under the id, an older node, or an id it never listed', () => {
  for (const same of [false, undefined]) {
    const v = before();
    applyEvent(v, 5, { ...started('a', same), atMs: 9001 });
    const a = tiles(v)[0];
    expect(a).toMatchObject({ tokens: 0, startedMs: 9001 });
    expect(a?.session).toBeUndefined();
  }
  const v = emptyView();
  applyEvent(v, 1, { ...started('b', true), atMs: 5000 });
  expect(tiles(v)[0]).toMatchObject({ tokens: 0, startedMs: 5000, closed: false });
  // Another machine's agent with the same local id is another agent.
  const w = before();
  applyEvent(w, 5, { ...started('a', true, 'm'), atMs: 9001 });
  expect(tiles(w).find((t) => t.id === 'm/a')).toMatchObject({ tokens: 0 });
  expect(tiles(w).find((t) => t.id === 'n/a')).toMatchObject({ tokens: 12 });
});
