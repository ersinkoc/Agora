//! Which devices this machine trusts, and how that changes (ADR-0061).
//!
//! The trusted-devices file (`<data>/trusted-devices`, owner-only) holds one device per line: `<id> <public key as 130 hex digits>`,
//! as `agora node trust ID KEY` has always written it. ADR-0061 adds optional fields after the key, `expires=<ms>` and `via=<device>`,
//! for a device another device endorsed. A line with any other field is skipped, never trusted, so a typo cannot widen trust; a node
//! from before ADR-0061 skips such a line too (it trusts less, never more).
//!
//! Trust grows only two ways: the owner says yes on the machine (`agora node trust`, `agora node enroll`, `agora node trust
//! --from-hub`), or a device this machine already trusts signs an endorsement, verified here ([`DeviceTrust::endorse`]). The hub only
//! carries endorsements; it can neither make nor change one. A revocation signed by a trusted device ([`DeviceTrust::revoke`]) and
//! `agora node untrust` narrow it at once, and are remembered (`<data>/device-revocations`) so an older endorsement the hub sends again
//! cannot bring the device back. Every endorsement and revocation applied or refused is written to `<data>/device-trust.log`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use agora_protocol::v1::{DeviceEndorsement, DeviceRevocation};
use agora_protocol::{Error, ErrorCode};

/// The longest an endorsement lasts from when it was signed, and how far each use renews it (ADR-0061: 90 days, renewable by use).
pub const MAX_ENDORSEMENT_MS: u64 = 90 * 24 * 60 * 60 * 1000;
/// How far ahead of this machine's clock a signer's clock may be.
const SKEW_MS: u64 = 5 * 60 * 1000;
/// A renewal is written only when it moves the expiry by more than this (a device used all day writes once).
const RENEW_STEP_MS: u64 = 24 * 60 * 60 * 1000;

/// Remembered revocations, one `<device id> <issued ms>` per line.
pub const REVOCATIONS_FILE: &str = "device-revocations";
/// What this machine did with each endorsement and revocation, one line each, for its owner.
pub const AUDIT_FILE: &str = "device-trust.log";

/// One device of the trusted-devices file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub key: Vec<u8>,
    /// `None`: confirmed on the machine, no end. `Some`: endorsed, trusted until then.
    pub expires_ms: Option<u64>,
    /// The device that endorsed it.
    pub via: Option<String>,
}

impl Entry {
    pub fn live(&self, now_ms: u64) -> bool {
        self.expires_ms.is_none_or(|e| now_ms < e)
    }

    fn line(&self) -> String {
        let mut l = format!("{} {}", self.id, hex(&self.key));
        if let Some(e) = self.expires_ms {
            l.push_str(&format!(" expires={e}"));
        }
        if let Some(v) = &self.via {
            l.push_str(&format!(" via={v}"));
        }
        l
    }
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn from_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || !s.is_ascii() {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// `[A-Za-z0-9._-]{1,64}`: what the file can hold.
pub fn valid_device_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// An uncompressed P-256 point encoding (65 bytes, `04` first).
pub fn valid_key(key: &[u8]) -> bool {
    key.len() == 65 && key.first() == Some(&4)
}

/// One line of the file: `None` for a blank line or a comment, `Some(None)` for a line that does not parse (skipped, never
/// trusted), `Some(Some(entry))` for a device.
pub fn parse_line(line: &str) -> Option<Option<Entry>> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let mut words = line.split_whitespace();
    let (Some(id), Some(key)) = (words.next(), words.next()) else {
        return Some(None);
    };
    let Some(key) = from_hex(key).filter(|k| valid_device_id(id) && valid_key(k)) else {
        return Some(None);
    };
    let mut entry = Entry {
        id: id.to_string(),
        key,
        expires_ms: None,
        via: None,
    };
    for w in words {
        match w.split_once('=') {
            Some(("expires", v)) if entry.expires_ms.is_none() => match v.parse() {
                Ok(ms) => entry.expires_ms = Some(ms),
                Err(_) => return Some(None),
            },
            Some(("via", v)) if entry.via.is_none() && valid_device_id(v) => {
                entry.via = Some(v.to_string());
            }
            _ => return Some(None),
        }
    }
    Some(Some(entry))
}

/// Every device the file lists (the last line wins for an id written twice) and how many lines were skipped.
pub fn read_entries(path: &Path) -> (Vec<Entry>, usize) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return (Vec::new(), 0);
    };
    let mut out: Vec<Entry> = Vec::new();
    let mut skipped = 0;
    for line in text.lines() {
        match parse_line(line) {
            None => {}
            Some(None) => skipped += 1,
            Some(Some(e)) => {
                out.retain(|o| o.id != e.id);
                out.push(e);
            }
        }
    }
    (out, skipped)
}

