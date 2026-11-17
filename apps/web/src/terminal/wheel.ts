// What a turn of the mouse wheel (or a two-finger swipe) does in a terminal, decided as a real terminal does. Pure: the renderer passes
// what the program asked for and the event, and does what this says.
//
//   mouse reporting on, plain wheel   the program gets the wheel as mouse buttons 4 and 5 (xterm sends them)
//   mouse reporting on, Shift held    the window scrolls its own history (Shift is the "ignore the program's mouse" key)
//   alternate screen, no mouse        arrow keys (xterm's alternateScroll, DEC mode 1007): `less` and `man` scroll with the wheel
//   otherwise                         the history scrolls
//
// A wheel notch moves three lines; a trackpad moves the lines its swipe covers (one line per cell height), and keeps the
// remainder so a slow swipe still moves.

export interface WheelEventLike {
  deltaY: number;
  /** 0 pixels, 1 lines, 2 pages. */
  deltaMode: number;
  /** The legacy field: a multiple of 120 for a wheel notch, anything for a trackpad. */
  wheelDeltaY?: number;
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
  /** Let the renderer's own handling run (it sends the mouse report). */
  | { kind: 'pass' }
  /** Do nothing. */
  | { kind: 'ignore' }
  /** Scroll the history by this many lines (negative: towards older lines). */
  | { kind: 'scroll'; lines: number }
  /** Send this many arrow keys. */
  | { kind: 'arrows'; count: number; up: boolean };

export const LINES_PER_NOTCH = 3;

// A notch is a big step that the legacy field counts in whole 120s; a trackpad sends small steps (a browser may still report the
// legacy field as a multiple of 120 for them, so the size is checked too).
const isNotch = (e: WheelEventLike) => e.deltaMode === 0 && Math.abs(e.deltaY) >= 40 && !!e.wheelDeltaY && Math.abs(e.wheelDeltaY) % 120 === 0;

export class Wheel {
  private carry = 0;

  /** Lines (a fraction) this event covers; positive is down (newer). */
  private lines(e: WheelEventLike, s: WheelState): number {
    if (e.deltaMode === 1) return e.deltaY;
    if (e.deltaMode === 2) return e.deltaY * s.rows;
    if (isNotch(e)) return (Math.sign(e.deltaY) * Math.abs((e.wheelDeltaY ?? 0) / 120)) * LINES_PER_NOTCH;
    return e.deltaY / Math.max(1, s.cellHeight);
  }

  decide(e: WheelEventLike, s: WheelState): WheelAction {
    if (e.deltaY === 0) return { kind: 'ignore' };
    const exact = this.lines(e, s);
    this.carry += exact;
    const whole = Math.trunc(this.carry);
    // Report mouse wheel events only when a whole line of swipe has gone by, so a trackpad does not flood the program.
    const mouseMode = s.mouse && !e.shiftKey;
    if (mouseMode) {
      if (isNotch(e) || e.deltaMode !== 0 || whole !== 0) { this.carry -= whole; return { kind: 'pass' }; }
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
    const whole = Math.trunc(this.carry / Math.max(1, s.cellHeight));
    if (whole === 0) return { kind: 'ignore' };
    this.carry -= whole * s.cellHeight;
    if (s.mouse) return { kind: 'ignore' };
    if (s.altScreen) return { kind: 'arrows', count: Math.min(Math.abs(whole), s.rows), up: whole < 0 };
    return { kind: 'scroll', lines: whole };
  }
}
