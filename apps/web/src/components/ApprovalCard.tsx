import { useId, useLayoutEffect, useRef } from 'react';
import { t } from '../i18n';
import type { AgentTileModel } from '../model';
import { approvalName } from '../net/approvalText';
import { decidableInline } from '../net/fleet';
import { detailText } from '../net/tool';
import { visible, type ApprovalModel } from '../net/view';
import { kindOf } from './AgentTile';
import { AlwaysNote, AlwaysOnly, alwaysNoteId, type AlwaysSurface } from './AllowAlways';
import { ApprovalRequest, AnswerButtons, ASK_ICON, askKind, askTitle, type ApprovalDensity, PINNED_ANSWERS_REM, useBelowRem, useNarrowPane, usePinnedRow } from './ApprovalRequest';
import { Icon } from './Icon';
import { AgentAvatar, Button } from './kit';
import { ReviewableRequest } from './ReviewRequest';
import type { StartChoice } from './StartApproval';
import { requestShown, useAnswerGuard, type AnswerGuard, type AnswerResult } from './useAnswerGuard';

// The short form names the command or file; what the agent asked for, in full, stays one click (or key) away before Allow.
export function FullRequest({ detail }: { detail: string }) {
  if (detailText(detail) === detail) return null;
  return (
    <details className="text-xs text-muted">
      <summary className="cursor-pointer">{t('approval-full-request')}</summary>
      <pre dir="ltr" className="m-0 mt-1 max-h-48 overflow-auto font-mono break-all whitespace-pre-wrap">{detail}</pre>
    </details>
  );
}

export type RequestAgent = Pick<AgentTileModel, 'accent' | 'kind' | 'name'>;

// A plain request where it can be answered (the side panel, above a terminal, above a chat's composer): the ask whole, as one mono chip
// that wraps and is never cut, the agent's full text one click away when the short form leaves something out, then Deny, "Allow always"
// where offered, and Allow. In a pane (`narrowable`), below 360 px it is drawn in the row density with the buttons under the text; nothing
// of the ask is cut there either (the pane is where Review leads). `guard` decides when the answers send (T44, `useAnswerGuard`): off
// during the dwell, while an answer goes and once it went, and while the link is down (with the reason under the ask). Above a chat's
// composer in a short pane it is one row (#1082, `usePinnedRow`): the ask whole with Deny and Allow where `decidableInline` allows and the
// pane is wide enough for them beside it, else the title and Review, which opens the whole card in place.
export function PlainRequest({ approval: x, density, surface, agent, alert, busy, narrowable, guard, onAnswer, onOpen }: {
  approval: ApprovalModel;
  density: ApprovalDensity;
  surface: AlwaysSurface;
  agent?: RequestAgent;
  alert?: boolean;
  busy?: boolean;
  narrowable?: boolean;
  guard: AnswerGuard;
  onAnswer?: (approved: boolean, always?: boolean) => void;
  onOpen?: () => void;
}) {
  const host = useRef<HTMLDivElement>(null);
  const narrow = useNarrowPane(host, narrowable, guard.restart);
  const pin = usePinnedRow(guard.restart);
  const roomy = !useBelowRem(host, PINNED_ANSWERS_REM, 'width', pin.short);
  // The row's two forms put the buttons in different places: a change between them (the pane getting wider or narrower) restarts the dwell.
  const inline = pin.row && roomy && !!onAnswer && decidableInline(x);
  const drawnInline = useRef(inline);
  useLayoutEffect(() => { if (drawnInline.current !== inline) { drawnInline.current = inline; guard.restart(); } }, [inline, guard.restart]);
  const name = agent?.name || approvalName(x);
  const d = x.detail ? detailText(x.detail) : '';
  const ask = visible(d ? `${x.action} · ${d}` : x.action);
  const why = `approval-stale-${useId()}`;
  const sent = guard.status(x.id) === 'sent';
  const off = { disabled: busy || guard.stale, 'aria-disabled': !guard.canAnswer(x.id) || undefined, 'aria-describedby': guard.stale ? why : undefined };
  const avatar = agent ? <AgentAvatar accent={agent.accent || 'var(--agent-slate)'} kind={kindOf({ kind: agent.kind, name })} size="s" /> : undefined;
  const where = x.machineName?.trim() ? <bdi>{visible(x.machineName)}</bdi> : undefined;
  if (pin.row) {
    // The button goes with the row: the card that opens takes the focus, so it is not lost to the page.
    const open = () => { pin.open(); if (typeof requestAnimationFrame === 'function') requestAnimationFrame(() => host.current?.focus()); };
    return (
      <div ref={host} tabIndex={-1} className="approval-req-host outline-none">
        <ApprovalRequest density="row" alert={alert} icon={ASK_ICON[askKind(x)]} avatar={avatar} title={askTitle(x, name)} where={where}
          className={inline ? undefined : 'approval-req--bar'} attrs={{ 'data-pinned-row': inline ? 'answer' : 'review' }}
          actions={inline
            ? <AnswerButtons approval={x} size="sm" off={off} onDeny={() => onAnswer?.(false)} onAllow={() => onAnswer?.(true)} />
            : <Button size="sm" variant="primary" aria-expanded={false} aria-label={t('approval-review-for', { name })} onClick={open}>{t('approval-review')}</Button>}>
          {inline && <p className="approval-req__what"><span className="approval-req__cmd" dir="auto">{ask}</span></p>}
          {inline && guard.stale && <p id={why} className="m-0 text-xs text-muted">{t('fleet-stale-no-answer')}</p>}
          {onAnswer && sent && <p role="status" className="m-0 text-xs text-muted">{t('approval-sent')}</p>}
        </ApprovalRequest>
      </div>
    );
  }
  return (
    <div ref={host} tabIndex={pin.short ? -1 : undefined} className={`approval-req-host${pin.short ? ' outline-none' : ''}`}>
      <ApprovalRequest density={narrow ? 'row' : density} stacked={narrow} alert={alert} icon={ASK_ICON[askKind(x)]}
        avatar={avatar}
        title={askTitle(x, name)}
        where={where}
        titleEnd={onOpen && <button type="button" className="ide-iconbtn approval-req__open" onClick={onOpen} aria-label={t('approval-open-agent', { name })} title={t('approval-open-agent', { name })}><Icon name="chevron" className="rtl:rotate-180" /></button>}
        actions={onAnswer && <AnswerButtons approval={x} size="sm" off={off} alwaysNote={alwaysNoteId(x, surface)}
          onDeny={() => onAnswer(false)} onAllow={() => onAnswer(true)} onAlways={() => onAnswer(true, true)} />}
        foot={x.always ? <AlwaysNote approval={x} surface={surface} /> : undefined}>
        <p className="approval-req__what"><span className="approval-req__cmd" dir="auto">{ask}</span></p>
        {x.detail && <FullRequest detail={x.detail} />}
        {/* ADR-0101: "Allow always" only when the request offers it; in the place of Allow (the primary) when it offers nothing else. */}
        <AlwaysOnly approval={x} className="mt-1" />
        {onAnswer && guard.stale && <p id={why} className="m-0 text-xs text-muted">{t('fleet-stale-no-answer')}</p>}
        {onAnswer && sent && <p role="status" className="m-0 text-xs text-muted">{t('approval-sent')}</p>}
      </ApprovalRequest>
    </div>
  );
}

