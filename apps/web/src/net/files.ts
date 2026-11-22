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

// Text for the viewer, or undefined for a file that is not text (a NUL byte, or not valid UTF-8). UTF-16 with a byte-order mark
// (what Windows PowerShell's `>` writes) is text, decoded, by the node's rule for diffs (#940, `agora_core::text::utf16_text`).
export function decodeText(bytes: Uint8Array): string | undefined {
  const utf16 = decodeUtf16(bytes);
  if (utf16 !== null) return utf16;
  if (bytes.includes(0)) return undefined;
  try {
    return new TextDecoder('utf-8', { fatal: true }).decode(bytes);
  } catch {
    return undefined;
  }
}

// Control characters other than a tab, a line break, a carriage return and a form feed (a NUL among them): binary, not text.
// eslint-disable-next-line no-control-regex
const NOT_TEXT = /[\u0000-\u0008\u000b\u000e-\u001f\u007f-\u009f]/;

// UTF-16 with a byte-order mark (FF FE little-endian, FE FF big-endian), without the mark: its text, undefined when it is not
// text (an odd length, a lone surrogate, a control character), or null when there is no mark.
function decodeUtf16(bytes: Uint8Array): string | undefined | null {
  const little = bytes[0] === 0xff && bytes[1] === 0xfe;
  if (!little && !(bytes[0] === 0xfe && bytes[1] === 0xff)) return null;
  if (bytes.length % 2 !== 0) return undefined;
  try {
    // The decoder drops the mark itself.
    const text = new TextDecoder(little ? 'utf-16le' : 'utf-16be', { fatal: true }).decode(bytes);
    return NOT_TEXT.test(text) ? undefined : text;
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
