import { useSyncExternalStore } from 'react';
import { t } from '../i18n';
import { finishedMarks } from '../net/finished';
import { Mark } from './StateBadge';
import { Pill } from './kit';

/** Whether this agent finished while the person was not looking at it, and they have not seen it since (net/finished.ts). */
export const useFinishedUnseen = (agentId: string): boolean =>
  useSyncExternalStore(finishedMarks.subscribe, () => finishedMarks.has(agentId), () => false);

// The "finished, not seen yet" mark: the Finished green and its check, never amber, so it cannot be read as "needs you" (which keeps the
// amber dot on the avatar and its own place on the list). `compact` (a tab, a row of the agent list): a small tinted disc with the check,
// its words in its tooltip and for screen readers. Otherwise (a tile on Home, a row on the phone): a pill with the word as well.
export function FinishedMark({ agentId, compact }: { agentId: string; compact?: boolean }) {
  const on = useFinishedUnseen(agentId);
  if (!on) return null;
  const words = t('finished-unseen');
  if (compact) {
    return (
      <span className="unseen-mark" title={words} data-finished-unseen="">
        <Mark state="finished" />
        <span className="sr-only">{words}</span>
      </span>
    );
  }
  return (
    <span className="flex-none" title={words} data-finished-unseen="">
      <Pill tone="finished" icon={<Mark state="finished" />}><span aria-hidden>{t('finished-unseen-short')}</span><span className="sr-only">{words}</span></Pill>
    </span>
  );
}
