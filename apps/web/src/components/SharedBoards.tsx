import { createContext, useContext, useId, useMemo, useState } from 'react';
import { getLanguage, t } from '../i18n';
import type { BoardModel, BoardTaskModel } from '../net/view';
import { BoardView } from './BoardView';
import { Icon } from './Icon';
import { ProjectBoard } from './ProjectBoard';
import { OWNER_MAX, plain, TITLE_MAX } from './boardColumns';

export const ShowAcceptedTasks = createContext(false);

const NOTICE_LIMIT = 50;
const compareId = (a: string, b: string) => (a < b ? -1 : a === b ? 0 : 1);

interface AcceptedTaskNotice {
  board: BoardModel;
  task: BoardTaskModel;
  revision: number;
  decidedAtMs: number;
}

export function acceptedTaskNotices(boards: readonly BoardModel[]): AcceptedTaskNotice[] {
  const notices: AcceptedTaskNotice[] = [];
  for (const board of boards) {
    if (!board.reviewEnabled) continue;
    for (const task of board.tasks) {
      const review = task.review;
      if (task.status === 'done' && review?.state === 'accepted' && review.decidedAtMs !== undefined) {
        notices.push({ board, task, revision: review.revision, decidedAtMs: review.decidedAtMs });
      }
    }
  }
  notices.sort((a, b) => b.decidedAtMs - a.decidedAtMs
    || compareId(a.board.projectId, b.board.projectId) || compareId(a.task.id, b.task.id));
  return notices.slice(0, NOTICE_LIMIT);
}

export function SharedBoards({ boards }: { boards: readonly BoardModel[] }) {
  const owner = useContext(ShowAcceptedTasks);
  const titleId = useId();
  const notices = useMemo(() => owner ? acceptedTaskNotices(boards) : [], [boards, owner]);
  const [opened, setOpened] = useState<{ projectId: string; trigger: HTMLButtonElement }>();
  const active = opened && boards.find(board => board.projectId === opened.projectId);
  const open = (projectId: string, trigger: HTMLButtonElement) => setOpened({ projectId, trigger });
  const dateFormat = notices.length > 0 ? new Intl.DateTimeFormat(getLanguage(), { dateStyle: 'medium', timeStyle: 'short' }) : undefined;

  // A machine's ID is needed only when more than one machine shares its name.
  const machines = [...new Map(boards.flatMap(b => b.folders).map(f => [f.nodeId, { id: f.nodeId, name: f.machine }] as const)).values()];
  return <div className="grid gap-2">
    {dateFormat && <section aria-labelledby={titleId} className="min-w-0 rounded-m border border-line bg-surface-0">
      <h3 id={titleId} className="m-0 px-3 py-2 text-xs font-semibold normal-case tracking-normal text-ink">{t('board-recently-accepted')}</h3>
      <ul className="m-0 max-h-64 list-none overflow-auto px-1 pb-1">
        {notices.map(({ board, task, revision, decidedAtMs }) => {
          const date = new Date(decidedAtMs);
          const title = plain(task.title, TITLE_MAX) || t('board-task-untitled');
          return <li key={`${task.id}/${revision}`}>
            <button type="button" aria-haspopup="dialog" title={t('board-accepted-open')}
              className="ide-row min-h-11 w-full items-center justify-between gap-2 px-2 py-1.5 text-start"
              onClick={e => open(board.projectId, e.currentTarget)}>
              <span className="grid min-w-0 gap-0.5">
                <span dir="auto" className="truncate text-xs font-medium text-ink">{title}</span>
                <span className="flex min-w-0 flex-wrap items-baseline gap-x-2 text-xs text-muted">
                  <span dir="auto" className="truncate">{plain(board.project, OWNER_MAX)}</span>
                  <time dateTime={date.toISOString()}>{dateFormat.format(date)}</time>
                </span>
              </span>
              <Icon name="board" className="size-4 shrink-0 text-muted" />
            </button>
          </li>;
        })}
      </ul>
    </section>}
    {boards.map(board => <ProjectBoard key={board.projectId} board={board} machines={machines} onOpen={trigger => open(board.projectId, trigger)} />)}
    {active && <BoardView key={active.projectId} board={active} onClose={() => setOpened(undefined)} returnFocus={opened?.trigger} />}
  </div>;
}
