import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { idleMinutes, setIdleMinutes, watchIdle } from './idle';

function store() {
  const m = new Map<string, string>();
  return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) };
}

describe('idle lock', () => {
  beforeEach(() => { vi.useFakeTimers(); });
  afterEach(() => { vi.useRealTimers(); });

  it('is off by default and keeps only the offered choices', () => {
    const s = store();
    expect(idleMinutes(s)).toBe(0);
    setIdleMinutes(10, s);
    expect(idleMinutes(s)).toBe(10);
    s.setItem('agora.idleLock', '7');
    expect(idleMinutes(s)).toBe(0);
  });

  it('locks after the chosen minutes without activity, and activity starts the count again', () => {
    const target = new EventTarget();
    const onIdle = vi.fn();
    watchIdle(5, onIdle, target);
    vi.advanceTimersByTime(4 * 60_000);
    target.dispatchEvent(new Event('keydown'));
    vi.advanceTimersByTime(4 * 60_000);
    expect(onIdle).not.toHaveBeenCalled();
    vi.advanceTimersByTime(60_000 + 1);
    expect(onIdle).toHaveBeenCalledTimes(1);
    // Once locked it stops watching: more activity and time change nothing.
    target.dispatchEvent(new Event('pointerdown'));
    vi.advanceTimersByTime(10 * 60_000);
    expect(onIdle).toHaveBeenCalledTimes(1);
  });

  it('watches nothing when off and stops when asked', () => {
    const target = new EventTarget();
    const onIdle = vi.fn();
    watchIdle(0, onIdle, target);
    const stop = watchIdle(10, onIdle, target);
    stop();
    vi.advanceTimersByTime(60 * 60_000);
    expect(onIdle).not.toHaveBeenCalled();
  });
});
