//! Client sign-in: a one-time code the owner gets from the hub (`agora hub login`, or printed at start) is exchanged
//! once for a session token. Sessions expire and are stored only as hashes, so a leaked database or log does not hold a
//! usable credential; the code is single use, short lived, and guessing is rate limited like enrollment. Passkeys
//! (ADR-0015) will replace the code as the way to prove who is signing in; the session stays.

use std::sync::Arc;

use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::enroll::{hash, new_code, Lockout, CODE_TTL_MS};
use crate::password::{Client, Guard, PasswordError, Scope, ScreenPassword};
use crate::store::{SqliteStore, StoreError};

/// How long a session lasts.
/// A session lasts 30 days and is extended on use (see `check`), so a person who uses the window is not asked for a code again; signing out
/// or revoking ends it at once.
pub const SESSION_TTL_MS: u64 = 30 * 24 * 60 * 60 * 1000;

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
        match self.exchange(code, now_ms, "") {
            Err(SignInError::BadCode) => {
                self.lockout.failed(now_ms);
                Err(SignInError::BadCode)
            }
            Ok(token) => {
                self.lockout.succeeded();
                Ok(token)
            }
            Err(e) => Err(e),
        }
    }

    /// [`Self::redeem`] without its lockout: the caller rate limits (the hub's [`crate::password::Guard`]). `origin` is what the
    /// audit row names (the client's address and browser).
    pub fn exchange(&self, code: &str, now_ms: u64, origin: &str) -> Result<String, SignInError> {
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
        let _ = crate::audit::Audit::new(self.store).record_in(
            &crate::audit::session_id(&token_hash(&token)),
            now_ms,
            user.as_deref().unwrap_or("owner"),
            "sign_in",
            origin,
            "ok",
        );
        Ok(token)
    }

    /// A session without a code: the caller has already proved who it is (a passkey assertion).
    pub fn start_session(&self, now_ms: u64) -> Result<String, SignInError> {
        self.start_session_for(None, now_ms)
    }

    /// As [`Self::start_session`], for a person of the team: their passkey proved who is here (ADR-0049).
    pub fn start_session_for(
        &self,
        user: Option<&str>,
        now_ms: u64,
    ) -> Result<String, SignInError> {
        self.start_session_from(user, now_ms, "passkey")
    }

    /// As [`Self::start_session_for`], audited with `origin` (how and from where).
    pub fn start_session_from(
        &self,
        user: Option<&str>,
        now_ms: u64,
        origin: &str,
    ) -> Result<String, SignInError> {
        let token = new_token()?;
        let conn = self.store.lock()?;
        conn.execute(
            "DELETE FROM sessions WHERE expires_ms <= ?1",
            [to_i64(now_ms)],
        )?;
        conn.execute(
            "INSERT INTO sessions (token_hash, created_ms, expires_ms, user_id) VALUES (?1, ?2, ?3, ?4)",
            params![
                token_hash(&token),
                to_i64(now_ms),
                to_i64(now_ms.saturating_add(SESSION_TTL_MS)),
                user
            ],
        )?;
        drop(conn);
        let _ = crate::audit::Audit::new(self.store).record_in(
            &crate::audit::session_id(&token_hash(&token)),
            now_ms,
            user.unwrap_or("owner"),
            "sign_in",
            origin,
            "ok",
        );
        Ok(token)
    }

    /// Is this a live session?
    pub fn check(&self, token: &str, now_ms: u64) -> bool {
        let Ok(conn) = self.store.lock() else {
            return false;
        };
        let hash = token_hash(token);
        let live = conn
            .query_row(
                "SELECT expires_ms FROM sessions WHERE token_hash = ?1",
                [&hash],
                |r| r.get::<_, i64>(0),
            )
            .optional()
            .ok()
            .flatten()
            .filter(|exp| *exp > to_i64(now_ms));
        // Use extends the session: once less than half of its time is left it runs a full term again from now.
        if let Some(exp) = live {
            if exp - to_i64(now_ms) < to_i64(SESSION_TTL_MS / 2) {
                let _ = conn.execute(
                    "UPDATE sessions SET expires_ms = ?2 WHERE token_hash = ?1",
                    params![hash, to_i64(now_ms.saturating_add(SESSION_TTL_MS))],
                );
            }
        }
        live.is_some()
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

    /// Ends every session but `keep` (a password change from the window signs out everywhere else).
    pub fn revoke_others(&self, keep: &str) -> Result<usize, SignInError> {
        Ok(self.store.lock()?.execute(
            "DELETE FROM sessions WHERE token_hash != ?1",
            [token_hash(keep)],
        )?)
    }

    /// Ends this one session (sign out, or "Lock" in the window).
    pub fn end(&self, token: &str) -> Result<bool, SignInError> {
        Ok(self.store.lock()?.execute(
            "DELETE FROM sessions WHERE token_hash = ?1",
            [token_hash(token)],
        )? > 0)
    }
}

