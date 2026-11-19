import type { TranscriptItem } from './transcript';
import type { ChatLine } from './chat';
import { toolView, type ToolAction } from './tool';

// How a conversation is laid out on screen, as data: which items form one agent turn, how a tool step is toned, how far a plan is.
// Pure, so it is tested without a page.

/** An agent's turn: what it said and did between two things a person said (or a day change). */
export type TurnItem = Exclude<TranscriptItem, { kind: 'date' } | { kind: 'user' }>;
export type Row = Exclude<TranscriptItem, TurnItem> | { kind: 'turn'; id: string; items: TurnItem[]; atMs: number };

/** When an item happened, when it says (0 when it does not). */
export function atOf(item: TranscriptItem): number {
  switch (item.kind) {
    case 'date': case 'reply': return item.atMs;
    case 'steps': return item.lines.find((l) => l.atMs > 0)?.atMs ?? 0;
    default: return item.line.atMs;
  }
}

/** Consecutive agent items (replies, steps, plans, diffs, notices) become one turn; a person's message or a date ends it. */
export function groupTurns(items: readonly TranscriptItem[]): Row[] {
  const out: Row[] = [];
  for (const item of items) {
    if (item.kind === 'date' || item.kind === 'user') { out.push(item); continue; }
    const prev = out[out.length - 1];
    if (prev?.kind === 'turn') {
      prev.items.push(item);
      if (prev.atMs === 0) prev.atMs = atOf(item);
    } else out.push({ kind: 'turn', id: `t${item.id}`, items: [item], atMs: atOf(item) });
  }
  return out;
}

/** The tint of a step's icon: a change takes the accent, a command the "needs you" colour (it acts on the machine), the rest stay quiet. */
export type StepTone = 'edit' | 'run' | 'plain';
export function stepTone(action: ToolAction): StepTone {
  return action === 'edit' || action === 'delete' || action === 'move' ? 'edit' : action === 'run' ? 'run' : 'plain';
}

/** A plan step's progress, when the node reports one. Today it does not (the steps are text only), so no step is ever drawn done. */
export type StepStatus = 'pending' | 'in_progress' | 'completed';
export interface PlanProgress { done: number; total: number; percent: number }

/** How far a plan is. `undefined` when no step has a status: a plan without status never shows progress. */
export function planProgress(status: readonly (StepStatus | undefined)[]): PlanProgress | undefined {
  if (status.length === 0 || status.every((s) => s === undefined)) return undefined;
  const done = status.filter((s) => s === 'completed').length;
  return { done, total: status.length, percent: Math.round((done / status.length) * 100) };
}

/** Whether a tool's subject (a path the agent gave, often absolute) names the file a diff entry is about. */
const samePath = (subject: string, path: string): boolean => {
  const a = subject.replace(/\\/g, '/');
  const b = path.replace(/\\/g, '/');
  if (!a || !b) return false;
  if (a === b) return true;
  // One may be the other with a folder prefix (an absolute path against the repository path), but only when the shorter one has a folder
  // of its own: a bare name ("x.rs") names only that exact path, never "other/dir/x.rs".
  const [short, long] = a.length <= b.length ? [a, b] : [b, a];
  return short.includes('/') && long.endsWith(`/${short}`);
};

/**
 * An edit step and the diff the agent sent for the same file are one thing to read: the diff opens inside the step's row. Each diff
 * attaches to the earliest earlier edit step of its turn that names the same file and has none yet (two edits of one file get their diffs in order); a diff that matches no step stays
 * its own item. Returns the items still to draw and, per step entry id, the diff entries that belong to it.
 */
export function pairDiffs(items: readonly TurnItem[]): { items: TurnItem[]; attached: Map<number, ChatLine> } {
  const attached = new Map<number, ChatLine>();
  const kept: TurnItem[] = [];
  const edits: { id: number; subject: string }[] = [];
  for (const item of items) {
    if (item.kind === 'steps') {
      for (const l of item.lines) {
        const v = toolView(l.text);
        if (v.action === 'edit' && v.subject) edits.push({ id: l.id, subject: v.subject });
      }
    }
    if (item.kind === 'diff' && item.line.path) {
      const hit = edits.find((e) => !attached.has(e.id) && samePath(e.subject, item.line.path));
      if (hit) { attached.set(hit.id, item.line); continue; }
    }
    kept.push(item);
  }
  return { items: kept, attached };
}

/** Where an agent's pending requests are drawn: pinned above a chat composer, or (a terminal agent, or a closed chat agent that has no
 *  composer) as a bar under the pane's header. A request must always be drawn somewhere. */
export const asksUnderHeader = (agent: { terminal?: boolean; closed?: boolean }): boolean => agent.terminal !== false || !!agent.closed;

/** The first letter of an agent's name, for its avatar tile ("claude · amber" is C). */
export function initial(name: string): string {
  const c = [...name.trim()][0];
  return c ? c.toLocaleUpperCase() : '?';
}
