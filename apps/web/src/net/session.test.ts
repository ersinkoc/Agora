import { describe, expect, it } from 'vitest';
import { readCode, readToken, signIn } from './session';

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
    expect(await signIn('x', reply(401))).toEqual({ ok: false, reason: 'refused' });
    expect(await signIn('x', reply(429))).toEqual({ ok: false, reason: 'too-many' });
    expect(await signIn('x', reply(500))).toEqual({ ok: false, reason: 'failed' });
    expect(await signIn('x', reply(200, {}))).toEqual({ ok: false, reason: 'failed' });
    expect(await signIn('x', (async () => { throw new Error('offline'); }) as unknown as typeof fetch)).toEqual({ ok: false, reason: 'failed' });
  });
});
