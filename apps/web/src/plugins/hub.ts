// Plugins installed on the hub (ADR-0114 decision 6, build step 3; docs/ux/plugins.md, "Plugins installed on the hub"). The hub's owner
// installs a package on the hub's machine; this window lists what is installed, fetches each package in pieces (a package is up to 2 MiB,
// a frame at most 1 MiB) and checks it again exactly as a file loaded for development, so nothing the hub says is taken on trust: the
// package's SHA-256 must be the one listed and the checked manifest must name the listed id and version. A package that fails is left
// out. Each person's grants are kept by the hub, one per plugin, for the version they were given for.
import type { agora } from '../gen/agora.js';
import type { CommandBody } from '../net/live';
import { num } from '../net/view';
import { isCapability } from './capabilities';
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
