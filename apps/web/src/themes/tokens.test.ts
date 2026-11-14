import { readFileSync } from 'node:fs';
import { afterEach, describe, expect, it } from 'vitest';
import { setLanguage, t } from '../i18n';
import { issueText, themeName, tokenLabel } from './messages';
import {
  BUILT_IN, CONTRAST_PAIRS, DERIVED, TOKEN_NAMES, type ParsedTheme, type ThemeIssue, applyTheme, baseTokens, contrast, contrastIssues,
  cssVariables, deserialiseTheme, parseTheme, readValue, serialiseTheme, themeFile,
} from './tokens';

const file = (tokens: Record<string, unknown> = {}, extra: Record<string, unknown> = {}) => ({ 'agora-theme': 1, name: 'Mine', scheme: 'dark', tokens, ...extra });
const issues = (p: ParsedTheme): ThemeIssue[] => (p.ok ? [] : p.issues);

describe('theme tokens', () => {
  it('ships Dark and Light exactly as design/tokens.css draws them before the script runs', () => {
    const css = readFileSync('src/design/tokens.css', 'utf8');
    const block = (selector: string) => {
      const i = css.indexOf(selector);
      return css.slice(i, css.indexOf('}', i));
    };
    const value = (src: string, name: string) => new RegExp(`--${name}:\\s*([^;]+);`).exec(src)?.[1]?.trim();
    for (const [scheme, src] of [['dark', block(":root,\n[data-theme='dark']")], ['light', block("[data-theme='light']")], ['light', block(':root:not([data-theme])')]] as const) {
      for (const n of TOKEN_NAMES.filter((n) => n.startsWith('surface') || n.startsWith('text') && n !== 'text-16' || n.startsWith('state') || ['border', 'accent', 'on-accent'].includes(n))) {
        expect(value(src, n), `${scheme} ${n}`).toBe(baseTokens(scheme)[n]);
      }
    }
    const root = block(':root {');
    for (const n of ['font-ui', 'font-mono', 'text-16', 'radius-xs', 'radius-s', 'radius-m', 'radius-l'] as const) expect(value(root, n), n).toBe(baseTokens('dark')[n]);
  });

  it('computes WCAG contrast', () => {
    expect(contrast('#000000', '#ffffff')).toBeCloseTo(21, 5);
    expect(contrast('#ffffff', '#ffffff')).toBeCloseTo(1, 5);
    expect(contrast('#777777', '#ffffff')).toBeCloseTo(4.48, 2);
    expect(contrast('#ffffff', '#777777')).toBe(contrast('#777777', '#ffffff'));
  });

  it('keeps every built-in theme readable, and each reads back from its own file', () => {
    expect(BUILT_IN.map((b) => b.id)).toEqual(['dark', 'light', 'high-contrast', 'large-text', 'slate', 'sand']);
    for (const b of BUILT_IN) {
      expect(contrastIssues(b.tokens), b.id).toEqual([]);
      const back = deserialiseTheme(serialiseTheme(themeFile(b, 'x')));
      expect(back, b.id).toEqual({ ok: true, name: 'x', scheme: b.scheme, tokens: b.tokens });
    }
    // High contrast really is higher: text on every surface at least 15:1 (AAA is 7).
    const hc = BUILT_IN.find((b) => b.id === 'high-contrast')!;
    for (const s of ['surface-0', 'surface-1', 'surface-2'] as const) expect(contrast(hc.tokens.text, hc.tokens[s])).toBeGreaterThan(15);
  });

  it('fills what a file leaves out from its scheme, and writes values in one form', () => {
    const p = parseTheme(file({ accent: '#33A', 'text-16': 18, 'radius-s': '9.5px', 'font-ui': '  Inter,   sans-serif ' }, { scheme: 'light', name: ' Pale ' }));
    expect(p.ok).toBe(true);
    if (!p.ok) return;
    expect(p.name).toBe('Pale');
    expect(p.tokens).toEqual({ ...baseTokens('light'), accent: '#3333aa', 'text-16': '18px', 'radius-s': '9.5px', 'font-ui': 'Inter, sans-serif' });
    expect(parseTheme(file()).ok).toBe(true); // no tokens at all: the scheme's default
  });

  it('refuses unknown, derived and invalid tokens, and says which', () => {
    const p = parseTheme(file({ 'surface-9': '#000000', 'text-14': '15px', accent: 'blue', 'text-16': '40px', 'space-1': '4em', 'font-mono': 'x; } body { color: red', border: '#12345' }));
    expect(issues(p)).toEqual([
      { code: 'unknown-token', token: 'surface-9' },
      { code: 'derived-token', token: 'text-14', base: 'text-16' },
      { code: 'bad-value', token: 'accent' },
      { code: 'bad-value', token: 'text-16' },
      { code: 'bad-value', token: 'space-1' },
      { code: 'bad-value', token: 'font-mono' },
      { code: 'bad-value', token: 'border' },
    ]);
    for (const [k, base] of Object.entries(DERIVED)) expect(issues(parseTheme(file({ [k]: '1px' })))).toEqual([{ code: 'derived-token', token: k, base }]);
    for (const v of ['url(x)', "'Inter", 'a{b}', '', 'x'.repeat(201), 3]) expect(readValue('font-ui', v), String(v)).toBeUndefined();
    for (const v of ['#12', 'rgb(0,0,0)', '#ggg', 0x123456, '#1234567']) expect(readValue('accent', v), String(v)).toBeUndefined();
    for (const v of [2, '2px', '7px', '5 px', '5', -4, Number.NaN, Infinity]) expect(readValue('space-1', v), String(v)).toBeUndefined();
  });

  it('refuses files that are not themes, or from a newer version', () => {
    expect(deserialiseTheme('{')).toEqual({ ok: false, issues: [{ code: 'json' }] });
    for (const v of [null, [], 'x', {}, { 'agora-theme': '1' }, { 'agora-theme': 0 }, { 'agora-theme': 1.5 }]) expect(issues(parseTheme(v)), JSON.stringify(v)).toEqual([{ code: 'invalid' }]);
    expect(issues(parseTheme({ ...file(), 'agora-theme': 2 }))).toEqual([{ code: 'newer' }]);
    expect(issues(parseTheme(file({}, { scheme: 'sepia' })))).toEqual([{ code: 'scheme' }]);
    expect(issues(parseTheme(file({}, { tokens: [] })))).toEqual([{ code: 'invalid' }]);
    expect(issues(parseTheme(file({}, { name: '' })))).toEqual([{ code: 'name' }]);
    expect(issues(parseTheme(file({}, { name: 'x'.repeat(61) })))).toEqual([{ code: 'name' }]);
    expect(issues(parseTheme(file({}, { colours: {} })))).toEqual([{ code: 'unknown-field', field: 'colours' }]);
    // A name never carries control or direction characters to the screen.
    const p = parseTheme(file({}, { name: 'A‮B\u0007C' }));
    expect(p.ok && p.name).toBe('ABC');
  });

  it('refuses colours whose text would be hard to read, naming each pair', () => {
    const p = parseTheme(file({ 'text-muted': '#3a3a40', 'on-accent': '#9097ff', 'state-unknown': '#2a2a30' }));
    const bad = issues(p).map((i) => (i.code === 'contrast' ? `${i.fg}/${i.bg}/${i.min}` : i.code));
    expect(bad).toEqual([
      'text-muted/surface-0/4.5', 'text-muted/surface-1/4.5', 'text-muted/surface-2/4.5', 'on-accent/accent/4.5',
      'state-unknown/surface-0/3', 'state-unknown/surface-1/3', 'state-unknown/surface-2/3',
    ]);
    for (const i of issues(p)) if (i.code === 'contrast') expect(i.ratio).toBeLessThan(i.min);
    // Every pair the check knows is a real token pair.
    for (const c of CONTRAST_PAIRS) expect(TOKEN_NAMES).toContain(c.fg);
  });

  it('turns a theme into CSS variables; sizes follow the browser text size', () => {
    const large = BUILT_IN.find((b) => b.id === 'large-text')!;
    const vars = cssVariables(large);
    expect(vars['--surface-0']).toBe(baseTokens('dark')['surface-0']);
    expect(vars['--text-scale']).toBe('1.25');
    expect(vars['--space-1']).toBe('0.25rem');
    expect(vars['--radius-s']).toBe('10px');
    expect(Object.keys(vars).filter((k) => !TOKEN_NAMES.includes(k.slice(2) as never))).toEqual(['--text-scale']);
    const set = new Map<string, string>();
    const root = { style: { setProperty: (k: string, v: string) => { set.set(k, v); }, removeProperty: (k: string) => set.delete(k) }, dataset: {} as Record<string, string | undefined> };
    applyTheme(root, BUILT_IN.find((b) => b.id === 'sand')!);
    expect(root.dataset.theme).toBe('light');
    expect(set.get('--accent')).toBe('#9a4a1f');
    expect(set.get('--text-scale')).toBe('1');
    expect(set.size).toBe(TOKEN_NAMES.length + 1);
    // Back to Dark: the stylesheet's own values show again (and `data-theme` flips still work).
    applyTheme(root, BUILT_IN[0]!);
    expect(set.size).toBe(0);
    expect(root.dataset.theme).toBe('dark');
  });
});

