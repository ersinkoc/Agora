// @vitest-environment jsdom
// #1082: a request pinned above a chat's composer in a pane too short for the card and the conversation is drawn as one row. The row is a
// change of shape only, so these tests pin that it keeps every T44 rule: Allow and Deny only for what `decidableInline` shows whole (the
// ask whole beside them), never "always" from the row (ADR-0101), Review (the whole card) for anything else, a fresh dwell whenever the
// row and the card swap (the buttons move), the window's holds across surfaces, and a start's Allow only in Review's dialog.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage } from '../i18n';
import type { AgentTileModel } from '../model';
import { demoStartApproval } from '../fixtures';
import { ATTENTION_DWELL_MS } from '../net/attention';
import type { ApprovalModel } from '../net/view';
import { ApprovalBanner } from './ApprovalBanner';
import { ApprovalCard } from './ApprovalCard';
import { NARROW_PX, PINNED_ANSWERS_REM, PinnedShort, SHORT_PANE_PX } from './ApprovalRequest';

const PINNED_ANSWERS_PX = PINNED_ANSWERS_REM * 16; // at the default text size
import { PromptBox } from './PromptBox';
import { AnswerHoldsProvider } from './useAnswerGuard';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
beforeEach(() => {
  setLanguage('en');
  vi.spyOn(document, 'hasFocus').mockImplementation(() => true);
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
  delete (globalThis as { ResizeObserver?: unknown }).ResizeObserver;
  document.documentElement.style.removeProperty('font-size');
});

const agent: AgentTileModel = { id: 'n1/a1', name: 'codex · teal', accent: 'var(--agent-teal)', machine: 'laptop', machineId: 'n1', workspace: 'w', project: 'agora', state: 'needs-you', preview: '', terminal: false };
const ask = (over: Partial<ApprovalModel> = {}): ApprovalModel =>
  ({ id: 'n1/p1', node: 'n1', localId: 'p1', agentId: 'n1/a1', agentName: 'codex · teal', machineName: 'laptop', action: 'run_command', detail: 'cargo test -p agora-hub', expiresAtMs: 0, ...over });
const buttons = (within: ParentNode = host) => [...within.querySelectorAll('button')];
const button = (text: string, within: ParentNode = host) => buttons(within).find((b) => b.textContent === text) as HTMLButtonElement | undefined;
const card = () => host.querySelector('section.approval-req') as HTMLElement;
const dwell = () => act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS + 10); });
const pinned = (x: ApprovalModel, short: boolean, onResolve: (a: ApprovalModel, ok: boolean, c?: unknown, always?: boolean) => unknown) =>
  <PinnedShort.Provider value={short}><ApprovalBanner approval={x} agent={agent} onResolve={onResolve as never} pinned /></PinnedShort.Provider>;

