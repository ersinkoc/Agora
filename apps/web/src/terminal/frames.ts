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
