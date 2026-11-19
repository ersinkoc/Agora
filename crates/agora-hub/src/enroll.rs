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

/// ADR-0085: a machine gets at most one certificate an hour.
pub const RENEW_MIN_GAP_MS: u64 = 60 * 60 * 1000;
/// ADR-0085: an older certificate of a machine is revoked this long after the machine first connected with a newer one.
pub const SUPERSEDED_GRACE_MS: u64 = 7 * 24 * 60 * 60 * 1000;

/// What a renewal needs to know about the certificate that asks for it.
pub struct RenewFrom<'a> {
    pub serial_hex: &'a str,
    /// The node id the certificate names.
    pub node_id: &'a str,
    /// When the certificate ends, seconds since 1970.
    pub not_after_secs: i64,
}

/// Why a renewal was refused; the word is the `reason` of `enroll.renew_refused`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenewRefusal {
    Unknown,
    Revoked,
    Expired,
    TooOld,
    TooFrequent,
}

impl RenewRefusal {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Revoked => "revoked",
            Self::Expired => "expired",
            Self::TooOld => "too_old",
            Self::TooFrequent => "too_frequent",
        }
    }
}

#[derive(Debug)]
pub enum RenewError {
    Refused(RenewRefusal),
    /// The CA would not sign the request (an unparseable CSR).
    Ca(CaError),
    Store(StoreError),
}

impl From<RenewRefusal> for RenewError {
    fn from(r: RenewRefusal) -> Self {
        Self::Refused(r)
    }
}

impl From<CaError> for RenewError {
    fn from(e: CaError) -> Self {
        Self::Ca(e)
    }
}

impl From<StoreError> for RenewError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

impl From<rusqlite::Error> for RenewError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Store(e.into())
    }
}

/// Crockford-style alphabet without look-alikes (no 0/O, 1/I/L, U).
/// No 0/O, 1/I/L or U, and nothing that could be taken for the display hyphen.
const ALPHABET: &[u8; 30] = b"23456789ABCDEFGHJKMNPQRSTVWXYZ";

#[derive(Debug)]
pub enum EnrollError {
    /// Unknown, expired or already used code. Deliberately indistinguishable.
    BadCode,
    /// Pairing: the node id belongs to an enrolled machine and the owner did not say "replace".
    NameInUse,
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

    /// Pairing (ADR-0065): issues the certificate the owner approved, for the node id and CSR the session froze. A node id that is
    /// enrolled and not revoked is refused unless `replace`; with it the old certificates are revoked (and a connection they still
    /// hold is closed), exactly as enrolling again does. There is no code: the owner's approval was the authority.
    pub fn issue_paired(
        &self,
        ca: &Ca,
        node_id: &str,
        csr_pem: &str,
        replace: bool,
        now_ms: u64,
    ) -> Result<IssuedCert, EnrollError> {
        let mut conn = self.store.lock()?;
        let tx = conn.transaction()?;
        let taken: bool = tx
            .query_row(
                "SELECT 1 FROM devices WHERE node_id = ?1 AND revoked_ms IS NULL LIMIT 1",
                [node_id],
                |_| Ok(true),
            )
            .optional()?
            .is_some();
        if taken && !replace {
            return Err(EnrollError::NameInUse);
        }
        let issued = ca.issue_node(node_id, csr_pem)?;
        tx.execute(
            "UPDATE devices SET revoked_ms = ?2 WHERE node_id = ?1 AND revoked_ms IS NULL",
            params![node_id, to_i64(now_ms)],
        )?;
        tx.execute(
            "INSERT INTO devices (serial, node_id, enrolled_ms) VALUES (?1, ?2, ?3)",
            params![issued.serial_hex, node_id, to_i64(now_ms)],
        )?;
        tx.commit()?;
        Ok(issued)
    }

