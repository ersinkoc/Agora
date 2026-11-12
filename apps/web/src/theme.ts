// The colour scheme: follow the system (default) or force dark or light. Remembered in this browser only; when the browser cannot
// store it the choice still holds for the visit. Tokens do the rest (design/tokens.css); terminals repaint on the event.
export type ThemePreference = 'system' | 'dark' | 'light';
const KEY = 'agora.theme';
const listeners = new Set<() => void>();
let current: ThemePreference = 'system';

function stored(): ThemePreference {
  try { const v = window.localStorage.getItem(KEY); return v === 'dark' || v === 'light' ? v : 'system'; } catch { return 'system'; }
}
function apply(p: ThemePreference) {
  const root = document.documentElement;
  if (p === 'system') delete root.dataset.theme; else root.dataset.theme = p;
}
export function initTheme() {
  current = stored();
  apply(current);
  window.matchMedia?.('(prefers-color-scheme: dark)').addEventListener?.('change', () => { if (current === 'system') notify(); });
}
function notify() { window.dispatchEvent(new Event('agora-theme')); listeners.forEach(l => l()); }
export function setTheme(p: ThemePreference) {
  current = p;
  try { if (p === 'system') window.localStorage.removeItem(KEY); else window.localStorage.setItem(KEY, p); } catch { /* not stored: holds for this visit */ }
  apply(p);
  notify();
}
export const getThemePreference = () => current;
export const subscribeTheme = (l: () => void) => { listeners.add(l); return () => { listeners.delete(l); }; };
export function resolvedScheme(): 'dark' | 'light' {
  if (current !== 'system') return current;
  return window.matchMedia?.('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
}
