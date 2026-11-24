// @vitest-environment jsdom
// Redesign PR 5: a request to start an agent (ADR-0057) and a switch to act without asking (ADR-0090 Amendment A) are short cards with
// Review, wherever they wait in chrome (the side panel, above a terminal, above a composer). These tests pin the safety rules the change
// must keep (approvals.md, T44, T15, T51): the card is a summary and never offers Allow (never decided on cut text); Review shows the
// whole request, and Allow is only there; Review's answers wait out a dwell of their own from the moment it opens; the start's confirmation
// for a helper that would not ask and the switch's own guards are unchanged; an answer anywhere holds the request everywhere.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage } from '../i18n';
import type { AgentTileModel } from '../model';
import { demoSkipApproval, demoStartApproval, demoStartLongApproval, demoStartNoAskApproval, demoStartSettingsApproval } from '../fixtures';
import { ATTENTION_DWELL_MS } from '../net/attention';
import { forgetSkipCapable, markSkipCapable } from '../net/skipApproval';
import type { ApprovalModel } from '../net/view';
import { ApprovalBanner } from './ApprovalBanner';
import { ApprovalCard } from './ApprovalCard';
import { NARROW_PX } from './ApprovalRequest';
import { Details, Inspector } from './Inspector';
import { AnswerHoldsProvider } from './useAnswerGuard';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
beforeEach(() => {
  setLanguage('en');
  vi.useFakeTimers();
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
  delete (globalThis as { ResizeObserver?: unknown }).ResizeObserver;
});

const agent: AgentTileModel = { id: 'a1', name: 'claude · amber', accent: 'var(--agent-amber)', kind: 'claude', machine: 'Laptop', workspace: 'login-fix', state: 'needs-you', preview: '' };
const long = demoStartLongApproval as ApprovalModel;
const card = () => host.querySelector('[data-review-card]') as HTMLElement;
const dialog = () => document.querySelector('[data-review-dialog]') as HTMLElement | null;
const names = (within: ParentNode) => [...within.querySelectorAll('button')].map((b) => b.textContent);
const press = (within: ParentNode, name: string) => { const b = [...within.querySelectorAll('button')].find((x) => x.textContent === name) as HTMLButtonElement; act(() => b.click()); };
const get = (within: ParentNode, name: string) => [...within.querySelectorAll('button')].find((x) => x.textContent === name) as HTMLButtonElement;
const off = (b: HTMLButtonElement) => b.disabled || b.getAttribute('aria-disabled') === 'true';
const dwell = (ms = ATTENTION_DWELL_MS) => act(() => { vi.advanceTimersByTime(ms); });
const flush = async () => { await act(async () => { await Promise.resolve(); await Promise.resolve(); }); };

