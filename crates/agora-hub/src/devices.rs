//! Registered devices (ADR-0061): the browsers and terminal clients each person signs with, and the endorsements and revocations the
//! hub carries to machines.
//!
//! The hub's list is only a list of candidates. A machine never trusts a device because it is listed here: its owner says yes on the
//! machine, or a device the machine already trusts signs an endorsement, which the machine verifies itself. So nothing in this module
//! can widen what a machine trusts; at worst a lying hub offers a candidate the owner then refuses after comparing fingerprints.

use agora_protocol::v1::{
    DeviceAskApproval, DeviceEndorsement, DeviceInfo, DeviceRegister, DeviceRevocation,
};
use agora_protocol::{Error, ErrorCode};
use prost::Message;
use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::store::{SqliteStore, StoreError};

/// The longest an endorsement may last from when it was signed (ADR-0061: 90 days, renewed on the machine by use).
pub const MAX_ENDORSEMENT_MS: u64 = 90 * 24 * 60 * 60 * 1000;
/// Most machines one endorsement may name.
pub const MAX_SCOPE: usize = 64;
/// Most devices one person may have registered and not revoked (a browser profile made per visit must not fill the hub).
pub const MAX_DEVICES_PER_USER: usize = 64;
/// Most revocations kept per person.
pub const MAX_REVOCATIONS_PER_USER: usize = 256;
/// A revocation is kept this long, then forgotten: every endorsement signed before it has long expired (at most 90 days), so
/// forgetting it cannot bring a device back.
pub const REVOCATION_KEPT_MS: u64 = 400 * 24 * 60 * 60 * 1000;
/// How far a registration's `signed_at_ms` may be from the hub's clock, and an ask's.
const REGISTER_SKEW_MS: u64 = 10 * 60 * 1000;
const ASK_SKEW_MS: u64 = 5 * 60 * 1000;

/// What people compare: the first 12 bytes (96 bits) of the SHA-256 of the 65-byte public key, lowercase hex in groups of 4 hex
/// digits joined by `-` (`f947-3cb5-e304-81be-ecf2-64cd`). Vectors: `schema/fixtures/signing/device-trust-v1.json`.
pub fn fingerprint(public_key: &[u8]) -> String {
    let digest = Sha256::digest(public_key);
    let hex: Vec<String> = digest
        .get(..12)
        .unwrap_or_default()
        .chunks(2)
        .map(|pair| pair.iter().map(|b| format!("{b:02x}")).collect())
        .collect();
    hex.join("-")
}

