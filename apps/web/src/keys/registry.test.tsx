// @vitest-environment jsdom
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { describe, expect, it, vi } from 'vitest';
import { EMPTY_FILE, keymap, type KeyFile } from './file';
import { CHORD_MS, canRun, createDispatcher, handlerOf, listenForKeys, registerCommand, runCommand, useCommands, whereOf, type Where } from './registry';
import type { KeyEventLike } from './strokes';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const key = (spec: string): KeyEventLike => {
  const parts = spec.split('+');
  const k = parts.at(-1) as string;
  const code = /^[a-z]$/.test(k) ? `Key${k.toUpperCase()}` : /^[0-9]$/.test(k) ? `Digit${k}` : k;
  const named: Record<string, string> = { escape: 'Escape', tab: 'Tab', f2: 'F2', control: 'Control' };
  return { key: named[k] ?? k, code: named[k] ? named[k] : code, ctrlKey: parts.includes('ctrl'), altKey: parts.includes('alt'), shiftKey: parts.includes('shift'), metaKey: parts.includes('meta') };
};

function setup(file: KeyFile = EMPTY_FILE, opts: { mac?: boolean; unavailable?: string[] } = {}) {
  let now = 1000;
  const ran: string[] = [];
  const d = createDispatcher({ map: () => keymap(file), mac: opts.mac ?? false, now: () => now, available: (id) => !opts.unavailable?.includes(id), run: (id) => { ran.push(id); return true; } });
  const press = (spec: string, where: Where = 'other') => d.handle(key(spec), where);
  return { d, ran, press, wait: (ms: number) => { now += ms; } };
}

describe('the registry', () => {
  it('runs the handler on screen; the one registered last wins until it goes', () => {
    const a = vi.fn(); const b = vi.fn();
    const offA = registerCommand('t.one', { run: a });
    const offB = registerCommand('t.one', { run: b });
    expect(runCommand('t.one')).toBe(true);
    expect(b).toHaveBeenCalledTimes(1);
    offB();
    runCommand('t.one');
    expect(a).toHaveBeenCalledTimes(1);
    offA();
    expect(runCommand('t.one')).toBe(false);
  });
  it('does not run a command that cannot act now', () => {
    const run = vi.fn();
    const off = registerCommand('t.one', { run, available: false });
    expect(canRun('t.one')).toBe(false);
    expect(runCommand('t.one')).toBe(false);
    expect(run).not.toHaveBeenCalled();
    off();
  });
  it('a component offers its commands while it is on screen, with what it has now', () => {
    const host = document.createElement('div');
    const root = createRoot(host);
    const seen: number[] = [];
    const Part = ({ n, offer }: { n: number; offer: boolean }) => { useCommands({ 't.one': offer && { run: () => seen.push(n) } }); return null; };
    act(() => root.render(<Part n={1} offer />));
    runCommand('t.one');
    act(() => root.render(<Part n={2} offer />));
    runCommand('t.one');
    act(() => root.render(<Part n={3} offer={false} />));
    expect(runCommand('t.one')).toBe(false);
    act(() => root.render(<Part n={4} offer />));
    act(() => root.unmount());
    expect(handlerOf('t.one')).toBeUndefined();
    expect(seen).toEqual([1, 2]);
  });
});

