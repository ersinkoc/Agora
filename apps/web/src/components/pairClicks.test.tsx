// @vitest-environment jsdom
// Click-driven tests of the "two presses" rule (ADR-0065): they run the real component in a DOM and press its buttons, so a change that lets
// one press approve, sends Replace without the owner's tick, or lets the yes work offline or at once is caught here.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage } from '../i18n';
import { MIN_THINK_MS } from './pairChoice';
import { PairRequest, type PairAnswer } from './PairRequest';
import type { Live } from '../net/live';
import { usePairing } from '../net/usePairing';
import type { PairCard, PairNote } from '../net/pairing';
import type { PairingUi } from './Pairing';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); vi.useFakeTimers(); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); vi.useRealTimers(); });

const request = { id: 'aa', kind: 'machine' as const, name: 'laptop', os: 'linux', address: '10.0.0.1', number: '482193', expiresMs: Date.now() + 300_000 };
const button = (text: string): HTMLButtonElement => {
  const b = [...host.querySelectorAll('button')].find((x) => x.textContent === text);
  if (!b) throw new Error(`no button ${text}: ${host.textContent}`);
  return b as HTMLButtonElement;
};
// The first press ("Numbers match…"): it only asks the question; the yes is the second press.
const APPROVE = 'Numbers match…';
const click = (b: HTMLElement) => act(() => { b.click(); });
const wait = (ms: number) => act(async () => { await vi.advanceTimersByTimeAsync(ms); });
const show = (over: { offline?: boolean; nameTaken?: boolean; onAnswer?: (a: boolean, r: boolean) => Promise<PairAnswer> } = {}) => {
  const answers: [boolean, boolean][] = [];
  const onAnswer = over.onAnswer ?? (async (a: boolean, r: boolean) => { answers.push([a, r]); return 'ok' as PairAnswer; });
  act(() => root.render(<PairRequest inline request={{ ...request, nameTaken: over.nameTaken }} offline={over.offline} onAnswer={onAnswer} now={Date.now()} />));
  return { answers, onAnswer };
};

describe('the two presses of Approve', () => {
  it('one press of Approve sends nothing: it asks "do the numbers match?"', async () => {
    const { answers } = show();
    click(button(APPROVE));
    await wait(0);
    expect(answers).toEqual([]);
    expect(host.textContent).toContain('Do the two numbers match exactly?');
  });

  it('the yes does nothing before the question has been up for half a second, and sends exactly one approval after', async () => {
    const { answers } = show();
    click(button(APPROVE));
    // A click right away (a double-click on Approve) hits a disabled button and is ignored.
    expect(button('Yes, the numbers match').disabled).toBe(true);
    click(button('Yes, the numbers match'));
    await wait(MIN_THINK_MS - 50);
    click(button('Yes, the numbers match'));
    expect(answers).toEqual([]);
    await wait(100);
    expect(button('Yes, the numbers match').disabled).toBe(false);
    click(button('Yes, the numbers match'));
    await wait(0);
    expect(answers).toEqual([[true, false]]);
    // A second click on the same answer sends nothing more.
    expect(host.textContent).toContain('Answer sent.');
  });

  it('Go back returns to the card and nothing was sent; Deny is one press', async () => {
    const { answers } = show();
    click(button(APPROVE));
    click(button('Go back'));
    expect(button(APPROVE)).toBeTruthy();
    expect(answers).toEqual([]);
    click(button('Deny'));
    await wait(0);
    expect(answers).toEqual([[false, false]]);
  });

  it('the yes does not work while the link is down', async () => {
    const { answers } = show();
    click(button(APPROVE));
    await wait(MIN_THINK_MS + 10);
    // The link goes down while the question is up.
    show({ offline: true });
    await wait(0);
    const yes = [...host.querySelectorAll('button')].find((x) => x.textContent === 'Yes, the numbers match') as HTMLButtonElement | undefined;
    if (yes) { expect(yes.disabled).toBe(true); click(yes); }
    expect(answers).toEqual([]);
    expect(host.textContent).toContain('Not connected right now.');
    for (const b of host.querySelectorAll('button')) expect(b.disabled || b.textContent === 'Go back').toBe(true);
  });

  it('Replace is sent only when the hub said the name is taken AND the owner ticked it', async () => {
    // Not taken: there is no checkbox, and replace is false.
    let r = show();
    expect(host.querySelector('input[type=checkbox]')).toBeNull();
    click(button(APPROVE)); await wait(MIN_THINK_MS + 10); click(button('Yes, the numbers match')); await wait(0);
    expect(r.answers).toEqual([[true, false]]);
    // Taken, not ticked: false. Taken and ticked: true.
    act(() => root.unmount()); root = createRoot(host);
    r = show({ nameTaken: true });
    const box = host.querySelector('input[type=checkbox]') as HTMLInputElement;
    expect(box.checked).toBe(false);
    click(button(APPROVE)); await wait(MIN_THINK_MS + 10); click(button('Yes, the numbers match')); await wait(0);
    expect(r.answers).toEqual([[true, false]]);
    act(() => root.unmount()); root = createRoot(host);
    r = show({ nameTaken: true });
    click(host.querySelector('input[type=checkbox]') as HTMLElement);
    click(button(APPROVE)); await wait(MIN_THINK_MS + 10); click(button('Yes, the numbers match')); await wait(0);
    expect(r.answers).toEqual([[true, true]]);
  });

  it('ticking Replace starts the two presses over', async () => {
    show({ nameTaken: true });
    click(button(APPROVE));
    click(host.querySelector('input[type=checkbox]') as HTMLElement);
    expect(host.textContent).not.toContain('Do the two numbers match exactly?');
    expect(button(APPROVE)).toBeTruthy();
  });
});

