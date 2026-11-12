// Closed agents stay in the list until the owner clears them. The clearing is this browser's (the hub keeps its record): remembered
// locally, and a failure to store it only means the agents come back next visit.
const KEY = 'agora.dismissed';
const listeners = new Set<() => void>();
let ids: ReadonlySet<string> = (() => {
  try { const v = JSON.parse(window.localStorage.getItem(KEY) ?? '[]'); return new Set<string>(Array.isArray(v) ? v.filter((x) => typeof x === 'string') : []); } catch { return new Set<string>(); }
})();
export const dismissedIds = () => ids;
export const subscribeDismissed = (l: () => void) => { listeners.add(l); return () => { listeners.delete(l); }; };
export function dismiss(more: readonly string[]) {
  const next = new Set(ids);
  more.forEach((id) => next.add(id));
  ids = next;
  try { window.localStorage.setItem(KEY, JSON.stringify([...next].slice(-500))); } catch { /* holds for this visit */ }
  listeners.forEach((l) => l());
}
