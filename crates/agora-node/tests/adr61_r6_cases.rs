#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Regression tests from the sixth review of #446 (ADR-0061): takeover timing on the node's clock, and endorsements signed before their endorser was revoked.
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
const MIN: u64 = 60_000;
const HOUR: u64 = 3_600_000;
const DAY: u64 = 86_400_000;
fn dir(n: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("adr61-r6c-{n}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}
fn end(by: &Key, by_id: &str, dev: &str, key: &[u8], issued: u64, ttl: u64) -> DeviceEndorsement {
    let mut e = DeviceEndorsement {
        device_id: dev.into(),
        public_key: key.to_vec(),
        name: "x".into(),
        nodes: vec![],
        issued_at_ms: issued,
        expires_at_ms: issued + ttl,
        endorser_device_id: by_id.into(),
        signature: vec![],
    };
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
fn entry(f: &std::path::Path, id: &str) -> Option<device_trust::Entry> {
    device_trust::read_entries(f)
        .0
        .into_iter()
        .find(|x| x.id == id)
}
fn alive(f: &std::path::Path, id: &str, now: u64) -> bool {
    let es = device_trust::read_entries(f).0;
    es.iter()
        .any(|e| e.id == id && device_trust::alive(&es, e, now))
}

/// (4) An equal-limit peer takes over a STILL-VALID device in the last 5 minutes before it runs out, by dating its endorsement
/// ahead within the accepted skew; the device's own endorser is then locked out for up to 90 days.
#[test]
fn c1_an_equal_peer_cannot_take_over_a_live_device_inside_the_skew_window() {
    let d = dir("c1");
    let f = d.join("trusted-devices");
    let (m2, m3, p) = (Key::new(), Key::new(), Key::new());
    device_trust::confirm(&f, "m2", &m2.public()).unwrap();
    device_trust::confirm(&f, "m3", &m3.public()).unwrap();
    let t = DeviceTrust::new("n", &f);
    t.set_policy_limits(pol(&[("m2", &["a"]), ("m3", &["a"])]));
    let now = agora_node::grants::now_ms();
    t.endorse(&end(&m2, "m2", "p", &p.public(), now, HOUR), now)
        .unwrap();
    let at = now + 57 * MIN; // p is valid for 3 more minutes
    assert!(alive(&f, "p", at));
    let r = t.endorse(
        &end(&m3, "m3", "p", &p.public(), at + 4 * MIN, 90 * DAY),
        at,
    );
    assert_eq!(
        r.unwrap_err().code.as_str(),
        "auth.forbidden",
        "a live device is not taken over inside the skew window"
    );
    let back = t.endorse(
        &end(&m2, "m2", "p", &p.public(), at + MIN, 90 * DAY),
        at + MIN,
    );
    assert!(
        back.is_ok(),
        "its own endorser still renews it: {:?}",
        back.map_err(|e| e.code.as_str().to_string())
    );
    assert_eq!(
        entry(&f, "p").unwrap().via.as_deref(),
        Some("m2"),
        "a live device changed endorser"
    );
}

/// (4) After it runs out, equal peers: whoever lands first keeps it; no flip-flop while valid (outside the skew window).
#[test]
fn c2_equal_peers_do_not_flip_flop_after_expiry() {
    let d = dir("c2");
    let f = d.join("trusted-devices");
    let (m2, m3, p) = (Key::new(), Key::new(), Key::new());
    device_trust::confirm(&f, "m2", &m2.public()).unwrap();
    device_trust::confirm(&f, "m3", &m3.public()).unwrap();
    let t = DeviceTrust::new("n", &f);
    t.set_policy_limits(pol(&[("m2", &["a"]), ("m3", &["a"])]));
    let now = agora_node::grants::now_ms();
    t.endorse(&end(&m2, "m2", "p", &p.public(), now, HOUR), now)
        .unwrap();
    let late = now + 2 * HOUR;
    t.endorse(&end(&m3, "m3", "p", &p.public(), late, 90 * DAY), late)
        .unwrap();
    let r = t.endorse(
        &end(&m2, "m2", "p", &p.public(), late + MIN, 90 * DAY),
        late + MIN,
    );
    assert!(r.is_err(), "m2 cannot take it back while valid");
    assert_eq!(entry(&f, "p").unwrap().via.as_deref(), Some("m3"));
}

/// (3) A wider-but-limited device takes over a sibling's run-out device: the device widens to the taker's limit; and the taker could
/// already revoke the sibling's devices (and the sibling's confirmed device) without the takeover.
#[test]
fn c3_wider_limited_takeover_widens_the_device_and_adds_no_revoke_power() {
    let d = dir("c3");
    let f = d.join("trusted-devices");
    let (m1, m2, p, q) = (Key::new(), Key::new(), Key::new(), Key::new());
    device_trust::confirm(&f, "m1", &m1.public()).unwrap();
    device_trust::confirm(&f, "m2", &m2.public()).unwrap();
    let pl = pol(&[("m1", &["a", "b"]), ("m2", &["a"])]);
    let t = DeviceTrust::new("n", &f);
    t.set_policy_limits(pl.clone());
    let now = agora_node::grants::now_ms();
    t.endorse(&end(&m2, "m2", "p", &p.public(), now, HOUR), now)
        .unwrap();
    t.endorse(&end(&m2, "m2", "q", &q.public(), now, 30 * DAY), now)
        .unwrap();
    // without any takeover, m1 may revoke m2's q (and m2 itself)
    let d2 = dir("c3b");
    let f2 = d2.join("trusted-devices");
    std::fs::copy(&f, &f2).unwrap();
    let t2 = DeviceTrust::new("n", &f2);
    t2.set_policy_limits(pl.clone());
    assert!(t2.revoke(&rev(&m1, "m1", "q", now + 1), now + 1).unwrap());
    assert!(
        t2.revoke(&rev(&m1, "m1", "m2", now + 1), now + 1).unwrap(),
        "a limited member removes another member's confirmed device"
    );
    let late = now + 2 * HOUR;
    t.endorse(&end(&m1, "m1", "p", &p.public(), late, 90 * DAY), late)
        .unwrap();
    let lim = device_trust::effective_limit(&device_trust::read_entries(&f).0, &pl, "p");
    assert_eq!(
        lim,
        Some(["a".to_string(), "b".to_string()].into()),
        "p widened from {{a}} to {{a,b}}"
    );
    // m2 (own endorser before) cannot take it back: m2 does not cover m1
    assert!(t
        .endorse(
            &end(&m2, "m2", "p", &p.public(), late + 91 * DAY, 90 * DAY),
            late + 91 * DAY
        )
        .is_err());
}

/// (2) Own-endorser renewal when that endorser was narrowed, ran out, or was revoked.
#[test]
fn c4_endorsements_signed_before_their_endorser_was_revoked_stay_refused() {
    let d = dir("c4");
    let f = d.join("trusted-devices");
    let (o, v, x) = (Key::new(), Key::new(), Key::new());
    device_trust::confirm(&f, "o", &o.public()).unwrap();
    let t = DeviceTrust::new("n", &f);
    let now = agora_node::grants::now_ms();
    t.endorse(&end(&o, "o", "v", &v.public(), now, 10 * DAY), now)
        .unwrap();
    t.endorse(&end(&v, "v", "x", &x.public(), now, DAY), now)
        .unwrap();
    // narrowed: v -> {a}; x was given everything
    t.set_policy_limits(pol(&[("v", &["a"])]));
    let r = t.endorse(
        &end(&v, "v", "x", &x.public(), now + 2 * DAY, 30 * DAY),
        now + 2 * DAY,
    );
    assert!(r.is_err(), "narrowed endorser renews a wider device");
    // v (narrowed) cannot take it over afresh either (via == v takes the renewal branch)
    t.set_policy_limits(pol(&[]));
    // expired endorser
    let r = t.endorse(
        &end(&v, "v", "x", &x.public(), now + 11 * DAY, 30 * DAY),
        now + 11 * DAY,
    );
    assert!(r.is_err(), "an expired endorser renews");
    // owner renews v, v renews x late (allowed by design), then o revokes v: x goes too, and v's old endorsement replays are refused
    t.endorse(
        &end(&o, "o", "v", &v.public(), now + 11 * DAY, 30 * DAY),
        now + 11 * DAY,
    )
    .unwrap();
    let old = end(&v, "v", "x", &x.public(), now + 11 * DAY, 30 * DAY);
    t.endorse(&old, now + 11 * DAY).unwrap();
    t.revoke(&rev(&o, "o", "v", now + 12 * DAY), now + 12 * DAY)
        .unwrap();
    assert!(entry(&f, "x").is_none());
    t.endorse(
        &end(&o, "o", "v", &v.public(), now + 12 * DAY + 1, 30 * DAY),
        now + 12 * DAY + 1,
    )
    .unwrap();
    let r = t.endorse(&old, now + 13 * DAY);
    assert!(
        r.is_err(),
        "x came back through a replayed endorsement signed before v was revoked"
    );
}
