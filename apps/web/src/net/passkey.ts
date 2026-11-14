// Passkeys (WebAuthn, ADR-0015): add one while signed in, then sign in with it instead of a code. The hub keeps only the
// public key; the private key stays in this device's authenticator. The browser and the hub do the checking: this file
// only carries the messages between them.
import { rememberToken } from './session';

export function b64urlEncode(bytes: ArrayBuffer | Uint8Array): string {
  const u = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  let s = '';
  for (const b of u) s += String.fromCharCode(b);
  return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

export function b64urlDecode(text: string): Uint8Array<ArrayBuffer> {
  const s = atob(text.replace(/-/g, '+').replace(/_/g, '/'));
  const out = new Uint8Array(new ArrayBuffer(s.length));
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i);
  return out;
}

export function passkeysSupported(): boolean {
  return typeof window !== 'undefined' && 'PublicKeyCredential' in window && !!navigator.credentials;
}

export type PasskeyResult = { ok: true; token?: string } | { ok: false; reason: 'too-many' | 'failed' | 'cancelled' };

interface Env { fetcher: typeof fetch; credentials: Pick<CredentialsContainer, 'create' | 'get'> }

const defaults = (): Env => ({ fetcher: (...a) => fetch(...a), credentials: navigator.credentials });

async function post(env: Env, op: string, body: unknown, bearer?: string): Promise<Response> {
  return env.fetcher(`/api/passkey/${op}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', ...(bearer ? { authorization: `Bearer ${bearer}` } : {}) },
    body: JSON.stringify(body),
  });
}

// What the authenticator says when the user dismisses its prompt.
const cancelled = (e: unknown) => e instanceof DOMException && (e.name === 'NotAllowedError' || e.name === 'AbortError');

// Adds a passkey for this hub. `session` is the token of the signed-in session.
// `password`: the screen password, which the hub asks for when this browser is on another machine (ADR-0067): a passkey replaces the
// password, so a session alone may not add one from elsewhere.
export async function addPasskey(session: string, env: Env = defaults(), password?: string): Promise<PasskeyResult> {
  const pw = password ? { password } : {};
  try {
    const begin = await post(env, 'register-begin', pw, session);
    if (!begin.ok) return { ok: false, reason: 'failed' };
    const { challenge, rpId } = (await begin.json()) as { challenge: string; rpId: string };
    const cred = (await env.credentials.create({
      publicKey: {
        challenge: b64urlDecode(challenge),
        rp: { name: 'Agora', id: rpId },
        user: { id: crypto.getRandomValues(new Uint8Array(16)), name: 'owner', displayName: 'Owner' },
        pubKeyCredParams: [{ type: 'public-key', alg: -7 }],
        authenticatorSelection: { residentKey: 'required', userVerification: 'required' },
        attestation: 'none',
      },
    })) as PublicKeyCredential | null;
    const response = cred?.response as AuthenticatorAttestationResponse | undefined;
    const key = response?.getPublicKey?.();
    if (!cred || !response || !key) return { ok: false, reason: 'failed' };
    const done = await post(env, 'register-finish', {
      id: cred.id,
      publicKey: b64urlEncode(key),
      clientDataJSON: b64urlEncode(response.clientDataJSON),
      ...pw,
    }, session);
    return done.ok ? { ok: true } : { ok: false, reason: 'failed' };
  } catch (e) {
    return { ok: false, reason: cancelled(e) ? 'cancelled' : 'failed' };
  }
}

// Signs in with a passkey; the session token is remembered for this tab like a code sign-in's.
export async function signInWithPasskey(env: Env = defaults()): Promise<PasskeyResult> {
  try {
    const begin = await post(env, 'login-begin', {});
    if (!begin.ok) return { ok: false, reason: 'failed' };
    const { challenge, rpId, ids } = (await begin.json()) as { challenge: string; rpId: string; ids: string[] };
    if (ids.length === 0) return { ok: false, reason: 'failed' };
    const cred = (await env.credentials.get({
      publicKey: {
        challenge: b64urlDecode(challenge),
        rpId,
        allowCredentials: ids.map((id) => ({ type: 'public-key' as const, id: b64urlDecode(id) })),
        userVerification: 'required',
      },
    })) as PublicKeyCredential | null;
    const r = cred?.response as AuthenticatorAssertionResponse | undefined;
    if (!cred || !r) return { ok: false, reason: 'failed' };
    const done = await post(env, 'login-finish', {
      id: cred.id,
      authenticatorData: b64urlEncode(r.authenticatorData),
      clientDataJSON: b64urlEncode(r.clientDataJSON),
      signature: b64urlEncode(r.signature),
    });
    if (done.status === 429) return { ok: false, reason: 'too-many' };
    if (!done.ok) return { ok: false, reason: 'failed' };
    const body = (await done.json()) as { token?: unknown };
    if (typeof body.token !== 'string') return { ok: false, reason: 'failed' };
    rememberToken(body.token);
    return { ok: true, token: body.token };
  } catch (e) {
    return { ok: false, reason: cancelled(e) ? 'cancelled' : 'failed' };
  }
}