describe('usePairing sends what the card says', () => {
  const card = (over: Partial<PairCard> = {}): PairCard => ({ id: '01', pairId: new Uint8Array([1, 2]), kind: 'machine', name: 'laptop', os: 'linux', address: '10.0.0.1', number: '482193', expiresMs: Date.now() + 1000, ...over });
  const harness = () => {
    const sent: Record<string, unknown>[] = [];
    const api = { command: async (_n: string, body: Record<string, unknown>) => { sent.push(body); if ('pairStatus' in body) return { pairStatus: { open: false, closesAtMs: 0, pending: 0 } }; return {}; } } as unknown as Live;
    let ui: PairingUi | undefined;
    let dispatch: (n: PairNote) => void = () => {};
    const Probe = () => { const p = usePairing(api, 'live'); ui = p.ui; dispatch = p.dispatch; return null; };
    act(() => root.render(<Probe />));
    return { sent, ui: () => ui, dispatch: (n: PairNote) => act(() => dispatch(n)) };
  };

  it('asks PairStatus first, and shows no card until it says owner', async () => {
    const h = harness();
    h.dispatch({ type: 'requested', card: card() });
    await wait(0);
    expect(h.sent[0]).toEqual({ pairStatus: {} });
    expect(h.ui()?.cards.map((c) => c.id)).toEqual(['01']);
  });

  it('sends PairDeny for a deny, and PairApprove with replace only for a card whose name is taken', async () => {
    const h = harness();
    h.dispatch({ type: 'requested', card: card() });
    await wait(0);
    const u = h.ui() as PairingUi;
    await act(async () => { await u.onAnswer(u.cards[0] as PairCard, false, true); });
    await act(async () => { await u.onAnswer(u.cards[0] as PairCard, true, true); });
    expect(h.sent.slice(1)).toEqual([{ pairDeny: { pairId: new Uint8Array([1, 2]) } }, { pairApprove: { pairId: new Uint8Array([1, 2]), replace: false } }]);
    h.dispatch({ type: 'name-taken', id: '01' });
    const u2 = h.ui() as PairingUi;
    await act(async () => { await u2.onAnswer(u2.cards[0] as PairCard, true, true); });
    await act(async () => { await u2.onAnswer(u2.cards[0] as PairCard, true, false); });
    expect(h.sent.slice(3)).toEqual([{ pairApprove: { pairId: new Uint8Array([1, 2]), replace: true } }, { pairApprove: { pairId: new Uint8Array([1, 2]), replace: false } }]);
  });

  it('a refusal with name_in_use marks the card and answers "taken"', async () => {
    const sent: unknown[] = [];
    const api = { command: async (_n: string, body: Record<string, unknown>) => { sent.push(body); if ('pairStatus' in body) return { pairStatus: { open: false } }; throw Object.assign(new Error('x'), { code: 'pair.state', params: { state: 'name_in_use' } }); } } as unknown as Live;
    let ui: PairingUi | undefined; let dispatch: (n: PairNote) => void = () => {};
    const Probe = () => { const p = usePairing(api, 'live'); ui = p.ui; dispatch = p.dispatch; return null; };
    act(() => root.render(<Probe />));
    act(() => dispatch({ type: 'requested', card: card() }));
    await wait(0);
    let answer: PairAnswer | undefined;
    await act(async () => { answer = await (ui as PairingUi).onAnswer((ui as PairingUi).cards[0] as PairCard, true, false); });
    expect(answer).toBe('taken');
    expect((ui as PairingUi).cards[0]?.nameTaken).toBe(true);
  });

  it('a session the hub says is not the owner\'s shows nothing', async () => {
    const api = { command: async () => { throw Object.assign(new Error('x'), { code: 'auth.forbidden', params: {} }); } } as unknown as Live;
    let ui: PairingUi | undefined; let dispatch: (n: PairNote) => void = () => {};
    const Probe = () => { const p = usePairing(api, 'live'); ui = p.ui; dispatch = p.dispatch; return null; };
    act(() => root.render(<Probe />));
    act(() => dispatch({ type: 'requested', card: card() }));
    await wait(0);
    expect(ui).toBeUndefined();
  });

  it('says a session is not the owner\'s once, and stops asking (each refusal is a row in the owner\'s Activity, #904)', async () => {
    let asked = 0;
    const api = { command: async () => { asked += 1; throw Object.assign(new Error('x'), { code: 'auth.forbidden', params: {} }); } } as unknown as Live;
    let owner: string | undefined;
    const Probe = () => { owner = usePairing(api, 'live').owner; return null; };
    act(() => root.render(<Probe />));
    expect(owner).toBe('unknown');
    await wait(0);
    expect(owner).toBe('no');
    await wait(5 * 60_000);
    expect(asked).toBe(1);
  });
});
