import { getLanguage, t } from '../i18n';

/** "12 seconds ago", in the window's language. */
export function ago(ms: number, now: number): string {
  if (!ms) return t('network-never');
  const s = Math.max(0, Math.round((now - ms) / 1000));
  const rtf = new Intl.RelativeTimeFormat(getLanguage(), { numeric: 'auto' });
  if (s < 60) return rtf.format(-s, 'second');
  if (s < 3600) return rtf.format(-Math.round(s / 60), 'minute');
  if (s < 86400) return rtf.format(-Math.round(s / 3600), 'hour');
  return rtf.format(-Math.round(s / 86400), 'day');
}

/** "1 h 5 min", "42 s": how long something ran. The unit marks are the same in every language, like KB. */
export function duration(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s} s`;
  if (s < 3600) return `${Math.floor(s / 60)} min ${s % 60} s`;
  const h = Math.floor(s / 3600);
  return `${h} h ${Math.floor((s % 3600) / 60)} min`;
}
