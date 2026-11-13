import { useSyncExternalStore } from 'react';
import { t } from '../i18n';
import { getThemePreference, setTheme, subscribeTheme, type ThemePreference } from '../theme';
import { Icon, type IconName } from './Icon';

const CHOICES: { id: ThemePreference; icon: IconName; label: string }[] = [
  { id: 'system', icon: 'monitor', label: 'theme-system' },
  { id: 'dark', icon: 'moon', label: 'theme-dark' },
  { id: 'light', icon: 'sun', label: 'theme-light' },
];

export function ThemePicker() {
  const pref = useSyncExternalStore(subscribeTheme, getThemePreference, getThemePreference);
  return <div role="group" aria-label={t('theme-label')} className="flex items-center rounded-xs border border-line bg-surface-0 p-0.5">
    {CHOICES.map(c => <button key={c.id} type="button" className="ide-iconbtn size-6" aria-pressed={pref === c.id} onClick={() => setTheme(c.id)} aria-label={t(c.label)} title={t(c.label)}><Icon name={c.icon} className="size-3.5" /></button>)}
  </div>;
}
