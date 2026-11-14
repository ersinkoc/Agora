// Themes are data (ADR-0068 rung 2, docs/ux/themes.md): a theme file names design tokens from design/tokens.css and their values.
// This module is the one door from untrusted text (an imported file, this browser's storage) into a theme: it never throws, refuses
// unknown or invalid tokens, and refuses colours whose text would be hard to read (WCAG 2.2 AA). Nothing here touches the DOM.

export const THEME_FORMAT = 1;
export const MAX_NAME = 60;
const MAX_FONT = 200;

export type Scheme = 'dark' | 'light';
type Kind = { kind: 'color' } | { kind: 'font' } | { kind: 'px'; min: number; max: number };

// The tokens a theme may set, in the order the editor shows them. Each is a CSS variable of the same name (`--surface-0`).
export const TOKENS = {
  'surface-0': { kind: 'color' },
  'surface-1': { kind: 'color' },
  'surface-2': { kind: 'color' },
  border: { kind: 'color' },
  text: { kind: 'color' },
  'text-muted': { kind: 'color' },
  accent: { kind: 'color' },
  'on-accent': { kind: 'color' },
  'state-working': { kind: 'color' },
  'state-needs-you': { kind: 'color' },
  'state-finished': { kind: 'color' },
  'state-idle': { kind: 'color' },
  'state-unknown': { kind: 'color' },
  'font-ui': { kind: 'font' },
  'font-mono': { kind: 'font' },
  'text-16': { kind: 'px', min: 12, max: 24 },
  'space-1': { kind: 'px', min: 3, max: 6 },
  'radius-xs': { kind: 'px', min: 0, max: 16 },
  'radius-s': { kind: 'px', min: 0, max: 24 },
  'radius-m': { kind: 'px', min: 0, max: 32 },
  'radius-l': { kind: 'px', min: 0, max: 40 },
} as const satisfies Record<string, Kind>;
export type TokenName = keyof typeof TOKENS;
export type Tokens = Record<TokenName, string>;
export const TOKEN_NAMES = Object.keys(TOKENS) as TokenName[];
export const COLOR_TOKENS = TOKEN_NAMES.filter((n) => TOKENS[n].kind === 'color');
export const kindOf = (name: TokenName): Kind => TOKENS[name];
export const isTokenName = (n: string): n is TokenName => Object.prototype.hasOwnProperty.call(TOKENS, n);

// Tokens that exist in design/tokens.css but follow from another: a theme sets the one they follow.
export const DERIVED: Readonly<Record<string, TokenName>> = {
  'text-12': 'text-16', 'text-13': 'text-16', 'text-14': 'text-16', 'text-20': 'text-16', 'text-28': 'text-16',
  'space-2': 'space-1', 'space-3': 'space-1', 'space-4': 'space-1', 'space-6': 'space-1', 'space-8': 'space-1',
};

export interface Theme {
  id: string;
  name: string; // a user theme's own name; a built-in theme's is a locale key (`nameKey`)
  nameKey?: string;
  scheme: Scheme;
  tokens: Tokens;
  builtIn?: boolean;
}

const FONT_UI = "'Inter Variable', Inter, system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif";
const FONT_MONO = "ui-monospace, 'SF Mono', 'JetBrains Mono', Menlo, Consolas, monospace";
const SHAPE = { 'font-ui': FONT_UI, 'font-mono': FONT_MONO, 'text-16': '16px', 'space-1': '4px', 'radius-xs': '5px', 'radius-s': '8px', 'radius-m': '12px', 'radius-l': '18px' } as const;

