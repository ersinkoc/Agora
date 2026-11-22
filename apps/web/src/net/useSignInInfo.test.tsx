// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { HubConnection, SignInInfo } from './session';
import { useSignInInfo } from './useSignInInfo';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
beforeEach(() => { host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); });

// What the hub's GET /api/session says right now; the test changes it as the CLI would.
let set = false;
let calls = 0;
const fetcher = (async () => { calls += 1; return new Response(JSON.stringify({ passwordRequired: false, passwordSet: set })); }) as typeof fetch;
let seen: SignInInfo = { passwordRequired: false, passwordSet: false };
let refresh: () => void = () => {};
function Probe({ hub }: { hub: HubConnection | undefined }) {
  [seen, refresh] = useSignInInfo(hub, fetcher);
  return null;
}
const settle = () => act(async () => { for (let i = 0; i < 5; i += 1) await Promise.resolve(); });

describe('whether a screen password is set, as the window knows it (#909)', () => {
  beforeEach(() => { set = false; calls = 0; });

  it('is read again when the window signs in, so a password set while the tab was open shows as set', async () => {
    act(() => root.render(<Probe hub={{ url: 'wss://localhost', token: 'a' }} />));
    await settle();
    expect(seen.passwordSet).toBe(false);
    // `agora password set` on the hub's machine: every session ends, the window shows the sign-in page, then signs in again.
    set = true;
    act(() => root.render(<Probe hub={undefined} />));
    await settle();
    act(() => root.render(<Probe hub={{ url: 'wss://localhost', token: 'b' }} />));
    await settle();
    expect(seen.passwordSet).toBe(true);
  });

  it('is read again on request, and not again when nothing changed', async () => {
    const hub = { url: 'wss://localhost', token: 'a' };
    act(() => root.render(<Probe hub={hub} />));
    await settle();
    act(() => root.render(<Probe hub={hub} />));
    await settle();
    expect(calls).toBe(1);
    set = true;
    act(() => refresh());
    await settle();
    expect(seen.passwordSet).toBe(true);
  });
});
