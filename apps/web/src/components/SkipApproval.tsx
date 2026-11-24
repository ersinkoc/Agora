import { useEffect, useId, useRef, useState, type ReactElement, type ReactNode } from 'react';
import { t } from '../i18n';
import { ATTENTION_DWELL_MS } from '../net/attention';
import { approvalName } from '../net/approvalText';
import { dismissUnknown, isDismissedUnknown, useSkipCapable } from '../net/skipApproval';
import { visible, type ApprovalModel } from '../net/view';
import { ApprovalRequest, ASK_ICON, type ApprovalDensity } from './ApprovalRequest';
import { Button, KeyValue } from './kit';
import { useAnswerHolds } from './useAnswerGuard';

// ADR-0090 Amendment A: a window asked to switch a running agent to a mode or option in which it acts without asking, and the machine
// wants the owner's signed answer first. Every field is the machine's own rendering of the request (`detail`), shown as it gave it with
// any hidden character written as a visible escape: the agent's names for the setting and the choice are its own words, so the ids that
// the answer is signed over are shown beside them. Nothing here is rebuilt from the window's own option labels.
// The card is answerable only for a machine the window has heard offer this (`approval.skip_permissions`) and only in the shape it expects;
// any other shape says to update the window and signs nothing.

// A switch's own guards, wherever its answers are drawn (the short card's Deny, Review's Deny and Allow, the phone's card):
// - Allow stays off for a short dwell (as the attention dialog and the phone bar do), restarted whenever what the card says changes, so a
//   click meant for something else never allows it ("Read it first", with a bar that fills).
// - After a click both answers are off for a few seconds: one click sends one decision.
// - The window's holds (#1048, `useAnswerGuard`) are shared with every surface: while an answer to this request is on its way, or once it
//   went, from this card or another, both answers here are off.
// - Deny has no dwell; it is off only while busy, held or answered (main-window.md). Except in Review's dialog (`denyWaits`, #1070):
//   there Deny waits out the same dwell as Allow from the moment the dialog opens, so a double click on Review (from a card, Home's row
//   or the attention dialog) never lands on Deny either.
// `busy` (a surface that cannot take an answer now: one on its way, the link down) turns both answers off.
export function useSkipAnswer(approval: ApprovalModel, busy: boolean | undefined, onResolve: ((approved: boolean) => unknown) | undefined, denyWaits = false) {
  const capable = useSkipCapable(approval.node);
  const wait = `skip-wait-${useId()}`;
  const [ready, setReady] = useState(false);
  const readyNow = useRef(false); // `ready` as the click handler sees it, even before React has drawn the change
  const [sent, setSent] = useState(false);
  const once = useRef(false); // set at once by the first click: state alone would let two clicks in one frame both go
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => {
    setReady(false); setSent(false); once.current = false; readyNow.current = false;
    const h = setTimeout(() => { readyNow.current = true; setReady(true); }, ATTENTION_DWELL_MS);
    return () => clearTimeout(h);
  }, [approval.id, approval.detail]);
  useEffect(() => () => clearTimeout(timer.current), []);
  // Drawn outside the window's provider (a card alone) it keeps no answer that went, only the one on its way: its own lock says the rest.
  const holds = useAnswerHolds([]);
  const held = holds.status(approval.id) !== 'open';
  const answer = (ok: boolean) => {
    if (once.current || busy || !onResolve || holds.status(approval.id) !== 'open') return;
    // The press itself is refused during the dwell, not only the button (Allow always; Deny where it waits too).
    if (!readyNow.current && (ok || denyWaits)) return;
    once.current = true;
    setSent(true);
    clearTimeout(timer.current);
    timer.current = setTimeout(() => { once.current = false; setSent(false); }, 4000); // a failed answer can be tried again
    holds.hold(approval.id, () => onResolve(ok));
  };
  const answerable = capable && !!onResolve;
  const off = sent || !!busy || held || !answerable;
  return { capable, answerable, ready, wait, answer, denyOff: off || (denyWaits && !ready), allowOff: off || !ready };
}

/** The setting a switch changes, in the agent's words (a mode is the window's word "Mode"). */
const settingOf = (skip: NonNullable<ApprovalModel['skip']>) => (skip.optionType === 'mode' ? t('session-mode') : visible(skip.optionName) || visible(skip.option));

/** The title of a switch: the ask ("Let claude · amber act without asking?"). */
export const skipTitle = (approval: ApprovalModel) => t('skip-approval-title', { name: visible(approvalName(approval)) });

