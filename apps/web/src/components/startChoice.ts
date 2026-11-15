import type { ProjectChoice } from '../net/view';

// Which machine the start window shows selected. The one already chosen while it is online; otherwise the one the window was
// opened for, a machine with a known project, or the first online machine. A machine that comes online after the window opened
// (a node that has just dialed in) is then picked up instead of leaving nothing selected and no folders to choose from.
export function machineFor(current: string, online: readonly string[], initialNode: string | undefined, projects: readonly ProjectChoice[]): string {
  if (online.includes(current)) return current;
  if (initialNode && online.includes(initialNode)) return initialNode;
  return projects.find(p => online.includes(p.node))?.node ?? online[0] ?? '';
}

/** ADR-0069: the programs a machine could not find for the agents it marked "not installed", each once, in order; `general` when one
 * of them came from an older machine that does not name its program, so the general sentence is shown for it. */
export function missingPrograms(kinds: { installed: boolean; program?: string }[]): { programs: string[]; general: boolean } {
  const programs: string[] = [];
  let general = false;
  for (const k of kinds) {
    if (k.installed) continue;
    if (!k.program) general = true;
    else if (!programs.includes(k.program)) programs.push(k.program);
  }
  return { programs, general };
}

/** ADR-0069: where a machine's program runs from when its folder is not on that machine's PATH, as shown: the last two parts only
 * (`…\Downloads\agora.exe`), since the full path can hold the machine's user name. */
export function shortBinary(binary: string): string {
  const sep = binary.includes('\\') ? '\\' : '/';
  const parts = binary.split(/[\\/]/).filter(Boolean);
  return parts.length > 2 ? `…${sep}${parts.slice(-2).join(sep)}` : parts.join(sep);
}

/** The command that makes a machine trust this device. When the machine's program is not on its PATH (`binary` set), it is run by
 * its file name from its own folder (`.\agora.exe` on Windows, `./agora` elsewhere), so no full path is shown or needed. */
export function trustCommand(binary: string | undefined, id: string, key: string): string {
  if (!binary) return `agora node trust ${id} ${key}`;
  const windows = binary.includes('\\');
  const file = binary.split(/[\\/]/).filter(Boolean).pop() ?? 'agora';
  return `${windows ? '.\\' : './'}${file} node trust ${id} ${key}`;
}
