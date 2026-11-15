// The terminal's text size: one choice for every terminal in this browser, remembered locally (a failure to store it is fine).
const KEY = 'agora.terminal.size';
export const MIN_SIZE = 10;
export const MAX_SIZE = 22;
let size = 13;
try { const v = Number(window.localStorage.getItem(KEY)); if (v >= MIN_SIZE && v <= MAX_SIZE) size = v; } catch { /* default */ }
export const terminalSize = () => size;
export function setTerminalSize(next: number) {
  size = Math.min(MAX_SIZE, Math.max(MIN_SIZE, Math.round(next)));
  try { window.localStorage.setItem(KEY, String(size)); } catch { /* holds for this visit */ }
  window.dispatchEvent(new Event('agora-terminal-size'));
}
