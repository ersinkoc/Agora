// Keys as data (ADR-0068 decision 6, docs/ux/keybindings.md): a key string from a keybindings file, a key pressed, and the words a key
// is shown with. Pure: no DOM, no storage, so the parser, the matcher and the rules about where a key may act are tested without a page.
//
// A key string is one stroke or a chord of two, separated by one space: `ctrl+k`, `alt+shift+1`, `n`, `ctrl+k ctrl+s`. A stroke is
// modifiers joined by `+` and one key last. Letters, digits and punctuation are matched by their place on the keyboard (the `code` of the
// event), so Shift or a keyboard layout cannot change which key it is (as the side panel's Alt+Shift+1 and the safe-mode key were already).

export interface Stroke { ctrl: boolean; alt: boolean; shift: boolean; meta: boolean; key: string }
/** One or two strokes. */
export type Binding = readonly Stroke[];

export const MAX_KEY_STRING = 64;
export const MAX_STROKES = 2;

// The keys a stroke may end in, by the name a file writes. Letters and digits are added below.
const NAMED = ['enter', 'escape', 'tab', 'space', 'backspace', 'delete', 'insert', 'home', 'end', 'pageup', 'pagedown', 'up', 'down', 'left', 'right'] as const;
const FKEYS = Array.from({ length: 12 }, (_, i) => `f${i + 1}`);
// Punctuation by its place (US layout names the place): the event's `code` and the character a file writes.
const PUNCT: Record<string, string> = {
  Minus: '-', Equal: '=', BracketLeft: '[', BracketRight: ']', Backslash: '\\', Semicolon: ';', Quote: "'", Comma: ',', Period: '.', Slash: '/', Backquote: '`',
};
const PUNCT_KEYS = new Set(Object.values(PUNCT));
const isLetter = (k: string) => /^[a-z]$/.test(k);
const isDigit = (k: string) => /^[0-9]$/.test(k);
const KNOWN = new Set<string>([...NAMED, ...FKEYS]);
export const isKeyName = (k: string): boolean => isLetter(k) || isDigit(k) || PUNCT_KEYS.has(k) || KNOWN.has(k);

const ALIASES: Record<string, string> = { esc: 'escape', return: 'enter', del: 'delete', ins: 'insert', arrowup: 'up', arrowdown: 'down', arrowleft: 'left', arrowright: 'right', pgup: 'pageup', pgdn: 'pagedown' };
const MODS: Record<string, keyof Omit<Stroke, 'key'>> = { ctrl: 'ctrl', control: 'ctrl', alt: 'alt', option: 'alt', opt: 'alt', shift: 'shift', meta: 'meta', cmd: 'meta', command: 'meta', super: 'meta', win: 'meta' };

/** One stroke from its text (`ctrl+shift+k`), or undefined when it is not one: an unknown key or modifier, a modifier twice, no key. */
export function parseStroke(text: string): Stroke | undefined {
  // `+` cannot be a key here (`=` is that key's place): a trailing `+` is malformed rather than guessed.
  const parts = text.toLowerCase().split('+');
  if (parts.some((p) => p === '')) return undefined;
  const s: Stroke = { ctrl: false, alt: false, shift: false, meta: false, key: '' };
  for (let i = 0; i < parts.length; i++) {
    const p = parts[i] as string;
    const mod = MODS[p];
    if (i < parts.length - 1) {
      if (!mod || s[mod]) return undefined;
      s[mod] = true;
    } else {
      const key = ALIASES[p] ?? p;
      if (!isKeyName(key)) return undefined;
      s.key = key;
    }
  }
  return s;
}

/** A key string (`ctrl+k ctrl+s`) as its strokes, or undefined when any part is malformed. Exactly one space between strokes. */
export function parseBinding(text: unknown): Binding | undefined {
  if (typeof text !== 'string' || text.length === 0 || text.length > MAX_KEY_STRING) return undefined;
  if (text !== text.trim()) return undefined;
  const parts = text.split(' ');
  if (parts.length > MAX_STROKES) return undefined;
  const out: Stroke[] = [];
  for (const p of parts) {
    const s = parseStroke(p);
    if (!s) return undefined;
    out.push(s);
  }
  return out;
}

/** The one spelling of a stroke: modifiers in a fixed order (ctrl, alt, shift, meta), then the key. Two equal strokes write the same. */
export function strokeText(s: Stroke): string {
  return [s.ctrl && 'ctrl', s.alt && 'alt', s.shift && 'shift', s.meta && 'meta', s.key].filter(Boolean).join('+');
}
export const bindingText = (b: Binding): string => b.map(strokeText).join(' ');
export const sameStroke = (a: Stroke, b: Stroke): boolean => a.key === b.key && a.ctrl === b.ctrl && a.alt === b.alt && a.shift === b.shift && a.meta === b.meta;
/** `a` is `b`, or the start of it (a chord). */
export const startsWith = (b: Binding, a: Binding): boolean => a.length <= b.length && a.every((s, i) => sameStroke(s, b[i] as Stroke));

// ---- A key pressed --------------------------------------------------------------------------------------------------------------

export interface KeyEventLike { key: string; code?: string; ctrlKey: boolean; altKey: boolean; shiftKey: boolean; metaKey: boolean; isComposing?: boolean; keyCode?: number }

const KEY_NAMES: Record<string, string> = {
  Enter: 'enter', Escape: 'escape', Esc: 'escape', Tab: 'tab', ' ': 'space', Spacebar: 'space', Backspace: 'backspace', Delete: 'delete', Insert: 'insert',
  Home: 'home', End: 'end', PageUp: 'pageup', PageDown: 'pagedown', ArrowUp: 'up', ArrowDown: 'down', ArrowLeft: 'left', ArrowRight: 'right',
};

