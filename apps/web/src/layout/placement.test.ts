import { describe, expect, it } from 'vitest';
import { placeMenu } from './placement';

const vp = { left: 0, top: 0, right: 1000, bottom: 800 };
const at = (top: number, left = 100) => ({ left, top, right: left + 80, bottom: top + 28 });

describe('placeMenu', () => {
  it('opens below when it fits', () => {
    const p = placeMenu(at(100), { width: 200, height: 300 }, vp);
    expect(p.side).toBe('below');
    expect(p.top).toBe(132);
    expect(p.maxHeight).toBeGreaterThanOrEqual(300);
  });
  it('opens above when the trigger sits at the bottom', () => {
    const p = placeMenu(at(760), { width: 200, height: 300 }, vp);
    expect(p.side).toBe('above');
    expect(p.top + 300).toBeLessThanOrEqual(756);
    expect(p.top).toBeGreaterThanOrEqual(0);
  });
  it('caps the height on the roomier side when neither fits', () => {
    const p = placeMenu(at(300), { width: 200, height: 900 }, vp);
    expect(p.side).toBe('below');
    expect(p.maxHeight).toBe(800 - 328 - 4 - 8);
    const q = placeMenu(at(500), { width: 200, height: 900 }, vp);
    expect(q.side).toBe('above');
    expect(q.top).toBe(8);
    expect(q.maxHeight).toBe(500 - 4 - 8);
  });
  it('stays inside a tiny viewport', () => {
    const tiny = { left: 0, top: 0, right: 200, bottom: 120 };
    const p = placeMenu(at(50, 150), { width: 300, height: 400 }, tiny);
    expect(p.left).toBe(8);
    expect(p.top).toBeGreaterThanOrEqual(8);
    expect(p.maxHeight).toBeGreaterThanOrEqual(0);
  });
  it('uses the visible area above a software keyboard', () => {
    const shown = { left: 0, top: 0, right: 390, bottom: 400 }; // the keyboard covers the rest
    const p = placeMenu(at(360, 20), { width: 200, height: 250 }, shown);
    expect(p.side).toBe('above');
    expect(p.top + 250).toBeLessThanOrEqual(356);
  });
  it('clamps horizontally and honours end alignment and rtl', () => {
    expect(placeMenu(at(100, 950), { width: 200, height: 100 }, vp).left).toBe(792);
    expect(placeMenu(at(100, 300), { width: 200, height: 100 }, vp, 'end').left).toBe(180);
    expect(placeMenu(at(100, 300), { width: 200, height: 100 }, vp, 'start', true).left).toBe(180);
    expect(placeMenu(at(100, -50), { width: 200, height: 100 }, vp).left).toBe(8);
  });
});
