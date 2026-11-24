// @vitest-environment jsdom
import { readFileSync } from 'node:fs';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage, t } from '../i18n';
import type { AgentTileModel, ConfigOptionModel, SessionModel } from '../model';
import { MAX_LINES, mergeLines, trimLines, type ChatLine, type ChatSource } from '../net/chat';
import { REFUSAL_KEYS, refusalKey } from '../net/refusal';
import { applyCommand, uniqueCommands } from '../net/slash';
import { commandsOf } from '../net/view';
import { ChatActionsContext, type ChatActions } from './chatActions';
import { PromptBox } from './PromptBox';
import { SessionBar } from './SessionBar';
import { choiceSkips, choiceState } from './sessionChoice';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); });

const L = (id: number, over: Partial<ChatLine> = {}): ChatLine => ({ id, atMs: 0, role: 'agent', kind: 'text', text: `m${id}`, path: '', ...over });
const agent = (over: Partial<AgentTileModel> = {}): AgentTileModel => ({ id: 'n/a', name: 'a', accent: '#fff', machine: 'm', workspace: '/w', state: 'idle', preview: '', terminal: false, ...over });
const base: SessionModel = { modes: [], models: [], currentMode: '', currentModel: '', contextTokens: 0, contextWindow: 0 };
const opt = (id: string, name: string, skips = false) => ({ id, name, description: '', skipsPermissions: skips });
const bool = (current: string, skips = true): ConfigOptionModel => ({ id: 'auto', name: 'Auto approve', description: '', category: 'custom', type: 'boolean', current, skipsPermissions: skips, choices: [] });
const effort = (skipLow = false): ConfigOptionModel => ({ id: 'effort', name: 'Effort', description: '', category: 'thought_level', type: 'select', current: 'high', skipsPermissions: false, choices: [opt('low', 'Low', skipLow), opt('high', 'High')] });

describe('1. a boolean that skips permissions: turning it ON asks the owner, turning it OFF never does', () => {
  it('a value is the current one or one that can be sent (the machine decides whether it needs approval)', () => {
    expect(choiceState(bool('true'), 'false')).toBe('ok');
    expect(choiceState(bool('true'), 'true')).toBe('current');
    expect(choiceState(bool('false'), 'true')).toBe('ok');
    expect(choiceState(bool('false'), 'false')).toBe('current');
    expect(choiceSkips(bool('false'), 'true')).toBe(true);
    expect(choiceSkips(bool('false'), 'false')).toBe(false);
    expect(choiceSkips(bool('false', false), 'true')).toBe(false); // a plain boolean is free
  });
  it('the menu shows Off and On both enabled for auto_approve=true, and sends the Off', async () => {
    const setConfig = vi.fn(async () => true);
    act(() => root.render(<SessionBar part="controls" agentKey="n/a" session={{ ...base, config: [bool('true')] }} setConfig={setConfig} />));
    act(() => (host.querySelector('button[aria-haspopup=menu][aria-label*="More settings"]') as HTMLElement).click());
    act(() => (document.querySelector('[role=menuitem]') as HTMLElement).click());
    const items = [...document.querySelectorAll<HTMLButtonElement>('[role=menuitemradio]')];
    const off = items.find((b) => b.textContent?.includes('Off')) as HTMLButtonElement;
    expect(off.disabled).toBe(false);
    await act(async () => { off.click(); });
    expect(setConfig).toHaveBeenCalledWith('n/a', 'auto', 'false');
  });
});

