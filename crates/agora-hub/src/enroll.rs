//! Device enrollment (ADR-0015, threat T3 "rogue node enrollment"). The user asks the hub for a one-time code in
//! the window; the node presents it with its CSR; the hub issues a certificate for the node id the user approved.
//!
//! - Codes are random, short-lived and single use; only their hash is stored.
//! - Wrong guesses are rate limited: after [`MAX_FAILURES`] failures the redeemer is locked out for
//!   [`LOCKOUT_MS`], so guessing a code is not practical within its lifetime.
//! - Every issued certificate is recorded by serial so it can be revoked at once; a revoked device is refused
//!   by [`Enrollment::is_active`] whatever its certificate's own expiry.

use sha2::{Digest, Sha256};

use rusqlite::{params, OptionalExtension};

use crate::ca::{Ca, CaError, IssuedCert};
use crate::store::{SqliteStore, StoreError};

/// Code lifetime. Long enough to type on another machine, short enough to be useless later.
pub const CODE_TTL_MS: u64 = 10 * 60 * 1000;
pub const MAX_FAILURES: u32 = 5;
pub const LOCKOUT_MS: u64 = 60 * 1000;

/// Crockford-style alphabet without look-alikes (no 0/O, 1/I/L, U).
/// No 0/O, 1/I/L or U, and nothing that could be taken for the display hyphen.
const ALPHABET: &[u8; 30] = b"23456789ABCDEFGHJKMNPQRSTVWXYZ";

#[derive(Debug)]
pub enum EnrollError {
    /// Unknown, expired or already used code. Deliberately indistinguishable.
    BadCode,
    /// Too many wrong codes recently; try again after the lockout.
    LockedOut {
        retry_after_ms: u64,
    },
    Ca(CaError),
    Store(StoreError),
}

impl From<StoreError> for EnrollError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

impl From<rusqlite::Error> for EnrollError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Store(e.into())
    }
}

impl From<CaError> for EnrollError {
    fn from(e: CaError) -> Self {
        Self::Ca(e)
    }
}

/// A random one-time code shown as `XXXX-XXXX` (about 39 bits). Bytes are drawn by rejection so every symbol is equally
/// likely.
pub(crate) fn new_code() -> Result<String, StoreError> {
    let limit = u8::try_from(256 - 256 % ALPHABET.len()).unwrap_or(u8::MAX);
    let mut chars: Vec<char> = Vec::with_capacity(8);
    while chars.len() < 8 {
        let mut raw = [0u8; 16];
        getrandom::getrandom(&mut raw).map_err(|e| StoreError(e.to_string()))?;
        chars.extend(
            raw.iter()
                .filter(|b| **b < limit)
                .map(|b| char::from(ALPHABET[usize::from(*b) % ALPHABET.len()])),
        );
    }
    chars.truncate(8);
    Ok(format!(
        "{}-{}",
        chars[..4].iter().collect::<String>(),
        chars[4..].iter().collect::<String>()
    ))
}

pub(crate) fn hash(code: &str) -> Vec<u8> {
    // Normalize what a person might type: case and the display hyphen.
    let clean: String = code
        .chars()
        .filter(|c| *c != '-' && !c.is_whitespace())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    Sha256::digest(clean.as_bytes()).to_vec()
}

fn to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

/// Failed-guess bookkeeping, kept apart from [`Enrollment`] so one instance can be shared by every connection.
#[derive(Debug, Default)]
pub struct Lockout {
    state: std::sync::Mutex<(u32, u64)>,
}

impl Lockout {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn check(&self, now_ms: u64) -> Option<u64> {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        (now_ms < s.1).then(|| s.1 - now_ms)
    }

    pub(crate) fn failed(&self, now_ms: u64) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.0 += 1;
        if s.0 >= MAX_FAILURES {
            s.0 = 0;
            s.1 = now_ms.saturating_add(LOCKOUT_MS);
        }
    }

    pub(crate) fn succeeded(&self) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).0 = 0;
    }
}

pub struct Enrollment<'a> {
    store: &'a SqliteStore,
    lockout: std::sync::Arc<Lockout>,
}

impl<'a> Enrollment<'a> {
    pub fn new(store: &'a SqliteStore) -> Self {
        Self::with_lockout(store, std::sync::Arc::new(Lockout::new()))
    }

    /// Shares the failed-guess counter with other `Enrollment`s (one per connection, one lockout for the hub).
    pub fn with_lockout(store: &'a SqliteStore, lockout: std::sync::Arc<Lockout>) -> Self {
        Self { store, lockout }
    }

