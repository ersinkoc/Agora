import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { t } from '../i18n';
import { Icon } from '../components/Icon';
import { Menu, type MenuItem } from './Menu';
import { hiddenTabs, scrollEdges } from './overflow';

export interface StripTab { id: string; label: string; active: boolean }

// The tab row of a group. When the tabs are wider than the space they scroll sideways, the edge that has more fades, the tab in front is
// always brought into view, and a "more" menu lists the tabs that are cut off so none is out of reach (also on a touch screen). The tabs
// themselves (drag, close, keys) are the caller's; this only owns the strip around them.
export function TabStrip({ tabs, onPick, children, trailing }: { tabs: readonly StripTab[]; onPick: (id: string) => void; children: ReactNode; trailing?: ReactNode }) {
  const strip = useRef<HTMLDivElement>(null);
  const [edges, setEdges] = useState({ start: false, end: false, overflowing: false });
  const measure = useCallback(() => {
    const el = strip.current;
    if (!el) return;
    const next = scrollEdges(el);
    setEdges((cur) => (cur.start === next.start && cur.end === next.end && cur.overflowing === next.overflowing ? cur : next));
  }, []);
  useLayoutEffect(measure);
  useEffect(() => {
    const el = strip.current;
    if (!el || typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [measure]);
  const front = tabs.find((x) => x.active)?.id;
  useEffect(() => { if (front) document.getElementById(`tab-${front}`)?.scrollIntoView?.({ inline: 'nearest', block: 'nearest' }); }, [front, tabs.length]);

  // Measured when the menu opens: the tabs that are cut off by either edge of the strip.
  const hidden = (): MenuItem[] => {
    const box = strip.current?.getBoundingClientRect();
    const spans = tabs.map((x) => { const r = document.getElementById(`tab-${x.id}`)?.getBoundingClientRect(); return { id: x.id, left: r?.left ?? 0, right: r?.right ?? 0 }; });
    const ids = new Set(box ? hiddenTabs(box, spans) : tabs.map((x) => x.id));
    return tabs.filter((x) => ids.has(x.id)).map((x) => ({ label: x.label, checked: x.active, run: () => onPick(x.id) }));
  };

  return (
    <div className="flex min-w-0 flex-1 items-stretch">
      <div ref={strip} role="tablist" aria-label={t('console-agents')} onScroll={measure} data-fade-start={edges.start || undefined} data-fade-end={edges.end || undefined}
        className="tab-strip flex min-w-0 flex-1 overflow-x-auto quiet-scroll">
        {children}
      </div>
      {edges.overflowing && <span className="flex flex-none items-center"><Menu label={t('ide-more-tabs')} items={hidden} align="end"><Icon name="chevronDown" /></Menu></span>}
      {trailing}
    </div>
  );
}
