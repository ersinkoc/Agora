#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Regression tests from the third review of #446 (ADR-0061 limits along endorsement chains): each failed on c83c18e.

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
const DAY: u64 = 86_400_000;
fn e_ttl(by: &Key, by_id: &str, dev: &str, key: &[u8], issued: u64, ttl: u64) -> DeviceEndorsement {
    let mut e = endorse(by, by_id, dev, key, issued);
    e.expires_at_ms = issued + ttl;
    e.signature = by.sign(&endorse_message(&e));
    e
}
fn rev(by: &Key, by_id: &str, dev: &str, at: u64) -> DeviceRevocation {
    let mut r = DeviceRevocation {
        device_id: dev.into(),
        issued_at_ms: at,
        revoker_device_id: by_id.into(),
        signature: vec![],
    };
    r.signature = by.sign(&revoke_message(&r));
    r
}
fn pol(v: &[(&str, &[&str])]) -> BTreeMap<String, BTreeSet<String>> {
    v.iter()
        .map(|(d, p)| (d.to_string(), p.iter().map(|s| s.to_string()).collect()))
        .collect()
}
fn limits(
    file: &std::path::Path,
    p: BTreeMap<String, BTreeSet<String>>,
) -> agora_node::limits::DeviceLimits {
    let l = agora_node::limits::DeviceLimits::new(p);
    l.follow_trust_file(file);
    l
}

/// A: policy limits added/narrowed on an endorser later do not reach the devices it endorsed; renew-by-use keeps them forever.
#[test]
fn a_demotion_reaches_the_devices_endorsed_before() {
    let d = dir("demote");
    let file = d.join("trusted-devices");
    let u = Key::new();
    device_trust::confirm(&file, "u", &u.public()).unwrap();
    let now = agora_node::grants::now_ms();
    let t = DeviceTrust::new("laptop", &file);
    t.endorse(&e_ttl(&u, "u", "k", &Key::new().public(), now, DAY), now)
        .unwrap();
    // also a narrowing case: w {web,api} endorses j, then is narrowed to {web}
    let w = Key::new();
    device_trust::confirm(&file, "w", &w.public()).unwrap();
    t.set_policy_limits(pol(&[("w", &["web", "api"])]));
    t.endorse(&e_ttl(&w, "w", "j", &Key::new().public(), now, DAY), now)
        .unwrap();
    // Restart with the owner's new policy: u limited to web, w narrowed to web.
    let p = pol(&[("u", &["web"]), ("w", &["web"])]);
    let t2 = DeviceTrust::new("laptop", &file);
    t2.set_policy_limits(p.clone());
    let l = limits(&file, p);
    assert!(!l.allows_scope("u", "node"));
    assert!(!l.allows_project("w", "api"));
    let k_node = l.allows_scope("k", "node");
    let j_api = l.allows_project("j", "api");
    // renew by use: k lives 90 days from each use, no limit check
    t2.renew("k", now + DAY / 2 + 1);
    let k_alive_later = t2.trusted_ids(now + 30 * DAY).contains(&"k".to_string());
    // and u (now limited) cannot revoke k
    let u_revoke = t2.revoke(&rev(&u, "u", "k", now + 1), now + 1).is_ok();
    assert!(!k_node, "k follows u's demotion");
    assert!(!j_api, "j follows w's narrowing");
    assert!(
        !k_alive_later,
        "a use no longer covered by its endorser does not renew it"
    );
    assert!(
        u_revoke,
        "u may remove what it endorsed, which is within its limits now"
    );
}

