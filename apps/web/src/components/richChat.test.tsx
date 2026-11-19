// @vitest-environment jsdom
import { readFileSync } from 'node:fs';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { renderToStaticMarkup } from 'react-dom/server';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage, t } from '../i18n';
import { demoRichConversation } from '../demoChat';
import type { AgentTileModel, ConfigOptionModel, SessionModel } from '../model';
import type { ChatLine, ChatSource } from '../net/chat';
import { transcript } from '../net/transcript';
import { ChatActionsContext, type ChatActions } from './chatActions';
import { Item, ToolSteps } from './ChatEntry';
import { PromptBox } from './PromptBox';
import { SessionBar } from './SessionBar';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); });

const L = (id: number, over: Partial<ChatLine> = {}): ChatLine => ({ id, atMs: 0, role: 'agent', kind: 'text', text: `m${id}`, path: '', ...over });
const call = (id: number, over: Partial<ChatLine> = {}): ChatLine => L(id, { kind: 'tool', text: '{"tool":"Read","arguments":{"file_path":"a.rs"}}', callId: `c${id}`, ...over });
const agent = (over: Partial<AgentTileModel> = {}): AgentTileModel => ({ id: 'n/a', name: 'a', accent: '#fff', machine: 'm', workspace: '/w', state: 'idle', preview: '', terminal: false, ...over });
const draw = (lines: ChatLine[]) => renderToStaticMarkup(<>{transcript(lines).map((i) => <Item key={i.id} item={i} notice={() => null} />)}</>);

describe('a tool call with the node\'s data', () => {
  it('shows a status mark for each state, said in words, and nothing without a status (an old node)', () => {
    for (const [status, word] of [['pending', 'Waiting to start'], ['in_progress', 'Running'], ['completed', 'Done'], ['failed', 'Failed']] as const) {
      expect(renderToStaticMarkup(<ToolSteps lines={[call(1, { status })]} />), status).toContain(`aria-label="${word}"`);
    }
    const old = renderToStaticMarkup(<ToolSteps lines={[call(1)]} />);
    expect(old).not.toContain('role="img"');
    expect(old).not.toContain('data-took');
  });
  it('shows how long it took, only once the node measured both ends', () => {
    expect(renderToStaticMarkup(<ToolSteps lines={[call(1, { status: 'completed', startedMs: 1000, endedMs: 1600 })]} />)).toContain('0.6 s');
    expect(renderToStaticMarkup(<ToolSteps lines={[call(1, { status: 'in_progress', startedMs: 1000 })]} />)).not.toContain('data-took');
  });
  it('uses the node\'s kind for the sentence and the files it touched as chips, once each', () => {
    const html = renderToStaticMarkup(<ToolSteps lines={[call(1, { text: '{"tool":"x","arguments":{}}', callKind: 'search', locations: [{ path: 'src/a.rs', line: 3 }, { path: 'src/b.rs', line: 0 }, { path: 'c.rs', line: 0 }] })]} />);
    expect(html).toContain('Search');
    expect(html).toContain('a.rs:3');
    expect(html).toContain('title="src/a.rs:3"');
    expect(html).toContain('+1'); // two chips and "+1"
    expect(html).not.toContain('<a '); // a path is never a link
  });
  it('folds what a run printed to its last lines, with the rest on request, and says when the node shortened it', () => {
    const out = Array.from({ length: 30 }, (_, i) => `line ${i + 1}`).join('\n');
    act(() => root.render(<ToolSteps lines={[call(1, { text: '{"tool":"execute","arguments":{"command":"cargo test"}}', callKind: 'run', status: 'completed', output: out, outputCut: true })]} open />));
    const pre = host.querySelector('[data-output] pre') as HTMLElement;
    expect(pre.textContent?.split('\n')[0]).toBe('line 19');
    expect(pre.textContent).toContain('line 30');
    expect(host.textContent).toContain('The output was shortened.');
    const more = [...host.querySelectorAll('button')].find((b) => /Show all 30 lines/.test(b.textContent ?? '')) as HTMLElement;
    act(() => more.click());
    expect((host.querySelector('[data-output] pre') as HTMLElement).textContent).toContain('line 1\n');
  });
  it('shows a running command\'s output from the start, and draws the output as text, never as HTML', () => {
    act(() => root.render(<ToolSteps lines={[call(1, { text: '{"tool":"execute","arguments":{"command":"x"}}', status: 'in_progress', output: '<img src=x onerror=alert(1)>' })]} open />));
    expect(host.querySelector('details details')?.hasAttribute('open')).toBe(true);
    expect(host.querySelector('img')).toBeNull();
    expect(host.textContent).toContain('<img src=x onerror=alert(1)>');
  });
  it('opens a group with a failed call, folded or not, and says so; a spinner follows the calls themselves', () => {
    const html = renderToStaticMarkup(<ToolSteps lines={[call(1, { status: 'failed' })]} flag="call-failed" />);
    expect(html).toMatch(/<details[^>]*\sopen/);
    expect(html).toContain('A step failed');
    expect(renderToStaticMarkup(<ToolSteps lines={[call(1, { status: 'completed' })]} running />)).not.toContain('animate-spin'); // the agent works, but no call does
    expect(renderToStaticMarkup(<ToolSteps lines={[call(1, { status: 'in_progress' })]} />)).toContain('animate-spin');
  });
  it('redraws a call that changed in place (the group is not memoised on its length)', () => {
    act(() => root.render(<ToolSteps lines={[call(1, { status: 'in_progress' })]} />));
    expect(host.innerHTML).toContain('aria-label="Running"');
    act(() => root.render(<ToolSteps lines={[call(1, { status: 'completed' })]} />));
    expect(host.innerHTML).toContain('aria-label="Done"');
    expect(host.innerHTML).not.toContain('aria-label="Running"');
  });
});

