// Files of an agent's workspace, as the node returns them (`files.list`, `files.read`). Pure helpers only.

export interface FileItem { name: string; isDir: boolean; size: number; /** ADR-0080: the node keeps this file private by its name. */ secret?: boolean }

// The node refuses to read a secret file with this code (a client that lists the folder never asks; an older one may).
export const SECRET_CODE = 'files.secret';
/** What a failed read is shown as: the private-file message for `files.secret`, the general one otherwise. */
export const readFailure = (e: unknown): 'secret' | 'failed' => (isSecretError(e) ? 'secret' : 'failed');
export const isSecretError = (e: unknown): boolean => typeof e === 'object' && e !== null && (e as { code?: unknown }).code === SECRET_CODE;
export interface WorkspaceFiles {
  list: (path: string) => Promise<FileItem[]>;
  read: (path: string) => Promise<Uint8Array>;
}

// Text for the viewer, or undefined for a file that is not text (a NUL byte, or not valid UTF-8).
export function decodeText(bytes: Uint8Array): string | undefined {
  if (bytes.includes(0)) return undefined;
  try {
    return new TextDecoder('utf-8', { fatal: true }).decode(bytes);
  } catch {
    return undefined;
  }
}

export function joinPath(dir: string, name: string): string {
  return dir ? `${dir}/${name}` : name;
}

export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}