describe('a request pinned above a chat in a short pane is one row', () => {
  it('a plain yes or no shown whole: the ask whole and Deny, Allow beside it (no "always" from the row), under the dwell', () => {
    vi.useFakeTimers();
    const onResolve = vi.fn(() => true);
    act(() => root.render(pinned(ask({ always: { label: 'Always allow cargo test', optionId: 'always-1', only: false } }), true, onResolve)));
    expect(card().getAttribute('data-density')).toBe('row');
    expect(card().getAttribute('data-pinned-row')).toBe('answer');
    expect(card().querySelector('.approval-req__cmd')!.textContent).toBe('run_command · cargo test -p agora-hub');
    expect(buttons().map((b) => b.textContent)).toEqual(['Deny', 'Allow']);
    act(() => button('Allow')!.click());
    expect(onResolve).not.toHaveBeenCalled();
    dwell();
    act(() => button('Allow')!.click());
    act(() => button('Allow')?.click());
    expect(onResolve).toHaveBeenCalledTimes(1);
    expect(onResolve).toHaveBeenCalledWith(expect.objectContaining({ id: 'n1/p1' }), true);
  });

  it('the pane getting short (or tall again) moves the buttons, so the dwell starts again', () => {
    vi.useFakeTimers();
    const onResolve = vi.fn(() => true);
    act(() => root.render(pinned(ask(), false, onResolve)));
    expect(card().getAttribute('data-density')).toBe('card');
    dwell();
    act(() => root.render(pinned(ask(), true, onResolve)));
    expect(card().getAttribute('data-pinned-row')).toBe('answer');
    act(() => button('Allow')!.click());
    expect(onResolve).not.toHaveBeenCalled();
    dwell();
    act(() => root.render(pinned(ask(), false, onResolve)));
    expect(card().getAttribute('data-density')).toBe('card');
    act(() => button('Allow')!.click());
    expect(onResolve).not.toHaveBeenCalled();
    dwell();
    act(() => button('Allow')!.click());
    expect(onResolve).toHaveBeenCalledTimes(1);
  });

  it('anything not shown whole has no answer in the row: Review opens the whole card, whose answers wait out a fresh dwell', () => {
    vi.useFakeTimers();
    const onResolve = vi.fn(() => true);
    const long = ask({ detail: 'x'.repeat(200) });
    act(() => root.render(pinned(long, true, onResolve)));
    dwell();
    expect(card().getAttribute('data-pinned-row')).toBe('review');
    expect(button('Allow')).toBeUndefined();
    expect(button('Deny')).toBeUndefined();
    expect(card().textContent).not.toContain('x'.repeat(200));
    act(() => button('Review')!.click());
    expect(card().getAttribute('data-density')).toBe('card');
    expect(card().querySelector('.approval-req__cmd')!.textContent).toContain('x'.repeat(200));
    act(() => button('Allow')!.click());
    expect(onResolve).not.toHaveBeenCalled();
    dwell();
    act(() => button('Allow')!.click());
    expect(onResolve).toHaveBeenCalledTimes(1);
  });

  it('a request that offers only "always" is answered in its card, never from the row', () => {
    act(() => root.render(pinned(ask({ always: { label: 'Allow this session', optionId: 'session', only: true } }), true, vi.fn())));
    expect(card().getAttribute('data-pinned-row')).toBe('review');
    expect(buttons().map((b) => b.textContent)).toEqual(['Review']);
  });

  // Every width observer of the card is told the card's new width (jsdom lays nothing out).
  const widths = () => {
    const cbs: ((e: { contentRect: { width: number; height: number } }[]) => void)[] = [];
    (globalThis as { ResizeObserver?: unknown }).ResizeObserver = class {
      constructor(cb: (typeof cbs)[number]) { cbs.push(cb); }
      observe() { /* reported by hand */ }
      disconnect() { /* nothing */ }
    };
    return (w: number) => { for (const cb of cbs) cb([{ contentRect: { width: w, height: 0 } }]); };
  };

  it('a pane both short and narrow has no room for the answers beside the ask: the title and Review', () => {
    const report = widths();
    vi.useFakeTimers();
    act(() => root.render(pinned(ask(), true, vi.fn())));
    expect(card().getAttribute('data-pinned-row')).toBe('answer');
    act(() => report(NARROW_PX - 100));
    expect(card().getAttribute('data-pinned-row')).toBe('review');
    expect(buttons().map((b) => b.textContent)).toEqual(['Review']);
  });

  it('below the width where the buttons sit beside the ask it is the title and Review, and a change between the two restarts the dwell', () => {
    const report = widths();
    vi.useFakeTimers();
    const onResolve = vi.fn(() => true);
    act(() => root.render(pinned(ask(), true, onResolve)));
    act(() => report(PINNED_ANSWERS_PX - 60)); // wider than a narrow pane, too narrow for the buttons beside the ask
    expect(card().getAttribute('data-pinned-row')).toBe('review');
    expect(button('Allow')).toBeUndefined();
    dwell();
    act(() => report(PINNED_ANSWERS_PX + 40));
    expect(card().getAttribute('data-pinned-row')).toBe('answer');
    act(() => button('Allow')!.click());
    expect(onResolve).not.toHaveBeenCalled();
    dwell();
    act(() => button('Allow')!.click());
    expect(onResolve).toHaveBeenCalledTimes(1);
  });

  // Codex on #1087: at the 125 % text size (a 20 px root) the callout stacks its buttons below 28rem = 560 px, so a 504 px card has them
  // under the title; a gate in px (480) still drew the answers there. The gate is in rem, as the CSS is.
  it('at a larger text size the gate is larger too: a card the CSS stacks at that size is the title and Review', () => {
    document.documentElement.style.fontSize = '20px';
    const report = widths();
    act(() => root.render(pinned(ask(), true, vi.fn())));
    act(() => report(504));
    expect(card().getAttribute('data-pinned-row')).toBe('review');
    expect(buttons().map((b) => b.textContent)).toEqual(['Review']);
    act(() => report(PINNED_ANSWERS_REM * 20 + 8));
    expect(card().getAttribute('data-pinned-row')).toBe('answer');
  });

  it('the text size changing alone (the card keeps its width) moves the row between its forms, and the dwell starts again', async () => {
    const report = widths();
    vi.useFakeTimers();
    const onResolve = vi.fn(() => true);
    act(() => root.render(pinned(ask(), true, onResolve)));
    act(() => report(512));
    expect(card().getAttribute('data-pinned-row')).toBe('answer');
    dwell();
    await act(async () => { document.documentElement.style.fontSize = '20px'; });
    expect(card().getAttribute('data-pinned-row')).toBe('review');
    expect(button('Allow')).toBeUndefined();
    dwell();
    await act(async () => { document.documentElement.style.removeProperty('font-size'); });
    expect(card().getAttribute('data-pinned-row')).toBe('answer');
    act(() => button('Allow')!.click());
    expect(onResolve).not.toHaveBeenCalled();
    dwell();
    act(() => button('Allow')!.click());
    expect(onResolve).toHaveBeenCalledTimes(1);
  });

  it('an answer from the row is held for the whole window: the side panel\'s card of the same request sends nothing', () => {
    vi.useFakeTimers();
    let settle: (v: boolean) => void = () => {};
    const onResolve = vi.fn(() => new Promise<boolean>((r) => { settle = r; }));
    const panel = vi.fn(() => true);
    act(() => root.render(
      <AnswerHoldsProvider listed={['n1/p1']}>
        <div id="pane">{pinned(ask(), true, onResolve)}</div>
        <div id="panel"><ApprovalCard approval={ask()} agent={agent} onResolve={panel} /></div>
      </AnswerHoldsProvider>));
    dwell();
    const pane = host.querySelector('#pane')!;
    const side = host.querySelector('#panel')!;
    act(() => button('Allow', pane)!.click());
    expect(onResolve).toHaveBeenCalledTimes(1);
    act(() => button('Allow', side)!.click());
    expect(button('Allow', side)!.getAttribute('aria-disabled')).toBe('true');
    expect(panel).not.toHaveBeenCalled();
    void settle;
  });

  it('a start in a short pane is the title and Review only: Deny and Allow are in Review\'s dialog', () => {
    vi.useFakeTimers();
    const onResolve = vi.fn();
    act(() => root.render(pinned(demoStartApproval as ApprovalModel, true, onResolve)));
    const row = host.querySelector('[data-review-card]') as HTMLElement;
    expect(row.getAttribute('data-pinned-row')).toBe('review');
    expect(buttons(row).map((b) => b.textContent)).toEqual(['Review']);
    act(() => button('Review', row)!.click());
    expect(document.querySelector('[data-review-dialog]')).not.toBeNull();
    expect(onResolve).not.toHaveBeenCalled();
  });
});

