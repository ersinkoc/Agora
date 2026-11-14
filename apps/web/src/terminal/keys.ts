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
