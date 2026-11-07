//! Envelope and framing (docs/specs/protocol.md section 2, ADR-0011).
//!
//! A frame is `{ stream_id, kind, seq?, payload }`. This layer only moves opaque payloads: it does not
//! assume WebSocket (or any transport) semantics, so the same bytes work over WSS, QUIC streams, or SSH stdio.
//!
//! Wire layout (all integers are unsigned LEB128 varints except `kind` and `flags`, which are one byte):
//!
//! ```text
//! stream_id  varint (u32)
//! kind       u8     (meaning belongs to the layer above)
//! flags      u8     bit 0 = seq present; other bits must be zero
//! seq        varint (u64), only if flags bit 0
//! len        varint (u32), payload length
//! payload    len bytes
//! ```

use crate::error::{Error, ErrorCode};

/// Well-known stream ids (spec section 2). Terminal and file streams use ids from [`FIRST_DYNAMIC_STREAM`].
pub const STREAM_CONTROL: u32 = 0;
pub const STREAM_COMMANDS: u32 = 1;
pub const STREAM_EVENTS: u32 = 2;
pub const FIRST_DYNAMIC_STREAM: u32 = 3;

/// Default maximum payload size. Terminal output is chunked far below this so input is never queued behind it.
pub const DEFAULT_MAX_PAYLOAD: usize = 1 << 20;

const FLAG_SEQ: u8 = 1;
const KNOWN_FLAGS: u8 = FLAG_SEQ;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub stream_id: u32,
    pub kind: u8,
    pub seq: Option<u64>,
    pub payload: Vec<u8>,
}

fn invalid(field: &str) -> Error {
    Error::new(ErrorCode::ProtocolInvalidMessage).with("field", field)
}

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

/// `Ok(None)` means more bytes are needed.
fn get_varint(buf: &[u8], pos: &mut usize, field: &str) -> Result<Option<u64>, Error> {
    let mut value = 0u64;
    for i in 0..10 {
        let Some(&b) = buf.get(*pos + i) else {
            return Ok(None);
        };
        let bits = u64::from(b & 0x7f);
        if i == 9 && bits > 1 {
            return Err(invalid(field)); // overflows u64
        }
        value |= bits << (7 * i);
        if b & 0x80 == 0 {
            if i > 0 && b == 0 {
                return Err(invalid(field)); // non-canonical (padded) encoding
            }
            *pos += i + 1;
            return Ok(Some(value));
        }
    }
    Err(invalid(field)) // more than 10 bytes
}

impl Frame {
    /// Append the encoded frame to `out`. Fails if the payload exceeds `max_payload`.
    pub fn encode_into(&self, out: &mut Vec<u8>, max_payload: usize) -> Result<(), Error> {
        if self.payload.len() > max_payload || u32::try_from(self.payload.len()).is_err() {
            return Err(invalid("payload"));
        }
        put_varint(out, u64::from(self.stream_id));
        out.push(self.kind);
        out.push(if self.seq.is_some() { FLAG_SEQ } else { 0 });
        if let Some(seq) = self.seq {
            put_varint(out, seq);
        }
        put_varint(out, self.payload.len() as u64);
        out.extend_from_slice(&self.payload);
        Ok(())
    }

    pub fn encode(&self, max_payload: usize) -> Result<Vec<u8>, Error> {
        let mut out = Vec::with_capacity(self.payload.len() + 16);
        self.encode_into(&mut out, max_payload)?;
        Ok(out)
    }
}

/// Incremental decoder: feed bytes as they arrive, pull complete frames.
#[derive(Debug)]
pub struct Decoder {
    buf: Vec<u8>,
    start: usize,
    max_payload: usize,
}

