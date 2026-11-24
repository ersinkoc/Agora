import { t } from '../i18n';
import type { UiState } from '../model';

// Shape per state (visual-language.md): filled dot, ring with notch, check, hollow dot, dashed ring.
export function Mark({ state, failed }: { state: UiState; failed?: boolean }) {
  const common = { className: 'state__mark', viewBox: '0 0 12 12', 'aria-hidden': true } as const;
  // Ended with an error: a cross in a ring, in the neutral text colour (shape and words say it, no state colour is borrowed).
  if (failed) return <svg {...common}><circle cx="6" cy="6" r="5" fill="none" stroke="currentColor" strokeWidth="1.2" /><path d="M4 4l4 4M8 4l-4 4" stroke="currentColor" strokeWidth="1.4" /></svg>;
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

const stateClass = (state: UiState, failed?: boolean) => `state state--${failed ? 'failed' : state}`;
const stateWord = (state: UiState, failed?: boolean) => t(failed ? 'state-failed' : `state-${state}`);

// `word`: a shorter name for the state where the full one does not fit (the counts of the fleet console say "Failed").
export function StateBadge({ state, failed, word }: { state: UiState; failed?: boolean; word?: string }) {
  return (
    <span className={stateClass(state, failed)}>
      <Mark state={state} failed={failed} />
      {word ?? stateWord(state, failed)}
    </span>
  );
}

// The shape alone, for dense rows and tabs; the state's name stays available to screen readers.
export function StateMark({ state, failed }: { state: UiState; failed?: boolean }) {
  return (
    <span className={stateClass(state, failed)}>
      <Mark state={state} failed={failed} />
      <span className="sr-only">{stateWord(state, failed)}</span>
    </span>
  );
}
