#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Regression tests from the independent review of #446 (ADR-0061 with ADR-0049 limits): each failed on the code before the fix.

use agora_node::approvals::{P256Verifier, Verifier};
use agora_node::device_trust::{self, endorse_message, revoke_message, DeviceTrust};
use agora_protocol::v1::{DeviceEndorsement, DeviceRevocation};
use ring::rand::SystemRandom;
use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
use std::collections::{BTreeMap, BTreeSet};

struct Key(EcdsaKeyPair);
impl Key {
    fn new() -> Self {
        let rng = SystemRandom::new();
        let p = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        Key(EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, p.as_ref(), &rng).unwrap())
    }
    fn public(&self) -> Vec<u8> {
        self.0.public_key().as_ref().to_vec()
    }
    fn sign(&self, m: &[u8]) -> Vec<u8> {
        self.0
            .sign(&SystemRandom::new(), m)
            .unwrap()
            .as_ref()
            .to_vec()
    }
}

fn dir(n: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("adr61-review-{n}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn endorse(by: &Key, by_id: &str, dev: &str, key: &[u8], issued: u64) -> DeviceEndorsement {
    let mut e = DeviceEndorsement {
        device_id: dev.into(),
        public_key: key.to_vec(),
        name: "x".into(),
        nodes: vec![],
        issued_at_ms: issued,
        expires_at_ms: issued + 24 * 3600 * 1000,
        endorser_device_id: by_id.into(),
        signature: vec![],
    };
    e.signature = by.sign(&endorse_message(&e));
    e
}

fn policy(device: &str, project: &str) -> BTreeMap<String, BTreeSet<String>> {
    BTreeMap::from([(device.to_string(), BTreeSet::from([project.to_string()]))])
}

/// H1: a device the policy limits to project `web` endorses a fresh key of its holder: the new device is limited to `web` too, also
/// for a node that starts again (the limit is kept in the trusted-devices file).
#[test]
fn a_limited_device_can_only_endorse_a_device_limited_the_same_way() {
    let d = dir("limits");
    let file = d.join("trusted-devices");
    let member = Key::new();
    device_trust::confirm(&file, "member-web", &member.public()).unwrap();
    let limits = agora_node::limits::DeviceLimits::new(policy("member-web", "web"));
    limits.follow_trust_file(&file);
    assert!(!limits.allows_scope("member-web", "node"));

    let fresh = Key::new();
    let now = agora_node::grants::now_ms();
    let t = DeviceTrust::new("laptop", &file);
    t.set_policy_limits(policy("member-web", "web"));
    t.endorse(
        &endorse(&member, "member-web", "member-new", &fresh.public(), now),
        now,
    )
    .unwrap();

    let (v, _) = P256Verifier::load(&file);
    let msg = b"anything";
    assert!(
        v.verify("member-new", msg, &fresh.sign(msg)),
        "it is trusted"
    );
    assert!(
        !limits.allows_scope("member-new", "node"),
        "but never for the whole machine"
    );
    assert!(!limits.allows_project("member-new", "api"));
    assert!(limits.allows_project("member-new", "web"));
    // After a restart (a new limits object reading the same files).
    let again = agora_node::limits::DeviceLimits::new(policy("member-web", "web"));
    again.follow_trust_file(&file);
    assert!(!again.allows_scope("member-new", "node"));
    assert!(!again.allows_project("member-new", "api"));
}

/// H1: a limited device cannot revoke the owner's (unlimited) device, only itself or devices limited within its own projects.
#[test]
fn a_limited_device_cannot_revoke_a_wider_device() {
    let d = dir("limited-revoke");
    let file = d.join("trusted-devices");
    let (owner, member, fresh) = (Key::new(), Key::new(), Key::new());
    device_trust::confirm(&file, "owner", &owner.public()).unwrap();
    device_trust::confirm(&file, "member-web", &member.public()).unwrap();
    let t = DeviceTrust::new("laptop", &file);
    t.set_policy_limits(policy("member-web", "web"));
    let now = agora_node::grants::now_ms();
    t.endorse(
        &endorse(&member, "member-web", "member-new", &fresh.public(), now),
        now,
    )
    .unwrap();
    let revoke = |by: &Key, by_id: &str, dev: &str, at: u64| {
        let mut r = DeviceRevocation {
            device_id: dev.into(),
            issued_at_ms: at,
            revoker_device_id: by_id.into(),
            signature: vec![],
        };
        r.signature = by.sign(&revoke_message(&r));
        r
    };
    let err = t
        .revoke(&revoke(&member, "member-web", "owner", now + 1), now + 1)
        .unwrap_err();
    assert_eq!(err.code.as_str(), "auth.forbidden");
    assert!(t.trusted_ids(now + 1).contains(&"owner".to_string()));
    assert!(t
        .revoke(
            &revoke(&member, "member-web", "member-new", now + 2),
            now + 2
        )
        .unwrap());
    assert!(t
        .revoke(&revoke(&owner, "owner", "member-web", now + 3), now + 3)
        .unwrap());
    assert_eq!(t.trusted_ids(now + 3), ["owner"]);
}

/// L1: an endorsement dated up to 5 minutes ahead does not outlive a revocation signed (honestly, "now") within that window.
#[test]
fn a_prompt_revocation_wins_over_a_future_dated_endorsement() {
    let d = dir("since");
    let file = d.join("trusted-devices");
    let (owner, rogue, x) = (Key::new(), Key::new(), Key::new());
    device_trust::confirm(&file, "owner", &owner.public()).unwrap();
    device_trust::confirm(&file, "rogue", &rogue.public()).unwrap();
    let t = DeviceTrust::new("laptop", &file);
    let now = agora_node::grants::now_ms();
    t.endorse(
        &endorse(&rogue, "rogue", "x", &x.public(), now + 4 * 60 * 1000),
        now,
    )
    .unwrap();
    let mut r = DeviceRevocation {
        device_id: "x".into(),
        issued_at_ms: now + 1000,
        revoker_device_id: "owner".into(),
        signature: vec![],
    };
    r.signature = owner.sign(&revoke_message(&r));
    assert!(t.revoke(&r, now + 1000).unwrap(), "the revocation applies");
    assert!(!t.trusted_ids(now + 2000).contains(&"x".to_string()));
}

/// L2: a revocation dated far in the future is refused, never remembered: it can neither block the device for good nor evict others.
#[test]
fn a_far_future_revocation_is_refused() {
    let d = dir("future-rev");
    let file = d.join("trusted-devices");
    let a = Key::new();
    device_trust::confirm(&file, "a", &a.public()).unwrap();
    let t = DeviceTrust::new("laptop", &file);
    let now = agora_node::grants::now_ms();
    let mut r = DeviceRevocation {
        device_id: "victim".into(),
        issued_at_ms: u64::MAX / 2,
        revoker_device_id: "a".into(),
        signature: vec![],
    };
    r.signature = a.sign(&revoke_message(&r));
    assert!(t.revoke(&r, now).is_err());
    let b = Key::new();
    let e = endorse(&a, "a", "victim", &b.public(), now);
    assert!(
        t.endorse(&e, now).is_ok(),
        "nothing about it was remembered"
    );
}