describe('a long start request is a summary with Review', () => {
  it('shows two lines (what would start, the first line of the prompt and how many follow) and no Allow until Review shows it whole', () => {
    const on = vi.fn();
    act(() => root.render(<ApprovalCard approval={long} agent={agent} onResolve={on} onOpen={() => {}} />));
    const lines = [...card().querySelectorAll('.approval-req__line')].map((l) => l.textContent);
    expect(lines).toHaveLength(2);
    expect(lines[0]).toBe('/home/dev/code/login-fix'); // the kind is in the title
    expect(lines[1]).toContain('Write unit tests for src/auth.rs.');
    expect(lines[1]).toContain('6 more lines');
    expect(card().textContent).not.toContain('skew window'); // the rest of the prompt is not in the card
    expect(card().querySelector('pre')).toBeNull();
    expect(names(card())).toEqual(['', 'Deny', 'Review']); // the first is the icon button that opens the agent
    expect(card().querySelector('.approval-req__open')!.getAttribute('aria-label')).toBe('Open claude · amber');
    expect(get(card(), 'Allow')).toBeUndefined();
    dwell(ATTENTION_DWELL_MS * 3);
    expect(get(card(), 'Allow')).toBeUndefined(); // never, however long it waits
    // The title is the ask, with the kind; the machine sits beside it.
    expect(card().querySelector('.approval-req__ask')!.textContent).toBe('claude · amber asks to start a codex agent');
    expect(card().querySelector('.approval-req__where')!.textContent).toBe('Laptop');
    press(card(), 'Review');
    const d = dialog()!;
    expect(d).not.toBeNull();
    expect(d.querySelector('pre')!.textContent).toBe(long.start!.prompt);
    expect(off(get(d, 'Allow'))).toBe(true); // Review's own dwell, started when it opened
    dwell();
    expect(off(get(d, 'Allow'))).toBe(false);
    press(d, 'Allow');
    expect(on).toHaveBeenCalledWith(true, { grant: 0, nested: false });
  });

  it('Review shows the whole request: folder, settings, options, the whole prompt, the choices; Close and Escape answer nothing', () => {
    const on = vi.fn();
    act(() => root.render(<ApprovalCard approval={demoStartSettingsApproval as ApprovalModel} onResolve={on} />));
    press(card(), 'Review');
    const d = dialog()!;
    const text = d.textContent!;
    expect(d.getAttribute('role')).toBe('dialog');
    expect(document.getElementById(d.getAttribute('aria-labelledby')!)!.textContent).toBe('claude · amber on Laptop asks to start a codex agent');
    expect(text).toContain('/home/dev/code/login-fix');
    for (const st of demoStartSettingsApproval.start.settings) expect(text).toContain(st.value);
    expect(d.querySelector('pre')!.textContent).toBe(demoStartSettingsApproval.start.prompt);
    expect(names(d)).toEqual(['Close', 'Deny', 'Allow']);
    expect(document.activeElement).toBe(d); // nothing is focused for the person
    press(d, 'Close');
    expect(dialog()).toBeNull();
    press(card(), 'Review');
    act(() => { dialog()!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })); });
    expect(dialog()).toBeNull();
    expect(on).not.toHaveBeenCalled();
    // The grant choices are in Review, and what is chosen there is what is sent.
    act(() => root.render(<ApprovalCard approval={demoStartApproval as ApprovalModel} onResolve={on} />));
    press(card(), 'Review');
    act(() => (dialog()!.querySelectorAll('[role=radiogroup] [role=radio]')[1] as HTMLButtonElement).click());
    dwell();
    press(dialog()!, 'Allow');
    expect(on).toHaveBeenCalledWith(true, { grant: 3, nested: false });
  });

  it('says in the summary, before the folder, that a helper would not ask or would run outside the asker\'s folder; hidden characters are written out', () => {
    const outside = { ...demoStartNoAskApproval, start: { ...demoStartNoAskApproval.start, inside: false, prompt: 'go‮evil\nmore' } } as ApprovalModel;
    act(() => root.render(<ApprovalCard approval={outside} onResolve={() => {}} />));
    const [first, second] = [...card().querySelectorAll('.approval-req__line')];
    const parts = [...first!.children].map((c) => c.textContent);
    expect(parts).toEqual(["Don't ask", 'outside its folder', '/home/dev/code/login-fix']);
    expect(first!.children[0]!.className).toContain('text-danger');
    expect(first!.children[1]!.className).toContain('text-danger');
    expect(second!.textContent).toContain('go\\u{202e}evil');
    expect(second!.textContent).not.toContain('‮');
    expect(second!.textContent).toContain('1 more line');
  });

  it('Escape in Review closes Review only, never the side panel it was opened from', () => {
    const onOpenChange = vi.fn();
    act(() => root.render(<Inspector fleet={[agent]} attention={[agent]} approvals={[{ ...long, agentId: agent.id }]} changed={[]} boards={[]} messages={[]}
      onOpen={() => {}} onResolve={() => {}} open onOpenChange={onOpenChange} tab="needs" onTab={() => {}} />));
    press(card(), 'Review');
    expect(dialog()).not.toBeNull();
    act(() => { dialog()!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })); });
    expect(dialog()).toBeNull();
    expect(onOpenChange).not.toHaveBeenCalled();
    expect(card()).not.toBeNull();
  });

  it('a card that cannot be answered still has Review, which shows it whole with no answer', () => {
    act(() => root.render(<ApprovalBanner approval={long} agent={agent} />));
    expect(names(card())).toEqual(['Review']);
    press(card(), 'Review');
    dwell(ATTENTION_DWELL_MS * 2);
    expect(dialog()!.querySelector('pre')!.textContent).toBe(long.start!.prompt);
    // ADR-0106 (#1140): read-only, as in a viewer's window: no answer and no grant to choose, only Close.
    expect(names(dialog()!)).not.toContain('Allow');
    expect(names(dialog()!)).not.toContain('Deny');
    expect(dialog()!.querySelector('[data-grant-choice]')).toBeNull();
  });
  it('a switch that cannot be answered (a viewer\'s window, #1140) shows it whole in Review with no answer, and no Deny on its card', () => {
    const skip = demoSkipApproval as ApprovalModel;
    markSkipCapable(skip.node);
    act(() => root.render(<ApprovalBanner approval={skip} agent={agent} />));
    expect(names(card())).toEqual(['Review']);
    press(card(), 'Review');
    dwell(ATTENTION_DWELL_MS * 2);
    expect(dialog()!.textContent).toContain(skip.skip!.folder);
    expect(names(dialog()!)).not.toContain('Allow');
    expect(names(dialog()!)).not.toContain('Deny');
    act(() => root.render(<ApprovalBanner approval={skip} agent={agent} onResolve={() => true} />));
    expect(names(dialog()!)).toEqual(expect.arrayContaining(['Deny', 'Allow']));
  });
  it('the same card with an answer has Deny and Allow in Review (the read-only one above is not a card that never draws them)', () => {
    act(() => root.render(<ApprovalBanner approval={long} agent={agent} onResolve={() => true} />));
    press(card(), 'Review');
    dwell(ATTENTION_DWELL_MS * 2);
    expect(names(dialog()!)).toContain('Allow');
    expect(names(dialog()!)).toContain('Deny');
    expect(dialog()!.querySelector('[data-grant-choice]')).not.toBeNull();
  });

  it('a narrow pane draws it in the row density with the buttons under the text, and the dwell restarts', () => {
    let report: (w: number) => void = () => {};
    (globalThis as { ResizeObserver?: unknown }).ResizeObserver = class {
      constructor(cb: (e: { contentRect: { width: number } }[]) => void) { report = (w) => cb([{ contentRect: { width: w } }]); }
      observe() { /* reported by hand */ }
      disconnect() { /* nothing */ }
    };
    const on = vi.fn();
    act(() => root.render(<ApprovalBanner approval={long} agent={agent} onResolve={on} pinned />));
    dwell();
    act(() => report(NARROW_PX - 160));
    expect(card().getAttribute('data-density')).toBe('row');
    expect(card().className).toContain('approval-req--stacked');
    press(card(), 'Deny'); // the buttons moved under the text: the dwell started again
    expect(on).not.toHaveBeenCalled();
    dwell();
    press(card(), 'Deny');
    expect(on).toHaveBeenCalledWith(long, false, { grant: 0, nested: false });
  });
});

