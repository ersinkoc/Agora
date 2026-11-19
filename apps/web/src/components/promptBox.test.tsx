// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage } from '../i18n';
import type { AgentTileModel } from '../model';
import type { ChatLine, ChatSource } from '../net/chat';
import { keepPosition } from '../net/follow';
import { groupFlag } from '../net/transcript';
import { sendsOnEnter } from './composer';
import { PromptBox } from './PromptBox';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); });

const agent = (over: Partial<AgentTileModel> = {}): AgentTileModel => ({ id: 'n/a', name: 'a', accent: '#fff', machine: 'm', workspace: '/w', state: 'idle', preview: '', terminal: false, ...over });
const user = (id: number): ChatLine => ({ id, atMs: 0, role: 'user', kind: 'text', text: `request ${id}`, path: '' });
const chunk = (id: number): ChatLine => ({ id, atMs: 0, role: 'agent', kind: 'text', text: 'x', path: '' });

// jsdom has no layout: every drawn child of the log is 100 px tall, and the log remembers its own scrollTop.
function layout(log: HTMLElement) {
  let top = 0;
  Object.defineProperty(log, 'scrollHeight', { get: () => log.children.length * 100 });
  Object.defineProperty(log, 'clientHeight', { get: () => 500 });
  Object.defineProperty(log, 'scrollTop', { get: () => top, set: (v: number) => { top = v; } });
}

describe('older entries and streaming', () => {
  it('keeps the reader where they were after "Show earlier", and a streamed chunk does not jump them to the bottom', () => {
    let push: (l: ChatLine[]) => void = () => {};
    const source: ChatSource = { connect(on) { push = (l) => on(l, false); on(Array.from({ length: 400 }, (_, i) => user(i + 1)), true); return { close() {} }; } };
    act(() => root.render(<PromptBox agent={agent()} source={source} />));
    const log = host.querySelector<HTMLElement>('[role=log]') as HTMLElement;
    layout(log);
    // The reader scrolls to the top of what is drawn.
    act(() => { log.scrollTop = 0; log.dispatchEvent(new Event('scroll')); });
    const button = [...host.querySelectorAll('button')].find((b) => /Show earlier/.test(b.textContent ?? '')) as HTMLElement;
    expect(button).toBeTruthy();
    const heightBefore = log.scrollHeight;
    act(() => button.click());
    const added = log.scrollHeight - heightBefore;
    expect(added).toBeGreaterThan(0);
    // The line they were reading is still in view: the position moved down by exactly what was drawn above it.
    expect(log.scrollTop).toBe(added);
    const kept = log.scrollTop;
    act(() => push([chunk(1000)]));
    act(() => push([chunk(1001)]));
    expect(log.scrollTop).toBe(kept); // not at the bottom: a streamed chunk leaves them alone
    expect(host.textContent).toContain('Jump to latest, 2 new');
  });
  it('follows the tail while the reader is at the bottom', () => {
    let push: (l: ChatLine[]) => void = () => {};
    const source: ChatSource = { connect(on) { push = (l) => on(l, false); on([user(1)], true); return { close() {} }; } };
    act(() => root.render(<PromptBox agent={agent()} source={source} />));
    const log = host.querySelector<HTMLElement>('[role=log]') as HTMLElement;
    layout(log);
    act(() => push([chunk(2)]));
    expect(log.scrollTop).toBe(log.scrollHeight);
  });
  it('computes the compensation from the heights', () => expect(keepPosition(0, 1000, 13000)).toBe(12000));
});

describe('the composer', () => {
  it('does not send on the Enter that confirms an input method candidate (Safari fires it after compositionend)', () => {
    expect(sendsOnEnter({ key: 'Enter', shiftKey: false, isComposing: false, keyCode: 13 }, 1000)).toBe(true);
    expect(sendsOnEnter({ key: 'Enter', shiftKey: true, keyCode: 13 }, 1000)).toBe(false);
    expect(sendsOnEnter({ key: 'Enter', shiftKey: false, isComposing: true, keyCode: 13 }, 1000)).toBe(false);
    expect(sendsOnEnter({ key: 'Enter', shiftKey: false, isComposing: false, keyCode: 229 }, 1000)).toBe(false);
    expect(sendsOnEnter({ key: 'Enter', shiftKey: false, isComposing: false, keyCode: 13 }, 10)).toBe(false); // just after compositionend
  });
  it('keeps text typed while a send is in flight, and is off with a reason once the agent has ended', async () => {
    let finish: (ok: boolean) => void = () => {};
    const send = vi.fn(() => new Promise<boolean>((r) => { finish = r; }));
    act(() => root.render(<PromptBox agent={agent()} send={send} />));
    const box = host.querySelector('textarea') as HTMLTextAreaElement;
    const type = (v: string) => act(() => { Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set?.call(box, v); box.dispatchEvent(new Event('input', { bubbles: true })); });
    type('first');
    await act(async () => { (host.querySelector('button[type=submit]') as HTMLElement).click(); });
    type('first and more');
    await act(async () => { finish(true); });
    expect(box.value).toBe('first and more');
    act(() => root.render(<PromptBox agent={agent({ closed: true })} send={send} />));
    expect((host.querySelector('textarea') as HTMLTextAreaElement).disabled).toBe(true);
    expect(host.textContent).toContain('This agent has ended');
  });
});

describe('a folded group never hides what needs you', () => {
  it('flags the newest group while the agent needs you or ended with an error, and opens it', () => {
    expect(groupFlag({ state: 'needs-you' }, true)).toBe('needs-you');
    expect(groupFlag({ state: 'needs-you' }, false)).toBeUndefined();
    expect(groupFlag({ state: 'working' }, true)).toBeUndefined();
    expect(groupFlag({ state: 'finished', closed: true, exitCode: 1 }, true)).toBe('failed');
    expect(groupFlag({ state: 'finished', closed: true, exitCode: 0 }, true)).toBeUndefined();
    const tool: ChatLine = { id: 1, atMs: 0, role: 'agent', kind: 'tool', text: '{"tool":"Bash","arguments":{"command":"ls"}}', path: '' };
    const source: ChatSource = { connect(on) { on([tool], true); return { close() {} }; } };
    act(() => root.render(<PromptBox agent={agent({ state: 'needs-you' })} source={source} />));
    const group = host.querySelector('details[data-steps]') as HTMLDetailsElement;
    expect(group.open).toBe(true);
    expect(group.dataset.flag).toBe('needs-you');
    expect(group.textContent).toContain('Waiting for you');
  });
});