describe('a key press', () => {
  it('runs the command bound to it', () => {
    const s = setup();
    expect(s.press('ctrl+k')).toEqual({ kind: 'ran', command: 'palette.open' });
    expect(s.press('meta+k')).toEqual({ kind: 'ran', command: 'palette.open' });
    expect(s.press('n')).toEqual({ kind: 'ran', command: 'attention.next' });
    expect(s.press('alt+3')).toEqual({ kind: 'ran', command: 'pane.tab-3' });
    expect(s.press('f2')).toEqual({ kind: 'none' });
  });
  it('matches the modifiers exactly', () => {
    const s = setup();
    expect(s.press('ctrl+alt+k').kind).toBe('none');
    expect(s.press('shift+n').kind).toBe('none');
    expect(s.press('ctrl+shift+b').kind).toBe('none');
  });
  it('a command that cannot act lets the key go on to the next command bound to it, or to the page', () => {
    const f: KeyFile = { entries: [{ command: 'layout.preset-2x2', keys: ['ctrl+b'] }] };
    expect(setup(f).press('ctrl+b')).toEqual({ kind: 'ran', command: 'layout.preset-2x2' }); // the person's own first
    expect(setup(f, { unavailable: ['layout.preset-2x2'] }).press('ctrl+b')).toEqual({ kind: 'ran', command: 'window.agents' });
    expect(setup(EMPTY_FILE, { unavailable: ['pane.tab-9'] }).press('alt+9')).toEqual({ kind: 'none' });
  });
  it('the person\'s keys replace the defaults', () => {
    const s = setup({ entries: [{ command: 'palette.open', keys: ['ctrl+p'] }, { command: 'attention.next', keys: [] }] });
    expect(s.press('ctrl+p')).toEqual({ kind: 'ran', command: 'palette.open' });
    expect(s.press('ctrl+k').kind).toBe('none');
    expect(s.press('n').kind).toBe('none');
  });
  it('never treats a fixed key as a command, nor a key another handler already took', () => {
    // A file cannot hold one (file.ts refuses it); a map built by hand still cannot make it act.
    const s = setup({ entries: [{ command: 'go.home', keys: ['escape', 'ctrl+alt+shift+s', 'ctrl+c'] }] });
    for (const k of ['escape', 'ctrl+alt+shift+s', 'ctrl+c']) expect(s.press(k).kind, k).toBe('none');
    expect(s.d.handle({ ...key('ctrl+k'), defaultPrevented: true }, 'other').kind).toBe('none');
    expect(s.ran).toEqual([]);
  });
});

describe('the terminal keeps its keys', () => {
  it('Ctrl+K, Ctrl+B, Alt+1, Alt+Shift+1 and N are the program\'s while a terminal has the keyboard', () => {
    const s = setup();
    for (const k of ['ctrl+k', 'ctrl+b', 'alt+1', 'alt+shift+1', 'n', 'ctrl+c', 'escape']) expect(s.press(k, 'terminal').kind, k).toBe('none');
    expect(s.ran).toEqual([]);
  });
  it('only a key no program can receive reaches the window: Ctrl+Shift+K opens the palette, and ⌘K on a Mac', () => {
    expect(setup().press('ctrl+shift+k', 'terminal')).toEqual({ kind: 'ran', command: 'palette.open' });
    expect(setup({ entries: [] }, { mac: true }).press('meta+k', 'terminal')).toEqual({ kind: 'ran', command: 'palette.open' });
    expect(setup().press('meta+b', 'terminal')).toEqual({ kind: 'ran', command: 'window.agents' });
  });
  it('a binding the person gives a terminal-safe key works from there; any other stays the program\'s', () => {
    const s = setup({ entries: [{ command: 'go.home', keys: ['ctrl+shift+h'] }, { command: 'go.machines', keys: ['ctrl+m'] }] });
    expect(s.press('ctrl+shift+h', 'terminal')).toEqual({ kind: 'ran', command: 'go.home' });
    expect(s.press('ctrl+m', 'terminal').kind).toBe('none');
  });
  it('no chord starts in a terminal (its second key would be the program\'s)', () => {
    const s = setup({ entries: [{ command: 'go.home', keys: ['ctrl+shift+g h'] }] });
    expect(s.press('ctrl+shift+g', 'terminal').kind).toBe('none');
    expect(s.press('h', 'terminal').kind).toBe('none');
  });
  it('in a real page: a key from inside a terminal goes to it, Ctrl+Shift+K reaches the window', () => {
    const term = document.createElement('div');
    term.className = 'xterm';
    const area = document.createElement('textarea');
    term.append(area);
    document.body.append(term);
    expect(whereOf(area)).toBe('terminal');
    const ran: string[] = [];
    const off = listenForKeys({ map: () => keymap(EMPTY_FILE), mac: false, available: () => true, run: (id) => { ran.push(id); return true; } });
    const fire = (init: KeyboardEventInit) => { const e = new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init }); area.dispatchEvent(e); return e; };
    expect(fire({ key: 'k', code: 'KeyK', ctrlKey: true }).defaultPrevented).toBe(false);
    expect(fire({ key: 'n', code: 'KeyN' }).defaultPrevented).toBe(false);
    expect(fire({ key: 'K', code: 'KeyK', ctrlKey: true, shiftKey: true }).defaultPrevented).toBe(true);
    expect(ran).toEqual(['palette.open']);
    off();
    term.remove();
  });
});

