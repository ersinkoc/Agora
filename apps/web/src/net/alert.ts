// Tells the user about approvals while the tab is in the background: a count in the tab title and,
// when the user allowed it, a system notification for each approval that is new.
import type { ApprovalModel } from './view';

// Approvals in `now` that were not in `seen`.
export function freshApprovals(seen: ReadonlySet<string>, now: readonly ApprovalModel[]): ApprovalModel[] {
  return now.filter((a) => !seen.has(a.id));
}

// The tab title with the waiting count in front: `(2) Agora`.
export function titleWithCount(base: string, waiting: number): string {
  return waiting > 0 ? `(${waiting}) ${base}` : base;
}
