#![allow(clippy::unwrap_used)]
//! ADR-0087 rung 2 fixtures: a FAKE signed agent-package list (`tests/fixtures/agent-list/fake-list-v1.*`, signed by a key made from a
//! fixed public seed, so it proves nothing about any real key). Fixtures are only ever added, never edited: the next format change
//! adds `fake-list-v2.*`. With `AGORA_WRITE_FIXTURES=1` a missing signed file is written, as `agora-protocol`'s golden test does.
//! The ADR-0092 converter's output is checked here to fit the same installer shape (docs/specs/agent-packages.md, "The converter").

use std::fs;
use std::path::PathBuf;

use agora_adapters::agent_packages::{verify_list, Installer, ListError, VersionPolicy};
use agora_adapters::registry::RegistryError;
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::{json, Value};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/agent-list")
}
fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
/// A key from a public, fixed seed: fake by construction.
fn key(seed: u8) -> Ed25519KeyPair {
    Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).unwrap()
}
fn pubhex(k: &Ed25519KeyPair) -> String {
    hex(k.public_key().as_ref())
}
fn sign(k: &Ed25519KeyPair, payload: &str) -> String {
    let mut s = serde_json::to_string_pretty(
        &json!({"payload": payload, "sig": hex(k.sign(payload.as_bytes()).as_ref())}),
    )
    .unwrap();
    s.push('\n');
    s
}
fn payload(version: u64, revoked_second: bool) -> String {
    let agents: Value =
        serde_json::from_str(&fs::read_to_string(dir().join("fake-list-v1.agents.json")).unwrap())
            .unwrap();
    serde_json::to_string(&json!({
        "version": version, "expires_ms": 2_000_000_000_000_u64,
        "publishers": [
            {"id": "fake-official", "key": pubhex(&key(11))},
            {"id": "fake-second", "key": pubhex(&key(12)), "revoked": revoked_second},
        ],
        "agents": agents,
    }))
    .unwrap()
}

fn signed_fixture() -> String {
    let path = dir().join("fake-list-v1.signed.json");
    let want = sign(&key(1), &payload(7, false));
    if !path.exists() && std::env::var_os("AGORA_WRITE_FIXTURES").is_some() {
        fs::write(&path, &want).unwrap();
    }
    let got = fs::read_to_string(&path).unwrap().replace("\r\n", "\n");
    // Ed25519 is deterministic: the stored file is exactly what the fixed seed signs now, so it cannot be stale.
    assert_eq!(got, want, "fixture differs from what its source signs");
    got
}

#[test]
fn the_fake_list_opens_and_every_kind_is_read() {
    let l = verify_list(&signed_fixture(), &pubhex(&key(1)), 1, 7).unwrap();
    assert_eq!(l.version, 7);
    let kinds: Vec<_> = l.agents.iter().map(|a| a.installer.kind()).collect();
    assert_eq!(
        kinds,
        [
            "npm-global",
            "binary-download",
            "binary-download",
            "package-manager",
            "vendor-script"
        ]
    );
    let goose = &l.offer("fake-goose").unwrap()[0];
    assert!(matches!(goose.policy, VersionPolicy::Range { .. }));
    let Installer::BinaryDownload { platforms } = &goose.installer else {
        panic!("not a binary download")
    };
    assert_eq!(platforms.len(), 2);
    assert!(l
        .without_manifest(&["fake-claude", "fake-goose"])
        .contains(&"fake-node"));
}

#[test]
fn the_fake_list_fails_under_the_wrong_root_an_old_version_and_a_revoked_publisher() {
    let s = signed_fixture();
    assert_eq!(
        verify_list(&s, &pubhex(&key(2)), 1, 0),
        Err(ListError::Signed(RegistryError::BadSignature))
    );
    assert_eq!(
        verify_list(&s, &pubhex(&key(1)), 1, 8),
        Err(ListError::Signed(RegistryError::Rollback {
            seen: 8,
            offered: 7
        }))
    );
    // A signed list where the second publisher is revoked: it opens, the revoked publisher's entry is not offered.
    let revoked = sign(&key(1), &payload(8, true));
    let l = verify_list(&revoked, &pubhex(&key(1)), 1, 7).unwrap();
    assert!(l.offer("fake-claude").is_ok());
    assert_eq!(
        l.offer("fake-script"),
        Err(ListError::Revoked("fake-second".into()))
    );
    // Bytes changed after signing.
    let tampered = s.replace("fake-claude", "fake-clauds");
    assert!(verify_list(&tampered, &pubhex(&key(1)), 1, 0).is_err());
}

/// The converter's draft for a binary distribution (ADR-0092) is the same `binary-download` installer: its platform names, URL, SHA-256
/// and `cmd` are accepted as they are, once the converter-only keys are dropped and the fields a review adds are filled in.
#[test]
fn converter_output_fits_the_entry_format_after_review() {
    let conv = dir().join("../registry/goose.package.json");
    let draft: Value = serde_json::from_str(&fs::read_to_string(conv).unwrap()).unwrap();
    let inst = draft["installers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["kind"] == "binary-download")
        .unwrap();
    let mut platforms = inst["platforms"].clone();
    for p in platforms.as_object_mut().unwrap().values_mut() {
        p.as_object_mut().unwrap().remove("env_names_not_copied");
    }
    let entry = json!({
        "name": "goose", "display": draft["display"], "publisher": "fake-official",
        "changelog": "https://example.invalid/goose/changes",
        "homepage": draft["source"]["website"],
        "version_policy": "pin", "version": draft["source"]["version"],
        "provides": inst["provides"],
        "verify": {"args": ["--version"], "regex": "goose [0-9]+"},
        "installer": {"kind": "binary-download", "platforms": platforms},
    });
    let p = serde_json::to_string(&json!({
        "version": 1, "expires_ms": 2_000_000_000_000_u64,
        "publishers": [{"id": "fake-official", "key": pubhex(&key(11))}],
        "agents": [entry],
    }))
    .unwrap();
    let l = verify_list(&sign(&key(1), &p), &pubhex(&key(1)), 1, 0).unwrap();
    let Installer::BinaryDownload { platforms } = &l.agents[0].installer else {
        panic!("not a binary download")
    };
    assert_eq!(platforms.len(), 7);
    assert_eq!(platforms["linux-x86_64"].cmd.as_deref(), Some("./goose"));
}