describe('thinking, the plan, permission records and waiting prompts', () => {
  it('draws thinking folded as "Thought for N s", dashed, as plain text; "Thinking…" while it goes on', () => {
    const lines = [L(1, { kind: 'thought', atMs: 10_000, text: '<b>x</b>' }), L(2, { atMs: 19_000 })];
    const html = draw(lines);
    expect(html).toContain('Thought for 9.0 s');
    expect(html).toContain('border-dashed');
    expect(html).toContain('&lt;b&gt;x&lt;/b&gt;');
    expect(html).not.toMatch(/<details[^>]*\sopen/);
    const live = renderToStaticMarkup(<Item item={transcript([L(1, { kind: 'thought', atMs: 5000 })])[1] as never} running notice={() => null} />);
    expect(live).toContain('Thinking…');
  });
  it('draws the plan with a tick per done step, the step in hand marked, and a progress bar', () => {
    const html = draw([L(1, { kind: 'plan', text: 'a\nb\nc\nd', steps: [{ text: 'a', status: 'completed' }, { text: 'b', status: 'completed' }, { text: 'c', status: 'in_progress' }, { text: 'd', status: 'pending' }] })]);
    expect(html).toContain('2 of 4 done');
    expect(html).toContain('aria-valuenow="50"');
    expect(html.match(/aria-current="step"/g)).toHaveLength(1);
    expect(html).toMatch(/data-status="completed"/);
  });
  it('a plan from an old node shows no progress and no ticks', () => {
    const html = draw([L(1, { kind: 'plan', text: 'a\nb' })]);
    expect(html).not.toContain('progressbar');
    expect(html).not.toContain('aria-current');
  });
  it('a permission record is a view: its state in words, no Allow or Deny, and a way to the pinned request only while it waits', () => {
    const rec = (outcome: ChatLine['outcome']) => transcript([L(1, { kind: 'permission', text: 'run_command', approvalId: 'ap', outcome })])[0] as never;
    for (const [outcome, word] of [['pending', 'Asked for permission'], ['approved', 'Permission given'], ['denied', 'Permission refused'], ['cancelled', 'Not answered']] as const) {
      const html = renderToStaticMarkup(<Item item={rec(outcome)} notice={() => null} onShowRequest={() => {}} />);
      expect(html, outcome).toContain(word);
      expect(html).toContain('run_command');
      expect(html).not.toMatch(/Allow|Deny|Approve/);
      expect(html.includes('Show the request'), outcome).toBe(outcome === 'pending');
    }
    expect(renderToStaticMarkup(<Item item={rec('pending')} notice={() => null} />)).not.toContain('Show the request'); // nothing pinned to go to
  });
  it('a prompt behind the turn says Waiting, and one the node will not send says Not sent', () => {
    const user = transcript([L(1, { role: 'user', queued: true })])[0] as never;
    expect(renderToStaticMarkup(<Item item={user} queued="waiting" notice={() => null} />)).toContain('Waiting');
    expect(renderToStaticMarkup(<Item item={user} queued="not-sent" notice={() => null} />)).toContain('Not sent');
    expect(renderToStaticMarkup(<Item item={user} notice={() => null} />)).not.toContain('Waiting');
  });
  it('the demo conversation of the design exercises all of it without throwing', () => {
    const html = draw(demoRichConversation());
    for (const word of ['Thought for', 'Asked for permission', '2 of 4 done', 'Failed']) expect(html).toContain(word);
  });
});

