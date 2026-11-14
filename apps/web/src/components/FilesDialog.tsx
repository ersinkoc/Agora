import * as Dialog from '@radix-ui/react-dialog';
import { useCallback, useRef } from 'react';
import { t } from '../i18n';
import type { AgentTileModel } from '../model';
import type { WorkspaceFiles } from '../net/files';
import { FilesView } from './FilesView';
import { Icon } from './Icon';

// The files of one agent's workspace in a window of their own (the same view a Files pane shows). The agent's tile is replaced on
// every fleet update; the files are read again only when asked, or for another agent.
export function FilesDialog({ agent, files, load, onClose }: {
  agent: AgentTileModel;
  files: WorkspaceFiles;
  load?: (a: AgentTileModel, path?: string) => Promise<string>;
  onClose: () => void;
}) {
  const latest = useRef(agent);
  latest.current = agent;
  const loadFor = useCallback((path?: string) => (load ? load(latest.current, path) : Promise.resolve('')), [load, agent.id]); // eslint-disable-line react-hooks/exhaustive-deps
  return (
    <Dialog.Root open onOpenChange={(o) => { if (!o) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-40 bg-surface-0/70 backdrop-blur-sm" />
        <Dialog.Content aria-describedby={undefined} onEscapeKeyDown={(e) => { if ((e.target as Element | null)?.closest?.('[data-finding]')) e.preventDefault(); }} className="fixed inset-2 z-50 flex flex-col overflow-hidden rounded-l border border-line bg-surface-1 shadow-card md:inset-x-[3vw] md:inset-y-[4vh]">
          <header className="flex flex-none items-center justify-between gap-3 border-b border-line px-4 py-2.5">
            <div className="min-w-0">
              <Dialog.Title className="m-0 truncate text-base font-semibold tracking-normal text-ink normal-case">{t('files-title', { name: agent.name })}</Dialog.Title>
              <p className="muted m-0 flex min-w-0 items-center gap-1.5 truncate text-xs">
                <Icon name="machine" className="size-3.5 flex-none" /><span className="truncate">{agent.machine}</span>
                <Icon name="folder" className="ms-1 size-3.5 flex-none" /><span className="truncate">{agent.project || agent.workspace}</span>
                {agent.branch && <><Icon name="branch" className="ms-1 size-3.5 flex-none" /><span className="truncate font-mono">{agent.branch}</span></>}
              </p>
            </div>
            <Dialog.Close className="button">{t('diff-close')}</Dialog.Close>
          </header>
          <div className="min-h-0 flex-1">
            <FilesView files={files} load={load && agent.git !== false ? loadFor : undefined} changed={agent.filesChanged} />
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
