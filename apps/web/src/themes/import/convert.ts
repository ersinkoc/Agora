// Theme import (docs/ux/themes.md "Import from another app", roadmap M5.1): a colour file from iTerm2, Alacritty, Ghostty, Windows
// Terminal or VS Code becomes a theme file of format 2 (ADR-0113). Each converter is pure and synchronous: text in, a format-2 file and
// notes out (what was left out or guessed), never an exception. The file it makes is built key by key from closed lists (no key of the
// input is copied through), and it is then read by `readTheme` like any hand-written theme, so the floor, the unsafe pieces, the
// readability pairs and the size cap hold for an import exactly as for a file. Nothing here touches the DOM but the property list
// parser (read.ts), and nothing reaches the network.
import { MAX_FILE_BYTES, THEME_FORMAT_2, cleanName, safeName } from '../file';
import { ANSI, ANSI_NAMES, SYNTAX_KEYS, TERMINAL_KEYS, type AnsiName, type SyntaxKey, type TerminalKey } from '../look';
import { MAX_NAME, type ColourToken, type Scheme, baseTokens, contrast, contrastIssues } from '../tokens';
import { type Flat, type PlistValue, readJsonc, readKeyValue, readPlist, readToml, readYaml } from './read';

export const IMPORT_FORMATS = ['iterm2', 'alacritty', 'ghostty', 'windows-terminal', 'vscode'] as const;
export type ImportFormat = (typeof IMPORT_FORMATS)[number];

// What a person is told about an import before it is added (locales/<lang>/web.ftl `theme-import-note-*`). `keys` are names from the
// file, cleaned of control and direction characters and cut short; React shows them as text.
export type ImportNote =
  | { code: 'scheme'; scheme: Scheme } // which of the person's two themes it fills
  | { code: 'derived' } // the window's colours were made from the terminal's
  | { code: 'derived-kept' } // ...and could not be made readable, so the window keeps Agora's own
  | { code: 'missing'; keys: string[]; more: number } // terminal colours the file does not give: Agora's own are used
  | { code: 'ignored'; keys: string[]; more: number } // what the file has that a theme has no place for
  | { code: 'unreadable'; keys: string[]; more: number } // colours that could not be read
  | { code: 'alpha'; keys: string[]; more: number } // see-through colours, mixed over the background
  | { code: 'background-alpha' } // a see-through background: the window is never see-through, so it is drawn solid
  | { code: 'selection-alpha' } // the selection's transparency: Agora draws the selection see-through itself
  | { code: 'colour-space'; space: 'p3' | 'other' } // iTerm2: converted from Display P3, or read as sRGB
  | { code: 'variant'; used: Scheme } // iTerm2: the file has light and dark colours; one set was used
  | { code: 'first-scheme'; count: number } // Windows Terminal: several schemes; the first was used
  | { code: 'fallback'; key: TerminalKey; from: string } // VS Code: a terminal colour taken from the editor's
  | { code: 'syntax'; count: number } // VS Code: code colours taken from its token colours
  | { code: 'window-colours'; count: number } // VS Code: its own window colours, left out (the window's are made from the terminal's)
  | { code: 'include' }; // VS Code: the theme builds on another file, which is not read

export type ImportError = 'too-large' | 'unknown-format' | 'unreadable' | 'entities' | 'no-colours';
export type Converted =
  | { ok: true; format: ImportFormat; scheme: Scheme; name: string; file: Record<string, unknown>; notes: ImportNote[] }
  | { ok: false; error: ImportError; format?: ImportFormat };

// A colour as a file gives it: sRGB components 0–255 and an alpha 0–1.
export interface Rgba { r: number; g: number; b: number; a: number }
// What a reader found, before the shared steps (alpha, scheme, derivation, notes) make it a theme.
interface Source {
  name?: unknown;
  colours: Map<TerminalKey, Rgba>;
  ignored: string[];
  unreadable: string[];
  notes: ImportNote[];
  syntax?: Map<SyntaxKey, Rgba>;
}
const source = (): Source => ({ colours: new Map(), ignored: [], unreadable: [], notes: [] });

// ---- Colours --------------------------------------------------------------------------------------------------------------------

