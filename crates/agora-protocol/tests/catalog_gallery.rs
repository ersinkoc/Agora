//! ADR-0120: golden fixtures for the gallery's additions to the community catalogs: the date and popularity in both catalogs' lists,
//! the plugins' screenshot alt texts, and the screenshot command asked for, answered, and refused while the catalog is off. They live in
//! `schema/fixtures/catalog-gallery/` as `agora.v1.<Message>.<case>.json` plus `.bin`. A fixture is never edited, only added.
//! To create the `.bin` of a new JSON fixture: `AGORA_WRITE_FIXTURES=1 cargo test -p agora-protocol --test catalog_gallery`.
#![allow(clippy::expect_used, clippy::panic)] // expect and panic are allowed in tests

use std::path::PathBuf;

use agora_protocol::v1::{self, command_ok, command_request, command_response};
use prost::Message;
use prost_reflect::{DescriptorPool, DynamicMessage};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schema/fixtures/catalog-gallery")
}

fn bin(name: &str) -> Vec<u8> {
    std::fs::read(dir().join(format!("{name}.bin"))).expect("read bin")
}

#[test]
fn each_fixture_round_trips_in_both_encodings_and_the_typed_message() {
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
        let typed = match message.as_str() {
            "agora.v1.CommandRequest" => {
                v1::CommandRequest::decode(bin.as_slice()).map(|m| m.encode_to_vec())
            }
            "agora.v1.CommandResponse" => {
                v1::CommandResponse::decode(bin.as_slice()).map(|m| m.encode_to_vec())
            }
            other => panic!("no typed decoder for {other}"),
        };
        assert_eq!(
            typed.expect("typed decode"),
            bin,
            "{file}: typed re-encode differs"
        );
    }
    assert_eq!(
        seen, 5,
        "the request, its answer and refusal, and the two lists"
    );
}

fn ok(case: &str) -> command_ok::Result {
    match v1::CommandResponse::decode(bin(&format!("agora.v1.CommandResponse.{case}")).as_slice())
        .expect("decode")
        .result
    {
        Some(command_response::Result::Ok(v1::CommandOk { result: Some(r) })) => r,
        other => panic!("not an answer: {other:?}"),
    }
}

#[test]
fn the_typed_messages_read_what_the_fixtures_say() {
    let req = v1::CommandRequest::decode(
        bin("agora.v1.CommandRequest.plugin_catalog_screenshot").as_slice(),
    )
    .expect("decode");
    assert!(req.target_node.is_empty(), "the hub answers it itself");
    match req.command {
        Some(command_request::Command::PluginCatalogScreenshot(s)) => {
            assert_eq!((s.plugin_id.as_str(), s.index), ("sample-agents", 1));
        }
        other => panic!("not a screenshot request: {other:?}"),
    }
    match ok("plugin_catalog_screenshot") {
        command_ok::Result::PluginCatalogScreenshot(s) => {
            assert_eq!(s.media_type, "image/png");
            assert!(s.data.starts_with(b"\x89PNG\r\n\x1a\n"), "a PNG");
            assert!(!s.alt.is_empty());
        }
        other => panic!("not a screenshot: {other:?}"),
    }
    let off = v1::CommandResponse::decode(
        bin("agora.v1.CommandResponse.plugin_catalog_screenshot_off").as_slice(),
    )
    .expect("decode");
    match off.result {
        Some(command_response::Result::Err(e)) => assert_eq!(e.code, "plugin.catalog_off"),
        other => panic!("not a refusal: {other:?}"),
    }
    match ok("plugin_catalog_list_gallery") {
        command_ok::Result::PluginCatalogList(l) => {
            let e = &l.entries[0];
            assert_eq!((e.added.as_str(), e.popularity), ("2026-11-20", 128));
            assert_eq!(e.screenshots.len(), 2);
        }
        other => panic!("not a plugin list: {other:?}"),
    }
    match ok("theme_catalog_list_gallery") {
        command_ok::Result::ThemeCatalogList(l) => {
            let e = &l.entries[0];
            assert_eq!((e.added.as_str(), e.popularity), ("2026-11-18", 42));
        }
        other => panic!("not a theme list: {other:?}"),
    }
}
