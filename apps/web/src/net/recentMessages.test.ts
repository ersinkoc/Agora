// ADR-0103 (#1020): a window that opens, reloads or reconnects into a snapshot shows the recent Team chat. The snapshot's
// `recent_messages` are folded in through the live `messagePosted` path (pairing, `via`, the cap), not through `applyEvent`'s position
// rule, which would drop them: their positions are at or before the snapshot's.
import { describe, expect, it } from 'vitest';
import { agora } from '../gen/agora.js';
import { applyEvent, fromSnapshot, recentMessages } from './view';

const v1 = agora.v1;

function posted(pos: number, producer: string, from: string, to: string, body: string) {
  return { pos, event: { producerId: producer, seq: pos, atMs: 1000 + pos, messagePosted: { messageId: `m-${pos}`, fromAgentId: from, to, body } } };
}

const agents = [
  { agentId: 'a/x', nodeId: 'a', kind: 'claude', running: true },
  { agentId: 'b/z', nodeId: 'b', kind: 'codex', running: true },
];

describe('recent messages in the snapshot (ADR-0103)', () => {
  it('shows a message sent before the window opened', () => {
    const v = fromSnapshot({ pos: 9, agents, recentMessages: [posted(4, 'a', 'x', 'y', 'before you came')] });
    expect(recentMessages(v).map((m) => [m.body, m.fromId, m.atMs])).toEqual([['before you came', 'a/x', 1004]]);
  });

  it('keeps the snapshot order, oldest first, and includes a message at the snapshot position itself', () => {
    const v = fromSnapshot({ pos: 6, agents, recentMessages: [posted(2, 'a', 'x', 'y', 'one'), posted(5, 'a', 'x', 'y', 'two'), posted(6, 'a', 'x', 'y', 'three')] });
    expect(recentMessages(v).map((m) => m.body)).toEqual(['three', 'two', 'one']);
  });

  it('pairs the two copies of a message between machines into one row, as live', () => {
    const v = fromSnapshot({ pos: 9, agents, recentMessages: [posted(3, 'a', 'x', 'b/z', 'ping'), posted(4, 'b', 'a/x', 'z', 'ping')] });
    const m = recentMessages(v);
    expect(m).toHaveLength(1);
    expect(m[0]).toMatchObject({ body: 'ping', crossMachine: true, fromId: 'a/x' });
    expect(m[0]?.via).toBeUndefined();
  });

  it('marks a receiver copy whose sender copy the window was not sent with the machine that said so', () => {
    const v = fromSnapshot({ pos: 9, agents, recentMessages: [posted(4, 'b', 'a/x', 'z', 'trust me')] });
    expect(recentMessages(v)[0]).toMatchObject({ from: expect.any(String), fromId: 'a/x', via: 'b' });
  });

  it('pairs a copy from the snapshot with its twin that arrives live', () => {
    const v = fromSnapshot({ pos: 9, agents, recentMessages: [posted(8, 'b', 'a/x', 'z', 'late twin')] });
    expect(recentMessages(v)[0]?.via).toBe('b');
    applyEvent(v, 10, v1.Event.create({ producerId: 'a', seq: 10, atMs: 1010, messagePosted: { messageId: 'm-1', fromAgentId: 'x', to: 'b/z', body: 'late twin' } }));
    const m = recentMessages(v);
    expect(m).toHaveLength(1);
    expect(m[0]?.via).toBeUndefined();
  });

  it('folds each position once and leaves one after the snapshot to the live stream', () => {
    const v = fromSnapshot({ pos: 9, agents, recentMessages: [posted(4, 'a', 'x', 'y', 'once'), posted(4, 'a', 'x', 'y', 'once'), posted(12, 'a', 'x', 'y', 'later')] });
    expect(recentMessages(v).map((m) => m.body)).toEqual(['once']);
    // The live event for position 12 still applies, once.
    applyEvent(v, 12, v1.Event.create({ producerId: 'a', seq: 12, atMs: 1012, messagePosted: { messageId: 'm-12', fromAgentId: 'x', to: 'y', body: 'later' } }));
    applyEvent(v, 12, v1.Event.create({ producerId: 'a', seq: 12, atMs: 1012, messagePosted: { messageId: 'm-12', fromAgentId: 'x', to: 'y', body: 'later' } }));
    expect(recentMessages(v).map((m) => m.body)).toEqual(['later', 'once']);
  });

  it('ignores any other body and an entry without an event', () => {
    const v = fromSnapshot({
      pos: 9, agents,
      recentMessages: [
        { pos: 2, event: { producerId: 'a', seq: 2, atMs: 1002, nodeOnline: { nodeId: 'a', hostname: 'evil' } } },
        { pos: 3 },
        posted(5, 'a', 'x', 'y', 'kept'),
      ],
    });
    expect(recentMessages(v).map((m) => m.body)).toEqual(['kept']);
    expect(v.nodes.size).toBe(0);
  });

  it('keeps the newest 50, as live', () => {
    const many = Array.from({ length: 60 }, (_, i) => posted(i + 1, 'a', 'x', 'y', `m${i + 1}`));
    const v = fromSnapshot({ pos: 60, agents, recentMessages: many });
    expect(v.messages).toHaveLength(50);
    expect(v.messages[0]?.body).toBe('m11');
    expect(v.messages[49]?.body).toBe('m60');
  });

  it('decodes from the wire and keeps a body as plain text', () => {
    const body = '<img src=x onerror=alert(1)> **not markdown**';
    const bytes = v1.Snapshot.encode(v1.Snapshot.create({ pos: 3, recentMessages: [posted(2, 'a', 'x', 'y', body)] })).finish();
    const v = fromSnapshot(v1.Snapshot.decode(bytes));
    expect(recentMessages(v)[0]?.body).toBe(body);
  });

  it('an older snapshot with no messages shows none, as before', () => {
    expect(fromSnapshot({ pos: 3, agents }).messages).toEqual([]);
  });
});
