// Terminal stream frames (docs/specs/protocol.md "Node-local control socket"): the node writes, on the stream the
// client chose, kind 1 snapshot (ANSI that repaints the screen), kind 0 raw output, kind 2 resize (cols, rows as
// u16 LE) and kind 3 exit (i32 LE; negative = signal). A client that falls behind receives a fresh snapshot. When the
// subscription asked for history (ADR-0100), kind 4 frames come first: ANSI that puts the newest lines into the window's
// scrollback at the size of the snapshot that follows them.

export type TerminalEvent =
  | { type: 'data'; bytes: Uint8Array }
  // `history`: the kind 4 frames of the same subscription that came before it, joined (`historyPairing`); painted with it.
  | { type: 'snapshot'; bytes: Uint8Array; history?: Uint8Array }
  | { type: 'resize'; cols: number; rows: number }
  | { type: 'exit'; code: number };

// What a frame decodes to: a kind 4 history frame is not an event of its own; the source pairs it with its snapshot.
export type TerminalFrame = TerminalEvent | { type: 'history'; bytes: Uint8Array };

export const KIND_DATA = 0;
export const KIND_SNAPSHOT = 1;
export const KIND_RESIZE = 2;
export const KIND_EXIT = 3;
export const KIND_HISTORY = 4;

// Unknown kinds and malformed payloads give `undefined`: newer nodes may add kinds, and a bad frame must not
// take the terminal down.
export function decodeTerminalFrame(kind: number, payload: Uint8Array): TerminalFrame | undefined {
  const view = new DataView(payload.buffer, payload.byteOffset, payload.byteLength);
  switch (kind) {
    case KIND_DATA:
      return { type: 'data', bytes: payload };
    case KIND_SNAPSHOT:
      return { type: 'snapshot', bytes: payload };
    case KIND_RESIZE:
      if (payload.byteLength !== 4) return undefined;
      const cols = view.getUint16(0, true);
      const rows = view.getUint16(2, true);
      if (cols < 1 || cols > 1000 || rows < 1 || rows > 1000) return undefined;
      return { type: 'resize', cols, rows };
    case KIND_EXIT:
      if (payload.byteLength !== 4) return undefined;
      return { type: 'exit', code: view.getInt32(0, true) };
    case KIND_HISTORY:
      return { type: 'history', bytes: payload };
    default:
      return undefined;
  }
}

// Keys typed into the terminal go to the node as raw bytes (UTF-8 of what xterm produced).
export function inputBytes(data: string): Uint8Array {
  return new TextEncoder().encode(data);
}

// The older mouse reports (X10 and the UTF-8 form) are bytes: each character xterm gives is one byte, not text to encode as UTF-8.
export function binaryBytes(data: string): Uint8Array {
  const out = new Uint8Array(data.length);
  for (let i = 0; i < data.length; i++) out[i] = data.charCodeAt(i) & 0xff;
  return out;
}

// A snapshot is made at the size the node's terminal has, and the node sends that size directly after it (a resize frame). A window that
// paints the snapshot before it has taken that size parses lines of the wrong width: a 158-column screen into 80 columns wraps and
// scrolls its first rows away, and an idle program never sends anything to bring them back (an empty terminal on a first open, only
// sometimes: it depends on which the browser gets to first). So a snapshot waits for the size frame that follows it, and is painted
// just after the window has taken that size. Anything else that arrives first (output, exit, a later snapshot, or a short wait for a
// node that sends no size) paints it at once, so nothing is held back for long and the order is kept.
// A snapshot that comes with history (kind 4, ADR-0100) carries it (`historyPairing`), so both wait together and are written at once,
// at the right width (renderer.ts, `reset`).
export const SNAPSHOT_WAIT_MS = 150;

function joined(parts: Uint8Array[]): Uint8Array {
  if (parts.length === 1) return parts[0] as Uint8Array;
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let at = 0;
  for (const p of parts) { out.set(p, at); at += p.length; }
  return out;
}

// The most history held for one snapshot: one replay, as the node caps it (512 KiB, docs/specs/protocol.md, kind 4).
export const HISTORY_PENDING_MAX = 512 * 1024;

// Pairs the history frames (kind 4) of one subscription with the snapshot they come before (ADR-0100 decision 2): `frame` gives what to
// hand on (the snapshot then carries the history, joined), or nothing for a history frame, which is held. Nothing ever waits for
// history: a snapshot with none before it (an older node, a window that asked for none) is handed on as it is. Held history belongs to
// one subscription, so the source calls `reset` whenever one ends without its snapshot (the connection lost, the relay ended) and
// before the next starts, and any other frame in between drops it too: its frames never reach a later subscription's snapshot (Codex review of #964). More than one replay
// (`HISTORY_PENDING_MAX`) is never held: past it the history is dropped, and the rest of it until the snapshot is ignored.
export function historyPairing(): { frame(f: TerminalFrame): TerminalEvent | undefined; reset(): void } {
  let parts: Uint8Array[] = [];
  let size = 0;
  let over = false;
  const reset = () => { parts = []; size = 0; over = false; };
  return {
    frame(f) {
      if (f.type === 'history') {
        if (over || size + f.bytes.length > HISTORY_PENDING_MAX) { parts = []; size = 0; over = true; return undefined; }
        parts.push(f.bytes);
        size += f.bytes.length;
        return undefined;
      }
      // Anything else between history and its snapshot (it never comes) means the two no longer belong together.
      if (f.type !== 'snapshot') { reset(); return f; }
      const history = parts.length ? joined(parts) : undefined;
      reset();
      return history ? { type: 'snapshot', bytes: f.bytes, history } : f;
    },
    reset,
  };
}

export function snapshotAfterSize(apply: (e: TerminalEvent) => void): { push(e: TerminalEvent): void; dispose(): void } {
  let held: TerminalEvent | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const release = () => {
    if (timer !== undefined) { clearTimeout(timer); timer = undefined; }
    const h = held;
    held = undefined;
    if (h) apply(h);
  };
  return {
    push(e) {
      if (e.type === 'snapshot') {
        release();
        held = e;
        timer = setTimeout(release, SNAPSHOT_WAIT_MS);
        return;
      }
      if (e.type === 'resize') { apply(e); release(); return; }
      release();
      apply(e);
    },
    dispose() { if (timer !== undefined) clearTimeout(timer); timer = undefined; held = undefined; },
  };
}