describe('theme messages', () => {
  afterEach(() => setLanguage('en'));
  for (const lang of ['en', 'tr']) {
    it(`has every theme name, token and problem in ${lang}`, () => {
      setLanguage(lang);
      for (const b of BUILT_IN) expect(themeName(b), b.id).not.toBe(b.nameKey);
      for (const n of TOKEN_NAMES) expect(tokenLabel(n), n).not.toBe(`theme-token-${n}`);
      const all: ThemeIssue[] = [
        { code: 'json' }, { code: 'invalid' }, { code: 'newer' }, { code: 'name' }, { code: 'scheme' }, { code: 'unknown-field', field: 'f' },
        { code: 'unknown-token', token: 'x' }, { code: 'derived-token', token: 'text-14', base: 'text-16' },
        { code: 'bad-value', token: 'accent' }, { code: 'bad-value', token: 'font-ui' }, { code: 'bad-value', token: 'space-1' },
        { code: 'contrast', fg: 'text', bg: 'surface-0', ratio: 3.27, min: 4.5 },
      ];
      for (const i of all) expect(issueText(i), i.code).not.toMatch(/^theme-|\{\$|Unknown variable/);
      for (const k of ['theme-for-light', 'theme-for-dark', 'theme-error-full']) expect(t(k, { max: 1 }), k).not.toBe(k);
    });
  }
  it('says the ratio rounded down, so a shown 4.5 always passes', () => {
    expect(issueText({ code: 'contrast', fg: 'text', bg: 'surface-0', ratio: 4.4999, min: 4.5 })).toBe('Text on Background is 4.4:1; it needs at least 4.5:1 to be readable.');
    expect(issueText({ code: 'bad-value', token: 'space-1' })).toBe('Spacing ("space-1") must be between 3 and 6 px.');
  });
});