describe('the chat pane says when it is short', () => {
  it('measures the whole pane: below SHORT_PANE_PX the pinned request is a row, above it the card', () => {
    // Only the pane's own observer is told of its new size (the log's and the card's have nothing to learn here).
    const observers: { el?: Element; cb: (e: { contentRect: { width: number; height: number } }[]) => void }[] = [];
    let height = SHORT_PANE_PX - 100;
    (globalThis as { ResizeObserver?: unknown }).ResizeObserver = class {
      private readonly o: (typeof observers)[number];
      constructor(cb: (typeof observers)[number]['cb']) { this.o = { cb }; observers.push(this.o); }
      observe(el: Element) { this.o.el = el; }
      disconnect() { /* nothing */ }
    };
    const report = (h: number) => { for (const o of observers) if (o.el === host.firstElementChild) o.cb([{ contentRect: { width: 800, height: h } }]); };
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(() => ({ width: 800, height, top: 0, left: 0, right: 800, bottom: height, x: 0, y: 0, toJSON: () => ({}) }) as DOMRect);
    Element.prototype.scrollTo ??= () => {};
    act(() => root.render(<PromptBox agent={agent} approvals={<ApprovalBanner approval={ask()} agent={agent} onResolve={() => true} pinned />} />));
    expect(card().getAttribute('data-pinned-row')).toBe('answer');
    height = SHORT_PANE_PX + 200;
    act(() => report(height));
    expect(card().getAttribute('data-density')).toBe('card');
    expect(card().getAttribute('data-pinned-row')).toBeNull();
  });
});
