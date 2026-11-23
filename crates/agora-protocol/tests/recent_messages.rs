//! ADR-0103: golden fixture for the field appended to `Snapshot` (`recent_messages`): a snapshot that carries one message between
//! machines as both of its copies, the sender's (`to` is `<node>/<agent>`) and the receiver's (`from` is already `<origin>/<agent>`).
//! The fixture in `schema/fixtures/v1` (no appended field set) is never edited; this one lives in `schema/fixtures/recent-messages/`
//! as `agora.v1.<Message>.<case>.json` plus `.bin`. A fixture is never edited, only added.
//! To create the `.bin` of a new JSON fixture: `AGORA_WRITE_FIXTURES=1 cargo test -p agora-protocol --test recent_messages`.
#![allow(clippy::expect_used)] // expect is allowed in tests

use std::path::PathBuf;

use agora_protocol::v1;
use prost::Message;
use prost_reflect::{DescriptorPool, DynamicMessage};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schema/fixtures/recent-messages")
}

#[test]
fn the_snapshot_with_recent_messages_round_trips_and_decodes_in_the_typed_message() {
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
        assert_eq!(message, "agora.v1.Snapshot", "{file}");
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
            v1::Snapshot::decode(bin.as_slice())
                .expect("typed decode")
                .encode_to_vec(),
            bin,
            "{path:?}: typed re-encode differs"
        );
    }
    assert_eq!(seen, 1, "one fixture per case");
}

fn bin(name: &str) -> Vec<u8> {
    std::fs::read(dir().join(name)).expect("read bin")
}

#[test]
fn the_typed_snapshot_reads_both_copies_with_their_envelopes() {
    let s = v1::Snapshot::decode(bin("agora.v1.Snapshot.recent_messages.bin").as_slice())
        .expect("decode");
    assert_eq!(s.pos, 42);
    let got: Vec<_> = s
        .recent_messages
        .iter()
        .map(|p| {
            let e = p.event.as_ref().expect("event");
            let Some(v1::event::Body::MessagePosted(m)) = &e.body else {
                panic!("not MessagePosted");
            };
            (
                p.pos,
                e.producer_id.as_str(),
                e.at_ms,
                m.from_agent_id.as_str(),
                m.to.as_str(),
                m.body.as_str(),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            (
                40,
                "alpha",
                1_700_000_600_000,
                "agent-1",
                "beta/agent-2",
                "ping"
            ),
            (
                41,
                "beta",
                1_700_000_600_150,
                "alpha/agent-1",
                "agent-2",
                "ping"
            ),
        ]
    );
}

/// A window from before ADR-0103 reads the new snapshot as it read every snapshot: the appended field is an unknown one it skips, and
/// everything else decodes unchanged. Modelled by the published `Snapshot` fixture: the new field added to it leaves its other fields
/// as they were, and cutting the field off again gives back the published bytes.
#[test]
fn the_published_snapshot_is_unchanged_with_the_field_added_and_removed() {
    let published = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../schema/fixtures/v1/agora.v1.Snapshot.bin"),
    )
    .expect("read the published fixture");
    let mut s = v1::Snapshot::decode(published.as_slice()).expect("decode");
    assert!(s.recent_messages.is_empty());
    let extra = v1::Snapshot::decode(bin("agora.v1.Snapshot.recent_messages.bin").as_slice())
        .expect("decode");
    s.recent_messages = extra.recent_messages;
    let with = s.encode_to_vec();
    assert!(
        with.starts_with(&published),
        "an appended field only adds bytes"
    );
    let mut back = v1::Snapshot::decode(with.as_slice()).expect("decode");
    back.recent_messages.clear();
    assert_eq!(back.encode_to_vec(), published);
}
