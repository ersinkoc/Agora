// The hub's community plugin catalog in the window (ADR-0114 decision 7, build step 4): listed read only while the owner has it on,
// nothing from an older hub that refuses the message, and nothing asked in safe mode. Entries the window cannot use are left out, and
// a capability name it does not know is counted, never shown as its own words.
import { describe, expect, it } from 'vitest';
import type { agora } from '../gen/agora.js';
import type { CommandBody } from '../net/live';
import { type HubLink, compareVersions, loadHubCatalog } from './hub';
import { createPluginStore } from './store';

function hubWith(answer: agora.v1.IPluginCatalogListResult | Error) {
  const sent: string[] = [];
  const link: HubLink = {
    async command(body: CommandBody): Promise<agora.v1.CommandOk.$Properties> {
      sent.push(Object.keys(body)[0] ?? '');
      if (body.pluginCatalogList) { if (answer instanceof Error) throw answer; return { pluginCatalogList: answer }; }
      if (body.pluginList) return { pluginList: { plugins: [] } };
      if (body.pluginGrantList) return { pluginGrantList: { grants: [] } };
      throw new Error('protocol.invalid_message');
    },
  };
  return { link, sent };
}

const entry = (over: Partial<agora.v1.IPluginCatalogEntry> = {}): agora.v1.IPluginCatalogEntry => ({
  pluginId: 'demo', name: 'Demo', version: '1.2.0', author: 'Someone', licence: 'MIT', description: 'Lists the agents.',
  capabilities: ['agents.read'], sha256: 'a'.repeat(64), size: 1000, ...over,
});

describe('the hub\'s plugin catalog', () => {
  it('is listed only while it is on, and an older hub gives none', async () => {
    expect(await loadHubCatalog(hubWith({}).link)).toEqual({ on: false });
    expect(await loadHubCatalog(hubWith(new Error('protocol.unsupported')).link)).toBeUndefined();
    const on = await loadHubCatalog(hubWith({ on: true, entries: [entry()], updatedAtMs: 5 }).link);
    expect(on).toEqual({ on: true, entries: [{ id: 'demo', name: 'Demo', version: '1.2.0', author: 'Someone', licence: 'MIT', description: 'Lists the agents.', capabilities: ['agents.read'], unknown: 0 }] });
  });

  it('leaves out an entry it cannot use and counts a capability it does not know', async () => {
    const c = await loadHubCatalog(hubWith({ on: true, entries: [
      entry({ pluginId: 'BAD' }), entry({ pluginId: 'v', version: '1.2' }), entry(), entry({ name: 'Twice' }),
      entry({ pluginId: 'new', capabilities: ['agents.read', 'terminal.write'] }),
    ] }).link);
    if (!c?.on) throw new Error('off');
    expect(c.entries.map((e) => e.id)).toEqual(['demo', 'new']);
    expect(c.entries[1]).toMatchObject({ capabilities: ['agents.read'], unknown: 1 });
  });

  it('follows the hub in the store, and safe mode asks nothing', async () => {
    const s = createPluginStore({ version: '0.3.39' });
    const h = hubWith({ on: true, entries: [entry()] });
    await s.attachHub(h.link);
    expect(s.state().catalog).toMatchObject({ on: true, entries: [{ id: 'demo' }] });
    await s.attachHub(undefined);
    expect(s.state().catalog).toBeUndefined();
    const safe = createPluginStore({ version: '0.3.39', safe: true });
    const quiet = hubWith({ on: true, entries: [entry()] });
    await safe.attachHub(quiet.link);
    expect(quiet.sent).toEqual([]);
    expect(safe.state().catalog).toBeUndefined();
  });

  it('orders versions by their numbers', () => {
    expect(compareVersions('1.10.0', '1.9.9')).toBe(1);
    expect(compareVersions('1.0.0', '1.0.0')).toBe(0);
    expect(compareVersions('0.9.0', '1.0.0')).toBe(-1);
  });
});
