//! Client sign-in: a one-time code the owner gets from the hub (`agora hub login`, or printed at start) is exchanged
//! once for a session token. Sessions expire and are stored only as hashes, so a leaked database or log does not hold a
//! usable credential; the code is single use, short lived, and guessing is rate limited like enrollment. Passkeys
//! (ADR-0015) will replace the code as the way to prove who is signing in; the session stays.

use std::sync::Arc;

use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::enroll::{hash, new_code, Lockout, CODE_TTL_MS};
use crate::store::{SqliteStore, StoreError};

/// How long a session lasts.
pub const SESSION_TTL_MS: u64 = 12 * 60 * 60 * 1000;

#[derive(Debug, PartialEq, Eq)]
pub enum SignInError {
    /// A wrong, expired or used code: indistinguishable on purpose.
    BadCode,
    LockedOut {
        retry_after_ms: u64,
    },
    Store(String),
}

impl From<StoreError> for SignInError {
    fn from(e: StoreError) -> Self {
        Self::Store(e.to_string())
    }
}

impl From<rusqlite::Error> for SignInError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Store(e.to_string())
    }
}

fn to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

pub struct SignIn<'a> {
    store: &'a SqliteStore,
    lockout: Arc<Lockout>,
}

impl<'a> SignIn<'a> {
    pub fn new(store: &'a SqliteStore, lockout: Arc<Lockout>) -> Self {
        Self { store, lockout }
    }

    /// A new one-time sign-in code (`XXXX-XXXX`, valid 10 minutes).
    pub fn create_code(&self, now_ms: u64) -> Result<String, SignInError> {
        let code = new_code()?;
        let conn = self.store.lock()?;
        conn.execute(
            "DELETE FROM signin_codes WHERE expires_ms <= ?1",
            [to_i64(now_ms)],
        )?;
        conn.execute(
            "INSERT INTO signin_codes (code_hash, expires_ms) VALUES (?1, ?2)",
            params![hash(&code), to_i64(now_ms.saturating_add(CODE_TTL_MS))],
        )?;
        Ok(code)
    }

    /// Spends `code` and returns a new session token (128 random bits, hex).
    pub fn redeem(&self, code: &str, now_ms: u64) -> Result<String, SignInError> {
        if let Some(retry_after_ms) = self.lockout.check(now_ms) {
            return Err(SignInError::LockedOut { retry_after_ms });
        }
        let mut raw = [0u8; 16];
        getrandom::getrandom(&mut raw).map_err(|e| SignInError::Store(e.to_string()))?;
        let token: String = raw.iter().map(|b| format!("{b:02x}")).collect();
        let mut conn = self.store.lock()?;
        let tx = conn.transaction()?;
        let live: Option<i64> = tx
            .query_row(
                "SELECT expires_ms FROM signin_codes WHERE code_hash = ?1",
                [hash(code)],
                |r| r.get(0),
            )
            .optional()?;
        if live.is_none_or(|exp| exp <= to_i64(now_ms)) {
            drop(tx);
            drop(conn);
            self.lockout.failed(now_ms);
            return Err(SignInError::BadCode);
        }
        tx.execute(
            "DELETE FROM signin_codes WHERE code_hash = ?1",
            [hash(code)],
        )?;
        tx.execute(
            "DELETE FROM sessions WHERE expires_ms <= ?1",
            [to_i64(now_ms)],
        )?;
        tx.execute(
            "INSERT INTO sessions (token_hash, created_ms, expires_ms) VALUES (?1, ?2, ?3)",
            params![
                token_hash(&token),
                to_i64(now_ms),
                to_i64(now_ms.saturating_add(SESSION_TTL_MS))
            ],
        )?;
        tx.commit()?;
        self.lockout.succeeded();
        Ok(token)
    }

