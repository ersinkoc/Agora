//! ADR-0114 decision 7 (build step 4): golden fixtures for the community plugin catalog's one message, as a window and the hub exchange
//! it: the list asked for, and answered while the catalog is off and once it is on. They live in `schema/fixtures/plugin-catalog/` as
//! `agora.v1.<Message>.<case>.json` plus `.bin`. A fixture is never edited, only added.
//! To create the `.bin` of a new JSON fixture: `AGORA_WRITE_FIXTURES=1 cargo test -p agora-protocol --test plugin_catalog`.
#![allow(clippy::expect_used, clippy::panic)] // expect and panic are allowed in tests

use std::path::PathBuf;

use agora_protocol::v1::{self, command_ok, command_request};
use prost::Message;
use prost_reflect::{DescriptorPool, DynamicMessage};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schema/fixtures/plugin-catalog")
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
            "agora.v1.CommandOk" => {
                v1::CommandOk::decode(bin.as_slice()).map(|m| m.encode_to_vec())
            }
            other => panic!("no typed decoder for {other}"),
        };
        assert_eq!(
            typed.expect("typed decode"),
            bin,
            "{file}: typed re-encode differs"
        );
    }
    assert_eq!(seen, 3, "the request and two answers");
}

#[test]
fn the_typed_messages_read_what_the_fixtures_say() {
    let req =
        v1::CommandRequest::decode(bin("agora.v1.CommandRequest.plugin_catalog_list").as_slice())
            .expect("decode");
    assert!(req.target_node.is_empty(), "the hub answers it itself");
    assert!(matches!(
        req.command,
        Some(command_request::Command::PluginCatalogList(_))
    ));
    let ok = |case: &str| match v1::CommandOk::decode(
        bin(&format!("agora.v1.CommandOk.{case}")).as_slice(),
    )
    .expect("decode")
    .result
    {
        Some(command_ok::Result::PluginCatalogList(l)) => l,
        other => panic!("not a catalog list: {other:?}"),
    };
    let off = ok("plugin_catalog_list_off");
    assert!(!off.on && off.entries.is_empty() && off.updated_at_ms == 0);
    let on = ok("plugin_catalog_list_on");
    assert!(on.on);
    assert_eq!(on.updated_at_ms, 1_764_150_000_000);
    let e = &on.entries[0];
    assert_eq!(
        (
            e.plugin_id.as_str(),
            e.version.as_str(),
            e.licence.as_str(),
            e.sha256.len(),
            e.size
        ),
        ("sample-agents", "0.2.0", "Apache-2.0", 64, 1834)
    );
    assert_eq!(e.capabilities, ["agents.read", "theme.read"]);
    assert!(!e.description.is_empty());
}
