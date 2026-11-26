// Theme packs (ADR-0113 decision 5, docs/ux/themes.md "Theme packs"): one JSON file that bundles themes, saved layouts and keybindings, so
// a look can be shared with the arrangement and keys it was made for. Like a theme file, a pack is data and never run: it is read with
// JSON.parse only, its fields are a closed set at every level, and each part is read by that part's own reader (file.ts `readTheme`,
// layout/tree.ts `parseLayout` behind a stricter walk, keys/file.ts `readKeysValue`), so a pack can hold nothing a theme, a layout or a
// keybindings file could not. One problem anywhere refuses the whole pack, with every problem listed at once. A layout names panels only
// by id: one this window does not have (a plugin's, a newer client's) shows the "unknown panel" placeholder, and a pack never installs or
// enables anything. Reading applies nothing; the import dialog applies only the parts the person chooses. Nothing here touches the DOM.
import { type KeyFile, type KeyIssue, MAX_FILE_BYTES as MAX_KEYS_BYTES, keysObject, readKeysValue } from '../keys/file';
import { MAX_SAVED, MAX_NAME as MAX_LAYOUT_NAME, type Named } from '../layout/store';
import { type Bind, type Layout, type LayoutError, type LayoutNode, MAX_DEPTH, parseLayout, panesOf } from '../layout/tree';
import { MAX_FILE_BYTES, MAX_ISSUES, THEME_FORMAT_2, type ThemeData, readTheme, writeTheme } from './file';
import { isUnsafe } from './look';
import { MAX_NAME, type Theme, type ThemeIssue } from './tokens';

export const PACK_FORMAT = 1;
export const PACK_KEY = 'agora-pack';
/** The largest pack read: a theme file's cap (the per-person settings cap of ADR-0113 decision 6). */
export const MAX_PACK_BYTES = MAX_FILE_BYTES;
/** The most themes and layouts a pack holds: as many as the person can keep. */
export const MAX_PACK_THEMES = 20;
export const MAX_PACK_LAYOUTS = MAX_SAVED;

export type PackPart = 'themes' | 'layouts' | 'keybindings';
export type PackIssue =
  | { code: 'json' | 'invalid' | 'newer' | 'empty' | 'name' }
  | { code: 'too-large'; max: number }
  | { code: 'unknown-field'; field: string }
  | { code: 'part'; part: PackPart }
  | { code: 'too-many'; part: 'themes' | 'layouts'; max: number }
  | { code: 'theme-format'; index: number }
  | { code: 'theme'; index: number; issue: ThemeIssue }
  | { code: 'layout-entry' | 'layout-name' | 'layout-agent'; index: number }
  | { code: 'layout-twice'; index: number; name: string }
  | { code: 'layout-field'; index: number; field: string }
  | { code: 'layout-panel'; index: number; panel: string }
  | { code: 'layout'; index: number; error: LayoutError }
  | { code: 'keys'; issue: KeyIssue }
  | { code: 'more'; count: number };

export interface PackTheme { theme: ThemeData; warnings: ThemeIssue[] }
export interface PackKeys { file: KeyFile; unknown: string[] }
export interface Pack { name: string; themes: PackTheme[]; layouts: Named[]; keys?: PackKeys }
export type ReadPack = { ok: true; pack: Pack } | { ok: false; issues: PackIssue[] };

const isObject = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const clean = (v: string) => v.replace(/[\u0000-\u001f\u007f-\u009f‎‏‪-‮⁦-⁩]/g, '');
const shortKey = (k: string) => clean(k).trim().slice(0, 40);
// A name: 1 to `max` characters once control and direction characters are gone, and none of the pieces a theme value may not hold.
function readName(v: unknown, max: number): string | undefined {
  if (typeof v !== 'string') return undefined;
  const n = clean(v).trim();
  return n.length > 0 && n.length <= max && !isUnsafe(n) ? n : undefined;
}

const TOP = new Set([PACK_KEY, 'name', 'themes', 'layouts', 'keybindings']);
const LAYOUT_ENTRY = new Set(['name', 'layout']);
const LAYOUT_TOP = new Set(['version', 'root']);
const NODE: Record<string, ReadonlySet<string>> = {
  pane: new Set(['type', 'id', 'panel', 'bind']),
  tabs: new Set(['type', 'active', 'children']),
  split: new Set(['type', 'direction', 'ratios', 'children']),
};
/** A panel id: a built-in's word (`terminal`), or a plugin's `plugin.<plugin id>.<panel id>` (ADR-0114), or a newer client's of the same
 *  shape. Anything else is not an id, so no other text reaches the "unknown panel" placeholder. */
