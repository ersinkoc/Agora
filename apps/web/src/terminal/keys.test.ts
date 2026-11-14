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
