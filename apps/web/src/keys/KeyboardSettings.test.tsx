// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage, t } from '../i18n';
import { COMMANDS } from './commands';
import { CHORD_MS } from './registry';
import { KeyboardSettings } from './KeyboardSettings';
import { KEYS_KEY, createKeyStore, loadKeys, type KeyStore } from './store';
import { writeKeys } from './file';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const memory = () => { const m = new Map<string, string>(); return { m, kv: { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => { m.set(k, v); }, removeItem: (k: string) => { m.delete(k); } } }; };

let host: HTMLElement;
let root: Root;
let store: KeyStore;
let mem: ReturnType<typeof memory>;
beforeEach(() => {
  setLanguage('en');
  mem = memory();
  store = createKeyStore({ kv: mem.kv });
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => { act(() => root.unmount()); host.remove(); vi.useRealTimers(); });

const draw = () => act(() => root.render(<KeyboardSettings store={store} mac={false} />));
const row = (id: string) => host.querySelector<HTMLElement>(`[data-command="${id}"]`) as HTMLElement;
const button = (within: Element, name: string | RegExp) => [...within.querySelectorAll('button')].find((b) => (typeof name === 'string' ? b.textContent === name || b.getAttribute('aria-label') === name : name.test(b.getAttribute('aria-label') ?? b.textContent ?? ''))) as HTMLButtonElement;
const caps = (id: string) => [...row(id).querySelectorAll('.keys-caps kbd')].map((k) => k.textContent);
const press = (el: Element, init: KeyboardEventInit) => act(() => { el.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init })); });
const record = (id: string) => { act(() => button(row(id), t('keys-change')).click()); return button(row(id), t('keys-recording')); };

describe('Settings > Keyboard', () => {
  it('lists every command with its keys, where it acts, and its id for the file', () => {
    draw();
    expect(host.querySelectorAll('.keys-row')).toHaveLength(COMMANDS.length);
    expect(caps('palette.open')).toEqual(['Ctrl+K', 'Meta+K', 'Ctrl+Shift+K']);
    expect(caps('attention.next')).toEqual(['N']);
    expect(row('go.home').textContent).toContain(t('keys-no-keys'));
    expect(row('pane.tab-1').textContent).toContain('pane.tab-1');
    expect(row('pane.tab-1').textContent).toContain(t('keys-context-tabs'));
  });

  it('a new key is recorded by pressing it, kept in this browser, and the row says it changed', () => {
    vi.useFakeTimers();
    draw();
    const b = record('go.home');
    press(b, { key: 'h', code: 'KeyH', ctrlKey: true, shiftKey: true });
    expect(caps('go.home')).toEqual([]); // waiting for a second key, or the end of the wait
    act(() => { vi.advanceTimersByTime(CHORD_MS); });
    expect(caps('go.home')).toEqual(['Ctrl+Shift+H']);
    expect(row('go.home').textContent).toContain(t('keys-changed'));
    expect(loadKeys(mem.kv).file.entries).toEqual([{ command: 'go.home', keys: ['ctrl+shift+h'] }]);
  });

  it('two keys pressed in a row are a chord', () => {
    draw();
    const b = record('go.keyboard');
    press(b, { key: 'g', code: 'KeyG', ctrlKey: true });
    expect(row('go.keyboard').textContent).toContain(t('keys-press-second', { key: 'Ctrl+G' }));
    press(button(row('go.keyboard'), t('keys-recording')), { key: 's', code: 'KeyS' });
    expect(caps('go.keyboard')).toEqual(['Ctrl+G S']);
  });

  it('a key pressed while recording goes to the recorder, not the window', () => {
    draw();
    const seen = vi.fn();
    window.addEventListener('keydown', seen);
    const b = record('go.home');
    press(b, { key: 'b', code: 'KeyB', ctrlKey: true });
    expect(seen).not.toHaveBeenCalled();
    window.removeEventListener('keydown', seen);
  });

  it('Esc stops recording and changes nothing', () => {
    draw();
    const b = record('attention.next');
    press(b, { key: 'Escape', code: 'Escape' });
    expect(caps('attention.next')).toEqual(['N']);
    expect(mem.m.has(KEYS_KEY)).toBe(false);
  });

  it('refuses a fixed key with the reason, and keeps the old keys', () => {
    draw();
    const b = record('attention.next');
    press(b, { key: 'c', code: 'KeyC', ctrlKey: true });
    press(button(row('attention.next'), t('keys-recording')), { key: 'v', code: 'KeyV', ctrlKey: true });
    expect(row('attention.next').querySelector('[role="alert"]')?.textContent).toBe(t('keys-refused-fixed', { key: 'Ctrl+C Ctrl+V', reason: t('keys-fixed-editing') }));
    expect(caps('attention.next')).toEqual(['N']);
  });

  it('the safe-mode key is fixed: no Change, no Clear, and it is listed with the keys that stay as they are', () => {
    draw();
    const r = row('window.safe-mode');
    expect(r.textContent).toContain(t('keys-fixed'));
    expect(r.querySelectorAll('button')).toHaveLength(0);
    expect(host.textContent).toContain(t('keys-fixed-title'));
    expect(host.querySelector('.keys-fixed')?.textContent).toContain('Ctrl+Alt+Shift+S');
  });

  it('shows the keys two commands share, on both rows', () => {
    store.set({ entries: [{ command: 'go.home', keys: ['ctrl+b'] }] });
    draw();
    expect(row('go.home').querySelector('.keys-clash')?.textContent).toContain(t('keys-conflict-same', { key: 'Ctrl+B', name: t('keys-cmd-agents') }));
    expect(row('window.agents').getAttribute('data-conflict')).toBe('true');
    act(() => store.set({ entries: [{ command: 'go.home', keys: ['ctrl+k ctrl+h'] }] }));
    expect(row('palette.open').querySelector('.keys-clash')?.textContent).toContain(t('keys-conflict-prefix', { key: 'Ctrl+K', name: t('palette-go-home') }));
  });

  it('clears a command\'s keys, gives one its defaults back, and resets all after asking', () => {
    draw();
    act(() => button(row('attention.next'), t('keys-clear-named', { name: t('palette-next-attention') })).click());
    expect(row('attention.next').textContent).toContain(t('keys-no-keys'));
    act(() => button(row('attention.next'), t('keys-reset-named', { name: t('palette-next-attention') })).click());
    expect(caps('attention.next')).toEqual(['N']);
    act(() => store.set({ entries: [{ command: 'go.home', keys: ['f2'] }] }));
    act(() => button(host, t('keys-reset-all')).click());
    act(() => button(document.body, t('keys-reset-keep')).click());
    expect(caps('go.home')).toEqual(['F2']);
    act(() => button(host, t('keys-reset-all')).click());
    act(() => button(document.querySelector('.kit-dialog') as Element, t('keys-reset-confirm')).click());
    expect(row('go.home').textContent).toContain(t('keys-no-keys'));
    expect(mem.m.has(KEYS_KEY)).toBe(false);
  });

  it('imports a file through the same checks, listing every problem, and changes nothing when refused', () => {
    draw();
    act(() => button(host, t('keys-import')).click());
    const area = document.querySelector<HTMLTextAreaElement>('.keys-import textarea') as HTMLTextAreaElement;
    const type = (text: string) => act(() => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set?.call(area, text);
      area.dispatchEvent(new Event('input', { bubbles: true }));
    });
    type(JSON.stringify({ version: 1, bindings: [{ command: 'go.home', keys: ['ctrl+'] }, { command: 'request.allow', keys: ['y'] }] }));
    act(() => button(document.querySelector('.keys-import') as Element, t('layout-import-do')).click());
    const problems = document.getElementById('keys-import-problems')?.textContent ?? '';
    expect(problems).toContain(t('keys-error-key', { command: 'go.home', key: 'ctrl+' }));
    expect(problems).toContain(t('keys-error-request', { command: 'request.allow' }));
    expect(store.state().file.entries).toEqual([]);
    type(writeKeys({ entries: [{ command: 'go.home', keys: ['ctrl+shift+h'] }, { command: 'later.cmd', keys: ['f4'] }] }));
    act(() => button(document.querySelector('.keys-import') as Element, t('layout-import-do')).click());
    expect(document.querySelector('.keys-import')).toBeNull();
    expect(caps('go.home')).toEqual(['Ctrl+Shift+H']);
    expect(host.textContent).toContain(t('keys-unknown', { count: 1 }));
    expect(host.querySelector('.keys-unknown')?.textContent).toBe('later.cmd');
  });

  it('finds a command by its name, its id or its key', () => {
    draw();
    const input = host.querySelector<HTMLInputElement>('.keys-search input') as HTMLInputElement;
    const find = (q: string) => { act(() => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set?.call(input, q); input.dispatchEvent(new Event('input', { bubbles: true })); }); return [...host.querySelectorAll('.keys-row')].map((r) => r.getAttribute('data-command')); };
    expect(find('agent list')).toEqual(['window.agents']);
    expect(find('layout.reset')).toEqual(['layout.reset']);
    expect(find('alt+shift+3')).toEqual(['side.place-3']);
    expect(find('zzz')).toEqual([]);
    expect(host.textContent).toContain(t('keys-none-match'));
  });
});

