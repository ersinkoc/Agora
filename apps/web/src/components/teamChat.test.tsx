// @vitest-environment jsdom
import { readFileSync } from 'node:fs';
import { act, useState } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { setLanguage } from '../i18n';
import type { AgentTileModel } from '../model';
import type { MessageModel } from '../net/view';
import { Inspector } from './Inspector';
import type { RailTab } from './rail';
import { TeamChat } from './TeamChat';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); document.body.innerHTML = ''; });

const NOW = 1_800_000_000_000;
const long = 'Fine. While you are there: the expiry check compares against SystemTime::now() inside is_expired, which makes it impossible to test deterministically, so I would pass the clock in. Also the token lifetime is hardcoded as 3600 seconds in issue(), but the spec says fifteen minutes.';
const msg = (over: Partial<MessageModel> = {}): MessageModel => ({ from: 'codex · teal', fromId: 'n1/a2', to: 'claude · plum', body: 'Short.', atMs: NOW - 120_000, crossMachine: false, ...over });
const teal: AgentTileModel = { id: 'n1/a2', name: 'codex · teal', kind: 'codex', accent: 'var(--agent-teal)', machine: 'laptop', workspace: '/w', state: 'working', preview: '' };
const key = (k: string) => act(() => { (document.activeElement ?? document.body).dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true })); });
const dialog = () => document.querySelector<HTMLElement>('[role="dialog"]');
// Radix gives focus back on the next task after the view closes.
const tick = () => act(async () => { await new Promise((r) => setTimeout(r, 0)); });
const rows = () => [...host.querySelectorAll<HTMLButtonElement>('button.team-msg')];

// jsdom has no layout: a body "is cut" when its text is longer than two lines would hold (the real two-line cut is checked in Chromium by
// the panel screenshot script).
function fakeLayout() {
  const proto = HTMLElement.prototype;
  const saved = [Object.getOwnPropertyDescriptor(proto, 'scrollHeight'), Object.getOwnPropertyDescriptor(proto, 'clientHeight')] as const;
  Object.defineProperty(proto, 'scrollHeight', { configurable: true, get(this: HTMLElement) { return (this.textContent ?? '').length > 120 ? 120 : 40; } });
  Object.defineProperty(proto, 'clientHeight', { configurable: true, get() { return 40; } });
  const RO = globalThis.ResizeObserver;
  globalThis.ResizeObserver = class { observe() {} unobserve() {} disconnect() {} } as unknown as typeof ResizeObserver;
  return () => {
    for (const [name, d] of [['scrollHeight', saved[0]], ['clientHeight', saved[1]]] as const) { if (d) Object.defineProperty(proto, name, d); else delete (proto as unknown as Record<string, unknown>)[name]; }
    globalThis.ResizeObserver = RO;
  };
}

describe('team chat rows', () => {
  it('show the avatar, who wrote to whom, a short relative time, and the text cut to two lines', () => {
    act(() => root.render(<TeamChat messages={[msg({ body: long })]} fleet={[teal]} now={NOW} />));
    const row = rows()[0] as HTMLButtonElement;
    expect(row.querySelector('.kit-avatar')?.getAttribute('style')).toContain('var(--agent-teal)');
    expect(row.querySelector('.kit-avatar__icon')).not.toBeNull();
    expect(row.querySelector('.team-msg__from')?.textContent).toBe('codex · teal');
    expect(row.querySelector('.team-msg__to')?.textContent).toContain('to');
    expect(row.querySelector('.team-msg__to')?.textContent).toContain('claude · plum');
    const time = row.querySelector('time');
    expect(time?.textContent).toMatch(/2 min/);
    expect(time?.getAttribute('dateTime')).toBe(new Date(NOW - 120_000).toISOString());
    const body = row.querySelector('.team-msg__body') as HTMLElement;
    expect(body.className).toContain('team-msg__body'); // app.css: line-clamp-2
    expect(body.textContent).toBe(long); // the whole text is there for a screen reader; only its drawing is cut
  });

  it('say "Show all" only when the two lines cut the text, and a row opens the whole text in a reading view', () => {
    const restore = fakeLayout();
    try {
      act(() => root.render(<TeamChat messages={[msg({ body: long }), msg({ body: 'Short.', atMs: NOW - 60_000 })]} fleet={[teal]} now={NOW} />));
      const [cut, whole] = rows() as [HTMLButtonElement, HTMLButtonElement];
      expect(cut.querySelector('.team-msg__more')?.textContent).toBe('Show all');
      expect(whole.querySelector('.team-msg__more')).toBeNull();
      act(() => cut.click());
      expect(dialog()?.querySelector('[data-team-reader-body]')?.textContent).toBe(long);
      expect(dialog()?.textContent).toContain('codex · teal');
    } finally { restore(); }
  });

  it('a sender this window does not list gets a neutral avatar with the first letter of its name', () => {
    act(() => root.render(<TeamChat messages={[msg({ from: 'desktop/gemini · rose', fromId: 'desktop/x' })]} fleet={[teal]} now={NOW} />));
    const avatar = rows()[0]?.querySelector('.kit-avatar');
    expect(avatar?.getAttribute('style')).toContain('var(--agent-slate)');
    expect(avatar?.textContent).toBe('G');
  });

  it('a message between machines names the other machine in a tag; one another machine reported says who', () => {
    act(() => root.render(<TeamChat messages={[msg({ crossMachine: true, toMachine: 'win', via: 'laptop' })]} now={NOW} />));
    const tags = [...(rows()[0]?.querySelectorAll('.kit-tag') ?? [])].map((x) => x.textContent);
    expect(tags).toEqual(['win', 'reported by laptop']);
    expect(rows()[0]?.textContent).toContain('between machines'); // said to a screen reader, and in the tag's tooltip
  });
});

