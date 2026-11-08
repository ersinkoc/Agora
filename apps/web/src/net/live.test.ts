import { describe, expect, it } from 'vitest';
import { agora } from '../gen/agora.js';
import { decodeFrame, encodeFrame, EVENT_KIND, STREAM_CLOSE, STREAM_COMMANDS, STREAM_EVENTS } from './frame';
import { connectLive, type LinkState, type Socket } from './live';
import { costByProject, applyEvent, emptyView, fromSnapshot, machines, recentMessages, machinesOnline, projectChoices, tiles, type View } from './view';

const v1 = agora.v1;

function ev(producer: string, seq: number, body: Record<string, unknown>) {
  return v1.Event.create({ producerId: producer, seq, atMs: 1000 + seq, ...body });
}

describe('view', () => {
  it('folds events into tiles, keyed by producer', () => {
    const v = emptyView();
    applyEvent(v, 1, ev('laptop', 1, { nodeOnline: { nodeId: 'laptop', hostname: 'Laptop' } }));
    applyEvent(v, 2, ev('laptop', 2, { worktreeCreated: { worktreeId: 'w1', checkoutId: 'c1', path: '/x/login-fix', branch: 'login-fix' } }));
    applyEvent(v, 3, ev('laptop', 3, { agentStarted: { agentId: 'a1', nodeId: 'laptop', worktreeId: 'w1', kind: 'claude' } }));
    applyEvent(v, 4, ev('other', 1, { agentStarted: { agentId: 'a1', nodeId: 'other', worktreeId: 'w9', kind: 'codex' } }));
    applyEvent(v, 5, ev('laptop', 4, { agentStateChanged: { agentId: 'a1', state: 1, reported: true } }));
    const t = tiles(v);
    expect(t.map((x) => x.id).sort()).toEqual(['laptop/a1', 'other/a1']); // same local id, two machines
    const a = t.find((x) => x.id === 'laptop/a1');
    expect(a).toMatchObject({ machine: 'Laptop', workspace: 'login-fix', state: 'working', reportedBy: 'agent' });
    expect(machinesOnline(v)).toBe(1);
  });

  it('names the workspace by project folder and hides machine-made branches', () => {
    const v = emptyView();
    applyEvent(v, 1, ev('n', 1, { checkoutDiscovered: { checkoutId: 'c', nodeId: 'n', projectId: 'p', path: '/home/me/shop' } }));
    applyEvent(v, 2, ev('n', 2, { worktreeCreated: { worktreeId: 'w', checkoutId: 'c', path: '/x/agent-1', branch: 'agent/shell-1' } }));
    applyEvent(v, 3, ev('n', 3, { worktreeCreated: { worktreeId: 'w2', checkoutId: 'c', path: '/x/fix', branch: 'login-fix' } }));
    applyEvent(v, 4, ev('n', 4, { agentStarted: { agentId: 'a', nodeId: 'n', worktreeId: 'w', kind: 'shell' } }));
    applyEvent(v, 5, ev('n', 5, { agentStarted: { agentId: 'b', nodeId: 'n', worktreeId: 'w2', kind: 'shell' } }));
    const t = tiles(v);
    expect(t.find((x) => x.id === 'n/a')?.workspace).toBe('shop');
    expect(t.find((x) => x.id === 'n/b')?.workspace).toBe('shop (login-fix)');
  });

  it('lists recent messages, newest first, and marks the ones that cross machines', () => {
    const v = emptyView();
    applyEvent(v, 1, ev('a', 1, { agentStarted: { agentId: 'x', nodeId: 'a', worktreeId: 'w', kind: 'claude' } }));
    applyEvent(v, 2, ev('a', 2, { messagePosted: { messageId: 'm1', fromAgentId: 'x', to: 'y', body: 'local' } }));
    applyEvent(v, 3, ev('a', 3, { messagePosted: { messageId: 'm2', fromAgentId: 'x', to: 'b/z', body: 'remote' } }));
    const m = recentMessages(v);
    expect(m.map((x) => x.body)).toEqual(['remote', 'local']);
    expect(m.map((x) => x.crossMachine)).toEqual([true, false]);
    expect(m[1]?.from).toBe('claude · ' + m[1]?.from.split(' · ')[1]);
  });

  it('adds up estimated cost per project and skips agents without a price', () => {
    const v = emptyView();
    applyEvent(v, 1, ev('n', 1, { checkoutDiscovered: { checkoutId: 'c', nodeId: 'n', projectId: 'p', path: '/home/me/shop' } }));
    applyEvent(v, 2, ev('n', 2, { worktreeCreated: { worktreeId: 'w', checkoutId: 'c', path: '/x', branch: 'b' } }));
    for (const [i, id] of ['a', 'b', 'c'].entries()) applyEvent(v, 3 + i, ev('n', 3 + i, { agentStarted: { agentId: id, nodeId: 'n', worktreeId: 'w', kind: 'shell' } }));
    expect(costByProject(v, new Map([['n/a', 1_000_000], ['n/b', 500_000]]))).toEqual([{ label: 'shop', microUsd: 1_500_000 }]);
    expect(costByProject(v, new Map())).toEqual([]);
  });

  it('a waiting approval makes the agent need you, and answering it clears that', () => {
    const v = emptyView();
    applyEvent(v, 1, ev('n', 1, { agentStarted: { agentId: 'a', nodeId: 'n', worktreeId: 'w', kind: 'claude' } }));
    applyEvent(v, 2, ev('n', 2, { approvalRequested: { approvalId: 'p', agentId: 'a', action: 'write' } }));
    expect(tiles(v)[0]?.state).toBe('needs-you');
    applyEvent(v, 3, ev('n', 3, { approvalResolved: { approvalId: 'p' } }));
    expect(tiles(v)[0]?.state).toBe('idle');
  });

  it('ignores events at or before its position, so a replayed tail is harmless', () => {
    const v = emptyView();
    const e = ev('n', 1, { agentStarted: { agentId: 'a', nodeId: 'n', worktreeId: 'w', kind: 'claude' } });
    applyEvent(v, 5, e);
    applyEvent(v, 5, ev('n', 2, { agentExited: { agentId: 'a', exitCode: 0 } }));
    expect(tiles(v)[0]?.state).toBe('idle');
  });

  it('lists machines, online first, and follows their resources', () => {
    const v = emptyView();
    applyEvent(v, 1, ev('b', 1, { nodeOnline: { nodeId: 'b', hostname: 'Build box', os: 'linux' } }));
    applyEvent(v, 2, ev('a', 1, { nodeOnline: { nodeId: 'a', hostname: 'Alpha', os: 'macos' } }));
    applyEvent(v, 3, ev('hub', 1, { nodeOffline: { nodeId: 'a' } }));
    applyEvent(v, 4, ev('b', 2, { nodeResources: { nodeId: 'b', agentsRunning: 3 } }));
    expect(machines(v).map((m) => [m.name, m.online, m.agentsRunning])).toEqual([['Build box', true, 3], ['Alpha', false, 0]]);
  });

  it('adds up reported token usage', () => {
    const v = emptyView();
    applyEvent(v, 1, ev('n', 1, { agentStarted: { agentId: 'a', nodeId: 'n', worktreeId: 'w', kind: 'claude' } }));
    applyEvent(v, 2, ev('n', 2, { agentUsageReported: { agentId: 'a', model: 'm', inputTokens: 100, outputTokens: 20 } }));
    applyEvent(v, 3, ev('n', 3, { agentUsageReported: { agentId: 'a', model: 'm', inputTokens: 5, outputTokens: 5 } }));
    expect(tiles(v)[0]?.tokens).toBe(130);
  });

  it('offers projects only on machines that are online', () => {
    const v = emptyView();
    applyEvent(v, 1, ev('a', 1, { nodeOnline: { nodeId: 'a', hostname: 'Alpha' } }));
    applyEvent(v, 2, ev('a', 2, { checkoutDiscovered: { checkoutId: 'c1', nodeId: 'a', projectId: '4be2b2e8', path: '/home/me/webapp' } }));
    applyEvent(v, 3, ev('b', 1, { nodeOnline: { nodeId: 'b', hostname: 'Beta' } }));
    applyEvent(v, 4, ev('b', 2, { checkoutDiscovered: { checkoutId: 'c1', nodeId: 'b', projectId: 'api', path: '/a' } }));
    applyEvent(v, 5, ev('hub', 1, { nodeOffline: { nodeId: 'b' } }));
    expect(projectChoices(v)).toEqual([{ node: 'a', projectId: '4be2b2e8', label: 'Alpha · webapp' }]);
  });

  it('starts from a snapshot', () => {
    const v = fromSnapshot(v1.Snapshot.create({
      pos: 7,
      nodes: [{ nodeId: 'n', hostname: 'Box', online: true }],
      agents: [{ agentId: 'n/a', nodeId: 'n', kind: 'codex', state: 2, running: true }],
    }));
    expect(v.pos).toBe(7);
    expect(tiles(v)[0]).toMatchObject({ machine: 'Box', state: 'needs-you' });
  });
});

