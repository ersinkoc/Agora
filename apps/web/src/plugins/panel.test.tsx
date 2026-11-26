// @vitest-environment jsdom
// A plugin panel in the window (ADR-0114 decisions 2 and 4; layout.md, "Host-set facts"): the window's pane bar names it with the plugin's
// own name as plain text and the "plugin" marker (on a tab too), it is offered in Show… and the chooser by that name, its frame is
// sandboxed with scripts only, a pane naming a plugin panel this window does not have shows the placeholder, and a plugin never stands
// in for a built-in panel. Safe mode and the frame's page policy are checked here too.
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { setLanguage } from '../i18n';
import type { AgentTileModel } from '../model';
import type { AnswerGuard } from '../components/useAnswerGuard';
import { BUILTIN_REGISTRY, type HostActions } from '../layout/builtinPanels';
import { workspaceOf, type Workspace as WorkspaceState } from '../layout/engine';
import type { PanelRegistry, PanelState } from '../layout/panels';
import { LAYOUT_VERSION, type LayoutNode } from '../layout/tree';
import { Workspace, panelName } from '../layout/Workspace';
import { PluginFrame } from './PluginFrame';
import { MAX_WATCHES } from './protocol';
import { viewOf } from './projection';
import { FRAME_POLICY, FRAME_SANDBOX, frameUrl } from './frameDoc';
import { checkPackage } from './package';
import { pluginPanels, withPlugins } from './registry';
import { isSafe, isSafeKey, safeAddress, unsafeAddress } from './safe';
import type { LoadedPlugin } from './store';
import { ENTRY, baseManifest, bundle } from './testPackage';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
(globalThis as { ResizeObserver?: unknown }).ResizeObserver = class { observe() {} unobserve() {} disconnect() {} };

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); document.body.querySelectorAll('[role=menu]').forEach((m) => m.remove()); });

const agent = (id: string, name: string): AgentTileModel => ({ id, name, accent: 'var(--agent-teal)', machine: 'Laptop', workspace: 'proj', project: 'proj', state: 'working', preview: '' });
const fleet = [agent('n/a', 'claude · amber')];
const state: PanelState = Object.freeze({ fleet, machines: [], approvals: [], attention: [], changed: [], messages: [], boards: [], projects: [], now: 0, link: 'live' as const });
const none = () => {};
const actions: HostActions = Object.freeze({ openAgent: none, openFromHome: none, openTool: none, review: none, history: none, browse: none, showNeeds: none, showChanges: none, dismiss: none, answers: {} as AnswerGuard, chat: {} });
const ws = (root: LayoutNode): WorkspaceState => workspaceOf({ version: LAYOUT_VERSION, root });
const pane = (id: string, panel: string) => ({ type: 'pane' as const, id, panel });

async function loaded(name: string, extra: Record<string, unknown> = {}): Promise<LoadedPlugin> {
  const files = { 'panel.js': ENTRY };
  const r = await checkPackage(bundle(files, { manifest: baseManifest(files, { name, ...extra }) }), '0.3.39');
  if (!r.ok) throw new Error(r.code);
  return { plugin: r.plugin, text: '', dev: true, state: 'ready', granted: ['agents.read'] };
}
function draw(start: WorkspaceState, panels: PanelRegistry) {
  act(() => root.render(<div style={{ position: 'relative', width: 800, height: 600 }}>
    <Workspace ws={start} dispatch={() => {}} fleet={fleet} bindings={{ attention: new Map() }} narrow={false} panels={panels} state={state} sources={{}} actions={actions}
      terminalFor={() => ({ connect: () => ({ sendInput() {}, resize() {}, close() {} }) })} welcome={<p data-welcome>welcome</p>} />
  </div>));
}

