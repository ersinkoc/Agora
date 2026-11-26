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
/// ADR-0085: a refused renewal is written to the audit trail at most once per machine per reason in this long.
pub const REFUSAL_AUDIT_GAP_MS: u64 = 60 * 60 * 1000;
/// ADR-0085: how many times an hour a machine may have the same renewal answered again (its answer was lost).
pub const RESENDS_PER_HOUR: i64 = 5;
const RESEND_WINDOW_MS: u64 = 60 * 60 * 1000;

/// What [`Enrollment::renew`] decided.
enum Decision {
    Refuse(RenewRefusal),
    /// The same certificate again (the request is the one it was issued for, and it never connected).
    Resend(IssuedCert),
    Issue,
}

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
    /// The certificate is valid but a newer one of the machine exists: only the newest may renew.
    Superseded,
}

impl RenewRefusal {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Revoked => "revoked",
            Self::Expired => "expired",
            Self::TooOld => "too_old",
            Self::TooFrequent => "too_frequent",
            Self::Superseded => "superseded",
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
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.1 != 0 && now_ms >= s.1 {
            s.1 = 0;
        }
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
        let tx = crate::store::write_tx(&mut conn)?;
        let code_hash = hash(code);
        let live: Option<i64> = tx
            .query_row(
                "SELECT expires_ms FROM enrollment_codes WHERE code_hash = ?1",
                [&code_hash],
                |r| r.get(0),
            )
            .optional()?;
        let Some(expires_ms) = live else {
            drop(tx);
            drop(conn);
            self.lockout.failed(now_ms);
            return Err(EnrollError::BadCode);
        };
        if expires_ms <= to_i64(now_ms) {
            tx.execute(
                "DELETE FROM enrollment_codes WHERE code_hash = ?1",
                [&code_hash],
            )?;
            tx.commit()?;
            drop(conn);
            self.lockout.failed(now_ms);
            return Err(EnrollError::BadCode);
        }
        let issued = ca.issue_node(node_id, csr_pem)?;
        tx.execute(
            "DELETE FROM enrollment_codes WHERE code_hash = ?1",
            [&code_hash],
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
        let tx = crate::store::write_tx(&mut conn)?;
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
    /// the same usages and a fresh key (the CSR's key; its name, extensions and usages are ignored, as at enrollment), records it (with
    /// its PEM and the hash of the CSR's key) and an audit row in one transaction, and revokes nothing: the previous certificate stays
    /// valid until its own end (see [`Self::revoke_superseded`]).
    ///
    /// Only the machine's **newest** active certificate may renew. An older one, still valid during its grace, may connect but is refused
    /// as `superseded`: otherwise a copy of an old certificate and key could renew hourly, become the newest, and have the real machine's
    /// certificate revoked a week later. A refusal is an audit row too (see [`REFUSAL_AUDIT_GAP_MS`]).
    ///
    /// **A renewal whose answer was lost** (the machine crashed or timed out after the hub issued) is answered again, idempotently: when
    /// the newest active certificate B was issued by a renewal, has never connected, and the request carries EXACTLY the public key of the
    /// request B was issued for, the hub returns B again (same serial; nothing new issued or revoked; an `enroll.renew_resent` audit row;
    /// at most [`RESENDS_PER_HOUR`] an hour per machine and outside the hour limit for new certificates). This gives a thief nothing: B's
    /// private key never left the machine, so a thief holding only an older certificate cannot send that key, and a different request from
    /// an older certificate is still `superseded`. Once B has connected it is no longer offered. The machine must keep the key and request
    /// of a renewal until it has stored the answer.
    pub fn renew(
        &self,
        ca: &Ca,
        from: &RenewFrom<'_>,
        csr_pem: &str,
        now_ms: u64,
    ) -> Result<IssuedCert, RenewError> {
        let mut conn = self.store.lock()?;
        let tx = crate::store::write_tx(&mut conn)?;
        match Self::decide(&tx, ca, from, csr_pem, now_ms)? {
            Decision::Refuse(why) => {
                Self::audit_refusal(&tx, from, why, now_ms)?;
                tx.commit()?;
                Err(RenewError::Refused(why))
            }
            Decision::Resend(again) => {
                tx.execute(
                    "INSERT INTO audit (at_ms, who, session, action, node, outcome) VALUES (?1, ?2, '', 'enroll.renew_resent', ?2, ?3)",
                    params![
                        to_i64(now_ms),
                        from.node_id,
                        format!("presented={} serial={}", from.serial_hex, again.serial_hex)
                    ],
                )?;
                tx.commit()?;
                Ok(again)
            }
            Decision::Issue => {
                let key_hash = crate::ca::csr_key_hash(csr_pem)?;
                let issued = ca.issue_node(from.node_id, csr_pem)?;
                tx.execute(
                    "INSERT INTO devices (serial, node_id, enrolled_ms, cert_pem, csr_key_hash) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        issued.serial_hex,
                        from.node_id,
                        to_i64(now_ms),
                        issued.cert_pem,
                        key_hash.to_vec()
                    ],
                )?;
                tx.execute(
                    "INSERT INTO audit (at_ms, who, session, action, node, outcome) VALUES (?1, ?2, '', 'enroll.renew', ?2, ?3)",
                    params![
                        to_i64(now_ms),
                        from.node_id,
                        format!("ok serial={} presented={}", issued.serial_hex, from.serial_hex)
                    ],
                )?;
                tx.commit()?;
                Ok(issued)
            }
        }
    }

