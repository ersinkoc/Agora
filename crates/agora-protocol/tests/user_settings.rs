//! ADR-0113 decision 6 (build step 4): golden fixtures for a person's own settings value, as a window and the hub exchange them: a read
//! and a write, the answer for a value never written and for one kept, and the two refusals (a stale write, a value over the cap). They
//! live in `schema/fixtures/user-settings/` as `agora.v1.<Message>.<case>.json` plus `.bin`. A fixture is never edited, only added.
//! To create the `.bin` of a new JSON fixture: `AGORA_WRITE_FIXTURES=1 cargo test -p agora-protocol --test user_settings`.
#![allow(clippy::expect_used, clippy::panic)] // expect and panic are allowed in tests

use std::path::PathBuf;

use agora_protocol::v1::{self, command_ok, command_request, command_response};
use prost::Message;
use prost_reflect::{DescriptorPool, DynamicMessage};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schema/fixtures/user-settings")
}

#[test]
fn each_fixture_round_trips_in_both_encodings() {
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
            "{file}: JSON differs from binary"
        );
        let decoded = DynamicMessage::decode(desc, bin.as_slice()).expect("decode bin");
        assert_eq!(decoded.encode_to_vec(), bin, "{file}: re-encode differs");
    }
    assert_eq!(seen, 6);
}

fn bin(name: &str) -> Vec<u8> {
    std::fs::read(dir().join(format!("{name}.bin"))).expect("read bin")
}

fn ok_result(name: &str) -> command_ok::Result {
    let r = v1::CommandResponse::decode(bin(name).as_slice()).expect("decode");
    match r.result {
        Some(command_response::Result::Ok(v1::CommandOk { result: Some(ok) })) => ok,
        other => panic!("{name}: {other:?}"),
    }
}

#[test]
fn the_typed_messages_read_each_case() {
    let get =
        v1::CommandRequest::decode(bin("agora.v1.CommandRequest.user_settings_get").as_slice())
            .expect("decode");
    assert!(get.target_node.is_empty());
    let Some(command_request::Command::UserSettingsGet(g)) = get.command else {
        panic!("{get:?}");
    };
    assert_eq!(g.key, "themes");
    let set =
        v1::CommandRequest::decode(bin("agora.v1.CommandRequest.user_settings_set").as_slice())
            .expect("decode");
    let Some(command_request::Command::UserSettingsSet(s)) = set.command else {
        panic!("{set:?}");
    };
    assert_eq!((s.key.as_str(), s.base_version), ("themes", 3));
    // The value is the window's own bytes; the hub never reads them.
    assert!(s.value.starts_with(b"{\"agora-theme-sync\":1"));

    let command_ok::Result::UserSettings(none) =
        ok_result("agora.v1.CommandResponse.user_settings_none")
    else {
        panic!("none");
    };
    assert_eq!(
        none,
        v1::UserSettingsValue {
            key: "themes".into(),
            tag: "3fa1c09d7b2e4a6f8c5d1e0b9a7f6c42".into(),
            ..Default::default()
        }
    );
    let command_ok::Result::UserSettings(kept) =
        ok_result("agora.v1.CommandResponse.user_settings_value")
    else {
        panic!("kept");
    };
    assert_eq!(
        (kept.version, kept.value.as_slice()),
        (4, s.value.as_slice())
    );
    assert!(kept.updated_ms > 0);
    assert_eq!(kept.tag, none.tag, "the same person's tag on every value");

    for (name, code, param, value) in [
        (
            "agora.v1.CommandResponse.settings_stale",
            "settings.stale",
            "version",
            4,
        ),
        (
            "agora.v1.CommandResponse.settings_too_large",
            "settings.too_large",
            "max",
            262_144,
        ),
    ] {
        let r = v1::CommandResponse::decode(bin(name).as_slice()).expect("decode");
        let Some(command_response::Result::Err(e)) = r.result else {
            panic!("{name}");
        };
        assert_eq!(e.code, code);
        assert_eq!(
            e.params.get(param).and_then(|p| p.value.clone()),
            Some(v1::param_value::Value::Int(value))
        );
        let known = agora_protocol::ErrorCode::parse(code);
        assert_eq!(known.as_str(), code, "a code this build knows");
        assert_eq!(known.params(), [param]);
    }
}
