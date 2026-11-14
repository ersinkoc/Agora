#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Property test from the sixth review of #446 (ADR-0061): random endorse, renew, revoke, expire and narrow steps never escalate a
//! limit, downgrade a wider device, or lock the owner out. Deterministic seeds; `SEEDS=N` runs more.
use agora_node::device_trust::{
    self, endorse_message, revoke_message, DeviceTrust, Endorsed, Entry,
};
use agora_protocol::v1::{DeviceEndorsement, DeviceRevocation};
use ring::rand::SystemRandom;
use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

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
type Pol = BTreeMap<String, BTreeSet<String>>;
type Lim = Option<BTreeSet<String>>;

fn dir(n: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("adr61-r6p-{n}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}
fn copy_dir(from: &Path, to: &Path) {
    let _ = std::fs::remove_dir_all(to);
    std::fs::create_dir_all(to).unwrap();
    for f in std::fs::read_dir(from).unwrap() {
        let f = f.unwrap();
        if f.file_type().unwrap().is_file() {
            std::fs::copy(f.path(), to.join(f.file_name())).unwrap();
        }
    }
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
fn set(v: &[&str]) -> BTreeSet<String> {
    v.iter().map(|s| s.to_string()).collect()
}
fn within(a: &Lim, b: &Lim) -> bool {
    match (a, b) {
        (_, None) => true,
        (None, Some(_)) => false,
        (Some(x), Some(y)) => x.is_subset(y),
    }
}
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn pick(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}
#[derive(Debug, Clone, PartialEq)]
struct View {
    alive: bool,
    lim: Lim,
    via: Option<String>,
    exp: Option<u64>,
}
fn view(entries: &[Entry], p: &Pol, now: u64) -> BTreeMap<String, View> {
    entries
        .iter()
        .map(|e| {
            (
                e.id.clone(),
                View {
                    alive: device_trust::alive(entries, e, now),
                    lim: device_trust::effective_limit(entries, p, &e.id),
                    via: e.via.clone(),
                    exp: e.expires_ms,
                },
            )
        })
        .collect()
}

const IDS: [&str; 7] = ["o", "m1", "m2", "p", "q", "r", "s"];
const CAND: [&str; 4] = ["p", "q", "r", "s"];

#[test]
fn property_random_endorse_renew_revoke_expire_narrow() {
    let keys: BTreeMap<&str, Key> = IDS.iter().map(|i| (*i, Key::new())).collect();
    let mut stats: BTreeMap<&str, u32> = BTreeMap::new();
    let mut examples: BTreeMap<&str, String> = BTreeMap::new();
    let seeds = std::env::var("SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50u64);
    for seed in 1..=seeds {
        let d = dir(&format!("s{seed}"));
        let file = d.join("trusted-devices");
        for c in ["o", "m1", "m2"] {
            device_trust::confirm(&file, c, &keys[c].public()).unwrap();
        }
        let mut p: Pol = BTreeMap::new();
        p.insert("m1".into(), set(&["a", "b"]));
        p.insert("m2".into(), set(&["a"]));
        let t = DeviceTrust::new("node", &file);
        t.set_policy_limits(p.clone());
        let mut now = agora_node::grants::now_ms();
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut log: Vec<String> = vec![];
        for _step in 0..60 {
            let before = device_trust::read_entries(&file).0;
            let vb = view(&before, &p, now);
            let op = rng.pick(10);
            let signer = IDS[rng.pick(IDS.len())];
            let target = CAND[rng.pick(CAND.len())];
            let mut signed_by_limited: Option<Lim> = None;
            match op {
                0..=4 => {
                    let issued = [now, now - HOUR, now + 4 * MIN, now - 30 * DAY][rng.pick(4)];
                    let ttl = [DAY, 10 * DAY, 90 * DAY, HOUR][rng.pick(4)];
                    let r = t.endorse(
                        &end(
                            &keys[signer],
                            signer,
                            target,
                            &keys[target].public(),
                            issued,
                            ttl,
                        ),
                        now,
                    );
                    log.push(format!(
                        "t+{} endorse {signer}->{target} iss{:+} ttl{} = {:?}",
                        0,
                        issued as i64 - now as i64,
                        ttl,
                        r.as_ref()
                            .map(|_| ())
                            .map_err(|e| e.code.as_str().to_string())
                    ));
                    if r.is_ok() {
                        if let Some(v) = vb.get(signer) {
                            if v.lim.is_some() {
                                signed_by_limited = Some(v.lim.clone());
                            }
                        }
                    }
                    // a live endorsed device must never change its endorser
                    if let (Ok(Endorsed::Trusted(_)), Some(v)) = (&r, vb.get(target)) {
                        if v.alive && v.via.as_deref() != Some(signer) {
                            *stats.entry("live_takeover").or_default() += 1;
                            examples.entry("live_takeover").or_insert(log.join("\n"));
                        }
                    }
                }
                5 => {
                    let r = t.revoke(&rev(&keys[signer], signer, target, now), now);
                    log.push(format!(
                        "revoke {signer}x{target} = {:?}",
                        r.as_ref().map_err(|e| e.code.as_str().to_string())
                    ));
                    if r.is_ok() {
                        if let Some(v) = vb.get(signer) {
                            if v.lim.is_some() {
                                signed_by_limited = Some(v.lim.clone());
                            }
                        }
                    }
                }
                6 => {
                    t.renew(target, now);
                    log.push(format!("use {target}"));
                }
                7 => {
                    let dt = [HOUR, DAY, 11 * DAY, 91 * DAY][rng.pick(4)];
                    now += dt;
                    log.push(format!("advance {dt}"));
                }
                8 => {
                    let w = ["m1", "p", "q", "r"][rng.pick(4)];
                    let v = [set(&["a"]), set(&["a", "b"]), set(&["b"])][rng.pick(3)].clone();
                    p.insert(w.into(), v.clone());
                    t.set_policy_limits(p.clone());
                    log.push(format!("policy {w}={v:?}"));
                }
                _ => {
                    // owner recovery, in a copy: endorse directly if not alive, else revoke and endorse again
                    let cd = d.with_extension("copy");
                    copy_dir(&d, &cd);
                    let cf = cd.join("trusted-devices");
                    let ct = DeviceTrust::new("node", &cf);
                    ct.set_policy_limits(p.clone());
                    let alive = vb.get(target).map(|v| v.alive).unwrap_or(false);
                    if !alive && vb.contains_key(target) {
                        let r = ct.endorse(
                            &end(
                                &keys["o"],
                                "o",
                                target,
                                &keys[target].public(),
                                now,
                                30 * DAY,
                            ),
                            now,
                        );
                        if r.as_ref()
                            .is_err_and(|e| e.code.as_str() == "auth.forbidden")
                        {
                            *stats
                                .entry("owner_cannot_reendorse_dead_entry")
                                .or_default() += 1;
                            examples
                                .entry("owner_cannot_reendorse_dead_entry")
                                .or_insert(format!(
                                    "{}\n=> owner endorse {target}: {:?}\n{}",
                                    log.join("\n"),
                                    r.err().map(|e| e.code.as_str().to_string()),
                                    std::fs::read_to_string(&cf).unwrap()
                                ));
                        }
                    }
                    let _ = ct.revoke(&rev(&keys["o"], "o", target, now), now);
                    let r = ct.endorse(
                        &end(
                            &keys["o"],
                            "o",
                            target,
                            &keys[target].public(),
                            now + 1,
                            30 * DAY,
                        ),
                        now + 1,
                    );
                    let es = device_trust::read_entries(&cf).0;
                    let ok = es.iter().any(|e| {
                        e.id == target
                            && device_trust::alive(&es, e, now + 1)
                            && e.via.as_deref() == Some("o")
                    });
                    if !ok {
                        *stats
                            .entry("owner_cannot_recover_even_with_revoke")
                            .or_default() += 1;
                        examples
                            .entry("owner_cannot_recover_even_with_revoke")
                            .or_insert(format!(
                                "{}\n=> {:?}",
                                log.join("\n"),
                                r.err().map(|e| e.code.as_str().to_string())
                            ));
                    }
                }
            }
            let after = device_trust::read_entries(&file).0;
            let va = view(&after, &p, now);
            // I1: an endorsed device is never wider than its endorser (now) along the chain
            for (id, v) in &va {
                if let Some(via) = &v.via {
                    if let Some(pv) = va.get(via) {
                        if !within(&v.lim, &pv.lim) {
                            *stats.entry("wider_than_endorser").or_default() += 1;
                            examples
                                .entry("wider_than_endorser")
                                .or_insert(format!("{}\n{id}", log.join("\n")));
                        }
                    }
                }
            }
            // I2: a limited signer never removes/narrows/shortens/re-parents a device not within its own limit
            if let Some(mine) = signed_by_limited {
                for (id, v) in &vb {
                    if within(&v.lim, &mine) {
                        continue;
                    }
                    let a = va.get(id);
                    let harmed = match a {
                        None => true,
                        Some(a) => {
                            (v.alive && !a.alive)
                                || a.via != v.via
                                || !within(&v.lim, &a.lim)
                                || a.exp < v.exp
                        }
                    };
                    if harmed {
                        *stats.entry("limited_harms_wider").or_default() += 1;
                        examples
                            .entry("limited_harms_wider")
                            .or_insert(format!("{}\n{id}: {v:?} -> {a:?}", log.join("\n")));
                    }
                }
            }
        }
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(d.with_extension("copy"));
    }
    for (k, v) in &examples {
        eprintln!("=== {k} (count {})\n{v}\n", stats[k]);
    }
    eprintln!("STATS {stats:?}");
    assert!(stats.is_empty(), "invariant violations: {stats:?}");
}
