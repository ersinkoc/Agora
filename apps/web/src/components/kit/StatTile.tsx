import type { ReactNode } from 'react';
import type { Tone } from './Pill';

// A count with its name: a big number and a label with its mark. Only a count that asks for you is tinted (`tone`, used for Needs you),
// and only while it is above zero; a zero is quieter (muted number, still readable). With `onPress` it is a filter button (aria-pressed);
// `title` says what pressing it does.
export function StatTile({ value, label, mark, tone, pressed, onPress, title }: {
  value: number | string; label: ReactNode; mark?: ReactNode; tone?: Tone; pressed?: boolean; onPress?: () => void; title?: string;
}) {
  const zero = value === 0;
  const cls = ['kit-stat', tone && !zero && `kit-stat--tinted kit-tone-${tone}`, zero && 'kit-stat--zero'].filter(Boolean).join(' ');
  const body = <><span className="kit-stat__n">{value}</span><span className="kit-stat__label">{mark}{label}</span></>;
  if (onPress) return <button type="button" className={cls} aria-pressed={!!pressed} title={title} onClick={onPress}>{body}</button>;
  return <div className={cls}>{body}</div>;
}
