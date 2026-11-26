// The terminal's text size and how it behaves: one choice for every terminal in this browser, remembered locally (a failure to store
// it is fine). The size has its own key and event from before the others existed; the other choices share one record.
const KEY = 'agora.terminal.size';
export const MIN_SIZE = 10;
export const MAX_SIZE = 22;
let size = 13;
try { const v = Number(window.localStorage.getItem(KEY)); if (v >= MIN_SIZE && v <= MAX_SIZE) size = v; } catch { /* default */ }
export const terminalSize = () => size;
export function setTerminalSize(next: number) {
  const rounded = Math.round(next);
  if (Number.isFinite(rounded)) size = Math.min(MAX_SIZE, Math.max(MIN_SIZE, rounded));
  try { window.localStorage.setItem(KEY, String(size)); } catch { /* holds for this visit */ }
  window.dispatchEvent(new Event('agora-terminal-size'));
}

export interface TerminalPrefs {
  /** Row height as a multiple of the font size. A real terminal's is 1: box-drawing and block characters then join. */
  lineHeight: 1 | 1.1 | 1.2;
  /** Lines of history this window keeps for each terminal. */
  scrollback: 1000 | 2000 | 5000 | 10000 | 20000;
  /** Lift a program's own colours that are hard to read on the pane's background (off: they are drawn as the program chose). */
  readable: boolean;
  /** Copy the selection to the clipboard as soon as it is made (a terminal on X11 does it for the primary selection). */
  copyOnSelect: boolean;
  /** Draw with the graphics card: box-drawing and block characters are exact (the page's text drawing leaves gaps). Opt-in until it is
   confirmed on a real scaled screen; a browser allows about sixteen graphics contexts per page. */
  gpu: boolean;
}

export const LINE_HEIGHTS: readonly TerminalPrefs['lineHeight'][] = [1, 1.1, 1.2];
export const SCROLLBACKS: readonly TerminalPrefs['scrollback'][] = [1000, 2000, 5000, 10000, 20000];
export const DEFAULT_PREFS: TerminalPrefs = { lineHeight: 1, scrollback: 5000, readable: false, copyOnSelect: false, gpu: false };

const PREFS_KEY = 'agora.terminal.prefs';

/** What was stored, kept only where it is a value this version knows (anything else falls back to the default). */
export function parsePrefs(raw: string | null | undefined): TerminalPrefs {
  let o: Record<string, unknown> = {};
  try { const v: unknown = raw ? JSON.parse(raw) : {}; if (v && typeof v === 'object') o = v as Record<string, unknown>; } catch { /* default */ }
  const pick = <T,>(v: unknown, allowed: readonly T[], d: T): T => (allowed as readonly unknown[]).includes(v) ? (v as T) : d;
  return {
    lineHeight: pick(o.lineHeight, LINE_HEIGHTS, DEFAULT_PREFS.lineHeight),
    scrollback: pick(o.scrollback, SCROLLBACKS, DEFAULT_PREFS.scrollback),
    readable: typeof o.readable === 'boolean' ? o.readable : DEFAULT_PREFS.readable,
    copyOnSelect: typeof o.copyOnSelect === 'boolean' ? o.copyOnSelect : DEFAULT_PREFS.copyOnSelect,
    gpu: typeof o.gpu === 'boolean' ? o.gpu : DEFAULT_PREFS.gpu,
  };
}

let prefs: TerminalPrefs = DEFAULT_PREFS;
try { prefs = parsePrefs(window.localStorage.getItem(PREFS_KEY)); } catch { /* default */ }
export const terminalPrefs = () => prefs;
export function setTerminalPrefs(next: Partial<TerminalPrefs>) {
  prefs = parsePrefs(JSON.stringify({ ...prefs, ...next }));
  try { window.localStorage.setItem(PREFS_KEY, JSON.stringify(prefs)); } catch { /* holds for this visit */ }
  window.dispatchEvent(new Event('agora-terminal-prefs'));
}
