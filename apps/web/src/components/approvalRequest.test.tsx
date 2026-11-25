// @vitest-environment jsdom
// Redesign PR 4: one approval request in three densities (row: Home's queue; card: the side panel, above a terminal and a composer;
// full: the attention dialog). It is a presentation change, so these tests pin that each density keeps its screen's safety rules: the
// dwell where the screen has one, read first (never decided on cut text), Review for what is not shown whole, the danger Allow for an
// agent that would act without asking with its confirmation, one copy of each request in the panel, and no toast over an open list.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage } from '../i18n';
import type { AgentTileModel } from '../model';
import { demoSkipApproval, demoStartApproval, demoStartNoAskApproval } from '../fixtures';
import { ATTENTION_DWELL_MS, attentionPrefs } from '../net/attention';
import { attentionList } from '../net/fleet';
import { markSkipCapable } from '../net/skipApproval';
import type { ApprovalModel } from '../net/view';
import { ApprovalBanner } from './ApprovalBanner';
import { ApprovalCard, FullRequest } from './ApprovalCard';
import { askKind, askTitle, NARROW_PX } from './ApprovalRequest';
import { AttentionLayer } from './AttentionLayer';
import { AttentionQueue } from './FleetConsole';
import { Details } from './Inspector';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
beforeEach(() => {
  setLanguage('en');
  localStorage.clear();
  attentionPrefs.reset();
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
});

const agent: AgentTileModel = { id: 'n1/a1', name: 'codex · teal', accent: 'var(--agent-teal)', machine: 'laptop', machineId: 'n1', workspace: 'w', project: 'agora', state: 'needs-you', preview: '' };
const waiting: AgentTileModel = { ...agent, id: 'n1/a2', name: 'claude · plum', accent: 'var(--agent-plum)' };
const ask = (over: Partial<ApprovalModel> = {}): ApprovalModel =>
  ({ id: 'n1/p1', node: 'n1', localId: 'p1', agentId: 'n1/a1', agentName: 'codex · teal', machineName: 'laptop', action: 'run_command', detail: 'cargo test -p agora-hub', expiresAtMs: 0, ...over });
const buttons = (within: ParentNode = document) => [...within.querySelectorAll('button')];
const button = (text: string, within: ParentNode = document) => buttons(within).find((b) => b.textContent === text) as HTMLButtonElement | undefined;
const wait = (ms: number) => act(async () => { await vi.advanceTimersByTimeAsync(ms); });

describe('the title is the ask, from a closed set of kinds', () => {
  it('names well-known actions and says "asks permission" for anything else, whatever the agent calls it', () => {
    expect(askKind(ask())).toBe('run');
    expect(askKind(ask({ action: 'Edit src/auth.rs' }))).toBe('edit');
    expect(askKind(ask({ action: 'constructor' }))).toBe('other'); // a lookup, never an object's own keys
    expect(askKind(ask({ action: 'run​_command' }))).toBe('other'); // a hidden character is not the word
    expect(askTitle(ask())).toBe('codex · teal wants to run a command');
    expect(askTitle(ask({ action: 'frobnicate' }))).toBe('codex · teal asks permission');
    expect(askTitle(demoStartApproval)).toBe('claude · amber wants to start another agent');
  });
});

