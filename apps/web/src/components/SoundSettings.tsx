import { useSyncExternalStore } from 'react';
import { t } from '../i18n';
import { SOUND_DEFAULTS, SOUND_VOLUMES, soundPrefs, type SoundKind, type SoundPrefs } from '../net/sounds';
import { SAFE } from '../plugins/safe';
import { canPlaySounds, playSound, unlockSounds } from './chime';
import { Icon } from './Icon';
import { Button, SegmentedControl, SettingRow, Toggle } from './kit';
import { SettingsCard } from './SettingsCard';

/** This device's sound choices, live. */
export const useSoundPrefs = (): SoundPrefs => useSyncExternalStore(soundPrefs.subscribe, () => soundPrefs.get(), () => SOUND_DEFAULTS);

// This device: a sound when an agent finishes or needs you. Off until the person turns it on here (a window never surprises anyone), kept
// in this browser only. Each sound can be tried. A browser that cannot make sound, and safe mode (ADR-0114), have the switch off and
// disabled, and say why under it.
export function SoundSettings() {
  const p = useSoundPrefs();
  const can = canPlaySounds();
  const on = p.on && can;
  const hint = SAFE ? 'sounds-safe-mode' : !can ? 'sounds-unsupported' : on ? 'sounds-on-hint' : 'sounds-hint';
  const tryIt = (kind: SoundKind) => { unlockSounds(); playSound(kind, p.volume); };
  const row = (kind: SoundKind, key: 'finished' | 'needsYou') => (
    <SettingRow label={t(`sounds-${kind}`)} hint={t(`sounds-${kind}-hint`)}>
      {(labelId, hintId) => <>
        <Button variant="ghost" size="sm" aria-label={t('sounds-try-named', { name: t(`sounds-${kind}`) })} onClick={() => tryIt(kind)}><Icon name="play" className="size-3.5" />{t('sounds-try')}</Button>
        <Toggle checked={p[key]} labelledBy={labelId} describedBy={hintId} onChange={(next) => soundPrefs.set({ [key]: next })} />
      </>}
    </SettingRow>
  );
  return (
    <SettingsCard id="sounds" icon="volume" title={t('sounds-title')} description={t('sounds-desc')}>
      <SettingRow label={t('sounds-label')} hint={t(hint)}>
        {(labelId, hintId) => <Toggle checked={on} disabled={!can} labelledBy={labelId} describedBy={hintId}
          onChange={(next) => { if (next) unlockSounds(); soundPrefs.set({ on: next }); }} />}
      </SettingRow>
      {on && <>
        <SettingRow label={t('sounds-volume')}>
          {(labelId) => <SegmentedControl labelledBy={labelId} value={p.volume} onChange={(v) => soundPrefs.set({ volume: v })}
            items={SOUND_VOLUMES.map((v) => ({ value: v, label: t(`sounds-volume-${v}`) }))} />}
        </SettingRow>
        {row('finished', 'finished')}
        {row('needs-you', 'needsYou')}
        <SettingRow label={t('sounds-in-front')} hint={t('sounds-in-front-hint')}>
          {(labelId, hintId) => <Toggle checked={p.inFront} labelledBy={labelId} describedBy={hintId} onChange={(next) => soundPrefs.set({ inFront: next })} />}
        </SettingRow>
      </>}
    </SettingsCard>
  );
}
