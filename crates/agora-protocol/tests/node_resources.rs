//! ADR-0136: `NodeResources` and `NodeSnapshot` gain the machine's CPU, memory and disks, and `DiskUsage` is new. The published
//! fixtures (written before the fields existed, never edited) still decode with the new fields empty, and the new fields round-trip.
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
fn old_bytes_decode_with_the_machine_figures_not_reported() {
    let r = v1::NodeResources::decode(fixture("NodeResources").as_slice()).unwrap();
    assert!(
        r.cpu_count == 0
            && r.cpu_busy_permille == 0
            && r.memory_total_bytes == 0
            && r.memory_available_bytes == 0
            && r.disks.is_empty()
            && r.workspaces_bytes == 0
    );
    let n = v1::NodeSnapshot::decode(fixture("NodeSnapshot").as_slice()).unwrap();
    assert!(n.cpu_count == 0 && n.disks.is_empty() && n.resources_at_ms == 0);
}

#[test]
fn the_machine_figures_round_trip() {
    let disk = v1::DiskUsage {
        mount: "/".into(),
        total_bytes: 500 << 30,
        free_bytes: 40 << 30,
        holds_data: true,
        holds_workspaces: true,
    };
    let r = v1::NodeResources {
        node_id: "blackview".into(),
        cpu_count: 16,
        cpu_busy_permille: 375,
        memory_total_bytes: 32 << 30,
        memory_available_bytes: 20 << 30,
        disks: vec![disk.clone()],
        workspaces_bytes: 12 << 30,
        ..Default::default()
    };
    assert_eq!(
        v1::NodeResources::decode(r.encode_to_vec().as_slice()).unwrap(),
        r
    );
    let n = v1::NodeSnapshot {
        node_id: "blackview".into(),
        cpu_count: 16,
        disks: vec![disk],
        resources_at_ms: 1,
        ..Default::default()
    };
    assert_eq!(
        v1::NodeSnapshot::decode(n.encode_to_vec().as_slice()).unwrap(),
        n
    );
}