describe('the two-line cut is in the style sheet', () => {
  // jsdom draws no layout, so this pins the rule itself; the drawn height (two lines) is checked in Chromium by the panel screenshot script.
  const css = readFileSync('src/design/app.css', 'utf8'); // vitest runs in apps/web
  const rule = (sel: string) => new RegExp(`\\${sel} \\{([^}]*)\\}`).exec(css)?.[1] ?? '';
  it('cuts a row body to two lines, and the reading view keeps line breaks and scrolls', () => {
    expect(rule('.team-msg__body')).toMatch(/\bline-clamp-2\b/);
    expect(rule('.team-reader__body')).toMatch(/\bwhitespace-pre-wrap\b/);
    expect(rule('.team-reader__body')).toMatch(/\boverflow-auto\b/);
  });
});

describe('agent text is untrusted', () => {
  const html = '<img src=x onerror="window.__pwned=1"><b>bold</b><script>window.__pwned=1</script>';
  it('a message, a sender and a recipient containing HTML show as text, in the row and in the reading view', () => {
    act(() => root.render(<TeamChat messages={[msg({ body: html, from: '<i>evil</i>', to: '<a href="https://x">y</a>' })]} now={NOW} />));
    const row = rows()[0] as HTMLButtonElement;
    expect(row.querySelector('img, b, i, a, script')).toBeNull();
    expect(row.querySelector('.team-msg__body')?.textContent).toBe(html);
    expect(row.textContent).toContain('<i>evil</i>');
    act(() => row.click());
    const view = dialog() as HTMLElement;
    expect(view.querySelector('img, b, i, a, script')).toBeNull();
    expect(view.querySelector('[data-team-reader-body]')?.textContent).toBe(html);
    expect((window as { __pwned?: number }).__pwned).toBeUndefined();
  });
  it('control and bidirectional characters are named on screen; line breaks stay in the reading view', () => {
    act(() => root.render(<TeamChat messages={[msg({ body: 'line one\nline‮two\u0007', from: 'codex⁦ · teal' })]} now={NOW} />));
    const row = rows()[0] as HTMLButtonElement;
    expect(row.querySelector('.team-msg__from')?.textContent).toBe('codex\\u{2066} · teal');
    act(() => row.click());
    expect(dialog()?.querySelector('[data-team-reader-body]')?.textContent).toBe('line one\nline\\u{202e}two\\u{7}');
  });
  it('makes the full hidden-character set visible in names and message text', () => {
    act(() => root.render(<TeamChat messages={[msg({ from: 'co\u3164dex', to: 'cla\u034fude', body: 'İş\u2800 sürüyor' })]} now={NOW} />));
    const row = rows()[0] as HTMLButtonElement;
    for (const shown of ['co\\u{3164}dex', 'cla\\u{34f}ude', 'İş\\u{2800} sürüyor']) expect(row.textContent).toContain(shown);
    expect([...row.textContent ?? ''].filter((c) => ['\u3164', '\u034f', '\u2800'].includes(c))).toEqual([]);
  });
  it('verifies Team Chat text across isolate, private-use and control characters', () => {
    act(() => root.render(<TeamChat messages={[msg({ from: '模型\u2066', to: 'İş\ue000', body: 'ça\u0001lış' })]} now={NOW} />));
    const row = rows()[0] as HTMLButtonElement;
    act(() => row.click());
    for (const shown of ['模型\\u{2066}', 'İş\\u{e000}', 'ça\\u{1}lış']) expect(dialog()?.textContent).toContain(shown);
  });
});

describe('the reading view from the keyboard', () => {
  function Harness({ messages }: { messages: MessageModel[] }) {
    const [open, setOpen] = useState(true);
    const [tab, setTab] = useState<RailTab>('messages');
    return <Inspector fleet={[teal]} now={NOW} attention={[]} approvals={[]} changed={[]} boards={[]} messages={messages} projects={[]} onOpen={() => {}} onResolve={() => {}}
      open={open} onOpenChange={setOpen} tab={tab} onTab={setTab} />;
  }
  it('a row is a button in the tab order; Esc closes the view only, the panel stays open and focus is back on the row', async () => {
    act(() => root.render(<Harness messages={[msg({ body: long }), msg({ body: 'Second.' })]} />));
    expect(host.querySelector('#inspector-panel h2')?.textContent).toBe('Team chat');
    const row = rows()[0] as HTMLButtonElement;
    expect(row.tagName).toBe('BUTTON');
    expect(row.tabIndex).toBe(0);
    expect(row.getAttribute('aria-haspopup')).toBe('dialog');
    act(() => row.focus());
    act(() => row.click()); // Enter or Space on a focused button
    expect(dialog()).not.toBeNull();
    expect(dialog()?.contains(document.activeElement)).toBe(true); // focus moved into the view
    key('Escape');
    expect(dialog()).toBeNull();
    expect(host.querySelector('#inspector-panel')).not.toBeNull(); // the panel under it did not close
    await tick();
    expect(document.activeElement).toBe(rows()[0]);
    key('Escape'); // a second Escape is the panel's again
    expect(host.querySelector('#inspector-panel')).toBeNull();
  });
  it('the view has a labelled close button that also returns to the row', async () => {
    act(() => root.render(<Harness messages={[msg({ body: long })]} />));
    const row = rows()[0] as HTMLButtonElement;
    act(() => row.focus());
    act(() => row.click());
    expect(dialog()?.getAttribute('aria-labelledby')).toBeTruthy();
    const close = dialog()?.querySelector<HTMLButtonElement>('button[aria-label="Close"]') as HTMLButtonElement;
    act(() => close.click());
    expect(dialog()).toBeNull();
    await tick();
    expect(document.activeElement).toBe(rows()[0]);
  });
});
