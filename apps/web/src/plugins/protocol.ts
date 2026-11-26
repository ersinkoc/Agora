// The messages between the window and a plugin's frame (ADR-0114 decision 3; docs/ux/plugins.md, "Messages"). Version 1.
//
// Frame → window (every message is `{ agora: 1, type, … }`):
//   boot                          the host's loader in the frame is ready for the plugin's files (once)
//   read    { id, cap }           one answer for a capability
//   watch   { id, cap }           an answer now and an `update` each time it changes, until `unwatch`
//   unwatch { id }
// Window → frame:
//   load    { plugin, panel, granted, entry, files }   the checked files (once, in answer to `boot`)
//   result  { id, ok: true, data } | { id, ok: false, error }   error: not-granted | unknown-capability | unavailable | too-many
//   update  { id, data }
//
// The window takes a message only from a frame it made (by `event.source`, as an opaque frame's origin is "null"), only of a known type
// and shape, at most MAX_MESSAGE_BYTES long and at most RATE per second (with a burst of BURST). Anything else is ignored and counted.
import { type Capability, isCapability } from './capabilities';

export const PROTOCOL_VERSION = 1;
export const MAX_MESSAGE_BYTES = 4096;
export const RATE = 20;
export const BURST = 40;
export const MAX_WATCHES = 16;

export type Inbound =
  | { type: 'boot' }
  | { type: 'read'; id: number; cap: string }
  | { type: 'watch'; id: number; cap: string }
  | { type: 'unwatch'; id: number };
export type ReadError = 'not-granted' | 'unknown-capability' | 'unavailable' | 'too-many';
export type Outbound =
  | { agora: 1; type: 'load'; plugin: { id: string; name: string; version: string }; panel: string; granted: readonly Capability[]; entry: string; scheme: 'dark' | 'light'; files: { name: string; type: string; bytes: Uint8Array }[] }
  | { agora: 1; type: 'result'; id: number; ok: true; data: unknown }
  | { agora: 1; type: 'result'; id: number; ok: false; error: ReadError }
  | { agora: 1; type: 'update'; id: number; data: unknown };

/** Why a frame's message was ignored: of no known type or shape, too long, or over the rate. (One from no frame of this window is
 *  counted by the router as a stray.) */
export type DropReason = 'type' | 'size' | 'rate';
export type Counts = Record<DropReason, number>;
export const noCounts = (): Counts => ({ type: 0, size: 0, rate: 0 });

const isId = (v: unknown): v is number => typeof v === 'number' && Number.isInteger(v) && v >= 0 && v <= Number.MAX_SAFE_INTEGER;

/** The size of a message as JSON, or undefined for one that is not plain data (a cycle, a function, a transferable). */
export function sizeOf(data: unknown): number | undefined {
  try {
    const s = JSON.stringify(data);
    return typeof s === 'string' ? s.length : undefined;
  } catch { return undefined; }
}

/** A message's type and fields, or undefined for anything that is not exactly one of the inbound messages. */
export function parseInbound(data: unknown): Inbound | undefined {
  if (typeof data !== 'object' || data === null || Array.isArray(data)) return undefined;
  const m = data as Record<string, unknown>;
  if (m.agora !== PROTOCOL_VERSION) return undefined;
  const keys = Object.keys(m).sort().join(',');
  switch (m.type) {
    case 'boot': return keys === 'agora,type' ? { type: 'boot' } : undefined;
    case 'read':
    case 'watch':
      return keys === 'agora,cap,id,type' && isId(m.id) && typeof m.cap === 'string' && m.cap.length <= 64 ? { type: m.type, id: m.id, cap: m.cap } : undefined;
    case 'unwatch': return keys === 'agora,id,type' && isId(m.id) ? { type: 'unwatch', id: m.id } : undefined;
    default: return undefined;
  }
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

/** One frame's filter: size, then shape and type, then rate (a message that is not one of ours never spends the frame's rate). */
export function frameFilter(clock: () => number = () => Date.now()) {
  const counts = noCounts();
  const allow = rateLimiter();
  return {
    counts,
    take(data: unknown): Inbound | undefined {
      const size = sizeOf(data);
      if (size === undefined || size > MAX_MESSAGE_BYTES) { counts.size++; return undefined; }
      const m = parseInbound(data);
      if (!m) { counts.type++; return undefined; }
      if (!allow(clock())) { counts.rate++; return undefined; }
      return m;
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
      const m = f.filter.take(e.data);
      if (!m) return false;
      f.on(m);
      return true;
    },
    get size() { return frames.size; },
  };
}
export type MessageRouter = ReturnType<typeof messageRouter>;
