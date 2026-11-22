// The urxvt mouse encoding (DEC private mode 1015), which xterm.js does not write: it logs "not supported" and keeps the encoding it
// had, so a program that asked for 1015 got classic bytes it misreads. xterm.js's own encoding decides as before: SGR (1006) or SGR
// pixels (1016) win whenever they are on, as in VTE and as programs got until now. Only where xterm.js would write the classic form is
// the report written in the urxvt form instead, while the program has 1015 on, from xterm.js's event (cells, no column limit). Pure,
// except `followMouseEncoding`.
import type { Terminal } from '@xterm/xterm';

/** Whether the program has the urxvt encoding (1015) on: set and reset by that mode alone, off after a full reset (RIS). */
export class UrxvtMode {
  on = false;

  apply(params: readonly (number | readonly number[])[], set: boolean): void {
    if (params.some((p) => p === 1015)) this.on = set;
  }

  reset(): void { this.on = false; }
}

/** A mouse event as xterm.js hands it to an encoding: column and row from 1, pixels, button and action as xterm.js numbers them. */
export interface MouseReportEvent {
  col: number;
  row: number;
  x: number;
  y: number;
  /** 0 left, 1 middle, 2 right, 3 none, 4 wheel, 8 and up the extra buttons. */
  button: number;
  /** 0 up, 1 down, 2 and 3 wheel left and right, 32 move. */
  action: number;
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
}

const WHEEL = 4;
const UP = 0;
const MOVE = 32;

// The button code every encoding starts from (xterm's): modifiers, the button, wheel and motion bits. Only SGR says which button was
// let go; the others report a release as button 3.
function buttonCode(e: MouseReportEvent, sgr: boolean): number {
  let code = (e.ctrl ? 16 : 0) | (e.shift ? 4 : 0) | (e.alt ? 8 : 0);
  if (e.button === WHEEL) return code | 64 | e.action;
  code |= e.button & 3;
  if (e.button & 4) code |= 64;
  if (e.button & 8) code |= 128;
  if (e.action === MOVE) code |= MOVE;
  else if (e.action === UP && !sgr) code |= 3;
  return code;
}

/** The report for this event in the classic form (`ESC [ M` and three bytes; nothing past column or row 223) or the urxvt form
 * (`ESC [ Cb ; Cx ; Cy M`). */
export function encodeMouse(e: MouseReportEvent, form: 'classic' | 'urxvt'): string {
  const code = buttonCode(e, false);
  if (form === 'urxvt') return `\x1b[${code + 32};${e.col};${e.row}M`;
  const b = [code + 32, e.col + 32, e.row + 32];
  return b.some((n) => n > 255) ? '' : `\x1b[M${String.fromCharCode(...b)}`;
}

interface MouseServiceLike { addEncoding(name: string, encode: (e: MouseReportEvent) => string): void }

/** Follows mode 1015 and makes xterm.js write its classic reports in the urxvt form while it is on. xterm.js's classic encoding is
 * replaced through its mouse service, which is not public API; when a later xterm.js lacks it this returns undefined and xterm.js
 * keeps its own (no urxvt). Its SGR encodings are left as they are. The report still goes out through `onBinary`, as a classic one
 * does; the urxvt form is ASCII, so its bytes are the same. */
export function followMouseEncoding(term: Terminal): UrxvtMode | undefined {
  const svc = (term as unknown as { _core?: { coreMouseService?: Partial<MouseServiceLike> } })._core?.coreMouseService;
  if (typeof svc?.addEncoding !== 'function') return undefined;
  const mode = new UrxvtMode();
  // Each handler returns false so xterm.js still handles the sequence itself (the other modes in it, its own encoding).
  term.parser.registerCsiHandler({ prefix: '?', final: 'h' }, (params) => { mode.apply(params, true); return false; });
  term.parser.registerCsiHandler({ prefix: '?', final: 'l' }, (params) => { mode.apply(params, false); return false; });
  term.parser.registerEscHandler({ final: 'c' }, () => { mode.reset(); return false; });
  svc.addEncoding.call(svc, 'DEFAULT', (e) => encodeMouse(e, mode.on ? 'urxvt' : 'classic'));
  return mode;
}
