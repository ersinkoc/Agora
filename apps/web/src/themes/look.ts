// What theme format 2 adds to the tokens of format 1 (ADR-0113, docs/ux/themes.md "Format, version 2"): the colours of the window's
// regions, the terminal's palette, optional syntax colours, the window's chrome and the interface text. Plain data: names, the kind
// of value each takes, and what the window draws when a theme leaves one out. Every value here reaches the page as one CSS variable
// set per name (tokens.ts `applyTheme`) or as a data attribute whose value is one of the lists below; nothing here touches the DOM.
import type { Scheme } from './tokens';

export const hasOwn = (o: object, k: string) => Object.prototype.hasOwnProperty.call(o, k);

// ---- Regions -------------------------------------------------------------------------------------------------------------------

// The parts of the window a theme may colour on their own, in the order the editor shows them. Each has a surface, a text and a
// line colour, each optional: a region a theme leaves alone is drawn in today's tokens (`REGION_DEFAULT`).
export const REGIONS = [
  'sidebar', 'tab-bar', 'pane-bar', 'status-line', 'cards', 'inputs', 'buttons', 'agent-entries', 'person-entries', 'code-blocks',
] as const;
export type RegionName = (typeof REGIONS)[number];
export const REGION_PARTS = ['surface', 'text', 'line'] as const;
export type RegionPart = (typeof REGION_PARTS)[number];
export type RegionColours = Partial<Record<RegionPart, string>>;
export type Regions = Partial<Record<RegionName, RegionColours>>;
export const isRegion = (n: string): n is RegionName => (REGIONS as readonly string[]).includes(n);
export const isRegionPart = (n: string): n is RegionPart => (REGION_PARTS as readonly string[]).includes(n);
// The CSS variable of a region's part (`--sidebar-surface`). app.css reads it with today's token as the fallback.
export const regionVar = (r: RegionName, p: RegionPart) => `--${r}-${p}`;

// What a region is drawn in when the theme does not say: a theme token, or a colour the kit mixes from them (`tint-accent`,
// `line-accent`). The readability check uses these for the parts a theme leaves out. The agent's entries have no surface of their
// own today (they sit on the pane): their surface is the pane's background, their line none.
export type RegionDefault = 'surface-0' | 'surface-1' | 'surface-2' | 'text' | 'text-muted' | 'border' | 'tint-accent' | 'line-accent';
export const REGION_DEFAULT: Readonly<Record<RegionName, Readonly<Record<RegionPart, RegionDefault>>>> = {
  sidebar: { surface: 'surface-1', text: 'text', line: 'border' },
  'tab-bar': { surface: 'surface-0', text: 'text', line: 'border' }, // the tab in front, and the lines between tabs
  'pane-bar': { surface: 'surface-1', text: 'text', line: 'border' },
  'status-line': { surface: 'surface-1', text: 'text-muted', line: 'border' },
  cards: { surface: 'surface-1', text: 'text', line: 'border' },
  inputs: { surface: 'surface-1', text: 'text', line: 'border' },
  buttons: { surface: 'surface-2', text: 'text', line: 'border' },
  'agent-entries': { surface: 'surface-0', text: 'text', line: 'border' },
  'person-entries': { surface: 'tint-accent', text: 'text', line: 'line-accent' },
  'code-blocks': { surface: 'surface-1', text: 'text', line: 'border' },
};

// ---- Terminal ------------------------------------------------------------------------------------------------------------------

// The 16 colours programs draw with, in their usual order, and the four the terminal itself draws.
export const ANSI_NAMES = [
  'black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white',
  'bright-black', 'bright-red', 'bright-green', 'bright-yellow', 'bright-blue', 'bright-magenta', 'bright-cyan', 'bright-white',
] as const;
export type AnsiName = (typeof ANSI_NAMES)[number];
export const TERMINAL_KEYS = ['background', 'foreground', 'cursor', 'selection', ...ANSI_NAMES] as const;
export type TerminalKey = (typeof TERMINAL_KEYS)[number];
export type TerminalColours = Partial<Record<TerminalKey, string>>;
export const isTerminalKey = (n: string): n is TerminalKey => (TERMINAL_KEYS as readonly string[]).includes(n);
export const terminalVar = (k: TerminalKey) => `--terminal-${k}`;
// The palette a terminal uses when the theme gives none: one per scheme, readable on that scheme's background.
export const ANSI: Readonly<Record<Scheme, readonly string[]>> = {
  dark: ['#3b3b44', '#f87171', '#4ade80', '#fbbf24', '#60a5fa', '#c084fc', '#22d3ee', '#d4d4d8', '#71717a', '#fca5a5', '#86efac', '#fde68a', '#93c5fd', '#d8b4fe', '#67e8f9', '#fafafa'],
  light: ['#27272a', '#b91c1c', '#15803d', '#a16207', '#1d4ed8', '#7e22ce', '#0e7490', '#52525b', '#52525b', '#dc2626', '#16a34a', '#ca8a04', '#2563eb', '#9333ea', '#0891b2', '#18181b'],
};
// The terminal's background stands in for one ANSI colour: black on a dark background, white on a light one. That one is never
// asked to stand out from it.
export const backgroundTwin = (darkBackground: boolean): AnsiName => (darkBackground ? 'black' : 'white');

// ---- Syntax colours ------------------------------------------------------------------------------------------------------------