/// The hub's sign-in: answers `/api/session`, `/api/password` and `/api/passkey/*`, and decides whether a bearer
/// token is a live session. Codes, the screen password and passkeys are rate limited per address and for the hub by one
/// [`Guard`] (ADR-0067).
pub struct HubSessions {
    hub: Arc<crate::hub::Hub>,
    lockout: Arc<Lockout>,
    guard: Guard,
    challenges: crate::passkey::Challenges,
    password_rounds: u32,
}

/// What a failed sign-in step leaves in the audit trail (the owner sees why; the client never does).
const WRONG_CODE: &str = "signin.wrong_code";
const WRONG_PASSWORD: &str = "signin.wrong_password";
const LOCKED_OUT: &str = "signin.locked_out";

impl HubSessions {
    pub fn new(hub: Arc<crate::hub::Hub>) -> Self {
        Self {
            hub,
            lockout: Arc::new(Lockout::new()),
            guard: Guard::new(),
            challenges: crate::passkey::Challenges::new(),
            password_rounds: crate::password::ITERATIONS,
        }
    }

    /// Hashes passwords set from the window with `rounds` PBKDF2 rounds instead of [`crate::password::ITERATIONS`]. For tests only.
    pub fn with_password_rounds(mut self, rounds: u32) -> Self {
        self.password_rounds = rounds;
        self
    }

    fn signin(&self) -> SignIn<'_> {
        SignIn::new(self.hub.store(), Arc::clone(&self.lockout))
    }

    fn screen_password(&self) -> ScreenPassword<'_> {
        ScreenPassword::with_iterations(self.hub.store(), self.password_rounds)
    }

    /// An audit row for a step that did not prove anyone: who is unknown, the session is none.
    fn audit_failure(&self, action: &str, how: &str, client: &Client, now: u64, outcome: &str) {
        let _ = crate::audit::Audit::new(self.hub.store()).record(
            now,
            "-",
            action,
            &format!("{how} {}", client.describe()),
            outcome,
        );
    }
}

impl crate::web::SessionApi for HubSessions {
    fn passkey(
        &self,
        op: &str,
        bearer: Option<&str>,
        host: &str,
        client: &Client,
        body: &serde_json::Value,
    ) -> (u16, String) {
        self.passkey_op(op, bearer, host, client, body)
    }

