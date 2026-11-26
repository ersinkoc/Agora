import type { ProjectChoice } from '../net/view';
import { visible } from '../net/hidden';

// Which machine the start window shows selected. The one already chosen while it is online; otherwise the one the window was
// opened for, a machine with a known project, or the first online machine. A machine that comes online after the window opened
// (a node that has just dialed in) is then picked up instead of leaving nothing selected and no folders to choose from.
export function machineFor(current: string, online: readonly string[], initialNode: string | undefined, projects: readonly ProjectChoice[]): string {
  if (online.includes(current)) return current;
  if (initialNode && online.includes(initialNode)) return initialNode;
  return projects.find(p => online.includes(p.node))?.node ?? online[0] ?? '';
}

/** ADR-0069 and finding L9: why the chosen kind cannot start, as a message id and its arguments, shown only while that kind is chosen: its
 * program by name, or the general sentence for a kind from an older machine that does not name it. Nothing for an installed kind. */
export function missingNote(k: { installed: boolean; program?: string } | undefined): [string, Record<string, string>?] | undefined {
  if (!k || k.installed) return undefined;
  return k.program ? ['start-not-installed-program', { program: visible(k.program) }] : ['start-not-installed-why'];
}

/** ADR-0069: where a machine's program runs from when its folder is not on that machine's PATH, as shown: the last two parts only
 * (`…\Downloads\agora.exe`), since the full path can hold the machine's user name. */
export function shortBinary(binary: string, os?: string): string {
  const sep = isWindows(binary, os) ? '\\' : '/';
  const parts = binary.split(/[\\/]/).filter(Boolean);
  return parts.length > 2 ? `…${sep}${parts.slice(-2).join(sep)}` : parts.join(sep);
}

/** Whether `binary` is a file name only, with no folder (what a member gets from the hub, #598). */
export function isBareName(binary: string): boolean {
  return !/[\\/]/.test(binary);
}

/** Whether the machine runs Windows: from its `os`, which holds whatever form of the path arrives (a bare file name has no `\`); from
 * the path only when the machine did not say. */
function isWindows(binary: string, os?: string): boolean {
  return os ? os === 'windows' : binary.includes('\\');
}

/** The command that makes a machine trust this device. When the machine's program is not on its PATH (`binary` set), it is run by
 * its file name from its own folder (`.\agora.exe` on Windows, `./agora` elsewhere), so no full path is shown or needed. A name
 * with anything beyond letters, digits, `.`, `_` and `-` (a second download is `agora (1).exe`) is quoted so the shell takes it as
 * one word: PowerShell's call form `& '.\agora (1).exe'` on Windows, `'./agora (1)'` elsewhere. */
export function trustCommand(binary: string | undefined, os: string | undefined): string {
  // ADR-0061: the machine fetches this person's registered devices from the hub and asks on its own terminal, after the person
  // compares the fingerprint with "This device"; no device id or 130-character key to copy.
  const args = 'node trust --from-hub';
  if (!binary) return `agora ${args}`;
  const windows = isWindows(binary, os);
  const file = binary.split(/[\\/]/).filter(Boolean).pop() ?? 'agora';
  const plain = /^[A-Za-z0-9._-]+$/.test(file);
  if (windows) return plain ? `.\\${file} ${args}` : `& '.\\${file.replace(/'/g, "''")}' ${args}`;
  return plain ? `./${file} ${args}` : `'./${file.replace(/'/g, "'\\''")}' ${args}`;
}
