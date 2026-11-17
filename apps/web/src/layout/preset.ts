import { PRESETS, type Preset } from './engine';
import type { LayoutNode } from './tree';

// The pictogram of each preset and the way to tell which one a layout is. Pure: nothing here knows the DOM or a pane's contents.

// A preset as rectangles in a 40 × 28 box (what the layout menu draws), and as the shape of its tree: `p` is a pane or a tab
// group, `r(...)` a row of children and `c(...)` a column.
export const PICTOGRAM_BOX = { width: 40, height: 28 } as const;
type Rect = readonly [x: number, y: number, w: number, h: number];

const grid = (cols: number, rows: number): Rect[] => {
  const gap = 2, pad = 2;
  const w = (PICTOGRAM_BOX.width - 2 * pad - gap * (cols - 1)) / cols;
  const h = (PICTOGRAM_BOX.height - 2 * pad - gap * (rows - 1)) / rows;
  return Array.from({ length: cols * rows }, (_, i) => [pad + (i % cols) * (w + gap), pad + Math.floor(i / cols) * (h + gap), w, h] as const);
};

const focusRects = (): Rect[] => {
  const h = (PICTOGRAM_BOX.height - 4 - 2 * 2) / 3;
  return [[2, 2, 24, 24], ...[0, 1, 2].map((i) => [28, 2 + i * (h + 2), 10, h] as const)];
};

export const pictogram = (preset: Preset): readonly Rect[] => (preset === 'focus' ? focusRects() : grid(...(preset.split('x').map(Number) as [number, number])));

const SHAPES: Record<Preset, string> = {
  '1x1': 'p', '2x1': 'r(p,p)', '1x2': 'c(p,p)', '2x2': 'c(r(p,p),r(p,p))', '3x2': 'c(r(p,p,p),r(p,p,p))', focus: 'r(p,c(p,p,p))',
};

const shapeOf = (n: LayoutNode): string => (n.type === 'split' ? `${n.direction === 'row' ? 'r' : 'c'}(${n.children.map(shapeOf).join(',')})` : 'p');

/** The preset a layout has the shape of (whatever the panes show), or undefined for a layout made by hand. */
export const presetOf = (root: LayoutNode): Preset | undefined => PRESETS.find((p) => SHAPES[p] === shapeOf(root));
