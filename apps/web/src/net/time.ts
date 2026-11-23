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

/** "4.1 s", "0.6 s", "12 s", "2 min 5 s": a short run keeps one decimal (a step is often a second or two); longer ones use `duration`. */
export function preciseDuration(ms: number): string {
  const v = Math.max(0, ms);
  if (v < 10_000) return `${(Math.round(v / 100) / 10).toFixed(1)} s`;
  return duration(v);
}

/** "12 sec", "5 min", "2 hr", "3 days": how long ago, short enough for the end of a row, in the window's language (the browser's own unit
 *  words). */
export function agoShort(ms: number, now: number): string {
  if (!ms) return t('network-never');
  const s = Math.max(0, Math.round((now - ms) / 1000));
  const [n, unit] = s < 60 ? [s, 'second'] : s < 3600 ? [Math.round(s / 60), 'minute'] : s < 86400 ? [Math.round(s / 3600), 'hour'] : [Math.round(s / 86400), 'day'];
  return new Intl.NumberFormat(getLanguage(), { style: 'unit', unit, unitDisplay: 'short' }).format(n);
}
