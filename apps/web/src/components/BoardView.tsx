import * as Dialog from '@radix-ui/react-dialog';
import { useState } from 'react';
import { t } from '../i18n';
import type { BoardModel, BoardTaskModel } from '../net/view';
import { COLUMN_PAGE, HELD_SHOWN, OWNER_MAX, TITLE_MAX, columnsOf, plain, type ColumnId } from './boardColumns';
import { EmptyState } from './EmptyState';
import { NoAskBadge } from './NoAskBadge';
import { StateBadge } from './StateBadge';

function Card({ task }: { task: BoardTaskModel }) {
  const held = task.held ?? [];
  return (
    <li className="grid min-w-0 gap-1 rounded-s border border-line bg-surface-0 p-2.5 text-[0.8125rem]" data-task-id={task.id}>
      {/* Task text is untrusted: plain text only, cut short, never HTML or a link. */}
      <span dir="auto" className={`break-words ${task.status === 'done' ? 'text-muted' : ''}`}>{plain(task.title, TITLE_MAX) || t('board-task-untitled')}</span>
      {task.status !== 'open' && task.owner && <span className="flex min-w-0 flex-wrap items-center gap-1.5 text-xs text-muted">
        <span className="min-w-0 break-words">{t('board-card-owner', { name: plain(task.owner, OWNER_MAX) })}</span>
        {task.status === 'claimed' && task.ownerState && <StateBadge state={task.ownerState} />}
        {task.status === 'claimed' && <NoAskBadge agent={{ skipsPermissions: task.ownerSkips }} compact />}
      </span>}
      {task.status === 'claimed' && held.length > 0 && <span className="grid min-w-0 gap-0.5 text-xs text-muted">
        <span>{t('board-card-holds', { count: held.length })}</span>
        {held.slice(0, HELD_SHOWN).map((path) => <code key={path} dir="ltr" className="truncate">{plain(path, 80)}</code>)}
        {held.length > HELD_SHOWN && <span>{t('board-card-more-paths', { count: held.length - HELD_SHOWN })}</span>}
      </span>}
    </li>
  );
}

export function BoardColumns({ board }: { board: BoardModel }) {
  const [shown, setShown] = useState<Partial<Record<ColumnId, number>>>({});
  const columns = columnsOf(board.tasks, shown);
  if (board.tasks.length === 0) return <EmptyState icon="check" title={t('board-view-empty-title')} hint={t('board-view-empty-hint')} />;
  return (
    <div className="grid min-h-0 flex-1 grid-cols-1 gap-3 overflow-auto p-3 md:grid-cols-3">
      {columns.map((col) => (
        <section key={col.id} aria-labelledby={`board-col-${col.id}`} data-column={col.id} className="grid min-w-0 content-start gap-2 rounded-m border border-line bg-surface-1 p-2.5">
          <h3 id={`board-col-${col.id}`} className="m-0 flex items-baseline justify-between gap-2 text-sm font-semibold normal-case tracking-normal text-ink">
            <span>{t(`board-col-${col.id}`)}</span><span className="text-xs font-normal text-muted">{col.total}</span>
          </h3>
          {col.total === 0 ? <p className="muted m-0 text-xs">{t(`board-col-${col.id}-empty`)}</p> : <ul className="m-0 grid list-none gap-2 p-0">
            {col.cards.map((task) => <Card key={task.id} task={task} />)}
          </ul>}
          {col.total > col.cards.length && <button type="button" className="button" onClick={() => setShown({ ...shown, [col.id]: col.cards.length + COLUMN_PAGE })}>
            {t('board-show-more', { count: col.cards.length, total: col.total })}
          </button>}
        </section>
      ))}
    </div>
  );
}

// ADR-0095 step 1: a project's tasks in three columns, a deep view opened on demand. Read-only: nothing here changes a task.
export function BoardView({ board, onClose }: { board: BoardModel; onClose: () => void }) {
  return (
    <Dialog.Root open onOpenChange={(o) => { if (!o) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-40 bg-surface-0/70 backdrop-blur-sm" />
        <Dialog.Content aria-describedby={undefined} onKeyDown={(e) => { if (!e.ctrlKey && !e.metaKey && !e.altKey && e.key.length === 1) e.stopPropagation(); }}
          className="fixed inset-2 z-50 flex flex-col overflow-hidden rounded-l border border-line bg-surface-1 shadow-card md:inset-x-[3vw] md:inset-y-[4vh]">
          <header className="flex flex-none items-center justify-between gap-3 border-b border-line px-4 py-2.5">
            <Dialog.Title className="m-0 min-w-0 truncate text-base font-semibold tracking-normal text-ink normal-case">{t('board-view-title', { name: plain(board.project, OWNER_MAX) })}</Dialog.Title>
            <Dialog.Close className="button">{t('diff-close')}</Dialog.Close>
          </header>
          <BoardColumns board={board} />
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
