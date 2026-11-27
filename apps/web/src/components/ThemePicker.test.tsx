// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { setLanguage, t } from '../i18n';
import { themeStore } from '../themes/store';
import { ThemePicker } from './ThemePicker';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

describe('the visual theme chooser', () => {
  let host: HTMLElement;
  let root: Root;
  beforeEach(() => {
    setLanguage('en');
    localStorage.clear();
    themeStore.reload();
    host = document.createElement('div');
    document.body.append(host);
    root = createRoot(host);
    act(() => root.render(<ThemePicker />));
  });
  afterEach(() => {
    act(() => root.unmount());
    document.querySelector('[data-theme-chooser]')?.remove();
    host.remove();
    act(() => themeStore.reset());
  });

  const click = (el: Element | null) => act(() => (el as HTMLElement).click());
  const chooser = () => document.querySelector('[data-theme-chooser]') as HTMLElement;

  it('opens from one labelled icon and previews every theme in its scheme', () => {
    click(host.querySelector(`button[aria-label="${t('theme-chooser-open')}"]`));
    expect(chooser().textContent).toContain(t('theme-chooser-title'));
    expect(chooser().querySelectorAll('[data-theme-choice]').length).toBe(13);
    expect(chooser().querySelector('[data-theme-choice="starter-aurora"] [aria-hidden]')).not.toBeNull();
    click([...chooser().querySelectorAll('[role="radio"]')].find((x) => x.textContent === t('theme-light')) ?? null);
    expect(chooser().querySelectorAll('[data-theme-choice]').length).toBe(8);
    expect(chooser().querySelector('[data-theme-choice="starter-arctic"]')).not.toBeNull();
  });

  it('chooses a preview card and keeps the mode controls in the same dialog', () => {
    click(host.querySelector('button'));
    click(chooser().querySelector('[data-theme-choice="starter-aurora"]'));
    expect(themeStore.state().choice.dark).toBe('starter-aurora');
    expect(chooser().querySelector('[data-theme-choice="starter-aurora"]')?.getAttribute('aria-pressed')).toBe('true');
    click([...chooser().querySelectorAll('[role="radio"]')].find((x) => x.textContent === t('theme-system-short')) ?? null);
    expect(themeStore.state().mode).toBe('system');
  });
});