class FakeSocket implements Socket {
  binaryType = '';
  onopen: (() => void) | null = null;
  onmessage: ((e: { data: unknown }) => void) | null = null;
  onclose: (() => void) | null = null;
  sent: Uint8Array[] = [];
  closed = false;
  constructor(readonly url: string, readonly protocols: string[]) {}
  send(d: Uint8Array) { this.sent.push(d); }
  close() { this.closed = true; this.onclose?.(); }
  deliver(kind: number, payload: Uint8Array) {
    const b = encodeFrame({ streamId: STREAM_EVENTS, kind, payload });
    this.onmessage?.({ data: b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength) });
  }
}

describe('live connection', () => {
  it('subscribes with the token as a subprotocol, follows events, and resumes from its position', async () => {
    const sockets: FakeSocket[] = [];
    const states: LinkState[] = [];
    let latest: View | undefined;
    const live = connectLive({
      url: 'ws://hub/', token: 'tok',
      socket: (u, p) => { const s = new FakeSocket(u, p); sockets.push(s); return s; },
      backoffMs: () => 0,
      onState: (s) => states.push(s),
      onView: (v) => { latest = v; },
    });
    const s1 = sockets[0] as FakeSocket;
    expect(s1.protocols).toEqual(['agora.v1', 'agora.bearer.tok']);
    s1.onopen?.();
    const sub = decodeFrame(s1.sent[0] as Uint8Array);
    expect(sub?.kind).toBe(EVENT_KIND.SUBSCRIBE);
    expect(v1.Subscribe.decode(sub?.payload as Uint8Array).afterPos).toBeFalsy(); // no position yet: snapshot first

    s1.deliver(EVENT_KIND.SNAPSHOT, v1.Snapshot.encode({ pos: 3, nodes: [{ nodeId: 'n', hostname: 'Box', online: true }] }).finish());
    s1.deliver(EVENT_KIND.POSITIONED_EVENT, v1.PositionedEvent.encode({
      pos: 4, event: ev('n', 9, { agentStarted: { agentId: 'a', nodeId: 'n', worktreeId: 'w', kind: 'claude' } }),
    }).finish());
    expect(latest?.pos).toBe(4);
    expect(tiles(latest as View)).toHaveLength(1);
    expect(states.at(-1)).toBe('live');

    s1.onclose?.(); // the hub went away
    expect(states.at(-1)).toBe('offline');
    await new Promise((r) => setTimeout(r, 5));
    const s2 = sockets[1] as FakeSocket;
    s2.onopen?.();
    expect(v1.Subscribe.decode(decodeFrame(s2.sent[0] as Uint8Array)?.payload as Uint8Array).afterPos).toBeTruthy();
    live.close();
    expect(s2.closed).toBe(true);
  });

  it('closes on a malformed frame and starts over from a snapshot', async () => {
    const sockets: FakeSocket[] = [];
    connectLive({
      url: 'ws://hub/', token: 't', backoffMs: () => 0,
      socket: (u, p) => { const s = new FakeSocket(u, p); sockets.push(s); return s; },
      onState: () => {}, onView: () => {},
    });
    const s = sockets[0] as FakeSocket;
    s.onmessage?.({ data: new Uint8Array([0xff, 0xff]).buffer });
    expect(s.closed).toBe(true);
  });
});