impl Decoder {
    pub fn new(max_payload: usize) -> Self {
        Self {
            buf: Vec::new(),
            start: 0,
            max_payload,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) {
        if self.start > 0 && self.start == self.buf.len() {
            self.buf.clear();
            self.start = 0;
        }
        self.buf.extend_from_slice(bytes);
    }

    /// Bytes received but not yet consumed as frames.
    pub fn buffered(&self) -> usize {
        self.buf.len() - self.start
    }

    /// The next complete frame, `Ok(None)` if more bytes are needed, or an error for malformed input.
    /// After an error the stream cannot be resynchronised; the connection must be closed.
    pub fn next_frame(&mut self) -> Result<Option<Frame>, Error> {
        let buf = &self.buf[self.start..];
        let mut pos = 0;
        let Some(stream_id) = get_varint(buf, &mut pos, "stream_id")? else {
            return Ok(None);
        };
        let stream_id = u32::try_from(stream_id).map_err(|_| invalid("stream_id"))?;
        let (Some(&kind), Some(&flags)) = (buf.get(pos), buf.get(pos + 1)) else {
            return Ok(None);
        };
        pos += 2;
        if flags & !KNOWN_FLAGS != 0 {
            return Err(invalid("flags"));
        }
        let seq = if flags & FLAG_SEQ != 0 {
            match get_varint(buf, &mut pos, "seq")? {
                Some(s) => Some(s),
                None => return Ok(None),
            }
        } else {
            None
        };
        let Some(len) = get_varint(buf, &mut pos, "len")? else {
            return Ok(None);
        };
        let len = usize::try_from(len).map_err(|_| invalid("len"))?;
        if len > self.max_payload {
            return Err(invalid("len"));
        }
        if buf.len() - pos < len {
            return Ok(None);
        }
        let payload = buf[pos..pos + len].to_vec();
        self.start += pos + len;
        Ok(Some(Frame {
            stream_id,
            kind,
            seq,
            payload,
        }))
    }
}

/// Frame `kind` values on the control and events streams (ADR-0026, ADR-0027). The payload is the named message.
pub mod kinds {
    // Stream 0 (control)
    pub const HELLO: u8 = 0;
    pub const WELCOME: u8 = 1;
    pub const RESUME: u8 = 2;
    pub const RESUME_OK: u8 = 3;
    pub const SNAPSHOT_REQUIRED: u8 = 4;
    pub const PING: u8 = 5;
    pub const PONG: u8 = 6;
    // Stream 2 (events)
    pub const EVENT: u8 = 0;
    pub const EVENT_ACK: u8 = 1;
    pub const SUBSCRIBE: u8 = 2;
    pub const SNAPSHOT: u8 = 3;
    pub const POSITIONED_EVENT: u8 = 4;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Small deterministic generator so tests need no extra dependency.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    fn random_frame(r: &mut Rng) -> Frame {
        let len = (r.next() % 300) as usize;
        Frame {
            stream_id: if r.next().is_multiple_of(2) {
                (r.next() % 8) as u32
            } else {
                r.next() as u32
            },
            kind: r.next() as u8,
            seq: (r.next().is_multiple_of(2)).then(|| {
                if r.next().is_multiple_of(2) {
                    r.next() % 1000
                } else {
                    r.next()
                }
            }),
            payload: (0..len).map(|_| r.next() as u8).collect(),
        }
    }

    #[test]
    fn round_trips_arbitrary_frames_split_at_arbitrary_points() {
        let mut r = Rng(0x9e37_79b9_7f4a_7c15);
        for _ in 0..500 {
            let frames: Vec<Frame> = (0..(1 + r.next() % 5))
                .map(|_| random_frame(&mut r))
                .collect();
            let mut wire = Vec::new();
            for f in &frames {
                assert_eq!(f.encode_into(&mut wire, DEFAULT_MAX_PAYLOAD), Ok(()));
            }
            let mut dec = Decoder::new(DEFAULT_MAX_PAYLOAD);
            let mut out = Vec::new();
            let mut i = 0;
            while i < wire.len() {
                let n = (1 + (r.next() % 40) as usize).min(wire.len() - i);
                dec.push(&wire[i..i + n]);
                i += n;
                while let Ok(Some(f)) = dec.next_frame() {
                    out.push(f);
                }
            }
            assert_eq!(out, frames);
            assert_eq!(dec.buffered(), 0);
        }
    }

    #[test]
    fn decoding_random_garbage_never_panics_and_errors_are_invalid_message() {
        let mut r = Rng(12345);
        for _ in 0..2000 {
            let bytes: Vec<u8> = (0..(r.next() % 64)).map(|_| r.next() as u8).collect();
            let mut dec = Decoder::new(256);
            dec.push(&bytes);
            while let Ok(Some(_)) = dec.next_frame() {}
            if let Err(e) = dec.next_frame() {
                assert_eq!(e.code, ErrorCode::ProtocolInvalidMessage);
            }
        }
    }

    fn decode_one(bytes: &[u8]) -> Result<Option<Frame>, Error> {
        let mut d = Decoder::new(16);
        d.push(bytes);
        d.next_frame()
    }

    #[test]
    fn rejects_oversize_payload_on_both_sides() {
        let f = Frame {
            stream_id: 3,
            kind: 0,
            seq: None,
            payload: vec![0; 17],
        };
        assert!(f.encode(16).is_err());
        let wire = Frame {
            payload: vec![0; 17],
            ..f
        }
        .encode(1024)
        .unwrap_or_default();
        let err = decode_one(&wire).err().map(|e| e.code);
        assert_eq!(err, Some(ErrorCode::ProtocolInvalidMessage));
    }

    #[test]
    fn rejects_unknown_flags_and_padded_varints() {
        assert!(decode_one(&[3, 0, 0b10, 0]).is_err());
        assert!(decode_one(&[0x83, 0x00, 0, 0, 0]).is_err()); // stream_id 3 encoded in two bytes
    }

    #[test]
    fn incomplete_input_waits_for_more() {
        assert_eq!(decode_one(&[]), Ok(None));
        assert_eq!(decode_one(&[3, 1, 0, 5, 1, 2]), Ok(None));
        let ok = decode_one(&[3, 1, 0, 2, 9, 8]);
        assert_eq!(
            ok,
            Ok(Some(Frame {
                stream_id: 3,
                kind: 1,
                seq: None,
                payload: vec![9, 8]
            }))
        );
    }

    #[test]
    fn golden_bytes_are_stable() {
        // This layout is a published contract: never edit this fixture, only add new ones.
        let f = Frame {
            stream_id: 2,
            kind: 7,
            seq: Some(300),
            payload: vec![0xAA, 0xBB],
        };
        assert_eq!(f.encode(16), Ok(vec![2, 7, 1, 0xAC, 0x02, 2, 0xAA, 0xBB]));
    }
}
