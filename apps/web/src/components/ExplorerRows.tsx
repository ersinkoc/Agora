import { cloneElement, useLayoutEffect, useRef, useState, type ComponentPropsWithoutRef, type ReactElement } from 'react';

const ROWS_PER_CHUNK = 12;
const NEAR_VIEWPORT = '240px 0px';
const LIST_CLASS = 'm-0 list-none p-0';

type Rows<T> = {
  items: readonly T[];
  idOf: (item: T) => string;
  renderRow: (item: T) => ReactElement<ComponentPropsWithoutRef<'li'>>;
  current?: string;
  viewport: HTMLElement | null;
  windowed: boolean;
};

export function ExplorerRows<T>({ items, idOf, renderRow, current, viewport, windowed }: Rows<T>) {
  const list = useRef<HTMLUListElement>(null);
  const [near, setNear] = useState<ReadonlySet<number>>(new Set());
  const [focused, setFocused] = useState<string>();
  const [heights, setHeights] = useState<ReadonlyMap<number, { rows: number; pixels: number }>>(new Map());
  const chunkCount = Math.ceil(items.length / ROWS_PER_CHUNK);

  useLayoutEffect(() => {
    if (!viewport || !windowed || !list.current) return;
    const spacers = list.current.querySelectorAll<HTMLLIElement>('[data-explorer-spacer]');
    const visibility = new IntersectionObserver(entries => {
      setNear(previous => {
        const next = new Set(previous);
        let changed = false;
        for (const entry of entries) {
          const chunk = Number((entry.target as HTMLLIElement).dataset.explorerSpacer);
          if (entry.isIntersecting === next.has(chunk)) continue;
          if (entry.isIntersecting) next.add(chunk);
          else next.delete(chunk);
          changed = true;
        }
        return changed ? next : previous;
      });
    }, { root: viewport, rootMargin: NEAR_VIEWPORT });
    const sizes = new ResizeObserver(entries => {
      setHeights(previous => {
        const next = new Map(previous);
        let changed = false;
        for (const entry of entries) {
          const spacer = entry.target as HTMLLIElement;
          const pixels = entry.contentRect.height;
          if (spacer.dataset.windowVisible !== 'true' || pixels <= 0) continue;
          const chunk = Number(spacer.dataset.explorerSpacer);
          const rows = Number(spacer.dataset.windowRows);
          const measured = next.get(chunk);
          if (measured?.rows === rows && measured.pixels === pixels) continue;
          next.set(chunk, { rows, pixels });
          changed = true;
        }
        return changed ? next : previous;
      });
    });
    for (const spacer of spacers) {
      visibility.observe(spacer);
      sizes.observe(spacer);
    }
    return () => {
      visibility.disconnect();
      sizes.disconnect();
    };
  }, [chunkCount, viewport, windowed]);

  const children: ReactElement[] = [];
  let gridRow = 1;
  // Each row is keyed in the same list even when earlier updates move it across a chunk boundary.
  // Spacer items span those grid rows, keeping the observer's area and the list height while the rows are absent.
  for (let start = 0; start < items.length; start += ROWS_PER_CHUNK) {
    const batch = items.slice(start, start + ROWS_PER_CHUNK);
    const chunk = start / ROWS_PER_CHUNK;
    const mounted = !windowed || near.has(chunk) || batch.some(item => {
      const id = idOf(item);
      return id === current || id === focused;
    });
    const firstGridRow = gridRow;
    const span = mounted ? batch.length : 1;
    gridRow += span;
    if (windowed) {
      const measured = heights.get(chunk);
      const estimate = `calc(${batch.length} * var(--explorer-row-size))`;
      children.push(<li key={`spacer:${chunk}`} role="presentation" aria-hidden="true" className="pointer-events-none"
        data-explorer-spacer={chunk} data-window-rows={batch.length} data-window-visible={mounted}
        style={{ gridRow: `${firstGridRow} / span ${span}`, gridColumn: 1,
          minBlockSize: mounted ? 0 : measured?.rows === batch.length ? measured.pixels : estimate }} />);
    }
    if (!mounted) continue;
    for (const [offset, item] of batch.entries()) {
      const id = idOf(item);
      const position = start + offset + 1;
      children.push(cloneElement(renderRow(item), {
        key: `row:${id}`,
        'aria-posinset': position,
        'aria-setsize': items.length,
        style: windowed ? { gridRow: firstGridRow + offset, gridColumn: 1 } : undefined,
        onFocusCapture: () => setFocused(id),
        onBlurCapture: event => {
          if (!event.currentTarget.contains(event.relatedTarget)) {
            setFocused(previous => previous === id ? undefined : previous);
          }
        },
      }));
    }
  }
  if (items.length === 0) return null;
  return <ul ref={list} role="list" className={windowed ? `${LIST_CLASS} grid grid-cols-1` : LIST_CLASS}>{children}</ul>;
}
