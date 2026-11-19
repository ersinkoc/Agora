// How this viewer groups the fleet console's tiles: by machine or by project. Remembered in this browser only (a viewer's own choice,
// like the text size); storage that is blocked or full only means the choice holds for this visit.
import { useSyncExternalStore } from 'react';
import { GROUP_BYS, type GroupBy } from './fleet';

const KEY = 'agora.fleet.group';
let current: GroupBy | undefined;

export const parseGroupBy = (raw: unknown): GroupBy => GROUP_BYS.find((g) => g === raw) ?? 'machine';

export function loadGroupBy(kv: Pick<Storage, 'getItem'> | undefined = storage()): GroupBy {
  if (current) return current;
  let raw: unknown;
  try { raw = kv?.getItem(KEY); } catch { raw = undefined; }
  current = parseGroupBy(raw);
  return current;
}

export function saveGroupBy(by: GroupBy, kv: Pick<Storage, 'setItem'> | undefined = storage()) {
  current = by;
  try { kv?.setItem(KEY, by); } catch { /* holds for this visit */ }
  for (const l of listeners) l();
}

// The fleet console and the agent list are one choice: both follow it.
const listeners = new Set<() => void>();
const subscribe = (cb: () => void) => { listeners.add(cb); return () => { listeners.delete(cb); }; };
export const useGroupBy = (): GroupBy => useSyncExternalStore(subscribe, () => loadGroupBy(), () => 'machine');

// For tests: forget what this visit held.
export const resetGroupBy = () => { current = undefined; };

function storage(): Storage | undefined {
  try { return typeof window !== 'undefined' ? window.localStorage : undefined; } catch { return undefined; }
}
