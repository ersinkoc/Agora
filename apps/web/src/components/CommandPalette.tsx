import * as Dialog from '@radix-ui/react-dialog';
import { useMemo, useRef, useState } from 'react';
import { rank } from '../fuzzy';
import { t } from '../i18n';

export interface PaletteItem {
  id: string;
  label: string;
  run: () => void;
}

// Modal list with a search box. Arrow keys move, Enter runs, Escape closes; focus returns to the opener.
export function CommandPalette({ items, onClose }: { items: readonly PaletteItem[]; onClose: () => void }) {
  const [query, setQuery] = useState('');
  const [active, setActive] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const results = useMemo(() => rank(items, query, (i) => i.label), [items, query]);
  const shown = Math.min(active, Math.max(results.length - 1, 0));

  const choose = (item: PaletteItem | undefined) => {
    if (!item) return;
    onClose();
    item.run();
  };

  return (
    <Dialog.Root open onOpenChange={(open) => { if (!open) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-40 bg-surface-0/70 backdrop-blur-sm" />
        <Dialog.Content
          aria-describedby={undefined}
          onOpenAutoFocus={(e) => { e.preventDefault(); input.current?.focus(); }}
          className="fixed top-[12vh] left-1/2 z-50 w-[min(560px,calc(100vw-2rem))] -translate-x-1/2 overflow-hidden rounded-l border border-line bg-surface-1 shadow-card"
          onKeyDown={(e) => {
            if (e.key === 'ArrowDown') { e.preventDefault(); setActive(Math.min(shown + 1, results.length - 1)); }
            else if (e.key === 'ArrowUp') { e.preventDefault(); setActive(Math.max(shown - 1, 0)); }
            else if (e.key === 'Enter') choose(results[shown]);
          }}
        >
          <Dialog.Title className="sr-only">{t('palette-title')}</Dialog.Title>
        <input
          ref={input}
          className="w-full rounded-none border-0 border-b border-line bg-transparent px-4 py-4 text-base outline-none"
          value={query}
          placeholder={t('palette-placeholder')}
          aria-label={t('palette-title')}
          role="combobox"
          aria-expanded="true"
          aria-controls="palette-list"
          aria-activedescendant={results[shown] ? `palette-${results[shown].id}` : undefined}
          onChange={(e) => { setQuery(e.target.value); setActive(0); }}
        />
        <ul id="palette-list" className="m-0 max-h-[50vh] list-none overflow-y-auto p-2" role="listbox">
          {results.length === 0 && <li className="px-3 py-2 muted">{t('palette-empty')}</li>}
          {results.map((r, i) => (
            <li
              key={r.id}
              id={`palette-${r.id}`}
              role="option"
              aria-selected={i === shown}
              className={`cursor-pointer rounded-s px-3 py-2 ${i === shown ? 'bg-surface-2 ring-1 ring-accent' : ''}`}
              onMouseEnter={() => setActive(i)}
              onClick={() => choose(r)}
            >
              {r.label}
            </li>
          ))}
        </ul>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
