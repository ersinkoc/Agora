import { readFileSync } from 'node:fs';
import { afterEach, describe, expect, it } from 'vitest';
import { setLanguage, t } from '../i18n';
import { terminalTheme } from '../terminal/palette';
import { cloneLooks, deserialise, judge, readTheme, serialise, writeTheme } from './file';
import { ANSI, ANSI_NAMES, CHROME, CHROME_NAMES, DEFAULT_CHROME, FLOOR, FLOOR_ITEMS, REGIONS, REGION_PARTS, SYNTAX_KEYS, regionVar, terminalVar } from './look';
import { chromeLabel, chromeValueLabel, floorLabel, floorReason, issueText } from './messages';
import { USER_KEY, createThemeStore, loadState } from './store';
import { BUILT_IN, type Theme, type ThemeIssue, applyTheme, baseTokens, contrast, parseTheme, themeFile } from './tokens';

const v1 = (tokens: Record<string, unknown> = {}, extra: Record<string, unknown> = {}) => ({ 'agora-theme': 1, name: 'Mine', scheme: 'dark', tokens, ...extra });
const v2 = (extra: Record<string, unknown> = {}) => ({ 'agora-theme': 2, name: 'Harbour', ...extra });
const issues = (r: ReturnType<typeof readTheme>): ThemeIssue[] => (r.ok ? [] : r.issues);
const codes = (r: ReturnType<typeof readTheme>) => issues(r).map((i) => i.code);
function memory(init: Record<string, string> = {}) {
  const m = new Map(Object.entries(init));
  return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => { m.set(k, v); }, removeItem: (k: string) => { m.delete(k); }, m };
}
function page() {
  const vars = new Map<string, string>();
  return { vars, root: { style: { setProperty: (k: string, v: string) => { vars.set(k, v); }, removeProperty: (k: string) => vars.delete(k) }, dataset: {} as Record<string, string | undefined> } };
}

// A theme that uses every part of format 2: both schemes, inherits, regions, the terminal, syntax, chrome and text.
const harbour = () => v2({
  inherits: 'slate',
  dark: {
    tokens: { accent: '#5cc8c2' },
    regions: { sidebar: { surface: '#0b1016', text: '#e6edf3' }, 'code-blocks': { line: '#2a3541' } },
    terminal: { background: '#05080c', foreground: '#d8e1ea', red: '#ff6b6b', 'bright-black': '#5c6773' },
    syntax: { keyword: '#c099ff' },
  },
  light: { tokens: { accent: '#0b6e69', 'on-accent': '#ffffff' } },
  shape: { 'radius-m': 8 },
  text: { 'font-mono': "'JetBrains Mono', monospace", weight: 500, 'line-height': 1.5, ligatures: false },
  chrome: { 'pane-gap': 6, 'tab-style': 'pill', density: 'compact', 'pane-borders': 'always', 'tab-bar': 'bottom', 'state-display': 'word', shadow: 'none', motion: 'reduced', 'line-width': 2 },
});

