import { t } from '../i18n';
import type { UiState } from '../model';

// Shape per state (visual-language.md): filled dot, ring with notch, check, hollow dot, dashed ring.
function Mark({ state }: { state: UiState }) {
  const common = { className: 'state__mark', viewBox: '0 0 12 12', 'aria-hidden': true } as const;
  switch (state) {
    case 'working':
      return <svg {...common}><circle cx="6" cy="6" r="5" fill="currentColor" /></svg>;
    case 'needs-you':
      return (
        <svg {...common}>
          <circle cx="6" cy="6" r="4.2" fill="none" stroke="currentColor" strokeWidth="1.8" strokeDasharray="22 4" />
          <path d="M6 0.5v3" stroke="currentColor" strokeWidth="1.8" />
        </svg>
      );
    case 'finished':
      return <svg {...common}><path d="M2 6.5l2.5 2.5L10 3" fill="none" stroke="currentColor" strokeWidth="1.8" /></svg>;
    case 'idle':
      return <svg {...common}><circle cx="6" cy="6" r="4.2" fill="none" stroke="currentColor" strokeWidth="1.6" /></svg>;
    default:
      return <svg {...common}><circle cx="6" cy="6" r="4.2" fill="none" stroke="currentColor" strokeWidth="1.6" strokeDasharray="2.5 2" /></svg>;
  }
}

export function StateBadge({ state }: { state: UiState }) {
  return (
    <span className={`state state--${state}`}>
      <Mark state={state} />
      {t(`state-${state}`)}
    </span>
  );
}

// The shape alone, for dense rows and tabs; the state's name stays available to screen readers.
export function StateMark({ state }: { state: UiState }) {
  return (
    <span className={`state state--${state}`}>
      <Mark state={state} />
      <span className="sr-only">{t(`state-${state}`)}</span>
    </span>
  );
}
