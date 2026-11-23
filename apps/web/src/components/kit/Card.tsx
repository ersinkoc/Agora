import { useId, type ReactNode } from 'react';

// A surface for one thing (a machine, a setting group, a tile): flat, one hairline, no shadow. `onClick` makes the whole card one button;
// `selected` is an accent ring on an accent tint (aria-pressed says it to assistive tech).
export function Card({ children, density = 'normal', selected, onClick, label, className = '' }: {
  children: ReactNode; density?: 'normal' | 'dense' | 'flush'; selected?: boolean; onClick?: () => void; label?: string; className?: string;
}) {
  const cls = `kit-card${density === 'normal' ? '' : ` kit-card--${density}`} ${className}`.trim();
  const sel = selected ? { 'data-selected': '' } : {};
  if (onClick) return <button type="button" className={cls} aria-pressed={selected ?? undefined} aria-label={label} onClick={onClick} {...sel}>{children}</button>;
  return <div className={cls} aria-label={label} role={label ? 'group' : undefined} {...sel}>{children}</div>;
}

// A row in a list: a leading avatar or icon, the label, trailing facts. The current row is an accent tint and a bolder label (no edge bar).
// A link when it has `href`, a button when it has `onClick`, plain text otherwise.
export function ListRow({ leading, label, meta, current, href, onClick }: {
  leading?: ReactNode; label: ReactNode; meta?: ReactNode; current?: boolean; href?: string; onClick?: () => void;
}) {
  const inner = <>{leading}<span className="kit-row__label">{label}</span>{meta ? <span className="kit-row__meta">{meta}</span> : null}</>;
  const cur = current ? { 'aria-current': (href ? 'page' : 'true') as 'page' | 'true' } : {};
  if (href) return <a className="kit-row" href={href} {...cur}>{inner}</a>;
  if (onClick) return <button type="button" className="kit-row" onClick={onClick} {...cur}>{inner}</button>;
  return <div className="kit-row" {...cur}>{inner}</div>;
}

// One setting: its name and an optional one-line hint at the start, its control at the end. The control is given the ids it needs:
// `children(labelId, hintId)` lets it say `aria-labelledby` and `aria-describedby`, so the name is said once, by the row.
export function SettingRow({ label, hint, children }: { label: ReactNode; hint?: ReactNode; children: (labelId: string, hintId: string | undefined) => ReactNode }) {
  const id = useId();
  const hintId = hint ? `${id}-hint` : undefined;
  return (
    <div className="kit-setting">
      <div className="kit-setting__text">
        <span id={`${id}-label`} className="kit-setting__label">{label}</span>
        {hint ? <span id={hintId} className="kit-setting__hint">{hint}</span> : null}
      </div>
      <div className="kit-setting__control">{children(`${id}-label`, hintId)}</div>
    </div>
  );
}

// Facts as name and value in two columns (a description list); ids, paths and versions in `mono`.
export function KeyValue({ items }: { items: readonly { label: ReactNode; value: ReactNode; mono?: boolean }[] }) {
  return (
    <dl className="kit-kv">
      {items.map((it, i) => <div key={i} className="contents"><dt>{it.label}</dt><dd className={it.mono ? 'kit-kv__mono' : undefined}>{it.value}</dd></div>)}
    </dl>
  );
}
