import * as Dialog from '@radix-ui/react-dialog';
import { useCallback, useRef } from 'react';
import { t } from '../i18n';
import type { AgentTileModel } from '../model';
import type { DiffSource } from '../net/diff';
import { ChangesView } from './ChangesView';
import { Icon } from './Icon';
import { visible } from '../net/hidden';

// What one agent's workspace holds that is not committed, as a review screen (docs/ux/review.md): changed files on the left, one
// file's changes on the right. The agent's tile is replaced on every fleet update; the changes are read again only when asked, or
// when another agent's are shown.
export function DiffDialog({ agent, load, read, onClose }: {
  agent: AgentTileModel;
  load: (a: AgentTileModel, path?: string) => Promise<DiffSource>;
  read?: (path: string) => Promise<Uint8Array>;
  onClose: () => void;
}) {
  const latest = useRef(agent);
  latest.current = agent;
  const readRef = useRef(read);
  readRef.current = read;
  const loadFor = useCallback((path?: string) => load(latest.current, path), [load, agent.id]); // eslint-disable-line react-hooks/exhaustive-deps
  const readFor = useCallback((path: string) => (readRef.current ? readRef.current(path) : Promise.reject(new Error('no files'))), []);
  return (
    <Dialog.Root open onOpenChange={(open) => { if (!open) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-40 bg-surface-0/70 backdrop-blur-sm" />
        {/* The window's one-letter shortcuts (`n`) must not act behind the dialog, from its header and Close button too. */}
        <Dialog.Content aria-describedby={undefined} onKeyDown={(e) => { if (!e.ctrlKey && !e.metaKey && !e.altKey && e.key.length === 1) e.stopPropagation(); }}
          className="fixed inset-2 z-50 flex flex-col overflow-hidden rounded-l border border-line bg-surface-1 shadow-card md:inset-x-[3vw] md:inset-y-[4vh]">
          <header className="flex flex-none items-center justify-between gap-3 border-b border-line px-4 py-2.5">
            <div className="min-w-0">
              <Dialog.Title className="m-0 truncate text-base font-semibold tracking-normal text-ink normal-case">{t('diff-title', { name: visible(agent.name) })}</Dialog.Title>
              <p className="muted m-0 flex min-w-0 items-center gap-1.5 truncate text-xs">
                <Icon name="machine" className="size-3.5 flex-none" /><span className="truncate">{visible(agent.machine)}</span>
                <Icon name="folder" className="ms-1 size-3.5 flex-none" /><span className="truncate">{visible(agent.project || agent.workspace)}</span>
                {agent.branch && <><Icon name="branch" className="ms-1 size-3.5 flex-none" /><span className="truncate font-mono">{visible(agent.branch)}</span></>}
              </p>
            </div>
            <Dialog.Close className="button">{t('diff-close')}</Dialog.Close>
          </header>
          <div className="min-h-0 flex-1">
            <ChangesView load={loadFor} read={read ? readFor : undefined} changed={agent.filesChanged} />
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
