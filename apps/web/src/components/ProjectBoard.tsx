import { t } from '../i18n';
import type { BoardModel } from '../net/view';

export function ProjectBoard({ board: b }: { board: BoardModel }) {
  return (
    <div data-project-id={b.projectId} className="grid min-w-0 gap-1.5 rounded-s border border-line bg-surface-0 p-2.5">
      <strong className="break-words text-xs">{b.project}</strong>
      <span className="text-xs text-muted">{t('board-project-identity')}</span>
      <code dir="ltr" className="select-text break-all text-[0.6875rem] text-muted">{b.projectId}</code>
      {b.folders.length > 0 && <details className="min-w-0 text-xs text-muted">
        <summary className="cursor-pointer">{t('board-project-folders', { count: b.folders.length })}</summary>
        <ul className="m-0 grid list-none gap-2 p-0 pt-2">
          {b.folders.map(folder => <li key={`${folder.nodeId}\u0000${folder.path}`} className="grid min-w-0 gap-0.5">
            <span className="break-words">{folder.machine === folder.nodeId ? folder.machine : t('machine-name-id', { name: folder.machine, id: folder.nodeId })}</span>
            <code dir="ltr" className="select-text break-all">{folder.path}</code>
          </li>)}
        </ul>
      </details>}
      <ul className="m-0 grid list-none gap-1 p-0 text-[0.8125rem]">
        {b.tasks.map(task => <li key={task.id} className="flex items-baseline justify-between gap-2">
          <span className={`min-w-0 break-words ${task.status === 'done' ? 'text-muted line-through' : ''}`}>{task.title}</span>
          <span className="shrink-0 text-xs text-muted">{task.status === 'open' ? t('board-open') : task.status === 'done' ? t('board-done') : task.owner}</span>
        </li>)}
      </ul>
      {b.leases.length > 0 && <ul className="m-0 grid list-none gap-1 border-t border-line p-0 pt-1.5 text-xs text-muted">
        {b.leases.map(lease => <li key={`${lease.holder}\u0000${lease.path}`} className="flex justify-between gap-2"><span className="truncate font-mono">{lease.path}</span><span className="shrink-0">{lease.holder}</span></li>)}
      </ul>}
    </div>
  );
}
