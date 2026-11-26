// The messages between the window and a plugin's frame (ADR-0114 decision 3; docs/ux/plugins.md, "Messages"). Version 1.
//
// Frame → window (every message is `{ agora: 1, type, … }`):
//   boot                          the host's loader in the frame is ready for the plugin's files (once)
//   read    { id, cap }           one answer for a capability
//   watch   { id, cap }           an answer now and an `update` each time it changes, until `unwatch`
//   unwatch { id }
// Window → frame:
//   load    { plugin, panel, granted, entry, scheme, lang, dir, files }   the checked files (once, in answer to `boot`)
//   result  { id, ok: true, data } | { id, ok: false, error }   error: not-granted | unknown-capability | unavailable | too-many
//   update  { id, data }
//
// The window takes a message only from a frame it made (by `event.source`, as an opaque frame's origin is "null"), at most RATE per
// second (with a burst of BURST), only a plain object of a known type with exactly its fields, a capability name at most MAX_CAP long.
// Anything else is ignored and counted, without serialising or walking it, and stops the frame (STOP_AFTER_RATE).
import { type Capability, isCapability } from './capabilities';

export const PROTOCOL_VERSION = 1;
export const RATE = 20;
export const BURST = 40;
export const MAX_WATCHES = 16;
/** A frame is stopped at its first malformed or oversized message (of no known type or shape, or a field over its bound): reading a
 *  message costs the window as much as the frame put in it (bounded only by memory), and the window's own loader never sends such a
 *  message, so a working plugin is never stopped. Well-formed messages over the rate are not read and only dropped, until this many,
 *  then the frame is stopped too (each one still costs the window the browser's copy of it). */
export const STOP_AFTER_RATE = BURST;

export type Inbound =
  | { type: 'boot' }
  | { type: 'read'; id: number; cap: string }
  | { type: 'watch'; id: number; cap: string }
  | { type: 'unwatch'; id: number };
export type ReadError = 'not-granted' | 'unknown-capability' | 'unavailable' | 'too-many';
export type Outbound =
  | { agora: 1; type: 'load'; plugin: { id: string; name: string; version: string }; panel: string; granted: readonly Capability[]; entry: string; scheme: 'dark' | 'light'; lang: string; dir: 'ltr' | 'rtl'; files: { name: string; type: string; bytes: Uint8Array }[] }
  | { agora: 1; type: 'result'; id: number; ok: true; data: unknown }
  | { agora: 1; type: 'result'; id: number; ok: false; error: ReadError }
  | { agora: 1; type: 'update'; id: number; data: unknown };

/** Why a frame's message was ignored: of no known type or shape, too long, or over the rate. (One from no frame of this window is
 *  counted by the router as a stray.) */
export type DropReason = 'type' | 'size' | 'rate';
export type Counts = Record<DropReason, number>;
export const noCounts = (): Counts => ({ type: 0, size: 0, rate: 0 });

const isId = (v: unknown): v is number => typeof v === 'number' && Number.isInteger(v) && v >= 0 && v <= Number.MAX_SAFE_INTEGER;

/** The most fields an inbound message has; a message with more is refused before anything else is read from it. */
const MAX_KEYS = 4;
/** The longest capability name a message may carry (every name is far shorter). */
export const MAX_CAP = 64;

/** Only a plain object is read at all: a typed array, a Map, a Blob or anything else that is not `{ … }` is refused without walking
 *  or measuring it (a 16 MB array would otherwise be serialised to be measured, and the window would stall on it). */
export const isPlainObject = (v: unknown): v is Record<string, unknown> => {
  if (typeof v !== 'object' || v === null) return false;
  const proto = Object.getPrototypeOf(v);
  return proto === Object.prototype || proto === null;
};

/** The field names of a plain object, or undefined when it has more than MAX_KEYS (the scan stops there). */
function fewKeys(m: Record<string, unknown>): string[] | undefined {
  const keys: string[] = [];
  for (const k in m) {
    if (!Object.prototype.hasOwnProperty.call(m, k)) continue;
    keys.push(k);
    if (keys.length > MAX_KEYS) return undefined;
  }
  return keys.sort();
}

/** A message's type and fields, or why it is refused: `type` for anything that is not exactly one of the inbound messages, `size` for
 *  one of the right shape with a field longer than its bound. Cheap whatever is sent: no serialising, no walking past MAX_KEYS fields. */
