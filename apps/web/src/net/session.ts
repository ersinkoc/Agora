// Where the hub is and how this browser proves itself. The hub serves this app, so its address is our own origin.
// Signing in trades a one-time code (`agora hub login`, typed in or given as `#code=...`, a fragment that is never sent
// to a server) for a session token via POST /api/session. The token lives for the tab and expires on the hub (12 h).
// A long-lived token can still be given as `#token=...` for scripts and development. A passkey (net/passkey.ts) signs in
// without a code once one has been added.

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
  try { store = window.localStorage; } catch { store = undefined; }
  const { token, rest } = readToken(window.location.hash, store);
  if (!token) return undefined;
  if (rest !== window.location.hash) {
    window.history.replaceState(null, '', window.location.pathname + window.location.search + rest);
  }
  const scheme = window.location.protocol === 'https:' ? 'wss' : 'ws';
  return { url: `${scheme}://${window.location.host}/`, token };
}

export type SignInResult = { ok: true; token: string } | { ok: false; reason: 'refused' | 'too-many' | 'failed' };

// Trades a one-time code for a session token and remembers it in this browser.
export async function signIn(code: string, fetcher: typeof fetch = fetch): Promise<SignInResult> {
  try {
    const r = await fetcher('/api/session', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ code: code.trim() }),
    });
    if (r.status === 401) return { ok: false, reason: 'refused' };
    if (r.status === 429) return { ok: false, reason: 'too-many' };
    if (!r.ok) return { ok: false, reason: 'failed' };
    const body = (await r.json()) as { token?: unknown };
    if (typeof body.token !== 'string') return { ok: false, reason: 'failed' };
    rememberToken(body.token);
    return { ok: true, token: body.token };
  } catch {
    return { ok: false, reason: 'failed' };
  }
}

// Keeps a session token in this browser (30 days, extended on use; sign out forgets it).
export function rememberToken(token: string): void {
  try { window.localStorage.setItem(KEY, token); } catch { /* private mode: lasts until reload */ }
}

export function forgetToken(): void {
  try { window.localStorage.removeItem(KEY); } catch { /* unavailable */ }
}

// `#code=XXXX-XXXX` in the address: returns the code and the rest of the fragment.
export function readCode(hash: string): { code?: string; rest: string } {
  const m = /^#code=([^&]+)(?:&(.*))?$/.exec(hash);
  if (!m) return { rest: hash };
  return { code: decodeURIComponent(m[1] as string), rest: m[2] ? `#${m[2]}` : '' };
}