const clamp = (n: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, n));
const byte = (n: number) => Math.round(clamp(n, 0, 255)).toString(16).padStart(2, '0');
export const hexOf = (c: Pick<Rgba, 'r' | 'g' | 'b'>) => `#${byte(c.r)}${byte(c.g)}${byte(c.b)}`;
const rgbOf = (hex: string): Rgba => ({ r: Number.parseInt(hex.slice(1, 3), 16), g: Number.parseInt(hex.slice(3, 5), 16), b: Number.parseInt(hex.slice(5, 7), 16), a: 1 });
// `b` mixed into `a` by `t` (0 is `a`), per sRGB component, as #rrggbb.
export function mix(a: string, b: string, t: number): string {
  const x = rgbOf(a);
  const y = rgbOf(b);
  return hexOf({ r: x.r + (y.r - x.r) * t, g: x.g + (y.g - x.g) * t, b: x.b + (y.b - x.b) * t });
}
// A colour drawn at its alpha over a solid background, as the screen shows it.
const over = (c: Rgba, bg: string) => mix(bg, hexOf(c), c.a);

// A hex colour as the five apps write them: `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, `0xrrggbb` (Alacritty), or bare `rrggbb` where
// the app allows it (Ghostty, Alacritty).
export function parseHex(v: unknown, bare = false): Rgba | undefined {
  if (typeof v !== 'string') return undefined;
  const m = /^(#|0x)?([0-9a-f]{3,8})$/i.exec(v.trim());
  if (!m || (!m[1] && !bare)) return undefined;
  const h = (m[2] as string).toLowerCase();
  const n = h.length;
  if (m[1] === '0x' && n !== 6) return undefined;
  if (n === 3 || n === 4) {
    const d = [...h].map((c) => Number.parseInt(c + c, 16));
    return { r: d[0] as number, g: d[1] as number, b: d[2] as number, a: n === 4 ? (d[3] as number) / 255 : 1 };
  }
  if (n === 6 || n === 8) {
    const d = [0, 2, 4, 6].map((i) => Number.parseInt(h.slice(i, i + 2), 16));
    return { r: d[0] as number, g: d[1] as number, b: d[2] as number, a: n === 8 ? (d[3] as number) / 255 : 1 };
  }
  return undefined;
}

// sRGB's transfer function and Display P3's (they share it), and P3's primaries in sRGB's (linear light; CSS Color 4).
const toLinear = (c: number) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
const fromLinear = (c: number) => (c <= 0.0031308 ? c * 12.92 : 1.055 * c ** (1 / 2.4) - 0.055);
const P3_TO_SRGB = [
  [1.2249401, -0.2249404, 0],
  [-0.0420569, 1.0420571, 0],
  [-0.0196376, -0.0786361, 1.0982735],
] as const;
function p3ToSrgb(r: number, g: number, b: number): [number, number, number] {
  const lin = [toLinear(r), toLinear(g), toLinear(b)];
  return P3_TO_SRGB.map((row) => fromLinear(clamp(row[0] * (lin[0] as number) + row[1] * (lin[1] as number) + row[2] * (lin[2] as number), 0, 1))) as [number, number, number];
}

// ---- Names ----------------------------------------------------------------------------------------------------------------------

// The theme's name: the file's own, else the file name without its extension, else the caller's. Each is untrusted: control and
// direction characters and the pieces a theme refuses (`;`, `<`, `url(` …) are removed, and it is cut to the longest name allowed.
export function importName(own: unknown, fileName: string, fallback: string): string {
  const base = fileName.replace(/^.*[\\/]/, '').replace(/\.(itermcolors|toml|ya?ml|json|jsonc|conf|config|theme)$/i, '');
  for (const c of [own, base, fallback]) {
    const clean = cleanName(c);
    if (clean === '') continue;
    // Cut before cleaning too: an imported name has no cap of its own, and cleaning runs until nothing is left to remove.
    const safe = safeName(clean.slice(0, 4 * MAX_NAME)).slice(0, MAX_NAME).trim();
    if (safe !== '' && safe !== '—') return safe;
  }
  return '—';
}
// A list of names from a file, as a note shows them: the first few, cleaned, and how many more.
const MAX_SHOWN = 6;
const shown = (keys: readonly string[]) => {
  const unique = [...new Set(keys.map((k) => cleanName(k).slice(0, 40)).filter((k) => k !== ''))];
  return { keys: unique.slice(0, MAX_SHOWN), more: Math.max(0, unique.length - MAX_SHOWN) };
};

// ---- The window's colours, made from the terminal's ------------------------------------------------------------------------------

// Format 2 has no rule that makes the window's colours from a terminal palette (a scheme's colours left out come from Dark or Light),
// so an import makes them here, conservatively: the background is the window's, the panels and raised surfaces a little of the text
// mixed into it, the lines a little more; the text is the terminal's text, the secondary text the text mixed toward the background;
// the accent and the states are the palette's blue, yellow, green, cyan and magenta. Each one is held to the pair `readTheme` asks of
// it (4.5:1 for text and the accent, 3:1 for a state, on all three surfaces): a colour that falls short is mixed toward the text until
// it reads. When even that cannot hold (the text itself is not readable on the background), there are no derived colours.
const SURFACE_STEPS = [[0.05, 0.1], [0.03, 0.06], [0.015, 0.03], [0, 0]] as const;
type Palette = Record<AnsiName, string> & { background: string; foreground: string };
export function deriveTokens(p: Palette): Record<ColourToken, string> | undefined {
  const bg = p.background;
  const fg = p.foreground;
  for (const [one, two] of SURFACE_STEPS) {
    const surfaces = [bg, mix(bg, fg, one), mix(bg, fg, two)];
    const worst = (c: string) => Math.min(...surfaces.map((s) => contrast(c, s)));
    if (worst(fg) < 4.5) continue;
    // The first of the candidates that reads at `want` (a margin, so the widget kit's tinted backgrounds read too), else the first
    // that reads at `min`, else the first mixed toward the text until it does (the text itself always reads).
    const pick = (candidates: readonly string[], min: number, want = min): string => {
      const ok = candidates.find((c) => worst(c) >= want) ?? candidates.find((c) => worst(c) >= min);
      if (ok) return ok;
      for (let i = 1; i <= 10; i++) { const m = mix(candidates[0] as string, fg, i / 10); if (worst(m) >= min) return m; }
      return fg;
    };
    const muted = pick([0.35, 0.3, 0.25, 0.2, 0.15, 0.1].map((t) => mix(fg, bg, t)), 4.5, 6);
    const accent = pick([p['bright-blue'], p.blue, p['bright-cyan'], p.cyan, p['bright-magenta'], p.magenta], 4.5);
    const onAccent = [bg, '#000000', '#ffffff'].reduce((a, b) => (contrast(b, accent) > contrast(a, accent) ? b : a));
    const tokens: Record<ColourToken, string> = {
      'surface-0': surfaces[0] as string, 'surface-1': surfaces[1] as string, 'surface-2': surfaces[2] as string,
      border: mix(bg, fg, Math.max(0.16, two + 0.08)), text: fg, 'text-muted': muted, accent, 'on-accent': onAccent,
      'state-working': pick([p.blue, p['bright-blue'], p.cyan], 3, 5),
      'state-needs-you': pick([p.yellow, p['bright-yellow']], 3, 5),
      'state-finished': pick([p.green, p['bright-green']], 3, 5),
      'state-idle': pick([mix(fg, bg, 0.4), muted], 3, 5),
      'state-unknown': pick([mix(fg, bg, 0.35), muted], 3, 5),
    };
    // The same pairs `readTheme` refuses on: a set that does not pass them is not offered.
    if (contrastIssues({ ...baseTokens('dark'), ...tokens }).length === 0) return tokens;
  }
  return undefined;
}

// ---- From what a reader found to a theme file -----------------------------------------------------------------------------------

// The scheme a background belongs to, by the validator's own rule (`backgroundTwin`): dark when white reads better on it.
export const schemeOf = (bg: string): Scheme => (contrast(bg, '#ffffff') > contrast(bg, '#000000') ? 'dark' : 'light');

function build(format: ImportFormat, src: Source, fileName: string, fallbackName: string): Converted {
  if (src.colours.size === 0) return { ok: false, error: 'no-colours', format };
  // The scheme and how the window's colours were made come first; then what the reader noticed, then what is missing or left out.
  const notes: ImportNote[] = [];
  const bgRaw = src.colours.get('background');
  const fgRaw = src.colours.get('foreground');
  const later: ImportNote[] = [...src.notes];
  if (bgRaw && bgRaw.a < 1) later.push({ code: 'background-alpha' });
  // Without a background, the text says which scheme (light text is a dark theme); without either, dark.
  const scheme: Scheme = bgRaw ? schemeOf(hexOf(bgRaw)) : fgRaw ? (schemeOf(hexOf(fgRaw)) === 'dark' ? 'light' : 'dark') : 'dark';
  const base = baseTokens(scheme);
  const bg = bgRaw ? hexOf(bgRaw) : base['surface-0'];
  const terminal: Partial<Record<TerminalKey, string>> = {};
  const seeThrough: string[] = [];
  for (const k of TERMINAL_KEYS) {
    const c = src.colours.get(k);
    if (!c) continue;
    if (k === 'background' || c.a >= 1) terminal[k] = hexOf(c);
    else if (k === 'selection') { terminal[k] = hexOf(c); later.push({ code: 'selection-alpha' }); }
    else { terminal[k] = over(c, bg); seeThrough.push(k); }
  }
  if (seeThrough.length > 0) later.push({ code: 'alpha', ...shown(seeThrough) });
  notes.push({ code: 'scheme', scheme });

  const palette = Object.fromEntries(ANSI_NAMES.map((n, i) => [n, terminal[n] ?? (ANSI[scheme][i] as string)])) as Palette;
  palette.background = bg;
  palette.foreground = terminal.foreground ?? base.text;
  const tokens = deriveTokens(palette);
  notes.push({ code: tokens ? 'derived' : 'derived-kept' }, ...later);

  const missing = TERMINAL_KEYS.filter((k) => !terminal[k]);
  if (missing.length > 0) notes.push({ code: 'missing', ...shown(missing) });
  if (src.unreadable.length > 0) notes.push({ code: 'unreadable', ...shown(src.unreadable) });
  if (src.ignored.length > 0) notes.push({ code: 'ignored', ...shown(src.ignored) });

  const section: Record<string, unknown> = {};
  if (tokens) section.tokens = tokens;
  section.terminal = Object.fromEntries(TERMINAL_KEYS.filter((k) => terminal[k]).map((k) => [k, terminal[k]]));
  if (src.syntax && src.syntax.size > 0) {
    const codeBg = tokens ? tokens['surface-1'] : base['surface-1'];
    section.syntax = Object.fromEntries(SYNTAX_KEYS.filter((k) => src.syntax?.has(k)).map((k) => [k, over(src.syntax?.get(k) as Rgba, codeBg)]));
    notes.push({ code: 'syntax', count: src.syntax.size });
  }
  const name = importName(src.name, fileName, fallbackName);
  return { ok: true, format, scheme, name, file: { 'agora-theme': THEME_FORMAT_2, name, [scheme]: section }, notes };
}

// ---- iTerm2 (.itermcolors, an XML property list) --------------------------------------------------------------------------------

const ITERM_KEYS: readonly [string, TerminalKey][] = [
  ...ANSI_NAMES.map((n, i): [string, TerminalKey] => [`Ansi ${i} Color`, n]),
  ['Background Color', 'background'], ['Foreground Color', 'foreground'], ['Cursor Color', 'cursor'], ['Selection Color', 'selection'],
];
function itermColour(v: PlistValue | undefined): { c?: Rgba; space?: 'p3' | 'other' } {
  if (v?.kind !== 'dict') return {};
  const num = (k: string) => { const x = v.entries.get(k); return x?.kind === 'number' ? x.value : undefined; };
  const r = num('Red Component');
  const g = num('Green Component');
  const b = num('Blue Component');
  if (r === undefined || g === undefined || b === undefined) return {};
  const a = num('Alpha Component') ?? 1;
  const spaceValue = v.entries.get('Color Space');
  const space = spaceValue?.kind === 'string' ? spaceValue.value : 'sRGB';
  const [sr, sg, sb] = space === 'P3' ? p3ToSrgb(clamp(r, 0, 1), clamp(g, 0, 1), clamp(b, 0, 1)) : [clamp(r, 0, 1), clamp(g, 0, 1), clamp(b, 0, 1)];
  return { c: { r: sr * 255, g: sg * 255, b: sb * 255, a: clamp(a, 0, 1) }, space: space === 'P3' ? 'p3' : space === 'sRGB' ? undefined : 'other' };
}
export function fromITerm2(text: string, fileName = '', fallbackName = ''): Converted {
  const read = readPlist(text);
  if (!read.ok) return { ok: false, error: read.why, format: 'iterm2' };
  const src = source();
  // iTerm2 3.5 can keep a light and a dark set (`Background Color (Dark)`): the plain keys if there are any, else the dark set, else
  // the light one. The other set is named among what was left out.
  const has = (suffix: string) => ITERM_KEYS.some(([k]) => read.top.has(`${k}${suffix}`));
  const suffix = has('') ? '' : has(' (Dark)') ? ' (Dark)' : has(' (Light)') ? ' (Light)' : '';
  if (suffix !== '') src.notes.push({ code: 'variant', used: suffix === ' (Dark)' ? 'dark' : 'light' });
  const used = new Set<string>();
  const spaces = new Set<'p3' | 'other'>();
  for (const [k, key] of ITERM_KEYS) {
    const name = `${k}${suffix}`;
    if (!read.top.has(name)) continue;
    used.add(name);
    const { c, space } = itermColour(read.top.get(name));
    if (!c) { src.unreadable.push(name); continue; }
    if (space) spaces.add(space);
    src.colours.set(key, c);
  }
  for (const s of spaces) src.notes.push({ code: 'colour-space', space: s });
  for (const k of read.top.keys()) if (!used.has(k)) src.ignored.push(k);
  return build('iterm2', src, fileName, fallbackName);
}

// ---- Alacritty (.toml, and the .yml of before 0.13) -----------------------------------------------------------------------------

const ALACRITTY_KEYS: readonly [string, TerminalKey][] = [
  ['colors.primary.background', 'background'], ['colors.primary.foreground', 'foreground'],
  ['colors.cursor.cursor', 'cursor'], ['colors.selection.background', 'selection'],
  ...ANSI_NAMES.map((n): [string, TerminalKey] => [`colors.${n.startsWith('bright-') ? 'bright' : 'normal'}.${n.replace('bright-', '')}`, n]),
];
// `copied`: keys a YAML alias was read through (`colors: *harbour` reads `schemes.harbour.…`): not left out, so not named.
function fromAlacrittyValues(values: Flat, skipped: string[], fileName: string, fallbackName: string, copied?: ReadonlySet<string>): Converted {
  const src = source();
  const used = new Set<string>();
  for (const [k, key] of ALACRITTY_KEYS) {
    const v = values.get(k);
    if (v === undefined) continue;
    used.add(k);
    // `CellForeground` / `CellBackground`: the cursor or selection takes the cell's colour; the nearest is the text or background.
    const cell = /^cell(foreground|background)$/i.exec(v);
    const c = cell ? parseHex(values.get(`colors.primary.${(cell[1] as string).toLowerCase()}`) ?? '', true) : parseHex(v, true);
    if (c) src.colours.set(key, c);
    else src.unreadable.push(k);
  }
  for (const k of values.keys()) if (!used.has(k) && !copied?.has(k)) src.ignored.push(k);
  src.ignored.push(...skipped);
  return build('alacritty', src, fileName, fallbackName);
}
export function fromAlacritty(text: string, fileName = '', fallbackName = ''): Converted {
  const r = readToml(text);
  return fromAlacrittyValues(r.values, r.skipped, fileName, fallbackName);
}
export function fromAlacrittyYaml(text: string, fileName = '', fallbackName = ''): Converted {
  const r = readYaml(text);
  return fromAlacrittyValues(r.values, r.skipped, fileName, fallbackName, r.copied);
}

// ---- Ghostty (a theme file: `key = value`) ---------------------------------------------------------------------------------------

const GHOSTTY_KEYS: Readonly<Record<string, TerminalKey>> = { background: 'background', foreground: 'foreground', 'cursor-color': 'cursor', 'selection-background': 'selection' };
export function fromGhostty(text: string, fileName = '', fallbackName = ''): Converted {
  const r = readKeyValue(text);
  const src = source();
  for (const [k, v] of r.pairs) {
    if (k === 'palette') {
      const m = /^(\d{1,3})\s*=\s*(.+)$/.exec(v);
      const i = m ? Number(m[1]) : -1;
      if (i < 0 || i > 255) { src.unreadable.push(`palette = ${v}`); continue; }
      if (i > 15) { src.ignored.push(`palette = ${i}`); continue; }
      const c = parseHex(scalarOf(m?.[2] ?? ''), true);
      if (c) src.colours.set(ANSI_NAMES[i] as AnsiName, c);
      else src.unreadable.push(`palette = ${i}`);
      continue;
    }
    const key = Object.prototype.hasOwnProperty.call(GHOSTTY_KEYS, k) ? GHOSTTY_KEYS[k] : undefined;
    if (!key) { src.ignored.push(k); continue; }
    // `cell-foreground` / `cell-background` (newer Ghostty): the nearest is the text or the background.
    const cell = /^cell-(foreground|background)$/i.exec(v);
    const c = cell ? (src.colours.get((cell[1] as string).toLowerCase() as TerminalKey) ?? undefined) : parseHex(v, true);
    if (c) src.colours.set(key, c);
    else src.unreadable.push(k);
  }
  src.ignored.push(...r.skipped);
  return build('ghostty', src, fileName, fallbackName);
}
const scalarOf = (s: string) => s.trim().replace(/^"(.*)"$/, '$1');

// ---- Windows Terminal (a `schemes` entry) ---------------------------------------------------------------------------------------

const WT_KEYS: readonly [string, TerminalKey][] = [
  ['background', 'background'], ['foreground', 'foreground'], ['cursorColor', 'cursor'], ['selectionBackground', 'selection'],
  ['black', 'black'], ['red', 'red'], ['green', 'green'], ['yellow', 'yellow'], ['blue', 'blue'], ['purple', 'magenta'], ['cyan', 'cyan'],
  ['white', 'white'], ['brightBlack', 'bright-black'], ['brightRed', 'bright-red'], ['brightGreen', 'bright-green'],
  ['brightYellow', 'bright-yellow'], ['brightBlue', 'bright-blue'], ['brightPurple', 'bright-magenta'], ['brightCyan', 'bright-cyan'],
  ['brightWhite', 'bright-white'],
];
const isObject = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const own = (o: Record<string, unknown>, k: string) => (Object.prototype.hasOwnProperty.call(o, k) ? o[k] : undefined);
const MAX_ENTRIES = 4096;
const keysOf = (o: Record<string, unknown>) => Object.keys(o).slice(0, MAX_ENTRIES);

function fromWindowsTerminalValue(value: unknown, fileName: string, fallbackName: string): Converted {
  const src = source();
  // A whole settings file (`schemes: [...]`) or a list of schemes: the first one.
  const list = Array.isArray(value) ? value : isObject(value) && Array.isArray(own(value, 'schemes')) ? (own(value, 'schemes') as unknown[]) : undefined;
  const scheme = list ? list.find(isObject) : value;
  if (!isObject(scheme)) return { ok: false, error: 'no-colours', format: 'windows-terminal' };
  if (list && list.length > 1) src.notes.push({ code: 'first-scheme', count: list.length });
  src.name = own(scheme, 'name');
  const known = new Set(['name', ...WT_KEYS.map(([k]) => k)]);
  for (const [k, key] of WT_KEYS) {
    const v = own(scheme, k);
    if (v === undefined) continue;
    const c = parseHex(v);
    if (c) src.colours.set(key, c);
    else src.unreadable.push(k);
  }
  for (const k of keysOf(scheme)) if (!known.has(k)) src.ignored.push(k);
  return build('windows-terminal', src, fileName, fallbackName);
}
export function fromWindowsTerminal(text: string, fileName = '', fallbackName = ''): Converted {
  const value = readJsonc(text);
  if (value === undefined) return { ok: false, error: 'unreadable', format: 'windows-terminal' };
  return fromWindowsTerminalValue(value, fileName, fallbackName);
}

// ---- VS Code (a colour theme: `colors` and `tokenColors`) -----------------------------------------------------------------------

const VSCODE_ANSI: Readonly<Record<AnsiName, string>> = Object.fromEntries(ANSI_NAMES.map((n) => [
  n, `terminal.ansi${n.split('-').map((w) => (w[0] as string).toUpperCase() + w.slice(1)).join('')}`,
])) as Record<AnsiName, string>;
const VSCODE_KEYS: readonly [string, TerminalKey][] = [
  ['terminal.background', 'background'], ['terminal.foreground', 'foreground'], ['terminalCursor.foreground', 'cursor'],
  ['terminal.selectionBackground', 'selection'], ...ANSI_NAMES.map((n): [string, TerminalKey] => [VSCODE_ANSI[n], n]),
];
// What VS Code draws a terminal in when the theme does not say: the panel's, then the editor's colours.
const VSCODE_FALLBACK: readonly [TerminalKey, readonly string[]][] = [
  ['background', ['panel.background', 'editor.background']], ['foreground', ['editor.foreground', 'foreground']],
];
// Code colours from TextMate scopes, most specific first: the first rule whose scope is one of these (or starts with one and a dot).
const SCOPES: Readonly<Record<SyntaxKey, readonly string[]>> = {
  keyword: ['keyword.control', 'keyword', 'storage.modifier'],
  string: ['string.quoted', 'string'],
  comment: ['comment', 'punctuation.definition.comment'],
  number: ['constant.numeric', 'constant'],
  function: ['entity.name.function', 'support.function'],
  type: ['entity.name.type', 'support.type', 'storage.type', 'entity.name.class'],
  attr: ['entity.other.attribute-name', 'variable.other.property', 'support.type.property-name'],
  meta: ['meta.preprocessor', 'keyword.control.directive', 'meta'],
};
const MAX_RULES = 2048;
function syntaxFrom(rules: unknown[]): Map<SyntaxKey, Rgba> {
  // Each rule's scopes (a string with commas, or a list) and its foreground.
  const read: { scopes: string[]; c: Rgba }[] = [];
  for (const r of rules.slice(0, MAX_RULES)) {
    if (!isObject(r)) continue;
    const settings = own(r, 'settings');
    const c = isObject(settings) ? parseHex(own(settings, 'foreground')) : undefined;
    if (!c) continue;
    const s = own(r, 'scope');
    const scopes = (typeof s === 'string' ? s.split(',') : Array.isArray(s) ? s.filter((x): x is string => typeof x === 'string') : []).slice(0, 64).map((x) => x.trim());
    read.push({ scopes, c });
  }
  const out = new Map<SyntaxKey, Rgba>();
  for (const k of SYNTAX_KEYS) {
    for (const want of SCOPES[k]) {
      const hit = read.find((r) => r.scopes.includes(want)) ?? read.find((r) => r.scopes.some((s) => s.startsWith(`${want}.`)));
      if (hit) { out.set(k, hit.c); break; }
    }
  }
  return out;
}
function fromVsCodeValue(value: unknown, fileName: string, fallbackName: string): Converted {
  if (!isObject(value)) return { ok: false, error: 'no-colours', format: 'vscode' };
  const src = source();
  src.name = own(value, 'name');
  const colors = own(value, 'colors');
  const c = isObject(colors) ? colors : {};
  const used = new Set<string>();
  for (const [k, key] of VSCODE_KEYS) {
    const v = own(c, k);
    if (v === undefined) continue;
    used.add(k);
    const rgba = parseHex(v);
    if (rgba) src.colours.set(key, rgba);
    else src.unreadable.push(k);
  }
  for (const [key, from] of VSCODE_FALLBACK) {
    if (src.colours.has(key)) continue;
    for (const k of from) {
      const rgba = parseHex(own(c, k));
      if (!rgba) continue;
      src.colours.set(key, rgba);
      used.add(k);
      src.notes.push({ code: 'fallback', key, from: k });
      break;
    }
  }
  const tokenColors = own(value, 'tokenColors');
  if (Array.isArray(tokenColors)) src.syntax = syntaxFrom(tokenColors);
  else if (tokenColors !== undefined) src.ignored.push('tokenColors');
  // VS Code's own window colours are many (a theme has a hundred or more): counted in one note, not named.
  const windowColours = Object.keys(c).filter((k) => !used.has(k)).length;
  if (windowColours > 0) src.notes.push({ code: 'window-colours', count: windowColours });
  if (own(value, 'include') !== undefined) src.notes.push({ code: 'include' });
  for (const k of keysOf(value)) if (!['name', 'colors', 'tokenColors', 'type', '$schema', 'include'].includes(k)) src.ignored.push(k);
  return build('vscode', src, fileName, fallbackName);
}
export function fromVsCode(text: string, fileName = '', fallbackName = ''): Converted {
  const value = readJsonc(text);
  if (value === undefined) return { ok: false, error: 'unreadable', format: 'vscode' };
  return fromVsCodeValue(value, fileName, fallbackName);
}

// ---- Which app a file is from ---------------------------------------------------------------------------------------------------

// By the file's extension where it says, else by its first lines: `<` an XML property list, `{` or `[` JSON (VS Code when it has
// `colors` or `tokenColors`, else Windows Terminal), TOML table headers or dotted `colors.` keys Alacritty, a top-level `colors:` its
// old YAML, and `palette =`, `background =` … Ghostty.
export function detectFormat(text: string, fileName = ''): ImportFormat | 'json' | 'alacritty-yaml' | undefined {
  const ext = /\.([a-z]+)$/i.exec(fileName)?.[1]?.toLowerCase();
  if (ext === 'itermcolors') return 'iterm2';
  if (ext === 'toml') return 'alacritty';
  if (ext === 'yml' || ext === 'yaml') return 'alacritty-yaml';
  if (ext === 'json' || ext === 'jsonc') return 'json';
  const head = text.replace(/^﻿/, '').slice(0, 4096);
  const first = head.trimStart()[0];
  if (first === '<') return 'iterm2';
  if (first === '{' || first === '[' && !/^\s*\[[A-Za-z_"']/.test(head)) return 'json';
  if (/^\s*\/[/*]/.test(head)) return 'json';
  if (/^\s*\[\s*colors[\].]/m.test(head) || /^\s*colors(\.[A-Za-z_]+)+\s*=/m.test(head)) return 'alacritty';
  if (/^colors\s*:/m.test(head) || /^\s+primary\s*:\s*$/m.test(head)) return 'alacritty-yaml';
  if (/^\s*(palette|background|foreground|cursor-color|selection-background)\s*=/m.test(head)) return 'ghostty';
  return undefined;
}

// The one entry point: the file's text and name in, a format-2 file and its notes out. The size cap is asked before anything is read.
// `fallbackName` is the name used when neither the file nor its name gives one (the caller's words, from the locale).
export function convertTheme(text: string, fileName = '', fallbackName = ''): Converted {
  try {
    if (new Blob([text]).size > MAX_FILE_BYTES) return { ok: false, error: 'too-large' };
    const kind = detectFormat(text, fileName);
    if (kind === 'iterm2') return fromITerm2(text, fileName, fallbackName);
    if (kind === 'alacritty') return fromAlacritty(text, fileName, fallbackName);
    if (kind === 'alacritty-yaml') return fromAlacrittyYaml(text, fileName, fallbackName);
    if (kind === 'ghostty') return fromGhostty(text, fileName, fallbackName);
    if (kind === 'json') {
      const value = readJsonc(text);
      if (value === undefined) return { ok: false, error: 'unreadable' };
      const vscode = isObject(value) && (own(value, 'colors') !== undefined || own(value, 'tokenColors') !== undefined);
      return vscode ? fromVsCodeValue(value, fileName, fallbackName) : fromWindowsTerminalValue(value, fileName, fallbackName);
    }
    return { ok: false, error: 'unknown-format' };
  } catch {
    // Every reader is written not to throw; this is the last guard (a stack or memory limit of the browser), so a hostile file is
    // refused rather than breaking the editor.
    return { ok: false, error: 'unreadable' };
  }
}
