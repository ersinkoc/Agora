import { useEffect, useRef, useState } from 'react';
import { t } from '../../i18n';
import { Icon } from '../Icon';
import { Button } from './Button';

// A command or id to copy, shown exactly as it is typed (never translated), in mono, with a Copy button. Where the clipboard is missing
// or refuses (an insecure origin, a denied permission), the text is selected instead and a line says to copy it with the keyboard.
export function CopyChip({ text, label }: { text: string; label?: string }) {
  const [state, setState] = useState<'copied' | 'selected'>();
  const code = useRef<HTMLElement>(null);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);
  const select = () => {
    const el = code.current;
    const sel = typeof window !== 'undefined' ? window.getSelection() : null;
    if (el && sel) { sel.removeAllRanges(); sel.selectAllChildren(el); }
    clearTimeout(timer.current);
    setState('selected');
  };
  const copy = async () => {
    try {
      if (!navigator.clipboard?.writeText) throw new Error('no clipboard');
      await navigator.clipboard.writeText(text);
      setState('copied');
      clearTimeout(timer.current);
      timer.current = setTimeout(() => setState(undefined), 2000);
    } catch { select(); }
  };
  return (
    <div className="kit-copy">
      <div className="kit-copy__box">
        <code ref={code} dir="ltr" className="kit-copy__text">{text}</code>
        <Button variant="ghost" size="sm" className="kit-copy__btn" aria-label={label ? `${t('address-copy')} ${label}` : undefined} onClick={() => void copy()}>
          <Icon name={state === 'copied' ? 'check' : 'copy'} className="size-3.5" />{t(state === 'copied' ? 'address-copied' : 'address-copy')}
        </Button>
      </div>
      <span role="status" className={state === 'selected' ? 'kit-copy__note' : 'sr-only'}>{state ? t(state === 'copied' ? 'address-copied' : 'copy-selected') : ''}</span>
    </div>
  );
}
