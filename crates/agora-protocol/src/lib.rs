//! Generated protocol types (from schema/), codecs, handshake and capability negotiation. See docs/specs/protocol.md.

pub mod error;
pub mod frame;

pub use error::{Error, ErrorCode, ParamValue};
pub use frame::{Decoder, Frame};

/// Messages of protocol generation 1, generated from `schema/agora/v1/*.proto`.
#[allow(clippy::all, clippy::pedantic, missing_docs)]
pub mod v1 {
    include!(concat!(env!("OUT_DIR"), "/agora.v1.rs"));
}

#[cfg(test)]
mod tests {
    use super::v1::{Ping, Pong};
    use prost::Message;

    #[test]
    fn ping_round_trips() {
        let ping = Ping { nonce: 42 };
        let bytes = ping.encode_to_vec();
        assert_eq!(Ping::decode(bytes.as_slice()).map(|p| p.nonce), Ok(42));
    }

    #[test]
    fn ping_and_pong_share_wire_layout() {
        // Field 1, varint 42: the numbers in the schema are the compatibility contract.
        assert_eq!(Ping { nonce: 42 }.encode_to_vec(), vec![0x08, 42]);
        assert_eq!(Pong { nonce: 42 }.encode_to_vec(), vec![0x08, 42]);
    }
}