describe('row: the Home queue', () => {
  it('keeps the queue\'s dwell: Allow does nothing until it has passed, then answers once; the order is Deny then Allow', async () => {
    vi.useFakeTimers();
    const onResolve = vi.fn(() => true);
    act(() => root.render(<AttentionQueue items={attentionList([ask()], [agent])} machines={[]} now={0} onOpen={() => {}} onResolve={onResolve} />));
    const card = host.querySelector('[data-density="row"]')!;
    expect(card.textContent).toContain('codex · teal wants to run a command');
    expect(buttons(card).map((b) => b.textContent)).toEqual(['Deny', 'Allow']);
    act(() => button('Allow', card)!.click());
    expect(onResolve).not.toHaveBeenCalled();
    expect(button('Allow', card)!.getAttribute('aria-disabled')).toBe('true');
    await wait(ATTENTION_DWELL_MS + 10);
    act(() => button('Allow', card)!.click());
    act(() => button('Allow', card)?.click());
    expect(onResolve).toHaveBeenCalledTimes(1);
  });

  it('never answers cut text: a request that is not shown whole has Review only', () => {
    act(() => root.render(<AttentionQueue items={attentionList([ask({ detail: 'line one\nline two' })], [agent])} machines={[]} now={0} onOpen={() => {}} onResolve={() => true} />));
    const card = host.querySelector('[data-density="row"]')!;
    expect(buttons(card).map((b) => b.textContent)).toEqual(['Review and decide']);
  });
});

