import { useSyncExternalStore } from 'react';
import { t } from '../i18n';
import { DESKTOP_NOTIFY_DEFAULTS, desktopNotifyPrefs, type DesktopNotifyPrefs } from '../net/desktopNotify';
import { SAFE } from '../plugins/safe';
import { SettingRow, Toggle } from './kit';
import { SettingsCard } from './SettingsCard';

const useAppNotifyPrefs = (): DesktopNotifyPrefs =>
  useSyncExternalStore(desktopNotifyPrefs.subscribe, () => desktopNotifyPrefs.get(), () => DESKTOP_NOTIFY_DEFAULTS);

// This device, inside the desktop app only: the app's own notification when an agent finishes or needs you (net/desktopNotify.ts). Off
// until the person turns it on here, kept in this app's page storage only. Safe mode (ADR-0114) has the switch off and disabled, and says
// why under it. The system's own permission is asked by the system itself, at the first notification (macOS asks once).
export function AppNotifySettings({ safe = SAFE }: { safe?: boolean }) {
  const p = useAppNotifyPrefs();
  const on = p.on && !safe;
  const hint = safe ? 'app-notify-safe-mode' : on ? 'app-notify-on-hint' : 'app-notify-hint';
  const row = (label: string, key: 'finished' | 'needsYou') => (
    <SettingRow label={t(label)}>
      {(labelId) => <Toggle checked={p[key]} labelledBy={labelId} onChange={(next) => desktopNotifyPrefs.set({ [key]: next })} />}
    </SettingRow>
  );
  return (
    <SettingsCard id="app-notifications" icon="bell" title={t('app-notify-title')} description={t('app-notify-desc')}>
      <SettingRow label={t('app-notify-label')} hint={t(hint)}>
        {(labelId, hintId) => <Toggle checked={on} disabled={safe} labelledBy={labelId} describedBy={hintId} onChange={(next) => desktopNotifyPrefs.set({ on: next })} />}
      </SettingRow>
      {on && <>
        {row('app-notify-finished', 'finished')}
        {row('app-notify-needs-you', 'needsYou')}
      </>}
    </SettingsCard>
  );
}
