//! ADR-0090: golden fixtures for the new fields of `ChatEntry`, whose own fixture (`schema/fixtures/v1`) is never edited.
//! They live in `schema/fixtures/chat-rich/` as `<message>.<case>.json` plus `.bin`. A fixture is never edited, only added.
//! To create the `.bin` of a new JSON fixture: `AGORA_WRITE_FIXTURES=1 cargo test -p agora-protocol --test chat_rich`.
#![allow(clippy::expect_used)] // expect is allowed in tests

use std::path::PathBuf;

use prost::Message;
use prost_reflect::{DescriptorPool, DynamicMessage};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schema/fixtures/chat-rich")
}

#[test]
fn the_new_chat_entry_shapes_round_trip_and_decode_in_the_typed_message() {
    let pool = DescriptorPool::decode(agora_protocol::FILE_DESCRIPTOR_SET).expect("descriptor set");
    let desc = pool
        .get_message_by_name("agora.v1.ChatEntry")
        .expect("ChatEntry");
    let write = std::env::var_os("AGORA_WRITE_FIXTURES").is_some();
    let mut seen = 0;
    for entry in std::fs::read_dir(dir()).expect("read dir") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        seen += 1;
        let json = std::fs::read_to_string(&path).expect("read json");
        let mut de = serde_json::Deserializer::from_str(&json);
        let from_json = DynamicMessage::deserialize(desc.clone(), &mut de).expect("parse json");
        let bin_path = path.with_extension("bin");
        if write && !bin_path.exists() {
            std::fs::write(&bin_path, from_json.encode_to_vec()).expect("write bin");
        }
        let bin = std::fs::read(&bin_path).expect("read bin (run with AGORA_WRITE_FIXTURES=1)");
        assert_eq!(
            from_json.encode_to_vec(),
            bin,
            "{path:?}: JSON differs from binary"
        );
        let decoded = DynamicMessage::decode(desc.clone(), bin.as_slice()).expect("decode bin");
        assert_eq!(decoded.encode_to_vec(), bin, "{path:?}: re-encode differs");
        // The typed message reads it and writes the same bytes.
        let typed = agora_protocol::v1::ChatEntry::decode(bin.as_slice()).expect("typed decode");
        assert_eq!(
            typed.encode_to_vec(),
            bin,
            "{path:?}: typed re-encode differs"
        );
    }
    assert_eq!(seen, 5, "one fixture per new shape");
}
