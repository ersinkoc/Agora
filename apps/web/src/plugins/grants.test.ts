// Grants (ADR-0114 decision 3, "Granting", and Verification, "Grants"): the person allows each capability; one not allowed answers "not
// granted"; a version that asks for more is held until the person looks again. The development store keeps packages and grants in
// this browser only, checks every package again at start, and loads nothing in safe mode.
import { describe, expect, it } from 'vitest';
import { grant, grantState, granted, readGrants } from './grants';
import { checkPackage } from './package';
import { DEV_KEY, GRANTS_KEY, MAX_DEV_PLUGINS, createPluginStore } from './store';
import { ENTRY, baseManifest, bundle } from './testPackage';

const m1 = { version: '1.0.0', capabilities: ['agents.read', 'theme.read'] as const };
const m2 = { version: '1.1.0', capabilities: ['agents.read', 'theme.read', 'chat.read'] as const };

describe('grants', () => {
  it('asks a new plugin, answers only what was allowed, and holds a version that asks for more', () => {
    expect(grantState(m1, undefined)).toBe('new');
    const g = grant(m1, ['agents.read', 'usage.read']);
    expect(g).toEqual({ version: '1.0.0', seen: ['agents.read', 'theme.read'], granted: ['agents.read'] });
    expect(grantState(m1, g)).toBe('ready');
    expect(granted(m1, g)).toEqual(['agents.read']);
    // The next version adds chat.read: held, and its frame is answered for nothing until the person looks again.
    expect(grantState(m2, g)).toBe('held');
    expect(granted(m2, g)).toEqual([]);
    const g2 = grant(m2, ['agents.read']);
    expect(grantState(m2, g2)).toBe('ready');
    expect(granted(m2, g2)).toEqual(['agents.read']);
    // A version that asks for less is not held, and is answered only for what it still asks.
    expect(granted({ capabilities: ['theme.read'] }, g2)).toEqual([]);
  });

  it('reads stored grants without trusting them: unknown capabilities and grants never shown are dropped', () => {
    expect(readGrants({ a: { version: '1', seen: ['agents.read', 'terminal.read'], granted: ['agents.read', 'chat.read', 'terminal.read'] }, b: 'x', c: null })).toEqual({
      a: { version: '1', seen: ['agents.read'], granted: ['agents.read'] },
    });
    expect(readGrants('nope')).toEqual({});
    expect(readGrants([1])).toEqual({});
  });
});

function memory() {
  const m = new Map<string, string>();
  return { m, kv: { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => { m.set(k, v); }, removeItem: (k: string) => { m.delete(k); } } };
}
const file = (bytes: Uint8Array) => ({ size: bytes.length, arrayBuffer: async () => bytes.slice().buffer as ArrayBuffer });

describe('the development store', () => {
  it('keeps a loaded package and what the person allowed, in this browser, and checks it again at the next start', async () => {
    const { m, kv } = memory();
    const s = createPluginStore({ kv, version: '0.3.39' });
    const r = await s.check(file(bundle()));
    expect(r.ok).toBe(true);
    if (!r.ok || !r.text) return;
    expect(s.install(r.plugin, r.text, ['agents.read'])).toBe('kept');
    expect(s.state().plugins.map((p) => [p.plugin.manifest.id, p.state, p.granted, p.dev])).toEqual([['demo', 'ready', ['agents.read'], true]]);
    expect(JSON.parse(m.get(GRANTS_KEY) ?? '{}')).toEqual({ demo: { version: '1.0.0', seen: ['agents.read'], granted: ['agents.read'] } });

    const again = createPluginStore({ kv, version: '0.3.39' });
    expect(again.state().ready).toBe(false);
    await again.start();
    expect(again.state().plugins.map((p) => p.granted)).toEqual([['agents.read']]);

    // A kept package that no longer passes (edited in storage) is left out.
    const tampered = JSON.parse(m.get(DEV_KEY) ?? '[]');
    tampered[0].text = tampered[0].text.replace(/"panel.js":"[^"]+"/, '"panel.js":"YWxlcnQoMSk="');
    m.set(DEV_KEY, JSON.stringify(tampered));
    const third = createPluginStore({ kv, version: '0.3.39' });
    await third.start();
    expect(third.state().plugins).toEqual([]);
  });

  it('changes what a plugin sees, removes it with its grant, and refuses a fifth plugin', async () => {
    const { m, kv } = memory();
    const s = createPluginStore({ kv, version: '0.3.39' });
    for (let i = 0; i < MAX_DEV_PLUGINS; i++) {
      const files = { 'panel.js': `${ENTRY}//${i}` };
      const r = await checkPackage(bundle(files, { manifest: baseManifest(files, { id: `p${i}` }) }), '0.3.39');
      if (r.ok) expect(s.install(r.plugin, 'x', [])).toBe('kept');
    }
    const files = { 'panel.js': ENTRY };
    const fifth = await checkPackage(bundle(files, { manifest: baseManifest(files, { id: 'p9' }) }), '0.3.39');
    if (fifth.ok) expect(s.install(fifth.plugin, 'x', [])).toBe('full');
    s.regrant('p0', ['agents.read']);
    expect(s.state().plugins.find((p) => p.plugin.manifest.id === 'p0')?.granted).toEqual(['agents.read']);
    s.remove('p0');
    expect(s.state().plugins.map((p) => p.plugin.manifest.id)).toEqual(['p1', 'p2', 'p3']);
    expect(Object.keys(JSON.parse(m.get(GRANTS_KEY) ?? '{}'))).toEqual(['p1', 'p2', 'p3']);
  });

  it('holds for the visit when this browser cannot store, and refuses a file over 2 MiB before reading it', async () => {
    const kv = { getItem: () => null, setItem: () => { throw new Error('quota'); }, removeItem: () => {} };
    const s = createPluginStore({ kv, version: '0.3.39' });
    const r = await s.check(file(bundle()));
    if (r.ok && r.text) expect(s.install(r.plugin, r.text, [])).toBe('visit');
    expect(s.state().plugins).toHaveLength(1);
    let read = false;
    expect(await s.check({ size: 3 * 1024 * 1024, arrayBuffer: async () => { read = true; return new ArrayBuffer(0); } })).toEqual({ ok: false, code: 'too-large' });
    expect(read).toBe(false);
  });

  it('loads nothing in safe mode', async () => {
    const { kv } = memory();
    const s0 = createPluginStore({ kv, version: '0.3.39' });
    const r = await s0.check(file(bundle()));
    if (r.ok && r.text) s0.install(r.plugin, r.text, ['agents.read']);
    const safe = createPluginStore({ kv, version: '0.3.39', safe: true });
    await safe.start();
    expect(safe.state()).toMatchObject({ plugins: [], safe: true, ready: true });
    expect((await safe.check(file(bundle()))).ok).toBe(false);
  });
});
