import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { agora } from '../gen/agora.js';
import { connectLive, type Socket } from './live';
import { decodeFrame, encodeFrame, EVENT_KIND, STREAM_CLOSE, STREAM_COMMANDS, STREAM_EVENTS } from './frame';

class DeferredSocket implements Socket {
  binaryType = '';
  onopen: (() => void) | null = null;
  onmessage: ((e: { data: unknown }) => void) | null = null;
  onclose: (() => void) | null = null;
  sent: Uint8Array[] = [];
  closes = 0;
  send(data: Uint8Array) { this.sent.push(data); }
  close() { this.closes++; } // The close event is released explicitly by each test.
}

function setup(onRejected?: () => void) {
  const sockets: DeferredSocket[] = [];
  const seen: number[] = [];
  const states: string[] = [];
  const live = connectLive({
    url: 'ws://hub/', token: 't', wake: () => () => {},
    socket: () => { const s = new DeferredSocket(); sockets.push(s); return s; },
    onState: s => states.push(s), onView: v => seen.push(v.pos),
    onRejected,
  });
  return { live, sockets, seen, states };
}

function snapshot(pos: number) {
  return { data: encodeFrame({ streamId: STREAM_EVENTS, kind: EVENT_KIND.SNAPSHOT,
    payload: agora.v1.Snapshot.encode({ pos }).finish() }) };
}

describe('live connection lifecycle', () => {
  beforeEach(() => { vi.useFakeTimers(); });
  afterEach(() => { vi.useRealTimers(); });

  it('drops a snapshot queued before close and released after close', () => {
    const t = setup();
    const s = t.sockets[0]!;
    s.onopen!();
    const deliver = s.onmessage!;
    deliver(snapshot(1));
    expect(t.seen).toEqual([1]);
    t.live.close();
    deliver(snapshot(2));
    expect(t.seen).toEqual([1]);
    expect(t.states.at(-1)).toBe('live');
  });

  it('drops old callbacks released after a replacement has become live', () => {
    const t = setup();
    const old = t.sockets[0]!;
    const oldOpen = old.onopen!;
    const oldMessage = old.onmessage!;
    oldOpen();
    old.onclose!();
    t.live.retry();
    const current = t.sockets[1]!;
    current.onopen!();
    current.onmessage!(snapshot(9));
    const sent = old.sent.length;
    oldOpen();
    oldMessage(snapshot(2));
    expect(old.sent).toHaveLength(sent);
    expect(t.seen).toEqual([9]);
    current.onmessage!(snapshot(10));
    expect(t.seen).toEqual([9, 10]);
    t.live.close();
  });

  it('drops an opening callback released after close, including repeated close', () => {
    const t = setup();
    const s = t.sockets[0]!;
    const open = s.onopen!;
    t.live.close();
    t.live.close();
    open();
    s.onmessage!(snapshot(1));
    s.onclose!();
    t.live.retry();
    expect(s.sent).toEqual([]);
    expect(t.seen).toEqual([]);
    expect(t.sockets).toHaveLength(1);
  });

  it('settles pending commands at close before the close event arrives', async () => {
    const t = setup();
    const s = t.sockets[0]!;
    s.onopen!();
    const control = t.live.command('n', { agentList: {} });
    s.onmessage!({ data: encodeFrame({ streamId: STREAM_COMMANDS, kind: 1,
      payload: agora.v1.CommandResponse.encode({ requestId: 'c1', ok: {} }).finish() }) });
    expect(await control).toBeDefined();
    const waiting = t.live.command('n', { agentList: {} });
    const settled = expect(waiting).rejects.toMatchObject({ code: 'node.offline' });
    t.live.close();
    await settled;
    await expect(t.live.command('n', { agentList: {} })).rejects.toMatchObject({ code: 'node.offline' });
    s.onclose!();
    expect(t.states).toEqual(['connecting']);
  });

  it('closing without an open connection rejects new commands and never reconnects', async () => {
    const t = setup();
    t.live.close();
    await expect(t.live.command('n', { agentList: {} })).rejects.toMatchObject({ code: 'node.offline' });
    t.sockets[0]!.onclose!();
    vi.runAllTimers();
    expect(t.sockets).toHaveLength(1);
  });

  it('settles every pending command when the second silent refusal rejects the session', async () => {
    let rejections = 0;
    const t = setup(() => { rejections++; t.live.close(); });
    t.sockets[0]!.onopen!();
    const control = t.live.command('n', { agentList: {} }).catch(e => e.code);
    t.sockets[0]!.onclose!();
    expect(await control).toBe('node.offline');
    expect(rejections).toBe(0);
    t.live.retry();
    const s = t.sockets[1]!;
    s.onopen!();
    let settled = 0;
    const pending = Array.from({ length: 3 }, () => t.live.command('n', { agentList: {} })
      .catch(e => { settled++; return e.code; }));
    s.onclose!(); // release rejection only after the commands exist
    await Promise.resolve();
    expect(rejections).toBe(1);
    expect(settled).toBe(3);
    expect(await Promise.all(pending)).toEqual(['node.offline', 'node.offline', 'node.offline']);
    await expect(t.live.command('n', { agentList: {} })).rejects.toMatchObject({ code: 'node.offline' });
    s.onclose!();
    t.live.retry();
    vi.runAllTimers();
    expect(t.sockets).toHaveLength(2);
    expect(rejections).toBe(1);
  });

  it('rejects a silent second connection even with no pending command', () => {
    const rejected = vi.fn();
    const t = setup(rejected);
    t.sockets[0]!.onopen!();
    t.sockets[0]!.onclose!();
    t.live.retry();
    t.sockets[1]!.onopen!();
    t.sockets[1]!.onclose!();
    expect(rejected).toHaveBeenCalledTimes(1);
    t.live.close();
  });

  it('a response on the replacement socket resets the consecutive refusal count', async () => {
    const rejected = vi.fn();
    const t = setup(rejected);
    t.sockets[0]!.onopen!();
    t.sockets[0]!.onclose!();
    t.live.retry();
    const s = t.sockets[1]!;
    s.onopen!();
    s.onmessage!(snapshot(1));
    const pending = t.live.command('n', { agentList: {} }).catch(e => e.code);
    s.onclose!();
    expect(await pending).toBe('node.offline');
    expect(rejected).not.toHaveBeenCalled();
    expect(t.states.at(-1)).toBe('offline');
    t.live.close();
  });

  it('decodes a received byte view without its backing buffer prefix or suffix', () => {
    for (const [offset, tail] of [[0, 19], [7, 0], [7, 19]] as const) {
      const t = setup();
      const s = t.sockets[0]!;
      s.onopen!();
      const packet = snapshot(5).data;
      const backing = new Uint8Array(offset + packet.length + tail).fill(0xff);
      backing.set(packet, offset);
      s.onmessage!({ data: backing.subarray(offset, offset + packet.length) });
      expect(t.seen).toEqual([5]);
      t.live.close();
    }
  });

  it('uses DataView byte boundaries and refuses a truncated view of a complete backing frame', () => {
    const t = setup();
    const s = t.sockets[0]!;
    s.onopen!();
    const packet = snapshot(6).data;
    const backing = new Uint8Array(packet.length + 23).fill(0xff);
    backing.set(packet, 9);
    s.onmessage!({ data: new DataView(backing.buffer, 9, packet.length) });
    expect(t.seen).toEqual([6]);
    // The full backing buffer is valid, but the view supplied by the transport is incomplete.
    s.onmessage!({ data: packet.subarray(0, packet.length - 1) });
    expect(t.seen).toEqual([6]);
    t.live.close();
  });

  it('closes the connection when the WebSocket delivers a text message', () => {
    const rejected = vi.fn();
    const t = setup(rejected);
    const s = t.sockets[0]!;
    s.onopen!();
    s.onmessage!({ data: 'not a binary frame' });
    expect(s.closes).toBe(1);
    s.onclose!();
    t.live.retry();
    const next = t.sockets[1]!;
    next.onopen!();
    next.onmessage!({ data: 'still not binary' });
    next.onclose!();
    expect(rejected).not.toHaveBeenCalled();
    t.live.close();
  });
});

