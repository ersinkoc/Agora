import * as Dialog from '@radix-ui/react-dialog';
import { useCallback, useEffect, useState } from 'react';
import { t } from '../i18n';
import type { AgentTileModel } from '../model';
import { decodeText, formatSize, joinPath, type FileItem, type WorkspaceFiles } from '../net/files';

type Listing = FileItem[] | 'loading' | 'failed';

function Tree({ dir, depth, listings, open, load, toggle, pick, picked }: {
  dir: string; depth: number; listings: Record<string, Listing>; open: Set<string>;
  load: (dir: string) => void; toggle: (path: string) => void; pick: (path: string, size: number) => void; picked?: string;
}) {
  const list = listings[dir];
  useEffect(() => { if (list === undefined) load(dir); }, [dir, list, load]);
  if (list === undefined || list === 'loading') return <li className="muted px-2 py-1" style={{ paddingLeft: 8 + depth * 14 }}>…</li>;
  if (list === 'failed') return <li role="alert" className="px-2 py-1 text-xs text-needs-you" style={{ paddingLeft: 8 + depth * 14 }}>{t('files-failed')}</li>;
  return (
    <>
      {list.map((f) => {
        const path = joinPath(dir, f.name);
        return (
          <li key={path}>
            <button type="button" aria-expanded={f.isDir ? open.has(path) : undefined}
              className={`flex w-full items-center gap-1.5 rounded-s px-2 py-1 text-left font-mono text-[0.8125rem] hover:bg-surface-2 ${picked === path ? 'bg-surface-2 text-accent' : ''}`}
              style={{ paddingLeft: 8 + depth * 14 }} onClick={() => (f.isDir ? toggle(path) : pick(path, f.size))}>
              <span aria-hidden className="w-3 flex-none text-muted">{f.isDir ? (open.has(path) ? '▾' : '▸') : ''}</span>
              <span className="truncate">{f.name}</span>
            </button>
            {f.isDir && open.has(path) && (
              <ul className="m-0 list-none p-0"><Tree dir={path} depth={depth + 1} listings={listings} open={open} load={load} toggle={toggle} pick={pick} picked={picked} /></ul>
            )}
          </li>
        );
      })}
    </>
  );
}

// The files of one agent's workspace: a folder tree that opens lazily, and the text of the file you pick.
export function FilesDialog({ agent, files, onClose }: { agent: AgentTileModel; files: WorkspaceFiles; onClose: () => void }) {
  const [listings, setListings] = useState<Record<string, Listing>>({});
  const [open, setOpen] = useState<Set<string>>(new Set());
  const [picked, setPicked] = useState<{ path: string; size: number } | undefined>();
  const [content, setContent] = useState<string | 'loading' | 'binary' | 'failed' | undefined>();

  const load = useCallback((dir: string) => {
    setListings((l) => (l[dir] === undefined ? { ...l, [dir]: 'loading' } : l));
    files.list(dir).then(
      (items) => setListings((l) => ({ ...l, [dir]: items })),
      () => setListings((l) => ({ ...l, [dir]: 'failed' })),
    );
  }, [files]);
  const toggle = (path: string) => setOpen((o) => { const n = new Set(o); if (n.has(path)) n.delete(path); else n.add(path); return n; });
  const pick = (path: string, size: number) => {
    setPicked({ path, size });
    setContent('loading');
    files.read(path).then(
      (b) => setContent(decodeText(b) ?? 'binary'),
      () => setContent('failed'),
    );
  };

  return (
    <Dialog.Root open onOpenChange={(o) => { if (!o) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-40 bg-surface-0/70 backdrop-blur-sm" />
        <Dialog.Content aria-describedby={undefined} className="fixed inset-4 z-50 flex flex-col overflow-hidden rounded-l border border-line bg-surface-1 shadow-card md:inset-x-[8vw] md:inset-y-[6vh]">
          <header className="flex flex-none items-center justify-between gap-3 border-b border-line px-4 py-3">
            <div className="min-w-0">
              <Dialog.Title className="m-0 truncate text-base font-semibold tracking-normal text-ink normal-case">{t('files-title', { name: agent.name })}</Dialog.Title>
              <p className="muted m-0 truncate">{agent.machine} · {agent.workspace}</p>
            </div>
            <Dialog.Close className="button">{t('diff-close')}</Dialog.Close>
          </header>
          <div className="grid min-h-0 flex-1 grid-cols-1 md:grid-cols-[18rem_minmax(0,1fr)]">
            <ul className="m-0 max-h-48 list-none overflow-auto border-b border-line p-2 md:max-h-none md:border-r md:border-b-0" aria-label={t('files-tree')}>
              <Tree dir="" depth={0} listings={listings} open={open} load={load} toggle={toggle} pick={pick} picked={picked?.path} />
            </ul>
            <div className="min-h-0 overflow-auto">
              {!picked ? <p className="muted p-4">{t('files-pick')}</p> : (
                <>
                  <p className="m-0 flex items-center justify-between gap-3 border-b border-line bg-surface-2 px-4 py-2 font-mono text-[0.8125rem] font-semibold"><span className="truncate">{picked.path}</span><span className="flex-none text-xs font-normal text-muted">{formatSize(picked.size)}</span></p>
                  {content === 'loading' ? <p className="muted p-4">{t('start-loading')}</p>
                    : content === 'binary' ? <p className="muted p-4">{t('files-binary')}</p>
                    : content === 'failed' ? <p role="alert" className="p-4">{t('files-too-large')}</p>
                    : <pre className="m-0 p-4 font-mono text-xs leading-5 whitespace-pre">{content}</pre>}
                </>
              )}
            </div>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
