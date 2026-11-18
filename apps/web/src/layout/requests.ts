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

// The same for the project the window shows (the command palette's "Project: ...").
let pendingProject: string | undefined;
const projectListeners = new Set<() => void>();
export function requestProject(id: string) {
  pendingProject = id;
  projectListeners.forEach((l) => l());
}
export function takeProjectRequest(): string | undefined {
  const p = pendingProject;
  pendingProject = undefined;
  return p;
}
export const subscribeProjectRequests = (l: () => void) => { projectListeners.add(l); return () => { projectListeners.delete(l); }; };
