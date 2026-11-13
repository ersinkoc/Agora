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

use rusqlite::params;

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

    fn stored(&self) -> Option<String> {
        self.store
            .setting(SETTING)
            .ok()
            .flatten()
            .and_then(|v| String::from_utf8(v).ok())
    }

    pub fn is_set(&self) -> bool {
        self.stored().is_some()
    }

    /// Sets (or replaces) the password.
    pub fn set(&self, password: &str) -> Result<(), PasswordError> {
        check_new(password)?;
        let hashed = hash_with(password, self.iterations)?;
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
        self.stored().is_some_and(|s| verify(&s, candidate))
    }
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
    fn bucket(&self) -> IpAddr {
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
}

#[derive(Debug, Default)]
struct GuardState {
    by_address: HashMap<IpAddr, Tally>,
    global: Tally,
}

/// Rate limits for everything that proves who is signing in (codes, the password, passkeys): per address and for the whole hub.
/// A try is counted when it starts ([`Guard::attempt`]) and given back when it succeeds, so many tries sent at once cannot all
/// pass the check before the first failure is counted.
#[derive(Debug, Default)]
pub struct Guard {
    state: Mutex<GuardState>,
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
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
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
        s.by_address
            .entry(key)
            .or_default()
            .count(now_ms, ADDRESS_FREE_FAILURES, cap, FORGET_MS);
        if !local {
            s.global.count(
                now_ms,
                GLOBAL_FREE_FAILURES,
                MAX_LOCKOUT_MS,
                GLOBAL_FORGET_MS,
            );
        }
        Ok(())
    }

    /// The try that [`Self::attempt`] started proved who is here: the address starts again from zero.
    pub fn succeeded(&self, client: &Client) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.by_address.remove(&client.bucket());
        if !client.is_local() {
            s.global.failures = s.global.failures.saturating_sub(1);
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
        let stored = hash_with("abcdefgh", 20_000).unwrap();
        let time = |c: &str| {
            let t = std::time::Instant::now();
            for _ in 0..5 {
                assert!(!verify(&stored, c));
            }
            t.elapsed().as_secs_f64()
        };
        let (near, far) = (time("abcdefgX"), time("Xbcdefgh"));
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
        g.succeeded(&c);
        for _ in 0..ADDRESS_FREE_FAILURES {
            assert_eq!(g.attempt(&c, Scope::All, now), Ok(()));
        }
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
}