describe('a plugin panel in the window', () => {
  it('is named by the plugin\'s own name as text, never as one of the window\'s words, with the marker beside it', async () => {
    // A plugin named like a Fluent key of the window ("Needs you") is shown as exactly that text.
    const reg = withPlugins(BUILTIN_REGISTRY, [await loaded('attention-title')]);
    expect(panelName(reg, 'plugin.demo.main')).toBe('attention-title');
    draw(ws({ type: 'split', direction: 'row', ratios: [0.5, 0.5], children: [pane('p1', 'plugin.demo.main'), pane('p2', 'usage')] }), reg);
    const bar = host.querySelector('.pane-bar') as HTMLElement;
    expect(bar.querySelector('[data-pane-name]')?.textContent).toBe('attention-title');
    expect(bar.querySelector('[data-plugin-mark]')?.textContent).toBe('Plugin development');
    expect(host.textContent).not.toContain('Needs you');
    // Only the plugin's pane has the marker.
    expect(host.querySelectorAll('[data-plugin-mark]')).toHaveLength(1);
  });

  it('keeps the marker on its tab in a tab group', async () => {
    const reg = withPlugins(BUILTIN_REGISTRY, [await loaded('Agents at a glance')]);
    draw(ws({ type: 'split', direction: 'row', ratios: [0.5, 0.5], children: [{ type: 'tabs', active: 1, children: [pane('p1', 'plugin.demo.main'), pane('p3', 'usage')] }, pane('p2', 'tasks')] }), reg);
    const tab = host.querySelector('#tab-p1') as HTMLElement;
    expect(tab.textContent).toContain('Agents at a glance');
    expect(tab.querySelector('[data-plugin-mark]')).not.toBeNull();
  });

  it('is offered in the empty pane\'s chooser by its name', async () => {
    const reg = withPlugins(BUILTIN_REGISTRY, [await loaded('Agents at a glance')]);
    draw(ws({ type: 'split', direction: 'row', ratios: [0.5, 0.5], children: [pane('p1', 'empty'), pane('p2', 'usage')] }), reg);
    expect([...host.querySelectorAll('button.ide-chip')].map((b) => b.textContent)).toContain('Agents at a glance');
  });

  it('draws a frame with scripts only and nothing else of the window\'s', async () => {
    const reg = withPlugins(BUILTIN_REGISTRY, [await loaded('Demo')]);
    draw(ws({ type: 'split', direction: 'row', ratios: [0.5, 0.5], children: [pane('p1', 'plugin.demo.main'), pane('p2', 'usage')] }), reg);
    const frame = host.querySelector('iframe') as HTMLIFrameElement;
    expect(frame.getAttribute('sandbox')).toBe('allow-scripts');
    expect(frame.getAttribute('src')).toBe('/plugin-frame.html');
    expect(frame.getAttribute('referrerpolicy')).toBe('no-referrer');
    expect(frame.closest('[data-pane]')?.getAttribute('data-pane')).toBe('p1');
    expect(FRAME_SANDBOX).toBe('allow-scripts');
    const src = readFileSync(join(__dirname, 'PluginFrame.tsx'), 'utf8') + readFileSync(join(__dirname, 'frameDoc.ts'), 'utf8');
    for (const flag of ['allow-same-origin', 'allow-forms', 'allow-popups', 'allow-modals', 'allow-top-navigation', 'allow-downloads']) expect(src.includes(`"${flag}`) || src.includes(`'${flag}`), flag).toBe(false);
  });

  it('keeps the frame inert until the person clicks into it, and inert again once they click elsewhere', async () => {
    const reg = withPlugins(BUILTIN_REGISTRY, [await loaded('Demo')]);
    draw(ws({ type: 'split', direction: 'row', ratios: [0.5, 0.5], children: [pane('p1', 'plugin.demo.main'), pane('p2', 'usage')] }), reg);
    const frame = () => host.querySelector('iframe') as HTMLIFrameElement;
    const box = host.querySelector('[data-plugin-box]') as HTMLElement;
    expect(frame().hasAttribute('inert')).toBe(true);
    expect(box.getAttribute('aria-label')).toBe('Demo (plugin): click or press Enter to use it');
    act(() => { box.dispatchEvent(new Event('pointerdown', { bubbles: true })); });
    expect(frame().hasAttribute('inert')).toBe(false);
    act(() => { (host.querySelector('[data-pane="p2"]') as HTMLElement).dispatchEvent(new Event('pointerdown', { bubbles: true })); });
    expect(frame().hasAttribute('inert')).toBe(true);
    // Enter on the frame's box is the keyboard's way in.
    act(() => { box.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true })); });
    expect(frame().hasAttribute('inert')).toBe(false);
  });

  it('offers safe mode in a plugin pane\'s menu', async () => {
    const reg = withPlugins(BUILTIN_REGISTRY, [await loaded('Demo')]);
    draw(ws({ type: 'split', direction: 'row', ratios: [0.5, 0.5], children: [pane('p1', 'plugin.demo.main'), pane('p2', 'usage')] }), reg);
    const menus = [...host.querySelectorAll('button[aria-haspopup]')] as HTMLButtonElement[];
    act(() => { menus[0]?.click(); });
    const items = [...document.querySelectorAll('[role=menuitem]')].map((x) => x.textContent);
    expect(items).toContain('Open in safe mode');
    act(() => { (document.activeElement as HTMLElement | null)?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })); });
  });

  it('shows the placeholder for a plugin panel this window does not have, and never replaces a built-in', async () => {
    draw(ws({ type: 'split', direction: 'row', ratios: [0.5, 0.5], children: [pane('p1', 'plugin.gone.main'), pane('p2', 'usage')] }), BUILTIN_REGISTRY);
    expect(host.querySelector('[data-pane="p1"]')?.textContent).toContain('plugin.gone.main');
    expect(host.querySelector('iframe')).toBeNull();
    const p = await loaded('Demo', { panels: [{ id: 'approvals', kind: 'global' }] });
    const reg = withPlugins(BUILTIN_REGISTRY, [p, p]);
    expect(reg.get('approvals')).toStrictEqual(BUILTIN_REGISTRY.get('approvals'));
    for (const id of BUILTIN_REGISTRY.ids) { expect(reg.get(id)?.render, id).toBe(BUILTIN_REGISTRY.get(id)?.render); expect(reg.get(id)?.plugin, id).toBeUndefined(); }
    expect(pluginPanels([p, p]).map((d) => d.id)).toEqual(['plugin.demo.approvals', 'plugin.demo.approvals']);
    expect(reg.ids.filter((id) => id.startsWith('plugin.'))).toEqual(['plugin.demo.approvals']);
    // No plugin panel stands alone without the bar or draws a terminal.
    for (const d of pluginPanels([p])) expect([d.lone, d.streams]).toEqual([false, false]);
  });
});

