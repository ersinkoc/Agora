import { describe, expect, it } from 'vitest';
import { notches, Wheel, type WheelEventLike, type WheelState } from './wheel';

const state = (o: Partial<WheelState> = {}): WheelState => ({ altScreen: false, mouse: false, rows: 24, cellHeight: 18, ...o });
const notch = (dir: 1 | -1, o: Partial<WheelEventLike> = {}): WheelEventLike => ({ deltaY: 100 * dir, deltaMode: 0, wheelDeltaY: -120 * dir, shiftKey: false, ctrlKey: false, altKey: false, ...o });
const pad = (dy: number, o: Partial<WheelEventLike> = {}): WheelEventLike => ({ deltaY: dy, deltaMode: 0, wheelDeltaY: -dy * 3, shiftKey: false, ctrlKey: false, altKey: false, ...o });

describe('the wheel in a terminal', () => {
  it('scrolls the history three lines per notch', () => {
    const w = new Wheel();
    expect(w.decide(notch(-1), state())).toEqual({ kind: 'scroll', lines: -3 });
    expect(w.decide(notch(1), state())).toEqual({ kind: 'scroll', lines: 3 });
  });

  it('moves a trackpad by the lines its swipe covers and keeps the remainder', () => {
    const w = new Wheel();
    // 18 px rows: ten events of 5 px are 50 px, two lines and 14 px left over.
    const lines = Array.from({ length: 10 }, () => w.decide(pad(-5), state())).reduce((n, a) => n + (a.kind === 'scroll' ? a.lines : 0), 0);
    expect(lines).toBe(-2);
    expect(w.decide(pad(-5), state())).toEqual({ kind: 'scroll', lines: -1 });
  });

  it('counts a line-mode wheel (Firefox) and a page-mode one', () => {
    expect(new Wheel().decide({ ...notch(1), deltaMode: 1, deltaY: 3 }, state())).toEqual({ kind: 'scroll', lines: 3 });
    expect(new Wheel().decide({ ...notch(1), deltaMode: 2, deltaY: 1 }, state({ rows: 30 }))).toEqual({ kind: 'scroll', lines: 30 });
  });

  it('gives the wheel to a program that asked for mouse reports', () => {
    expect(new Wheel().decide(notch(-1), state({ mouse: true }))).toEqual({ kind: 'pass' });
    expect(new Wheel().decide(notch(-1), state({ mouse: true, altScreen: true }))).toEqual({ kind: 'pass' });
  });

  it('does not flood a program with a trackpad swipe: one report per line covered', () => {
    const w = new Wheel();
    const kinds = Array.from({ length: 6 }, () => w.decide(pad(-6), state({ mouse: true })).kind);
    // 6 px events, 18 px rows: every third event completes a line.
    expect(kinds).toEqual(['ignore', 'ignore', 'pass', 'ignore', 'ignore', 'pass']);
  });

  it('scrolls the window own history with Shift even when the program has the mouse', () => {
    expect(new Wheel().decide(notch(-1, { shiftKey: true }), state({ mouse: true }))).toEqual({ kind: 'scroll', lines: -3 });
    // The alternate screen has no history: Shift+wheel does nothing there.
    expect(new Wheel().decide(notch(-1, { shiftKey: true }), state({ mouse: true, altScreen: true }))).toEqual({ kind: 'ignore' });
  });

  it('turns the wheel into arrow keys on the alternate screen when the program has no mouse (mode 1007)', () => {
    expect(new Wheel().decide(notch(-1), state({ altScreen: true }))).toEqual({ kind: 'arrows', count: 3, up: true });
    expect(new Wheel().decide(notch(1), state({ altScreen: true }))).toEqual({ kind: 'arrows', count: 3, up: false });
    // Never more arrows than rows, so a fast fling cannot flood a program.
    expect(new Wheel().decide({ ...notch(1), deltaMode: 2, deltaY: 5 }, state({ altScreen: true, rows: 10 }))).toEqual({ kind: 'arrows', count: 10, up: false });
  });

  // One notch as each engine reports it (recorded in Chromium, Firefox and WebKit with Playwright 1.63): Chromium counts the legacy
  // field in 120s, Firefox and WebKit make it three times the pixels.
  const engines: [string, WheelEventLike][] = [
    ['Chromium', notch(1)],
    ['Firefox', notch(1, { wheelDeltaY: -300 })],
    ['WebKit', notch(1, { wheelDeltaY: -300 })],
    ['Firefox, line mode', notch(1, { deltaMode: 1, deltaY: 3, wheelDeltaY: -9 })],
  ];
  it.each(engines)('counts one %s notch as three lines, three arrows and one mouse report', (_name, down) => {
    const up = { ...down, deltaY: -down.deltaY, wheelDeltaY: -(down.wheelDeltaY ?? 0) };
    const w = new Wheel();
    expect(w.decide(up, state())).toEqual({ kind: 'scroll', lines: -3 });
    expect(w.decide(down, state())).toEqual({ kind: 'scroll', lines: 3 });
    // Repeated notches stay at three each: nothing is carried over to make a fourth.
    const alt = new Wheel();
    for (let i = 0; i < 4; i++) expect(alt.decide(up, state({ altScreen: true }))).toEqual({ kind: 'arrows', count: 3, up: true });
    expect(alt.decide(down, state({ altScreen: true }))).toEqual({ kind: 'arrows', count: 3, up: false });
    expect(new Wheel().decide(up, state({ mouse: true }))).toEqual({ kind: 'pass' });
  });

  it('counts notches the engine joined into one event', () => {
    expect(notches(notch(1, { deltaY: 200, wheelDeltaY: -240 }))).toBe(2); // Chromium
    expect(notches(notch(1, { deltaY: 200, wheelDeltaY: -600 }))).toBe(2); // Firefox, WebKit
    // Chromium's notch is not always 100 px; its legacy field still says one.
    expect(notches(notch(1, { deltaY: 53, wheelDeltaY: -120 }))).toBe(1);
    // A big step whose legacy field is missing counts by its pixels.
    expect(notches(notch(1, { deltaY: 100, wheelDeltaY: undefined }))).toBe(1);
  });

  it('keeps a trackpad in pixels in every engine, also when its legacy field is a multiple of 120', () => {
    expect(notches(pad(5))).toBe(0);
    expect(notches(pad(-30, { wheelDeltaY: 120 }))).toBe(0);
    const w = new Wheel();
    // 18 px rows: six 6 px steps (Firefox and WebKit legacy field: three times the pixels) are two lines.
    const lines = Array.from({ length: 6 }, () => w.decide(pad(6, { wheelDeltaY: -18 }), state())).reduce((n, a) => n + (a.kind === 'scroll' ? a.lines : 0), 0);
    expect(lines).toBe(2);
  });

  it('ignores a wheel event that moves nothing', () => {
    expect(new Wheel().decide(pad(0), state())).toEqual({ kind: 'ignore' });
  });
});