describe('card: the side panel, above a terminal, above a composer', () => {
  it('shows the ask whole (never cut) and the agent\'s full text before Allow', () => {
    const long = JSON.stringify({ command: 'cargo test -p agora-hub session::expiry', description: 'x'.repeat(400) });
    act(() => root.render(<ApprovalCard approval={ask({ detail: long })} agent={agent} onResolve={() => {}} />));
    const card = host.querySelector('[data-density="card"]')!;
    const chip = card.querySelector('.approval-req__cmd')!;
    expect(chip.textContent).toBe('run_command · cargo test -p agora-hub session::expiry');
    expect(chip.className).not.toMatch(/truncate|line-clamp/);
    expect(card.querySelector('details pre')!.textContent).toBe(long); // read first: the whole request is one click away
    expect(buttons(card).map((b) => b.textContent)).toEqual(['Deny', 'Allow']);
  });

  it('makes full approval request text visible', () => {
    const detail = JSON.stringify({ command: 'Ru\u202en\u3164 tests', description: 'ça\u034f' });
    act(() => root.render(<FullRequest detail={detail} />));
    const text = host.querySelector('pre')?.textContent ?? '';
    for (const shown of ['Ru\\u{202e}n\\u{3164} tests', 'ça\\u{34f}']) expect(text).toContain(shown);
    expect([...text].filter((c) => ['\u202e', '\u3164', '\u034f'].includes(c))).toEqual([]);
  });

  it('verifies full approval text across isolate and private-use characters', () => {
    const detail = JSON.stringify({ command: '文\u2066件', description: 'ño\ue000' });
    act(() => root.render(<FullRequest detail={detail} />));
    expect(host.querySelector('pre')?.textContent).toContain('文\\u{2066}件');
    expect(host.querySelector('pre')?.textContent).toContain('ño\\u{e000}');
    expect(detail).toContain('\u2066');
    expect(detail).toContain('\ue000');
  });

  it('a narrow pane draws it in the row density with the buttons under the text, the ask still whole', () => {
    let report: (w: number) => void = () => {};
    (globalThis as { ResizeObserver?: unknown }).ResizeObserver = class {
      constructor(cb: (e: { contentRect: { width: number } }[]) => void) { report = (w) => cb([{ contentRect: { width: w } }]); }
      observe() { /* reported by hand */ }
      disconnect() { /* nothing */ }
    };
    vi.useFakeTimers();
    const onResolve = vi.fn();
    act(() => root.render(<ApprovalBanner approval={ask()} agent={agent} onResolve={onResolve} pinned />));
    expect(host.querySelector('[data-density="card"]')).not.toBeNull();
    act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS); });
    act(() => report(NARROW_PX - 170)); // audit screenshot 35: a chat pane about 190 px wide
    const narrow = host.querySelector('section.approval-req')!;
    expect(narrow.getAttribute('data-density')).toBe('row');
    expect(narrow.className).toContain('approval-req--stacked');
    expect(narrow.querySelector('.approval-req__cmd')!.textContent).toBe('run_command · cargo test -p agora-hub');
    expect(buttons(narrow).map((b) => b.textContent)).toEqual(['Deny', 'Allow']);
    // #1045: the buttons moved under the text, so the dwell starts again (T44: a ready Allow must not move under a click).
    act(() => button('Allow', narrow)!.click());
    expect(onResolve).not.toHaveBeenCalled();
    act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS); });
    act(() => button('Allow', narrow)!.click());
    expect(onResolve).toHaveBeenCalledWith(expect.objectContaining({ id: 'n1/p1' }), true);
    act(() => report(NARROW_PX + 40));
    expect(host.querySelector('section.approval-req')!.getAttribute('data-density')).toBe('card');
  });

  // Redesign PR 5: a start and a switch are short cards with Review; their Allow is in Review's dialog, where all of it is shown.
  const dialog = () => document.querySelector('[data-review-dialog]') as HTMLElement;
  it('a switch to act without asking keeps its dwell and "Read it first", and its Allow is the danger button', async () => {
    vi.useFakeTimers();
    markSkipCapable(demoSkipApproval.node);
    const onResolve = vi.fn();
    act(() => root.render(<ApprovalBanner approval={demoSkipApproval as ApprovalModel} onResolve={onResolve} />));
    expect(buttons(host).map((b) => b.textContent)).toEqual(['Deny', 'Review']);
    act(() => button('Review', host)!.click());
    const allow = button('Allow', dialog())!;
    expect(allow.className).toContain('kit-btn--danger');
    expect(buttons(dialog()).map((b) => b.textContent)).toEqual(['Close', 'Deny', 'Allow']);
    expect(allow.disabled).toBe(true);
    expect(dialog().textContent).toContain('Read it first');
    act(() => allow.click());
    expect(onResolve).not.toHaveBeenCalled();
    await wait(ATTENTION_DWELL_MS + 10);
    act(() => button('Allow', dialog())!.click());
    expect(onResolve).toHaveBeenCalledWith(expect.objectContaining({ id: demoSkipApproval.id }), true, undefined);
  });

  it('a start that would not ask has a danger Allow that still asks for the confirmation first; a plain start keeps the primary', () => {
    vi.useFakeTimers();
    const onResolve = vi.fn();
    act(() => root.render(<ApprovalCard approval={demoStartNoAskApproval as ApprovalModel} onResolve={onResolve} />));
    expect(buttons(host).map((b) => b.textContent)).toEqual(['Deny', 'Review']);
    act(() => button('Review', host)!.click());
    expect(buttons(dialog()).map((b) => b.textContent)).toEqual(['Close', 'Deny', 'Allow']);
    expect(button('Allow', dialog())!.className).toContain('kit-btn--danger');
    act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS); }); // Review's own T44 dwell
    act(() => button('Allow', dialog())!.click());
    expect(onResolve).not.toHaveBeenCalled();
    expect(dialog().querySelector('[role=alertdialog]')).not.toBeNull();
    expect(document.activeElement?.textContent).toBe('Keep asking'); // T15: the primary, focused answer keeps asking
    act(() => button('Start without asking', dialog())!.click());
    expect(onResolve).toHaveBeenCalledWith(true, { grant: 0, nested: false });
    act(() => root.render(<ApprovalCard approval={demoStartApproval as ApprovalModel} onResolve={onResolve} />));
    act(() => button('Review', host)!.click());
    expect(button('Allow', dialog())!.className).toContain('kit-btn--primary');
    expect(button('Allow', dialog())!.className).not.toContain('kit-btn--danger');
  });
});