export function classify(data: unknown): Inbound | 'type' | 'size' {
  if (!isPlainObject(data)) return 'type';
  const keys = fewKeys(data)?.join(',');
  if (keys === undefined || data.agora !== PROTOCOL_VERSION) return 'type';
  switch (data.type) {
    case 'boot': return keys === 'agora,type' ? { type: 'boot' } : 'type';
    case 'read':
    case 'watch':
      if (keys !== 'agora,cap,id,type' || !isId(data.id) || typeof data.cap !== 'string') return 'type';
      return data.cap.length > MAX_CAP ? 'size' : { type: data.type, id: data.id, cap: data.cap };
    case 'unwatch': return keys === 'agora,id,type' && isId(data.id) ? { type: 'unwatch', id: data.id } : 'type';
    default: return 'type';
  }
}

/** A message's type and fields, or undefined for anything that is not exactly one of the inbound messages within its bounds. */
export function parseInbound(data: unknown): Inbound | undefined {
  const m = classify(data);
  return typeof m === 'string' ? undefined : m;
}

/** A token bucket: RATE messages a second, BURST at once. `now` is in milliseconds. */
export function rateLimiter(rate = RATE, burst = BURST) {
  let tokens = burst;
  let at: number | undefined;
  return (now: number): boolean => {
    if (at !== undefined) tokens = Math.min(burst, tokens + ((now - at) / 1000) * rate);
    at = now;
    if (tokens < 1) return false;
    tokens -= 1;
    return true;
  };
}

/** What a capability answers for a frame: "not granted" for one the person did not grant, "unknown capability" for a name that is not
 *  one, before anything is read. */
export function gate(cap: string, granted: readonly Capability[]): { ok: true; cap: Capability } | { ok: false; error: ReadError } {
  if (!isCapability(cap)) return { ok: false, error: 'unknown-capability' };
  if (!granted.includes(cap)) return { ok: false, error: 'not-granted' };
  return { ok: true, cap };
}

/** One frame's filter: nothing from a stopped frame, then the rate (a message over it is not read), then the shape, which is
 *  cheap whatever is sent. Nothing is serialised to be measured: a message of the right shape is at most a few hundred bytes. */
export function frameFilter(clock: () => number = () => Date.now(), onFlood: () => void = () => {}) {
  const counts = noCounts();
  const allow = rateLimiter();
  return {
    counts,
    /** Whether the frame must be stopped: a malformed or oversized message, or STOP_AFTER_RATE over the rate. */
    get flooding() { return counts.type + counts.size > 0 || counts.rate >= STOP_AFTER_RATE; },
    /** `read` gives the message's data. It is called only after the rate allows the message: a browser copies a message's data out
     *  only when it is read, so a message over the rate costs the window nothing. */
    take(read: () => unknown): Inbound | undefined {
      // A stopped frame's messages still queued are never read: its host takes the frame away a moment later, and every read would
      // copy what the frame put in the message into the window.
      if (counts.type + counts.size > 0 || counts.rate >= STOP_AFTER_RATE) { counts.rate++; return undefined; }
      if (!allow(clock())) { if (++counts.rate === STOP_AFTER_RATE) onFlood(); return undefined; }
      const m = classify(read());
      if (typeof m !== 'string') return m;
      counts[m]++;
      onFlood();
      return undefined;
    },
  };
}
export type FrameFilter = ReturnType<typeof frameFilter>;

/** Where messages from frames arrive. Every frame registers its own window; a message whose `source` is none of them is ignored and
 *  counted, so a page, an extension or another frame cannot speak for a plugin. */
export function messageRouter() {
  const frames = new Map<unknown, { filter: FrameFilter; on: (m: Inbound) => void }>();
  const strays = { count: 0 };
  return {
    strays,
    add(source: unknown, filter: FrameFilter, on: (m: Inbound) => void) {
      frames.set(source, { filter, on });
      return () => { if (frames.get(source)?.on === on) frames.delete(source); };
    },
    /** Handles one `message` event. Returns whether it was taken. */
    receive(e: { source: unknown; data: unknown }): boolean {
      const f = e.source ? frames.get(e.source) : undefined;
      if (!f) { strays.count++; return false; }
      const m = f.filter.take(() => e.data);
      if (!m) return false;
      f.on(m);
      return true;
    },
    get size() { return frames.size; },
  };
}
export type MessageRouter = ReturnType<typeof messageRouter>;
