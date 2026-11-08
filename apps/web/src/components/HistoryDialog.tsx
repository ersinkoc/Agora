import * as Dialog from '@radix-ui/react-dialog';
import { useEffect, useState } from 'react';
import { t } from '../i18n';
import type { AgentTileModel } from '../model';
import type { CommitItem } from '../net/history';

// The commits already made in one agent's workspace, newest first. Subjects and names are text a person or model wrote: plain text only.
export function HistoryDialog({ agent, load, onClose }: { agent: AgentTileModel; load: (a: AgentTileModel) => Promise<CommitItem[]>; onClose: () => void }) {
  const [commits, setCommits] = useState<CommitItem[] | 'loading' | 'failed'>('loading');
  useEffect(() => {
    let live = true;
    load(agent).then((c) => { if (live) setCommits(c); }, () => { if (live) setCommits('failed'); });
    return () => { live = false; };
  }, [agent, load]);
  return (
    <Dialog.Root open onOpenChange={(open) => { if (!open) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-40 bg-surface-0/70 backdrop-blur-sm" />
        <Dialog.Content aria-describedby={undefined} className="fixed inset-4 z-50 flex flex-col overflow-hidden rounded-l border border-line bg-surface-1 shadow-card md:inset-x-[14vw] md:inset-y-[8vh]">
          <header className="flex flex-none items-center justify-between gap-3 border-b border-line px-4 py-3">
            <div className="min-w-0">
              <Dialog.Title className="m-0 truncate text-base font-semibold tracking-normal text-ink normal-case">{t('history-title', { name: agent.name })}</Dialog.Title>
              <p className="muted m-0 truncate">{agent.machine} · {agent.workspace}</p>
            </div>
            <Dialog.Close className="button">{t('diff-close')}</Dialog.Close>
          </header>
          <div className="min-h-0 flex-1 overflow-auto p-4">
            {commits === 'loading' ? <p className="muted">{t('start-loading')}</p>
              : commits === 'failed' ? <p role="alert">{t('history-failed')}</p>
              : commits.length === 0 ? <p className="muted">{t('history-none')}</p>
              : (
                <ol className="m-0 grid list-none gap-2 p-0">
                  {commits.map((c) => (
                    <li key={c.id} className="rounded-m border border-line px-3 py-2">
                      <div className="break-words font-medium">{c.subject || t('history-no-subject')}</div>
                      <div className="muted mt-0.5"><span className="font-mono">{c.id.slice(0, 8)}</span> · {c.author} · {new Date(c.atMs).toLocaleString()}</div>
                    </li>
                  ))}
                </ol>
              )}
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
