import { useCallback, useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { t } from '../i18n';
import { visible } from '../net/view';
import { placeMenu } from './placement';
import { Icon } from '../components/Icon';

export interface MenuItem {
  label: string;
  run?: () => void;
  items?: MenuItem[]; // a submenu, shown in place of this list with a way back
  checked?: boolean;
  disabled?: boolean;
  divider?: boolean; // a line above this item
  lead?: ReactNode; // a mark before the label (a state)
  hint?: string; // quiet words after the label (the state, in words)
  tile?: ReactNode; // drawn as a tile in a grid with the neighbouring tiles (a picture; the label is its name for tooltip and screen readers)
}

const TILE_COLUMNS = 3;

// The tiles that follow one another from `from`.
const run = (items: MenuItem[], from: number): MenuItem[] => {
  const out: MenuItem[] = [];
  for (let i = from; i < items.length && items[i]?.tile; i++) out.push(items[i] as MenuItem);
  return out;
};

// A menu button: arrows move, Enter or Space chooses, Right or Enter opens a submenu, Left goes back, Escape closes and
// returns focus to the button. Submenus replace the list (one level visible at a time), which also works on a phone.
export function Menu({ label, items, children, className = 'ide-iconbtn', align = 'end' }: { label: string; items: MenuItem[] | (() => MenuItem[]); children: ReactNode; className?: string; align?: 'start' | 'end' }) {
  const [open, setOpen] = useState(false);
  const [trail, setTrail] = useState<MenuItem[]>([]);
  const [hint, setHint] = useState<string | undefined>(); // the name of the tile the pointer or focus is on
  const button = useRef<HTMLButtonElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const root = typeof items === 'function' ? (open ? items() : []) : items;
  const page = trail.length ? (trail[trail.length - 1]?.items ?? []) : root;
  // The list is drawn in a portal at fixed coordinates, so no ancestor's overflow clips it, and is placed by the room there is:
  // below the button when it fits, above when that has more room, clamped to the visible viewport (above a software keyboard
  // too), with its own scroll when it is taller than the room.
  const place = useCallback(() => {
    const el = list.current;
    const btn = button.current;
    if (!el || !btn) return;
    const r = btn.getBoundingClientRect();
    const vv = window.visualViewport;
    const view = vv ? { left: vv.offsetLeft, top: vv.offsetTop, right: vv.offsetLeft + vv.width, bottom: vv.offsetTop + vv.height } : { left: 0, top: 0, right: window.innerWidth, bottom: window.innerHeight };
    el.style.maxHeight = 'none';
    const natural = { width: el.offsetWidth, height: Math.min(el.scrollHeight + (el.offsetHeight - el.clientHeight), 384) }; // at most 24rem
    const p = placeMenu(r, natural, view, align, getComputedStyle(el).direction === 'rtl');
    el.style.left = `${p.left}px`;
    el.style.top = `${p.top}px`;
    el.style.maxHeight = `${Math.min(p.maxHeight, 384)}px`;
  }, [align]);
  const close = (refocus = true) => { setOpen(false); setTrail([]); if (refocus) button.current?.focus(); };

  useEffect(() => {
    if (!open) return;
    const away = (e: PointerEvent) => { if (!list.current?.contains(e.target as Node) && !button.current?.contains(e.target as Node)) close(false); };
    document.addEventListener('pointerdown', away);
    return () => document.removeEventListener('pointerdown', away);
  }, [open]); // eslint-disable-line react-hooks/exhaustive-deps
  useLayoutEffect(() => { if (open) place(); }, [open, trail, hint, place, page.length]);
  // Resizing, scrolling (outside the list) and the keyboard coming or going move the list with its button.
  useEffect(() => {
    if (!open) return;
    const again = (e?: Event) => {
      if (e?.target instanceof Node && list.current?.contains(e.target)) return;
      const r = button.current?.getBoundingClientRect();
      if (r && (r.bottom < 0 || r.top > window.innerHeight)) { close(false); return; } // the button scrolled out of sight
      place();
    };
    const vv = window.visualViewport;
    window.addEventListener('resize', again);
    document.addEventListener('scroll', again, true);
    vv?.addEventListener('resize', again);
    vv?.addEventListener('scroll', again);
    return () => { window.removeEventListener('resize', again); document.removeEventListener('scroll', again, true); vv?.removeEventListener('resize', again); vv?.removeEventListener('scroll', again); };
  }, [open, place]); // eslint-disable-line react-hooks/exhaustive-deps
  // The first item takes focus whenever a list opens.
  useEffect(() => { if (open) list.current?.querySelector<HTMLElement>('[role^=menuitem]:not([disabled])')?.focus({ preventScroll: true }); }, [open, trail]);

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
    // On a grid of tiles the arrows walk it by row and column; at its edge Up and Down leave it for the list.
    const tile = (document.activeElement as HTMLElement | null)?.closest<HTMLElement>('[data-tile]');
    if (tile) {
      const tiles = [...(tile.parentElement?.querySelectorAll<HTMLElement>('[data-tile]:not([disabled])') ?? [])];
      const i = tiles.indexOf(tile);
      const step = e.key === 'ArrowRight' ? (rtl ? -1 : 1) : e.key === 'ArrowLeft' ? (rtl ? 1 : -1) : e.key === 'ArrowDown' ? TILE_COLUMNS : e.key === 'ArrowUp' ? -TILE_COLUMNS : 0;
      if (step !== 0 && !(trail.length && Math.abs(step) === 1 && e.key === back)) {
        const target = tiles[i + step];
        if (target) { e.preventDefault(); target.focus(); return; }
        if (Math.abs(step) === 1) { e.preventDefault(); return; } // the row's end: stay
      }
    }
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
    <span className="relative inline-flex max-w-full">
      <button ref={button} type="button" className={className} aria-haspopup="menu" aria-expanded={open} aria-label={visible(label)} title={visible(label)}
        onClick={() => (open ? close() : setOpen(true))}
        onKeyDown={(e) => { if (e.key === 'ArrowDown' && !open) { e.preventDefault(); setOpen(true); } }}>
        {children}
      </button>
      {open && createPortal(
        <div ref={list} role="menu" aria-label={visible((trail.length ? trail[trail.length - 1]?.label : label) ?? '')} onKeyDown={onKey}
          style={{ position: 'fixed', left: 0, top: 0 }}
          className="z-50 grid min-w-56 max-w-[min(20rem,calc(100vw-1rem))] gap-px overflow-y-auto overscroll-contain rounded-s border border-line bg-surface-1 p-1 text-[0.8125rem] text-ink shadow-card">
          {trail.length > 0 && (
            <button type="button" role="menuitem" className="ide-row gap-1.5 text-muted" onClick={() => setTrail((cur) => cur.slice(0, -1))}>
              <Icon name="chevron" className="size-3 rotate-180 rtl:rotate-0" />{t('layout-menu-back')}
            </button>
          )}
          {page.map((item, i) => item.tile ? (
            // Tiles of one run sit in a three-column grid: the first of a run draws all of them, the rest draw nothing.
            page[i - 1]?.tile ? null : (
              <div key={`${i}-${item.label}`} className="grid gap-1">
                <div className="grid grid-cols-3 gap-1" data-tiles>
                  {run(page, i).map((x, j) => (
                    <button key={x.label} type="button" role="menuitemradio" aria-checked={!!x.checked} aria-label={visible(x.label)} title={visible(x.label)} data-index={i + j} data-tile disabled={x.disabled}
                      tabIndex={x.checked || (j === 0 && !run(page, i).some((y) => y.checked)) ? 0 : -1}
                      className="menu-tile" onClick={() => choose(x)} onPointerEnter={() => setHint(x.label)} onPointerLeave={() => setHint(undefined)} onFocus={() => setHint(x.label)} onBlur={() => setHint(undefined)}>
                      {x.tile}
                    </button>
                  ))}
                </div>
                {/* The name of the tile you are on, or of the chosen one: the pictures alone are not the only way to tell them apart. */}
                <p className="m-0 min-h-4 px-1 text-xs text-muted" aria-hidden>{visible(hint ?? run(page, i).find((x) => x.checked)?.label ?? '')}</p>
              </div>
            )
          ) : (
            <div key={`${i}-${item.label}`} className={item.divider ? 'mt-1 border-t border-line pt-1' : ''}>
              <button type="button" role={item.checked === undefined ? 'menuitem' : 'menuitemradio'} aria-checked={item.checked} aria-haspopup={item.items ? 'menu' : undefined}
                data-index={i} disabled={item.disabled} className="ide-row justify-between disabled:opacity-40" onClick={() => choose(item)}>
                <span className="flex min-w-0 items-center gap-2">
                  {item.checked !== undefined && <span aria-hidden className={`size-1.5 flex-none rounded-full ${item.checked ? 'bg-accent' : ''}`} />}
                  {item.lead}
                  <span className="truncate">{visible(item.label)}</span>
                </span>
                {item.hint && <span className="flex-none text-xs text-muted">{visible(item.hint)}</span>}
                {item.items && <Icon name="chevron" className="size-3 rtl:rotate-180" />}
              </button>
            </div>
          ))}
        </div>,
        document.body,
      )}
    </span>
  );
}