describe('format 1 files still open', () => {
  it('reads exactly as before, as a one-scheme theme, and its format-2 export reads back the same', () => {
    for (const b of BUILT_IN) {
      const file = themeFile(b, 'x');
      const r = readTheme(file);
      expect(r.ok, b.id).toBe(true);
      if (!r.ok) continue;
      const old = parseTheme(file);
      expect(old.ok && r.theme.tokens, b.id).toEqual(old.ok && old.tokens);
      expect(r.theme.scheme).toBe(b.scheme);
      expect(Object.keys(r.theme.looks)).toEqual([b.scheme]);
      expect(r.theme.chrome).toEqual(DEFAULT_CHROME);
      const out = writeTheme(r.theme, 'x');
      expect(out['agora-theme']).toBe(2);
      const back = readTheme(JSON.parse(serialise(out)));
      expect(back).toEqual({ ok: true, theme: r.theme });
      if (back.ok) expect(serialise(writeTheme(back.theme, 'x'))).toBe(serialise(out));
    }
  });

  it('lifts a base text size under 16 px to 16 (no text under 12 px) with a warning, and still converts and reloads', () => {
    for (const [size, lifted] of [[12, true], [15, true], [16, false], [24, false]] as const) {
      const r = readTheme(v1({ 'text-16': size }));
      expect(r.ok, String(size)).toBe(true);
      if (!r.ok) continue;
      expect(r.theme.tokens['text-16']).toBe(lifted ? '16px' : `${size}px`);
      expect(r.warnings?.filter((w) => w.code === 'text-lifted') ?? []).toEqual(lifted ? [{ code: 'text-lifted', from: `${size}px`, to: '16px' }] : []);
      expect(readTheme(writeTheme(r.theme, 'x')).ok).toBe(true);
    }
    // A theme stored by the earlier rung (format 1, 12 px) survives a reload, and is stored again as format 2.
    const kv = memory({ [USER_KEY]: JSON.stringify([{ id: 'user-1', file: v1({ 'text-16': '12px' }, { name: 'Small; tight' }) }]) });
    const s = createThemeStore({ kv });
    expect(s.state().user.map((u) => [u.name, u.tokens['text-16']])).toEqual([['Small tight', '16px']]);
    s.update('user-1', { name: 'Small' });
    const stored = JSON.parse(kv.m.get(USER_KEY) as string);
    expect(stored[0].file['agora-theme']).toBe(2);
    expect(loadState(kv).user.map((u) => u.name)).toEqual(['Small']);
  });

  it('says why a format-1 token that names the floor is refused', () => {
    expect(issues(readTheme(v1({ danger: '#ff0000', accent: '#8b93ff' })))).toEqual([{ code: 'floor', field: 'tokens.danger', item: 'danger' }]);
  });
});

