import { useState, useSyncExternalStore } from 'react';
import { t } from '../i18n';
import { ATTENTION_MODES, attentionPrefs, type AttentionMode, type NotifyMode } from '../net/attention';

// This device: how the window gets the person's attention when an agent is blocked. Kept in this browser only. Notification permission is
// asked for only here, on the click of "Turn on notifications", never when the window loads.
export function AttentionSettings() {
  const mode = useSyncExternalStore(attentionPrefs.subscribe, () => attentionPrefs.mode(), () => 'modal' as AttentionMode);
  const notify = useSyncExternalStore(attentionPrefs.subscribe, () => attentionPrefs.notify(), () => 'off' as NotifyMode);
  const [perm, setPerm] = useState<NotificationPermission | 'unsupported'>(() => (typeof Notification === 'undefined' ? 'unsupported' : Notification.permission));
  const on = notify !== 'off' && perm === 'granted';
  const enable = () => {
    try {
      void Notification.requestPermission().then((p) => { setPerm(p); attentionPrefs.setNotify(p === 'granted' ? 'generic' : 'off'); }).catch(() => setPerm('denied'));
    } catch { setPerm('unsupported'); }
  };
  return (
    <section className="stack" aria-labelledby="attention-settings">
      <h2 id="attention-settings" className="m-0">{t('attention-settings-title')}</h2>
      <p className="muted m-0">{t('attention-settings-intro')}</p>
      <fieldset className="stack m-0 border-0 p-0">
        {ATTENTION_MODES.map((m) => (
          <label key={m} className="flex items-center gap-2">
            <input type="radio" name="attention-mode" checked={mode === m} onChange={() => attentionPrefs.setMode(m)} />
            <span>{t(`attention-mode-${m}`)}</span>
          </label>
        ))}
      </fieldset>
      <p className="muted m-0">{t('attention-mode-hint')}</p>
      <h3 className="m-0 text-sm">{t('attention-notify-title')}</h3>
      {perm === 'unsupported' && <p className="muted m-0">{t('attention-notify-unsupported')}</p>}
      {perm === 'denied' && <p className="muted m-0">{t('attention-notify-blocked')}</p>}
      {perm !== 'unsupported' && perm !== 'denied' && !on && <p className="m-0"><button type="button" className="button" onClick={enable}>{t('attention-notify-enable')}</button></p>}
      {on && <>
        <p className="muted m-0">{t('attention-notify-on')}</p>
        <label className="flex items-center gap-2">
          <input type="checkbox" checked={notify === 'named'} onChange={(e) => attentionPrefs.setNotify(e.target.checked ? 'named' : 'generic')} />
          <span>{t('attention-notify-names')}</span>
        </label>
        <p className="muted m-0">{t('attention-notify-names-warn')}</p>
        <p className="m-0"><button type="button" className="button" onClick={() => attentionPrefs.setNotify('off')}>{t('attention-notify-off')}</button></p>
      </>}
    </section>
  );
}
