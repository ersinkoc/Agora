import { getLanguage, t } from '../i18n';

/** "12 seconds ago", in the window's language; `short` ("12 sec. ago") for a dense row. */
export function ago(ms: number, now: number, style: 'long' | 'short' = 'long'): string {
  if (!ms) return t('network-never');
  const s = Math.max(0, Math.round((now - ms) / 1000));
  const rtf = new Intl.RelativeTimeFormat(getLanguage(), { numeric: 'auto', style });
  if (s < 60) return rtf.format(-s, 'second');
  if (s < 3600) return rtf.format(-Math.round(s / 60), 'minute');
  if (s < 86400) return rtf.format(-Math.round(s / 3600), 'hour');
  return rtf.format(-Math.round(s / 86400), 'day');
}

/** "today, 9:33 PM", "yesterday, 9:05 AM", "Nov 12, 2026, 2:20 PM" (in Turkish "bugün, 21:33" and so on): when something last happened, in the window's language. A day, then the time;
 * the year only when it is not this one. `now` is a parameter so the same moment always reads the same way. */
export function when(ms: number, now: number): string {
  const lang = getLanguage();
  const at = new Date(ms);
  const time = new Intl.DateTimeFormat(lang, { hour: 'numeric', minute: '2-digit' }).format(at);
  const midnight = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const days = Math.round((midnight(new Date(now)) - midnight(at)) / 86_400_000);
  if (days === 0 || days === 1) return t('when-day-time', { day: new Intl.RelativeTimeFormat(lang, { numeric: 'auto' }).format(-days, 'day'), time });
  const sameYear = at.getFullYear() === new Date(now).getFullYear();
  return new Intl.DateTimeFormat(lang, { day: 'numeric', month: 'short', ...(sameYear ? {} : { year: 'numeric' }), hour: 'numeric', minute: '2-digit' }).format(at);
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
