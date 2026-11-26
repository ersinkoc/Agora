// The plugins this browser loaded for development (ADR-0114 decision 6, "Development"; docs/ux/plugins.md, "Development loading"). A
// package is loaded from a local file into this browser only, works without the hub, and is marked "development" wherever it shows. The
// package text and the person's grants are kept in this browser's storage; every start checks each package again, exactly as a new file.
// Nothing here is sent anywhere. In safe mode no plugin is checked or shown; the list of kept plugins is read only to name them, so
// the person can remove one from there.
//
// Build step 3 adds the plugins installed on the hub (`attachHub`, plugins/hub.ts): listed and fetched from the hub at each load, checked
// here again exactly as a file, with each person's grants kept by the hub. A plugin the owner removed is gone at the window's next load.
// A development plugin with the id of a hub plugin stands in for it in this browser (its own grants, in this browser's storage). The
// two never share a grant. In safe mode nothing is asked of the hub.
//
// Build step 4 adds the hub's community catalog (`catalog`): what the hub's owner can install, listed read only while the owner has it
// on. Nothing is installed from a window.
import type { Capability } from './capabilities';
import { type GrantRecord, type GrantState, grant, grantState, granted, readGrants } from './grants';
import { type Catalog, type HubLink, loadCatalogScreenshot, loadHubCatalog, loadHubGrants, loadHubPlugins } from './hub';
import { type CheckedPlugin, type PackageResult, MANIFEST, MAX_PACKAGE_BYTES, checkPackage, readManifest } from './package';
import { SAFE } from './safe';

export const DEV_KEY = 'agora.plugins.dev.v1';
export const GRANTS_KEY = 'agora.plugins.grants.v1';
export const MAX_DEV_PLUGINS = 4;

type KV = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

export interface LoadedPlugin {
  readonly plugin: CheckedPlugin;
  /** A development plugin's package as it was loaded, kept so the next start can check it again; empty for a hub plugin. */
  readonly text: string;
  /** Loaded from a file into this browser ("development"), or installed on the hub (false). */
  readonly dev: boolean;
  readonly grant?: GrantRecord;
  readonly state: GrantState;
  /** What the frame is answered for. */
  readonly granted: readonly Capability[];
}
export interface PluginState {
  readonly plugins: readonly LoadedPlugin[];
  readonly safe: boolean;
  readonly ready: boolean;
  /** Hub plugins a development plugin of the same id stands in for in this browser: listed as replaced, never run. */
  readonly replaced: readonly LoadedPlugin[];
  /** The hub's community catalog (read only); undefined without a hub, from an older hub, and in safe mode. */
  readonly catalog?: Catalog;
}

function read(kv: KV | undefined, key: string): unknown {
  try { const s = kv?.getItem(key); return s ? JSON.parse(s) : undefined; } catch { return undefined; }
}
/** Writes, or says it could not (a full or blocked storage); what was loaded still holds for this visit. */
function write(kv: KV | undefined, key: string, value: unknown): boolean {
  try { if (value === undefined) kv?.removeItem(key); else kv?.setItem(key, JSON.stringify(value)); return true; } catch { return false; }
}

export interface PluginStoreEnv { kv?: KV; version: string; safe?: boolean }

