import { readFileSync } from 'node:fs';
import { afterEach, describe, expect, it } from 'vitest';
import { setLanguage, t } from '../i18n';
import { issueText, themeName, tokenLabel } from './messages';
import {
  AGENT_COLOURS, AGENT_COLOUR_NAMES, BUILT_IN, CONTRAST_PAIRS, agentColours, DERIVED, TOKEN_NAMES, type ParsedTheme, type ThemeIssue, applyTheme, baseTokens, contrast, contrastIssues,
  TINT_PAIRS, cssVariables, deserialiseTheme, parseTheme, readValue, serialiseTheme, themeFile, themeWarnings, tintIssues,
} from './tokens';
import { CONTROLS, DANGER, INDICATORS, TINT_ROLES, kitPalette, tints, type KitColour } from './tints';

const file = (tokens: Record<string, unknown> = {}, extra: Record<string, unknown> = {}) => ({ 'agora-theme': 1, name: 'Mine', scheme: 'dark', tokens, ...extra });
const issues = (p: ParsedTheme): ThemeIssue[] => (p.ok ? [] : p.issues);

describe('theme tokens', () => {
  it('ships Dark and Light exactly as design/tokens.css draws them before the script runs', () => {
    // A Windows checkout (core.autocrlf) has CRLF line ends; the selectors below are written with LF.
    const css = readFileSync('src/design/tokens.css', 'utf8').replace(/\r\n/g, '\n');
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

  it('draws agent colours from one list, visible on the surfaces of every theme (at least 3:1)', () => {
    const css = readFileSync('src/design/tokens.css', 'utf8').replace(/\r\n/g, '\n');
    const block = (selector: string) => {
      const i = css.indexOf(selector);
      return css.slice(i, css.indexOf('}', i));
    };
    // Dark and Light are drawn by the stylesheet itself: it holds the same values.
    for (const [scheme, src] of [['dark', block(":root,\n[data-theme='dark']")], ['light', block("[data-theme='light']")], ['light', block(':root:not([data-theme])')]] as const) {
      for (const n of AGENT_COLOUR_NAMES) expect(new RegExp(`--agent-${n}:\\s*([^;]+);`).exec(src)?.[1]?.trim(), `${scheme} --agent-${n}`).toBe(AGENT_COLOURS[scheme][n]);
      expect(agentColours(baseTokens(scheme))).toBe(AGENT_COLOURS[scheme]);
    }
    // Every built-in theme, and a Light-slot theme with Dark's surfaces (a valid import), gets a set that shows on its own surfaces.
    const lightSlotDarkSurfaces = { ...baseTokens('light'), ...Object.fromEntries((['surface-0', 'surface-1', 'surface-2'] as const).map((s) => [s, baseTokens('dark')[s]])) };
    for (const [id, tokens] of [...BUILT_IN.map((t) => [t.id, t.tokens] as const), ['light-slot-dark-surfaces', lightSlotDarkSurfaces] as const]) {
      const vars = cssVariables({ tokens });
      for (const n of AGENT_COLOUR_NAMES) {
        for (const surface of ['surface-0', 'surface-1', 'surface-2'] as const) {
          expect(contrast(vars[`--agent-${n}`] as string, tokens[surface]), `${id} --agent-${n} on ${surface}`).toBeGreaterThanOrEqual(3);
        }
      }
    }
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

  it('checks text on each coloured background the way the widget kit mixes it: the role colour, text and secondary text, at 4.5:1', () => {
    expect(TINT_PAIRS.map((p) => `${p.fg}/${p.tint}/${p.min}`)).toEqual(TINT_ROLES.flatMap((r) => {
      const own = { accent: 'accent', working: 'state-working', needs: 'state-needs-you', finished: 'state-finished', danger: 'danger', idle: 'state-idle' }[r];
      return [own, 'text', 'text-muted'].map((fg) => `${fg}/${r}/4.5`);
    }));
    for (const b of BUILT_IN) {
      expect(tintIssues(b.tokens, b.scheme), b.id).toEqual([]);
      expect(themeWarnings(b.tokens, b.scheme), b.id).toEqual([]); // nor any of the other warnings (#1054)
      const p = parseTheme(themeFile(b, 'x'));
      expect(p.ok && p.warnings, b.id).toBeUndefined();
    }
  });

  it('warns, without refusing, about a theme that passes the surface pairs but not text on its own coloured background', () => {
    // Idle reads 3.8:1 on Panels (state colours need 3:1 there), but as words on the Idle background it is 3.4:1.
    const p = parseTheme(file({ 'state-idle': '#707078' }));
    expect(p.ok).toBe(true);
    if (!p.ok) return;
    expect(contrastIssues(p.tokens)).toEqual([]); // what the check said before: readable
    const bg = tints(p.tokens, 'dark').idle;
    expect(p.warnings).toEqual([{ code: 'tint-contrast', fg: 'state-idle', tint: 'idle', ratio: contrast('#707078', bg), min: 4.5 }]);
    expect(issueText(p.warnings![0]!)).toBe('Idle on Idle background is 3.4:1; it needs at least 4.5:1 to be readable.');
    // A refused theme says why it is refused; the warnings come once it is accepted.
    const both = parseTheme(file({ 'state-idle': '#707078', 'text-muted': '#3a3a40' }));
    expect(issues(both).every((i) => i.code === 'contrast')).toBe(true);
  });

  it('checks the fixed red against the theme\'s own panels, and mixes Light at its own strength', () => {
    // Dark with navy panels: readable on its surfaces, but the red's own background is mixed from those panels, and the red reads 4.4:1 there.
    const p = parseTheme(file({ 'surface-1': '#000088' }));
    expect(p.ok).toBe(true);
    const w = p.ok ? p.warnings ?? [] : [];
    expect(w.map((i) => (i.code === 'tint-contrast' ? `${i.fg}/${i.tint}` : i.code))).toEqual(['text-muted/needs', 'text-muted/finished', 'danger/danger', 'state-idle/idle']);
    expect(issueText(w.find((i) => i.code === 'tint-contrast' && i.fg === 'danger')!)).toBe('Red (errors) on Error background is 4.4:1; it needs at least 4.5:1 to be readable.');
    // Each scheme mixes at its own strength (app.css `--tint`): 14 % in Dark, 10 % in Light.
    expect(tints(baseTokens('dark'), 'dark').needs).toBe('#2c261d');
    expect(tints(baseTokens('light'), 'light').needs).toBe('#f7eee8');
  });

  it('warns about the fixed red as words on the theme\'s own surfaces (#1054)', () => {
    // Raised surfaces a little lighter than Dark's: text, secondary text and the accent still read 4.5:1 there, the red 4.2:1.
    const p = parseTheme(file({ 'surface-2': '#38383e', 'text-muted': '#c4c4cc', accent: '#b0b6ff' }));
    expect(p.ok).toBe(true);
    if (!p.ok) return;
    expect(tintIssues(p.tokens, 'dark')).toEqual([]); // what #1051 checked: nothing to say
    expect(p.warnings).toEqual([{ code: 'kit-contrast', fg: 'danger', bg: 'surface-2', ratio: contrast(DANGER.dark, '#38383e'), min: 4.5 }]);
    expect(issueText(p.warnings![0]!)).toBe('Red (errors) on Raised surfaces is 4.2:1; it needs at least 4.5:1 to be readable.');
  });

  it('warns about an agent\'s initial on its coloured badge (#1054)', () => {
    // Dimmer text still reads 4.5:1 on every surface, but the badge is the agent colour mixed into the panels: 4.4:1 on Mint.
    const p = parseTheme(file({ text: '#9c9ca4' }));
    expect(p.ok).toBe(true);
    if (!p.ok) return;
    expect(contrastIssues(p.tokens)).toEqual([]);
    expect(tintIssues(p.tokens, 'dark')).toEqual([]);
    const badge = kitPalette(p.tokens, 'dark', agentColours(p.tokens))['avatar-mint'];
    expect(p.warnings).toEqual([{ code: 'kit-contrast', fg: 'text', bg: 'avatar-mint', ratio: contrast('#9c9ca4', badge), min: 4.5 }]);
    expect(issueText(p.warnings![0]!)).toBe('Text on Mint agent badge is 4.4:1; it needs at least 4.5:1 to be readable.');
  });

  it('warns about a control indicator below 3:1, naming the control (#1054)', () => {
    // Idle at 3.2:1 on the surfaces (enough for a state colour) is 2.7:1 as an icon on its own chip.
    const p = parseTheme(file({ 'state-idle': '#66666e' }));
    expect(p.ok).toBe(true);
    if (!p.ok) return;
    const chip = kitPalette(p.tokens, 'dark', agentColours(p.tokens))['chip-idle'];
    const control = { code: 'control-contrast', control: 'icon', fg: 'state-idle', bg: 'chip-idle', ratio: contrast('#66666e', chip), min: 3 };
    expect(p.warnings).toContainEqual(control);
    expect(tintIssues(p.tokens, 'dark').map((i) => `${i.fg}/${i.tint}`)).toEqual(['state-idle/idle']); // #1051 saw only the words
    expect(issueText(control as ThemeIssue)).toBe('Icon: Idle on Idle icon background is 2.7:1; it needs at least 3:1 to be seen.');
  });

  it('checks every control indicator the widget kit draws, once, and leaves out pairs a stricter check already asks for', () => {
    // Accent, secondary text and the label on the accent are refused below 4.5:1 on the surfaces: a 3:1 warning would only repeat it.
    const p = parseTheme(file({ 'text-muted': '#3a3a40' }));
    expect(p.ok).toBe(false);
    const draft = { ...baseTokens('dark'), 'text-muted': '#3a3a40' };
    expect(themeWarnings(draft, 'dark').filter((w) => w.code === 'control-contrast')).toEqual([]);
    for (const row of INDICATORS) expect(CONTROLS).toContain(row.control);
    expect(new Set(INDICATORS.map((r) => r.control))).toEqual(new Set(CONTROLS));
  });

  it('handles a white accent with a grey label (the #1018 review): accepted with nothing to warn about in Dark, refused in Light', () => {
    const dark = parseTheme(file({ accent: '#ffffff', 'on-accent': '#666666' }));
    expect(dark.ok).toBe(true);
    expect(dark.ok && dark.warnings).toBeUndefined();
    expect(dark.ok && tintIssues(dark.tokens, 'dark')).toEqual([]); // white on its own background, the label is never on a tint
    expect(dark.ok && tints(dark.tokens, 'dark').accent).toBe('#2c2c2f');
    const light = parseTheme(file({ accent: '#ffffff', 'on-accent': '#666666' }, { scheme: 'light' }));
    expect(issues(light).map((i) => (i.code === 'contrast' ? `${i.fg}/${i.bg}` : i.code))).toEqual(['accent/surface-0', 'accent/surface-1', 'accent/surface-2']);
  });

  it('turns a theme into CSS variables; sizes follow the browser text size', () => {
    const large = BUILT_IN.find((b) => b.id === 'large-text')!;
    const vars = cssVariables(large);
    expect(vars['--surface-0']).toBe(baseTokens('dark')['surface-0']);
    expect(vars['--text-scale']).toBe('1.25');
    expect(vars['--space-1']).toBe('0.25rem');
    expect(vars['--radius-s']).toBe('10px');
    expect(Object.keys(vars).filter((k) => !TOKEN_NAMES.includes(k.slice(2) as never))).toEqual(['--text-scale', ...AGENT_COLOUR_NAMES.map((n) => `--agent-${n}`)]);
    const set = new Map<string, string>();
    const root = { style: { setProperty: (k: string, v: string) => { set.set(k, v); }, removeProperty: (k: string) => set.delete(k) }, dataset: {} as Record<string, string | undefined> };
    applyTheme(root, BUILT_IN.find((b) => b.id === 'sand')!);
    expect(root.dataset.theme).toBe('light');
    expect(set.get('--accent')).toBe('#9a4a1f');
    expect(set.get('--text-scale')).toBe('1');
    expect(set.get('--agent-slate')).toBe(AGENT_COLOURS.light.slate); // sand's surfaces are light
    expect(set.size).toBe(TOKEN_NAMES.length + 1 + AGENT_COLOUR_NAMES.length);
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
        ...TINT_ROLES.map((tint): ThemeIssue => ({ code: 'tint-contrast', fg: tint === 'danger' ? 'danger' : 'text', tint, ratio: 3.27, min: 4.5 })),
        // Every colour the kit draws, as either side of a pair, and every control.
        ...(Object.keys(kitPalette(baseTokens('dark'), 'dark', AGENT_COLOURS.dark)) as KitColour[]).map((bg): ThemeIssue => ({ code: 'kit-contrast', fg: bg, bg, ratio: 3.27, min: 4.5 })),
        ...INDICATORS.map((r): ThemeIssue => ({ code: 'control-contrast', control: r.control, fg: r.fg, bg: r.bg[0]!, ratio: 2.1, min: 3 })),
      ];
      for (const i of all) expect(issueText(i), JSON.stringify(i)).not.toMatch(/theme-|\{\$|Unknown variable/);
      for (const k of ['theme-for-light', 'theme-for-dark', 'theme-error-full', 'theme-contrast-warning', 'theme-contrast-also', 'theme-import-warning']) expect(t(k, { max: 1 }), k).not.toBe(k);
    });
  }
  it('says the ratio rounded down, so a shown 4.5 always passes', () => {
    expect(issueText({ code: 'contrast', fg: 'text', bg: 'surface-0', ratio: 4.4999, min: 4.5 })).toBe('Text on Background is 4.4:1; it needs at least 4.5:1 to be readable.');
    expect(issueText({ code: 'bad-value', token: 'space-1' })).toBe('Spacing ("space-1") must be between 3 and 6 px.');
  });
});
