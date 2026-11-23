import * as Dialog from '@radix-ui/react-dialog';
import { useId, useRef, useState, type ReactElement, type ReactNode } from 'react';
import { t } from '../i18n';
import { approvalName } from '../net/approvalText';
import { visible, type ApprovalModel } from '../net/view';
import { kindOf } from './AgentTile';
import type { RequestAgent } from './ApprovalCard';
import { ApprovalRequest, ASK_ICON, useNarrowPane, usePinnedRow } from './ApprovalRequest';
import { Icon } from './Icon';
import { AgentAvatar, Button } from './kit';
import { SkipApproval, SkipSummary, SkipUnknown, skipTitle, useSkipAnswer } from './SkipApproval';
import { StartApproval, StartSummary, startTitle, type StartChoice } from './StartApproval';
import { requestShown, ShareHolds, useAnswerGuard, type AnswerGuard, type AnswerResult } from './useAnswerGuard';

// Redesign PR 5: a request to start an agent (ADR-0057) or to let a running agent act without asking (ADR-0090 Amendment A), where it
// waits in chrome (the side panel, above a terminal, above a chat's composer): a card in the one request frame with a short summary (two
// lines at most) and Review, which opens the whole request in a dialog. Allow is only there, where all of it is shown (approvals.md:
// never decided on cut text; `decidableInline` never answers these inline). The card keeps Deny, which needs no reading:
// - a start's Deny goes through the surface's guard (T44: the dwell and one answer per request, `useAnswerGuard`);
// - a switch's Deny keeps the switch's own rules (no dwell; off while busy, held or answered: `useSkipAnswer`, main-window.md).
// Review's dialog has guards of its own, started when it opens, so a second click on Review never lands on Allow: a start's answers wait
// out a fresh dwell (its own `useAnswerGuard`), a switch's Deny and Allow its own dwell (`denyWaits`, #1070) and "Read it first": in a
// freshly opened Review, no answer goes before its open dwell, for a start or a switch alike. Every answer, on the card or in the
// dialog, holds the request in the same holds as the surface's guard (the window's: an answer anywhere holds it everywhere).
// #1070: Home's row and the attention dialog open this same dialog (`ReviewDialog`) straight away, so a start or a switch takes one Review
// wherever it is seen; they pass it the raw answer (never their own guard's, which would hold the request before the dialog's guard sends).
export type ReviewResolve = (approved: boolean, choice?: StartChoice) => AnswerResult;

export function ReviewableRequest(props: {
  approval: ApprovalModel;
  agent?: RequestAgent;
  guard: AnswerGuard;
  busy?: boolean;
  narrowable?: boolean;
  alert?: boolean;
  // Opens the agent that asks (the side panel lists no separate row for it).
  onOpen?: () => void;
  // Left out where it cannot be answered: Review still shows the whole request, with no answers.
  onResolve?: ReviewResolve;
}) {
  const x = props.approval;
  if (!x.start && !x.skip) return x.skipUnknown ? <SkipUnknown approval={x} /> : null;
  return <ShareHolds holds={props.guard.holds}><ReviewCard {...props} /></ShareHolds>;
}

function ReviewCard({ approval: x, agent, guard, busy, narrowable, alert, onOpen, onResolve }: Parameters<typeof ReviewableRequest>[0]) {
  const host = useRef<HTMLDivElement>(null);
  const narrow = useNarrowPane(host, narrowable, guard.restart);
  // #1082: above a chat's composer in a short pane, one row: the title and Review, which opens the same dialog (the summary and Deny are
  // read and answered there; Allow is only ever there).
  const { row } = usePinnedRow(guard.restart);
  const [open, setOpen] = useState(false);
  const why = `review-stale-${useId()}`;
  const name = agent?.name || approvalName(x);
  const sent = guard.status(x.id) === 'sent';
  const stale = !!onResolve && guard.stale;
  return (
    <div ref={host} className="approval-req-host">
      <ApprovalRequest density={narrow || row ? 'row' : 'card'} stacked={narrow && !row} alert={alert} icon={ASK_ICON[x.start ? 'start' : 'skip']}
        className={row ? 'approval-req--bar' : undefined} attrs={{ 'data-review-card': x.start ? 'start' : 'skip', ...(row ? { 'data-pinned-row': 'review' } : {}) }}
        avatar={agent ? <AgentAvatar accent={agent.accent || 'var(--agent-slate)'} kind={kindOf({ kind: agent.kind, name })} size="s" /> : undefined}
        title={x.start ? startTitle(x, x.start, false) : skipTitle(x)}
        where={x.machineName?.trim() ? <bdi>{visible(x.machineName)}</bdi> : undefined}
        titleEnd={onOpen && <button type="button" className="ide-iconbtn approval-req__open" onClick={onOpen} aria-label={t('approval-open-agent', { name })} title={t('approval-open-agent', { name })}><Icon name="chevron" className="rtl:rotate-180" /></button>}
        actions={<>
          {onResolve && !row && (x.start
            ? <Button size="sm" disabled={busy || guard.stale} aria-disabled={!guard.canAnswer(x.id) || undefined} aria-describedby={guard.stale ? why : undefined}
                onClick={() => guard.answer(x.id, () => onResolve(false, { grant: 0, nested: false }))}>{t('approval-deny')}</Button>
            : <SkipDeny approval={x} busy={busy || guard.stale} onResolve={onResolve} />)}
          <Button size="sm" variant="primary" aria-haspopup="dialog" aria-label={t('approval-review-for', { name })} onClick={() => setOpen(true)}>{t('approval-review')}</Button>
        </>}>
        {!row && (x.start ? <StartSummary start={x.start} /> : <SkipSummary approval={x} />)}
        {stale && !row && <p id={why} className="m-0 text-xs text-muted">{t('fleet-stale-no-answer')}</p>}
        {onResolve && sent && <p role="status" className="m-0 text-xs text-muted">{t('approval-sent')}</p>}
      </ApprovalRequest>
      {open && <ReviewDialog approval={x} agent={agent} busy={busy} stale={guard.stale} onResolve={onResolve} onClose={() => setOpen(false)} />}
    </div>
  );
}

