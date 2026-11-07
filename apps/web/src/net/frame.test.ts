import { describe, expect, it } from 'vitest';
import { decodeFrame, encodeFrame, EVENT_KIND, STREAM_EVENTS } from './frame';

describe('frame codec', () => {
  it('round trips with and without a sequence number', () => {
    for (const f of [
      { streamId: STREAM_EVENTS, kind: EVENT_KIND.SUBSCRIBE, payload: new Uint8Array([1, 2, 3]) },
      { streamId: 300, kind: 4, seq: 2 ** 40 + 5, payload: new Uint8Array(200) },
    ]) {
      const d = decodeFrame(encodeFrame(f));
      expect(d).toEqual({ seq: undefined, ...f });
    }
  });

  it('matches the Rust encoding byte for byte', () => {
    // stream 2, kind 2 (subscribe), no seq, empty payload.
    expect(Array.from(encodeFrame({ streamId: 2, kind: 2, payload: new Uint8Array() }))).toEqual([2, 2, 0, 0]);
    // stream 1, kind 0, seq 300 (0xAC 0x02), payload "hi".
    expect(Array.from(encodeFrame({ streamId: 1, kind: 0, seq: 300, payload: new Uint8Array([104, 105]) }))).toEqual([
      1, 0, 1, 0xac, 0x02, 2, 104, 105,
    ]);
  });

  it('refuses malformed input instead of throwing', () => {
    expect(decodeFrame(new Uint8Array())).toBeUndefined();
    expect(decodeFrame(new Uint8Array([2, 2, 0]))).toBeUndefined(); // truncated
    expect(decodeFrame(new Uint8Array([2, 2, 0, 3, 1]))).toBeUndefined(); // length longer than the data
    expect(decodeFrame(new Uint8Array([2, 2, 0, 0, 9]))).toBeUndefined(); // trailing bytes: one frame per message
    expect(decodeFrame(new Uint8Array([2, 2, 2, 0]))).toBeUndefined(); // unknown flag
    expect(decodeFrame(new Uint8Array([0x82, 0x00, 2, 0, 0]))).toBeUndefined(); // padded varint
  });
});
