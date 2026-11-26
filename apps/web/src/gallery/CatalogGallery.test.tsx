// @vitest-environment jsdom
// The Gallery (ADR-0120): both community catalogs on one screen, sorted by the popularity and day the catalogs publish, searched,
// themes previewed from their own files and installed in one step, plugins shown with the screenshots the hub kept and the owner's
// command. The window asks only the hub.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { agora } from '../gen/agora.js';
import { setLanguage, t } from '../i18n';
import type { CommandBody } from '../net/live';
import { pluginStore } from '../plugins/store';
import { type CatalogAsk, setCatalogSource } from '../themes/catalog';
import { themeStore } from '../themes/store';
import { CatalogGallery, type GalleryTab } from './CatalogGallery';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const theme = (name: string, accent: string) => JSON.stringify({ 'agora-theme': 1, name, scheme: 'dark', tokens: { accent, 'on-accent': '#000000' } });
const THEMES: Record<string, { name: string; author: string; added: string; popularity: number; data: string }> = {
  harbour: { name: 'Harbour', author: 'Mira', added: '2026-10-01', popularity: 40, data: theme('Harbour', '#4fb3d9') },
  ember: { name: 'Ember', author: 'Tolga', added: '', popularity: 90, data: theme('Ember', '#f08a4b') },
  moss: { name: 'Moss', author: 'Alex', added: '2026-11-20', popularity: 5, data: theme('Moss', '#8fc46b') },
};
const themeEntry = (id: string) => {
  const f = THEMES[id]!;
  return { id, name: f.name, version: 1, author: f.author, licence: 'MIT', sha256: 'a'.repeat(64), size: f.data.length, added: f.added, popularity: f.popularity };
};
const asked: string[] = [];
const ask: CatalogAsk = async (body) => {
  if ('themeCatalogList' in body) return { themeCatalogList: { on: true, updatedAtMs: 1, entries: Object.keys(THEMES).map(themeEntry) } };
  asked.push(body.themeCatalogGet.id);
  return { themeCatalogTheme: { entry: themeEntry(body.themeCatalogGet.id), data: THEMES[body.themeCatalogGet.id]!.data } };
};

const PNG = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, 0x49, 0x48, 0x44, 0x52]);
const plugin = (over: Partial<agora.v1.IPluginCatalogEntry>): agora.v1.IPluginCatalogEntry => ({
  pluginId: 'board', name: 'Build board', version: '1.2.0', author: 'Mira', licence: 'MIT', description: 'Shows builds.',
  capabilities: ['agents.read'], sha256: 'a'.repeat(64), size: 1000, ...over,
});
const link = {
  async command(body: CommandBody): Promise<agora.v1.CommandOk.$Properties> {
    if (body.pluginCatalogList) return { pluginCatalogList: { on: true, updatedAtMs: 1, entries: [
      plugin({ popularity: 3, added: '2026-11-01', screenshots: ['The board in the dark theme'] }),
      plugin({ pluginId: 'timer', name: 'Focus timer', author: 'Noor', capabilities: [], popularity: 12 }),
    ] } };
    if (body.pluginCatalogScreenshot) {
      if (body.pluginCatalogScreenshot.pluginId !== 'board' || body.pluginCatalogScreenshot.index !== 0) throw new Error('plugin.not_found');
      return { pluginCatalogScreenshot: { mediaType: 'image/png', data: PNG, alt: 'The board in the dark theme' } };
    }
    if (body.pluginList) return { pluginList: { plugins: [] } };
    if (body.pluginGrantList) return { pluginGrantList: { grants: [] } };
    throw new Error('protocol.invalid_message');
  },
};

let host: HTMLElement;
let root: Root;
const flush = () => act(async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); });
beforeEach(async () => {
  setLanguage('en'); localStorage.clear(); themeStore.reload(); asked.length = 0;
  host = document.createElement('div'); document.body.append(host); root = createRoot(host);
  setCatalogSource(ask);
  await pluginStore.attachHub(link);
});
afterEach(async () => {
  act(() => root.unmount()); host.remove(); setCatalogSource(undefined); await pluginStore.attachHub(undefined);
  act(() => { for (const u of themeStore.state().user) themeStore.remove(u.id); themeStore.reset(); });
});

