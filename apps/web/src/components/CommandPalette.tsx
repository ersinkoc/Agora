import * as Dialog from '@radix-ui/react-dialog';
import { useEffect, useMemo, useRef, useState } from 'react';
import { rank } from '../fuzzy';
import { t } from '../i18n';
import { visible } from '../net/hidden';
import { Icon, type IconName } from './Icon';
import { AgentAvatar, EmptyState } from './kit';

export type PaletteGroup = 'go' | 'actions' | 'agents' | 'places' | 'tasks' | 'appearance';

export interface PaletteItem {
  id: string;
  label: string;
  run: () => void;
  /** The heading it is listed under; without one it is an action. */
  group?: PaletteGroup;
  icon?: IconName;
  /** An agent's row leads with the agent's avatar (its colour and kind) instead of an icon. */
  agent?: { accent: string; kind: string };
  /** The shortcut that does the same outside the palette, as the keyboard spells it. Only shortcuts that exist. */
  keys?: string;
  /** The same for `aria-keyshortcuts` (`Control+B`), when it is spelled differently from `keys`. */
  ariaKeys?: string;
}

// The headings in the order an empty query lists them.
export const PALETTE_GROUPS: readonly PaletteGroup[] = ['go', 'agents', 'actions', 'places', 'tasks', 'appearance'];

// The results as the list shows them: in groups, the entries of a group in rank order. With no query the groups come in their fixed order;
// with one, each group sits where its best match ranks. Arrows, Enter and the highlighted row all follow this order, never the raw ranking.
export function groupPalette(results: readonly PaletteItem[], fixedOrder = false): { group: PaletteGroup; items: PaletteItem[] }[] {
  const out = new Map<PaletteGroup, PaletteItem[]>();
  for (const r of results) {
    const g = r.group ?? 'actions';
    const list = out.get(g);
    if (list) list.push(r); else out.set(g, [r]);
  }
  const groups = [...out].map(([group, items]) => ({ group, items }));
  return fixedOrder ? groups.sort((a, b) => PALETTE_GROUPS.indexOf(a.group) - PALETTE_GROUPS.indexOf(b.group)) : groups;
}

const optionId = (id: string) => `palette-${id}`;

// Modal list with a search box. Arrow keys move, Enter runs, Escape closes; focus returns to the opener.
export function CommandPalette({ items, onClose }: { items: readonly PaletteItem[]; onClose: () => void }) {
  const [query, setQuery] = useState('');
  const [active, setActive] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const groups = useMemo(() => groupPalette(rank(items, query, (i) => visible(i.label)), query.trim() === ''), [items, query]);
  const flat = useMemo(() => groups.flatMap((g) => g.items), [groups]);
  const shown = Math.min(active, Math.max(flat.length - 1, 0));
  const current = flat[shown];
  // The highlighted row stays in view as the arrows move through a long list.
  useEffect(() => { if (current) document.getElementById(optionId(current.id))?.scrollIntoView?.({ block: 'nearest' }); }, [current]);

  const choose = (item: PaletteItem | undefined) => {
    if (!item) return;
    onClose();
    item.run();
  };

  let index = 0;
  return (
    <Dialog.Root open onOpenChange={(open) => { if (!open) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-40 bg-surface-0/70 backdrop-blur-sm" />
        <Dialog.Content
          aria-describedby={undefined}
          onOpenAutoFocus={(e) => { e.preventDefault(); input.current?.focus(); }}
          className="palette"
          onKeyDown={(e) => {
            if (e.key === 'ArrowDown') { e.preventDefault(); setActive(Math.min(shown + 1, flat.length - 1)); }
            else if (e.key === 'ArrowUp') { e.preventDefault(); setActive(Math.max(shown - 1, 0)); }
            else if (e.key === 'Enter') choose(current);
          }}
        >
          <Dialog.Title className="sr-only">{t('palette-title')}</Dialog.Title>
          <div className="palette__search">
            <Icon name="search" />
            <input
              ref={input}
              className="palette__input"
              value={query}
              placeholder={t('palette-placeholder')}
              aria-label={t('palette-title')}
              role="combobox"
              aria-expanded="true"
              aria-controls="palette-list"
              aria-activedescendant={current ? optionId(current.id) : undefined}
              onChange={(e) => { setQuery(e.target.value); setActive(0); }}
            />
          </div>
          <div id="palette-list" role="listbox" aria-label={t('palette-title')} className="palette__list" hidden={flat.length === 0}>
            {groups.map((g) => (
              <div key={g.group} role="group" aria-labelledby={`palette-group-${g.group}`} className="palette__group">
                <div id={`palette-group-${g.group}`} role="presentation" className="palette__heading">{t(`palette-group-${g.group}`)}</div>
                {g.items.map((r) => {
                  const i = index++;
                  return (
                    <div key={r.id} id={optionId(r.id)} role="option" aria-selected={i === shown} aria-keyshortcuts={r.ariaKeys ?? r.keys} className="palette__row"
                      onMouseEnter={() => setActive(i)} onClick={() => choose(r)}>
                      {r.agent ? <AgentAvatar accent={r.agent.accent} kind={r.agent.kind} /> : <span aria-hidden className="palette__icon"><Icon name={r.icon ?? 'step'} /></span>}
                      <span className="palette__label">{visible(r.label)}</span>
                      {r.keys && <kbd aria-hidden className="ide-kbd">{r.keys}</kbd>}
                    </div>
                  );
                })}
              </div>
            ))}
          </div>
          {flat.length === 0 && <EmptyState icon="search" title={t('palette-empty')} line={t('palette-empty-hint')} />}
          <div className="palette__foot" aria-hidden>
            <span><kbd className="ide-kbd">↑</kbd><kbd className="ide-kbd">↓</kbd>{t('palette-key-move')}</span>
            <span><kbd className="ide-kbd">Enter</kbd>{t('palette-key-open')}</span>
            <span><kbd className="ide-kbd">Esc</kbd>{t('palette-key-close')}</span>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
