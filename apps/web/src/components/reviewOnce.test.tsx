// @vitest-environment jsdom
// #1070: a start or a switch takes one Review wherever it is seen. Home's row and the attention dialog open the same Review dialog the
// agent's card opens (not the agent, where Review had to be pressed again), and that dialog keeps every rule of the card's: a dwell of its
// own from the moment it opens (T44), the window's holds (an answer in it holds the request on the card, and one on the card holds it), the
// danger Allow with its confirmation for a start that would not ask, and a switch's own guards. Also the kit's segmented control for the
// grant (chosen by keyboard) and the Close agent dialog (Keep first and focused; Escape closes no agent and reaches nothing behind it).
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage, t } from '../i18n';
import type { AgentTileModel } from '../model';
import { demoSkipApproval, demoStartApproval, demoStartNoAskApproval } from '../fixtures';
import { ATTENTION_DWELL_MS, attentionPrefs } from '../net/attention';
import { closeAsk } from '../net/closeAgent';
import { attentionList } from '../net/fleet';
import { forgetSkipCapable, markSkipCapable } from '../net/skipApproval';
import type { ApprovalModel } from '../net/view';
import { ApprovalCard } from './ApprovalCard';
import { AttentionLayer } from './AttentionLayer';
import { CloseAgentContext, CloseConfirm } from './CloseAgent';
import { AttentionQueue } from './FleetConsole';
import { StartApproval, type StartChoice } from './StartApproval';
import { AnswerHoldsProvider } from './useAnswerGuard';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
beforeEach(() => {
  setLanguage('en');
  vi.useFakeTimers();
  vi.setSystemTime(1_000_000);
  localStorage.clear();
  attentionPrefs.reset();
  vi.spyOn(document, 'hasFocus').mockImplementation(() => true);
  vi.spyOn(document, 'visibilityState', 'get').mockImplementation(() => 'visible');
  closeAsk.cancel();
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  document.body.replaceChildren();
  vi.useRealTimers();
  vi.restoreAllMocks();
  forgetSkipCapable();
  closeAsk.cancel();
});

const agent: AgentTileModel = { id: 'a1', name: 'claude · amber', accent: 'var(--agent-amber)', kind: 'claude', machine: 'Laptop', machineId: 'demo', workspace: 'login-fix', state: 'needs-you', preview: '' };
const start = demoStartApproval as ApprovalModel;
const dialog = () => document.querySelector('[data-review-dialog]') as HTMLElement | null;
const attentionCard = () => document.querySelector('.attention-card:not([data-review-dialog])') as HTMLElement | null;
const card = () => host.querySelector('[data-surface="card"] [data-review-card]') as HTMLElement;
const get = (within: ParentNode, name: string) => [...within.querySelectorAll('button')].find((x) => x.textContent === name) as HTMLButtonElement;
const press = (within: ParentNode, name: string) => { const b = get(within, name); if (!b) throw new Error(`no button ${name}`); act(() => b.click()); };
const off = (b: HTMLButtonElement) => b.disabled || b.getAttribute('aria-disabled') === 'true';
const dwell = (ms = ATTENTION_DWELL_MS) => act(() => { vi.advanceTimersByTime(ms); });
const settleAll = () => act(async () => { await vi.advanceTimersByTimeAsync(0); });
const key = (el: Element, k: string) => act(() => { el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true })); });

type Resolve = (a: ApprovalModel, ok: boolean, choice?: StartChoice) => unknown;
// Home's queue and the agent's card (the side panel's) in one window: the window's holds over both, as in `App`.
function homeAndCard(list: ApprovalModel[], on: Resolve, onOpen = vi.fn()) {
  return (
    <AnswerHoldsProvider listed={list.map((x) => x.id)}>
      <div data-surface="home"><AttentionQueue items={attentionList(list, [agent])} machines={[]} now={1_000_000} onOpen={onOpen} onResolve={on as never} /></div>
      <div data-surface="card">{list.map((x) => <ApprovalCard key={x.id} approval={x} agent={agent} onResolve={(ok, choice) => on(x, ok, choice) as never} />)}</div>
    </AnswerHoldsProvider>
  );
}
const homeReview = () => get(host.querySelector('[data-surface="home"]')!, t('fleet-review'));

