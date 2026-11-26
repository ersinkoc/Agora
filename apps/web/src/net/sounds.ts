// Sounds when an agent finishes or needs you (docs/ux/main-window.md, "Sounds"), pure: this device's choices, and the rules that decide
// whether a sound plays and when. The audio itself is components/chime.ts; what changed is net/finished.ts.
//
// The rules, in order: never in safe mode (ADR-0114); never when sounds are off on this device (the default), when that sound is off or when
// the agent is muted; never for the agent in the focused pane while the window is in front; only while the window is hidden or not focused,
// unless the person asked for sounds while it is in front too. A burst is one sound: at most one a second, the most urgent of the burst
// (needs you before finished), and each agent at most once per change of its state (net/finished.ts makes one event per change).

export type SoundKind = 'finished' | 'needs-you';
export const SOUND_KINDS: readonly SoundKind[] = ['finished', 'needs-you'];
export type SoundVolume = 'low' | 'medium' | 'high';
export const SOUND_VOLUMES: readonly SoundVolume[] = ['low', 'medium', 'high'];

export interface SoundPrefs {
  on: boolean;
  volume: SoundVolume;
  finished: boolean;
  needsYou: boolean;
  // Also while the window is in front (never for the agent in the focused pane).
  inFront: boolean;
}
export const SOUND_DEFAULTS: SoundPrefs = Object.freeze({ on: false, volume: 'medium', finished: true, needsYou: true, inFront: false });

// What is stored, read back whatever it holds: anything unknown is the default.
export function parseSoundPrefs(raw: unknown): SoundPrefs {
  let v: Record<string, unknown> = {};
  try { const o = typeof raw === 'string' ? JSON.parse(raw) : undefined; if (o && typeof o === 'object' && !Array.isArray(o)) v = o as Record<string, unknown>; } catch { /* the default */ }
  const bool = (x: unknown, d: boolean) => (typeof x === 'boolean' ? x : d);
  return {
    on: bool(v.on, SOUND_DEFAULTS.on),
    volume: SOUND_VOLUMES.includes(v.volume as SoundVolume) ? (v.volume as SoundVolume) : SOUND_DEFAULTS.volume,
    finished: bool(v.finished, SOUND_DEFAULTS.finished),
    needsYou: bool(v.needsYou, SOUND_DEFAULTS.needsYou),
    inFront: bool(v.inFront, SOUND_DEFAULTS.inFront),
  };
}

// The agents muted here, by hub-wide key (`<machine>/<agent>`), newest last. Bounded, so a long-lived browser does not grow it without end;
// the oldest mute goes first.
export const MUTED_MAX = 500;
export function parseMuted(raw: unknown): string[] {
  try {
    const o = typeof raw === 'string' ? JSON.parse(raw) : undefined;
    return Array.isArray(o) ? [...new Set(o.filter((x): x is string => typeof x === 'string' && x.length > 0 && x.length <= 512))].slice(-MUTED_MAX) : [];
  } catch { return []; }
}

// Both belong to this browser only, like the attention choices: storage that is blocked or full means they hold for this visit.
const PREFS_KEY = 'agora.sounds.v1';
const MUTED_KEY = 'agora.sounds.muted.v1';
type Kv = Pick<Storage, 'getItem' | 'setItem'>;
const local = (): Kv | undefined => { try { return typeof window !== 'undefined' ? window.localStorage : undefined; } catch { return undefined; } };
const read = (key: string, kv: Kv | undefined): string | undefined => { try { return kv?.getItem(key) ?? undefined; } catch { return undefined; } };
const write = (key: string, value: string, kv: Kv | undefined) => { try { kv?.setItem(key, value); } catch { /* holds for this visit */ } };

