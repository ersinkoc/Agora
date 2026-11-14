// Where the layout lives between visits: this browser's storage, under versioned keys. Reading never throws (storage can be
// missing, full, or hold something written by another version); writing can fail and then only costs the memory.
import { type Workspace, reduce, workspaceOf } from './engine';
import { type Layout, bindOf, defaultLayout, panesOf, parseLayout } from './tree';

export const CURRENT_KEY = 'agora.workspace.v1';
export const SAVED_KEY = 'agora.layouts.v1';
const OLD_KEY = 'agora.layout'; // the tabs of the window before layouts (net/layout.ts)
const MAX_SAVED = 50;
export const MAX_NAME = 60;

type KV = Pick<Storage, 'getItem' | 'setItem'>;
export interface Stored {
  ws: Workspace;
  touched: boolean; // the user arranged the window: stop opening agents for them
}
export interface Named { name: string; layout: Layout }

function storage(): KV | undefined {
  try { return typeof window !== 'undefined' ? window.localStorage : undefined; } catch { return undefined; }
}
function read(kv: KV | undefined, key: string): unknown {
  try { const v = kv?.getItem(key); return v ? JSON.parse(v) : undefined; } catch { return undefined; }
}
function write(kv: KV | undefined, key: string, value: unknown) {
  try { kv?.setItem(key, JSON.stringify(value)); } catch { /* holds for this visit */ }
}
const str = (v: unknown) => (typeof v === 'string' && v.length > 0 ? v : undefined);

// The tabs a user had open before layouts became one tab group (or, when they watched them side by side, a 2 × 2 grid of the
// first four with the rest as tabs), so an update loses nothing.
export function migrate(old: unknown): Stored | undefined {
  if (!old || typeof old !== 'object' || !Array.isArray((old as { pinned?: unknown }).pinned)) return undefined;
  const o = old as { pinned: unknown[]; active?: unknown; layout?: unknown; touched?: unknown };
  const pinned = [...new Set(o.pinned.filter((x): x is string => typeof x === 'string' && x.length > 0))].slice(0, 24);
  let ws = workspaceOf(defaultLayout());
  if (pinned.length > 0) ws = reduce(ws, { type: 'preset', preset: o.layout === 'grid' && pinned.length > 1 ? '2x2' : '1x1', agents: pinned });
  const active = str(o.active);
  const pane = active ? panesOf(ws.layout.root).find((p) => bindOf(p) === active) : undefined;
  if (pane) ws = reduce(ws, { type: 'activate', pane: pane.id });
  return { ws, touched: !!o.touched };
}

let memo: Stored | undefined;

// The window as it was: this browser's saved workspace, else the old tabs, else one empty pane that fills itself (as before).
export function loadWorkspace(kv: KV | undefined = storage()): Stored {
  if (memo) return memo;
  const v = read(kv, CURRENT_KEY) as { layout?: unknown; focus?: unknown; zoom?: unknown; touched?: unknown } | undefined;
  const parsed = v && typeof v === 'object' ? parseLayout(v.layout) : undefined;
  if (parsed?.ok) {
    const ws = workspaceOf(parsed.layout);
    const focus = str(v?.focus);
    const zoom = str(v?.zoom);
    let next = focus ? reduce(ws, { type: 'focus', pane: focus }) : ws;
    if (zoom) next = reduce(next, { type: 'maximize', pane: zoom });
    if (focus) next = reduce(next, { type: 'focus', pane: focus });
    memo = { ws: next, touched: !!v?.touched };
  } else {
    // Nothing usable (never saved, unreadable, or written by a newer version): what the old window had open, if anything.
    memo = migrate(read(kv, OLD_KEY)) ?? { ws: workspaceOf(defaultLayout()), touched: false };
  }
  return memo;
}

let timer: ReturnType<typeof setTimeout> | undefined;
// Kept for this visit at once (leaving for Full size and coming back finds it); written to storage at once, or `delay` ms after the
// last change when the window saves while a divider is dragged.
export function saveWorkspace(next: Stored, kv: KV | undefined = storage(), delay = 0) {
  memo = next;
  const put = () => { timer = undefined; if (memo) write(kv, CURRENT_KEY, { version: 1, layout: memo.ws.layout, focus: memo.ws.focus, zoom: memo.ws.zoom, touched: memo.touched }); };
  if (timer) clearTimeout(timer);
  if (delay > 0) timer = setTimeout(put, delay); else put();
}

// Only for tests: forget what was read, as a fresh page would.
export function forgetLoaded() { memo = undefined; }

export function savedLayouts(kv: KV | undefined = storage()): Named[] {
  const v = read(kv, SAVED_KEY) as { items?: unknown } | undefined;
  if (!v || !Array.isArray(v.items)) return [];
  const out: Named[] = [];
  for (const item of v.items as unknown[]) {
    const name = item && typeof item === 'object' ? str((item as { name?: unknown }).name)?.slice(0, MAX_NAME) : undefined;
    const parsed = item && typeof item === 'object' ? parseLayout((item as { layout?: unknown }).layout) : undefined;
    // One a newer version wrote, or one that is broken, is skipped here and left for that version.
    if (name && parsed?.ok && !out.some((x) => x.name === name)) out.push({ name, layout: parsed.layout });
  }
  return out;
}

function writeSaved(list: Named[], kv: KV | undefined) {
  write(kv, SAVED_KEY, { version: 1, items: list.slice(0, MAX_SAVED) });
}

// Saving under a name that exists replaces that layout.
export function saveNamed(name: string, layout: Layout, kv: KV | undefined = storage()): Named[] {
  const n = name.trim().slice(0, MAX_NAME);
  if (!n) return savedLayouts(kv);
  const list = savedLayouts(kv).filter((x) => x.name !== n);
  const next = [{ name: n, layout }, ...list].slice(0, MAX_SAVED);
  writeSaved(next, kv);
  return next;
}

export function renameNamed(from: string, to: string, kv: KV | undefined = storage()): Named[] {
  const n = to.trim().slice(0, MAX_NAME);
  const list = savedLayouts(kv);
  if (!n || n === from || list.some((x) => x.name === n)) return list;
  const next = list.map((x) => (x.name === from ? { ...x, name: n } : x));
  writeSaved(next, kv);
  return next;
}

export function deleteNamed(name: string, kv: KV | undefined = storage()): Named[] {
  const next = savedLayouts(kv).filter((x) => x.name !== name);
  writeSaved(next, kv);
  return next;
}
