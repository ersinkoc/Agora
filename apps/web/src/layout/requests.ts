// Asking the window to show a panel from outside it (the command palette, This device): the request waits until the window is
// there to take it, so it survives the navigation back to the window.
let pending: string | undefined;
const listeners = new Set<() => void>();

export function requestPanel(panel: string) {
  pending = panel;
  listeners.forEach((l) => l());
}
export function takePanelRequest(): string | undefined {
  const p = pending;
  pending = undefined;
  return p;
}
export const subscribePanelRequests = (l: () => void) => { listeners.add(l); return () => { listeners.delete(l); }; };
