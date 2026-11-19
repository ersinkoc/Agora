import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { agora } from '../gen/agora.js';
import { connectLive, type Socket } from './live';
import { encodeFrame, EVENT_KIND, STREAM_COMMANDS, STREAM_EVENTS } from './frame';

class DeferredSocket implements Socket {
  binaryType = '';
  onopen: (() => void) | null = null;
  onmessage: ((e: { data: unknown }) => void) | null = null;
  onclose: (() => void) | null = null;
  sent: Uint8Array[] = [];
  send(data: Uint8Array) { this.sent.push(data); }
  close() {} // The close event is released explicitly by each test.
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
});
