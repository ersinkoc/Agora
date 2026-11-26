// @vitest-environment jsdom
// Theme packs (ADR-0113 decision 5, docs/ux/themes.md "Theme packs"): strict reading that reuses each part's own reader and refuses the
// whole pack with every problem at once; plugin panels stay ids (the placeholder) and nothing is installed; fixed keys are refused; an
// exported pack reads back the same; the import dialog applies nothing until the person ticks a part, and merges keys only after showing
// what they change.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage, t } from '../i18n';
import { EMPTY_FILE, mergeKeys, readKeys, writeKeys } from '../keys/file';
import { createKeyStore, type KeyStore } from '../keys/store';
import { PANEL_INFO } from '../layout/panels';
import { MAX_SAVED, SAVED_KEY, addNamed, savedLayouts } from '../layout/store';
import { type Layout, panesOf } from '../layout/tree';
import { readTheme, writeTheme } from './file';
import {
  MAX_PACK_BYTES, type PackIssue, deserialisePack, missingPanels, packLayout, readPack, serialisePack, writePack,
} from './pack';
import { ExportPack, ImportPack, packIssueText } from './PackDialogs';
import { createPackStore } from './kept';
import { MAX_USER_THEMES, createThemeStore, themeStore, type ThemeStore } from './store';
import { ThemeEditor } from './ThemeEditor';
import { BUILT_IN } from './tokens';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const HARBOUR = { 'agora-theme': 2, name: 'Harbour', inherits: 'slate', dark: { tokens: { accent: '#5cc8c2' } }, light: { tokens: { accent: '#0b6e69', 'on-accent': '#ffffff' } } };
const REVIEW: Layout = { version: 1, root: { type: 'split', direction: 'row', ratios: [0.5, 0.5], children: [{ type: 'pane', id: 'p1', panel: 'changes', bind: { follow: 'selection' } }, { type: 'pane', id: 'p2', panel: 'plugin.clock.face' }] } };
const KEYS = { version: 1, bindings: [{ command: 'palette.open', keys: ['ctrl+p'] }, { command: 'layout.preset-2x2', keys: ['ctrl+k 2'] }] };
const PACK = { 'agora-pack': 1, name: 'Harbour set', themes: [HARBOUR], layouts: [{ name: 'Review', layout: REVIEW }], keybindings: KEYS };

const refused = (v: unknown): PackIssue[] => { const r = readPack(v); if (r.ok) throw new Error('read'); return r.issues; };
const codes = (v: unknown) => refused(v).map((i) => i.code);
const memory = () => { const m = new Map<string, string>(); return { m, kv: { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => { m.set(k, v); }, removeItem: (k: string) => { m.delete(k); } } }; };

