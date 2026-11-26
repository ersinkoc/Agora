// @vitest-environment jsdom
// The plugins dialog with a plugin installed on the hub (ADR-0114 decision 6): it is marked as the hub's, not "development"; it has no
// Remove (only the hub's owner removes it, on the hub's machine); the person looks at what it asks to see and shows it, and the grant is
// kept by the hub; then they can stop showing it.
import { createHash } from 'node:crypto';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { agora } from '../gen/agora.js';
import { setLanguage } from '../i18n';
import type { CommandBody } from '../net/live';
import { checkPackage } from './package';
import { PluginsDialog } from './PluginsDialog';
import { pluginStore } from './store';
import { ENTRY, baseManifest, bundle } from './testPackage';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(async () => { act(() => root.unmount()); host.remove(); await pluginStore.attachHub(undefined); });

const files = { 'panel.js': ENTRY };
const bytes = bundle(files, { manifest: baseManifest(files, { id: 'agents', name: 'Agent list', capabilities: ['agents.read', 'theme.read'] }) });
const sha = createHash('sha256').update(bytes).digest('hex');
const sent: CommandBody[] = [];
let grant: agora.v1.IPluginGrant | undefined;
let catalog: agora.v1.IPluginCatalogListResult = {};
const hub = {
  async command(body: CommandBody): Promise<agora.v1.CommandOk.$Properties> {
    sent.push(body);
    if (body.pluginList) return { pluginList: { plugins: [{ pluginId: 'agents', version: '1.0.0', name: 'Agent list', sha256: sha, size: bytes.length, capabilities: ['agents.read', 'theme.read'] }] } };
    if (body.pluginFetch) { const at = Number(body.pluginFetch.offset ?? 0); return { pluginFetch: { pluginId: 'agents', sha256: sha, totalSize: bytes.length, offset: at, data: bytes.slice(at) } }; }
    if (body.pluginGrantList) return { pluginGrantList: { grants: grant ? [grant] : [] } };
    if (body.pluginGrantSet) { grant = { pluginId: 'agents', version: '1.0.0', seen: ['agents.read', 'theme.read'], granted: body.pluginGrantSet.granted ?? [] }; return { empty: {} }; }
    if (body.pluginGrantRemove) { grant = undefined; return { empty: {} }; }
    if (body.pluginCatalogList) return { pluginCatalogList: catalog };
    throw new Error('protocol.invalid_message');
  },
};

const button = (text: string) => [...document.querySelectorAll('button')].find((b) => b.textContent === text);
const settle = async () => { for (let i = 0; i < 5; i++) await act(async () => { await Promise.resolve(); }); };

