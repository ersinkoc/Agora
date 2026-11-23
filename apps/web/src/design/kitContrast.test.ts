import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { AGENT_COLOUR_NAMES, BUILT_IN, TOKEN_NAMES, agentColours, contrast, parseTheme, type Scheme, type Theme, type Tokens } from '../themes/tokens';

// The widget kit's derived colours (app.css, ADR-0102) computed the way the browser mixes them (`color-mix(in oklab, …)`), from the real
// token values: the built-in themes (themes/tokens.ts), the fixed red (tokens.css) and the mix strengths in app.css. Text needs 4.5:1;
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

// sRGB hex ⇄ OKLab (Björn Ottosson's matrices, as CSS Color 4), mixed per channel like color-mix().
const lin = (c: number) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
const delin = (c: number) => (c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055);
function toLab(hex: string): number[] {
  const [r, g, b] = [1, 3, 5].map((i) => lin(Number.parseInt(hex.slice(i, i + 2), 16) / 255)) as [number, number, number];
  const [l, m, s] = [0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b, 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b, 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b].map(Math.cbrt) as [number, number, number];
  return [0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s, 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s, 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s];
}
function fromLab([L, a, b]: number[]): string {
  const [l, m, s] = [L! + 0.3963377774 * a! + 0.2158037573 * b!, L! - 0.1055613458 * a! - 0.0638541728 * b!, L! - 0.0894841775 * a! - 1.291485548 * b!].map((x) => x ** 3) as [number, number, number];
  const rgb = [4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s, -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s, -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s];
  return `#${rgb.map((c) => Math.round(Math.min(1, Math.max(0, delin(c))) * 255).toString(16).padStart(2, '0')).join('')}`;
}
const mix = (a: string, p: number, b: string) => { const A = toLab(a); const B = toLab(b); return fromLab(A.map((x, i) => x * p + (B[i] as number) * (1 - p))); };

type Role = 'accent' | 'working' | 'needs' | 'finished' | 'danger' | 'idle';
const ROLES: Role[] = ['accent', 'working', 'needs', 'finished', 'danger', 'idle'];
const SURFACES = ['surface-0', 'surface-1', 'surface-2'] as const;
function derived(tokens: Tokens, scheme: Scheme) {
  const fg: Record<Role, string> = { accent: tokens.accent, working: tokens['state-working'], needs: tokens['state-needs-you'], finished: tokens['state-finished'], danger: RED[scheme], idle: tokens['state-idle'] };
  const s1 = tokens['surface-1'];
  const of = (p: number) => Object.fromEntries(ROLES.map((r) => [r, mix(fg[r], p, s1)])) as Record<Role, string>;
  return { fg, tint: of(TINT[scheme]), line: of(LINE), chip: of(CHIP), danger: RED[scheme], onDanger: ON_DANGER[scheme] };
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
    expect(TINT.light).toBeLessThanOrEqual(0.1);
    expect(TINT.dark).toBeLessThanOrEqual(0.16);
    // The system-light block says the same as the explicit Light one.
    expect(value(block(appCss, ':root:not([data-theme]) { --tint'), 'tint')).toBe(value(block(appCss, "[data-theme='light'] { --tint"), 'tint'));
    expect([LINE, AVATAR, CHIP, RAISED, STRONG]).toEqual([0.4, 0.22, 0.22, 0.04, 0.18]);
    expect(ON_DANGER).toEqual({ dark: '#0b0b14', light: '#ffffff' });
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

  for (const theme of [...BUILT_IN, ...(CUSTOM ? [CUSTOM] : [])]) {
    describe(theme.id, () => {
      const t = theme.tokens;
      const d = derived(t, theme.scheme);
      const at = (fgc: string, bg: string) => contrast(fgc, bg);
      it('text on every tint: the role colour, ink and muted text reach 4.5:1', () => {
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
