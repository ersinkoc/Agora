//! The screen password (ADR-0067 rung 1): one password of the owner that a new sign-in from another machine must give, on top of
//! its one-time code. The owner on the hub's own machine (a loopback peer) is never asked.
//!
//! - Stored only as a salted, slow hash: PBKDF2-HMAC-SHA256 (`ring`), [`ITERATIONS`] rounds, a random 16-byte salt per password.
//!   The stored text names its round count, so a hash made with another count still verifies. Compared in constant time.
//! - Guessing is rate limited by [`Guard`]: per client address (an IPv6 address by its /64) and for the whole hub, with a
//!   lockout that doubles up to an hour. The loopback address is never locked for longer than a minute and never by the global
//!   cap, so an attacker elsewhere cannot lock the owner out of their own machine.
//! - The password is never logged and never returned; only "is one set" leaves the hub.

use std::collections::HashMap;
use std::net::IpAddr;
use std::num::NonZeroU32;
use std::sync::Mutex;

use rusqlite::{params, OptionalExtension};

use crate::store::{EventStore, SqliteStore, StoreError};

/// PBKDF2 rounds for a new password (OWASP's figure for PBKDF2-HMAC-SHA256).
pub const ITERATIONS: u32 = 600_000;
/// Shortest password accepted, in characters.
pub const MIN_CHARS: usize = 8;
/// Longest password accepted, in bytes: hashing has a cost and a request body has a limit.
pub const MAX_BYTES: usize = 1024;

const SALT_LEN: usize = 16;
const HASH_LEN: usize = 32;
const SETTING: &str = "screen_password";
const SCHEME: &str = "pbkdf2-sha256";

static ALGORITHM: ring::pbkdf2::Algorithm = ring::pbkdf2::PBKDF2_HMAC_SHA256;

#[derive(Debug, PartialEq, Eq)]
pub enum PasswordError {
    /// Fewer than [`MIN_CHARS`] characters (or nothing at all).
    TooShort,
    /// More than [`MAX_BYTES`] bytes.
    TooLong,
    Store(String),
}

impl From<StoreError> for PasswordError {
    fn from(e: StoreError) -> Self {
        Self::Store(e.to_string())
    }
}

impl From<rusqlite::Error> for PasswordError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Store(e.to_string())
    }
}

/// Is `password` acceptable as a new screen password?
pub fn check_new(password: &str) -> Result<(), PasswordError> {
    if password.len() > MAX_BYTES {
        return Err(PasswordError::TooLong);
    }
    if password.trim().is_empty() || password.chars().count() < MIN_CHARS {
        return Err(PasswordError::TooShort);
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

/// The stored form of `password`: `pbkdf2-sha256$<rounds>$<salt hex>$<hash hex>`.
pub fn hash_with(password: &str, iterations: u32) -> Result<String, PasswordError> {
    let rounds = NonZeroU32::new(iterations).ok_or(PasswordError::Store("rounds".into()))?;
    let mut salt = [0u8; SALT_LEN];
    getrandom::getrandom(&mut salt).map_err(|e| PasswordError::Store(e.to_string()))?;
    let mut out = [0u8; HASH_LEN];
    ring::pbkdf2::derive(ALGORITHM, rounds, &salt, password.as_bytes(), &mut out);
    Ok(format!(
        "{SCHEME}${iterations}${}${}",
        hex(&salt),
        hex(&out)
    ))
}

/// Does `candidate` match the stored form `stored`? Constant time in the candidate (ring's verify); a malformed `stored` matches
/// nothing.
pub fn verify(stored: &str, candidate: &str) -> bool {
    let mut parts = stored.split('$');
    let (Some(SCHEME), Some(rounds), Some(salt), Some(hash), None) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return false;
    };
    let (Some(rounds), Some(salt), Some(hash)) = (
        rounds.parse().ok().and_then(NonZeroU32::new),
        unhex(salt),
        unhex(hash),
    ) else {
        return false;
    };
    if candidate.len() > MAX_BYTES {
        return false;
    }
    ring::pbkdf2::verify(ALGORITHM, rounds, &salt, candidate.as_bytes(), &hash).is_ok()
}

/// The owner's screen password in the hub's store (a setting; no table of its own).
pub struct ScreenPassword<'a> {
    store: &'a SqliteStore,
    iterations: u32,
}

impl<'a> ScreenPassword<'a> {
    pub fn new(store: &'a SqliteStore) -> Self {
        Self::with_iterations(store, ITERATIONS)
    }

    /// As [`Self::new`] with another round count for new passwords. Only tests use it: hashing 600 000 rounds in a debug build is
    /// slow, and a stored hash carries its own count.
    pub fn with_iterations(store: &'a SqliteStore, iterations: u32) -> Self {
        Self { store, iterations }
    }

    pub(crate) fn stored(&self) -> Result<Option<String>, PasswordError> {
        let Some(v) = self.store.setting(SETTING)? else {
            return Ok(None);
        };
        String::from_utf8(v)
            .map(Some)
            .map_err(|e| PasswordError::Store(e.to_string()))
    }

    pub fn is_set(&self) -> bool {
        !matches!(self.stored(), Ok(None))
    }

    /// Sets (or replaces) the password.
    pub fn set(&self, password: &str) -> Result<(), PasswordError> {
        check_new(password)?;
        let hashed = hash_with(password, self.iterations)?;
        self.set_hash(&hashed)
    }

