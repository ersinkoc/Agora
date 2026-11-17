// A program can put a link on text (OSC 8) and the terminal finds addresses in plain output. What a click opens is decided here: only
// web addresses, and never with the opener's access to this window.

const MAX = 2048;

/** The address to open, or `undefined` when it is not a plain web address (javascript:, data:, file:, an address with a password...). */
export function safeLink(uri: string): string | undefined {
  if (!uri || uri.length > MAX) return undefined;
  let u: URL;
  try { u = new URL(uri); } catch { return undefined; }
  if (u.protocol !== 'http:' && u.protocol !== 'https:') return undefined;
  if (u.username || u.password) return undefined;
  return u.href;
}

export const LINK_FEATURES = 'noopener,noreferrer';

export function openLink(uri: string): void {
  const href = safeLink(uri);
  if (href) window.open(href, '_blank', LINK_FEATURES);
}