describe('one Review from Home (#1070)', () => {
  it('Review on the row opens the whole request over Home, not the agent; Allow waits out the dialog\'s own dwell and sends the grant chosen', async () => {
    const on = vi.fn((..._a: unknown[]) => true);
    const onOpen = vi.fn();
    act(() => root.render(homeAndCard([start], on, onOpen)));
    dwell(5 * ATTENTION_DWELL_MS); // Home's own dwell is long over: the dialog's is its own
    expect(homeReview().getAttribute('aria-haspopup')).toBe('dialog');
    act(() => homeReview().click());
    expect(onOpen).not.toHaveBeenCalled();
    const d = dialog()!;
    expect(d).not.toBeNull();
    expect(d.textContent).toContain('Write unit tests for src/auth.rs.'); // the whole prompt, in Review
    expect(document.activeElement).toBe(d); // nothing is focused for the person
    // A second click where Review was lands on nothing: Allow is off for the dialog's dwell.
    expect(off(get(d, 'Allow'))).toBe(true);
    press(d, 'Allow');
    press(d, 'Deny');
    expect(on).not.toHaveBeenCalled();
    dwell();
    act(() => (d.querySelectorAll('[role=radiogroup] [role=radio]')[1] as HTMLButtonElement).click());
    press(d, 'Allow');
    expect(on).toHaveBeenCalledTimes(1);
    expect(on).toHaveBeenCalledWith(expect.objectContaining({ id: start.id }), true, { grant: 3, nested: false });
    await settleAll();
    // Held with the window: the agent's card says it went and its Deny is off, and the dialog sends no second answer.
    expect(card().textContent).toContain(t('approval-sent'));
    dwell();
    expect(off(get(card(), 'Deny'))).toBe(true);
    press(dialog()!, 'Deny');
    expect(on).toHaveBeenCalledTimes(1);
  });

  it('an answer on its way from the agent\'s card holds Review opened from Home, and one that failed frees it after a fresh dwell', async () => {
    let settle: (v: boolean) => void = () => {};
    const on = vi.fn((..._a: unknown[]) => new Promise<boolean>((r) => { settle = r; }));
    act(() => root.render(homeAndCard([start], on)));
    dwell();
    press(card(), 'Deny');
    expect(on).toHaveBeenCalledTimes(1);
    act(() => homeReview().click());
    dwell(5 * ATTENTION_DWELL_MS);
    expect(off(get(dialog()!, 'Allow'))).toBe(true);
    press(dialog()!, 'Allow');
    expect(on).toHaveBeenCalledTimes(1);
    await act(async () => { settle(false); await Promise.resolve(); });
    await settleAll();
    expect(off(get(dialog()!, 'Allow'))).toBe(true); // a fresh dwell first
    dwell();
    press(dialog()!, 'Allow');
    expect(on).toHaveBeenCalledTimes(2);
  });

  it('Close and Escape answer nothing; a request that leaves the list takes Review with it, and it does not come back by itself', () => {
    const on = vi.fn();
    act(() => root.render(homeAndCard([start], on)));
    act(() => homeReview().click());
    press(dialog()!, t('approval-review-close'));
    expect(dialog()).toBeNull();
    act(() => homeReview().click());
    key(dialog()!, 'Escape');
    expect(dialog()).toBeNull();
    act(() => homeReview().click());
    act(() => root.render(homeAndCard([], on)));
    expect(dialog()).toBeNull();
    act(() => root.render(homeAndCard([start], on)));
    expect(dialog()).toBeNull();
    expect(on).not.toHaveBeenCalled();
  });

  it('opening and closing Review over Home starts Home\'s dwell again: a second click on Close never lands on a row\'s Allow', () => {
    const on = vi.fn((..._a: unknown[]) => true);
    const plain = { id: 'demo/1', node: 'demo', localId: '1', agentId: 'a1', agentName: 'claude · amber', action: 'Edit src/auth.rs', detail: '+12 −3', expiresAtMs: 0 } as ApprovalModel;
    act(() => root.render(homeAndCard([plain, start], on)));
    const home = () => host.querySelector('[data-surface="home"]')!;
    dwell(5 * ATTENTION_DWELL_MS);
    expect(off(get(home(), 'Allow'))).toBe(false); // the plain row is ready
    act(() => homeReview().click());
    expect(off(get(home(), 'Allow'))).toBe(true);
    dwell(5 * ATTENTION_DWELL_MS);
    press(dialog()!, t('approval-review-close'));
    press(home(), 'Allow');
    expect(on).not.toHaveBeenCalled();
    dwell();
    press(home(), 'Allow');
    expect(on).toHaveBeenCalledTimes(1);
    expect(on).toHaveBeenCalledWith(expect.objectContaining({ id: plain.id }), true);
  });

  it('a start that would not ask keeps its danger Allow and confirmation in Review opened from Home', () => {
    const on = vi.fn();
    const x = demoStartNoAskApproval as ApprovalModel;
    act(() => root.render(homeAndCard([x], on)));
    act(() => homeReview().click());
    dwell();
    const allow = get(dialog()!, 'Allow');
    expect(allow.getAttribute('data-variant')).toBe('danger');
    press(dialog()!, 'Allow');
    expect(on).not.toHaveBeenCalled();
    const confirm = dialog()!.querySelector('[role=alertdialog]') as HTMLElement;
    expect(document.activeElement).toBe(get(confirm, t('session-confirm-no')));
    press(confirm, t('start-confirm-skip-yes'));
    expect(on).toHaveBeenCalledWith(expect.objectContaining({ id: x.id }), true, { grant: 0, nested: false });
  });

  it('a switch opens its Review from Home: Allow is the danger button and waits for its own dwell with "Read it first"', () => {
    markSkipCapable(demoSkipApproval.node);
    const on = vi.fn((..._a: unknown[]) => true);
    const x = demoSkipApproval as ApprovalModel;
    act(() => root.render(homeAndCard([x], on)));
    act(() => homeReview().click());
    const d = dialog()!;
    expect(d.querySelector('[data-skip-approval]')).not.toBeNull();
    expect(get(d, 'Allow').getAttribute('data-variant')).toBe('danger');
    expect(get(d, 'Allow').disabled).toBe(true);
    expect(d.textContent).toContain(t('skip-approval-read-first'));
    dwell(ATTENTION_DWELL_MS + 10);
    press(d, 'Allow');
    expect(on).toHaveBeenCalledWith(expect.objectContaining({ id: x.id }), true, undefined);
    expect(get(card(), 'Deny').disabled).toBe(true); // held on the card too
  });
});