/// Replaces the line of `id` (or adds `entry` at the end; `None` removes it), keeping every other line as it was. Written beside the
/// file and renamed over it, owner-only from the start (`agora_platform::write_private`).
fn rewrite(path: &Path, id: &str, entry: Option<&Entry>) -> std::io::Result<()> {
    let old = std::fs::read_to_string(path).unwrap_or_default();
    let mut out: String = old
        .lines()
        .filter(|l| l.split_whitespace().next() != Some(id))
        .map(|l| format!("{l}\n"))
        .collect();
    if let Some(e) = entry {
        out.push_str(&e.line());
        out.push('\n');
    }
    agora_platform::write_private(path, out.as_bytes())
}

/// The owner on the machine trusts `id` with `key`, with no end (`agora node trust`, and a yes at enrollment or `--from-hub`). A
/// remembered revocation of it is forgotten: the person at the machine just decided otherwise.
pub fn confirm(file: &Path, id: &str, key: &[u8]) -> Result<(), String> {
    if !valid_device_id(id) {
        return Err("the device id may contain only letters, digits, '.', '_' and '-'".into());
    }
    if !valid_key(key) {
        return Err("the key is not an uncompressed P-256 point (65 bytes)".into());
    }
    rewrite(
        file,
        id,
        Some(&Entry {
            id: id.to_string(),
            key: key.to_vec(),
            expires_ms: None,
            via: None,
        }),
    )
    .map_err(|e| e.to_string())?;
    forget_revocation(&file.with_file_name(REVOCATIONS_FILE), id).map_err(|e| e.to_string())
}

/// The owner on the machine stops trusting `id` (`agora node untrust`): removed, and remembered as revoked now so the hub cannot bring
/// it back with an endorsement it carried before. `Ok(false)`: it was not trusted (it is remembered as revoked all the same).
pub fn untrust(file: &Path, id: &str, now_ms: u64) -> Result<bool, String> {
    if !valid_device_id(id) {
        return Err("the device id may contain only letters, digits, '.', '_' and '-'".into());
    }
    let was = read_entries(file).0.iter().any(|e| e.id == id);
    if was {
        rewrite(file, id, None).map_err(|e| e.to_string())?;
    }
    remember_revocation(&file.with_file_name(REVOCATIONS_FILE), id, now_ms)
        .map_err(|e| e.to_string())?;
    audit(
        &file.with_file_name(AUDIT_FILE),
        now_ms,
        "untrust",
        id,
        "owner",
        "ok",
    );
    Ok(was)
}

fn read_revocations(path: &Path) -> Vec<(String, u64)> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let (id, at) = l.trim().split_once(' ')?;
            Some((id.to_string(), at.trim().parse().ok()?))
        })
        .collect()
}

fn write_revocations(path: &Path, list: &[(String, u64)]) -> std::io::Result<()> {
    let text: String = list.iter().map(|(id, at)| format!("{id} {at}\n")).collect();
    agora_platform::write_private(path, text.as_bytes())
}

/// Keeps the latest revocation time of `id`.
fn remember_revocation(path: &Path, id: &str, at_ms: u64) -> std::io::Result<()> {
    let mut list = read_revocations(path);
    let latest = list
        .iter()
        .filter(|(d, _)| d == id)
        .map(|(_, a)| *a)
        .max()
        .unwrap_or(0)
        .max(at_ms);
    list.retain(|(d, _)| d != id);
    list.push((id.to_string(), latest));
    write_revocations(path, &list)
}

