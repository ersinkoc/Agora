import { describe, expect, it } from 'vitest';
import { readToken } from './session';

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