describe('a freshly opened Review answers nothing before its open dwell, for a switch too (#1070)', () => {
  it('a double click on Home\'s Review for a switch sends nothing; Deny works after the dwell', () => {
    markSkipCapable(demoSkipApproval.node);
    const on = vi.fn((..._a: unknown[]) => true);
    const x = demoSkipApproval as ApprovalModel;
    act(() => root.render(homeAndCard([x], on)));
    dwell(5 * ATTENTION_DWELL_MS);
    // The first click opens Review; the second lands where its answers are: on Deny, and on Allow.
    act(() => homeReview().click());
    const d = dialog()!;
    expect(get(d, 'Deny').disabled).toBe(true);
    expect(get(d, 'Allow').disabled).toBe(true);
    press(d, 'Deny');
    press(d, 'Allow');
    expect(on).not.toHaveBeenCalled();
    dwell(ATTENTION_DWELL_MS + 10);
    expect(get(d, 'Deny').disabled).toBe(false);
    press(d, 'Deny');
    expect(on).toHaveBeenCalledTimes(1);
    expect(on).toHaveBeenCalledWith(expect.objectContaining({ id: x.id }), false, undefined);
  });

  it('the switch\'s own card keeps a Deny with no dwell', () => {
    markSkipCapable(demoSkipApproval.node);
    const on = vi.fn((..._a: unknown[]) => true);
    act(() => root.render(homeAndCard([demoSkipApproval as ApprovalModel], on)));
    expect(get(card(), 'Deny').disabled).toBe(false);
    press(card(), 'Deny');
    expect(on).toHaveBeenCalledTimes(1);
  });

  it('a start\'s Deny and Allow in Review wait too', () => {
    const on = vi.fn((..._a: unknown[]) => true);
    act(() => root.render(homeAndCard([start], on)));
    dwell(5 * ATTENTION_DWELL_MS);
    act(() => homeReview().click());
    press(dialog()!, 'Deny');
    press(dialog()!, 'Allow');
    expect(on).not.toHaveBeenCalled();
    dwell();
    press(dialog()!, 'Deny');
    expect(on).toHaveBeenCalledWith(expect.objectContaining({ id: start.id }), false, { grant: 0, nested: false });
  });
});

