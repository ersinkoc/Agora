// @vitest-environment jsdom
// Theme import (docs/ux/themes.md "Import from another app"): one small fixture per app, written for these tests from a palette of our
// own (no third-party theme is copied), each held to a golden format-2 file and its notes; the result goes through the same theme
// check as a hand-written file, so the floor still holds; the size cap; and hostile input (random and mutated text, deep nesting,
// entity and alias bombs, prototype keys), which never throws, never hangs and never makes a file the theme check would not read.
import { readFileSync } from 'node:fs';
import { beforeEach, describe, expect, it } from 'vitest';
import { setLanguage, t } from '../../i18n';
import { MAX_FILE_BYTES, readTheme, serialise } from '../file';
import { ANSI_NAMES, FLOOR, SYNTAX_KEYS, TERMINAL_KEYS } from '../look';
import { importErrorText, importNoteText } from '../messages';
import { createThemeStore } from '../store';
import { COLOR_TOKENS, contrast } from '../tokens';
import {
  type Converted, type ImportError, type ImportNote, IMPORT_FORMATS, convertTheme, deriveTokens, detectFormat, fromAlacritty, fromGhostty,
  fromITerm2, fromVsCode, fromWindowsTerminal, hexOf, importName, mix, parseHex,
} from './convert';
import { readJsonc, readToml, readYaml } from './read';

const dir = 'src/themes/import/fixtures';
const fixture = (f: string) => readFileSync(`${dir}/${f}`, 'utf8');
// Each fixture, the name the browser gives its file, and the app it is from.
const FIXTURES = [
  ['harbour.itermcolors', 'harbour.itermcolors', 'iterm2'],
  ['harbour.toml', 'harbour.toml', 'alacritty'],
  ['harbour.yml', 'alacritty.yml', 'alacritty'],
  ['harbour-light', 'Harbour Light', 'ghostty'],
  ['harbour-settings.json', 'settings.json', 'windows-terminal'],
  ['harbour-color-theme.json', 'harbour-color-theme.json', 'vscode'],
] as const;

beforeEach(() => setLanguage('en'));

