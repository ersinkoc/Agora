// The plugins this browser loaded for development (ADR-0114 decision 6, "Development"; docs/ux/plugins.md, "Development loading"). A
// package is loaded from a local file into this browser only, works without the hub, and is marked "development" wherever it shows. The
// package text and the person's grants are kept in this browser's storage; every start checks each package again, exactly as a new file.
// Nothing here is sent anywhere. In safe mode nothing is read and no plugin is shown.
//
// Hub storage, install for a hub, and per-person grants kept by the hub are ADR-0114 build step 3; the catalog is step 4.
import type { Capability } from './capabilities';
import { type GrantRecord, type GrantState, grant, grantState, granted, readGrants } from './grants';
import { type CheckedPlugin, type PackageResult, MAX_PACKAGE_BYTES, checkPackage } from './package';
import { SAFE } from './safe';

export const DEV_KEY = 'agora.plugins.dev.v1';
export const GRANTS_KEY = 'agora.plugins.grants.v1';
export const MAX_DEV_PLUGINS = 4;

type KV = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

export interface LoadedPlugin {
  readonly plugin: CheckedPlugin;
  /** The package as it was loaded, kept so the next start can check it again. */
  readonly text: string;
  readonly dev: true;
  readonly grant?: GrantRecord;
  readonly state: GrantState;
  /** What the frame is answered for. */
  readonly granted: readonly Capability[];
}
export interface PluginState { readonly plugins: readonly LoadedPlugin[]; readonly safe: boolean; readonly ready: boolean }

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
  let state: PluginState = Object.freeze({ plugins: Object.freeze([]), safe, ready: safe });
  const listeners = new Set<() => void>();
  const set = (plugins: readonly LoadedPlugin[], ready = true) => { state = Object.freeze({ plugins: Object.freeze([...plugins]), safe, ready }); listeners.forEach((l) => l()); };
  const grants = () => readGrants(read(env.kv, GRANTS_KEY));
  const entry = (plugin: CheckedPlugin, text: string, g: GrantRecord | undefined): LoadedPlugin =>
    Object.freeze({ plugin, text, dev: true as const, grant: g, state: grantState(plugin.manifest, g), granted: granted(plugin.manifest, g) });
  const persist = (plugins: readonly LoadedPlugin[]) => write(env.kv, DEV_KEY, plugins.length ? plugins.map((p) => ({ text: p.text })) : undefined);
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
      const others = state.plugins.filter((p) => p.plugin.manifest.id !== id);
      if (others.length >= MAX_DEV_PLUGINS) return 'full';
      const g = grant(plugin.manifest, allow);
      const kept = write(env.kv, GRANTS_KEY, { ...grants(), [id]: g });
      const plugins = [...others, entry(plugin, text, g)];
      const stored = persist(plugins);
      set(plugins);
      return kept && stored ? 'kept' : 'visit';
    },
    /** The person looked at a plugin's capabilities again (a held plugin, or "Change what it can see"). */
    regrant(id: string, allow: readonly Capability[]) {
      const p = state.plugins.find((x) => x.plugin.manifest.id === id);
      if (!p) return;
      const g = grant(p.plugin.manifest, allow);
      write(env.kv, GRANTS_KEY, { ...grants(), [id]: g });
      set(state.plugins.map((x) => (x === p ? entry(p.plugin, p.text, g) : x)));
    },
    remove(id: string) {
      const plugins = state.plugins.filter((p) => p.plugin.manifest.id !== id);
      const g = grants();
      delete g[id];
      write(env.kv, GRANTS_KEY, Object.keys(g).length ? g : undefined);
      persist(plugins);
      set(plugins);
    },
  };
}
export type PluginStore = ReturnType<typeof createPluginStore>;

function storage(): KV | undefined {
  try { return typeof window !== 'undefined' ? window.localStorage : undefined; } catch { return undefined; }
}
export const AGORA_VERSION: string = typeof __AGORA_VERSION__ === 'string' ? __AGORA_VERSION__ : '0.0.0';
export const pluginStore: PluginStore = createPluginStore({ kv: SAFE ? undefined : storage(), version: AGORA_VERSION, safe: SAFE });
