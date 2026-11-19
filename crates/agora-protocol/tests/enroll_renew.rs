//! ADR-0085: golden fixtures for the new arms of `CommandRequest` and `CommandOk` (`renew`), whose own fixtures (`schema/fixtures/v1`,
//! sampled from the first arm of each oneof) are never edited. The new messages `RenewRequest` and `RenewResponse` have fixtures there.
//! These live in `schema/fixtures/enroll-renew/` as `agora.v1.<Message>.<case>.json` plus `.bin`. A fixture is never edited, only added.
//! To create the `.bin` of a new JSON fixture: `AGORA_WRITE_FIXTURES=1 cargo test -p agora-protocol --test enroll_renew`.
#![allow(clippy::expect_used)] // expect is allowed in tests

use std::path::PathBuf;

use prost::Message;
use prost_reflect::{DescriptorPool, DynamicMessage};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schema/fixtures/enroll-renew")
}

#[test]
fn the_renew_arms_round_trip_and_decode_in_the_typed_message() {
    let pool = DescriptorPool::decode(agora_protocol::FILE_DESCRIPTOR_SET).expect("descriptor set");
    let write = std::env::var_os("AGORA_WRITE_FIXTURES").is_some();
    let mut seen = 0;
    for entry in std::fs::read_dir(dir()).expect("read dir") {
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
        let typed = match message.as_str() {
            "agora.v1.CommandRequest" => agora_protocol::v1::CommandRequest::decode(bin.as_slice())
                .map(|m| m.encode_to_vec()),
            "agora.v1.CommandOk" => {
                agora_protocol::v1::CommandOk::decode(bin.as_slice()).map(|m| m.encode_to_vec())
            }
            other => panic!("no typed decoder for {other}"),
        };
        assert_eq!(
            typed.expect("typed decode"),
            bin,
            "{path:?}: typed re-encode differs"
        );
    }
    assert_eq!(seen, 2, "one fixture per new arm");
}
