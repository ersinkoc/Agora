import { describe, expect, it } from 'vitest';
import { COMMANDS } from './commands';
import { EMPTY_FILE, MAX_ENTRIES, MAX_FILE_BYTES, conflicts, keymap, keysOf, readKeys, readKeysValue, withKeys, writeKeys, type KeyFile, type KeyIssue } from './file';
import { bindingText } from './strokes';

const file = (bindings: unknown[], extra: object = {}) => JSON.stringify({ version: 1, bindings, ...extra });
const refused = (text: string): KeyIssue[] => { const r = readKeys(text); if (r.ok) throw new Error('read'); return r.issues; };
const codes = (text: string) => refused(text).map((i) => i.code);
const shown = (f: KeyFile, id: string) => keysOf(keymap(f), id).map(bindingText);

describe('reading a keybindings file', () => {
  it('reads commands and their keys, in one spelling, and writes back the same file', () => {
    const r = readKeys(file([{ command: 'palette.open', keys: ['Ctrl+P', 'meta+p'] }, { command: 'layout.preset-2x2', keys: ['ctrl+k 2'] }, { command: 'attention.next', keys: [] }]));
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.file.entries).toEqual([{ command: 'palette.open', keys: ['ctrl+p', 'meta+p'] }, { command: 'layout.preset-2x2', keys: ['ctrl+k 2'] }, { command: 'attention.next', keys: [] }]);
    expect(r.unknown).toEqual([]);
    const again = readKeys(writeKeys(r.file));
    expect(again.ok && again.file).toEqual(r.file);
  });
  it('keeps a command this window does not know, marks it, and writes it back (a newer window\'s file survives)', () => {
    const r = readKeys(file([{ command: 'future.thing', keys: ['ctrl+shift+y'] }]));
    expect(r.ok && r.unknown).toEqual(['future.thing']);
    if (!r.ok) return;
    expect(writeKeys(r.file)).toContain('future.thing');
    expect(keymap(r.file).some((x) => x.command === 'future.thing')).toBe(false); // binds nothing here
  });
  it('refuses a malformed key string, and the whole file with it', () => {
    expect(refused(file([{ command: 'palette.open', keys: ['ctrl+p'] }, { command: 'go.home', keys: ['ctrl+'] }]))).toEqual([{ code: 'key', command: 'go.home', key: 'ctrl+' }]);
    expect(codes(file([{ command: 'go.home', keys: [3] }]))).toEqual(['key']);
    expect(codes(file([{ command: 'go.home', keys: ['ctrl+k ctrl+s ctrl+x'] }]))).toEqual(['key']);
  });
  it('refuses unknown fields at every level (a closed set)', () => {
    expect(refused(file([], { name: 'mine' }))).toEqual([{ code: 'field', at: 'name' }]);
    expect(refused(file([{ command: 'go.home', keys: [], when: 'terminalFocus' }]))).toEqual([{ code: 'field', at: 'bindings[0].when' }]);
  });
  it('refuses what is not a file, a newer one, a large one, too many entries', () => {
    expect(codes('{')).toEqual(['json']);
    expect(codes('[]')).toEqual(['shape']);
    expect(codes(JSON.stringify({ version: 1 }))).toEqual(['shape']);
    expect(codes(JSON.stringify({ version: '1', bindings: [] }))).toEqual(['shape']);
    expect(codes(JSON.stringify({ version: 1.5, bindings: [] }))).toEqual(['shape']);
    expect(codes(JSON.stringify({ version: 2, bindings: [] }))).toEqual(['newer']);
    expect(codes(file(['go.home']))).toEqual(['shape']);
    expect(codes(' '.repeat(MAX_FILE_BYTES + 1))).toEqual(['too-big']);
    expect(codes(file(Array.from({ length: MAX_ENTRIES + 1 }, (_, i) => ({ command: `x.c${i}`, keys: [] }))))).toEqual(['too-many']);
    expect(codes(file([{ command: 'go.home', keys: ['a', 'b', 'c', 'd', 'e'] }]))).toEqual(['too-many-keys']);
  });
  it('refuses a name that is not a command id, and a command listed twice', () => {
    for (const command of ['', 'Go.Home', '__proto__', 'go..home', 'go.home ', '<b>x</b>', `a${'b'.repeat(64)}`]) {
      expect(codes(file([{ command, keys: [] }])), command).toEqual(['command']);
    }
    expect(codes(file([{ command: 'go.home', keys: [] }, { command: 'go.home', keys: ['ctrl+h'] }]))).toEqual(['twice']);
    // A word that names something on every object is only an unknown command: kept, binding nothing.
    const r = readKeys(file([{ command: 'constructor', keys: ['ctrl+shift+y'] }]));
    expect(r.ok && r.unknown).toEqual(['constructor']);
    if (r.ok) expect(keymap(r.file).some((x) => x.command === 'constructor')).toBe(false);
  });
  it('never lets a file give keys to answering a request', () => {
    for (const command of ['request.allow', 'request.deny', 'approval.always', 'approve', 'allow.once', 'deny']) {
      expect(codes(file([{ command, keys: ['ctrl+shift+y'] }])), command).toEqual(['request']);
    }
    expect(COMMANDS.some((c) => /^(request|approval|approve|deny|allow)\b/.test(c.id))).toBe(false);
  });
  it('never lets a file change the safe-mode command or use a fixed key for anything', () => {
    expect(codes(file([{ command: 'window.safe-mode', keys: ['ctrl+shift+y'] }]))).toEqual(['fixed-command']);
    expect(codes(file([{ command: 'window.safe-mode', keys: [] }]))).toEqual(['fixed-command']);
    expect(refused(file([{ command: 'go.home', keys: ['shift+ctrl+alt+s'] }]))).toEqual([{ code: 'fixed-key', command: 'go.home', key: 'ctrl+alt+shift+s', reason: 'safe-mode' }]);
    expect(refused(file([{ command: 'go.home', keys: ['cmd+option+shift+s'] }]))[0]).toMatchObject({ reason: 'safe-mode' });
    expect(refused(file([{ command: 'go.home', keys: ['escape'] }]))[0]).toMatchObject({ reason: 'navigation' });
    expect(refused(file([{ command: 'go.home', keys: ['ctrl+k tab'] }]))[0]).toMatchObject({ reason: 'navigation' }); // any stroke of a chord
    expect(refused(file([{ command: 'go.home', keys: ['ctrl+shift+c'] }]))[0]).toMatchObject({ reason: 'terminal' });
    expect(refused(file([{ command: 'go.home', keys: ['ctrl+v'] }]))[0]).toMatchObject({ reason: 'editing' });
  });
  it('reports every problem at once, capped', () => {
    expect(codes(file([{ command: 'go.home', keys: ['ctrl+'] }, { command: 'go.machines', keys: ['escape'] }], { x: 1 }))).toEqual(['field', 'key', 'fixed-key']);
    const many = refused(file(Array.from({ length: 80 }, (_, i) => ({ command: `x.c${i}`, keys: ['nope+k'] }))));
    expect(many).toHaveLength(51);
    expect(many.at(-1)).toEqual({ code: 'more', count: 30 });
  });
  it('is data and never run: a value is only ever a string compared with the list', () => {
    const r = readKeysValue({ version: 1, bindings: [{ command: 'go.home', keys: ['ctrl+h'] }], toString: undefined });
    expect(r.ok).toBe(false); // an unknown field, refused, not called
    expect(readKeys('{"version":1,"bindings":[{"command":"go.home","keys":["alert(1)"]}]}').ok).toBe(false);
  });
});

