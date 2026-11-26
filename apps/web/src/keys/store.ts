// The person's keybindings file, kept in this browser (docs/ux/keybindings.md). Like themes and layouts it stays here (the hub is not
// involved); when the browser cannot store, a change still holds for the visit. Another tab's change applies here at once. Reading never
// throws: a stored file that cannot be read (another version's, edited by hand) is set aside and the defaults stand.
import { type Bound, EMPTY_FILE, type KeyFile, keymap, readKeys, writeKeys } from './file';

export const KEYS_KEY = 'agora.keybindings.v1';
type KV = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

export interface KeyState {
  file: KeyFile;
  /** Commands the file names that this window does not have (kept, not used). */
  unknown: readonly string[];
  /** The keys in force: the file over the defaults. */
  map: readonly Bound[];
  /** What was stored could not be read, so the defaults stand (the stored text is left as it was until the next change). */
  unreadable: boolean;
}

function stateOf(file: KeyFile, unknown: readonly string[], unreadable = false): KeyState {
  return { file, unknown, map: keymap(file), unreadable };
}

export function loadKeys(kv: KV | undefined): KeyState {
  let text: string | null = null;
  try { text = kv?.getItem(KEYS_KEY) ?? null; } catch { /* the defaults */ }
  if (!text) return stateOf(EMPTY_FILE, []);
  const r = readKeys(text);
  return r.ok ? stateOf(r.file, r.unknown) : stateOf(EMPTY_FILE, [], true);
}

/**
 * `safe`: the window is in safe mode (ADR-0114): the defaults only, and this browser's storage is neither read nor written, so a file
 * that took a key the person needs cannot follow them there; a change made in safe mode holds for that visit only.
 */
export function createKeyStore(env: { kv?: KV; safe?: boolean; events?: Pick<Window, 'addEventListener'> } = {}) {
  const kv = env.safe ? undefined : env.kv;
  let state = loadKeys(kv);
  const listeners = new Set<() => void>();
  const notify = () => listeners.forEach((l) => l());
  const save = (file: KeyFile, unknown: readonly string[]) => {
    state = stateOf(file, unknown);
    try { if (file.entries.length === 0) kv?.removeItem(KEYS_KEY); else kv?.setItem(KEYS_KEY, writeKeys(file)); } catch { /* holds for this visit */ }
    notify();
  };
  env.events?.addEventListener('storage', (e: StorageEvent) => { if (kv && e.key === KEYS_KEY) { state = loadKeys(kv); notify(); } });
  return {
    state: () => state,
    subscribe: (l: () => void) => { listeners.add(l); return () => { listeners.delete(l); }; },
    /** Replaces the file (one already read through `readKeys`, or one the editor made from it). */
    set: (file: KeyFile, unknown: readonly string[] = state.unknown.filter((u) => file.entries.some((e) => e.command === u))) => save(file, unknown),
    /** Every command back to its default keys. */
    reset: () => save(EMPTY_FILE, []),
  };
}
export type KeyStore = ReturnType<typeof createKeyStore>;

const browserKv = (): KV | undefined => { try { return typeof window !== 'undefined' ? window.localStorage : undefined; } catch { return undefined; } };
export const keyStore: KeyStore = createKeyStore({ kv: browserKv(), events: typeof window !== 'undefined' ? window : undefined });
