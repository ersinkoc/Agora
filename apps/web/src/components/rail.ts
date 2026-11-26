import type { IconName } from './Icon';

// The right panel's icon rail: which subjects it offers, in what order, and the one remembered between visits. Its keys (Alt+Shift+1 to 5
// by place) are the side panel's commands, keys/commands.ts.
// Pure, so it is tested without a page.

export type RailTab = 'needs' | 'changes' | 'board' | 'messages' | 'usage';

export interface RailItem { id: RailTab; icon: IconName; labelKey: string; badge?: number; alert?: boolean }

export interface RailCounts { needs: number; changed: number; messages: number; hasBoard: boolean; hasUsage: boolean }

// The rail's items, in a fixed order so a key always means the same place. Board and Usage appear only when there is something.
export function railItems(c: RailCounts): RailItem[] {
  return [
    { id: 'needs', icon: 'bell', labelKey: 'attention-title', badge: c.needs, alert: true },
    { id: 'changes', icon: 'diff', labelKey: 'ide-tab-changes', badge: c.changed },
    ...(c.hasBoard ? [{ id: 'board' as const, icon: 'board' as const, labelKey: 'board-title' }] : []),
    { id: 'messages', icon: 'chat', labelKey: 'ide-tab-messages', badge: c.messages },
    ...(c.hasUsage ? [{ id: 'usage' as const, icon: 'gauge' as const, labelKey: 'ide-tab-usage' }] : []),
  ];
}

// A badge reads as a number up to 99, then "99+": a long count never widens the rail.
export const badgeText = (n: number): string => (n > 99 ? '99+' : String(n));

// The remembered subject is only used while the rail still offers it (a board that went away falls back to Needs you).
export function resolveTab(wanted: RailTab | undefined, items: readonly RailItem[]): RailTab {
  return wanted && items.some((x) => x.id === wanted) ? wanted : 'needs';
}

// `cost` is the old name of `usage`: a device that saved it still lands on the same subject.
const KEY = 'agora.rail.tab';
const TABS: readonly RailTab[] = ['needs', 'changes', 'board', 'messages', 'usage'];
export function loadRailTab(kv: Pick<Storage, 'getItem'> | undefined = storage()): RailTab | undefined {
  try { const v = kv?.getItem(KEY); return v === 'cost' ? 'usage' : TABS.find((x) => x === v); } catch { return undefined; }
}
export function saveRailTab(tab: RailTab, kv: Pick<Storage, 'setItem'> | undefined = storage()): void {
  try { kv?.setItem(KEY, tab); } catch { /* holds for this visit */ }
}
function storage(): Storage | undefined {
  try { return typeof window !== 'undefined' ? window.localStorage : undefined; } catch { return undefined; }
}

// A rail button's accessible name carries its count ("Needs you, 1"): the badge itself is only decoration.
export const railLabel = (text: string, badge?: number): string => (badge ? `${text}, ${badge}` : text);
