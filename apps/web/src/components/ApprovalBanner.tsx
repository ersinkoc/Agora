import type { ApprovalModel } from '../net/view';
import { PlainRequest, type RequestAgent } from './ApprovalCard';
import { StartApproval, type StartChoice } from './StartApproval';
import { SkipApproval } from './SkipApproval';

// What one agent waits on, in its pane: above the terminal, or (`pinned`) above a chat's composer. The same request card as the side
// panel's (redesign PR 4), which a pane narrower than 360 px draws in the row density with the buttons under the text.
export function ApprovalBanner({ approval: x, agent, onResolve, pinned }: {
  approval: ApprovalModel;
  agent?: RequestAgent;
  onResolve?: (approval: ApprovalModel, approved: boolean, choice?: StartChoice, always?: boolean) => void;
  pinned?: boolean;
}) {
  // ADR-0057: a request to start an agent shows everything it asks for (read-only where it cannot be answered).
  if (x.start) return <StartApproval approval={x} start={x.start} busy={!onResolve} onResolve={(ok, choice) => onResolve?.(x, ok, choice)} />;
  if (x.skip || x.skipUnknown) return <SkipApproval approval={x} onResolve={onResolve ? (ok) => onResolve(x, ok) : undefined} />;
  const card = <PlainRequest approval={x} density="card" surface="banner" agent={agent} alert narrowable
    onAnswer={onResolve ? (ok, always) => (always ? onResolve(x, true, undefined, true) : onResolve(x, ok)) : undefined} />;
  // Above a terminal it sits in the pane with a margin of its own; above a composer the composer's column holds it.
  return pinned ? card : <div className="px-2 pt-2">{card}</div>;
}