describe('full: the attention dialog', () => {
  it('its title is the ask with the avatar and the machine, and it keeps the dwell', async () => {
    vi.useFakeTimers();
    const onResolve = vi.fn(async () => undefined);
    act(() => root.render(<AttentionLayer items={attentionList([ask()], [agent])} onResolve={onResolve} onReview={() => {}} />));
    await wait(0);
    const dialog = document.querySelector('.attention-card')!;
    expect(document.getElementById(dialog.getAttribute('aria-labelledby')!)!.textContent).toContain('codex · teal wants to run a command');
    expect(dialog.querySelector('[data-density="full"] .kit-avatar')).not.toBeNull();
    expect(dialog.querySelector('.kit-tag')!.textContent).toBe('laptop');
    expect(buttons(dialog).map((b) => b.textContent)).toEqual(['Later', 'Deny', 'Allow']);
    act(() => button('Allow', dialog)!.click());
    expect(onResolve).not.toHaveBeenCalled();
    await wait(ATTENTION_DWELL_MS + 10);
    act(() => button('Allow', dialog)!.click());
    expect(onResolve).toHaveBeenCalledTimes(1);
  });

  it('a request it cannot show whole is read in its full card: Review only, never Allow', async () => {
    vi.useFakeTimers();
    act(() => root.render(<AttentionLayer items={attentionList([ask({ detail: 'a\nb' })], [agent])} onResolve={vi.fn()} onReview={() => {}} />));
    await wait(ATTENTION_DWELL_MS + 10);
    expect(buttons(document.querySelector('.attention-card')!).map((b) => b.textContent)).toEqual(['Later', 'Review']);
  });
});

describe('one copy of each request', () => {
  const details = (approvals: ApprovalModel[], attention: AgentTileModel[], onOpen = vi.fn()) => (
    <Details tab="needs" fleet={attention} attention={attention} approvals={approvals} changed={[]} boards={[]} messages={[]} onOpen={onOpen} onResolve={() => {}} />);

  it('the panel lists an agent once: by its request\'s card, which opens the agent; one that waits in its terminal keeps its row', () => {
    const onOpen = vi.fn();
    act(() => root.render(details([ask()], [agent, waiting], onOpen)));
    expect(host.textContent!.split('codex · teal').length - 1).toBe(1);
    expect([...host.querySelectorAll('.ide-row')].map((r) => r.textContent)).toEqual([expect.stringContaining('claude · plum')]);
    act(() => (host.querySelector('.approval-req__open') as HTMLButtonElement).click());
    expect(onOpen).toHaveBeenCalledWith('n1/a1');
  });

  it('a start and a switch are listed once too: by their cards, which open the agent; a switch this window cannot read keeps the row', () => {
    markSkipCapable(demoSkipApproval.node);
    const onOpen = vi.fn();
    for (const x of [demoStartApproval, demoStartNoAskApproval, demoSkipApproval]) {
      act(() => root.render(details([{ ...(x as ApprovalModel), agentId: agent.id }], [agent], onOpen)));
      expect(host.querySelectorAll('.ide-row')).toHaveLength(0);
      expect(host.querySelectorAll('[data-review-card]')).toHaveLength(1);
      act(() => (host.querySelector('.approval-req__open') as HTMLButtonElement).click());
    }
    expect(onOpen.mock.calls).toEqual([[agent.id], [agent.id], [agent.id]]);
    act(() => root.render(details([ask({ action: 'agent.skip_permissions', detail: '{}', skipUnknown: true })], [agent])));
    expect([...host.querySelectorAll('.ide-row')].map((r) => r.textContent)).toEqual([expect.stringContaining('codex · teal')]);
  });

  it('the toast is hidden while a Needs you list is on screen, and comes back when it closes', () => {
    attentionPrefs.setMode('banner');
    const layer = <AttentionLayer key="layer" items={attentionList([ask()], [agent])} onResolve={vi.fn()} onReview={() => {}} />;
    act(() => root.render(<>{layer}</>));
    expect(document.querySelector('.attention-banner')).not.toBeNull();
    expect(document.querySelector('.attention-banner')!.className).toContain('end-4'); // logical: the end side in either direction
    act(() => root.render(<>{layer}<div key="panel">{details([ask()], [agent])}</div></>));
    expect(document.querySelector('.attention-banner')).toBeNull();
    act(() => root.render(<>{layer}</>));
    expect(document.querySelector('.attention-banner')).not.toBeNull();
  });
});
