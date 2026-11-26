// Reload on save (roadmap M5.7; docs/ux/plugins.md, "Reload on save"): a development plugin's watched file is checked again once a change
// has held for one look; a package that passes takes the old one's place with the person's grant, one that fails or holds another plugin
// leaves the old one running and says why, a new capability holds the plugin, and nothing is watched in safe mode or after a removal.
import { describe, expect, it } from 'vitest';
import { createDevReload, canWatchFiles, pickPackage, POLL_MS, type WatchedFile } from './devReload';
import { checkPackage } from './package';
import { createPluginStore } from './store';
import { ENTRY, baseManifest, bundle } from './testPackage';

const V = '0.3.39';

function memory() {
  const m = new Map<string, string>();
  return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => { m.set(k, v); }, removeItem: (k: string) => { m.delete(k); } };
}
const pkg = (extra: Record<string, unknown> = {}, entry = ENTRY) => {
  const files = { 'panel.js': entry };
  return bundle(files, { manifest: baseManifest(files, extra) });
};

/** A file on disk as the picker's handle sees it: what it holds and when it last changed. */
function disk(bytes: Uint8Array) {
  const f = { bytes, at: 1000, fail: false };
  const handle: WatchedFile = {
    async getFile() {
      if (f.fail) throw new Error('NotFoundError');
      const b = f.bytes;
      return { size: b.length, lastModified: f.at, arrayBuffer: async () => b.slice().buffer };
    },
  };
  return { f, handle, save(b: Uint8Array) { f.bytes = b; f.at += 1000; } };
}

function manualTimers() {
  const live = new Set<number>();
  let n = 0;
  return { live, set: (_f: () => void, ms: number) => { expect(ms).toBe(POLL_MS); live.add(++n); return n; }, clear: (t: unknown) => { live.delete(t as number); } };
}

async function setup(opts: { safe?: boolean } = {}) {
  const kv = memory();
  const store = createPluginStore({ kv, version: V, safe: opts.safe });
  await store.start();
  const first = pkg();
  const r = await checkPackage(first, V);
  if (!r.ok) throw new Error(r.code);
  if (!opts.safe) store.install(r.plugin, new TextDecoder().decode(first), ['agents.read']);
  const timers = manualTimers();
  const reload = createDevReload(store, timers);
  const d = disk(first);
  reload.watch('demo', d.handle, await d.handle.getFile());
  return { kv, store, reload, timers, d };
}
const current = (s: ReturnType<typeof createPluginStore>) => s.state().plugins[0]!;

describe('reload on save', () => {
  it('loads a changed file once the change has held for one look, keeping the grant', async () => {
    const { kv, store, reload, timers, d } = await setup();
    expect(timers.live.size).toBe(1);
    expect(reload.state().watching).toEqual(['demo']);
    const before = current(store).plugin;
    await reload.tick();
    expect(current(store).plugin).toBe(before);
    d.save(pkg({ version: '1.0.1' }));
    await reload.tick();
    // Seen once: it may still be being written.
    expect(current(store).plugin).toBe(before);
    await reload.tick();
    const now = current(store);
    expect(now.plugin).not.toBe(before);
    expect(now.plugin.manifest.version).toBe('1.0.1');
    expect(now.state).toBe('ready');
    expect(now.granted).toEqual(['agents.read']);
    expect(reload.state().problems).toEqual({});
    // The next start of the window loads the new package too.
    const again = createPluginStore({ kv, version: V });
    await again.start();
    expect(again.state().plugins.map((p) => [p.plugin.manifest.version, p.state])).toEqual([['1.0.1', 'ready']]);
  });

  it('keeps the last package that passed when a change is refused, and says why until one passes', async () => {
    const { store, reload, d } = await setup();
    const before = current(store).plugin;
    d.save(new TextEncoder().encode('{"agora-plugin-bundle":1,"files":{"x/../y.js":""}}'));
    await reload.tick();
    await reload.tick();
    expect(current(store).plugin).toBe(before);
    expect(reload.state().problems.demo).toEqual({ code: 'path', detail: 'x/../y.js' });
    d.save(pkg({ id: 'another' }));
    await reload.tick();
    await reload.tick();
    expect(reload.state().problems.demo).toEqual({ code: 'other-id', detail: 'another' });
    expect(current(store).plugin).toBe(before);
    d.save(pkg({ version: '2.0.0' }));
    await reload.tick();
    await reload.tick();
    expect(current(store).plugin.manifest.version).toBe('2.0.0');
    expect(reload.state().problems).toEqual({});
  });

  it('holds a version that asks to see more, never granting it', async () => {
    const { store, reload, d } = await setup();
    d.save(pkg({ capabilities: ['agents.read', 'chat.read'] }));
    await reload.tick();
    await reload.tick();
    expect(current(store).plugin.manifest.capabilities).toEqual(['agents.read', 'chat.read']);
    expect(current(store).state).toBe('held');
    expect(current(store).granted).toEqual([]);
  });

  it('says when the file cannot be read any more, and keeps the plugin', async () => {
    const { store, reload, d } = await setup();
    d.f.fail = true;
    await reload.tick();
    expect(reload.state().problems.demo).toEqual({ code: 'unreadable' });
    expect(store.state().plugins).toHaveLength(1);
  });

  it('stops watching a plugin that is removed or loaded again from a file it cannot watch', async () => {
    const { store, reload, timers } = await setup();
    store.remove('demo');
    expect(reload.state().watching).toEqual([]);
    expect(timers.live.size).toBe(0);
    const second = await setup();
    second.reload.unwatch('demo');
    expect(second.timers.live.size).toBe(0);
  });

  it('watches nothing in safe mode', async () => {
    const { reload, timers } = await setup({ safe: true });
    expect(reload.state().watching).toEqual([]);
    expect(timers.live.size).toBe(0);
  });

  it('uses the picker that can read a file again only where there is one', async () => {
    expect(canWatchFiles({})).toBe(false);
    expect(canWatchFiles(undefined)).toBe(false);
    const d = disk(pkg());
    const w = { showOpenFilePicker: async (o: { types: { description: string }[] }) => { expect(o.types[0]?.description).toBe('Agora plugin package'); return [d.handle]; } };
    expect(canWatchFiles(w)).toBe(true);
    const picked = await pickPackage('Agora plugin package', w);
    expect(picked?.handle).toBe(d.handle);
    expect(picked?.file.size).toBe(d.f.bytes.length);
    // Cancelled: nothing.
    expect(await pickPackage('x', { showOpenFilePicker: async () => { throw new DOMException('', 'AbortError'); } })).toBeUndefined();
  });
});
