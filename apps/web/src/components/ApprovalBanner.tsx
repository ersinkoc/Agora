import type { ApprovalModel } from '../net/view';
import { PlainRequest, useOwnGuard, type RequestAgent } from './ApprovalCard';
import { ReviewableRequest } from './ReviewRequest';
import type { StartChoice } from './StartApproval';
import type { AnswerGuard, AnswerResult } from './useAnswerGuard';

// What one agent waits on, in its pane: above the terminal, or (`pinned`) above a chat's composer. The same request card as the side
// panel's (redesign PR 4), which a pane narrower than 360 px draws in the row density with the buttons under the text. `guard` is the
// surface's (one dwell for every card it draws, T44); a banner drawn alone has its own. Every answer but a switch's goes through it.
export function ApprovalBanner({ approval: x, agent, guard, onResolve, pinned }: {
  approval: ApprovalModel;
  agent?: RequestAgent;
  guard?: AnswerGuard;
  onResolve?: (approval: ApprovalModel, approved: boolean, choice?: StartChoice, always?: boolean) => AnswerResult;
  pinned?: boolean;
}) {
  const own = useOwnGuard(x, agent);
  const g = guard ?? own;
  const send = (go: () => AnswerResult) => g.answer(x.id, go);
  // ADR-0057 and ADR-0090 Amendment A (redesign PR 5): a start, or a switch to act without asking, is a short card with Review, which
  // shows the whole request (read-only where it cannot be answered). A switch keeps its own guards (its Allow dwell and one-click lock,
  // and Deny always available: main-window.md), not the surface's.
  const card = x.start || x.skip || x.skipUnknown
    ? <ReviewableRequest approval={x} agent={agent} guard={g} alert narrowable onResolve={onResolve ? (ok, choice) => onResolve(x, ok, choice) : undefined} />
    : <PlainRequest approval={x} density="card" surface="banner" agent={agent} alert narrowable guard={g}
    onAnswer={onResolve ? (ok, always) => send(() => (always ? onResolve(x, true, undefined, true) : onResolve(x, ok))) : undefined} />;
  // Above a terminal it sits in the pane with a margin of its own; above a composer the composer's column holds it.
  return pinned ? card : <div className="px-2 pt-2 empty:hidden">{card}</div>;
}

// Every banner one surface draws (a pane, the full-size screen), under that surface's guard.
export function ApprovalBanners({ approvals, agent, guard, onResolve, pinned }: {
  approvals: readonly ApprovalModel[];
  agent?: RequestAgent;
  guard: AnswerGuard;
  onResolve?: (approval: ApprovalModel, approved: boolean, choice?: StartChoice, always?: boolean) => AnswerResult;
  pinned?: boolean;
}) {
  return <>{approvals.map((x) => <ApprovalBanner key={x.id} approval={x} agent={agent} guard={guard} onResolve={onResolve} pinned={pinned} />)}</>;
}
