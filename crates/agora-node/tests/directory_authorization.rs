#![allow(clippy::unwrap_used, clippy::expect_used)]
use agora_node::{
    approvals::{NodeApprovals, P256Verifier},
    directory::signed_message,
    grants::{Grants, SignedCommands},
};
use agora_protocol::v1::{self, command_request::Command as Cmd};
use ring::{
    rand::SystemRandom,
    signature::{self, KeyPair},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
fn key() -> signature::EcdsaKeyPair {
    let rng = SystemRandom::new();
    let pk =
        signature::EcdsaKeyPair::generate_pkcs8(&signature::ECDSA_P256_SHA256_FIXED_SIGNING, &rng)
            .unwrap();
    signature::EcdsaKeyPair::from_pkcs8(
        &signature::ECDSA_P256_SHA256_FIXED_SIGNING,
        pk.as_ref(),
        &rng,
    )
    .unwrap()
}
fn grants(key: &signature::EcdsaKeyPair) -> Grants {
    let mut v = P256Verifier::new();
    assert!(v.trust("browser", key.public_key().as_ref()));
    Grants::new(
        "node-a",
        Arc::new(NodeApprovals::new("node-a", Box::new(v))),
        SignedCommands::default(),
    )
}
fn launch(key: &signature::EcdsaKeyPair, nonce: &str) -> v1::AgentLaunch {
    let expires = 101_000;
    let message = signed_message(
        "node-a",
        "launch",
        "/selected",
        "probe",
        "id:project",
        false,
        false,
        nonce,
        expires,
    );
    let signature = key
        .sign(&SystemRandom::new(), &message)
        .unwrap()
        .as_ref()
        .to_vec();
    v1::AgentLaunch {
        path: "/selected".into(),
        kind: "probe".into(),
        project_id: "id:project".into(),
        authorization: Some(v1::DirectoryAuthorization {
            device_id: "browser".into(),
            nonce: nonce.into(),
            expires_at_ms: expires,
            signature,
        }),
        ..Default::default()
    }
}
#[test]
fn signatures_bind_every_launch_field_and_refuse_replay_including_after_restart() {
    let key = key();
    let nonce = "00112233445566778899aabbccddeeff";
    let a = launch(&key, nonce);
    let file = std::env::temp_dir().join(format!("directory-nonce-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&file);
    let g = grants(&key).with_directory_nonce_file(file.clone());
    for f in 0..6 {
        let mut b = a.clone();
        match f {
            0 => b.path = "/other".into(),
            1 => b.kind = "other".into(),
            2 => b.project_id = "other".into(),
            3 => b.separate_worktree = true,
            4 => b.accept_identity_change = true,
            _ => b.authorization.as_mut().unwrap().expires_at_ms += 1,
        };
        assert!(g.directory_action(&Cmd::AgentLaunch(b), 100_000).is_err());
    }
    assert!(g
        .directory_action(&Cmd::AgentLaunch(a.clone()), 100_000)
        .is_ok());
    assert!(g
        .directory_action(&Cmd::AgentLaunch(a.clone()), 100_000)
        .is_err());
    assert!(grants(&key)
        .with_directory_nonce_file(file.clone())
        .directory_action(&Cmd::AgentLaunch(a), 100_000)
        .is_err());
    let _ = std::fs::remove_file(file);
}
#[test]
fn unsigned_expired_cross_node_and_project_limited_requests_cannot_register_arbitrary_folders() {
    let key = key();
    let a = launch(&key, "00112233445566778899aabbccddeeff");
    let g = grants(&key);
    let mut missing = a.clone();
    missing.authorization = None;
    assert!(g
        .directory_action(&Cmd::AgentLaunch(missing), 100_000)
        .is_err());
    assert!(g
        .directory_action(&Cmd::AgentLaunch(a.clone()), 102_000)
        .is_err());
    let mut verifier = P256Verifier::new();
    verifier.trust("browser", key.public_key().as_ref());
    let other = Grants::new(
        "node-b",
        Arc::new(NodeApprovals::new("node-b", Box::new(verifier))),
        SignedCommands::default(),
    );
    assert!(other
        .directory_action(&Cmd::AgentLaunch(a.clone()), 100_000)
        .is_err());
    let limited = grants(&key).with_limits(Arc::new(agora_node::limits::DeviceLimits::new(
        BTreeMap::from([("browser".into(), BTreeSet::from(["id:project".into()]))]),
    )));
    assert!(limited
        .directory_action(&Cmd::AgentLaunch(a), 100_000)
        .is_err());
}
fn list(key: &signature::EcdsaKeyPair, op: &str, path: &str, nonce: &str) -> v1::DirectoryList {
    let expires = 101_000;
    let message = signed_message("node-a", op, path, "", "", false, false, nonce, expires);
    let signature = key
        .sign(&SystemRandom::new(), &message)
        .unwrap()
        .as_ref()
        .to_vec();
    v1::DirectoryList {
        path: path.into(),
        authorization: Some(v1::DirectoryAuthorization {
            device_id: "browser".into(),
            nonce: nonce.into(),
            expires_at_ms: expires,
            signature,
        }),
    }
}
#[test]
fn a_folder_listing_needs_the_owners_signature_for_that_folder() {
    let key = key();
    let g = grants(&key);
    let n = |i: u8| format!("{i:032x}");
    // Signed for listing this folder: allowed once.
    let ok = list(&key, "list", "/Users/me", &n(1));
    assert!(g
        .directory_action(&Cmd::DirectoryList(ok.clone()), 100_000)
        .is_ok());
    assert!(
        g.directory_action(&Cmd::DirectoryList(ok), 100_000)
            .is_err(),
        "replay"
    );
    // Unsigned, for another folder, or a signature made for inspecting: refused.
    let mut unsigned = list(&key, "list", "/Users/me", &n(2));
    unsigned.authorization = None;
    assert!(g
        .directory_action(&Cmd::DirectoryList(unsigned), 100_000)
        .is_err());
    let mut moved = list(&key, "list", "/Users/me", &n(3));
    moved.path = "/".into();
    assert!(g
        .directory_action(&Cmd::DirectoryList(moved), 100_000)
        .is_err());
    let inspect = list(&key, "inspect", "/Users/me", &n(4));
    assert!(g
        .directory_action(&Cmd::DirectoryList(inspect), 100_000)
        .is_err());
    // An empty path (the home folder) is signed like any other.
    assert!(g
        .directory_action(&Cmd::DirectoryList(list(&key, "list", "", &n(5))), 100_000)
        .is_ok());
    // A device limited to some projects never lists a machine's folders.
    let limited = grants(&key).with_limits(Arc::new(agora_node::limits::DeviceLimits::new(
        BTreeMap::from([("browser".into(), BTreeSet::from(["id:project".into()]))]),
    )));
    assert!(limited
        .directory_action(
            &Cmd::DirectoryList(list(&key, "list", "/Users/me", &n(6))),
            100_000
        )
        .is_err());
}

/// ADR-0059: a launch that chose options is signed over them and over which of them the user confirmed (`directory-action-v3`,
/// two length-prefixed lists); a hub can neither add, remove or swap an option nor add or strip a confirmation.
#[test]
fn launch_options_are_signed_so_a_hub_can_neither_add_nor_remove_one() {
    let key = key();
    let strs = |v: &[&str]| v.iter().map(|o| (*o).to_string()).collect::<Vec<String>>();
    let sign = |options: &[&str], confirmed: &[&str], nonce: &str| {
        let (options, confirmed) = (strs(options), strs(confirmed));
        let message = agora_node::directory::launch_signed_message(
            "node-a",
            "/selected",
            "probe",
            "id:project",
            false,
            false,
            &options,
            &confirmed,
            nonce,
            101_000,
        );
        let mut a = launch(&key, nonce);
        a.launch_options = options;
        a.confirmed_launch_options = confirmed;
        a.authorization.as_mut().unwrap().signature = key
            .sign(&SystemRandom::new(), &message)
            .unwrap()
            .as_ref()
            .to_vec();
        a
    };
    let g = grants(&key);
    // Signed without options: the hub adds one, or a confirmation.
    let mut added = launch(&key, "00000000000000000000000000000001");
    added.launch_options = strs(&["skip-permissions"]);
    assert!(g
        .directory_action(&Cmd::AgentLaunch(added.clone()), 100_000)
        .is_err());
    added.confirmed_launch_options = strs(&["skip-permissions"]);
    assert!(g
        .directory_action(&Cmd::AgentLaunch(added), 100_000)
        .is_err());
    let signed = sign(
        &["skip-permissions"],
        &["skip-permissions"],
        "00000000000000000000000000000002",
    );
    for (options, confirmed) in [
        (vec![], vec![]),
        (strs(&["skip-permissions"]), vec![]),
        (strs(&["resume-last"]), strs(&["skip-permissions"])),
        (
            strs(&["skip-permissions", "resume-last"]),
            strs(&["skip-permissions"]),
        ),
        (strs(&["skip-permissions!"]), vec![]),
    ] {
        let mut b = signed.clone();
        b.launch_options = options;
        b.confirmed_launch_options = confirmed;
        assert!(g.directory_action(&Cmd::AgentLaunch(b), 100_000).is_err());
    }
    // A signature over the unconfirmed option does not confirm it: the hub cannot add the confirmation afterwards.
    let mut unconfirmed = sign(
        &["skip-permissions"],
        &[],
        "00000000000000000000000000000003",
    );
    unconfirmed.confirmed_launch_options = strs(&["skip-permissions"]);
    assert!(g
        .directory_action(&Cmd::AgentLaunch(unconfirmed), 100_000)
        .is_err());
    // One id `a,b` is not the two ids `a` and `b`.
    let mut comma = sign(&["a,b"], &[], "00000000000000000000000000000004");
    comma.launch_options = strs(&["a", "b"]);
    assert!(g
        .directory_action(&Cmd::AgentLaunch(comma), 100_000)
        .is_err());
    assert!(g
        .directory_action(&Cmd::AgentLaunch(signed), 100_000)
        .is_ok());
}
