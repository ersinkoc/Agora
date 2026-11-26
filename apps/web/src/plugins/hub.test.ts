// Plugins installed on the hub (ADR-0114 decision 6, build step 3): the window lists them, fetches each package in pieces, checks it
// again as a file, and keeps each person's grants on the hub; a plugin the owner removed is gone at the next load; a version that asks for
// more is held until the person looks again; safe mode asks the hub nothing; and a development plugin never takes a hub plugin's grant.
import { createHash } from 'node:crypto';
import { describe, expect, it } from 'vitest';
import type { agora } from '../gen/agora.js';
import type { CommandBody } from '../net/live';
import { type HubLink, fetchPackage } from './hub';
import { checkPackage } from './package';
import { pluginPanels } from './registry';
import { createPluginStore } from './store';
import { ENTRY, baseManifest, bundle } from './testPackage';

const V = '0.3.39';
const sha = (b: Uint8Array) => createHash('sha256').update(b).digest('hex');

/** A hub as the Rust one answers (crates/agora-hub/src/plugins.rs), pieces of `piece` bytes. */
function fakeHub(piece = 100) {
  const installed = new Map<string, { version: string; caps: string[]; bytes: Uint8Array }>();
  const grants = new Map<string, { version: string; seen: string[]; granted: string[] }>();
  const sent: string[] = [];
  let refuse = false;
  const link: HubLink = {
    async command(body: CommandBody): Promise<agora.v1.CommandOk.$Properties> {
      const kind = Object.keys(body)[0] ?? '';
      sent.push(kind);
      if (refuse) throw new Error('auth.forbidden');
      if (body.pluginList) {
        return { pluginList: { plugins: [...installed].map(([id, p]) => ({ pluginId: id, version: p.version, name: id, sha256: sha(p.bytes), size: p.bytes.length, capabilities: p.caps, installedMs: 1 })) } };
      }
      if (body.pluginFetch) {
        const f = body.pluginFetch;
        const p = installed.get(f.pluginId ?? '');
        if (!p || sha(p.bytes) !== f.sha256) throw new Error('plugin.not_found');
        const at = Number(f.offset ?? 0);
        return { pluginFetch: { pluginId: f.pluginId, sha256: f.sha256, totalSize: p.bytes.length, offset: at, data: p.bytes.slice(at, at + piece) } };
      }
      if (body.pluginGrantList) return { pluginGrantList: { grants: [...grants].map(([id, g]) => ({ pluginId: id, ...g })) } };
      if (body.pluginGrantSet) {
        const g = body.pluginGrantSet;
        const p = installed.get(g.pluginId ?? '');
        if (!p || p.version !== g.version || sha(p.bytes) !== g.sha256) throw new Error('plugin.not_found');
        grants.set(g.pluginId ?? '', { version: p.version, seen: p.caps, granted: p.caps.filter((c) => (g.granted ?? []).includes(c)) });
        return { empty: {} };
      }
      if (body.pluginGrantRemove) { grants.delete(body.pluginGrantRemove.pluginId ?? ''); return { empty: {} }; }
      throw new Error('protocol.invalid_message');
    },
  };
  const install = (id: string, version: string, caps: string[]) => {
    const files = { 'panel.js': `${ENTRY}//${id}${version}` };
    installed.set(id, { version, caps, bytes: bundle(files, { manifest: baseManifest(files, { id, version, capabilities: caps }) }) });
  };
  return { link, install, installed, grants, sent, refuseAll: () => { refuse = true; } };
}

const ids = (s: ReturnType<typeof createPluginStore>) => s.state().plugins.map((p) => `${p.plugin.manifest.id}@${p.plugin.manifest.version}:${p.dev ? 'dev' : 'hub'}:${p.state}`);