    /// A new one-time code, shown as `XXXX-XXXX`. about 39 bits of randomness.
    pub fn create_code(&self, now_ms: u64) -> Result<String, EnrollError> {
        let code = new_code()?;
        let conn = self.store.lock()?;
        // Expired codes are of no use to anyone; drop them as we go.
        conn.execute(
            "DELETE FROM enrollment_codes WHERE expires_ms <= ?1",
            [to_i64(now_ms)],
        )?;
        conn.execute(
            "INSERT INTO enrollment_codes (code_hash, expires_ms) VALUES (?1, ?2)",
            params![hash(&code), to_i64(now_ms.saturating_add(CODE_TTL_MS))],
        )?;
        Ok(code)
    }

    /// Spends `code` and signs the node's CSR as `node_id`. The code is consumed only if a certificate is issued.
    pub fn redeem(
        &mut self,
        ca: &Ca,
        code: &str,
        node_id: &str,
        csr_pem: &str,
        now_ms: u64,
    ) -> Result<IssuedCert, EnrollError> {
        if let Some(retry_after_ms) = self.lockout.check(now_ms) {
            return Err(EnrollError::LockedOut { retry_after_ms });
        }
        // Issue first: a bad CSR or node id must not burn a valid code (the user would have to start over).
        let mut conn = self.store.lock()?;
        let tx = conn.transaction()?;
        let live: Option<i64> = tx
            .query_row(
                "SELECT expires_ms FROM enrollment_codes WHERE code_hash = ?1",
                [hash(code)],
                |r| r.get(0),
            )
            .optional()?;
        if live.is_none_or(|exp| exp <= to_i64(now_ms)) {
            drop(tx);
            drop(conn);
            self.lockout.failed(now_ms);
            return Err(EnrollError::BadCode);
        }
        let issued = ca.issue_node(node_id, csr_pem)?;
        tx.execute(
            "DELETE FROM enrollment_codes WHERE code_hash = ?1",
            [hash(code)],
        )?;
        // One live identity per node id: enrolling again under a name replaces what was there (the old certificate is revoked,
        // and a connection it still holds is closed), so a redeemed code can never add a second route for an existing node.
        tx.execute(
            "UPDATE devices SET revoked_ms = ?2 WHERE node_id = ?1 AND revoked_ms IS NULL",
            params![node_id, to_i64(now_ms)],
        )?;
        tx.execute(
            "INSERT INTO devices (serial, node_id, enrolled_ms) VALUES (?1, ?2, ?3)",
            params![issued.serial_hex, node_id, to_i64(now_ms)],
        )?;
        tx.commit()?;
        self.lockout.succeeded();
        Ok(issued)
    }

    /// Revokes every certificate issued for `node_id`. Immediate: the next connection check refuses it.
    pub fn revoke(&self, node_id: &str, now_ms: u64) -> Result<usize, EnrollError> {
        let conn = self.store.lock()?;
        let n = conn.execute(
            "UPDATE devices SET revoked_ms = ?2 WHERE node_id = ?1 AND revoked_ms IS NULL",
            params![node_id, to_i64(now_ms)],
        )?;
        Ok(n)
    }