// A switch's Deny on its short card: the switch's own guards (no dwell; one click, one decision; off while busy, held or answered).
function SkipDeny({ approval, busy, onResolve }: { approval: ApprovalModel; busy?: boolean; onResolve: ReviewResolve }) {
  const g = useSkipAnswer(approval, busy, (ok) => onResolve(ok));
  return <Button size="sm" disabled={g.denyOff} onClick={() => g.answer(false)}>{t('approval-deny')}</Button>;
}

// The whole request, in the full density, over the window. Nothing is focused for the person when it opens (the dialog itself is), and
// Escape, a click outside and Close only close it: never an answer. What it shows is the request itself, so a change of it starts the
// dwell again; a request that leaves the list takes its card, and this dialog, with it. Opened by a card, Home's row or the attention
// dialog (#1070): each draws it only while the request is listed, under its own holds (`ShareHolds`), and passes the raw answer.
export function ReviewDialog({ approval: x, agent, busy, stale, onResolve, onClose }: {
  approval: ApprovalModel;
  agent?: RequestAgent;
  busy?: boolean;
  stale: boolean;
  onResolve?: ReviewResolve;
  onClose: () => void;
}) {
  const content = useRef<HTMLDivElement>(null);
  const close = <Button size="sm" onClick={onClose}>{t('approval-review-close')}</Button>;
  const titleWrap = (head: ReactElement) => <Dialog.Title asChild>{head}</Dialog.Title>;
  return (
    <Dialog.Root open onOpenChange={(o) => { if (!o) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-40 bg-surface-0/70 backdrop-blur-sm" />
        {/* Its keys stay in it: Escape closes this dialog only (never the side panel it was opened from, whose React tree the portal
            shares), and a typed letter is never a window shortcut behind it. */}
        <Dialog.Content ref={content} tabIndex={-1} aria-describedby={undefined} data-review-dialog onOpenAutoFocus={(e) => { e.preventDefault(); content.current?.focus(); }}
          onKeyDown={(e) => { if (e.key === 'Escape' || (!e.ctrlKey && !e.metaKey && !e.altKey && e.key.length === 1)) e.stopPropagation(); }}
          className="attention-card review-card fixed left-1/2 top-1/2 z-50 flex max-h-[calc(100dvh-2rem)] w-[min(36rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 flex-col rounded-m shadow-pop outline-none">
          {x.start
            ? <ReviewStart approval={x} agent={agent} busy={busy} stale={stale} onResolve={onResolve} titleWrap={titleWrap} lead={close} />
            : <SkipApproval key={`${x.id}:${x.detail}`} approval={x} density="full" titleWrap={titleWrap} lead={close} denyWaits busy={busy || stale} onResolve={onResolve ? (ok) => onResolve(ok) : undefined} />}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

// A start in Review: its own dwell from the moment it opens (and after every change of the request, the person coming back, the link
// coming back, an answer settling anywhere), in the window's holds.
function ReviewStart({ approval: x, agent, busy, stale, onResolve, titleWrap, lead }: {
  approval: ApprovalModel;
  agent?: RequestAgent;
  busy?: boolean;
  stale: boolean;
  onResolve?: ReviewResolve;
  titleWrap: (head: ReactElement) => ReactNode;
  lead: ReactNode;
}) {
  const g = useAnswerGuard([x.id], (status) => JSON.stringify([requestShown(x, agent), x.start ?? null, status(x.id)]), stale);
  const start = x.start;
  if (!start) return null;
  const sent = g.status(x.id) === 'sent';
  return (
    <StartApproval key={`${x.id}:${x.detail}`} approval={x} start={start} density="full" titleWrap={titleWrap} lead={lead}
      busy={busy || !onResolve || !g.canAnswer(x.id)} onResolve={(ok, choice) => { if (onResolve) g.answer(x.id, () => onResolve(ok, choice)); }}
      foot={onResolve && (stale || sent) ? <p role={sent ? 'status' : undefined} className="m-0 text-xs text-muted">{t(sent ? 'approval-sent' : 'fleet-stale-no-answer')}</p> : undefined} />
  );
}
