//! ADR-0114 decision 6 (build step 3): golden fixtures for the plugin arms of `CommandRequest` and `CommandOk`, whose own fixtures
//! (`schema/fixtures/v1`, sampled from the first arm of each oneof) are never edited. They live in `schema/fixtures/plugins/` as
//! `agora.v1.<Message>.<case>.json` plus `.bin`. A fixture is never edited, only added.
//! To create the `.bin` of a new JSON fixture: `AGORA_WRITE_FIXTURES=1 cargo test -p agora-protocol --test plugins`.
#![allow(clippy::expect_used)] // expect is allowed in tests

use std::path::PathBuf;

use agora_protocol::v1::{self, command_ok, command_request};
use prost::Message;
use prost_reflect::{DescriptorPool, DynamicMessage};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schema/fixtures/plugins")
}

fn bin(name: &str) -> Vec<u8> {
    std::fs::read(dir().join(format!("{name}.bin"))).expect("read bin")
}

#[test]
fn each_plugin_shape_round_trips_and_decodes_in_the_typed_message() {
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
            "{path:?}: typed re-encode differs"
        );
    }
    assert_eq!(seen, 8, "five requests and three answers");
}

#[test]
fn the_typed_messages_read_what_the_fixtures_say() {
    let req = |case: &str| {
        v1::CommandRequest::decode(bin(&format!("agora.v1.CommandRequest.{case}")).as_slice())
            .expect("decode")
    };
    let ok = |case: &str| {
        v1::CommandOk::decode(bin(&format!("agora.v1.CommandOk.{case}")).as_slice())
            .expect("decode")
    };
    // Each request names no node: the hub answers them itself.
    for case in [
        "plugin_list",
        "plugin_fetch",
        "plugin_grant_list",
        "plugin_grant_set",
        "plugin_grant_remove",
    ] {
        assert!(req(case).target_node.is_empty(), "{case}");
    }
    let Some(command_request::Command::PluginFetch(f)) = req("plugin_fetch").command else {
        panic!("not a fetch")
    };
    assert_eq!(
        (f.plugin_id.as_str(), f.sha256.len(), f.offset),
        ("sample-agents", 64, 512 * 1024)
    );
    let Some(command_request::Command::PluginGrantSet(g)) = req("plugin_grant_set").command else {
        panic!("not a grant")
    };
    assert_eq!(
        (g.version.as_str(), g.granted.as_slice(), g.sha256.len()),
        ("0.2.0", &["agents.read".to_string()][..], 64)
    );
    let Some(command_ok::Result::PluginList(l)) = ok("plugin_list").result else {
        panic!("not a list")
    };
    assert_eq!(l.plugins.len(), 1);
    assert_eq!(l.plugins[0].capabilities, ["agents.read", "theme.read"]);
    let Some(command_ok::Result::PluginFetch(p)) = ok("plugin_fetch").result else {
        panic!("not a piece")
    };
    assert_eq!(
        (p.offset, p.total_size, p.data.as_slice()),
        (0, 12, &b"{\"agora-plug"[..])
    );
    // A grant is kept for the version it was given for, with what that version asked for.
    let Some(command_ok::Result::PluginGrantList(g)) = ok("plugin_grant_list").result else {
        panic!("not grants")
    };
    assert_eq!(
        (
            g.grants[0].version.as_str(),
            g.grants[0].seen.len(),
            g.grants[0].granted.len()
        ),
        ("0.1.0", 2, 1)
    );
}

/// Only the owner installs, on the hub's machine (`agora hub plugin add`): no message a client can send installs or removes a plugin.
/// The plugin arms of `CommandRequest` are exactly the five that read the installed plugins and keep the sender's own grants.
#[test]
fn no_message_installs_or_removes_a_plugin() {
    let pool = DescriptorPool::decode(agora_protocol::FILE_DESCRIPTOR_SET).expect("descriptor set");
    let req = pool
        .get_message_by_name("agora.v1.CommandRequest")
        .expect("CommandRequest");
    let mut arms: Vec<String> = req
        .fields()
        .filter(|f| f.name().starts_with("plugin"))
        .map(|f| f.name().to_string())
        .collect();
    arms.sort();
    assert_eq!(
        arms,
        [
            "plugin_fetch",
            "plugin_grant_list",
            "plugin_grant_remove",
            "plugin_grant_set",
            "plugin_list"
        ]
    );
    // And the schema's plugin messages are only these: none carries a package towards the hub.
    let mut names: Vec<String> = pool
        .all_messages()
        .filter(|m| m.name().starts_with("Plugin"))
        .map(|m| m.name().to_string())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "PluginFetch",
            "PluginFetchResult",
            "PluginGrant",
            "PluginGrantList",
            "PluginGrantListResult",
            "PluginGrantRemove",
            "PluginGrantSet",
            "PluginInfo",
            "PluginList",
            "PluginListResult"
        ]
    );
}