export function createPluginStore(env: PluginStoreEnv) {
  const safe = !!env.safe;
  let state: PluginState = Object.freeze({ plugins: Object.freeze([]), safe, ready: safe, replaced: Object.freeze([]) });
  const listeners = new Set<() => void>();
  // The development plugins and the hub's, kept apart; what is shown is both, a development one standing in for a hub one of its id.
  let devs: readonly LoadedPlugin[] = [];
  let hubs: readonly LoadedPlugin[] = [];
  let hub: HubLink | undefined;
  let hubTurn = 0;
  let started = safe;
  const cache = new Map<string, CheckedPlugin>();
  // Each hub plugin's package hash: a grant names the package the person was shown.
  let hubSha = new Map<string, string>();
  let catalog: Catalog | undefined;
  const show = () => {
    const shadowed = (h: LoadedPlugin) => devs.some((d) => d.plugin.manifest.id === h.plugin.manifest.id);
    state = Object.freeze({ plugins: Object.freeze([...devs, ...hubs.filter((h) => !shadowed(h))]), safe, ready: started, replaced: Object.freeze(hubs.filter(shadowed)), catalog });
    listeners.forEach((l) => l());
  };
  const set = (plugins: readonly LoadedPlugin[]) => { devs = Object.freeze([...plugins]); show(); };
  const grants = () => readGrants(read(env.kv, GRANTS_KEY));
  const entry = (plugin: CheckedPlugin, text: string, g: GrantRecord | undefined, dev = true): LoadedPlugin =>
    Object.freeze({ plugin, text, dev, grant: g, state: grantState(plugin.manifest, g), granted: granted(plugin.manifest, g) });
  const persist = (plugins: readonly LoadedPlugin[]) => write(env.kv, DEV_KEY, plugins.length ? plugins.map((p) => ({ text: p.text })) : undefined);
  const devOf = (id: string) => devs.find((x) => x.plugin.manifest.id === id);
  const hubOf = (id: string) => (devOf(id) ? undefined : hubs.find((x) => x.plugin.manifest.id === id));
  const encode = (s: string) => new TextEncoder().encode(s);

  return {
    state: () => state,
    subscribe(l: () => void) { listeners.add(l); return () => { listeners.delete(l); }; },
    /** Reads what this browser kept and checks every package again; one that no longer passes is left out (and dropped). */
    async start(): Promise<void> {
      if (safe) return;
      const kept = read(env.kv, DEV_KEY);
      const g = grants();
      const out: LoadedPlugin[] = [];
      if (Array.isArray(kept)) {
        for (const k of kept.slice(0, MAX_DEV_PLUGINS)) {
          const text = (k as { text?: unknown })?.text;
          if (typeof text !== 'string') continue;
          const r = await checkPackage(encode(text), env.version);
          if (r.ok && !out.some((p) => p.plugin.manifest.id === r.plugin.manifest.id)) out.push(entry(r.plugin, text, g[r.plugin.manifest.id]));
        }
      }
      started = true;
      set(out);
    },
    /** Checks a local file as a package (the install screen comes next). Its size is checked before it is read. */
    async check(file: Pick<File, 'size' | 'arrayBuffer'>): Promise<PackageResult & { text?: string }> {
      if (safe) return { ok: false, code: 'format' };
      if (file.size > MAX_PACKAGE_BYTES) return { ok: false, code: 'too-large' };
      const bytes = new Uint8Array(await file.arrayBuffer());
      const r = await checkPackage(bytes, env.version);
      return r.ok ? { ...r, text: new TextDecoder().decode(bytes) } : r;
    },
    /** Keeps a checked package with what the person allowed of it. A plugin with the same id is replaced. Returns whether this browser
     *  could keep it (otherwise it holds for this visit only), or 'full' when MAX_DEV_PLUGINS are loaded already. */
    install(plugin: CheckedPlugin, text: string, allow: readonly Capability[]): 'kept' | 'visit' | 'full' {
      const id = plugin.manifest.id;
      const others = devs.filter((p) => p.plugin.manifest.id !== id);
      if (others.length >= MAX_DEV_PLUGINS) return 'full';
      const g = grant(plugin.manifest, allow);
      const kept = write(env.kv, GRANTS_KEY, { ...grants(), [id]: g });
      const plugins = [...others, entry(plugin, text, g)];
      const stored = persist(plugins);
      set(plugins);
      return kept && stored ? 'kept' : 'visit';
    },
    /** The person looked at a plugin's capabilities (a hub plugin not shown yet, a held plugin, or "What it can see"). A development
     *  plugin's grant is kept in this browser at once; a hub plugin's by the hub, and it changes here only once the hub kept it (false
     *  when it did not). */
    async regrant(id: string, allow: readonly Capability[]): Promise<boolean> {
      const d = devOf(id);
      if (d) {
        const g = grant(d.plugin.manifest, allow);
        write(env.kv, GRANTS_KEY, { ...grants(), [id]: g });
        set(devs.map((x) => (x === d ? entry(d.plugin, d.text, g) : x)));
        return true;
      }
      const h = hubOf(id);
      if (!h || !hub) return false;
      const m = h.plugin.manifest;
      const g = grant(m, allow);
      try { await hub.command({ pluginGrantSet: { pluginId: m.id, version: m.version, granted: [...g.granted], sha256: hubSha.get(m.id) ?? '' } }); } catch { return false; }
      hubs = hubs.map((x) => (x.plugin === h.plugin ? entry(h.plugin, '', g, false) : x));
      show();
      return true;
    },
    /** The person stops showing a hub plugin: the hub forgets their grant, and its panes close (it stays installed for everyone else). */
    async hide(id: string): Promise<boolean> {
      const h = hubOf(id);
      if (!h || !hub) return false;
      try { await hub.command({ pluginGrantRemove: { pluginId: id } }); } catch { return false; }
      hubs = hubs.map((x) => (x.plugin === h.plugin ? entry(h.plugin, '', undefined, false) : x));
      show();
      return true;
    },
    /** Follows the hub this window is connected to: the plugins installed there and this person's grants, read and checked now (each
     *  connection, so a plugin the owner removed is gone and one added is offered), and the catalog's entries. `undefined` (signed
     *  out) leaves only the development plugins. Nothing is asked in safe mode. */
    async attachHub(link: HubLink | undefined): Promise<void> {
      if (safe) return;
      const turn = ++hubTurn;
      hub = link;
      if (!link) { hubs = []; hubSha = new Map(); catalog = undefined; show(); return; }
      const [found, given, listed] = await Promise.all([loadHubPlugins(link, env.version, cache), loadHubGrants(link), loadHubCatalog(link)]);
      if (turn !== hubTurn) return;
      catalog = listed;
      hubs = found.map(({ plugin }) => entry(plugin, '', given[plugin.manifest.id], false));
      hubSha = new Map(found.map(({ plugin, info }) => [plugin.manifest.id, info.sha256]));
      show();
    },
    /** ADR-0120: screenshot `index` of catalog plugin `id`, from the hub this window follows; undefined without one, in safe mode, or
     *  when the hub refuses it. */
    async screenshot(id: string, index: number): Promise<{ src: string; alt: string } | undefined> {
      return safe || !hub ? undefined : loadCatalogScreenshot(hub, id, index);
    },
    /** The plugins this browser keeps, by their manifests only (nothing is checked or run): what safe mode lists for removal. */
    kept(): { id: string; name: string; version: string }[] {
      const list = read(env.kv, DEV_KEY);
      if (!Array.isArray(list)) return [];
      return list.flatMap((k) => { const m = manifestOf((k as { text?: unknown })?.text); return m ? [{ id: m.id, name: m.name, version: m.version }] : []; });
    },
    /** Forgets a development plugin: its package and its grant, in this browser and on screen, in safe mode too. A hub plugin is not
     *  removed from a window (the owner removes it on the hub; a person can stop showing it). */
    remove(id: string) {
      const list = read(env.kv, DEV_KEY);
      const left = Array.isArray(list) ? list.filter((k) => manifestOf((k as { text?: unknown })?.text)?.id !== id) : [];
      write(env.kv, DEV_KEY, left.length ? left : undefined);
      const g = grants();
      delete g[id];
      write(env.kv, GRANTS_KEY, Object.keys(g).length ? g : undefined);
      set(devs.filter((p) => p.plugin.manifest.id !== id));
    },
    /** A development plugin's file changed and its new package passed the checks (reload on save, devReload.ts): it takes the loaded
     *  one's place with the grant the person gave, so its frames start again. A capability the person was never shown holds it, as
     *  any new version does. False when no plugin of its id is loaded. */
    replace(plugin: CheckedPlugin, text: string): boolean {
      // Only the development plugins: the hub's are kept apart (`hubs`), never written to this browser's storage.
      const p = devOf(plugin.manifest.id);
      if (safe || !p) return false;
      const plugins = devs.map((x) => (x === p ? entry(plugin, text, p.grant) : x));
      persist(plugins);
      set(plugins);
      return true;
    },
  };
}
export type PluginStore = ReturnType<typeof createPluginStore>;

/** A kept package's manifest, read as text without checking the package (only to name it); undefined for anything unreadable. */
function manifestOf(text: unknown) {
  try {
    if (typeof text !== 'string' || text.length > MAX_PACKAGE_BYTES * 2) return undefined;
    const files = (JSON.parse(text) as { files?: Record<string, unknown> })?.files;
    const b64 = files && Object.prototype.hasOwnProperty.call(files, MANIFEST) ? files[MANIFEST] : undefined;
    if (typeof b64 !== 'string') return undefined;
    const m = readManifest(JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(Uint8Array.from(atob(b64), (c) => c.charCodeAt(0)))));
    return m.ok ? m.manifest : undefined;
  } catch { return undefined; }
}

function storage(): KV | undefined {
  try { return typeof window !== 'undefined' ? window.localStorage : undefined; } catch { return undefined; }
}
export const AGORA_VERSION: string = typeof __AGORA_VERSION__ === 'string' ? __AGORA_VERSION__ : '0.0.0';
// In safe mode the store keeps its storage only to list and remove what is kept; it checks and shows nothing.
export const pluginStore: PluginStore = createPluginStore({ kv: storage(), version: AGORA_VERSION, safe: SAFE });
