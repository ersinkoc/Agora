// @vitest-environment jsdom
// Reload on save in the window (roadmap M5.7; docs/ux/plugins.md, "Reload on save"): a development plugin loaded again from its changed
// file gets a new frame (the loader takes one `load`), inert like any new frame; a mere new view keeps the frame; a changed file that was
// not loaded is said above the frame in the window's words; and the plugins dialog uses the picker that can read a file again and says
// the plugin is watched.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { setLanguage, t } from '../i18n';
import type { AgentTileModel } from '../model';
import type { AnswerGuard } from '../components/useAnswerGuard';
import { BUILTIN_REGISTRY, type HostActions } from '../layout/builtinPanels';
import { workspaceOf } from '../layout/engine';
import type { PanelRegistry, PanelState } from '../layout/panels';
import { LAYOUT_VERSION } from '../layout/tree';
import { Workspace } from '../layout/Workspace';
import { devReload, type WatchedFile } from './devReload';
import { checkPackage } from './package';
import { PluginsDialog, reloadProblemText } from './PluginsDialog';
import { withPlugins } from './registry';
import { type LoadedPlugin, pluginStore } from './store';
import { ENTRY, baseManifest, bundle } from './testPackage';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
(globalThis as { ResizeObserver?: unknown }).ResizeObserver = class { observe() {} unobserve() {} disconnect() {} };

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  pluginStore.remove('demo');
  delete (window as { showOpenFilePicker?: unknown }).showOpenFilePicker;
});

const agent = (id: string, name: string): AgentTileModel => ({ id, name, accent: 'var(--agent-teal)', machine: 'Laptop', workspace: 'proj', project: 'proj', state: 'working', preview: '' });
const fleet = [agent('n/a', 'claude · amber')];
const state: PanelState = Object.freeze({ fleet, machines: [], approvals: [], attention: [], changed: [], messages: [], boards: [], projects: [], now: 0, link: 'live' as const });
const none = () => {};
const actions: HostActions = Object.freeze({ openAgent: none, openFromHome: none, openTool: none, review: none, history: none, browse: none, showNeeds: none, showChanges: none, dismiss: none, answers: {} as AnswerGuard, chat: {} });
const start = workspaceOf({ version: LAYOUT_VERSION, root: { type: 'split', direction: 'row', ratios: [0.5, 0.5], children: [{ type: 'pane', id: 'p1', panel: 'plugin.demo.main' }, { type: 'pane', id: 'p2', panel: 'usage' }] } });
function draw(panels: PanelRegistry) {
  act(() => root.render(<div style={{ position: 'relative', width: 800, height: 600 }}>
    <Workspace ws={start} dispatch={() => {}} fleet={fleet} bindings={{ attention: new Map() }} narrow={false} panels={panels} state={state} sources={{}} actions={actions}
      terminalFor={() => ({ connect: () => ({ sendInput() {}, resize() {}, close() {} }) })} welcome={<p />} />
  </div>));
}

const files = { 'panel.js': ENTRY };
const pkg = (version: string) => bundle(files, { manifest: baseManifest(files, { version }) });
async function loaded(version: string): Promise<LoadedPlugin> {
  const r = await checkPackage(pkg(version), '0.3.39');
  if (!r.ok) throw new Error(r.code);
  return { plugin: r.plugin, text: '', dev: true, state: 'ready', granted: ['agents.read'] };
}

