// @vitest-environment jsdom
// Click-driven tests of the phone's bar for what needs you: they run the real screen in a DOM and press its buttons, so a change that
// lets a request to start an agent, or a machine asking to join, be answered from the bar, or that lets an answer go out while the link
// is down or twice at once, is caught here.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage } from '../i18n';
import type { AgentTileModel } from '../model';
import { demoStartApproval } from '../fixtures';
import type { ApprovalModel } from '../net/view';
import type { PairingUi } from '../components/Pairing';
import { PhoneAgent, PhoneHome } from './Phone';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); });

const agent: AgentTileModel = { id: 'n1/a1', name: 'claude · amber', accent: '', machine: 'laptop', machineId: 'n1', workspace: 'w', project: 'login-fix', state: 'needs-you', preview: '' };
const ask = (id: string, over: Partial<ApprovalModel> = {}): ApprovalModel =>
  ({ id: `n1/${id}`, node: 'n1', localId: id, agentId: 'n1/a1', agentName: 'amber', action: 'Edit src/auth.rs', detail: '+12 -3', expiresAtMs: 0, ...over });
const machines = [{ id: 'n1', name: 'laptop', os: 'linux', online: true, agentsRunning: 1, lastSeenMs: 0 }];
const dock = () => host.querySelector('.phone-dock') as HTMLElement | null;
const press = (scope: ParentNode, text: string) => {
  const b = [...scope.querySelectorAll('button')].find((x) => x.textContent === text) as HTMLButtonElement | undefined;
  if (!b) throw new Error(`no button ${text}`);
  act(() => { b.click(); });
  return b;
};
const home = (approvals: ApprovalModel[], over: Partial<Parameters<typeof PhoneHome>[0]> = {}) => {
  const onResolve = vi.fn(async () => undefined);
  act(() => root.render(<PhoneHome agents={[agent]} machines={machines} approvals={approvals} onResolve={onResolve} link="live" onOpen={() => undefined} {...over} />));
  return onResolve;
};

describe('the phone bar for what needs you', () => {
  it('answers a short plain request with one press, and the card does not repeat the buttons', () => {
    const r = ask('p1');
    const onResolve = home([r]);
    expect(dock()?.textContent).toContain('Edit src/auth.rs · +12 -3');
    expect(host.querySelectorAll('button.button--primary').length).toBe(1);
    press(dock()!, 'Allow');
    expect(onResolve).toHaveBeenCalledWith(r, true, undefined);
    press(dock()!, 'Deny');
  });
  it('a second press waits for the first', async () => {
    let done: () => void = () => undefined;
    const onResolve = vi.fn(() => new Promise<void>((res) => { done = res; }));
    act(() => root.render(<PhoneHome agents={[agent]} machines={machines} approvals={[ask('p1')]} onResolve={onResolve} link="live" onOpen={() => undefined} />));
    press(dock()!, 'Allow');
    press(dock()!, 'Allow');
    press(dock()!, 'Deny');
    expect(onResolve).toHaveBeenCalledTimes(1);
    await act(async () => { done(); });
  });
  it('never offers Allow or Deny in the bar for a request to start an agent; its own card answers it', () => {
    const onResolve = home([demoStartApproval as ApprovalModel]);
    const d = dock()!;
    expect([...d.querySelectorAll('button')].map((b) => b.textContent)).toEqual(['Review and decide']);
    press(d, 'Review and decide');
    expect(onResolve).not.toHaveBeenCalled();
  });
  it('does not answer long, multi-line or hidden-character text from the bar', () => {
    for (const over of [{ detail: 'x'.repeat(200) }, { detail: 'a\nb' }, { action: 'rm‮ -rf' }]) {
      home([ask('p9', over)]);
      expect([...dock()!.querySelectorAll('button')].map((b) => b.textContent)).toEqual(['Review and decide']);
    }
  });
  it('is disabled with its reason while the link is down, and sends nothing', () => {
    const onResolve = home([ask('p1')], { link: 'offline' });
    const d = dock()!;
    expect(d.textContent).toContain('Cannot answer while the connection is down.');
    for (const b of d.querySelectorAll('button')) { expect((b as HTMLButtonElement).disabled).toBe(true); (b as HTMLButtonElement).click(); }
    expect(onResolve).not.toHaveBeenCalled();
    // The card in the list is held back the same way.
    expect([...host.querySelectorAll('.phone-card button')].filter((b) => /^(Allow|Deny)$/.test(b.textContent ?? '')).every((b) => (b as HTMLButtonElement).disabled)).toBe(true);
  });
  it('a machine that asks to join is only reviewed from the bar, never approved', () => {
    const answers: unknown[] = [];
    const pairing: PairingUi = { cards: [{ id: 'aa', kind: 'machine', name: 'new-box', os: 'linux', address: '10.0.0.9', number: '482193', expiresMs: Date.now() + 300_000 }], recent: [], offline: false, onAnswer: async (...a: unknown[]) => { answers.push(a); return 'ok'; }, onWindow: async () => true } as unknown as PairingUi;
    home([], { agents: [{ ...agent, state: 'working' }], pairing });
    expect([...dock()!.querySelectorAll('button')].map((b) => b.textContent)).toEqual(['Review and decide']);
    press(dock()!, 'Review and decide');
    expect(answers).toEqual([]);
  });
  it('on an agent, the bar answers its first plain request and the top card keeps no second pair of buttons', () => {
    const r = ask('p1');
    const onResolve = vi.fn(async () => undefined);
    act(() => root.render(<PhoneAgent agent={agent} agents={[agent]} approvals={[r]} machines={machines} onResolve={onResolve} onOpen={() => undefined} onBack={() => undefined} />));
    expect(host.querySelectorAll('button.button--primary').length).toBe(1);
    press(dock()!, 'Allow');
    expect(onResolve).toHaveBeenCalledTimes(1);
  });
});
