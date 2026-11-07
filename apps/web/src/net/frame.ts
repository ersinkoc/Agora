// Frame envelope (docs/specs/protocol.md, crates/agora-protocol/src/frame.rs). One frame per WebSocket binary message:
//   stream_id varint, kind u8, flags u8 (bit 0 = seq), seq varint?, len varint, payload.

export const STREAM_CONTROL = 0;
export const STREAM_COMMANDS = 1;
export const STREAM_EVENTS = 2;

// ADR-0026 frame kinds on the events stream.
export const EVENT_KIND = { EVENT: 0, EVENT_ACK: 1, SUBSCRIBE: 2, SNAPSHOT: 3, POSITIONED_EVENT: 4 } as const;

export interface Frame {
  streamId: number;
  kind: number;
  seq?: number;
  payload: Uint8Array;
}

export const MAX_PAYLOAD = 1 << 20;

function putVarint(out: number[], v: number): void {
  let n = v;
  while (n >= 0x80) {
    out.push((n % 0x80) | 0x80);
    n = Math.floor(n / 0x80);
  }
  out.push(n);
}

export function encodeFrame(f: Frame): Uint8Array {
  if (f.payload.byteLength > MAX_PAYLOAD) throw new RangeError('payload too large');
  const head: number[] = [];
  putVarint(head, f.streamId);
  head.push(f.kind, f.seq === undefined ? 0 : 1);
  if (f.seq !== undefined) putVarint(head, f.seq);
  putVarint(head, f.payload.byteLength);
  const out = new Uint8Array(head.length + f.payload.byteLength);
  out.set(head, 0);
  out.set(f.payload, head.length);
  return out;
}

// Reads a canonical varint that fits in 2^53 (sequence numbers and lengths never get near that). `undefined` for
// anything malformed or truncated: a bad frame closes the connection, it never throws into the UI.
function getVarint(buf: Uint8Array, at: { pos: number }): number | undefined {
  let value = 0;
  for (let i = 0; i < 8; i++) {
    const b = buf[at.pos + i];
    if (b === undefined) return undefined;
    value += (b & 0x7f) * 2 ** (7 * i);
    if ((b & 0x80) === 0) {
      if (i > 0 && b === 0) return undefined; // padded, not canonical
      at.pos += i + 1;
      return value;
    }
  }
  return undefined;
}

export function decodeFrame(buf: Uint8Array): Frame | undefined {
  const at = { pos: 0 };
  const streamId = getVarint(buf, at);
  if (streamId === undefined || streamId > 0xffffffff) return undefined;
  const kind = buf[at.pos];
  const flags = buf[at.pos + 1];
  if (kind === undefined || flags === undefined || (flags & ~1) !== 0) return undefined;
  at.pos += 2;
  let seq: number | undefined;
  if (flags & 1) {
    seq = getVarint(buf, at);
    if (seq === undefined) return undefined;
  }
  const len = getVarint(buf, at);
  if (len === undefined || len > MAX_PAYLOAD || at.pos + len !== buf.byteLength) return undefined;
  return { streamId, kind, seq, payload: buf.subarray(at.pos, at.pos + len) };
}
