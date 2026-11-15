import { useSyncExternalStore } from 'react';
import { t } from '../i18n';
import { type Mode, themeStore } from '../themes/store';
import { Icon, type IconName } from './Icon';

const CHOICES: { id: Mode; icon: IconName; label: string }[] = [
  { id: 'system', icon: 'monitor', label: 'theme-system' },
  { id: 'dark', icon: 'moon', label: 'theme-dark' },
  { id: 'light', icon: 'sun', label: 'theme-light' },
];

export function ThemePicker() {
  const pref = useSyncExternalStore(themeStore.subscribe, themeStore.state, themeStore.state).mode;
  return <div role="group" aria-label={t('theme-label')} className="flex items-center rounded-xs border border-line bg-surface-0 p-0.5">
    {CHOICES.map(c => <button key={c.id} type="button" className="ide-iconbtn size-6" aria-pressed={pref === c.id} onClick={() => themeStore.setMode(c.id)} aria-label={t(c.label)} title={t(c.label)}><Icon name={c.icon} className="size-3.5" /></button>)}
  </div>;
}
