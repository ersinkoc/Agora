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

fn new_token() -> Result<String, SignInError> {
    let mut raw = [0u8; 16];
    getrandom::getrandom(&mut raw).map_err(|e| SignInError::Store(e.to_string()))?;
    Ok(raw.iter().map(|b| format!("{b:02x}")).collect())
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
        self.create_code_for(None, now_ms)
    }

    /// As [`Self::create_code`], for a person of the team (ADR-0049): the session it is exchanged for is theirs.
    pub fn create_code_for(&self, user: Option<&str>, now_ms: u64) -> Result<String, SignInError> {
        let code = new_code()?;
        let conn = self.store.lock()?;
        conn.execute(
            "DELETE FROM signin_codes WHERE expires_ms <= ?1",
            [to_i64(now_ms)],
        )?;
        conn.execute(
            "INSERT INTO signin_codes (code_hash, expires_ms, user_id) VALUES (?1, ?2, ?3)",
            params![
                hash(&code),
                to_i64(now_ms.saturating_add(CODE_TTL_MS)),
                user
            ],
        )?;
        Ok(code)
    }

    /// Spends `code` and returns a new session token (128 random bits, hex).
    pub fn redeem(&self, code: &str, now_ms: u64) -> Result<String, SignInError> {
        if let Some(retry_after_ms) = self.lockout.check(now_ms) {
            return Err(SignInError::LockedOut { retry_after_ms });
        }
        let token = new_token()?;
        let mut conn = self.store.lock()?;
        let tx = conn.transaction()?;
        let live: Option<(i64, Option<String>)> = tx
            .query_row(
                "SELECT expires_ms, user_id FROM signin_codes WHERE code_hash = ?1",
                [hash(code)],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if live.as_ref().is_none_or(|(exp, _)| *exp <= to_i64(now_ms)) {
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
        let user = live.and_then(|(_, u)| u);
        tx.execute(
            "INSERT INTO sessions (token_hash, created_ms, expires_ms, user_id) VALUES (?1, ?2, ?3, ?4)",
            params![
                token_hash(&token),
                to_i64(now_ms),
                to_i64(now_ms.saturating_add(SESSION_TTL_MS)),
                user
            ],
        )?;
        tx.commit()?;
        drop(conn);
        let _ = crate::audit::Audit::new(self.store).record(
            now_ms,
            user.as_deref().unwrap_or("owner"),
            "sign_in",
            "",
            "ok",
        );
        self.lockout.succeeded();
        Ok(token)
    }

    /// A session without a code: the caller has already proved who it is (a passkey assertion).
    pub fn start_session(&self, now_ms: u64) -> Result<String, SignInError> {
        let token = new_token()?;
        let conn = self.store.lock()?;
        conn.execute(
            "DELETE FROM sessions WHERE expires_ms <= ?1",
            [to_i64(now_ms)],
        )?;
        conn.execute(
            "INSERT INTO sessions (token_hash, created_ms, expires_ms) VALUES (?1, ?2, ?3)",
            params![
                token_hash(&token),
                to_i64(now_ms),
                to_i64(now_ms.saturating_add(SESSION_TTL_MS))
            ],
        )?;
        drop(conn);
        let _ = crate::audit::Audit::new(self.store)
            .record(now_ms, "owner", "sign_in", "passkey", "ok");
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

    /// What the holder of this session may do (ADR-0049); `None` for a session that is not live or whose person was removed.
    pub fn access(&self, token: &str, now_ms: u64) -> Option<crate::users::Access> {
        if !self.check(token, now_ms) {
            return None;
        }
        crate::users::Users::new(self.store).access_of_session(&token_hash(token))
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
    challenges: crate::passkey::Challenges,
}

impl HubSessions {
    pub fn new(hub: Arc<crate::hub::Hub>) -> Self {
        Self {
            hub,
            lockout: Arc::new(Lockout::new()),
            challenges: crate::passkey::Challenges::new(),
        }
    }
}

impl crate::web::SessionApi for HubSessions {
    fn passkey(
        &self,
        op: &str,
        bearer: Option<&str>,
        host: &str,
        body: &serde_json::Value,
    ) -> (u16, String) {
        self.passkey_op(op, bearer, host, body)
    }

    fn audit(&self, bearer: Option<&str>, query: &str) -> (u16, String) {
        let Some(token) = bearer else {
            return (401, String::new());
        };
        let signin = SignIn::new(self.hub.store(), Arc::clone(&self.lockout));
        match signin.access(token, crate::server::now_ms()) {
            None => return (401, String::new()),
            Some(a) if a.role != crate::users::Role::Owner => return (403, String::new()),
            Some(_) => {}
        }
        let (mut since, mut who, mut limit) = (0u64, None, 500usize);
        for pair in query.split('&').filter(|p| !p.is_empty()) {
            match pair.split_once('=') {
                Some(("since", v)) => since = v.parse().unwrap_or(0),
                Some(("limit", v)) => limit = v.parse().unwrap_or(500).clamp(1, 5000),
                Some(("who", v)) if v.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') => {
                    who = Some(v);
                }
                _ => return (400, String::new()),
            }
        }
        match crate::audit::Audit::new(self.hub.store()).list(since, who, limit) {
            Ok(rows) => (
                200,
                serde_json::Value::Array(rows.iter().map(crate::audit::json_value).collect())
                    .to_string(),
            ),
            Err(_) => (500, String::new()),
        }
    }

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

impl HubSessions {
    fn passkey_op(
        &self,
        op: &str,
        bearer: Option<&str>,
        host: &str,
        body: &serde_json::Value,
    ) -> (u16, String) {
        use crate::passkey::{self as pk, Purpose};
        let now = crate::server::now_ms();
        let store = self.hub.store();
        let field = |k: &str| {
            body.get(k)
                .and_then(|v| v.as_str())
                .and_then(pk::b64url_decode)
        };
        let signed_in =
            bearer.is_some_and(|t| SignIn::new(store, Arc::clone(&self.lockout)).check(t, now));
        let json = |v: serde_json::Value| (200, v.to_string());
        match op {
            // Adding a passkey needs a signed-in session: the code (or another passkey) proved who is here.
            "register-begin" if signed_in => match self.challenges.issue(Purpose::Register, now) {
                Ok(c) => json(serde_json::json!({ "challenge": c, "rpId": pk::rp_id(host) })),
                Err(_) => (500, String::new()),
            },
            "register-finish" if signed_in => {
                let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("");
                match (field("publicKey"), field("clientDataJSON")) {
                    (Some(key), Some(cd)) => {
                        match pk::register(store, &self.challenges, host, id, &key, &cd, now) {
                            Ok(()) => (200, "{}".into()),
                            Err(pk::PasskeyError::Refused) => (403, String::new()),
                            Err(_) => (500, String::new()),
                        }
                    }
                    _ => (400, String::new()),
                }
            }
            "register-begin" | "register-finish" => (401, String::new()),
            "login-begin" => match (pk::ids(store), self.challenges.issue(Purpose::Login, now)) {
                (Ok(ids), Ok(c)) => {
                    json(serde_json::json!({ "challenge": c, "rpId": pk::rp_id(host), "ids": ids }))
                }
                _ => (500, String::new()),
            },
            "login-finish" => {
                if self.lockout.check(now).is_some() {
                    return (429, String::new());
                }
                let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let (Some(auth), Some(cd), Some(sig)) = (
                    field("authenticatorData"),
                    field("clientDataJSON"),
                    field("signature"),
                ) else {
                    return (400, String::new());
                };
                match pk::login(store, &self.challenges, host, id, &auth, &cd, &sig, now) {
                    Ok(()) => {
                        match SignIn::new(store, Arc::clone(&self.lockout)).start_session(now) {
                            Ok(t) => {
                                self.lockout.succeeded();
                                json(serde_json::json!({ "token": t }))
                            }
                            Err(_) => (500, String::new()),
                        }
                    }
                    Err(pk::PasskeyError::Refused) => {
                        self.lockout.failed(now);
                        (401, String::new())
                    }
                    Err(_) => (500, String::new()),
                }
            }
            _ => (404, String::new()),
        }
    }
}

impl crate::server::ClientAuth for HubSessions {
    fn check(&self, token: &str) -> bool {
        self.access(token).is_some()
    }

    fn access(&self, token: &str) -> Option<crate::users::Access> {
        SignIn::new(self.hub.store(), Arc::clone(&self.lockout))
            .access(token, crate::server::now_ms())
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

    #[test]
    fn a_code_made_for_a_person_gives_a_session_with_their_role_and_removing_them_ends_it() {
        use crate::users::{Role, Users};
        let store = SqliteStore::open_in_memory().unwrap();
        let signin = si(&store);
        let user = Users::new(&store).add("Eda", Role::Viewer, 1).unwrap();
        let token = signin
            .redeem(&signin.create_code_for(Some(&user.id), 10).unwrap(), 11)
            .unwrap();
        assert_eq!(
            signin.access(&token, 12).map(|a| a.role),
            Some(Role::Viewer)
        );
        // The owner's code has no person: the owner's role, as before.
        let owner = signin.redeem(&signin.create_code(10).unwrap(), 11).unwrap();
        assert_eq!(signin.access(&owner, 12).map(|a| a.role), Some(Role::Owner));
        Users::new(&store).set_role(&user.id, Role::Member).unwrap();
        assert_eq!(
            signin.access(&token, 12).map(|a| a.role),
            Some(Role::Member)
        );
        Users::new(&store).remove(&user.id).unwrap();
        assert_eq!(signin.access(&token, 12), None);
        assert_eq!(signin.access("not-a-token", 12), None);
    }
}
