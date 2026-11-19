//! The node renews its own certificate (ADR-0085, step 2): the files it keeps in the link folder, the order it writes them in, how a
//! stopped process finds its way back, and the timer that asks the hub.
//!
//! The link folder holds `url`, `node-id`, `ca.pem`, `cert.pem` and `key.pem` (written by enrollment or pairing). A renewal adds three
//! files, all private to this user and each written whole (a temporary file renamed over the target):
//!
//! - `renew-pending.pem`: the fresh key and its request. **Written before the request is sent** and kept until the answer is stored,
//!   so a crash or a lost answer is met by sending the SAME request again (the hub answers it with the same certificate, nothing new
//!   issued).
//! - `previous.pem`: the certificate and key that were in use before the new pair was written; removed once a connection made with the
//!   new certificate was accepted, so a pair is never given up before its replacement has worked once.
//! - `renew-state`: what the hub last said when it refused (`join_again`, `superseded`, `too_frequent`, `rejected`), for `agora doctor`.
//!
//! The order of an install (see [`install`]) keeps the folder usable at every point: after each step either the old pair or the new
//! pair can be read as a whole, or [`recover`] makes it so (it runs under the folder's guard whenever the link is loaded).
//!
//! Nothing here holds a lock that agents or the supervisor use, and nothing waits on the network while holding the folder's guard.
//! Agents never depend on any of it: a renewal that cannot finish leaves the node as it was.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::cert_renew::{check_renewed, due, pair_matches, validity, Pending, Refusal};
use crate::uplink_client::{CertSource, HubLink, LinkError, RenewError};

/// The fresh key and request of a renewal that has not been answered yet.
pub const PENDING: &str = "renew-pending.pem";
/// The pair that was in use before the last renewal, until the new one has connected.
pub const PREVIOUS: &str = "previous.pem";
/// What the hub last refused, for `agora doctor`.
pub const STATE: &str = "renew-state";

/// The capability a hub offers when it handles renewals.
pub const CAPABILITY: &str = "enroll.renew";

/// What the hub said, kept as one short file next to the certificate it was said about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    /// `join_again`, `superseded`, `too_frequent` or `rejected`.
    pub code: String,
    /// The `reason` the hub gave (or why the answer was rejected): a stable word, shown only as a parameter.
    pub detail: String,
    /// When the certificate this was said about ends: a state about another certificate (the machine joined again) is ignored.
    pub ends: i64,
    /// Seconds since 1970 before which no new try is made; 0 for a refusal that is never retried.
    pub retry_at: i64,
}

impl State {
    fn encode(&self) -> String {
        format!(
            "{}\n{}\n{}\n{}\n",
            self.code, self.detail, self.ends, self.retry_at
        )
    }

    fn decode(text: &str) -> Option<Self> {
        let mut l = text.lines();
        Some(Self {
            code: l.next()?.to_string(),
            detail: l.next()?.to_string(),
            ends: l.next()?.parse().ok()?,
            retry_at: l.next()?.parse().ok()?,
        })
    }
}

/// The state recorded in `dir` for the certificate that ends at `ends` (none for a state about another certificate).
pub fn state(dir: &Path, ends: i64) -> Option<State> {
    let s = State::decode(&std::fs::read_to_string(dir.join(STATE)).ok()?)?;
    (s.ends == ends).then_some(s)
}

/// Whether a renewal was started in `dir` and its answer is not stored yet.
pub fn pending_exists(dir: &Path) -> bool {
    dir.join(PENDING).exists()
}

/// The file operations a renewal makes, so tests can stop between any two of them as a crash would.
pub trait Ops: Send + Sync {
    /// Writes `bytes` to `path`, readable by this user only, whole or not at all.
    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()>;
    /// Removes `path`; a file that is not there is not an error.
    fn remove(&self, path: &Path) -> io::Result<()>;
}

/// The real operations: the platform's private atomic write.
pub struct Real;

impl Ops for Real {
    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        agora_platform::write_private(path, bytes)
    }

    fn remove(&self, path: &Path) -> io::Result<()> {
        match std::fs::remove_file(path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

fn read(dir: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(dir.join(name)).ok()
}

/// The pair of a `previous.pem`: the certificate, then the key.
fn split_pair(text: &str) -> Option<(String, String)> {
    let at = text.find("-----BEGIN")?;
    let end = text.find("-----END CERTIFICATE-----")? + "-----END CERTIFICATE-----".len();
    if at > end {
        return None;
    }
    Some((
        text[at..end].trim().to_string(),
        text[end..].trim().to_string(),
    ))
}

/// Puts the new certificate and its key in place of the pair in use, in an order that leaves the folder readable whole after every
/// step: (1) the pair in use is copied to `previous.pem`, (2) the new certificate is written over `cert.pem`, (3) the key of the
/// request over `key.pem`, (4) `renew-pending.pem` is dropped, (5) `renew-state` is dropped. Between 2 and 3 the files do not match:
/// [`recover`] finishes the install from `renew-pending.pem`, which is still there. Each write is a temporary file renamed over its
/// target, so no step leaves half a file. The caller holds the folder's guard.
pub fn install(
    dir: &Path,
    ops: &dyn Ops,
    in_use: (&str, &str),
    new_cert_pem: &str,
    pending: &Pending,
) -> io::Result<()> {
    ops.write(
        &dir.join(PREVIOUS),
        format!("{}\n{}\n", in_use.0.trim_end(), in_use.1.trim_end()).as_bytes(),
    )?;
    ops.write(&dir.join("cert.pem"), new_cert_pem.as_bytes())?;
    ops.write(
        &dir.join("key.pem"),
        format!("{}\n", pending.key_pem).as_bytes(),
    )?;
    ops.remove(&dir.join(PENDING))?;
    ops.remove(&dir.join(STATE))
}

/// Brings the folder to a state where `cert.pem` and `key.pem` are one pair, after a stop between the steps of [`install`] (the caller
/// holds the guard):
/// - the certificate belongs to the pending key but the key file does not: the install is finished (the key is written, the pending
///   file dropped);
/// - the pair matches and the pending key is the one in use: the pending file is a leftover and is dropped;
/// - nothing matches and `previous.pem` holds a whole pair: that pair is put back.
///
/// Returns whether anything was changed. Does nothing for a folder whose files already match.
pub fn recover(dir: &Path, ops: &dyn Ops) -> bool {
    let (Some(cert), Some(key)) = (read(dir, "cert.pem"), read(dir, "key.pem")) else {
        return false;
    };
    let pending = read(dir, PENDING).and_then(|t| Pending::decode(&t));
    let matches = pair_matches(&cert, &key);
    if let Some(p) = &pending {
        if pair_matches(&cert, &p.key_pem) {
            // The new certificate is in place. Finish: the key, then drop what is left of the renewal.
            let ok = matches
                || ops
                    .write(&dir.join("key.pem"), format!("{}\n", p.key_pem).as_bytes())
                    .is_ok();
            if ok {
                let _ = ops.remove(&dir.join(PENDING));
                let _ = ops.remove(&dir.join(STATE));
            }
            return true;
        }
    }
    if matches {
        return false;
    }
    // A mismatch the pending key does not explain: go back to the pair that was in use, if it is whole.
    if let Some((c, k)) = read(dir, PREVIOUS).and_then(|t| split_pair(&t)) {
        if pair_matches(&c, &k) {
            let _ = ops.write(&dir.join("cert.pem"), c.as_bytes());
            let _ = ops.write(&dir.join("key.pem"), format!("{k}\n").as_bytes());
            return true;
        }
    }
    false
}

/// How long the timer waits in each case.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// Between looks at the certificate when nothing is due (an hour).
    pub check_every: Duration,
    /// Between looks while the hub has not said it handles renewals (it may not be connected yet, or it is an older hub).
    pub unknown_every: Duration,
    /// After a refusal `too_frequent` (an hour: the hub's own limit).
    pub too_frequent: Duration,
    /// After the first failure to reach the hub, doubling up to `retry_max`.
    pub retry_first: Duration,
    pub retry_max: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            check_every: Duration::from_secs(3600),
            unknown_every: Duration::from_secs(15),
            too_frequent: Duration::from_secs(3600),
            retry_first: Duration::from_secs(60),
            retry_max: Duration::from_secs(3600),
        }
    }
}

