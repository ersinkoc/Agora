import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import {
  AVATAR_STRENGTH, CHIP_STRENGTH, DANGER, INDICATORS as EDITOR_INDICATORS, ON_DANGER as EDITOR_ON_DANGER, RAISED_STRENGTH, ROLE_KIT_COLOUR, TINT_ROLES, TINT_STRENGTH,
  type KitColour, kitPalette, mix, roleColours, roleMix,
} from '../themes/tints';
import {
  AGENT_COLOUR_NAMES, BUILT_IN, TOKEN_NAMES, agentColours, contrast, parseTheme, requiredContrast, themeWarnings, tintIssues, type Scheme, type Theme, type Tokens,
} from '../themes/tokens';

// The widget kit's derived colours (app.css, ADR-0102) computed the way the browser mixes them (`color-mix(in oklab, …)`, themes/tints.ts,
// the same math the theme editor's readability check uses), from the real token values: the built-in themes (themes/tokens.ts), the
// fixed red (tokens.css) and the mix strengths in app.css. Text needs 4.5:1;
// control indicators (a switch's track and thumb, a selected ring, the focus ring, a filled button) 3:1. The `--line-*` hairlines are
// decoration around a tinted surface whose words carry the meaning: they only have to stay visible (a floor, not 3:1).
const read = (f: string) => readFileSync(f, 'utf8').replace(/\r\n/g, '\n');
const tokensCss = read('src/design/tokens.css');
const appCss = read('src/design/app.css');
const block = (css: string, selector: string) => { const i = css.indexOf(selector); expect(i, selector).toBeGreaterThanOrEqual(0); return css.slice(i, css.indexOf('}', i)); };
const value = (src: string, name: string) => new RegExp(`--${name}:\\s*([^;]+);`).exec(src)?.[1]?.trim() ?? '';
const pct = (v: string) => { const m = /^(\d+(?:\.\d+)?)%$/.exec(v); expect(m, v).not.toBeNull(); return Number(m?.[1]) / 100; };

const schemeBlock: Record<Scheme, string> = { dark: block(tokensCss, ":root,\n[data-theme='dark']"), light: block(tokensCss, "[data-theme='light']") };
const RED: Record<Scheme, string> = { dark: value(schemeBlock.dark, 'diff-removed'), light: value(schemeBlock.light, 'diff-removed') };
const ON_DANGER: Record<Scheme, string> = { dark: value(schemeBlock.dark, 'on-danger'), light: value(schemeBlock.light, 'on-danger') };
const TINT: Record<Scheme, number> = { dark: pct(value(block(appCss, ":root,\n[data-theme='dark'] { --tint"), 'tint')), light: pct(value(block(appCss, "[data-theme='light'] { --tint"), 'tint')) };
const LINE = pct(value(block(appCss, ':root,\n[data-theme] {'), 'line'));
const AVATAR = pct(/--avatar\) (\d+%), var\(--surface-1\)\); box-shadow/.exec(appCss)?.[1] ?? '');
const RAISED = pct(/--surface-raised: color-mix\(in oklab, var\(--text\) (\d+%), var\(--surface-1\)\)/.exec(appCss)?.[1] ?? '');
const STRONG = pct(/--border-strong: color-mix\(in oklab, var\(--text\) (\d+%), var\(--border\)\)/.exec(appCss)?.[1] ?? '');
const CHIP = pct(/\.kit-chip \{[^}]*--kit-fg\) (\d+%)/.exec(appCss)?.[1] ?? '');

const ROLES = TINT_ROLES;
const SURFACES = ['surface-0', 'surface-1', 'surface-2'] as const;
// The mix strengths and the red come from app.css and tokens.css themselves; the shared module is held equal to them below.
function derived(tokens: Tokens, scheme: Scheme) {
  const of = (p: number) => roleMix(tokens, scheme, p);
  return { fg: roleColours(tokens, scheme), tint: of(TINT[scheme]), line: of(LINE), chip: of(CHIP), danger: RED[scheme], onDanger: ON_DANGER[scheme] };
}