describe('reading a pack', () => {
  it('reads each part with its own reader: themes, named layouts and keybindings', () => {
    const r = readPack(PACK);
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.pack.name).toBe('Harbour set');
    expect(r.pack.themes).toHaveLength(1);
    expect(r.pack.themes[0]?.theme).toEqual((readTheme(HARBOUR) as { theme: unknown }).theme);
    expect(r.pack.layouts.map((l) => l.name)).toEqual(['Review']);
    expect(r.pack.keys?.file.entries.map((e) => e.command)).toEqual(['palette.open', 'layout.preset-2x2']);
    // Each part is optional.
    expect(readPack({ 'agora-pack': 1, name: 'Only keys', keybindings: KEYS }).ok).toBe(true);
    expect(readPack({ 'agora-pack': 1, name: 'Only a layout', layouts: [{ name: 'One', layout: REVIEW }] }).ok).toBe(true);
  });

  it('refuses what is not a pack, a newer one, a large one, an empty one, and one without a name', () => {
    expect(deserialisePack('{').ok || (deserialisePack('{') as { issues: PackIssue[] }).issues).toEqual([{ code: 'json' }]);
    expect(codes([])).toEqual(['invalid']);
    expect(codes({ name: 'x' })).toEqual(['invalid']);
    expect(codes({ 'agora-pack': 2, name: 'x' })).toEqual(['newer']);
    expect(codes({ 'agora-pack': 1, name: 'x' })).toEqual(['empty']);
    expect(codes({ 'agora-pack': 1, name: 'x', themes: [], keybindings: { version: 1, bindings: [] } })).toEqual(['empty']);
    expect(codes({ 'agora-pack': 1, themes: [HARBOUR] })).toEqual(['name']);
    expect(codes({ 'agora-pack': 1, name: 'a<b', themes: [HARBOUR] })).toEqual(['name']);
    const big = deserialisePack(JSON.stringify({ ...PACK, name: 'x'.repeat(MAX_PACK_BYTES) }));
    expect(big.ok || big.issues).toEqual([{ code: 'too-large', max: 256 }]);
  });

  it('refuses the whole pack for one bad part, with every problem of every part at once', () => {
    const issues = refused({
      ...PACK, script: 'alert(1)',
      themes: [HARBOUR, { ...HARBOUR, dark: { tokens: { accent: 'url(x)' } } }],
      layouts: [{ name: 'Review', layout: REVIEW, onload: 1 }],
      keybindings: { version: 1, bindings: [{ command: 'window.safe-mode', keys: ['ctrl+s'] }] },
    });
    expect(issues).toEqual([
      { code: 'unknown-field', field: 'script' },
      { code: 'theme', index: 1, issue: { code: 'unsafe', field: 'dark.tokens.accent' } },
      { code: 'layout-field', index: 0, field: 'onload' },
      { code: 'keys', issue: { code: 'fixed-command', command: 'window.safe-mode' } },
    ]);
    expect(issues.map(packIssueText).every((s) => s.length > 0 && !s.includes('pack-'))).toBe(true);
  });

  it('reads themes with the theme rules: the floor, unknown fields, readability, and format 2 only', () => {
    expect(refused({ ...PACK, themes: [{ ...HARBOUR, dark: { tokens: { danger: '#ff0000' } } }] })).toEqual([
      { code: 'theme', index: 0, issue: { code: 'floor', field: 'dark.tokens.danger', item: 'danger' } },
    ]);
    expect(refused({ ...PACK, themes: [{ ...HARBOUR, css: 'flat' }] })).toEqual([{ code: 'theme', index: 0, issue: { code: 'unknown-field', field: 'css' } }]);
    expect(refused({ ...PACK, themes: [{ ...HARBOUR, dark: { tokens: { text: '#0b1016', 'surface-0': '#0b1016' } } }] })[0]?.code).toBe('theme');
    expect(codes({ ...PACK, themes: [{ 'agora-theme': 1, name: 'Old', scheme: 'dark', tokens: {} }] })).toEqual(['theme-format']);
    expect(codes({ ...PACK, themes: ['harbour.json'] })).toEqual(['theme-format']);
    expect(refused({ ...PACK, themes: [{ ...HARBOUR, 'agora-theme': 3 }] })).toEqual([{ code: 'theme', index: 0, issue: { code: 'newer' } }]);
    expect(codes({ ...PACK, themes: Array.from({ length: MAX_USER_THEMES + 1 }, () => HARBOUR) })).toEqual(['too-many']);
    expect(codes({ ...PACK, themes: HARBOUR })).toEqual(['part']);
  });

  it('holds layouts to the closed set: panels by id only, no agents, no unknown fields, the layout rules', () => {
    const pane = (extra: object) => ({ ...PACK, layouts: [{ name: 'L', layout: { version: 1, root: { type: 'pane', id: 'p1', panel: 'usage', ...extra } } }] });
    expect(refused(pane({ style: 'x' }))).toEqual([{ code: 'layout-field', index: 0, field: 'root.style' }]);
    expect(refused(pane({ panel: 'javascript:alert(1)' }))).toEqual([{ code: 'layout-panel', index: 0, panel: 'javascript:alert(1)' }]);
    expect(refused(pane({ panel: 'Usage Panel' }))).toEqual([{ code: 'layout-panel', index: 0, panel: 'Usage Panel' }]);
    expect(refused(pane({ bind: { agent: 'laptop/claude-1' } }))).toEqual([{ code: 'layout-agent', index: 0 }]);
    expect(refused(pane({ bind: { follow: 'everything' } }))).toEqual([{ code: 'layout-field', index: 0, field: 'root.bind' }]);
    expect(refused(pane({ id: '<x>' }))).toEqual([{ code: 'layout-field', index: 0, field: 'root.id' }]);
    expect(refused({ ...PACK, layouts: [{ name: 'L', layout: { version: 1, root: REVIEW.root, extra: true } }] })).toEqual([{ code: 'layout-field', index: 0, field: 'extra' }]);
    expect(refused({ ...PACK, layouts: [{ name: 'L', layout: { version: 2, root: REVIEW.root } }] })).toEqual([{ code: 'layout', index: 0, error: 'newer' }]);
    let deep: unknown = { type: 'pane', panel: 'usage' };
    for (let i = 0; i < 7; i++) deep = { type: 'split', direction: i % 2 ? 'row' : 'column', children: [deep, { type: 'pane', panel: 'usage' }] };
    expect(refused({ ...PACK, layouts: [{ name: 'L', layout: { version: 1, root: deep } }] })).toEqual([{ code: 'layout', index: 0, error: 'too-deep' }]);
    expect(refused({ ...PACK, layouts: [{ name: 'L', layout: REVIEW }, { name: 'L', layout: REVIEW }] })).toEqual([{ code: 'layout-twice', index: 1, name: 'L' }]);
    expect(refused({ ...PACK, layouts: [{ layout: REVIEW }, 'x'] })).toEqual([{ code: 'layout-name', index: 0 }, { code: 'layout-entry', index: 1 }]);
    expect(codes({ ...PACK, layouts: Array.from({ length: MAX_SAVED + 1 }, (_, i) => ({ name: `L${i}`, layout: REVIEW })) })).toEqual(['too-many']);
  });

  it('keeps a plugin panel as its id: the window has no such panel, so the pane shows the placeholder; nothing is installed', () => {
    const r = readPack(PACK);
    if (!r.ok) throw new Error('read');
    const layout = r.pack.layouts[0]?.layout as Layout;
    expect(panesOf(layout.root).map((p) => p.panel)).toEqual(['changes', 'plugin.clock.face']);
    expect(PANEL_INFO.get('plugin.clock.face')).toBeUndefined();
    expect(missingPanels(layout, (id) => !!PANEL_INFO.get(id))).toEqual({ plugins: ['plugin.clock.face'], unknown: [] });
    expect(missingPanels({ version: 1, root: { type: 'pane', id: 'p1', panel: 'browser' } }, (id) => !!PANEL_INFO.get(id))).toEqual({ plugins: [], unknown: ['browser'] });
  });

  it('keeps the fixed-key rules: the safe-mode key, moving around, a request command', () => {
    const keys = (bindings: unknown[]) => ({ ...PACK, keybindings: { version: 1, bindings } });
    expect(refused(keys([{ command: 'go.home', keys: ['ctrl+alt+shift+s'] }]))).toEqual([{ code: 'keys', issue: { code: 'fixed-key', command: 'go.home', key: 'ctrl+alt+shift+s', reason: 'safe-mode' } }]);
    expect(refused(keys([{ command: 'go.home', keys: ['escape'] }]))[0]).toMatchObject({ code: 'keys', issue: { code: 'fixed-key', reason: 'navigation' } });
    expect(refused(keys([{ command: 'request.allow', keys: ['ctrl+y'] }]))).toEqual([{ code: 'keys', issue: { code: 'request', command: 'request.allow' } }]);
    expect(refused({ ...PACK, keybindings: { version: 1, bindings: [], when: 'always' } })).toEqual([{ code: 'keys', issue: { code: 'field', at: 'when' } }]);
    expect(codes({ ...PACK, keybindings: [] })).toEqual(['part']);
  });

  it('lists at most 50 problems, then how many more', () => {
    const issues = refused({ ...PACK, layouts: Array.from({ length: 30 }, () => ({ nm: 1, layout: 1 })) });
    expect(issues).toHaveLength(51);
    expect(issues[50]).toEqual({ code: 'more', count: 30 * 3 - 50 });
  });
});

