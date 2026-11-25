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

  it.each([['single keypress', 0], ['keypress followed by a click', 900]])('waits five minutes after the last activity: %s', (_case, gap) => {
    const target = new EventTarget();
    const onIdle = vi.fn();
    const stop = watchIdle(5, onIdle, target);
    try {
      vi.advanceTimersByTime(60_000);
      target.dispatchEvent(new Event('keydown'));
      if (gap) {
        vi.advanceTimersByTime(gap);
        target.dispatchEvent(new Event('pointerdown'));
      }
      vi.advanceTimersByTime(5 * 60_000 - 1);
      expect(onIdle, 'must not lock before five minutes of inactivity').not.toHaveBeenCalled();
      vi.advanceTimersByTime(1);
      expect(onIdle).toHaveBeenCalledTimes(1);
    } finally {
      stop();
    }
  });

  it('locks at the first activity after a sleep longer than the limit, even before the timer fires', () => {
    const target = new EventTarget();
    const onIdle = vi.fn();
    const stop = watchIdle(5, onIdle, target);
    try {
      vi.advanceTimersByTime(60_000);
      // The computer slept two hours: the wall clock moved, the timers did not.
      vi.setSystemTime(Date.now() + 2 * 60 * 60_000);
      target.dispatchEvent(new Event('pointermove'));
      expect(onIdle).toHaveBeenCalledTimes(1);
      vi.advanceTimersByTime(10 * 60_000);
      expect(onIdle, 'locks once, and the stopped watcher stays stopped').toHaveBeenCalledTimes(1);
    } finally {
      stop();
    }
  });

  it('stops after its timer re-armed', () => {
    const target = new EventTarget();
    const onIdle = vi.fn();
    const stop = watchIdle(5, onIdle, target);
    vi.advanceTimersByTime(60_000);
    target.dispatchEvent(new Event('keydown'));
    vi.advanceTimersByTime(4 * 60_000);
    stop();
    vi.advanceTimersByTime(60 * 60_000);
    expect(onIdle).not.toHaveBeenCalled();
    expect(vi.getTimerCount()).toBe(0);
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