// Every colour a kit rule can name, resolved for one theme: its tokens, the fixed red and its label, and the derived roles.
function palette(tokens: Tokens, scheme: Scheme): Record<string, string> {
  const d = derived(tokens, scheme);
  const tints = Object.fromEntries(ROLES.map((r) => [`tint-${r}`, d.tint[r]]));
  return { ...tokens, ...tints, danger: RED[scheme], 'on-danger': ON_DANGER[scheme], 'surface-raised': mix(tokens.text, RAISED, tokens['surface-1']), 'border-strong': mix(tokens.text, STRONG, tokens.border) };
}
// The declarations of one kit rule in app.css, and the token a property of it names.
const rule = (selector: string) => { const i = appCss.indexOf(`  ${selector} {`); expect(i, selector).toBeGreaterThanOrEqual(0); return appCss.slice(i, appCss.indexOf('}\n', i)); };
const uses = (selector: string, prop: string) => {
  const name = new RegExp(`[{;]\\s*${prop}:[^;]*var\\(--([a-z0-9-]+)\\)`).exec(rule(selector))?.[1];
  expect(name, `${selector} ${prop}`).toBeTruthy();
  return name as string;
};
// The control indicators (ADR-0102 item 6, WCAG 1.4.11), read from the kit's own rules: [what, the indicator's colour, what it must stand out from].
const INDICATORS: readonly [string, [string, string], (string | [string, string])[]][] = [
  ['segmented: chosen ring', [".kit-seg__item[aria-checked='true']", 'box-shadow'], [[".kit-seg__item[aria-checked='true']", 'background'], 'surface-2']],
  ['risky segmented: chosen ring', [".kit-seg--risk .kit-seg__item[aria-checked='true']", 'box-shadow'], [[".kit-seg__item[aria-checked='true']", 'background'], ['.kit-seg--risk', 'background']]],
  ['switch off: outline', ['.kit-toggle', 'border'], ['surface-0', 'surface-1', 'surface-2']],
  ['switch off: thumb', ['.kit-toggle::after', 'background'], ['surface-0', 'surface-1', 'surface-2']],
  ['switch on: track', [".kit-toggle[aria-checked='true']", 'background'], ['surface-0', 'surface-1', 'surface-2']],
  ['switch on: thumb', [".kit-toggle[aria-checked='true']::after", 'background'], [[".kit-toggle[aria-checked='true']", 'background']]],
  ['risky switch on: track', [".kit-toggle--risk[aria-checked='true']", 'background'], ['surface-0', 'surface-1', 'surface-2']],
  ['risky switch on: thumb', [".kit-toggle--risk[aria-checked='true']::after", 'background'], [[".kit-toggle--risk[aria-checked='true']", 'background']]],
  ['selected card: ring', ['.kit-card[data-selected]', 'border-color'], [['.kit-card[data-selected]', 'background'], 'surface-0', 'surface-1']],
  ['pressed count: ring', [".kit-stat[aria-pressed='true']", 'border-color'], ['surface-0', 'surface-1']],
  ['primary button: fill', ['.kit-btn--primary', 'background'], ['surface-0', 'surface-1', 'surface-2']],
  ['danger button: fill', ['.kit-btn--danger', 'background'], ['surface-0', 'surface-1', 'surface-2']],
];
// A custom Dark theme that parseTheme accepts (its label reads 5.7:1 on its own white accent) but whose label colour fails on the red.
const CUSTOM = (() => { const p = parseTheme({ 'agora-theme': 1, name: 'White accent', scheme: 'dark', tokens: { accent: '#ffffff', 'on-accent': '#666666' } }); return p.ok ? { id: 'custom-white-accent', scheme: p.scheme, tokens: p.tokens } as Theme : undefined; })();

