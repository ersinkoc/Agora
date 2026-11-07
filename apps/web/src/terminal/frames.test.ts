import { describe, expect, it } from 'vitest';
import { decodeTerminalFrame, inputBytes } from './frames';

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