describe('the keys kept in this browser', () => {
  it('a stored file that cannot be read leaves the defaults in force and says so', () => {
    const { kv } = memory();
    kv.setItem(KEYS_KEY, '{"version":1,"bindings":[{"command":"go.home","keys":["escape"]}]}');
    const s = loadKeys(kv);
    expect(s.unreadable).toBe(true);
    expect(s.file.entries).toEqual([]);
  });
  it('in safe mode the defaults only: the stored file is neither read nor written', () => {
    const { m, kv } = memory();
    kv.setItem(KEYS_KEY, writeKeys({ entries: [{ command: 'palette.open', keys: [] }] }));
    const safe = createKeyStore({ kv, safe: true });
    expect(safe.state().file.entries).toEqual([]);
    safe.set({ entries: [{ command: 'go.home', keys: ['f2'] }] });
    expect(safe.state().file.entries).toHaveLength(1); // for this visit
    expect(m.get(KEYS_KEY)).toContain('palette.open');
  });
  it('another tab\'s change applies here', () => {
    const { kv } = memory();
    const events = new EventTarget();
    const s = createKeyStore({ kv, events: events as unknown as Window });
    kv.setItem(KEYS_KEY, writeKeys({ entries: [{ command: 'go.home', keys: ['f2'] }] }));
    const changed = vi.fn();
    s.subscribe(changed);
    events.dispatchEvent(Object.assign(new Event('storage'), { key: KEYS_KEY }));
    expect(changed).toHaveBeenCalled();
    expect(s.state().file.entries).toEqual([{ command: 'go.home', keys: ['f2'] }]);
  });
  it('storage that throws keeps a change for the visit', () => {
    const s = createKeyStore({ kv: { getItem: () => { throw new Error('blocked'); }, setItem: () => { throw new Error('full'); }, removeItem: () => { throw new Error('blocked'); } } });
    expect(() => s.set({ entries: [{ command: 'go.home', keys: ['f2'] }] })).not.toThrow();
    expect(s.state().file.entries).toHaveLength(1);
  });
});
