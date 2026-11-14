#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Regression tests from the fifth review of #446 (ADR-0061): one rule for renewing and taking over an endorsed device.

use agora_node::device_trust::{self, endorse_message, DeviceTrust};
use agora_protocol::v1::DeviceEndorsement;
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
    let d = std::env::temp_dir().join(format!("adr61-r5-{n}-{}", std::process::id()));
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
fn pol(v: &[(&str, &[&str])]) -> BTreeMap<String, BTreeSet<String>> {
    v.iter()
        .map(|(d, p)| (d.to_string(), p.iter().map(|s| s.to_string()).collect()))
        .collect()
}

fn entry(file: &std::path::Path, id: &str) -> Option<device_trust::Entry> {
    device_trust::read_entries(file)
        .0
        .into_iter()
        .find(|x| x.id == id)
}

/// A1: owner renews its own endorsed device before expiry; the node was offline and gets it (hub replay on connect) after expiry.
#[test]
fn a1_its_own_endorser_renews_it_after_expiry_without_the_timing_rule() {
    let d = dir("a1");
    let file = d.join("trusted-devices");
    let (o, x) = (Key::new(), Key::new());
    device_trust::confirm(&file, "owner", &o.public()).unwrap();
    let t = DeviceTrust::new("laptop", &file);
    let now = agora_node::grants::now_ms();
    t.endorse(&e_ttl(&o, "owner", "x", &x.public(), now, DAY), now)
        .unwrap();
    let at = entry(&file, "x").unwrap().expires_ms.unwrap();
    // signed an hour before expiry by x's own endorser
    let renew = e_ttl(&o, "owner", "x", &x.public(), at - 3_600_000, 90 * DAY);
    let r = t.endorse(&renew, at + 60_000);
    // owner with a clock 1 min behind, signing 30 s after expiry (node time)
    let skewed = e_ttl(&o, "owner", "x", &x.public(), at - 30_000, 90 * DAY);
    let r2 = t.endorse(&skewed, at + 30_000);
    assert!(
        r.is_ok() && r2.is_ok(),
        "its own endorser renews it, late or with a skewed clock"
    );
    assert_eq!(entry(&file, "x").unwrap().via.as_deref(), Some("owner"));
}

/// A2: orphan through depth overflow (endorser re-endorsed afresh one level deeper) -> limited device takes it, owner locked out.
#[test]
fn a2_a_narrower_device_never_takes_an_orphan_from_a_wider_endorser() {
    let d = dir("a2");
    let file = d.join("trusted-devices");
    let (o, f, e, x, m) = (Key::new(), Key::new(), Key::new(), Key::new(), Key::new());
    device_trust::confirm(&file, "owner", &o.public()).unwrap();
    device_trust::confirm(&file, "m", &m.public()).unwrap();
    let p = pol(&[("m", &["web"])]);
    let t = DeviceTrust::new("laptop", &file);
    t.set_policy_limits(p.clone());
    let now = agora_node::grants::now_ms();
    t.endorse(&e_ttl(&o, "owner", "f", &f.public(), now, 90 * DAY), now)
        .unwrap();
    t.endorse(&e_ttl(&o, "owner", "e", &e.public(), now, DAY), now)
        .unwrap();
    t.endorse(&e_ttl(&e, "e", "x", &x.public(), now, DAY), now)
        .unwrap();
    let at = entry(&file, "e").unwrap().expires_ms.unwrap();
    // f (unlimited, depth 1) endorses e afresh after it ran out: e is now depth 2, x depth 3.
    t.endorse(&e_ttl(&f, "f", "e", &e.public(), at, 30 * DAY), at)
        .unwrap();
    let lim = device_trust::effective_limit(&device_trust::read_entries(&file).0, &p, "x");
    assert_eq!(
        lim,
        Some(BTreeSet::new()),
        "x's chain is broken: it may do nothing"
    );
    let take = e_ttl(&m, "m", "x", &x.public(), at + 1, 30 * DAY);
    let r = t.endorse(&take, at + 1);
    assert_eq!(
        r.unwrap_err().code.as_str(),
        "auth.forbidden",
        "m does not cover x's endorser e"
    );
    let r2 = t.endorse(
        &e_ttl(&o, "owner", "x", &x.public(), at + 2, 30 * DAY),
        at + 2,
    );
    assert!(r2.is_ok(), "{:?}", r2.as_ref().map_err(|e| e.code.as_str()));
    assert_eq!(
        entry(&file, "x").unwrap().via.as_deref(),
        Some("owner"),
        "the owner re-parents it"
    );
}

/// A3: orphan whose own expiry is still ahead (endorser line lost) -> timing rule skipped; stale limited endorsement lands.
#[test]
fn a3_an_orphan_is_not_taken_with_an_endorsement_signed_before_it_ran_out() {
    let d = dir("a3");
    let file = d.join("trusted-devices");
    let (o, e, x, m) = (Key::new(), Key::new(), Key::new(), Key::new());
    device_trust::confirm(&file, "owner", &o.public()).unwrap();
    device_trust::confirm(&file, "m", &m.public()).unwrap();
    let p = pol(&[("m", &["web"])]);
    let t = DeviceTrust::new("laptop", &file);
    t.set_policy_limits(p.clone());
    let now = agora_node::grants::now_ms();
    let stale = e_ttl(&m, "m", "x", &x.public(), now, 90 * DAY); // signed first, kept by hub
    t.endorse(&e_ttl(&o, "owner", "e", &e.public(), now, 30 * DAY), now)
        .unwrap();
    t.endorse(&e_ttl(&e, "e", "x", &x.public(), now, 30 * DAY), now)
        .unwrap();
    // e's line becomes unreadable (e.g. hand edit / a newer field) -> x orphaned, own expiry 30 days ahead
    let txt = std::fs::read_to_string(&file).unwrap();
    let txt: String = txt
        .lines()
        .filter(|l| !l.starts_with("e "))
        .map(|l| format!("{l}\n"))
        .collect();
    std::fs::write(&file, txt).unwrap();
    let r = t.endorse(&stale, now + 1000);
    assert_eq!(
        r.unwrap_err().code.as_str(),
        "auth.forbidden",
        "signed before its trust ran out"
    );
}
