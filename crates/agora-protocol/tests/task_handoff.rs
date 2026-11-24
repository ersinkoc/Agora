//! ADR-0109 wire fixtures. To add a binary for a new JSON fixture, run
//! `AGORA_WRITE_FIXTURES=1 cargo test -p agora-protocol --test task_handoff`.
#![allow(clippy::expect_used)]

use std::path::PathBuf;

use agora_protocol::frame::{DEFAULT_MAX_PAYLOAD, STREAM_COMMANDS};
use agora_protocol::handshake::{negotiate, Support, TASK_HANDOFF_V1};
use agora_protocol::{v1, Frame};
use prost::Message;
use prost_reflect::{DescriptorPool, DynamicMessage};

fn fixture<M: Message + Default>(pool: &DescriptorPool, name: &str, case: &str) -> M {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schema/fixtures/task-handoff");
    let json = std::fs::read_to_string(dir.join(format!("{name}.{case}.json"))).expect("read JSON");
    let descriptor = pool.get_message_by_name(name).expect("fixture message");
    let mut de = serde_json::Deserializer::from_str(&json);
    let from_json = DynamicMessage::deserialize(descriptor, &mut de).expect("parse JSON");
    let path = dir.join(format!("{name}.{case}.bin"));
    if std::env::var_os("AGORA_WRITE_FIXTURES").is_some() && !path.exists() {
        std::fs::write(&path, from_json.encode_to_vec()).expect("write binary");
    }
    let bytes = std::fs::read(&path).expect("read binary");
    assert_eq!(from_json.encode_to_vec(), bytes, "{path:?}");
    let typed = M::decode(bytes.as_slice()).expect("decode binary");
    assert_eq!(typed.encode_to_vec(), bytes, "{path:?}");
    typed
}

#[test]
fn golden_delivery_names_the_run_and_the_receipt_echoes_both_ids() {
    let pool = DescriptorPool::decode(agora_protocol::FILE_DESCRIPTOR_SET).expect("schema");
    let start: v1::AgentStarted = fixture(&pool, "agora.v1.AgentStarted", "run");
    let request: v1::CommandRequest = fixture(&pool, "agora.v1.CommandRequest", "deliver");
    let reply: v1::CommandResponse = fixture(&pool, "agora.v1.CommandResponse", "receipt");

    let Some(v1::command_request::Command::DeliverTaskHandoff(delivery)) = request.command else {
        panic!("expected durable handoff command");
    };
    let Some(v1::command_response::Result::Ok(v1::CommandOk {
        result: Some(v1::command_ok::Result::TaskHandoffReceipt(receipt)),
    })) = reply.result
    else {
        panic!("expected handoff receipt");
    };
    assert!(start.same_process);
    assert_eq!(start.run_id.as_slice(), &(16..32).collect::<Vec<u8>>());
    assert_eq!(delivery.receiver_run_id, start.run_id);
    assert_eq!(delivery.agent_id, start.agent_id);
    assert_eq!(delivery.project_id, start.project_id);
    assert_eq!(
        delivery.delivery_id.as_slice(),
        &(0..16).collect::<Vec<u8>>()
    );
    assert_eq!(receipt.delivery_id, delivery.delivery_id);
    assert_eq!(receipt.receiver_run_id, delivery.receiver_run_id);
    assert_eq!(reply.request_id, request.request_id);
}

#[test]
fn a_published_agent_start_still_round_trips_without_a_run_id() {
    let bytes = include_bytes!("../../../schema/fixtures/v1/agora.v1.AgentStarted.bin");
    let older = v1::AgentStarted::decode(bytes.as_slice()).expect("decode published agent start");
    assert!(older.run_id.is_empty());
    assert_eq!(older.encode_to_vec(), bytes);
}

#[test]
fn capability_is_selected_only_when_both_peers_offer_it() {
    let older = Support {
        generations: vec![1],
        capabilities: Vec::new(),
        required: Vec::new(),
    };
    let capable = Support {
        capabilities: vec![TASK_HANDOFF_V1.into()],
        ..older.clone()
    };
    let mut hello = v1::Hello {
        protocol_generations: vec![1],
        ..Default::default()
    };

    assert!(negotiate(&hello, &capable, "peer", 1)
        .expect("older node can connect")
        .capabilities
        .is_empty());
    hello.capabilities.push(TASK_HANDOFF_V1.into());
    assert!(negotiate(&hello, &older, "peer", 1)
        .expect("node can connect to an older hub")
        .capabilities
        .is_empty());
    assert_eq!(
        negotiate(&hello, &capable, "peer", 1)
            .expect("both peers have the feature")
            .capabilities,
        [TASK_HANDOFF_V1]
    );
}

#[test]
fn delivery_with_full_mesh_body_and_refs_fits_the_command_frame() {
    let mesh_bytes = 16 * 1024;
    let mut refs = vec!["r".repeat(mesh_bytes / 20); 20];
    refs[0].push_str(&"r".repeat(mesh_bytes % 20));
    let body = v1::DeliverTaskHandoff {
        delivery_id: vec![1; 16],
        receiver_run_id: vec![2; 16],
        agent_id: "a".repeat(4096),
        project_id: "p".repeat(4096),
        task_id: format!("t-{}", u64::MAX),
        summary: "é".repeat(mesh_bytes / 2),
        refs,
        valid_until_ms: 1_700_086_400_000,
        sender_node: "n".repeat(64),
        sender_agent_id: "s".repeat(4096),
    };
    assert_eq!(body.summary.len(), mesh_bytes);
    assert_eq!(body.refs.iter().map(String::len).sum::<usize>(), mesh_bytes);
    let request = v1::CommandRequest {
        request_id: format!("h{}", u64::MAX),
        command: Some(v1::command_request::Command::DeliverTaskHandoff(body)),
        ..Default::default()
    };
    let payload = request.encode_to_vec();
    assert_eq!(
        v1::CommandRequest::decode(payload.as_slice()).expect("decode delivery"),
        request
    );
    assert!(Frame {
        stream_id: STREAM_COMMANDS,
        kind: 0,
        seq: None,
        payload,
    }
    .encode(DEFAULT_MAX_PAYLOAD)
    .is_ok());
}
