// The message set (ADR-0114 decision 3 and Verification): the window takes a message only from a frame it made, of a known type and shape,
// within the size limit and the rate; anything else is ignored and counted. A capability not granted answers "not granted".
import { describe, expect, it } from 'vitest';
import { BURST, MAX_MESSAGE_BYTES, RATE, frameFilter, gate, messageRouter, parseInbound, rateLimiter, sizeOf } from './protocol';

const read = (id: number, cap = 'agents.read') => ({ agora: 1, type: 'read', id, cap });

describe('the message set', () => {
  it('knows exactly boot, read, watch and unwatch of version 1, with nothing extra', () => {
    expect(parseInbound({ agora: 1, type: 'boot' })).toEqual({ type: 'boot' });
    expect(parseInbound(read(3))).toEqual({ type: 'read', id: 3, cap: 'agents.read' });
    expect(parseInbound({ agora: 1, type: 'watch', id: 0, cap: 'theme.read' })).toEqual({ type: 'watch', id: 0, cap: 'theme.read' });
    expect(parseInbound({ agora: 1, type: 'unwatch', id: 9 })).toEqual({ type: 'unwatch', id: 9 });
    for (const bad of [
      null, 'read', [read(1)], { type: 'read', id: 1, cap: 'agents.read' }, { ...read(1), agora: 2 }, { ...read(1), extra: true },
      { ...read(1), id: -1 }, { ...read(1), id: 1.5 }, { ...read(1), id: '1' }, { ...read(1), cap: 7 }, { agora: 1, type: 'sign', id: 1 },
      { agora: 1, type: 'answer', id: 1, approval: 'a' }, { agora: 1, type: 'prompt', text: 'rm -rf /' }, { agora: 1, type: 'boot', x: 1 },
    ]) expect(parseInbound(bad), JSON.stringify(bad)).toBeUndefined();
  });

  it('sizes a message as JSON and refuses one that is not plain data', () => {
    expect(sizeOf(read(1))).toBe(JSON.stringify(read(1)).length);
    const cycle: Record<string, unknown> = {};
    cycle.self = cycle;
    expect(sizeOf(cycle)).toBeUndefined();
    expect(sizeOf(() => 1)).toBeUndefined();
  });
});

describe('a frame\'s filter', () => {
  it('ignores and counts an unknown type and an oversized message', () => {
    const f = frameFilter(() => 0);
    expect(f.take({ agora: 1, type: 'open-window', url: 'https://example.com' })).toBeUndefined();
    expect(f.take({ ...read(1), cap: 'x'.repeat(MAX_MESSAGE_BYTES) })).toBeUndefined();
    expect(f.take(read(2))).toEqual({ type: 'read', id: 2, cap: 'agents.read' });
    expect(f.counts).toEqual({ type: 1, size: 1, rate: 0 });
  });

  it('takes at most a burst at once and the rate after it; ignored messages never spend the rate', () => {
    let now = 0;
    const f = frameFilter(() => now);
    for (let i = 0; i < 100; i++) f.take({ agora: 1, type: 'nope' });
    let taken = 0;
    for (let i = 0; i < BURST + 10; i++) if (f.take(read(i))) taken++;
    expect(taken).toBe(BURST);
    expect(f.counts.rate).toBe(10);
    now += 1000;
    taken = 0;
    for (let i = 0; i < RATE * 3; i++) if (f.take(read(i))) taken++;
    expect(taken).toBe(RATE);
  });

  it('refills the bucket over time, never past the burst', () => {
    const allow = rateLimiter(10, 5);
    for (let i = 0; i < 5; i++) expect(allow(0)).toBe(true);
    expect(allow(0)).toBe(false);
    expect(allow(100)).toBe(true);
    expect(allow(100)).toBe(false);
    let n = 0;
    for (let i = 0; i < 20; i++) if (allow(100_000)) n++;
    expect(n).toBe(5);
  });
});

describe('the router', () => {
  it('takes a message only from a frame it knows, by its window, and counts the rest as strays', () => {
    const r = messageRouter();
    const frameA = {};
    const frameB = {};
    const got: unknown[] = [];
    const off = r.add(frameA, frameFilter(() => 0), (m) => got.push(['a', m]));
    r.add(frameB, frameFilter(() => 0), (m) => got.push(['b', m]));
    expect(r.receive({ source: frameA, data: read(1) })).toBe(true);
    expect(r.receive({ source: frameB, data: read(2) })).toBe(true);
    // The window itself, another page, an extension, an unknown frame, no source at all.
    for (const source of [{}, null, undefined, globalThis]) expect(r.receive({ source, data: read(3) })).toBe(false);
    expect(r.strays.count).toBe(4);
    expect(got).toEqual([['a', { type: 'read', id: 1, cap: 'agents.read' }], ['b', { type: 'read', id: 2, cap: 'agents.read' }]]);
    off();
    expect(r.receive({ source: frameA, data: read(4) })).toBe(false);
    expect(r.size).toBe(1);
  });
});

describe('the gate', () => {
  it('answers "not granted" for a capability the person did not allow and "unknown capability" for a name that is none', () => {
    expect(gate('agents.read', ['agents.read'])).toEqual({ ok: true, cap: 'agents.read' });
    expect(gate('usage.read', ['agents.read'])).toEqual({ ok: false, error: 'not-granted' });
    expect(gate('chat.read', [])).toEqual({ ok: false, error: 'not-granted' });
    for (const cap of ['terminal.read', 'approvals.read', 'actions.sign', '__proto__', '']) expect(gate(cap, ['agents.read'])).toEqual({ ok: false, error: 'unknown-capability' });
  });
});
