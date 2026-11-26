// The message set (ADR-0114 decision 3 and Verification): the window takes a message only from a frame it made, of a known type and shape,
// within the size limit and the rate; anything else is ignored and counted. A capability not granted answers "not granted".
import { describe, expect, it } from 'vitest';
import { BURST, MAX_CAP, RATE, STOP_AFTER, classify, frameFilter, gate, isPlainObject, messageRouter, parseInbound, rateLimiter } from './protocol';

const read = (id: number, cap = 'agents.read') => ({ agora: 1, type: 'read', id, cap });
const take = (f: ReturnType<typeof frameFilter>, data: unknown) => f.take(() => data);

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

  it('reads only plain objects with at most four fields, without serialising or walking anything else', () => {
    for (const v of [new Uint8Array(16), new ArrayBuffer(8), new Map(), new Date(), [1], 'x', 1, null, undefined]) {
      expect(isPlainObject(v), String(v)).toBe(false);
      expect(classify(v)).toBe('type');
    }
    expect(isPlainObject(Object.create(null))).toBe(true);
    // A huge typed array is refused at once: nothing measures it.
    const big = new Uint8Array(16e6);
    const t0 = performance.now();
    for (let i = 0; i < 4; i++) expect(classify(big)).toBe('type');
    expect(performance.now() - t0).toBeLessThan(50);
    // An object with many fields is refused after the fifth, and its toJSON is never called.
    let called = false;
    const many: Record<string, unknown> = { agora: 1, type: 'read', id: 1, cap: 'agents.read', toJSON: () => { called = true; return {}; } };
    for (let i = 0; i < 1000; i++) many[`k${i}`] = i;
    expect(classify(many)).toBe('type');
    expect(called).toBe(false);
    // A capability name over its bound is too long, not a wrong type.
    expect(classify({ ...read(1), cap: 'x'.repeat(MAX_CAP + 1) })).toBe('size');
    expect(classify({ ...read(1), cap: 'x'.repeat(MAX_CAP) })).toEqual({ type: 'read', id: 1, cap: 'x'.repeat(MAX_CAP) });
  });
});

describe('a frame\'s filter', () => {
  it('ignores and counts an unknown type and an oversized message', () => {
    const f = frameFilter(() => 0);
    expect(take(f, { agora: 1, type: 'open-window', url: 'https://example.com' })).toBeUndefined();
    expect(take(f, { ...read(1), cap: 'x'.repeat(10_000) })).toBeUndefined();
    expect(take(f, new Uint8Array(1_000_000))).toBeUndefined();
    expect(take(f, read(2))).toEqual({ type: 'read', id: 2, cap: 'agents.read' });
    expect(f.counts).toEqual({ type: 2, size: 1, rate: 0 });
  });

  it('says once that a frame floods, after STOP_AFTER of its messages are ignored for any reason', () => {
    let floods = 0;
    const f = frameFilter(() => 0, () => { floods++; });
    for (let i = 0; i < STOP_AFTER - 1; i++) take(f, i % 2 ? new Uint8Array(8) : { agora: 1, type: 'nope' });
    expect([floods, f.flooding]).toEqual([0, false]);
    for (let i = 0; i < BURST * 2; i++) take(f, read(i));
    expect([floods, f.flooding]).toEqual([1, true]);
  });

  it('reads a message\'s data only once the rate allows it', () => {
    const f = frameFilter(() => 0);
    let reads = 0;
    for (let i = 0; i < BURST + 25; i++) f.take(() => { reads++; return read(i); });
    expect(reads).toBe(BURST);
    expect(f.counts.rate).toBe(25);
  });

  it('spends the rate before anything else, so a flood of any kind costs one token each and is cut at the burst', () => {
    let now = 0;
    const f = frameFilter(() => now);
    let taken = 0;
    for (let i = 0; i < BURST + 10; i++) if (take(f, i % 2 ? read(i) : new Uint8Array(1_000_000))) taken++;
    expect(taken).toBe(BURST / 2);
    expect(f.counts.rate).toBe(10);
    expect(f.counts.type).toBe(BURST / 2);
    now += 1000;
    taken = 0;
    for (let i = 0; i < RATE * 3; i++) if (take(f, read(i))) taken++;
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
