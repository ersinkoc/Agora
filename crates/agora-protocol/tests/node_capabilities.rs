//! ADR-0132: `NodeSnapshot.capabilities` and the hub's `NodeCapabilitiesChanged` are additive. The published snapshot fixture (written
//! before the field existed, never edited) decodes with the capabilities unknown, and the new field and event round-trip.
#![allow(clippy::unwrap_used)]

use agora_protocol::v1;
use prost::Message;

#[test]
fn an_older_snapshot_says_nothing_about_capabilities_and_the_new_ones_round_trip() {
    let bytes = std::fs::read(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../schema/fixtures/v1/agora.v1.NodeSnapshot.bin"),
    )
    .unwrap();
    assert!(v1::NodeSnapshot::decode(bytes.as_slice())
        .unwrap()
        .capabilities
        .is_none());
    let known_empty = v1::NodeSnapshot {
        node_id: "n".into(),
        capabilities: Some(v1::NodeCapabilities::default()),
        ..Default::default()
    };
    let back = v1::NodeSnapshot::decode(known_empty.encode_to_vec().as_slice()).unwrap();
    assert_eq!(
        back.capabilities,
        Some(v1::NodeCapabilities::default()),
        "known and empty stays known"
    );
    let event = v1::Event {
        producer_id: "hub".into(),
        seq: 1,
        at_ms: 2,
        body: Some(v1::event::Body::NodeCapabilitiesChanged(
            v1::NodeCapabilitiesChanged {
                node_id: "n".into(),
                generation: 3,
                capabilities: Some(v1::NodeCapabilities {
                    names: vec!["task.dispatch".into()],
                }),
            },
        )),
    };
    assert_eq!(
        v1::Event::decode(event.encode_to_vec().as_slice()).unwrap(),
        event
    );
}
