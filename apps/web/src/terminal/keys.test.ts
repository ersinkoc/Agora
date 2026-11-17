import { describe, expect, it } from 'vitest';
import { BAR_KEYS, ctrlChar, keyBytes, withCtrl } from './keys';

describe('key bar bytes (ADR-0067 rung 2)', () => {
  it('sends Esc and Tab as themselves', () => {
    expect(keyBytes('esc')).toBe('\x1b');
    expect(keyBytes('tab')).toBe('\t');
    // Ctrl changes neither.
    expect(keyBytes('esc', { ctrl: true })).toBe('\x1b');
    expect(keyBytes('tab', { ctrl: true })).toBe('\t');
  });

  it('sends arrows in the cursor mode the program asked for', () => {
    expect(['up', 'down', 'right', 'left'].map((k) => keyBytes(k as 'up'))).toEqual(['\x1b[A', '\x1b[B', '\x1b[C', '\x1b[D']);
    expect(['up', 'down', 'right', 'left'].map((k) => keyBytes(k as 'up', { appCursor: true }))).toEqual(['\x1bOA', '\x1bOB', '\x1bOC', '\x1bOD']);
  });

  it('sends Ctrl+arrow with the modifier, whatever the cursor mode', () => {
    expect(keyBytes('left', { ctrl: true })).toBe('\x1b[1;5D');
    expect(keyBytes('right', { ctrl: true, appCursor: true })).toBe('\x1b[1;5C');
  });

  it('has the keys the rung names, Esc first', () => {
    expect(BAR_KEYS).toEqual(['esc', 'tab', 'left', 'up', 'down', 'right']);
  });

  it('turns a typed character into its control code', () => {
    expect(ctrlChar('c')).toBe('\x03');
    expect(ctrlChar('C')).toBe('\x03');
    expect(ctrlChar('a')).toBe('\x01');
    expect(ctrlChar('z')).toBe('\x1a');
    expect(ctrlChar('d')).toBe('\x04');
    expect(ctrlChar('@')).toBe('\x00');
    expect(ctrlChar(' ')).toBe('\x00');
    expect(ctrlChar('[')).toBe('\x1b');
    expect(ctrlChar('\\')).toBe('\x1c');
    expect(ctrlChar(']')).toBe('\x1d');
    expect(ctrlChar('^')).toBe('\x1e');
    expect(ctrlChar('_')).toBe('\x1f');
    expect(ctrlChar('?')).toBe('\x7f');
    expect(['2', '3', '4', '5', '6', '7', '8'].map(ctrlChar)).toEqual(['\x00', '\x1b', '\x1c', '\x1d', '\x1e', '\x1f', '\x7f']);
  });

  it('leaves characters with no control form, and longer input, as typed', () => {
    expect(ctrlChar('1')).toBe('1');
    expect(ctrlChar('ş')).toBe('ş');
    expect(withCtrl('ç')).toBe('ç');
    expect(withCtrl('c')).toBe('\x03');
    // A word the keyboard completed, or a paste: never garbled.
    expect(withCtrl('cd')).toBe('cd');
    expect(withCtrl('')).toBe('');
    // One emoji is one character but two UTF-16 units; it has no control form.
    expect(withCtrl('😀')).toBe('😀');
  });
});

import { enhancedKey, CsiUFlags, shortcut } from './keys';
import { binaryBytes } from './frames';
import { DEFAULT_PREFS, parsePrefs } from './prefs';

const ev = (key: string, o: Partial<Record<'ctrlKey' | 'shiftKey' | 'altKey' | 'metaKey', boolean>> = {}) => ({ key, ctrlKey: false, shiftKey: false, altKey: false, metaKey: false, ...o });

describe('terminal window shortcuts', () => {
  it('copies with Ctrl+Shift+C or Ctrl+Insert, never with Ctrl+C (that interrupts the program)', () => {
    expect(shortcut(ev('C', { ctrlKey: true, shiftKey: true }), false)).toBe('copy');
    expect(shortcut(ev('Insert', { ctrlKey: true }), false)).toBe('copy');
    expect(shortcut(ev('c', { ctrlKey: true }), false)).toBeUndefined();
    expect(shortcut(ev('c', { ctrlKey: true, altKey: true, shiftKey: true }), false)).toBeUndefined();
  });
  it('uses Cmd+C and Cmd+F on a Mac', () => {
    expect(shortcut(ev('c', { metaKey: true }), true)).toBe('copy');
    expect(shortcut(ev('f', { metaKey: true }), true)).toBe('find');
    expect(shortcut(ev('c', { ctrlKey: true, shiftKey: true }), true)).toBeUndefined();
  });
  it('finds with Ctrl+Shift+F, leaving Ctrl+F to the program', () => {
    expect(shortcut(ev('F', { ctrlKey: true, shiftKey: true }), false)).toBe('find');
    expect(shortcut(ev('f', { ctrlKey: true }), false)).toBeUndefined();
  });
});

