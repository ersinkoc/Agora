// Where the hub is and how this browser proves itself. The hub serves this app, so its address is our own origin.
// The token arrives once in the URL fragment (`#token=...`, never sent to a server or written to logs), is kept for
// the tab's lifetime, and is removed from the address bar. Real sign-in (passkeys, ADR-0015) replaces this.

export interface HubConnection { url: string; token: string }

const KEY = 'agora.hubToken';

export function readToken(hash: string, store: Pick<Storage, 'getItem' | 'setItem'> | undefined): { token?: string; rest: string } {
  const m = /^#token=([^&]+)(?:&(.*))?$/.exec(hash);
  if (m) {
    const token = decodeURIComponent(m[1] as string);
    try { store?.setItem(KEY, token); } catch { /* private mode: the token lasts until reload */ }
    return { token, rest: m[2] ? `#${m[2]}` : '' };
  }
  let saved: string | null = null;
  try { saved = store?.getItem(KEY) ?? null; } catch { /* unavailable */ }
  return { token: saved ?? undefined, rest: hash };
}

export function hubConnection(): HubConnection | undefined {
  let store: Storage | undefined;
  try { store = window.sessionStorage; } catch { store = undefined; }
  const { token, rest } = readToken(window.location.hash, store);
  if (!token) return undefined;
  if (rest !== window.location.hash) {
    window.history.replaceState(null, '', window.location.pathname + window.location.search + rest);
  }
  const scheme = window.location.protocol === 'https:' ? 'wss' : 'ws';
  return { url: `${scheme}://${window.location.host}/`, token };
}
