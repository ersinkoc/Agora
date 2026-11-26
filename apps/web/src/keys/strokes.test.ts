import { describe, expect, it } from 'vitest';
import { ariaKeys, bindingLabel, bindingText, parseBinding, parseStroke, primary, reachesPastTerminal, strokeOf, typesText, type Binding, type KeyEventLike } from './strokes';

const ev = (o: Partial<KeyEventLike>): KeyEventLike => ({ key: '', code: '', ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, ...o });
const b = (text: string) => parseBinding(text) as Binding;

describe('key strings', () => {
  it('reads one stroke or a chord of two, in one spelling whatever the order or case of the modifiers', () => {
    expect(bindingText(b('ctrl+k'))).toBe('ctrl+k');
    expect(bindingText(b('Shift+Ctrl+K'))).toBe('ctrl+shift+k');
    expect(bindingText(b('meta+alt+shift+s'))).toBe('alt+shift+meta+s');
    expect(bindingText(b('ctrl+k ctrl+s'))).toBe('ctrl+k ctrl+s');
    expect(bindingText(b('n'))).toBe('n');
    expect(bindingText(b('alt+shift+1'))).toBe('alt+shift+1');
    expect(bindingText(b('ctrl+/'))).toBe('ctrl+/');
    expect(bindingText(b('f5'))).toBe('f5');
  });
  it('knows a few other names for the same modifier or key', () => {
    expect(bindingText(b('cmd+option+esc'))).toBe('alt+meta+escape');
    expect(bindingText(b('control+arrowup'))).toBe('ctrl+up');
  });
  it('refuses anything else, rather than guessing', () => {
    for (const bad of ['', ' ', 'ctrl+', '+k', 'ctrl++', 'ctrl', 'ctrl+ctrl+k', 'hyper+k', 'ctrl+kk', 'ctrl+k  ctrl+s', ' ctrl+k', 'ctrl+k ', 'a b c', 'ctrl+numpad1', 'é', 'ctrl+k\tctrl+s', 'x'.repeat(65)]) {
      expect(parseBinding(bad), JSON.stringify(bad)).toBeUndefined();
    }
    for (const bad of [null, 3, {}, ['ctrl+k'], undefined]) expect(parseBinding(bad)).toBeUndefined();
    expect(parseStroke('shift+meta+ctrl+alt+z')).toEqual({ ctrl: true, alt: true, shift: true, meta: true, key: 'z' });
  });
});

describe('a key pressed', () => {
  it('names letters, digits and punctuation by their place, so Shift and the layout do not change the key', () => {
    expect(strokeOf(ev({ key: '!', code: 'Digit1', altKey: true, shiftKey: true }))).toEqual({ ctrl: false, alt: true, shift: true, meta: false, key: '1' });
    expect(strokeOf(ev({ key: '¡', code: 'Digit1', altKey: true }))?.key).toBe('1'); // Option+1 on a Mac
    expect(strokeOf(ev({ key: 'K', code: 'KeyK', ctrlKey: true, shiftKey: true }))?.key).toBe('k');
    expect(strokeOf(ev({ key: 'ş', code: 'Semicolon' }))?.key).toBe(';'); // a Turkish keyboard: the same place
    expect(strokeOf(ev({ key: 'Escape', code: 'Escape' }))?.key).toBe('escape');
    expect(strokeOf(ev({ key: 'ArrowUp', code: 'ArrowUp' }))?.key).toBe('up');
    expect(strokeOf(ev({ key: 'F2', code: 'F2' }))?.key).toBe('f2');
  });
  it('falls back to the key itself for an event without a place (one a script made)', () => {
    expect(strokeOf(ev({ key: 'k', code: undefined, ctrlKey: true }))?.key).toBe('k');
    expect(strokeOf(ev({ key: 'N' }))?.key).toBe('n');
  });
  it('is no stroke for a modifier alone, a keypad key, or a key typed into an input method', () => {
    expect(strokeOf(ev({ key: 'Control', code: 'ControlLeft', ctrlKey: true }))).toBeUndefined();
    expect(strokeOf(ev({ key: '1', code: 'Numpad1' }))).toBeUndefined();
    expect(strokeOf(ev({ key: 'n', code: 'KeyN', isComposing: true }))).toBeUndefined();
    expect(strokeOf(ev({ key: 'Process', code: 'KeyN', keyCode: 229 }))).toBeUndefined();
  });
});

describe('where a key may act', () => {
  it('a key that types text in a field is that field\'s (Option+key too on a Mac)', () => {
    expect(typesText(b('n')[0]!, false)).toBe(true);
    expect(typesText(b('shift+n')[0]!, false)).toBe(true);
    expect(typesText(b('space')[0]!, false)).toBe(true);
    expect(typesText(b('alt+1')[0]!, false)).toBe(false);
    expect(typesText(b('alt+1')[0]!, true)).toBe(true);
    expect(typesText(b('ctrl+k')[0]!, true)).toBe(false);
    expect(typesText(b('meta+k')[0]!, true)).toBe(false);
    expect(typesText(b('f2')[0]!, false)).toBe(false);
  });
  it('only a key no terminal program can receive goes past a terminal: any ⌘ key, or Ctrl+Shift with a letter', () => {
    for (const k of ['meta+k', 'ctrl+shift+k', 'ctrl+shift+p', 'meta+b']) expect(reachesPastTerminal(b(k)[0]!), k).toBe(true);
    for (const k of ['ctrl+k', 'ctrl+b', 'alt+1', 'alt+shift+1', 'n', 'ctrl+alt+shift+k', 'ctrl+shift+1', 'ctrl+shift+up', 'f2', 'escape']) expect(reachesPastTerminal(b(k)[0]!), k).toBe(false);
  });
});

describe('the words a key is shown with', () => {
  it('as before this rung elsewhere than a Mac: Ctrl K in the title bar, N, Alt+Shift+1', () => {
    expect(bindingLabel(b('ctrl+k'), false, ' ')).toBe('Ctrl K');
    expect(bindingLabel(b('n'), false)).toBe('N');
    expect(bindingLabel(b('alt+shift+1'), false)).toBe('Alt+Shift+1');
    expect(bindingLabel(b('ctrl+k ctrl+s'), false)).toBe('Ctrl+K Ctrl+S');
    expect(bindingLabel(b('escape'), false)).toBe('Esc');
  });
  it('a Mac\'s own symbols: ⌘ K in the title bar, ⌥⇧1', () => {
    expect(bindingLabel(b('meta+k'), true, ' ')).toBe('⌘ K');
    expect(bindingLabel(b('alt+shift+1'), true)).toBe('⌥⇧1');
    expect(bindingLabel(b('ctrl+alt+shift+meta+s'), true)).toBe('⌃⌥⇧⌘S');
  });
  it('spells aria-keyshortcuts as WAI-ARIA does', () => {
    expect(ariaKeys(b('ctrl+k'))).toBe('Control+K');
    expect(ariaKeys(b('alt+shift+1'))).toBe('Alt+Shift+1');
    expect(ariaKeys(b('n'))).toBe('N');
    expect(ariaKeys(b('meta+up'))).toBe('Meta+ArrowUp');
  });
  it('shows the ⌘ key of a command on a Mac and the Ctrl one elsewhere; the others still work', () => {
    const keys = [b('ctrl+k'), b('meta+k'), b('ctrl+shift+k')];
    expect(bindingText(primary(keys, false)!)).toBe('ctrl+k');
    expect(bindingText(primary(keys, true)!)).toBe('meta+k');
    expect(bindingText(primary([b('n')], true)!)).toBe('n');
    expect(primary([], false)).toBeUndefined();
  });
});
