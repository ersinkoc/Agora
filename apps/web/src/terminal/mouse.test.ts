import { describe, expect, it } from 'vitest';
import { MouseEncodingModes, urxvtReport } from './mouse';

const classic = (b: number, col: number, row: number) => `\x1b[M${String.fromCharCode(32 + b, 33 + col, 33 + row)}`;

describe('the urxvt mouse encoding (mode 1015)', () => {
  it('writes a click as urxvt does, from the classic report or the SGR one', () => {
    // Cell (5,3), 0-based: press button 1, then the release.
    expect(urxvtReport(classic(0, 5, 3))).toBe('\x1b[32;6;4M');
    expect(urxvtReport(classic(3, 5, 3))).toBe('\x1b[35;6;4M');
    expect(urxvtReport('\x1b[<0;6;4M')).toBe('\x1b[32;6;4M');
    expect(urxvtReport('\x1b[<0;6;4m')).toBe('\x1b[35;6;4M');
  });
  it('keeps the wheel, motion and modifiers, and coordinates past the classic form\'s limit', () => {
    expect(urxvtReport(classic(64, 0, 0))).toBe('\x1b[96;1;1M');
    expect(urxvtReport('\x1b[<65;300;80M')).toBe('\x1b[97;300;80M');
    expect(urxvtReport('\x1b[<32;10;2M')).toBe('\x1b[64;10;2M'); // drag with button 1
    expect(urxvtReport('\x1b[<35;10;2M')).toBe('\x1b[67;10;2M'); // motion, no button
    expect(urxvtReport('\x1b[<20;7;7m')).toBe('\x1b[55;7;7M'); // Ctrl+Shift, released: button 3, modifiers kept
    expect(urxvtReport(classic(222, 222, 222))).toBe('\x1b[254;223;223M');
  });
  it('leaves everything that is not a mouse report alone', () => {
    for (const s of ['a', '\x1b[A', '\x1bOA', '\x1b[I', '\x1b[13;2u', '\x1b[?1;2c', '\x1b[<0;6;4', '\x1b[M !', '\x1b[<0;6;4Mx']) expect(urxvtReport(s)).toBeUndefined();
  });
});

describe('the encoding a program asked for', () => {
  const after = (...steps: [number[], boolean][]) => { const m = new MouseEncodingModes(); for (const [p, set] of steps) m.apply(p, set); return m.encoding; };
  it('is the last one set, as in xterm', () => {
    expect(after()).toBe('classic');
    expect(after([[1000, 1015], true])).toBe('urxvt');
    expect(after([[1006], true], [[1015], true])).toBe('urxvt');
    expect(after([[1015], true], [[1006], true])).toBe('sgr');
    expect(after([[1016], true])).toBe('sgr-pixels');
    expect(after([[1005], true])).toBe('utf8');
  });
  it('is ended only by a reset of that same encoding', () => {
    expect(after([[1015], true], [[1015], false])).toBe('classic');
    expect(after([[1006], true], [[1015], true], [[1006], false])).toBe('urxvt');
    expect(after([[1015], true], [[1000, 1002], false])).toBe('urxvt');
    expect(after([[1015], true], [[1006], true], [[1006], false])).toBe('classic');
  });
  it('goes back to the classic form on a full reset, and ignores sub-parameters', () => {
    const m = new MouseEncodingModes();
    m.apply([1015], true);
    m.reset();
    expect(m.encoding).toBe('classic');
    m.apply([[1015, 1]], true);
    expect(m.encoding).toBe('classic');
  });
});
