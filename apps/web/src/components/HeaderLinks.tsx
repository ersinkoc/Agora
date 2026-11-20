import { useSyncExternalStore } from 'react';
import { t } from '../i18n';
import { isMachinesHash, isSettingsHash } from '../net/paletteExtras';
import { Icon, type IconName } from './Icon';

// The two places of the title bar's right side, as icons beside Lock: Settings (this device and the person's machines' trust) and
// Machines (what runs where). The page that is open is marked with `aria-current` and the accent colour.
const LINKS: { icon: IconName; href: string; label: string; here: (hash: string) => boolean }[] = [
  { icon: 'settings', href: '#/device', label: 'settings-link', here: isSettingsHash },
  { icon: 'stack', href: '#/machines', label: 'machines-link', here: isMachinesHash },
];

const subscribe = (cb: () => void) => { window.addEventListener('hashchange', cb); return () => window.removeEventListener('hashchange', cb); };

// `hash`: the page's own, when it knows it; else the address bar is followed (and nothing is marked while rendering on a server).
export function HeaderLinks({ hash }: { hash?: string }) {
  const own = useSyncExternalStore(subscribe, () => window.location.hash, () => '');
  const now = hash ?? own;
  return <nav aria-label={t('header-links')} className="flex flex-none items-center gap-0.5">
    {LINKS.map((l) => {
      const here = l.here(now);
      return <a key={l.href} href={l.href} className={`ide-iconbtn${here ? ' text-accent' : ''}`} aria-current={here ? 'page' : undefined} aria-label={t(l.label)} title={t(l.label)}><Icon name={l.icon} /></a>;
    })}
  </nav>;
}
