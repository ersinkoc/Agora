// Where a dropdown goes: pure, given the trigger's rectangle, the menu's natural size and the visible viewport.
// It opens below the trigger when it fits, flips above when there is more room there, is clamped inside the
// viewport on both axes and gets a height cap so that it scrolls inside instead of leaving the screen.
export interface Box { left: number; top: number; right: number; bottom: number }
export interface Size { width: number; height: number }
export interface Placement { left: number; top: number; maxHeight: number; side: 'below' | 'above' }

const GAP = 4;
const EDGE = 8;

export function placeMenu(trigger: Box, menu: Size, viewport: Box, align: 'start' | 'end' = 'start', rtl = false): Placement {
  const vw = viewport.right - viewport.left;
  const below = viewport.bottom - trigger.bottom - GAP - EDGE;
  const above = trigger.top - viewport.top - GAP - EDGE;
  // Below wins whenever the whole menu fits there; otherwise the roomier side does.
  const side = menu.height <= below || below >= above ? 'below' : 'above';
  const room = Math.max(0, side === 'below' ? below : above);
  const height = Math.min(menu.height, room);
  const top = side === 'below' ? trigger.bottom + GAP : trigger.top - GAP - height;
  const width = Math.min(menu.width, Math.max(0, vw - 2 * EDGE));
  const startEdge = (align === 'start') !== rtl; // line up with the trigger's left edge, else its right edge
  const wanted = startEdge ? trigger.left : trigger.right - width;
  const left = Math.min(Math.max(wanted, viewport.left + EDGE), Math.max(viewport.left + EDGE, viewport.right - EDGE - width));
  return { left, top, maxHeight: room, side };
}