describe('each app\'s colour file becomes a format-2 theme', () => {
  for (const [f, name, format] of FIXTURES) {
    it(`${format}: ${f} matches its golden file and is accepted by the theme check`, async () => {
      const r = convertTheme(fixture(f), name, 'Imported theme');
      expect(r.ok).toBe(true);
      if (!r.ok) return;
      expect(r.format).toBe(format);
      await expect(`${JSON.stringify({ format: r.format, scheme: r.scheme, name: r.name, file: r.file, notes: r.notes }, null, 2)}\n`)
        .toMatchFileSnapshot(`fixtures/${f}.golden.json`);
      const judged = readTheme(r.file);
      expect(judged.ok, JSON.stringify(judged)).toBe(true);
      // Only what format 2 knows, built from closed lists: the scheme's tokens, terminal and syntax colours, nothing of the file's own.
      expect(Object.keys(r.file).sort()).toEqual(['agora-theme', 'name', r.scheme].sort());
      const section = r.file[r.scheme] as Record<string, Record<string, string>>;
      for (const k of Object.keys(section)) expect(['tokens', 'terminal', 'syntax']).toContain(k);
      for (const k of Object.keys(section.tokens ?? {})) expect(COLOR_TOKENS as readonly string[]).toContain(k);
      for (const k of Object.keys(section.terminal ?? {})) expect(TERMINAL_KEYS as readonly string[]).toContain(k);
      for (const k of Object.keys(section.syntax ?? {})) expect(SYNTAX_KEYS as readonly string[]).toContain(k);
      // Every note is a sentence in both languages.
      for (const lang of ['en', 'tr']) {
        setLanguage(lang);
        for (const n of r.notes) { const s = importNoteText(n); expect(s, JSON.stringify(n)).not.toMatch(/theme-import|\{\s*\$/); }
      }
    });
  }

  it('fills the scheme of its background only, and says so', () => {
    const dark = convertTheme(fixture('harbour.toml'), 'harbour.toml') as Extract<Converted, { ok: true }>;
    const light = convertTheme(fixture('harbour-light'), 'Harbour Light') as Extract<Converted, { ok: true }>;
    expect([dark.scheme, light.scheme]).toEqual(['dark', 'light']);
    expect(dark.file.light).toBeUndefined();
    expect(light.file.dark).toBeUndefined();
    expect(dark.notes).toContainEqual({ code: 'scheme', scheme: 'dark' });
    expect(light.notes).toContainEqual({ code: 'scheme', scheme: 'light' });
    // The store adds it as the theme of that scheme only.
    const store = createThemeStore();
    const saved = store.import(serialise(light.file));
    expect(saved.ok).toBe(true);
    expect(store.state().choice).toEqual({ dark: 'dark', light: saved.ok ? saved.theme.id : '' });
  });

  it('reads iTerm2\'s float components, Display P3 and alpha', () => {
    const plist = (entries: string) => `<?xml version="1.0"?><plist version="1.0"><dict>${entries}</dict></plist>`;
    const colour = (key: string, r: number, g: number, b: number, extra = '') =>
      `<key>${key}</key><dict><key>Red Component</key><real>${r}</real><key>Green Component</key><real>${g}</real><key>Blue Component</key><real>${b}</real>${extra}</dict>`;
    const r = fromITerm2(plist(
      colour('Background Color', 0, 0, 0) + colour('Foreground Color', 1, 1, 1)
      + colour('Ansi 1 Color', 1, 0, 0, '<key>Color Space</key><string>P3</string>')
      + colour('Ansi 2 Color', 0, 1, 0, '<key>Alpha Component</key><real>0.5</real>')
      + colour('Ansi 3 Color', 2, -1, Number.NaN),
    ), 'x.itermcolors');
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    const term = (r.file.dark as { terminal: Record<string, string> }).terminal;
    expect(term.background).toBe('#000000');
    expect(term.red).toBe('#ff0000'); // P3's red is outside sRGB: clamped to sRGB's
    expect(term.green).toBe('#008000'); // half of green over black
    expect(term.yellow).toBeUndefined(); // NaN is not a number: the colour could not be read
    expect(r.notes).toContainEqual({ code: 'unreadable', keys: ['Ansi 3 Color'], more: 0 });
    expect(r.notes).toContainEqual({ code: 'colour-space', space: 'p3' });
    expect(r.notes).toContainEqual({ code: 'alpha', keys: ['green'], more: 0 });
    // A P3 colour inside sRGB lands on its sRGB value.
    const mid = fromITerm2(plist(colour('Background Color', 0.4, 0.4, 0.4, '<key>Color Space</key><string>P3</string>')), 'x') as Extract<Converted, { ok: true }>;
    expect((mid.file.dark as { terminal: Record<string, string> }).terminal.background).toBe('#666666');
    // The dark set of a file with both, when it has no plain keys.
    const variants = fromITerm2(plist(colour('Background Color (Dark)', 0, 0, 0) + colour('Background Color (Light)', 1, 1, 1)), 'x') as Extract<Converted, { ok: true }>;
    expect(variants.scheme).toBe('dark');
    expect(variants.notes).toContainEqual({ code: 'variant', used: 'dark' });
    expect(variants.notes).toContainEqual({ code: 'ignored', keys: ['Background Color (Light)'], more: 0 });
  });

  it('reads the colour notations of each app and refuses the rest', () => {
    expect(parseHex('#abc')).toEqual({ r: 0xaa, g: 0xbb, b: 0xcc, a: 1 });
    expect(parseHex('#11223380')?.a).toBeCloseTo(0x80 / 255);
    expect(parseHex('0x112233')).toEqual({ r: 0x11, g: 0x22, b: 0x33, a: 1 });
    expect(parseHex('112233')).toBeUndefined();
    expect(parseHex('112233', true)).toEqual({ r: 0x11, g: 0x22, b: 0x33, a: 1 });
    for (const bad of ['red', 'rgb(1,2,3)', '#12345', '0x123', '#1234567', '', 12, null]) expect(parseHex(bad, true), String(bad)).toBeUndefined();
    expect(mix('#000000', '#ffffff', 0.5)).toBe('#808080');
    expect(hexOf({ r: 300, g: -5, b: 127.6 })).toBe('#ff0080');
  });

  it('takes a VS Code terminal colour from the editor when the theme has none, and code colours from its token colours', () => {
    const r = fromVsCode('{"colors":{"editor.background":"#1e1e1e","editor.foreground":"#d4d4d4","terminal.ansiRed":"#f44747"},"tokenColors":[{"scope":"comment.line","settings":{"foreground":"#6a9955"}}]}', 'v.json');
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    const dark = r.file.dark as { terminal: Record<string, string>; syntax: Record<string, string> };
    expect(dark.terminal).toMatchObject({ background: '#1e1e1e', foreground: '#d4d4d4', red: '#f44747' });
    expect(dark.syntax).toEqual({ comment: '#6a9955' }); // `comment.line` starts with `comment.`
    expect(r.notes).toContainEqual({ code: 'fallback', key: 'background', from: 'editor.background' });
    // `include` and a token colours path name other files: never read, named as left out.
    const inc = fromVsCode('{"include":"./base.json","tokenColors":"./tokens.tmTheme","colors":{"terminal.background":"#000000"}}', 'v.json') as Extract<Converted, { ok: true }>;
    expect(inc.notes).toContainEqual({ code: 'ignored', keys: ['tokenColors'], more: 0 });
    expect(inc.notes).toContainEqual({ code: 'include' });
    const many = fromVsCode(JSON.stringify({ colors: { 'terminal.background': '#000000', 'editor.background': '#111111', 'sideBar.background': '#222222', danger: '#ff0000' } }), 'v.json') as Extract<Converted, { ok: true }>;
    expect(many.notes).toContainEqual({ code: 'window-colours', count: 3 });
  });

  it('reads one Windows Terminal scheme, a list of them or a settings file with them', () => {
    const one = '{"name":"One","background":"#101010","foreground":"#eeeeee","purple":"#aa55ff","brightPurple":"#cc88ff"}';
    for (const text of [one, `[${one}]`, `{"schemes":[${one}]}`]) {
      const r = fromWindowsTerminal(text, 'x.json') as Extract<Converted, { ok: true }>;
      expect(r.name).toBe('One');
      expect((r.file.dark as { terminal: Record<string, string> }).terminal).toMatchObject({ magenta: '#aa55ff', 'bright-magenta': '#cc88ff' });
    }
  });

  it('reads Ghostty\'s palette lines, bare hex and cell colours', () => {
    const r = fromGhostty('background = 101010\nforeground = "#eeeeee"\ncursor-color = cell-foreground\npalette = 9 = #ff0000\npalette = 300=#000000\npalette = 3=goldenrod\n', 'g') as Extract<Converted, { ok: true }>;
    expect((r.file.dark as { terminal: Record<string, string> }).terminal).toMatchObject({ background: '#101010', foreground: '#eeeeee', cursor: '#eeeeee', 'bright-red': '#ff0000' });
    expect(r.notes).toContainEqual({ code: 'unreadable', keys: ['palette = 300=#000000', 'palette = 3'], more: 0 });
  });

  it('tells the apps apart by extension, else by content', () => {
    expect(detectFormat('', 'a.itermcolors')).toBe('iterm2');
    expect(detectFormat('', 'a.TOML')).toBe('alacritty');
    expect(detectFormat('', 'a.yaml')).toBe('alacritty-yaml');
    expect(detectFormat('', 'a.json')).toBe('json');
    expect(detectFormat('<?xml version="1.0"?>', 'x')).toBe('iterm2');
    expect(detectFormat('// c\n{"a":1}', 'x')).toBe('json');
    expect(detectFormat('[colors.primary]\nbackground = "#000000"', 'x')).toBe('alacritty');
    expect(detectFormat('colors.primary.background = "#000000"', 'x')).toBe('alacritty');
    expect(detectFormat('colors:\n  primary:\n', 'x')).toBe('alacritty-yaml');
    expect(detectFormat('# a theme\npalette = 0=#000000', 'x')).toBe('ghostty');
    expect(detectFormat('hello', 'x')).toBeUndefined();
    expect(convertTheme('hello', 'notes.txt')).toEqual({ ok: false, error: 'unknown-format' });
    expect(convertTheme('{"unrelated": true}', 'x.json')).toEqual({ ok: false, error: 'no-colours', format: 'windows-terminal' });
    expect(convertTheme('{ not json', 'x.json')).toEqual({ ok: false, error: 'unreadable' });
  });
});

describe('the floor and the theme check hold after import', () => {
  it('never copies a key of the file, so a floor name in a file is left out, not themed', () => {
    const floorNames = Object.keys(FLOOR);
    const colors = Object.fromEntries(floorNames.map((k) => [k, '#ff0000']));
    const vs = convertTheme(JSON.stringify({ name: 'x', colors: { ...colors, 'terminal.background': '#000000', 'terminal.foreground': '#ffffff' } }), 'x.json') as Extract<Converted, { ok: true }>;
    const wt = convertTheme(JSON.stringify({ name: 'x', background: '#000000', foreground: '#ffffff', ...colors }), 'x.json') as Extract<Converted, { ok: true }>;
    const toml = convertTheme(`[colors.primary]\nbackground = "#000000"\nforeground = "#ffffff"\n[colors.danger]\nfocus = "#ff0000"\n`, 'x.toml') as Extract<Converted, { ok: true }>;
    for (const r of [vs, wt, toml]) {
      const text = JSON.stringify(r.file);
      for (const k of floorNames) expect(text, k).not.toContain(`"${k}"`);
      expect(readTheme(r.file).ok).toBe(true);
      expect(r.notes.some((n) => n.code === 'ignored' || n.code === 'window-colours')).toBe(true);
    }
  });

  it('goes through the same door as a theme file: a converted file with a floor key or an unsafe value is refused', () => {
    const r = convertTheme(fixture('harbour.toml'), 'harbour.toml') as Extract<Converted, { ok: true }>;
    const dark = r.file.dark as { tokens: Record<string, string> };
    const store = createThemeStore();
    expect(store.import(serialise({ ...r.file, dark: { ...dark, tokens: { ...dark.tokens, danger: '#00ff00' } } }))).toMatchObject({ ok: false, issues: [{ code: 'floor', item: 'danger' }] });
    expect(store.import(serialise({ ...r.file, name: 'a;b' }))).toMatchObject({ ok: false, issues: [{ code: 'unsafe', field: 'name' }] });
    expect(store.state().user).toHaveLength(0);
    expect(store.import(serialise(r.file)).ok).toBe(true);
  });

  it('cleans a name from the file: control and direction characters and the pieces a theme refuses are removed, and it is cut short', () => {
    expect(importName('Night\u202e\u0007 <script>; url(x) }', 'f.json', 'fallback')).toBe('Night script> x)');
    expect(importName('', 'C:\\themes\\Deep Sea.itermcolors', 'fallback')).toBe('Deep Sea');
    expect(importName(';;;', '<<<', 'Imported theme')).toBe('Imported theme');
    expect(importName('x'.repeat(200), '', '')).toHaveLength(60);
    expect(importName(42, '', '')).toBe('—');
    // Review of audit F2000: cleaning repeats until nothing is left to remove, so a long name is cut first (one removal per pass here).
    const nested = 'u'.repeat(65_000) + 'rl('.repeat(65_000);
    const t0 = performance.now();
    expect(importName(nested, '', '')).toBe('u'.repeat(60));
    expect(performance.now() - t0).toBeLessThan(1000);
    const r = convertTheme(JSON.stringify({ name: '<img src=x onerror=alert(1)>;', background: '#000000', foreground: '#ffffff' }), 'x.json') as Extract<Converted, { ok: true }>;
    expect(readTheme(r.file).ok).toBe(true);
    expect(r.name).not.toMatch(/[<;]/);
  });

  it('a palette whose text is hard to read on its background is refused by the theme check, as a hand-written one is', () => {
    const r = convertTheme('background = 202020\nforeground = 303030\n', 'low') as Extract<Converted, { ok: true }>;
    expect(r.notes).toContainEqual({ code: 'derived-kept' });
    const judged = readTheme(r.file);
    expect(judged.ok).toBe(false);
    if (!judged.ok) expect(judged.issues.map((i) => i.code)).toContain('terminal-contrast');
  });

  it('makes window colours that pass the theme check from any palette whose text reads on its background (property)', () => {
    let seed = 7;
    const rnd = () => { seed = (seed * 1103515245 + 12345) % 2147483648; return seed / 2147483648; };
    const colour = () => hexOf({ r: rnd() * 255, g: rnd() * 255, b: rnd() * 255 });
    let tried = 0;
    // Random backgrounds, mid-greys among them (the hardest: both black and white text only just read there).
    const backgrounds = () => (tried % 5 === 0 ? hexOf({ r: 110 + rnd() * 20, g: 110 + rnd() * 20, b: 110 + rnd() * 20 }) : colour());
    while (tried < 400) {
      const bg = backgrounds();
      const fg = colour();
      if (contrast(fg, bg) < 4.5) continue;
      tried++;
      const palette = Object.fromEntries(ANSI_NAMES.map((n) => [n, colour()]));
      const tokens = deriveTokens({ ...palette, background: bg, foreground: fg } as Parameters<typeof deriveTokens>[0]);
      expect(tokens, `${bg} ${fg}`).toBeDefined();
      const text = JSON.stringify({ name: 'p', ...Object.fromEntries(ANSI_NAMES.map((n) => [n.replace(/^bright-(.)/, (_, c: string) => `bright${c.toUpperCase()}`).replace('magenta', 'purple').replace('Magenta', 'Purple'), palette[n]])), background: bg, foreground: fg });
      const r = convertTheme(text, 'p.json');
      expect(r.ok).toBe(true);
      if (!r.ok) continue;
      expect(r.notes, `${bg} ${fg}`).toContainEqual({ code: 'derived' });
      const judged = readTheme(r.file);
      expect(judged.ok, `${bg} ${fg} ${JSON.stringify(judged.ok ? '' : judged.issues)}`).toBe(true);
    }
  });
});

describe('bounded and hostile input', () => {
  it('refuses a file over the theme file cap before reading it', () => {
    const big = `{"name":"x","background":"#000000","foreground":"#ffffff","pad":"${'x'.repeat(MAX_FILE_BYTES)}"}`;
    expect(convertTheme(big, 'x.json')).toEqual({ ok: false, error: 'too-large' });
    // Counted in bytes, as the cap of a theme file is: 100 000 two-byte letters are over 256 KiB.
    expect(convertTheme(`# ${'ğ'.repeat(140_000)}`, 'g')).toEqual({ ok: false, error: 'too-large' });
  });

  it('refuses an XML entity or an internal DTD before parsing, and never loads an external one', () => {
    const bomb = '<?xml version="1.0"?><!DOCTYPE plist [<!ENTITY a "aaaaaaaaaa"><!ENTITY b "&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;">]><plist><dict><key>&b;</key><real>1</real></dict></plist>';
    expect(convertTheme(bomb, 'x.itermcolors')).toEqual({ ok: false, error: 'entities', format: 'iterm2' });
    expect(convertTheme('<?xml version="1.0"?><!DOCTYPE plist SYSTEM "file:///etc/passwd" [ ]><plist/>', 'x.itermcolors')).toMatchObject({ ok: false, error: 'entities' });
    expect(convertTheme('<plist><dict><key>Background Color</key><dict>', 'x.itermcolors')).toMatchObject({ ok: false, error: 'unreadable' });
  });

  it('keeps a key like __proto__ a plain key', () => {
    const toml = readToml('[__proto__]\npolluted = "yes"\n[constructor.prototype]\npolluted = "yes"\n__proto__.x = "1"\n');
    expect(toml.values.get('__proto__.polluted')).toBe('yes');
    const yaml = readYaml('__proto__:\n  polluted: yes\nconstructor:\n  prototype:\n    polluted: yes\n');
    expect(yaml.values.get('__proto__.polluted')).toBe('yes');
    readJsonc('{"__proto__":{"polluted":"yes"},"colors":{"__proto__":{"polluted":"yes"}}}');
    convertTheme('{"__proto__":{"polluted":"yes"},"colors":{"__proto__":{"polluted":"yes"},"terminal.background":"#000000"}}', 'x.json');
    convertTheme('__proto__ = 1\nbackground = 000000\n', 'g');
    expect(({} as Record<string, unknown>).polluted).toBeUndefined();
  });

  it('reads deep nesting, alias bombs and long lines in bounded time without throwing', () => {
    const cases: [string, string][] = [
      ['x.json', '['.repeat(200_000)],
      ['x.json', `${'{"a":'.repeat(40_000)}1${'}'.repeat(40_000)}`],
      ['x.itermcolors', `<plist>${'<dict><key>a</key>'.repeat(10_000)}</plist>`],
      ['x.itermcolors', `<plist><dict>${'<key>a</key><real>1</real>'.repeat(9_000)}</dict></plist>`],
      ['x.toml', `[colors.primary]\n${'a.'.repeat(100_000)}b = "#000000"`],
      ['x.toml', `x = [${'['.repeat(100_000)}`],
      ['x.toml', `x = """${'\n'.repeat(50_000)}`],
      ['x.yml', `a: &a\n  b: c\n${Array.from({ length: 2000 }, (_, i) => `k${i}: *a`).join('\n')}`],
      ['x.yml', `${'  '.repeat(50_000)}a: b`],
      ['g', `palette = ${'9'.repeat(200_000)}=#000000`],
      ['g', `${'#'.repeat(200_000)}`],
      ['x.json', `"${'\\'.repeat(200_001)}`],
      ['x.json', `{${',  '.repeat(80_000)}}`],
      ['x', `<!DOCTYPE${'<!DOCTYPE'.repeat(25_000)}`],
    ];
    for (const [name, text] of cases) {
      const start = performance.now();
      const r = convertTheme(text, name);
      expect(performance.now() - start, name + text.slice(0, 30)).toBeLessThan(2000);
      if (r.ok) expect(() => readTheme(r.file)).not.toThrow();
    }
  });

  it('never throws on random or mutated input, and any file it makes is one the theme check reads (fuzz)', () => {
    let seed = 42;
    const rnd = () => { seed = (seed * 1103515245 + 12345) % 2147483648; return seed / 2147483648; };
    const pieces = ['#', '0x', '=', ':', '[', ']', '{', '}', '"', "'", '\n', '  ', ',', '<', '>', '/', '*', '&', 'palette', 'colors', '<key>', '<dict>', '</dict>', '\\', '\u202e', '\u0000', '__proto__', 'ff', '1e309', 'NaN'];
    const mutate = (s: string) => {
      let out = s;
      const n = 1 + Math.floor(rnd() * 8);
      for (let i = 0; i < n; i++) {
        const at = Math.floor(rnd() * (out.length + 1));
        const op = rnd();
        if (op < 0.35) out = out.slice(0, at) + (pieces[Math.floor(rnd() * pieces.length)] as string) + out.slice(at);
        else if (op < 0.7) out = out.slice(0, at) + out.slice(at + 1 + Math.floor(rnd() * 20));
        else out = out.slice(0, at) + String.fromCharCode(Math.floor(rnd() * 0x3000)) + out.slice(at);
      }
      return out;
    };
    const sources = FIXTURES.map(([f, name]) => [fixture(f), name] as const);
    let made = 0;
    for (let i = 0; i < 1500; i++) {
      const [text, name] = i % 10 === 0
        ? [Array.from({ length: Math.floor(rnd() * 400) }, () => pieces[Math.floor(rnd() * pieces.length)]).join(''), (['x.json', 'x.toml', 'x.yml', 'x.itermcolors', 'g'] as const)[i % 5] as string]
        : [mutate((sources[i % sources.length] as readonly [string, string])[0]), (sources[i % sources.length] as readonly [string, string])[1]];
      let r: Converted | undefined;
      expect(() => { r = convertTheme(text, name, 'Imported theme'); }).not.toThrow();
      if (!r || !r.ok) continue;
      made++;
      expect(IMPORT_FORMATS).toContain(r.format);
      expect(() => readTheme(JSON.parse(serialise(r?.ok ? r.file : {})))).not.toThrow();
      // What it makes is the shape of format 2: never a value the theme check calls unsafe or unknown.
      const judged = readTheme(r.file);
      if (!judged.ok) for (const issue of judged.issues) expect(['contrast', 'terminal-contrast', 'region-contrast', 'more'], JSON.stringify(issue)).toContain(issue.code);
    }
    expect(made).toBeGreaterThan(500);
  });
});

describe('words', () => {
  const notes: ImportNote[] = [
    { code: 'scheme', scheme: 'dark' }, { code: 'scheme', scheme: 'light' }, { code: 'derived' }, { code: 'derived-kept' },
    { code: 'missing', keys: ['cursor', 'bright-white'], more: 2 }, { code: 'ignored', keys: ['a'], more: 0 }, { code: 'unreadable', keys: ['b'], more: 0 },
    { code: 'alpha', keys: ['red'], more: 0 }, { code: 'background-alpha' }, { code: 'selection-alpha' }, { code: 'colour-space', space: 'p3' },
    { code: 'colour-space', space: 'other' }, { code: 'variant', used: 'light' }, { code: 'first-scheme', count: 3 },
    { code: 'fallback', key: 'background', from: 'editor.background' }, { code: 'syntax', count: 1 }, { code: 'syntax', count: 6 },
    { code: 'window-colours', count: 1 }, { code: 'window-colours', count: 130 }, { code: 'include' },
  ];
  const errors: ImportError[] = ['too-large', 'unknown-format', 'unreadable', 'entities', 'no-colours'];
  for (const lang of ['en', 'tr']) {
    it(`has every note, error and app name in ${lang}`, () => {
      setLanguage(lang);
      const all = [...notes.map(importNoteText), ...errors.map(importErrorText), ...IMPORT_FORMATS.map((f) => t(`theme-import-format-${f}`))];
      for (const s of all) expect(s).not.toMatch(/theme-import|\{\s*\$|undefined/);
      expect(importNoteText({ code: 'missing', keys: ['cursor', 'bright-white'], more: 2 })).toContain(t('theme-terminal-cursor'));
      expect(importErrorText('too-large')).toContain('256');
    });
  }
});
