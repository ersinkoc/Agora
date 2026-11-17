// How this viewer groups the fleet console's tiles: by machine or by project. Remembered in this browser only (a viewer's own choice,
// like the text size); storage that is blocked or full only means the choice holds for this visit.
import type { GroupBy } from './fleet';

const KEY = 'agora.fleet.group';
let current: GroupBy | undefined;

export const parseGroupBy = (raw: unknown): GroupBy => (raw === 'project' ? 'project' : 'machine');

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
}

// For tests: forget what this visit held.
export const resetGroupBy = () => { current = undefined; };

function storage(): Storage | undefined {
  try { return typeof window !== 'undefined' ? window.localStorage : undefined; } catch { return undefined; }
}
