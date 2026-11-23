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
import { DOCK_DWELL_MS, PhoneAgent, PhoneHome } from './Phone';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
// These tests are about request cards, not terminals: a real terminal (xterm, loaded lazily) needs browser APIs jsdom lacks
// (matchMedia, canvas), and whether its lazy import resolves inside a test depends on how warm the module cache is (#1052's CI).
vi.mock('../terminal/TerminalView', () => ({ TerminalView: () => null }));

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); vi.useFakeTimers(); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); vi.useRealTimers(); });

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
const dwell = (ms = DOCK_DWELL_MS) => act(async () => { await vi.advanceTimersByTimeAsync(ms); });
const home = (approvals: ApprovalModel[], over: Partial<Parameters<typeof PhoneHome>[0]> = {}) => {
  const onResolve = vi.fn(async () => undefined);
  act(() => root.render(<PhoneHome agents={[agent]} machines={machines} approvals={approvals} onResolve={onResolve} link="live" onOpen={() => undefined} {...over} />));
  return onResolve;
};

describe('a switch to a mode that skips permissions on the phone', () => {
  it('is read in the node\'s own rendering, never as raw JSON, and the bar never answers it', async () => {
    const { markSkipCapable, forgetSkipCapable } = await import('../net/skipApproval');
    const detail = JSON.stringify({ node: 'n1', agent: 'a1', agent_kind: 'claude-acp', folder: '/home/dev/p', option_type: 'mode', option: 'mode', option_name: '', value: 'yolo', value_name: 'Bypass', skips: true, scope: 'session' });
    const r = ask('s1', { action: 'agent.skip_permissions', detail, skip: { node: 'n1', agent: 'a1', agentKind: 'claude-acp', folder: '/home/dev/p', optionType: 'mode', option: 'mode', optionName: '', value: 'yolo', valueName: 'Bypass' } });
    markSkipCapable('n1');
    const onResolve = home([r]);
    await dwell(DOCK_DWELL_MS + 700);
    expect(host.textContent).toContain('act without asking?');
    expect(host.textContent).toContain('/home/dev/p');
    expect(host.textContent).not.toContain('"option_type"');
    expect(dock()?.textContent ?? '').not.toContain('Allow'); // the bar never answers it
    press(host, 'Allow');
    expect(onResolve.mock.calls.map((c) => c.slice(0, 2))).toEqual([[r, true]]);
    forgetSkipCapable();
  });
});

