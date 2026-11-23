// The widget kit's derived colours (design/app.css, ADR-0102), computed the way the browser mixes them: `color-mix(in oklab, …)`.
// One copy of the math for the theme editor's readability check (tokens.ts) and the kit's contrast test (design/kitContrast.test.ts),
// which also holds these constants equal to app.css and tokens.css. Plain data and arithmetic: nothing here touches the DOM.
import type { Scheme, Tokens } from './tokens';

// How strong a soft state background is (`--tint` in app.css): Light stays at 10 % so amber text on its tint keeps 4.5:1.
export const TINT_STRENGTH: Readonly<Record<Scheme, number>> = { dark: 0.14, light: 0.1 };
// The one red (`--danger`, the fixed `--diff-removed` of tokens.css): never a theme colour.
export const DANGER: Readonly<Record<Scheme, string>> = { dark: '#f87171', light: '#a82117' };

// The roles that get a tint, in the order the editor lists them, and the colour each one is mixed from.
export const TINT_ROLES = ['accent', 'working', 'needs', 'finished', 'danger', 'idle'] as const;
export type TintRole = (typeof TINT_ROLES)[number];
export const ROLE_TOKEN = {
  accent: 'accent', working: 'state-working', needs: 'state-needs-you', finished: 'state-finished', idle: 'state-idle',
} as const satisfies Record<Exclude<TintRole, 'danger'>, string>;
type RoleToken = (typeof ROLE_TOKEN)[keyof typeof ROLE_TOKEN];

// sRGB hex ⇄ OKLab (Björn Ottosson's matrices, as CSS Color 4), mixed per channel like color-mix().
const lin = (c: number) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
const delin = (c: number) => (c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055);
function toLab(hex: string): [number, number, number] {
  const [r, g, b] = [1, 3, 5].map((i) => lin(Number.parseInt(hex.slice(i, i + 2), 16) / 255)) as [number, number, number];
  const [l, m, s] = [0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b, 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b, 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b].map(Math.cbrt) as [number, number, number];
  return [0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s, 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s, 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s];
}
function fromLab([L, a, b]: [number, number, number]): string {
  const [l, m, s] = [L + 0.3963377774 * a + 0.2158037573 * b, L - 0.1055613458 * a - 0.0638541728 * b, L - 0.0894841775 * a - 1.291485548 * b].map((x) => x ** 3) as [number, number, number];
  const rgb = [4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s, -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s, -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s];
  return `#${rgb.map((c) => Math.round(Math.min(1, Math.max(0, delin(c))) * 255).toString(16).padStart(2, '0')).join('')}`;
}
// `color-mix(in oklab, a p, b)` for p in 0..1, as #rrggbb (both colours #rrggbb).
export function mix(a: string, p: number, b: string): string {
  const A = toLab(a);
  const B = toLab(b);
  return fromLab([A[0] * p + B[0] * (1 - p), A[1] * p + B[1] * (1 - p), A[2] * p + B[2] * (1 - p)]);
}

// The colour a role is drawn in: the theme's own, or the fixed red.
export function roleColours(tokens: Pick<Tokens, RoleToken>, scheme: Scheme): Record<TintRole, string> {
  return Object.fromEntries(TINT_ROLES.map((r) => [r, r === 'danger' ? DANGER[scheme] : tokens[ROLE_TOKEN[r]]])) as Record<TintRole, string>;
}
// Each role's colour mixed into the panel colour at strength p (`--tint-*` at the scheme's `--tint`, `--line-*` at `--line`, …).
export function roleMix(tokens: Pick<Tokens, 'surface-1' | RoleToken>, scheme: Scheme, p: number): Record<TintRole, string> {
  const fg = roleColours(tokens, scheme);
  return Object.fromEntries(TINT_ROLES.map((r) => [r, mix(fg[r], p, tokens['surface-1'])])) as Record<TintRole, string>;
}
export const tints = (tokens: Tokens, scheme: Scheme) => roleMix(tokens, scheme, TINT_STRENGTH[scheme]);