/**
 * The stroke a key press is, or undefined for a press that is no stroke: a modifier on its own, a key no binding can name (the numeric
 * keypad, media keys), or a key typed into an input method's composition. Letters, digits and punctuation by `code` (their place);
 * an event without one (made by a script) falls back to `key`.
 */
export function strokeOf(e: KeyEventLike): Stroke | undefined {
  if (e.isComposing || e.keyCode === 229) return undefined;
  const mods = { ctrl: e.ctrlKey, alt: e.altKey, shift: e.shiftKey, meta: e.metaKey };
  const code = e.code ?? '';
  let key: string | undefined;
  let m: RegExpExecArray | null;
  if ((m = /^Key([A-Z])$/.exec(code))) key = (m[1] as string).toLowerCase();
  else if ((m = /^Digit([0-9])$/.exec(code))) key = m[1] as string;
  else if (PUNCT[code]) key = PUNCT[code];
  else if (/^F([1-9]|1[0-2])$/.test(e.key)) key = e.key.toLowerCase();
  else if (KEY_NAMES[e.key]) key = KEY_NAMES[e.key];
  else if (!code && e.key.length === 1) {
    const k = e.key.toLowerCase();
    if (isLetter(k) || isDigit(k) || PUNCT_KEYS.has(k)) key = k;
  }
  return key ? { ...mods, key } : undefined;
}

// ---- Where a key may act ----------------------------------------------------------------------------------------------------------

const printable = (k: string) => isLetter(k) || isDigit(k) || PUNCT_KEYS.has(k) || k === 'space';

/**
 * A stroke that types text in a field: a printable key alone or with Shift, and on a Mac with Option too (Option+1 is "¡" there). A
 * binding never takes such a key from a text field, so typing a name or a prompt is never a command.
 */
export function typesText(s: Stroke, mac: boolean): boolean {
  if (s.ctrl || s.meta) return false;
  if (s.alt && !mac) return false;
  return printable(s.key);
}

/**
 * A stroke a terminal program can never receive in this window, so the window may hear it while a terminal has the keyboard: any ⌘
 * (Meta) key, and Ctrl+Shift with a letter (the terminal has no bytes for it). Everything else (Ctrl+K, Alt+1, a letter) is the
 * program's. The terminal's own Ctrl+Shift+C, F and V are fixed keys (commands.ts), so no binding has them.
 */
export function reachesPastTerminal(s: Stroke): boolean {
  if (s.meta) return true;
  return s.ctrl && s.shift && !s.alt && isLetter(s.key);
}

// ---- The words a key is shown with --------------------------------------------------------------------------------------------------

// Key caps as keyboards print them: the same words in every language, as before this rung (`Ctrl K`, `Alt+Shift+1`).
const CAP: Record<string, string> = {
  enter: 'Enter', escape: 'Esc', tab: 'Tab', space: 'Space', backspace: 'Backspace', delete: 'Delete', insert: 'Insert', home: 'Home', end: 'End',
  pageup: 'PageUp', pagedown: 'PageDown', up: '↑', down: '↓', left: '←', right: '→',
};
const capOf = (k: string) => CAP[k] ?? k.toUpperCase();

/**
 * A stroke as the window shows it: `Ctrl+Shift+K` (on a Mac `⌃⌥⇧⌘K`, its own symbols). `sep` joins a modifier and the key (the title
 * bar's hint uses a space: `Ctrl K`, `⌘ K`); chords are joined by a space.
 */
export function strokeLabel(s: Stroke, mac: boolean, sep?: string): string {
  if (mac) {
    const mods = `${s.ctrl ? '⌃' : ''}${s.alt ? '⌥' : ''}${s.shift ? '⇧' : ''}${s.meta ? '⌘' : ''}`;
    return mods ? `${mods}${sep ?? ''}${capOf(s.key)}` : capOf(s.key);
  }
  const mods = [s.ctrl && 'Ctrl', s.alt && 'Alt', s.shift && 'Shift', s.meta && 'Meta'].filter(Boolean) as string[];
  return [...mods, capOf(s.key)].join(sep ?? '+');
}
export const bindingLabel = (b: Binding, mac: boolean, sep?: string): string => b.map((s) => strokeLabel(s, mac, sep)).join(' ');

/** A binding as `aria-keyshortcuts` spells it (WAI-ARIA: `Control+K`, `Alt+Shift+1`, chords separated by a space). */
export function ariaKeys(b: Binding): string {
  return b.map((s) => [s.ctrl && 'Control', s.alt && 'Alt', s.shift && 'Shift', s.meta && 'Meta', ariaKey(s.key)].filter(Boolean).join('+')).join(' ');
}
const ARIA: Record<string, string> = { escape: 'Escape', space: 'Space', up: 'ArrowUp', down: 'ArrowDown', left: 'ArrowLeft', right: 'ArrowRight' };
const ariaKey = (k: string) => ARIA[k] ?? (CAP[k] ?? k.toUpperCase());

/**
 * Of a command's keys, the one to show: on a Mac one with ⌘ when there is one, elsewhere one without; else the first. The others still
 * work (Ctrl+K opens the palette on a Mac too).
 */
export function primary(keys: readonly Binding[], mac: boolean): Binding | undefined {
  return keys.find((b) => b.some((s) => s.meta) === mac) ?? keys[0];
}

/** This browser runs on an Apple system (the desktop app's WebKit included). */
export const IS_MAC = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
