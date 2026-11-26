// Plugins installed on the hub (ADR-0114 decision 6, build step 3; docs/ux/plugins.md, "Plugins installed on the hub"). The hub's owner
// installs a package on the hub's machine; this window lists what is installed, fetches each package in pieces (a package is up to 2 MiB,
// a frame at most 1 MiB) and checks it again exactly as a file loaded for development, so nothing the hub says is taken on trust: the
// package's SHA-256 must be the one listed and the checked manifest must name the listed id and version. A package that fails is left
// out. Each person's grants are kept by the hub, one per plugin, for the version they were given for.
import type { agora } from '../gen/agora.js';
import type { CommandBody } from '../net/live';
import { num } from '../net/view';
import { dayOf, popularityOf } from '../themes/catalog';
import { screenshotAddress } from '../gallery/order';
import { type Capability, isCapability } from './capabilities';
import type { GrantRecord } from './grants';
import { type CheckedPlugin, MAX_PACKAGE_BYTES, checkPackage, sha256Hex } from './package';

/** What the plugin store needs of the hub: a command it answers itself (no node named). */
export interface HubLink { command(body: CommandBody): Promise<agora.v1.CommandOk.$Properties> }

/** The most hub plugins a window loads (the hub keeps at most as many). */
export const MAX_HUB_PLUGINS = 16;

/** A package's bytes, fetched in pieces by its listed hash; undefined when the hub no longer has it or sends anything else. */
export async function fetchPackage(hub: HubLink, id: string, sha256: string, size: number): Promise<Uint8Array | undefined> {
  if (size > MAX_PACKAGE_BYTES || size < 0) return undefined;
  const out = new Uint8Array(size);
  let at = 0;
  for (;;) {
    let piece: agora.v1.IPluginFetchResult | null | undefined;
    try { piece = (await hub.command({ pluginFetch: { pluginId: id, sha256, offset: at } })).pluginFetch; } catch { return undefined; }
    if (!piece || piece.pluginId !== id || piece.sha256 !== sha256 || num(piece.totalSize) !== size || num(piece.offset) !== at) return undefined;
    const data = piece.data ?? new Uint8Array();
    if (data.length === 0) return at === size ? out : undefined;
    if (at + data.length > size) return undefined;
    out.set(data, at);
    at += data.length;
  }
}

export interface HubPlugin { readonly plugin: CheckedPlugin; readonly info: { readonly sha256: string } }

/** The installed plugins that pass this window's checks, by id. `cache` keeps checked packages by their hash, so a window that connects
 *  again fetches only what changed. Any refusal of the list (an older hub that does not know it) means none. */
export async function loadHubPlugins(hub: HubLink, version: string, cache: Map<string, CheckedPlugin>): Promise<HubPlugin[]> {
  let list: agora.v1.IPluginInfo[];
  try { list = (await hub.command({ pluginList: {} })).pluginList?.plugins ?? []; } catch { return []; }
  const out: HubPlugin[] = [];
  const kept = new Set<string>();
  for (const info of list.slice(0, MAX_HUB_PLUGINS)) {
    const id = info.pluginId ?? '', sha = info.sha256 ?? '';
    if (!id || !/^[0-9a-f]{64}$/.test(sha) || out.some((p) => p.plugin.manifest.id === id)) continue;
    let plugin = cache.get(sha);
    if (!plugin) {
      const bytes = await fetchPackage(hub, id, sha, num(info.size));
      if (!bytes || (await sha256Hex(bytes)) !== sha) continue;
      const r = await checkPackage(bytes, version);
      if (!r.ok) continue;
      plugin = r.plugin;
    }
    if (plugin.manifest.id !== id || plugin.manifest.version !== info.version) continue;
    cache.set(sha, plugin);
    kept.add(sha);
    out.push({ plugin, info: { sha256: sha } });
  }
  // Only what the hub still has stays cached.
  for (const sha of [...cache.keys()]) if (!kept.has(sha)) cache.delete(sha);
  return out;
}

/** This person's grants as the hub keeps them, by plugin id; anything unreadable is left out (that plugin asks again). */
export async function loadHubGrants(hub: HubLink): Promise<Record<string, GrantRecord>> {
  const out: Record<string, GrantRecord> = Object.create(null);
  let grants: agora.v1.IPluginGrant[];
  try { grants = (await hub.command({ pluginGrantList: {} })).pluginGrantList?.grants ?? []; } catch { return out; }
  for (const g of grants) {
    if (!g.pluginId || !g.version) continue;
    const seen = (g.seen ?? []).filter(isCapability);
    out[g.pluginId] = Object.freeze({ version: g.version, seen: Object.freeze(seen), granted: Object.freeze((g.granted ?? []).filter(isCapability).filter((c) => seen.includes(c))) });
  }
  return out;
}

