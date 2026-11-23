import { useLayoutEffect, useRef, useState, type ReactNode } from 'react';

// Keep group headers in the tree. Only the lists of agents below them are windowed, in small batches.
const ROWS_PER_CHUNK = 12;
const NEAR_VIEWPORT = '240px 0px';
const LIST_CLASS = 'm-0 list-none p-0';

type Rows<T> = {
  items: readonly T[];
  idOf: (item: T) => string;
  renderRow: (item: T) => ReactNode;
  current?: string;
  viewport: HTMLElement | null;
  windowed: boolean;
};

function Chunk<T>({ items, idOf, renderRow, current, viewport, windowed }: Rows<T>) {
  const element = useRef<HTMLDivElement>(null);
  const [near, setNear] = useState(false);
  const [focused, setFocused] = useState(false);
  const [height, setHeight] = useState<{ rows: number; pixels: number }>();
  const selected = current !== undefined && items.some(item => idOf(item) === current);
  const mounted = !windowed || near || focused || selected;

  useLayoutEffect(() => {
    const chunk = element.current;
    if (!chunk || !viewport || !windowed) return;
    const observer = new IntersectionObserver(([entry]) => setNear(entry?.isIntersecting ?? false), {
      root: viewport,
      rootMargin: NEAR_VIEWPORT,
    });
    observer.observe(chunk);
    return () => observer.disconnect();
  }, [viewport, windowed]);

  useLayoutEffect(() => {
    const chunk = element.current;
    if (!chunk || !mounted || !windowed) return;
    const observer = new ResizeObserver(([entry]) => {
      const pixels = entry?.contentRect.height ?? 0;
      if (pixels > 0) setHeight(previous => previous?.rows === items.length && previous.pixels === pixels
        ? previous
        : { rows: items.length, pixels });
    });
    observer.observe(chunk);
    return () => observer.disconnect();
  }, [mounted, items.length, windowed]);

  // Keep the real height after the first visit. Before that, estimate from a typical row or the touch target size.
  const blockSize = height?.rows === items.length ? height.pixels : `calc(${items.length} * var(--explorer-row-size))`;
  return <div ref={element} aria-hidden={mounted ? undefined : true} style={mounted ? undefined : { blockSize }}
    onFocusCapture={() => setFocused(true)}
    onBlurCapture={event => {
      if (!event.currentTarget.contains(event.relatedTarget)) setFocused(false);
    }}>
    {mounted && <ul className={LIST_CLASS}>{items.map(renderRow)}</ul>}
  </div>;
}

export function ExplorerRows<T>(props: Rows<T>) {
  const { items, idOf } = props;

  const chunks = [];
  for (const [start, item] of items.entries()) {
    if (start % ROWS_PER_CHUNK === 0) {
      chunks.push(<Chunk key={idOf(item)} {...props} items={items.slice(start, start + ROWS_PER_CHUNK)} />);
    }
  }
  return <>{chunks}</>;
}
