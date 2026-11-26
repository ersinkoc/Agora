// @vitest-environment jsdom
// The community theme catalog in the theme editor (ADR-0113 decision 6): browse what the hub keeps, preview, install a copy, and see
// "newer version" on an installed theme without it ever changing by itself. The window asks only the hub.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { setLanguage, t } from '../i18n';
import { type CatalogAsk, catalogAskFor, installedOf, listOf, setCatalogSource } from './catalog';
import { demoCatalog } from './catalogDemo';
import { writeTheme } from './file';
import { USER_KEY, loadState, themeStore } from './store';
import { ThemeEditor } from './ThemeEditor';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const NIGHT_V1 = JSON.stringify({ 'agora-theme': 1, name: 'Night', scheme: 'dark', tokens: { accent: '#ff9f43', 'on-accent': '#1a1000' } });
const NIGHT_V2 = JSON.stringify({ 'agora-theme': 1, name: 'Night', scheme: 'dark', tokens: { accent: '#5cc8c2', 'on-accent': '#000000' } });
const BAD = JSON.stringify({ 'agora-theme': 1, name: 'Bad', scheme: 'dark', tokens: {}, script: 'alert(1)' });
const entry = (id: string, name: string, version: number, data: string) => ({ id, name, version, author: 'Ada', licence: 'MIT', sha256: 'a'.repeat(64), size: data.length });

// A hub that keeps a catalog: `files` by id, `on` as its owner set it. Every question is recorded.
function hub(files: Record<string, { name: string; version: number; data: string }>, on = true) {
  const asked: string[] = [];
  const ask: CatalogAsk = async (body) => {
    if ('themeCatalogList' in body) {
      asked.push('list');
      return { themeCatalogList: on ? { on: true, updatedAtMs: 1, entries: Object.entries(files).map(([id, f]) => entry(id, f.name, f.version, f.data)) } : {} };
    }
    asked.push(`get ${body.themeCatalogGet.id}`);
    const f = files[body.themeCatalogGet.id];
    if (!on) throw Object.assign(new Error('off'), { code: 'theme.catalog_off' });
    if (!f) throw Object.assign(new Error('nf'), { code: 'theme.not_found' });
    return { themeCatalogTheme: { entry: entry(body.themeCatalogGet.id, f.name, f.version, f.data), data: f.data } };
  };
  return { ask, asked };
}

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); localStorage.clear(); themeStore.reload(); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  setCatalogSource(undefined);
  act(() => { for (const u of themeStore.state().user) themeStore.remove(u.id); themeStore.reset(); });
});

const flush = () => act(async () => { for (let i = 0; i < 5; i++) await Promise.resolve(); });
async function render() { act(() => root.render(<ThemeEditor />)); await flush(); }
const catalog = () => host.querySelector('[data-theme-catalog]');
const card = (id: string) => host.querySelector(`[data-catalog-entry="${id}"]`) as HTMLElement;
const buttonIn = (el: Element, label: string) => [...el.querySelectorAll('button')].find((b) => b.textContent === label) as HTMLButtonElement;
async function click(el: HTMLElement) { act(() => el.click()); await flush(); }

