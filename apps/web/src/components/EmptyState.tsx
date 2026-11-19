import { Icon, type IconName } from './Icon';

// A panel with nothing in it yet: an icon, a short friendly title and one line on what will appear. Tokens only; never an error look.
export function EmptyState({ icon, title, hint }: { icon: IconName; title: string; hint: string }) {
  return (
    <div className="empty-state" role="status">
      <span aria-hidden className="empty-state__icon"><Icon name={icon} className="size-5" /></span>
      <p className="empty-state__title">{title}</p>
      <p className="empty-state__hint">{hint}</p>
    </div>
  );
}

// A placeholder while something loads: pulses softly (still when the reader asked for reduced motion). Decorative: the container says what is loading.
export function Skeleton({ className = '' }: { className?: string }) {
  return <span aria-hidden className={`skeleton ${className}`} />;
}
