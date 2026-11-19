// A row of tabs that is wider than its space: which edge has more to scroll to (for the fade), and which tabs are cut off or hidden (for
// the "more" menu). Pure, so it is tested without a page; the component feeds it measurements.

export interface Scroll { scrollLeft: number; clientWidth: number; scrollWidth: number }

// `scrollLeft` is negative in a right-to-left row in most browsers: only its size matters. A pixel of slack absorbs rounding.
export function scrollEdges(s: Scroll): { start: boolean; end: boolean; overflowing: boolean } {
  const at = Math.abs(s.scrollLeft);
  const overflowing = s.scrollWidth - s.clientWidth > 1;
  return { overflowing, start: overflowing && at > 1, end: overflowing && at + s.clientWidth < s.scrollWidth - 1 };
}

export interface Span { id: string; left: number; right: number }

// The tabs that are not wholly inside the strip, in their order. Screen coordinates, so a mirrored row needs no special case.
export function hiddenTabs(view: { left: number; right: number }, tabs: readonly Span[]): string[] {
  return tabs.filter((x) => x.left < view.left - 1 || x.right > view.right + 1).map((x) => x.id);
}