describe('a text field keeps what types text', () => {
  it('a letter is typed; Ctrl, Alt and ⌘ keys still act (as before this rung)', () => {
    const s = setup();
    expect(s.press('n', 'text').kind).toBe('none');
    expect(s.press('ctrl+k', 'text')).toEqual({ kind: 'ran', command: 'palette.open' });
    expect(s.press('alt+1', 'text')).toEqual({ kind: 'ran', command: 'pane.tab-1' });
    expect(s.press('alt+shift+2', 'text')).toEqual({ kind: 'ran', command: 'side.place-2' });
  });
  it('on a Mac Option+key types a character, so it stays the field\'s', () => {
    const s = setup(EMPTY_FILE, { mac: true });
    expect(s.press('alt+1', 'text').kind).toBe('none');
    expect(s.press('alt+1', 'other')).toEqual({ kind: 'ran', command: 'pane.tab-1' });
  });
  it('knows a field, a terminal and anything else', () => {
    const el = (html: string) => { const d = document.createElement('div'); d.innerHTML = html; return d.firstElementChild as Element; };
    expect(whereOf(el('<input>'))).toBe('text');
    expect(whereOf(el('<textarea></textarea>'))).toBe('text');
    expect(whereOf(el('<select></select>'))).toBe('text');
    expect(whereOf(el('<div role="combobox"></div>'))).toBe('text');
    expect(whereOf(el('<button>x</button>'))).toBe('other');
    expect(whereOf(document)).toBe('other');
    expect(whereOf(null)).toBe('other');
  });
});

describe('chords', () => {
  const chord: KeyFile = { entries: [{ command: 'go.keyboard', keys: ['ctrl+k ctrl+s'] }, { command: 'go.home', keys: ['ctrl+g h'] }] };
  it('the first key waits for the second, which runs the command', () => {
    const s = setup(chord);
    expect(s.press('ctrl+k').kind).toBe('pending'); // the chord waits rather than opening the palette (a conflict Settings shows)
    expect(s.press('control').kind).toBe('none'); // a modifier on its own keeps it waiting
    expect(s.press('ctrl+s')).toEqual({ kind: 'ran', command: 'go.keyboard' });
    expect(s.press('ctrl+g').kind).toBe('pending');
    expect(s.press('h', 'text')).toEqual({ kind: 'ran', command: 'go.home' });
  });
  it('a key that does not complete it drops the chord and is taken as a key of its own', () => {
    const s = setup(chord);
    s.press('ctrl+g');
    expect(s.press('n')).toEqual({ kind: 'ran', command: 'attention.next' });
    expect(s.d.pending()).toBeUndefined();
    s.press('ctrl+g');
    expect(s.press('escape').kind).toBe('none');
    expect(s.press('h').kind).toBe('none');
  });
  it('stops waiting after CHORD_MS', () => {
    const s = setup(chord);
    s.press('ctrl+g');
    s.wait(CHORD_MS + 1);
    expect(s.press('h').kind).toBe('none');
    expect(s.ran).toEqual([]);
  });
});

describe('the window listens after the page', () => {
  it('a dialog that keeps a letter to itself keeps it from the window, as before this rung', () => {
    const ran: string[] = [];
    const off = listenForKeys({ map: () => keymap(EMPTY_FILE), mac: false, available: () => true, run: (id) => { ran.push(id); return true; } });
    const dialog = document.createElement('div');
    dialog.addEventListener('keydown', (e) => e.stopPropagation());
    const button = document.createElement('button');
    dialog.append(button);
    document.body.append(dialog);
    button.dispatchEvent(new KeyboardEvent('keydown', { key: 'n', code: 'KeyN', bubbles: true }));
    expect(ran).toEqual([]);
    document.body.dispatchEvent(new KeyboardEvent('keydown', { key: 'n', code: 'KeyN', bubbles: true }));
    expect(ran).toEqual(['attention.next']);
    off();
    dialog.remove();
  });
});
