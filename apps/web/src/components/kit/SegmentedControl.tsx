import { useRef, type KeyboardEvent, type ReactNode } from 'react';

// One choice out of a few, all visible (Group by, a theme, a view): a radio group. One tab stop; the arrow keys move and choose (mirrored in
// a right-to-left page), Home and End go to the ends. The chosen item is a raised thumb with ink text. `risk`: the chosen item lets an agent
// act without asking, so the control and its thumb are red (ADR-0102); the screen that uses it keeps its own confirm step.
export interface Segment<T extends string> { value: T; label: ReactNode; icon?: ReactNode; title?: string }

export function SegmentedControl<T extends string>({ items, value, onChange, label, labelledBy, describedBy, risk, disabled }: {
  items: readonly Segment<T>[]; value: T; onChange: (value: T) => void; label?: string; labelledBy?: string; describedBy?: string; risk?: boolean; disabled?: boolean;
}) {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const at = Math.max(0, items.findIndex((i) => i.value === value));
  const move = (to: number) => {
    const n = (to + items.length) % items.length;
    const item = items[n];
    if (!item) return;
    onChange(item.value);
    refs.current[n]?.focus();
  };
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const rtl = getComputedStyle(e.currentTarget).direction === 'rtl';
    const step = { ArrowRight: rtl ? -1 : 1, ArrowLeft: rtl ? 1 : -1, ArrowDown: 1, ArrowUp: -1 }[e.key];
    if (step !== undefined) move(at + step);
    else if (e.key === 'Home') move(0);
    else if (e.key === 'End') move(items.length - 1);
    else return;
    e.preventDefault();
  };
  return (
    <div role="radiogroup" aria-label={label} aria-labelledby={labelledBy} aria-describedby={describedBy} aria-disabled={disabled || undefined}
      className={`kit-seg${risk ? ' kit-seg--risk' : ''}`} onKeyDown={onKey}>
      {items.map((it, i) => (
        <button key={it.value} ref={(el) => { refs.current[i] = el; }} type="button" role="radio" aria-checked={i === at} tabIndex={i === at ? 0 : -1}
          title={it.title} disabled={disabled} className="kit-seg__item" onClick={() => onChange(it.value)}>
          {it.icon}{it.label}
        </button>
      ))}
    </div>
  );
}
