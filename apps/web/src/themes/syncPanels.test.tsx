// @vitest-environment jsdom
// ADR-0113 step 4 in the window: the switch in Settings > This device says where syncing stands and is off where it cannot sync; the theme
// editor lists kept packs (opened in the Import pack preview, nothing applied) and shows anything synced it cannot read, with its problems.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { setLanguage, t } from '../i18n';
import { CommandError } from '../net/live';
import { writeTheme } from './file';
import { createPackStore } from './kept';
import { writePack } from './pack';
import { createThemeStore } from './store';
import { type SyncAsk, MAX_SYNC_BYTES, createThemeSync } from './sync';
import { SyncedItems, ThemeSyncSetting } from './SyncPanels';
import { BUILT_IN } from './tokens';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const sand = BUILT_IN.find((b) => b.id === 'sand')!;
let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); localStorage.clear(); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); });

function parts(safe = false) {
  const themes = createThemeStore({});
  const packs = createPackStore();
  const sync = createThemeSync({ themes, packs, safe, setTimer: () => 0, clearTimer: () => {} });
  return { themes, packs, sync };
}
const toggle = () => host.querySelector<HTMLButtonElement>('[role="switch"]')!;
const hint = () => host.querySelector('.kit-setting__hint')?.textContent;

describe('Keep my themes the same on my devices', () => {
  it('is on with a hub that keeps them, and turning it off keeps them in this browser', async () => {
    const { sync } = parts();
    const ask: SyncAsk = async (body) => ({ userSettings: { key: 'themes', value: new Uint8Array(), version: 'userSettingsSet' in body ? 1 : 0 } });
    act(() => root.render(<ThemeSyncSetting sync={sync} />));
    expect(host.textContent).toContain(t('theme-sync-label'));
    expect(hint()).toBe(t('theme-sync-local'));
    expect(toggle().disabled).toBe(true);
    await act(async () => { sync.attach(ask); await sync.settled(); await sync.settled(); });
    expect(toggle().getAttribute('aria-checked')).toBe('true');
    expect(hint()).toBe(t('theme-sync-synced'));
    act(() => toggle().click());
    expect(toggle().getAttribute('aria-checked')).toBe('false');
    expect(hint()).toBe(t('theme-sync-off'));
  });

  it('is off and disabled with an older hub, and in safe mode, saying why', async () => {
    const older = parts();
    act(() => root.render(<ThemeSyncSetting sync={older.sync} />));
    await act(async () => { older.sync.attach(async () => { throw new CommandError('auth.forbidden', { action: '' }); }); await older.sync.settled(); });
    expect(toggle().disabled).toBe(true);
    expect(toggle().getAttribute('aria-checked')).toBe('false');
    expect(hint()).toBe(t('theme-sync-older'));
    const safe = parts(true);
    act(() => root.render(<ThemeSyncSetting sync={safe.sync} />));
    expect(toggle().disabled).toBe(true);
    expect(hint()).toBe(t('theme-sync-safe'));
    expect(t('theme-sync-too-large', { max: MAX_SYNC_BYTES / 1024 })).toContain('256');
  });
});

describe('Your packs and devices', () => {
  it('lists kept packs, opens one at its preview with nothing ticked, and forgets one', () => {
    const { themes, packs, sync } = parts();
    act(() => root.render(<SyncedItems sync={sync} packs={packs} themes={themes} />));
    expect(host.querySelector('[data-theme-synced]')).toBeNull(); // nothing to show
    act(() => { packs.keep(writePack({ name: 'Harbour set', themes: [{ theme: sand, name: 'Harbour' }], layouts: [] })); });
    expect(host.querySelector('[data-kept-pack]')?.textContent).toContain('Harbour set');
    expect(host.textContent).toContain('1 theme');
    const open = [...host.querySelectorAll('button')].find((b) => b.textContent === t('theme-synced-open'))!;
    act(() => open.click());
    const dialog = document.querySelector('[data-pack-dialog]')!;
    expect(dialog.getAttribute('data-pack-dialog')).toBe('preview');
    expect([...dialog.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')].every((b) => !b.checked)).toBe(true);
    expect(themes.state().user).toEqual([]);
    act(() => [...dialog.querySelectorAll('button')].find((b) => b.textContent === t('pack-back'))!.click());
    act(() => [...document.querySelectorAll('button')].find((b) => b.textContent === t('layout-cancel'))!.click());
    act(() => [...host.querySelectorAll('button')].find((b) => b.textContent === t('theme-synced-forget'))!.click());
    expect(packs.list()).toEqual([]);
  });

  it('shows a synced theme it cannot read with its problems, never applies it, and removes it from the person\'s devices', async () => {
    const { themes, packs, sync } = parts();
    let value = JSON.stringify({ 'agora-theme-sync': 1, themes: [{ uid: 'a'.repeat(24), edited: 1, file: { ...writeTheme(sand, 'Broken'), 'agora-theme': 9 } }] });
    let version = 1;
    const ask: SyncAsk = async (body) => {
      if ('userSettingsSet' in body) { value = new TextDecoder().decode(body.userSettingsSet.value); version += 1; }
      return { userSettings: { key: 'themes', value: new TextEncoder().encode(value), version } };
    };
    act(() => root.render(<SyncedItems sync={sync} packs={packs} themes={themes} />));
    await act(async () => { sync.attach(ask); await sync.settled(); await sync.settled(); });
    const card = host.querySelector('[data-synced-invalid]')!;
    expect(card.textContent).toContain('"Broken"');
    expect(card.querySelector('[role="alert"]')?.textContent?.length).toBeGreaterThan(0);
    expect(themes.state().user).toEqual([]);
    act(() => [...card.querySelectorAll('button')].find((b) => b.textContent === t('theme-synced-remove'))!.click());
    expect(themes.syncSnapshot().removed.map((r) => r.uid)).toEqual(['a'.repeat(24)]);
  });
});