    pub(crate) fn set_hash(&self, hashed: &str) -> Result<(), PasswordError> {
        self.store.set_setting(SETTING, hashed.as_bytes())?;
        Ok(())
    }

    /// Removes the password: sign-ins from elsewhere need only their code again.
    pub fn clear(&self) -> Result<bool, PasswordError> {
        let n = self
            .store
            .lock()?
            .execute("DELETE FROM settings WHERE key = ?1", params![SETTING])?;
        Ok(n > 0)
    }

    /// Does `candidate` match? `false` when no password is set.
    pub fn verify(&self, candidate: &str) -> bool {
        matches!(self.stored(), Ok(Some(s)) if verify(&s, candidate))
    }
}

const VOUCHED: &str = "passkeys_vouched";

/// Records that the passkey `id` was added where the screen password would not have been asked (the hub's own machine) or was
/// given. Such passkeys outlive a password change; every other one does not ([`drop_unvouched_passkeys`]).
pub fn vouch_passkey(store: &SqliteStore, id: &str) -> Result<(), PasswordError> {
    let mut conn = store.lock()?;
    let tx = crate::store::write_tx(&mut conn)?;
    set_vouch_in(&tx, id, true)?;
    tx.commit()?;
    Ok(())
}

/// Forgets the vouch for `id` when no passkey has that id (its passkey went with its person): ids are chosen by the client, so a
/// passkey added later under the same id without the password must not inherit it ([`drop_unvouched_passkeys`]).
pub fn forget_stale_vouch(store: &SqliteStore, id: &str) -> Result<(), PasswordError> {
    let mut conn = store.lock()?;
    let tx = crate::store::write_tx(&mut conn)?;
    let exists = tx
        .query_row("SELECT 1 FROM passkeys WHERE id = ?1", [id], |_| Ok(()))
        .optional()?
        .is_some();
    if !exists {
        set_vouch_in(&tx, id, false)?;
    }
    tx.commit()?;
    Ok(())
}

/// Within the caller's transaction: `id` vouched for (`true`) or not (`false`) in the list of vouched passkeys. An unreadable list
/// is an error, never an empty one.
pub(crate) fn set_vouch_in(
    conn: &rusqlite::Connection,
    id: &str,
    vouched: bool,
) -> Result<(), PasswordError> {
    let mut ids = vouched_in(conn)?;
    let has = ids.iter().any(|v| v == id);
    if vouched && !has {
        ids.push(id.to_string());
    } else if !vouched && has {
        ids.retain(|v| v != id);
    } else {
        return Ok(());
    }
    write_vouched_in(conn, &ids)
}

fn vouched_in(conn: &rusqlite::Connection) -> Result<Vec<String>, PasswordError> {
    let raw: Option<Vec<u8>> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [VOUCHED],
            |r| r.get(0),
        )
        .optional()?;
    let Some(v) = raw else {
        return Ok(Vec::new());
    };
    let text = String::from_utf8(v).map_err(|e| PasswordError::Store(e.to_string()))?;
    Ok(text
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect())
}

fn write_vouched_in(conn: &rusqlite::Connection, ids: &[String]) -> Result<(), PasswordError> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![VOUCHED, ids.join("\n").as_bytes()],
    )?;
    Ok(())
}

/// Removes every passkey that was added from another machine without the screen password (when none was set, or before this
/// rule existed): setting, changing or clearing the password must not leave a way in that never knew it. Returns how many.
pub fn drop_unvouched_passkeys(store: &SqliteStore) -> Result<usize, PasswordError> {
    let mut conn = store.lock()?;
    let tx = crate::store::write_tx(&mut conn)?;
    let dropped = drop_unvouched_in(&tx)?;
    tx.commit()?;
    Ok(dropped)
}

/// [`drop_unvouched_passkeys`] within the caller's transaction.
fn drop_unvouched_in(conn: &rusqlite::Connection) -> Result<usize, PasswordError> {
    let keep = vouched_in(conn)?;
    let ids: Vec<String> = conn
        .prepare("SELECT id FROM passkeys")?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let mut dropped = 0;
    for id in ids.iter().filter(|id| !keep.contains(id)) {
        dropped += conn.execute("DELETE FROM passkeys WHERE id = ?1", params![id])?;
    }
    let still: Vec<String> = keep.into_iter().filter(|k| ids.contains(k)).collect();
    write_vouched_in(conn, &still)?;
    Ok(dropped)
}

/// A password change as one transaction (Codex review of #1027): the new hash is stored, every session but the one whose token
/// hash is `keep_session` ends, and every unvouched passkey goes, or nothing changes. The hash is derived before, outside it.
/// Returns how many sessions ended.
pub(crate) fn commit_change(
    store: &SqliteStore,
    hashed: &str,
    keep_session: &[u8],
) -> Result<usize, PasswordError> {
    commit(store, Some(hashed), Some(keep_session)).map(|revoked| revoked.unwrap_or(0))
}

/// `agora password set` (#1029): stores the already derived `hashed`, ends every session and drops every unvouched passkey in one
/// write transaction, or changes nothing. Returns how many sessions ended.
pub fn set_and_sign_out_all(store: &SqliteStore, hashed: &str) -> Result<usize, PasswordError> {
    commit(store, Some(hashed), None).map(|revoked| revoked.unwrap_or(0))
}

/// `agora password clear` (#1029): removes the password, ends every session and drops every unvouched passkey in one write
/// transaction, or changes nothing. `None` when no password was set: then nothing changes either.
pub fn clear_and_sign_out_all(store: &SqliteStore) -> Result<Option<usize>, PasswordError> {
    commit(store, None, None)
}