    /// ADR-0085: renews a machine's certificate. `from` is the certificate of the connection that asked (already verified by TLS and
    /// found active by the caller; every fact here is checked again against the record). Issues a certificate for the same node id with
    /// the same usages and a fresh key (the CSR's key; its name, extensions and usages are ignored, as at enrollment), records it and an
    /// audit row in one transaction, and revokes nothing: the previous certificate stays valid until its own end (see
    /// [`Self::revoke_superseded`]).
    pub fn renew(
        &self,
        ca: &Ca,
        from: &RenewFrom<'_>,
        csr_pem: &str,
        now_ms: u64,
    ) -> Result<IssuedCert, RenewError> {
        let mut conn = self.store.lock()?;
        let tx = conn.transaction()?;
        let row: Option<(String, i64, Option<i64>)> = tx
            .query_row(
                "SELECT node_id, enrolled_ms, revoked_ms FROM devices WHERE serial = ?1",
                [from.serial_hex],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        // A serial the hub never issued, or one issued to another machine than the certificate names: nothing to renew.
        let Some((node, issued_ms, revoked)) = row.filter(|(n, _, _)| n == from.node_id) else {
            return Err(RenewError::Refused(RenewRefusal::Unknown));
        };
        if revoked.is_some() {
            return Err(RenewRefusal::Revoked.into());
        }
        let now_s = i64::try_from(now_ms / 1000).unwrap_or(i64::MAX);
        if from.not_after_secs <= now_s {
            return Err(RenewRefusal::Expired.into());
        }
        // The hub's own record of when it issued the certificate: a certificate older than one lifetime cannot be prolonged, whatever
        // end it carries.
        if issued_ms.saturating_add(to_i64(ca.lifetime_ms())) <= to_i64(now_ms) {
            return Err(RenewRefusal::TooOld.into());
        }
        // One certificate per machine per hour, counting every certificate the hub issued for it (enrollment included).
        let latest: i64 = tx.query_row(
            "SELECT MAX(enrolled_ms) FROM devices WHERE node_id = ?1",
            [&node],
            |r| r.get(0),
        )?;
        if latest.saturating_add(to_i64(RENEW_MIN_GAP_MS)) > to_i64(now_ms) {
            return Err(RenewRefusal::TooFrequent.into());
        }
        let issued = ca.issue_node(&node, csr_pem)?;
        tx.execute(
            "INSERT INTO devices (serial, node_id, enrolled_ms) VALUES (?1, ?2, ?3)",
            params![issued.serial_hex, node, to_i64(now_ms)],
        )?;
        tx.execute(
            "INSERT INTO audit (at_ms, who, session, action, node, outcome) VALUES (?1, ?2, '', 'enroll.renew', ?2, ?3)",
            params![to_i64(now_ms), node, format!("ok serial={}", issued.serial_hex)],
        )?;
        tx.commit()?;
        Ok(issued)
    }

    /// ADR-0085 grace: revokes every certificate that a newer one of the same machine has replaced, once the machine first connected
    /// with that newer one at least [`SUPERSEDED_GRACE_MS`] ago. Returns how many it revoked. A connection still open on a revoked
    /// certificate is closed by the usual check.
    pub fn revoke_superseded(&self, now_ms: u64) -> Result<usize, EnrollError> {
        let conn = self.store.lock()?;
        let n = conn.execute(
            "UPDATE devices SET revoked_ms = ?1 WHERE revoked_ms IS NULL AND EXISTS (
                 SELECT 1 FROM devices n WHERE n.node_id = devices.node_id AND n.enrolled_ms > devices.enrolled_ms
                 AND n.revoked_ms IS NULL AND n.first_seen_ms IS NOT NULL AND n.first_seen_ms + ?2 <= ?1)",
            params![to_i64(now_ms), to_i64(SUPERSEDED_GRACE_MS)],
        )?;
        Ok(n)
    }

    /// Notes that a machine connected with this certificate (the first time only), then applies [`Self::revoke_superseded`].
    pub fn note_connected(&self, serial_hex: &str, now_ms: u64) -> Result<(), EnrollError> {
        {
            let conn = self.store.lock()?;
            conn.execute(
                "UPDATE devices SET first_seen_ms = ?2 WHERE serial = ?1 AND first_seen_ms IS NULL",
                params![serial_hex, to_i64(now_ms)],
            )?;
        }
        self.revoke_superseded(now_ms)?;
        Ok(())
    }

