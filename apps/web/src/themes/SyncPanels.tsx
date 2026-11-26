// ADR-0113 step 4 in the window (docs/ux/themes.md "Keeping themes the same on your devices"):
// - `ThemeSyncSetting`: Settings > This device > Appearance, "Keep my themes the same on my devices", with where it stands under it;
// - `SyncedItems`: the theme editor's "Your packs and devices" section: the packs the person kept (open one in the Import pack preview,
//   or forget it), themes from other devices that do not fit here, and anything synced this window cannot read, with its problems.
// Every name from a theme or a pack is shown as text.
import { useState, useSyncExternalStore } from 'react';
import { t } from '../i18n';
import { Button, SettingRow, Toggle } from '../components/kit';
import { type KeptPack, type PackStore, MAX_KEPT_PACKS, packStore } from './kept';
import { issueText } from './messages';
import { ImportPack, packIssueText } from './PackDialogs';
import { MAX_USER_THEMES, type ThemeStore, themeStore } from './store';
import { type SyncStatus, type ThemeSync, MAX_SYNC_BYTES, themeSync } from './sync';

/** The sentence under the switch for each state. */
export const syncHint = (s: SyncStatus): string => t(`theme-sync-${s}`, { max: MAX_SYNC_BYTES / 1024 });

export function ThemeSyncSetting({ sync = themeSync }: { sync?: ThemeSync }) {
  const view = useSyncExternalStore(sync.subscribe, sync.view, sync.view);
  const s = view.status;
  // Off and disabled where it cannot sync at all: safe mode, no hub, an older hub.
  const cannot = s === 'safe' || s === 'local' || s === 'older' || s === 'paused';
  return (
    <SettingRow label={t('theme-sync-label')} hint={syncHint(s)}>
      {(labelId, hintId) => <Toggle checked={!cannot && sync.enabled()} disabled={cannot} labelledBy={labelId} describedBy={hintId} onChange={(on) => sync.setEnabled(on)} />}
    </SettingRow>
  );
}

const partsText = (k: KeptPack) => [
  k.pack.themes.length ? t('pack-sync-themes', { count: k.pack.themes.length }) : '',
  k.pack.layouts.length ? t('pack-sync-layouts', { count: k.pack.layouts.length }) : '',
  k.pack.keys ? t('pack-sync-keys', { count: k.pack.keys.file.entries.length }) : '',
].filter(Boolean).join(' · ');

export function SyncedItems({ sync = themeSync, packs = packStore, themes = themeStore }: { sync?: ThemeSync; packs?: PackStore; themes?: ThemeStore }) {
  const view = useSyncExternalStore(sync.subscribe, sync.view, sync.view);
  const kept = useSyncExternalStore(packs.subscribe, packs.list, packs.list);
  const [open, setOpen] = useState<KeptPack | undefined>();
  const waiting = view.waiting.filter((w) => w.kind === 'theme');
  const waitingPacks = view.waiting.filter((w) => w.kind === 'pack');
  const invalid = view.invalid;
  if (kept.length === 0 && view.waiting.length === 0 && invalid.length === 0) return null;
  return (
    <section className="grid gap-2" aria-labelledby="theme-synced-title" data-theme-synced>
      <h3 id="theme-synced-title" className="ide-section m-0 px-0">{t('theme-synced-title')}</h3>
      <p className="muted m-0 text-xs">{t('theme-synced-hint')}</p>
      {kept.length > 0 && (
        <ul className="m-0 grid list-none gap-2 p-0" aria-label={t('theme-synced-packs')}>
          {kept.map((k) => (
            <li key={k.uid} className="kit-card kit-card--dense flex flex-wrap items-center gap-2" data-kept-pack={k.uid}>
              <div className="grid min-w-0 flex-1">
                <span className="truncate font-semibold">{k.pack.name}</span>
                <span className="muted truncate text-xs">{partsText(k)}</span>
              </div>
              <Button size="sm" onClick={() => setOpen(k)}>{t('theme-synced-open')}</Button>
              <Button size="sm" variant="ghost" onClick={() => packs.forget(k.uid)}>{t('theme-synced-forget')}</Button>
            </li>
          ))}
        </ul>
      )}
      {waiting.length > 0 && (
        <div className="kit-card kit-card--dense grid gap-1 text-xs" data-synced-waiting>
          <span className="font-semibold">{t('theme-synced-waiting', { count: waiting.length, max: MAX_USER_THEMES })}</span>
          <span className="muted">{waiting.map((w) => w.name).join(', ')}</span>
        </div>
      )}
      {waitingPacks.length > 0 && (
        <div className="kit-card kit-card--dense grid gap-1 text-xs" data-synced-waiting-packs>
          <span className="font-semibold">{t('theme-synced-waiting-packs', { count: waitingPacks.length, max: MAX_KEPT_PACKS })}</span>
          <span className="muted">{waitingPacks.map((w) => w.name).join(', ')}</span>
        </div>
      )}
      {invalid.map((i) => (
        <div key={i.uid} className="kit-card kit-card--dense grid gap-1 text-xs" data-synced-invalid={i.uid}>
          <div className="flex flex-wrap items-center gap-2">
            <span className="min-w-0 flex-1 truncate font-semibold">
              {i.name ? t(i.kind === 'theme' ? 'theme-synced-invalid-theme' : 'theme-synced-invalid-pack', { name: i.name }) : t(i.kind === 'theme' ? 'theme-synced-invalid-theme-unnamed' : 'theme-synced-invalid-pack-unnamed')}
            </span>
            <Button size="sm" variant="ghost" onClick={() => (i.kind === 'theme' ? themes.forget(i.uid) : packs.forget(i.uid))}>{t('theme-synced-remove')}</Button>
          </div>
          <ul role="alert" className="m-0 grid list-none gap-0.5 p-0 text-danger">
            {(i.kind === 'theme' ? i.issues.map(issueText) : i.issues.map(packIssueText)).map((l, n) => <li key={n}>{l}</li>)}
          </ul>
        </div>
      ))}
      {open && <ImportPack kept={open} onClose={() => setOpen(undefined)} />}
    </section>
  );
}
