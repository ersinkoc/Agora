import type { ApprovalModel } from '../net/view';
import { PlainRequest, useOwnGuard, type RequestAgent } from './ApprovalCard';
import { StartApproval, type StartChoice } from './StartApproval';
import { SkipApproval } from './SkipApproval';
import type { AnswerGuard, AnswerResult } from './useAnswerGuard';

// What one agent waits on, in its pane: above the terminal, or (`pinned`) above a chat's composer. The same request card as the side
// panel's (redesign PR 4), which a pane narrower than 360 px draws in the row density with the buttons under the text. `guard` is the
// surface's (one dwell for every card it draws, T44); a banner drawn alone has its own. Every answer goes through it.
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
  // ADR-0057: a request to start an agent shows everything it asks for (read-only where it cannot be answered).
  if (x.start) return <StartApproval approval={x} start={x.start} busy={!onResolve || !g.canAnswer(x.id)} onResolve={(ok, choice) => onResolve && send(() => onResolve(x, ok, choice))} />;
  if (x.skip || x.skipUnknown) return <SkipApproval approval={x} busy={!g.canAnswer(x.id)} onResolve={onResolve ? (ok) => send(() => onResolve(x, ok)) : undefined} />;
  const card = <PlainRequest approval={x} density="card" surface="banner" agent={agent} alert narrowable guard={g}
    onAnswer={onResolve ? (ok, always) => send(() => (always ? onResolve(x, true, undefined, true) : onResolve(x, ok))) : undefined} />;
  // Above a terminal it sits in the pane with a margin of its own; above a composer the composer's column holds it.
  return pinned ? card : <div className="px-2 pt-2">{card}</div>;
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
