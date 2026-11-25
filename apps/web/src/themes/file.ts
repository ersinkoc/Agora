// Theme files, both formats (docs/ux/themes.md; format 2 is ADR-0113). This is the one door from untrusted text (an imported file,
// this browser's storage, an edit) into a theme of either format: it never throws, reports every problem at once, refuses unknown
// fields at every level, values outside their kind, anything that names what no theme may change (the floor), and any value with a
// piece that could become CSS or reach outside the page. A format-1 file is read by tokens.ts `parseTheme` exactly as before and
// becomes a one-scheme theme; every theme is written back as format 2. Nothing here touches the DOM.
import {
  ANSI, ANSI_NAMES, CHROME, CHROME_NAMES, DEFAULT_CHROME, DEFAULT_TEXT, LINE_HEIGHT, MIN_BASE_TEXT, REGIONS, REGION_DEFAULT, REGION_PARTS,
  SYNTAX_KEYS, TERMINAL_KEYS, TEXT_FIELDS, UNSAFE, WEIGHTS, type AnsiName, type Chrome, type ChromeName, type LookExtras, type RegionColours,
  type RegionDefault, type RegionName, type Regions, type SyntaxColours, type TerminalColours, type TextStyle, type Weight, backgroundTwin,
  floorOf, hasOwn, isChromeName, isRegion, isRegionPart, isSyntaxKey, isTerminalKey, isUnsafe,
} from './look';
import { roleMix, tints } from './tints';
// How much of the accent app.css mixes into the panels for a hairline (`--line`).
const LINE_STRENGTH = 0.4;
import {
  BUILT_IN, COLOR_TOKENS, DERIVED, MAX_NAME, type Look, type Looks, type Scheme, type Theme, type ThemeIssue, type TokenName,
  type Tokens, baseTokens, chromeOf, contrast, contrastIssues, isTokenName, looksOf, parseTheme, readValue, textOf, themeWarnings,
} from './tokens';

export const THEME_FORMAT_2 = 2;
export const SCHEMES = ['dark', 'light'] as const satisfies readonly Scheme[];
// The tokens a format-2 file keeps outside the schemes, shared by both: sizes and corners in `shape`, the fonts in `text`.
export const SHAPE_TOKENS = ['text-16', 'space-1', 'radius-xs', 'radius-s', 'radius-m', 'radius-l'] as const satisfies readonly TokenName[];
export const FONT_TOKENS = ['font-ui', 'font-mono'] as const satisfies readonly TokenName[];
const isColourToken = (n: string) => (COLOR_TOKENS as readonly string[]).includes(n);
const isShapeToken = (n: string) => (SHAPE_TOKENS as readonly string[]).includes(n);
const isFontToken = (n: string) => (FONT_TOKENS as readonly string[]).includes(n);

// A theme as read from a file, before it has an id.
export type ThemeData = Required<Pick<Theme, 'looks' | 'chrome' | 'text'>> & Pick<Theme, 'name' | 'scheme' | 'tokens'>;
export type ReadTheme = { ok: true; theme: ThemeData; warnings?: ThemeIssue[] } | { ok: false; issues: ThemeIssue[] };

const isObject = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const cleanName = (v: unknown) => (typeof v === 'string' ? v.replace(/[\u0000-\u001f\u007f-\u009f‎‏‪-‮⁦-⁩]/g, '').trim() : '');
const shortKey = (k: string) => cleanName(k).slice(0, 40);
const HEX = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i;
function readColour(v: unknown): string | undefined {
  if (typeof v !== 'string' || !HEX.test(v.trim())) return undefined;
  const h = v.trim().toLowerCase();
  return h.length === 4 ? `#${h[1]}${h[1]}${h[2]}${h[2]}${h[3]}${h[3]}` : h;
}
// A whole number of pixels in a range: the number, or "6px".
function readPx(v: unknown, min: number, max: number): number | undefined {
  const n = typeof v === 'number' ? v : typeof v === 'string' && /^\d{1,3}px$/.test(v.trim()) ? Number.parseInt(v, 10) : NaN;
  return Number.isInteger(n) && n >= min && n <= max ? n : undefined;
}

