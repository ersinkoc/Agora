import { describe, expect, it } from 'vitest';
import { endSession, readCode, readToken, setPassword, signIn, signInInfo, unlock } from './session';

function store() {
  const m = new Map<string, string>();
  return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) };
}

describe('hub token', () => {
  it('is taken from the fragment, kept, and removed from the address', () => {
    const s = store();
    expect(readToken('#token=ab%2Bc&/agent/x', s)).toEqual({ token: 'ab+c', rest: '#/agent/x' });
    expect(readToken('#/agent/x', s)).toEqual({ token: 'ab+c', rest: '#/agent/x' }); // later loads use the saved one
  });
  it('is absent when nothing was given', () => {
    expect(readToken('', store())).toEqual({ token: undefined, rest: '' });
  });
});

describe('sign-in', () => {
  const reply = (status: number, body?: unknown) => (async () => new Response(body === undefined ? '' : JSON.stringify(body), { status })) as unknown as typeof fetch;
  it('reads a code from the fragment', () => {
    expect(readCode('#code=AB12-CD34&/agent/x')).toEqual({ code: 'AB12-CD34', rest: '#/agent/x' });
    expect(readCode('#/x')).toEqual({ rest: '#/x' });
  });
  it('maps the hub answers', async () => {
    expect(await signIn('x', undefined, reply(401))).toEqual({ ok: false, reason: 'refused' });
    expect(await signIn('x', undefined, reply(429))).toEqual({ ok: false, reason: 'too-many' });
    expect(await signIn('x', undefined, reply(500))).toEqual({ ok: false, reason: 'failed' });
    expect(await signIn('x', undefined, reply(200, {}))).toEqual({ ok: false, reason: 'failed' });
    expect(await signIn('x', undefined, (async () => { throw new Error('offline'); }) as unknown as typeof fetch)).toEqual({ ok: false, reason: 'failed' });
  });
});

describe('screen password', () => {
  const calls: { url: string; init?: RequestInit }[] = [];
  const reply = (status: number, body?: unknown) => (async (url: string, init?: RequestInit) => {
    calls.push({ url, init });
    return new Response(body === undefined ? '' : JSON.stringify(body), { status });
  }) as unknown as typeof fetch;

  it('sends the password with the code only when one was typed', async () => {
    calls.length = 0;
    await signIn(' AB12-CD34 ', 'open sesame', reply(200, { token: 't' }));
    await signIn('AB12-CD34', undefined, reply(200, { token: 't' }));
    expect(JSON.parse(calls[0]!.init!.body as string)).toEqual({ code: 'AB12-CD34', password: 'open sesame' });
    expect(JSON.parse(calls[1]!.init!.body as string)).toEqual({ code: 'AB12-CD34' });
  });

  it('learns from the hub whether a password is needed, and an older hub needs none', async () => {
    expect(await signInInfo(reply(200, { passwordRequired: true, passwordSet: true }))).toEqual({ passwordRequired: true, passwordSet: true });
    expect(await signInInfo(reply(404))).toEqual({ passwordRequired: false, passwordSet: false });
    expect(await signInInfo((async () => new Response('<html>', { status: 200 })) as unknown as typeof fetch)).toEqual({ passwordRequired: false, passwordSet: false });
  });

  it('ends the session at the hub when the window locks', async () => {
    calls.length = 0;
    await endSession('tok', reply(200, {}));
    expect(calls[0]!.url).toBe('/api/session');
    expect(calls[0]!.init!.method).toBe('DELETE');
    expect((calls[0]!.init!.headers as Record<string, string>).authorization).toBe('Bearer tok');
  });

  it('maps unlock and set-password answers', async () => {
    expect(await unlock('t', 'p', reply(200, {}))).toEqual({ ok: true });
    expect(await unlock('t', 'p', reply(401))).toEqual({ ok: false, reason: 'refused' });
    expect(await unlock('t', 'p', reply(429))).toEqual({ ok: false, reason: 'too-many' });
    calls.length = 0;
    expect(await setPassword('t', 'new password', 'old one', reply(200, { revoked: 2 }))).toEqual({ ok: true });
    expect(JSON.parse(calls[0]!.init!.body as string)).toEqual({ password: 'new password', current: 'old one' });
    expect(await setPassword('t', 'short', undefined, reply(400, { code: 'password.too_short' }))).toEqual({ ok: false, reason: 'too-short' });
  });
});
