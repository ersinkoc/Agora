import { toHex, type Device } from './approval';

export interface DirectoryInfo { path: string; projectId: string; gitRoot: string; branch: string; portable: boolean; previousProjectId?: string }
export function directoryMessage(node: string, operation: string, path: string, kind: string, project: string, option: boolean, identityChange: boolean, nonce: string, expires: number): Uint8Array {
  return new TextEncoder().encode(['directory-action-v1', node, operation, path, kind, project, option ? '1' : '0', identityChange ? '1' : '0', nonce, String(expires)].join('\0'));
}
/** ADR-0059: a list as signed: its count, then `:<byte length>:<item>` per item (UTF-8 bytes), so no two lists sign alike. */
export function signedList(items: readonly string[]): string {
  return String(items.length) + items.map((i) => `:${new TextEncoder().encode(i).length}:${i}`).join('');
}
/** ADR-0059: a launch that chose launch options signs the ids and, separately, the ids the user confirmed (`directory-action-v3`;
 * vectors shared with the node and the terminal client in schema/fixtures/signing/directory-action-v3.json). */
export function launchMessage(node: string, path: string, kind: string, project: string, separate: boolean, identityChange: boolean, launchOptions: readonly string[], confirmed: readonly string[], nonce: string, expires: number): Uint8Array {
  return new TextEncoder().encode(['directory-action-v3', node, 'launch', path, kind, project, separate ? '1' : '0', identityChange ? '1' : '0', signedList(launchOptions), signedList(confirmed), nonce, String(expires)].join('\0'));
}
export async function signDirectory(device: Device, node: string, operation: 'inspect' | 'launch' | 'list' | 'resume', path: string, kind = '', project = '', option = false, identityChange = false, launchOptions: readonly string[] = [], confirmed: readonly string[] = []) {
  const nonce = toHex(crypto.getRandomValues(new Uint8Array(16)));
  const expiresAtMs = Date.now() + 120_000;
  const message = operation === 'launch' && (launchOptions.length > 0 || confirmed.length > 0)
    ? launchMessage(node, path, kind, project, option, identityChange, launchOptions, confirmed, nonce, expiresAtMs)
    : directoryMessage(node, operation, path, kind, project, option, identityChange, nonce, expiresAtMs);
  const signature = new Uint8Array(await crypto.subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, device.keys.privateKey, message as BufferSource));
  return { deviceId: device.id, nonce, expiresAtMs, signature };
}

/** One answer of `directory.list` (ADR-0053): the canonical folder, its parent (empty at a root) and the sub-folders in it. */
export interface DirectoryListing { path: string; parent: string; entries: { name: string; isGit: boolean }[]; truncated: boolean }
/** The folder inside `base`, spelled with the separator `base` uses (the machine may be Windows). */
export function childPath(base: string, name: string): string {
  const sep = base.includes('\\') && !base.includes('/') ? '\\' : '/';
  return base.endsWith(sep) ? base + name : base + sep + name;
}
/** Why a folder could not be selected (`directory.inspect`), as one of the web's own texts. `auth.forbidden`: the machine limits this
 * device (or the hub this person) to other projects, a lasting refusal and not a bad path. */
export function directoryReasonKey(code: string | undefined): string {
  switch (code) {
    case 'approval.signature_invalid': return 'approval-not-trusted';
    case 'protocol.capability_missing': return 'start-directory-unsupported';
    case 'auth.forbidden': return 'start-directory-forbidden';
    default: return 'start-directory-failed';
  }
}
// ADR-0060: why a Resume or Start again did not happen, as one of the web's own texts.
export function resumeReasonKey(code: string): string {
  switch (code) {
    case 'agent.folder_changed': return 'resume-failed-folder';
    case 'agent.parent_not_running': return 'resume-failed-parent';
    case 'agent.session_missing': return 'resume-failed-session';
    case 'agent.not_interrupted': return 'resume-failed-gone';
    case 'agent.still_running': return 'resume-failed-still-running';
    case 'approval.signature_invalid': return 'approval-not-trusted';
    case 'protocol.capability_missing': return 'resume-failed-unsupported';
    case 'node.offline': return 'resume-failed-offline';
    case 'auth.forbidden': return 'resume-failed-forbidden';
    default: return 'resume-failed';
  }
}
