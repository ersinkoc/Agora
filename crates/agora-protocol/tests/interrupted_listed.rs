//! ADR-0107: golden fixture for the field appended to `AgentListResult` (`interrupted`, 2): a list with one ended agent in `agents` and
//! one agent the node found interrupted, as the node announces it (`AgentInterrupted`). The fixture in `schema/fixtures/v1` (no
//! appended field set) is never edited; this one lives in `schema/fixtures/interrupted-listed/` as `agora.v1.<Message>.<case>.json`
//! plus `.bin`. A fixture is never edited, only added. To create the `.bin` of a new JSON fixture:
//! `AGORA_WRITE_FIXTURES=1 cargo test -p agora-protocol --test interrupted_listed`.
#![allow(clippy::expect_used)] // expect is allowed in tests

use std::path::PathBuf;

use agora_protocol::v1;
use prost::Message;
use prost_reflect::{DescriptorPool, DynamicMessage};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schema/fixtures/interrupted-listed")
}

fn bin(name: &str) -> Vec<u8> {
    std::fs::read(dir().join(name)).expect("read bin")
}

#[test]
fn the_list_with_interrupted_agents_round_trips_and_decodes_in_the_typed_message() {
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
        assert_eq!(message, "agora.v1.AgentListResult", "{file}");
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
        assert_eq!(
            v1::AgentListResult::decode(bin.as_slice())
                .expect("typed decode")
                .encode_to_vec(),
            bin,
            "{path:?}: typed re-encode differs"
        );
    }
    assert_eq!(seen, 1, "one fixture per case");
}

#[test]
fn the_typed_list_reads_the_interrupted_agent_as_the_event_says_it() {
    let l = v1::AgentListResult::decode(bin("agora.v1.AgentListResult.interrupted.bin").as_slice())
        .expect("decode");
    assert_eq!(l.agents.len(), 1);
    assert_eq!(l.agents[0].agent_id, "agent-2");
    assert!(!l.agents[0].running);
    assert_eq!(
        l.interrupted,
        [v1::AgentInterrupted {
            agent_id: "agent-3".into(),
            kind: "codex".into(),
            project_id: "demo".into(),
            folder: "/home/u/work/b".into(),
            parent_agent_id: "agent-2".into(),
            resumable: true,
            started_at_ms: 1_700_000_000_000,
            interrupted_at_ms: 1_700_000_600_000,
            last_state: v1::AgentState::Working as i32,
            still_running: true,
            parent_node_id: String::new(),
            parent_node_agent_id: String::new(),
        }]
    );
}

/// `AgentListResult` as a reader from before ADR-0107 (a 0.3.36 CLI) knows it: field 1 only.
#[derive(Clone, PartialEq, prost::Message)]
struct OldAgentListResult {
    #[prost(message, repeated, tag = "1")]
    agents: Vec<v1::AgentSummary>,
}

/// An old reader asking a new node skips the appended field (an unknown field) and reads `agents` as it always did: it never sees the
/// interrupted rows, so it cannot misread them as stopped ones.
#[test]
fn an_old_reader_reads_the_agents_and_skips_the_interrupted_ones() {
    let new = bin("agora.v1.AgentListResult.interrupted.bin");
    let old = OldAgentListResult::decode(new.as_slice()).expect("old decode");
    let typed = v1::AgentListResult::decode(new.as_slice()).expect("decode");
    assert_eq!(old.agents, typed.agents);
    assert_eq!(old.agents.len(), 1);
}

/// A new reader asking an old node gets the published shape: no interrupted agents, `agents` as it was. Adding the field to the
/// published fixture only adds bytes, and cutting it off again gives back the published bytes.
#[test]
fn the_published_list_is_unchanged_with_the_field_added_and_removed() {
    let published = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../schema/fixtures/v1/agora.v1.AgentListResult.bin"),
    )
    .expect("read the published fixture");
    let mut l = v1::AgentListResult::decode(published.as_slice()).expect("decode");
    assert!(l.interrupted.is_empty());
    let extra =
        v1::AgentListResult::decode(bin("agora.v1.AgentListResult.interrupted.bin").as_slice())
            .expect("decode");
    l.interrupted = extra.interrupted;
    let with = l.encode_to_vec();
    assert!(
        with.starts_with(&published),
        "an appended field only adds bytes"
    );
    let mut back = v1::AgentListResult::decode(with.as_slice()).expect("decode");
    back.interrupted.clear();
    assert_eq!(back.encode_to_vec(), published);
}
