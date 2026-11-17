import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { t } from '../i18n';
import { Icon } from '../components/Icon';
import { LINE_HEIGHTS, SCROLLBACKS, setTerminalPrefs, terminalPrefs, type TerminalPrefs } from './prefs';

const subscribe = (cb: () => void) => { window.addEventListener('agora-terminal-prefs', cb); return () => window.removeEventListener('agora-terminal-prefs', cb); };
const num = (n: number) => new Intl.NumberFormat(document.documentElement.lang || undefined, { minimumFractionDigits: 1 }).format(n);

// How terminals look and behave in this browser: one choice for all of them (prefs.ts). A menu on the pane's header; Esc closes it.
export function TerminalSettings() {
  const p: TerminalPrefs = useSyncExternalStore(subscribe, terminalPrefs, terminalPrefs);
  const [open, setOpen] = useState(false);
  const box = useRef<HTMLSpanElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  // Closing by Esc (from anywhere while it is open) gives the focus back to the button; a press outside closes it without taking the
  // focus from what was pressed.
  const close = (refocus: boolean) => { setOpen(false); if (refocus) trigger.current?.focus(); };
  useEffect(() => {
    if (!open) return;
    const away = (e: Event) => { if (!box.current?.contains(e.target as Node)) setOpen(false); };
    const esc = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.preventDefault(); close(true); } };
    document.addEventListener('pointerdown', away, true);
    document.addEventListener('keydown', esc, true);
    return () => { document.removeEventListener('pointerdown', away, true); document.removeEventListener('keydown', esc, true); };
  }, [open]);
  return (
    <span ref={box} className="terminal-menu">
      <button ref={trigger} type="button" className="ide-iconbtn" aria-expanded={open} aria-haspopup="true" aria-label={t('terminal-settings')} title={t('terminal-settings')} onClick={() => (open ? close(true) : setOpen(true))}><Icon name="more" /></button>
      {open && (
        <div role="group" aria-label={t('terminal-settings')} className="terminal-menu__panel">
          <label className="grid gap-1">
            <span>{t('terminal-line-height')}</span>
            <select value={p.lineHeight} onChange={(e) => setTerminalPrefs({ lineHeight: Number(e.target.value) as TerminalPrefs['lineHeight'] })}>
              {LINE_HEIGHTS.map((h) => <option key={h} value={h}>{num(h)}</option>)}
            </select>
            <span className="text-muted">{t('terminal-line-height-note')}</span>
          </label>
          <label className="grid gap-1">
            <span>{t('terminal-history')}</span>
            <select value={p.scrollback} onChange={(e) => setTerminalPrefs({ scrollback: Number(e.target.value) as TerminalPrefs['scrollback'] })}>
              {SCROLLBACKS.map((n) => <option key={n} value={n}>{new Intl.NumberFormat(document.documentElement.lang || undefined).format(n)}</option>)}
            </select>
          </label>
          <label className="flex items-center gap-2"><input type="checkbox" checked={p.readable} onChange={(e) => setTerminalPrefs({ readable: e.target.checked })} />{t('terminal-readable')}</label>
          <label className="flex items-center gap-2"><input type="checkbox" checked={p.copyOnSelect} onChange={(e) => setTerminalPrefs({ copyOnSelect: e.target.checked })} />{t('terminal-copy-on-select')}</label>
          <label className="flex items-center gap-2"><input type="checkbox" checked={p.gpu} onChange={(e) => setTerminalPrefs({ gpu: e.target.checked })} />{t('terminal-gpu')}</label>
        </div>
      )}
    </span>
  );
}