// Redesign PR 5: a switch in short, for a card that offers Deny and Review (never Allow): the setting and the choice (the agent's words
// with the ids the answer is signed over), then the folder, one line each, cut at the end. The whole card is read in Review.
export function SkipSummary({ approval }: { approval: ApprovalModel }) {
  const skip = approval.skip;
  if (!skip) return null;
  return (
    <div className="approval-req__summary" data-skip-summary>
      <p className="approval-req__line">
        <span className="approval-req__clip">{settingOf(skip)}{skip.optionType === 'config' && <> <code dir="ltr" className="font-mono">{visible(skip.option)}</code></>}
          {' · '}{t('skip-approval-choice')} <span className="text-ink">{visible(skip.valueName)}</span> <code dir="ltr" className="font-mono">{visible(skip.value)}</code></span>
      </p>
      <p className="approval-req__line"><span className="approval-req__clip font-mono" dir="ltr">{visible(skip.folder)}</span></p>
    </div>
  );
}

// A switch's card of a shape this window does not understand: it says so and can only be hidden here. A decision in a shape this window
// does not understand is never signed or sent.
export function SkipUnknown({ approval, density = 'card' }: { approval: ApprovalModel; density?: ApprovalDensity }) {
  useSkipCapable(approval.node); // redrawn when it is dismissed
  if (isDismissedUnknown(approval.id)) return null;
  return (
    <ApprovalRequest density={density} alert icon={ASK_ICON.skip} title={t('skip-approval-unknown-title')}
      actions={<Button size="sm" onClick={() => dismissUnknown(approval.id)}>{t('skip-approval-dismiss')}</Button>}>
      <p className="m-0 text-xs text-muted">{t('skip-approval-unknown')}</p>
    </ApprovalRequest>
  );
}

// The whole switch, in the request frame: Review's dialog (`full`) and the phone's card. Panes and the side panel show `SkipSummary`
// with Deny and Review. `busy` turns both answers off.
export function SkipApproval({ approval, busy, onResolve, density = 'card', titleWrap, lead, denyWaits }: {
  approval: ApprovalModel;
  busy?: boolean;
  /** What it returns says how the answer went (`AnswerResult`): `false`, a promise of it or one that rejects is a failed answer. */
  onResolve?: (approved: boolean) => unknown;
  density?: ApprovalDensity;
  titleWrap?: (title: ReactElement) => ReactNode;
  // Before the answers (Review's Close).
  lead?: ReactNode;
  /** Review's dialog (#1070): Deny waits out the open dwell too. */
  denyWaits?: boolean;
}) {
  const g = useSkipAnswer(approval, busy, onResolve, denyWaits);
  const skip = approval.skip;
  if (!skip) return <SkipUnknown approval={approval} density={density} />;
  const facts = [
    ...(approval.machineName?.trim() ? [{ label: t('skip-approval-machine'), value: visible(approval.machineName) }] : []),
    { label: t('start-approval-folder'), value: <span dir="ltr">{visible(skip.folder)}</span>, mono: true },
    { label: t('skip-approval-setting'), value: <>{settingOf(skip)}{skip.optionType === 'config' && <> <code dir="ltr" className="font-mono text-muted">{visible(skip.option)}</code></>}</> },
    { label: t('skip-approval-choice'), value: <>{visible(skip.valueName)} <code dir="ltr" className="font-mono text-muted">{visible(skip.value)}</code></> },
  ];
  return (
    <ApprovalRequest density={density} alert={density !== 'full'} icon={ASK_ICON.skip} title={skipTitle(approval)} titleWrap={titleWrap} className="approval-req--skip" attrs={{ 'data-skip-approval': '' }}
      foot={g.answerable && !g.ready ? <span id={g.wait} className="text-xs text-muted">{t('skip-approval-read-first')}</span> : undefined}
      actions={<>
        {lead}
        {/* Redesign PR 4: Deny, then Allow, as on every request; this Allow lets the agent act without asking, so it is the danger button
            (ADR-0102). The dwell and "Read it first" are unchanged. */}
        {/* Read-only without `onResolve` (a viewer's window, ADR-0106): no answers at all. */}
        {onResolve && <>
          <Button size="sm" disabled={g.denyOff} onClick={() => g.answer(false)}>{t('approval-deny')}</Button>
          <Button size="sm" variant="danger" className="relative overflow-hidden" disabled={g.allowOff} aria-describedby={!g.ready ? g.wait : undefined} onClick={() => g.answer(true)}>
            {t('approval-allow')}
            {g.answerable && <span aria-hidden className={`absolute inset-x-0 bottom-0 h-0.5 origin-left bg-current opacity-60 transition-transform ease-linear motion-reduce:transition-none ${g.ready ? 'scale-x-100' : 'scale-x-0'}`} style={{ transitionDuration: `${ATTENTION_DWELL_MS}ms` }} />}
          </Button>
        </>}
      </>}>
      <div className="flex flex-col gap-2 text-xs">
        <p className="m-0">{t('skip-approval-body')}</p>
        <KeyValue items={facts} />
        <p className="m-0 text-muted">{t('skip-approval-scope')}</p>
        {onResolve && !g.capable && <p role="note" className="m-0 text-muted">{t('skip-approval-other-window')}</p>}
      </div>
    </ApprovalRequest>
  );
}