describe('a plugin installed on the hub in the plugins dialog', () => {
  it('is the hub\'s, not removable here, shown once the person allows it, and can be hidden again', async () => {
    await pluginStore.attachHub(hub);
    act(() => root.render(<PluginsDialog onClose={() => {}} />));
    const row = document.querySelector('[data-plugin-row="agents"]');
    expect(row?.textContent).toContain('Agent list');
    expect(row?.textContent).toContain('installed on this hub');
    expect(row?.textContent).not.toContain('development');
    expect(row?.textContent).toContain('Not shown');
    expect(button('Remove')).toBeUndefined();
    act(() => button('Look at it')?.click());
    expect(document.body.textContent).toContain('Show Agent list?');
    const box = document.querySelector<HTMLInputElement>('[data-cap="agents.read"]');
    act(() => box?.click());
    act(() => button('Show it')?.click());
    await settle();
    expect(sent.find((b) => b.pluginGrantSet)?.pluginGrantSet).toEqual({ pluginId: 'agents', version: '1.0.0', granted: ['agents.read'], sha256: sha });
    expect(pluginStore.state().plugins[0]?.state).toBe('ready');
    expect(document.querySelector('[data-plugin-row="agents"]')?.textContent).toContain('It sees one thing you allowed');
    act(() => button('Stop showing')?.click());
    await settle();
    expect(sent.some((b) => b.pluginGrantRemove)).toBe(true);
    expect(pluginStore.state().plugins[0]?.state).toBe('new');
  });

  it('lists a hub plugin a development plugin of its id replaces here, and says so on the install screen', async () => {
    grant = undefined;
    await pluginStore.attachHub(hub);
    const devFiles = { 'panel.js': `${ENTRY}//dev` };
    const r = await checkPackage(bundle(devFiles, { manifest: baseManifest(devFiles, { id: 'agents', name: 'Agent list (mine)', version: '2.0.0' }) }), '0.3.39');
    if (!r.ok) throw new Error(r.code);
    pluginStore.install(r.plugin, 'x', []);
    act(() => root.render(<PluginsDialog onClose={() => {}} />));
    const rows = [...document.querySelectorAll('[data-plugin-row="agents"]')];
    expect(rows.length).toBe(2);
    const shadowed = document.querySelector('[data-plugin-replaced]');
    expect(shadowed?.textContent).toContain('Agent list');
    expect(shadowed?.textContent).toContain('Replaced in this browser');
    expect(shadowed?.querySelector('button')).toBeNull();
    act(() => button('What it can see')?.click());
    expect(document.querySelector('[data-plugin-replaces]')?.textContent).toContain("Replaces the hub's Agent list in this browser");
    act(() => button('Cancel')?.click());
    pluginStore.remove('agents');
  });

  it('lists the catalog read only while it is on: what is not installed, a newer version, the owner\'s command, text as text', async () => {
    grant = undefined;
    const e = (pluginId: string, version: string, over: agora.v1.IPluginCatalogEntry = {}): agora.v1.IPluginCatalogEntry => ({
      pluginId, version, name: `Name ${pluginId}`, author: 'Someone', licence: 'MIT', capabilities: ['agents.read', 'chat.read'], sha256: 'a'.repeat(64), size: 10, ...over,
    });
    catalog = {};
    await pluginStore.attachHub(hub);
    act(() => root.render(<PluginsDialog onClose={() => {}} />));
    expect(document.querySelector('[data-plugin-catalog]')).toBeNull();
    act(() => root.unmount());
    root = createRoot(host);
    catalog = { on: true, entries: [
      e('agents', '1.0.0'),
      e('newer', '2.0.0', { name: '<img src=x onerror=alert(1)>', description: 'Shows <b>bold</b>' }),
      e('agents-two', '0.1.0', { capabilities: [] }),
    ] };
    await pluginStore.attachHub(hub);
    act(() => root.render(<PluginsDialog onClose={() => {}} />));
    // The installed version of `agents` is not offered again.
    expect(document.querySelector('[data-catalog-row="agents"]')).toBeNull();
    const row = document.querySelector('[data-catalog-row="newer"]');
    expect(row?.textContent).toContain('<img src=x onerror=alert(1)>');
    expect(row?.querySelector('img, b')).toBeNull();
    expect(row?.textContent).toContain('The conversation of the agent its pane shows');
    expect(row?.querySelector('[data-catalog-command]')?.textContent).toBe('agora hub plugins install newer');
    expect(row?.querySelector('button')).toBeNull();
    expect(document.querySelector('[data-catalog-row="agents-two"]')?.textContent).toContain('It asks to see nothing of yours.');
    act(() => root.unmount());
    root = createRoot(host);
    // A newer version of an installed plugin shows the owner's upgrade command.
    catalog = { on: true, entries: [e('agents', '1.1.0')] };
    await pluginStore.attachHub(hub);
    act(() => root.render(<PluginsDialog onClose={() => {}} />));
    expect(document.querySelector('[data-catalog-row="agents"] [data-catalog-command]')?.textContent).toBe('agora hub plugins upgrade agents');
    expect(document.querySelector('[data-catalog-row="agents"]')?.textContent).toContain('A newer version');
    catalog = {};
  });
});
