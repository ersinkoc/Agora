// What the window had open: the tabs, which one is in front, one terminal or side by side, which panels are shown. Kept outside the
// window component, so going to Full size, Machines or This device and back finds it as it was; also remembered across reloads in
// this browser (a failure to store it only costs the memory).
export interface Layout {
  pinned: string[];
  active?: string;
  layout: 'single' | 'grid';
  touched: boolean; // the user chose the tabs themselves: stop opening agents for them
  showAgents?: boolean;
  showDetails?: boolean;
  leftWidth?: number;
  rightWidth?: number;
}
const KEY = 'agora.layout';
let current: Layout | undefined;

export function loadLayout(): Layout | undefined {
  if (current) return current;
  try {
    const v = JSON.parse(window.localStorage.getItem(KEY) ?? 'null');
    if (v && Array.isArray(v.pinned)) {
      current = {
        pinned: v.pinned.filter((x: unknown) => typeof x === 'string').slice(0, 24),
        active: typeof v.active === 'string' ? v.active : undefined,
        layout: v.layout === 'grid' ? 'grid' : 'single',
        touched: !!v.touched,
        showAgents: typeof v.showAgents === 'boolean' ? v.showAgents : undefined,
        showDetails: typeof v.showDetails === 'boolean' ? v.showDetails : undefined,
        leftWidth: typeof v.leftWidth === 'number' ? v.leftWidth : undefined,
        rightWidth: typeof v.rightWidth === 'number' ? v.rightWidth : undefined,
      };
    }
  } catch { /* nothing remembered */ }
  return current;
}
export function saveLayout(next: Layout) {
  current = next;
  try { window.localStorage.setItem(KEY, JSON.stringify(next)); } catch { /* holds for this visit */ }
}
