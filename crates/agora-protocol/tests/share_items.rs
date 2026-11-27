//! ADR-0122: `AgentStarted.shared_items` and `AgentSnapshot.shared_items` are additive: the published fixtures (never edited) decode
//! with them empty, and they round-trip.
#![allow(clippy::unwrap_used)]

use agora_protocol::v1;
use prost::Message;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../schema/fixtures/v1")
            .join(format!("agora.v1.{name}.bin")),
    )
    .unwrap()
}

#[test]
fn old_bytes_decode_with_no_shared_items_and_the_new_field_round_trips() {
    assert!(v1::AgentStarted::decode(fixture("AgentStarted").as_slice())
        .unwrap()
        .shared_items
        .is_empty());
    assert!(
        v1::AgentSnapshot::decode(fixture("AgentSnapshot").as_slice())
            .unwrap()
            .shared_items
            .is_empty()
    );
    let item = v1::SharedItemRef {
        name: "db-tools".into(),
        item_hash: "c5c882981a65c2384993763faefc1f41b35ed86a9aaad80a5e2cffe342cd258d".into(),
    };
    let s = v1::AgentStarted {
        agent_id: "agent-1".into(),
        shared_items: vec![item.clone()],
        ..Default::default()
    };
    assert_eq!(
        v1::AgentStarted::decode(s.encode_to_vec().as_slice()).unwrap(),
        s
    );
    let a = v1::AgentSnapshot {
        agent_id: "n/agent-1".into(),
        shared_items: vec![item],
        ..Default::default()
    };
    assert_eq!(
        v1::AgentSnapshot::decode(a.encode_to_vec().as_slice()).unwrap(),
        a
    );
}