describe('the theme catalog', () => {
  it('is not drawn without a connection, while the owner keeps it off, or when the hub cannot say', async () => {
    await render();
    expect(catalog()).toBeNull();
    const off = hub({ night: { name: 'Night', version: 1, data: NIGHT_V1 } }, false);
    act(() => setCatalogSource(off.ask));
    await flush();
    expect(catalog()).toBeNull();
    expect(off.asked).toEqual(['list']);
    act(() => setCatalogSource(async () => { throw Object.assign(new Error('x'), { code: 'auth.forbidden' }); }));
    await flush();
    expect(catalog()).toBeNull();
  });

  it('lists the catalog\'s themes, previews one in its own colours and installs a copy that remembers where it came from', async () => {
    const h = hub({ night: { name: 'Night', version: 1, data: NIGHT_V1 }, bad: { name: 'Bad', version: 1, data: BAD } });
    setCatalogSource(h.ask);
    await render();
    expect(catalog()?.textContent).toContain(t('theme-catalog'));
    expect(card('night').textContent).toContain('Night');
    expect(card('night').textContent).toContain(t('theme-catalog-by', { author: 'Ada', licence: 'MIT' }));
    expect(card('night').textContent).toContain(t('theme-catalog-version', { version: 1 }));

    await click(buttonIn(card('night'), t('theme-catalog-preview')));
    const preview = card('night').querySelector('[data-catalog-preview]') as HTMLElement;
    expect(preview).not.toBeNull();
    expect(preview.querySelector('[data-terminal-sample]')).not.toBeNull();
    // Previewing changes nothing: no theme was added and the window keeps its theme.
    expect(themeStore.state().user).toHaveLength(0);

    // A theme the window's own rules refuse is shown with its reasons and cannot be installed.
    await click(buttonIn(card('bad'), t('theme-catalog-install')));
    expect(card('bad').textContent).toContain(t('theme-catalog-refused'));
    expect(card('bad').textContent).toContain('script');
    expect(themeStore.state().user).toHaveLength(0);

    await click(buttonIn(card('night'), t('theme-catalog-install')));
    const [mine] = themeStore.state().user;
    expect(mine?.name).toBe('Night');
    expect(mine?.catalog).toEqual({ id: 'night', version: 1 });
    expect(themeStore.active().id).toBe(mine?.id); // installed and shown
    expect(card('night').textContent).toContain(t('theme-catalog-installed'));
    expect(buttonIn(card('night'), t('theme-catalog-install'))).toBeUndefined();
    // Kept with its origin beside the file, so it survives a reload; an export is the theme alone.
    const stored = JSON.parse(localStorage.getItem(USER_KEY) ?? '[]');
    expect(stored[0].catalog).toEqual({ id: 'night', version: 1 });
    expect(loadState(localStorage).user[0]?.catalog).toEqual({ id: 'night', version: 1 });
    expect(Object.keys(writeTheme(mine as never, 'Night'))).not.toContain('catalog');
    // Only the hub was asked, and only for what the person did.
    expect(h.asked).toEqual(['list', 'get night', 'get bad', 'get night']);
  });

  it('marks a newer version of an installed theme and changes it only when the person picks it', async () => {
    const r = themeStore.install(NIGHT_V1, { id: 'night', version: 1 });
    expect(r.ok).toBe(true);
    const id = themeStore.state().user[0]?.id as string;
    const h = hub({ night: { name: 'Night', version: 2, data: NIGHT_V2 } });
    setCatalogSource(h.ask);
    await render();
    expect(card('night').querySelector('[data-newer]')?.textContent).toBe(t('theme-catalog-newer', { version: 2 }));
    // Nothing changed by itself: the installed theme is still version 1 with its own accent.
    expect(themeStore.state().user[0]?.catalog?.version).toBe(1);
    expect(themeStore.state().user[0]?.tokens.accent).toBe('#ff9f43');
    expect(h.asked).toEqual(['list']);

    await click(buttonIn(card('night'), t('theme-catalog-update', { version: 2 })));
    const after = themeStore.state().user;
    expect(after).toHaveLength(1);
    expect(after[0]?.id).toBe(id); // the same theme of the person, in its place
    expect(after[0]?.catalog).toEqual({ id: 'night', version: 2 });
    expect(after[0]?.tokens.accent).toBe('#5cc8c2');
    expect(themeStore.state().choice.dark).toBe(id);
    expect(card('night').querySelector('[data-newer]')).toBeNull();
    expect(card('night').textContent).toContain(t('theme-catalog-installed'));
  });

  it('asks the hub only once the window has its first view, and the demo window its sample catalog', async () => {
    const sent: [string, unknown][] = [];
    const api = { command: async (node: string, body: unknown) => { sent.push([node, body]); return {}; } };
    expect(catalogAskFor(api, false, true)).toBeUndefined();
    expect(catalogAskFor(undefined, false, true)).toBeUndefined();
    expect(catalogAskFor(undefined, false, false)).toBe(demoCatalog);
    await catalogAskFor(api, true, true)?.({ themeCatalogList: {} });
    expect(sent).toEqual([['', { themeCatalogList: {} }]]); // no machine named: the hub answers it
    const l = listOf((await demoCatalog({ themeCatalogList: {} })).themeCatalogList);
    expect(l.on && l.entries.map((e) => e.id)).toEqual(['harbour', 'meadow']);
  });

  it('reads the hub\'s answers strictly', () => {
    expect(listOf({})).toEqual({ on: false });
    const l = listOf({ on: true, updatedAtMs: 5, entries: [entry('ok', 'Ok', 2, '{}'), { id: 'Bad Id', version: 1 }, { id: 'zero', version: 0 }] });
    expect(l.on && l.entries.map((e) => e.id)).toEqual(['ok']);
    const e = { id: 'night', name: 'Night', version: 3, author: '', licence: '', size: 1, added: '', popularity: 0 };
    expect(installedOf(e, [])).toEqual({ kind: 'no' });
    const v2 = { ...themeStore.active(), id: 'user-1', catalog: { id: 'night', version: 2 } };
    expect(installedOf(e, [v2])).toMatchObject({ kind: 'older', from: 2 });
    expect(installedOf({ ...e, version: 2 }, [v2])).toMatchObject({ kind: 'current' });
  });
});