/// The password becomes `hashed` (or none), the sessions end (all, or all but `keep_session`) and the unvouched passkeys go, in one
/// write transaction. `None`, with nothing changed, when clearing and no password was set.
fn commit(
    store: &SqliteStore,
    hashed: Option<&str>,
    keep_session: Option<&[u8]>,
) -> Result<Option<usize>, PasswordError> {
    let mut conn = store.lock()?;
    let tx = crate::store::write_tx(&mut conn)?;
    match hashed {
        Some(hashed) => {
            tx.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![SETTING, hashed.as_bytes()],
            )?;
        }
        None => {
            if tx.execute("DELETE FROM settings WHERE key = ?1", params![SETTING])? == 0 {
                return Ok(None);
            }
        }
    }
    let revoked = match keep_session {
        Some(keep) => tx.execute("DELETE FROM sessions WHERE token_hash != ?1", params![keep])?,
        None => tx.execute("DELETE FROM sessions", [])?,
    };
    drop_unvouched_in(&tx)?;
    tx.commit()?;
    Ok(Some(revoked))
}

/// Who is asking, as far as the hub can tell: the TCP peer address and a short summary of the user agent, for rate limits and the
/// audit trail.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Client {
    pub addr: Option<IpAddr>,
    pub user_agent: String,
}

impl Client {
    pub fn new(addr: Option<IpAddr>, user_agent: &str) -> Self {
        Self {
            addr,
            user_agent: agent_summary(user_agent),
        }
    }

    /// On the hub's own machine (a loopback peer, IPv4 or IPv6, also as an IPv4-mapped IPv6 address). An unknown address is not.
    pub fn is_local(&self) -> bool {
        self.addr.is_some_and(|a| match a {
            IpAddr::V4(v4) => v4.is_loopback(),
            IpAddr::V6(v6) => {
                v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
            }
        })
    }

    /// What the audit trail names: `<address> <browser>/<system>`.
    pub fn describe(&self) -> String {
        let addr = self.addr.map_or_else(|| "-".to_string(), |a| a.to_string());
        if self.user_agent.is_empty() {
            addr
        } else {
            format!("{addr} {}", self.user_agent)
        }
    }

    /// The bucket a rate limit counts in: an IPv6 address by its /64 (one host usually holds a whole one).
    pub(crate) fn bucket(&self) -> IpAddr {
        match self.addr {
            None => IpAddr::from([0u8, 0, 0, 0]),
            Some(IpAddr::V6(v6)) => match v6.to_ipv4_mapped() {
                Some(v4) => IpAddr::V4(v4),
                None => {
                    let s = v6.segments();
                    IpAddr::from([s[0], s[1], s[2], s[3], 0, 0, 0, 0])
                }
            },
            Some(v4) => v4,
        }
    }
}

/// A short, fixed-vocabulary summary of a `User-Agent` header (`firefox/linux`): enough to tell devices apart in the audit trail,
/// nothing the sender can inject.
pub fn agent_summary(ua: &str) -> String {
    let browser = [
        ("Edg/", "edge"),
        ("OPR/", "opera"),
        ("Firefox/", "firefox"),
        ("Chrome/", "chrome"),
        ("Safari/", "safari"),
        ("curl/", "curl"),
    ]
    .iter()
    .find(|(k, _)| ua.contains(k))
    .map(|(_, v)| *v);
    let system = [
        ("Android", "android"),
        ("iPhone", "ios"),
        ("iPad", "ios"),
        ("Windows", "windows"),
        ("Mac OS", "macos"),
        ("CrOS", "chromeos"),
        ("Linux", "linux"),
    ]
    .iter()
    .find(|(k, _)| ua.contains(k))
    .map(|(_, v)| *v);
    match (browser, system) {
        (None, None) if ua.is_empty() => String::new(),
        (b, s) => format!("{}/{}", b.unwrap_or("other"), s.unwrap_or("other")),
    }
}

/// Free tries from one address before it is locked out.
pub const ADDRESS_FREE_FAILURES: u32 = 5;
/// Free failures from all other addresses together before every address but loopback is locked out.
pub const GLOBAL_FREE_FAILURES: u32 = 50;
/// The first lockout; each further failure doubles it.
pub const FIRST_LOCKOUT_MS: u64 = 60 * 1000;
/// The longest lockout.
pub const MAX_LOCKOUT_MS: u64 = 60 * 60 * 1000;
/// The longest lockout of the loopback address: the owner on the hub's machine waits at most this long.
pub const LOCAL_MAX_LOCKOUT_MS: u64 = 60 * 1000;
/// A count with no failure for this long starts again from zero.
const FORGET_MS: u64 = 24 * 60 * 60 * 1000;
const GLOBAL_FORGET_MS: u64 = 60 * 60 * 1000;
/// Addresses remembered at most; the oldest is forgotten first.
const MAX_TRACKED: usize = 4096;

#[derive(Debug, Default, Clone, Copy)]
struct Tally {
    failures: u32,
    until_ms: u64,
    last_ms: u64,
}

impl Tally {
    fn locked(&self, now_ms: u64) -> Option<u64> {
        (now_ms < self.until_ms).then(|| self.until_ms - now_ms)
    }