// The pieces of a format-1 name that a format-2 file refuses: removed when a format-1 theme becomes format 2, so every theme format
// 1 accepts stays one format 2 accepts (the name never reaches CSS; it is text on the page).
const UNSAFE_ALL = UNSAFE.map((r) => new RegExp(r.source, `${r.flags}g`));
// A name of nothing but those pieces becomes a dash (a mark, not a word: no language's text lives in code).
const safeName = (name: string) => UNSAFE_ALL.reduce((n, r) => n.replace(r, ''), name).replace(/\s+/g, ' ').trim() || '—';

// ---- Reading --------------------------------------------------------------------------------------------------------------------

export function readTheme(value: unknown): ReadTheme {
  if (!isObject(value)) return { ok: false, issues: [{ code: 'invalid' }] };
  const format = value['agora-theme'];
  if (typeof format !== 'number' || !Number.isInteger(format) || format < 1) return { ok: false, issues: [{ code: 'invalid' }] };
  if (format > THEME_FORMAT_2) return { ok: false, issues: [{ code: 'newer' }] };
  return format === 1 ? readFormat1(value) : readFormat2(value);
}
export function deserialise(text: string): ReadTheme {
  let value: unknown;
  try { value = JSON.parse(text); } catch { return { ok: false, issues: [{ code: 'json' }] }; }
  return readTheme(value);
}

// Format 1, read as before (tokens.ts), plus what the floor asks of it: the base text size is lifted to 16 px, with a warning, so no
// text is under 12 px; and a token that names what no theme may change says so instead of "not a theme token".
function readFormat1(value: Record<string, unknown>): ReadTheme {
  const floor: ThemeIssue[] = [];
  if (isObject(value.tokens)) for (const k of Object.keys(value.tokens)) { const item = floorOf(k); if (item) floor.push({ code: 'floor', field: `tokens.${k}`, item }); }
  const p = parseTheme(value);
  if (!p.ok) {
    const named = new Set(floor.map((f) => (f.code === 'floor' ? f.field.slice('tokens.'.length) : '')));
    const rest = p.issues.filter((i) => !(i.code === 'unknown-token' && named.has(i.token)));
    return { ok: false, issues: [...floor, ...rest] };
  }
  if (floor.length > 0) return { ok: false, issues: floor };
  const warnings: ThemeIssue[] = [...(p.warnings ?? [])];
  const tokens = { ...p.tokens };
  if (Number.parseFloat(tokens['text-16']) < MIN_BASE_TEXT) {
    warnings.push({ code: 'text-lifted', from: tokens['text-16'], to: `${MIN_BASE_TEXT}px` });
    tokens['text-16'] = `${MIN_BASE_TEXT}px`;
  }
  const theme: ThemeData = {
    name: safeName(p.name), scheme: p.scheme, tokens,
    looks: { [p.scheme]: { tokens, regions: {}, terminal: {}, syntax: {} } }, chrome: { ...DEFAULT_CHROME }, text: { ...DEFAULT_TEXT },
  };
  return warnings.length > 0 ? { ok: true, theme, warnings } : { ok: true, theme };
}

const TOP_FIELDS = ['agora-theme', 'name', 'inherits', 'dark', 'light', 'shape', 'text', 'chrome'];
const LOOK_FIELDS = ['tokens', 'regions', 'terminal', 'syntax'];
const MAX_DEPTH = 6;