// ---- the screen

const rich = (): ChatLine[] => [L(1, { role: 'user', text: 'go' }), call(2, { status: 'in_progress', startedMs: 1000 })];
function mount(lines: ChatLine[], a: AgentTileModel, actions: ChatActions = {}, extra: { send?: never } = {}) {
  let push: (l: ChatLine[], replace?: boolean) => void = () => {};
  const source: ChatSource = { connect(on) { push = (l, replace = false) => on(l, replace); on(lines, true); return { close() {} }; } };
  act(() => root.render(<ChatActionsContext.Provider value={actions}><PromptBox agent={a} source={source} send={async () => true} {...extra} approvals={<div id="bar">bar</div>} /></ChatActionsContext.Provider>));
  return { push: (l: ChatLine[], replace?: boolean) => act(() => push(l, replace)) };
}
const buttonNamed = (re: RegExp) => [...host.querySelectorAll('button')].find((b) => re.test(`${b.textContent} ${b.getAttribute('aria-label') ?? ''}`)) as HTMLElement | undefined;

describe('Stop turn', () => {
  it('is offered only while the agent works, only with what shows a node that speaks the rich form, and only when the app can send it', () => {
    const cancelTurn = vi.fn(async () => true);
    mount(rich(), agent({ state: 'working' }), { cancelTurn });
    expect(buttonNamed(/Stop turn/)).toBeTruthy();
    act(() => root.render(<ChatActionsContext.Provider value={{ cancelTurn }}><PromptBox agent={agent({ state: 'idle' })} source={{ connect(on) { on(rich(), true); return { close() {} }; } }} /></ChatActionsContext.Provider>));
    expect(buttonNamed(/Stop turn/)).toBeUndefined(); // not working
  });
  it('is not shown for an old node: its entries carry none of the new fields and it never answered a prompt in the rich form', () => {
    mount([L(1, { role: 'user' }), L(2)], agent({ state: 'working' }), { cancelTurn: async () => true });
    expect(buttonNamed(/Stop turn/)).toBeUndefined();
  });
  it('is shown once the node answered a prompt in the rich form, or offers settings or commands, even before any rich entry', () => {
    mount([L(1)], agent({ id: 'n/a', state: 'working' }), { cancelTurn: async () => true, rich: new Set(['n/a']) });
    expect(buttonNamed(/Stop turn/)).toBeTruthy();
    act(() => root.unmount());
    root = createRoot(host);
    mount([L(1)], agent({ state: 'working', session: { ...base, commands: [{ name: 'x', description: '', inputHint: '' }] } }), { cancelTurn: async () => true });
    expect(buttonNamed(/Stop turn/)).toBeTruthy();
  });
  it('sends AgentCancelTurn without dropping the queue, for this agent, and is told apart from stopping the agent', async () => {
    const cancelTurn = vi.fn(async () => true);
    mount(rich(), agent({ state: 'working' }), { cancelTurn });
    const b = buttonNamed(/Stop turn/) as HTMLElement;
    expect(b.getAttribute('title')).toMatch(/Ends this turn only\. The agent keeps running/);
    expect(host.textContent).not.toMatch(/Stop agent/);
    await act(async () => { b.click(); });
    expect(cancelTurn).toHaveBeenCalledWith('n/a', false);
  });
  it('says when the turn did not stop', async () => {
    mount(rich(), agent({ state: 'working' }), { cancelTurn: async () => false });
    await act(async () => { (buttonNamed(/Stop turn/) as HTMLElement).click(); });
    expect(host.textContent).toContain('The turn did not stop.');
  });
});