describe('format 2', () => {
  it('reads both schemes, inherits, regions, the terminal, syntax, chrome and text', () => {
    const r = readTheme(harbour());
    expect(r.ok, JSON.stringify(issues(r))).toBe(true);
    if (!r.ok) return;
    const slate = BUILT_IN.find((b) => b.id === 'slate') as Theme;
    const { looks, chrome, text } = r.theme;
    // Dark comes from Slate (inherited), Light from the default Light; the shape and fonts are shared.
    expect(looks.dark?.tokens).toEqual({ ...slate.tokens, accent: '#5cc8c2', 'radius-m': '8px', 'font-mono': "'JetBrains Mono', monospace" });
    expect(looks.light?.tokens).toEqual({ ...baseTokens('light'), accent: '#0b6e69', 'on-accent': '#ffffff', 'radius-m': '8px', 'font-mono': "'JetBrains Mono', monospace" });
    expect(looks.dark?.regions).toEqual({ sidebar: { surface: '#0b1016', text: '#e6edf3' }, 'code-blocks': { line: '#2a3541' } });
    expect(looks.dark?.terminal).toEqual({ background: '#05080c', foreground: '#d8e1ea', red: '#ff6b6b', 'bright-black': '#5c6773' });
    expect(looks.dark?.syntax).toEqual({ keyword: '#c099ff' });
    expect(looks.light?.regions).toEqual({});
    expect(chrome).toEqual({ ...DEFAULT_CHROME, 'pane-gap': 6, 'tab-style': 'pill', density: 'compact', 'pane-borders': 'always', 'tab-bar': 'bottom', 'state-display': 'word', shadow: 'none', motion: 'reduced', 'line-width': 2 });
    expect(text).toEqual({ weight: 500, 'line-height': 1.5, ligatures: false });
    expect(r.theme.scheme).toBe('dark');
  });

  it('a theme can be a few lines: inherits alone gives that theme\'s scheme', () => {
    const r = readTheme(v2({ inherits: 'sand', chrome: { 'tab-style': 'filled' } }));
    expect(r.ok && Object.keys(r.theme.looks)).toEqual(['light']);
    expect(r.ok && r.theme.tokens).toEqual(BUILT_IN.find((b) => b.id === 'sand')?.tokens);
  });

  it('exports format 2, and an exported file read back is identical', () => {
    const r = readTheme(harbour());
    if (!r.ok) throw new Error('harbour');
    const out = serialise(writeTheme(r.theme, 'Harbour'));
    const back = deserialise(out);
    expect(back.ok && back.theme).toEqual(r.theme);
    if (back.ok) expect(serialise(writeTheme(back.theme, 'Harbour'))).toBe(out);
    // Everything is written out (not `inherits`), so the file reads the same where the built-in themes differ.
    const file = JSON.parse(out);
    expect(file.inherits).toBeUndefined();
    expect(Object.keys(file.chrome)).toEqual(CHROME_NAMES);
    expect(Object.keys(file.dark.tokens)).toHaveLength(13);
    // Every built-in theme exported as format 2 reads back.
    for (const b of BUILT_IN) expect(readTheme(writeTheme(b, 'x')).ok, b.id).toBe(true);
  });

  it('refuses a newer format, an unknown field at every level, and bad values, listing every problem at once', () => {
    expect(codes(readTheme({ 'agora-theme': 3, name: 'x' }))).toEqual(['newer']);
    expect(codes(readTheme(v2()))).toEqual(['no-look']);
    const r = readTheme(v2({
      wallpaper: 'x', inherits: 'nope',
      dark: { tokens: { 'text-16': '18px', 'surface-9': '#000000', accent: 'blue', 'text-14': '15px' }, colours: {}, regions: { header: {}, sidebar: { glow: '#ffffff', text: 'white' } }, terminal: { pink: '#ff00ff', red: 12 }, syntax: { tag: '#ffffff' } },
      light: 'yes',
      shape: { 'font-ui': 'Inter', 'space-1': '9px', 'text-12': '12px', 'radius-q': 1 },
      text: { weight: 450, 'line-height': 3, ligatures: 'no', 'text-16': '16px', kerning: true },
      chrome: { 'pane-gap': 17, 'line-width': 1.5, 'tab-style': 'tabs', 'tab-bar': 'left', glow: 1, icons: 'emoji', motion: 'OFF' },
    }));
    expect(issues(r)).toEqual(expect.arrayContaining([
      { code: 'unknown-field', field: 'wallpaper' },
      { code: 'inherits', id: 'nope' },
      { code: 'bad-field', field: 'light', expect: 'group' },
      { code: 'moved-token', field: 'shape.font-ui', token: 'font-ui', to: 'text' },
      { code: 'bad-value', token: 'space-1' },
      { code: 'derived-token', token: 'text-12', base: 'text-16' },
      { code: 'unknown-field', field: 'shape.radius-q' },
      { code: 'bad-field', field: 'text.weight', expect: 'list', values: '300, 400, 500' },
      { code: 'bad-field', field: 'text.line-height', expect: 'line-height', min: 1.2, max: 1.8 },
      { code: 'bad-field', field: 'text.ligatures', expect: 'switch' },
      { code: 'moved-token', field: 'text.text-16', token: 'text-16', to: 'shape' },
      { code: 'unknown-field', field: 'text.kerning' },
      { code: 'bad-field', field: 'chrome.pane-gap', expect: 'size', min: 0, max: 16 },
      { code: 'bad-field', field: 'chrome.line-width', expect: 'size', min: 0, max: 2 },
      { code: 'bad-field', field: 'chrome.tab-style', expect: 'list', values: 'underline, filled, pill' },
      { code: 'bad-field', field: 'chrome.tab-bar', expect: 'list', values: 'top, bottom' },
      { code: 'unknown-field', field: 'chrome.glow' },
      { code: 'bad-field', field: 'chrome.icons', expect: 'list', values: 'lines' },
      { code: 'bad-field', field: 'chrome.motion', expect: 'list', values: 'system, reduced, off' },
      { code: 'unknown-field', field: 'dark.colours' },
      { code: 'moved-token', field: 'dark.tokens.text-16', token: 'text-16', to: 'shape' },
      { code: 'unknown-token', token: 'surface-9' },
      { code: 'bad-value', token: 'accent' },
      { code: 'derived-token', token: 'text-14', base: 'text-16' },
      { code: 'unknown-field', field: 'dark.regions.header' },
      { code: 'unknown-field', field: 'dark.regions.sidebar.glow' },
      { code: 'bad-field', field: 'dark.regions.sidebar.text', expect: 'color' },
      { code: 'unknown-field', field: 'dark.terminal.pink' },
      { code: 'bad-field', field: 'dark.terminal.red', expect: 'color' },
      { code: 'unknown-field', field: 'dark.syntax.tag' },
    ]));
    expect(issues(r)).toHaveLength(30);
  });

  it('refuses a value with a piece that could become CSS or reach outside the page, wherever it is', () => {
    const pieces = ['url(', 'URL(', '@import', ';', '}', 'expression(', '<'];
    for (const piece of pieces) {
      for (const [where, file] of [
        ['name', v2({ name: `Night ${piece}`, inherits: 'dark' })],
        ['text.font-ui', v2({ inherits: 'dark', text: { 'font-ui': `Inter${piece}` } })],
        ['chrome.tab-style', v2({ inherits: 'dark', chrome: { 'tab-style': `pill${piece}` } })],
        ['dark.regions.cards.surface', v2({ dark: { regions: { cards: { surface: `#111111${piece}` } } } })],
        ['dark.terminal.red', v2({ dark: { terminal: { red: `${piece}x` } } })],
        ['inherits', v2({ inherits: `dark${piece}` })],
      ] as const) {
        const r = readTheme(file);
        expect(r.ok, `${piece} in ${where}`).toBe(false);
        expect(issues(r), `${piece} in ${where}`).toContainEqual({ code: 'unsafe', field: where });
      }
    }
    // A value that is refused as unsafe is not also judged as a bad value.
    expect(issues(readTheme(v2({ dark: { terminal: { red: 'url(x)' } } })))).toEqual([{ code: 'unsafe', field: 'dark.terminal.red' }]);
  });

  it('refuses a theme that sets what no theme can change, saying why', () => {
    const cases: [Record<string, unknown>, string, string][] = [
      [{ dark: { tokens: { danger: '#ff0000' } } }, 'dark.tokens.danger', 'danger'],
      [{ dark: { tokens: { 'diff-removed': '#ff0000' } } }, 'dark.tokens.diff-removed', 'diff-removed'],
      [{ dark: { tokens: { 'on-danger': '#000000' } } }, 'dark.tokens.on-danger', 'danger'],
      [{ dark: { regions: { 'approval-card': { surface: '#000000' } } } }, 'dark.regions.approval-card', 'approval-card'],
      [{ inherits: 'dark', chrome: { 'focus-ring': 'off' } }, 'chrome.focus-ring', 'focus-ring'],
      [{ dark: { regions: { 'needs-you': { surface: '#000000' } } } }, 'dark.regions.needs-you', 'needs-you'],
      [{ inherits: 'dark', chrome: { 'needs-you': 'hidden' } }, 'chrome.needs-you', 'needs-you'],
      [{ inherits: 'dark', 'plugin-marker': false }, 'plugin-marker', 'plugin-marker'],
      [{ dark: { approval: {} } }, 'dark.approval', 'approval-card'],
      [{ dark: { regions: { cards: { 'focus-ring': '#000000' } } } }, 'dark.regions.cards.focus-ring', 'focus-ring'],
    ];
    for (const [extra, field, item] of cases) expect(issues(readTheme(v2(extra))), field).toContainEqual({ code: 'floor', field, item });
    // The floor is matched as a whole name: the colour of the "needs you" state and the accent are themeable as in format 1.
    expect(readTheme(v2({ dark: { tokens: { 'state-needs-you': '#ffb000', accent: '#ff9f43' } } })).ok).toBe(true);
    expect(Object.values(FLOOR)).toEqual(expect.arrayContaining(['danger', 'diff-removed', 'approval-card', 'focus-ring', 'needs-you', 'plugin-marker']));
    // No text under 12 px: in format 2 the base size is at least 16 px.
    expect(issues(readTheme(v2({ inherits: 'dark', shape: { 'text-16': 15 } })))).toEqual([{ code: 'text-floor', field: 'shape.text-16', min: 16 }]);
    expect(readTheme(v2({ inherits: 'dark', shape: { 'text-16': 16 } })).ok).toBe(true);
  });
});

