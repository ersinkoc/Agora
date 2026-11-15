import { useState } from 'react';
import { t } from '../i18n';
import { Icon } from './Icon';
import { formatPairNumber, pairTitle, type PairRequestModel } from './pairChoice';

// ADR-0065 decision 3a: a machine that wants to join, or a browser on another machine that wants to sign in, waits for the owner's
// answer in a window they are already signed in to. The card names who asks and from where, and shows the number the other screen
// shows: the number is what ties this card to the machine or browser in front of the person, so they approve only when both match.
// A deny, or a mismatch on the other side, ends that request; asking again starts a new one with a new number.
export function PairRequest({ request, onAnswer, inline }: {
  request: PairRequestModel;
  // Shown in place (the gallery) instead of floating over the window.
  inline?: boolean;
  // `true` approves, `false` denies; resolves to whether the answer reached the hub.
  onAnswer: (approve: boolean) => Promise<boolean>;
}) {
  const [state, setState] = useState<'idle' | 'busy' | 'failed'>('idle');
  const answer = async (approve: boolean) => {
    setState('busy');
    setState((await onAnswer(approve)) ? 'idle' : 'failed');
  };
  const id = `pair-request-${request.id}`;
  return (
    <section role="dialog" aria-labelledby={id} aria-describedby={`${id}-compare`}
      className={`${inline ? 'w-full max-w-[24rem] ' : 'fixed bottom-4 end-4 z-40 w-[min(24rem,calc(100vw-2rem))] '}grid gap-3 rounded-m border border-line bg-surface-1 p-4 shadow-card`}>
      <div className="flex items-start gap-2">
        <Icon name={request.kind === 'machine' ? 'machine' : 'globe'} className="mt-0.5 size-4 text-accent" />
        <h2 id={id} className="m-0 flex-1 text-sm font-semibold tracking-normal text-ink normal-case">{pairTitle(request)}</h2>
      </div>
      <div className="grid gap-1">
        <span className="text-xs text-muted">{t('pair-number')}</span>
        <span className="font-mono text-[length:var(--text-20)] tracking-wide text-ink">{formatPairNumber(request.number)}</span>
        <span id={`${id}-compare`} className="text-xs text-muted">{t(request.kind === 'machine' ? 'pair-machine-compare' : 'pair-browser-compare')}</span>
      </div>
      {state === 'failed' && <p role="alert" className="m-0 text-needs-you">{t('pair-failed')}</p>}
      <div className="flex justify-end gap-2">
        <button type="button" className="button" disabled={state === 'busy'} onClick={() => void answer(false)}>{t('pair-deny')}</button>
        <button type="button" className="button button--primary" disabled={state === 'busy'} onClick={() => void answer(true)}>{t('pair-approve')}</button>
      </div>
    </section>
  );
}
