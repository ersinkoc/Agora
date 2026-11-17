import { useEffect, useReducer, useState } from 'react';
import { t } from '../i18n';
import { span } from '../net/fleet';
import { Icon } from './Icon';
import { initialPairCard, pairCardReducer, pairTitle, systemName, type PairCardState, type PairRequestModel } from './pairChoice';

/** What the hub answered when the owner decided: `taken` is `pair.state name_in_use` (the name already belongs to an enrolled machine). */
export type PairAnswer = 'ok' | 'taken' | 'failed';

// ADR-0065 decision 3a: a machine that wants to join, or a browser on another machine that wants to sign in, waits for the owner's
// answer in a window they are already signed in to. The card names who asks and from where, and shows the number the other screen
// shows: the number is what ties this card to the machine or browser in front of the person, so they approve only when both match.
// Approving takes two presses ("Approve", then "Yes, the numbers match"), so a reflex click never lets a machine in. A deny, or a mismatch
// on the other side, ends that request; asking again starts a new one with a new number. Nothing here is stored: the number lives in this
// component's props and nowhere else.
export function PairRequest({ request, onAnswer, inline, offline, now: clock }: {
  request: PairRequestModel & { expiresMs?: number; nameTaken?: boolean };
  // Shown in place (the gallery, the queue) instead of floating over the window.
  inline?: boolean;
  // `approve` answers yes; `replace` says an enrolled machine of the same name is replaced. Resolves to what the hub said.
  onAnswer: (approve: boolean, replace: boolean) => Promise<PairAnswer>;
  // The link to the hub is not live: an answer cannot be sent, and the card says so.
  offline?: boolean;
  // The time to count down from; by default the clock, once a second.
  now?: number;
}) {
  const [state, act] = useReducer(pairCardReducer, initialPairCard);
  const [ticked, setTicked] = useState(() => Date.now());
  const counting = clock === undefined && request.expiresMs !== undefined;
  useEffect(() => {
    if (!counting) return;
    const timer = setInterval(() => setTicked(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [counting]);
  const send = async (approve: boolean) => {
    act({ type: 'send' });
    const r = await onAnswer(approve, approve && !!request.nameTaken && state.replace).catch((): PairAnswer => 'failed');
    act({ type: 'answered', answer: r });
  };
  return <PairCardView request={request} state={state} inline={inline} offline={offline} now={clock ?? ticked}
    onApprove={() => act({ type: 'approve' })} onBack={() => act({ type: 'back' })} onSend={(a) => void send(a)} onReplace={(on) => act({ type: 'replace', on })} />;
}

// The card as a pure function of its state (render is pure): what it shows for the request, the clock, the link and the owner's steps.
export function PairCardView({ request, state, inline, offline, now, onApprove, onBack, onSend, onReplace }: {
  request: PairRequestModel & { expiresMs?: number; nameTaken?: boolean };
  state: PairCardState;
  inline?: boolean;
  offline?: boolean;
  now: number;
  onApprove: () => void;
  onBack: () => void;
  onSend: (approve: boolean) => void;
  onReplace: (on: boolean) => void;
}) {
  const { step, failed, replace } = state;
  const left = request.expiresMs === undefined ? undefined : Math.max(0, Math.ceil((request.expiresMs - now) / 1000));
  const expired = left === 0;
  // Replace is a choice only when the hub said the name is taken (`pair.state name_in_use`), and it starts unticked every time.
  const taken = !!request.nameTaken;
  const id = `pair-request-${request.id}`;
  const blocked = !!offline || expired;
  const why = offline ? 'pair-offline' : expired ? 'pair-ran-out' : undefined;
  const digits = /^\d{6}$/.test(request.number) ? [request.number.slice(0, 3), request.number.slice(3)] : [request.number];
  const busy = step === 'busy' || step === 'sent';
  return (
    <section role="dialog" tabIndex={-1} aria-labelledby={id} aria-describedby={`${id}-compare`}
      className={`${inline ? 'w-full max-w-[24rem] ' : 'fixed bottom-4 end-4 z-40 w-[min(24rem,calc(100vw-2rem))] '}grid gap-3 rounded-m border border-line bg-surface-1 p-4 shadow-card`}>
      <div className="flex items-start gap-2">
        <Icon name={request.kind === 'machine' ? 'machine' : 'globe'} className="mt-0.5 size-4 text-accent" />
        <h2 id={id} className="m-0 flex-1 text-sm font-semibold tracking-normal text-ink normal-case">{pairTitle(request)}</h2>
      </div>
      {/* Who asks, one fact for each row. The name and the system are what the other side says about itself; the address is what the
          hub saw. They never share a sentence, so one cannot pass for another. Each is isolated (`bdi`) so text that runs the other way
          cannot reorder the labels around it. */}
      <dl className="m-0 grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
        <dt className="text-muted">{t('pair-name')}</dt>
        <dd className="m-0 min-w-0 break-words text-ink"><bdi>{request.name}</bdi></dd>
        {request.kind === 'machine' && systemName(request.os) && (<>
          <dt className="text-muted">{t('pair-system')}</dt>
          <dd className="m-0 min-w-0 break-words text-ink"><bdi>{systemName(request.os)}</bdi></dd>
        </>)}
        <dt className="text-muted">{t('pair-address')}</dt>
        <dd className="m-0 min-w-0 break-words font-mono text-ink"><bdi dir="ltr">{request.address}</bdi></dd>
      </dl>
      <div className="grid gap-1">
        <span className="text-xs text-muted">{t('pair-number')}</span>
        <span dir="ltr" className="pair-number">
          {digits.map((d, i) => <span key={i}>{d}</span>)}
        </span>
        <span id={`${id}-compare`} className="text-xs text-muted">{t(request.kind === 'machine' ? 'pair-machine-compare' : 'pair-browser-compare')}</span>
      </div>
      {left !== undefined && !expired && <p className="m-0 text-xs text-muted" role="timer">{t('pair-expires-in', { time: span(left * 1000) })}</p>}
      {taken && !expired && (
        <label className="flex items-start gap-2 text-xs text-ink">
          <input type="checkbox" className="mt-0.5" checked={replace} disabled={busy || blocked} onChange={(e) => onReplace(e.target.checked)} />
          <span><strong className="font-semibold">{t('pair-replace')}</strong><br /><span className="text-muted">{t('pair-replace-why', { name: request.name })}</span></span>
        </label>
      )}
      {failed && <p role="alert" className="m-0 text-needs-you">{t('pair-failed')}</p>}
      {why && <p id={`${id}-why`} className="m-0 text-xs text-muted">{t(why)}</p>}
      {step === 'sent' && <p role="status" className="m-0 text-xs text-muted">{t('pair-sent')}</p>}
      {step === 'confirm' ? (
        <div className="grid gap-2">
          <p role="alert" className="m-0 text-xs font-semibold text-ink">{t('pair-confirm')}</p>
          <div className="flex justify-end gap-2">
            <button type="button" className="button" onClick={onBack}>{t('pair-back')}</button>
            <button type="button" className="button button--primary" disabled={blocked} aria-describedby={why ? `${id}-why` : undefined} onClick={() => onSend(true)}>{t('pair-confirm-yes')}</button>
          </div>
        </div>
      ) : (
        <div className="flex justify-end gap-2">
          <button type="button" className="button" disabled={busy || blocked} aria-describedby={why ? `${id}-why` : undefined} onClick={() => onSend(false)}>{t('pair-deny')}</button>
          <button type="button" className="button button--primary" disabled={busy || blocked} aria-describedby={why ? `${id}-why` : undefined} onClick={onApprove}>{t('pair-approve')}</button>
        </div>
      )}
    </section>
  );
}