// A guard of its own for a request drawn alone (no surface passed one): its dwell restarts when what it shows changes.
export function useOwnGuard(x: ApprovalModel, agent?: RequestAgent): AnswerGuard {
  return useAnswerGuard([x.id], (status) => JSON.stringify([requestShown(x, agent), status(x.id)]));
}

// The side panel's card for one request. `onOpen` opens the agent that asks (the panel lists no separate row for it). `guard` is the side
// panel's (one dwell for its whole list); a card drawn alone has its own. Every answer but a switch's goes through it. Without `onResolve`
// (a viewer's window, ADR-0106: the hub would refuse the answer) the card is read-only: the whole ask, and no answers.
export function ApprovalCard({ approval, busy, agent, guard, onResolve, onOpen }: {
  approval: ApprovalModel;
  busy?: boolean;
  agent?: RequestAgent;
  guard?: AnswerGuard;
  onResolve?: (approved: boolean, choice?: StartChoice, always?: boolean) => AnswerResult;
  onOpen?: () => void;
}) {
  const own = useOwnGuard(approval, agent);
  const g = guard ?? own;
  // ADR-0057 and ADR-0090 Amendment A (redesign PR 5): a request to start an agent, or to let one act without asking, is a short card with
  // Review, which shows everything it asks for before Allow. A start's answers go through this list's guard; a switch keeps its own
  // guards (its Allow dwell and one-click lock, and Deny always available: main-window.md), not this list's.
  if (approval.start || approval.skip || approval.skipUnknown) return <ReviewableRequest approval={approval} agent={agent} guard={g} busy={busy} onOpen={onOpen} onResolve={onResolve} />;
  return <PlainRequest approval={approval} density="card" surface="card" agent={agent} busy={busy} guard={g} onOpen={onOpen}
    onAnswer={onResolve ? (ok, always) => g.answer(approval.id, () => (always ? onResolve(true, undefined, true) : onResolve(ok))) : undefined} />;
}