describe('widget kit colours (ADR-0102)', () => {
  it('mixes like the browser (the values proposal (d) measured for Dark and Light)', () => {
    expect(mix('#fbbf24', 0.14, '#111114')).toBe('#2c261d');
    expect(mix('#a35a00', 0.1, '#ffffff')).toBe('#f7eee8');
    expect(mix('#f87171', 0.4, '#111114')).toBe('#643637');
  });

  it('reads its strengths from app.css; Light keeps its tint at 10 % or less so amber text stays readable', () => {
    expect(RED).toEqual({ dark: '#f87171', light: '#a82117' });
    // The theme editor's readability check (themes/tints.ts) mixes with the same strengths and the same red.
    expect(TINT).toEqual(TINT_STRENGTH);
    expect(RED).toEqual(DANGER);
    expect(TINT.light).toBeLessThanOrEqual(0.1);
    expect(TINT.dark).toBeLessThanOrEqual(0.16);
    // The system-light block says the same as the explicit Light one.
    expect(value(block(appCss, ':root:not([data-theme]) { --tint'), 'tint')).toBe(value(block(appCss, "[data-theme='light'] { --tint"), 'tint'));
    expect([LINE, AVATAR, CHIP, RAISED, STRONG]).toEqual([0.4, 0.22, 0.22, 0.04, 0.18]);
    expect(ON_DANGER).toEqual({ dark: '#0b0b14', light: '#ffffff' });
    // The theme editor's check (themes/tints.ts) draws the badges, chips, chosen option and the label on the red the same way.
    expect([AVATAR_STRENGTH, CHIP_STRENGTH, RAISED_STRENGTH]).toEqual([AVATAR, CHIP, RAISED]);
    expect(EDITOR_ON_DANGER).toEqual(ON_DANGER);
    expect(value(block(tokensCss, ':root:not([data-theme])'), 'on-danger')).toBe(ON_DANGER.light);
  });

  it('the label on the red is fixed beside it, never the theme\'s on-accent: a valid custom theme cannot make it unreadable', () => {
    expect(CUSTOM, 'the custom theme is accepted').toBeDefined();
    expect(TOKEN_NAMES).not.toContain('on-danger'); // a theme file cannot set it
    expect(uses('.kit-btn--danger', 'color')).toBe('on-danger');
    expect(uses(".kit-toggle--risk[aria-checked='true']::after", 'background')).toBe('on-danger');
    const t = (CUSTOM as Theme).tokens;
    expect(contrast(t['on-accent'], t.accent)).toBeGreaterThanOrEqual(4.5); // why parseTheme accepts it
    expect(contrast(t['on-accent'], RED.dark)).toBeLessThan(3); // what reusing on-accent on the red would give
    expect(contrast(ON_DANGER.dark, RED.dark)).toBeGreaterThanOrEqual(4.5);
  });

  it('the theme editor checks the indicators these rules draw, the same colours against the same backgrounds (#1054)', () => {
    // The kit's rows above, read from app.css, are the editor's first rows; then the focus ring on every tint and each role's icon on its chip.
    const name = (x: string | [string, string]) => (typeof x === 'string' ? x : uses(x[0], x[1]));
    expect(EDITOR_INDICATORS.slice(0, INDICATORS.length).map((r) => [r.fg, [...r.bg]])).toEqual(INDICATORS.map(([, ind, against]) => [name(ind), against.map(name)]));
    expect(EDITOR_INDICATORS.slice(INDICATORS.length)).toEqual([
      { control: 'focus-ring', fg: 'accent', bg: ROLES.map((r) => `tint-${r}`) },
      ...ROLES.map((r) => ({ control: 'icon', fg: ROLE_KIT_COLOUR[r], bg: [`chip-${r}`] })),
    ]);
    expect(rule('.kit-chip')).toMatch(/background: color-mix\(in oklab, var\(--kit-fg\) \d+%, var\(--surface-1\)\); color: var\(--kit-fg\)/);
    for (const r of ROLES) expect(new RegExp(`\\.kit-tone-${r} \\{ --kit-fg: var\\(--${ROLE_KIT_COLOUR[r]}\\)`).test(appCss), `kit-tone-${r}`).toBe(true);
    // Everything this test asks, the editor asks at least as strictly (a refusal, or a warning), so a custom theme cannot fail here unseen.
    const asked: [KitColour, KitColour, number][] = [
      ...ROLES.flatMap((r) => ([ROLE_KIT_COLOUR[r], 'text', 'text-muted'] as const).map((fg): [KitColour, KitColour, number] => [fg, `tint-${r}`, 4.5])),
      ['on-danger', 'danger', 4.5], ['on-accent', 'accent', 4.5],
      ...SURFACES.map((s): [KitColour, KitColour, number] => ['danger', s, 4.5]),
      ...AGENT_COLOUR_NAMES.map((n): [KitColour, KitColour, number] => ['text', `avatar-${n}`, 4.5]),
      ...EDITOR_INDICATORS.flatMap((r) => r.bg.map((bg): [KitColour, KitColour, number] => [r.fg, bg, 3])),
    ];
    for (const [fg, bg, min] of asked) expect(requiredContrast(fg, bg), `${fg} on ${bg}`).toBeGreaterThanOrEqual(min);
  });

  for (const theme of [...BUILT_IN, ...(CUSTOM ? [CUSTOM] : [])]) {
    describe(theme.id, () => {
      const t = theme.tokens;
      const d = derived(t, theme.scheme);
      const at = (fgc: string, bg: string) => contrast(fgc, bg);
      it('text on every tint: the role colour, ink and muted text reach 4.5:1', () => {
        expect(tintIssues(t, theme.scheme), `${theme.id}: the theme editor's check agrees`).toEqual([]);
        expect(themeWarnings(t, theme.scheme), `${theme.id}: and so do its other warnings`).toEqual([]);
        // The editor mixes every colour the kit draws as this test does.
        const p = palette(t, theme.scheme);
        const editor = kitPalette(t, theme.scheme, agentColours(t));
        for (const [k, v] of Object.entries(editor)) {
          const own = k.startsWith('chip-') ? d.chip[k.slice(5) as (typeof ROLES)[number]] : k.startsWith('avatar-') ? mix(agentColours(t)[k.slice(7) as (typeof AGENT_COLOUR_NAMES)[number]], AVATAR, t['surface-1']) : p[k];
          expect(v, `${theme.id}: ${k}`).toBe(own);
        }
        for (const r of ROLES) for (const [name, c] of [[r, d.fg[r]], ['text', t.text], ['text-muted', t['text-muted']]] as const) {
          expect(at(c, d.tint[r]), `${theme.id}: ${name} on tint-${r}`).toBeGreaterThanOrEqual(4.5);
        }
        expect(at(d.onDanger, d.danger), `${theme.id}: on-danger on danger`).toBeGreaterThanOrEqual(4.5);
        for (const s of SURFACES) expect(at(d.danger, t[s]), `${theme.id}: danger on ${s}`).toBeGreaterThanOrEqual(4.5);
        for (const n of AGENT_COLOUR_NAMES) expect(at(t.text, mix(agentColours(t)[n], AVATAR, t['surface-1'])), `${theme.id}: letter on the ${n} avatar`).toBeGreaterThanOrEqual(4.5);
      });
      it('control indicators, read from the kit\'s own rules, reach 3:1: segmented ring, switch, selected rings, button fills', () => {
        const p = palette(t, theme.scheme);
        const colour = (x: string | [string, string]) => p[typeof x === 'string' ? x : uses(x[0], x[1])] as string;
        for (const [what, ind, against] of INDICATORS) {
          const c = colour(ind);
          expect(c, `${what}: ${uses(ind[0], ind[1])}`).toMatch(/^#[0-9a-f]{6}$/);
          for (const bg of against) expect(at(c, colour(bg)), `${theme.id}: ${what} (${uses(ind[0], ind[1])}) against ${typeof bg === 'string' ? bg : uses(bg[0], bg[1])}`).toBeGreaterThanOrEqual(3);
        }
        // The chosen risky segment's label is red text on the raised thumb: 4.5:1.
        expect(uses(".kit-seg--risk .kit-seg__item[aria-checked='true']", 'color')).toBe('danger');
        expect(at(d.danger, colour([".kit-seg__item[aria-checked='true']", 'background'])), `${theme.id}: risky segment label`).toBeGreaterThanOrEqual(4.5);
        // Labels on the filled buttons are text: 4.5:1.
        for (const b of ['.kit-btn--primary', '.kit-btn--danger']) expect(at(colour([b, 'color']), colour([b, 'background'])), `${theme.id}: ${b} label`).toBeGreaterThanOrEqual(4.5);
        // The focus ring (the base :focus-visible outline) on every tint, and each role's icon on its chip.
        expect(/:focus-visible \{ outline: 2px solid var\(--accent\)/.test(appCss)).toBe(true);
        for (const r of ROLES) {
          expect(at(t.accent, d.tint[r]), `${theme.id}: focus ring on tint-${r}`).toBeGreaterThanOrEqual(3);
          expect(at(d.fg[r], d.chip[r]), `${theme.id}: ${r} icon on its chip`).toBeGreaterThanOrEqual(3);
        }
      });
      it('decorative hairlines stay visible against the surface and their own tint', () => {
        for (const r of ROLES) {
          expect(at(d.line[r], t['surface-1']), `${theme.id}: line-${r} on surface-1`).toBeGreaterThanOrEqual(1.4);
          expect(at(d.line[r], d.tint[r]), `${theme.id}: line-${r} on tint-${r}`).toBeGreaterThanOrEqual(1.4);
        }
      });
    });
  }
});

describe('widget kit CSS', () => {
  const kit = appCss.slice(appCss.indexOf('/* Derived roles for the widget kit'), appCss.indexOf('/* People who asked for less motion'));
  it('draws colour as a surface, never a stripe, and uses tokens only', () => {
    expect(kit.length).toBeGreaterThan(1000);
    expect(kit).not.toMatch(/border-(inline-start|inline-end|left|right)\s*:/);
    expect(kit).not.toMatch(/inset\s+-?\d+px\s+0\s+0/); // an inset side bar
    // No literal colour outside the shadow (rgb() alpha in --shadow-pop only).
    const noShadow = kit.replace(/--shadow-pop:[^;]+;/g, '');
    expect(noShadow).not.toMatch(/#[0-9a-f]{3,8}\b|rgb\(|hsl\(/i);
    expect(noShadow).not.toMatch(/\b(left|right):/); // logical properties only
  });
});