describe('2. the history a window holds is bounded', () => {
  it('20000 entries from a snapshot keep the newest 5000 and count the rest', () => {
    const all = Array.from({ length: 20_000 }, (_, i) => L(i + 1));
    const t0 = trimLines(all);
    expect(t0.lines).toHaveLength(MAX_LINES);
    expect(t0.dropped).toBe(15_000);
    expect(t0.lines[0]?.id).toBe(15_001);
  });
  it('a hostile endless stream of new ids keeps memory bounded and the cost of a chunk does not grow with the stream', () => {
    let held: ChatLine[] = [];
    let dropped = 0;
    const times: number[] = [];
    for (let i = 1; i <= 20_000; i++) {
      const s = performance.now();
      const t0 = trimLines(mergeLines(held, [L(i)]));
      held = t0.lines; dropped += t0.dropped;
      if (i % 5000 === 0) times.push(performance.now() - s);
    }
    expect(held.length).toBeLessThanOrEqual(MAX_LINES);
    expect(dropped).toBe(15_000);
    // After the cap is reached each chunk costs the same: the last is nowhere near 20x the first at the cap.
    expect(Math.max(...times.slice(1))).toBeLessThan(50);
  });
  it('an id that is not held is never dropped, an older one goes in by number, a held one is replaced in place', () => {
    const held = [L(10), L(20)];
    expect(mergeLines(held, [L(5)]).map((l) => l.id)).toEqual([5, 10, 20]); // ids restarted: not lost
    expect(mergeLines(held, [L(15)]).map((l) => l.id)).toEqual([10, 15, 20]);
    expect(mergeLines(held, [L(20, { text: 'new' })]).map((l) => l.text)).toEqual(['m10', 'new']);
  });
  it('a repeat that differs only in role, notice code or params, or approval id, is not taken for "nothing new"', () => {
    const h = [L(1, { kind: 'notice', code: 'a', params: { n: '1' } })];
    expect(mergeLines(h, [L(1, { kind: 'notice', code: 'b', params: { n: '1' } })])[0]?.code).toBe('b');
    expect(mergeLines(h, [L(1, { kind: 'notice', code: 'a', params: { n: '2' } })])[0]?.params).toEqual({ n: '2' });
    expect(mergeLines([L(1)], [L(1, { role: 'user' })])[0]?.role).toBe('user');
    expect(mergeLines([L(1, { kind: 'permission', approvalId: 'x' })], [L(1, { kind: 'permission', approvalId: 'y' })])[0]?.approvalId).toBe('y');
  });
  it('the screen says that earlier entries are not loaded once the cap is reached', () => {
    const source: ChatSource = { connect(on) { on(Array.from({ length: 5200 }, (_, i) => L(i + 1, { role: 'user' })), true); return { close() {} }; } };
    act(() => root.render(<PromptBox agent={agent()} source={source} />));
    expect(host.querySelector('[data-trimmed]')?.textContent).toBe('200 earlier entries are not loaded.');
  });
});

describe('3. nothing of one agent\'s composer carries over to another', () => {
  const queued = (id: number): ChatLine => L(id, { role: 'user', text: `q${id}`, queued: true });
  it('an open "drop the queue" question, a typed text and a stop error of agent A are not on agent B (keyed by the agent)', () => {
    const lines = [L(1, { kind: 'tool', status: 'in_progress', text: '{"tool":"x"}' }), queued(2)];
    const source: ChatSource = { connect(on) { on(lines, true); return { close() {} }; } };
    const actions: ChatActions = { cancelTurn: async () => true };
    const draw = (a: AgentTileModel) => act(() => root.render(<ChatActionsContext.Provider value={actions}><PromptBox key={a.id} agent={a} source={source} /></ChatActionsContext.Provider>));
    draw(agent({ id: 'n/a', state: 'working' }));
    act(() => ([...host.querySelectorAll('button')].find((b) => /Drop the queue/.test(b.textContent ?? '')) as HTMLElement).click());
    expect(host.querySelector('[role=alertdialog]')).not.toBeNull();
    draw(agent({ id: 'n/b', state: 'working' }));
    expect(host.querySelector('[role=alertdialog]')).toBeNull();
  });
});

describe('4. choosing a slash command replaces only the command word', () => {
  const review = { name: 'review', description: '', inputHint: '' };
  it('keeps what was typed after the caret', () => {
    expect(applyCommand('/re and then more text', 3, review)).toEqual({ text: '/review and then more text', caret: 8 });
    expect(applyCommand('/', 1, review)).toEqual({ text: '/review ', caret: 8 });
    expect(applyCommand('/rev\nsecond line', 4, review).text).toBe('/review\nsecond line');
    expect(applyCommand('/review', 7, { ...review, name: '/review' }).text).toBe('/review ');
  });
  it('in the composer: Enter on the highlighted command keeps the rest of the box', () => {
    const s: SessionModel = { ...base, commands: [{ name: 'review', description: 'd', inputHint: '' }] };
    act(() => root.render(<PromptBox agent={agent({ session: s })} send={async () => true} />));
    const box = host.querySelector('textarea') as HTMLTextAreaElement;
    act(() => { Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set?.call(box, '/re keep this'); box.setSelectionRange(3, 3); box.dispatchEvent(new Event('input', { bubbles: true })); });
    act(() => { box.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true })); });
    expect(box.value).toBe('/review keep this');
  });
});

describe('5. a command name that repeats is one row', () => {
  const c = (name: string, description = '') => ({ name, description, inputHint: '' });
  it('the first wins, in the client\'s reading of the event and in the composer', () => {
    expect(commandsOf([c('a', '1'), c('/A', '2'), c('b')]).map((x) => x.description || x.name)).toEqual(['1', 'b']);
    expect(uniqueCommands([c('a'), c('a'), c('b')]).map((x) => x.name)).toEqual(['a', 'b']);
    const s: SessionModel = { ...base, commands: [c('dup', 'first'), c('dup', 'second')] };
    act(() => root.render(<PromptBox agent={agent({ session: s })} send={async () => true} />));
    const box = host.querySelector('textarea') as HTMLTextAreaElement;
    act(() => { Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set?.call(box, '/'); box.setSelectionRange(1, 1); box.dispatchEvent(new Event('input', { bubbles: true })); });
    expect(host.querySelectorAll('[data-slash-option]')).toHaveLength(1);
    expect(host.textContent).toContain('first');
  });
});