    fn audit(&self, bearer: Option<&str>, query: &str) -> (u16, String) {
        let Some(token) = bearer else {
            return (401, String::new());
        };
        match self.signin().access(token, crate::server::now_ms()) {
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

    fn info(&self, client: &Client) -> serde_json::Value {
        let set = self.screen_password().is_set();
        serde_json::json!({ "passwordRequired": set && !client.is_local(), "passwordSet": set })
    }

    fn sign_in(&self, req: &crate::web::SignInRequest<'_>) -> crate::web::SignInReply {
        use crate::web::SignInReply;
        let now = crate::server::now_ms();
        let client = req.client;
        if self.guard.attempt(client, Scope::All, now).is_err() {
            self.audit_failure("sign_in", "code", client, now, LOCKED_OUT);
            return SignInReply::TooMany;
        }
        // The password is checked before the code, and whenever one is asked for, so the time taken says nothing about the code.
        let password = self.screen_password();
        if !client.is_local() && password.is_set() && !password.verify(req.password.unwrap_or("")) {
            self.audit_failure("sign_in", "code", client, now, WRONG_PASSWORD);
            return SignInReply::Refused;
        }
        match self
            .signin()
            .exchange(req.code, now, &format!("code {}", client.describe()))
        {
            Ok(t) => {
                self.guard.succeeded(client, Scope::All);
                SignInReply::Token(t)
            }
            Err(SignInError::BadCode) => {
                self.audit_failure("sign_in", "code", client, now, WRONG_CODE);
                SignInReply::Refused
            }
            Err(_) => SignInReply::Failed,
        }
    }

    fn end(&self, bearer: &str) -> u16 {
        let now = crate::server::now_ms();
        let signin = self.signin();
        let Some(access) = signin.access(bearer, now) else {
            return 401;
        };
        match signin.end(bearer) {
            Ok(_) => {
                let _ = crate::audit::Audit::new(self.hub.store()).record_in(
                    &access.session,
                    now,
                    &access.user,
                    "sign_out",
                    "",
                    "ok",
                );
                200
            }
            Err(_) => 500,
        }
    }

    fn account(
        &self,
        op: &str,
        bearer: Option<&str>,
        client: &Client,
        body: &serde_json::Value,
    ) -> (u16, String) {
        let now = crate::server::now_ms();
        let Some(token) = bearer else {
            return (401, String::new());
        };
        let Some(access) = self.signin().access(token, now) else {
            return (401, String::new());
        };
        let text = |k: &str| body.get(k).and_then(|v| v.as_str());
        let password = self.screen_password();
        let audit = |action: &str, outcome: &str| {
            let _ = crate::audit::Audit::new(self.hub.store()).record_in(
                &access.session,
                now,
                &access.user,
                action,
                &client.describe(),
                outcome,
            );
        };
        // The current password, when one is set: rate limited like a sign-in.
        let proves = |action: &str, candidate: Option<&str>| -> Result<(), (u16, String)> {
            if self.guard.attempt(client, Scope::All, now).is_err() {
                audit(action, LOCKED_OUT);
                return Err((429, String::new()));
            }
            if !password.verify(candidate.unwrap_or("")) {
                audit(action, WRONG_PASSWORD);
                return Err((401, String::new()));
            }
            self.guard.succeeded(client, Scope::All);
            Ok(())
        };
        match op {
            // Setting or changing the screen password: the owner only, with the current one when there is one; every other
            // session ends.
            "password" => {
                if access.role != crate::users::Role::Owner {
                    return (403, String::new());
                }
                let Some(new) = text("password") else {
                    return (400, String::new());
                };
                if let Err(e) = crate::password::check_new(new) {
                    let code = match e {
                        PasswordError::TooLong => "password.too_long",
                        _ => "password.too_short",
                    };
                    return (400, serde_json::json!({ "code": code }).to_string());
                }
                if password.is_set() {
                    if let Err(e) = proves("password_set", text("current")) {
                        return e;
                    }
                }
                if password.set(new).is_err() {
                    return (500, String::new());
                }
                let revoked = self.signin().revoke_others(token).unwrap_or(0);
                let _ = crate::password::drop_unvouched_passkeys(self.hub.store());
                audit("password_set", "ok");
                (200, serde_json::json!({ "revoked": revoked }).to_string())
            }
            _ => (404, String::new()),
        }
    }
}

impl HubSessions {
    /// The screen password for a step that a passkey could otherwise skip later: asked of a client on another machine when one is
    /// set, rate limited and audited like a sign-in. `Ok(true)` when the step is vouched for (the hub's own machine, or the right
    /// password); `Ok(false)` when no password is set and the client is elsewhere (such a passkey is dropped when a password is set).
    fn remote_password(
        &self,
        client: &Client,
        body: &serde_json::Value,
        now: u64,
    ) -> Result<bool, (u16, String)> {
        if client.is_local() {
            return Ok(true);
        }
        let password = self.screen_password();
        if !password.is_set() {
            return Ok(false);
        }
        if self.guard.attempt(client, Scope::All, now).is_err() {
            self.audit_failure("passkey_add", "password", client, now, LOCKED_OUT);
            return Err((429, String::new()));
        }
        let candidate = body.get("password").and_then(|v| v.as_str()).unwrap_or("");
        if !password.verify(candidate) {
            self.audit_failure("passkey_add", "password", client, now, WRONG_PASSWORD);
            return Err((401, String::new()));
        }
        self.guard.succeeded(client, Scope::All);
        Ok(true)
    }

    fn passkey_op(
        &self,
        op: &str,
        bearer: Option<&str>,
        host: &str,
        client: &Client,
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
        // Who is asking: a passkey belongs to the person whose session added it (ADR-0049), never to someone else.
        let who = bearer
            .and_then(|t| self.signin().access(t, now))
            .map(|a| (a.user != "owner").then_some(a.user));
        let signed_in = who.is_some();
        let json = |v: serde_json::Value| (200, v.to_string());
        match op {
            // Adding a passkey needs a signed-in session: the code (or another passkey) proved who is here. From another machine,
            // with a screen password set, it needs the password too: a passkey replaces the password, so a session token alone
            // (taken from a window) must not be able to make one (ADR-0067).
            "register-begin" | "register-finish" if signed_in => {
                let vouched = match self.remote_password(client, body, now) {
                    Ok(v) => v,
                    Err(e) => return e,
                };
                if op == "register-begin" {
                    return match self.challenges.issue(Purpose::Register, now) {
                        Ok(c) => {
                            json(serde_json::json!({ "challenge": c, "rpId": pk::rp_id(host) }))
                        }
                        Err(_) => (500, String::new()),
                    };
                }
                let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("");
                match (field("publicKey"), field("clientDataJSON")) {
                    (Some(key), Some(cd)) => {
                        match pk::register(
                            store,
                            &self.challenges,
                            host,
                            id,
                            &key,
                            &cd,
                            now,
                            who.as_ref().and_then(|u| u.as_deref()),
                        ) {
                            Ok(()) => {
                                if vouched {
                                    let _ = crate::password::vouch_passkey(store, id);
                                }
                                (200, "{}".into())
                            }
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
            // A passkey assertion replaces the screen password for that device (ADR-0067): only the per-address limit applies,
            // so the owner can still get in while the global cap holds.
            "login-finish" => {
                let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let (Some(auth), Some(cd), Some(sig)) = (
                    field("authenticatorData"),
                    field("clientDataJSON"),
                    field("signature"),
                ) else {
                    return (400, String::new());
                };
                if self.guard.attempt(client, Scope::AddressOnly, now).is_err() {
                    self.audit_failure("sign_in", "passkey", client, now, LOCKED_OUT);
                    return (429, String::new());
                }
                match pk::login(store, &self.challenges, host, id, &auth, &cd, &sig, now) {
                    Ok(user) => match self.signin().start_session_from(
                        user.as_deref(),
                        now,
                        &format!("passkey {}", client.describe()),
                    ) {
                        Ok(t) => {
                            self.guard.succeeded(client, Scope::AddressOnly);
                            json(serde_json::json!({ "token": t }))
                        }
                        Err(_) => (500, String::new()),
                    },
                    Err(pk::PasskeyError::Refused) => {
                        self.audit_failure(
                            "sign_in",
                            "passkey",
                            client,
                            now,
                            "signin.passkey_refused",
                        );
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
        self.signin().access(token, crate::server::now_ms())
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
    fn using_a_session_extends_it() {
        let store = SqliteStore::open_in_memory().unwrap();
        let s = si(&store);
        let code = s.create_code(0).unwrap();
        let token = s.redeem(&code, 1_000).unwrap();
        // Used after more than half of its term: it runs a full term again from then.
        let later = SESSION_TTL_MS * 3 / 4;
        assert!(s.check(&token, later));
        assert!(
            s.check(&token, SESSION_TTL_MS + 2_000),
            "alive after its first deadline: it was extended"
        );
        // Not used again: it ends one full term after that use, not at its first deadline, and not later.
        assert!(
            !s.check(&token, later + SESSION_TTL_MS + 5_000),
            "but not for ever"
        );
        let code = s.create_code(later).unwrap();
        let other = s.redeem(&code, later).unwrap();
        assert!(
            s.check(&other, later + SESSION_TTL_MS - 1_000),
            "a fresh session lasts a full term"
        );
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
    fn each_session_has_its_own_id_in_the_audit_trail() {
        // L16: two windows of the same person are told apart by their session's short id, on the sign-in row and on what they do.
        let store = SqliteStore::open_in_memory().unwrap();
        let s = si(&store);
        let a = s.redeem(&s.create_code(0).unwrap(), 1_000).unwrap();
        let b = s.start_session(1_100).unwrap();
        let (ida, idb) = (
            s.access(&a, 2_000).unwrap().session,
            s.access(&b, 2_000).unwrap().session,
        );
        assert_eq!(ida.len(), 6);
        assert!(ida.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(ida, idb);
        assert!(!a.contains(&ida), "the id is not part of the token");
        let audit = crate::audit::Audit::new(&store);
        audit
            .record_in(&ida, 3_000, "owner", "AgentStop", "n1", "sent")
            .unwrap();
        let rows = audit.list(0, None, 10).unwrap();
        let sessions: Vec<(&str, &str)> = rows
            .iter()
            .map(|r| (r.action.as_str(), r.session.as_str()))
            .collect();
        assert_eq!(
            sessions,
            [
                ("sign_in", ida.as_str()),
                ("sign_in", idb.as_str()),
                ("AgentStop", ida.as_str())
            ]
        );
        let conn = store.lock().unwrap();
        assert!(
            conn.execute(
                "UPDATE audit SET session = 'x' WHERE action = 'AgentStop'",
                []
            )
            .is_err(),
            "the session of a row is as fixed as the rest of it"
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

    mod screen_password {
        use super::super::*;
        use super::si;
        use crate::password::{Client, ScreenPassword, ADDRESS_FREE_FAILURES};
        use crate::web::{SessionApi, SignInReply, SignInRequest};

        const PW: &str = "correct horse battery";

        fn hub() -> (Arc<crate::hub::Hub>, HubSessions) {
            let hub =
                Arc::new(crate::hub::Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap());
            let api = HubSessions::new(Arc::clone(&hub)).with_password_rounds(1_000);
            (hub, api)
        }

        fn code(hub: &crate::hub::Hub) -> String {
            si(hub.store())
                .create_code(crate::server::now_ms())
                .unwrap()
        }

        fn away(last: u8) -> Client {
            Client::new(
                Some([203, 0, 113, last].into()),
                "Mozilla/5.0 (Android 15) Firefox/140.0",
            )
        }

        fn home() -> Client {
            Client::new(Some("127.0.0.1".parse().unwrap()), "")
        }

        fn sign_in(
            api: &HubSessions,
            code: &str,
            password: Option<&str>,
            client: &Client,
        ) -> SignInReply {
            api.sign_in(&SignInRequest {
                code,
                password,
                client,
            })
        }

        fn token(r: SignInReply) -> Option<String> {
            match r {
                SignInReply::Token(t) => Some(t),
                _ => None,
            }
        }

        #[test]
        fn without_a_password_a_code_is_enough_from_anywhere() {
            let (hub, api) = hub();
            assert_eq!(
                api.info(&away(1)),
                serde_json::json!({"passwordRequired": false, "passwordSet": false})
            );
            assert!(token(sign_in(&api, &code(&hub), None, &away(1))).is_some());
            assert!(token(sign_in(&api, &code(&hub), None, &home())).is_some());
        }

        #[test]
        fn another_machine_needs_the_password_and_the_hubs_own_does_not() {
            let (hub, api) = hub();
            ScreenPassword::with_iterations(hub.store(), 1_000)
                .set(PW)
                .unwrap();
            assert_eq!(api.info(&away(1))["passwordRequired"], true);
            assert_eq!(api.info(&home())["passwordRequired"], false);
            assert_eq!(api.info(&home())["passwordSet"], true);
            let c = code(&hub);
            assert!(matches!(
                sign_in(&api, &c, None, &away(1)),
                SignInReply::Refused
            ));
            assert!(matches!(
                sign_in(&api, &c, Some("wrong one!"), &away(1)),
                SignInReply::Refused
            ));
            // A wrong password does not spend the code: the owner can type it again.
            let t = token(sign_in(&api, &c, Some(PW), &away(1))).unwrap();
            assert!(api.signin().check(&t, crate::server::now_ms()));
            // The owner at the machine is never asked.
            assert!(token(sign_in(&api, &code(&hub), None, &home())).is_some());
            // The right password with a wrong code is refused the same way.
            assert!(matches!(
                sign_in(&api, "AAAA-AAAA", Some(PW), &away(2)),
                SignInReply::Refused
            ));
            // Every step is in the audit trail with where it came from; the password never is.
            let rows = crate::audit::Audit::new(hub.store())
                .list(0, None, 50)
                .unwrap();
            let seen: Vec<(String, String)> = rows
                .iter()
                .map(|r| (r.node.clone(), r.outcome.clone()))
                .collect();
            assert!(
                seen.contains(&(
                    "code 203.0.113.1 firefox/android".into(),
                    "signin.wrong_password".into()
                )),
                "{seen:?}"
            );
            assert!(
                seen.contains(&("code 203.0.113.1 firefox/android".into(), "ok".into())),
                "{seen:?}"
            );
            assert!(
                seen.contains(&("code 127.0.0.1".into(), "ok".into())),
                "{seen:?}"
            );
            assert!(
                seen.contains(&(
                    "code 203.0.113.2 firefox/android".into(),
                    "signin.wrong_code".into()
                )),
                "{seen:?}"
            );
            assert!(!format!("{rows:?}").contains(PW));
        }

        #[test]
        fn guessing_from_one_address_locks_it_out_but_not_the_owner_at_the_machine() {
            let (hub, api) = hub();
            ScreenPassword::with_iterations(hub.store(), 1_000)
                .set(PW)
                .unwrap();
            let c = code(&hub);
            for _ in 0..ADDRESS_FREE_FAILURES {
                assert!(matches!(
                    sign_in(&api, &c, Some("guess guess"), &away(9)),
                    SignInReply::Refused
                ));
            }
            // Locked: even the right password and code are refused, as "too many".
            assert!(matches!(
                sign_in(&api, &c, Some(PW), &away(9)),
                SignInReply::TooMany
            ));
            assert!(
                token(sign_in(&api, &c, Some(PW), &away(10))).is_some(),
                "another address"
            );
            assert!(
                token(sign_in(&api, &code(&hub), None, &home())).is_some(),
                "the owner"
            );
            let rows = crate::audit::Audit::new(hub.store())
                .list(0, None, 50)
                .unwrap();
            assert!(rows.iter().any(|r| r.outcome == "signin.locked_out"));
        }

        #[test]
        fn changing_the_password_needs_the_old_one_and_signs_out_everywhere_else() {
            let (hub, api) = hub();
            let owner = token(sign_in(&api, &code(&hub), None, &home())).unwrap();
            let other = token(sign_in(&api, &code(&hub), None, &away(3))).unwrap();
            let set = |tok: Option<&str>, body: serde_json::Value| {
                api.account("password", tok, &away(4), &body)
            };
            assert_eq!(set(None, serde_json::json!({"password": PW})).0, 401);
            assert_eq!(
                set(Some(&owner), serde_json::json!({"password": "short"})),
                (400, r#"{"code":"password.too_short"}"#.into())
            );
            // No password yet: none to give.
            assert_eq!(
                set(Some(&owner), serde_json::json!({"password": PW})).0,
                200
            );
            let now = crate::server::now_ms();
            assert!(
                api.signin().check(&owner, now),
                "this window stays signed in"
            );
            assert!(!api.signin().check(&other, now), "every other session ends");
            // Changing it needs the current one.
            assert_eq!(
                set(
                    Some(&owner),
                    serde_json::json!({"password": "a new password"})
                )
                .0,
                401
            );
            assert_eq!(
                set(
                    Some(&owner),
                    serde_json::json!({"password": "a new password", "current": "nope nope"})
                )
                .0,
                401
            );
            assert_eq!(
                set(
                    Some(&owner),
                    serde_json::json!({"password": "a new password", "current": PW})
                )
                .0,
                200
            );
            assert!(ScreenPassword::new(hub.store()).verify("a new password"));
            let rows = crate::audit::Audit::new(hub.store())
                .list(0, None, 50)
                .unwrap();
            assert_eq!(
                rows.iter()
                    .filter(|r| r.action == "password_set" && r.outcome == "ok")
                    .count(),
                2
            );
            assert!(rows
                .iter()
                .any(|r| r.action == "password_set" && r.outcome == "signin.wrong_password"));
            // Only the owner may.
            let member = crate::users::Users::new(hub.store())
                .add("Eda", crate::users::Role::Member, 1)
                .unwrap();
            let m = api
                .signin()
                .redeem(
                    &api.signin().create_code_for(Some(&member.id), now).unwrap(),
                    now,
                )
                .unwrap();
            assert_eq!(
                set(
                    Some(&m),
                    serde_json::json!({"password": "x2345678", "current": "a new password"})
                )
                .0,
                403
            );
        }

        #[test]
        fn locking_ends_the_session_and_leaves_machines_alone() {
            let (hub, api) = hub();
            let tok = token(sign_in(&api, &code(&hub), None, &home())).unwrap();
            // Locking or signing out ends a window's session only: machines keep their own credentials and their agents run on.
            hub.store()
                .lock()
                .unwrap()
                .execute(
                    "INSERT INTO devices (serial, node_id, enrolled_ms) VALUES ('01', 'laptop', 1)",
                    [],
                )
                .unwrap();
            assert_eq!(api.end(&tok), 200);
            assert!(crate::enroll::Enrollment::new(hub.store())
                .is_active("01")
                .unwrap());
            assert!(!api.signin().check(&tok, crate::server::now_ms()));
            assert_eq!(api.end(&tok), 401);
            let rows = crate::audit::Audit::new(hub.store())
                .list(0, None, 50)
                .unwrap();
            assert!(rows.iter().any(|r| r.action == "sign_out"));
        }

        fn register(api: &HubSessions, tok: &str, client: &Client, body: serde_json::Value) -> u16 {
            api.passkey("register-begin", Some(tok), "hub.example", client, &body)
                .0
        }

        #[test]
        fn a_session_alone_adds_no_passkey_from_elsewhere_once_a_password_is_set() {
            let (hub, api) = hub();
            let local = token(sign_in(&api, &code(&hub), None, &home())).unwrap();
            // No password yet: unchanged.
            assert_eq!(register(&api, &local, &away(6), serde_json::json!({})), 200);
            ScreenPassword::with_iterations(hub.store(), 1_000)
                .set(PW)
                .unwrap();
            // The token was taken at the machine, used from the LAN: the password is needed, and misses count.
            assert_eq!(register(&api, &local, &away(6), serde_json::json!({})), 401);
            assert_eq!(
                register(
                    &api,
                    &local,
                    &away(6),
                    serde_json::json!({"password": "nope nope"})
                ),
                401
            );
            assert_eq!(
                register(&api, &local, &away(6), serde_json::json!({"password": PW})),
                200
            );
            assert_eq!(
                register(&api, &local, &home(), serde_json::json!({})),
                200,
                "the hub's own machine is not asked"
            );
            for _ in 0..ADDRESS_FREE_FAILURES {
                let _ = register(
                    &api,
                    &local,
                    &away(7),
                    serde_json::json!({"password": "guess guess"}),
                );
            }
            assert_eq!(
                register(&api, &local, &away(7), serde_json::json!({"password": PW})),
                429
            );
            let rows = crate::audit::Audit::new(hub.store())
                .list(0, None, 50)
                .unwrap();
            assert!(rows
                .iter()
                .any(|r| r.action == "passkey_add" && r.outcome == WRONG_PASSWORD));
        }

        #[test]
        fn a_password_change_keeps_only_passkeys_that_never_skipped_it() {
            let (hub, api) = hub();
            let add = |id: &str| {
                hub.store()
                    .lock()
                    .unwrap()
                    .execute(
                        "INSERT INTO passkeys (id, public_key, sign_count, created_ms) VALUES (?1, x'00', 0, 1)",
                        [id],
                    )
                    .unwrap();
            };
            add("from-elsewhere-before");
            add("at-the-machine");
            crate::password::vouch_passkey(hub.store(), "at-the-machine").unwrap();
            let owner = token(sign_in(&api, &code(&hub), None, &home())).unwrap();
            assert_eq!(
                api.account(
                    "password",
                    Some(&owner),
                    &home(),
                    &serde_json::json!({"password": PW})
                )
                .0,
                200
            );
            assert_eq!(
                crate::passkey::ids(hub.store()).unwrap(),
                ["at-the-machine"]
            );
        }
    }
}
