import * as Dialog from '@radix-ui/react-dialog';
import { useEffect, useState } from 'react';
import { t } from '../i18n';
import type { AgentTileModel } from '../model';
import { parseDiff, type DiffFile } from '../net/diff';

// What one agent changed in its workspace and has not committed: every file, its added and removed lines.
export function DiffDialog({ agent, load, onClose }: { agent: AgentTileModel; load: (a: AgentTileModel) => Promise<string>; onClose: () => void }) {
  const [files, setFiles] = useState<DiffFile[] | 'loading' | 'failed'>('loading');
  useEffect(() => {
    let live = true;
    load(agent).then((d) => { if (live) setFiles(parseDiff(d)); }, () => { if (live) setFiles('failed'); });
    return () => { live = false; };
  }, [agent, load]);
  return (
    <Dialog.Root open onOpenChange={(open) => { if (!open) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-40 bg-surface-0/70 backdrop-blur-sm" />
        <Dialog.Content aria-describedby={undefined} className="fixed inset-4 z-50 flex flex-col overflow-hidden rounded-l border border-line bg-surface-1 shadow-card md:inset-x-[10vw] md:inset-y-[6vh]">
          <header className="flex flex-none items-center justify-between gap-3 border-b border-line px-4 py-3">
            <div className="min-w-0">
              <Dialog.Title className="m-0 truncate text-base font-semibold tracking-normal text-ink normal-case">{t('diff-title', { name: agent.name })}</Dialog.Title>
              <p className="muted m-0 truncate">{agent.machine} · {agent.workspace}</p>
            </div>
            <Dialog.Close className="button">{t('diff-close')}</Dialog.Close>
          </header>
          <div className="min-h-0 flex-1 overflow-auto p-4">
            {files === 'loading' ? <p className="muted">{t('start-loading')}</p>
              : files === 'failed' ? <p role="alert">{t('diff-failed')}</p>
              : files.length === 0 ? <p className="muted">{t('changes-none')}</p>
              : files.map((f) => (
                <section key={f.path} className="mb-5 overflow-hidden rounded-m border border-line">
                  <h3 className="m-0 flex items-center justify-between gap-3 border-b border-line bg-surface-2 px-3 py-2 font-mono text-[0.8125rem] font-semibold">
                    <span className="truncate">{f.path}</span>
                    <span className="flex-none text-xs"><span className="text-finished">+{f.added}</span> <span className="text-needs-you">−{f.removed}</span></span>
                  </h3>
                  <pre className="m-0 overflow-x-auto font-mono text-xs leading-5">
                    {f.lines.map((l, i) => (
                      <div key={i} className={l.kind === 'add' ? 'bg-finished/15' : l.kind === 'del' ? 'bg-needs-you/15' : l.kind === 'hunk' ? 'bg-surface-2 text-muted' : ''}>
                        <span className="inline-block w-5 select-none text-center text-muted">{l.kind === 'add' ? '+' : l.kind === 'del' ? '−' : ''}</span>{l.text || ' '}
                      </div>
                    ))}
                  </pre>
                </section>
              ))}
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