    /// Counts one more try; locks when the free tries are spent.
    fn count(&mut self, now_ms: u64, free: u32, cap_ms: u64, forget_ms: u64) {
        if now_ms.saturating_sub(self.last_ms) > forget_ms {
            self.failures = 0;
        }
        self.failures = self.failures.saturating_add(1);
        self.last_ms = now_ms;
        if self.failures >= free {
            let doublings = (self.failures - free).min(16);
            let ms = FIRST_LOCKOUT_MS
                .saturating_mul(1u64 << doublings)
                .min(cap_ms);
            self.until_ms = now_ms.saturating_add(ms);
        }
    }

    /// Recomputes the deadline after one provisional failure was given back.
    fn recount_lock(&mut self, free: u32, cap_ms: u64) {
        if self.failures < free {
            self.until_ms = 0;
            return;
        }
        let doublings = (self.failures - free).min(16);
        let ms = FIRST_LOCKOUT_MS
            .saturating_mul(1u64 << doublings)
            .min(cap_ms);
        self.until_ms = self.last_ms.saturating_add(ms);
    }
}

#[derive(Debug, Default)]
struct GuardState {
    by_address: HashMap<IpAddr, Tally>,
    global: Tally,
    /// Lock decisions never move backward with the wall clock, so a deadline already observed to end cannot return.
    observed_ms: u64,
}

/// Rate limits for everything that proves who is signing in (codes, the password, passkeys): per address and for the whole hub.
/// A try is counted when it starts ([`Guard::attempt`]) and given back when it succeeds, so many tries sent at once cannot all
/// pass the check before the first failure is counted.
#[derive(Debug, Default)]
pub struct Guard {
    state: Mutex<GuardState>,
}

/// What one counted try changed ([`Guard::attempt_undoable`]), to take it back exactly ([`Guard::undo`]).
#[derive(Debug, Clone, Copy)]
pub struct Undo {
    key: IpAddr,
    prev: Option<Tally>,
    set: Tally,
}

/// How a try stands with the [`Guard`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Per-address and global limits (codes and the password).
    All,
    /// The per-address limit only: a passkey cannot be guessed, and it is how the owner gets in while the global cap holds.
    AddressOnly,
}

impl Guard {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts a try: `Err(retry_after_ms)` while `client` is locked out, else the try is counted as a failure until
    /// [`Self::succeeded`] gives it back.
    pub fn attempt(&self, client: &Client, scope: Scope, now_ms: u64) -> Result<(), u64> {
        self.attempt_undoable(client, scope, now_ms).map(|_| ())
    }