// ---- The community catalog (ADR-0114 decision 7, build step 4) ------------------------------------------------------------------
// The hub's owner pins the catalog's key, fetches its signed index and installs from it on the hub's machine; the window only lists
// what the hub kept (`PluginCatalogList`), read only, so the person sees what is available and the command the owner would run.
// Everything in an entry is the index's text: shown as text, never as the window's words.

/** One plugin of the catalog as the window shows it. */
export interface CatalogPlugin {
  readonly id: string; readonly name: string; readonly version: string; readonly author: string; readonly licence: string;
  readonly description: string;
  /** What it asks to see, as this window knows the names (an unknown one is counted in `unknown`). */
  readonly capabilities: readonly Capability[];
  readonly unknown: number;
  /** ADR-0120: the day the catalog first listed it (`YYYY-MM-DD`, empty when not given), the popularity it publishes (0: none), and
   *  one alt text per screenshot the hub kept (fetched by its place with `loadCatalogScreenshot`). */
  readonly added: string;
  readonly popularity: number;
  readonly screenshots: readonly string[];
}
/** Off (the owner has not turned it on), or its entries. */
export type Catalog = { readonly on: false } | { readonly on: true; readonly entries: readonly CatalogPlugin[] };

/** The most catalog entries a window lists (the hub keeps at most as many). */
export const MAX_CATALOG = 200;
const ID = /^[a-z0-9][a-z0-9-]{0,39}$/;
const VERSION = /^(0|[1-9]\d{0,5})\.(0|[1-9]\d{0,5})\.(0|[1-9]\d{0,5})$/;
const text = (v: unknown, max: number) => (typeof v === 'string' ? v.slice(0, max) : '');

/** Orders two `x.y.z` versions by their numbers (as the hub's `compare_versions`). */
export function compareVersions(a: string, b: string): number {
  const p = (s: string) => s.split('.').map((x) => Number(x) || 0);
  const [x, y] = [p(a), p(b)];
  for (let i = 0; i < 3; i++) { const d = (x[i] ?? 0) - (y[i] ?? 0); if (d !== 0) return Math.sign(d); }
  return 0;
}

/** ADR-0120: screenshot `index` of catalog plugin `id`, as an address an `<img>` shows (checked again: a PNG or WebP of the type the hub
 *  named), with its alt text; undefined when the hub refuses it, the link is down, or it is not such an image. */
export async function loadCatalogScreenshot(hub: HubLink, id: string, index: number): Promise<{ src: string; alt: string } | undefined> {
  let r: agora.v1.IPluginCatalogScreenshotResult | null | undefined;
  try { r = (await hub.command({ pluginCatalogScreenshot: { pluginId: id, index } })).pluginCatalogScreenshot; } catch { return undefined; }
  const data = r?.data instanceof Uint8Array ? r.data : undefined;
  const src = data && data.length <= 512 * 1024 ? screenshotAddress(typeof r?.mediaType === 'string' ? r.mediaType : '', data) : undefined;
  return src ? { src, alt: text(r?.alt, 200) } : undefined;
}

/** The catalog as the hub keeps it; undefined when the hub cannot say (an older hub refuses the message, or the link is down). */
export async function loadHubCatalog(hub: HubLink): Promise<Catalog | undefined> {
  let r: agora.v1.IPluginCatalogListResult | null | undefined;
  try { r = (await hub.command({ pluginCatalogList: {} })).pluginCatalogList; } catch { return undefined; }
  if (!r?.on) return Object.freeze({ on: false as const });
  const entries: CatalogPlugin[] = [];
  for (const e of (r.entries ?? []).slice(0, MAX_CATALOG)) {
    const id = text(e.pluginId, 40), version = text(e.version, 20);
    if (!ID.test(id) || !VERSION.test(version) || entries.some((x) => x.id === id)) continue;
    const caps = (e.capabilities ?? []).slice(0, 16);
    entries.push(Object.freeze({
      id, version, name: text(e.name, 60) || id, author: text(e.author, 100), licence: text(e.licence, 60), description: text(e.description, 300),
      capabilities: Object.freeze(caps.filter(isCapability)), unknown: caps.filter((c) => !isCapability(c)).length,
      added: dayOf(e.added), popularity: popularityOf(e.popularity),
      screenshots: Object.freeze((e.screenshots ?? []).slice(0, 4).map((a) => text(a, 200))),
    }));
  }
  return Object.freeze({ on: true as const, entries: Object.freeze(entries) });
}
