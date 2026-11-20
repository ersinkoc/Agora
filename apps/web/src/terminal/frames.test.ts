import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { decodeTerminalFrame, inputBytes, snapshotAfterSize, SNAPSHOT_WAIT_MS, type TerminalEvent } from './frames';

const u16x2 = (a: number, b: number) => new Uint8Array(new Uint16Array([a, b]).buffer); // little-endian hosts
const i32 = (v: number) => new Uint8Array(new Int32Array([v]).buffer);

describe('terminal frames', () => {
  it('decodes data and snapshot as bytes', () => {
    expect(decodeTerminalFrame(0, new Uint8Array([104, 105]))).toEqual({ type: 'data', bytes: new Uint8Array([104, 105]) });
    expect(decodeTerminalFrame(1, new Uint8Array([27]))?.type).toBe('snapshot');
  });
  it('decodes resize as cols then rows, little-endian', () => {
    expect(decodeTerminalFrame(2, u16x2(120, 40))).toEqual({ type: 'resize', cols: 120, rows: 40 });
  });
  it('decodes exit codes including signals (negative)', () => {
    expect(decodeTerminalFrame(3, i32(0))).toEqual({ type: 'exit', code: 0 });
    expect(decodeTerminalFrame(3, i32(-9))).toEqual({ type: 'exit', code: -9 });
  });
  it('ignores unknown kinds and malformed payloads instead of throwing', () => {
    expect(decodeTerminalFrame(9, new Uint8Array())).toBeUndefined();
    expect(decodeTerminalFrame(2, new Uint8Array([1, 2, 3]))).toBeUndefined();
    expect(decodeTerminalFrame(3, new Uint8Array())).toBeUndefined();
  });
  it('reads a view onto a larger buffer correctly', () => {
    const big = new Uint8Array([0, 0, 0xff, 0xff, 0xff, 0xff, 0, 0]);
    expect(decodeTerminalFrame(3, big.subarray(2, 6))).toEqual({ type: 'exit', code: -1 });
  });
  it('encodes typed text as UTF-8', () => {
    expect(Array.from(inputBytes('é'))).toEqual([0xc3, 0xa9]);
  });
});

describe('a snapshot is painted after the size that follows it', () => {
  beforeEach(() => { vi.useFakeTimers(); });
  afterEach(() => { vi.useRealTimers(); });
  const snap: TerminalEvent = { type: 'snapshot', bytes: new Uint8Array([1]) };
  const size: TerminalEvent = { type: 'resize', cols: 158, rows: 39 };
  const data: TerminalEvent = { type: 'data', bytes: new Uint8Array([2]) };
  const run = (...events: TerminalEvent[]) => { const out: string[] = []; const g = snapshotAfterSize((e) => out.push(e.type)); events.forEach((e) => g.push(e)); return { out, g }; };
  it('takes the size first and paints the snapshot then', () => {
    expect(run(snap, size).out).toEqual(['resize', 'snapshot']);
  });
  it('keeps output behind the snapshot it follows', () => {
    expect(run(snap, size, data).out).toEqual(['resize', 'snapshot', 'data']);
    expect(run(snap, data).out).toEqual(['snapshot', 'data']);
  });
  it('paints a snapshot that no size follows after a short wait', () => {
    const { out } = run(snap);
    expect(out).toEqual([]);
    vi.advanceTimersByTime(SNAPSHOT_WAIT_MS);
    expect(out).toEqual(['snapshot']);
  });
  it('paints a snapshot before a later one and passes a size on its own through', () => {
    expect(run(snap, snap, size).out).toEqual(['snapshot', 'resize', 'snapshot']);
    expect(run(size, data).out).toEqual(['resize', 'data']);
  });
  it('drops what it holds when disposed', () => {
    const { out, g } = run(snap);
    g.dispose();
    vi.advanceTimersByTime(SNAPSHOT_WAIT_MS * 2);
    expect(out).toEqual([]);
  });
});
