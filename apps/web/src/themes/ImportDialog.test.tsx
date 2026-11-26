// @vitest-environment jsdom
// "Import from another app…" in the theme editor: a file over the cap is refused before it is read; any other is converted and shown
// with its notes and what the theme check says, and nothing is added until Add theme; a refused one cannot be added.
import { readFileSync } from 'node:fs';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage, t } from '../i18n';
import { MAX_FILE_BYTES } from './file';
import { importErrorText } from './messages';
import { themeStore } from './store';
import { ThemeEditor } from './ThemeEditor';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); localStorage.clear(); themeStore.reload(); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); act(() => { for (const u of themeStore.state().user) themeStore.remove(u.id); themeStore.reset(); }); });

const fixture = (f: string) => readFileSync(`src/themes/import/fixtures/${f}`, 'utf8');
const dialog = () => document.querySelector<HTMLElement>('[data-theme-import]');
const buttonIn = (el: ParentNode, name: string) => [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent === name);
async function choose(file: File) {
  act(() => { root.render(<ThemeEditor />); });
  const input = host.querySelector('[data-import-other]') as HTMLInputElement;
  Object.defineProperty(input, 'files', { value: [file], configurable: true });
  await act(async () => { input.dispatchEvent(new Event('change', { bubbles: true })); });
}

describe('import from another app', () => {
  it('has the button beside Import…, and its file field takes any file (a Ghostty theme has no extension)', () => {
    act(() => { root.render(<ThemeEditor />); });
    expect(buttonIn(host, t('theme-import-other'))).toBeDefined();
    expect((host.querySelector('[data-import-other]') as HTMLInputElement).accept).toBe('');
  });

  it('shows what the file became, with its notes, and adds it only on Add theme', async () => {
    await choose(new File([fixture('harbour-color-theme.json')], 'harbour-color-theme.json'));
    const d = dialog() as HTMLElement;
    expect(d).not.toBeNull();
    expect(d.textContent).toContain(t('theme-import-from', { format: 'VS Code', name: 'Harbour' }));
    expect(d.querySelectorAll('[data-import-notes] li').length).toBeGreaterThan(3);
    expect(d.textContent).toContain(t('theme-import-note-derived'));
    expect(d.querySelectorAll('[data-import-palette] > span')).toHaveLength(16);
    expect(d.querySelector('[data-terminal-sample]')).not.toBeNull();
    expect(themeStore.state().user).toHaveLength(0);
    act(() => { (buttonIn(d, t('theme-import-add')) as HTMLButtonElement).click(); });
    expect(dialog()).toBeNull();
    expect(themeStore.state().user).toHaveLength(1);
    const added = themeStore.state().user[0];
    expect(themeStore.state().choice.dark).toBe(added?.id);
    expect(themeStore.state().choice.light).toBe('light'); // the dark scheme only
    expect(document.documentElement.style.getPropertyValue('--terminal-red')).toBe('#f2777a');
  });

  it('adds nothing on Cancel', async () => {
    await choose(new File([fixture('harbour.toml')], 'harbour.toml'));
    act(() => { (buttonIn(dialog() as HTMLElement, t('theme-import-cancel')) as HTMLButtonElement).click(); });
    expect(dialog()).toBeNull();
    expect(themeStore.state().user).toHaveLength(0);
  });

  it('shows why a theme the theme check refuses cannot be added, and offers no Add', async () => {
    await choose(new File(['background = 202020\nforeground = 303030\n'], 'Dim'));
    const d = dialog() as HTMLElement;
    expect(d.textContent).toContain(t('theme-import-refused'));
    expect((buttonIn(d, t('theme-import-add')) as HTMLButtonElement).disabled).toBe(true);
    expect(d.textContent).toContain(t('theme-import-note-derived-kept'));
  });

  it('refuses a file over the cap before reading it, and a file of no known app', async () => {
    const big = new File(['x'], 'big.json');
    Object.defineProperty(big, 'size', { value: MAX_FILE_BYTES + 1 });
    const read = vi.spyOn(big, 'text');
    await choose(big);
    expect(read).not.toHaveBeenCalled();
    expect(dialog()?.textContent).toContain(importErrorText('too-large'));
    expect(buttonIn(dialog() as HTMLElement, t('theme-import-add'))).toBeUndefined();
    act(() => { (buttonIn(dialog() as HTMLElement, t('theme-import-cancel')) as HTMLButtonElement).click(); });
    await choose(new File(['hello'], 'notes.txt'));
    expect(dialog()?.textContent).toContain(importErrorText('unknown-format'));
  });
});