async function render(tab: GalleryTab = 'themes') { act(() => root.render(<CatalogGallery tab={tab} onTab={() => {}} onBack={() => {}} />)); await flush(); }
const ids = (attr: string) => [...host.querySelectorAll(`[${attr}]`)].map((e) => e.getAttribute(attr));
const button = (el: ParentNode, label: string) => [...el.querySelectorAll('button')].find((b) => b.textContent === label) as HTMLButtonElement;
const setValue = (el: HTMLInputElement | HTMLSelectElement, value: string) => {
  const proto = el instanceof HTMLSelectElement ? HTMLSelectElement.prototype : HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(proto, 'value')!.set!.call(el, value);
  el.dispatchEvent(new Event(el instanceof HTMLSelectElement ? 'change' : 'input', { bubbles: true }));
};

describe('the gallery', () => {
  it('opens on the most popular theme, sorts by day and name, and searches', async () => {
    await render();
    expect(ids('data-gallery-theme')).toEqual(['ember', 'harbour', 'moss']);
    const sort = host.querySelector('[data-gallery-sort]') as HTMLSelectElement;
    act(() => setValue(sort, 'new'));
    expect(ids('data-gallery-theme')).toEqual(['moss', 'harbour', 'ember']);
    act(() => setValue(sort, 'name'));
    expect(ids('data-gallery-theme')).toEqual(['ember', 'harbour', 'moss']);
    act(() => setValue(host.querySelector('input[type=search]') as HTMLInputElement, 'mira'));
    expect(ids('data-gallery-theme')).toEqual(['harbour']);
    act(() => setValue(host.querySelector('input[type=search]') as HTMLInputElement, 'nothing like it'));
    expect(host.textContent).toContain(t('catalog-gallery-no-match'));
  });

  it('draws each theme from its own file and installs one in a step; it is then offered to use', async () => {
    await render();
    expect(asked.sort()).toEqual(['ember', 'harbour', 'moss']);
    const card = host.querySelector('[data-gallery-theme="moss"]') as HTMLElement;
    expect(card.querySelector('[data-catalog-preview]')).not.toBeNull();
    expect(card.querySelector('[data-popularity="5"]')).not.toBeNull();
    expect(card.querySelector('[data-added="2026-11-20"]')).not.toBeNull();
    act(() => button(card, t('theme-catalog-install')).click());
    await flush();
    expect(themeStore.state().user.map((u) => u.catalog)).toEqual([{ id: 'moss', version: 1 }]);
    expect(button(host.querySelector('[data-gallery-theme="moss"]')!, t('catalog-gallery-use'))).toBeDefined();
    act(() => button(host.querySelector('[data-gallery-theme="moss"]')!, t('catalog-gallery-details')).click());
    expect(host.querySelector('[data-gallery-details="moss"] [data-catalog-preview]')).not.toBeNull();
  });

  it('shows a plugin\'s screenshot only as an image, what it asks to see, and the owner\'s command', async () => {
    await render('plugins');
    expect(ids('data-gallery-plugin')).toEqual(['timer', 'board']);
    const img = host.querySelector('[data-gallery-plugin="board"] img') as HTMLImageElement;
    expect(img.getAttribute('src')).toMatch(/^data:image\/png;base64,/);
    expect(img.alt).toBe('The board in the dark theme');
    expect(host.querySelector('[data-gallery-plugin="timer"] img')).toBeNull();
    expect(host.querySelector('[data-gallery-plugin="timer"]')!.textContent).toContain(t('plugin-grant-nothing'));
    act(() => button(host.querySelector('[data-gallery-plugin="board"]')!, t('catalog-gallery-details')).click());
    await flush();
    expect(host.querySelector('[data-gallery-details="board"] [data-catalog-command]')!.textContent).toBe('agora hub plugins install board');
  });

  it('says so when the window cannot ask a hub', async () => {
    setCatalogSource(undefined);
    await pluginStore.attachHub(undefined);
    await render();
    expect(host.textContent).toContain(t('catalog-gallery-unavailable'));
    await render('plugins');
    expect(host.textContent).toContain(t('catalog-gallery-unavailable'));
  });
});