describe('readability of format 2', () => {
  it('refuses a region whose text is hard to read on its surface, and states, secondary text and the accent on a surface of its own', () => {
    const r = readTheme(v2({ dark: { regions: { 'status-line': { text: '#26262c' } } } }));
    expect(issues(r)).toEqual([{ code: 'region-contrast', region: 'status-line', fg: 'region-text', ratio: contrast('#26262c', baseTokens('dark')['surface-1']), min: 4.5 }]);
    const light = readTheme(v2({ dark: { regions: { sidebar: { surface: '#d0d0d0', text: '#000000' } } } }));
    const fgs = issues(light).map((i) => (i.code === 'region-contrast' ? `${i.fg}:${i.min}` : i.code));
    expect(fgs).toEqual(expect.arrayContaining(['text-muted:4.5', 'accent:4.5', 'state-working:3', 'state-finished:3', 'state-needs-you:3']));
    expect(fgs).not.toContain('region-text:4.5');
    // Every region's pair is asked.
    for (const region of REGIONS) {
      const p = readTheme(v2({ dark: { regions: { [region]: { surface: '#ffffff', text: '#fefefe' } } } }));
      expect(issues(p), region).toContainEqual(expect.objectContaining({ code: 'region-contrast', region, fg: 'region-text' }));
    }
  });

  it('refuses a terminal whose text is hard to read, and only warns about the 16 colours and the cursor', () => {
    expect(codes(readTheme(v2({ dark: { terminal: { background: '#202020', foreground: '#505050' } } })))).toEqual(['terminal-contrast']);
    const r = readTheme(v2({ dark: { terminal: { blue: '#0a0a30', cursor: '#101012', black: '#000000' } } }));
    expect(r.ok).toBe(true);
    expect(r.ok && r.warnings?.map((w) => (w.code === 'ansi-contrast' ? w.colour : w.code))).toEqual(['blue', 'cursor-contrast']);
    // The window's own palette is never warned about on the window's own background.
    for (const b of BUILT_IN) expect(judge(b).warnings, b.id).toEqual([]);
  });

  it('warns about a syntax colour that is hard to read on the code blocks', () => {
    const r = readTheme(v2({ light: { syntax: { comment: '#e0e0e0' } } }));
    expect(r.ok && r.warnings).toEqual([{ code: 'syntax-contrast', key: 'comment', ratio: contrast('#e0e0e0', '#ffffff'), min: 4.5 }]);
  });

  it('says which scheme a problem is in when the theme has both', () => {
    const r = readTheme(v2({ dark: { tokens: { text: '#202020' } }, light: {} }));
    expect(issues(r).every((i) => i.scheme === 'dark')).toBe(true);
    expect(issues(readTheme(v2({ dark: { tokens: { text: '#202020' } } }))).every((i) => i.scheme === undefined)).toBe(true);
  });
});