describe('live stream ownership', () => {
  beforeEach(() => { vi.useFakeTimers(); });
  afterEach(() => { vi.useRealTimers(); });

  const subscriptions = (s: DeferredSocket) => s.sent.map(b => decodeFrame(b)!).filter(f => f.streamId >= 3 && f.kind === 0)
    .map(f => agora.v1.CommandRequest.decode(f.payload).requestId);
  const refuse = (s: DeferredSocket, requestId: string) => s.onmessage!({ data: encodeFrame({ streamId: STREAM_COMMANDS, kind: 1,
    payload: agora.v1.CommandResponse.encode({ requestId, err: { code: 'node.offline' } }).finish() }) });
  const ended = (s: DeferredSocket, stream: number) => s.onmessage!({ data: encodeFrame({ streamId: stream, kind: STREAM_CLOSE, payload: new Uint8Array() }) });
  const flush = async () => { for (let i = 0; i < 5; i++) await Promise.resolve(); };

  // F1200: the node leaves while a subscription is in flight; the hub ends the relay it opened and refuses the subscription.
  for (const kind of ['terminal', 'chat'] as const) {
    it(`keeps one resubscription for a ${kind} whose relay ended while its subscription was refused`, async () => {
      const t = setup();
      const s = t.sockets[0]!;
      s.onopen!();
      if (kind === 'terminal') t.live.terminal('n', 'x').connect(() => {});
      else t.live.chat('n', 'x').connect(() => {});
      const answered = new Set<string>();
      const open = () => subscriptions(s).filter(id => !answered.has(id));
      const first = open()[0]!;
      ended(s, 3);
      answered.add(first);
      refuse(s, first);
      await flush();
      await vi.advanceTimersByTimeAsync(2000);
      expect(open()).toHaveLength(1);
      // The node stays away: one retry every two seconds, not two.
      for (let i = 0; i < 20; i++) {
        for (const id of open()) { answered.add(id); refuse(s, id); }
        await flush();
        await vi.advanceTimersByTimeAsync(500);
      }
      expect(subscriptions(s)).toHaveLength(7);
      t.live.close();
    });
  }

  // Codex review of #792: the hub may refuse the subscription before it ends the relay; the close's earlier retry still wins.
  for (const kind of ['terminal', 'chat'] as const) {
    it(`retries a ${kind} after 500 ms when its subscription was refused before its relay ended`, async () => {
      const t = setup();
      const s = t.sockets[0]!;
      s.onopen!();
      if (kind === 'terminal') t.live.terminal('n', 'x').connect(() => {});
      else t.live.chat('n', 'x').connect(() => {});
      refuse(s, subscriptions(s)[0]!);
      await flush();
      ended(s, 3);
      await vi.advanceTimersByTimeAsync(499);
      expect(subscriptions(s)).toHaveLength(1);
      await vi.advanceTimersByTimeAsync(1);
      expect(subscriptions(s)).toHaveLength(2);
      // Still one retry: the refusal's later one was replaced, not added.
      await vi.advanceTimersByTimeAsync(5000);
      expect(subscriptions(s)).toHaveLength(2);
      t.live.close();
    });
  }

  // Codex review of #792: retry deadlines are compared on a monotonic clock; a wall clock that steps back changes nothing.
  it('keeps the earlier retry when the wall clock steps back between the two requests', async () => {
    const t = setup();
    const s = t.sockets[0]!;
    s.onopen!();
    t.live.terminal('n', 'x').connect(() => {});
    ended(s, 3);
    vi.setSystemTime(Date.now() - 10_000);
    refuse(s, subscriptions(s)[0]!);
    await flush();
    await vi.advanceTimersByTimeAsync(500);
    expect(subscriptions(s)).toHaveLength(2);
    await vi.advanceTimersByTimeAsync(5000);
    expect(subscriptions(s)).toHaveLength(2);
    t.live.close();
  });

  it('fake timers drive the monotonic clock the retries use', () => {
    const before = performance.now();
    vi.advanceTimersByTime(1234);
    expect(performance.now() - before).toBe(1234);
  });

  // Codex review of #792: a closed pane's pending retry is cancelled, so it neither fires nor keeps the pane's callbacks alive.
  for (const kind of ['terminal', 'chat'] as const) {
    it(`closing a ${kind} cancels its pending retry`, async () => {
      const t = setup();
      const s = t.sockets[0]!;
      s.onopen!();
      const session = kind === 'terminal' ? t.live.terminal('n', 'x').connect(() => {}) : t.live.chat('n', 'x').connect(() => {});
      ended(s, 3);
      const waiting = vi.getTimerCount();
      session.close();
      expect(vi.getTimerCount()).toBe(waiting - 1);
      await vi.advanceTimersByTimeAsync(5000);
      expect(subscriptions(s)).toHaveLength(1);
      t.live.close();
    });
  }

  it('drops a retry due on a dropped connection and still retries on the new one', async () => {
    const t = setup();
    const old = t.sockets[0]!;
    old.onopen!();
    t.live.terminal('n', 'x').connect(() => {});
    old.onmessage!({ data: encodeFrame({ streamId: STREAM_COMMANDS, kind: 1,
      payload: agora.v1.CommandResponse.encode({ requestId: subscriptions(old)[0]!, ok: {} }).finish() }) });
    ended(old, 3);
    old.onclose!();
    t.live.retry();
    const fresh = t.sockets[1]!;
    fresh.onopen!();
    await vi.advanceTimersByTimeAsync(5000);
    expect(subscriptions(fresh)).toHaveLength(1);
    ended(fresh, 3);
    await vi.advanceTimersByTimeAsync(500);
    expect(subscriptions(fresh)).toHaveLength(2);
    t.live.close();
  });

  // F1201: a pane that went away must not resize or type into the terminal afterwards.
  it('sends nothing for a terminal session after it was closed', () => {
    const t = setup();
    const s = t.sockets[0]!;
    s.onopen!();
    // The connection says the role first (ADR-0106; here an older hub's snapshot): only then does a terminal get sizes and typing.
    s.onmessage!(snapshot(1));
    const session = t.live.terminal('n', 'x').connect(() => {});
    const commands = () => s.sent.map(b => decodeFrame(b)!).filter(f => f.streamId === STREAM_COMMANDS && f.kind === 0).length;
    const closes = () => s.sent.map(b => decodeFrame(b)!).filter(f => f.kind === STREAM_CLOSE).length;
    session.resize(80, 24);
    session.sendInput(new Uint8Array([0x61]));
    const before = commands();
    expect(before).toBe(2);
    session.close();
    session.close();
    session.resize(20, 5);
    session.sendInput(new Uint8Array([0x0d]));
    expect(commands()).toBe(before);
    expect(closes()).toBe(1);
    t.live.close();
  });
});
