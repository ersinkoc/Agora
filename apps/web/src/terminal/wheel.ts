// What a turn of the mouse wheel (or a two-finger swipe) does in a terminal, decided as a real terminal does. Pure: the renderer passes
// what the program asked for and the event, and does what this says.
//
//   Ctrl held (or a trackpad pinch)   the browser's own zoom; nothing reaches the terminal
//   mouse reporting on, plain wheel   the program gets the wheel as mouse buttons 4 and 5 (xterm sends them)
//   mouse reporting on, Shift held    the window scrolls its own history (Shift is the "ignore the program's mouse" key)
//   alternate screen, no mouse        arrow keys (xterm's alternateScroll, DEC mode 1007): `less` and `man` scroll with the wheel
//   otherwise                         the history scrolls
//
// A wheel notch moves three lines. A trackpad moves the lines its swipe covers: on the main screen xterm.js's own scroll bar scrolls
// it by its pixels, as before; elsewhere one line per row height, keeping the remainder so a slow swipe still moves.

export interface WheelEventLike {
  deltaY: number;
  /** Browsers turn Shift+wheel into a sideways scroll: `deltaY` is 0 and this carries the turn. */
  deltaX?: number;
  /** 0 pixels, 1 lines, 2 pages. */
  deltaMode: number;
  /** The legacy field: 120 per notch in Chromium's wheel; three times `deltaY` in Firefox and WebKit, where it says nothing more. */
  wheelDeltaY?: number;
  wheelDeltaX?: number;
  /** Milliseconds; events close together are one gesture. */
  timeStamp?: number;
  shiftKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
}

export interface WheelState {
  altScreen: boolean;
  /** The program asked for mouse reports (any of the modes 9, 1000, 1002, 1003). */
  mouse: boolean;
  rows: number;
  /** Pixel height of one row. */
  cellHeight: number;
}

export type WheelAction =
  /** Let the terminal's own handling run (the mouse report, or its scroll bar for a trackpad on the main screen). */
  | { kind: 'pass' }
  /** Not the terminal's: the browser does what it does with it (Ctrl+wheel and a pinch zoom the page). */
  | { kind: 'browser' }
  /** Do nothing. */
  | { kind: 'ignore' }
  /** Scroll the history by this many lines (negative: towards older lines). */
  | { kind: 'scroll'; lines: number }
  /** Send this many arrow keys. */
  | { kind: 'arrows'; count: number; up: boolean };

export const LINES_PER_NOTCH = 3;
/** Wheel events closer together than this are one gesture: a swipe and its momentum, or a wheel spun fast. */
export const GESTURE_GAP_MS = 200;
/** A step smaller than this is never a notch. */
const NOTCH_MIN_PX = 40;

// Chromium's wheel counts the legacy field in whole 120s, whatever the pixel size of its notch. Firefox and WebKit make it three times
// the pixels, for a wheel and a trackpad alike, so there it counts nothing.
function legacyNotches(e: WheelEventLike): number {
  const legacy = Math.abs(e.wheelDeltaY ?? 0);
  if (!legacy || Math.abs(legacy - 3 * Math.abs(e.deltaY)) < 1.5 || legacy % 120 !== 0) return 0;
  return legacy / 120;
}

// Tells a wheel's notches from a trackpad's pixels, in pixel mode, in any engine, without knowing a device's notch size. A trackpad's
// gesture starts with small steps and keeps going (momentum included) with steps of any size; a wheel turns in steps of one size (a
// multiple when the engine joins several). So: a big step after a pause is a notch, and its size is the notch's; within a gesture a step
// is a notch only after a notch and only as a whole multiple of that size; anything else, and everything after a trackpad's step in the
// same gesture, is pixels. Before, any step of 40 px or more was a notch, and a fast swipe in Firefox or WebKit jumped three lines at a
// time. Not driven by a real trackpad or wheel here: the browsers' synthetic wheel is a notch of 100 px.
export class NotchDetector {
  private last: number | undefined;
  private notch = false;
  private size = 0;

