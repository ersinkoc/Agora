import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { agora } from '../gen/agora.js';
import { CONTROL_KIND, decodeFrame, encodeFrame, EVENT_KIND, STREAM_CONTROL, STREAM_EVENTS } from './frame';
import { connectLive, defaultBackoff, OPEN_WITHIN_MS, PING_EVERY_MS, PONG_WITHIN_MS, RETRY_CAP_MS, RETRY_FAST_ATTEMPTS, RETRY_SLOW_CAP_MS, type LinkState, type Socket } from './live';

class FakeSocket implements Socket {
  binaryType = '';
  onopen: (() => void) | null = null;
  onmessage: ((e: { data: unknown }) => void) | null = null;
  onclose: (() => void) | null = null;
  sent: Uint8Array[] = [];
  closed = false;
  send(d: Uint8Array) { this.sent.push(d); }
  close() { this.closed = true; this.onclose?.(); }
  frame(streamId: number, kind: number, payload: Uint8Array) {
    const b = encodeFrame({ streamId, kind, payload });
    this.onmessage?.({ data: b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength) });
  }
  pings() { return this.sent.map((b) => decodeFrame(b)).filter((f) => f?.streamId === STREAM_CONTROL && f.kind === CONTROL_KIND.PING).length; }
}

function setup(opts: { backoffMs?: (n: number) => number } = {}) {
  const sockets: FakeSocket[] = [];
  const states: LinkState[] = [];
  let poke: (() => void) | undefined;
  const live = connectLive({
    url: 'ws://hub/', token: 't',
    socket: () => { const s = new FakeSocket(); sockets.push(s); return s; },
    backoffMs: opts.backoffMs ?? (() => 1000),
    wake: (p) => { poke = p; return () => { poke = undefined; }; },
    onState: (s) => states.push(s), onView: () => {},
  });
  const last = () => sockets.at(-1) as FakeSocket;
  // Opens the newest socket and gives it the hub's snapshot, as a healthy hub does.
  const goLive = () => {
    last().onopen?.();
    last().frame(STREAM_EVENTS, EVENT_KIND.SNAPSHOT, agora.v1.Snapshot.encode({ pos: 1 }).finish());
  };
  return { live, sockets, states, last, goLive, poke: () => poke?.() };
}

describe('window to hub liveness', () => {
  beforeEach(() => { vi.useFakeTimers(); });
  afterEach(() => { vi.useRealTimers(); });

  it('a hub that goes silent on an open connection is noticed: Hub unreachable, then a new connection', () => {
    const t = setup();
    t.goLive();
    expect(t.states.at(-1)).toBe('live');
    vi.advanceTimersByTime(PING_EVERY_MS);
    expect(t.last().pings()).toBe(2); // the first when it opened
    // Nothing comes back, but the browser never reports the connection closed.
    vi.advanceTimersByTime(PONG_WITHIN_MS);
    expect(t.states.at(-1)).toBe('offline');
    expect(t.sockets[0]?.closed).toBe(true);
    expect(t.sockets).toHaveLength(1);
    vi.advanceTimersByTime(1000);
    expect(t.sockets).toHaveLength(2);
    expect(t.states.at(-1)).toBe('connecting');
    t.live.close();
  });

  it('a silent connection that is given up never counts as a refused session', () => {
    const t = setup();
    for (let i = 0; i < 3; i++) {
      t.last().onopen?.(); // opens, then nothing at all
      vi.advanceTimersByTime(PING_EVERY_MS + PONG_WITHIN_MS + 1000);
    }
    expect(t.sockets.length).toBeGreaterThanOrEqual(4);
    t.live.close();
  });

  it('an idle hub that answers every ping is never dropped', () => {
    const t = setup();
    t.goLive();
    for (let i = 0; i < 40; i++) {
      vi.advanceTimersByTime(PING_EVERY_MS);
      t.last().frame(STREAM_CONTROL, CONTROL_KIND.PONG, new Uint8Array());
    }
    expect(t.sockets).toHaveLength(1);
    expect(t.states.at(-1)).toBe('live');
    expect(t.last().pings()).toBe(41); // one when it opened

    t.live.close();
  });

  it('a dial that never opens is given up and tried again', () => {
    const t = setup();
    vi.advanceTimersByTime(OPEN_WITHIN_MS);
    expect(t.states.at(-1)).toBe('offline');
    vi.advanceTimersByTime(1000);
    expect(t.sockets).toHaveLength(2);
    t.live.close();
  });
});