let prefs: SoundPrefs | undefined;
let muted: readonly string[] | undefined;
const listeners = new Set<() => void>();
const emit = () => listeners.forEach((l) => l());
export const soundPrefs = {
  get: (kv: Kv | undefined = local()): SoundPrefs => (prefs ??= parseSoundPrefs(read(PREFS_KEY, kv))),
  set(change: Partial<SoundPrefs>, kv: Kv | undefined = local()) {
    prefs = { ...soundPrefs.get(kv), ...change };
    write(PREFS_KEY, JSON.stringify(prefs), kv);
    emit();
  },
  muted: (kv: Kv | undefined = local()): readonly string[] => (muted ??= parseMuted(read(MUTED_KEY, kv))),
  isMuted: (agent: string, kv: Kv | undefined = local()): boolean => soundPrefs.muted(kv).includes(agent),
  setMuted(agent: string, on: boolean, kv: Kv | undefined = local()) {
    const now = soundPrefs.muted(kv).filter((x) => x !== agent);
    muted = on ? [...now, agent].slice(-MUTED_MAX) : now;
    write(MUTED_KEY, JSON.stringify(muted), kv);
    emit();
  },
  subscribe(l: () => void) { listeners.add(l); return () => { listeners.delete(l); }; },
  // For tests: forget what this visit held.
  reset() { prefs = undefined; muted = undefined; emit(); },
};

// One event: a sound that may play. `agent` is the hub-wide key of the agent it is for (an event for no agent is never muted by one).
export interface SoundEvent { kind: SoundKind; agent?: string }
// Where the person is: the window shown and focused, and the agent in its focused pane (if any).
export interface SoundPresence { visible: boolean; focused: boolean; viewing?: string }

export const inFront = (p: SoundPresence): boolean => p.visible && p.focused;

// Whether this event may make a sound at all, by the rules above (the burst rule is `gate`).
export function audible(ev: SoundEvent, p: SoundPrefs, mutedAgents: readonly string[], presence: SoundPresence, safe: boolean): boolean {
  if (safe || !p.on) return false;
  if (ev.kind === 'finished' ? !p.finished : !p.needsYou) return false;
  if (ev.agent !== undefined && mutedAgents.includes(ev.agent)) return false;
  if (inFront(presence)) {
    if (ev.agent !== undefined && presence.viewing === ev.agent) return false;
    if (!p.inFront) return false;
  }
  return true;
}

// The most urgent of a burst: needs you before finished.
export const URGENCY: Record<SoundKind, number> = { 'needs-you': 2, finished: 1 };
export const mostUrgent = (evs: readonly SoundEvent[]): SoundEvent | undefined =>
  evs.reduce<SoundEvent | undefined>((best, e) => (!best || URGENCY[e.kind] > URGENCY[best.kind] ? e : best), undefined);

// At most one sound in this long.
export const SOUND_GAP_MS = 1000;
// What waits for the end of the gap, bounded (only the most urgent of it plays).
const PENDING_MAX = 32;

// The burst rule's memory: when the last sound played, and what came during the gap since.
export interface Gate { lastMs?: number; pending: readonly SoundEvent[] }
export const openGate = (): Gate => ({ pending: [] });

// Events that may sound (already `audible`) at `nowMs`: play the most urgent now when the gap since the last sound is over; otherwise they
// wait, and `waitMs` says when to look again (`flush`). Decided from the times alone, so a timer slowed in a hidden tab cannot add a sound.
export function gate(g: Gate, evs: readonly SoundEvent[], nowMs: number): { play?: SoundEvent; waitMs?: number; next: Gate } {
  const all = [...g.pending, ...evs].slice(-PENDING_MAX);
  if (all.length === 0) return { next: g };
  if (g.lastMs === undefined || nowMs - g.lastMs >= SOUND_GAP_MS) return { play: mostUrgent(all), next: { lastMs: nowMs, pending: [] } };
  return { waitMs: g.lastMs + SOUND_GAP_MS - nowMs, next: { lastMs: g.lastMs, pending: all } };
}

// At the end of the gap: what waited, checked again against the rules as they are now (the person may have come back meanwhile, or muted
// the agent), then played as one sound.
export function flush(g: Gate, nowMs: number, stillAudible: (ev: SoundEvent) => boolean): { play?: SoundEvent; waitMs?: number; next: Gate } {
  return gate({ lastMs: g.lastMs, pending: [] }, g.pending.filter(stillAudible), nowMs);
}
