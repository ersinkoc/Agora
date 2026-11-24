import type { ReactNode } from 'react';
import { t } from '../i18n';
import type { AgentTileModel } from '../model';
import type { MessageModel } from '../net/view';
import { Details, type DetailsTab } from './Inspector';

// The side panel's lists for the gallery: the team chat with long and short messages, and the kit empty state of two lists, at the panel's
// width. The clock is fixed so the gallery does not change by itself. (Usage is left out: its plan limits read the live quota clock.)
export const GALLERY_NOW = Date.UTC(2026, 0, 5, 12, 0, 0);

const none = { attention: [], approvals: [], changed: [], boards: [], messages: [], onOpen: () => {}, onResolve: () => {} };

function Panel({ tab, title, children }: { tab: DetailsTab; title: string; children: ReactNode }) {
  return (
    <section className="grid w-[21rem] max-w-full content-start overflow-hidden rounded-m border border-line bg-surface-1" data-panel-demo={tab} aria-label={title}>
      <div className="flex h-9 items-center border-b border-line px-3"><h2 className="panel-title">{title}</h2></div>
      <div className="p-3">{children}</div>
    </section>
  );
}

export function PanelGallery({ messages, fleet }: { messages: readonly MessageModel[]; fleet: readonly AgentTileModel[] }) {
  return (
    <div className="gallery__row items-start">
      <Panel tab="messages" title={t('ide-tab-messages')}><Details tab="messages" {...none} fleet={fleet} now={GALLERY_NOW} messages={messages} /></Panel>
      <Panel tab="changes" title={t('ide-tab-changes')}><Details tab="changes" {...none} now={GALLERY_NOW} /></Panel>
      <Panel tab="board" title={t('board-title')}><Details tab="board" {...none} now={GALLERY_NOW} /></Panel>
    </div>
  );
}
