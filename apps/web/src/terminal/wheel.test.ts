import { describe, expect, it } from 'vitest';
import { NotchDetector, Wheel, type WheelAction, type WheelEventLike, type WheelState } from './wheel';

const state = (o: Partial<WheelState> = {}): WheelState => ({ altScreen: false, mouse: false, rows: 24, cellHeight: 18, ...o });
// A notch as Playwright's wheel(0, 100) gives it in Chromium; `pad` a trackpad step as Firefox and WebKit report one (the legacy field
// three times the pixels). Without a time stamp, every event follows a pause.
const notch = (dir: 1 | -1, o: Partial<WheelEventLike> = {}): WheelEventLike => ({ deltaY: 100 * dir, deltaMode: 0, wheelDeltaY: -120 * dir, shiftKey: false, ctrlKey: false, altKey: false, ...o });
const pad = (dy: number, o: Partial<WheelEventLike> = {}): WheelEventLike => ({ deltaY: dy, deltaMode: 0, wheelDeltaY: -dy * 3, shiftKey: false, ctrlKey: false, altKey: false, ...o });
const total = (as: WheelAction[]) => as.reduce((n, a) => n + (a.kind === 'scroll' ? a.lines : a.kind === 'arrows' ? (a.up ? -a.count : a.count) : 0), 0);
// A gesture: these steps 16 ms apart, starting at `t0`.
const gesture = (steps: number[], t0 = 1000, make = pad) => steps.map((dy, i) => make(dy, { timeStamp: t0 + i * 16 }));