describe('one Review from the attention dialog (#1070)', () => {
  const layer = (list: ApprovalModel[], on: Resolve, onReview = vi.fn()) => (
    <AnswerHoldsProvider listed={list.map((x) => x.id)}>
      <AttentionLayer items={attentionList(list, [agent])} onResolve={on as never} onReview={onReview} />
      <div data-surface="card">{list.map((x) => <ApprovalCard key={x.id} approval={x} agent={agent} onResolve={(ok, choice) => on(x, ok, choice) as never} />)}</div>
    </AnswerHoldsProvider>
  );

  it('its Review opens the whole request in the card\'s place, with a dwell of its own, in the window\'s holds', async () => {
    const on = vi.fn((..._a: unknown[]) => true);
    const onReview = vi.fn();
    act(() => root.render(layer([start], on, onReview)));
    await settleAll();
    expect(attentionCard()).not.toBeNull();
    expect(attentionCard()!.textContent).toContain(t('attention-start-read'));
    dwell();
    press(attentionCard()!, t('attention-review'));
    expect(onReview).not.toHaveBeenCalled(); // not to the agent's card, where Review would be pressed again
    expect(attentionCard()).toBeNull(); // in its place, never stacked over it
    const d = dialog()!;
    expect(off(get(d, 'Allow'))).toBe(true);
    press(d, 'Allow');
    expect(on).not.toHaveBeenCalled();
    dwell();
    press(d, 'Allow');
    expect(on).toHaveBeenCalledTimes(1);
    expect(on).toHaveBeenCalledWith(expect.objectContaining({ id: start.id }), true, { grant: 0, nested: false });
    await settleAll();
    expect(card().textContent).toContain(t('approval-sent'));
  });

  it('nothing opens over Review while it is open; closing it leaves the request put off, and the banner brings the card back', async () => {
    const on = vi.fn();
    const other = { id: 'demo/7', node: 'demo', localId: '7', agentId: 'a1', agentName: 'claude · amber', action: 'Edit src/other.rs', detail: '+1 −1', expiresAtMs: 0 } as ApprovalModel;
    act(() => root.render(layer([start], on)));
    await settleAll();
    dwell();
    press(attentionCard()!, t('attention-review'));
    act(() => root.render(layer([start, other], on)));
    await settleAll();
    dwell(2000);
    expect(attentionCard()).toBeNull();
    expect(document.querySelector('.attention-banner')).toBeNull();
    expect(dialog()).not.toBeNull();
    key(dialog()!, 'Escape');
    expect(dialog()).toBeNull();
    expect(on).not.toHaveBeenCalled();
    await settleAll();
    dwell(1000);
    // The other request opens now; the one reviewed waits behind it, put off as after Later.
    expect(attentionCard()!.textContent).toContain('Edit src/other.rs');
    press(attentionCard()!, t('attention-later'));
    const banner = document.querySelector('.attention-banner') as HTMLElement;
    expect(banner).not.toBeNull();
    // The modals traded places (the card went as Review came, then another card): with none open, the window is clickable again.
    await settleAll();
    expect(document.querySelector('[role=dialog],[role=alertdialog]')).toBeNull();
    expect(document.body.style.pointerEvents).not.toBe('none');
    press(banner, t('attention-review'));
    expect(attentionCard()!.textContent).toContain(t('attention-start-read'));
  });
});