describe('the phone bar for what needs you', () => {
  it('answers a short plain request with one press, and the card does not repeat the buttons', async () => {
    const r = ask('p1');
    const onResolve = home([r]);
    await dwell();
    expect(dock()?.textContent).toContain('Edit src/auth.rs · +12 -3');
    expect(host.querySelectorAll('button.kit-btn--primary').length).toBe(1);
    press(dock()!, 'Allow');
    expect(onResolve).toHaveBeenCalledWith(r, true);
    press(dock()!, 'Deny');
  });
  it('a second press waits for the first', async () => {
    let done: () => void = () => undefined;
    const onResolve = vi.fn(() => new Promise<void>((res) => { done = res; }));
    act(() => root.render(<PhoneHome agents={[agent]} machines={machines} approvals={[ask('p1')]} onResolve={onResolve} link="live" onOpen={() => undefined} />));
    await dwell();
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
  it('on an agent, the bar answers its first plain request and the top card keeps no second pair of buttons', async () => {
    const r = ask('p1');
    const onResolve = vi.fn(async () => undefined);
    act(() => root.render(<PhoneAgent agent={agent} agents={[agent]} approvals={[r]} machines={machines} onResolve={onResolve} onOpen={() => undefined} onBack={() => undefined} />));
    expect(host.querySelectorAll('button.kit-btn--primary').length).toBe(1);
    await dwell();
    press(dock()!, 'Allow');
    expect(onResolve).toHaveBeenCalledTimes(1);
  });

  describe('when the request on the bar changes', () => {
    const A = ask('A');
    const B = ask('B', { action: 'Run sudo reboot', detail: '' });
    const show = (approvals: ApprovalModel[], onResolve: (a: ApprovalModel, ok: boolean) => unknown) =>
      act(() => root.render(<PhoneHome agents={[agent]} machines={machines} approvals={approvals} onResolve={onResolve} link="live" onOpen={() => undefined} />));
    it('a tap right after the swap answers nothing, and the new request shows its buttons disabled', async () => {
      const onResolve = vi.fn(async () => undefined);
      show([A, B], onResolve);
      await dwell();
      const old = [...dock()!.querySelectorAll('button')].find((b) => b.textContent === 'Allow') as HTMLButtonElement;
      show([B], onResolve);
      expect(dock()!.textContent).toContain('Run sudo reboot');
      expect(dock()!.querySelector('button.kit-btn--primary')?.getAttribute('aria-disabled')).toBe('true');
      act(() => { old.click(); });
      press(dock()!, 'Allow');
      expect(onResolve).not.toHaveBeenCalled();
    });
    it('a tap after the dwell answers the new request, with its own id', async () => {
      const onResolve = vi.fn(async () => undefined);
      show([A, B], onResolve);
      await dwell();
      show([B], onResolve);
      await dwell(DOCK_DWELL_MS - 1);
      press(dock()!, 'Allow');
      expect(onResolve).not.toHaveBeenCalled();
      await dwell(1);
      press(dock()!, 'Allow');
      expect(onResolve).toHaveBeenCalledTimes(1);
      expect((onResolve.mock.calls[0] as unknown[])[0]).toBe(B);
    });
    it('a tap on A and a second one on B within the window answers only A', async () => {
      let done: () => void = () => undefined;
      const onResolve = vi.fn(() => new Promise<void>((res) => { done = res; }));
      show([A, B], onResolve);
      await dwell();
      press(dock()!, 'Allow');
      show([B], onResolve);
      await dwell();
      press(dock()!, 'Allow');
      press(dock()!, 'Deny');
      expect(onResolve).toHaveBeenCalledTimes(1);
      expect((onResolve.mock.calls[0] as unknown[])[0]).toBe(A);
      await act(async () => { done(); });
    });
  });
  describe('the dwell starts again after any interlude', () => {
    const A = ask('A');
    const B = ask('B', { action: 'Run ls', detail: '' });
    const R = ask('R', { detail: 'x'.repeat(200) });
    const show = (approvals: ApprovalModel[], onResolve: (a: ApprovalModel, ok: boolean) => unknown) =>
      act(() => root.render(<PhoneHome agents={[agent]} machines={machines} approvals={approvals} onResolve={onResolve} link="live" onOpen={() => undefined} />));
    it('A ready, a review item shown for a while, then A again: a tap at once answers nothing, one after the dwell answers A', async () => {
      const onResolve = vi.fn(async () => undefined);
      show([A], onResolve);
      await dwell();
      show([R, A], onResolve);
      await dwell(1500);
      show([A], onResolve);
      press(dock()!, 'Allow');
      press(dock()!, 'Deny');
      expect(onResolve).not.toHaveBeenCalled();
      await dwell();
      press(dock()!, 'Allow');
      expect(onResolve).toHaveBeenCalledTimes(1);
      expect((onResolve.mock.calls[0] as unknown[])[0]).toBe(A);
    });
    it('A, B, A within the dwell: a tap 50 ms after A came back answers nothing', async () => {
      const onResolve = vi.fn(async () => undefined);
      show([A], onResolve);
      await dwell();
      show([B], onResolve);
      await dwell(100);
      show([A], onResolve);
      await dwell(50);
      press(dock()!, 'Allow');
      expect(onResolve).not.toHaveBeenCalled();
      await dwell();
      press(dock()!, 'Allow');
      expect(onResolve).toHaveBeenCalledTimes(1);
    });
    it('Deny waits out the dwell too, and after it sends a deny with the id on screen', async () => {
      const onResolve = vi.fn(async () => undefined);
      show([A], onResolve);
      await dwell(DOCK_DWELL_MS - 1);
      press(dock()!, 'Deny');
      expect(onResolve).not.toHaveBeenCalled();
      await dwell(1);
      press(dock()!, 'Deny');
      expect(onResolve).toHaveBeenCalledTimes(1);
      expect(onResolve.mock.calls[0]).toEqual([A, false]);
    });
  });
  describe('what a screen reader and the keyboard get', () => {
    it('is a polite status (never an alert), and its reason is inside it', () => {
      home([ask('p1')], { link: 'offline' });
      const d = dock()!;
      expect(d.getAttribute('role')).toBe('status');
      expect(d.getAttribute('aria-live')).toBe('polite');
      expect(d.textContent).toContain('Cannot answer while the connection is down.');
      expect(d.querySelector('[role=alert]')).toBeNull();
    });
    it('moves a focus that was on the request that went to the bar, not to the page; takes no focus otherwise', async () => {
      const r = vi.fn(async () => undefined);
      const show = (a: ApprovalModel[]) => act(() => root.render(<PhoneHome agents={[agent]} machines={machines} approvals={a} onResolve={r} link="live" onOpen={() => undefined} />));
      show([ask('A'), ask('B', { action: 'Run ls' })]);
      await dwell();
      const allow = [...dock()!.querySelectorAll('button')].find((b) => b.textContent === 'Allow') as HTMLButtonElement;
      act(() => { allow.focus(); });
      expect(document.activeElement).toBe(allow);
      show([ask('B', { action: 'Run ls' })]);
      expect(document.activeElement).toBe(dock());
      show([ask('C', { action: 'Run pwd' })]);
      // Focus elsewhere: a new request does not take it.
      const other = document.createElement('input'); document.body.append(other);
      act(() => { other.focus(); });
      show([ask('D', { action: 'Run id' })]);
      expect(document.activeElement).toBe(other);
      other.remove();
    });
  });
  it('writes the agent and machine names in the bar with hidden characters visible, and the machine the agent header names', () => {
    home([ask('p1')], { agents: [{ ...agent, name: 'claude\u202e amber', machine: 'Laptop' }] });
    const head = dock()!.textContent!;
    expect(head).toContain('claude\\u{202e} amber');
    expect(head).toContain('Laptop');
    expect(head).not.toContain('n1');
  });
  it('shows one notice of the offline link, not two banners', () => {
    home([ask('p1')], { link: 'offline', onRetry: () => undefined });
    expect(host.querySelectorAll('[role=status]:not(.phone-dock)').length).toBe(1);
    expect(host.textContent).not.toContain('Cannot reach your Agora');
  });
});
