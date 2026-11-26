// The theme packs the person imported (ADR-0113 decision 6, step 4; docs/ux/themes.md "Keeping themes the same on your devices"). A pack
// is kept as the file it was, so the person's other devices can offer it: there it opens in the same Import pack preview, and nothing of
// it is applied until the person picks its parts, as for any pack. Each kept pack has a lasting id across devices; a forgotten one leaves
// a dated mark, so another device does not bring it back. Kept in this browser (and through the hub by sync.ts); reading never throws.
import { SAFE } from '../plugins/safe';
import { type Pack, readPack } from './pack';
import { type Removed, newUid, removedOf, timeOf, UID } from './store';

export const PACKS_KEY = 'agora.packs.v1';
export const PACKS_REMOVED_KEY = 'agora.packs.removed.v1';
/** The most packs kept: a ninth replaces the one kept longest ago. */
export const MAX_KEPT_PACKS = 8;

type KV = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;
export interface KeptPack { uid: string; added: number; file: Record<string, unknown>; pack: Pack }
export interface PackSnapshot { packs: KeptPack[]; removed: Removed[] }

function read(kv: KV | undefined, key: string): unknown {
  try { const v = kv?.getItem(key); return v ? JSON.parse(v) : undefined; } catch { return undefined; }
}
function write(kv: KV | undefined, key: string, value: unknown) {
  try { if (value === undefined) kv?.removeItem(key); else kv?.setItem(key, JSON.stringify(value)); } catch { /* holds for this visit */ }
}
const isObject = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);

/** One kept pack as stored or synced, read again with the pack rules; `undefined` when its id or its file is not one. */
export function keptOf(v: unknown): KeptPack | undefined {
  if (!isObject(v) || typeof v.uid !== 'string' || !UID.test(v.uid) || !isObject(v.file)) return undefined;
  const r = readPack(v.file);
  return r.ok ? { uid: v.uid, added: timeOf(v.added), file: v.file, pack: r.pack } : undefined;
}

export function createPackStore(kv?: KV) {
  const load = (): KeptPack[] => {
    const list = read(kv, PACKS_KEY);
    const out: KeptPack[] = [];
    if (Array.isArray(list)) for (const e of list.slice(0, MAX_KEPT_PACKS)) { const k = keptOf(e); if (k && !out.some((o) => o.uid === k.uid)) out.push(k); }
    return out;
  };
  let packs = load();
  let removed = removedOf(read(kv, PACKS_REMOVED_KEY));
  const listeners = new Set<() => void>();
  const notify = () => listeners.forEach((l) => l());
  const persist = () => {
    write(kv, PACKS_KEY, packs.length ? packs.map((p) => ({ uid: p.uid, added: p.added, file: p.file })) : undefined);
    write(kv, PACKS_REMOVED_KEY, removed.length ? removed : undefined);
  };
  const same = (a: Record<string, unknown>, b: Record<string, unknown>) => JSON.stringify(a) === JSON.stringify(b);
  return {
    list: () => packs,
    subscribe: (l: () => void) => { listeners.add(l); return () => { listeners.delete(l); }; },
    // Without storage (safe mode) nothing stored is read, so the packs kept this visit stay.
    reload: () => { if (!kv) return; packs = load(); removed = removedOf(read(kv, PACKS_REMOVED_KEY)); notify(); },
    /** Keeps a pack the person imported (the same file once). The ninth replaces the one kept longest ago, which is marked as forgotten. */
    keep(file: Record<string, unknown>): KeptPack | undefined {
      const have = packs.find((p) => same(p.file, file));
      if (have) return have;
      const r = readPack(file);
      if (!r.ok) return undefined;
      const kept: KeptPack = { uid: newUid(), added: Date.now(), file, pack: r.pack };
      let next = [...packs, kept];
      if (next.length > MAX_KEPT_PACKS) {
        const oldest = next.reduce((a, b) => (b.added < a.added ? b : a));
        removed = removedOf([{ uid: oldest.uid, at: Date.now() }, ...removed]);
        next = next.filter((p) => p !== oldest);
      }
      packs = next;
      persist();
      notify();
      return kept;
    },
    /** Forgets a pack here and, through the hub, on the person's other devices (its themes, layouts and keys stay where they were added). */
    forget(uid: string) {
      if (!UID.test(uid)) return;
      packs = packs.filter((p) => p.uid !== uid);
      removed = removedOf([{ uid, at: Date.now() }, ...removed]);
      persist();
      notify();
    },
    syncSnapshot: (): PackSnapshot => ({ packs: [...packs], removed: [...removed] }),
    /** A merge's result, kept as given (sync.ts read every pack with the pack rules). */
    applySync(next: PackSnapshot) {
      packs = next.packs.slice(0, MAX_KEPT_PACKS);
      removed = removedOf(next.removed);
      persist();
      notify();
    },
  };
}
export type PackStore = ReturnType<typeof createPackStore>;

// ---- The page's store ---------------------------------------------------------------------------------------------------------
function storage(): KV | undefined {
  try { return typeof window !== 'undefined' ? window.localStorage : undefined; } catch { return undefined; }
}
// In safe mode (ADR-0114) nothing stored is read and nothing is kept past the visit, as for themes.
export const packStore: PackStore = createPackStore(SAFE ? undefined : storage());

/** Another tab's kept packs apply here at once, as for themes. */
export function initKeptPacks() {
  window.addEventListener('storage', (e) => { if (e.key === null || e.key === PACKS_KEY || e.key === PACKS_REMOVED_KEY) packStore.reload(); });
}
