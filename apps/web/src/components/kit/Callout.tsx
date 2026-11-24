import { useId, type ReactNode } from 'react';
import { Icon, type IconName } from '../Icon';

// A notice or a request in a tinted box: an icon chip, a title, a body and actions at the end. Colour is the surface, never a bar on one side.
// `needs` is a request that waits on the person (amber), `info` a neutral notice (accent), `danger` an error or a destructive choice (red),
// `success` a good outcome (green). `alert` makes it a live region, for an error that appears after an action.
export type CalloutTone = 'needs' | 'info' | 'danger' | 'success';
const TONE = { needs: 'needs', info: 'accent', danger: 'danger', success: 'finished' } as const;

export function Callout({ tone = 'info', icon, title, children, actions, alert }: {
  tone?: CalloutTone; icon: IconName; title: ReactNode; children?: ReactNode; actions?: ReactNode; alert?: boolean;
}) {
  const id = useId();
  return (
    <section className={`kit-callout kit-tone-${TONE[tone]}`} aria-labelledby={id} role={alert ? 'alert' : undefined}>
      <span aria-hidden className="kit-chip"><Icon name={icon} /></span>
      <p id={id} className="kit-callout__title">{title}</p>
      {children ? <div className="kit-callout__body">{children}</div> : null}
      {actions ? <div className="kit-callout__actions">{actions}</div> : null}
    </section>
  );
}

// Nothing here yet: an icon in a tinted chip, a short title, one line on what will appear, and at most one action. Never an error look.
// `ok` is the "all clear" variant (green check chip).
export function EmptyState({ icon, title, line, action, ok }: { icon: IconName; title: ReactNode; line: ReactNode; action?: ReactNode; ok?: boolean }) {
  return (
    <div className={`kit-empty kit-tone-${ok ? 'finished' : 'accent'}`} role="status">
      <span aria-hidden className="kit-chip"><Icon name={icon} className="size-5" /></span>
      <p className="kit-empty__title">{title}</p>
      <p className="kit-empty__hint">{line}</p>
      {action ? <div className="kit-empty__action">{action}</div> : null}
    </div>
  );
}