    /// Is this certificate serial enrolled and not revoked? Unknown serials are not active.
    pub fn is_active(&self, serial_hex: &str) -> Result<bool, EnrollError> {
        let conn = self.store.lock()?;
        let revoked: Option<Option<i64>> = conn
            .query_row(
                "SELECT revoked_ms FROM devices WHERE serial = ?1",
                [serial_hex],
                |r| r.get(0),
            )
            .optional()?;
        Ok(matches!(revoked, Some(None)))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    #[test]
    fn a_code_is_two_groups_of_four_from_the_unambiguous_alphabet() {
        for _ in 0..500 {
            let c = new_code().unwrap();
            let (a, b) = c.split_once('-').expect("one hyphen");
            assert_eq!((a.len(), b.len()), (4, 4), "{c}");
            assert!(
                c.chars()
                    .filter(|c| *c != '-')
                    .all(|c| ALPHABET.contains(&(c as u8))),
                "{c}"
            );
        }
    }

    use super::*;
    use crate::ca::new_device_request;

    fn setup() -> (SqliteStore, Ca, String) {
        let (_, csr) = new_device_request().unwrap();
        (
            SqliteStore::open_in_memory().unwrap(),
            Ca::generate("ca").unwrap(),
            csr,
        )
    }

    #[test]
    fn a_code_enrolls_one_device_once() {
        let (store, ca, csr) = setup();
        let mut e = Enrollment::new(&store);
        let code = e.create_code(1000).unwrap();
        assert_eq!(code.len(), 9);
        let issued = e.redeem(&ca, &code, "laptop", &csr, 2000).unwrap();
        assert!(e.is_active(&issued.serial_hex).unwrap());
        assert!(matches!(
            e.redeem(&ca, &code, "other", &csr, 3000),
            Err(EnrollError::BadCode)
        ));
    }

    #[test]
    fn enrolling_again_under_a_name_replaces_the_old_identity_instead_of_adding_a_second_one() {
        let (store, ca, csr) = setup();
        let mut e = Enrollment::new(&store);
        let first = {
            let c = e.create_code(0).unwrap();
            e.redeem(&ca, &c, "vps", &csr, 1).unwrap()
        };
        let second = {
            let c = e.create_code(0).unwrap();
            // A reinstalled machine has a new key, hence a new request.
            let (_, fresh) = new_device_request().unwrap();
            e.redeem(&ca, &c, "vps", &fresh, 2).unwrap()
        };
        assert!(!e.is_active(&first.serial_hex).unwrap());
        assert!(e.is_active(&second.serial_hex).unwrap());
        // Other names are untouched.
        let other = {
            let c = e.create_code(0).unwrap();
            let (_, fresh) = new_device_request().unwrap();
            e.redeem(&ca, &c, "laptop", &fresh, 3).unwrap()
        };
        assert!(e.is_active(&other.serial_hex).unwrap());
        assert!(e.is_active(&second.serial_hex).unwrap());
    }

    #[test]
    fn codes_are_forgiving_about_case_and_hyphen_but_expire() {
        let (store, ca, csr) = setup();
        let mut e = Enrollment::new(&store);
        let code = e.create_code(0).unwrap();
        let typed = code.replace('-', " ").to_lowercase();
        assert!(e.redeem(&ca, &typed, "a", &csr, 1).is_ok());
        let late = e.create_code(0).unwrap();
        assert!(matches!(
            e.redeem(&ca, &late, "b", &csr, CODE_TTL_MS),
            Err(EnrollError::BadCode)
        ));
    }

    #[test]
    fn a_bad_request_does_not_burn_a_valid_code() {
        let (store, ca, csr) = setup();
        let mut e = Enrollment::new(&store);
        let code = e.create_code(0).unwrap();
        assert!(matches!(
            e.redeem(&ca, &code, "bad id!", &csr, 1),
            Err(EnrollError::Ca(_))
        ));
        assert!(e.redeem(&ca, &code, "good-id", &csr, 2).is_ok());
    }

    #[test]
    fn guessing_is_locked_out_even_for_the_right_code() {
        let (store, ca, csr) = setup();
        let mut e = Enrollment::new(&store);
        let code = e.create_code(0).unwrap();
        for i in 0..MAX_FAILURES {
            assert!(matches!(
                e.redeem(&ca, "AAAA-AAAA", "n", &csr, u64::from(i)),
                Err(EnrollError::BadCode)
            ));
        }
        match e.redeem(&ca, &code, "n", &csr, 10) {
            Err(EnrollError::LockedOut { retry_after_ms }) => assert!(retry_after_ms > 0),
            other => panic!("expected lockout, got {other:?}", other = other.map(|_| ())),
        }
        // After the lockout the real code still works.
        assert!(e.redeem(&ca, &code, "n", &csr, LOCKOUT_MS + 100).is_ok());
    }

    #[test]
    fn revocation_is_immediate_and_unknown_serials_are_inactive() {
        let (store, ca, csr) = setup();
        let mut e = Enrollment::new(&store);
        let code = e.create_code(0).unwrap();
        let issued = e.redeem(&ca, &code, "node-1", &csr, 1).unwrap();
        assert_eq!(e.revoke("node-1", 5).unwrap(), 1);
        assert!(!e.is_active(&issued.serial_hex).unwrap());
        assert!(!e.is_active("deadbeef").unwrap());
        assert_eq!(e.revoke("node-1", 6).unwrap(), 0);
    }

    #[test]
    fn stored_codes_are_hashed() {
        let (store, _, _) = setup();
        let e = Enrollment::new(&store);
        let code = e.create_code(0).unwrap();
        let conn = store.lock().unwrap();
        let stored: Vec<u8> = conn
            .query_row("SELECT code_hash FROM enrollment_codes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(stored.len(), 32);
        assert!(!stored.windows(4).any(|w| code.as_bytes().starts_with(w)));
    }
}
