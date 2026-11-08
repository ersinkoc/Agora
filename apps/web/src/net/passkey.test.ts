import { describe, expect, it } from 'vitest';
import { addPasskey, b64urlDecode, b64urlEncode, signInWithPasskey } from './passkey';

const bytes = (...n: number[]) => new Uint8Array(n).buffer;
const reply = (status: number, body: unknown = {}) => new Response(JSON.stringify(body), { status });

describe('passkey', () => {
  it('encodes base64url without padding and round-trips', () => {
    expect(b64urlEncode(bytes(251, 255, 254))).toBe('-__-');
    expect(Array.from(b64urlDecode('-__-'))).toEqual([251, 255, 254]);
    expect(b64urlEncode(bytes(1))).toBe('AQ');
  });

  it('adds a passkey: asks the hub for a challenge, creates the credential, sends the public key', async () => {
    const sent: { op: string; body: Record<string, string>; auth?: string }[] = [];
    const fetcher = (async (url: string, init: RequestInit) => {
      const op = String(url).split('/').pop() as string;
      sent.push({ op, body: JSON.parse(init.body as string), auth: (init.headers as Record<string, string>).authorization });
      return op === 'register-begin' ? reply(200, { challenge: 'AQID', rpId: 'hub.test' }) : reply(200);
    }) as unknown as typeof fetch;
    const credentials = {
      create: async () => ({ id: 'cred_1', response: { getPublicKey: () => bytes(9, 9), clientDataJSON: bytes(7) } }),
      get: async () => null,
    } as unknown as CredentialsContainer;
    const r = await addPasskey('sess', { fetcher, credentials });
    expect(r).toEqual({ ok: true });
    expect(sent.map((s) => s.op)).toEqual(['register-begin', 'register-finish']);
    expect(sent[1]?.auth).toBe('Bearer sess');
    expect(sent[1]?.body).toMatchObject({ id: 'cred_1', publicKey: 'CQk', clientDataJSON: 'Bw' });
  });

  it('signs in and keeps the token; a dismissed prompt is "cancelled", too many tries is reported', async () => {
    const credential = { id: 'cred_1', response: { authenticatorData: bytes(1), clientDataJSON: bytes(2), signature: bytes(3) } };
    const fetcher = (async (url: string) =>
      String(url).endsWith('login-begin') ? reply(200, { challenge: 'AQID', rpId: 'hub.test', ids: ['cred_1'] }) : reply(200, { token: 'tok' })) as unknown as typeof fetch;
    const ok = await signInWithPasskey({ fetcher, credentials: { create: async () => null, get: async () => credential } as unknown as CredentialsContainer });
    expect(ok).toEqual({ ok: true, token: 'tok' });

    const dismissed = await signInWithPasskey({ fetcher, credentials: { create: async () => null, get: async () => { throw new DOMException('no', 'NotAllowedError'); } } as unknown as CredentialsContainer });
    expect(dismissed).toEqual({ ok: false, reason: 'cancelled' });

    const limited = (async (url: string) =>
      String(url).endsWith('login-begin') ? reply(200, { challenge: 'AQID', rpId: 'h', ids: ['Yw'] }) : reply(429)) as unknown as typeof fetch;
    expect(await signInWithPasskey({ fetcher: limited, credentials: { create: async () => null, get: async () => credential } as unknown as CredentialsContainer }))
      .toEqual({ ok: false, reason: 'too-many' });
  });

  it('does not ask the authenticator when the hub knows no passkey', async () => {
    const fetcher = (async () => reply(200, { challenge: 'AQID', rpId: 'h', ids: [] })) as unknown as typeof fetch;
    const credentials = { create: async () => null, get: async () => { throw new Error('should not be called'); } } as unknown as CredentialsContainer;
    expect(await signInWithPasskey({ fetcher, credentials })).toEqual({ ok: false, reason: 'failed' });
  });
});