    /// [`Self::attempt`], with what it takes to take this one try back exactly ([`Self::undo`]).
    pub fn attempt_undoable(
        &self,
        client: &Client,
        scope: Scope,
        now_ms: u64,
    ) -> Result<Undo, u64> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let now_ms = now_ms.max(s.observed_ms);
        s.observed_ms = now_ms;
        let local = client.is_local();
        let key = client.bucket();
        let mine = s.by_address.get(&key).and_then(|t| t.locked(now_ms));
        let global = if local || scope == Scope::AddressOnly {
            None
        } else {
            s.global.locked(now_ms)
        };
        if let Some(wait) = mine.into_iter().chain(global).max() {
            return Err(wait);
        }
        if !s.by_address.contains_key(&key) && s.by_address.len() >= MAX_TRACKED {
            let oldest = s
                .by_address
                .iter()
                .min_by_key(|(_, t)| t.last_ms)
                .map(|(k, _)| *k);
            if let Some(k) = oldest {
                s.by_address.remove(&k);
            }
        }
        let cap = if local {
            LOCAL_MAX_LOCKOUT_MS
        } else {
            MAX_LOCKOUT_MS
        };
        let prev = s.by_address.get(&key).copied();
        let tally = s.by_address.entry(key).or_default();
        tally.count(now_ms, ADDRESS_FREE_FAILURES, cap, FORGET_MS);
        let undo = Undo {
            key,
            prev,
            set: *tally,
        };
        // A passkey cannot be guessed: its tries never count toward the hub-wide cap (nor can its successes lower it).
        if !local && scope == Scope::All {
            s.global.count(
                now_ms,
                GLOBAL_FREE_FAILURES,
                MAX_LOCKOUT_MS,
                GLOBAL_FORGET_MS,
            );
        }
        Ok(undo)
    }

    /// Takes back exactly the one try [`Self::attempt_undoable`] counted, for a try that proved a credential which is not a
    /// sign-in (a metrics token): the address's count and lockout go back to what they were before that try, in neither
    /// direction changed by it. What other tries added meanwhile stays: only this try's own count is removed, and a lockout
    /// is put back only while it is still the one this try set, so a valid token cannot be interleaved with wrong guesses to
    /// wipe their count, nor lock the address by its own try.
    pub fn undo(&self, undo: Undo) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(t) = s.by_address.get_mut(&undo.key) else {
            return;
        };
        let untouched = t.failures == undo.set.failures
            && t.until_ms == undo.set.until_ms
            && t.last_ms == undo.set.last_ms;
        if untouched {
            // Nothing else happened to the address meanwhile: exactly the state before this try.
            match undo.prev {
                Some(prev) => *t = prev,
                None => {
                    s.by_address.remove(&undo.key);
                }
            }
            return;
        }
        // Another try counted meanwhile: only this try's own count goes, and a lockout only while it is still the one this
        // try set.
        t.failures = t.failures.saturating_sub(1);
        if t.until_ms == undo.set.until_ms {
            t.until_ms = undo.prev.unwrap_or_default().until_ms;
        }
    }

    /// The try that [`Self::attempt`] started (with the same `scope`) proved who is here: the address starts again from zero, and
    /// a code or password try is given back to the hub-wide count.
    pub fn succeeded(&self, client: &Client, scope: Scope) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.by_address.remove(&client.bucket());
        if !client.is_local() && scope == Scope::All {
            s.global.failures = s.global.failures.saturating_sub(1);
            s.global.recount_lock(GLOBAL_FREE_FAILURES, MAX_LOCKOUT_MS);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn remote(last: u8) -> Client {
        Client::new(Some(IpAddr::from([203, 0, 113, last])), "")
    }

    #[test]
    fn a_hash_round_trips_and_names_its_rounds_and_salt() {
        let a = hash_with("correct horse", 1_000).unwrap();
        let b = hash_with("correct horse", 1_000).unwrap();
        assert!(a.starts_with("pbkdf2-sha256$1000$"));
        assert_ne!(a, b, "a random salt per password");
        assert!(!a.contains("correct horse"));
        assert!(verify(&a, "correct horse"));
        assert!(!verify(&a, "correct horsf"));
        assert!(!verify(&a, ""));
        assert!(
            !verify("pbkdf2-sha256$0$00$00", "x"),
            "zero rounds is malformed"
        );
        assert!(!verify("garbage", "garbage"));
        assert!(!verify(&format!("{a}$extra"), "correct horse"));
        assert!(!verify(&a, &"x".repeat(MAX_BYTES + 1)));
    }

    #[test]
    fn the_default_round_count_is_the_adrs() {
        assert_eq!(ITERATIONS, 600_000);
        let store = SqliteStore::open_in_memory().unwrap();
        assert_eq!(ScreenPassword::new(&store).iterations, ITERATIONS);
    }

    #[test]
    fn verifying_takes_the_same_time_for_a_near_miss_and_a_far_miss() {
        // ring compares the derived keys with a constant-time compare; this checks only that a miss at the first byte does not return
        // much sooner than one at the last (a loose bound: the derivation dominates either way).
        // #1150: timed one verification at a time, the two kinds taking turns, and each kind judged by its fastest run. A stall of the
        // machine (another test, a busy runner) only ever adds time, and it lands on one run, not on one kind as a whole: when each
        // kind was timed once, as one block after the other, a single stall in one block put it past the bound.
        let stored = hash_with("abcdefgh", 20_000).unwrap();
        let once = |c: &str| {
            let t = std::time::Instant::now();
            assert!(!verify(&stored, c));
            t.elapsed().as_secs_f64()
        };
        // Warmed up first, so neither kind pays for the first run's cold caches.
        once("abcdefgX");
        once("Xbcdefgh");
        let (mut near, mut far) = (f64::INFINITY, f64::INFINITY);
        for _ in 0..9 {
            near = near.min(once("abcdefgX"));
            far = far.min(once("Xbcdefgh"));
        }
        assert!(far > near * 0.5 && near > far * 0.5, "{near} vs {far}");
    }

    #[test]
    fn short_empty_and_huge_passwords_are_refused() {
        assert_eq!(check_new(""), Err(PasswordError::TooShort));
        assert_eq!(check_new("        "), Err(PasswordError::TooShort));
        assert_eq!(check_new("seven77"), Err(PasswordError::TooShort));
        assert_eq!(check_new("çok güzel"), Ok(()));
        assert_eq!(
            check_new(&"x".repeat(MAX_BYTES + 1)),
            Err(PasswordError::TooLong)
        );
    }

    #[test]
    fn the_store_keeps_only_the_hash_and_clear_removes_it() {
        let store = SqliteStore::open_in_memory().unwrap();
        let pw = ScreenPassword::with_iterations(&store, 1_000);
        assert!(!pw.is_set());
        assert!(!pw.verify(""), "no password matches nothing");
        pw.set("hunter2hunter2").unwrap();
        assert!(pw.is_set());
        assert!(pw.verify("hunter2hunter2"));
        assert!(!pw.verify("hunter2"));
        let raw = store.setting(SETTING).unwrap().unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("hunter2"));
        assert_eq!(pw.set("short"), Err(PasswordError::TooShort));
        assert!(
            pw.verify("hunter2hunter2"),
            "a refused change keeps the old one"
        );
        assert!(pw.clear().unwrap());
        assert!(!pw.is_set());
        assert!(!pw.clear().unwrap());
    }

    #[test]
    fn local_addresses_and_summaries() {
        assert!(Client::new(Some("127.0.0.1".parse().unwrap()), "").is_local());
        assert!(Client::new(Some("::1".parse().unwrap()), "").is_local());
        assert!(Client::new(Some("::ffff:127.0.0.1".parse().unwrap()), "").is_local());
        assert!(!Client::new(Some("192.168.1.4".parse().unwrap()), "").is_local());
        assert!(!Client::new(None, "").is_local(), "unknown is not local");
        assert_eq!(
            agent_summary("Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0"),
            "firefox/linux"
        );
        assert_eq!(
            agent_summary("Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1"),
            "safari/ios"
        );
        assert_eq!(agent_summary("<script>"), "other/other");
        assert_eq!(agent_summary(""), "");
        assert_eq!(
            Client::new(Some("10.0.0.2".parse().unwrap()), "curl/8.0").describe(),
            "10.0.0.2 curl/other"
        );
    }

    #[test]
    fn an_address_is_locked_out_for_longer_each_time_up_to_an_hour() {
        let g = Guard::new();
        let c = remote(1);
        let mut now = 0;
        for _ in 0..ADDRESS_FREE_FAILURES {
            assert_eq!(g.attempt(&c, Scope::All, now), Ok(()));
        }
        assert_eq!(g.attempt(&c, Scope::All, now), Err(FIRST_LOCKOUT_MS));
        assert_eq!(
            g.attempt(&remote(2), Scope::All, now),
            Ok(()),
            "another address is not locked"
        );
        let mut waits = vec![];
        for _ in 0..10 {
            now += MAX_LOCKOUT_MS + 1;
            assert_eq!(g.attempt(&c, Scope::All, now), Ok(()));
            waits.push(g.attempt(&c, Scope::All, now).unwrap_err());
        }
        assert_eq!(
            &waits[..3],
            &[
                2 * FIRST_LOCKOUT_MS,
                4 * FIRST_LOCKOUT_MS,
                8 * FIRST_LOCKOUT_MS
            ]
        );
        assert_eq!(*waits.last().unwrap(), MAX_LOCKOUT_MS);
        // A success starts the address again from zero.
        now += MAX_LOCKOUT_MS + 1;
        assert_eq!(g.attempt(&c, Scope::All, now), Ok(()));
        g.succeeded(&c, Scope::All);
        for _ in 0..ADDRESS_FREE_FAILURES {
            assert_eq!(g.attempt(&c, Scope::All, now), Ok(()));
        }
    }

    #[test]
    fn f383_an_ended_password_lockout_does_not_return_after_clock_rollback() {
        let guard = Guard::new();
        let client = remote(1);
        for _ in 0..ADDRESS_FREE_FAILURES {
            assert_eq!(guard.attempt(&client, Scope::AddressOnly, 0), Ok(()));
        }
        assert_eq!(
            guard.attempt(&client, Scope::AddressOnly, 1),
            Err(FIRST_LOCKOUT_MS - 1)
        );
        assert_eq!(guard.attempt(&remote(2), Scope::AddressOnly, 1), Ok(()));
        println!("CONTROL OK");

        let after = FIRST_LOCKOUT_MS + 1;
        let valid = guard
            .attempt_undoable(&client, Scope::AddressOnly, after)
            .unwrap();
        guard.undo(valid);
        let actual = guard.attempt(&client, Scope::AddressOnly, 1);
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
    fn verifies_f383_password_lockout_deadline_boundaries() {
        let locked = |last| {
            let guard = Guard::new();
            let client = remote(last);
            for _ in 0..ADDRESS_FREE_FAILURES {
                guard.attempt(&client, Scope::AddressOnly, 10).unwrap();
            }
            (guard, client)
        };
        let observe_valid = |guard: &Guard, client: &Client, now| {
            let undo = guard
                .attempt_undoable(client, Scope::AddressOnly, now)
                .unwrap();
            guard.undo(undo);
        };

        let (active, client) = locked(1);
        assert_eq!(
            active.attempt(&client, Scope::AddressOnly, 10 + FIRST_LOCKOUT_MS - 1),
            Err(1)
        );

        let (exact, client) = locked(2);
        observe_valid(&exact, &client, 10 + FIRST_LOCKOUT_MS);
        assert!(exact.attempt(&client, Scope::AddressOnly, 10).is_ok());

        let (later, client) = locked(3);
        observe_valid(&later, &client, 10 + FIRST_LOCKOUT_MS + 500);
        assert!(later.attempt(&client, Scope::AddressOnly, 11).is_ok());
        println!("FIX VERIFIED");
    }

    #[test]
    fn an_ipv6_host_is_one_address_however_many_it_uses() {
        let g = Guard::new();
        for i in 0..ADDRESS_FREE_FAILURES {
            let a: IpAddr = format!("2001:db8:1:2::{}", i + 1).parse().unwrap();
            assert_eq!(g.attempt(&Client::new(Some(a), ""), Scope::All, 0), Ok(()));
        }
        let other: IpAddr = "2001:db8:1:2:ffff::9".parse().unwrap();
        assert!(g
            .attempt(&Client::new(Some(other), ""), Scope::All, 0)
            .is_err());
    }

    #[test]
    fn the_global_cap_locks_everyone_but_loopback_and_passkeys() {
        let g = Guard::new();
        let mut last = 0u8;
        let mut failures = 0;
        while failures < GLOBAL_FREE_FAILURES {
            last += 1;
            for _ in 0..2 {
                assert_eq!(g.attempt(&remote(last), Scope::All, 0), Ok(()));
                failures += 1;
            }
        }
        let fresh = remote(250);
        assert_eq!(g.attempt(&fresh, Scope::All, 1), Err(FIRST_LOCKOUT_MS - 1));
        assert_eq!(
            g.attempt(&fresh, Scope::AddressOnly, 1),
            Ok(()),
            "a passkey"
        );
        let owner = Client::new(Some("127.0.0.1".parse().unwrap()), "");
        assert_eq!(
            g.attempt(&owner, Scope::All, 1),
            Ok(()),
            "the owner on the machine"
        );
    }

    #[test]
    fn f423_a_success_at_the_global_threshold_does_not_leave_a_lockout() {
        let g = Guard::new();
        for last in 1..GLOBAL_FREE_FAILURES {
            assert_eq!(g.attempt(&remote(last as u8), Scope::All, 0), Ok(()));
        }
        assert_eq!(g.attempt(&remote(200), Scope::AddressOnly, 0), Ok(()));
        println!("CONTROL OK");

        let successful = remote(201);
        assert_eq!(g.attempt(&successful, Scope::All, 0), Ok(()));
        g.succeeded(&successful, Scope::All);
        let actual = g.attempt(&remote(202), Scope::All, 1);
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
    fn verifies_f423_success_recomputes_the_global_deadline() {
        let fill = |failures: u32| {
            let guard = Guard::new();
            for last in 1..=failures {
                guard.attempt(&remote(last as u8), Scope::All, 10).unwrap();
            }
            guard
        };

        let below = fill(GLOBAL_FREE_FAILURES - 2);
        let c = remote(200);
        below.attempt(&c, Scope::All, 10).unwrap();
        below.succeeded(&c, Scope::All);
        assert!(below.attempt(&remote(201), Scope::All, 11).is_ok());

        let threshold = fill(GLOBAL_FREE_FAILURES - 1);
        let c = remote(202);
        threshold.attempt(&c, Scope::All, 10).unwrap();
        threshold.succeeded(&c, Scope::All);
        assert!(threshold.attempt(&remote(203), Scope::All, 11).is_ok());

        let locked = fill(GLOBAL_FREE_FAILURES);
        assert_eq!(
            locked.attempt(&remote(204), Scope::All, 11),
            Err(FIRST_LOCKOUT_MS - 1)
        );
        println!("FIX VERIFIED");
    }

    #[test]
    fn loopback_is_never_locked_for_more_than_a_minute() {
        let g = Guard::new();
        let owner = Client::new(Some("::1".parse().unwrap()), "");
        let mut now = 0;
        for _ in 0..40 {
            match g.attempt(&owner, Scope::All, now) {
                Ok(()) => {}
                Err(wait) => {
                    assert!(wait <= LOCAL_MAX_LOCKOUT_MS);
                    now += wait;
                }
            }
        }
    }

    #[test]
    fn passkey_tries_neither_fill_nor_drain_the_hub_wide_cap() {
        let g = Guard::new();
        // Many passkey misses from many addresses: the global cap is untouched.
        for last in 0..100u8 {
            for _ in 0..ADDRESS_FREE_FAILURES {
                assert_eq!(g.attempt(&remote(last), Scope::AddressOnly, 0), Ok(()));
            }
            assert!(
                g.attempt(&remote(last), Scope::AddressOnly, 0).is_err(),
                "the per-address limit still holds for passkeys"
            );
        }
        assert_eq!(g.attempt(&remote(200), Scope::All, 0), Ok(()));
        // Fill the cap with code misses; passkey successes do not give any of it back.
        let mut n = 1;
        let mut last = 0u8;
        'fill: loop {
            last += 1;
            for _ in 0..4 {
                if n >= GLOBAL_FREE_FAILURES {
                    break 'fill;
                }
                assert_eq!(
                    g.attempt(
                        &Client::new(Some([198, 51, 100, last].into()), ""),
                        Scope::All,
                        0
                    ),
                    Ok(())
                );
                n += 1;
            }
        }
        for i in 0..20u8 {
            let c = Client::new(Some([192, 0, 2, i].into()), "");
            assert_eq!(g.attempt(&c, Scope::AddressOnly, 0), Ok(()));
            g.succeeded(&c, Scope::AddressOnly);
        }
        assert!(
            g.attempt(&remote(201), Scope::All, 0).is_err(),
            "still locked"
        );
    }

    /// Review of #668: a valid metrics try after an earlier lockout has expired changes nothing in the address's bucket, so the
    /// next valid try is not refused by a lockout the valid one made itself.
    #[test]
    fn a_valid_try_after_an_expired_lockout_changes_nothing() {
        let g = Guard::new();
        let c = remote(77);
        for _ in 0..ADDRESS_FREE_FAILURES {
            assert_eq!(g.attempt(&c, Scope::AddressOnly, 0), Ok(()));
        }
        assert!(g.attempt(&c, Scope::AddressOnly, 1).is_err(), "locked");
        let snapshot = |g: &Guard| {
            let s = g.state.lock().unwrap();
            let t = s.by_address.get(&c.bucket()).copied().unwrap();
            (t.failures, t.until_ms, t.last_ms)
        };
        let before = snapshot(&g);
        let after_lockout = FIRST_LOCKOUT_MS + 1;
        for _ in 0..3 {
            let undo = g
                .attempt_undoable(&c, Scope::AddressOnly, after_lockout)
                .expect("the lockout has ended");
            g.undo(undo);
            assert_eq!(snapshot(&g), before, "a valid try changed the bucket");
        }
        // A wrong try still counts and locks again.
        assert_eq!(g.attempt(&c, Scope::AddressOnly, after_lockout), Ok(()));
        assert!(g
            .attempt(&c, Scope::AddressOnly, after_lockout + 1)
            .is_err());
    }

    /// A valid try for an address with no failures leaves no trace, and one that overlaps another try keeps that try's count.
    #[test]
    fn a_valid_try_leaves_no_trace_and_keeps_what_overlapped_it() {
        let g = Guard::new();
        let c = remote(78);
        let undo = g.attempt_undoable(&c, Scope::AddressOnly, 0).unwrap();
        g.undo(undo);
        assert!(g.state.lock().unwrap().by_address.is_empty());
        let first = g.attempt_undoable(&c, Scope::AddressOnly, 0).unwrap();
        assert_eq!(g.attempt(&c, Scope::AddressOnly, 0), Ok(()));
        g.undo(first);
        let s = g.state.lock().unwrap();
        assert_eq!(s.by_address.get(&c.bucket()).unwrap().failures, 1);
    }

    #[test]
    fn drop_unvouched_passkeys_propagates_store_error_without_deleting_passkeys() {
        let store = SqliteStore::open_in_memory().unwrap();
        {
            let conn = store.lock().unwrap();
            conn.execute(
                "INSERT INTO passkeys (id, public_key, sign_count, created_ms, user_id) VALUES ('pk-v', X'01', 0, 1, NULL)",
                [],
            )
            .unwrap();
        }
        vouch_passkey(&store, "pk-v").unwrap();
        store
            .lock()
            .unwrap()
            .execute("DROP TABLE settings", [])
            .unwrap();
        assert!(drop_unvouched_passkeys(&store).is_err());
        let count: i64 = store
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM passkeys WHERE id = 'pk-v'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
    }

    /// A store with a password, two sessions, a vouched and an unvouched passkey, for the #1029 tests.
    fn seeded() -> SqliteStore {
        let store = SqliteStore::open_in_memory().unwrap();
        ScreenPassword::with_iterations(&store, 1_000)
            .set("old screen password")
            .unwrap();
        {
            let conn = store.lock().unwrap();
            conn.execute_batch(
                "INSERT INTO sessions (token_hash, created_ms, expires_ms) VALUES (x'aa', 1, 9999999999999), (x'bb', 1, 9999999999999);
                 INSERT INTO passkeys (id, public_key, sign_count, created_ms) VALUES ('pk-v', x'01', 0, 1), ('pk-u', x'02', 0, 1);",
            )
            .unwrap();
        }
        vouch_passkey(&store, "pk-v").unwrap();
        store
    }

    fn counts(store: &SqliteStore) -> (i64, Vec<String>) {
        let conn = store.lock().unwrap();
        let sessions = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
            .unwrap();
        let passkeys = conn
            .prepare("SELECT id FROM passkeys ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        (sessions, passkeys)
    }

    /// Makes the last step of a change, writing the list of vouched passkeys, fail: after the hash, the sessions and the passkeys.
    fn fail_the_vouched_write(store: &SqliteStore) {
        store
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_vouched_update BEFORE UPDATE ON settings WHEN NEW.key = 'passkeys_vouched'
                 BEGIN SELECT RAISE(ABORT, 'injected'); END;
                 CREATE TRIGGER fail_vouched_insert BEFORE INSERT ON settings WHEN NEW.key = 'passkeys_vouched'
                 BEGIN SELECT RAISE(ABORT, 'injected'); END;",
            )
            .unwrap();
    }

    /// #1029: `agora password set` stores the hash, ends every session and drops the unvouched passkeys together.
    #[test]
    fn set_and_sign_out_all_ends_every_session_and_drops_unvouched_passkeys() {
        let store = seeded();
        let hashed = hash_with("new screen password", 1_000).unwrap();
        assert_eq!(set_and_sign_out_all(&store, &hashed), Ok(2));
        assert!(ScreenPassword::new(&store).verify("new screen password"));
        assert_eq!(counts(&store), (0, vec!["pk-v".to_string()]));
    }

    /// #1029: a `set` that fails at its last step changes nothing: the old password, the sessions and the passkeys stay.
    #[test]
    fn a_set_that_fails_part_way_changes_nothing() {
        let store = seeded();
        fail_the_vouched_write(&store);
        let hashed = hash_with("new screen password", 1_000).unwrap();
        assert!(matches!(
            set_and_sign_out_all(&store, &hashed),
            Err(PasswordError::Store(_))
        ));
        let screen = ScreenPassword::new(&store);
        assert!(screen.verify("old screen password"));
        assert!(!screen.verify("new screen password"));
        assert_eq!(
            counts(&store),
            (2, vec!["pk-u".to_string(), "pk-v".to_string()])
        );
    }

    /// #1029: `agora password clear` removes the password, ends every session and drops the unvouched passkeys together; with no
    /// password set it changes nothing.
    #[test]
    fn clear_and_sign_out_all_ends_every_session_and_does_nothing_without_a_password() {
        let store = seeded();
        assert_eq!(clear_and_sign_out_all(&store), Ok(Some(2)));
        assert!(!ScreenPassword::new(&store).is_set());
        assert_eq!(counts(&store), (0, vec!["pk-v".to_string()]));
        store
            .lock()
            .unwrap()
            .execute_batch(
                "INSERT INTO sessions (token_hash, created_ms, expires_ms) VALUES (x'cc', 1, 9999999999999);
                 INSERT INTO passkeys (id, public_key, sign_count, created_ms) VALUES ('pk-u', x'02', 0, 1);",
            )
            .unwrap();
        assert_eq!(clear_and_sign_out_all(&store), Ok(None));
        assert_eq!(
            counts(&store),
            (1, vec!["pk-u".to_string(), "pk-v".to_string()])
        );
    }

    /// #1029: a `clear` that fails at its last step changes nothing: the password, the sessions and the passkeys stay.
    #[test]
    fn a_clear_that_fails_part_way_changes_nothing() {
        let store = seeded();
        fail_the_vouched_write(&store);
        assert!(matches!(
            clear_and_sign_out_all(&store),
            Err(PasswordError::Store(_))
        ));
        assert!(ScreenPassword::new(&store).verify("old screen password"));
        assert_eq!(
            counts(&store),
            (2, vec!["pk-u".to_string(), "pk-v".to_string()])
        );
    }
}
