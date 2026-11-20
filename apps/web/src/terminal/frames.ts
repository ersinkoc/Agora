// Terminal stream frames (docs/specs/protocol.md "Node-local control socket"): the node writes, on the stream the
// client chose, kind 1 snapshot (ANSI that repaints the screen), kind 0 raw output, kind 2 resize (cols, rows as
// u16 LE) and kind 3 exit (i32 LE; negative = signal). A client that falls behind receives a fresh snapshot.

export type TerminalEvent =
  | { type: 'data'; bytes: Uint8Array }
  | { type: 'snapshot'; bytes: Uint8Array }
  | { type: 'resize'; cols: number; rows: number }
  | { type: 'exit'; code: number };

export const KIND_DATA = 0;
export const KIND_SNAPSHOT = 1;
export const KIND_RESIZE = 2;
export const KIND_EXIT = 3;

// Unknown kinds and malformed payloads give `undefined`: newer nodes may add kinds, and a bad frame must not
// take the terminal down.
export function decodeTerminalFrame(kind: number, payload: Uint8Array): TerminalEvent | undefined {
  const view = new DataView(payload.buffer, payload.byteOffset, payload.byteLength);
  switch (kind) {
    case KIND_DATA:
      return { type: 'data', bytes: payload };
    case KIND_SNAPSHOT:
      return { type: 'snapshot', bytes: payload };
    case KIND_RESIZE:
      if (payload.byteLength !== 4) return undefined;
      return { type: 'resize', cols: view.getUint16(0, true), rows: view.getUint16(2, true) };
    case KIND_EXIT:
      if (payload.byteLength !== 4) return undefined;
      return { type: 'exit', code: view.getInt32(0, true) };
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
export const SNAPSHOT_WAIT_MS = 150;

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
