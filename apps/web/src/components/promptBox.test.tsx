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
// jsdom has no ResizeObserver: this one is fired by hand, as the browser would after a layout that changed an observed size.
const observers = new Set<() => void>();
(globalThis as { ResizeObserver?: unknown }).ResizeObserver = class {
  private fire: () => void;
  constructor(cb: () => void) { this.fire = () => cb(); }
  observe() { observers.add(this.fire); }
  unobserve() {}
  disconnect() { observers.delete(this.fire); }
};
const frames = () => new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r())));
const resizeObserved = () => act(() => { for (const fire of observers) fire(); });

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); });

const agent = (over: Partial<AgentTileModel> = {}): AgentTileModel => ({ id: 'n/a', name: 'a', accent: '#fff', machine: 'm', workspace: '/w', state: 'idle', preview: '', terminal: false, ...over });
const user = (id: number): ChatLine => ({ id, atMs: 0, role: 'user', kind: 'text', text: `request ${id}`, path: '' });
const chunk = (id: number): ChatLine => ({ id, atMs: 0, role: 'agent', kind: 'text', text: 'x', path: '' });

// jsdom has no layout: every drawn child of the log is 100 px tall, the log is 500 px tall unless a test resizes it, and it remembers its
// own scrollTop (clamped to its end, as a browser does).
function layout(log: HTMLElement) {
  let top = 0;
  let height = 500;
  let extra = 0; // content that grew in place (a fold opened)
  const scrollHeight = () => (log.firstElementChild?.children.length ?? log.children.length) * 100 + extra;
  Object.defineProperty(log, 'scrollHeight', { get: scrollHeight });
  Object.defineProperty(log, 'clientHeight', { get: () => height });
  Object.defineProperty(log, 'scrollTop', { get: () => top, set: (v: number) => { top = Math.max(0, Math.min(v, scrollHeight() - height)); } });
  return { resize: (h: number) => { height = h; }, grow: (px: number) => { extra += px; } };
}
const gap = (log: HTMLElement) => log.scrollHeight - log.scrollTop - log.clientHeight;
const scroll = (log: HTMLElement, top: number) => act(() => { log.scrollTop = top; log.dispatchEvent(new Event('scroll')); });

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
    const source: ChatSource = { connect(on) { push = (l) => on(l, false); on(Array.from({ length: 20 }, (_, i) => user(i + 1)), true); return { close() {} }; } };
    act(() => root.render(<PromptBox agent={agent()} source={source} />));
    const log = host.querySelector<HTMLElement>('[role=log]') as HTMLElement;
    layout(log);
    act(() => push([chunk(100)]));
    expect(log.scrollHeight).toBeGreaterThan(log.clientHeight + 64);
    expect(gap(log)).toBe(0);
  });
  it('computes the compensation from the heights', () => expect(keepPosition(0, 1000, 13000)).toBe(12000));
});

// #1060: a window or pane resize scrolls the log (the browser moves or clamps its offset), and that is not the reader scrolling away.
describe('following through a resize', () => {
  const following = () => {
    let push: (l: ChatLine[]) => void = () => {};
    const source: ChatSource = { connect(on) { push = (l) => on(l, false); on(Array.from({ length: 20 }, (_, i) => user(i + 1)), true); return { close() {} }; } };
    act(() => root.render(<PromptBox agent={agent()} source={source} />));
    const log = host.querySelector<HTMLElement>('[role=log]') as HTMLElement;
    const size = layout(log);
    act(() => push([chunk(100)]));
    expect(gap(log)).toBe(0);
    return { log, size, push: (l: ChatLine[]) => push(l) };
  };
  const pill = () => /Jump to latest/.test(host.textContent ?? '');

  it('keeps following when the log is shrunk and the browser scrolls it off its bottom', () => {
    const { log, size } = following();
    size.resize(300);
    scroll(log, log.scrollHeight - 300 - 200); // the scroll the resize caused: 200 px off the bottom
    expect(pill()).toBe(false);
    expect(gap(log)).toBe(0); // pinned back to the newest entry
  });
  it('pins to the bottom when the log or its content changes size while following, with no scroll event', () => {
    const { log, size } = following();
    const top = log.scrollTop;
    size.resize(300); // shorter: the same offset is now 200 px off the bottom, and no scroll event comes
    expect(gap(log)).toBe(200);
    resizeObserved();
    expect(log.scrollTop).toBe(top + 200);
    expect(gap(log)).toBe(0);
    expect(pill()).toBe(false);
  });
  it('keeps following when a resize clamped the offset and the scroll event comes once the size is back', async () => {
    // A full-page capture resizes the window 900 -> 1 -> 900: the observer sees the log taller, its offset is clamped, and the scroll
    // event arrives after the size is back, about 66 px off the bottom with the same sizes as before.
    const { log, size } = following();
    size.resize(566);
    resizeObserved();
    const clamped = log.scrollHeight - 566; // the offset the taller log clamped to
    size.resize(500);
    scroll(log, clamped);
    expect(pill()).toBe(false);
    expect(gap(log)).toBe(0);
    // Once the frames after the resize have passed, a scroll up is the reader's again.
    await act(frames);
    scroll(log, log.scrollTop - 300);
    expect(pill()).toBe(true);
  });
  it('still stops following when the reader scrolls up, and a resize then leaves them where they are', () => {
    const { log, size } = following();
    scroll(log, log.scrollTop - 400);
    expect(pill()).toBe(true);
    const top = log.scrollTop;
    size.resize(300);
    resizeObserved();
    scroll(log, top); // the resize's own scroll event
    expect(log.scrollTop).toBe(top);
    expect(pill()).toBe(true);
    scroll(log, log.scrollTop - 100); // and the reader keeps reading upwards
    expect(pill()).toBe(true);
  });
  it('pins when the content grows in place while following, but not when the reader opened something in the log', () => {
    const { log, size } = following();
    size.grow(150);
    resizeObserved();
    expect(gap(log)).toBe(0); // content that grew on its own: still on the newest entry
    act(() => (log.firstElementChild as HTMLElement).dispatchEvent(new MouseEvent('click', { bubbles: true })));
    size.grow(150); // the fold the reader opened
    resizeObserved();
    expect(gap(log)).toBe(150); // what they opened stays under the pointer
    expect(pill()).toBe(true); // and the log has left the tail
  });
  it('lets the reader scroll away while entries stream in', () => {
    const { log, push } = following();
    act(() => push([chunk(101)]));
    scroll(log, log.scrollTop - 300);
    expect(pill()).toBe(true);
    act(() => push([chunk(102)]));
    expect(host.textContent).toContain('Jump to latest, 1 new');
  });
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
