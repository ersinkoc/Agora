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