describe('writing a pack', () => {
  it('reads back what it wrote: the same themes, the layouts as panels only, the same keys', () => {
    const sand = BUILT_IN.find((b) => b.id === 'sand')!;
    const harbour = (readTheme(HARBOUR) as { ok: true; theme: Parameters<typeof writeTheme>[0] }).theme;
    const bound: Layout = { version: 1, root: { type: 'tabs', active: 1, children: [{ type: 'pane', id: 'a', panel: 'agent', bind: { agent: 'laptop/claude-1' } }, { type: 'pane', id: 'b', panel: 'terminal', bind: { follow: 'attention' } }] } };
    const keys = (readKeys(JSON.stringify(KEYS)) as { ok: true; file: typeof EMPTY_FILE }).file;
    const file = writePack({ name: 'Mine', themes: [{ theme: harbour, name: 'Harbour' }, { theme: sand, name: 'Sand' }], layouts: [{ name: 'Review', layout: REVIEW }, { name: 'Tabs', layout: bound }], keys });
    const back = deserialisePack(serialisePack(file));
    if (!back.ok) throw new Error(JSON.stringify(back.issues));
    expect(back.pack.name).toBe('Mine');
    expect(back.pack.themes.map((x) => writeTheme(x.theme, x.theme.name))).toEqual([writeTheme(harbour, 'Harbour'), writeTheme(sand, 'Sand')]);
    expect(back.pack.layouts).toEqual([{ name: 'Review', layout: REVIEW }, { name: 'Tabs', layout: packLayout(bound) }]);
    // The agent is someone's own: the pane is written unbound; following what needs you is kept.
    expect(panesOf(back.pack.layouts[1]!.layout.root).map((p) => p.bind)).toEqual([undefined, { follow: 'attention' }]);
    expect(back.pack.keys?.file).toEqual(keys);
    // And once more: what is read back writes the same file.
    expect(writePack({ name: back.pack.name, themes: back.pack.themes.map((x) => ({ theme: x.theme, name: x.theme.name })), layouts: back.pack.layouts, keys: back.pack.keys?.file })).toEqual(file);
  });

  it('leaves out a part with nothing in it', () => {
    expect(writePack({ name: 'x', themes: [], layouts: [], keys: EMPTY_FILE })).toEqual({ 'agora-pack': 1, name: 'x' });
  });
});