describe('the queue', () => {
  const queued = (id: number, text: string) => L(id, { role: 'user', text, queued: true });
  it('shows "1 queued" with the waiting prompt\'s text, and goes when it is sent', () => {
    const s = mount([...rich(), queued(3, 'Then update the changelog')], agent({ state: 'working' }), { cancelTurn: async () => true });
    expect(host.querySelector('[data-queue]')?.textContent).toContain('1 queued');
    expect(host.querySelector('[data-queue]')?.textContent).toContain('Then update the changelog');
    s.push([L(3, { role: 'user', text: 'Then update the changelog' })]);
    expect(host.querySelector('[data-queue]')).toBeNull();
    expect(host.querySelectorAll('[data-role=user]')).toHaveLength(2); // the same entry, not a second one
  });
  it('asks before dropping, says it also stops the turn, and sends drop_queue only on yes', async () => {
    const cancelTurn = vi.fn(async () => true);
    mount([...rich(), queued(3, 'a'), queued(4, 'b')], agent({ state: 'working' }), { cancelTurn });
    expect(host.querySelector('[data-queue]')?.textContent).toContain('2 queued');
    act(() => (buttonNamed(/Drop the queue/) as HTMLElement).click());
    expect(cancelTurn).not.toHaveBeenCalled();
    expect(host.querySelector('[role=alertdialog]')?.textContent).toContain('also stops the agent');
    act(() => (buttonNamed(/Keep them/) as HTMLElement).click());
    expect(host.querySelector('[role=alertdialog]')).toBeNull();
    expect(cancelTurn).not.toHaveBeenCalled();
    act(() => (buttonNamed(/Drop the queue/) as HTMLElement).click());
    await act(async () => { (buttonNamed(/Drop and stop the turn/) as HTMLElement).click(); });
    expect(cancelTurn).toHaveBeenCalledWith('n/a', true);
  });
  it('the notice that the queue was dropped clears the waiting marks: nothing waits, the prompts read Not sent', () => {
    const s = mount([...rich(), queued(3, 'a')], agent({ state: 'working' }), { cancelTurn: async () => true });
    expect(host.querySelector('[data-queue]')).not.toBeNull();
    s.push([L(4, { kind: 'notice', code: 'adapter.queue_dropped', params: { count: '1' } })]);
    expect(host.querySelector('[data-queue]')).toBeNull();
    expect(host.textContent).toContain('Not sent');
    expect(host.querySelector('[data-queued=waiting]')).toBeNull();
  });
  it('offers no way to drop it on an old node', () => {
    mount([L(1), queued(2, 'x')], agent({ state: 'working' }), {});
    expect(buttonNamed(/Drop the queue/)).toBeUndefined();
  });
});

