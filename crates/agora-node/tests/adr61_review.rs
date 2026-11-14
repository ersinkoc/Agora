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

fn endorse_for(
    by: &Key,
    by_id: &str,
    dev: &str,
    key: &[u8],
    issued: u64,
    ttl: u64,
) -> DeviceEndorsement {
    let mut e = endorse(by, by_id, dev, key, issued);
    e.expires_at_ms = issued + ttl;
    e.signature = by.sign(&endorse_message(&e));
    e
}

fn revocation(by: &Key, by_id: &str, dev: &str, at: u64) -> DeviceRevocation {
    let mut r = DeviceRevocation {
        device_id: dev.into(),
        issued_at_ms: at,
        revoker_device_id: by_id.into(),
        signature: vec![],
    };
    r.signature = by.sign(&revoke_message(&r));
    r
}

const DAY: u64 = 86_400_000;

/// Second review, HIGH: a limited device endorses a fresh key under an id the policy names more widely. The policy narrows what
/// was inherited, never widens it.
#[test]
fn an_id_the_policy_names_widely_does_not_widen_an_inherited_limit() {
    let d = dir("collide");
    let file = d.join("trusted-devices");
    let m = Key::new();
    device_trust::confirm(&file, "m-web", &m.public()).unwrap();
    let policy: BTreeMap<String, BTreeSet<String>> = BTreeMap::from([
        ("m-web".into(), BTreeSet::from(["web".into()])),
        (
            "lead-laptop".into(),
            BTreeSet::from(["web".into(), "api".into(), "prod".into()]),
        ),
    ]);
    let t = DeviceTrust::new("laptop", &file);
    t.set_policy_limits(policy.clone());
    let limits = agora_node::limits::DeviceLimits::new(policy);
    limits.follow_trust_file(&file);
    let fresh = Key::new();
    let now = agora_node::grants::now_ms();
    t.endorse(
        &endorse_for(&m, "m-web", "lead-laptop", &fresh.public(), now, DAY),
        now,
    )
    .unwrap();
    assert!(!limits.allows_project("m-web", "prod"));
    assert!(
        !limits.allows_project("lead-laptop", "prod"),
        "not escalated"
    );
    assert!(limits.allows_project("lead-laptop", "web"));
}

/// Second review, MEDIUM: a limited device cannot re-endorse a wider device to take it over (downgrade, then revoke).
#[test]
fn a_limited_device_cannot_take_over_a_wider_device_by_endorsing_it_again() {
    let d = dir("downgrade");
    let file = d.join("trusted-devices");
    let (owner, m, e) = (Key::new(), Key::new(), Key::new());
    device_trust::confirm(&file, "owner", &owner.public()).unwrap();
    device_trust::confirm(&file, "m-web", &m.public()).unwrap();
    let policy: BTreeMap<String, BTreeSet<String>> =
        BTreeMap::from([("m-web".into(), BTreeSet::from(["web".into()]))]);
    let t = DeviceTrust::new("laptop", &file);
    t.set_policy_limits(policy.clone());
    let limits = agora_node::limits::DeviceLimits::new(policy);
    limits.follow_trust_file(&file);
    let now = agora_node::grants::now_ms();
    t.endorse(
        &endorse_for(&owner, "owner", "owner-phone", &e.public(), now, 10 * DAY),
        now,
    )
    .unwrap();
    assert!(limits.allows_scope("owner-phone", "node"));
    assert!(t
        .revoke(&revocation(&m, "m-web", "owner-phone", now + 1), now + 1)
        .is_err());
    let again = endorse_for(&m, "m-web", "owner-phone", &e.public(), now + 2, 20 * DAY);
    assert_eq!(
        t.endorse(&again, now + 2).unwrap_err().code.as_str(),
        "auth.forbidden"
    );
    assert!(limits.allows_scope("owner-phone", "node"), "not downgraded");
    assert!(t
        .revoke(&revocation(&m, "m-web", "owner-phone", now + 3), now + 3)
        .is_err());
    assert!(t.trusted_ids(now + 3).contains(&"owner-phone".to_string()));
    // Its own endorser renews it and nothing else changes.
    t.endorse(
        &endorse_for(
            &owner,
            "owner",
            "owner-phone",
            &e.public(),
            now + 4,
            30 * DAY,
        ),
        now + 4,
    )
    .unwrap();
    let entry = device_trust::read_entries(&file)
        .0
        .into_iter()
        .find(|x| x.id == "owner-phone")
        .unwrap();
    assert_eq!(
        (entry.via.as_deref(), entry.projects),
        (Some("owner"), None)
    );
}

/// Second review, LOW: a project name the trusted-devices file cannot hold makes the endorsement fail loudly, never a device that
/// silently loses its limit.
#[test]
fn a_project_name_the_file_cannot_hold_is_refused() {
    let d = dir("spaces");
    let file = d.join("trusted-devices");
    let m = Key::new();
    device_trust::confirm(&file, "m", &m.public()).unwrap();
    let t = DeviceTrust::new("laptop", &file);
    t.set_policy_limits(BTreeMap::from([(
        "m".into(),
        BTreeSet::from(["my web".into()]),
    )]));
    let now = agora_node::grants::now_ms();
    let err = t
        .endorse(
            &endorse_for(&m, "m", "m2", &Key::new().public(), now, DAY),
            now,
        )
        .unwrap_err();
    assert_eq!(err.code.as_str(), "node.policy_denied");
    assert_eq!(t.trusted_ids(now), ["m"]);
}