describe('plugins installed on the hub', () => {
  it('are fetched in pieces, checked again, and offered only once the person shows them', async () => {
    const hub = fakeHub(64);
    hub.install('agents', '1.0.0', ['agents.read', 'theme.read']);
    const s = createPluginStore({ version: V });
    await s.attachHub(hub.link);
    expect(ids(s)).toEqual(['agents@1.0.0:hub:new']);
    expect(hub.sent.filter((k) => k === 'pluginFetch').length).toBeGreaterThan(3);
    // Not shown yet: no panel, nothing granted.
    expect(pluginPanels(s.state().plugins)).toEqual([]);
    expect(await s.regrant('agents', ['agents.read'])).toBe(true);
    expect(hub.grants.get('agents')).toEqual({ version: '1.0.0', seen: ['agents.read', 'theme.read'], granted: ['agents.read'] });
    expect(ids(s)).toEqual(['agents@1.0.0:hub:ready']);
    expect(s.state().plugins[0]?.granted).toEqual(['agents.read']);
    const panels = pluginPanels(s.state().plugins);
    expect(panels.map((p) => [p.id, p.plugin])).toEqual([['plugin.agents.main', { name: 'Demo', dev: false }]]);
  });

  it('keeps grants on the hub, so another window of the person reads them back', async () => {
    const hub = fakeHub();
    hub.install('agents', '1.0.0', ['agents.read', 'theme.read']);
    const first = createPluginStore({ version: V });
    await first.attachHub(hub.link);
    await first.regrant('agents', ['theme.read']);
    const second = createPluginStore({ version: V });
    await second.attachHub(hub.link);
    expect(ids(second)).toEqual(['agents@1.0.0:hub:ready']);
    expect(second.state().plugins[0]?.granted).toEqual(['theme.read']);
    // Stop showing: the hub forgets the grant and its panes close; it stays installed.
    expect(await second.hide('agents')).toBe(true);
    expect(hub.grants.has('agents')).toBe(false);
    expect(ids(second)).toEqual(['agents@1.0.0:hub:new']);
    expect(pluginPanels(second.state().plugins)).toEqual([]);
  });

  it('is gone at the next load once the owner removed it', async () => {
    const hub = fakeHub();
    hub.install('agents', '1.0.0', ['agents.read']);
    hub.install('tasks', '1.0.0', ['tasks.read']);
    const s = createPluginStore({ version: V });
    await s.attachHub(hub.link);
    expect(ids(s)).toEqual(['agents@1.0.0:hub:new', 'tasks@1.0.0:hub:new']);
    hub.installed.delete('agents');
    await s.attachHub(hub.link);
    expect(ids(s)).toEqual(['tasks@1.0.0:hub:new']);
    // Signed out: only development plugins are left.
    await s.attachHub(undefined);
    expect(ids(s)).toEqual([]);
  });

  it('holds a version that asks for more until the person looks again', async () => {
    const hub = fakeHub();
    hub.install('agents', '1.0.0', ['agents.read']);
    const s = createPluginStore({ version: V });
    await s.attachHub(hub.link);
    await s.regrant('agents', ['agents.read']);
    hub.install('agents', '1.1.0', ['agents.read', 'chat.read']);
    await s.attachHub(hub.link);
    expect(ids(s)).toEqual(['agents@1.1.0:hub:held']);
    expect(s.state().plugins[0]?.granted).toEqual([]);
    expect(pluginPanels(s.state().plugins)).toEqual([]);
    expect(await s.regrant('agents', ['agents.read'])).toBe(true);
    expect(ids(s)).toEqual(['agents@1.1.0:hub:ready']);
    expect(hub.grants.get('agents')?.version).toBe('1.1.0');
    // A version that asks for nothing more stays ready.
    hub.install('agents', '1.2.0', ['agents.read']);
    await s.attachHub(hub.link);
    expect(ids(s)).toEqual(['agents@1.2.0:hub:ready']);
  });

  it('leaves out a package whose bytes are not the listed ones, and a grant the hub did not keep changes nothing', async () => {
    const hub = fakeHub();
    hub.install('agents', '1.0.0', ['agents.read']);
    hub.install('liar', '1.0.0', ['agents.read']);
    // The list says version 2.0.0, the package says 1.0.0: left out.
    const link: HubLink = {
      async command(body) {
        const ok = await hub.link.command(body);
        if (ok.pluginList?.plugins) ok.pluginList.plugins = ok.pluginList.plugins.map((p) => (p.pluginId === 'liar' ? { ...p, version: '2.0.0' } : p));
        return ok;
      },
    };
    const s = createPluginStore({ version: V });
    await s.attachHub(link);
    expect(ids(s)).toEqual(['agents@1.0.0:hub:new']);
    // A piece that does not add up, or a hub that does not have it: nothing.
    expect(await fetchPackage(hub.link, 'agents', '0'.repeat(64), 10)).toBeUndefined();
    expect(await fetchPackage(hub.link, 'agents', sha(hub.installed.get('agents')?.bytes ?? new Uint8Array()), 3)).toBeUndefined();
    // The owner replaced it meanwhile: the hub refuses the grant for the old version, and the window keeps what it had.
    hub.install('agents', '1.0.1', ['agents.read']);
    expect(await s.regrant('agents', ['agents.read'])).toBe(false);
    expect(ids(s)).toEqual(['agents@1.0.0:hub:new']);
    // Or put another package in under the same version (asking for more): the grant names the package shown, so it is refused too.
    hub.install('agents', '1.0.0', ['agents.read', 'chat.read']);
    expect(await s.regrant('agents', ['agents.read'])).toBe(false);
    expect(hub.grants.has('agents')).toBe(false);
  });

  it('means no hub plugins with an older hub, and asks the hub nothing in safe mode', async () => {
    const older = fakeHub();
    older.install('agents', '1.0.0', ['agents.read']);
    older.refuseAll();
    const s = createPluginStore({ version: V });
    await s.attachHub(older.link);
    expect(ids(s)).toEqual([]);
    const hub = fakeHub();
    hub.install('agents', '1.0.0', ['agents.read']);
    const safe = createPluginStore({ version: V, safe: true });
    await safe.attachHub(hub.link);
    expect(hub.sent).toEqual([]);
    expect(safe.state().plugins).toEqual([]);
  });

  it('lets a development plugin of the same id stand in for the hub one, with its own grant in this browser', async () => {
    const m = new Map<string, string>();
    const kv = { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v), removeItem: (k: string) => void m.delete(k) };
    const hub = fakeHub();
    hub.install('agents', '1.0.0', ['agents.read']);
    const s = createPluginStore({ kv, version: V });
    await s.start();
    await s.attachHub(hub.link);
    await s.regrant('agents', ['agents.read']);
    const files = { 'panel.js': `${ENTRY}//dev` };
    const r = await checkPackage(bundle(files, { manifest: baseManifest(files, { id: 'agents', version: '2.0.0', capabilities: ['agents.read'] }) }), V);
    if (!r.ok) throw new Error(r.code);
    s.install(r.plugin, 'x', []);
    expect(ids(s)).toEqual(['agents@2.0.0:dev:ready']);
    expect(s.state().plugins[0]?.granted).toEqual([]);
    // The hub's is kept apart, listed as replaced, never run.
    expect(s.state().replaced.map((p) => `${p.plugin.manifest.id}@${p.plugin.manifest.version}`)).toEqual(['agents@1.0.0']);
    // Its grant is this browser's; the hub's record is untouched.
    await s.regrant('agents', ['agents.read']);
    expect(hub.sent.filter((k) => k === 'pluginGrantSet').length).toBe(1);
    expect(JSON.parse(m.get('agora.plugins.grants.v1') ?? '{}').agents.version).toBe('2.0.0');
    // Removed from this browser: the hub plugin is back, with the person's hub grant.
    s.remove('agents');
    expect(ids(s)).toEqual(['agents@1.0.0:hub:ready']);
    expect(s.state().replaced).toEqual([]);
  });
  it('keeps the hub plugins apart when a development plugin is reloaded on save', async () => {
    const m = new Map<string, string>();
    const kv = { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v), removeItem: (k: string) => void m.delete(k) };
    const hub = fakeHub();
    hub.install('agents', '1.0.0', ['agents.read']);
    const s = createPluginStore({ kv, version: V });
    await s.start();
    await s.attachHub(hub.link);
    const dev = async (version: string) => {
      const files = { 'panel.js': `${ENTRY}//clock${version}` };
      const r = await checkPackage(bundle(files, { manifest: baseManifest(files, { id: 'clock', version, capabilities: [] }) }), V);
      if (!r.ok) throw new Error(r.code);
      return r.plugin;
    };
    s.install(await dev('1.0.0'), 'one', []);
    expect(s.replace(await dev('1.0.1'), 'two')).toBe(true);
    expect(ids(s)).toEqual(['clock@1.0.1:dev:ready', 'agents@1.0.0:hub:new']);
    // Only the development plugin is kept in this browser; the hub's is never written there.
    expect(JSON.parse(m.get('agora.plugins.dev.v1') ?? '[]')).toEqual([{ text: 'two' }]);
    // A hub plugin is not a development plugin to replace.
    const hubOne = s.state().plugins.find((p) => !p.dev)!.plugin;
    expect(s.replace(hubOne, 'x')).toBe(false);
    // Signed out: only the development plugin is left.
    await s.attachHub(undefined);
    expect(ids(s)).toEqual(['clock@1.0.1:dev:ready']);
  });
});
