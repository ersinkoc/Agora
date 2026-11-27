import * as Dialog from '@radix-ui/react-dialog';
import { useRef, useState } from 'react';
import { t } from '../i18n';
import type { BoardModel, BoardTaskModel } from '../net/view';
import { COLUMN_PAGE, HELD_SHOWN, OWNER_MAX, TITLE_MAX, columnsOf, plain, type ColumnId } from './boardColumns';
import { belowAnotherDialog } from './dialogStack';
import { GiveButton, useCanGive } from './GiveTask';
import { giveTaskOf } from '../net/give';
import { Icon, type IconName } from './Icon';
import { EmptyState, StatusPill, Tag, type Tone } from './kit';
import { NoAskBadge } from './NoAskBadge';
import { BoardHandoffs } from './BoardHandoffs';
import { BoardReview } from './BoardReview';

// Each column's mark and hue (the kit's tones): Backlog is a neutral fact, Doing is working blue, Done is finished green. The word and the
// count say the same, so the colour is never the only sign.
const COLUMN_LOOK: Record<ColumnId, { tone: Tone; icon: IconName }> = { backlog: { tone: 'plain', icon: 'stack' }, doing: { tone: 'working', icon: 'play' }, review: { tone: 'plain', icon: 'files' }, done: { tone: 'finished', icon: 'check' } };

// `give`: the board this card is on, when its Backlog cards offer "Give to an agent" (ADR-0095 step 2).
function Card({ task, projectId, give }: { task: BoardTaskModel; projectId: string; give?: { projectId: string; project: string } }) {
  const held = task.held ?? [];
  return (
    <li className="kit-card kit-card--dense min-w-0 gap-1.5" data-task-id={task.id}>
      {/* Task text is untrusted: plain text only, cut short, never HTML or a link. */}
      <span dir="auto" className={`break-words ${task.status === 'done' ? 'text-muted' : 'font-medium'}`}>{plain(task.title, TITLE_MAX) || t('board-task-untitled')}</span>
      {task.status !== 'open' && task.owner && <span className="flex min-w-0 flex-wrap items-center gap-1.5 text-xs text-muted">
        <span className="min-w-0 break-words">{t('board-card-owner', { name: plain(task.owner, OWNER_MAX) })}</span>
        {task.status === 'claimed' && task.ownerState && <StatusPill state={task.ownerState} />}
        {task.status === 'claimed' && <NoAskBadge agent={{ skipsPermissions: task.ownerSkips }} />}
      </span>}
      {task.status === 'claimed' && held.length > 0 && <span className="grid min-w-0 gap-1 text-xs text-muted">
        <span>{t('board-card-holds', { count: held.length })}</span>
        {held.slice(0, HELD_SHOWN).map((path) => <code key={path} dir="ltr" className="truncate font-mono">{plain(path, 80)}</code>)}
        {held.length > HELD_SHOWN && <span>{t('board-card-more-paths', { count: held.length - HELD_SHOWN })}</span>}
      </span>}
      {task.review && task.taskId && <BoardReview key={task.review.revision} projectId={projectId} taskId={task.taskId} review={task.review} />}
      {task.handoffs && <BoardHandoffs deliveries={task.handoffs} />}
      {give && task.status === 'open' && <GiveButton task={giveTaskOf(give, task)} />}
    </li>
  );
}

export function BoardColumns({ board }: { board: BoardModel }) {
  const [shown, setShown] = useState<Partial<Record<ColumnId, number>>>({});
  const columns = columnsOf(board.tasks, shown, board.reviewEnabled);
  const canGive = useCanGive(board.projectId);
  if (board.tasks.length === 0) return <div className="grid min-h-0 flex-1 place-content-center bg-surface-0 p-3"><EmptyState icon="board" title={t('board-view-empty-title')} line={t('board-view-empty-hint')} /></div>;
  // The cards are kit cards on surface-1 over the board's surface-0, so they stand apart in both themes; a column is a quiet frame with its header.
  return (
    <div className={`grid min-h-0 flex-1 grid-cols-1 gap-3 overflow-auto bg-surface-0 p-3 ${columns.length === 4 ? 'md:grid-cols-2 xl:grid-cols-4' : 'md:grid-cols-3'}`}>
      {columns.map((col) => (
        <section key={col.id} aria-labelledby={`board-col-${col.id}`} data-column={col.id} className="grid min-w-0 content-start gap-2 rounded-m border border-line p-2">
          <h3 id={`board-col-${col.id}`} className={`m-0 flex items-center gap-2 px-1 pt-0.5 text-strong font-semibold normal-case tracking-normal text-ink kit-tone-${COLUMN_LOOK[col.id].tone}`}>
            <span aria-hidden className="kit-chip size-6!"><Icon name={COLUMN_LOOK[col.id].icon} className="size-3.5" /></span>
            <span className="min-w-0 flex-1 truncate">{t(`board-col-${col.id}`)}</span><Tag>{col.total}</Tag>
          </h3>
          {col.total === 0 ? <p className="m-0 rounded-s border border-dashed border-line px-3 py-4 text-center text-xs text-muted">{t(`board-col-${col.id}-empty`)}</p> : <ul className="m-0 grid list-none gap-2 p-0">
            {col.cards.map((task) => <Card key={task.id} task={task} projectId={board.projectId} give={canGive && col.id === 'backlog' ? board : undefined} />)}
          </ul>}
          {col.total > col.cards.length && <button type="button" className="kit-btn kit-btn--ghost kit-btn--sm" onClick={() => setShown({ ...shown, [col.id]: col.cards.length + COLUMN_PAGE })}>
            {t('board-show-more', { count: col.cards.length, total: col.total })}
          </button>}
        </section>
      ))}
    </div>
  );
}

// ADR-0095 step 1: a project's tasks in three columns, a deep view opened on demand. Step 2: a Backlog card offers "Give to an agent"
// (the dialog, net/give.ts and GiveTask.tsx, sends it); nothing else here changes a task.
export function BoardView({ board, onClose }: { board: BoardModel; onClose: () => void }) {
  const content = useRef<HTMLDivElement>(null);
  return (
    <Dialog.Root open onOpenChange={(o) => { if (!o) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-40 bg-surface-0/70 backdrop-blur-sm" />
        <Dialog.Content aria-describedby={undefined}
          // A dialog opened over the board (Give to an agent) takes Escape; the board never closes under it (dialogStack.ts).
          ref={content} onEscapeKeyDown={(e) => { if (belowAnotherDialog(content.current)) e.preventDefault(); }}
          onKeyDown={(e) => { if (!e.ctrlKey && !e.metaKey && !e.altKey && e.key.length === 1) e.stopPropagation(); }}
          className="fixed inset-2 z-50 flex flex-col overflow-hidden rounded-l border border-line bg-surface-raised shadow-pop md:inset-x-[3vw] md:inset-y-[4vh]">
          {/* The kit's dialog header: an icon chip, the title, and the icon close used by every other dialog. */}
          <header className="flex flex-none items-center gap-3 border-b border-line px-4 py-3">
            <span aria-hidden className="kit-chip kit-tone-accent"><Icon name="board" className="size-4" /></span>
            <Dialog.Title className="m-0 min-w-0 flex-1 truncate text-base font-semibold tracking-normal text-ink normal-case">{t('board-view-title', { name: plain(board.project, OWNER_MAX) })}</Dialog.Title>
            <Dialog.Close className="ide-iconbtn" aria-label={t('diff-close')} title={t('diff-close')}><Icon name="close" /></Dialog.Close>
          </header>
          <BoardColumns board={board} />
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