// Every string in a file, with where it is, so a value with a piece that could become CSS is refused wherever it is.
function strings(v: unknown, path: string, depth: number, out: { path: string; value: string }[]): boolean {
  if (depth > MAX_DEPTH) return false;
  if (typeof v === 'string') out.push({ path, value: v });
  else if (Array.isArray(v)) { for (let i = 0; i < v.length; i++) if (!strings(v[i], `${path}[${i}]`, depth + 1, out)) return false; }
  else if (isObject(v)) { for (const [k, x] of Object.entries(v)) if (!strings(x, path ? `${path}.${shortKey(k)}` : shortKey(k), depth + 1, out)) return false; }
  return true;
}

function readFormat2(value: Record<string, unknown>): ReadTheme {
  const issues: ThemeIssue[] = [];
  const found: { path: string; value: string }[] = [];
  if (!strings(value, '', 0, found)) return { ok: false, issues: [{ code: 'invalid' }] };
  const unsafe = new Set(found.filter((s) => isUnsafe(s.value)).map((s) => s.path));
  for (const field of unsafe) issues.push({ code: 'unsafe', field });
  // A field whose value was refused as unsafe is not judged again as a value.
  const ok = (path: string) => !unsafe.has(path);
  const floorKey = (key: string, field: string) => { const item = floorOf(key); if (item) issues.push({ code: 'floor', field, item }); return !!item; };
  const unknown = (field: string) => issues.push({ code: 'unknown-field', field: shortKey(field) });

  for (const k of Object.keys(value)) if (!TOP_FIELDS.includes(k) && !floorKey(k, k)) unknown(k);
  const name = cleanName(value.name);
  if (ok('name') && (name.length === 0 || name.length > MAX_NAME)) issues.push({ code: 'name' });

  // `inherits`: a built-in theme whose values stand in for the ones this file leaves out (for its own scheme, and its shape and fonts).
  let base: Theme | undefined;
  if (value.inherits !== undefined) {
    base = typeof value.inherits === 'string' ? BUILT_IN.find((b) => b.id === value.inherits) : undefined;
    if (!base && ok('inherits')) issues.push({ code: 'inherits', id: typeof value.inherits === 'string' ? shortKey(value.inherits) : '' });
  }

  const group = (v: unknown, field: string): Record<string, unknown> | undefined => {
    if (v === undefined) return undefined;
    if (isObject(v)) return v;
    if (ok(field)) issues.push({ code: 'bad-field', field, expect: 'group' });
    return undefined;
  };
  const colour = (v: unknown, field: string): string | undefined => {
    const c = readColour(v);
    if (c === undefined && ok(field)) issues.push({ code: 'bad-field', field, expect: 'color' });
    return c;
  };

  // Shape and fonts: shared by both schemes.
  const shared: Partial<Tokens> = {};
  const shape = group(value.shape, 'shape');
  for (const [k, v] of Object.entries(shape ?? {})) {
    const field = `shape.${k}`;
    if (floorKey(k, field)) continue;
    if (hasOwn(DERIVED, k)) { issues.push({ code: 'derived-token', token: k, base: DERIVED[k] as TokenName }); continue; }
    if (!isShapeToken(k)) { if (isFontToken(k)) issues.push({ code: 'moved-token', field, token: k, to: 'text' }); else if (isColourToken(k)) issues.push({ code: 'moved-token', field, token: k, to: 'tokens' }); else unknown(field); continue; }
    if (!ok(field)) continue;
    const read = readValue(k as TokenName, v);
    if (read === undefined) issues.push({ code: 'bad-value', token: k as TokenName });
    else if (k === 'text-16' && Number.parseFloat(read) < MIN_BASE_TEXT) issues.push({ code: 'text-floor', field, min: MIN_BASE_TEXT });
    else shared[k as TokenName] = read;
  }
  const text: TextStyle = { ...DEFAULT_TEXT };
  const textGroup = group(value.text, 'text');
  for (const [k, v] of Object.entries(textGroup ?? {})) {
    const field = `text.${k}`;
    if (floorKey(k, field)) continue;
    if (!(TEXT_FIELDS as readonly string[]).includes(k)) { if (isShapeToken(k)) issues.push({ code: 'moved-token', field, token: k, to: 'shape' }); else unknown(field); continue; }
    if (!ok(field)) continue;
    if (isFontToken(k)) {
      const read = readValue(k as TokenName, v);
      if (read === undefined) issues.push({ code: 'bad-value', token: k as TokenName });
      else shared[k as TokenName] = read;
    } else if (k === 'weight') {
      if (typeof v === 'number' && (WEIGHTS as readonly number[]).includes(v)) text.weight = v as Weight;
      else issues.push({ code: 'bad-field', field, expect: 'list', values: WEIGHTS.join(', ') });
    } else if (k === 'line-height') {
      if (v === 'normal') text['line-height'] = 'normal';
      else if (typeof v === 'number' && Number.isFinite(v) && v >= LINE_HEIGHT.min && v <= LINE_HEIGHT.max) text['line-height'] = Math.round(v * 100) / 100;
      else issues.push({ code: 'bad-field', field, expect: 'line-height', min: LINE_HEIGHT.min, max: LINE_HEIGHT.max });
    } else if (k === 'ligatures') {
      if (typeof v === 'boolean') text.ligatures = v;
      else issues.push({ code: 'bad-field', field, expect: 'switch' });
    }
  }

  const chrome: Chrome = { ...DEFAULT_CHROME };
  const chromeGroup = group(value.chrome, 'chrome');
  for (const [k, v] of Object.entries(chromeGroup ?? {})) {
    const field = `chrome.${k}`;
    if (floorKey(k, field)) continue;
    if (!isChromeName(k)) { unknown(field); continue; }
    if (!ok(field)) continue;
    const kind = CHROME[k];
    if (kind.kind === 'px') {
      const n = readPx(v, kind.min, kind.max);
      if (n === undefined) issues.push({ code: 'bad-field', field, expect: 'size', min: kind.min, max: kind.max });
      else (chrome as Record<ChromeName, unknown>)[k] = n;
    } else {
      const values = kind.values as readonly string[];
      // The value kept is the list's own word, never the file's string.
      const i = typeof v === 'string' ? values.indexOf(v) : -1;
      if (i < 0) issues.push({ code: 'bad-field', field, expect: 'list', values: values.join(', ') });
      else (chrome as Record<ChromeName, unknown>)[k] = values[i];
    }
  }

  // The schemes: each section present, and the scheme of the theme it inherits from.
  const raw: Partial<Record<Scheme, { tokens: Partial<Tokens>; extras: LookExtras }>> = {};
  for (const scheme of SCHEMES) {
    const section = group(value[scheme], scheme);
    if (!section && !(base && base.scheme === scheme && value[scheme] === undefined)) continue;
    const own: Partial<Tokens> = {};
    const extras: LookExtras = { regions: {}, terminal: {}, syntax: {} };
    for (const k of Object.keys(section ?? {})) if (!LOOK_FIELDS.includes(k) && !floorKey(k, `${scheme}.${k}`)) unknown(`${scheme}.${k}`);
    for (const [k, v] of Object.entries(group(section?.tokens, `${scheme}.tokens`) ?? {})) {
      const field = `${scheme}.tokens.${k}`;
      if (floorKey(k, field)) continue;
      if (hasOwn(DERIVED, k)) { issues.push({ code: 'derived-token', token: k, base: DERIVED[k] as TokenName }); continue; }
      if (!isTokenName(k)) { issues.push({ code: 'unknown-token', token: shortKey(k) }); continue; }
      if (!isColourToken(k)) { issues.push({ code: 'moved-token', field, token: k, to: isFontToken(k) ? 'text' : 'shape' }); continue; }
      if (!ok(field)) continue;
      const c = readColour(v);
      if (c === undefined) issues.push({ code: 'bad-value', token: k });
      else own[k] = c;
    }
    for (const [r, v] of Object.entries(group(section?.regions, `${scheme}.regions`) ?? {})) {
      const field = `${scheme}.regions.${r}`;
      if (floorKey(r, field)) continue;
      if (!isRegion(r)) { unknown(field); continue; }
      const parts: RegionColours = {};
      for (const [p, c] of Object.entries(group(v, field) ?? {})) {
        if (floorKey(p, `${field}.${p}`)) continue;
        if (!isRegionPart(p)) { unknown(`${field}.${p}`); continue; }
        if (!ok(`${field}.${p}`)) continue;
        const read = colour(c, `${field}.${p}`);
        if (read) parts[p] = read;
      }
      if (Object.keys(parts).length > 0) extras.regions[r] = parts;
    }
    for (const [k, c] of Object.entries(group(section?.terminal, `${scheme}.terminal`) ?? {})) {
      const field = `${scheme}.terminal.${k}`;
      if (floorKey(k, field)) continue;
      if (!isTerminalKey(k)) { unknown(field); continue; }
      if (!ok(field)) continue;
      const read = colour(c, field);
      if (read) extras.terminal[k] = read;
    }
    for (const [k, c] of Object.entries(group(section?.syntax, `${scheme}.syntax`) ?? {})) {
      const field = `${scheme}.syntax.${k}`;
      if (floorKey(k, field)) continue;
      if (!isSyntaxKey(k)) { unknown(field); continue; }
      if (!ok(field)) continue;
      const read = colour(c, field);
      if (read) extras.syntax[k] = read;
    }
    raw[scheme] = { tokens: own, extras };
  }
  if (!raw.dark && !raw.light && issues.length === 0) issues.push({ code: 'no-look' });
  if (issues.length > 0) return { ok: false, issues: dedupe(issues) };

  // Every value is readable: resolve each scheme from its base, then judge readability.
  const looks: Looks = {};
  for (const scheme of SCHEMES) {
    const r = raw[scheme];
    if (!r) continue;
    const from = base && base.scheme === scheme ? base.tokens : baseTokens(scheme);
    const shapeFrom = base ? Object.fromEntries([...SHAPE_TOKENS, ...FONT_TOKENS].map((n) => [n, base.tokens[n]])) : {};
    looks[scheme] = { tokens: { ...from, ...shapeFrom, ...shared, ...r.tokens }, ...r.extras };
  }
  const data = themeData(name, looks, chrome, text);
  const judged = judge(data);
  if (judged.refusals.length > 0) return { ok: false, issues: judged.refusals };
  return judged.warnings.length > 0 ? { ok: true, theme: data, warnings: judged.warnings } : { ok: true, theme: data };
}

