// The demo window's catalog (no hub): two sample themes answered the way a hub answers, so the catalog can be seen, tried and
// captured for the docs without a hub. Never used when the window is signed in to a hub.
import type { CatalogAsk } from './catalog';

const HARBOUR = JSON.stringify({
  'agora-theme': 2, name: 'Harbour', inherits: 'slate',
  dark: {
    tokens: { accent: '#5cc8c2' },
    regions: { sidebar: { surface: '#0b1016' }, 'person-entries': { surface: '#14302f', line: '#2b6b68' } },
    terminal: { background: '#0b1016', foreground: '#d8e1ea', red: '#ff7b72' },
    syntax: { keyword: '#ff9e8f' },
  },
  light: { tokens: { accent: '#0b6e69', 'on-accent': '#ffffff' } },
  shape: { 'radius-m': '10px' },
  chrome: { 'pane-borders': 'always', 'pane-gap': 8, 'tab-style': 'pill', density: 'compact' },
});
const MEADOW = JSON.stringify({ 'agora-theme': 1, name: 'Meadow', scheme: 'light', tokens: { accent: '#2f6f3e', 'on-accent': '#ffffff' } });

const THEMES: Record<string, { name: string; version: number; author: string; licence: string; data: string }> = {
  harbour: { name: 'Harbour', version: 2, author: 'Agora contributors', licence: 'MIT', data: HARBOUR },
  meadow: { name: 'Meadow', version: 1, author: 'Agora contributors', licence: 'CC0-1.0', data: MEADOW },
};
const entry = (id: string) => {
  const th = THEMES[id] as (typeof THEMES)[string];
  return { id, name: th.name, version: th.version, author: th.author, licence: th.licence, sha256: '', size: th.data.length };
};

export const demoCatalog: CatalogAsk = async (body) => {
  if ('themeCatalogList' in body) return { themeCatalogList: { on: true, updatedAtMs: 0, entries: Object.keys(THEMES).map(entry) } };
  const th = THEMES[body.themeCatalogGet.id];
  if (!th) throw Object.assign(new Error('theme.not_found'), { code: 'theme.not_found' });
  return { themeCatalogTheme: { entry: entry(body.themeCatalogGet.id), data: th.data } };
};