    /// Does an enrolled, not revoked machine already have this node id?
    pub fn node_in_use(&self, node_id: &str) -> Result<bool, EnrollError> {
        let conn = self.store.lock()?;
        Ok(conn
            .query_row(
                "SELECT 1 FROM devices WHERE node_id = ?1 AND revoked_ms IS NULL LIMIT 1",
                [node_id],
                |_| Ok(true),
            )
            .optional()?
            .is_some())
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

    /// When each machine's current certificate was issued (milliseconds), for the machines that have one. The hub does not store
    /// a certificate's end; it is this plus the CA's lifetime.
    pub fn issued_ms_by_node(&self) -> Result<std::collections::HashMap<String, u64>, EnrollError> {
        let conn = self.store.lock()?;
        let mut stmt = conn.prepare(
            "SELECT node_id, MAX(enrolled_ms) FROM devices WHERE revoked_ms IS NULL GROUP BY node_id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        let mut out = std::collections::HashMap::new();
        for row in rows {
            let (node, ms) = row?;
            out.insert(node, u64::try_from(ms).unwrap_or(0));
        }
        Ok(out)
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
    fn the_hub_knows_when_each_machines_current_certificate_was_issued() {
        let (store, ca, _) = setup();
        let mut e = Enrollment::new(&store);
        let c1 = e.create_code(1000).unwrap();
        e.redeem(&ca, &c1, "a", &new_device_request().unwrap().1, 2000)
            .unwrap();
        let c2 = e.create_code(1000).unwrap();
        e.redeem(&ca, &c2, "a", &new_device_request().unwrap().1, 3000)
            .unwrap();
        let c3 = e.create_code(1000).unwrap();
        e.redeem(&ca, &c3, "b", &new_device_request().unwrap().1, 4000)
            .unwrap();
        let m = e.issued_ms_by_node().unwrap();
        assert_eq!((m["a"], m["b"], m.len()), (3000, 4000, 2));
        e.revoke("b", 5000).unwrap();
        assert!(!e.issued_ms_by_node().unwrap().contains_key("b"));
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

    // ---- ADR-0085: renewal ----

    const HOUR: u64 = 60 * 60 * 1000;
    const DAY: u64 = 24 * HOUR;

    /// A machine enrolled at `at` (ms). Returns the record's serial and the certificate's own end (seconds).
    fn enrolled(store: &SqliteStore, ca: &Ca, node: &str, at: u64) -> (String, i64) {
        let mut e = Enrollment::new(store);
        let code = e.create_code(at).unwrap();
        let (_, csr) = new_device_request().unwrap();
        let issued = e.redeem(ca, &code, node, &csr, at).unwrap();
        let end = crate::ca::cert_not_after_secs(&issued.cert_pem).unwrap();
        (issued.serial_hex, end)
    }

    fn from<'a>(serial: &'a str, node: &'a str, end: i64) -> RenewFrom<'a> {
        RenewFrom {
            serial_hex: serial,
            node_id: node,
            not_after_secs: end,
        }
    }

    fn refusal(r: Result<IssuedCert, RenewError>) -> &'static str {
        match r {
            Err(RenewError::Refused(why)) => why.reason(),
            other => panic!("expected a refusal, got {:?}", other.map(|c| c.serial_hex)),
        }
    }

    /// A day after the machine was enrolled: inside the certificate's life, past the hour.
    fn later(at: u64) -> u64 {
        at + DAY
    }

    fn device_rows(store: &SqliteStore) -> i64 {
        store
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM devices", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn a_renewal_is_refused_for_an_unknown_serial_and_for_a_serial_of_another_machine() {
        let (store, ca, _) = setup();
        let e = Enrollment::new(&store);
        let (serial, end) = enrolled(&store, &ca, "a", 1_000_000);
        let (_, csr) = new_device_request().unwrap();
        let now = later(1_000_000);
        assert_eq!(
            refusal(e.renew(&ca, &from("deadbeef", "a", end), &csr, now)),
            "unknown"
        );
        // The certificate names "b" but the serial was issued to "a".
        assert_eq!(
            refusal(e.renew(&ca, &from(&serial, "b", end), &csr, now)),
            "unknown"
        );
    }

    #[test]
    fn a_revoked_certificate_cannot_renew() {
        let (store, ca, _) = setup();
        let e = Enrollment::new(&store);
        let (serial, end) = enrolled(&store, &ca, "a", 1_000_000);
        e.revoke("a", 1_000_500).unwrap();
        let (_, csr) = new_device_request().unwrap();
        assert_eq!(
            refusal(e.renew(&ca, &from(&serial, "a", end), &csr, later(1_000_000))),
            "revoked"
        );
        assert_eq!(device_rows(&store), 1, "nothing was issued");
    }

    #[test]
    fn a_renewal_after_the_certificates_own_end_is_refused() {
        let (store, ca, _) = setup();
        let e = Enrollment::new(&store);
        let (_, csr) = new_device_request().unwrap();
        // The certificate is said to end at 5000 s; the hub's clock decides, to the millisecond.
        let (a, _) = enrolled(&store, &ca, "a", 1_000_000);
        assert!(e.renew(&ca, &from(&a, "a", 5000), &csr, 4_999_999).is_ok());
        let (b, _) = enrolled(&store, &ca, "b", 1_000_000);
        assert_eq!(
            refusal(e.renew(&ca, &from(&b, "b", 5000), &csr, 5_000_000)),
            "expired"
        );
        let (c, _) = enrolled(&store, &ca, "c", 1_000_000);
        assert_eq!(
            refusal(e.renew(&ca, &from(&c, "c", 5000), &csr, 9_000_000)),
            "expired"
        );
    }

    #[test]
    fn a_certificate_the_hub_issued_more_than_a_lifetime_ago_is_too_old_whatever_end_it_carries() {
        let (store, ca, _) = setup();
        let e = Enrollment::new(&store);
        let (serial, _) = enrolled(&store, &ca, "a", 1_000_000);
        let (_, csr) = new_device_request().unwrap();
        // The record says it was issued at 1_000_000 ms; the certificate claims a far later end.
        let claimed_end = i64::MAX / 4;
        let now = 1_000_000 + ca.lifetime_ms();
        assert_eq!(
            refusal(e.renew(&ca, &from(&serial, "a", claimed_end), &csr, now)),
            "too_old"
        );
        // One millisecond earlier it is still inside the lifetime.
        assert!(e
            .renew(&ca, &from(&serial, "a", claimed_end), &csr, now - 1)
            .is_ok());
    }

    #[test]
    fn at_most_one_certificate_an_hour_per_machine() {
        let (store, ca, _) = setup();
        let e = Enrollment::new(&store);
        let (serial, end) = enrolled(&store, &ca, "a", 1_000_000);
        let (_, csr) = new_device_request().unwrap();
        // Enrollment itself counts: a machine that has just enrolled cannot renew within the hour.
        assert_eq!(
            refusal(e.renew(&ca, &from(&serial, "a", end), &csr, 1_000_000 + HOUR - 1)),
            "too_frequent"
        );
        let first = e
            .renew(&ca, &from(&serial, "a", end), &csr, 1_000_000 + HOUR)
            .unwrap();
        // A second one within the hour of the first is refused, from either certificate.
        let (_, csr2) = new_device_request().unwrap();
        for who in [serial.as_str(), first.serial_hex.as_str()] {
            assert_eq!(
                refusal(e.renew(&ca, &from(who, "a", end), &csr2, 1_000_000 + 2 * HOUR - 1)),
                "too_frequent"
            );
        }
        assert!(e
            .renew(&ca, &from(&serial, "a", end), &csr2, 1_000_000 + 2 * HOUR)
            .is_ok());
        // Another machine's certificates are not counted.
        let (other, other_end) = enrolled(&store, &ca, "b", 1_000_000 + HOUR);
        assert!(e
            .renew(
                &ca,
                &from(&other, "b", other_end),
                &csr2,
                1_000_000 + 2 * HOUR
            )
            .is_ok());
    }

    /// `(common name, number of alternative names, extended key usages as text, public key bytes, is a CA)` of a certificate.
    fn facts(pem: &str) -> (String, usize, String, Vec<u8>, bool) {
        let der = x509_parser::pem::parse_x509_pem(pem.as_bytes()).unwrap().1;
        let c = der.parse_x509().unwrap();
        let cn = c
            .subject()
            .iter_common_name()
            .next()
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        let sans = c
            .subject_alternative_name()
            .unwrap()
            .map_or(0, |s| s.value.general_names.len());
        let eku = format!("{:?}", c.extended_key_usage().unwrap().unwrap().value);
        (
            cn,
            sans,
            eku,
            c.public_key().subject_public_key.data.to_vec(),
            c.is_ca(),
        )
    }

    #[test]
    fn a_renewal_keeps_the_node_id_and_the_usages_and_takes_the_key_of_the_csr() {
        let (store, ca, _) = setup();
        let e = Enrollment::new(&store);
        let (first_serial, end) = enrolled(&store, &ca, "laptop", 1_000_000);
        let first_pem = {
            // The first certificate's text is not kept by the hub; enroll again under another name to compare usages.
            let mut en = Enrollment::new(&store);
            let code = en.create_code(1_000_000).unwrap();
            let (_, c) = new_device_request().unwrap();
            en.redeem(&ca, &code, "reference", &c, 1_000_000)
                .unwrap()
                .cert_pem
        };
        let (_, csr) = new_device_request().unwrap();
        let renewed = e
            .renew(
                &ca,
                &from(&first_serial, "laptop", end),
                &csr,
                later(1_000_000),
            )
            .unwrap();
        assert_ne!(renewed.serial_hex, first_serial);
        let (a, b) = (facts(&first_pem), facts(&renewed.cert_pem));
        assert_eq!((a.1, &a.2, a.4), (b.1, &b.2, b.4));
        assert_eq!(b.0, "laptop");
        assert!(
            b.2.contains("client_auth: true") && !b.2.contains("server_auth: true"),
            "{}",
            b.2
        );
        assert!(!b.4, "a machine certificate is never a CA");
        // The key is the one the new CSR carries.
        let csr_der = x509_parser::pem::parse_x509_pem(csr.as_bytes()).unwrap().1;
        use x509_parser::prelude::FromDer;
        let csr_key = x509_parser::certification_request::X509CertificationRequest::from_der(
            &csr_der.contents,
        )
        .unwrap()
        .1
        .certification_request_info
        .subject_pki
        .subject_public_key
        .data
        .to_vec();
        assert_eq!(b.3, csr_key);
        assert_ne!(a.3, b.3);
        // The new one is listed as active and the old one stays valid (grace): nothing is revoked at renewal.
        assert!(e.is_active(&renewed.serial_hex).unwrap());
        assert!(e.is_active(&first_serial).unwrap());
    }

    #[test]
    fn a_name_or_extension_in_the_csr_is_ignored_in_a_renewal_as_at_enrollment() {
        use rcgen::{
            BasicConstraints, CertificateParams, DistinguishedName, DnType,
            ExtendedKeyUsagePurpose, IsCa, KeyPair,
        };
        let (store, ca, _) = setup();
        let e = Enrollment::new(&store);
        let (serial, end) = enrolled(&store, &ca, "laptop", 1_000_000);
        // A request asking to be "admin", with a DNS name, server usage and CA rights.
        let key = KeyPair::generate().unwrap();
        let mut p = CertificateParams::new(vec!["evil.example".to_string()]).unwrap();
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "admin");
        p.distinguished_name = dn;
        p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let csr = p.serialize_request(&key).unwrap().pem().unwrap();
        let renewed = e
            .renew(&ca, &from(&serial, "laptop", end), &csr, later(1_000_000))
            .unwrap();
        let (cn, sans, eku, _, is_ca) = facts(&renewed.cert_pem);
        assert_eq!(cn, "laptop");
        assert_eq!(sans, 0);
        assert!(
            eku.contains("client_auth: true") && !eku.contains("server_auth: true"),
            "{eku}"
        );
        assert!(!is_ca);
    }

    #[test]
    fn a_renewal_with_a_bad_csr_is_a_ca_error_and_issues_nothing() {
        let (store, ca, _) = setup();
        let e = Enrollment::new(&store);
        let (serial, end) = enrolled(&store, &ca, "a", 1_000_000);
        assert!(matches!(
            e.renew(&ca, &from(&serial, "a", end), "not a csr", later(1_000_000)),
            Err(RenewError::Ca(_))
        ));
        assert_eq!(device_rows(&store), 1);
    }

    #[test]
    fn a_renewal_writes_an_audit_row_with_the_serial_the_node_and_the_time() {
        let (store, ca, _) = setup();
        let e = Enrollment::new(&store);
        let (serial, end) = enrolled(&store, &ca, "laptop", 1_000_000);
        let (_, csr) = new_device_request().unwrap();
        let now = later(1_000_000);
        let renewed = e
            .renew(&ca, &from(&serial, "laptop", end), &csr, now)
            .unwrap();
        let rows = crate::audit::Audit::new(&store).list(0, None, 10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (
                rows[0].action.as_str(),
                rows[0].node.as_str(),
                rows[0].at_ms
            ),
            ("enroll.renew", "laptop", now)
        );
        assert_eq!(rows[0].outcome, format!("ok serial={}", renewed.serial_hex));
        // A refused renewal writes none.
        let (_, csr2) = new_device_request().unwrap();
        assert!(e
            .renew(&ca, &from(&serial, "laptop", end), &csr2, now + 1)
            .is_err());
        assert_eq!(
            crate::audit::Audit::new(&store)
                .list(0, None, 10)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn older_certificates_are_revoked_a_week_after_the_machine_first_connects_with_a_newer_one() {
        let (store, ca, _) = setup();
        let e = Enrollment::new(&store);
        let (old, end) = enrolled(&store, &ca, "laptop", 1_000_000);
        let (_, csr) = new_device_request().unwrap();
        let t0 = later(1_000_000);
        let new = e
            .renew(&ca, &from(&old, "laptop", end), &csr, t0)
            .unwrap()
            .serial_hex;
        // Connecting with the old one starts nothing; the new one has not connected yet.
        e.note_connected(&old, t0 + 1).unwrap();
        assert_eq!(e.revoke_superseded(t0 + 30 * DAY).unwrap(), 0);
        assert!(e.is_active(&old).unwrap());
        // The machine first connects with the new one at t1; the old one lives on for a week less a moment...
        let t1 = t0 + 2 * HOUR;
        e.note_connected(&new, t1).unwrap();
        // ...and a later connection with it does not move the start.
        e.note_connected(&new, t1 + 3 * DAY).unwrap();
        assert!(e.is_active(&old).unwrap());
        assert_eq!(
            e.revoke_superseded(t1 + SUPERSEDED_GRACE_MS - 1).unwrap(),
            0
        );
        assert!(e.is_active(&old).unwrap());
        assert_eq!(e.revoke_superseded(t1 + SUPERSEDED_GRACE_MS).unwrap(), 1);
        assert!(!e.is_active(&old).unwrap());
        assert!(
            e.is_active(&new).unwrap(),
            "the newest certificate is never revoked by this"
        );
        // Another machine's certificates are untouched.
        let (other, _) = enrolled(&store, &ca, "desk", 1_000_000);
        assert_eq!(e.revoke_superseded(t1 + 100 * DAY).unwrap(), 0);
        assert!(e.is_active(&other).unwrap());
    }
}
