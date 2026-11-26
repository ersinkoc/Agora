//! ADR-0113 decision 6: golden fixtures for the community theme catalog's commands, as a window and the hub exchange them: the list and
//! one theme asked for, the list while the catalog is off and once it is on, a theme, and the two refusals. They live in
//! `schema/fixtures/theme-catalog/` as `agora.v1.<Message>.<case>.json` plus `.bin`. A fixture is never edited, only added.
//! To create the `.bin` of a new JSON fixture: `AGORA_WRITE_FIXTURES=1 cargo test -p agora-protocol --test theme_catalog`.
#![allow(clippy::expect_used, clippy::panic)] // expect and panic are allowed in tests

use std::path::PathBuf;

use agora_protocol::v1::{self, command_ok, command_request, command_response};
use prost::Message;
use prost_reflect::{DescriptorPool, DynamicMessage};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schema/fixtures/theme-catalog")
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
    assert_eq!(seen, 7);
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
    let list =
        v1::CommandRequest::decode(bin("agora.v1.CommandRequest.theme_catalog_list").as_slice())
            .expect("decode");
    assert!(list.target_node.is_empty());
    assert!(matches!(
        list.command,
        Some(command_request::Command::ThemeCatalogList(_))
    ));
    let get =
        v1::CommandRequest::decode(bin("agora.v1.CommandRequest.theme_catalog_get").as_slice())
            .expect("decode");
    let Some(command_request::Command::ThemeCatalogGet(g)) = get.command else {
        panic!("{get:?}");
    };
    assert_eq!(g.id, "night");

    let command_ok::Result::ThemeCatalogList(off) =
        ok_result("agora.v1.CommandResponse.theme_catalog_off")
    else {
        panic!("off");
    };
    assert_eq!(off, v1::ThemeCatalogListResult::default());
    let command_ok::Result::ThemeCatalogList(on) =
        ok_result("agora.v1.CommandResponse.theme_catalog_on")
    else {
        panic!("on");
    };
    assert!(on.on);
    assert_eq!((on.entries.len(), on.entries[0].version), (1, 2));
    let command_ok::Result::ThemeCatalogTheme(theme) =
        ok_result("agora.v1.CommandResponse.theme_catalog_theme")
    else {
        panic!("theme");
    };
    let entry = theme.entry.expect("entry");
    assert_eq!(entry.size as usize, theme.data.len());
    assert_eq!(entry, on.entries[0]);

    for (name, code) in [
        (
            "agora.v1.CommandResponse.theme_catalog_get_off",
            "theme.catalog_off",
        ),
        (
            "agora.v1.CommandResponse.theme_not_found",
            "theme.not_found",
        ),
    ] {
        let r = v1::CommandResponse::decode(bin(name).as_slice()).expect("decode");
        let Some(command_response::Result::Err(e)) = r.result else {
            panic!("{name}");
        };
        assert_eq!(e.code, code);
        assert_eq!(
            agora_protocol::ErrorCode::parse(code).as_str(),
            code,
            "a code this build knows"
        );
    }
}
