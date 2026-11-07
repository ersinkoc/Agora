import { useEffect, useMemo, useRef, useState } from 'react';
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

  useEffect(() => {
    const opener = document.activeElement as HTMLElement | null;
    input.current?.focus();
    return () => opener?.focus?.();
  }, []);

  const choose = (item: PaletteItem | undefined) => {
    if (!item) return;
    onClose();
    item.run();
  };

  return (
    <div className="palette__backdrop" onMouseDown={onClose}>
      <div
        className="palette"
        role="dialog"
        aria-modal="true"
        aria-label={t('palette-title')}
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key === 'Escape') onClose();
          else if (e.key === 'ArrowDown') { e.preventDefault(); setActive(Math.min(shown + 1, results.length - 1)); }
          else if (e.key === 'ArrowUp') { e.preventDefault(); setActive(Math.max(shown - 1, 0)); }
          else if (e.key === 'Enter') choose(results[shown]);
        }}
      >
        <input
          ref={input}
          className="palette__input"
          value={query}
          placeholder={t('palette-placeholder')}
          aria-label={t('palette-title')}
          role="combobox"
          aria-expanded="true"
          aria-controls="palette-list"
          aria-activedescendant={results[shown] ? `palette-${results[shown].id}` : undefined}
          onChange={(e) => { setQuery(e.target.value); setActive(0); }}
        />
        <ul id="palette-list" className="palette__list" role="listbox">
          {results.length === 0 && <li className="palette__empty muted">{t('palette-empty')}</li>}
          {results.map((r, i) => (
            <li
              key={r.id}
              id={`palette-${r.id}`}
              role="option"
              aria-selected={i === shown}
              className={`palette__item${i === shown ? ' palette__item--active' : ''}`}
              onMouseEnter={() => setActive(i)}
              onClick={() => choose(r)}
            >
              {r.label}
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}