describe('keys a plain terminal cannot tell apart', () => {
  const off = { csiU: 0, modifyOtherKeys: 0 };
  it('sends nothing new to a program that did not ask', () => {
    expect(enhancedKey(ev('Enter', { shiftKey: true }), off)).toBeUndefined();
    expect(enhancedKey(ev('Enter', { ctrlKey: true }), off)).toBeUndefined();
  });
  it('sends the CSI u form once the program set the disambiguate flag', () => {
    const k = { csiU: 1, modifyOtherKeys: 0 };
    expect(enhancedKey(ev('Enter', { shiftKey: true }), k)).toBe('\x1b[13;2u');
    expect(enhancedKey(ev('Enter', { ctrlKey: true }), k)).toBe('\x1b[13;5u');
    expect(enhancedKey(ev('Enter', { ctrlKey: true, shiftKey: true, altKey: true }), k)).toBe('\x1b[13;8u');
    expect(enhancedKey(ev('Backspace', { ctrlKey: true }), k)).toBe('\x1b[127;5u');
    expect(enhancedKey(ev('Tab', { ctrlKey: true }), k)).toBe('\x1b[9;5u');
  });
  it('leaves a plain key, Shift+Tab and Cmd combinations alone', () => {
    const k = { csiU: 1, modifyOtherKeys: 2 };
    expect(enhancedKey(ev('Enter'), k)).toBeUndefined();
    expect(enhancedKey(ev('Tab', { shiftKey: true }), k)).toBeUndefined();
    expect(enhancedKey(ev('Enter', { metaKey: true }), k)).toBeUndefined();
    expect(enhancedKey(ev('a', { ctrlKey: true }), k)).toBeUndefined();
  });
  it('sends the modifyOtherKeys form when only that was asked for', () => {
    expect(enhancedKey(ev('Enter', { shiftKey: true }), { csiU: 0, modifyOtherKeys: 2 })).toBe('\x1b[27;2;13~');
  });
  it('keeps the CSI u flag stack: push, pop, set and the reply', () => {
    const f = new CsiUFlags();
    expect(f.reply()).toBe('\x1b[?0u');
    f.push(1); expect(f.flags).toBe(1); expect(f.reply()).toBe('\x1b[?1u');
    f.push(5); expect(f.flags).toBe(5);
    f.pop(); expect(f.flags).toBe(1);
    f.set(2, 2); expect(f.flags).toBe(3);
    f.set(1, 3); expect(f.flags).toBe(2);
    f.pop(); expect(f.flags).toBe(0);
    f.pop(5); expect(f.flags).toBe(0);
    f.push(1); f.reset(); expect(f.flags).toBe(0);
  });
});

describe('older mouse reports are bytes', () => {
  it('sends each character as one byte, not as UTF-8', () => {
    expect(Array.from(binaryBytes('\x1b[M \xc8#'))).toEqual([0x1b, 0x5b, 0x4d, 0x20, 0xc8, 0x23]);
  });
});

describe('terminal settings', () => {
  it('start like a real terminal and keep only values this version knows', () => {
    expect(DEFAULT_PREFS).toMatchObject({ lineHeight: 1, readable: false, copyOnSelect: false, gpu: false });
    expect(parsePrefs(undefined)).toEqual(DEFAULT_PREFS);
    expect(parsePrefs('not json')).toEqual(DEFAULT_PREFS);
    expect(parsePrefs(JSON.stringify({ lineHeight: 1.1, scrollback: 10000, readable: true, copyOnSelect: true, gpu: true }))).toEqual({ lineHeight: 1.1, scrollback: 10000, readable: true, copyOnSelect: true, gpu: true });
    expect(parsePrefs(JSON.stringify({ lineHeight: 3, scrollback: 7, readable: 'yes', copyOnSelect: 1, gpu: 'yes' }))).toEqual(DEFAULT_PREFS);
  });
});