describe('updates in place on the screen', () => {
  it('one row for a call however often it is updated, and the tail is followed without a jump pill', () => {
    const s = mount(rich(), agent({ state: 'working' }));
    const log = host.querySelector<HTMLElement>('[role=log]') as HTMLElement;
    let top = 0;
    Object.defineProperty(log, 'scrollHeight', { get: () => 900 });
    Object.defineProperty(log, 'clientHeight', { get: () => 500 });
    Object.defineProperty(log, 'scrollTop', { get: () => top, set: (v: number) => { top = v; } });
    s.push([call(2, { status: 'in_progress', output: 'a' })]);
    s.push([call(2, { status: 'completed', output: 'a\nb', startedMs: 1000, endedMs: 2000 })]);
    expect(host.querySelectorAll('[data-steps]')).toHaveLength(1);
    expect(host.querySelectorAll('[data-status]').length).toBeGreaterThan(0);
    expect(host.innerHTML).toContain('aria-label="Done"');
    expect(top).toBe(900); // still at the tail
  });
  it('a reader who scrolled up is not moved and gets no "new" count for an update that adds nothing', () => {
    const lines = Array.from({ length: 6 }, (_, i) => L(i + 1, { role: 'user' }));
    const s = mount([...lines, call(7, { status: 'in_progress' })], agent({ state: 'working' }));
    const log = host.querySelector<HTMLElement>('[role=log]') as HTMLElement;
    let top = 0;
    Object.defineProperty(log, 'scrollHeight', { get: () => 2000 });
    Object.defineProperty(log, 'clientHeight', { get: () => 500 });
    Object.defineProperty(log, 'scrollTop', { get: () => top, set: (v: number) => { top = v; } });
    act(() => { top = 0; log.dispatchEvent(new Event('scroll')); });
    s.push([call(7, { status: 'completed', endedMs: 5 })]);
    expect(top).toBe(0);
    expect(host.textContent).not.toMatch(/new\)?$|, \d+ new/);
    s.push([L(8, { role: 'user' })]);
    expect(host.textContent).toContain('1 new');
  });
  it('a failed call opens its group on the screen, and a group before a request that waits opens too', () => {
    mount([call(1, { status: 'failed' }), L(2, { kind: 'text', text: 'between' }), call(3, { status: 'in_progress' }), L(4, { kind: 'permission', text: 'x', outcome: 'pending', approvalId: 'ap' }), call(5, { status: 'completed' })], agent({ state: 'working' }));
    const groups = [...host.querySelectorAll<HTMLElement>('[data-steps]')];
    expect(groups.map((g) => g.dataset.flag)).toEqual(['call-failed', 'needs-you', undefined]);
    expect(groups.slice(0, 2).every((g) => g.hasAttribute('open'))).toBe(true);
  });
  it('"Show the request" takes the reader to the pinned bar and answers nothing', () => {
    mount([call(1, { status: 'in_progress' }), L(2, { kind: 'permission', text: 'x', outcome: 'pending', approvalId: 'ap' })], agent({ state: 'working' }));
    const bar = host.querySelector('[data-pinned-approvals]') as HTMLElement;
    const focus = vi.spyOn(bar, 'focus');
    act(() => (buttonNamed(/Show the request/) as HTMLElement).click());
    expect(focus).toHaveBeenCalled();
  });
});

const base: SessionModel = { modes: [], models: [], currentMode: '', currentModel: '', contextTokens: 62_000, contextWindow: 200_000 };
const opt = (id: string, name: string, skips = false) => ({ id, name, description: '', skipsPermissions: skips });
const effort: ConfigOptionModel = { id: 'effort', name: 'Reasoning', description: '', category: 'thought_level', type: 'select', current: 'high', skipsPermissions: false, choices: [opt('low', 'Low'), opt('high', 'High')] };
const mode: ConfigOptionModel = { id: 'mode', name: 'Mode', description: '', category: 'mode', type: 'select', current: 'plan', skipsPermissions: false, choices: [opt('plan', 'Plan'), opt('yolo', 'Bypass', true)] };
const open = (label: RegExp) => { act(() => (host.querySelector(`button[aria-haspopup=menu][aria-label*="${label.source}"]`) as HTMLElement).click()); };

