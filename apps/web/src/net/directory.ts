import { toHex, type Device } from './approval';

export interface DirectoryInfo { path: string; projectId: string; gitRoot: string; branch: string; portable: boolean; previousProjectId?: string }
export function directoryMessage(node: string, operation: string, path: string, kind: string, project: string, option: boolean, identityChange: boolean, nonce: string, expires: number): Uint8Array {
  return new TextEncoder().encode(['directory-action-v1', node, operation, path, kind, project, option ? '1' : '0', identityChange ? '1' : '0', nonce, String(expires)].join('\0'));
}
export async function signDirectory(device: Device, node: string, operation: 'inspect' | 'launch' | 'list', path: string, kind = '', project = '', option = false, identityChange = false) {
  const nonce = toHex(crypto.getRandomValues(new Uint8Array(16)));
  const expiresAtMs = Date.now() + 120_000;
  const message = directoryMessage(node, operation, path, kind, project, option, identityChange, nonce, expiresAtMs);
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
