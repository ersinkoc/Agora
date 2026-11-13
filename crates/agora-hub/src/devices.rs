//! Registered devices (ADR-0061): the browsers and terminal clients each person signs with, and the endorsements and revocations the
//! hub carries to machines.
//!
//! The hub's list is only a list of candidates. A machine never trusts a device because it is listed here: its owner says yes on the
//! machine, or a device the machine already trusts signs an endorsement, which the machine verifies itself. So nothing in this module
//! can widen what a machine trusts; at worst a lying hub offers a candidate the owner then refuses after comparing fingerprints.

use agora_protocol::v1::{DeviceEndorsement, DeviceInfo, DeviceRegister, DeviceRevocation};
use agora_protocol::{Error, ErrorCode};
use prost::Message;
use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::store::{SqliteStore, StoreError};

/// The longest an endorsement may last from when it was signed (ADR-0061: 90 days, renewed on the machine by use).
pub const MAX_ENDORSEMENT_MS: u64 = 90 * 24 * 60 * 60 * 1000;
/// Most machines one endorsement may name.
pub const MAX_SCOPE: usize = 64;
/// Most devices one person may register (a browser profile made per visit must not fill the hub).
pub const MAX_DEVICES_PER_USER: usize = 64;

/// What people compare: the first 8 bytes of the SHA-256 of the 65-byte public key, lowercase hex in groups of 2 bytes joined by `-`
/// (`3f9a-07c2-11be-d4e0`). Vectors: `schema/fixtures/signing/device-trust-v1.json`.
pub fn fingerprint(public_key: &[u8]) -> String {
    let digest = Sha256::digest(public_key);
    let hex: Vec<String> = digest
        .get(..8)
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

/// A name a person reads: at most 64 characters, no control characters.
pub fn valid_name(name: &str) -> bool {
    name.chars().count() <= 64 && !name.chars().any(char::is_control)
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
                    "UPDATE user_devices SET name = ?2, last_used_ms = ?3 WHERE device_id = ?1",
                    params![r.device_id, r.name, to_i64(now_ms)],
                )
                .map_err(|e| store_error(e.into()))?;
                Ok(())
            }
            None => {
                let count: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM user_devices WHERE user_id = ?1",
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
                    "INSERT INTO user_devices (device_id, user_id, name, public_key, created_ms, last_used_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                    params![r.device_id, user, r.name, r.public_key, to_i64(now_ms)],
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
                "SELECT device_id, name, public_key, created_ms, last_used_ms, user_id, approval_asked_ms, revoked_ms
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

    /// Every registered device: what a machine is offered at enrollment or `agora node trust --from-hub`, and `agora hub devices`.
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

    /// This device of `user` asks another device of theirs to approve it.
    pub fn ask_approval(&self, user: &str, id: &str, now_ms: u64) -> Result<(), Error> {
        self.own(user, id)?;
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
        if e.expires_at_ms <= now_ms
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
        self.own(user, &e.endorser_device_id)?;
        let conn = self.store.lock().map_err(store_error)?;
        conn.execute(
            "INSERT INTO device_endorsements (device_id, endorser, issued_ms, expires_ms, payload) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                e.device_id,
                e.endorser_device_id,
                to_i64(e.issued_at_ms),
                to_i64(e.expires_at_ms),
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
        self.own(user, &r.device_id)?;
        self.own(user, &r.revoker_device_id)?;
        let conn = self.store.lock().map_err(store_error)?;
        conn.execute(
            "INSERT INTO device_revocations (device_id, revoker, issued_ms, payload) VALUES (?1, ?2, ?3, ?4)",
            params![
                r.device_id,
                r.revoker_device_id,
                to_i64(r.issued_at_ms),
                r.encode_to_vec()
            ],
        )
        .map_err(|e| store_error(e.into()))?;
        conn.execute(
            "UPDATE user_devices SET revoked_ms = ?2, approval_asked_ms = 0 WHERE device_id = ?1",
            params![r.device_id, to_i64(now_ms)],
        )
        .map_err(|e| store_error(e.into()))?;
        Ok(())
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
            .prepare("SELECT payload FROM device_revocations ORDER BY id")
            .and_then(|mut s| {
                s.query_map([], |row| row.get::<_, Vec<u8>>(0))
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
mod tests {
    use super::*;

    fn key(n: u8) -> Vec<u8> {
        let mut k = vec![4u8; 65];
        k[1] = n;
        k
    }

    fn reg(id: &str, n: u8) -> DeviceRegister {
        DeviceRegister {
            device_id: id.into(),
            name: format!("Browser {n}"),
            public_key: key(n),
        }
    }

    fn endorsement(device: &str, n: u8, by: &str) -> DeviceEndorsement {
        DeviceEndorsement {
            device_id: device.into(),
            public_key: key(n),
            name: "New".into(),
            nodes: vec![],
            issued_at_ms: 1_000,
            expires_at_ms: 1_000 + MAX_ENDORSEMENT_MS,
            endorser_device_id: by.into(),
            signature: vec![0; 64],
        }
    }

    #[test]
    fn fingerprints_match_the_shared_vectors() {
        let v: serde_json::Value = serde_json::from_str(include_str!(
            "../../../schema/fixtures/signing/device-trust-v1.json"
        ))
        .unwrap();
        for f in v["fingerprints"].as_array().unwrap() {
            let k: Vec<u8> = (0..130)
                .step_by(2)
                .map(|i| {
                    u8::from_str_radix(&f["public_key"].as_str().unwrap()[i..i + 2], 16).unwrap()
                })
                .collect();
            assert_eq!(fingerprint(&k), f["fingerprint"].as_str().unwrap());
        }
    }

    #[test]
    fn a_device_id_stays_bound_to_its_key_and_its_person() {
        let store = SqliteStore::open_in_memory().unwrap();
        let d = Devices::new(&store);
        d.register("owner", &reg("web-1", 1), 10).unwrap();
        // Again later, under a new name: touched, not duplicated.
        let mut again = reg("web-1", 1);
        again.name = "Firefox".into();
        d.register("owner", &again, 20).unwrap();
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
        // Another key under the same id, or the same id for another person: refused.
        for (user, n) in [("owner", 2), ("eda", 1)] {
            let e = d.register(user, &reg("web-1", n), 30).unwrap_err();
            assert_eq!(e.code, ErrorCode::DeviceKeyMismatch);
        }
        assert!(d.list_for("eda").unwrap().is_empty());
        assert_eq!(d.list_all().unwrap().len(), 1);
        for bad in [
            DeviceRegister {
                device_id: "bad id".into(),
                ..reg("x", 1)
            },
            DeviceRegister {
                public_key: vec![4; 64],
                ..reg("web-2", 1)
            },
            DeviceRegister {
                name: "a\u{7}b".into(),
                ..reg("web-3", 1)
            },
        ] {
            assert_eq!(
                d.register("owner", &bad, 1).unwrap_err().code,
                ErrorCode::ProtocolInvalidMessage
            );
        }
    }

    #[test]
    fn endorsements_are_kept_only_between_a_persons_own_devices() {
        let store = SqliteStore::open_in_memory().unwrap();
        let d = Devices::new(&store);
        d.register("owner", &reg("web-old", 1), 1).unwrap();
        d.register("owner", &reg("web-new", 2), 2).unwrap();
        d.register("eda", &reg("web-eda", 3), 3).unwrap();
        d.ask_approval("owner", "web-new", 5).unwrap();
        assert_eq!(d.list_for("owner").unwrap()[1].approval_asked_ms, 5);
        // Someone else's device can neither be asked for nor endorsed nor used to endorse.
        assert_eq!(
            d.ask_approval("eda", "web-new", 5).unwrap_err().code,
            ErrorCode::DeviceUnknown
        );
        assert_eq!(
            d.record_endorsement("owner", &endorsement("web-new", 2, "web-eda"), 10)
                .unwrap_err()
                .code,
            ErrorCode::DeviceUnknown
        );
        assert_eq!(
            d.record_endorsement("eda", &endorsement("web-new", 2, "web-eda"), 10)
                .unwrap_err()
                .code,
            ErrorCode::DeviceUnknown
        );
        // A key other than the registered one is refused before any machine sees it.
        assert_eq!(
            d.record_endorsement("owner", &endorsement("web-new", 9, "web-old"), 10)
                .unwrap_err()
                .code,
            ErrorCode::DeviceKeyMismatch
        );
        // Longer than 90 days, or already over: refused.
        let mut long = endorsement("web-new", 2, "web-old");
        long.expires_at_ms += 1;
        assert_eq!(
            d.record_endorsement("owner", &long, 10).unwrap_err().code,
            ErrorCode::DeviceEndorsementExpired
        );
        d.record_endorsement("owner", &endorsement("web-new", 2, "web-old"), 10)
            .unwrap();
        let list = d.list_for("owner").unwrap();
        assert_eq!(list[1].endorsed_by, ["web-old"]);
        assert_eq!(list[1].approval_asked_ms, 0, "the card goes away");
        let (e, r) = d.for_machines(11).unwrap();
        assert_eq!((e.len(), r.len()), (1, 0));
        assert!(d
            .for_machines(2_000 + MAX_ENDORSEMENT_MS)
            .unwrap()
            .0
            .is_empty());
        d.record_revocation(
            "owner",
            &DeviceRevocation {
                device_id: "web-new".into(),
                issued_at_ms: 20,
                revoker_device_id: "web-old".into(),
                signature: vec![0; 64],
            },
            20,
        )
        .unwrap();
        assert_eq!(d.for_machines(21).unwrap().1.len(), 1);
        assert_eq!(d.list_for("owner").unwrap()[1].revoked_ms, 20);
    }
}