/// What one look did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// No link, or the certificate has more than a third of its life left.
    Idle,
    /// Due, but the hub has not said it handles renewals.
    WaitingForHub,
    /// A new certificate was stored; the next connection uses it.
    Renewed,
    /// The hub refused (or a refusal from earlier still holds): the code of the state recorded.
    Refused(String),
    /// The hub could not be reached, answered something else, or the answer was rejected: tried again after a wait.
    Failed,
}

type Fresh = dyn Fn() -> Option<(String, String)> + Send + Sync;
type Ask = dyn Fn(&HubLink, &str) -> Result<String, RenewError> + Send + Sync;
type Clock = dyn Fn() -> i64 + Send + Sync;
type Hold = dyn Fn() -> io::Result<Box<dyn std::any::Any + Send>> + Send + Sync;

/// The renewal of one link folder.
pub struct Renewer {
    dir: PathBuf,
    timing: Timing,
    /// Whether the hub's last welcome listed [`CAPABILITY`].
    hub_offers: AtomicBool,
    /// Consecutive failures to reach the hub, for the backoff.
    fails: AtomicU32,
    last_log: Mutex<String>,
    /// The lines logged so far (the last 100), for the tests of what is and is not said.
    logged: Mutex<Vec<String>>,
    fresh: Box<Fresh>,
    ask: Box<Ask>,
    clock: Box<Clock>,
    hold: Box<Hold>,
    ops: Arc<dyn Ops>,
}

fn secs_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// What a look found in the folder, read under its guard.
struct Snapshot {
    node_id: String,
    link: HubLink,
    pending: Option<Pending>,
}