describe('the agent\'s own settings', () => {
  it('draws mode, model and effort as menus from the config, in that order, with the window\'s own words', () => {
    const html = renderToStaticMarkup(<SessionBar part="controls" agentKey="n/a" session={{ ...base, config: [effort, mode] }} setConfig={async () => true} />);
    expect(html.indexOf('aria-label="Mode: Plan"')).toBeGreaterThan(-1);
    expect(html.indexOf('aria-label="Mode: Plan"')).toBeLessThan(html.indexOf('aria-label="Effort: High"'));
    expect(html).not.toContain('Reasoning'); // the agent's own name for effort is not the window's word
  });
  it('a config mode replaces the older mode picker, which would be the same thing twice', () => {
    const html = renderToStaticMarkup(<SessionBar part="controls" agentKey="n/a" session={{ ...base, modes: [opt('build', 'Build')], currentMode: 'build', config: [mode] }} />);
    expect(html.match(/aria-label="Mode:/g)).toHaveLength(1);
    expect(html).toContain('Mode: Plan');
  });
  it('without a config nothing changes: no effort menu is invented', () => {
    expect(renderToStaticMarkup(<SessionBar part="controls" agentKey="n/a" session={{ ...base, modes: [opt('build', 'Build')], currentMode: 'build' }} />).toLowerCase()).not.toContain('effort');
  });
  it('choosing a value sends AgentSetConfig for this agent, this option and this value', async () => {
    const setConfig = vi.fn(async () => true);
    act(() => root.render(<SessionBar part="controls" agentKey="n/a" session={{ ...base, config: [effort] }} setConfig={setConfig} />));
    open(/Effort/);
    const low = [...document.querySelectorAll('[role=menuitemradio]')].find((b) => b.textContent?.includes('Low')) as HTMLElement;
    await act(async () => { low.click(); });
    expect(setConfig).toHaveBeenCalledWith('n/a', 'effort', 'low');
  });
  it('choosing what is in use sends nothing', async () => {
    const setConfig = vi.fn(async () => true);
    act(() => root.render(<SessionBar part="controls" agentKey="n/a" session={{ ...base, config: [effort] }} setConfig={setConfig} />));
    open(/Effort/);
    const high = [...document.querySelectorAll('[role=menuitemradio]')].find((b) => b.textContent?.includes('High')) as HTMLElement;
    await act(async () => { high.click(); });
    expect(setConfig).not.toHaveBeenCalled();
  });
  // ADR-0058's "Switch anyway" confirmation is INTENTIONALLY GONE for a mode that skips permissions: the node refuses the switch
  // (`agent.option_unconfirmed`) because the schema has no signed field to carry the user's confirmation, so a dialog could never succeed.
  // The choice is disabled with why, and nothing is sent. (e2e-session-options.mjs asserts the same in a real browser.)
  it('a choice the node marks as acting without asking is shown DISABLED with why, and is never sent', async () => {
    const setConfig = vi.fn(async () => true);
    act(() => root.render(<SessionBar part="controls" agentKey="n/a" session={{ ...base, config: [mode] }} setConfig={setConfig} />));
    open(/Mode/);
    const yolo = [...document.querySelectorAll<HTMLButtonElement>('[role=menuitemradio]')].find((b) => b.textContent?.includes('Bypass')) as HTMLButtonElement;
    expect(yolo.disabled).toBe(true);
    expect(yolo.textContent).toContain('not available yet');
    expect(yolo.textContent).toContain('does not ask');
    await act(async () => { yolo.click(); });
    expect(setConfig).not.toHaveBeenCalled();
    expect(host.textContent).toContain('not available yet from here. Start the agent that way instead.');
    expect(host.querySelector('[role=alertdialog]')).toBeNull(); // no confirmation could make it succeed
  });
  it('a boolean the node marks as skipping: turning it ON is disabled (never sent)', () => {
    const auto: ConfigOptionModel = { id: 'auto', name: 'Auto approve', description: '', category: 'custom', type: 'boolean', current: 'false', skipsPermissions: true, choices: [] };
    act(() => root.render(<SessionBar part="controls" agentKey="n/a" session={{ ...base, config: [auto] }} setConfig={async () => true} />));
    open(/More settings/);
    act(() => (document.querySelector('[role=menuitem]') as HTMLElement).click()); // the submenu of the option
    const items = [...document.querySelectorAll<HTMLButtonElement>('[role=menuitemradio]')];
    expect(items.map((b) => b.textContent)).toEqual(expect.arrayContaining([expect.stringContaining('On'), expect.stringContaining('Off')]));
    expect(items.filter((b) => b.textContent?.includes('On')).every((b) => b.disabled)).toBe(true);
  });
  it('other settings go behind one "More settings" menu', () => {
    const html = renderToStaticMarkup(<SessionBar part="controls" agentKey="n/a" session={{ ...base, config: [effort, { ...effort, id: 'x', category: 'custom', name: 'Verbosity' }] }} />);
    expect(html).toContain('More settings');
    expect(html).not.toContain('Verbosity: ');
  });
  it('says the agent did not switch when the node refused', async () => {
    act(() => root.render(<SessionBar part="controls" agentKey="n/a" session={{ ...base, config: [effort] }} setConfig={async () => false} />));
    open(/Effort/);
    await act(async () => { ([...document.querySelectorAll('[role=menuitemradio]')].find((b) => b.textContent?.includes('Low')) as HTMLElement).click(); });
    expect(host.textContent).toContain('The agent did not switch.');
  });
});

describe('the "/" menu of the composer', () => {
  const session: SessionModel = { ...base, commands: [{ name: 'review', description: 'Review the <b>changes</b>', inputHint: '' }, { name: 'compact', description: 'Free context', inputHint: 'what to keep' }] };
  const box = () => host.querySelector('textarea') as HTMLTextAreaElement;
  const type = (v: string) => act(() => { Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set?.call(box(), v); box().setSelectionRange(v.length, v.length); box().dispatchEvent(new Event('input', { bubbles: true })); });
  const key = (k: string, init: KeyboardEventInit = {}) => { const e = new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true, ...init }); act(() => { box().dispatchEvent(e); }); return e; };

  it('opens on "/", lists the agent\'s commands as text, and narrows as you type', () => {
    mount([L(1)], agent({ session }));
    type('/');
    expect([...host.querySelectorAll('[data-slash-option]')].map((o) => o.querySelector('code')?.textContent)).toEqual(['/review', '/compact']);
    expect(host.querySelector('[data-slash-option]')?.textContent).toContain('Review the <b>changes</b>'); // text, never HTML
    expect(host.querySelector('[data-slash-option] b')).toBeNull();
    type('/com');
    expect([...host.querySelectorAll('[data-slash-option]')].map((o) => o.querySelector('code')?.textContent)).toEqual(['/compact']);
    type('hello');
    expect(host.querySelector('[role=listbox]')).toBeNull();
  });
  it('is a combobox for the keyboard: arrows move, Enter inserts the command and does not send, Escape closes', () => {
    const send = vi.fn(async () => true);
    mount([L(1)], agent({ session }), {}, { send } as never);
    type('/');
    expect(box().getAttribute('role')).toBe('combobox');
    expect(box().getAttribute('aria-expanded')).toBe('true');
    key('ArrowDown');
    expect(host.querySelectorAll('[data-slash-option]')[1]?.getAttribute('aria-selected')).toBe('true');
    expect(box().getAttribute('aria-activedescendant')).toBe('slash-opt-1');
    key('ArrowUp'); key('ArrowUp'); // wraps to the last
    expect(host.querySelectorAll('[data-slash-option]')[1]?.getAttribute('aria-selected')).toBe('true');
    const enter = key('Enter');
    expect(enter.defaultPrevented).toBe(true);
    expect(box().value).toBe('/compact ');
    expect(host.querySelector('[role=listbox]')).toBeNull();
    type('/r');
    key('Escape');
    expect(host.querySelector('[role=listbox]')).toBeNull();
  });
  it('Enter that only confirms an input method candidate neither sends nor chooses a command', () => {
    mount([L(1)], agent({ session }));
    type('/');
    key('Enter', { isComposing: true });
    expect(box().value).toBe('/');
    expect(host.querySelector('[role=listbox]')).not.toBeNull();
  });
  it('a click chooses a command; only text is put in the box, nothing is sent', () => {
    const send = vi.fn(async () => true);
    mount([L(1)], agent({ session }), {}, { send } as never);
    type('/');
    act(() => (host.querySelectorAll('[data-slash-option]')[0] as HTMLElement).click());
    expect(box().value).toBe('/review ');
    expect(send).not.toHaveBeenCalled();
  });
  it('without commands the box is as it was: no menu, no combobox', () => {
    mount([L(1)], agent({ session: base }));
    type('/');
    expect(host.querySelector('[role=listbox]')).toBeNull();
    expect(box().getAttribute('role')).toBeNull();
  });
});

