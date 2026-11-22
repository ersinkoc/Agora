//! ADR-0100 decision 2: a golden fixture for the field appended to `TerminalSubscribe` (`history_lines = 3`), whose own fixture
//! (`schema/fixtures/v1`, written before the field) is never edited. It lives in `schema/fixtures/terminal-history/` as
//! `agora.v1.<Message>.<case>.json` plus `.bin`. A fixture is never edited, only added.
//! To create the `.bin` of a new JSON fixture: `AGORA_WRITE_FIXTURES=1 cargo test -p agora-protocol --test terminal_history`.
#![allow(clippy::expect_used)] // expect is allowed in tests

use std::path::PathBuf;

use prost::Message;
use prost_reflect::{DescriptorPool, DynamicMessage};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schema/fixtures")
}

#[test]
fn the_history_field_round_trips_and_decodes_in_the_typed_message() {
    let pool = DescriptorPool::decode(agora_protocol::FILE_DESCRIPTOR_SET).expect("descriptor set");
    let write = std::env::var_os("AGORA_WRITE_FIXTURES").is_some();
    let mut seen = 0;
    for entry in std::fs::read_dir(root().join("terminal-history")).expect("read dir") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        seen += 1;
        let file = path
            .file_name()
            .expect("name")
            .to_string_lossy()
            .into_owned();
        let message = file.split('.').take(3).collect::<Vec<_>>().join(".");
        assert_eq!(message, "agora.v1.TerminalSubscribe", "{file}");
        let desc = pool.get_message_by_name(&message).expect("message");
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
        let typed =
            agora_protocol::v1::TerminalSubscribe::decode(bin.as_slice()).expect("typed decode");
        assert_eq!(
            typed.encode_to_vec(),
            bin,
            "{path:?}: typed re-encode differs"
        );
        assert_eq!(typed.history_lines, 2000, "{path:?}");
        assert_eq!(typed.terminal_id, "t-a1");
        assert_eq!(typed.format, "terminal.raw");
    }
    assert_eq!(seen, 1, "one fixture for the new field");
}

/// The fixture written before the field still decodes, with no history asked: an old window asks for none (ADR-0100 decision 3).
#[test]
fn a_subscription_from_before_the_field_asks_for_no_history() {
    let bin = std::fs::read(root().join("v1/agora.v1.TerminalSubscribe.bin")).expect("read bin");
    let typed = agora_protocol::v1::TerminalSubscribe::decode(bin.as_slice()).expect("decode");
    assert_eq!(typed.history_lines, 0);
    assert_eq!(typed.encode_to_vec(), bin, "nothing is added on re-encode");
}