/// A device id: what a node's trusted-devices file can hold (`[A-Za-z0-9._-]{1,64}`).
pub fn valid_device_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// A name a person reads: at most 64 characters, no control, format, bidi or other hidden characters (agora-core's list, ADR-0057).
pub fn valid_name(name: &str) -> bool {
    name.chars().count() <= 64
        && !name
            .chars()
            .any(|c| c.is_control() || agora_core::text::is_hidden(c))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn field(out: &mut Vec<u8>, x: &[u8]) {
    out.extend_from_slice(x.len().to_string().as_bytes());
    out.push(b':');
    out.extend_from_slice(x);
}

/// The bytes a device signs to register itself (proof of possession; docs/specs/protocol.md "Devices").
pub fn register_message(
    device_id: &str,
    name: &str,
    public_key: &[u8],
    signed_at_ms: u64,
) -> Vec<u8> {
    let mut m = b"agora-device-register-v1\0".to_vec();
    field(&mut m, device_id.as_bytes());
    field(&mut m, name.as_bytes());
    field(&mut m, hex(public_key).as_bytes());
    field(&mut m, signed_at_ms.to_string().as_bytes());
    m
}

/// The bytes a device signs to ask another device of its person to approve it.
pub fn ask_message(device_id: &str, signed_at_ms: u64) -> Vec<u8> {
    let mut m = b"agora-device-ask-v1\0".to_vec();
    field(&mut m, device_id.as_bytes());
    field(&mut m, signed_at_ms.to_string().as_bytes());
    m
}

/// The bytes an endorser signs (the node's `device_trust::endorse_message`).
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

/// The bytes a revoker signs.
pub fn revoke_message(r: &DeviceRevocation) -> Vec<u8> {
    let mut m = b"agora-device-revoke-v1\0".to_vec();
    field(&mut m, r.device_id.as_bytes());
    field(&mut m, r.issued_at_ms.to_string().as_bytes());
    field(&mut m, r.revoker_device_id.as_bytes());
    m
}

/// ES256 (r || s) by `key` over `message`.
pub fn verifies(key: &[u8], message: &[u8], signature: &[u8]) -> bool {
    ring::signature::UnparsedPublicKey::new(&ring::signature::ECDSA_P256_SHA256_FIXED, key)
        .verify(message, signature)
        .is_ok()
}

fn valid_key(key: &[u8]) -> bool {
    key.len() == 65 && key.first() == Some(&4)
}

fn invalid(field: &str) -> Error {
    Error::new(ErrorCode::ProtocolInvalidMessage).with("field", field)
}

fn store_error(e: StoreError) -> Error {
    eprintln!("devices: {e}");
    Error::new(ErrorCode::Internal).with("incident_id", "devices")
}

fn to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn to_u64(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

pub struct Devices<'a> {
    store: &'a SqliteStore,
}

impl<'a> Devices<'a> {
    pub fn new(store: &'a SqliteStore) -> Self {
        Self { store }
    }

    /// Registers (or touches) `r` under `user`. A device id is bound to its key and its person for good.
    pub fn register(&self, user: &str, r: &DeviceRegister, now_ms: u64) -> Result<(), Error> {
        if !valid_device_id(&r.device_id) {
            return Err(invalid("device_id"));
        }
        if !valid_name(&r.name) {
            return Err(invalid("name"));
        }
        if !valid_key(&r.public_key) {
            return Err(invalid("public_key"));
        }
        // Proof of possession, checked before anything is stored: only the holder of the key can register it.
        if r.signed_at_ms.abs_diff(now_ms) > REGISTER_SKEW_MS
            || !verifies(
                &r.public_key,
                &register_message(&r.device_id, &r.name, &r.public_key, r.signed_at_ms),
                &r.signature,
            )
        {
            return Err(Error::new(ErrorCode::DeviceSignatureInvalid)
                .with("device_id", r.device_id.as_str()));
        }
        let conn = self.store.lock().map_err(store_error)?;
        let existing: Option<(String, Vec<u8>)> = conn
            .query_row(
                "SELECT user_id, public_key FROM user_devices WHERE device_id = ?1",
                [&r.device_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| store_error(e.into()))?;
        match existing {
            Some((owner, key)) if owner != user || key != r.public_key => {
                Err(Error::new(ErrorCode::DeviceKeyMismatch)
                    .with("device_id", r.device_id.as_str()))
            }
            Some(_) => {
                conn.execute(
                    "UPDATE user_devices SET name = ?2, last_used_ms = ?3, signed_at_ms = ?4, signature = ?5 WHERE device_id = ?1",
                    params![r.device_id, r.name, to_i64(now_ms), to_i64(r.signed_at_ms), r.signature],
                )
                .map_err(|e| store_error(e.into()))?;
                Ok(())
            }
            None => {
                let count: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM user_devices WHERE user_id = ?1 AND revoked_ms = 0",
                        [user],
                        |row| row.get(0),
                    )
                    .map_err(|e| store_error(e.into()))?;
                if usize::try_from(count).unwrap_or(usize::MAX) >= MAX_DEVICES_PER_USER {
                    return Err(Error::new(ErrorCode::NodeResourceLimit)
                        .with("resource", "devices")
                        .with("limit", MAX_DEVICES_PER_USER as i64));
                }
                conn.execute(
                    "INSERT INTO user_devices (device_id, user_id, name, public_key, created_ms, last_used_ms, signed_at_ms, signature)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, ?7)",
                    params![r.device_id, user, r.name, r.public_key, to_i64(now_ms), to_i64(r.signed_at_ms), r.signature],
                )
                .map_err(|e| store_error(e.into()))?;
                Ok(())
            }
        }
    }

    fn rows(&self, user: Option<&str>) -> Result<Vec<DeviceInfo>, Error> {
        let conn = self.store.lock().map_err(store_error)?;
        let mut stmt = conn
            .prepare(
                "SELECT device_id, name, public_key, created_ms, last_used_ms, user_id, approval_asked_ms, revoked_ms, signed_at_ms, signature
                 FROM user_devices WHERE ?1 IS NULL OR user_id = ?1 ORDER BY created_ms, rowid",
            )
            .map_err(|e| store_error(e.into()))?;
        let mut out: Vec<DeviceInfo> = stmt
            .query_map([user], |row| {
                Ok(DeviceInfo {
                    device_id: row.get(0)?,
                    name: row.get(1)?,
                    public_key: row.get(2)?,
                    created_ms: to_u64(row.get(3)?),
                    last_used_ms: to_u64(row.get(4)?),
                    user_id: row.get(5)?,
                    approval_asked_ms: to_u64(row.get(6)?),
                    endorsed_by: Vec::new(),
                    revoked_ms: to_u64(row.get(7)?),
                    signed_at_ms: to_u64(row.get(8)?),
                    registration_signature: row.get(9)?,
                })
            })
            .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
            .map_err(|e| store_error(e.into()))?;
        let mut endorsers = conn
            .prepare(
                "SELECT endorser FROM device_endorsements WHERE device_id = ?1 ORDER BY id DESC",
            )
            .map_err(|e| store_error(e.into()))?;
        for d in &mut out {
            let by: Vec<String> = endorsers
                .query_map([&d.device_id], |row| row.get(0))
                .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
                .map_err(|e| store_error(e.into()))?;
            for e in by {
                if !d.endorsed_by.contains(&e) {
                    d.endorsed_by.push(e);
                }
            }
        }
        Ok(out)
    }

    /// The devices of one person.
    pub fn list_for(&self, user: &str) -> Result<Vec<DeviceInfo>, Error> {
        self.rows(Some(user))
    }

    /// The devices a machine is offered (enrollment, `agora node trust --from-hub`): those of the people who may act on `node`, the
    /// hub's owner (sessions without a person, and people with the owner role) and every person with a grant on a project of that node
    /// (ADR-0049). Other people's devices are never sent to it.
    pub fn list_for_node(&self, node: &str) -> Result<Vec<DeviceInfo>, Error> {
        let users = crate::users::Users::new(self.store);
        let people = users.list().map_err(store_error)?;
        let prefix = format!("{node}/");
        let mut relevant: Vec<String> = vec!["owner".to_string()];
        for u in people {
            let works_here = u.role == crate::users::Role::Owner
                || users
                    .grants_of(&u.id)
                    .map_err(store_error)?
                    .iter()
                    .any(|g| g.starts_with(&prefix));
            if works_here {
                relevant.push(u.id);
            }
        }
        Ok(self
            .rows(None)?
            .into_iter()
            .filter(|d| d.user_id.is_empty() || relevant.contains(&d.user_id))
            .collect())
    }

    /// Every registered device: `agora hub devices`.
    pub fn list_all(&self) -> Result<Vec<DeviceInfo>, Error> {
        self.rows(None)
    }

    fn device(&self, id: &str) -> Result<Option<(String, Vec<u8>)>, Error> {
        let conn = self.store.lock().map_err(store_error)?;
        conn.query_row(
            "SELECT user_id, public_key FROM user_devices WHERE device_id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| store_error(e.into()))
    }

    /// A registered device of `user`, or `device.unknown` (another person's device looks the same as none).
    fn own(&self, user: &str, id: &str) -> Result<Vec<u8>, Error> {
        match self.device(id)? {
            Some((owner, key)) if owner == user => Ok(key),
            _ => Err(Error::new(ErrorCode::DeviceUnknown).with("device_id", id)),
        }
    }

    /// This device of `user` asks another device of theirs to approve it; the ask is signed by the device itself, just now.
    pub fn ask_approval(
        &self,
        user: &str,
        a: &DeviceAskApproval,
        now_ms: u64,
    ) -> Result<(), Error> {
        let id = a.device_id.as_str();
        let key = self.own(user, id)?;
        if a.signed_at_ms.abs_diff(now_ms) > ASK_SKEW_MS
            || !verifies(&key, &ask_message(id, a.signed_at_ms), &a.signature)
        {
            return Err(Error::new(ErrorCode::DeviceSignatureInvalid).with("device_id", id));
        }
        let conn = self.store.lock().map_err(store_error)?;
        conn.execute(
            "UPDATE user_devices SET approval_asked_ms = ?2 WHERE device_id = ?1",
            params![id, to_i64(now_ms)],
        )
        .map_err(|e| store_error(e.into()))?;
        Ok(())
    }

    /// Checks what the hub can check of an endorsement `user` sends (both devices theirs, the key the one registered, a sane shape)
    /// and keeps it for the machines. The signature is the machine's to verify: the hub cannot, and need not, vouch for it.
    pub fn record_endorsement(
        &self,
        user: &str,
        e: &DeviceEndorsement,
        now_ms: u64,
    ) -> Result<(), Error> {
        if !valid_device_id(&e.device_id) {
            return Err(invalid("device_id"));
        }
        if !valid_device_id(&e.endorser_device_id) || e.endorser_device_id == e.device_id {
            return Err(invalid("endorser_device_id"));
        }
        if !valid_name(&e.name) {
            return Err(invalid("name"));
        }
        if e.signature.len() != 64 {
            return Err(invalid("signature"));
        }
        if e.nodes.len() > MAX_SCOPE || e.nodes.iter().any(|n| n.is_empty() || n.len() > 64) {
            return Err(invalid("nodes"));
        }
        if e.issued_at_ms > now_ms.saturating_add(ASK_SKEW_MS)
            || e.expires_at_ms <= now_ms
            || e.expires_at_ms <= e.issued_at_ms
            || e.expires_at_ms - e.issued_at_ms > MAX_ENDORSEMENT_MS
        {
            return Err(Error::new(ErrorCode::DeviceEndorsementExpired)
                .with("device_id", e.device_id.as_str()));
        }
        let key = self.own(user, &e.device_id)?;
        if key != e.public_key {
            return Err(
                Error::new(ErrorCode::DeviceKeyMismatch).with("device_id", e.device_id.as_str())
            );
        }
        let endorser = self.own(user, &e.endorser_device_id)?;
        // Verified before it is stored or carried: the hub keeps only what the endorser's registered key signed.
        if !verifies(&endorser, &endorse_message(e), &e.signature) {
            return Err(Error::new(ErrorCode::DeviceSignatureInvalid)
                .with("device_id", e.device_id.as_str()));
        }
        let conn = self.store.lock().map_err(store_error)?;
        // The same signed endorsement again is kept once.
        conn.execute(
            "INSERT OR IGNORE INTO device_endorsements (device_id, endorser, issued_ms, expires_ms, signature, payload) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                e.device_id,
                e.endorser_device_id,
                to_i64(e.issued_at_ms),
                to_i64(e.expires_at_ms),
                e.signature,
                e.encode_to_vec()
            ],
        )
        .map_err(|e| store_error(e.into()))?;
        conn.execute(
            "UPDATE user_devices SET approval_asked_ms = 0 WHERE device_id = ?1",
            [&e.device_id],
        )
        .map_err(|e| store_error(e.into()))?;
        Ok(())
    }

    /// Keeps a revocation `user` sends (of one of their devices, signed by one of their devices) for the machines.
    pub fn record_revocation(
        &self,
        user: &str,
        r: &DeviceRevocation,
        now_ms: u64,
    ) -> Result<(), Error> {
        if !valid_device_id(&r.device_id) {
            return Err(invalid("device_id"));
        }
        if !valid_device_id(&r.revoker_device_id) {
            return Err(invalid("revoker_device_id"));
        }
        if r.signature.len() != 64 {
            return Err(invalid("signature"));
        }
        // Never dated ahead of this clock (beyond the usual skew): a record from the future would outlive every bound.
        if r.issued_at_ms > now_ms.saturating_add(ASK_SKEW_MS) {
            return Err(invalid("issued_at_ms"));
        }
        self.own(user, &r.device_id)?;
        let revoker = self.own(user, &r.revoker_device_id)?;
        if !verifies(&revoker, &revoke_message(r), &r.signature) {
            return Err(Error::new(ErrorCode::DeviceSignatureInvalid)
                .with("device_id", r.device_id.as_str()));
        }
        let conn = self.store.lock().map_err(store_error)?;
        conn.execute(
            "DELETE FROM device_revocations WHERE issued_ms < ?1",
            [to_i64(now_ms.saturating_sub(REVOCATION_KEPT_MS))],
        )
        .map_err(|e| store_error(e.into()))?;
        let kept: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM device_revocations r JOIN user_devices d ON d.device_id = r.device_id WHERE d.user_id = ?1",
                [user],
                |row| row.get(0),
            )
            .map_err(|e| store_error(e.into()))?;
        if usize::try_from(kept).unwrap_or(usize::MAX) >= MAX_REVOCATIONS_PER_USER {
            return Err(Error::new(ErrorCode::NodeResourceLimit)
                .with("resource", "revocations")
                .with("limit", MAX_REVOCATIONS_PER_USER as i64));
        }
        conn.execute(
            "INSERT OR IGNORE INTO device_revocations (device_id, revoker, issued_ms, signature, payload) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                r.device_id,
                r.revoker_device_id,
                to_i64(r.issued_at_ms),
                r.signature,
                r.encode_to_vec()
            ],
        )
        .map_err(|e| store_error(e.into()))?;
        conn.execute(
            "UPDATE user_devices SET revoked_ms = CASE revoked_ms WHEN 0 THEN ?2 ELSE revoked_ms END, approval_asked_ms = 0 WHERE device_id = ?1",
            params![r.device_id, to_i64(now_ms)],
        )
        .map_err(|e| store_error(e.into()))?;
        Ok(())
    }

    /// Deletes a revoked device from the hub's list (`agora hub devices forget ID`). Its revocations stay, so machines keep refusing
    /// it. `Ok(false)`: no such device, or it is not revoked.
    pub fn forget(&self, id: &str) -> Result<bool, Error> {
        let conn = self.store.lock().map_err(store_error)?;
        let n = conn
            .execute(
                "DELETE FROM user_devices WHERE device_id = ?1 AND revoked_ms > 0",
                [id],
            )
            .map_err(|e| store_error(e.into()))?;
        conn.execute("DELETE FROM device_endorsements WHERE device_id = ?1", [id])
            .map_err(|e| store_error(e.into()))?;
        Ok(n > 0)
    }

    /// What a machine that connects is given again: every revocation, and every endorsement that has not expired, oldest first. The
    /// machine ignores what it already applied.
    pub fn for_machines(
        &self,
        now_ms: u64,
    ) -> Result<(Vec<DeviceEndorsement>, Vec<DeviceRevocation>), Error> {
        let conn = self.store.lock().map_err(store_error)?;
        let endorsements = conn
            .prepare("SELECT payload FROM device_endorsements WHERE expires_ms > ?1 ORDER BY id")
            .and_then(|mut s| {
                s.query_map([to_i64(now_ms)], |row| row.get::<_, Vec<u8>>(0))
                    .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
            })
            .map_err(|e| store_error(e.into()))?;
        let revocations = conn
            .prepare("SELECT payload FROM device_revocations WHERE issued_ms >= ?1 ORDER BY id")
            .and_then(|mut s| {
                s.query_map([to_i64(now_ms.saturating_sub(REVOCATION_KEPT_MS))], |row| {
                    row.get::<_, Vec<u8>>(0)
                })
                .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
            })
            .map_err(|e| store_error(e.into()))?;
        Ok((
            endorsements
                .iter()
                .filter_map(|b| DeviceEndorsement::decode(b.as_slice()).ok())
                .collect(),
            revocations
                .iter()
                .filter_map(|b| DeviceRevocation::decode(b.as_slice()).ok())
                .collect(),
        ))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub(crate) mod tests {
    use super::*;
    use ring::rand::SystemRandom;
    use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};

    /// A device key for tests, and what it signs.
    pub struct Key(EcdsaKeyPair);

    impl Key {
        pub fn new() -> Self {
            let rng = SystemRandom::new();
            let pkcs8 =
                EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
            Key(
                EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng)
                    .unwrap(),
            )
        }
        pub fn public(&self) -> Vec<u8> {
            self.0.public_key().as_ref().to_vec()
        }
        pub fn sign(&self, m: &[u8]) -> Vec<u8> {
            self.0
                .sign(&SystemRandom::new(), m)
                .unwrap()
                .as_ref()
                .to_vec()
        }
        pub fn register(&self, id: &str, name: &str, at: u64) -> DeviceRegister {
            DeviceRegister {
                device_id: id.into(),
                name: name.into(),
                public_key: self.public(),
                signed_at_ms: at,
                signature: self.sign(&register_message(id, name, &self.public(), at)),
            }
        }
        pub fn ask(&self, id: &str, at: u64) -> DeviceAskApproval {
            DeviceAskApproval {
                device_id: id.into(),
                signed_at_ms: at,
                signature: self.sign(&ask_message(id, at)),
            }
        }
        pub fn endorse(
            &self,
            by: &str,
            device: &str,
            key: &[u8],
            issued: u64,
            ttl: u64,
        ) -> DeviceEndorsement {
            let mut e = DeviceEndorsement {
                device_id: device.into(),
                public_key: key.to_vec(),
                name: "New".into(),
                nodes: vec![],
                issued_at_ms: issued,
                expires_at_ms: issued + ttl,
                endorser_device_id: by.into(),
                signature: Vec::new(),
            };
            e.signature = self.sign(&endorse_message(&e));
            e
        }
        pub fn revoke(&self, by: &str, device: &str, issued: u64) -> DeviceRevocation {
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

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn signed_bytes_and_fingerprints_match_the_shared_vectors() {
        let v: serde_json::Value = serde_json::from_str(include_str!(
            "../../../schema/fixtures/signing/device-trust-v1.json"
        ))
        .unwrap();
        let key = unhex(v["endorser_public_key"].as_str().unwrap());
        for f in v["fingerprints"].as_array().unwrap() {
            assert_eq!(
                fingerprint(&unhex(f["public_key"].as_str().unwrap())),
                f["fingerprint"].as_str().unwrap()
            );
        }
        for c in v["registrations"].as_array().unwrap() {
            let m = register_message(
                c["device_id"].as_str().unwrap(),
                c["name"].as_str().unwrap(),
                &unhex(c["public_key"].as_str().unwrap()),
                c["signed_at_ms"].as_u64().unwrap(),
            );
            assert_eq!(hex(&m), c["hex"].as_str().unwrap());
            assert!(verifies(&key, &m, &unhex(c["signature"].as_str().unwrap())));
        }
        for c in v["asks"].as_array().unwrap() {
            let m = ask_message(
                c["device_id"].as_str().unwrap(),
                c["signed_at_ms"].as_u64().unwrap(),
            );
            assert_eq!(hex(&m), c["hex"].as_str().unwrap());
            assert!(verifies(&key, &m, &unhex(c["signature"].as_str().unwrap())));
        }
        for c in v["endorsements"].as_array().unwrap() {
            let e = DeviceEndorsement {
                device_id: c["device_id"].as_str().unwrap().into(),
                public_key: unhex(c["public_key"].as_str().unwrap()),
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
                signature: unhex(c["signature"].as_str().unwrap()),
            };
            assert_eq!(hex(&endorse_message(&e)), c["hex"].as_str().unwrap());
        }
    }

    #[test]
    fn a_device_id_stays_bound_to_its_key_and_its_person_and_needs_its_key_to_register() {
        let store = SqliteStore::open_in_memory().unwrap();
        let d = Devices::new(&store);
        let (k1, k2) = (Key::new(), Key::new());
        d.register("owner", &k1.register("web-1", "Browser", 10), 10)
            .unwrap();
        d.register("owner", &k1.register("web-1", "Firefox", 20), 20)
            .unwrap();
        let list = d.list_for("owner").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(
            (
                list[0].name.as_str(),
                list[0].created_ms,
                list[0].last_used_ms
            ),
            ("Firefox", 10, 20)
        );
        assert!(!list[0].registration_signature.is_empty());
        for (user, k) in [("owner", &k2), ("eda", &k1)] {
            assert_eq!(
                d.register(user, &k.register("web-1", "x", 30), 30)
                    .unwrap_err()
                    .code,
                ErrorCode::DeviceKeyMismatch
            );
        }
        // A key nobody signed for (the hub or a client inventing one), a stale proof, a name with hidden characters: refused.
        let mut forged = k1.register("web-9", "x", 40);
        forged.public_key = k2.public();
        assert_eq!(
            d.register("owner", &forged, 40).unwrap_err().code,
            ErrorCode::DeviceSignatureInvalid
        );
        assert_eq!(
            d.register("owner", &k2.register("web-9", "x", 0), REGISTER_SKEW_MS + 1)
                .unwrap_err()
                .code,
            ErrorCode::DeviceSignatureInvalid
        );
        for name in ["a\u{7}b", "a\u{202e}b", "a\u{200b}b"] {
            assert_eq!(
                d.register("owner", &k2.register("web-9", name, 40), 40)
                    .unwrap_err()
                    .code,
                ErrorCode::ProtocolInvalidMessage
            );
        }
    }

    #[test]
    fn endorsements_and_revocations_are_verified_kept_once_and_bounded() {
        let store = SqliteStore::open_in_memory().unwrap();
        let d = Devices::new(&store);
        let (old, new, eda, stranger) = (Key::new(), Key::new(), Key::new(), Key::new());
        d.register("owner", &old.register("web-old", "Old", 1), 1)
            .unwrap();
        d.register("owner", &new.register("web-new", "New", 2), 2)
            .unwrap();
        d.register("eda", &eda.register("web-eda", "Eda", 3), 3)
            .unwrap();
        // Only the device itself can ask, and only now.
        assert_eq!(
            d.ask_approval("owner", &stranger.ask("web-new", 5), 5)
                .unwrap_err()
                .code,
            ErrorCode::DeviceSignatureInvalid
        );
        assert_eq!(
            d.ask_approval("owner", &new.ask("web-new", 5), 5 + ASK_SKEW_MS + 1)
                .unwrap_err()
                .code,
            ErrorCode::DeviceSignatureInvalid
        );
        assert_eq!(
            d.ask_approval("eda", &new.ask("web-new", 5), 5)
                .unwrap_err()
                .code,
            ErrorCode::DeviceUnknown
        );
        d.ask_approval("owner", &new.ask("web-new", 5), 5).unwrap();
        assert_eq!(d.list_for("owner").unwrap()[1].approval_asked_ms, 5);
        let day = 86_400_000;
        assert_eq!(
            d.record_endorsement(
                "owner",
                &eda.endorse("web-eda", "web-new", &new.public(), 1_000, day),
                10
            )
            .unwrap_err()
            .code,
            ErrorCode::DeviceUnknown
        );
        // Signed by someone other than the endorser it names: never stored.
        assert_eq!(
            d.record_endorsement(
                "owner",
                &stranger.endorse("web-old", "web-new", &new.public(), 1_000, day),
                10
            )
            .unwrap_err()
            .code,
            ErrorCode::DeviceSignatureInvalid
        );
        assert_eq!(
            d.record_endorsement(
                "owner",
                &old.endorse("web-old", "web-new", &stranger.public(), 1_000, day),
                10
            )
            .unwrap_err()
            .code,
            ErrorCode::DeviceKeyMismatch
        );
        assert_eq!(
            d.record_endorsement(
                "owner",
                &old.endorse(
                    "web-old",
                    "web-new",
                    &new.public(),
                    1_000,
                    MAX_ENDORSEMENT_MS + 1
                ),
                10
            )
            .unwrap_err()
            .code,
            ErrorCode::DeviceEndorsementExpired
        );
        let e = old.endorse("web-old", "web-new", &new.public(), 1_000, day);
        d.record_endorsement("owner", &e, 10).unwrap();
        d.record_endorsement("owner", &e, 11).unwrap();
        assert_eq!(
            d.for_machines(11).unwrap().0.len(),
            1,
            "the same signed endorsement is kept once"
        );
        assert_eq!(d.list_for("owner").unwrap()[1].endorsed_by, ["web-old"]);
        assert_eq!(d.list_for("owner").unwrap()[1].approval_asked_ms, 0);
        assert_eq!(
            d.record_revocation("owner", &stranger.revoke("web-old", "web-new", 20), 20)
                .unwrap_err()
                .code,
            ErrorCode::DeviceSignatureInvalid
        );
        let r = old.revoke("web-old", "web-new", 20);
        d.record_revocation("owner", &r, 20).unwrap();
        d.record_revocation("owner", &r, 21).unwrap();
        assert_eq!(d.for_machines(21).unwrap().1.len(), 1);
        assert_eq!(d.list_for("owner").unwrap()[1].revoked_ms, 20);
        // Forgotten after 400 days; a revoked device can be deleted from the list, a live one cannot.
        assert!(d
            .for_machines(21 + REVOCATION_KEPT_MS)
            .unwrap()
            .1
            .is_empty());
        assert!(!d.forget("web-old").unwrap());
        assert!(d.forget("web-new").unwrap());
        assert_eq!(d.list_for("owner").unwrap().len(), 1);
    }

    #[test]
    fn only_devices_not_revoked_count_toward_the_limit() {
        let store = SqliteStore::open_in_memory().unwrap();
        let d = Devices::new(&store);
        let keeper = Key::new();
        d.register("owner", &keeper.register("keep", "k", 1), 1)
            .unwrap();
        for i in 1..MAX_DEVICES_PER_USER {
            let k = Key::new();
            d.register("owner", &k.register(&format!("d{i}"), "x", 1), 1)
                .unwrap();
            if i == 1 {
                d.record_revocation("owner", &keeper.revoke("keep", "d1", 2), 2)
                    .unwrap();
            }
        }
        // 64 registered, one of them revoked: one more fits, then the limit holds.
        d.register("owner", &Key::new().register("more", "x", 3), 3)
            .unwrap();
        assert_eq!(
            d.register("owner", &Key::new().register("over", "x", 3), 3)
                .unwrap_err()
                .code,
            ErrorCode::NodeResourceLimit
        );
    }

    #[test]
    fn a_machine_is_offered_only_the_devices_of_people_who_work_on_it() {
        use crate::users::{Role, Users};
        let store = SqliteStore::open_in_memory().unwrap();
        let d = Devices::new(&store);
        let users = Users::new(&store);
        let eda = users.add("Eda", Role::Member, 1).unwrap();
        let ali = users.add("Ali", Role::Member, 1).unwrap();
        let boss = users.add("Boss", Role::Owner, 1).unwrap();
        users.grant(&eda.id, "laptop", "web").unwrap();
        users.grant(&ali.id, "vps", "api").unwrap();
        for (user, id) in [
            ("owner", "o"),
            (eda.id.as_str(), "e"),
            (ali.id.as_str(), "a"),
            (boss.id.as_str(), "b"),
        ] {
            d.register(user, &Key::new().register(id, "x", 1), 1)
                .unwrap();
        }
        let ids = |node: &str| -> Vec<String> {
            d.list_for_node(node)
                .unwrap()
                .into_iter()
                .map(|x| x.device_id)
                .collect()
        };
        assert_eq!(ids("laptop"), ["o", "e", "b"]);
        assert_eq!(ids("vps"), ["o", "a", "b"]);
        assert_eq!(ids("pi"), ["o", "b"]);
    }

    #[test]
    fn records_dated_in_the_future_are_refused() {
        let store = SqliteStore::open_in_memory().unwrap();
        let d = Devices::new(&store);
        let (a, b) = (Key::new(), Key::new());
        d.register("owner", &a.register("a", "A", 1), 1).unwrap();
        d.register("owner", &b.register("b", "B", 1), 1).unwrap();
        let far = u64::MAX / 2;
        assert_eq!(
            d.record_revocation("owner", &a.revoke("a", "b", far), 10)
                .unwrap_err()
                .code,
            ErrorCode::ProtocolInvalidMessage
        );
        assert!(d.for_machines(10).unwrap().1.is_empty());
        let e = a.endorse("a", "b", &b.public(), 10 + ASK_SKEW_MS + 1, 86_400_000);
        assert!(d.record_endorsement("owner", &e, 10).is_err());
    }
}
