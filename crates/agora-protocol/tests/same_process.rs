//! ADR-0104 decision 2: the golden fixture for `AgentStarted.same_process` (8), an agent re-stated by the node that announced its
//! current process before (taken up from the holder after a node restart, or re-stated in a resync). The fixture of `AgentStarted` in
//! `schema/fixtures/v1` is never edited; this one lives in `schema/fixtures/same-process/` as `agora.v1.<Message>.<case>.json` plus
//! `.bin`. A fixture is never edited, only added. To create the `.bin` of a new JSON fixture:
//! `AGORA_WRITE_FIXTURES=1 cargo test -p agora-protocol --test same_process`.
#![allow(clippy::expect_used)] // expect is allowed in tests

use std::path::PathBuf;

use prost::Message;
use prost_reflect::{DescriptorPool, DynamicMessage};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schema/fixtures/same-process")
}

#[test]
fn the_new_shape_round_trips_and_decodes_in_the_typed_message() {
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
        // The typed message reads it and writes the same bytes.
        let typed = match message.as_str() {
            "agora.v1.AgentStarted" => {
                let m = agora_protocol::v1::AgentStarted::decode(bin.as_slice()).expect("typed");
                assert!(m.same_process, "{path:?}: same_process is set");
                m.encode_to_vec()
            }
            other => panic!("no typed decoder for {other}"),
        };
        assert_eq!(typed, bin, "{path:?}: typed re-encode differs");
    }
    assert_eq!(seen, 1, "one fixture per case");
}

/// An `AgentStarted` from a node that predates the field (the published fixture) reads as a new process: the hub and the window
/// start its counters again from 0, as they always did.
#[test]
fn an_older_nodes_agent_started_reads_as_a_new_process() {
    let bin = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../schema/fixtures/v1/agora.v1.AgentStarted.bin"),
    )
    .expect("read the published fixture");
    let m = agora_protocol::v1::AgentStarted::decode(bin.as_slice()).expect("typed");
    assert!(!m.same_process);
    assert_eq!(m.encode_to_vec(), bin);
}
