// @vitest-environment jsdom
// The theme editor with format 2 (ADR-0113): a row for every new value, grouped; a live preview of each region with a terminal sample;
// the floor as locked rows with the reason; a change applies to the page; an export is format 2.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { setLanguage, t } from '../i18n';
import { readTheme, writeTheme } from './file';
import { ANSI_NAMES, CHROME_NAMES, FLOOR_ITEMS, REGIONS, REGION_PARTS, SYNTAX_KEYS } from './look';
import { chromeLabel, floorLabel, floorReason, regionLabel, regionPartLabel, syntaxLabel, terminalLabel } from './messages';
import { themeStore } from './store';
import { ThemeEditor, draftOf, fileOf } from './ThemeEditor';
import { type Look, looksOf } from './tokens';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); localStorage.clear(); themeStore.reload(); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); act(() => { for (const u of themeStore.state().user) themeStore.remove(u.id); themeStore.reset(); }); });

const render = () => act(() => root.render(<ThemeEditor />));
const click = (el: Element) => act(() => { (el as HTMLElement).click(); });
const button = (name: string | RegExp) => [...host.querySelectorAll<HTMLElement>('button')].find((b) => {
  const n = b.getAttribute('aria-label') ?? b.textContent ?? '';
  return typeof name === 'string' ? n === name : name.test(n);
});
const labelled = (label: string) => [...host.querySelectorAll<HTMLInputElement>('input:not([type="color"])')].find((i) => i.getAttribute('aria-label') === label || host.querySelector(`label[for="${i.id}"]`)?.textContent === label);
function type(input: HTMLInputElement, value: string) {
  act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set?.call(input, value);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

describe('the theme editor (format 2)', () => {
  it('has a row for every region part, terminal colour, code colour and chrome value, and the floor as locked rows with the reason', () => {
    render();
    click(button(t('theme-customize')) as HTMLElement);
    for (const r of REGIONS) {
      expect(host.querySelector(`[role="group"][aria-label="${regionLabel(r)}"]`), r).not.toBeNull();
      for (const p of REGION_PARTS) expect(labelled(regionPartLabel(r, p)), `${r} ${p}`).toBeDefined();
    }
    for (const k of ['background', 'foreground', 'cursor', 'selection', ...ANSI_NAMES] as const) expect(labelled(terminalLabel(k)), k).toBeDefined();
    for (const k of SYNTAX_KEYS) expect(labelled(syntaxLabel(k)), k).toBeDefined();
    for (const n of CHROME_NAMES) expect(host.textContent, n).toContain(chromeLabel(n));
    const floor = [...host.querySelectorAll('[data-floor]')];
    expect(floor.map((f) => f.getAttribute('data-floor'))).toEqual([...FLOOR_ITEMS]);
    for (const f of FLOOR_ITEMS) {
      const row = host.querySelector(`[data-floor="${f}"]`) as HTMLElement;
      expect(row.textContent).toContain(floorLabel(f));
      expect(row.textContent).toContain(floorReason(f));
      expect(row.textContent).toContain(t('theme-locked'));
      expect(row.querySelector('input, button, select')).toBeNull(); // locked: nothing to change
    }
  });

  it('shows a live preview of every region, with a terminal sample in the theme\'s palette', () => {
    render();
    const preview = host.querySelector('[data-theme-preview]') as HTMLElement;
    expect(preview).not.toBeNull();
    for (const r of REGIONS.filter((x) => x !== 'tab-bar')) expect(preview.querySelector(`[data-preview="${r}"]`), r).not.toBeNull();
    expect(preview.querySelector('.ide-tab[aria-selected="true"]')).not.toBeNull(); // the tab bar
    const sample = preview.querySelector('[data-terminal-sample]') as HTMLElement;
    expect(sample.style.background).toBe('rgb(9, 9, 11)'); // Dark's background
  });

  it('applies a chrome value or a region colour to the page as it is chosen, and exports format 2', () => {
    render();
    click(button(t('theme-customize')) as HTMLElement);
    click(button(t('theme-chrome-tab-style-pill')) as HTMLElement);
    expect(document.documentElement.dataset.tabStyle).toBe('pill');
    click(button(t('theme-chrome-density-compact')) as HTMLElement);
    expect(document.documentElement.dataset.density).toBe('compact');
    type(labelled(regionPartLabel('sidebar', 'surface')) as HTMLInputElement, '#0b1016');
    expect(document.documentElement.style.getPropertyValue('--sidebar-surface')).toBe('#0b1016');
    // A colour still being typed is kept in its field; the page keeps the last good one.
    type(labelled(regionPartLabel('sidebar', 'surface')) as HTMLInputElement, '#0b10');
    expect((labelled(regionPartLabel('sidebar', 'surface')) as HTMLInputElement).value).toBe('#0b10');
    expect(host.textContent).toContain(t('theme-incomplete'));
    expect(document.documentElement.style.getPropertyValue('--sidebar-surface')).toBe('#0b1016');
    // A region colour that is hard to read is listed and not applied.
    type(labelled(regionPartLabel('sidebar', 'surface')) as HTMLInputElement, '#f4f4f5');
    expect(host.textContent).toContain(t('theme-contrast-not-applied'));
    expect(document.documentElement.style.getPropertyValue('--sidebar-surface')).toBe('#0b1016');
    const out = writeTheme(themeStore.active(), 'x');
    expect(out['agora-theme']).toBe(2);
    expect((out.chrome as Record<string, unknown>)['tab-style']).toBe('pill');
    expect(readTheme(out).ok).toBe(true);
  });

  it('builds the file from what is typed: empty colours follow the theme, and a draft of a theme is the theme', () => {
    const theme = themeStore.active();
    const look = looksOf(theme)[theme.scheme] as Look;
    const draft = draftOf(theme, look);
    const { file, complete } = fileOf(theme, theme.scheme, draft);
    expect(complete).toBe(true);
    expect(file).toEqual(writeTheme(theme, theme.name));
    const tinted = fileOf(theme, theme.scheme, { ...draft, regions: { ...draft.regions, 'cards.surface': ' #222222 ' } });
    expect((tinted.file.dark as { regions: unknown }).regions).toEqual({ cards: { surface: '#222222' } });
    expect(fileOf(theme, theme.scheme, { ...draft, terminal: { ...draft.terminal, red: 'red' } }).complete).toBe(false);
  });
});