function dedupe(issues: ThemeIssue[]): ThemeIssue[] {
  const seen = new Set<string>();
  return issues.filter((i) => { const k = JSON.stringify(i); if (seen.has(k)) return false; seen.add(k); return true; });
}

// A theme's data from its looks: `scheme` and `tokens` are its first scheme's (dark before light).
export function themeData(name: string, looks: Looks, chrome: Chrome, text: TextStyle): ThemeData {
  const scheme: Scheme = looks.dark ? 'dark' : 'light';
  return { name, scheme, tokens: (looks[scheme] as Look).tokens, looks, chrome, text };
}

// ---- Readability ----------------------------------------------------------------------------------------------------------------

const STATES = ['state-working', 'state-needs-you', 'state-finished', 'state-idle', 'state-unknown'] as const;
// The colour a region part is drawn in for one scheme: the theme's own, or the token (or kit colour) it follows.
export function regionColour(look: Look, scheme: Scheme, region: RegionName, part: 'surface' | 'text' | 'line'): string {
  const own = look.regions[region]?.[part];
  if (own) return own;
  const d: RegionDefault = REGION_DEFAULT[region][part];
  if (d === 'tint-accent') return tints(look.tokens, scheme).accent;
  if (d === 'line-accent') return roleMix(look.tokens, scheme, LINE_STRENGTH).accent; // decoration only, never judged
  return look.tokens[d];
}
export const terminalColour = (look: Look, scheme: Scheme, k: (typeof TERMINAL_KEYS)[number]): string => {
  const own = look.terminal[k];
  if (own) return own;
  if (k === 'background') return look.tokens['surface-0'];
  if (k === 'foreground') return look.tokens.text;
  if (k === 'cursor' || k === 'selection') return look.tokens.accent;
  return ANSI[scheme][ANSI_NAMES.indexOf(k as AnsiName)] as string;
};