fn forget_revocation(path: &Path, id: &str) -> std::io::Result<()> {
    let mut list = read_revocations(path);
    let before = list.len();
    list.retain(|(d, _)| d != id);
    if list.len() == before {
        return Ok(());
    }
    write_revocations(path, &list)
}

/// One line of the machine's own record (best effort: a full disk must not stop a revocation).
fn audit(path: &Path, now_ms: u64, action: &str, device: &str, by: &str, outcome: &str) {
    use std::io::Write;
    let line = format!("{now_ms} {action} {device} by={by} {outcome}\n");
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    if let Ok(mut f) = options.open(path) {
        let _ = f.write_all(line.as_bytes());
    }
    eprintln!("devices: {action} {device} by={by} {outcome}");
}

fn field(out: &mut Vec<u8>, x: &[u8]) {
    out.extend_from_slice(x.len().to_string().as_bytes());
    out.push(b':');
    out.extend_from_slice(x);
}

/// The exact bytes an endorser signs (docs/specs/protocol.md "Devices"; vectors in `schema/fixtures/signing/device-trust-v1.json`).
pub fn endorse_message(e: &DeviceEndorsement) -> Vec<u8> {
    let mut m = b"agora-device-endorse-v1\0".to_vec();
    field(&mut m, e.device_id.as_bytes());
    field(&mut m, hex(&e.public_key).as_bytes());
    field(&mut m, e.name.as_bytes());
    field(&mut m, e.nodes.len().to_string().as_bytes());
    for n in &e.nodes {
        field(&mut m, n.as_bytes());
    }
    field(&mut m, e.issued_at_ms.to_string().as_bytes());
    field(&mut m, e.expires_at_ms.to_string().as_bytes());
    field(&mut m, e.endorser_device_id.as_bytes());
    m
}

/// The exact bytes a revoker signs.
pub fn revoke_message(r: &DeviceRevocation) -> Vec<u8> {
    let mut m = b"agora-device-revoke-v1\0".to_vec();
    field(&mut m, r.device_id.as_bytes());
    field(&mut m, r.issued_at_ms.to_string().as_bytes());
    field(&mut m, r.revoker_device_id.as_bytes());
    m
}

/// What people compare: the first 8 bytes of the SHA-256 of the key, lowercase hex in groups of 2 bytes joined by `-`.
pub fn fingerprint(public_key: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, public_key);
    digest
        .as_ref()
        .get(..8)
        .unwrap_or_default()
        .chunks(2)
        .map(hex)
        .collect::<Vec<_>>()
        .join("-")
}

/// A fingerprint as someone typed it: hex digits only, lowercased (`3F9A 07C2...`, `3f9a:07c2...` and `3f9a-07c2-...` are the same).
pub fn normalize_fingerprint(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_hexdigit)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn verifies(key: &[u8], message: &[u8], signature: &[u8]) -> bool {
    ring::signature::UnparsedPublicKey::new(&ring::signature::ECDSA_P256_SHA256_FIXED, key)
        .verify(message, signature)
        .is_ok()
}

/// What an endorsement did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endorsed {
    /// The device is trusted here from now on (until the returned expiry).
    Trusted(u64),
    /// It was trusted already; its expiry is now this (or it has none: confirmed on the machine).
    Already(Option<u64>),
}

/// The node's side of ADR-0061: verifies what the hub carries and keeps the trusted-devices file.
pub struct DeviceTrust {
    node_id: String,
    file: PathBuf,
    /// One change at a time from this process (the owner's CLI writes through the same atomic rename).
    lock: Mutex<()>,
}

impl DeviceTrust {
    /// `file` is the trusted-devices file; the revocations and the record live beside it.
    pub fn new(node_id: &str, file: &Path) -> Self {
        Self {
            node_id: node_id.to_string(),
            file: file.to_path_buf(),
            lock: Mutex::new(()),
        }
    }

    pub fn file(&self) -> &Path {
        &self.file
    }

    fn revocations(&self) -> PathBuf {
        self.file.with_file_name(REVOCATIONS_FILE)
    }