describe('what applying changes', () => {
  it('merges keys over the person\'s own, saying which of theirs it replaces and which keys would then clash', () => {
    const mine = (readKeys(JSON.stringify({ version: 1, bindings: [{ command: 'palette.open', keys: ['ctrl+o'] }, { command: 'go.home', keys: ['ctrl+h'] }] })) as { ok: true; file: typeof EMPTY_FILE }).file;
    const theirs = (readKeys(JSON.stringify({ version: 1, bindings: [{ command: 'palette.open', keys: ['ctrl+p'] }, { command: 'go.audit', keys: ['ctrl+h'] }, { command: 'future.x', keys: [] }] })) as { ok: true; file: typeof EMPTY_FILE }).file;
    const m = mergeKeys(mine, [], theirs, ['future.x']);
    expect(m.file.entries).toEqual([{ command: 'palette.open', keys: ['ctrl+p'] }, { command: 'go.home', keys: ['ctrl+h'] }, { command: 'go.audit', keys: ['ctrl+h'] }, { command: 'future.x', keys: [] }]);
    expect(m.replaced).toEqual([{ command: 'palette.open', mine: ['ctrl+o'], theirs: ['ctrl+p'] }]);
    expect(m.clashes).toEqual([{ a: 'go.home', b: 'go.audit', keys: 'ctrl+h', kind: 'same' }]);
    expect(m.unknown).toEqual(['future.x']);
    expect(m.issues).toEqual([]);
  });

  it('refuses a merge that could not be kept (it would be refused on the next load, and the person\'s keys with it)', () => {
    const many = (prefix: string, n: number) => ({ entries: Array.from({ length: n }, (_, i) => ({ command: `${prefix}.c${i}`, keys: [] })) });
    const m = mergeKeys(many('mine', 150), [], many('theirs', 100), []);
    expect(m.issues).toEqual([{ code: 'too-many' }]);
  });

  it('adds layouts as saved layouts, never replacing one, and adds none when they do not all fit', () => {
    const { m, kv } = memory();
    addNamed([{ name: 'Review', layout: REVIEW }], (name, n) => `${name} (${n})`, kv);
    const r = addNamed([{ name: 'Review', layout: REVIEW }, { name: 'Review (2)', layout: REVIEW }], (name, n) => `${name} (${n})`, kv);
    expect(r.ok && r.names).toEqual(['Review (2)', 'Review (2) (2)']);
    expect(savedLayouts(kv).map((l) => l.name)).toEqual(['Review (2)', 'Review (2) (2)', 'Review']);
    const long = 'x'.repeat(60);
    addNamed([{ name: long, layout: REVIEW }], (name, n) => `${name} (${n})`, kv);
    const again = addNamed([{ name: long, layout: REVIEW }], (name, n) => `${name} (${n})`, kv);
    expect(again.ok && again.names[0]).toBe(`${'x'.repeat(50)} (2)`);
    const before = m.get(SAVED_KEY);
    const full = addNamed(Array.from({ length: MAX_SAVED }, (_, i) => ({ name: `L${i}`, layout: REVIEW })), (name, n) => `${name} (${n})`, kv);
    expect(full).toEqual({ ok: false, room: MAX_SAVED - 5 });
    expect(m.get(SAVED_KEY)).toBe(before);
  });

  it('adds themes to the person\'s own without choosing one, and none when they do not all fit', () => {
    const store = createThemeStore({ kv: memory().kv });
    const harbour = (readTheme(HARBOUR) as { ok: true; theme: Parameters<ThemeStore['add']>[0][number] }).theme;
    const r = store.add([harbour, harbour]);
    expect(r.ok && r.themes.map((x) => x.id)).toEqual(['user-1', 'user-2']);
    expect(store.active().id).toBe('dark');
    expect(store.room()).toBe(MAX_USER_THEMES - 2);
    expect(store.add(Array.from({ length: MAX_USER_THEMES - 1 }, () => harbour))).toEqual({ ok: false, full: true });
    expect(store.state().user).toHaveLength(2);
  });
});

