import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { agora } from '../gen/agora.js';
import { encodeFrame, EVENT_KIND, STREAM_EVENTS } from './frame';
import { connectLive, defaultBackoff, RETRY_CAP_MS, type LinkState, type Socket } from './live';

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

// The window's own ping deadline (F1) was taken out of 0.3.8 with the node and hub silence deadlines: see
// docs/backlog/lan-test-findings.md R1. Its tests return with that design.

describe('coming back quickly', () => {
  beforeEach(() => { vi.useFakeTimers(); });
  afterEach(() => { vi.useRealTimers(); });

  it('waits at most five seconds between attempts, with jitter', () => {
    expect(defaultBackoff(0, () => 1)).toBe(500);
    expect(defaultBackoff(0, () => 0)).toBe(400);
    expect(defaultBackoff(2, () => 1)).toBe(2000);
    for (const n of [4, 5, 10, 40, 1000]) {
      expect(defaultBackoff(n, () => 1)).toBe(RETRY_CAP_MS);
      expect(defaultBackoff(n, () => 0)).toBe(RETRY_CAP_MS * 0.8);
    }
    const seen = new Set(Array.from({ length: 50 }, () => defaultBackoff(6)));
    expect(seen.size).toBeGreaterThan(1);
    for (const ms of seen) expect(ms).toBeLessThanOrEqual(RETRY_CAP_MS);
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

  it('on a live link, waking up does nothing', () => {
    const t = setup();
    t.goLive();
    t.poke();
    expect(t.sockets).toHaveLength(1);
    expect(t.states.at(-1)).toBe('live');
    t.live.close();
  });

  it('stops listening for those events when closed', () => {
    const t = setup();
    t.live.close();
    t.poke();
    expect(t.sockets).toHaveLength(1);
  });
});