    fn record(&self, now_ms: u64, action: &str, device: &str, by: &str, outcome: &str) {
        audit(
            &self.file.with_file_name(AUDIT_FILE),
            now_ms,
            action,
            device,
            by,
            outcome,
        );
    }

    /// The devices trusted now (live entries), sorted: what `NodeTrustedDevices` reports.
    pub fn trusted_ids(&self, now_ms: u64) -> Vec<String> {
        let mut ids: Vec<String> = read_entries(&self.file)
            .0
            .into_iter()
            .filter(|e| e.live(now_ms))
            .map(|e| e.id)
            .collect();
        ids.sort();
        ids
    }

    /// A trusted device that verifies `message`, or the error that says why not.
    fn signer(
        &self,
        entries: &[Entry],
        signer: &str,
        subject: &str,
        message: &[u8],
        signature: &[u8],
        now_ms: u64,
    ) -> Result<Entry, Error> {
        let Some(e) = entries.iter().find(|e| e.id == signer && e.live(now_ms)) else {
            return Err(Error::new(ErrorCode::DeviceUntrusted)
                .with("device_id", signer)
                .with("node_id", self.node_id.as_str()));
        };
        if !verifies(&e.key, message, signature) {
            return Err(Error::new(ErrorCode::DeviceSignatureInvalid).with("device_id", subject));
        }
        Ok(e.clone())
    }

    /// Applies an endorsement the hub carried, after checking everything ADR-0061 asks (docs/specs/protocol.md "Devices").
    pub fn endorse(&self, e: &DeviceEndorsement, now_ms: u64) -> Result<Endorsed, Error> {
        let _g = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let result = self.endorse_locked(e, now_ms);
        let outcome = match &result {
            Ok(Endorsed::Trusted(_)) => "trusted".to_string(),
            Ok(Endorsed::Already(_)) => "already".to_string(),
            Err(err) => err.code.as_str().to_string(),
        };
        self.record(
            now_ms,
            "endorse",
            &e.device_id,
            &e.endorser_device_id,
            &outcome,
        );
        result
    }

    fn endorse_locked(&self, e: &DeviceEndorsement, now_ms: u64) -> Result<Endorsed, Error> {
        let invalid = |f: &str| Error::new(ErrorCode::ProtocolInvalidMessage).with("field", f);
        if !valid_device_id(&e.device_id) {
            return Err(invalid("device_id"));
        }
        if !valid_device_id(&e.endorser_device_id) || e.endorser_device_id == e.device_id {
            return Err(invalid("endorser_device_id"));
        }
        let (entries, _) = read_entries(&self.file);
        let endorser = self.signer(
            &entries,
            &e.endorser_device_id,
            &e.device_id,
            &endorse_message(e),
            &e.signature,
            now_ms,
        )?;
        if !valid_key(&e.public_key) {
            return Err(invalid("public_key"));
        }
        if !e.nodes.is_empty() && !e.nodes.contains(&self.node_id) {
            return Err(Error::new(ErrorCode::DeviceNotInScope)
                .with("device_id", e.device_id.as_str())
                .with("node_id", self.node_id.as_str()));
        }
        if now_ms >= e.expires_at_ms
            || e.expires_at_ms <= e.issued_at_ms
            || e.issued_at_ms > now_ms.saturating_add(SKEW_MS)
            || e.expires_at_ms - e.issued_at_ms > MAX_ENDORSEMENT_MS
        {
            return Err(Error::new(ErrorCode::DeviceEndorsementExpired)
                .with("device_id", e.device_id.as_str()));
        }
        if read_revocations(&self.revocations())
            .iter()
            .any(|(d, at)| *d == e.device_id && *at >= e.issued_at_ms)
        {
            return Err(
                Error::new(ErrorCode::DeviceRevoked).with("device_id", e.device_id.as_str())
            );
        }
        // Never longer than the endorser's own trust here.
        let expires = endorser
            .expires_ms
            .map_or(e.expires_at_ms, |x| x.min(e.expires_at_ms));
        let io = |_| Error::new(ErrorCode::Internal).with("incident_id", "trusted-devices");
        match entries.iter().find(|x| x.id == e.device_id) {
            Some(x) if x.key != e.public_key => {
                Err(Error::new(ErrorCode::DeviceKeyMismatch)
                    .with("device_id", e.device_id.as_str()))
            }
            Some(x) if x.expires_ms.is_none() => Ok(Endorsed::Already(None)),
            Some(x) if x.expires_ms.is_some_and(|old| old >= expires) => {
                Ok(Endorsed::Already(x.expires_ms))
            }
            existing => {
                let entry = Entry {
                    id: e.device_id.clone(),
                    key: e.public_key.clone(),
                    expires_ms: Some(expires),
                    via: Some(e.endorser_device_id.clone()),
                };
                rewrite(&self.file, &e.device_id, Some(&entry)).map_err(io)?;
                Ok(match existing {
                    Some(_) => Endorsed::Already(Some(expires)),
                    None => Endorsed::Trusted(expires),
                })
            }
        }
    }