describe('the grant is the kit\'s segmented control (#1070)', () => {
  it('one tab stop; the arrow keys, Home and End choose; the choice is what is sent, and its meaning is shown whole whatever is chosen', () => {
    const answers: [boolean, StartChoice][] = [];
    act(() => root.render(<StartApproval approval={start} start={start.start!} density="full" onResolve={(ok, c) => answers.push([ok, c])} />));
    const group = host.querySelector('[role=radiogroup]') as HTMLElement;
    expect(group.getAttribute('aria-labelledby') && document.getElementById(group.getAttribute('aria-labelledby')!)?.textContent).toBe(t('start-approval-scope'));
    const radios = () => [...group.querySelectorAll<HTMLButtonElement>('[role=radio]')];
    expect(radios().map((r) => r.textContent)).toEqual([t('start-approval-once'), t('start-approval-more-short')]);
    expect(radios().map((r) => r.tabIndex)).toEqual([0, -1]);
    const meaning = () => host.querySelector('[data-grant-more]')?.textContent;
    const said = meaning();
    expect(said).toContain('Up to 3 codex agents in this folder');
    act(() => radios()[0]!.focus());
    key(radios()[0]!, 'ArrowRight');
    expect(radios().map((r) => r.getAttribute('aria-checked'))).toEqual(['false', 'true']);
    expect(document.activeElement).toBe(radios()[1]);
    expect(meaning()).toBe(said); // nothing appears, goes or moves under the control
    act(() => get(host, 'Allow').click());
    key(radios()[1]!, 'Home');
    expect(document.activeElement).toBe(radios()[0]);
    act(() => get(host, 'Allow').click());
    key(radios()[0]!, 'End');
    act(() => get(host, 'Allow').click());
    expect(answers).toEqual([[true, { grant: 3, nested: false }], [true, { grant: 0, nested: false }], [true, { grant: 3, nested: false }]]);
  });
});

describe('Close agent is the kit dialog (audit B15, #1070)', () => {
  const a: AgentTileModel = { ...agent, id: 'n1/a-1', state: 'idle', terminal: false };
  it('a danger chip and title, then Keep it running first and focused, then the danger Close agent', () => {
    act(() => root.render(<CloseAgentContext.Provider value={async () => undefined}><CloseConfirm agent={a} where="pane" /></CloseAgentContext.Provider>));
    act(() => closeAsk.ask(a.id, 'pane'));
    const d = document.querySelector('[role=alertdialog]') as HTMLElement;
    expect(d.querySelector('.kit-chip.kit-tone-danger')).not.toBeNull();
    expect(d.querySelector('h2')?.textContent).toBe(t('close-title', { name: a.name }));
    const buttons = [...d.querySelectorAll('button')];
    expect(buttons.map((b) => b.textContent)).toEqual([t('close-keep'), t('close-agent')]);
    expect(buttons[1]!.getAttribute('data-variant')).toBe('danger');
    expect(document.activeElement).toBe(buttons[0]);
    // Enter on what has the focus keeps the agent.
    act(() => (document.activeElement as HTMLButtonElement).click());
    expect(document.querySelector('[role=alertdialog]')).toBeNull();
  });

  it('Escape closes no agent and reaches nothing behind the dialog: not the pane it was opened from, not the window', () => {
    const close = vi.fn(async () => undefined);
    const pane = vi.fn();
    const win = vi.fn();
    window.addEventListener('keydown', win);
    try {
      act(() => root.render(<CloseAgentContext.Provider value={close}><div onKeyDown={(e) => { if (e.key === 'Escape') pane(); }}><CloseConfirm agent={a} where="pane" /></div></CloseAgentContext.Provider>));
      act(() => closeAsk.ask(a.id, 'pane'));
      key(document.activeElement!, 'Escape');
      expect(close).not.toHaveBeenCalled();
      expect(pane).not.toHaveBeenCalled();
      expect(win).not.toHaveBeenCalled();
      expect(document.querySelector('[role=alertdialog]')).toBeNull();
      expect(closeAsk.get()).toBeUndefined();
    } finally {
      window.removeEventListener('keydown', win);
    }
  });

  it('one dialog for one ask, even where the agent is drawn twice', () => {
    const close = vi.fn(async (_: AgentTileModel) => undefined);
    act(() => root.render(<CloseAgentContext.Provider value={close}><CloseConfirm agent={a} where="pane" /><CloseConfirm agent={a} where="pane" /></CloseAgentContext.Provider>));
    act(() => closeAsk.ask(a.id, 'pane'));
    expect(document.querySelectorAll('[role=alertdialog]').length).toBe(1);
  });
});
