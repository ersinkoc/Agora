// The desktop app's own notifications (ADR-0048 decision 5 and Amendment A; docs/ux/main-window.md, "Sounds"), pure: this device's
// choice, the rules that decide whether the window asks the app for one, and the small typed bridge to the app.
//
// Only inside the desktop app (apps/desktop), whose shell gives the hub's page one command, `notify_agents`, and nothing else. The page
// sends what happened (an agent finished, or needs you), the agent's id and its name made plain; the app writes the words itself from
// fixed phrases, checks everything again, limits the rate and opens the agent when the notification is clicked
// (apps/desktop/src-tauri/src/notify.rs). A plugin's frame is an opaque origin and has no way to the app.
//
// The rules, in order, the sounds' own (net/sounds.ts): never in safe mode (ADR-0114); never when they are off on this device (the
// default) or that kind is off; only while the window is hidden or not focused (a notification while the person is looking at the
// window says nothing they cannot see; so, unlike sounds, there is no "also while in front"); at most one request every 2 s, what came
// meanwhile going with the next one (`gate`), and each agent at most once per change of its state (net/finished.ts).
import { plainName } from './attention';
import { bounded, type Gate, type SoundEvent, type SoundKind, type SoundPresence, inFront } from './sounds';

export interface DesktopNotifyPrefs { on: boolean; finished: boolean; needsYou: boolean }
export const DESKTOP_NOTIFY_DEFAULTS: DesktopNotifyPrefs = Object.freeze({ on: false, finished: true, needsYou: true });

export function parseDesktopNotifyPrefs(raw: unknown): DesktopNotifyPrefs {
  let v: Record<string, unknown> = {};
  try { const o = typeof raw === 'string' ? JSON.parse(raw) : undefined; if (o && typeof o === 'object' && !Array.isArray(o)) v = o as Record<string, unknown>; } catch { /* the default */ }
  const bool = (x: unknown, d: boolean) => (typeof x === 'boolean' ? x : d);
  return { on: bool(v.on, DESKTOP_NOTIFY_DEFAULTS.on), finished: bool(v.finished, DESKTOP_NOTIFY_DEFAULTS.finished), needsYou: bool(v.needsYou, DESKTOP_NOTIFY_DEFAULTS.needsYou) };
}

// Kept in this app's page storage only, like the sounds.
const PREFS_KEY = 'agora.desktop-notify.v1';
type Kv = Pick<Storage, 'getItem' | 'setItem'>;
const local = (): Kv | undefined => { try { return typeof window !== 'undefined' ? window.localStorage : undefined; } catch { return undefined; } };
let prefs: DesktopNotifyPrefs | undefined;
const listeners = new Set<() => void>();
export const desktopNotifyPrefs = {
  get(kv: Kv | undefined = local()): DesktopNotifyPrefs {
    if (!prefs) { let raw: string | undefined; try { raw = kv?.getItem(PREFS_KEY) ?? undefined; } catch { /* the default */ } prefs = parseDesktopNotifyPrefs(raw); }
    return prefs;
  },
  set(change: Partial<DesktopNotifyPrefs>, kv: Kv | undefined = local()) {
    prefs = { ...desktopNotifyPrefs.get(kv), ...change };
    try { kv?.setItem(PREFS_KEY, JSON.stringify(prefs)); } catch { /* holds for this visit */ }
    listeners.forEach((l) => l());
  },
  subscribe(l: () => void) { listeners.add(l); return () => { listeners.delete(l); }; },
  reset() { prefs = undefined; listeners.forEach((l) => l()); },
};

// Whether this event may ask the app for a notification, by the rules above (the rate is `gate`).
export function notifiable(ev: SoundEvent, p: DesktopNotifyPrefs, presence: SoundPresence, safe: boolean): boolean {
  if (safe || !p.on) return false;
  if (ev.kind === 'finished' ? !p.finished : !p.needsYou) return false;
  return !inFront(presence);
}

// At most one request in this long; the app holds to the same gap itself.
export const NOTIFY_GAP_MS = 2000;

// Events that may notify (already `notifiable`) at `nowMs`: all of them go now when the gap since the last request is over, as one
// request; otherwise they wait for the end of the gap (`waitMs`, then `flush`). Unlike a sound, nothing of a burst is dropped here: the
// app makes one notification of it, naming the one agent or counting several.
export function gate(g: Gate, evs: readonly SoundEvent[], nowMs: number): { send?: SoundEvent[]; waitMs?: number; next: Gate } {
  const all = bounded([...g.pending, ...evs]);
  if (all.length === 0) return { next: g };
  if (g.lastMs === undefined || nowMs - g.lastMs >= NOTIFY_GAP_MS) return { send: all, next: { lastMs: nowMs, pending: [] } };
  return { waitMs: g.lastMs + NOTIFY_GAP_MS - nowMs, next: { lastMs: g.lastMs, pending: all } };
}

// At the end of the gap: what waited, checked again against the rules as they are now.
export function flush(g: Gate, nowMs: number, still: (ev: SoundEvent) => boolean): { send?: SoundEvent[]; waitMs?: number; next: Gate } {
  return gate({ lastMs: g.lastMs, pending: [] }, g.pending.filter(still), nowMs);
}

// ---- The bridge ---------------------------------------------------------------------------------------------------------------------

// What the app's command takes (apps/desktop/src-tauri/src/main.rs, `PageEvent`): the kind, the agent's hub-wide id and, only when it is
// plain, its name (`plainName`: the app checks it again and shows "An agent" otherwise).
export interface ShellEvent { kind: SoundKind; agent: string; name?: string }
// The longest id sent; the app takes no longer one.
export const AGENT_ID_MAX = 512;

export function shellEvents(evs: readonly SoundEvent[], nameOf: (agent: string) => string | undefined): ShellEvent[] {
  const out: ShellEvent[] = [];
  for (const ev of evs) {
    if (ev.agent === undefined || ev.agent.length === 0 || ev.agent.length > AGENT_ID_MAX) continue;
    const name = plainName(nameOf(ev.agent));
    out.push(name ? { kind: ev.kind, agent: ev.agent, name } : { kind: ev.kind, agent: ev.agent });
  }
  return out;
}

// The one way to the app, present only inside the desktop app: Tauri's own `invoke`, which its shell puts in the page's top frame (never
// in a frame of a plugin, which is opaque and has none).
type Invoke = (cmd: string, args: Record<string, unknown>) => Promise<unknown>;
interface ShellWindow { isTauri?: unknown; __TAURI_INTERNALS__?: { invoke?: unknown } }
const shellInvoke = (w: unknown): Invoke | undefined => {
  if (!w || typeof w !== 'object') return undefined;
  const s = w as ShellWindow;
  const invoke = s.__TAURI_INTERNALS__?.invoke;
  return s.isTauri === true && typeof invoke === 'function' ? (invoke as Invoke) : undefined;
};

/** This page runs inside the desktop app. */
export const inDesktopApp = (w: unknown = typeof window !== 'undefined' ? window : undefined): boolean => shellInvoke(w) !== undefined;

/** Asks the app for one notification of these events; nothing when not in the app or nothing to say, and an error is ignored. */
export function sendToApp(events: readonly ShellEvent[], w: unknown = typeof window !== 'undefined' ? window : undefined): boolean {
  const invoke = shellInvoke(w);
  if (!invoke || events.length === 0) return false;
  try { void Promise.resolve(invoke('notify_agents', { events })).catch(() => { /* an older app, or not allowed: nothing */ }); } catch { return false; }
  return true;
}