describe('the wheel in a terminal', () => {
  it('scrolls the history three lines per notch', () => {
    const w = new Wheel();
    expect(w.decide(notch(-1), state())).toEqual({ kind: 'scroll', lines: -3 });
    expect(w.decide(notch(1), state())).toEqual({ kind: 'scroll', lines: 3 });
  });

  it('leaves a trackpad over the main screen to the terminal own scroll bar, which scrolls it by its pixels', () => {
    const w = new Wheel();
    for (const e of gesture([3, 8, 20, 60, 110, 70, 30, 9, 2])) expect(w.decide(e, state())).toEqual({ kind: 'pass' });
  });

  it('moves a trackpad on the alternate screen by the lines its swipe covers and keeps the remainder', () => {
    const w = new Wheel();
    // 18 px rows: ten steps of 5 px are 50 px, two lines and 14 px left over.
    expect(total(gesture(Array(10).fill(-5)).map((e) => w.decide(e, state({ altScreen: true }))))).toBe(-2);
    expect(w.decide(pad(-5, { timeStamp: 1160 }), state({ altScreen: true }))).toEqual({ kind: 'arrows', count: 1, up: true });
  });

  it('keeps a fast swipe in pixels: big steps inside a trackpad gesture, and its momentum, are not notches', () => {
    // A fast swipe in Firefox or WebKit: small steps, then steps of 40 px and more, then momentum. One line per 18 px, in every step.
    const steps = [4, 12, 30, 64, 118, 140, 96, 70, 51, 40, 22, 9, 3];
    const w = new Wheel();
    expect(total(gesture(steps).map((e) => w.decide(e, state({ altScreen: true }))))).toBe(Math.trunc(steps.reduce((a, b) => a + b) / 18));
    const d = new NotchDetector();
    expect(gesture(steps).map((e) => d.classify(e))).toEqual(steps.map(() => 0));
  });

  it('counts a wheel spun fast as notches of the size its first step had, in every engine', () => {
    const d = new NotchDetector();
    // Firefox or WebKit, a 57 px notch: five steps 16 ms apart, one of them two notches joined.
    expect(gesture([57, 57, 114, 57, 57]).map((e) => d.classify(e))).toEqual([1, 1, 2, 1, 1]);
    // A step that is not a whole number of notches ends the run: a trackpad from there on.
    expect(gesture([57, 57, 80, 57], 5000).map((e) => d.classify(e))).toEqual([1, 1, 0, 0]);
    // After a pause it may be a notch again.
    expect(d.classify(pad(57, { timeStamp: 6000 }))).toBe(1);
    // Chromium: its legacy field counts whatever the pixels (a 53 px notch, two joined).
    expect(gesture([53, 106], 7000, (dy, o) => notch(1, { deltaY: dy, wheelDeltaY: -120 * Math.round(dy / 53), ...o })).map((e) => d.classify(e))).toEqual([1, 2]);
  });

  it('is pixels when the gesture started small, also if a later step is a multiple of 120 in the legacy field', () => {
    const d = new NotchDetector();
    expect(gesture([10, 100], 1000, (dy, o) => pad(dy, { wheelDeltaY: -120 * Math.round(dy / 100), ...o })).map((e) => d.classify(e))).toEqual([0, 0]);
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
    const kinds = gesture(Array(6).fill(-6)).map((e) => w.decide(e, state({ mouse: true })).kind);
    // 6 px events, 18 px rows: every third event completes a line.
    expect(kinds).toEqual(['ignore', 'ignore', 'pass', 'ignore', 'ignore', 'pass']);
  });

  it('scrolls the window own history with Shift even when the program has the mouse', () => {
    expect(new Wheel().decide(notch(-1, { shiftKey: true }), state({ mouse: true }))).toEqual({ kind: 'scroll', lines: -3 });
    // The alternate screen has no history: Shift+wheel does nothing there.
    expect(new Wheel().decide(notch(-1, { shiftKey: true }), state({ mouse: true, altScreen: true }))).toEqual({ kind: 'ignore' });
  });

  it('takes Shift+wheel as browsers send it, sideways', () => {
    const sideways = (dir: 1 | -1, o: Partial<WheelEventLike> = {}) => notch(dir, { deltaY: 0, wheelDeltaY: 0, deltaX: 100 * dir, wheelDeltaX: -120 * dir, shiftKey: true, ...o });
    expect(new Wheel().decide(sideways(-1), state())).toEqual({ kind: 'scroll', lines: -3 });
    expect(new Wheel().decide(sideways(1), state({ mouse: true }))).toEqual({ kind: 'scroll', lines: 3 });
    // Without Shift a sideways swipe is not the terminal's.
    expect(new Wheel().decide(sideways(1, { shiftKey: false }), state())).toEqual({ kind: 'pass' });
  });

  it('leaves Ctrl+wheel and a pinch to the browser (zoom), whatever the program asked for', () => {
    for (const s of [state(), state({ mouse: true }), state({ altScreen: true })]) {
      expect(new Wheel().decide(notch(1, { ctrlKey: true }), s)).toEqual({ kind: 'browser' });
      expect(new Wheel().decide(pad(2.5, { ctrlKey: true }), s)).toEqual({ kind: 'browser' });
    }
  });

  it('turns the wheel into arrow keys on the alternate screen when the program has no mouse (mode 1007)', () => {
    expect(new Wheel().decide(notch(-1), state({ altScreen: true }))).toEqual({ kind: 'arrows', count: 3, up: true });
    expect(new Wheel().decide(notch(1), state({ altScreen: true }))).toEqual({ kind: 'arrows', count: 3, up: false });
    // Never more arrows than rows, so a fast fling cannot flood a program.
    expect(new Wheel().decide({ ...notch(1), deltaMode: 2, deltaY: 5 }, state({ altScreen: true, rows: 10 }))).toEqual({ kind: 'arrows', count: 10, up: false });
  });

  // One notch as each engine reports it (recorded in Chromium, Firefox and WebKit with Playwright 1.63: synthetic, a real device may
  // differ): Chromium counts the legacy field in 120s, Firefox and WebKit make it three times the pixels.
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
    // Notches 30 ms apart stay at three each: nothing is carried over to make a fourth.
    const alt = new Wheel();
    for (let i = 0; i < 4; i++) expect(alt.decide({ ...up, timeStamp: 1000 + 30 * i }, state({ altScreen: true }))).toEqual({ kind: 'arrows', count: 3, up: true });
    expect(alt.decide(down, state({ altScreen: true }))).toEqual({ kind: 'arrows', count: 3, up: false });
    expect(new Wheel().decide(up, state({ mouse: true }))).toEqual({ kind: 'pass' });
  });

  it('leaves an event that moves nothing up or down to the terminal', () => {
    expect(new Wheel().decide(pad(0, { shiftKey: true }), state())).toEqual({ kind: 'pass' });
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