    /// What a renewal request gets: refused, answered again with a certificate already issued, or a new certificate. Checks, in order:
    /// unknown, revoked, expired, too_old; then which certificate asks (the newest, or an older one that may only be answered again, else
    /// `superseded`); then the hour limit for a new certificate.
    fn decide(
        tx: &rusqlite::Transaction<'_>,
        ca: &Ca,
        from: &RenewFrom<'_>,
        csr_pem: &str,
        now_ms: u64,
    ) -> Result<Decision, RenewError> {
        let row: Option<(String, i64, Option<i64>)> = tx
            .query_row(
                "SELECT node_id, enrolled_ms, revoked_ms FROM devices WHERE serial = ?1",
                [from.serial_hex],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        // A serial the hub never issued, or one issued to another machine than the certificate names: nothing to renew.
        let Some((_, issued_ms, revoked)) = row.filter(|(n, _, _)| n == from.node_id) else {
            return Ok(Decision::Refuse(RenewRefusal::Unknown));
        };
        if revoked.is_some() {
            return Ok(Decision::Refuse(RenewRefusal::Revoked));
        }
        let now_s = i64::try_from(now_ms / 1000).unwrap_or(i64::MAX);
        if from.not_after_secs <= now_s {
            return Ok(Decision::Refuse(RenewRefusal::Expired));
        }
        // The hub's own record of when it issued the certificate. Distinct from `Expired`, which trusts the end the certificate
        // carries: this one covers a certificate whose end is later than the hub's current lifetime allows (the lifetime was lowered
        // after it was issued, or the certificate was issued under a longer one), so such a certificate cannot be prolonged further
        // than one current lifetime from when the hub issued it.
        if issued_ms.saturating_add(to_i64(ca.lifetime_ms())) <= to_i64(now_ms) {
            return Ok(Decision::Refuse(RenewRefusal::TooOld));
        }
        // The machine's newest active certificate: its serial, whether it has connected, and what a renewal stored for it.
        type Newest = (String, bool, Option<String>, Option<Vec<u8>>);
        let newest: Option<Newest> = tx
            .query_row(
                "SELECT serial, first_seen_ms IS NOT NULL, cert_pem, csr_key_hash FROM devices WHERE node_id = ?1 AND revoked_ms IS NULL
                 ORDER BY enrolled_ms DESC, rowid DESC LIMIT 1",
                [from.node_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let Some((newest_serial, connected, newest_pem, newest_key)) = newest else {
            return Ok(Decision::Refuse(RenewRefusal::Superseded));
        };
        if newest_serial != from.serial_hex {
            // An older certificate. It gets nothing new; it can only be answered again, with the certificate the machine's own key
            // (which the older certificate does not give anyone) was issued for.
            let same_request = match (&newest_pem, &newest_key, connected) {
                (Some(_), Some(stored), false) => crate::ca::csr_key_hash(csr_pem)
                    .is_ok_and(|h| h.as_slice() == stored.as_slice()),
                _ => false,
            };
            let Some(pem) = newest_pem.filter(|_| same_request) else {
                return Ok(Decision::Refuse(RenewRefusal::Superseded));
            };
            // A certificate that has itself ended is not sent again: the machine could not use it.
            if crate::ca::cert_not_after_secs(&pem).map_or(true, |end| end <= now_s) {
                return Ok(Decision::Refuse(RenewRefusal::Expired));
            }
            let recent: i64 = tx.query_row(
                "SELECT COUNT(*) FROM audit WHERE action = 'enroll.renew_resent' AND node = ?1 AND at_ms > ?2",
                params![from.node_id, to_i64(now_ms.saturating_sub(RESEND_WINDOW_MS))],
                |r| r.get(0),
            )?;
            if recent >= RESENDS_PER_HOUR {
                return Ok(Decision::Refuse(RenewRefusal::TooFrequent));
            }
            return Ok(Decision::Resend(IssuedCert {
                cert_pem: pem,
                serial_hex: newest_serial,
            }));
        }
        // One new certificate per machine per hour, counting every certificate the hub issued for it (enrollment included). Only the
        // newest certificate gets here, so a copy of an older one cannot use up the real machine's hour.
        let latest: i64 = tx.query_row(
            "SELECT MAX(enrolled_ms) FROM devices WHERE node_id = ?1",
            [from.node_id],
            |r| r.get(0),
        )?;
        if latest.saturating_add(to_i64(RENEW_MIN_GAP_MS)) > to_i64(now_ms) {
            return Ok(Decision::Refuse(RenewRefusal::TooFrequent));
        }
        Ok(Decision::Issue)
    }

    /// Records a refused renewal as `enroll.renew_refused` (node, reason, presenting serial), at most once per node per reason per
    /// [`REFUSAL_AUDIT_GAP_MS`], so a flood of refusals cannot grow the audit table without bound.
    fn audit_refusal(
        tx: &rusqlite::Transaction<'_>,
        from: &RenewFrom<'_>,
        why: RenewRefusal,
        now_ms: u64,
    ) -> Result<(), RenewError> {
        let prefix = format!("reason={} ", why.reason());
        let recent: bool = tx
            .query_row(
                "SELECT 1 FROM audit WHERE action = 'enroll.renew_refused' AND node = ?1 AND at_ms > ?2
                 AND substr(outcome, 1, ?3) = ?4 LIMIT 1",
                params![
                    from.node_id,
                    to_i64(now_ms.saturating_sub(REFUSAL_AUDIT_GAP_MS)),
                    i64::try_from(prefix.len()).unwrap_or(0),
                    prefix
                ],
                |_| Ok(true),
            )
            .optional()?
            .is_some();
        if !recent {
            tx.execute(
                "INSERT INTO audit (at_ms, who, session, action, node, outcome) VALUES (?1, ?2, '', 'enroll.renew_refused', ?2, ?3)",
                params![
                    to_i64(now_ms),
                    from.node_id,
                    format!("{prefix}presented={}", from.serial_hex)
                ],
            )?;
        }
        Ok(())
    }

    /// ADR-0085 grace: revokes every certificate that a newer one of the same machine has replaced, once the machine first connected
    /// with that newer one at least [`SUPERSEDED_GRACE_MS`] ago. Returns how many it revoked. A connection still open on a revoked
    /// certificate is closed by the usual check. This sweeps every machine; the hub runs it daily, and a connection only applies it to
    /// its own machine ([`Self::note_connected`]). It trusts the hub's clock: a clock that jumps forward can revoke older certificates
    /// early (bounded: only certificates a newer one already replaced and that connected), and one that jumps back only delays it.
    pub fn revoke_superseded(&self, now_ms: u64) -> Result<usize, EnrollError> {
        self.revoke_superseded_where(None, now_ms)
    }

    fn revoke_superseded_where(
        &self,
        node_id: Option<&str>,
        now_ms: u64,
    ) -> Result<usize, EnrollError> {
        let conn = self.store.lock()?;
        let n = conn.execute(
            "UPDATE devices SET revoked_ms = ?1 WHERE revoked_ms IS NULL AND (?3 IS NULL OR node_id = ?3) AND EXISTS (
                 SELECT 1 FROM devices n WHERE n.node_id = devices.node_id AND n.enrolled_ms > devices.enrolled_ms
                 AND n.revoked_ms IS NULL AND n.first_seen_ms IS NOT NULL AND n.first_seen_ms + ?2 <= ?1)",
            params![to_i64(now_ms), to_i64(SUPERSEDED_GRACE_MS), node_id],
        )?;
        Ok(n)
    }

    /// Notes that a machine connected with this certificate (the first time only), then applies the superseded-certificate revocation
    /// to that machine alone (the global sweep is the daily one).
    pub fn note_connected(
        &self,
        node_id: &str,
        serial_hex: &str,
        now_ms: u64,
    ) -> Result<(), EnrollError> {
        {
            let conn = self.store.lock()?;
            conn.execute(
                "UPDATE devices SET first_seen_ms = ?2 WHERE serial = ?1 AND node_id = ?3 AND first_seen_ms IS NULL",
                params![serial_hex, to_i64(now_ms), node_id],
            )?;
        }
        self.revoke_superseded_where(Some(node_id), now_ms)?;
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
    fn f387_an_expired_enrollment_code_cannot_return_after_clock_rollback() {
        let (control_store, control_ca, control_csr) = setup();
        let mut control = Enrollment::new(&control_store);
        let fresh = control.create_code(0).unwrap();
        assert!(control
            .redeem(
                &control_ca,
                &fresh,
                "control",
                &control_csr,
                CODE_TTL_MS - 1
            )
            .is_ok());
        println!("CONTROL OK");

        let (store, ca, csr) = setup();
        let mut enrollment = Enrollment::new(&store);
        let code = enrollment.create_code(0).unwrap();
        assert!(matches!(
            enrollment.redeem(&ca, &code, "target", &csr, CODE_TTL_MS),
            Err(EnrollError::BadCode)
        ));
        let actual = enrollment.redeem(&ca, &code, "target", &csr, 1).map(|_| ());
        println!("EXPECTED: Err(BadCode)");
        println!("ACTUAL: {actual:?}");
        if matches!(actual, Err(EnrollError::BadCode)) {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert!(matches!(actual, Err(EnrollError::BadCode)));
    }

    #[test]
    fn verifies_f387_enrollment_code_expiry_boundaries() {
        let (fresh_store, fresh_ca, fresh_csr) = setup();
        let mut fresh = Enrollment::new(&fresh_store);
        let code = fresh.create_code(0).unwrap();
        assert!(fresh
            .redeem(&fresh_ca, &code, "fresh", &fresh_csr, CODE_TTL_MS - 1)
            .is_ok());

        let (exact_store, exact_ca, exact_csr) = setup();
        let mut exact = Enrollment::new(&exact_store);
        let code = exact.create_code(0).unwrap();
        assert!(matches!(
            exact.redeem(&exact_ca, &code, "exact", &exact_csr, CODE_TTL_MS),
            Err(EnrollError::BadCode)
        ));
        assert!(matches!(
            exact.redeem(&exact_ca, &code, "exact", &exact_csr, 1),
            Err(EnrollError::BadCode)
        ));

        let (late_store, late_ca, late_csr) = setup();
        let mut late = Enrollment::new(&late_store);
        let code = late.create_code(0).unwrap();
        assert!(matches!(
            late.redeem(&late_ca, &code, "late", &late_csr, CODE_TTL_MS + 500),
            Err(EnrollError::BadCode)
        ));
        assert!(matches!(
            late.redeem(&late_ca, &code, "late", &late_csr, 2),
            Err(EnrollError::BadCode)
        ));
        assert!(matches!(
            late.redeem(&late_ca, "unknown", "late", &late_csr, 3),
            Err(EnrollError::BadCode)
        ));
        println!("FIX VERIFIED");
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
    fn f382_an_ended_enrollment_lockout_does_not_return_after_clock_rollback() {
        let lockout = Lockout::new();
        for _ in 0..MAX_FAILURES {
            lockout.failed(0);
        }
        assert_eq!(lockout.check(1), Some(LOCKOUT_MS - 1));
        println!("CONTROL OK");

        assert_eq!(lockout.check(LOCKOUT_MS + 1), None);
        let actual = lockout.check(1);
        println!("EXPECTED: None");
        println!("ACTUAL: {actual:?}");
        if actual.is_none() {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert_eq!(actual, None);
    }

    #[test]
    fn verifies_f382_enrollment_lockout_deadline_boundaries() {
        let locked = || {
            let lockout = Lockout::new();
            for _ in 0..MAX_FAILURES {
                lockout.failed(10);
            }
            lockout
        };

        let exact = locked();
        assert_eq!(exact.check(10 + LOCKOUT_MS - 1), Some(1));
        assert_eq!(exact.check(10 + LOCKOUT_MS), None);
        assert_eq!(exact.check(10), None);

        let later = locked();
        assert_eq!(later.check(10 + LOCKOUT_MS + 500), None);
        assert_eq!(later.check(11), None);

        println!("FIX VERIFIED");
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
        // A second one within the hour of the first is refused for the newest certificate...
        let (_, csr2) = new_device_request().unwrap();
        assert_eq!(
            refusal(e.renew(
                &ca,
                &from(&first.serial_hex, "a", end),
                &csr2,
                1_000_000 + 2 * HOUR - 1
            )),
            "too_frequent"
        );
        assert!(e
            .renew(
                &ca,
                &from(&first.serial_hex, "a", end),
                &csr2,
                1_000_000 + 2 * HOUR
            )
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

    /// The security finding of the review of ADR-0085: a copy of an older certificate (still valid during its grace) must not be able to
    /// renew, or it could become the newest and have the real machine's certificate revoked a week later.
    #[test]
    fn only_the_newest_certificate_of_a_machine_may_renew() {
        let (store, ca, _) = setup();
        let e = Enrollment::new(&store);
        let (old, end) = enrolled(&store, &ca, "a", 1_000_000);
        let (_, csr) = new_device_request().unwrap();
        let t = 1_000_000 + HOUR;
        let newest = e.renew(&ca, &from(&old, "a", end), &csr, t).unwrap();
        // The machine moved to the newest certificate: it has connected with it (otherwise it would be the lost-answer case).
        e.note_connected("a", &newest.serial_hex, t + 1).unwrap();
        // The old one is still active (grace) and may still connect, but it may not renew, now or later.
        assert!(e.is_active(&old).unwrap());
        for later in [t + 1, t + HOUR, t + 5 * HOUR] {
            assert_eq!(
                refusal(e.renew(&ca, &from(&old, "a", end), &csr, later)),
                "superseded"
            );
        }
        // A flood from the old serial does not use up the real machine's hour or turn it into anything: after the hour the newest renews.
        for i in 0..50 {
            assert_eq!(
                refusal(e.renew(&ca, &from(&old, "a", end), &csr, t + 2 * HOUR + i)),
                "superseded"
            );
        }
        let third = e
            .renew(
                &ca,
                &from(&newest.serial_hex, "a", end),
                &csr,
                t + 2 * HOUR + 100,
            )
            .unwrap();
        assert!(e.is_active(&third.serial_hex).unwrap());
        e.note_connected("a", &third.serial_hex, t + 2 * HOUR + 101)
            .unwrap();
        // The newest is now the third; the second is superseded as well.
        assert_eq!(
            refusal(e.renew(&ca, &from(&newest.serial_hex, "a", end), &csr, t + 4 * HOUR)),
            "superseded"
        );
        // A revoked newest leaves the newest *active* one as the only that may renew.
        e.revoke("a", t + 5 * HOUR).unwrap();
        assert_eq!(
            refusal(e.renew(&ca, &from(&third.serial_hex, "a", end), &csr, t + 6 * HOUR)),
            "revoked"
        );
    }

    fn refusal_rows(store: &SqliteStore) -> Vec<crate::audit::Entry> {
        crate::audit::Audit::new(store)
            .list(0, None, 1000)
            .unwrap()
            .into_iter()
            .filter(|r| r.action == "enroll.renew_refused")
            .collect()
    }

    #[test]
    fn refusals_are_audited_with_the_reason_and_the_presenting_serial_at_most_once_an_hour_per_reason(
    ) {
        let (store, ca, _) = setup();
        let e = Enrollment::new(&store);
        let (old, end) = enrolled(&store, &ca, "a", 1_000_000);
        let (_, csr) = new_device_request().unwrap();
        let t = 1_000_000 + HOUR;
        let newest = e.renew(&ca, &from(&old, "a", end), &csr, t).unwrap();
        e.note_connected("a", &newest.serial_hex, t + 1).unwrap();
        // 100 refusals of the same kind in an hour: one row, with the node, reason and the serial that was presented.
        for i in 0..100 {
            let _ = e.renew(&ca, &from(&old, "a", end), &csr, t + 1 + i);
        }
        let rows = refusal_rows(&store);
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].node.as_str(), rows[0].who.as_str()), ("a", "a"));
        assert_eq!(
            rows[0].outcome,
            format!("reason=superseded presented={old}")
        );
        // Another reason is its own row (the newest asking too early)...
        assert_eq!(
            refusal(e.renew(&ca, &from(&newest.serial_hex, "a", end), &csr, t + 200)),
            "too_frequent"
        );
        assert_eq!(refusal_rows(&store).len(), 2);
        // ...another machine's refusal is its own row...
        let (b, b_end) = enrolled(&store, &ca, "b", 1_000_000);
        assert_eq!(
            refusal(e.renew(&ca, &from(&b, "b", b_end), &csr, 1_000_000 + 10)),
            "too_frequent"
        );
        assert_eq!(refusal_rows(&store).len(), 3);
        // ...and an hour later the same refusal is written again.
        assert_eq!(
            refusal(e.renew(&ca, &from(&old, "a", end), &csr, t + 1 + HOUR + 5)),
            "superseded"
        );
        assert_eq!(refusal_rows(&store).len(), 4);
        // A refused renewal issues nothing: the machine has exactly the certificates it had.
        assert_eq!(device_rows(&store), 3);
    }

    /// A request whose signature was not made by the key it carries is refused (proof of possession): a certificate must never be issued
    /// for a public key its asker cannot sign with.
    #[test]
    fn a_csr_not_signed_by_its_own_key_is_refused_and_issues_nothing() {
        let (store, ca, _) = setup();
        let e = Enrollment::new(&store);
        let (serial, end) = enrolled(&store, &ca, "a", 1_000_000);
        let (_, good) = new_device_request().unwrap();
        // Swap the public key of one request for another's, keeping the first one's signature: the bytes of key and signature are
        // both fixed size for P-256, so the request stays well formed.
        let (_, other) = new_device_request().unwrap();
        let (mut a_der, b_der) = (
            x509_parser::pem::parse_x509_pem(good.as_bytes())
                .unwrap()
                .1
                .contents,
            x509_parser::pem::parse_x509_pem(other.as_bytes())
                .unwrap()
                .1
                .contents,
        );
        use x509_parser::prelude::FromDer;
        let key_of = |der: &[u8]| {
            x509_parser::certification_request::X509CertificationRequest::from_der(der)
                .unwrap()
                .1
                .certification_request_info
                .subject_pki
                .subject_public_key
                .data
                .to_vec()
        };
        let (ka, kb) = (key_of(&a_der), key_of(&b_der));
        assert_eq!(ka.len(), kb.len());
        let at = a_der
            .windows(ka.len())
            .position(|w| w == ka.as_slice())
            .unwrap();
        a_der[at..at + ka.len()].copy_from_slice(&kb);
        let forged = pem_of(&a_der);
        assert!(
            matches!(
                e.renew(&ca, &from(&serial, "a", end), &forged, 1_000_000 + 2 * HOUR),
                Err(RenewError::Ca(_))
            ),
            "a request signed by another key was accepted"
        );
        assert_eq!(device_rows(&store), 1);
        // The untouched request is accepted, so the refusal was about the signature.
        assert!(e
            .renew(&ca, &from(&serial, "a", end), &good, 1_000_000 + 2 * HOUR)
            .is_ok());
    }

    fn pem_of(der: &[u8]) -> String {
        use std::fmt::Write;
        const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        let mut line = 0;
        let mut text = String::from("-----BEGIN CERTIFICATE REQUEST-----\n");
        for chunk in der.chunks(3) {
            let n = (u32::from(chunk[0]) << 16)
                | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
                | u32::from(*chunk.get(2).unwrap_or(&0));
            for i in 0..4 {
                if i <= chunk.len() {
                    out.push(char::from(B64[((n >> (18 - 6 * i)) & 63) as usize]));
                } else {
                    out.push('=');
                }
                line += 1;
                if line == 64 {
                    out.push('\n');
                    line = 0;
                }
            }
        }
        write!(text, "{out}").unwrap();
        if line != 0 {
            text.push('\n');
        }
        text.push_str("-----END CERTIFICATE REQUEST-----\n");
        text
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
        assert_eq!(
            rows[0].outcome,
            format!("ok serial={} presented={serial}", renewed.serial_hex)
        );
        // A refused renewal writes a row of its own kind, never an `enroll.renew` one.
        let (_, csr2) = new_device_request().unwrap();
        assert!(e
            .renew(&ca, &from(&serial, "laptop", end), &csr2, now + 1)
            .is_err());
        let all = crate::audit::Audit::new(&store).list(0, None, 10).unwrap();
        assert_eq!(all.iter().filter(|r| r.action == "enroll.renew").count(), 1);
        assert_eq!(refusal_rows(&store).len(), 1);
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
        e.note_connected("laptop", &old, t0 + 1).unwrap();
        assert_eq!(e.revoke_superseded(t0 + 30 * DAY).unwrap(), 0);
        assert!(e.is_active(&old).unwrap());
        // The machine first connects with the new one at t1; the old one lives on for a week less a moment...
        let t1 = t0 + 2 * HOUR;
        e.note_connected("laptop", &new, t1).unwrap();
        // ...and a later connection with it does not move the start.
        e.note_connected("laptop", &new, t1 + 3 * DAY).unwrap();
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

    fn is_revoked(store: &SqliteStore, serial: &str) -> bool {
        store
            .lock()
            .unwrap()
            .query_row(
                "SELECT revoked_ms IS NOT NULL FROM devices WHERE serial = ?1",
                [serial],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn active_count(store: &SqliteStore, node: &str) -> i64 {
        store
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM devices WHERE node_id = ?1 AND revoked_ms IS NULL",
                [node],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn resent_rows(store: &SqliteStore) -> Vec<crate::audit::Entry> {
        crate::audit::Audit::new(store)
            .list(0, None, 1000)
            .unwrap()
            .into_iter()
            .filter(|r| r.action == "enroll.renew_resent")
            .collect()
    }

    /// (1) A renewal whose answer was lost is answered again with the same certificate when the same request comes from the machine's
    /// previous certificate: nothing new is issued, nothing is revoked, and the hour limit for new certificates does not apply.
    #[test]
    fn a_lost_answer_is_answered_again_with_the_same_certificate() {
        let (store, ca, _) = setup();
        let e = Enrollment::new(&store);
        let (a, end) = enrolled(&store, &ca, "n", 1_000_000);
        let (_, csr) = new_device_request().unwrap();
        let t = 1_000_000 + HOUR;
        let b = e.renew(&ca, &from(&a, "n", end), &csr, t).unwrap();
        // The answer is lost; the machine asks again, with the same request, within the hour.
        let again = e.renew(&ca, &from(&a, "n", end), &csr, t + 60_000).unwrap();
        assert_eq!(again.serial_hex, b.serial_hex);
        assert_eq!(again.cert_pem, b.cert_pem);
        // Nothing new: the machine has its first and the one renewal, both still active, and a third ask is the same again.
        assert_eq!(device_rows(&store), 2);
        assert_eq!(active_count(&store, "n"), 2);
        assert!(!is_revoked(&store, &a) && !is_revoked(&store, &b.serial_hex));
        let third = e
            .renew(&ca, &from(&a, "n", end), &csr, t + 120_000)
            .unwrap();
        assert_eq!(third.serial_hex, b.serial_hex);
        // One audit row of the renewal, one per resend (with the presenting and the resent serial).
        let rows = resent_rows(&store);
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0].outcome,
            format!("presented={a} serial={}", b.serial_hex)
        );
        let all = crate::audit::Audit::new(&store).list(0, None, 100).unwrap();
        assert_eq!(all.iter().filter(|r| r.action == "enroll.renew").count(), 1);
        assert!(all.iter().all(|r| !r.outcome.contains("lost_answer")));
    }

    /// (2) A thief holding only an older certificate: a different request is `superseded`, the never-connected newest certificate is
    /// untouched (not revoked, nothing issued), and the refusal is an audit row.
    #[test]
    fn a_thief_with_an_older_certificate_gets_nothing_while_the_newest_has_never_connected() {
        let (store, ca, _) = setup();
        let e = Enrollment::new(&store);
        let (a, end) = enrolled(&store, &ca, "n", 1_000_000);
        let (_, csr) = new_device_request().unwrap();
        let t = 1_000_000 + HOUR;
        let b = e.renew(&ca, &from(&a, "n", end), &csr, t).unwrap();
        let (_, thief_csr) = new_device_request().unwrap();
        for i in 0..20 {
            assert_eq!(
                refusal(e.renew(&ca, &from(&a, "n", end), &thief_csr, t + HOUR * 2 + i)),
                "superseded"
            );
        }
        assert!(!is_revoked(&store, &b.serial_hex));
        assert_eq!(device_rows(&store), 2);
        assert!(resent_rows(&store).is_empty());
        assert_eq!(refusal_rows(&store).len(), 1);
        // A request that does not parse is just as refused.
        assert_eq!(
            refusal(e.renew(&ca, &from(&a, "n", end), "not a csr", t + HOUR * 2)),
            "superseded"
        );
        // And the machine, with its own key, still gets its certificate.
        assert_eq!(
            e.renew(&ca, &from(&a, "n", end), &csr, t + HOUR * 2)
                .unwrap()
                .serial_hex,
            b.serial_hex
        );
    }

    /// (3) Once the newest certificate has connected it is no longer resent, and the older one is superseded.
    #[test]
    fn a_resend_is_not_offered_after_the_newest_certificate_connects() {
        let (store, ca, _) = setup();
        let e = Enrollment::new(&store);
        let (a, end) = enrolled(&store, &ca, "n", 1_000_000);
        let (_, csr) = new_device_request().unwrap();
        let t = 1_000_000 + HOUR;
        let b = e.renew(&ca, &from(&a, "n", end), &csr, t).unwrap();
        assert!(e.renew(&ca, &from(&a, "n", end), &csr, t + 1).is_ok());
        e.note_connected("n", &b.serial_hex, t + 2).unwrap();
        assert_eq!(
            refusal(e.renew(&ca, &from(&a, "n", end), &csr, t + 3)),
            "superseded"
        );
        assert_eq!(resent_rows(&store).len(), 1);
    }

    /// (4) At most five resends an hour per machine; the sixth is `too_frequent`, and the hour after it works again.
    #[test]
    fn resends_are_limited_per_machine_per_hour() {
        let (store, ca, _) = setup();
        let e = Enrollment::new(&store);
        let (a, end) = enrolled(&store, &ca, "n", 1_000_000);
        let (other, other_end) = enrolled(&store, &ca, "m", 1_000_000);
        let (_, csr) = new_device_request().unwrap();
        let t = 1_000_000 + HOUR;
        e.renew(&ca, &from(&a, "n", end), &csr, t).unwrap();
        for i in 0..RESENDS_PER_HOUR {
            assert!(
                e.renew(
                    &ca,
                    &from(&a, "n", end),
                    &csr,
                    t + 1 + u64::try_from(i).unwrap()
                )
                .is_ok(),
                "resend {i}"
            );
        }
        assert_eq!(
            refusal(e.renew(&ca, &from(&a, "n", end), &csr, t + 100)),
            "too_frequent"
        );
        // Another machine has its own budget.
        let (_, csr2) = new_device_request().unwrap();
        let m_new = e
            .renew(&ca, &from(&other, "m", other_end), &csr2, t)
            .unwrap();
        assert!(e
            .renew(&ca, &from(&other, "m", other_end), &csr2, t + 1)
            .is_ok());
        assert_eq!(resent_rows(&store).len(), 6);
        // An hour after the first resend the budget is back.
        assert!(e
            .renew(&ca, &from(&a, "n", end), &csr, t + 1 + HOUR + 1)
            .is_ok());
        let _ = m_new;
    }

    /// (5) A request whose key matches an OLDER issued certificate is not resent: only the newest one is.
    #[test]
    fn only_the_newest_issued_certificate_is_resent() {
        let (store, ca, _) = setup();
        let e = Enrollment::new(&store);
        let (a, end) = enrolled(&store, &ca, "n", 1_000_000);
        let (_, csr_b) = new_device_request().unwrap();
        let (_, csr_c) = new_device_request().unwrap();
        let t = 1_000_000 + HOUR;
        let b = e.renew(&ca, &from(&a, "n", end), &csr_b, t).unwrap();
        e.note_connected("n", &b.serial_hex, t + 1).unwrap();
        // The next renewal (from B, the newest) issues C; the answer is lost.
        let c = e
            .renew(&ca, &from(&b.serial_hex, "n", end), &csr_c, t + 2 * HOUR)
            .unwrap();
        // B's request asked again (by A or B itself) is not C's: it is not resent.
        assert_eq!(
            refusal(e.renew(&ca, &from(&a, "n", end), &csr_b, t + 2 * HOUR + 1)),
            "superseded"
        );
        // C's request is resent to the machine's previous certificate B.
        let again = e
            .renew(
                &ca,
                &from(&b.serial_hex, "n", end),
                &csr_c,
                t + 2 * HOUR + 2,
            )
            .unwrap();
        assert_eq!(again.serial_hex, c.serial_hex);
        // A certificate that was not issued by a renewal (enrollment) has no stored request and is never resent.
        let (x, x_end) = enrolled(&store, &ca, "x", 1_000_000);
        let (_, csr_x) = new_device_request().unwrap();
        let y = e.renew(&ca, &from(&x, "x", x_end), &csr_x, t).unwrap();
        e.note_connected("x", &y.serial_hex, t + 1).unwrap();
        assert_eq!(
            refusal(e.renew(&ca, &from(&x, "x", x_end), &csr_x, t + 2)),
            "superseded"
        );
    }

    /// A certificate that has itself ended is not sent again, even for the request it was issued for.
    #[test]
    fn a_resend_of_a_certificate_that_has_ended_is_refused_as_expired() {
        let (store, ca, _) = setup();
        let e = Enrollment::new(&store);
        // The machine's records are dated 30 days ahead of the real clock, so that the new certificate (whose end is real: 90 days
        // from now) ends while the older one is still inside its lifetime by the hub's record.
        let real_ms = u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap();
        let t0 = real_ms + 30 * DAY;
        let (a, _) = enrolled(&store, &ca, "n", t0);
        let far = i64::MAX / 4;
        let (_, csr) = new_device_request().unwrap();
        let b = e.renew(&ca, &from(&a, "n", far), &csr, t0 + HOUR).unwrap();
        let b_end_ms =
            u64::try_from(crate::ca::cert_not_after_secs(&b.cert_pem).unwrap()).unwrap() * 1000;
        // Before B's end it is sent again; after its end it is refused, and nothing is sent.
        assert_eq!(
            e.renew(&ca, &from(&a, "n", far), &csr, b_end_ms - 2000)
                .unwrap()
                .serial_hex,
            b.serial_hex
        );
        assert_eq!(
            refusal(e.renew(&ca, &from(&a, "n", far), &csr, b_end_ms + 1000)),
            "expired"
        );
        assert_eq!(resent_rows(&store).len(), 1);
    }
}
