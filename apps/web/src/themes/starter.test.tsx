// @vitest-environment jsdom
// Starter themes (roadmap M5.2): well-known palettes shipped as theme files of format 2. Each file passes the same reader as any theme
// with no problem and no warning, the picker lists them in their own group, choosing one behaves like choosing a built-in theme, and
// their licences are noted.
import { readFileSync, readdirSync } from 'node:fs';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { setLanguage, t } from '../i18n';
import { deserialise, readTheme, writeTheme } from './file';
import { issueText, themeName } from './messages';
import { STARTER, STARTER_FILES } from './starter';
import { createThemeStore, loadState, themeStore } from './store';
import { ThemeEditor, themeGroups } from './ThemeEditor';
import { BUILT_IN, type Scheme, hasScheme, schemesOf } from './tokens';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

// Paths from apps/web, where the tests run (jsdom has no file URL for this module).
const FOLDER = 'src/themes/starter';
const read = (p: string) => readFileSync(p, 'utf8').replace(/\r\n/g, '\n');

function memory(init: Record<string, string> = {}) {
  const m = new Map(Object.entries(init));
  return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => { m.set(k, v); }, removeItem: (k: string) => { m.delete(k); }, m };
}

describe('the starter theme files', () => {
  it('are every file in starter/, each read with no problem and no warning', () => {
    const files = readdirSync(FOLDER).filter((f) => f.endsWith('.json')).sort();
    expect(STARTER_FILES.map((f) => `${f.id}.json`).sort()).toEqual(files);
    for (const f of STARTER_FILES) {
      expect(f.text.replace(/\r\n/g, '\n'), f.id).toBe(read(`${FOLDER}/${f.id}.json`));
      const r = deserialise(f.text);
      expect(r.ok ? (r.warnings ?? []).map(issueText) : r.issues.map(issueText), f.id).toEqual([]);
      expect(r.ok && r.warnings, f.id).toBeFalsy();
    }
    expect(STARTER).toHaveLength(STARTER_FILES.length);
  });

  it('are format 2, name their palette, set only colours, and read back the same when exported', () => {
    for (const f of STARTER_FILES) {
      const value = JSON.parse(f.text) as Record<string, unknown>;
      expect(value['agora-theme'], f.id).toBe(2);
      expect(Object.keys(value).filter((k) => !['agora-theme', 'name', 'dark', 'light'].includes(k)), f.id).toEqual([]);
      expect(String(value.name).startsWith(f.source.project), f.id).toBe(true);
      const theme = STARTER.find((s) => s.id === `starter-${f.id}`)!;
      const again = readTheme(writeTheme(theme, theme.name));
      expect(again.ok && again.theme.looks, f.id).toEqual(theme.looks);
    }
  });

  it('are read-only, never a built-in theme a file may inherit, and their ids do not meet the built-in ones', () => {
    for (const s of STARTER) {
      expect(s.builtIn && s.starter, s.id).toBeTruthy();
      expect(s.nameKey, s.id).toBeUndefined();
      expect(BUILT_IN.some((b) => b.id === s.id), s.id).toBe(false);
      const r = readTheme({ 'agora-theme': 2, name: 'x', inherits: s.id });
      expect(r.ok, s.id).toBe(false);
    }
  });

  it('carry their licence notice in THIRD_PARTY.md', () => {
    const notice = read('../../THIRD_PARTY.md');
    for (const f of STARTER_FILES) {
      expect(notice, f.id).toContain(f.source.project);
      expect(notice, f.id).toContain(f.source.licence);
    }
    expect(notice).toContain('apps/web/src/themes/starter/');
  });
});

describe('choosing a starter theme', () => {
  it('is kept like a built-in choice, fills each scheme it has, and survives a reload', () => {
    const kv = memory();
    const s = createThemeStore({ kv, prefersLight: () => false });
    const gruvbox = STARTER.find((x) => x.id === 'starter-gruvbox')!;
    expect(schemesOf(gruvbox)).toEqual(['dark', 'light']);
    s.choose(gruvbox.id);
    expect(s.state().choice).toEqual({ dark: gruvbox.id, light: gruvbox.id });
    expect(s.active().id).toBe(gruvbox.id);
    expect(loadState(kv).choice).toEqual({ dark: gruvbox.id, light: gruvbox.id });
    // A dark-only one fills the dark slot only.
    const dracula = STARTER.find((x) => x.id === 'starter-dracula')!;
    s.choose(dracula.id);
    expect(s.state().choice).toEqual({ dark: dracula.id, light: gruvbox.id });
    // Customize: a copy of the person's own, both schemes kept.
    const copy = s.copy(gruvbox, 'Gruvbox (mine)');
    expect(copy.ok && schemesOf(copy.theme)).toEqual(['dark', 'light']);
    expect(copy.ok && copy.theme.starter).toBeUndefined();
  });
});

describe('the picker', () => {
  let host: HTMLElement;
  let view: Root;
  beforeEach(() => { setLanguage('en'); localStorage.clear(); themeStore.reload(); host = document.createElement('div'); document.body.append(host); view = createRoot(host); });
  afterEach(() => { act(() => view.unmount()); host.remove(); act(() => { for (const u of themeStore.state().user) themeStore.remove(u.id); themeStore.reset(); }); });

  const listed = (s: Scheme) => {
    const select = host.querySelectorAll('select')[s === 'light' ? 0 : 1] as HTMLSelectElement;
    return [...select.querySelectorAll('optgroup')].map((g) => `${g.label}: ${[...g.querySelectorAll('option')].map((o) => o.textContent).join(', ')}`);
  };

  it('lists the starter themes in their own group under each scheme they have', () => {
    act(() => view.render(<ThemeEditor />));
    expect({ light: listed('light'), dark: listed('dark') }).toMatchInlineSnapshot(`
      {
        "dark": [
          "Built-in: Dark, High contrast, Large text, Slate",
          "Starter themes: Catppuccin Mocha, Dracula, Gruvbox, Rosé Pine, Tokyo Night",
        ],
        "light": [
          "Built-in: Light, Sand",
          "Starter themes: Gruvbox",
        ],
      }
    `);
    expect(themeGroups(themeStore.themes().filter((x) => hasScheme(x, 'dark'))).map((g) => g.key)).toEqual(['theme-group-built-in', 'theme-group-starter']);
  });

  it('shows a chosen starter theme read-only with where its colours come from, and Customize', () => {
    act(() => themeStore.choose('starter-catppuccin-mocha'));
    act(() => view.render(<ThemeEditor />));
    expect(host.textContent).toContain(t('theme-starter'));
    expect(host.textContent).toContain(t('theme-starter-hint', { project: 'Catppuccin', licence: 'MIT' }));
    expect(host.textContent).not.toContain(t('theme-delete'));
    const customize = [...host.querySelectorAll('button')].find((b) => b.textContent === t('theme-customize')) as HTMLButtonElement;
    act(() => customize.click());
    expect(themeName(themeStore.active())).toBe(t('theme-copy-name', { name: 'Catppuccin Mocha' }));
    expect(listed('dark').at(-1)).toBe(`${t('theme-group-yours')}: ${t('theme-copy-name', { name: 'Catppuccin Mocha' })}`);
  });
});