describe('Review keeps T44: a dwell of its own and the window\'s holds', () => {
  it('a second click on Review never lands on Allow, even when the card\'s own dwell has long run out', () => {
    const on = vi.fn();
    act(() => root.render(<ApprovalCard approval={long} onResolve={on} />));
    dwell(ATTENTION_DWELL_MS * 5);
    press(card(), 'Review');
    press(dialog()!, 'Allow');
    press(dialog()!, 'Deny');
    expect(on).not.toHaveBeenCalled();
    dwell();
    press(dialog()!, 'Deny');
    expect(on).toHaveBeenCalledWith(false, { grant: 0, nested: false });
  });

  it('the dwell starts again when the person comes back to the window and when the request changes', () => {
    const on = vi.fn();
    act(() => root.render(<ApprovalCard approval={long} onResolve={on} />));
    press(card(), 'Review');
    dwell();
    act(() => { window.dispatchEvent(new Event('focus')); });
    press(dialog()!, 'Allow');
    expect(on).not.toHaveBeenCalled();
    dwell();
    const other = { ...long, detail: `${long.detail} `, start: { ...long.start!, prompt: 'Something else entirely.' } };
    act(() => root.render(<ApprovalCard approval={other} onResolve={on} />));
    expect(dialog()!.querySelector('pre')!.textContent).toBe('Something else entirely.');
    press(dialog()!, 'Allow');
    expect(on).not.toHaveBeenCalled();
    dwell();
    press(dialog()!, 'Allow');
    expect(on).toHaveBeenCalledTimes(1);
  });

  it('the card\'s Deny keeps the surface\'s dwell; with the link down nothing answers and both say why', () => {
    const on = vi.fn();
    const draw = (stale: boolean) => act(() => root.render(<Details tab="needs" fleet={[agent]} attention={[agent]} approvals={[{ ...long, agentId: agent.id }]} changed={[]} boards={[]} messages={[]} onOpen={() => {}} onResolve={on} stale={stale} />));
    draw(false);
    press(card(), 'Deny');
    expect(on).not.toHaveBeenCalled(); // the side panel's dwell
    draw(true);
    dwell();
    expect(get(card(), 'Deny').disabled).toBe(true);
    expect(card().textContent).toContain('connection is down');
    press(card(), 'Review');
    expect(get(dialog()!, 'Allow').disabled).toBe(true);
    expect(dialog()!.textContent).toContain('connection is down');
    press(dialog()!, 'Close');
    draw(false);
    dwell();
    press(card(), 'Deny');
    expect(on).toHaveBeenCalledWith(expect.objectContaining({ id: long.id }), false, { grant: 0, nested: false }, undefined);
  });

  it('an answer in Review holds the request on the card and on every other surface, and one from elsewhere holds Review', async () => {
    let settle: (v: boolean) => void = () => {};
    const on = vi.fn((..._args: unknown[]) => new Promise<boolean>((r) => { settle = r; }));
    const list = [long];
    const both = <AnswerHoldsProvider listed={list.map((x) => x.id)}>
      <div data-surface="pane"><ApprovalBanner approval={long} onResolve={on} pinned /></div>
      <div data-surface="panel"><ApprovalCard approval={long} onResolve={(ok, choice) => on(long, ok, choice)} /></div>
    </AnswerHoldsProvider>;
    act(() => root.render(both));
    const pane = () => host.querySelector('[data-surface="pane"] [data-review-card]') as HTMLElement;
    const panel = () => host.querySelector('[data-surface="panel"] [data-review-card]') as HTMLElement;
    press(pane(), 'Review');
    dwell();
    press(dialog()!, 'Allow');
    expect(on).toHaveBeenCalledTimes(1);
    expect(off(get(dialog()!, 'Deny'))).toBe(true);
    dwell();
    expect(off(get(pane(), 'Deny'))).toBe(true);
    expect(off(get(panel(), 'Deny'))).toBe(true);
    press(panel(), 'Deny');
    expect(on).toHaveBeenCalledTimes(1);
    // It failed: answerable again, after a fresh dwell everywhere.
    await act(async () => { settle(false); await Promise.resolve(); });
    await flush();
    press(panel(), 'Deny');
    expect(on).toHaveBeenCalledTimes(1);
    dwell();
    press(panel(), 'Deny');
    expect(on).toHaveBeenCalledTimes(2);
    // Answered from the panel: Review in the pane is held too, and says it went once it did.
    expect(off(get(dialog()!, 'Allow'))).toBe(true);
    await act(async () => { settle(true); await Promise.resolve(); });
    await flush();
    dwell();
    expect(off(get(dialog()!, 'Allow'))).toBe(true);
    expect(dialog()!.textContent).toContain('Answer sent.');
    expect(pane().textContent).toContain('Answer sent.');
  });
});

