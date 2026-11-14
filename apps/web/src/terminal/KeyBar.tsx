import { useSyncExternalStore } from 'react';
import { t } from '../i18n';
import { Icon } from '../components/Icon';
import { BAR_KEYS, type BarKey } from './keys';

// A touch screen (a phone or a tablet): its keyboard has no Esc, Tab, Ctrl or arrows, so terminals get the key bar.
const TOUCH = '(pointer: coarse)';
const subscribeTouch = (cb: () => void) => { const m = typeof window !== 'undefined' ? window.matchMedia?.(TOUCH) : undefined; m?.addEventListener?.('change', cb); return () => m?.removeEventListener?.('change', cb); };
const isTouch = () => typeof window !== 'undefined' && !!window.matchMedia?.(TOUCH).matches;
export const useTouch = () => useSyncExternalStore(subscribeTouch, isTouch, () => false);

const ROTATE: Partial<Record<BarKey, string>> = { left: 'rotate-180', up: '-rotate-90', down: 'rotate-90', right: '' };
const LABEL: Record<BarKey, string> = { esc: 'key-esc', tab: 'key-tab', up: 'key-up', down: 'key-down', left: 'key-left', right: 'key-right' };

// Pressing a key must not take the focus from the terminal, or the touch keyboard closes: the press is cancelled and the click acts.
const keep = (e: { preventDefault(): void }) => e.preventDefault();

// ADR-0067 rung 2: Esc, Tab, a Ctrl that applies to the next key, the arrows and paste, under the terminal on a touch screen.
export function KeyBar({ ctrl, onKey, onCtrl, onPaste }: { ctrl: boolean; onKey: (k: BarKey) => void; onCtrl: () => void; onPaste?: () => void }) {
  return (
    <div role="toolbar" aria-label={t('key-bar')} dir="ltr" className="key-bar quiet-scroll">
      {BAR_KEYS.slice(0, 2).map((k) => <button key={k} type="button" className="key-bar__key" onPointerDown={keep} onMouseDown={keep} onClick={() => onKey(k)}>{t(LABEL[k])}</button>)}
      <button type="button" className="key-bar__key" aria-pressed={ctrl} onPointerDown={keep} onMouseDown={keep} onClick={onCtrl}>{t('key-ctrl')}</button>
      {BAR_KEYS.slice(2).map((k) => <button key={k} type="button" className="key-bar__key" aria-label={t(LABEL[k])} title={t(LABEL[k])} onPointerDown={keep} onMouseDown={keep} onClick={() => onKey(k)}><Icon name="chevron" className={`size-4 ${ROTATE[k] ?? ''}`} /></button>)}
      {onPaste && <button type="button" className="key-bar__key" onPointerDown={keep} onMouseDown={keep} onClick={onPaste}>{t('key-paste')}</button>}
    </div>
  );
}
