// The hub's community theme catalog (ADR-0113 decision 6, docs/ux/themes.md "The community catalog"). The hub fetched it on its
// owner's command and checked every theme; the window only asks the hub (`ThemeCatalogList`, `ThemeCatalogGet`), never another
// origin, and checks a theme again before it shows or installs it. Installing copies the theme into the person's own themes
// (store.ts `install`); it changes only when the person picks a newer version (`upgrade`).
import type { agora } from '../gen/agora';
import type { CatalogOrigin, Theme } from './tokens';

/** One theme of the catalog, as the window shows it. Every text is the index's: shown as text, never as markup. */
export interface CatalogEntry { id: string; name: string; version: number; author: string; licence: string; size: number }
/** The catalog as the hub answered: off (the owner has not turned it on), or its themes. */
export type CatalogList = { on: false } | { on: true; entries: CatalogEntry[]; updatedAtMs: number };

type Body = { themeCatalogList: object } | { themeCatalogGet: { id: string } };
/** How the window asks the hub (live.ts `command` with no machine named). */
export type CatalogAsk = (body: Body) => Promise<agora.v1.CommandOk.$Properties>;

const num = (v: unknown) => (typeof v === 'number' ? v : typeof v === 'string' ? Number(v) : typeof v === 'object' && v !== null && 'toNumber' in v ? (v as { toNumber: () => number }).toNumber() : 0);
const str = (v: unknown) => (typeof v === 'string' ? v : '');
const ID = /^[a-z0-9][a-z0-9-]{0,39}$/;

export function entryOf(e: agora.v1.ThemeCatalogEntry.$Properties | null | undefined): CatalogEntry | undefined {
  const id = str(e?.id);
  const version = num(e?.version);
  if (!ID.test(id) || !Number.isInteger(version) || version < 1) return undefined;
  return { id, name: str(e?.name) || id, version, author: str(e?.author), licence: str(e?.licence), size: num(e?.size) };
}

export function listOf(r: agora.v1.ThemeCatalogListResult.$Properties | null | undefined): CatalogList {
  if (!r?.on) return { on: false };
  const entries = (r.entries ?? []).map(entryOf).filter((e): e is CatalogEntry => e !== undefined);
  return { on: true, entries, updatedAtMs: num(r.updatedAtMs) };
}

/** What the person has of an entry: nothing, this version, or an older one (a newer version is available). */
export type Installed = { kind: 'no' } | { kind: 'current'; theme: Theme } | { kind: 'older'; theme: Theme; from: number };
export function installedOf(entry: CatalogEntry, themes: readonly Theme[]): Installed {
  const mine = themes.filter((t) => t.catalog?.id === entry.id);
  if (mine.length === 0) return { kind: 'no' };
  const current = mine.find((t) => (t.catalog as CatalogOrigin).version >= entry.version);
  if (current) return { kind: 'current', theme: current };
  const theme = mine.reduce((a, b) => ((b.catalog as CatalogOrigin).version > (a.catalog as CatalogOrigin).version ? b : a));
  return { kind: 'older', theme, from: (theme.catalog as CatalogOrigin).version };
}

// ---- Where the window asks ------------------------------------------------------------------------------------------------------
// The window sets this while it is connected to a hub (App.tsx); without it (the demo, signed out) the catalog is not shown.
let ask: CatalogAsk | undefined;
const listeners = new Set<() => void>();
export function setCatalogSource(next: CatalogAsk | undefined) {
  if (next === ask) return;
  ask = next;
  listeners.forEach((l) => l());
}
export const catalogSource = () => ask;
export function subscribeCatalogSource(l: () => void) { listeners.add(l); return () => { listeners.delete(l); }; }

/** The catalog, or `undefined` when the hub cannot say (an older hub, or the connection is down). */
export async function loadCatalog(from: CatalogAsk): Promise<CatalogList | undefined> {
  try { return listOf((await from({ themeCatalogList: {} })).themeCatalogList); } catch { return undefined; }
}

/** One theme's file as the hub keeps it; the error code when it is refused (`theme.not_found`, `theme.catalog_off`, a lost connection). */
export async function fetchCatalogTheme(from: CatalogAsk, id: string): Promise<{ ok: true; entry: CatalogEntry; data: string } | { ok: false; code: string }> {
  try {
    const r = (await from({ themeCatalogGet: { id } })).themeCatalogTheme;
    const entry = entryOf(r?.entry);
    if (!entry || entry.id !== id || typeof r?.data !== 'string') return { ok: false, code: 'theme.not_found' };
    return { ok: true, entry, data: r.data };
  } catch (e) {
    return { ok: false, code: typeof (e as { code?: unknown })?.code === 'string' ? (e as { code: string }).code : 'unavailable' };
  }
}
