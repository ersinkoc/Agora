import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { t } from '../i18n';
import { Icon } from '../components/Icon';
import { Menu, type MenuItem } from './Menu';
import { StateMark } from '../components/StateBadge';
import type { UiState } from '../model';
import { hiddenTabs, scrollEdges, tabSignal } from './overflow';

export interface StripTab { id: string; label: string; active: boolean; state?: UiState; failed?: boolean }

// The tab row of a group. When the tabs are wider than the space they scroll sideways, the edge that has more fades, the tab in front is
// always brought into view, and a "more" menu lists the tabs that are cut off so none is out of reach (also on a touch screen). The tabs
// themselves (drag, close, keys) are the caller's; this only owns the strip around them.
export function TabStrip({ tabs, onPick, children, trailing }: { tabs: readonly StripTab[]; onPick: (id: string) => void; children: ReactNode; trailing?: ReactNode }) {
  const strip = useRef<HTMLDivElement>(null);
  const [edges, setEdges] = useState({ start: false, end: false, overflowing: false });
  const [hiddenIds, setHiddenIds] = useState<string[]>([]);
  const front = tabs.find((x) => x.active)?.id;
  const frontRef = useRef(front);
  frontRef.current = front;
  const tabsRef = useRef(tabs);
  tabsRef.current = tabs;
  const measure = useCallback(() => {
    const el = strip.current;
    if (!el) return;
    // Shrinking the tabs pays only when it makes them all fit. When even at their floors they do not, every tab keeps its whole name
    // (up to its largest width) and the strip scrolls, instead of names cut short beside a gap the cut-off tab leaves (visual pass
    // 2026-11). Decided from the floors, which do not depend on this choice, so measuring again gives the same answer.
    const floors = tabsRef.current.reduce((sum, x) => {
      const n = document.getElementById(`tab-${x.id}`);
      if (!n) return sum;
      const width = n.getBoundingClientRect().width;
      const floor = parseFloat(getComputedStyle(n).minWidth);
      return sum + (x.active ? width : Math.min(width, Number.isFinite(floor) ? floor : 0));
    }, 0);
    el.toggleAttribute('data-crowded', floors > el.clientWidth + 1);
    const box = el.getBoundingClientRect();
    const spans = tabsRef.current.map((x) => { const r = document.getElementById(`tab-${x.id}`)?.getBoundingClientRect(); return { id: x.id, left: r?.left ?? 0, right: r?.right ?? 0 }; });
    const ids = hiddenTabs(box, spans);
    setHiddenIds((cur) => (cur.join('\n') === ids.join('\n') ? cur : ids));
    // A tab that is only partly in view would show a fragment of its title: it is not drawn (the "more" menu lists it) until scrolled whole into view.
    for (const x of tabsRef.current) { const n = document.getElementById(`tab-${x.id}`); if (n) n.toggleAttribute('data-cut', ids.includes(x.id) && !x.active); }
    const next = scrollEdges(el);
    setEdges((cur) => (cur.start === next.start && cur.end === next.end && cur.overflowing === next.overflowing ? cur : next));
  }, []);
  useLayoutEffect(measure);
  useEffect(() => {
    const el = strip.current;
    if (!el || typeof ResizeObserver === 'undefined') return;
    // A strip that got narrower keeps the tab in front in view, so the others (cut off, drawn in the "more" menu) are what leave.
    const ro = new ResizeObserver(() => { measure(); document.getElementById(`tab-${frontRef.current}`)?.scrollIntoView?.({ inline: 'nearest', block: 'nearest' }); });
    ro.observe(el);
    return () => ro.disconnect();
  }, [measure]);
  useEffect(() => { if (front) document.getElementById(`tab-${front}`)?.scrollIntoView?.({ inline: 'nearest', block: 'nearest' }); }, [front, tabs.length]);

  // The menu lists the tabs that are cut off, each with its state (a mark and in words), so one that needs you is never out of reach unseen.
  const hidden = (): MenuItem[] => tabs.filter((x) => hiddenIds.includes(x.id)).map((x) => ({
    label: x.label, checked: x.active, run: () => onPick(x.id),
    lead: x.state ? <StateMark state={x.state} failed={x.failed} /> : undefined,
    hint: x.state ? t(x.failed ? 'state-failed' : `state-${x.state}`) : undefined,
  }));
  const signal = tabSignal(tabs.map((x) => ({ id: x.id, needsYou: x.state === 'needs-you' && !x.failed, failed: x.failed, working: x.state === 'working' && !x.failed })), hiddenIds);
  const signalWord = signal === 'needs-you' ? t('ide-more-tabs-needs') : signal === 'failed' ? t('ide-more-tabs-failed') : signal === 'working' ? t('ide-more-tabs-working') : '';

  return (
    <div className="flex min-w-0 flex-1 items-stretch">
      <div ref={strip} role="tablist" aria-label={t('console-agents')} onScroll={measure} data-fade-start={edges.start || undefined} data-fade-end={edges.end || undefined}
        className="tab-strip flex min-w-0 flex-1 overflow-x-auto quiet-scroll">
        {children}
      </div>
      {edges.overflowing && <span className="flex flex-none items-center"><Menu label={signalWord ? `${t('ide-more-tabs')}: ${signalWord}` : t('ide-more-tabs')} items={hidden} align="end" className="ide-iconbtn relative">
        <Icon name="chevronDown" />{signal && <span aria-hidden data-signal={signal} className={`absolute end-0.5 top-0.5 size-2 rounded-full ${signal === 'needs-you' ? 'bg-needs-you' : signal === 'failed' ? 'bg-ink' : 'bg-working'}`} />}
      </Menu></span>}
      {trailing}
    </div>
  );
}