    /// Applies a revocation the hub carried. `Ok(true)`: the device was trusted here and is not any more.
    pub fn revoke(&self, r: &DeviceRevocation, now_ms: u64) -> Result<bool, Error> {
        let _g = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let result = self.revoke_locked(r, now_ms);
        let outcome = match &result {
            Ok(true) => "revoked".to_string(),
            Ok(false) => "remembered".to_string(),
            Err(err) => err.code.as_str().to_string(),
        };
        self.record(
            now_ms,
            "revoke",
            &r.device_id,
            &r.revoker_device_id,
            &outcome,
        );
        result
    }

    fn revoke_locked(&self, r: &DeviceRevocation, now_ms: u64) -> Result<bool, Error> {
        if !valid_device_id(&r.device_id) {
            return Err(Error::new(ErrorCode::ProtocolInvalidMessage).with("field", "device_id"));
        }
        let (entries, _) = read_entries(&self.file);
        self.signer(
            &entries,
            &r.revoker_device_id,
            &r.device_id,
            &revoke_message(r),
            &r.signature,
            now_ms,
        )?;
        let io = |_| Error::new(ErrorCode::Internal).with("incident_id", "trusted-devices");
        remember_revocation(&self.revocations(), &r.device_id, r.issued_at_ms).map_err(io)?;
        let was = entries.iter().any(|e| e.id == r.device_id);
        if was {
            rewrite(&self.file, &r.device_id, None).map_err(io)?;
        }
        Ok(was)
    }

