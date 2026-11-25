//! The metrics token (ADR-0076): a credential that does one thing, read `GET /api/metrics`.
//!
//! It is a separate kind from a session. It starts with `agm_` and holds 256 random bits; only its SHA-256 is stored, in a table
//! of its own, so it can never match a session and a session can never match it. It carries no user and no role, never starts or
//! renews a session, and is looked up on every scrape, so a revocation is immediate. Creating and revoking are audit events with
//! the name, never the token.

use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::store::{SqliteStore, StoreError};

/// What every metrics token starts with: recognisable in a config file and by a secret scanner.
pub const PREFIX: &str = "agm_";

/// `last_used_ms` is written at most once per this long, and rounded down to it.
const USED_GRANULE_MS: u64 = 60_000;

const MAX_NAME: usize = 64;

/// Live credentials are stored as raw 32-byte SHA-256 hashes. A leading byte makes an expired
/// credential's tombstone distinguishable even if the wall clock later moves backwards.
const RETIRED_PREFIX: u8 = 0xff;

#[derive(Debug, PartialEq, Eq)]
pub enum TokenError {
    /// The name is empty, too long, or has characters other than letters, digits, `.`, `_`, `-`.
    BadName,
    /// A live token already has this name.
    Exists,
    Store(String),
}

impl From<StoreError> for TokenError {
    fn from(e: StoreError) -> Self {
        Self::Store(e.to_string())
    }
}

impl From<rusqlite::Error> for TokenError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Store(e.to_string())
    }
}

/// One row as `list` shows it (never the token, never its hash). A zero means "never" (used, expires) or "not" (revoked).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    pub name: String,
    pub created_ms: u64,
    pub last_used_ms: u64,
    pub expires_ms: u64,
    pub revoked_ms: u64,
}

pub struct MetricsTokens<'a> {
    store: &'a SqliteStore,
}