// Code colours (app.css `--syntax-*`): mixed from the theme's own colours unless a theme sets them.
export const SYNTAX_KEYS = ['keyword', 'string', 'comment', 'number', 'function', 'type', 'attr', 'meta'] as const;
export type SyntaxKey = (typeof SYNTAX_KEYS)[number];
export type SyntaxColours = Partial<Record<SyntaxKey, string>>;
export const isSyntaxKey = (n: string): n is SyntaxKey => (SYNTAX_KEYS as readonly string[]).includes(n);
export const syntaxVar = (k: SyntaxKey) => `--syntax-${k}`;

// ---- Chrome --------------------------------------------------------------------------------------------------------------------

type ListKind<V extends string> = { kind: 'list'; values: readonly V[]; default: V };
type PxKind = { kind: 'px'; min: number; max: number; default: number };
const list = <V extends string>(values: readonly V[], d: V): ListKind<V> => ({ kind: 'list', values, default: d });
// How the window's frame is drawn. Every value is a word from a list or a whole number of pixels in a range: never CSS.
export const CHROME = {
  'pane-borders': list(['auto', 'always', 'off'] as const, 'auto'),
  'pane-gap': { kind: 'px', min: 0, max: 16, default: 0 },
  'line-width': { kind: 'px', min: 0, max: 2, default: 1 },
  'tab-bar': list(['top', 'bottom'] as const, 'top'),
  'tab-style': list(['underline', 'filled', 'pill'] as const, 'underline'),
  density: list(['compact', 'normal', 'roomy'] as const, 'normal'),
  'state-display': list(['symbol', 'dot', 'word'] as const, 'symbol'),
  // One icon set exists today; the field is there so a theme can name it, and a later set is one more word.
  icons: list(['lines'] as const, 'lines'),
  shadow: list(['none', 'soft', 'strong'] as const, 'soft'),
  motion: list(['system', 'reduced', 'off'] as const, 'system'),
} as const satisfies Record<string, ListKind<string> | PxKind>;
export type ChromeName = keyof typeof CHROME;
export const CHROME_NAMES = Object.keys(CHROME) as ChromeName[];
type ValueOf<K> = K extends PxKind ? number : K extends ListKind<infer V> ? V : never;
export type Chrome = { [K in ChromeName]: ValueOf<(typeof CHROME)[K]> };
export const DEFAULT_CHROME: Chrome = Object.fromEntries(CHROME_NAMES.map((n) => [n, CHROME[n].default])) as Chrome;
export const isChromeName = (n: string): n is ChromeName => hasOwn(CHROME, n);
// The data attribute a list value is shown by (`data-tab-style` on <html>), as a `dataset` key.
export const chromeData = (n: ChromeName) => n.replace(/-([a-z])/g, (_, c: string) => c.toUpperCase());

// ---- Interface text ------------------------------------------------------------------------------------------------------------

export const WEIGHTS = [300, 400, 500] as const;
export type Weight = (typeof WEIGHTS)[number];
export const LINE_HEIGHT = { min: 1.2, max: 1.8 } as const;
// `line-height` "normal" keeps each text size's own line height (today's); a number sets one for the interface's text.
export interface TextStyle { weight: Weight; 'line-height': number | 'normal'; ligatures: boolean }
export const DEFAULT_TEXT: TextStyle = { weight: 400, 'line-height': 'normal', ligatures: true };
export const TEXT_FIELDS = ['font-ui', 'font-mono', 'weight', 'line-height', 'ligatures'] as const;

// ---- One scheme's colours ------------------------------------------------------------------------------------------------------

// What a theme sets beyond the format-1 tokens, for one scheme. Absent entries are drawn in the defaults above.
export interface LookExtras { regions: Regions; terminal: TerminalColours; syntax: SyntaxColours }
export const noExtras = (): LookExtras => ({ regions: {}, terminal: {}, syntax: {} });

// ---- The floor (ADR-0113 decision 3) -------------------------------------------------------------------------------------------

// What no theme can change, by the name a file would use for it: setting one refuses the theme, saying why. Matched exactly as a
// key (a format-1 token like `state-needs-you` or `accent` is not one of these).
export const FLOOR = {
  danger: 'danger', 'on-danger': 'danger', 'diff-removed': 'diff-removed',
  'approval-card': 'approval-card', approval: 'approval-card', 'focus-ring': 'focus-ring', focus: 'focus-ring',
  'needs-you': 'needs-you', attention: 'needs-you', 'plugin-marker': 'plugin-marker',
} as const satisfies Record<string, string>;
export type FloorItem = (typeof FLOOR)[keyof typeof FLOOR];
export const FLOOR_ITEMS: readonly FloorItem[] = ['danger', 'diff-removed', 'approval-card', 'focus-ring', 'needs-you', 'plugin-marker'];
export const floorOf = (key: string): FloorItem | undefined => (hasOwn(FLOOR, key) ? FLOOR[key as keyof typeof FLOOR] : undefined);
// The smallest text the window draws is three quarters of the base size (`text-12`), and no text may be under 12 px.
export const MIN_BASE_TEXT = 16;
// Pieces that never belong in a theme value: they could end a declaration or reach outside the page.
export const UNSAFE = [/url\(/i, /@import/i, /;/, /\}/, /expression\(/i, /</] as const;
export const isUnsafe = (s: string) => UNSAFE.some((r) => r.test(s));
