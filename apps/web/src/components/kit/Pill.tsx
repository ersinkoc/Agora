import type { ReactNode } from 'react';
import { t } from '../../i18n';
import type { UiState } from '../../model';
import { Mark } from '../StateBadge';

// A tone is a colour, its soft tint and its hairline (app.css `.kit-tone-*`). One meaning per colour (ADR-0102): amber is only "needs you",
// red only danger, the accent is the brand and selection, green is finished, blue is working; plain is a neutral fact.
export type Tone = 'accent' | 'working' | 'needs' | 'finished' | 'danger' | 'idle' | 'plain';

export function Pill({ tone = 'plain', icon, children }: { tone?: Tone; icon?: ReactNode; children: ReactNode }) {
  return <span className={`kit-pill kit-tone-${tone}`}>{icon}{children}</span>;
}

// The one table from an agent's state to its tone: every pill, mark and count of a state takes its colour here, so amber follows Needs you
// and nothing else (the amber test counts this table, amberOnlyNeedsYou.test.ts).
const STATE_TONE: Record<UiState, Tone> = { working: 'working', 'needs-you': 'needs', finished: 'finished', idle: 'idle', unknown: 'plain' };
export const stateTone = (state: UiState): Tone => STATE_TONE[state];

// An agent's state as a pill: tint, colour, shape and word, never colour alone. Failed is red with a cross in a ring (ADR-0102).
export function StatusPill({ state, failed }: { state: UiState; failed?: boolean }) {
  return (
    <Pill tone={failed ? 'danger' : stateTone(state)} icon={<Mark state={state} failed={failed} />}>
      {t(failed ? 'fleet-failed' : `state-${state}`)}
    </Pill>
  );
}

// The state's shape alone, in its kit colour (failed in red), for dense rows and counts; its word stays for screen readers.
export function StatusMark({ state, failed }: { state: UiState; failed?: boolean }) {
  return (
    <span className={`kit-mark kit-tone-${failed ? 'danger' : stateTone(state)}`} title={t(failed ? 'fleet-failed' : `state-${state}`)}>
      <Mark state={state} failed={failed} />
      <span className="sr-only">{t(failed ? 'fleet-failed' : `state-${state}`)}</span>
    </span>
  );
}

// A neutral fact (an OS, a version, a branch, a count): grey, never a state colour. `mono` for versions, ids and branches.
export function Tag({ mono, children }: { mono?: boolean; children: ReactNode }) {
  return <span className={`kit-tag${mono ? ' kit-tag--mono' : ''}`}>{children}</span>;
}