fn to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn to_u64(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

fn hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

/// Replaces the usable token hash while keeping the row that `list` shows. The payload is a
/// domain-separated SHA-256 derivative with a tag that no live credential hash has.
fn retired_hash(token_hash: &[u8]) -> Vec<u8> {
    let mut digest = Sha256::new();
    digest.update(b"agora expired metrics token\0");
    digest.update(token_hash);
    let mut retired = Vec::with_capacity(33);
    retired.push(RETIRED_PREFIX);
    retired.extend_from_slice(&digest.finalize());
    retired
}

fn is_retired(token_hash: &[u8]) -> bool {
    token_hash.len() == 33 && token_hash.first() == Some(&RETIRED_PREFIX)
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Equal bytes, compared in time that does not depend on where they first differ.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Whether `token` has the shape of a metrics token: it is looked up here and never among sessions.
pub fn looks_like(token: &str) -> bool {
    token.starts_with(PREFIX)
}

impl<'a> MetricsTokens<'a> {
    pub fn new(store: &'a SqliteStore) -> Self {
        Self { store }
    }

    /// Makes a token called `name` and returns it: the only time it exists in the clear. `expires_ms` is when it stops working
    /// (`None`: never). A name whose token was revoked may be used again.
    pub fn create(
        &self,
        name: &str,
        now_ms: u64,
        expires_ms: Option<u64>,
    ) -> Result<String, TokenError> {
        if !valid_name(name) {
            return Err(TokenError::BadName);
        }
        let mut raw = [0u8; 32];
        getrandom::getrandom(&mut raw).map_err(|e| TokenError::Store(e.to_string()))?;
        let token = format!(
            "{PREFIX}{}",
            raw.iter().map(|b| format!("{b:02x}")).collect::<String>()
        );
        let conn = self.store.lock()?;
        let existing: Option<(Vec<u8>, i64, i64)> = conn
            .query_row(
                "SELECT token_hash, expires_ms, revoked_ms FROM metrics_tokens WHERE name = ?1",
                [name],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        if existing.is_some_and(|(token_hash, expires, revoked)| {
            !is_retired(&token_hash) && revoked == 0 && (expires == 0 || expires > to_i64(now_ms))
        }) {
            return Err(TokenError::Exists);
        }
        conn.execute("DELETE FROM metrics_tokens WHERE name = ?1", [name])?;
        conn.execute(
            "INSERT INTO metrics_tokens (name, token_hash, created_ms, expires_ms, last_used_ms, revoked_ms)
             VALUES (?1, ?2, ?3, ?4, 0, 0)",
            params![name, hash(&token), to_i64(now_ms), to_i64(expires_ms.unwrap_or(0))],
        )?;
        drop(conn);
        let _ = crate::audit::Audit::new(self.store).record(
            now_ms,
            "owner",
            "metrics_token.create",
            name,
            "ok",
        );
        Ok(token)
    }

    /// Ends the token called `name`. `false` when no live token has that name.
    pub fn revoke(&self, name: &str, now_ms: u64) -> Result<bool, TokenError> {
        let conn = self.store.lock()?;
        let row: Option<(Vec<u8>, i64, i64)> = conn
            .query_row(
                "SELECT token_hash, expires_ms, revoked_ms FROM metrics_tokens WHERE name = ?1",
                [name],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((token_hash, expires, revoked)) = row else {
            return Ok(false);
        };
        if is_retired(&token_hash) || revoked != 0 {
            return Ok(false);
        }
        if expires != 0 && expires <= to_i64(now_ms) {
            conn.execute(
                "UPDATE metrics_tokens SET token_hash = ?2 WHERE name = ?1 AND token_hash = ?3",
                params![name, retired_hash(&token_hash), token_hash],
            )?;
            return Ok(false);
        }
        let n = conn.execute(
            "UPDATE metrics_tokens SET revoked_ms = ?2 WHERE name = ?1 AND revoked_ms = 0
             AND (expires_ms = 0 OR expires_ms > ?3)",
            params![name, to_i64(now_ms.max(1)), to_i64(now_ms)],
        )?;
        drop(conn);
        if n > 0 {
            let _ = crate::audit::Audit::new(self.store).record(
                now_ms,
                "owner",
                "metrics_token.revoke",
                name,
                "ok",
            );
        }
        Ok(n > 0)
    }

    /// Every token, oldest first.
    pub fn list(&self) -> Result<Vec<Info>, TokenError> {
        let conn = self.store.lock()?;
        let mut st = conn.prepare(
            "SELECT name, created_ms, last_used_ms, expires_ms, revoked_ms FROM metrics_tokens ORDER BY created_ms, name",
        )?;
        let rows = st.query_map([], |r| {
            Ok(Info {
                name: r.get(0)?,
                created_ms: to_u64(r.get(1)?),
                last_used_ms: to_u64(r.get(2)?),
                expires_ms: to_u64(r.get(3)?),
                revoked_ms: to_u64(r.get(4)?),
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Is this a live metrics token (known, not revoked, not expired)? Looked up on every call. A use is written down at most once
    /// a minute, rounded to the minute.
    pub fn check(&self, token: &str, now_ms: u64) -> bool {
        if !looks_like(token) {
            return false;
        }
        let Ok(conn) = self.store.lock() else {
            return false;
        };
        let want = hash(token);
        let row: Option<(Vec<u8>, i64, i64, i64)> = conn
            .query_row(
                "SELECT token_hash, expires_ms, revoked_ms, last_used_ms FROM metrics_tokens WHERE token_hash = ?1",
                [&want],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .ok()
            .flatten();
        let Some((stored, expires, revoked, used)) = row else {
            return false;
        };
        // The lookup was by hash; the comparison that decides is constant-time all the same.
        if !same(&stored, &want) {
            return false;
        }
        if revoked != 0 {
            return false;
        }
        if expires != 0 && to_i64(now_ms) >= expires {
            let _ = conn.execute(
                "UPDATE metrics_tokens SET token_hash = ?2 WHERE token_hash = ?1",
                params![want, retired_hash(&stored)],
            );
            return false;
        }
        let minute = to_i64(now_ms - now_ms % USED_GRANULE_MS);
        if used < minute {
            let _ = conn.execute(
                "UPDATE metrics_tokens SET last_used_ms = ?2 WHERE token_hash = ?1",
                params![want, minute],
            );
        }
        true
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {

    #[test]
    fn token_hash_known_answer_does_not_change_across_the_sha2_crate_versions() {
        let hex: String = super::hash("abc")
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(
            hex,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
    use super::*;

    const DAY: u64 = 86_400_000;

    #[test]
    fn a_token_has_the_prefix_and_256_bits_and_checks_until_revoked() {
        let s = SqliteStore::open_in_memory().unwrap();
        let t = MetricsTokens::new(&s);
        let a = t.create("prom", 1_000, None).unwrap();
        let b = t.create("cron", 1_000, None).unwrap();
        assert!(a.starts_with("agm_") && a.len() == 4 + 64 && a != b);
        assert!(t.check(&a, 2_000) && t.check(&b, 2_000));
        assert!(!t.check("agm_nope", 2_000) && !t.check("", 2_000));
        assert!(t.revoke("prom", 3_000).unwrap());
        assert!(!t.check(&a, 3_001) && t.check(&b, 3_001));
        assert!(!t.revoke("prom", 3_002).unwrap(), "already revoked");
        assert!(!t.revoke("none", 3_002).unwrap());
    }

    #[test]
    fn an_expiry_ends_the_token_and_none_never_does() {
        let s = SqliteStore::open_in_memory().unwrap();
        let t = MetricsTokens::new(&s);
        let short = t.create("short", 0, Some(DAY)).unwrap();
        let long = t.create("long", 0, None).unwrap();
        assert!(t.check(&short, DAY - 1) && !t.check(&short, DAY));
        assert!(t.check(&long, 4000 * DAY));
    }

    #[test]
    fn f390_an_expired_metrics_token_cannot_return_after_clock_rollback() {
        let s = SqliteStore::open_in_memory().unwrap();
        let t = MetricsTokens::new(&s);
        let never = t.create("never", 0, None).unwrap();
        assert!(t.check(&never, 4000 * DAY));
        assert!(!t.check("agm_unknown", 1));
        println!("CONTROL OK");

        let short = t.create("short", 0, Some(DAY)).unwrap();
        assert!(!t.check(&short, DAY));
        let actual = t.check(&short, DAY - 1);
        println!("EXPECTED: false");
        println!("ACTUAL: {actual}");
        if !actual {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert!(!actual);
    }

    #[test]
    fn verifies_f390_metrics_token_expiry_boundaries() {
        let never_store = SqliteStore::open_in_memory().unwrap();
        let never = MetricsTokens::new(&never_store);
        let token = never.create("never", 0, None).unwrap();
        assert!(never.check(&token, 4000 * DAY));

        let exact_store = SqliteStore::open_in_memory().unwrap();
        let exact = MetricsTokens::new(&exact_store);
        let token = exact.create("exact", 0, Some(DAY)).unwrap();
        assert!(exact.check(&token, DAY - 1));
        assert!(!exact.check(&token, DAY));
        assert!(!exact.check(&token, DAY - 1));
        assert_eq!(exact.list().unwrap()[0].expires_ms, DAY);

        let late_store = SqliteStore::open_in_memory().unwrap();
        let late = MetricsTokens::new(&late_store);
        let token = late.create("late", 0, Some(DAY)).unwrap();
        assert!(!late.check(&token, DAY + 500));
        assert!(!late.check(&token, 1));
        assert!(!late.check("agm_unknown", 1));
        println!("FIX VERIFIED");
    }

    #[test]
    fn f391_an_expired_metrics_token_name_is_not_live() {
        let s = SqliteStore::open_in_memory().unwrap();
        let t = MetricsTokens::new(&s);
        t.create("revoked", 0, None).unwrap();
        assert!(t.revoke("revoked", 1).unwrap());
        assert!(t.create("revoked", 2, None).is_ok());
        println!("CONTROL OK");

        t.create("expired", 0, Some(DAY)).unwrap();
        let actual = t.create("expired", DAY, None).map(|_| ());
        println!("EXPECTED: Ok(())");
        println!("ACTUAL: {actual:?}");
        if actual.is_ok() {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert_eq!(actual, Ok(()));
    }

    #[test]
    fn verifies_f391_metrics_token_name_reuse_boundaries() {
        let store = SqliteStore::open_in_memory().unwrap();
        let tokens = MetricsTokens::new(&store);
        tokens.create("live", 0, Some(DAY)).unwrap();
        assert_eq!(
            tokens.create("live", DAY - 1, None),
            Err(TokenError::Exists)
        );
        tokens.create("never", 0, None).unwrap();
        assert_eq!(
            tokens.create("never", 4000 * DAY, None),
            Err(TokenError::Exists)
        );

        let old = tokens.create("exact", 0, Some(DAY)).unwrap();
        let exact = tokens.create("exact", DAY, None).unwrap();
        assert!(!tokens.check(&old, 1));
        assert!(tokens.check(&exact, DAY));

        tokens.create("late", 0, Some(DAY)).unwrap();
        assert!(tokens.create("late", DAY + 500, None).is_ok());
        tokens.create("revoked", 0, None).unwrap();
        assert!(tokens.revoke("revoked", 1).unwrap());
        assert!(tokens.create("revoked", 2, None).is_ok());
        println!("FIX VERIFIED");
    }

    #[test]
    fn f392_revoking_an_expired_metrics_token_reports_it_was_not_live() {
        let control_store = SqliteStore::open_in_memory().unwrap();
        let control = MetricsTokens::new(&control_store);
        control.create("live", 0, Some(DAY)).unwrap();
        assert!(control.revoke("live", DAY - 1).unwrap());
        assert!(!control.revoke("live", DAY).unwrap());
        println!("CONTROL OK");

        let store = SqliteStore::open_in_memory().unwrap();
        let tokens = MetricsTokens::new(&store);
        tokens.create("expired", 0, Some(DAY)).unwrap();
        let actual = tokens.revoke("expired", DAY).unwrap();
        println!("EXPECTED: false");
        println!("ACTUAL: {actual}");
        if !actual {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert!(!actual);
    }

    #[test]
    fn verifies_f392_metrics_token_revoke_boundaries() {
        let store = SqliteStore::open_in_memory().unwrap();
        let tokens = MetricsTokens::new(&store);
        assert!(!tokens.revoke("unknown", 1).unwrap());

        tokens.create("live", 0, Some(DAY)).unwrap();
        assert!(tokens.revoke("live", DAY - 1).unwrap());
        assert!(!tokens.revoke("live", DAY).unwrap());

        let exact = tokens.create("exact", 0, Some(DAY)).unwrap();
        assert!(!tokens.revoke("exact", DAY).unwrap());
        assert!(!tokens.check(&exact, DAY - 1));

        let late = tokens.create("late", 0, Some(DAY)).unwrap();
        assert!(!tokens.revoke("late", DAY + 500).unwrap());
        assert!(!tokens.check(&late, 1));
        println!("FIX VERIFIED");
    }

    #[test]
    fn f395_an_observed_expired_metrics_token_name_stays_reusable_after_clock_rollback() {
        let control_store = SqliteStore::open_in_memory().unwrap();
        let control = MetricsTokens::new(&control_store);
        control.create("name", 0, None).unwrap();
        assert!(control.revoke("name", 1).unwrap());
        assert!(control.create("name", 2, None).is_ok());
        println!("CONTROL OK");

        let store = SqliteStore::open_in_memory().unwrap();
        let tokens = MetricsTokens::new(&store);
        let expired = tokens.create("name", 0, Some(DAY)).unwrap();
        assert!(!tokens.check(&expired, DAY));
        let actual = tokens.create("name", DAY - 1, None).map(|_| ());
        println!("EXPECTED: Ok(())");
        println!("ACTUAL: {actual:?}");
        if actual.is_ok() {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert_eq!(actual, Ok(()));
    }

    #[test]
    fn f396_revoking_an_observed_expired_metrics_token_stays_false_after_clock_rollback() {
        let control_store = SqliteStore::open_in_memory().unwrap();
        let control = MetricsTokens::new(&control_store);
        control.create("live", 0, Some(DAY)).unwrap();
        assert!(control.revoke("live", DAY - 1).unwrap());
        assert!(!control.revoke("unknown", DAY - 1).unwrap());
        println!("CONTROL OK");

        let store = SqliteStore::open_in_memory().unwrap();
        let tokens = MetricsTokens::new(&store);
        let expired = tokens.create("expired", 0, Some(DAY)).unwrap();
        assert!(!tokens.check(&expired, DAY));
        let actual = tokens.revoke("expired", DAY - 1).unwrap();
        println!("EXPECTED: false");
        println!("ACTUAL: {actual}");
        if !actual {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert!(!actual);
    }

    #[test]
    fn verifies_f395_terminal_metrics_token_name_boundaries() {
        let store = SqliteStore::open_in_memory().unwrap();
        let tokens = MetricsTokens::new(&store);

        tokens.create("active", 0, None).unwrap();
        assert_eq!(tokens.create("active", DAY, None), Err(TokenError::Exists));

        let exact = tokens.create("exact", 0, Some(DAY)).unwrap();
        assert!(!tokens.check(&exact, DAY));
        assert!(tokens.create("exact", DAY - 1, None).is_ok());

        let late = tokens.create("late", 0, Some(DAY)).unwrap();
        assert!(!tokens.check(&late, DAY * 2));
        assert!(tokens.create("late", DAY - 1, None).is_ok());

        tokens.create("revoked", 0, None).unwrap();
        assert!(tokens.revoke("revoked", 1).unwrap());
        assert!(tokens.create("revoked", 0, None).is_ok());
        println!("FIX VERIFIED");
    }

    #[test]
    fn verifies_f396_terminal_metrics_token_revoke_boundaries() {
        let store = SqliteStore::open_in_memory().unwrap();
        let tokens = MetricsTokens::new(&store);

        tokens.create("live", 0, None).unwrap();
        assert!(tokens.revoke("live", DAY).unwrap());
        assert!(!tokens.revoke("live", DAY).unwrap());
        assert!(!tokens.revoke("unknown", DAY).unwrap());

        let exact = tokens.create("exact", 0, Some(DAY)).unwrap();
        assert!(!tokens.check(&exact, DAY));
        assert!(!tokens.revoke("exact", DAY - 1).unwrap());

        let late = tokens.create("late", 0, Some(DAY)).unwrap();
        assert!(!tokens.check(&late, DAY * 2));
        assert!(!tokens.revoke("late", DAY - 1).unwrap());
        println!("FIX VERIFIED");
    }

    #[test]
    fn names_are_checked_and_a_live_name_is_not_taken_twice_but_a_revoked_one_is_free() {
        let s = SqliteStore::open_in_memory().unwrap();
        let t = MetricsTokens::new(&s);
        for bad in ["", "a b", "x/y", &"n".repeat(65)] {
            assert_eq!(t.create(bad, 0, None), Err(TokenError::BadName), "{bad:?}");
        }
        let first = t.create("prom", 0, None).unwrap();
        assert_eq!(t.create("prom", 1, None), Err(TokenError::Exists));
        t.revoke("prom", 2).unwrap();
        let second = t.create("prom", 3, None).unwrap();
        assert!(!t.check(&first, 4) && t.check(&second, 4));
    }

    #[test]
    fn only_a_hash_is_stored_and_list_and_the_audit_trail_never_show_the_token() {
        let s = SqliteStore::open_in_memory().unwrap();
        let t = MetricsTokens::new(&s);
        let token = t.create("prom", 5, None).unwrap();
        t.revoke("prom", 6).unwrap();
        let hex = &token[PREFIX.len()..];
        let dump = {
            let conn = s.lock().unwrap();
            let mut text = String::new();
            let mut st = conn
                .prepare("SELECT name, hex(token_hash) FROM metrics_tokens")
                .unwrap();
            for row in st
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .unwrap()
            {
                let (n, h) = row.unwrap();
                text.push_str(&format!("{n} {h}\n"));
            }
            let mut st = conn
                .prepare("SELECT who, action, node, outcome FROM audit")
                .unwrap();
            for row in st
                .query_map([], |r| {
                    Ok(format!(
                        "{} {} {} {}\n",
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?
                    ))
                })
                .unwrap()
            {
                text.push_str(&row.unwrap());
            }
            text
        };
        assert!(!dump.contains(hex) && !dump.contains(&token));
        assert!(dump.contains("owner metrics_token.create prom ok"));
        assert!(dump.contains("owner metrics_token.revoke prom ok"));
        let shown = format!("{:?}", t.list().unwrap());
        assert!(!shown.contains(hex));
    }

    #[test]
    fn use_is_written_at_most_once_a_minute_rounded_to_the_minute() {
        let s = SqliteStore::open_in_memory().unwrap();
        let t = MetricsTokens::new(&s);
        let token = t.create("prom", 0, None).unwrap();
        assert_eq!(t.list().unwrap()[0].last_used_ms, 0);
        assert!(t.check(&token, 125_000));
        assert_eq!(t.list().unwrap()[0].last_used_ms, 120_000);
        assert!(t.check(&token, 150_000));
        assert_eq!(t.list().unwrap()[0].last_used_ms, 120_000);
        assert!(t.check(&token, 185_000));
        assert_eq!(t.list().unwrap()[0].last_used_ms, 180_000);
    }

    #[test]
    fn a_metrics_token_is_not_a_session_and_a_session_is_not_a_metrics_token() {
        use crate::enroll::Lockout;
        use crate::signin::SignIn;
        let s = SqliteStore::open_in_memory().unwrap();
        let token = MetricsTokens::new(&s).create("prom", 0, None).unwrap();
        let si = SignIn::new(&s, std::sync::Arc::new(Lockout::new()));
        assert!(!si.check(&token, 1) && si.access(&token, 1).is_none());
        let session = si.start_session(1).unwrap();
        assert!(!MetricsTokens::new(&s).check(&session, 2));
    }
}