describe('a start that would not ask, and a switch to act without asking: the danger Allow and its guards, in Review', () => {
  it('the start\'s Allow is the danger button and opens the confirmation (Keep asking primary and focused), which goes back to Review', () => {
    const on = vi.fn();
    act(() => root.render(<ApprovalCard approval={demoStartNoAskApproval as ApprovalModel} onResolve={on} />));
    expect(card().querySelector('.text-danger')!.textContent).toBe("Don't ask");
    press(card(), 'Review');
    const d = dialog()!;
    expect(get(d, 'Allow').className).toContain('kit-btn--danger');
    expect(d.textContent).toContain('without asking you first');
    dwell();
    press(d, 'Allow');
    expect(on).not.toHaveBeenCalled();
    expect(document.activeElement?.textContent).toBe('Keep asking');
    press(d, 'Keep asking');
    expect(d.querySelector('[role=alertdialog]')).toBeNull();
    expect(on).not.toHaveBeenCalled();
    press(d, 'Allow');
    press(d, 'Start without asking');
    expect(on).toHaveBeenCalledWith(true, { grant: 0, nested: false });
  });

  it('the switch\'s card has Deny with no dwell and Review; its Allow, only in Review, waits for its own dwell with "Read it first"', () => {
    markSkipCapable(demoSkipApproval.node);
    const on = vi.fn();
    act(() => root.render(<ApprovalBanner approval={demoSkipApproval as ApprovalModel} agent={agent} onResolve={on} />));
    expect(card().querySelector('.approval-req__ask')!.textContent).toBe('Let claude · amber act without asking?');
    const lines = [...card().querySelectorAll('.approval-req__line')].map((l) => l.textContent);
    expect(lines[0]).toContain('Bypass Permissions');
    expect(lines[0]).toContain('yolo'); // the id the answer is signed over, beside the agent's name for it
    expect(lines[1]).toBe('/home/dev/code/login-fix');
    expect(names(card())).toEqual(['Deny', 'Review']);
    expect(get(card(), 'Deny').disabled).toBe(false); // main-window.md: a switch's Deny has no dwell
    press(card(), 'Review');
    const d = dialog()!;
    expect(d.textContent).toContain('session only');
    expect(get(d, 'Allow').disabled).toBe(true);
    expect(d.textContent).toContain('Read it first');
    dwell(ATTENTION_DWELL_MS + 10);
    press(d, 'Allow');
    expect(on).toHaveBeenCalledWith(expect.objectContaining({ id: demoSkipApproval.id }), true, undefined);
    expect(get(card(), 'Deny').disabled).toBe(true); // one answer: the card is held too
  });

  it('a switch this window has not heard offered can be read in Review but not answered anywhere', () => {
    const on = vi.fn();
    act(() => root.render(<ApprovalCard approval={demoSkipApproval as ApprovalModel} onResolve={on} />));
    expect(get(card(), 'Deny').disabled).toBe(true);
    press(card(), 'Review');
    dwell(ATTENTION_DWELL_MS * 2);
    expect(get(dialog()!, 'Allow').disabled).toBe(true);
    expect(get(dialog()!, 'Deny').disabled).toBe(true);
    expect(dialog()!.textContent).toContain('has not heard this machine offer it');
  });
});