describe('a connect attempt that never opens', () => {
  beforeEach(() => { vi.useFakeTimers(); });
  afterEach(() => { vi.useRealTimers(); });

  it('is given up within the deadline even when the browser reports nothing, and the window goes live once the hub answers', () => {
    // A frozen hub: TCP accepted, nothing answered, and the browser never fires an error or a close.
    const t = setup({ backoffMs: () => 1000 });
    expect(t.states.at(-1)).toBe('connecting');
    vi.advanceTimersByTime(OPEN_WITHIN_MS - 1);
    expect(t.states.at(-1)).toBe('connecting');
    vi.advanceTimersByTime(1);
    expect(t.states.at(-1)).toBe('offline'); // "Hub unreachable"
    expect(t.sockets[0]?.closed).toBe(true);
    // Retried after one backoff; still frozen, so given up again: "connecting" never lasts longer than the deadline.
    vi.advanceTimersByTime(1000);
    expect(t.sockets).toHaveLength(2);
    vi.advanceTimersByTime(OPEN_WITHIN_MS);
    expect(t.states.at(-1)).toBe('offline');
    // The hub runs again: the next attempt opens and goes live within one backoff.
    vi.advanceTimersByTime(1000);
    expect(t.sockets).toHaveLength(3);
    t.goLive();
    expect(t.states.at(-1)).toBe('live');
    // An open connection has no open deadline; its liveness is the ping's (ADR-0070): an hour of a hub that only answers pings
    // does not drop it.
    for (let i = 0; i < 240; i++) {
      vi.advanceTimersByTime(PING_EVERY_MS);
      t.last().frame(STREAM_CONTROL, CONTROL_KIND.PONG, new Uint8Array());
    }
    expect(t.sockets).toHaveLength(3);
    expect(t.states.at(-1)).toBe('live');
    t.live.close();
  });
});

describe('coming back quickly', () => {
  beforeEach(() => { vi.useFakeTimers(); });
  afterEach(() => { vi.useRealTimers(); });

  it('waits at most a second and a half between attempts while the hub restarts, with jitter, then up to five seconds', () => {
    expect(RETRY_CAP_MS).toBe(1500);
    expect(defaultBackoff(0, () => 1)).toBe(500);
    expect(defaultBackoff(0, () => 0)).toBe(400);
    expect(defaultBackoff(1, () => 1)).toBe(1000);
    for (const n of [2, 4, 5, 10, 40, RETRY_FAST_ATTEMPTS - 1]) {
      expect(defaultBackoff(n, () => 1)).toBe(RETRY_CAP_MS);
      expect(defaultBackoff(n, () => 0)).toBe(RETRY_CAP_MS * 0.8);
    }
    for (const n of [RETRY_FAST_ATTEMPTS, 1000]) expect(defaultBackoff(n, () => 1)).toBe(RETRY_SLOW_CAP_MS);
    const seen = new Set(Array.from({ length: 50 }, () => defaultBackoff(6)));
    expect(seen.size).toBeGreaterThan(1);
    for (const ms of seen) expect(ms).toBeLessThanOrEqual(RETRY_CAP_MS);
  });

  it('a reconnect the hub has nothing new for goes live when the hub answers its ping, not at the next event', () => {
    const t = setup();
    t.goLive();
    t.last().close(); // the hub restarted
    vi.advanceTimersByTime(1000);
    t.last().onopen?.();
    // It subscribed from its position and pinged at once; nothing happened since, so the hub sends only the PONG.
    expect(t.last().pings()).toBe(1);
    expect(t.states.at(-1)).toBe('connecting');
    t.last().frame(STREAM_CONTROL, CONTROL_KIND.PONG, new Uint8Array());
    expect(t.states.at(-1)).toBe('live');
    t.live.close();
  });

  it('a first connection is not live on a PONG alone: it waits for the snapshot', () => {
    const t = setup();
    t.last().onopen?.();
    t.last().frame(STREAM_CONTROL, CONTROL_KIND.PONG, new Uint8Array());
    expect(t.states.at(-1)).toBe('connecting');
    t.live.close();
  });

  it('the browser coming online, the window becoming visible or getting the focus dials at once', () => {
    const t = setup({ backoffMs: () => 60_000 });
    t.goLive();
    t.last().close(); // the hub went away
    expect(t.states.at(-1)).toBe('offline');
    t.poke(); // for example `online`
    expect(t.sockets).toHaveLength(2);
    expect(t.states.at(-1)).toBe('connecting');
    // A dial under way is left alone: a burst of focus events does not start it over.
    t.poke();
    expect(t.sockets).toHaveLength(2);
    t.live.close();
  });

  it('Retry now dials at once instead of waiting', () => {
    const t = setup({ backoffMs: () => 60_000 });
    t.goLive();
    t.last().close();
    t.live.retry();
    expect(t.sockets).toHaveLength(2);
    // The scheduled attempt does not dial a second time.
    vi.advanceTimersByTime(60_000);
    expect(t.sockets).toHaveLength(2);
    t.live.close();
  });

  it('on a live link, waking up checks it with a ping and notices a silent one at once', () => {
    const t = setup();
    t.goLive();
    t.poke(); // the laptop woke up and the window became visible
    expect(t.last().pings()).toBe(2); // the first when it opened
    vi.advanceTimersByTime(PONG_WITHIN_MS);
    expect(t.states.at(-1)).toBe('offline');
    t.live.close();
  });

  it('stops listening for those events when closed', () => {
    const t = setup();
    t.live.close();
    t.poke();
    expect(t.sockets).toHaveLength(1);
  });
});
