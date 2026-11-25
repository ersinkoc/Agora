//! ADR-0110: golden fixtures for the error code `adapter.no_answer` and for `adapter.agent_exited` with stage `turn` (the answer to a
//! prompt or an approval answer), as a value of the existing error-code field. The published fixtures (`schema/fixtures/v1`) are never
//! edited. These live in `schema/fixtures/adapter-no-answer/` as `agora.v1.<Message>.<case>.json` plus `.bin`. A fixture is never
//! edited, only added. To create the `.bin` of a new JSON fixture:
//! `AGORA_WRITE_FIXTURES=1 cargo test -p agora-protocol --test adapter_no_answer`.
//!
//! `ErrorInfo.params` is a map, whose entries an encoder may write in any order, so the stored bytes are compared by what they decode
//! to, never byte for byte.
#![allow(clippy::expect_used)] // expect is allowed in tests

use std::path::PathBuf;

use agora_protocol::v1;
use agora_protocol::{Error, ErrorCode};
use prost::Message;
use prost_reflect::{DescriptorPool, DynamicMessage};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schema/fixtures/adapter-no-answer")
}

/// The typed `CommandResponse` (or `ErrorInfo`, wrapped in one) a fixture decodes to.
fn typed(message: &str, bytes: &[u8]) -> v1::CommandResponse {
    match message {
        "agora.v1.CommandResponse" => v1::CommandResponse::decode(bytes).expect("typed decode"),
        "agora.v1.ErrorInfo" => v1::CommandResponse {
            request_id: String::new(),
            result: Some(v1::command_response::Result::Err(
                v1::ErrorInfo::decode(bytes).expect("typed decode"),
            )),
        },
        other => panic!("no typed decoder for {other}"),
    }
}

#[test]
fn the_new_codes_round_trip_and_decode_in_the_typed_message() {
    let pool = DescriptorPool::decode(agora_protocol::FILE_DESCRIPTOR_SET).expect("descriptor set");
    let write = std::env::var_os("AGORA_WRITE_FIXTURES").is_some();
    let mut seen = Vec::new();
    for entry in std::fs::read_dir(dir()).expect("read dir") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
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
        let decoded = DynamicMessage::decode(desc.clone(), bin.as_slice()).expect("decode bin");
        assert_eq!(decoded, from_json, "{path:?}: JSON differs from binary");
        let response = typed(&message, &bin);
        assert_eq!(
            typed(&message, &response_bytes(&message, &response)),
            response,
            "{path:?}: typed re-encode differs"
        );
        seen.push((file, response));
    }
    seen.sort_by(|a, b| a.0.cmp(&b.0));
    let errors: Vec<v1::ErrorInfo> = seen
        .iter()
        .map(|(_, r)| match &r.result {
            Some(v1::command_response::Result::Err(e)) => e.clone(),
            other => panic!("not an error: {other:?}"),
        })
        .collect();
    assert_eq!(errors.len(), 3, "one fixture per new shape");
    // What the node builds for each (crates/agora-node/src/supervisor.rs) is exactly what the fixtures hold.
    let no_answer = |call: &str| {
        Error::new(ErrorCode::AdapterNoAnswer)
            .with("kind", "fake-adapter")
            .with("call", call)
            .with("waited_ms", 10_000i64)
            .to_info()
    };
    let exited = Error::new(ErrorCode::AdapterAgentExited)
        .with("stage", "turn")
        .with("exit_code", "3")
        .with("stderr_tail", "Error: the provider refused the request")
        .to_info();
    // Sorted by file name: CommandResponse.agent_exited_turn, CommandResponse.no_answer_approval, ErrorInfo.no_answer.
    assert_eq!(
        errors,
        vec![exited, no_answer("approval"), no_answer("prompt")]
    );
    // A client that does not know the code keeps it as a code it can show its general text for.
    assert_eq!(
        ErrorCode::parse("adapter.no_answer"),
        ErrorCode::AdapterNoAnswer
    );
}

fn response_bytes(message: &str, r: &v1::CommandResponse) -> Vec<u8> {
    match (message, &r.result) {
        ("agora.v1.ErrorInfo", Some(v1::command_response::Result::Err(e))) => e.encode_to_vec(),
        _ => r.encode_to_vec(),
    }
}