describe('applying a format-2 theme', () => {
  const read = () => { const r = readTheme(harbour()); if (!r.ok) throw new Error('harbour'); return { id: 'user-1', ...r.theme } as Theme; };

  it('maps chrome values to data attributes and variables, and a default theme removes them all', () => {
    const { vars, root } = page();
    applyTheme(root, read(), 'dark');
    expect(root.dataset).toMatchObject({ theme: 'dark', paneBorders: 'always', tabBar: 'bottom', tabStyle: 'pill', density: 'compact', stateDisplay: 'word', shadow: 'none', motion: 'reduced', paneGaps: 'on', ligatures: 'off', lineHeight: 'set' });
    expect(root.dataset.icons).toBeUndefined(); // the only icon set is the default
    expect([vars.get('--pane-gap'), vars.get('--line-width'), vars.get('--ui-weight'), vars.get('--ui-line-height')]).toEqual(['6px', '2px', '500', '1.5']);
    applyTheme(root, BUILT_IN[0] as Theme, 'dark');
    expect(vars.size).toBe(0);
    expect(Object.keys(root.dataset)).toEqual(['theme']);
  });

  it('every list value becomes its attribute, written from the list itself', () => {
    for (const n of CHROME_NAMES) {
      const kind = CHROME[n];
      if (kind.kind !== 'list') continue;
      for (const value of kind.values) {
        const r = readTheme(v2({ inherits: 'dark', chrome: { [n]: value } }));
        if (!r.ok) throw new Error(n);
        const { root } = page();
        applyTheme(root, { id: 'user-1', ...r.theme }, 'dark');
        const key = n.replace(/-([a-z])/g, (_, c: string) => c.toUpperCase());
        expect(root.dataset[key], `${n}=${value}`).toBe(value === kind.default ? undefined : value);
      }
    }
  });

  it('sets the regions, the terminal palette and syntax colours it has, and removes those the next theme lacks', () => {
    const { vars, root } = page();
    applyTheme(root, read(), 'dark');
    expect(vars.get(regionVar('sidebar', 'surface'))).toBe('#0b1016');
    expect(vars.get(regionVar('code-blocks', 'line'))).toBe('#2a3541');
    expect(vars.has(regionVar('cards', 'surface'))).toBe(false);
    expect(vars.get(terminalVar('background'))).toBe('#05080c');
    expect(vars.get('--syntax-keyword')).toBe('#c099ff');
    applyTheme(root, read(), 'light');
    expect(root.dataset.theme).toBe('light');
    expect(vars.get('--accent')).toBe('#0b6e69');
    expect([...vars.keys()].filter((k) => k.startsWith('--terminal-') || k.startsWith('--sidebar-') || k.startsWith('--syntax-'))).toEqual([]);
    expect(REGIONS.length * REGION_PARTS.length).toBe(30);
  });

  it('the terminal draws with the theme\'s palette, recomputed from the page, and exactly as before without one', () => {
    const { vars, root } = page();
    const readVar = (n: string) => vars.get(n) ?? ({ '--surface-0': '#09090b', '--text': '#f4f4f5', '--accent': '#8b93ff' } as Record<string, string>)[n] ?? '';
    // No theme: the pane's own colours and the scheme's palette, the accent as the selection (as before this format).
    const before = terminalTheme(readVar, false);
    expect(before).toMatchObject({ background: '#09090b', foreground: '#f4f4f5', cursor: '#8b93ff', selectionBackground: '#8b93ff66', selectionInactiveBackground: '#8b93ff33', black: ANSI.dark[0], brightWhite: ANSI.dark[15] });
    expect(terminalTheme(readVar, true).red).toBe(ANSI.light[1]);
    applyTheme(root, read(), 'dark');
    const after = terminalTheme(readVar, false);
    expect(after).toMatchObject({ background: '#05080c', foreground: '#d8e1ea', red: '#ff6b6b', brightBlack: '#5c6773', green: ANSI.dark[2] });
    // A theme change is a new reading of the page: the light look has no palette, so the defaults come back.
    applyTheme(root, read(), 'light');
    expect(terminalTheme(readVar, true)).toMatchObject({ red: ANSI.light[1], cursor: '#0b6e69' });
    expect(Object.keys(after).filter((k) => !['background', 'foreground', 'cursor', 'selectionBackground', 'selectionInactiveBackground'].includes(k))).toHaveLength(ANSI_NAMES.length);
  });

  it('sets no style property and no attribute for a refused theme', () => {
    const { vars, root } = page();
    const s = createThemeStore({ kv: memory(), root });
    s.apply();
    const attrs = { ...root.dataset };
    for (const bad of [
      v2({ inherits: 'dark', chrome: { 'tab-style': 'pill;}' } }),
      v2({ dark: { tokens: { danger: '#000000' } } }),
      v2({ dark: { regions: { sidebar: { text: '#111114' } } } }),
      v2({ inherits: 'dark', shape: { 'text-16': 12 } }),
    ]) {
      expect(s.import(JSON.stringify(bad)).ok).toBe(false);
      expect(vars.size).toBe(0);
      expect(root.dataset).toEqual(attrs);
    }
    // An edit that is refused leaves the page as it was.
    expect(s.import(JSON.stringify(harbour())).ok).toBe(true);
    const shown = new Map(vars);
    const id = s.active().id;
    expect(s.update(id, { file: { ...writeTheme(s.active(), 'x'), chrome: { motion: 'url(x)' } } }).ok).toBe(false);
    expect(vars).toEqual(shown);
  });
});

