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
  try { store = window.sessionStorage; } catch { store = undefined; }
  const { token, rest } = readToken(window.location.hash, store);
  if (!token) return undefined;
  if (rest !== window.location.hash) {
    window.history.replaceState(null, '', window.location.pathname + window.location.search + rest);
  }
  const scheme = window.location.protocol === 'https:' ? 'wss' : 'ws';
  return { url: `${scheme}://${window.location.host}/`, token };
}

export type SignInResult = { ok: true; token: string } | { ok: false; reason: 'refused' | 'too-many' | 'failed' };

// Trades a one-time code (and, from another machine, the screen password: ADR-0067) for a session token and remembers it for this tab.
export async function signIn(code: string, password?: string, fetcher: typeof fetch = fetch): Promise<SignInResult> {
  try {
    const r = await fetcher('/api/session', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(password ? { code: code.trim(), password } : { code: code.trim() }),
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

// Keeps a session token for this tab.
export function rememberToken(token: string): void {
  try { window.sessionStorage.setItem(KEY, token); } catch { /* private mode: lasts until reload */ }
}

export function forgetToken(): void {
  try { window.sessionStorage.removeItem(KEY); } catch { /* unavailable */ }
}

// `#code=XXXX-XXXX` in the address: returns the code and the rest of the fragment.
export function readCode(hash: string): { code?: string; rest: string } {
  const m = /^#code=([^&]+)(?:&(.*))?$/.exec(hash);
  if (!m) return { rest: hash };
  return { code: decodeURIComponent(m[1] as string), rest: m[2] ? `#${m[2]}` : '' };
}

// What the sign-in screen needs to know before it asks: does this hub want the screen password from here (it never does from its own
// machine), and is one set at all. An older hub answers with the page itself or nothing: then no password is asked.
export interface SignInInfo { passwordRequired: boolean; passwordSet: boolean }

export async function signInInfo(fetcher: typeof fetch = fetch): Promise<SignInInfo> {
  try {
    const r = await fetcher('/api/session', { method: 'GET', headers: { accept: 'application/json' } });
    if (!r.ok) return { passwordRequired: false, passwordSet: false };
    const body = (await r.json()) as { passwordRequired?: unknown; passwordSet?: unknown };
    return { passwordRequired: body.passwordRequired === true, passwordSet: body.passwordSet === true };
  } catch {
    return { passwordRequired: false, passwordSet: false };
  }
}

// Ends this session at the hub ("Lock" in the window, or the idle lock) and forgets it here. Agents are not touched.
export async function endSession(token: string, fetcher: typeof fetch = fetch): Promise<void> {
  forgetToken();
  try {
    await fetcher('/api/session', { method: 'DELETE', headers: { authorization: `Bearer ${token}` } });
  } catch { /* the hub is unreachable: the session ends when it expires */ }
}

export type PasswordResult = { ok: true } | { ok: false; reason: 'refused' | 'too-many' | 'too-short' | 'failed' };

const passwordReply = async (r: Response): Promise<PasswordResult> => {
  if (r.ok) return { ok: true };
  if (r.status === 401) return { ok: false, reason: 'refused' };
  if (r.status === 429) return { ok: false, reason: 'too-many' };
  if (r.status === 400) return { ok: false, reason: 'too-short' };
  return { ok: false, reason: 'failed' };
};

// Sets or changes the screen password (owner only; `current` when one is set). Every other session of the hub ends.
export async function setPassword(token: string, password: string, current?: string, fetcher: typeof fetch = fetch): Promise<PasswordResult> {
  try {
    return await passwordReply(await fetcher('/api/password', {
      method: 'POST',
      headers: { 'content-type': 'application/json', authorization: `Bearer ${token}` },
      body: JSON.stringify(current ? { password, current } : { password }),
    }));
  } catch {
    return { ok: false, reason: 'failed' };
  }
}

// The shortest screen password the hub accepts (crates/agora-hub/src/password.rs MIN_CHARS).
export const MIN_PASSWORD = 8;

// A `#code=` link: sign in at once, unless the hub wants the screen password from this browser; then the code waits in the form
// (`keep`) for the password, and is neither sent nor counted as a miss.
export async function signInFromLink(code: string, info: () => Promise<SignInInfo>, submit: (code: string) => void, keep: (code: string) => void): Promise<void> {
  if ((await info()).passwordRequired) keep(code);
  else submit(code);
}