// The two defaults are design/tokens.css itself (a test holds them equal), so the page looks the same before and after this runs.
const DARK: Tokens = {
  'surface-0': '#09090b', 'surface-1': '#111114', 'surface-2': '#1b1b20', border: '#26262c', text: '#f4f4f5', 'text-muted': '#a1a1aa',
  accent: '#8b93ff', 'on-accent': '#0b0b14', 'state-working': '#60a5fa', 'state-needs-you': '#fbbf24', 'state-finished': '#4ade80',
  'state-idle': '#8b8b96', 'state-unknown': '#71717a', ...SHAPE,
};
const LIGHT: Tokens = {
  'surface-0': '#f6f6f8', 'surface-1': '#ffffff', 'surface-2': '#efeff3', border: '#e1e1e8', text: '#17171c', 'text-muted': '#5b5b68',
  accent: '#4f46e5', 'on-accent': '#ffffff', 'state-working': '#1d6fd1', 'state-needs-you': '#a35a00', 'state-finished': '#15803d',
  'state-idle': '#5b5b68', 'state-unknown': '#71717a', ...SHAPE,
};
export const baseTokens = (scheme: Scheme): Tokens => (scheme === 'light' ? LIGHT : DARK);

const builtIn = (id: string, scheme: Scheme, tokens: Partial<Tokens> = {}): Theme =>
  ({ id, name: id, nameKey: `theme-name-${id}`, scheme, builtIn: true, tokens: { ...baseTokens(scheme), ...tokens } });

export const BUILT_IN: readonly Theme[] = [
  builtIn('dark', 'dark'),
  builtIn('light', 'light'),
  // Pure black and white, borders you can see, a yellow accent: for low vision and bright rooms.
  builtIn('high-contrast', 'dark', {
    'surface-0': '#000000', 'surface-1': '#0a0a0a', 'surface-2': '#1c1c1c', border: '#9a9a9a', text: '#ffffff', 'text-muted': '#e2e2e2',
    accent: '#ffd60a', 'on-accent': '#000000', 'state-working': '#7cc4ff', 'state-needs-you': '#ffb000', 'state-finished': '#4ff08d',
    'state-idle': '#d0d0d0', 'state-unknown': '#b4b4b4',
  }),
  // The dark colours with everything a quarter larger (text and the spacing around it).
  builtIn('large-text', 'dark', { 'text-16': '20px', 'radius-xs': '6px', 'radius-s': '10px', 'radius-m': '14px', 'radius-l': '20px' }),
  // A cool blue-grey dark with a teal accent.
  builtIn('slate', 'dark', {
    'surface-0': '#0f1419', 'surface-1': '#151c24', 'surface-2': '#1e2732', border: '#2a3541', text: '#e6edf3', 'text-muted': '#9fb0c0',
    accent: '#5cc8c2', 'on-accent': '#062320', 'state-working': '#7aa2f7', 'state-needs-you': '#e0af68', 'state-finished': '#9ece6a',
    'state-idle': '#8b98a5', 'state-unknown': '#738190',
  }),
  // A warm paper-like light with a terracotta accent and softer corners.
  builtIn('sand', 'light', {
    'surface-0': '#f5f1e8', 'surface-1': '#fbf8f2', 'surface-2': '#ece5d6', border: '#ddd3c0', text: '#2b2620', 'text-muted': '#5e5446',
    accent: '#9a4a1f', 'on-accent': '#ffffff', 'state-working': '#2f6690', 'state-needs-you': '#8a5300', 'state-finished': '#3b6e2a',
    'state-idle': '#5e5446', 'state-unknown': '#6c6356', 'radius-xs': '6px', 'radius-s': '10px', 'radius-m': '14px', 'radius-l': '20px',
  }),
];
export const DEFAULT_THEME: Record<Scheme, string> = { dark: 'dark', light: 'light' };

// ---- Values -------------------------------------------------------------------------------------------------------------------

