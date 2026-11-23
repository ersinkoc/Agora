// The widget kit's derived colours (design/app.css, ADR-0102), computed the way the browser mixes them: `color-mix(in oklab, …)`.
// One copy of the math for the theme editor's readability check (tokens.ts) and the kit's contrast test (design/kitContrast.test.ts),
// which also holds these constants equal to app.css and tokens.css. Plain data and arithmetic: nothing here touches the DOM.
import type { AgentColourName, ColourToken, Scheme, TokenName, Tokens } from './tokens';

// How strong a soft state background is (`--tint` in app.css): Light stays at 10 % so amber text on its tint keeps 4.5:1.
export const TINT_STRENGTH: Readonly<Record<Scheme, number>> = { dark: 0.14, light: 0.1 };
// The one red (`--danger`, the fixed `--diff-removed` of tokens.css): never a theme colour.
export const DANGER: Readonly<Record<Scheme, string>> = { dark: '#f87171', light: '#a82117' };
// The label and thumb on the red (`--on-danger` of tokens.css): fixed beside it, never the theme's on-accent.
export const ON_DANGER: Readonly<Record<Scheme, string>> = { dark: '#0b0b14', light: '#ffffff' };
// How much of a colour app.css mixes into the panels for an agent's badge (`.kit-avatar`), a role's icon chip (`.kit-chip`), and how
// much text into them for a chosen option (`--surface-raised`).
export const AVATAR_STRENGTH = 0.22;
export const CHIP_STRENGTH = 0.22;
export const RAISED_STRENGTH = 0.04;

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

// ---- What the kit draws from a theme, for the editor's checks (#1054) -------------------------------------------------------------

// A colour the kit can name: a theme token, the fixed red and its label, a chosen option's background, and each role's tint and icon
// chip, and each agent's badge.
export type KitColour = ColourToken | 'danger' | 'on-danger' | 'surface-raised' | `tint-${TintRole}` | `chip-${TintRole}` | `avatar-${AgentColourName}`;
// The colour each role is drawn in, by name (`--kit-fg` of `.kit-tone-*`).
export const ROLE_KIT_COLOUR = { ...ROLE_TOKEN, danger: 'danger' } as const satisfies Record<TintRole, KitColour>;

// Every colour of one theme the kit draws. The agent colours come in from tokens.ts (`agentColours`), which picks them by surfaces.
export function kitPalette(tokens: Tokens, scheme: Scheme, agents: Readonly<Record<AgentColourName, string>>): Record<KitColour, string> {
  // Only colour tokens hold a value that starts with `#` (tokens.ts `readValue` writes colours as #rrggbb; a font may not contain one).
  const own = Object.fromEntries(Object.entries(tokens).filter(([, v]) => v.startsWith('#')));
  const tint = tints(tokens, scheme);
  const chip = roleMix(tokens, scheme, CHIP_STRENGTH);
  return {
    ...own,
    danger: DANGER[scheme],
    'on-danger': ON_DANGER[scheme],
    'surface-raised': mix(tokens.text, RAISED_STRENGTH, tokens['surface-1']),
    ...Object.fromEntries(TINT_ROLES.map((r) => [`tint-${r}`, tint[r]])),
    ...Object.fromEntries(TINT_ROLES.map((r) => [`chip-${r}`, chip[r]])),
    ...Object.fromEntries(Object.entries(agents).map(([n, c]) => [`avatar-${n}`, mix(c, AVATAR_STRENGTH, tokens['surface-1'])])),
  } as Record<KitColour, string>;
}

// The control indicators (ADR-0102 item 6, WCAG 1.4.11: 3:1): the colour that shows the control's state, and what it must stand out
// from. The first rows are the kit's rules in app.css, in the order design/kitContrast.test.ts reads them (and holds equal); then the
// focus ring (the base `:focus-visible` outline) on every tint, and each role's icon on its chip.
export const CONTROLS = ['segmented', 'risk-segmented', 'switch-off', 'switch-on', 'risk-switch', 'selected-card', 'pressed-count', 'primary-button', 'danger-button', 'focus-ring', 'icon'] as const;
export type Control = (typeof CONTROLS)[number];
const ON_SURFACES = ['surface-0', 'surface-1', 'surface-2'] as const;
export const INDICATORS: readonly { control: Control; fg: KitColour; bg: readonly KitColour[] }[] = [
  { control: 'segmented', fg: 'accent', bg: ['surface-raised', 'surface-2'] }, // the chosen option's ring
  { control: 'risk-segmented', fg: 'danger', bg: ['surface-raised', 'tint-danger'] }, // a risky chosen option's ring (Start an agent)
  { control: 'switch-off', fg: 'text-muted', bg: ON_SURFACES }, // outline
  { control: 'switch-off', fg: 'text-muted', bg: ON_SURFACES }, // thumb
  { control: 'switch-on', fg: 'accent', bg: ON_SURFACES }, // track
  { control: 'switch-on', fg: 'on-accent', bg: ['accent'] }, // thumb
  { control: 'risk-switch', fg: 'danger', bg: ON_SURFACES },
  { control: 'risk-switch', fg: 'on-danger', bg: ['danger'] },
  { control: 'selected-card', fg: 'accent', bg: ['tint-accent', 'surface-0', 'surface-1'] },
  { control: 'pressed-count', fg: 'accent', bg: ['surface-0', 'surface-1'] },
  { control: 'primary-button', fg: 'accent', bg: ON_SURFACES },
  { control: 'danger-button', fg: 'danger', bg: ON_SURFACES },
  { control: 'focus-ring', fg: 'accent', bg: TINT_ROLES.map((r) => `tint-${r}` as const) },
  ...TINT_ROLES.map((r) => ({ control: 'icon' as const, fg: ROLE_KIT_COLOUR[r], bg: [`chip-${r}` as const] })),
];
