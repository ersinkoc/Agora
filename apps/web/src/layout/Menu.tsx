import { useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from 'react';
import { t } from '../i18n';
import { Icon } from '../components/Icon';

export interface MenuItem {
  label: string;
  run?: () => void;
  items?: MenuItem[]; // a submenu, shown in place of this list with a way back
  checked?: boolean;
  disabled?: boolean;
  divider?: boolean; // a line above this item
}

// A menu button: arrows move, Enter or Space chooses, Right or Enter opens a submenu, Left goes back, Escape closes and
// returns focus to the button. Submenus replace the list (one level visible at a time), which also works on a phone.
export function Menu({ label, items, children, className = 'ide-iconbtn', align = 'end' }: { label: string; items: MenuItem[] | (() => MenuItem[]); children: ReactNode; className?: string; align?: 'start' | 'end' }) {
  const [open, setOpen] = useState(false);
  const [trail, setTrail] = useState<MenuItem[]>([]);
  const button = useRef<HTMLButtonElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const root = typeof items === 'function' ? (open ? items() : []) : items;
  const page = trail.length ? (trail[trail.length - 1]?.items ?? []) : root;
  const close = (refocus = true) => { setOpen(false); setTrail([]); if (refocus) button.current?.focus(); };

  useEffect(() => {
    if (!open) return;
    const away = (e: PointerEvent) => { if (!list.current?.contains(e.target as Node) && !button.current?.contains(e.target as Node)) close(false); };
    document.addEventListener('pointerdown', away);
    return () => document.removeEventListener('pointerdown', away);
  }, [open]); // eslint-disable-line react-hooks/exhaustive-deps
  // The first item takes focus whenever a list opens.
  useEffect(() => { if (open) list.current?.querySelector<HTMLElement>('[role^=menuitem]:not([disabled])')?.focus(); }, [open, trail]);

  const choose = (item: MenuItem) => {
    if (item.disabled) return;
    if (item.items) { setTrail((cur) => [...cur, item]); return; }
    close();
    item.run?.();
  };
  const onKey = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    const all = [...(list.current?.querySelectorAll<HTMLElement>('[role^=menuitem]:not([disabled])') ?? [])];
    const at = all.indexOf(document.activeElement as HTMLElement);
    const rtl = getComputedStyle(e.currentTarget).direction === 'rtl';
    const back = rtl ? 'ArrowRight' : 'ArrowLeft';
    if (e.key === 'ArrowDown') { e.preventDefault(); all[(at + 1) % all.length]?.focus(); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); all[(at - 1 + all.length) % all.length]?.focus(); }
    else if (e.key === 'Home') { e.preventDefault(); all[0]?.focus(); }
    else if (e.key === 'End') { e.preventDefault(); all[all.length - 1]?.focus(); }
    else if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); if (trail.length) setTrail((cur) => cur.slice(0, -1)); else close(); }
    else if (e.key === back && trail.length) { e.preventDefault(); setTrail((cur) => cur.slice(0, -1)); }
    else if (e.key === (rtl ? 'ArrowLeft' : 'ArrowRight')) { const item = page[Number((document.activeElement as HTMLElement)?.dataset.index)]; if (item?.items) { e.preventDefault(); choose(item); } }
    else if (e.key === 'Tab') close(false);
  };

  return (
    <span className="relative inline-flex">
      <button ref={button} type="button" className={className} aria-haspopup="menu" aria-expanded={open} aria-label={label} title={label}
        onClick={() => (open ? close() : setOpen(true))}
        onKeyDown={(e) => { if (e.key === 'ArrowDown' && !open) { e.preventDefault(); setOpen(true); } }}>
        {children}
      </button>
      {open && (
        <div ref={list} role="menu" aria-label={trail.length ? trail[trail.length - 1]?.label : label} onKeyDown={onKey}
          className={`absolute top-full z-30 mt-1 grid max-h-[min(24rem,70vh)] min-w-56 max-w-[min(20rem,calc(100vw-2rem))] gap-px overflow-y-auto rounded-s border border-line bg-surface-1 p-1 text-[13px] text-ink shadow-card ${align === 'end' ? 'end-0' : 'start-0'}`}>
          {trail.length > 0 && (
            <button type="button" role="menuitem" className="ide-row gap-1.5 text-muted" onClick={() => setTrail((cur) => cur.slice(0, -1))}>
              <Icon name="chevron" className="size-3 rotate-180 rtl:rotate-0" />{t('layout-menu-back')}
            </button>
          )}
          {page.map((item, i) => (
            <div key={`${i}-${item.label}`} className={item.divider ? 'mt-1 border-t border-line pt-1' : ''}>
              <button type="button" role={item.checked === undefined ? 'menuitem' : 'menuitemradio'} aria-checked={item.checked} aria-haspopup={item.items ? 'menu' : undefined}
                data-index={i} disabled={item.disabled} className="ide-row justify-between disabled:opacity-40" onClick={() => choose(item)}>
                <span className="flex min-w-0 items-center gap-2">
                  {item.checked !== undefined && <span aria-hidden className={`size-1.5 flex-none rounded-full ${item.checked ? 'bg-accent' : ''}`} />}
                  <span className="truncate">{item.label}</span>
                </span>
                {item.items && <Icon name="chevron" className="size-3 rtl:rotate-180" />}
              </button>
            </div>
          ))}
        </div>
      )}
    </span>
  );
}