describe('reload on save in the window', () => {
  it('starts a new, inert frame for a package loaded again, and keeps the frame for anything else', async () => {
    const first = await loaded('1.0.0');
    draw(withPlugins(BUILTIN_REGISTRY, [first]));
    const before = host.querySelector('iframe') as HTMLIFrameElement;
    act(() => { (host.querySelector('[data-plugin-box]') as HTMLElement).dispatchEvent(new Event('pointerdown', { bubbles: true })); });
    expect(before.hasAttribute('inert')).toBe(false);
    // The same package drawn again (the window's state moved): the same frame, still in use.
    draw(withPlugins(BUILTIN_REGISTRY, [{ ...first }]));
    expect(host.querySelector('iframe')).toBe(before);
    // The package loaded again from its changed file: a new frame, which the loader boots afresh, inert until clicked into.
    draw(withPlugins(BUILTIN_REGISTRY, [await loaded('1.0.1')]));
    const after = host.querySelector('iframe') as HTMLIFrameElement;
    expect(after).not.toBe(before);
    expect(before.isConnected).toBe(false);
    expect(after.hasAttribute('inert')).toBe(true);
  });

  it('says above the frame, in the window\'s words, when a changed file was not loaded', async () => {
    const text = new TextDecoder().decode(pkg('1.0.0'));
    const r = await checkPackage(pkg('1.0.0'), '0.3.39');
    if (!r.ok) throw new Error(r.code);
    pluginStore.install(r.plugin, text, ['agents.read']);
    const gone: WatchedFile = { getFile: async () => { throw new Error('NotFoundError'); } };
    devReload.watch('demo', gone, { size: 1, lastModified: 1 });
    await act(async () => { await devReload.tick(); });
    draw(withPlugins(BUILTIN_REGISTRY, pluginStore.state().plugins));
    const note = host.querySelector('[data-plugin-reload-problem="demo"]') as HTMLElement;
    expect(note.textContent).toContain('The file can no longer be read');
    // Outside the frame's box: the frame cannot reach or draw over it.
    expect(note.closest('[data-plugin-box]')).toBeNull();
    expect(host.querySelector('iframe')).not.toBeNull();
    pluginStore.remove('demo');
    expect(devReload.state().watching).toEqual([]);
  });

  it('words every reason in English and Turkish', () => {
    for (const lang of ['en', 'tr']) {
      setLanguage(lang);
      for (const p of [{ code: 'hash', detail: 'panel.js' }, { code: 'other-id', detail: 'x' }, { code: 'unreadable' }] as const) {
        const s = reloadProblemText('Demo', p);
        expect(s, `${lang} ${p.code}`).not.toMatch(/plugin-(reload|error)-/);
        expect(s).toContain('Demo');
      }
      for (const k of ['plugin-reload-on', 'plugin-reload-unsupported', 'plugin-file-kind']) expect(t(k), k).not.toBe(k);
    }
    setLanguage('en');
  });

  it('loads through the picker that can read the file again, and says the plugin is watched', async () => {
    let bytes = pkg('1.0.0');
    let at = 5;
    // Read at once (a jsdom File reads on a later turn, which `act` does not wait for).
    const handle: WatchedFile = { getFile: async () => { const b = bytes; return { size: b.length, lastModified: at, arrayBuffer: async () => b.slice().buffer }; } };
    let asked = '';
    (window as { showOpenFilePicker?: unknown }).showOpenFilePicker = async (o: { types: { description: string }[] }) => { asked = o.types[0]?.description ?? ''; return [handle]; };
    act(() => root.render(<PluginsDialog onClose={() => {}} />));
    expect(document.querySelector('[data-plugin-reload-unsupported]')).toBeNull();
    const load = [...document.querySelectorAll('button')].find((b) => b.textContent === t('plugin-load')) as HTMLButtonElement;
    await act(async () => { load.click(); });
    expect(asked).toBe('Agora plugin package');
    await act(async () => { (document.querySelector('[data-plugin-save]') as HTMLButtonElement).click(); });
    expect(document.querySelector('[data-plugin-reload="demo"]')?.textContent).toBe('Starts again when its file changes.');
    expect(devReload.state().watching).toEqual(['demo']);
    bytes = pkg('1.0.1');
    at = 6;
    await act(async () => { await devReload.tick(); });
    expect(pluginStore.state().plugins[0]?.plugin.manifest.version).toBe('1.0.0');
    await act(async () => { await devReload.tick(); });
    expect(pluginStore.state().plugins[0]?.plugin.manifest.version).toBe('1.0.1');
    expect(document.querySelector('[data-plugin-reload="demo"]')).not.toBeNull();
  });

  it('says where a browser cannot watch a file', () => {
    act(() => root.render(<PluginsDialog onClose={() => {}} />));
    expect(document.querySelector('[data-plugin-reload-unsupported]')?.textContent).toBe(t('plugin-reload-unsupported'));
  });
});
