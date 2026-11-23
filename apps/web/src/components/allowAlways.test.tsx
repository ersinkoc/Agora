// @vitest-environment jsdom
// ADR-0101 step 4, in a DOM: "Allow always" appears only where the request offers it, answers ALWAYS, takes Allow's place when the agent
// offers nothing else, keeps the agent's own words out of the button's name, and "Asks less" shows only while it holds such an answer.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage } from '../i18n';
import type { AgentTileModel } from '../model';
import { attentionList } from '../net/fleet';
import { ATTENTION_DWELL_MS, attentionPrefs } from '../net/attention';
import type { ApprovalModel } from '../net/view';
import { ApprovalCard } from './ApprovalCard';
import { ApprovalBanner } from './ApprovalBanner';
import { AttentionLayer } from './AttentionLayer';
import { AsksLessChip } from './AllowAlways';
import { AttentionQueue } from './FleetConsole';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
beforeEach(() => {
  setLanguage('en');
  localStorage.clear();
  attentionPrefs.reset();
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
});

const ask = (over: Partial<ApprovalModel> = {}): ApprovalModel =>
  ({ id: 'n1/p1', node: 'n1', localId: 'p1', agentId: 'n1/a1', agentName: 'wstack · amber', machineName: 'laptop', action: 'run_command', detail: 'git status', expiresAtMs: 0, ...over });
const both = ask({ always: { label: 'Always allow git status', optionId: 'allow-always-1', only: false } });
const only = ask({ always: { label: '', optionId: 'edits', only: true } });
const names = () => [...document.querySelectorAll('button')].map((b) => b.textContent);
const press = (text: string) => { const b = [...document.querySelectorAll('button')].find((x) => x.textContent === text); if (!b) throw new Error(`no ${text}`); act(() => b.click()); };
// #1045: a card's answers wait out the T44 dwell, as on every surface.
const dwell = () => act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS); });

describe('the approval card', () => {
  it('has no third button when the request does not offer it', () => {
    act(() => root.render(<ApprovalCard approval={ask()} onResolve={() => {}} />));
    expect(names()).toEqual(['Deny', 'Allow']);
    expect(document.body.textContent).not.toContain('remembers');
  });

  it('adds "Allow always" between Deny and Allow, a secondary button that answers always, with the note and the agent\'s own words', () => {
    vi.useFakeTimers();
    // Each answer fails, so the card is answerable again after a fresh dwell and every button can be tried.
    const onResolve = vi.fn(() => false);
    act(() => root.render(<ApprovalCard approval={both} onResolve={onResolve} />));
    // Redesign PR 4: the order of every request, the attention dialog's: Deny, "Allow always", then Allow (the primary, last).
    expect(names()).toEqual(['Deny', 'Allow always', 'Allow']);
    const always = [...document.querySelectorAll('button')][1]!;
    expect(always.className).not.toContain('kit-btn--primary');
    expect([...document.querySelectorAll('button')][2]!.className).toContain('kit-btn--primary');
    expect(always.getAttribute('aria-describedby')).toBe('always-note-card-n1/p1');
    const note = document.getElementById('always-note-card-n1/p1')!;
    expect(note.textContent).toContain('wstack · amber remembers this answer');
    expect(note.textContent).toContain('In its own words: “Always allow git status”');
    // Codex's review: "for this session" is not said for a kind nobody checked.
    expect(document.body.textContent).not.toContain('for this session');
    dwell();
    press('Allow always');
    expect(onResolve).toHaveBeenLastCalledWith(true, undefined, true);
    dwell();
    press('Allow');
    expect(onResolve).toHaveBeenLastCalledWith(true);
    dwell();
    press('Deny');
    expect(onResolve).toHaveBeenLastCalledWith(false);
    expect(onResolve).toHaveBeenCalledTimes(3);
  });

  it('puts "Allow always" in the place of Allow, as the primary, when the agent offers nothing else', () => {
    vi.useFakeTimers();
    const onResolve = vi.fn();
    act(() => root.render(<ApprovalCard approval={only} onResolve={onResolve} />));
    expect(names()).toEqual(['Deny', 'Allow always']);
    expect([...document.querySelectorAll('button')][1]!.className).toContain('kit-btn--primary');
    expect(document.body.textContent).toContain('This agent offers no single answer for this request');
    expect(document.body.textContent).not.toContain('In its own words'); // no label: nothing quoted
    dwell();
    press('Allow always');
    expect(onResolve).toHaveBeenLastCalledWith(true, undefined, true);
  });

  it('never offers it on a start request, even if the machine said so', () => {
    act(() => root.render(<ApprovalCard approval={ask({ action: 'agent.start', detail: '{}', start: { agent: '', agentKind: '', node: '', kind: 'k', folder: '', prompt: '', inside: false, worktree: false, depth: 1, grantAllowed: false, settings: [], options: [] }, always: { label: 'x', optionId: 'o', only: false } })} onResolve={() => {}} />));
    expect(names()).not.toContain('Allow always');
  });
});