// The problems of one scheme's colours: `refusals` keep the theme from being used, `warnings` are listed with it.
export function lookIssues(look: Look, scheme: Scheme): { refusals: ThemeIssue[]; warnings: ThemeIssue[] } {
  const refusals: ThemeIssue[] = contrastIssues(look.tokens).map((c) => ({ code: 'contrast', ...c }));
  // A region the theme colours: its text on its surface; on a surface of its own, also the secondary text, the accent and every state.
  for (const r of REGIONS) {
    const own = look.regions[r];
    if (!own?.surface && !own?.text) continue;
    const bg = regionColour(look, scheme, r, 'surface');
    const pairs: { fg: 'region-text' | TokenName; colour: string; min: number }[] = [{ fg: 'region-text', colour: regionColour(look, scheme, r, 'text'), min: 4.5 }];
    if (own.surface) {
      pairs.push({ fg: 'text-muted', colour: look.tokens['text-muted'], min: 4.5 }, { fg: 'accent', colour: look.tokens.accent, min: 4.5 });
      for (const s of STATES) pairs.push({ fg: s, colour: look.tokens[s], min: 3 });
    }
    for (const p of pairs) { const ratio = contrast(p.colour, bg); if (ratio < p.min) refusals.push({ code: 'region-contrast', region: r, fg: p.fg, ratio, min: p.min }); }
  }
  const bg = terminalColour(look, scheme, 'background');
  const warnings: ThemeIssue[] = [];
  if (look.terminal.background || look.terminal.foreground) {
    const ratio = contrast(terminalColour(look, scheme, 'foreground'), bg);
    if (ratio < 4.5) refusals.push({ code: 'terminal-contrast', ratio, min: 4.5 });
  }
  // Programs choose the 16 colours: a hard one is only warned about. Asked of the colours the theme sets, and of all of them on a
  // background it sets (the window's own palette is chosen for its scheme); never of the one the background stands in for.
  const twin = backgroundTwin(contrast(bg, '#ffffff') > contrast(bg, '#000000'));
  for (const n of ANSI_NAMES) {
    if (n === twin || (!look.terminal[n] && !look.terminal.background)) continue;
    const ratio = contrast(terminalColour(look, scheme, n), bg);
    if (ratio < 3) warnings.push({ code: 'ansi-contrast', colour: n, ratio, min: 3 });
  }
  if (look.terminal.cursor || look.terminal.background) {
    const ratio = contrast(terminalColour(look, scheme, 'cursor'), bg);
    if (ratio < 3) warnings.push({ code: 'cursor-contrast', ratio, min: 3 });
  }
  const code = regionColour(look, scheme, 'code-blocks', 'surface');
  for (const k of SYNTAX_KEYS) {
    const own = look.syntax[k];
    if (!own) continue;
    const ratio = contrast(own, code);
    if (ratio < 4.5) warnings.push({ code: 'syntax-contrast', key: k, ratio, min: 4.5 });
  }
  warnings.push(...themeWarnings(look.tokens, scheme));
  return { refusals, warnings };
}
// Both schemes; a problem names its scheme when the theme has two.
export function judge(theme: Pick<Theme, 'scheme' | 'tokens' | 'looks'>): { refusals: ThemeIssue[]; warnings: ThemeIssue[] } {
  const looks = looksOf(theme);
  const both = !!looks.dark && !!looks.light;
  const refusals: ThemeIssue[] = [];
  const warnings: ThemeIssue[] = [];
  for (const scheme of SCHEMES) {
    const look = looks[scheme];
    if (!look) continue;
    const r = lookIssues(look, scheme);
    const tag = (i: ThemeIssue): ThemeIssue => (both ? { ...i, scheme } : i);
    refusals.push(...r.refusals.map(tag));
    warnings.push(...r.warnings.map(tag));
  }
  return { refusals, warnings };
}

