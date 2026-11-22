//! ADR-0101: golden fixtures for the fields appended to the approval messages (`ApprovalScope`, `scopes`, `always_label`,
//! `always_option_id`, `scope`, `ChatEntry.permission_always`, `AgentSnapshot.always_allowed`) and the new `Event` arm
//! (`AgentAlwaysAllowed`). The fixtures in `schema/fixtures/v1` (no appended field set) are never edited; these live in
//! `schema/fixtures/approval-always/` as `agora.v1.<Message>.<case>.json` plus `.bin`. A fixture is never edited, only added.
//! To create the `.bin` of a new JSON fixture: `AGORA_WRITE_FIXTURES=1 cargo test -p agora-protocol --test approval_always`.
#![allow(clippy::expect_used)] // expect is allowed in tests

use std::path::PathBuf;

use agora_protocol::v1;
use prost::Message;
use prost_reflect::{DescriptorPool, DynamicMessage};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schema/fixtures/approval-always")
}

fn typed_reencode(message: &str, bin: &[u8]) -> Vec<u8> {
    fn re<M: Message + Default>(bin: &[u8]) -> Vec<u8> {
        M::decode(bin).expect("typed decode").encode_to_vec()
    }
    match message {
        "agora.v1.ApprovalRequestParams" => re::<v1::ApprovalRequestParams>(bin),
        "agora.v1.ApprovalResolveParams" => re::<v1::ApprovalResolveParams>(bin),
        "agora.v1.CommandRequest" => re::<v1::CommandRequest>(bin),
        "agora.v1.Event" => re::<v1::Event>(bin),
        "agora.v1.ChatEntry" => re::<v1::ChatEntry>(bin),
        "agora.v1.Snapshot" => re::<v1::Snapshot>(bin),
        other => panic!("no typed decoder for {other}"),
    }
}

#[test]
fn the_new_shapes_round_trip_and_decode_in_the_typed_message() {
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
        assert_eq!(
            typed_reencode(&message, &bin),
            bin,
            "{path:?}: typed re-encode differs"
        );
    }
    assert_eq!(seen, 10, "one fixture per case");
}

fn bin(name: &str) -> Vec<u8> {
    std::fs::read(dir().join(name)).expect("read bin")
}

#[test]
fn the_typed_messages_read_the_appended_fields() {
    let once = v1::ApprovalScope::Once as i32;
    let always = v1::ApprovalScope::Always as i32;
    let p = v1::ApprovalRequestParams::decode(
        bin("agora.v1.ApprovalRequestParams.always.bin").as_slice(),
    )
    .expect("decode");
    assert_eq!(p.scopes, vec![once, always]);
    assert_eq!(p.always_label, "Always allow git status");
    assert_eq!(p.always_option_id, "allow-always-1");

    let r = v1::ApprovalResolveParams::decode(
        bin("agora.v1.ApprovalResolveParams.always.bin").as_slice(),
    )
    .expect("decode");
    assert_eq!(r.scope, always);

    let c = v1::CommandRequest::decode(
        bin("agora.v1.CommandRequest.approval_resolve_always.bin").as_slice(),
    )
    .expect("decode");
    let Some(v1::command_request::Command::ApprovalResolve(a)) = c.command else {
        panic!("not an approval decision");
    };
    assert_eq!(a.scope, always);

    let e = v1::Event::decode(bin("agora.v1.Event.agent_always_allowed.bin").as_slice())
        .expect("decode");
    let Some(v1::event::Body::AgentAlwaysAllowed(a)) = e.body else {
        panic!("not AgentAlwaysAllowed");
    };
    assert_eq!(
        (a.agent_id.as_str(), a.action.as_str()),
        ("agent-1", "run_command")
    );

    let s = v1::Snapshot::decode(bin("agora.v1.Snapshot.approval_always.bin").as_slice())
        .expect("decode");
    assert_eq!(s.agents[0].always_allowed, vec!["run_command".to_string()]);
    assert_eq!(s.approvals[0].scopes, vec![once, always]);
    assert_eq!(s.approvals[0].always_option_id, "edits");

    let chat = v1::ChatEntry::decode(bin("agora.v1.ChatEntry.permission_always.bin").as_slice())
        .expect("decode");
    assert!(chat.permission_always);
}

/// A decision from a client before ADR-0101 carries no scope: it reads as UNSPECIFIED, which every receiver takes as ONCE.
#[test]
fn an_old_decision_reads_as_unspecified() {
    let old = v1::ApprovalResolve {
        approval_id: "ap-1".into(),
        approved: true,
        signer_device_id: "dev".into(),
        signature: vec![1, 2, 3],
        ..Default::default()
    };
    let bytes = old.encode_to_vec();
    let back = v1::ApprovalResolve::decode(bytes.as_slice()).expect("decode");
    assert_eq!(back.scope, v1::ApprovalScope::Unspecified as i32);
}
