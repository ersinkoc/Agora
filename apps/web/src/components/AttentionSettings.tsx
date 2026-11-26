import { useState, useSyncExternalStore } from 'react';
import { t } from '../i18n';
import { ATTENTION_MODES, attentionPrefs, type AttentionMode, type NotifyMode } from '../net/attention';
import { inDesktopApp } from '../net/desktopNotify';
import { SegmentedControl, SettingRow, Toggle } from './kit';
import { SettingsCard } from './SettingsCard';

// This device: how the window gets the person's attention when an agent is blocked. Kept in this browser only. Notification permission is
// asked for only here, when the person turns the switch on, never when the window loads. A browser that blocks or cannot show
// notifications gets the switch off and disabled, and says why under it. Inside the desktop app the browser's notifications are not the
// way (the app gives the page no such permission): the switch is off and points to the app's own (App notifications, net/desktopNotify.ts).
export function AttentionSettings({ inApp = inDesktopApp() }: { inApp?: boolean }) {
  const mode = useSyncExternalStore(attentionPrefs.subscribe, () => attentionPrefs.mode(), () => 'modal' as AttentionMode);
  const notify = useSyncExternalStore(attentionPrefs.subscribe, () => attentionPrefs.notify(), () => 'off' as NotifyMode);
  const [perm, setPerm] = useState<NotificationPermission | 'unsupported'>(() => (inApp || typeof Notification === 'undefined' ? 'unsupported' : Notification.permission));
  const on = notify !== 'off' && perm === 'granted';
  const enable = () => {
    try {
      void Notification.requestPermission().then((p) => { setPerm(p); attentionPrefs.setNotify(p === 'granted' ? 'generic' : 'off'); }).catch(() => setPerm('denied'));
    } catch { setPerm('unsupported'); }
  };
  const cannot = perm === 'unsupported' || perm === 'denied';
  const notifyHint = inApp ? 'attention-notify-app' : perm === 'unsupported' ? 'attention-notify-unsupported' : perm === 'denied' ? 'attention-notify-blocked' : on ? 'attention-notify-on' : 'attention-notify-hint';
  return (
    <SettingsCard icon="bell" title={t('attention-settings-title')} description={t('attention-settings-desc')}>
      <SettingRow label={t('attention-mode-label')} hint={t('attention-mode-hint')}>
        {(labelId, hintId) => (
          <SegmentedControl labelledBy={labelId} describedBy={hintId} value={mode} onChange={(m) => attentionPrefs.setMode(m)}
            items={ATTENTION_MODES.map((m) => ({ value: m, label: t(`attention-mode-short-${m}`), title: t(`attention-mode-${m}`) }))} />
        )}
      </SettingRow>
      <SettingRow label={t('attention-notify-title')} hint={t(notifyHint)}>
        {(labelId, hintId) => <Toggle checked={on} disabled={cannot} labelledBy={labelId} describedBy={hintId} onChange={(next) => { if (next) enable(); else attentionPrefs.setNotify('off'); }} />}
      </SettingRow>
      {on && (
        <SettingRow label={t('attention-notify-names')} hint={t('attention-notify-names-warn')}>
          {(labelId, hintId) => <Toggle checked={notify === 'named'} labelledBy={labelId} describedBy={hintId} onChange={(next) => attentionPrefs.setNotify(next ? 'named' : 'generic')} />}
        </SettingRow>
      )}
    </SettingsCard>
  );
}
