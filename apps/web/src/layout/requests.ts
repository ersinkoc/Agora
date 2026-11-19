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

// The same for a tool pane (Terminal, Changes, Files, File manager) asked for from outside the window (the command palette, a terminal
// that was just started): which tool, and for which agent when the caller knows it. `wait` holds the request until that agent is in the
// fleet (a terminal started a moment ago is not there yet).
export interface ToolRequest { panel: string; agent?: string; wait?: boolean }
let pendingTools: ToolRequest[] = [];
const toolListeners = new Set<() => void>();
export function requestTool(req: ToolRequest) {
  pendingTools = [...pendingTools, req].slice(-8);
  toolListeners.forEach((l) => l());
}
export function takeToolRequests(): ToolRequest[] {
  const r = pendingTools;
  pendingTools = [];
  return r;
}
export const subscribeToolRequests = (l: () => void) => { toolListeners.add(l); return () => { toolListeners.delete(l); }; };