describe('the frame\'s page', () => {
  // A Rust string constant of web.rs, its line continuations joined.
  const rs = readFileSync(join(__dirname, '../../../../crates/agora-hub/src/web.rs'), 'utf8');
  const constant = (name: string) => {
    const m = new RegExp(`const ${name}: &str = "((?:[^"\\\\]|\\\\.)*)";`, 's').exec(rs);
    return m?.[1]?.replace(/\\\n\s*/g, '');
  };

  it('is served by the hub under the policy the window expects, and the window frames only its own origin', () => {
    expect(constant('PLUGIN_FRAME_CSP')).toBe(FRAME_POLICY);
    expect(constant('PLUGIN_FRAME_PATH')).toBe(`/${'plugin-frame.html'}`);
    expect(frameUrl('/')).toBe(constant('PLUGIN_FRAME_PATH'));
    expect(frameUrl('/app')).toBe('/app/plugin-frame.html');
    const app = constant('APP_CSP') ?? '';
    expect(app).toContain("frame-src 'self'");
    expect(app).toContain("frame-ancestors 'none'");
    // The window's own policy is not loosened for plugins: no blob scripts, no other frames.
    expect(app).not.toContain('blob:');
    expect(app).not.toMatch(/script-src/);
    for (const d of ["sandbox allow-scripts", "default-src 'none'", "connect-src 'none'", "frame-src 'none'", "form-action 'none'", "base-uri 'none'", "frame-ancestors 'self'"]) expect(FRAME_POLICY).toContain(d);
  });

  it('holds the same policy in the page itself, without what only a header can say, and runs only the loader', () => {
    const html = readFileSync(join(__dirname, '../../public/plugin-frame.html'), 'utf8');
    const doc = new DOMParser().parseFromString(html, 'text/html');
    const first = doc.head.firstElementChild as HTMLMetaElement;
    expect(first.httpEquiv).toBe('Content-Security-Policy');
    expect(first.content).toBe(FRAME_POLICY.replace('sandbox allow-scripts; ', '').replace("; frame-ancestors 'self'", ''));
    expect([...doc.scripts].map((x) => x.getAttribute('src'))).toEqual(['plugin-frame.js']);
    expect(doc.body.innerHTML.trim()).toBe('');
  });

  it('ships the loader beside the window\'s files, refusing to run anywhere but an opaque frame', () => {
    const loader = readFileSync(join(__dirname, '../../public/plugin-frame.js'), 'utf8');
    expect(loader).toContain("if (self.origin !== 'null' || window.parent === window) return;");
    expect(loader).toContain('if (e.source !== host) return;');
  });
});

