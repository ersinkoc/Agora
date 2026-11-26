// Safe mode (ADR-0114 decision 4; docs/ux/plugins.md, "Safe mode"): the window opens with no plugins and the built-in theme. Two ways in,
// neither of which a plugin, a theme or a keybinding can change:
// - `?safe` in the address (`https://hub/?safe#/…`);
// - the safe-mode key, Ctrl+Alt+Shift+S (⌘⌥⇧S on a Mac), heard first by the window itself before anything else on the page, which
//   reopens the window with `?safe`. A key pressed while a plugin's frame has the focus goes to the frame, not the window: click outside
//   it first, or use the address.
export const isSafe = (search: string): boolean => new URLSearchParams(search).has('safe');
export const SAFE: boolean = typeof location !== 'undefined' && isSafe(location.search);

/** The safe-mode key: Ctrl (⌘ on a Mac) + Alt + Shift + S, by the key's place, so a keyboard layout cannot move it. */
export const isSafeKey = (e: Pick<KeyboardEvent, 'code' | 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey'>): boolean =>
  e.code === 'KeyS' && e.altKey && e.shiftKey && (e.ctrlKey || e.metaKey);

/** The address of this window in safe mode: the same page and route, with `safe` added. */
export function safeAddress(href: string): string {
  const u = new URL(href);
  u.searchParams.set('safe', '');
  return u.href.replace(/([?&])safe=(?=&|#|$)/, '$1safe');
}
/** The address of this window out of safe mode. */
export function unsafeAddress(href: string): string {
  const u = new URL(href);
  u.searchParams.delete('safe');
  return u.href;
}

/** Listens for the safe-mode key on the window, in the capture phase, before any handler of the page's own. Called once, at start. */
export function installSafeKey(w: Window = window): void {
  w.addEventListener('keydown', (e) => {
    if (!isSafeKey(e)) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    w.location.assign(safeAddress(w.location.href));
  }, { capture: true });
}
