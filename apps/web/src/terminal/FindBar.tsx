import { useEffect, useRef, useState } from 'react';
import { t } from '../i18n';
import { Icon } from '../components/Icon';

// Find in what the terminal shows and has scrolled off (Ctrl+Shift+F, Cmd+F on a Mac). Enter goes to the next match, Shift+Enter to the
// one before, Esc closes it and gives the keyboard back to the terminal.
export function FindBar({ find, onClose }: { find: (text: string, backwards: boolean) => boolean; onClose: () => void }) {
  const input = useRef<HTMLInputElement>(null);
  const [none, setNone] = useState(false);
  useEffect(() => { input.current?.focus(); input.current?.select(); }, []);
  const go = (back: boolean) => { const q = input.current?.value ?? ''; setNone(q.length > 0 && !find(q, back)); };
  return (
    <div role="search" className="terminal-find" onKeyDown={(e) => { if (e.key === 'Escape') { e.preventDefault(); onClose(); } }}>
      <input ref={input} type="search" className="terminal-find__input" aria-label={t('terminal-find')} placeholder={t('terminal-find')} aria-invalid={none || undefined}
        onChange={() => go(false)}
        onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); go(e.shiftKey); } }} />
      {none && <span role="status" className="text-xs text-muted">{t('terminal-find-none')}</span>}
      <button type="button" className="ide-iconbtn" onClick={() => go(true)} aria-label={t('terminal-find-previous')} title={t('terminal-find-previous')}><Icon name="chevron" className="size-4 -rotate-90" /></button>
      <button type="button" className="ide-iconbtn" onClick={() => go(false)} aria-label={t('terminal-find-next')} title={t('terminal-find-next')}><Icon name="chevron" className="size-4 rotate-90" /></button>
      <button type="button" className="ide-iconbtn" onClick={onClose} aria-label={t('terminal-find-close')} title={t('terminal-find-close')}><Icon name="close" className="size-4" /></button>
    </div>
  );
}