describe('safe mode', () => {
  it('is on with ?safe in the address, and the key is Ctrl or ⌘ with Alt, Shift and S by its place', () => {
    expect(isSafe('?safe')).toBe(true);
    expect(isSafe('?x=1&safe=')).toBe(true);
    expect(isSafe('')).toBe(false);
    expect(isSafe('?unsafe')).toBe(false);
    const key = { code: 'KeyS', ctrlKey: true, metaKey: false, altKey: true, shiftKey: true };
    expect(isSafeKey(key)).toBe(true);
    expect(isSafeKey({ ...key, ctrlKey: false, metaKey: true })).toBe(true);
    expect(isSafeKey({ ...key, shiftKey: false })).toBe(false);
    expect(isSafeKey({ ...key, code: 'KeyA' })).toBe(false);
    expect(safeAddress('https://hub/#/demo')).toBe('https://hub/?safe#/demo');
    expect(safeAddress('https://hub/?a=1#/x')).toBe('https://hub/?a=1&safe#/x');
    expect(unsafeAddress('https://hub/?safe#/demo')).toBe('https://hub/#/demo');
  });

  it('keeps the stored theme and plugins out: neither store reads storage in safe mode', () => {
    expect(readFileSync(join(__dirname, '../themes/store.ts'), 'utf8')).toContain('kv: SAFE ? undefined : storage(),');
    // The plugin store keeps its storage only to list and remove what is kept (grants.test.ts: nothing is checked or shown).
    expect(readFileSync(join(__dirname, 'store.ts'), 'utf8')).toContain('createPluginStore({ kv: storage(), version: AGORA_VERSION, safe: SAFE })');
    // The key is listened for before the theme and the plugins start.
    const main = readFileSync(join(__dirname, '../main.tsx'), 'utf8');
    expect(main.indexOf('installSafeKey();')).toBeLessThan(main.indexOf('initTheme();'));
  });
});

describe('the bridge', () => {
  // F1822: a watch the window refuses at once (`unavailable`) is not running: the frame's loader forgets it and never sends `unwatch`, so
  // it takes none of the frame's MAX_WATCHES slots.
  it('keeps no slot for a watch it refused', async () => {
    const files = { 'panel.js': ENTRY };
    const r = await checkPackage(bundle(files, { manifest: baseManifest(files, { capabilities: ['agents.read', 'chat.read', 'changes.read'] }) }), '0.3.39');
    if (!r.ok) throw new Error(r.code);
    // A global pane is bound to no agent: its conversation and changes are unavailable.
    const view = viewOf({ pane: 'p1', kind: 'global', fleet, boards: [], now: 0 });
    act(() => root.render(<PluginFrame plugin={r.plugin} panel={r.plugin.manifest.panels[0]!} granted={['agents.read', 'chat.read', 'changes.read']} view={view} dev />));
    const win = (host.querySelector('iframe') as HTMLIFrameElement).contentWindow as Window;
    const got: { id: number; ok?: boolean; error?: string }[] = [];
    (win as unknown as { postMessage: (m: unknown) => void }).postMessage = (m) => { got.push(m as never); };
    const send = (data: unknown) => act(() => { window.dispatchEvent(new MessageEvent('message', { source: win, data })); });
    for (let id = 1; id <= MAX_WATCHES; id++) send({ agora: 1, type: 'watch', id, cap: id % 2 ? 'chat.read' : 'changes.read' });
    await act(async () => { await Promise.resolve(); });
    expect(got.filter((m) => m.error === 'unavailable')).toHaveLength(MAX_WATCHES);
    send({ agora: 1, type: 'watch', id: 100, cap: 'agents.read' });
    expect(got.find((m) => m.id === 100)).toMatchObject({ ok: true });
  });
});