/// B: empty intersection is an empty limit, never "unlimited", and survives the file and a depth-2 chain + restart.
#[test]
fn an_empty_intersection_stays_limited_through_a_chain_and_a_restart() {
    let d = dir("empty");
    let file = d.join("trusted-devices");
    let m = Key::new();
    device_trust::confirm(&file, "m", &m.public()).unwrap();
    let p = pol(&[("m", &["web"]), ("x", &["api"])]);
    let t = DeviceTrust::new("laptop", &file);
    t.set_policy_limits(p.clone());
    let now = agora_node::grants::now_ms();
    let xk = Key::new();
    t.endorse(&e_ttl(&m, "m", "x", &xk.public(), now, DAY), now)
        .unwrap();
    t.endorse(&e_ttl(&xk, "x", "y", &Key::new().public(), now, DAY), now)
        .unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("projects="), "{text}");
    let t2 = DeviceTrust::new("laptop", &file);
    t2.set_policy_limits(p.clone());
    let l = limits(&file, p);
    for dev in ["x", "y"] {
        for s in ["node", "project:web", "project:api"] {
            assert!(!l.allows_scope(dev, s), "{dev} {s}");
        }
    }
    // y (empty limit) cannot revoke m (unlimited? no, m is {web}) -- {} subset of {web}: m may revoke y; y may not revoke m
    assert!(t2.revoke(&rev(&xk, "x", "m", now + 1), now + 1).is_err());
}

/// C: a different endorser with equal limits cannot renew; an expired endorsee then blocks the id for everyone, the owner too.
#[test]
fn another_endorser_cannot_renew_but_an_expired_entry_can_be_endorsed_afresh() {
    let d = dir("other");
    let file = d.join("trusted-devices");
    let (a, b, o) = (Key::new(), Key::new(), Key::new());
    device_trust::confirm(&file, "a", &a.public()).unwrap();
    device_trust::confirm(&file, "b", &b.public()).unwrap();
    device_trust::confirm(&file, "owner", &o.public()).unwrap();
    let t = DeviceTrust::new("laptop", &file);
    t.set_policy_limits(pol(&[("a", &["web"]), ("b", &["web"])]));
    let now = agora_node::grants::now_ms();
    let ck = Key::new();
    t.endorse(&e_ttl(&a, "a", "c", &ck.public(), now, DAY), now)
        .unwrap();
    let r = t.endorse(
        &e_ttl(&b, "b", "c", &ck.public(), now + 1, 2 * DAY),
        now + 1,
    );
    assert_eq!(r.unwrap_err().code.as_str(), "auth.forbidden");
    // after c expires, the unlimited owner cannot re-endorse it either
    let later = now + 2 * DAY;
    let r = t.endorse(&e_ttl(&o, "owner", "c", &ck.public(), later, DAY), later);
    assert!(r.is_ok(), "{:?}", r.as_ref().map_err(|e| e.code.as_str()));
    let c = device_trust::read_entries(&file)
        .0
        .into_iter()
        .find(|x| x.id == "c")
        .unwrap();
    assert_eq!(
        (c.via.as_deref(), c.projects),
        (Some("owner"), None),
        "now the owner's"
    );
}

/// D: a revoked endorser cannot renew, and its endorsee is gone.
#[test]
fn a_revoked_endorser_cannot_renew() {
    let d = dir("revoked");
    let file = d.join("trusted-devices");
    let (o, a) = (Key::new(), Key::new());
    device_trust::confirm(&file, "owner", &o.public()).unwrap();
    let t = DeviceTrust::new("laptop", &file);
    let now = agora_node::grants::now_ms();
    t.endorse(&e_ttl(&o, "owner", "a", &a.public(), now, 10 * DAY), now)
        .unwrap();
    let ck = Key::new();
    t.endorse(&e_ttl(&a, "a", "c", &ck.public(), now, DAY), now)
        .unwrap();
    assert!(t.revoke(&rev(&o, "owner", "a", now + 1), now + 1).unwrap());
    assert!(t
        .endorse(
            &e_ttl(&a, "a", "c", &ck.public(), now + 2, 2 * DAY),
            now + 2
        )
        .is_err());
    assert_eq!(t.trusted_ids(now + 2), ["owner"]);
}
