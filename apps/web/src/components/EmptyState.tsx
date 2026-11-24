import type { IconName } from './Icon';
import { EmptyState as KitEmptyState } from './kit';

// A panel with nothing in it yet: the widget kit's empty state (an icon in a tinted chip, a short friendly title and one line on what will
// appear), so every empty panel looks the same. Tokens only; never an error look.
export function EmptyState({ icon, title, hint }: { icon: IconName; title: string; hint: string }) {
  return <KitEmptyState icon={icon} title={title} line={hint} />;
}

// A placeholder while something loads: pulses softly (still when the reader asked for reduced motion). Decorative: the container says what is loading.
export function Skeleton({ className = '' }: { className?: string }) {
  return <span aria-hidden className={`skeleton ${className}`} />;
}
