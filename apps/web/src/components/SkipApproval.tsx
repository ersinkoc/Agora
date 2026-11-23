import { useEffect, useId, useRef, useState } from 'react';
import { t } from '../i18n';
import { ATTENTION_DWELL_MS } from '../net/attention';
import { approvalName } from '../net/approvalText';
import { dismissUnknown, isDismissedUnknown, useSkipCapable } from '../net/skipApproval';
import { visible, type ApprovalModel } from '../net/view';
import { StateMark } from './StateBadge';
import { Button } from './kit';

// ADR-0090 Amendment A: a window asked to switch a running agent to a mode or option in which it acts without asking, and the machine
// wants the owner's signed answer first. Every field is the machine's own rendering of the request (`detail`), shown as it gave it with
// any hidden character written as a visible escape: the agent's names for the setting and the choice are its own words, so the ids that
// the answer is signed over are shown beside them. Nothing here is rebuilt from the window's own option labels.
// The card is answerable only for a machine the window has heard offer this (`approval.skip_permissions`) and only in the shape it expects;
// any other shape says to update the window and signs nothing.
export function SkipApproval({ approval, busy, onResolve }: {
  approval: ApprovalModel;
  busy?: boolean;
  onResolve?: (approved: boolean) => void;
}) {
  const capable = useSkipCapable(approval.node);
  const skip = approval.skip;
  // Allow stays off for a short dwell (as the attention dialog and the phone bar do), restarted whenever what the card says changes, so a
  // click meant for something else never allows it. After a click both answers are off for a few seconds: one click sends one decision.
  const wait = `skip-wait-${useId()}`;
  const [ready, setReady] = useState(false);
  const [sent, setSent] = useState(false);
  const once = useRef(false); // set at once by the first click: state alone would let two clicks in one frame both go
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => {
    setReady(false); setSent(false); once.current = false;
    const h = setTimeout(() => setReady(true), ATTENTION_DWELL_MS);
    return () => clearTimeout(h);
  }, [approval.id, approval.detail]);
  useEffect(() => () => clearTimeout(timer.current), []);
  const answer = (ok: boolean) => {
    if (once.current || !onResolve) return;
    once.current = true;
    setSent(true);
    clearTimeout(timer.current);
    timer.current = setTimeout(() => { once.current = false; setSent(false); }, 4000); // a failed answer can be tried again
    onResolve(ok);
  };
  if (!skip) {
    if (isDismissedUnknown(approval.id)) return null;
    return (
      <article role="alert" aria-label={t('skip-approval-unknown-title')} className="flex flex-col gap-1 border-b border-needs-you/60 bg-surface-2 px-3 py-2">
        <div className="flex items-center gap-2"><StateMark state="needs-you" /><strong className="min-w-0 flex-1 break-words">{t('skip-approval-unknown-title')}</strong></div>
        <p className="m-0 text-xs text-muted">{t('skip-approval-unknown')}</p>
        {/* Only hides the card in this window: a decision in a shape this window does not understand is never signed or sent. */}
        <div><button type="button" className="button px-3 py-1 text-xs" onClick={() => dismissUnknown(approval.id)}>{t('skip-approval-dismiss')}</button></div>
      </article>
    );
  }
  const who = visible(approvalName(approval));
  const title = t('skip-approval-title', { name: who });
  const answerable = capable && !!onResolve;
  const setting = skip.optionType === 'mode' ? t('session-mode') : visible(skip.optionName) || visible(skip.option);
  return (
    <article role="alert" aria-label={title} data-skip-approval className="flex flex-col gap-2 border-b border-needs-you/60 bg-surface-2 px-3 py-2">
      <div className="flex items-center gap-2">
        <StateMark state="needs-you" />
        <strong className="min-w-0 flex-1 break-words">{title}</strong>
      </div>
      <p className="m-0 text-xs">{t('skip-approval-body')}</p>
      <dl className="m-0 grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 text-xs">
        {approval.machineName?.trim() && <><dt className="text-muted">{t('skip-approval-machine')}</dt><dd className="m-0 break-words">{visible(approval.machineName)}</dd></>}
        <dt className="text-muted">{t('start-approval-folder')}</dt>
        <dd className="m-0 break-all font-mono" dir="ltr">{visible(skip.folder)}</dd>
        <dt className="text-muted">{t('skip-approval-setting')}</dt>
        <dd className="m-0 break-words">{setting}{skip.optionType === 'config' && <> <code dir="ltr" className="font-mono text-muted">{visible(skip.option)}</code></>}</dd>
        <dt className="text-muted">{t('skip-approval-choice')}</dt>
        <dd className="m-0 break-words">{visible(skip.valueName)} <code dir="ltr" className="font-mono text-muted">{visible(skip.value)}</code></dd>
      </dl>
      <p className="m-0 text-xs text-muted">{t('skip-approval-scope')}</p>
      {!capable && <p role="note" className="m-0 text-xs text-muted">{t('skip-approval-other-window')}</p>}
      {/* Redesign PR 4: Deny, then Allow, as on every request; this Allow lets the agent act without asking, so it is the danger button
          (ADR-0102). The dwell and "Read it first" are unchanged. */}
      <div className="flex flex-wrap items-center gap-2">
        <Button size="sm" disabled={sent || !answerable} onClick={() => answer(false)}>{t('approval-deny')}</Button>
        <Button size="sm" variant="danger" className="relative overflow-hidden" disabled={sent || !answerable || !ready} aria-describedby={!ready ? `${wait}` : undefined} onClick={() => answer(true)}>
          {t('approval-allow')}
          {answerable && <span aria-hidden className={`absolute inset-x-0 bottom-0 h-0.5 origin-left bg-current opacity-60 transition-transform ease-linear motion-reduce:transition-none ${ready ? 'scale-x-100' : 'scale-x-0'}`} style={{ transitionDuration: `${ATTENTION_DWELL_MS}ms` }} />}
        </Button>
        {answerable && !ready && <span id={wait} className="text-xs text-muted">{t('skip-approval-read-first')}</span>}
      </div>
    </article>
  );
}