    /// A device endorsed here was used (its signature verified): its trust runs 90 days from now. Written at most once a day per
    /// device; a device confirmed on the machine has no end and is left alone.
    pub fn renew(&self, device_id: &str, now_ms: u64) {
        let _g = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let (entries, _) = read_entries(&self.file);
        let Some(e) = entries.iter().find(|e| e.id == device_id && e.live(now_ms)) else {
            return;
        };
        let Some(old) = e.expires_ms else { return };
        let new = now_ms.saturating_add(MAX_ENDORSEMENT_MS);
        if new > old.saturating_add(RENEW_STEP_MS) {
            let mut renewed = e.clone();
            renewed.expires_ms = Some(new);
            let _ = rewrite(&self.file, device_id, Some(&renewed));
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use ring::rand::SystemRandom;
    use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};

    const VECTORS: &str = include_str!("../../../schema/fixtures/signing/device-trust-v1.json");
    const DAY: u64 = 24 * 60 * 60 * 1000;

    fn vectors() -> serde_json::Value {
        serde_json::from_str(VECTORS).unwrap()
    }

    fn endorsement_of(c: &serde_json::Value) -> DeviceEndorsement {
        DeviceEndorsement {
            device_id: c["device_id"].as_str().unwrap().into(),
            public_key: from_hex(c["public_key"].as_str().unwrap()).unwrap(),
            name: c["name"].as_str().unwrap().into(),
            nodes: c["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n.as_str().unwrap().to_string())
                .collect(),
            issued_at_ms: c["issued_at_ms"].as_u64().unwrap(),
            expires_at_ms: c["expires_at_ms"].as_u64().unwrap(),
            endorser_device_id: c["endorser_device_id"].as_str().unwrap().into(),
            signature: from_hex(c["signature"].as_str().unwrap()).unwrap(),
        }
    }

    #[test]
    fn the_signed_bytes_and_fingerprints_match_the_shared_vectors() {
        let v = vectors();
        let endorser = from_hex(v["endorser_public_key"].as_str().unwrap()).unwrap();
        for c in v["endorsements"].as_array().unwrap() {
            let e = endorsement_of(c);
            let m = endorse_message(&e);
            assert_eq!(hex(&m), c["hex"].as_str().unwrap(), "{}", c["case"]);
            assert!(verifies(&endorser, &m, &e.signature), "{}", c["case"]);
        }
        for c in v["revocations"].as_array().unwrap() {
            let r = DeviceRevocation {
                device_id: c["device_id"].as_str().unwrap().into(),
                issued_at_ms: c["issued_at_ms"].as_u64().unwrap(),
                revoker_device_id: c["revoker_device_id"].as_str().unwrap().into(),
                signature: from_hex(c["signature"].as_str().unwrap()).unwrap(),
            };
            let m = revoke_message(&r);
            assert_eq!(hex(&m), c["hex"].as_str().unwrap(), "{}", c["case"]);
            assert!(verifies(&endorser, &m, &r.signature));
        }
        for f in v["fingerprints"].as_array().unwrap() {
            let k = from_hex(f["public_key"].as_str().unwrap()).unwrap();
            assert_eq!(fingerprint(&k), f["fingerprint"].as_str().unwrap());
        }
        assert_eq!(
            normalize_fingerprint("3F9A 07c2:11BE-d4e0"),
            "3f9a07c211bed4e0"
        );
    }

    struct Key {
        pair: EcdsaKeyPair,
    }

    impl Key {
        fn new() -> Self {
            let rng = SystemRandom::new();
            let pkcs8 =
                EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
            Key {
                pair: EcdsaKeyPair::from_pkcs8(
                    &ECDSA_P256_SHA256_FIXED_SIGNING,
                    pkcs8.as_ref(),
                    &rng,
                )
                .unwrap(),
            }
        }
        fn public(&self) -> Vec<u8> {
            self.pair.public_key().as_ref().to_vec()
        }
        fn sign(&self, m: &[u8]) -> Vec<u8> {
            self.pair
                .sign(&SystemRandom::new(), m)
                .unwrap()
                .as_ref()
                .to_vec()
        }
        fn endorse(
            &self,
            by: &str,
            device: &str,
            key: &[u8],
            nodes: &[&str],
            issued: u64,
            ttl: u64,
        ) -> DeviceEndorsement {
            let mut e = DeviceEndorsement {
                device_id: device.into(),
                public_key: key.to_vec(),
                name: "New browser".into(),
                nodes: nodes.iter().map(|n| (*n).to_string()).collect(),
                issued_at_ms: issued,
                expires_at_ms: issued + ttl,
                endorser_device_id: by.into(),
                signature: Vec::new(),
            };
            e.signature = self.sign(&endorse_message(&e));
            e
        }
        fn revoke(&self, by: &str, device: &str, issued: u64) -> DeviceRevocation {
            let mut r = DeviceRevocation {
                device_id: device.into(),
                issued_at_ms: issued,
                revoker_device_id: by.into(),
                signature: Vec::new(),
            };
            r.signature = self.sign(&revoke_message(&r));
            r
        }
    }

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("agora-dt-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn an_endorsement_counts_only_from_a_trusted_device_for_this_machine_in_time_and_not_revoked() {
        let d = dir("endorse");
        let file = d.join(crate::approvals::TRUSTED_DEVICES_FILE);
        let (old, new, stranger) = (Key::new(), Key::new(), Key::new());
        confirm(&file, "web-old", &old.public()).unwrap();
        let t = DeviceTrust::new("laptop", &file);
        let now = 1_700_000_000_000;
        let code = |r: Result<Endorsed, Error>| r.unwrap_err().code;

        // The hub lists a device and makes up an endorsement for it: signed by no device this machine trusts.
        let forged = stranger.endorse("web-old", "web-new", &new.public(), &[], now, DAY);
        assert_eq!(
            code(t.endorse(&forged, now)),
            ErrorCode::DeviceSignatureInvalid
        );
        let unknown = stranger.endorse("web-x", "web-new", &new.public(), &[], now, DAY);
        assert_eq!(code(t.endorse(&unknown, now)), ErrorCode::DeviceUntrusted);
        // A real endorsement altered on the way (another key swapped in, a machine dropped).
        let mut swapped = old.endorse("web-old", "web-new", &new.public(), &[], now, DAY);
        swapped.public_key = stranger.public();
        assert_eq!(
            code(t.endorse(&swapped, now)),
            ErrorCode::DeviceSignatureInvalid
        );
        let mut widened = old.endorse("web-old", "web-new", &new.public(), &["vps"], now, DAY);
        widened.nodes.clear();
        assert_eq!(
            code(t.endorse(&widened, now)),
            ErrorCode::DeviceSignatureInvalid
        );
        // Signed, but for other machines, expired, too long, or from the future.
        let other = old.endorse("web-old", "web-new", &new.public(), &["vps"], now, DAY);
        assert_eq!(code(t.endorse(&other, now)), ErrorCode::DeviceNotInScope);
        let expired = old.endorse("web-old", "web-new", &new.public(), &[], now - 2 * DAY, DAY);
        assert_eq!(
            code(t.endorse(&expired, now)),
            ErrorCode::DeviceEndorsementExpired
        );
        let long = old.endorse(
            "web-old",
            "web-new",
            &new.public(),
            &[],
            now,
            MAX_ENDORSEMENT_MS + 1,
        );
        assert_eq!(
            code(t.endorse(&long, now)),
            ErrorCode::DeviceEndorsementExpired
        );
        let ahead = old.endorse("web-old", "web-new", &new.public(), &[], now + DAY, DAY);
        assert_eq!(
            code(t.endorse(&ahead, now)),
            ErrorCode::DeviceEndorsementExpired
        );
        assert_eq!(t.trusted_ids(now), ["web-old"], "nothing above added trust");

        // The real one, for this machine among others.
        let good = old.endorse(
            "web-old",
            "web-new",
            &new.public(),
            &["vps", "laptop"],
            now,
            30 * DAY,
        );
        assert_eq!(
            t.endorse(&good, now).unwrap(),
            Endorsed::Trusted(now + 30 * DAY)
        );
        assert_eq!(
            t.endorse(&good, now + 1).unwrap(),
            Endorsed::Already(Some(now + 30 * DAY))
        );
        assert_eq!(t.trusted_ids(now), ["web-new", "web-old"]);
        // It lasts until its expiry, then the machine stops trusting it.
        assert_eq!(t.trusted_ids(now + 30 * DAY), ["web-old"]);
        // A different key under the same id is refused.
        let clash = old.endorse("web-old", "web-new", &stranger.public(), &[], now, DAY);
        assert_eq!(code(t.endorse(&clash, now)), ErrorCode::DeviceKeyMismatch);

        let log = std::fs::read_to_string(d.join(AUDIT_FILE)).unwrap();
        assert!(log.contains("endorse web-new by=web-old trusted"));
        assert!(log.contains("device.signature_invalid"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn an_endorsed_device_lasts_no_longer_than_its_endorser_and_use_renews_it() {
        let d = dir("chain");
        let file = d.join(crate::approvals::TRUSTED_DEVICES_FILE);
        let (a, b, c) = (Key::new(), Key::new(), Key::new());
        confirm(&file, "a", &a.public()).unwrap();
        let t = DeviceTrust::new("laptop", &file);
        let now = 1_700_000_000_000;
        t.endorse(&a.endorse("a", "b", &b.public(), &[], now, 10 * DAY), now)
            .unwrap();
        // b (trusted for 10 days) endorses c for 60: c ends with b.
        assert_eq!(
            t.endorse(&b.endorse("b", "c", &c.public(), &[], now, 60 * DAY), now)
                .unwrap(),
            Endorsed::Trusted(now + 10 * DAY)
        );
        // Using b renews it to 90 days from the use.
        t.renew("b", now + 5 * DAY);
        let b_entry = read_entries(&file)
            .0
            .into_iter()
            .find(|e| e.id == "b")
            .unwrap();
        assert_eq!(b_entry.expires_ms, Some(now + 5 * DAY + MAX_ENDORSEMENT_MS));
        assert_eq!(b_entry.via.as_deref(), Some("a"));
        // A device confirmed on the machine has no end to renew.
        t.renew("a", now);
        assert_eq!(read_entries(&file).0[0].expires_ms, None);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_revocation_narrows_at_once_and_an_older_endorsement_cannot_bring_the_device_back() {
        let d = dir("revoke");
        let file = d.join(crate::approvals::TRUSTED_DEVICES_FILE);
        let (old, new, stranger) = (Key::new(), Key::new(), Key::new());
        confirm(&file, "web-old", &old.public()).unwrap();
        let t = DeviceTrust::new("laptop", &file);
        let now = 1_700_000_000_000;
        let e = old.endorse("web-old", "web-new", &new.public(), &[], now, DAY);
        t.endorse(&e, now).unwrap();
        // A revocation the hub made up does nothing.
        assert_eq!(
            t.revoke(&stranger.revoke("web-old", "web-new", now + 1), now + 1)
                .unwrap_err()
                .code,
            ErrorCode::DeviceSignatureInvalid
        );
        assert!(t.trusted_ids(now + 1).contains(&"web-new".to_string()));
        assert!(t
            .revoke(&old.revoke("web-old", "web-new", now + 2), now + 2)
            .unwrap());
        assert_eq!(t.trusted_ids(now + 2), ["web-old"]);
        // The hub sends the old endorsement again: refused.
        assert_eq!(
            t.endorse(&e, now + 3).unwrap_err().code,
            ErrorCode::DeviceRevoked
        );
        // A new endorsement signed after the revocation is a new decision.
        let again = old.endorse("web-old", "web-new", &new.public(), &[], now + 4, DAY);
        assert!(matches!(
            t.endorse(&again, now + 4),
            Ok(Endorsed::Trusted(_))
        ));
        // The owner untrusts it on the machine: same effect, without the hub.
        assert!(untrust(&file, "web-new", now + 5).unwrap());
        assert_eq!(
            t.endorse(&again, now + 6).unwrap_err().code,
            ErrorCode::DeviceRevoked
        );
        // A revoked revoker can sign nothing more.
        assert!(t
            .revoke(&old.revoke("web-old", "web-old", now + 7), now + 7)
            .unwrap());
        assert_eq!(t.trusted_ids(now + 7), Vec::<String>::new());
        assert_eq!(
            t.revoke(&old.revoke("web-old", "web-new", now + 8), now + 8)
                .unwrap_err()
                .code,
            ErrorCode::DeviceUntrusted
        );
        // Trusting it again on the machine clears what was remembered.
        confirm(&file, "web-old", &old.public()).unwrap();
        assert_eq!(t.trusted_ids(now + 9), ["web-old"]);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_file_keeps_its_format_and_unknown_fields_are_never_trusted() {
        let d = dir("format");
        let file = d.join(crate::approvals::TRUSTED_DEVICES_FILE);
        let k = Key::new();
        let h = hex(&k.public());
        std::fs::write(
            &file,
            format!(
                "# mine\nphone {h}\nweb-1 {h} expires=5 via=phone\nweb-2 {h} expire=5\nweb-3 {h} expires=x\nweb-4 {h} expires=1 expires=2\n"
            ),
        )
        .unwrap();
        let (entries, skipped) = read_entries(&file);
        assert_eq!(skipped, 3, "a typo in a field never widens trust");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].expires_ms, Some(5));
        // Changing one device keeps every other line (and the comment) as it was, owner-only.
        confirm(&file, "web-9", &k.public()).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.starts_with("# mine\nphone "));
        assert!(text.ends_with(&format!("web-9 {h}\n")));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(d);
    }
}
