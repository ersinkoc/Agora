import { useEffect, useReducer, useState } from 'react';
import { t } from '../i18n';
import { span } from '../net/fleet';
import { Icon } from './Icon';
import { Button } from './kit';
import { initialPairCard, isolated, MIN_THINK_MS, pairCardReducer, pairTitle, peerText, systemName, type PairCardState, type PairRequestModel } from './pairChoice';

/** What the hub answered when the owner decided: `taken` is `pair.state name_in_use` (the name already belongs to an enrolled machine). */
export type PairAnswer = 'ok' | 'taken' | 'failed';

// ADR-0065 decision 3a: a machine that wants to join, or a browser on another machine that wants to sign in, waits for the owner's
// answer in a window they are already signed in to. The card names who asks and from where, and shows the number the other screen
// shows: the number is what ties this card to the machine or browser in front of the person, so they approve only when both match.
// Approving takes two presses ("Numbers match…", then "Yes, the numbers match"), so a reflex click never lets a machine in. A deny, or a mismatch
// on the other side, ends that request; asking again starts a new one with a new number. Nothing here is stored: the number lives in this
// component's props and nowhere else.
export function PairRequest({ request, onAnswer, inline, wide, offline, now: clock }: {
  request: PairRequestModel & { expiresMs?: number; nameTaken?: boolean };
  // Shown in place (the gallery, the queue) instead of floating over the window.
  inline?: boolean;
  // As wide as its page (My machines): who asks at the start, the number and its presses at the end; narrow, they stack as usual.
  wide?: boolean;
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
  // The question has been up long enough to answer: a double-click on Approve must not reach the yes.
  useEffect(() => {
    if (state.step !== 'confirm') return;
    const timer = setTimeout(() => act({ type: 'armed' }), MIN_THINK_MS);
    return () => clearTimeout(timer);
  }, [state.step]);
  const send = async (approve: boolean) => {
    // Only a send the steps allow goes out (a yes only from the question once it is armed).
    if (pairCardReducer(state, { type: 'send', approve }) === state) return;
    act({ type: 'send', approve });
    const r = await onAnswer(approve, approve && !!request.nameTaken && state.replace).catch((): PairAnswer => 'failed');
    act({ type: 'answered', answer: r });
  };
  return <PairCardView request={request} state={state} inline={inline} wide={wide} offline={offline} now={clock ?? ticked}
    onApprove={() => act({ type: 'approve' })} onBack={() => act({ type: 'back' })} onSend={(a) => void send(a)} onReplace={(on) => act({ type: 'replace', on })} />;
}

// The card as a pure function of its state (render is pure): what it shows for the request, the clock, the link and the owner's steps.
export function PairCardView({ request, state, inline, wide, offline, now, onApprove, onBack, onSend, onReplace }: {
  request: PairRequestModel & { expiresMs?: number; nameTaken?: boolean };
  state: PairCardState;
  inline?: boolean;
  wide?: boolean;
  offline?: boolean;
  now: number;
  onApprove: () => void;
  onBack: () => void;
  onSend: (approve: boolean) => void;
  onReplace: (on: boolean) => void;
}) {
  const { step, failed, replace } = state;
  const name = peerText(request.name);
  const left = request.expiresMs === undefined ? undefined : Math.max(0, Math.ceil((request.expiresMs - now) / 1000));
  const expired = left === 0;
  // Replace is a choice only when the hub said the name is taken (`pair.state name_in_use`), and it starts unticked every time.
  const taken = !!request.nameTaken;
  const id = `pair-request-${request.id}`;
  const blocked = !!offline || expired;
  const why = offline ? 'pair-offline' : expired ? 'pair-ran-out' : undefined;
  const digits = /^\d{6}$/.test(request.number) ? [request.number.slice(0, 3), request.number.slice(3)] : [request.number];
  const busy = step === 'busy' || step === 'sent';
  // A request that waits on the owner: the "needs you" tint and hairline, an icon chip, never a bar on one side (ADR-0102). Floating, it is
  // an overlay and takes the one overlay shadow.
  const where = wide ? 'pair-card--wide w-full ' : inline ? 'w-full max-w-[24rem] ' : 'pair-card--float fixed bottom-4 end-4 z-40 w-[min(24rem,calc(100vw-2rem))] ';
  return (
    <section role="dialog" tabIndex={-1} aria-labelledby={id} aria-describedby={`${id}-compare`} className={`${where}pair-card kit-tone-needs`}>
      <div className="pair-card__who">
        <div className="flex items-center gap-2.5">
          <span aria-hidden className="kit-chip"><Icon name={request.kind === 'machine' ? 'machine' : 'globe'} /></span>
          <h2 id={id} className="m-0 flex-1 text-sm font-semibold tracking-normal text-ink normal-case">{pairTitle(request)}</h2>
        </div>
        {/* Who asks, one fact for each row. The name and the system are what the other side says about itself; the address is what the
            hub saw. They never share a sentence, so one cannot pass for another. Each is isolated (`bdi`) so text that runs the other way
            cannot reorder the labels around it. */}
        <dl className="m-0 grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
          <dt className="text-muted">{t('pair-name')}</dt>
          <dd className="m-0 min-w-0 break-words text-ink"><bdi>{name}</bdi></dd>
          {request.kind === 'machine' && systemName(request.os) && (<>
            <dt className="text-muted">{t('pair-system')}</dt>
            <dd className="m-0 min-w-0 break-words text-ink"><bdi>{peerText(systemName(request.os))}</bdi></dd>
          </>)}
          <dt className="text-muted">{t('pair-address')}</dt>
          <dd className="m-0 min-w-0 break-words font-mono text-ink"><bdi dir="ltr">{peerText(request.address)}</bdi></dd>
        </dl>
        {left !== undefined && !expired && <p className="m-0 text-xs text-muted" role="timer">{t('pair-expires-in', { time: span(left * 1000) })}</p>}
      </div>
      <div className="pair-card__decide">
        <div className="grid gap-1">
          <span className="text-xs text-muted">{t('pair-number')}</span>
          <span dir="ltr" className="pair-number">
            {digits.map((d, i) => <span key={i}>{d}</span>)}
          </span>
          <span id={`${id}-compare`} className="text-xs text-muted">{t(request.kind === 'machine' ? 'pair-machine-compare' : 'pair-browser-compare')}</span>
        </div>
        {taken && !expired && (
          <label className="flex items-start gap-2 text-xs text-ink">
            <input type="checkbox" className="mt-0.5" checked={replace} disabled={busy || blocked} onChange={(e) => onReplace(e.target.checked)} />
            <span><strong className="font-semibold">{t('pair-replace')}</strong><br /><span className="text-muted">{t('pair-replace-why', { name: isolated(request.name) })}</span></span>
          </label>
        )}
        {failed && <p role="alert" className="m-0 text-danger">{t('pair-failed')}</p>}
        {why && <p id={`${id}-why`} className="m-0 text-xs text-muted">{t(why)}</p>}
        {step === 'sent' && <p role="status" className="m-0 text-xs text-muted">{t('pair-sent')}</p>}
        {/* The question has a place of its own above the buttons, so the buttons never move when it appears: the yes is not under the
            pointer that pressed Approve. */}
        <p role={step === 'confirm' ? 'alert' : undefined} className="m-0 min-h-[1.25rem] text-xs font-semibold text-ink">{step === 'confirm' ? t('pair-confirm') : ''}</p>
        {step === 'confirm' ? (
          <div className="flex flex-wrap justify-end gap-2">
            <Button onClick={onBack}>{t('pair-back')}</Button>
            <Button variant="primary" disabled={blocked || !state.armed} aria-describedby={why ? `${id}-why` : undefined} onClick={() => onSend(true)}>{t('pair-confirm-yes')}</Button>
          </div>
        ) : (
          <div className="flex flex-wrap justify-end gap-2">
            <Button disabled={busy || blocked} aria-describedby={why ? `${id}-why` : undefined} onClick={() => onSend(false)}>{t('pair-deny')}</Button>
            <Button variant="primary" disabled={busy || blocked} aria-describedby={why ? `${id}-why` : undefined} onClick={onApprove}>{t('pair-approve')}</Button>
          </div>
        )}
      </div>
    </section>
  );
}
