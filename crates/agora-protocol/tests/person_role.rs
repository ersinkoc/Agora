//! ADR-0106: golden fixtures for the field appended to `Welcome` (`person_role`): the hub's answer to a window signed in as the owner,
//! a member and a viewer. The fixture in `schema/fixtures/v1` (no appended field set) is never edited; these live in
//! `schema/fixtures/person-role/` as `agora.v1.Welcome.<case>.json` plus `.bin`. A fixture is never edited, only added.
//! To create the `.bin` of a new JSON fixture: `AGORA_WRITE_FIXTURES=1 cargo test -p agora-protocol --test person_role`.
#![allow(clippy::expect_used)] // expect is allowed in tests

use std::path::PathBuf;

use agora_protocol::v1::{self, PersonRole};
use prost::Message;
use prost_reflect::{DescriptorPool, DynamicMessage};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schema/fixtures/person-role")
}

#[test]
fn each_welcome_with_a_role_round_trips_and_decodes_in_the_typed_message() {
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
        assert_eq!(message, "agora.v1.Welcome", "{file}");
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
            v1::Welcome::decode(bin.as_slice())
                .expect("typed decode")
                .encode_to_vec(),
            bin,
            "{path:?}: typed re-encode differs"
        );
    }
    assert_eq!(seen, 3, "one fixture per role");
}

fn welcome(case: &str) -> v1::Welcome {
    let bin = std::fs::read(dir().join(format!("agora.v1.Welcome.{case}.bin"))).expect("read bin");
    v1::Welcome::decode(bin.as_slice()).expect("decode")
}

#[test]
fn the_typed_welcome_reads_each_role() {
    for (case, role) in [
        ("owner", PersonRole::Owner),
        ("member", PersonRole::Member),
        ("viewer", PersonRole::Viewer),
    ] {
        let w = welcome(case);
        assert_eq!(w.person_role(), role, "{case}");
        assert_eq!(
            (w.chosen_generation, w.server_time_ms, w.capabilities.len()),
            (1, 1_700_000_600_000, 0),
            "{case}"
        );
    }
}

/// A client or node from before ADR-0106 reads the new `Welcome` as it read every one: the appended field is an unknown one it skips,
/// and everything else decodes unchanged. Modelled by the published `Welcome` fixture: the field added to it leaves its other fields as
/// they were, and cutting it off again gives back the published bytes. A published `Welcome` reads as unspecified (a node's, or an
/// older hub's).
#[test]
fn the_published_welcome_is_unchanged_with_the_field_added_and_removed() {
    let published = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../schema/fixtures/v1/agora.v1.Welcome.bin"),
    )
    .expect("read the published fixture");
    let mut w = v1::Welcome::decode(published.as_slice()).expect("decode");
    assert_eq!(w.person_role(), PersonRole::Unspecified);
    w.set_person_role(PersonRole::Viewer);
    let with = w.encode_to_vec();
    assert!(
        with.starts_with(&published),
        "an appended field only adds bytes"
    );
    let mut back = v1::Welcome::decode(with.as_slice()).expect("decode");
    back.set_person_role(PersonRole::Unspecified);
    assert_eq!(back.encode_to_vec(), published);
}