describe('the store with both schemes', () => {
  it('a theme with both schemes fills both slots, and copies keep everything', () => {
    let light = false;
    const s = createThemeStore({ kv: memory(), prefersLight: () => light });
    const r = s.import(JSON.stringify(harbour()));
    expect(r.ok).toBe(true);
    expect(s.state().choice).toEqual({ dark: 'user-1', light: 'user-1' });
    expect(s.active().id).toBe('user-1');
    light = true;
    expect(s.active().id).toBe('user-1');
    const copy = s.copy(s.active(), 'Harbour (mine)');
    expect(copy.ok && copy.theme.looks).toEqual(cloneLooks(s.byId('user-1')?.looks ?? {}));
    expect(copy.ok && copy.theme.chrome).toEqual(s.byId('user-1')?.chrome);
    // A forced scheme the theme lacks moves to the one it has.
    s.setMode('light');
    s.choose('slate');
    expect(s.state().mode).toBe('dark');
  });
});

describe('format 2 in words and in the stylesheet', () => {
  afterEach(() => setLanguage('en'));
  for (const lang of ['en', 'tr']) {
    it(`says every new problem, region, terminal colour, chrome value and floor reason in ${lang}`, () => {
      setLanguage(lang);
      const all: ThemeIssue[] = [
        { code: 'no-look' }, { code: 'moved-token', field: 'f', token: 'text-16', to: 'shape' }, { code: 'unsafe', field: 'f' }, { code: 'inherits', id: 'x' },
        ...(['color', 'size', 'list', 'font', 'line-height', 'switch', 'group'] as const).map((expect): ThemeIssue => ({ code: 'bad-field', field: 'f', expect, min: 0, max: 2, values: 'a, b' })),
        ...FLOOR_ITEMS.map((item): ThemeIssue => ({ code: 'floor', field: 'f', item })),
        { code: 'text-floor', field: 'shape.text-16', min: 16 }, { code: 'text-lifted', from: '12px', to: '16px' },
        ...REGIONS.map((region): ThemeIssue => ({ code: 'region-contrast', region, fg: 'region-text', ratio: 2, min: 4.5 })),
        { code: 'region-contrast', region: 'cards', fg: 'state-idle', ratio: 2, min: 3, scheme: 'light' },
        { code: 'terminal-contrast', ratio: 2, min: 4.5 }, { code: 'cursor-contrast', ratio: 2, min: 3 },
        ...ANSI_NAMES.map((colour): ThemeIssue => ({ code: 'ansi-contrast', colour, ratio: 2, min: 3 })),
        ...SYNTAX_KEYS.map((key): ThemeIssue => ({ code: 'syntax-contrast', key, ratio: 2, min: 4.5 })),
      ];
      for (const i of all) expect(issueText(i), JSON.stringify(i)).not.toMatch(/theme-|\{\$|Unknown variable/);
      for (const n of CHROME_NAMES) {
        expect(chromeLabel(n)).not.toMatch(/^theme-/);
        const kind = CHROME[n];
        if (kind.kind === 'list') for (const v of kind.values) expect(chromeValueLabel(n, v), `${n}-${v}`).not.toMatch(/^theme-/);
      }
      for (const f of FLOOR_ITEMS) expect([floorLabel(f), floorReason(f)].join()).not.toMatch(/theme-/);
      for (const k of ['theme-regions', 'theme-terminal', 'theme-syntax', 'theme-chrome', 'theme-floor', 'theme-floor-hint', 'theme-preview', 'theme-locked', 'theme-part-surface', 'theme-text-weight-300', 'theme-text-line-height-normal']) expect(t(k), k).not.toBe(k);
      expect(issueText({ code: 'contrast', fg: 'text', bg: 'surface-0', ratio: 3, min: 4.5, scheme: 'dark' })).toMatch(new RegExp(`^${t('theme-dark')}: `));
    });
  }

  it('keeps what needs the person out of every region: the floor resets their variables to the base tokens', () => {
    const css = readFileSync('src/design/app.css', 'utf8').replace(/\r\n/g, '\n');
    const i = css.indexOf(':is(.kit-callout, .approval, .approval-req, .attention-card, .tile--needs-you, .kit-tone-needs) {');
    expect(i).toBeGreaterThan(0);
    const floor = css.slice(i, css.indexOf('}', i));
    for (const r of ['cards', 'inputs', 'buttons', 'code-blocks'] as const) for (const p of REGION_PARTS) expect(floor, regionVar(r, p)).toContain(`${regionVar(r, p)}: var(--`);
    // Every region variable is read with today's token as its fallback, so nothing changes without a theme.
    for (const r of REGIONS) for (const p of REGION_PARTS) expect(css, regionVar(r, p)).toMatch(new RegExp(`var\\(${regionVar(r, p)}, (var\\(--[a-z0-9-]+\\)|transparent|inherit)\\)`));
    // "Needs you" keeps its own symbol whatever a theme's state display says.
    expect(css).toContain(':is(.state, .kit-mark):not(.state--needs-you, .kit-tone-needs)');
  });
});