describe('6. a choice this machine refuses by its own policy stays marked unavailable, with the reason', () => {
  it('after agent.option_forbidden the choice is disabled with a hint, the refusal is said in words, and the others stay available', async () => {
    const setConfig = vi.fn(async () => 'agent.option_forbidden');
    act(() => root.render(<SessionBar part="controls" agentKey="n/a" session={{ ...base, config: [effort(true)] }} setConfig={setConfig} />));
    act(() => (host.querySelector('button[aria-haspopup=menu][aria-label*="Effort"]') as HTMLElement).click());
    const find = (name: string) => [...document.querySelectorAll<HTMLButtonElement>('[role=menuitemradio]')].find((b) => b.textContent?.includes(name)) as HTMLButtonElement;
    expect(find('Low').disabled).toBe(false); // sent like any other: the machine decides
    await act(async () => { find('Low').click(); });
    expect(setConfig).toHaveBeenCalledWith('n/a', 'effort', 'low');
    expect(host.textContent).toContain("This machine's settings do not let the agent act without asking.");
    act(() => (host.querySelector('button[aria-haspopup=menu][aria-label*="Effort"]') as HTMLElement).click());
    expect(find('Low').disabled).toBe(true);
    expect(find('Low').textContent).toContain('not allowed on this machine');
  });
});

describe('7. entry ids that restart', () => {
  it('a snapshot replaces the held set, so a restarted node\'s low ids are all taken', () => {
    let held = mergeLines([], Array.from({ length: 50 }, (_, i) => L(i + 100)));
    held = mergeLines([], [L(1), L(2)]); // what the screen does with a snapshot: it does not merge into the old set
    expect(held.map((l) => l.id)).toEqual([1, 2]);
    expect(mergeLines(held, [L(3)]).map((l) => l.id)).toEqual([1, 2, 3]);
  });
});

describe('8. a refusal is said in plain words', () => {
  it('maps the known codes to words in both languages, and nothing for an unknown code', () => {
    expect(refusalKey('agent.option_unconfirmed')).toBe('refusal-unconfirmed');
    expect(refusalKey('agent.option_forbidden')).toBe('refusal-forbidden');
    expect(refusalKey('agent.queue_full')).toBe('prompt-queue-full');
    expect(refusalKey('protocol.capability_missing')).toBe('refusal-unsupported');
    expect(refusalKey('something.new')).toBeUndefined();
    for (const lang of ['en', 'tr']) {
      const ids = new Set([...readFileSync(`../../locales/${lang}/web.ftl`, 'utf8').matchAll(/^([a-z0-9-]+) =/gm)].map((m) => m[1]));
      for (const k of [...REFUSAL_KEYS, 'chat-trimmed', 'turn-stop-failed', 'session-change-failed']) expect(ids.has(k), `${lang} ${k}`).toBe(true);
    }
    expect(t('refusal-unsupported')).toBe('This agent\'s machine does not support that yet.');
  });
  it('a setting refused with a code says that, not the general sentence', async () => {
    act(() => root.render(<SessionBar part="controls" agentKey="n/a" session={{ ...base, config: [effort()] }} setConfig={async () => 'protocol.capability_missing'} />));
    act(() => (host.querySelector('button[aria-haspopup=menu][aria-label*="Effort"]') as HTMLElement).click());
    await act(async () => { ([...document.querySelectorAll('[role=menuitemradio]')].find((b) => b.textContent?.includes('Low')) as HTMLElement).click(); });
    expect(host.textContent).toContain('does not support that yet');
    expect(host.textContent).not.toContain('The agent did not switch.');
  });
  it('Stop turn and Drop the queue say why they did not go through', async () => {
    const lines = [L(1, { kind: 'tool', status: 'in_progress', text: '{"tool":"x"}' })];
    const source: ChatSource = { connect(on) { on(lines, true); return { close() {} }; } };
    act(() => root.render(<ChatActionsContext.Provider value={{ cancelTurn: async () => 'protocol.capability_missing' }}><PromptBox agent={agent({ state: 'working' })} source={source} /></ChatActionsContext.Provider>));
    await act(async () => { ([...host.querySelectorAll('button')].find((b) => /Stop turn/.test(b.textContent ?? '')) as HTMLElement).click(); });
    expect(host.textContent).toContain('does not support that yet');
  });
});