describe('the banner in the centre panel', () => {
  it('offers it only when offered and answers always', () => {
    vi.useFakeTimers();
    const onResolve = vi.fn();
    act(() => root.render(<ApprovalBanner approval={ask()} onResolve={onResolve} />));
    expect(names()).toEqual(['Deny', 'Allow']);
    act(() => root.render(<ApprovalBanner approval={both} onResolve={onResolve} pinned />));
    expect(names()).toEqual(['Deny', 'Allow always', 'Allow']);
    dwell();
    press('Allow always');
    expect(onResolve).toHaveBeenLastCalledWith(both, true, undefined, true);
    act(() => root.render(<ApprovalBanner approval={only} onResolve={onResolve} />));
    expect(names()).toEqual(['Deny', 'Allow always']);
  });
});

describe('the attention card', () => {
  const agent: AgentTileModel = { id: 'n1/a1', name: 'wstack · amber', accent: '', machine: 'laptop', machineId: 'n1', workspace: 'w', state: 'needs-you', preview: '' };
  beforeEach(() => {
    vi.useFakeTimers();
    vi.spyOn(document, 'hasFocus').mockImplementation(() => true);
  });
  const wait = (ms: number) => act(async () => { await vi.advanceTimersByTimeAsync(ms); });

  it('adds "Allow always" beside Allow once the dwell is over, and sends ALWAYS', async () => {
    const onResolve = vi.fn(async () => undefined);
    act(() => root.render(<AttentionLayer items={attentionList([both], [agent])} onResolve={onResolve} onReview={() => {}} />));
    await wait(0);
    expect(names()).toEqual(['Later', 'Deny', 'Allow always', 'Allow']);
    press('Allow always'); // too early: nothing
    expect(onResolve).not.toHaveBeenCalled();
    await wait(ATTENTION_DWELL_MS + 10);
    press('Allow always');
    expect(onResolve).toHaveBeenCalledWith(both, true, undefined, true);
  });

  it('sends a request that offers only "always" to its full card', async () => {
    act(() => root.render(<AttentionLayer items={attentionList([only], [agent])} onResolve={() => undefined} onReview={() => {}} />));
    await wait(0);
    expect(names()).not.toContain('Allow');
    expect(names()).not.toContain('Allow always');
    expect(names()).toContain('Review');
  });
});

describe('the fleet console\'s rows', () => {
  it('keep Allow and Deny, and have no plain Allow for an always-only request', () => {
    const agent: AgentTileModel = { id: 'n1/a1', name: 'wstack · amber', accent: '', machine: 'laptop', machineId: 'n1', workspace: 'w', state: 'needs-you', preview: '' };
    act(() => root.render(<AttentionQueue items={attentionList([both], [agent])} machines={[]} now={0} onResolve={() => {}} onOpen={() => {}} />));
    expect(names()).toEqual(['Deny', 'Allow']); // Deny first, the primary Allow last, as in the attention dialog
    act(() => root.render(<AttentionQueue items={attentionList([only], [agent])} machines={[]} now={0} onResolve={() => {}} onOpen={() => {}} />));
    expect(names()).toEqual(['Review and decide']);
  });
});

describe('"Asks less"', () => {
  it('shows while the agent holds an always answer, lists them in its name, and gives way to "Acts freely"', () => {
    act(() => root.render(<AsksLessChip agent={{ alwaysAllowed: [] }} />));
    expect(document.querySelector('[data-asks-less]')).toBeNull();
    act(() => root.render(<AsksLessChip agent={{ alwaysAllowed: ['run_command', 'write_file'] }} />));
    const chip = document.querySelector('[data-asks-less]')!;
    expect(chip.textContent).toBe('Asks less');
    expect(chip.getAttribute('tabindex')).toBe('0');
    const name = chip.getAttribute('aria-label') ?? '';
    expect(name).toContain('You let it remember the answers to 2 of its requests');
    expect(name).toContain('run_command · write_file');
    expect(name).not.toContain('for this session');
    // Neutral chrome, never the "needs you" colour of "Acts freely".
    expect(chip.className).not.toContain('needs-you');
    act(() => root.render(<AsksLessChip agent={{ alwaysAllowed: ['run_command'], skipsPermissions: true }} />));
    expect(document.querySelector('[data-asks-less]')).toBeNull();
    act(() => root.render(<AsksLessChip agent={{ alwaysAllowed: ['run_command'] }} nested />));
    expect(document.querySelector('[data-asks-less]')!.hasAttribute('tabindex')).toBe(false);
  });
});