// ---- The dialogs --------------------------------------------------------------------------------------------------------------------

let host: HTMLElement;
let root: Root;
let themes: ThemeStore;
let keys: KeyStore;
beforeEach(() => {
  setLanguage('en');
  localStorage.clear();
  themes = createThemeStore({ kv: memory().kv });
  keys = createKeyStore({ kv: memory().kv });
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => { act(() => root.unmount()); host.remove(); vi.restoreAllMocks(); });

const dialog = () => document.querySelector<HTMLElement>('[data-pack-dialog]') as HTMLElement;
const button = (name: string) => [...dialog().querySelectorAll('button')].find((b) => b.textContent === name) as HTMLButtonElement;
const click = (el: Element) => act(() => { (el as HTMLElement).click(); });
const boxes = () => [...dialog().querySelectorAll<HTMLInputElement>('input[type="checkbox"]')];
function paste(text: string) {
  const area = dialog().querySelector('textarea') as HTMLTextAreaElement;
  act(() => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set?.call(area, text);
    area.dispatchEvent(new Event('input', { bubbles: true }));
  });
}
const snapshot = () => ({ local: JSON.stringify(Object.entries(localStorage)), themes: themes.state(), keys: keys.state() });

describe('importing a pack in the window', () => {
  it('shows what the pack holds and applies nothing until a part is ticked', () => {
    let closed = false;
    act(() => root.render(<ImportPack onClose={() => { closed = true; }} themes={themes} keys={keys} mac={false} />));
    paste(serialisePack(PACK));
    const before = snapshot();
    click(button(t('pack-preview-do')));
    expect(dialog().getAttribute('data-pack-dialog')).toBe('preview');
    expect(dialog().textContent).toContain('Harbour set');
    expect(dialog().textContent).toContain('Harbour');
    expect(dialog().textContent).toContain('Review');
    expect(dialog().querySelector('[data-pack-plugin-panels]')?.textContent).toContain('plugin.clock.face');
    expect(boxes()).toHaveLength(3);
    expect(boxes().every((b) => !b.checked)).toBe(true);
    expect(button(t('pack-apply')).disabled).toBe(true);
    expect(snapshot()).toEqual(before);
    // Back and Cancel leave everything as it was.
    click(button(t('pack-back')));
    click(button(t('layout-cancel')));
    expect(closed).toBe(true);
    expect(snapshot()).toEqual(before);
  });

  it('applies only the parts ticked: a theme is added (not chosen), a layout is saved, nothing else is touched (no plugin)', () => {
    const packs = createPackStore();
    act(() => root.render(<ImportPack onClose={() => {}} themes={themes} keys={keys} packs={packs} mac={false} />));
    paste(serialisePack(PACK));
    click(button(t('pack-preview-do')));
    const keysBefore = keys.state();
    const othersBefore = Object.keys(localStorage).filter((k) => k !== SAVED_KEY);
    click(boxes()[0]!);
    click(boxes()[1]!);
    click(button(t('pack-apply')));
    expect(dialog().getAttribute('data-pack-dialog')).toBe('done');
    expect(themes.state().user.map((u) => u.name)).toEqual(['Harbour']);
    expect(themes.active().id).toBe('dark');
    expect(savedLayouts().map((l) => l.name)).toEqual(['Review']);
    expect(panesOf(savedLayouts()[0]!.layout.root).map((p) => p.panel)).toEqual(['changes', 'plugin.clock.face']);
    expect(keys.state()).toBe(keysBefore);
    // The only key of this browser that changed is the saved layouts' (themes and keys are in their own stores here): no plugin was
    // installed, allowed or enabled.
    expect(Object.keys(localStorage).filter((k) => k !== SAVED_KEY)).toEqual(othersBefore);
    expect(Object.keys(localStorage).some((k) => k.startsWith('agora.plugins'))).toBe(false);
    // ADR-0113 step 4: the pack is kept as the file it was, once, for the person's other devices.
    expect(packs.list().map((k) => k.file)).toEqual([PACK]);
  });

  it('merges keys only after a step that shows the conflicts, and Back there changes nothing', () => {
    keys.set({ entries: [{ command: 'palette.open', keys: ['ctrl+o'] }, { command: 'go.home', keys: ['ctrl+k 2'] }] });
    act(() => root.render(<ImportPack onClose={() => {}} themes={themes} keys={keys} mac={false} />));
    paste(serialisePack(PACK));
    click(button(t('pack-preview-do')));
    click(boxes()[2]!);
    click(button(t('pack-next')));
    expect(dialog().getAttribute('data-pack-dialog')).toBe('keys');
    expect(dialog().querySelector('[data-pack-replaced]')?.textContent).toContain('Ctrl+O');
    expect(dialog().querySelector('[data-pack-replaced]')?.textContent).toContain('Ctrl+P');
    expect(dialog().querySelector('[data-pack-clashes]')?.textContent).toContain('Ctrl+K 2');
    const before = keys.state();
    click(button(t('pack-back')));
    expect(keys.state()).toBe(before);
    click(button(t('pack-next')));
    click(button(t('pack-keys-merge-do')));
    expect(keys.state().file.entries).toEqual([{ command: 'palette.open', keys: ['ctrl+p'] }, { command: 'go.home', keys: ['ctrl+k 2'] }, { command: 'layout.preset-2x2', keys: ['ctrl+k 2'] }]);
    expect(themes.state().user).toEqual([]);
    expect(savedLayouts()).toEqual([]);
  });

  it('refuses a broken pack with every problem and changes nothing', () => {
    act(() => root.render(<ImportPack onClose={() => {}} themes={themes} keys={keys} mac={false} />));
    paste(JSON.stringify({ ...PACK, layouts: [{ name: 'X', layout: { version: 1, root: { type: 'pane', panel: 'usage', bind: { agent: 'a/b' } } } }], extra: 1 }));
    const before = snapshot();
    click(button(t('pack-preview-do')));
    const alert = dialog().querySelector('[role="alert"]') as HTMLElement;
    expect(alert.textContent).toContain(t('pack-error-unknown-field', { field: 'extra' }));
    expect(alert.textContent).toContain(t('pack-error-layout-agent', { n: 1 }));
    expect(snapshot()).toEqual(before);
  });

  it('adds nothing when the ticked themes do not fit, and says so', () => {
    const harbour = (readTheme(HARBOUR) as { ok: true; theme: Parameters<ThemeStore['add']>[0][number] }).theme;
    themes.add(Array.from({ length: MAX_USER_THEMES }, () => harbour));
    act(() => root.render(<ImportPack onClose={() => {}} themes={themes} keys={keys} mac={false} />));
    paste(serialisePack(PACK));
    click(button(t('pack-preview-do')));
    click(boxes()[0]!);
    click(boxes()[1]!);
    expect(dialog().textContent).toContain(t('pack-themes-room', { count: 0 }));
    click(button(t('pack-apply')));
    expect(dialog().querySelector('[role="alert"]')).not.toBeNull();
    expect(savedLayouts()).toEqual([]);
  });
});

