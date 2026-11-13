// Where each part of the tree is drawn, as fractions of the workspace. Panes are drawn flat (one list keyed by pane id, not
// nested in their splits), so a resize, a swap or a move changes only positions and never re-creates a terminal.
import type { LayoutNode, Pane } from './tree';

export interface Rect { x: number; y: number; w: number; h: number }
// One tab group, or one pane on its own: a bar on top and the pane in front below it.
export interface Frame { key: string; rect: Rect; panes: Pane[]; active: number; tabs: boolean }
// The line between child `index` and `index + 1` of the split at `path`. `start` is where child `index` begins inside the split
// and `ratio` its share, both as fractions of the split.
export interface Divider { path: number[]; index: number; direction: 'row' | 'column'; split: Rect; start: number; ratio: number; next: number }

export function arrange(node: LayoutNode, rect: Rect = { x: 0, y: 0, w: 1, h: 1 }, path: number[] = []): { frames: Frame[]; dividers: Divider[] } {
  if (node.type === 'pane') return { frames: [{ key: node.id, rect, panes: [node], active: 0, tabs: false }], dividers: [] };
  if (node.type === 'tabs') return { frames: [{ key: `tabs:${node.children[0]?.id ?? ''}`, rect, panes: node.children, active: node.active, tabs: true }], dividers: [] };
  const frames: Frame[] = [];
  const dividers: Divider[] = [];
  let start = 0;
  node.children.forEach((child, i) => {
    const ratio = node.ratios[i] as number;
    const r = node.direction === 'row'
      ? { x: rect.x + start * rect.w, y: rect.y, w: ratio * rect.w, h: rect.h }
      : { x: rect.x, y: rect.y + start * rect.h, w: rect.w, h: ratio * rect.h };
    const inner = arrange(child, r, [...path, i]);
    frames.push(...inner.frames);
    dividers.push(...inner.dividers);
    if (i + 1 < node.children.length) dividers.push({ path, index: i, direction: node.direction, split: rect, start, ratio, next: node.ratios[i + 1] as number });
    start += ratio;
  });
  return { frames, dividers };
}

// The share child `index` should get for a pointer at `pos` (a fraction along the split), as a change to its current share.
export const dragDelta = (d: Divider, pos: number) => pos - d.start - d.ratio;

// Where a drop lands on a frame: near an edge (a quarter of its size) beside it, in the middle as a tab. `x` and `y` are
// fractions of the frame; `x` already follows the reading direction (0 is the start edge).
export function dropPlace(x: number, y: number): 'center' | 'start' | 'end' | 'top' | 'bottom' {
  const edges = [['start', x], ['end', 1 - x], ['top', y], ['bottom', 1 - y]] as const;
  const [place, dist] = edges.reduce((a, b) => (b[1] < a[1] ? b : a));
  return dist < 0.25 ? place : 'center';
}
