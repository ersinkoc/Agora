//! Golden fixtures: every published message has a binary and a JSON fixture under `schema/fixtures/v1/`.
//!
//! A fixture is never edited, only added (AGENTS.md). The tests check that each stored pair still decodes and
//! re-encodes to the same bytes and the same proto3 JSON, and that every message has a fixture.
//! To create fixtures for new messages: `AGORA_WRITE_FIXTURES=1 cargo test -p agora-protocol --test golden`
//! (existing files are never overwritten).
#![allow(clippy::expect_used)] // expect is allowed in tests

use std::path::PathBuf;

use prost::Message;
use prost_reflect::{DescriptorPool, DynamicMessage, MessageDescriptor, Value};
use serde::Serialize;

fn pool() -> DescriptorPool {
    DescriptorPool::decode(agora_protocol::FILE_DESCRIPTOR_SET).expect("descriptor set")
}

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schema/fixtures/v1")
}

fn messages(pool: &DescriptorPool) -> Vec<MessageDescriptor> {
    let mut v: Vec<_> = pool
        .all_messages()
        .filter(|m| m.package_name() == "agora.v1" && !m.is_map_entry())
        .collect();
    v.sort_by(|a, b| a.full_name().cmp(b.full_name()));
    v
}

/// Deterministic sample: strings "x", numbers 1, bools true, first enum value that is not zero,
/// bytes [1,2,3], one element per repeated/map field, first arm of every oneof, nesting limited.
fn sample(desc: &MessageDescriptor, depth: u32) -> DynamicMessage {
    use prost_reflect::Kind;
    let mut msg = DynamicMessage::new(desc.clone());
    for field in desc.fields() {
        if let Some(oneof) = field.containing_oneof() {
            // Only the first arm of each oneof gets a value in the sample.
            if oneof.fields().next().map(|f| f.number()) != Some(field.number()) {
                continue;
            }
        }
        let scalar = |kind: &Kind| -> Option<Value> {
            Some(match kind {
                Kind::String => Value::String("x".into()),
                Kind::Bytes => Value::Bytes(vec![1, 2, 3].into()),
                Kind::Bool => Value::Bool(true),
                Kind::Int32 | Kind::Sint32 | Kind::Sfixed32 => Value::I32(1),
                Kind::Int64 | Kind::Sint64 | Kind::Sfixed64 => Value::I64(1),
                Kind::Uint32 | Kind::Fixed32 => Value::U32(1),
                Kind::Uint64 | Kind::Fixed64 => Value::U64(1),
                Kind::Float => Value::F32(1.0),
                Kind::Double => Value::F64(1.0),
                Kind::Enum(e) => Value::EnumNumber(
                    e.values()
                        .map(|v| v.number())
                        .find(|n| *n != 0)
                        .unwrap_or(0),
                ),
                Kind::Message(m) if depth < 4 => Value::Message(sample(m, depth + 1)),
                Kind::Message(_) => return None,
            })
        };
        if field.is_map() {
            if let Kind::Message(entry) = field.kind() {
                let key = entry.map_entry_key_field();
                let val = entry.map_entry_value_field();
                let entry_kv = scalar(&key.kind()).zip(scalar(&val.kind()));
                if let Some((k, v)) = entry_kv {
                    if let Some(k) = k.into_map_key() {
                        let mut m = std::collections::HashMap::new();
                        m.insert(k, v);
                        msg.set_field(&field, Value::Map(m));
                    }
                }
            }
        } else if field.is_list() {
            if let Some(v) = scalar(&field.kind()) {
                msg.set_field(&field, Value::List(vec![v]));
            }
        } else if let Some(v) = scalar(&field.kind()) {
            msg.set_field(&field, v);
        }
    }
    msg
}

fn to_json(msg: &DynamicMessage) -> String {
    let mut out =
        serde_json::Serializer::with_formatter(Vec::new(), serde_json::ser::PrettyFormatter::new());
    msg.serialize(&mut out).expect("json");
    let mut s = String::from_utf8(out.into_inner()).expect("utf8");
    s.push('\n');
    s
}

#[test]
fn every_message_has_binary_and_json_fixtures_that_round_trip() {
    let pool = pool();
    let write = std::env::var_os("AGORA_WRITE_FIXTURES").is_some();
    std::fs::create_dir_all(dir()).expect("mkdir");
    let mut missing = Vec::new();
    for desc in messages(&pool) {
        let name = desc.full_name().to_string();
        let (bin_path, json_path) = (
            dir().join(format!("{name}.bin")),
            dir().join(format!("{name}.json")),
        );
        if !bin_path.exists() || !json_path.exists() {
            if write && !bin_path.exists() && !json_path.exists() {
                let m = sample(&desc, 0);
                std::fs::write(&bin_path, m.encode_to_vec()).expect("write bin");
                std::fs::write(&json_path, to_json(&m)).expect("write json");
            } else {
                missing.push(name);
                continue;
            }
        }
        let bin = std::fs::read(&bin_path).expect("read bin");
        let json = std::fs::read_to_string(&json_path).expect("read json");

        let decoded = DynamicMessage::decode(desc.clone(), bin.as_slice()).expect("decode bin");
        assert_eq!(
            decoded.encode_to_vec(),
            bin,
            "{name}: binary re-encode differs"
        );
        assert_eq!(
            to_json(&decoded),
            json,
            "{name}: JSON of the binary fixture differs from the JSON fixture"
        );

        let mut de = serde_json::Deserializer::from_str(&json);
        let from_json = DynamicMessage::deserialize(desc.clone(), &mut de).expect("parse json");
        assert_eq!(
            from_json.encode_to_vec(),
            bin,
            "{name}: JSON fixture encodes to different bytes"
        );
    }
    assert!(
        missing.is_empty(),
        "missing fixtures (run with AGORA_WRITE_FIXTURES=1): {missing:?}"
    );
}

#[test]
fn no_orphan_fixtures() {
    let names: Vec<String> = messages(&pool())
        .iter()
        .map(|m| m.full_name().to_string())
        .collect();
    for entry in std::fs::read_dir(dir()).expect("read dir") {
        let file = entry
            .expect("entry")
            .file_name()
            .to_string_lossy()
            .into_owned();
        let stem = file.rsplit_once('.').map_or(file.as_str(), |(s, _)| s);
        assert!(
            names.iter().any(|n| n == stem),
            "fixture without a message: {file}"
        );
    }
}
