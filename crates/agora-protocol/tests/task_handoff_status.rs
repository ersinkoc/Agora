//! ADR-0121 golden fixtures. Generate new binaries with
//! `AGORA_WRITE_FIXTURES=1 cargo test -p agora-protocol --test task_handoff_status`.
#![allow(clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use agora_protocol::v1;
use prost::Message;
use prost_reflect::{DescriptorPool, DynamicMessage};

#[test]
fn status_snapshot_event_and_acknowledgment_match_the_fixtures() {
    let pool = DescriptorPool::decode(agora_protocol::FILE_DESCRIPTOR_SET).expect("descriptor");
    let dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schema/fixtures/task-handoff-status");
    for (name, example) in [
        ("Event", "failed"),
        ("Snapshot", "unresolved"),
        ("CommandRequest", "acknowledge"),
    ] {
        let message = format!("agora.v1.{name}");
        let desc = pool.get_message_by_name(&message).expect("message");
        let json =
            std::fs::read_to_string(dir.join(format!("{message}.{example}.json"))).expect("JSON");
        let mut de = serde_json::Deserializer::from_str(&json);
        let bytes = DynamicMessage::deserialize(desc, &mut de)
            .expect("fixture")
            .encode_to_vec();
        let path = dir.join(format!("{message}.{example}.bin"));
        if std::env::var_os("AGORA_WRITE_FIXTURES").is_some() && !path.exists() {
            std::fs::write(&path, &bytes).expect("write fixture");
        }
        assert_eq!(std::fs::read(path).expect("binary fixture"), bytes);
        let typed = match name {
            "Event" => v1::Event::decode(bytes.as_slice()).map(|m| m.encode_to_vec()),
            "Snapshot" => v1::Snapshot::decode(bytes.as_slice()).map(|m| m.encode_to_vec()),
            "CommandRequest" => {
                v1::CommandRequest::decode(bytes.as_slice()).map(|m| m.encode_to_vec())
            }
            _ => panic!("unknown fixture message"),
        };
        assert_eq!(typed.expect("typed fixture"), bytes);
    }
}