import { TouchScroll } from './wheel';

describe('a finger dragging over the terminal', () => {
  it('moves the history one line per row height, the content following the finger', () => {
    const t = new TouchScroll();
    t.start(300);
    expect(t.move(282, state())).toEqual({ kind: 'scroll', lines: 1 }); // finger up 18 px: newer lines
    expect(t.move(260, state())).toEqual({ kind: 'scroll', lines: 1 });
    expect(t.move(258, state())).toEqual({ kind: 'ignore' }); // 4 px more, plus 4 left over: not a line yet
    expect(t.move(300, state())).toEqual({ kind: 'scroll', lines: -2 }); // finger down: older lines
  });
  it('sends arrows on the alternate screen, and nothing while the program has the mouse', () => {
    const t = new TouchScroll();
    t.start(300);
    expect(t.move(246, state({ altScreen: true }))).toEqual({ kind: 'arrows', count: 3, up: false });
    expect(t.move(300, state({ altScreen: true }))).toEqual({ kind: 'arrows', count: 3, up: true });
    expect(t.move(100, state({ mouse: true }))).toEqual({ kind: 'ignore' });
  });
  it('does nothing before a touch starts or after it ends', () => {
    const t = new TouchScroll();
    expect(t.move(100, state())).toEqual({ kind: 'ignore' });
    t.start(100); t.end();
    expect(t.move(0, state())).toEqual({ kind: 'ignore' });
  });
});