impl Renewer {
    /// A renewer of `dir` that makes keys with `fresh` (a key and its request, as PEM texts) and takes the folder's guard with `hold`
    /// (dropping what it returns lets go). Uses the real clock, connection and files.
    pub fn new(
        dir: PathBuf,
        fresh: impl Fn() -> Option<(String, String)> + Send + Sync + 'static,
        hold: impl Fn() -> io::Result<Box<dyn std::any::Any + Send>> + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            dir,
            timing: Timing::default(),
            hub_offers: AtomicBool::new(false),
            fails: AtomicU32::new(0),
            last_log: Mutex::new(String::new()),
            logged: Mutex::new(Vec::new()),
            fresh: Box::new(fresh),
            ask: Box::new(crate::uplink_client::renew),
            clock: Box::new(secs_now),
            hold: Box::new(hold),
            ops: Arc::new(Real),
        })
    }

    /// For tests: the waits, the connection, the clock and the file operations replaced.
    #[allow(clippy::too_many_arguments)]
    pub fn with_parts(
        dir: PathBuf,
        timing: Timing,
        fresh: impl Fn() -> Option<(String, String)> + Send + Sync + 'static,
        hold: impl Fn() -> io::Result<Box<dyn std::any::Any + Send>> + Send + Sync + 'static,
        ask: impl Fn(&HubLink, &str) -> Result<String, RenewError> + Send + Sync + 'static,
        clock: impl Fn() -> i64 + Send + Sync + 'static,
        ops: Arc<dyn Ops>,
    ) -> Arc<Self> {
        Arc::new(Self {
            dir,
            timing,
            hub_offers: AtomicBool::new(false),
            fails: AtomicU32::new(0),
            last_log: Mutex::new(String::new()),
            logged: Mutex::new(Vec::new()),
            fresh: Box::new(fresh),
            ask: Box::new(ask),
            clock: Box::new(clock),
            hold: Box::new(hold),
            ops,
        })
    }

    /// Says whether the hub handles renewals (what [`CertSource::welcomed`] does).
    pub fn set_hub_offers(&self, offers: bool) {
        self.hub_offers.store(offers, Ordering::Release);
    }

    /// Log lines go to the node's log, once per change: a hub that stays away does not repeat itself every minute.
    fn log(&self, line: String) {
        let mut last = self.last_log.lock().unwrap_or_else(|e| e.into_inner());
        if *last != line {
            eprintln!("{line}");
            let mut all = self.logged.lock().unwrap_or_else(|e| e.into_inner());
            if all.len() >= 100 {
                all.remove(0);
            }
            all.push(line.clone());
            *last = line;
        }
    }

    /// What this renewer has logged.
    pub fn log_lines(&self) -> Vec<String> {
        self.logged
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// The link folder's files, read whole under its guard (recovering a stopped install first). `None`: no link here, or the guard
    /// or a file could not be had.
    fn snapshot(&self) -> Option<Snapshot> {
        let _guard = (self.hold)().ok()?;
        recover(&self.dir, self.ops.as_ref());
        let node_id = read(&self.dir, "node-id")?.trim().to_string();
        let link = HubLink {
            url: read(&self.dir, "url")?.trim().to_string(),
            ca_pem: read(&self.dir, "ca.pem")?,
            cert_pem: read(&self.dir, "cert.pem")?,
            key_pem: read(&self.dir, "key.pem")?,
            commands: None,
            mesh_out: None,
            grants: None,
            mesh: None,
            devices: None,
            state_dir: None,
            certs: None,
        };
        let pending = read(&self.dir, PENDING).and_then(|t| Pending::decode(&t));
        Some(Snapshot {
            node_id,
            link,
            pending,
        })
    }

    fn write_state(&self, s: &State) {
        let _ = self.ops.write(&self.dir.join(STATE), s.encode().as_bytes());
    }

    /// The wait after a failure to reach the hub: the first wait, doubled each time, up to the longest.
    fn backoff(&self) -> Duration {
        let n = self.fails.fetch_add(1, Ordering::AcqRel).min(16);
        self.timing
            .retry_first
            .saturating_mul(1u32 << n.min(12))
            .min(self.timing.retry_max)
    }

    /// One look at the certificate, and a renewal when it is due. Returns what happened and how long to wait before the next look.
    /// Never waits on the network while holding the folder's guard, and never touches the agents.
    pub fn tick(&self) -> (Outcome, Duration) {
        let now = (self.clock)();
        let Some(snap) = self.snapshot() else {
            return (Outcome::Idle, self.timing.check_every);
        };
        let Some((_, ends)) = validity(&snap.link.cert_pem) else {
            return (Outcome::Idle, self.timing.check_every);
        };
        // What the hub said about this very certificate stands until its time: a refusal that is never retried is not asked again.
        if let Some(s) = state(&self.dir, ends) {
            if s.code == "join_again" || s.code == "superseded" {
                return (Outcome::Refused(s.code), self.timing.check_every);
            }
            if s.retry_at > now {
                let wait = Duration::from_secs(u64::try_from(s.retry_at - now).unwrap_or(1));
                return (Outcome::Refused(s.code), wait.min(self.timing.check_every));
            }
        }
        if snap.pending.is_none() && due(&snap.link.cert_pem, now) != Some(true) {
            return (Outcome::Idle, self.timing.check_every);
        }
        if ends <= now {
            // Past its end the hub refuses the connection itself: nothing to ask. Only joining again helps.
            self.write_state(&State {
                code: "join_again".into(),
                detail: "expired".into(),
                ends,
                retry_at: 0,
            });
            self.log(
                "cert renewal: the certificate has ended; this machine must join again".into(),
            );
            return (
                Outcome::Refused("join_again".into()),
                self.timing.check_every,
            );
        }
        if !self.hub_offers.load(Ordering::Acquire) {
            return (Outcome::WaitingForHub, self.timing.unknown_every);
        }
        // The fresh key and request are written BEFORE they are sent, and kept until the answer is stored.
        let pending = match snap.pending.clone() {
            Some(p) => p,
            None => {
                let Some((key, csr)) = (self.fresh)() else {
                    self.log("cert renewal: a new key could not be made".into());
                    return (Outcome::Failed, self.backoff());
                };
                let p = Pending::new(&key, &csr);
                let stored = (self.hold)().and_then(|_g| {
                    self.ops
                        .write(&self.dir.join(PENDING), p.encode().as_bytes())
                });
                if let Err(e) = stored {
                    self.log(format!(
                        "cert renewal: the new key could not be kept: {}",
                        e.kind()
                    ));
                    return (Outcome::Failed, self.backoff());
                }
                p
            }
        };
        match (self.ask)(&snap.link, &pending.csr_pem) {
            Ok(cert_pem) => self.stored(&snap, &pending, &cert_pem, ends, now),
            Err(RenewError::Refused { code, reason }) if code == "enroll.renew_refused" => {
                self.refused(&snap, &reason, ends, now)
            }
            Err(RenewError::Refused { code, .. }) => {
                self.log(format!("cert renewal: the hub answered {code}"));
                (Outcome::Failed, self.backoff())
            }
            Err(RenewError::Link(e)) => {
                self.log(format!(
                    "cert renewal: the hub was not reached: {}",
                    brief(&e)
                ));
                (Outcome::Failed, self.backoff())
            }
        }
    }

    /// The hub answered with a certificate: check it, then put it in place.
    fn stored(
        &self,
        snap: &Snapshot,
        pending: &Pending,
        cert_pem: &str,
        ends: i64,
        now: i64,
    ) -> (Outcome, Duration) {
        if let Err(why) = check_renewed(
            &snap.link.ca_pem,
            cert_pem,
            &snap.node_id,
            pending,
            &snap.link.cert_pem,
            now,
        ) {
            // The pair in use is kept as it is. The request is kept too: the hub answers it again with the same certificate, and
            // asking anew would only be refused for an hour and then make the one it already issued the newest.
            self.write_state(&State {
                code: "rejected".into(),
                detail: why.code().into(),
                ends,
                retry_at: now + i64::try_from(self.timing.too_frequent.as_secs()).unwrap_or(3600),
            });
            self.log(format!(
                "cert renewal: the hub's answer was rejected: {}",
                why.code()
            ));
            return (Outcome::Failed, self.timing.too_frequent);
        }
        let done = (|| {
            let _guard = (self.hold)()?;
            // The files may have changed while the hub was asked (the machine joined again): this answer is then for a certificate
            // that is no longer in use, and nothing is written.
            let (cert, key) = (
                read(&self.dir, "cert.pem").unwrap_or_default(),
                read(&self.dir, "key.pem").unwrap_or_default(),
            );
            if cert != snap.link.cert_pem {
                self.ops.remove(&self.dir.join(PENDING))?;
                return Ok(false);
            }
            install(
                &self.dir,
                self.ops.as_ref(),
                (&cert, &key),
                cert_pem,
                pending,
            )?;
            Ok::<bool, io::Error>(true)
        })();
        match done {
            Ok(true) => {
                self.fails.store(0, Ordering::Release);
                self.log("cert renewal: a new certificate is in place".into());
                (Outcome::Renewed, self.timing.check_every)
            }
            Ok(false) => (Outcome::Idle, self.timing.check_every),
            Err(e) => {
                // A stop in the middle: the next look recovers the folder from the pending file and sends the same request again.
                self.log(format!(
                    "cert renewal: the new certificate could not be stored: {}",
                    e.kind()
                ));
                (Outcome::Failed, self.backoff())
            }
        }
    }

    /// The hub refused: what it means for this machine.
    fn refused(&self, snap: &Snapshot, reason: &str, ends: i64, now: i64) -> (Outcome, Duration) {
        let drop_pending = || {
            let _ = (self.hold)().map(|_g| self.ops.remove(&self.dir.join(PENDING)));
        };
        match Refusal::from_reason(reason) {
            Refusal::JoinAgain => {
                self.write_state(&State {
                    code: "join_again".into(),
                    detail: reason.into(),
                    ends,
                    retry_at: 0,
                });
                drop_pending();
                self.log(format!(
                    "cert renewal: the hub refused ({reason}); node {} must join again",
                    snap.node_id
                ));
                (
                    Outcome::Refused("join_again".into()),
                    self.timing.check_every,
                )
            }
            Refusal::Superseded => {
                self.write_state(&State {
                    code: "superseded".into(),
                    detail: reason.into(),
                    ends,
                    retry_at: 0,
                });
                drop_pending();
                // Security relevant: a newer certificate of this machine exists that this machine does not have.
                self.log(
                    "cert renewal: SECURITY: this machine's certificate is no longer the newest: another holder may exist".into(),
                );
                (
                    Outcome::Refused("superseded".into()),
                    self.timing.check_every,
                )
            }
            Refusal::TooFrequent => {
                let wait = self.timing.too_frequent;
                self.write_state(&State {
                    code: "too_frequent".into(),
                    detail: reason.into(),
                    ends,
                    retry_at: now + i64::try_from(wait.as_secs()).unwrap_or(3600),
                });
                self.log("cert renewal: the hub asks to wait; trying again later".into());
                (Outcome::Refused("too_frequent".into()), wait)
            }
            Refusal::Other(r) => {
                self.log(format!("cert renewal: the hub refused ({r})"));
                (Outcome::Failed, self.backoff())
            }
        }
    }
}