export const PANEL_ID = /^[a-z0-9][a-z0-9-]{0,39}(\.[a-z0-9][a-z0-9-]{0,39}){0,2}$/;
const PANE_ID = /^[A-Za-z0-9_-]{1,64}$/;

// The walk `parseLayout` does not do: it reads a layout from this browser's storage leniently (a newer client's fields are kept for it),
// while a pack is someone else's file and is held to the closed set. Shape errors it leaves to `parseLayout`, which says them its way.
function strictLayout(value: unknown, index: number, out: PackIssue[]) {
  const field = (path: string) => out.push({ code: 'layout-field', index, field: path });
  if (!isObject(value)) return;
  for (const k of Object.keys(value)) if (!LAYOUT_TOP.has(k)) field(shortKey(k));
  const walk = (v: unknown, path: string, depth: number) => {
    if (!isObject(v) || depth > MAX_DEPTH * 4) return;
    const allowed = typeof v.type === 'string' ? NODE[v.type] : undefined;
    if (!allowed) return;
    for (const k of Object.keys(v)) if (!allowed.has(k)) field(`${path}.${shortKey(k)}`);
    if (v.type === 'pane') {
      if (typeof v.panel === 'string' && !PANEL_ID.test(v.panel)) out.push({ code: 'layout-panel', index, panel: shortKey(v.panel) });
      if (v.id !== undefined && (typeof v.id !== 'string' || !PANE_ID.test(v.id))) field(`${path}.id`);
      if (v.bind !== undefined) {
        // Layouts in a pack name panels, never an agent: an agent is on someone's own machine.
        if (isObject(v.bind) && 'agent' in v.bind) out.push({ code: 'layout-agent', index });
        else if (!isObject(v.bind) || Object.keys(v.bind).length !== 1 || (v.bind.follow !== 'attention' && v.bind.follow !== 'selection')) field(`${path}.bind`);
      }
    }
    if (Array.isArray(v.children)) v.children.forEach((c: unknown, i: number) => walk(c, `${path}.children[${i}]`, depth + 1));
  };
  walk(value.root, 'root', 1);
}

/** Reads a pack already parsed from JSON. Never throws. */
export function readPack(value: unknown): ReadPack {
  if (!isObject(value)) return { ok: false, issues: [{ code: 'invalid' }] };
  const format = value[PACK_KEY];
  if (typeof format !== 'number' || !Number.isInteger(format) || format < 1) return { ok: false, issues: [{ code: 'invalid' }] };
  if (format > PACK_FORMAT) return { ok: false, issues: [{ code: 'newer' }] };
  const issues: PackIssue[] = [];
  for (const k of Object.keys(value)) if (!TOP.has(k)) issues.push({ code: 'unknown-field', field: shortKey(k) });
  const name = readName(value.name, MAX_NAME);
  if (name === undefined) issues.push({ code: 'name' });

  const themes: PackTheme[] = [];
  if (value.themes !== undefined) {
    if (!Array.isArray(value.themes)) issues.push({ code: 'part', part: 'themes' });
    else if (value.themes.length > MAX_PACK_THEMES) issues.push({ code: 'too-many', part: 'themes', max: MAX_PACK_THEMES });
    else value.themes.forEach((v: unknown, index: number) => {
      // Format 2 only: a pack is written by a window that writes format 2. A newer theme is left to the theme reader, which says so.
      const f = isObject(v) ? v['agora-theme'] : undefined;
      if (typeof f !== 'number' || f < THEME_FORMAT_2) { issues.push({ code: 'theme-format', index }); return; }
      const r = readTheme(v);
      if (r.ok) themes.push({ theme: r.theme, warnings: r.warnings ?? [] });
      else for (const issue of r.issues) issues.push({ code: 'theme', index, issue });
    });
  }

  const layouts: Named[] = [];
  if (value.layouts !== undefined) {
    if (!Array.isArray(value.layouts)) issues.push({ code: 'part', part: 'layouts' });
    else if (value.layouts.length > MAX_PACK_LAYOUTS) issues.push({ code: 'too-many', part: 'layouts', max: MAX_PACK_LAYOUTS });
    else value.layouts.forEach((v: unknown, index: number) => {
      if (!isObject(v)) { issues.push({ code: 'layout-entry', index }); return; }
      for (const k of Object.keys(v)) if (!LAYOUT_ENTRY.has(k)) issues.push({ code: 'layout-field', index, field: shortKey(k) });
      const n = readName(v.name, MAX_LAYOUT_NAME);
      if (n === undefined) issues.push({ code: 'layout-name', index });
      else if (layouts.some((l) => l.name === n)) issues.push({ code: 'layout-twice', index, name: n });
      const before = issues.length;
      strictLayout(v.layout, index, issues);
      const p = parseLayout(v.layout);
      if (!p.ok) issues.push({ code: 'layout', index, error: p.error });
      else if (n !== undefined && issues.length === before) layouts.push({ name: n, layout: p.layout });
    });
  }

  let keys: PackKeys | undefined;
  if (value.keybindings !== undefined) {
    if (!isObject(value.keybindings)) issues.push({ code: 'part', part: 'keybindings' });
    else if (new TextEncoder().encode(JSON.stringify(value.keybindings)).length > MAX_KEYS_BYTES) issues.push({ code: 'keys', issue: { code: 'too-big' } });
    else {
      const r = readKeysValue(value.keybindings);
      if (r.ok) keys = { file: r.file, unknown: r.unknown };
      else for (const issue of r.issues) issues.push({ code: 'keys', issue });
    }
  }

  if (issues.length === 0 && themes.length === 0 && layouts.length === 0 && (!keys || keys.file.entries.length === 0)) issues.push({ code: 'empty' });
  if (issues.length > 0) {
    return { ok: false, issues: issues.length > MAX_ISSUES ? [...issues.slice(0, MAX_ISSUES), { code: 'more', count: issues.length - MAX_ISSUES }] : issues };
  }
  return { ok: true, pack: { name: name as string, themes, layouts, ...(keys && keys.file.entries.length > 0 ? { keys } : {}) } };
}

