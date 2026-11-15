import type { ProjectChoice } from '../net/view';

// Which machine the start window shows selected. The one already chosen while it is online; otherwise the one the window was
// opened for, a machine with a known project, or the first online machine. A machine that comes online after the window opened
// (a node that has just dialed in) is then picked up instead of leaving nothing selected and no folders to choose from.
export function machineFor(current: string, online: readonly string[], initialNode: string | undefined, projects: readonly ProjectChoice[]): string {
  if (online.includes(current)) return current;
  if (initialNode && online.includes(initialNode)) return initialNode;
  return projects.find(p => online.includes(p.node))?.node ?? online[0] ?? '';
}