/// A short stable description of a link error for the log: no certificate or key text is ever in a [`LinkError`].
fn brief(e: &LinkError) -> String {
    match e {
        LinkError::Refused(c) => c.clone(),
        other => format!("{other:?}").chars().take(120).collect(),
    }
}

impl CertSource for Renewer {
    fn current(&self) -> Option<(String, String)> {
        let _guard = (self.hold)().ok()?;
        recover(&self.dir, self.ops.as_ref());
        Some((read(&self.dir, "cert.pem")?, read(&self.dir, "key.pem")?))
    }

    fn welcomed(&self, capabilities: &[String]) {
        self.set_hub_offers(capabilities.iter().any(|c| c == CAPABILITY));
    }

    fn accepted(&self, cert_pem: &str) {
        let Ok(_guard) = (self.hold)() else {
            return;
        };
        // Only a connection made with the certificate now in place lets go of the pair before it.
        if read(&self.dir, "cert.pem").is_some_and(|c| c.trim() == cert_pem.trim()) {
            let _ = self.ops.remove(&self.dir.join(PREVIOUS));
        }
    }
}

/// A running timer. Dropping it stops the thread.
pub struct Timer {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Timer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Looks at the certificate at start and then after each wait [`Renewer::tick`] names, until the returned timer is dropped. A panic in
/// a look is a failed look, not the end of the timer.
pub fn spawn(renewer: Arc<Renewer>) -> io::Result<Timer> {
    let stop = Arc::new(AtomicBool::new(false));
    let thread = {
        let stop = Arc::clone(&stop);
        std::thread::Builder::new()
            .name("agora-cert-renew".into())
            .spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    let wait =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| renewer.tick().1))
                            .unwrap_or_else(|_| renewer.backoff());
                    let end = std::time::Instant::now() + wait;
                    while !stop.load(Ordering::Acquire) {
                        let left = end.saturating_duration_since(std::time::Instant::now());
                        if left.is_zero() {
                            break;
                        }
                        std::thread::sleep(left.min(Duration::from_millis(100)));
                    }
                }
            })?
    };
    Ok(Timer {
        stop,
        thread: Some(thread),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use rcgen::{
        BasicConstraints, CertificateParams, CertificateSigningRequestParams, DistinguishedName,
        DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair, KeyUsagePurpose,
    };
    use std::sync::atomic::AtomicI64;
    use time::{Duration as TimeDuration, OffsetDateTime};

    const DAY: i64 = 86_400;

    fn dn(name: &str) -> DistinguishedName {
        let mut d = DistinguishedName::new();
        d.push(DnType::CommonName, name);
        d
    }

    struct Ca {
        key: KeyPair,
        pem: String,
        params: CertificateParams,
    }

    fn ca() -> Ca {
        let key = KeyPair::generate().unwrap();
        let mut p = CertificateParams::default();
        p.distinguished_name = dn("test ca");
        p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        p.key_usages = vec![KeyUsagePurpose::KeyCertSign];
        let pem = p.self_signed(&key).unwrap().pem();
        Ca {
            key,
            pem,
            params: p,
        }
    }

    fn request() -> (String, String) {
        let key = KeyPair::generate().unwrap();
        let mut p = CertificateParams::default();
        p.distinguished_name = dn("device");
        let csr = p.serialize_request(&key).unwrap().pem().unwrap();
        (key.serialize_pem(), csr)
    }

    /// A client certificate for the key behind `csr_pem`, naming `node`, lasting `life` seconds.
    fn issue(ca: &Ca, csr_pem: &str, node: &str, life: i64) -> String {
        let mut csr = CertificateSigningRequestParams::from_pem(csr_pem).unwrap();
        let p = &mut csr.params;
        p.distinguished_name = dn(node);
        p.is_ca = IsCa::ExplicitNoCa;
        p.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        p.not_before = OffsetDateTime::now_utc() - TimeDuration::minutes(5);
        p.not_after = OffsetDateTime::now_utc() + TimeDuration::seconds(life);
        let issuer = Issuer::new(ca.params.clone(), &ca.key);
        csr.signed_by(&issuer).unwrap().pem()
    }

    fn real_now() -> i64 {
        OffsetDateTime::now_utc().unix_timestamp()
    }

    fn folder(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ar{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A joined machine named `m` whose certificate lasts 60 days (a renewal's 90 days then end later).
    struct Joined {
        dir: PathBuf,
        ca: Ca,
        cert: String,
        key: String,
    }

    fn joined(name: &str) -> Joined {
        let dir = folder(name);
        let ca = ca();
        let (key, csr) = request();
        let cert = issue(&ca, &csr, "m", 60 * DAY);
        std::fs::write(dir.join("node-id"), "m").unwrap();
        std::fs::write(dir.join("url"), "wss://localhost:1").unwrap();
        std::fs::write(dir.join("ca.pem"), &ca.pem).unwrap();
        std::fs::write(dir.join("cert.pem"), &cert).unwrap();
        std::fs::write(dir.join("key.pem"), &key).unwrap();
        Joined { dir, ca, cert, key }
    }

    fn ca_of(j: &Joined) -> Ca {
        Ca {
            key: KeyPair::from_pem(&j.ca.key.serialize_pem()).unwrap(),
            pem: j.ca.pem.clone(),
            params: j.ca.params.clone(),
        }
    }

    type AskFn = Box<dyn Fn(&HubLink, &str) -> Result<String, RenewError> + Send + Sync>;

    fn timing() -> Timing {
        Timing {
            check_every: Duration::from_secs(1000),
            unknown_every: Duration::from_secs(10),
            too_frequent: Duration::from_secs(3600),
            retry_first: Duration::from_secs(60),
            retry_max: Duration::from_secs(480),
        }
    }

    /// A renewer whose clock is `offset` seconds past now, whose hub answers with `ask`, and whose file operations are `ops`.
    fn renewer(j: &Joined, offset: i64, ask: AskFn, ops: Arc<dyn Ops>) -> Arc<Renewer> {
        let c = Arc::new(AtomicI64::new(offset));
        renewer_clocked(j, c, ask, ops)
    }

    fn renewer_clocked(
        j: &Joined,
        offset: Arc<AtomicI64>,
        ask: AskFn,
        ops: Arc<dyn Ops>,
    ) -> Arc<Renewer> {
        let r = Renewer::with_parts(
            j.dir.clone(),
            timing(),
            || Some(request()),
            || Ok(Box::new(()) as Box<dyn std::any::Any + Send>),
            ask,
            move || real_now() + offset.load(Ordering::Relaxed),
            ops,
        );
        r.set_hub_offers(true);
        r
    }

    /// 45 days on: 15 of 60 days are left, so a renewal is due.
    const DUE: i64 = 45 * DAY;

    fn answering(ca: Ca, node: &'static str, life: i64) -> AskFn {
        Box::new(move |_, csr| Ok(issue(&ca, csr, node, life)))
    }

    fn refusing(reason: &'static str) -> AskFn {
        Box::new(move |_, _| {
            Err(RenewError::Refused {
                code: "enroll.renew_refused".into(),
                reason: reason.into(),
            })
        })
    }

    fn counting(n: &Arc<AtomicU32>, inner: AskFn) -> AskFn {
        let n = Arc::clone(n);
        Box::new(move |l, c| {
            n.fetch_add(1, Ordering::Relaxed);
            inner(l, c)
        })
    }

    fn away() -> AskFn {
        Box::new(|_, _| Err(RenewError::Link(LinkError::Transport("down".into()))))
    }

    fn read_file(d: &Path, n: &str) -> String {
        std::fs::read_to_string(d.join(n)).unwrap()
    }

    fn files(d: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(d)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    fn same_pair(d: &Path) -> bool {
        pair_matches(&read_file(d, "cert.pem"), &read_file(d, "key.pem"))
    }

    fn ends(j: &Joined) -> i64 {
        validity(&j.cert).unwrap().1
    }

    /// The old pair stays and nothing else changes: what every rejected answer must leave.
    fn assert_old_pair_kept(j: &Joined) {
        assert_eq!(read_file(&j.dir, "cert.pem"), j.cert);
        assert_eq!(read_file(&j.dir, "key.pem"), j.key);
        assert!(same_pair(&j.dir));
        assert!(!j.dir.join(PREVIOUS).exists());
    }

    #[test]
    fn nothing_is_asked_while_more_than_a_third_of_the_life_is_left() {
        let j = joined("notdue");
        let asked = Arc::new(AtomicU32::new(0));
        let r = renewer(&j, 30 * DAY, counting(&asked, away()), Arc::new(Real));
        let (o, wait) = r.tick();
        assert_eq!(o, Outcome::Idle);
        assert_eq!(wait, Duration::from_secs(1000));
        assert_eq!(asked.load(Ordering::Relaxed), 0);
        assert!(!pending_exists(&j.dir));
        let _ = std::fs::remove_dir_all(&j.dir);
    }

    #[test]
    fn a_hub_that_has_not_said_it_renews_is_not_asked() {
        let j = joined("nooffer");
        let asked = Arc::new(AtomicU32::new(0));
        let r = renewer(&j, DUE, counting(&asked, away()), Arc::new(Real));
        r.set_hub_offers(false);
        assert_eq!(r.tick(), (Outcome::WaitingForHub, Duration::from_secs(10)));
        assert_eq!(asked.load(Ordering::Relaxed), 0);
        // Nothing was made or kept for a hub that cannot answer.
        assert!(!pending_exists(&j.dir));
        // The welcome of a hub that does offer it (through the uplink) starts the renewal.
        r.welcomed(&["terminal.raw".to_string(), CAPABILITY.to_string()]);
        assert_eq!(r.tick().0, Outcome::Failed);
        assert_eq!(asked.load(Ordering::Relaxed), 1);
        // A hub that stops offering it (an older one took over) is not asked either.
        r.welcomed(&["terminal.raw".to_string()]);
        assert_eq!(r.tick().0, Outcome::WaitingForHub);
        assert_eq!(asked.load(Ordering::Relaxed), 1);
        let _ = std::fs::remove_dir_all(&j.dir);
    }

    /// What `ask` saw: the request, whether the same request was on disk (with its key) at that moment, and the certificate presented.
    type Seen = Arc<Mutex<Vec<(String, bool, String)>>>;

    fn watching(dir: &Path, seen: &Seen) -> AskFn {
        let (dir, seen) = (dir.to_path_buf(), Arc::clone(seen));
        Box::new(move |link, csr| {
            let on_disk = std::fs::read_to_string(dir.join(PENDING)).unwrap_or_default();
            let kept = Pending::decode(&on_disk).is_some_and(|p| p.csr_pem == csr.trim());
            seen.lock()
                .unwrap()
                .push((csr.to_string(), kept, link.cert_pem.clone()));
            Err(RenewError::Link(LinkError::Closed))
        })
    }

    #[test]
    fn the_fresh_key_and_request_are_kept_before_they_are_sent_and_the_same_request_is_sent_again()
    {
        let j = joined("order");
        let seen: Seen = Arc::default();
        let r = renewer(&j, DUE, watching(&j.dir, &seen), Arc::new(Real));
        assert_eq!(r.tick().0, Outcome::Failed);
        // The process stops here. A new one finds the pending request and sends the same one.
        let r2 = renewer(&j, DUE, watching(&j.dir, &seen), Arc::new(Real));
        assert_eq!(r2.tick().0, Outcome::Failed);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(
            seen[0].1 && seen[1].1,
            "the request was not on disk when it was sent"
        );
        assert_eq!(seen[0].0, seen[1].0, "a restart made another request");
        assert_eq!(
            seen[0].2, j.cert,
            "the current certificate is the one presented"
        );
        // The certificate in use is untouched by a renewal that did not finish.
        assert_old_pair_kept(&j);
        let _ = std::fs::remove_dir_all(&j.dir);
    }

    #[test]
    fn an_answer_is_stored_as_one_pair_and_the_old_pair_is_given_up_after_one_connection_with_the_new(
    ) {
        let j = joined("install");
        let r = renewer(&j, DUE, answering(ca_of(&j), "m", 90 * DAY), Arc::new(Real));
        assert_eq!(r.tick().0, Outcome::Renewed);
        let (cert, key) = (read_file(&j.dir, "cert.pem"), read_file(&j.dir, "key.pem"));
        assert_ne!(cert, j.cert);
        assert_ne!(key, j.key);
        assert!(same_pair(&j.dir));
        assert!(!pending_exists(&j.dir));
        // The pair that was in use is kept, whole, until the new one has connected.
        let (pc, pk) = split_pair(&read_file(&j.dir, PREVIOUS)).unwrap();
        assert_eq!((pc.as_str(), pk.as_str()), (j.cert.trim(), j.key.trim()));
        assert!(pair_matches(&pc, &pk));
        // The next connection uses the new pair.
        assert_eq!(r.current(), Some((cert.clone(), key)));
        // A connection with the OLD certificate (the one that was open) does not let go of it.
        r.accepted(&j.cert);
        assert!(j.dir.join(PREVIOUS).exists());
        r.accepted(&cert);
        assert!(!j.dir.join(PREVIOUS).exists());
        // It is not due again: the new certificate has its whole life.
        assert_eq!(r.tick().0, Outcome::Idle);
        assert_eq!(
            files(&j.dir),
            ["ca.pem", "cert.pem", "key.pem", "node-id", "url"]
        );
        let _ = std::fs::remove_dir_all(&j.dir);
    }

    #[test]
    fn an_answer_for_another_node_key_or_ca_is_rejected_and_the_old_pair_kept() {
        for (name, how) in [
            ("node", 0),
            ("key", 1),
            ("ca", 2),
            ("ended", 3),
            ("shorter", 4),
        ] {
            let j = joined(&format!("bad-{name}"));
            let other = ca();
            let ca = ca_of(&j);
            let ask: AskFn = Box::new(move |_, csr| match how {
                0 => Ok(issue(&ca, csr, "someone-else", 90 * DAY)),
                1 => Ok(issue(&ca, &request().1, "m", 90 * DAY)),
                2 => Ok(issue(&other, csr, "m", 90 * DAY)),
                3 => Ok(issue(&ca, csr, "m", -3600)),
                _ => Ok(issue(&ca, csr, "m", 5 * DAY)),
            });
            let r = renewer(&j, DUE, ask, Arc::new(Real));
            let (o, wait) = r.tick();
            assert_eq!(o, Outcome::Failed, "{name}");
            assert_eq!(wait, Duration::from_secs(3600), "{name}");
            assert_old_pair_kept(&j);
            // The request is kept: the hub answers it again with the same certificate, and asking anew would only be refused.
            assert!(pending_exists(&j.dir), "{name}");
            let s = state(&j.dir, ends(&j)).unwrap();
            assert_eq!(s.code, "rejected", "{name}");
            // Not asked again before its time.
            assert_eq!(r.tick().0, Outcome::Refused("rejected".into()), "{name}");
            let _ = std::fs::remove_dir_all(&j.dir);
        }
    }

    #[test]
    fn what_the_hub_refuses_decides_whether_it_is_asked_again() {
        for reason in ["unknown", "revoked", "expired", "too_old"] {
            let j = joined(&format!("join-{reason}"));
            let asked = Arc::new(AtomicU32::new(0));
            let r = renewer(&j, DUE, counting(&asked, refusing(reason)), Arc::new(Real));
            assert_eq!(
                r.tick().0,
                Outcome::Refused("join_again".into()),
                "{reason}"
            );
            let s = state(&j.dir, ends(&j)).unwrap();
            assert_eq!((s.code.as_str(), s.detail.as_str()), ("join_again", reason));
            // Never retried in a loop, however many looks follow, and nothing is left kept.
            for _ in 0..5 {
                assert_eq!(r.tick().0, Outcome::Refused("join_again".into()));
            }
            assert_eq!(asked.load(Ordering::Relaxed), 1, "{reason}");
            assert!(!pending_exists(&j.dir));
            assert_old_pair_kept(&j);
            let _ = std::fs::remove_dir_all(&j.dir);
        }
    }

    #[test]
    fn a_certificate_that_is_no_longer_the_newest_is_told_as_security_relevant_and_not_asked_again()
    {
        let j = joined("superseded");
        let asked = Arc::new(AtomicU32::new(0));
        let r = renewer(
            &j,
            DUE,
            counting(&asked, refusing("superseded")),
            Arc::new(Real),
        );
        assert_eq!(r.tick().0, Outcome::Refused("superseded".into()));
        for _ in 0..4 {
            r.tick();
        }
        assert_eq!(asked.load(Ordering::Relaxed), 1);
        assert_eq!(state(&j.dir, ends(&j)).unwrap().code, "superseded");
        assert!(r.log_lines().iter().any(|l| l.contains("SECURITY")
            && l.contains("no longer the newest")
            && l.contains("another holder may exist")));
        // A state about another certificate (this machine joined again since) does not stand.
        assert!(state(&j.dir, ends(&j) + 1).is_none());
        let _ = std::fs::remove_dir_all(&j.dir);
    }

    #[test]
    fn too_frequent_waits_an_hour_and_then_asks_the_same_request_again() {
        let j = joined("frequent");
        let clock = Arc::new(AtomicI64::new(DUE));
        let csrs = Arc::new(Mutex::new(Vec::<String>::new()));
        let (s2, ca) = (Arc::clone(&csrs), ca_of(&j));
        let ask: AskFn = Box::new(move |_, csr| {
            let mut seen = s2.lock().unwrap();
            seen.push(csr.to_string());
            if seen.len() == 1 {
                Err(RenewError::Refused {
                    code: "enroll.renew_refused".into(),
                    reason: "too_frequent".into(),
                })
            } else {
                Ok(issue(&ca, csr, "m", 90 * DAY))
            }
        });
        let r = renewer_clocked(&j, Arc::clone(&clock), ask, Arc::new(Real));
        let (o, wait) = r.tick();
        assert_eq!(
            (o, wait),
            (
                Outcome::Refused("too_frequent".into()),
                Duration::from_secs(3600)
            )
        );
        // Half an hour later: still waiting, not asked.
        clock.fetch_add(1800, Ordering::Relaxed);
        let (o, wait) = r.tick();
        assert_eq!(o, Outcome::Refused("too_frequent".into()));
        assert!(
            wait <= Duration::from_secs(1000) && wait > Duration::from_secs(900),
            "{wait:?}"
        );
        assert_eq!(csrs.lock().unwrap().len(), 1);
        // After the hour: the same request.
        clock.fetch_add(1801, Ordering::Relaxed);
        assert_eq!(r.tick().0, Outcome::Renewed);
        let csrs = csrs.lock().unwrap();
        assert_eq!(csrs.len(), 2);
        assert_eq!(csrs[0], csrs[1]);
        let _ = std::fs::remove_dir_all(&j.dir);
    }

    #[test]
    fn a_hub_that_cannot_be_reached_is_tried_again_with_a_growing_wait_and_the_node_is_untouched() {
        let j = joined("away");
        let r = renewer(&j, DUE, away(), Arc::new(Real));
        let waits: Vec<u64> = (0..6).map(|_| r.tick().1.as_secs()).collect();
        assert_eq!(waits, [60, 120, 240, 480, 480, 480]);
        // One line for the whole outage, not one per try.
        let lines = r.log_lines();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_old_pair_kept(&j);
        // An older hub that answers with another error is a temporary failure as well, not a refusal to remember.
        let r = renewer(
            &j,
            DUE,
            Box::new(|_, _| {
                Err(RenewError::Refused {
                    code: "protocol.invalid_message".into(),
                    reason: String::new(),
                })
            }),
            Arc::new(Real),
        );
        assert_eq!(r.tick().0, Outcome::Failed);
        assert!(state(&j.dir, ends(&j)).is_none());
        let _ = std::fs::remove_dir_all(&j.dir);
    }

    #[test]
    fn a_certificate_that_has_ended_needs_a_new_join_without_asking_anyone() {
        let j = joined("ended");
        let asked = Arc::new(AtomicU32::new(0));
        let r = renewer(&j, 100 * DAY, counting(&asked, away()), Arc::new(Real));
        assert_eq!(r.tick().0, Outcome::Refused("join_again".into()));
        assert_eq!(asked.load(Ordering::Relaxed), 0);
        let s = state(&j.dir, ends(&j)).unwrap();
        assert_eq!(
            (s.code.as_str(), s.detail.as_str()),
            ("join_again", "expired")
        );
        let _ = std::fs::remove_dir_all(&j.dir);
    }

    #[test]
    fn an_answer_for_a_certificate_that_is_no_longer_in_use_is_dropped() {
        let j = joined("rejoined");
        let dir = j.dir.clone();
        let ca = ca_of(&j);
        let r = renewer(
            &j,
            DUE,
            Box::new(move |_, csr| {
                // While the hub is asked, the machine joins again: the files are another pair now.
                let (key2, csr2) = request();
                std::fs::write(dir.join("cert.pem"), issue(&ca, &csr2, "m", 90 * DAY)).unwrap();
                std::fs::write(dir.join("key.pem"), key2).unwrap();
                Ok(issue(&ca, csr, "m", 90 * DAY))
            }),
            Arc::new(Real),
        );
        assert_eq!(r.tick().0, Outcome::Idle);
        assert!(same_pair(&j.dir));
        assert_ne!(read_file(&j.dir, "cert.pem"), j.cert);
        assert!(!pending_exists(&j.dir));
        assert!(!j.dir.join(PREVIOUS).exists());
        let _ = std::fs::remove_dir_all(&j.dir);
    }

    /// Fails the n-th operation (counting writes and removes), as a stop would; every earlier one happened.
    struct Stops {
        at: u32,
        seen: AtomicU32,
    }

    impl Stops {
        fn at(n: u32) -> Arc<Self> {
            Arc::new(Self {
                at: n,
                seen: AtomicU32::new(0),
            })
        }

        fn step(&self) -> io::Result<()> {
            if self.seen.fetch_add(1, Ordering::AcqRel) == self.at {
                return Err(io::Error::other("stopped"));
            }
            Ok(())
        }
    }

    impl Ops for Stops {
        fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
            self.step()?;
            Real.write(path, bytes)
        }

        fn remove(&self, path: &Path) -> io::Result<()> {
            self.step()?;
            Real.remove(path)
        }
    }

    /// An idempotent hub: one certificate per request, the same one again for the same request.
    fn idempotent(ca: Ca, made: &Arc<Mutex<Vec<String>>>) -> AskFn {
        let made = Arc::clone(made);
        Box::new(move |_, csr| {
            let mut seen = made.lock().unwrap();
            if let Some(first) = seen.first() {
                return Ok(first.clone());
            }
            let cert = issue(&ca, csr, "m", 90 * DAY);
            seen.push(cert.clone());
            Ok(cert)
        })
    }

    /// Whatever step the process stopped at, the folder is one whole pair (old or new) after the next start looks at it, the same
    /// request is sent again, and in the end exactly one new certificate is in place and nothing is left over.
    #[test]
    fn a_stop_between_any_two_steps_leaves_a_whole_pair_and_ends_with_one_new_certificate() {
        // The operations of one renewal: the request is kept (0), then the answer is stored: previous (1), certificate (2), key (3),
        // the request dropped (4), the state dropped (5). Stopping at 6 is no stop at all.
        for stop_at in 0..7u32 {
            let j = joined(&format!("stop{stop_at}"));
            let made = Arc::new(Mutex::new(Vec::<String>::new()));
            let first = renewer(&j, DUE, idempotent(ca_of(&j), &made), Stops::at(stop_at));
            let _ = first.tick();
            // No temporary file is left by a stop: each write is a rename.
            assert!(
                files(&j.dir).iter().all(|f| !f.contains(".tmp-")),
                "stop {stop_at}: {:?}",
                files(&j.dir)
            );
            // The next start: the files are looked at under the guard (`recover`), then the timer goes on.
            let second = renewer(&j, DUE, idempotent(ca_of(&j), &made), Arc::new(Real));
            assert!(second.current().is_some());
            assert!(
                same_pair(&j.dir),
                "stop {stop_at}: the folder is not one pair"
            );
            let _ = second.tick();
            let _ = second.tick();
            assert!(same_pair(&j.dir), "stop {stop_at}");
            // Exactly one new certificate was ever made (a crash before the first answer makes none until the second start asks),
            // and it is the one in place.
            let made = made.lock().unwrap();
            assert_eq!(made.len(), 1, "stop {stop_at}: not exactly one certificate");
            assert_eq!(
                read_file(&j.dir, "cert.pem").trim(),
                made[0].trim(),
                "stop {stop_at}"
            );
            assert!(!pending_exists(&j.dir), "stop {stop_at}");
            assert!(state(&j.dir, validity(&made[0]).unwrap().1).is_none());
            let _ = std::fs::remove_dir_all(&j.dir);
        }
    }

    /// The old pair is put back when the files do not match and nothing explains it but `previous.pem` (a stop that left a new key with
    /// the old certificate, as an install written in another order would).
    #[test]
    fn a_mismatched_folder_with_no_pending_request_goes_back_to_the_previous_pair() {
        let j = joined("restore");
        let (key2, _) = request();
        std::fs::write(
            j.dir.join(PREVIOUS),
            format!("{}\n{}\n", j.cert.trim(), j.key.trim()),
        )
        .unwrap();
        std::fs::write(j.dir.join("key.pem"), key2).unwrap();
        assert!(!same_pair(&j.dir));
        assert!(recover(&j.dir, &Real));
        assert!(same_pair(&j.dir));
        assert_eq!(read_file(&j.dir, "cert.pem").trim(), j.cert.trim());
        // A whole folder is left alone.
        assert!(!recover(&j.dir, &Real));
        let _ = std::fs::remove_dir_all(&j.dir);
    }

    #[test]
    fn no_key_or_request_text_reaches_the_log() {
        let j = joined("logs");
        let ca = ca_of(&j);
        let r = renewer(
            &j,
            DUE,
            Box::new(move |_, _| Ok(issue(&ca, &request().1, "m", 90 * DAY))),
            Arc::new(Real),
        );
        let _ = r.tick();
        let pending = Pending::decode(&read_file(&j.dir, PENDING)).unwrap();
        let body = |pem: &str| pem.lines().nth(1).unwrap().to_string();
        let lines = r.log_lines();
        assert!(!lines.is_empty());
        for line in lines {
            assert!(!line.contains("PRIVATE"), "{line}");
            for pem in [&pending.key_pem, &j.key, &pending.csr_pem, &j.cert] {
                assert!(!line.contains(&body(pem)), "{line}");
            }
        }
        let _ = std::fs::remove_dir_all(&j.dir);
    }

    #[test]
    fn the_timer_looks_at_start_and_stops_when_dropped() {
        let j = joined("timer");
        let asked = Arc::new(AtomicU32::new(0));
        let r = renewer(&j, DUE, counting(&asked, away()), Arc::new(Real));
        let timer = spawn(r).unwrap();
        let end = std::time::Instant::now() + Duration::from_secs(10);
        while asked.load(Ordering::Relaxed) == 0 {
            assert!(std::time::Instant::now() < end, "the timer never looked");
            std::thread::sleep(Duration::from_millis(20));
        }
        drop(timer);
        let _ = std::fs::remove_dir_all(&j.dir);
    }
}