describe('words', () => {
  it('every new dynamic key exists in both languages, and no money is shown anywhere in the header', () => {
    for (const lang of ['en', 'tr']) {
      const src = readFileSync(`../../locales/${lang}/web.ftl`, 'utf8');
      const ids = new Set([...src.matchAll(/^([a-z0-9-]+) =/gm)].map((m) => m[1]));
      for (const k of ['tool-status-pending', 'tool-status-running', 'tool-status-done', 'tool-status-failed', 'permission-pending', 'permission-approved', 'permission-denied', 'permission-cancelled',
        'prompt-sent', 'prompt-queued', 'prompt-queue-full', 'prompt-failed', 'session-effort', 'session-mode', 'session-model', 'chat-steps-call-failed']) expect(ids.has(k), `${lang} ${k}`).toBe(true);
    }
    setLanguage('tr');
    expect(t('turn-stop')).toBe('Turu durdur');
    expect(t('turn-stop')).not.toBe(t('turn-stop-hint'));
    setLanguage('en');
    const bar = renderToStaticMarkup(<SessionBar part="status" agentKey="n/a" session={{ ...base, costMicros: 1_500_000, currency: 'USD' }} tokens={78_300} />);
    expect(bar).toContain('78.3K');
    expect(bar).not.toMatch(/\$|USD|cost/i);
  });
});
