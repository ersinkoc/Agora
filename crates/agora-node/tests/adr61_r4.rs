#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Regression tests from the fourth review of #446 (ADR-0061): who may endorse an expired or orphaned device again.

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
    let d = std::env::temp_dir().join(format!("adr61-r4-{n}-{}", std::process::id()));
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

fn entry(file: &std::path::Path, id: &str) -> Option<device_trust::Entry> {
    device_trust::read_entries(file)
        .0
        .into_iter()
        .find(|x| x.id == id)
}

/// R4-1: a limited device cannot squat an expired, unlimited owner-endorsed device with a pre-signed (hub-replayed) endorsement;
/// the owner can endorse it afresh, and the limited device can neither renew nor revoke it.
#[test]
fn r4_limited_device_cannot_squat_an_expired_unlimited_device() {
    let d = dir("squat");
    let file = d.join("trusted-devices");
    let (o, m, x) = (Key::new(), Key::new(), Key::new());
    device_trust::confirm(&file, "owner", &o.public()).unwrap();
    device_trust::confirm(&file, "m", &m.public()).unwrap();
    let p = pol(&[("m", &["web"])]);
    let t = DeviceTrust::new("laptop", &file);
    t.set_policy_limits(p.clone());
    let now = agora_node::grants::now_ms();
    t.endorse(&e_ttl(&o, "owner", "x", &x.public(), now, DAY), now)
        .unwrap();
    // m signs its endorsement now (refused now, but the hub keeps it and replays it on every reconnect)
    let squat = e_ttl(&m, "m", "x", &x.public(), now, 90 * DAY);
    assert_eq!(
        t.endorse(&squat, now).unwrap_err().code.as_str(),
        "auth.forbidden"
    );
    let at = entry(&file, "x").unwrap().expires_ms.unwrap();
    // At expiry, the replay does not land: m's limits do not cover an unlimited device (and it was signed before x ran out).
    assert_eq!(
        t.endorse(&squat, at).unwrap_err().code.as_str(),
        "auth.forbidden"
    );
    // Even one signed after it ran out does not: m cannot cover what x could do.
    let late = e_ttl(&m, "m", "x", &x.public(), at, 90 * DAY);
    assert_eq!(
        t.endorse(&late, at).unwrap_err().code.as_str(),
        "auth.forbidden"
    );
    assert_eq!(entry(&file, "x").unwrap().via.as_deref(), Some("owner"));
    // The owner endorses it afresh.
    t.endorse(
        &e_ttl(&o, "owner", "x", &x.public(), at + 1, 90 * DAY),
        at + 1,
    )
    .unwrap();
    assert!(limits(&file, p.clone()).allows_project("x", "api"));
    // m can neither renew it nor revoke it.
    assert_eq!(
        t.endorse(&e_ttl(&m, "m", "x", &x.public(), at + 2, 90 * DAY), at + 2)
            .unwrap_err()
            .code
            .as_str(),
        "auth.forbidden"
    );
    assert!(t.revoke(&rev(&m, "m", "x", at + 3), at + 3).is_err());
    assert!(t.trusted_ids(at + 3).contains(&"x".to_string()));
}

/// R4 LOW: another endorser asking for a SHORTER expiry of a still-valid device is refused the same way as for a longer one.
#[test]
fn r4_another_endorser_is_refused_whatever_expiry_it_asks() {
    let d = dir("shorter");
    let file = d.join("trusted-devices");
    let (o, other, x) = (Key::new(), Key::new(), Key::new());
    device_trust::confirm(&file, "owner", &o.public()).unwrap();
    device_trust::confirm(&file, "other", &other.public()).unwrap();
    let t = DeviceTrust::new("laptop", &file);
    let now = agora_node::grants::now_ms();
    t.endorse(&e_ttl(&o, "owner", "x", &x.public(), now, 30 * DAY), now)
        .unwrap();
    for ttl in [DAY, 60 * DAY] {
        let r = t.endorse(
            &e_ttl(&other, "other", "x", &x.public(), now + 1, ttl),
            now + 1,
        );
        assert_eq!(r.unwrap_err().code.as_str(), "auth.forbidden", "ttl {ttl}");
    }
    // Its own endorser asking for less is simply already covered.
    assert!(matches!(
        t.endorse(&e_ttl(&o, "owner", "x", &x.public(), now + 1, DAY), now + 1),
        Ok(device_trust::Endorsed::Already(_))
    ));
}

/// R4-2: cycles and self-endorsement written into the file terminate and limit to nothing.
#[test]
fn r4_cycles_terminate() {
    let d = dir("cycle");
    let file = d.join("trusted-devices");
    let (a, b, s) = (Key::new(), Key::new(), Key::new());
    let far = agora_node::grants::now_ms() + DAY;
    std::fs::write(
        &file,
        format!(
            "a {} expires={far} via=b\nb {} expires={far} via=a\ns {} expires={far} via=s\n",
            device_trust::hex(&a.public()),
            device_trust::hex(&b.public()),
            device_trust::hex(&s.public())
        ),
    )
    .unwrap();
    let t = DeviceTrust::new("laptop", &file);
    let now = agora_node::grants::now_ms();
    assert!(t.trusted_ids(now).is_empty());
    let l = limits(&file, BTreeMap::new());
    for dev in ["a", "b", "s"] {
        assert!(!l.allows_scope(dev, "node"), "{dev}");
    }
    t.renew("a", now);
    assert!(t
        .endorse(&e_ttl(&a, "a", "z", &Key::new().public(), now, DAY), now)
        .is_err());
}

/// R4-3: an orphaned entry re-endorsed by a limited device never gets more than that device's limits, and a confirmed device is never
/// downgraded.
#[test]
fn r4_orphan_and_confirmed() {
    let d = dir("orphan");
    let file = d.join("trusted-devices");
    let (m, x, o) = (Key::new(), Key::new(), Key::new());
    device_trust::confirm(&file, "m", &m.public()).unwrap();
    device_trust::confirm(&file, "owner", &o.public()).unwrap();
    let now = agora_node::grants::now_ms();
    let far = now + DAY;
    let mut text = std::fs::read_to_string(&file).unwrap();
    text.push_str(&format!(
        "x {} expires={far} via=gone\n",
        device_trust::hex(&x.public())
    ));
    std::fs::write(&file, text).unwrap();
    let p = pol(&[("m", &["web"])]);
    let t = DeviceTrust::new("laptop", &file);
    t.set_policy_limits(p.clone());
    t.endorse(&e_ttl(&m, "m", "x", &x.public(), now, DAY), now)
        .unwrap();
    let l = limits(&file, p.clone());
    assert!(l.allows_project("x", "web") && !l.allows_project("x", "api"));
    let r = t
        .endorse(&e_ttl(&m, "m", "owner", &o.public(), now, DAY), now)
        .unwrap();
    assert_eq!(r, device_trust::Endorsed::Already(None));
    assert!(limits(&file, p).allows_project("owner", "api"));
}