    /// Is this a live session?
    pub fn check(&self, token: &str, now_ms: u64) -> bool {
        let Ok(conn) = self.store.lock() else {
            return false;
        };
        conn.query_row(
            "SELECT expires_ms FROM sessions WHERE token_hash = ?1",
            [token_hash(token)],
            |r| r.get::<_, i64>(0),
        )
        .optional()
        .ok()
        .flatten()
        .is_some_and(|exp| exp > to_i64(now_ms))
    }

    /// Ends every session (the owner's "sign out everywhere").
    pub fn revoke_all(&self) -> Result<usize, SignInError> {
        Ok(self.store.lock()?.execute("DELETE FROM sessions", [])?)
    }
}

/// The hub's sign-in: answers `POST /api/session` and decides whether a bearer token is a live session.
pub struct HubSessions {
    hub: Arc<crate::hub::Hub>,
    lockout: Arc<Lockout>,
}

impl HubSessions {
    pub fn new(hub: Arc<crate::hub::Hub>) -> Self {
        Self {
            hub,
            lockout: Arc::new(Lockout::new()),
        }
    }
}

impl crate::web::SessionApi for HubSessions {
    fn sign_in(&self, code: &str) -> crate::web::SignInReply {
        use crate::web::SignInReply;
        match SignIn::new(self.hub.store(), Arc::clone(&self.lockout))
            .redeem(code, crate::server::now_ms())
        {
            Ok(t) => SignInReply::Token(t),
            Err(SignInError::LockedOut { .. }) => SignInReply::TooMany,
            Err(_) => SignInReply::Refused,
        }
    }
}

impl crate::server::ClientAuth for HubSessions {
    fn check(&self, token: &str) -> bool {
        SignIn::new(self.hub.store(), Arc::clone(&self.lockout))
            .check(token, crate::server::now_ms())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn si(store: &SqliteStore) -> SignIn<'_> {
        SignIn::new(store, Arc::new(Lockout::new()))
    }

    #[test]
    fn a_code_is_exchanged_once_for_a_session_that_expires() {
        let store = SqliteStore::open_in_memory().unwrap();
        let s = si(&store);
        let code = s.create_code(0).unwrap();
        let token = s.redeem(&code.to_lowercase(), 1_000).unwrap();
        assert_eq!(token.len(), 32);
        assert!(s.check(&token, 2_000));
        assert!(!s.check("nope", 2_000));
        assert!(!s.check(&token, SESSION_TTL_MS + 2_000), "sessions expire");
        assert_eq!(
            s.redeem(&code, 3_000),
            Err(SignInError::BadCode),
            "single use"
        );
    }

    #[test]
    fn expired_codes_and_guessing_are_refused_alike_and_locked_out() {
        let store = SqliteStore::open_in_memory().unwrap();
        let s = si(&store);
        let code = s.create_code(0).unwrap();
        assert_eq!(s.redeem(&code, CODE_TTL_MS + 1), Err(SignInError::BadCode));
        for _ in 1..crate::enroll::MAX_FAILURES {
            assert_eq!(s.redeem("AAAA-AAAA", 10), Err(SignInError::BadCode));
        }
        let fresh = s.create_code(20).unwrap();
        // Five misses in a row (the expired code counted): even a valid code is refused until the lockout passes.
        assert!(matches!(
            s.redeem(&fresh, 30),
            Err(SignInError::LockedOut { .. })
        ));
        let after = crate::enroll::LOCKOUT_MS + 1_000;
        assert!(s.redeem(&fresh, after).is_ok());
    }

    #[test]
    fn sign_out_everywhere_ends_all_sessions_and_only_hashes_are_stored() {
        let store = SqliteStore::open_in_memory().unwrap();
        let s = si(&store);
        let token = s.redeem(&s.create_code(0).unwrap(), 1).unwrap();
        let stored: Vec<u8> = store
            .lock()
            .unwrap()
            .query_row("SELECT token_hash FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert!(!String::from_utf8_lossy(&stored).contains(&token));
        assert_eq!(s.revoke_all().unwrap(), 1);
        assert!(!s.check(&token, 2));
    }
}