/** Reads a pack's text: at most MAX_PACK_BYTES, JSON, then `readPack`. Never throws. */
export function deserialisePack(text: string): ReadPack {
  if (new TextEncoder().encode(text).length > MAX_PACK_BYTES) return { ok: false, issues: [{ code: 'too-large', max: MAX_PACK_BYTES / 1024 }] };
  let value: unknown;
  try { value = JSON.parse(text); } catch { return { ok: false, issues: [{ code: 'json' }] }; }
  return readPack(value);
}

/** Whether a parsed JSON value is a pack rather than a theme (the import field takes either). */
export const isPack = (value: unknown) => isObject(value) && PACK_KEY in value;

// ---- Writing ----------------------------------------------------------------------------------------------------------------------

// A layout as a pack carries it: panels only, so a pane bound to one of the person's agents is written unbound (following what needs you
// or the selection is kept).
function unbound(node: LayoutNode): LayoutNode {
  if (node.type === 'pane') {
    const keep: Bind | undefined = node.bind && 'follow' in node.bind ? node.bind : undefined;
    return { type: 'pane', id: node.id, panel: node.panel, ...(keep ? { bind: keep } : {}) };
  }
  if (node.type === 'tabs') return { ...node, children: node.children.map((p) => unbound(p) as typeof p) };
  return { ...node, children: node.children.map(unbound) };
}
export const packLayout = (layout: Layout): Layout => ({ version: layout.version, root: unbound(layout.root) });

/** What goes into a pack: each theme with the name it is written under (a built-in theme's in the window's language), saved layouts,
 *  and the keybindings file. */
export interface PackContents {
  name: string;
  themes: readonly { theme: Pick<Theme, 'scheme' | 'tokens' | 'looks' | 'chrome' | 'text'>; name: string }[];
  layouts: readonly Named[];
  keys?: KeyFile;
}
/** The pack file: reading it back gives the same themes (as `writeTheme` writes them), layouts (without agents) and keybindings. */
export function writePack(c: PackContents): Record<string, unknown> {
  const file: Record<string, unknown> = { [PACK_KEY]: PACK_FORMAT, name: c.name };
  if (c.themes.length > 0) file.themes = c.themes.map((x) => writeTheme(x.theme, x.name));
  if (c.layouts.length > 0) file.layouts = c.layouts.map((l) => ({ name: l.name, layout: packLayout(l.layout) }));
  if (c.keys && c.keys.entries.length > 0) file.keybindings = keysObject(c.keys);
  return file;
}
export const serialisePack = (file: Record<string, unknown>) => `${JSON.stringify(file, null, 2)}\n`;

// ---- What the window shows of a pack ----------------------------------------------------------------------------------------------

/** The panels a layout names that need something this window does not draw by itself: a plugin's (shown only once that plugin is
 *  installed and allowed, which a pack never does) or one this window does not know (a newer client's). Both show the placeholder. */
export function missingPanels(layout: Layout, known: (id: string) => boolean): { plugins: string[]; unknown: string[] } {
  const ids = [...new Set(panesOf(layout.root).map((p) => p.panel))].filter((id) => !known(id));
  return { plugins: ids.filter((id) => id.startsWith('plugin.')), unknown: ids.filter((id) => !id.startsWith('plugin.')) };
}