describe('the keys in force', () => {
  it('are the defaults with nothing changed, and those reproduce the shortcuts before this rung', () => {
    expect(shown(EMPTY_FILE, 'palette.open')).toEqual(['ctrl+k', 'meta+k', 'ctrl+shift+k']);
    expect(shown(EMPTY_FILE, 'attention.next')).toEqual(['n']);
    expect(shown(EMPTY_FILE, 'window.agents')).toEqual(['ctrl+b', 'meta+b', 'ctrl+shift+b']);
    for (let i = 1; i <= 9; i++) expect(shown(EMPTY_FILE, `pane.tab-${i}`)).toEqual([`alt+${i}`]);
    for (let i = 1; i <= 5; i++) expect(shown(EMPTY_FILE, `side.place-${i}`)).toEqual([`alt+shift+${i}`]);
    expect(shown(EMPTY_FILE, 'go.home')).toEqual([]);
  });
  it('a file replaces a command\'s keys; an empty list takes them away; a fixed command keeps its own', () => {
    const f: KeyFile = { entries: [{ command: 'palette.open', keys: ['ctrl+p'] }, { command: 'attention.next', keys: [] }, { command: 'window.safe-mode', keys: ['ctrl+shift+y'] }] };
    expect(shown(f, 'palette.open')).toEqual(['ctrl+p']);
    expect(shown(f, 'attention.next')).toEqual([]);
    expect(shown(f, 'window.safe-mode')).toEqual(['ctrl+alt+shift+s', 'alt+shift+meta+s']);
  });
  it('the person\'s own keys come first, so their choice wins a key over a default', () => {
    const map = keymap({ entries: [{ command: 'layout.preset-2x2', keys: ['ctrl+b'] }] });
    expect(map.find((x) => x.text === 'ctrl+b')?.command).toBe('layout.preset-2x2');
    expect(map.find((x) => x.command === 'layout.preset-2x2')?.own).toBe(true);
  });
  it('edits one command at a time, keeping the others and their order', () => {
    let f = withKeys(EMPTY_FILE, 'go.home', ['ctrl+h']);
    f = withKeys(f, 'go.machines', ['ctrl+m']);
    f = withKeys(f, 'go.home', ['f2']);
    expect(f.entries).toEqual([{ command: 'go.home', keys: ['f2'] }, { command: 'go.machines', keys: ['ctrl+m'] }]);
    expect(withKeys(f, 'go.home', undefined).entries).toEqual([{ command: 'go.machines', keys: ['ctrl+m'] }]);
  });
});

describe('conflicts', () => {
  it('the defaults have none', () => {
    expect(conflicts(keymap(EMPTY_FILE))).toEqual([]);
  });
  it('the same keys on two commands', () => {
    expect(conflicts(keymap({ entries: [{ command: 'go.home', keys: ['ctrl+b'] }] }))).toEqual([{ a: 'go.home', b: 'window.agents', keys: 'ctrl+b', kind: 'same' }]);
  });
  it('a key that starts another command\'s chord (the chord waits, so the shorter one cannot act)', () => {
    expect(conflicts(keymap({ entries: [{ command: 'go.keyboard', keys: ['ctrl+k ctrl+s'] }] }))).toEqual([{ a: 'go.keyboard', b: 'palette.open', keys: 'ctrl+k', kind: 'prefix' }]);
  });
  it('not two keys of one command', () => {
    expect(conflicts(keymap({ entries: [{ command: 'go.home', keys: ['ctrl+h', 'ctrl+h ctrl+j'] }] }))).toEqual([]);
  });
});
