import type { BoardTaskModel } from '../net/view';
import { visible as showHidden } from '../net/hidden';

// ADR-0095 step 1: the three columns of a project's board, over the statuses the mesh board already has (no message changes).
// Backlog = open (nobody holds it), Doing = claimed, Done = done. Pure: no I/O, no state.
export type ColumnId = 'backlog' | 'doing' | 'done';
export const COLUMN_ORDER: readonly ColumnId[] = ['backlog', 'doing', 'done'];
const STATUS_COLUMN: Record<BoardTaskModel['status'], ColumnId> = { open: 'backlog', claimed: 'doing', done: 'done' };

// A spammed board must not freeze the window: a column draws this many cards, more on request.
export const COLUMN_PAGE = 200;
export const TITLE_MAX = 120; // task text is untrusted; a card shows a short cut of it
export const OWNER_MAX = 48;
export const HELD_SHOWN = 3;

export interface Column { id: ColumnId; total: number; cards: BoardTaskModel[] }

export function columnsOf(tasks: readonly BoardTaskModel[], shown: Partial<Record<ColumnId, number>> = {}): Column[] {
  const all: Record<ColumnId, BoardTaskModel[]> = { backlog: [], doing: [], done: [] };
  for (const task of tasks) all[STATUS_COLUMN[task.status] ?? 'backlog'].push(task);
  return COLUMN_ORDER.map((id) => ({ id, total: all[id].length, cards: all[id].slice(0, Math.max(COLUMN_PAGE, shown[id] ?? 0)) }));
}

// Control characters (C0, C1) and bidirectional controls/isolates never reach the screen; the rest is shown as plain text.
// eslint-disable-next-line no-control-regex
const HIDDEN = /[\u0000-\u0008\u000B-\u001F\u007F-\u009F؜‎‏‪-‮⁦-⁩]/g;

// The text without those hidden characters, line breaks kept (a message read in full).
export const visible = (text: string): string => text.replace(HIDDEN, '');

// Plain text, one line, cut to `max` characters (by code point) with an ellipsis.
export function plain(text: string, max: number): string {
  const clean = showHidden(text).replace(/\s+/g, ' ').trim();
  const chars = Array.from(clean.length > max * 2 ? clean.slice(0, max * 2) : clean);
  return chars.length > max ? `${chars.slice(0, max - 1).join('')}…` : chars.join('');
}