describe('exporting a pack in the window', () => {
  it('writes the parts chosen into a file that imports back', async () => {
    const harbour = (readTheme(HARBOUR) as { ok: true; theme: Parameters<ThemeStore['add']>[0][number] }).theme;
    themes.add([harbour]);
    themes.choose('user-1');
    addNamed([{ name: 'Review', layout: REVIEW }], (name, n) => `${name} (${n})`);
    keys.set({ entries: [{ command: 'palette.open', keys: ['ctrl+p'] }] });
    let blob: Blob | undefined;
    vi.spyOn(URL, 'createObjectURL').mockImplementation((b) => { blob = b as Blob; return 'blob:x'; });
    vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => {});
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});
    act(() => root.render(<ExportPack onClose={() => {}} themes={themes} keys={keys} />));
    // The theme on screen is ticked to start with.
    expect(boxes().filter((b) => b.checked)).toHaveLength(1);
    const labels = [...dialog().querySelectorAll('.pack-row')].map((r) => r.textContent ?? '');
    click(boxes()[labels.findIndex((l) => l.startsWith('Review'))]!);
    click(boxes()[boxes().length - 1]!);
    click(button(t('pack-export-do')));
    const back = deserialisePack(await (blob as Blob).text());
    if (!back.ok) throw new Error(JSON.stringify(back.issues));
    expect(back.pack.name).toBe('Harbour');
    expect(back.pack.themes.map((x) => x.theme.name)).toEqual(['Harbour']);
    expect(back.pack.layouts.map((l) => l.name)).toEqual(['Review']);
    expect(writeKeys(back.pack.keys!.file)).toBe(writeKeys(keys.state().file));
  });
});

describe('the theme editor and packs', () => {
  it('has Import pack and Export as pack, and a pack given to Import… (a theme) says where it goes and adds nothing', async () => {
    themeStore.reload();
    act(() => root.render(<ThemeEditor />));
    const buttons = [...host.querySelectorAll('button')].map((b) => b.textContent);
    expect(buttons).toContain(t('pack-import'));
    expect(buttons).toContain(t('pack-export'));
    const input = host.querySelector('input[type="file"]') as HTMLInputElement;
    const file = new File([serialisePack(PACK)], 'set.json', { type: 'application/json' });
    Object.defineProperty(input, 'files', { value: [file], configurable: true });
    await act(async () => { input.dispatchEvent(new Event('change', { bubbles: true })); await new Promise((r) => setTimeout(r, 10)); });
    expect(host.querySelector('[role="alert"]')?.textContent).toBe(t('theme-import-is-pack'));
    expect(themeStore.state().user).toEqual([]);
  });
});
