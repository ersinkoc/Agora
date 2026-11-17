// ADR-0067 rung 2: the keys a touch keyboard does not have, as the bytes a terminal program expects. Pure: the key bar draws the
// buttons, this decides what each one sends.

export type BarKey = 'esc' | 'tab' | 'up' | 'down' | 'left' | 'right';
export const BAR_KEYS: readonly BarKey[] = ['esc', 'tab', 'left', 'up', 'down', 'right'];

const ARROW: Record<'up' | 'down' | 'right' | 'left', string> = { up: 'A', down: 'B', right: 'C', left: 'D' };

// `appCursor`: the program asked for application cursor keys (DECCKM, as vim and less do), so an arrow is `ESC O x` instead of
// `ESC [ x`. With Ctrl held an arrow carries the modifier (`ESC [ 1 ; 5 x`, what xterm sends), whatever the cursor mode.
export function keyBytes(key: BarKey, { ctrl = false, appCursor = false }: { ctrl?: boolean; appCursor?: boolean } = {}): string {
  switch (key) {
    case 'esc': return '\x1b';
    case 'tab': return '\t';
    default: {
      const c = ARROW[key];
      if (ctrl) return `\x1b[1;5${c}`;
      return appCursor ? `\x1bO${c}` : `\x1b[${c}`;
    }
  }
}

// What Ctrl plus one typed character sends (the C0 control codes, as a terminal keyboard makes them): a letter or `@ [ \ ] ^ _` is
// its code with the top bits cleared, space and `2` are NUL, `3`-`7` are ESC to US, `8` and `?` are DEL. Anything else has no control
// form and is sent as typed.
export function ctrlChar(ch: string): string {
  if (ch.length !== 1) return ch;
  const c = ch.charCodeAt(0);
  if ((c >= 0x61 && c <= 0x7a) || (c >= 0x40 && c <= 0x5f)) return String.fromCharCode(c & 0x1f);
  if (ch === ' ' || ch === '2') return '\x00';
  if (ch >= '3' && ch <= '7') return String.fromCharCode(c - 0x33 + 0x1b);
  if (ch === '8' || ch === '?') return '\x7f';
  return ch;
}

// Text from the touch keyboard while Ctrl is on: one typed character becomes its control code. Longer input (a word the keyboard
// completed, a paste) is sent as it is, so Ctrl never garbles text.
export function withCtrl(data: string): string {
  return [...data].length === 1 ? ctrlChar(data) : data;
}

// ---- Keyboard shortcuts of a terminal window (not sent to the program) ----

export interface KeyLike {
  key: string;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
  metaKey: boolean;
}

export type Shortcut = 'copy' | 'find';

// Ctrl+C belongs to the program (it interrupts), so a terminal copies with Ctrl+Shift+C (and Ctrl+Insert); on a Mac it is Cmd+C. Find is
// Ctrl+Shift+F (Cmd+F on a Mac), since Ctrl+F belongs to the program too.
export function shortcut(e: KeyLike, mac: boolean): Shortcut | undefined {
  const k = e.key.toLowerCase();
  if (mac) {
    if (e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey) return k === 'c' ? 'copy' : k === 'f' ? 'find' : undefined;
    return undefined;
  }
  if (e.ctrlKey && !e.altKey && !e.metaKey) {
    if (e.shiftKey && k === 'c') return 'copy';
    if (!e.shiftKey && k === 'insert') return 'copy';
    if (e.shiftKey && k === 'f') return 'find';
  }
  return undefined;
}

// ---- Keys with modifiers that a plain terminal cannot tell apart ----
// Shift+Enter and Enter are both CR in a plain terminal, yet a program (a coding agent's prompt, an editor) wants to tell "new line"
// from "send". Programs say so by switching on the CSI u keyboard protocol (`CSI > flags u`, flag 1: "disambiguate") or xterm's
// modifyOtherKeys (`CSI > 4 ; level m`). Only then does the window send these keys in the longer forms; a program that did not ask
// gets exactly the bytes it always got.

export interface EnhancedKeys {
  /** The CSI u keyboard flags the program set (0: off). */
  csiU: number;
  /** The modifyOtherKeys level the program set (0: off). */
  modifyOtherKeys: number;
}

const CODE: Record<string, number> = { Enter: 13, Tab: 9, Backspace: 127 };

export function enhancedKey(e: KeyLike, m: EnhancedKeys): string | undefined {
  const code = CODE[e.key];
  if (code === undefined || e.metaKey) return undefined;
  const mod = 1 + (e.shiftKey ? 1 : 0) + (e.altKey ? 2 : 0) + (e.ctrlKey ? 4 : 0);
  if (mod === 1) return undefined;
  // Shift+Tab already has its own legacy form (`CSI Z`) that every program understands.
  if (e.key === 'Tab' && mod === 2) return undefined;
  if (m.csiU & 1) return `\x1b[${code};${mod}u`;
  if (m.modifyOtherKeys > 0) return `\x1b[27;${mod};${code}~`;
  return undefined;
}

/** The CSI u keyboard protocol's flag stack: push, pop, set and the query reply (`CSI ? flags u`). */
export class CsiUFlags {
  private stack: number[] = [];
  flags = 0;
  push(f: number) { if (this.stack.length < 32) this.stack.push(this.flags); this.flags = f & 31; }
  pop(n = 1) { for (let i = 0; i < Math.max(1, n); i++) this.flags = this.stack.pop() ?? 0; }
  /** mode 1 sets the given bits, 2 adds them, 3 clears them. */
  set(f: number, mode: number) { f &= 31; this.flags = mode === 2 ? this.flags | f : mode === 3 ? this.flags & ~f : f; }
  reset() { this.stack = []; this.flags = 0; }
  reply(): string { return `\x1b[?${this.flags}u`; }
}