describe('commands and terminals', () => {
  function setup() {
    const sockets: FakeSocket[] = [];
    const live = connectLive({
      url: 'ws://hub/', token: 't', backoffMs: () => 0,
      socket: (u, p) => { const s = new FakeSocket(u, p); sockets.push(s); return s; },
      onState: () => {}, onView: () => {},
    });
    const s = sockets[0] as FakeSocket;
    s.onopen?.();
    const sentFrames = () => s.sent.map((b) => decodeFrame(b)).filter((f) => f !== undefined);
    return { live, s, sentFrames };
  }
  const answer = (s: FakeSocket, requestId: string, body: Record<string, unknown>) => {
    const b = encodeFrame({ streamId: STREAM_COMMANDS, kind: 1, payload: v1.CommandResponse.encode(v1.CommandResponse.create({ requestId, ...body })).finish() });
    s.onmessage?.({ data: b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength) });
  };

  it('sends a command with the target node and resolves with the answer or rejects with the code', async () => {
    const { live, s, sentFrames } = setup();
    const ok = live.command('laptop', { agentList: {} });
    const req = v1.CommandRequest.decode(sentFrames().at(-1)?.payload as Uint8Array);
    expect(req.targetNode).toBe('laptop');
    answer(s, req.requestId, { ok: { agentList: { agents: [] } } });
    expect((await ok).agentList).toBeTruthy();

    const bad = live.command('laptop', { agentList: {} });
    const req2 = v1.CommandRequest.decode(sentFrames().at(-1)?.payload as Uint8Array);
    answer(s, req2.requestId, { err: { code: 'node.offline' } });
    await expect(bad).rejects.toMatchObject({ code: 'node.offline' });
  });

  it('relays a terminal on its own stream: subscribe, output, input as commands, close', () => {
    const { live, s, sentFrames } = setup();
    const events: string[] = [];
    const session = live.terminal('laptop', 't-agent-1').connect((e) => events.push(e.type));
    const sub = sentFrames().at(-1);
    expect(sub?.streamId).toBe(3);
    const req = v1.CommandRequest.decode(sub?.payload as Uint8Array);
    expect(req.targetNode).toBe('laptop');
    expect(req.terminalSubscribe?.terminalId).toBe('t-agent-1');

    const deliver = (kind: number, payload: Uint8Array) => {
      const b = encodeFrame({ streamId: 3, kind, payload });
      s.onmessage?.({ data: b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength) });
    };
    deliver(1, new TextEncoder().encode('screen'));
    deliver(0, new TextEncoder().encode('out'));
    expect(events).toEqual(['snapshot', 'data']);

    session.sendInput(new TextEncoder().encode('ls\n'));
    const input = v1.CommandRequest.decode(sentFrames().at(-1)?.payload as Uint8Array);
    expect(sentFrames().at(-1)?.streamId).toBe(STREAM_COMMANDS);
    expect(input.terminalInput?.terminalId).toBe('t-agent-1');

    session.close();
    const last = sentFrames().at(-1);
    expect(last).toMatchObject({ streamId: 3, kind: STREAM_CLOSE });
    live.close();
  });
});

describe('refused sessions', () => {
  function run() {
    const sockets: FakeSocket[] = [];
    let rejected = 0;
    connectLive({
      url: 'ws://hub/', token: 'stale', backoffMs: () => 0,
      socket: (u, p) => { const s = new FakeSocket(u, p); sockets.push(s); return s; },
      onState: () => {}, onView: () => {}, onRejected: () => { rejected++; },
    });
    return { sockets, rejected: () => rejected };
  }
  it('reports a session the hub keeps closing without a word, but not an unreachable hub', async () => {
    const down = run();
    (down.sockets[0] as FakeSocket).onclose?.(); // never opened
    await new Promise((r) => setTimeout(r, 5));
    (down.sockets[1] as FakeSocket).onclose?.();
    await new Promise((r) => setTimeout(r, 5));
    expect(down.rejected()).toBe(0);

    const refused = run();
    for (let i = 0; i < 2; i++) {
      const s = refused.sockets[i] as FakeSocket;
      s.onopen?.();
      s.onclose?.();
      await new Promise((r) => setTimeout(r, 5));
    }
    expect(refused.rejected()).toBe(1);
    expect(refused.sockets).toHaveLength(2); // and it stops trying
  });
});
