// @vitest-environment jsdom
// ADR-0114 decision 4: which frame not in use is blamed when the window loses the keyboard to one (PluginFrame's guard).
import { describe, expect, it } from 'vitest';
import { stealer } from './PluginFrame';

describe('the frame that took the keyboard', () => {
  const frame = () => document.createElement('iframe');
  it('is the active element when that is a frame not in use, else the only frame not in use, else nobody', () => {
    const [a, b] = [frame(), frame()];
    expect(stealer(a, new Set([a, b]))).toBe(a);
    expect(stealer(document.body, new Set([a]))).toBe(a);
    expect(stealer(document.body, new Set([a, b])), 'several and none named: nobody is stopped').toBeUndefined();
    expect(stealer(document.body, new Set())).toBeUndefined();
    const inUse = frame();
    expect(stealer(inUse, new Set([a, b])), 'a frame in use is not one of them').toBeUndefined();
  });
});