// ---- Writing --------------------------------------------------------------------------------------------------------------------

// The format-2 file of a theme: every token, shape, text and chrome value written out (so it reads the same in a client whose
// defaults differ), and the regions, terminal and syntax colours the theme sets. Reading it back gives the same theme.
export function writeTheme(theme: Pick<Theme, 'scheme' | 'tokens' | 'looks' | 'chrome' | 'text'>, name: string): Record<string, unknown> {
  const looks = looksOf(theme);
  const first = looks[theme.scheme] ?? looks.dark ?? looks.light;
  const tokens = (first as Look).tokens;
  const file: Record<string, unknown> = { 'agora-theme': THEME_FORMAT_2, name };
  for (const scheme of SCHEMES) {
    const look = looks[scheme];
    if (!look) continue;
    const section: Record<string, unknown> = { tokens: Object.fromEntries(COLOR_TOKENS.map((n) => [n, look.tokens[n]])) };
    const regions = Object.fromEntries(REGIONS.filter((r) => look.regions[r] && Object.keys(look.regions[r] as object).length > 0)
      .map((r) => [r, Object.fromEntries(REGION_PARTS.filter((p) => look.regions[r]?.[p]).map((p) => [p, look.regions[r]?.[p]]))]));
    if (Object.keys(regions).length > 0) section.regions = regions;
    const terminal = Object.fromEntries(TERMINAL_KEYS.filter((k) => look.terminal[k]).map((k) => [k, look.terminal[k]]));
    if (Object.keys(terminal).length > 0) section.terminal = terminal;
    const syntax = Object.fromEntries(SYNTAX_KEYS.filter((k) => look.syntax[k]).map((k) => [k, look.syntax[k]]));
    if (Object.keys(syntax).length > 0) section.syntax = syntax;
    file[scheme] = section;
  }
  file.shape = Object.fromEntries(SHAPE_TOKENS.map((n) => [n, tokens[n]]));
  const text = textOf(theme);
  file.text = { 'font-ui': tokens['font-ui'], 'font-mono': tokens['font-mono'], weight: text.weight, 'line-height': text['line-height'], ligatures: text.ligatures };
  file.chrome = Object.fromEntries(CHROME_NAMES.map((n) => [n, chromeOf(theme)[n]]));
  return file;
}
export const serialise = (file: Record<string, unknown>) => JSON.stringify(file, null, 2);

// Shared by the editor and the store: a copy of a theme's looks, chrome and text that can be changed without touching the original.
export function cloneLooks(looks: Looks): Looks {
  const out: Looks = {};
  for (const s of SCHEMES) {
    const l = looks[s];
    if (!l) continue;
    out[s] = {
      tokens: { ...l.tokens }, terminal: { ...l.terminal } as TerminalColours, syntax: { ...l.syntax } as SyntaxColours,
      regions: Object.fromEntries(Object.entries(l.regions).map(([r, v]) => [r, { ...v }])) as Regions,
    };
  }
  return out;
}
// The other scheme added to a theme: that scheme's default colours with this theme's shape and fonts.
export function withScheme(looks: Looks, scheme: Scheme): Looks {
  if (looks[scheme]) return looks;
  const some = (looks.dark ?? looks.light) as Look;
  const shared = Object.fromEntries([...SHAPE_TOKENS, ...FONT_TOKENS].map((n) => [n, some.tokens[n]]));
  return { ...looks, [scheme]: { tokens: { ...baseTokens(scheme), ...shared }, regions: {}, terminal: {}, syntax: {} } };
}
