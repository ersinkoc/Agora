import { t } from '../i18n';
import { Icon } from './Icon';
import { Tag } from './kit';
import { BoardNotices } from './BoardNotices';
import { BoardPickup } from './BoardPickup';
import { sharesName } from '../model';
import type { BoardModel } from '../net/view';
import { visible } from '../net/hidden';

export function ProjectBoard({ board: b, machines = [], onOpen }: {
  board: BoardModel;
  machines?: readonly { id: string; name: string }[];
  onOpen: (trigger: HTMLButtonElement) => void;
}) {
  return (
    <div data-project-id={b.projectId} className="grid min-w-0 gap-1.5 rounded-m border border-line bg-surface-0 p-3">
      <strong className="break-words text-xs">{visible(b.project)}</strong>
      <span className="text-xs text-muted">{t('board-project-identity')}</span>
      <code dir="ltr" className="block min-w-0 select-text truncate text-xs text-muted" title={visible(b.projectId)}>{visible(b.projectId)}</code>
      {b.folders.length > 0 && <details className="min-w-0 text-xs text-muted">
        <summary className="cursor-pointer">{t('board-project-folders', { count: b.folders.length })}</summary>
        <ul className="m-0 grid list-none gap-2 p-0 pt-2">
          {b.folders.map(folder => <li key={`${folder.nodeId}\u0000${folder.path}`} className="grid min-w-0 gap-0.5">
            <span className="break-words">{sharesName({ id: folder.nodeId, name: folder.machine }, machines) ? t('machine-name-id', { name: visible(folder.machine), id: visible(folder.nodeId) }) : visible(folder.machine)}</span>
            <code dir="ltr" className="select-text break-all">{visible(folder.path)}</code>
          </li>)}
        </ul>
      </details>}
      {b.reviewEnabled && <BoardNotices projectId={b.projectId} />}
      {b.reviewEnabled && <BoardPickup projectId={b.projectId} />}
      {b.tasks.length > 0 && <button type="button" className="kit-btn kit-btn--sm w-fit" aria-haspopup="dialog" onClick={e => onOpen(e.currentTarget)}><Icon name="board" className="size-3.5" />{t('board-open-view', { count: b.tasks.length })}</button>}
      <ul className="m-0 grid list-none gap-1 p-0 text-[0.8125rem]">
        {b.tasks.map(task => <li key={task.id} className="flex items-baseline justify-between gap-2">
          <span className={`min-w-0 line-clamp-2 break-words ${task.status === 'done' ? 'text-muted line-through' : ''}`} title={visible(task.title)}>{visible(task.title)}</span>
          <span className="shrink-0"><Tag>{task.status === 'open' ? t('board-open') : task.status === 'done' ? t('board-done') : task.status === 'review' ? t('board-col-review') : visible(task.owner)}</Tag></span>
        </li>)}
      </ul>
      {b.leases.length > 0 && <ul className="m-0 grid list-none gap-1 border-t border-line p-0 pt-1.5 text-xs text-muted">
        {b.leases.map(lease => <li key={`${lease.holder}\u0000${lease.path}`} className="flex justify-between gap-2"><span className="truncate font-mono">{visible(lease.path)}</span><span className="shrink-0">{visible(lease.holder)}</span></li>)}
      </ul>}
    </div>
  );
}