const HEX = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i;
// A font list: family names, quoted or not, separated by commas. No CSS punctuation that could end the declaration.
const FONT = /^[A-Za-z0-9 _.,'"-]+$/;
const PX = /^(\d{1,3}(\.\d{1,2})?)px$/;

// A token's value in its canonical form (colours as lowercase #rrggbb, sizes as `Npx`), or undefined when it is not valid.
export function readValue(name: TokenName, v: unknown): string | undefined {
  const k = kindOf(name);
  if (k.kind === 'color') {
    if (typeof v !== 'string' || !HEX.test(v.trim())) return undefined;
    const h = v.trim().toLowerCase();
    return h.length === 4 ? `#${h[1]}${h[1]}${h[2]}${h[2]}${h[3]}${h[3]}` : h;
  }
  if (k.kind === 'font') {
    if (typeof v !== 'string') return undefined;
    const f = v.trim().replace(/\s+/g, ' ');
    return f.length > 0 && f.length <= MAX_FONT && FONT.test(f) && (f.match(/['"]/g)?.length ?? 0) % 2 === 0 ? f : undefined;
  }
  const n = typeof v === 'number' ? v : typeof v === 'string' && PX.test(v.trim()) ? Number(PX.exec(v.trim())?.[1]) : NaN;
  return Number.isFinite(n) && n >= k.min && n <= k.max ? `${Math.round(n * 100) / 100}px` : undefined;
}
export const pxOf = (v: string): number => Number.parseFloat(v);

// ---- Contrast (WCAG 2.2) ------------------------------------------------------------------------------------------------------

function luminance(hex: string): number {
  const c = [1, 3, 5].map((i) => Number.parseInt(hex.slice(i, i + 2), 16) / 255).map((v) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
  return 0.2126 * (c[0] as number) + 0.7152 * (c[1] as number) + 0.0722 * (c[2] as number);
}
export function contrast(a: string, b: string): number {
  const x = luminance(a);
  const y = luminance(b);
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}

// The pairs that must stay readable. Text (and the accent, which links use) needs 4.5:1 on every surface (AA, 1.4.3); state
// colours mark shapes that always come with a word, so they need 3:1 (AA, 1.4.11).
const SURFACES = ['surface-0', 'surface-1', 'surface-2'] as const;
export const CONTRAST_PAIRS: readonly { fg: TokenName; bg: TokenName; min: number }[] = [
  ...(['text', 'text-muted', 'accent'] as const).flatMap((fg) => SURFACES.map((bg) => ({ fg, bg, min: 4.5 }))),
  { fg: 'on-accent', bg: 'accent', min: 4.5 },
  ...(['state-working', 'state-needs-you', 'state-finished', 'state-idle', 'state-unknown'] as const).flatMap((fg) => SURFACES.map((bg) => ({ fg, bg, min: 3 }))),
];

export interface ContrastIssue { fg: TokenName; bg: TokenName; ratio: number; min: number }
// Rounded down to one decimal so a shown 4.5 always passes.
export const shownRatio = (r: number) => Math.floor(r * 10) / 10;
export function contrastIssues(tokens: Tokens): ContrastIssue[] {
  return CONTRAST_PAIRS.map((p) => ({ ...p, ratio: contrast(tokens[p.fg], tokens[p.bg]) })).filter((p) => p.ratio < p.min);
}

// ---- Theme files --------------------------------------------------------------------------------------------------------------

// The file people import and export. Tokens it leaves out come from the default theme of its scheme.
export interface ThemeFile { 'agora-theme': typeof THEME_FORMAT; name: string; scheme: Scheme; tokens: Partial<Tokens> }

export type ThemeIssue =
  | { code: 'json' | 'invalid' | 'newer' | 'name' | 'scheme' }
  | { code: 'unknown-field'; field: string }
  | { code: 'unknown-token'; token: string }
  | { code: 'derived-token'; token: string; base: TokenName }
  | { code: 'bad-value'; token: TokenName }
  | ({ code: 'contrast' } & ContrastIssue);
export type ParsedTheme = { ok: true; name: string; scheme: Scheme; tokens: Tokens } | { ok: false; issues: ThemeIssue[] };

const isObject = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
// Control and bidi characters never reach the screen in a name someone else wrote.
const cleanName = (v: unknown) => (typeof v === 'string' ? v.replace(/[\u0000-\u001f\u007f-\u009f‎‏‪-‮⁦-⁩]/g, '').trim() : '');

// Reads a theme file. Every problem is reported (not just the first), so a person fixing a file sees them all at once.
export function parseTheme(value: unknown): ParsedTheme {
  if (!isObject(value)) return { ok: false, issues: [{ code: 'invalid' }] };
  const format = value['agora-theme'];
  if (typeof format !== 'number' || !Number.isInteger(format) || format < 1) return { ok: false, issues: [{ code: 'invalid' }] };
  if (format > THEME_FORMAT) return { ok: false, issues: [{ code: 'newer' }] };
  const issues: ThemeIssue[] = [];
  for (const k of Object.keys(value)) if (!['agora-theme', 'name', 'scheme', 'tokens'].includes(k)) issues.push({ code: 'unknown-field', field: cleanName(k).slice(0, 40) });
  const name = cleanName(value.name);
  if (name.length === 0 || name.length > MAX_NAME) issues.push({ code: 'name' });
  const scheme = value.scheme;
  if (scheme !== 'dark' && scheme !== 'light') return { ok: false, issues: [...issues, { code: 'scheme' }] };
  if (value.tokens !== undefined && !isObject(value.tokens)) return { ok: false, issues: [...issues, { code: 'invalid' }] };
  const tokens: Tokens = { ...baseTokens(scheme) };
  for (const [k, v] of Object.entries(value.tokens ?? {})) {
    if (DERIVED[k]) { issues.push({ code: 'derived-token', token: k, base: DERIVED[k] as TokenName }); continue; }
    if (!isTokenName(k)) { issues.push({ code: 'unknown-token', token: cleanName(k).slice(0, 40) }); continue; }
    const read = readValue(k, v);
    if (read === undefined) issues.push({ code: 'bad-value', token: k });
    else tokens[k] = read;
  }
  // Contrast is judged only on colours that were all readable: a bad value already says what is wrong.
  if (!issues.some((i) => i.code === 'bad-value')) for (const c of contrastIssues(tokens)) issues.push({ code: 'contrast', ...c });
  return issues.length > 0 ? { ok: false, issues: dedupe(issues) } : { ok: true, name, scheme, tokens };
}

function dedupe(issues: ThemeIssue[]): ThemeIssue[] {
  const seen = new Set<string>();
  return issues.filter((i) => { const k = JSON.stringify(i); if (seen.has(k)) return false; seen.add(k); return true; });
}

export function deserialiseTheme(text: string): ParsedTheme {
  let value: unknown;
  try { value = JSON.parse(text); } catch { return { ok: false, issues: [{ code: 'json' }] }; }
  return parseTheme(value);
}

// The file for a theme: every token written out, so it reads the same in a client whose defaults differ.
export function themeFile(theme: Pick<Theme, 'scheme' | 'tokens'>, name: string): ThemeFile {
  return { 'agora-theme': THEME_FORMAT, name, scheme: theme.scheme, tokens: { ...theme.tokens } };
}
export const serialiseTheme = (file: ThemeFile) => JSON.stringify(file, null, 2);

// ---- Applying -----------------------------------------------------------------------------------------------------------------

// The CSS variables a theme sets on the page. Sizes become rem so the browser's own text size still counts: the base text
// size is a scale on the root font size, the spacing a multiple of it (Tailwind's spacing and text utilities are in rem).
export function cssVariables(theme: Pick<Theme, 'tokens'>): Record<string, string> {
  const vars: Record<string, string> = {};
  for (const n of TOKEN_NAMES) vars[`--${n}`] = theme.tokens[n];
  vars['--text-scale'] = String(pxOf(theme.tokens['text-16']) / 16);
  vars['--space-1'] = `${pxOf(theme.tokens['space-1']) / 16}rem`;
  return vars;
}

export interface StyleTarget {
  style: { setProperty(name: string, value: string): void; removeProperty(name: string): unknown };
  dataset: Record<string, string | undefined>;
}
// Dark and Light are design/tokens.css itself: for them the page's own rules are left to work (nothing set inline), so a script
// or the gallery that flips `data-theme` still sees both schemes. Any other theme sets its variables on the root.
export function applyTheme(root: StyleTarget, theme: Pick<Theme, 'tokens' | 'scheme'> & Partial<Pick<Theme, 'id' | 'builtIn'>>) {
  const plain = !!theme.builtIn && theme.id === DEFAULT_THEME[theme.scheme];
  for (const [k, v] of Object.entries(cssVariables(theme))) {
    if (plain) root.style.removeProperty(k);
    else root.style.setProperty(k, v);
  }
  root.dataset.theme = theme.scheme; // the scheme's shadows, `color-scheme` and the terminal's palette follow it
}
