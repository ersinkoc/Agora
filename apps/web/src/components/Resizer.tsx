import { useRef } from 'react';
import { t } from '../i18n';

// A thin divider between two panels: drag it, or focus it and use the arrow keys. `side` says which panel it sizes: the one on its
// left grows when dragged right ("start"), the one on its right grows when dragged left ("end").
export function Resizer({ side, value, min, max, onChange }: { side: 'start' | 'end'; value: number; min: number; max: number; onChange: (px: number) => void }) {
  const from = useRef<{ x: number; value: number } | undefined>(undefined);
  const clamp = (v: number) => Math.min(max, Math.max(min, Math.round(v)));
  const dir = (typeof document !== 'undefined' && document.dir === 'rtl' ? -1 : 1) * (side === 'start' ? 1 : -1);
  return (
    <div role="separator" aria-orientation="vertical" aria-label={t('ide-resize')} aria-valuenow={value} aria-valuemin={min} aria-valuemax={max} tabIndex={0}
      className="group relative h-full w-0 cursor-col-resize touch-none"
      onPointerDown={(e) => { e.currentTarget.setPointerCapture(e.pointerId); from.current = { x: e.clientX, value }; }}
      onPointerMove={(e) => { if (from.current) onChange(clamp(from.current.value + (e.clientX - from.current.x) * dir)); }}
      onPointerUp={() => { from.current = undefined; }}
      onKeyDown={(e) => {
        const step = e.shiftKey ? 48 : 16;
        const grow = side === 'start' ? 1 : -1; // ArrowRight grows a start panel and shrinks an end panel
        if (e.key === 'ArrowLeft') { e.preventDefault(); onChange(clamp(value - step * grow)); }
        else if (e.key === 'ArrowRight') { e.preventDefault(); onChange(clamp(value + step * grow)); }
      }}>
      <span className="absolute inset-y-0 -start-1 w-2 group-hover:bg-accent/40 group-focus-visible:bg-accent/60 group-active:bg-accent/60" aria-hidden />
    </div>
  );
}