  /** How many notches this pixel-mode event is; 0 for a trackpad's pixels. */
  classify(e: WheelEventLike): number {
    const t = e.timeStamp;
    const pause = t === undefined || this.last === undefined || t - this.last > GESTURE_GAP_MS;
    if (t !== undefined) this.last = t;
    const px = Math.abs(e.deltaY);
    let n = 0;
    if (px >= NOTCH_MIN_PX && (pause || this.notch)) {
      const legacy = legacyNotches(e);
      if (pause) { n = legacy || 1; this.size = px / n; }
      else {
        const r = px / this.size;
        n = legacy || (Math.round(r) >= 1 && Math.abs(r - Math.round(r)) < 0.05 ? Math.round(r) : 0);
      }
    }
    this.notch = n > 0;
    return n;
  }
}

export class Wheel {
  private carry = 0;
  private notches = new NotchDetector();

  decide(raw: WheelEventLike, s: WheelState): WheelAction {
    // Ctrl+wheel and a trackpad pinch (which browsers send as Ctrl+wheel) zoom, as everywhere else in the window.
    if (raw.ctrlKey) return { kind: 'browser' };
    // Shift+wheel arrives sideways; it is the history's (or the program's arrows').
    const e = raw.shiftKey && raw.deltaY === 0 && raw.deltaX ? { ...raw, deltaY: raw.deltaX, wheelDeltaY: raw.wheelDeltaX } : raw;
    // A sideways swipe is not the terminal's to decide.
    if (e.deltaY === 0) return { kind: 'pass' };
    const n = e.deltaMode === 0 ? this.notches.classify(e) : 0;
    const mouseMode = s.mouse && !e.shiftKey;
    // A trackpad over the main screen: xterm.js's scroll bar scrolls it smoothly by its pixels.
    if (!mouseMode && !s.altScreen && !e.shiftKey && e.deltaMode === 0 && n === 0) return { kind: 'pass' };
    const exact = e.deltaMode === 1 ? e.deltaY
      : e.deltaMode === 2 ? e.deltaY * s.rows
      : n ? Math.sign(e.deltaY) * n * LINES_PER_NOTCH
      : e.deltaY / Math.max(1, s.cellHeight);
    this.carry += exact;
    const whole = Math.trunc(this.carry);
    // Report mouse wheel events only when a whole line of swipe has gone by, so a trackpad does not flood the program.
    if (mouseMode) {
      if (n || e.deltaMode !== 0 || whole !== 0) { this.carry -= whole; return { kind: 'pass' }; }
      return { kind: 'ignore' };
    }
    if (whole === 0) return { kind: 'ignore' };
    this.carry -= whole;
    if (s.altScreen) {
      if (s.mouse) return { kind: 'ignore' };
      return { kind: 'arrows', count: Math.min(Math.abs(whole), s.rows), up: whole < 0 };
    }
    return { kind: 'scroll', lines: whole };
  }
}

// A finger dragging over the terminal (a phone or a tablet) moves the history one line for each row height the finger covers, the
// content following the finger. On the alternate screen the same drag is arrow keys; with mouse reporting on, taps are the program's
// and a drag does nothing.
export class TouchScroll {
  private last: number | undefined;
  private carry = 0;

  start(y: number): void { this.last = y; this.carry = 0; }
  end(): void { this.last = undefined; }

  move(y: number, s: WheelState): WheelAction {
    if (this.last === undefined) return { kind: 'ignore' };
    this.carry += this.last - y;
    this.last = y;
    const rowHeight = Math.max(1, s.cellHeight);
    const whole = Math.trunc(this.carry / rowHeight);
    if (whole === 0) return { kind: 'ignore' };
    this.carry -= whole * rowHeight;
    if (s.mouse) return { kind: 'ignore' };
    if (s.altScreen) return { kind: 'arrows', count: Math.min(Math.abs(whole), s.rows), up: whole < 0 };
    return { kind: 'scroll', lines: whole };
  }
}
