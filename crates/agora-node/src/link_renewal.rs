//! The node renews its own certificate (ADR-0085, step 2): the files it keeps in the link folder, the order it writes them in, how a
//! stopped process finds its way back, and the timer that asks the hub.
//!
//! The link folder holds `url`, `node-id`, `ca.pem`, `cert.pem` and `key.pem` (written by enrollment or pairing). A renewal adds three
//! files, all private to this user and each written whole (a temporary file renamed over the target):
//!
//! - `renew-pending.pem`: the fresh key and its request, and the fingerprint of the certificate they were made for. **Written before the
//!   request is sent** and kept until the answer is stored, so a crash or a lost answer is met by sending the SAME request again (the
//!   hub answers it with the same certificate, nothing new issued). A request made for another certificate than the one in use (the
//!   machine joined again) is dropped, not sent.
//! - `previous.pem`: the certificate and key that were in use before the new pair was written; removed once a connection made with the
//!   new certificate was accepted, so a pair is never given up before its replacement has worked once. It is put back only for the
//!   same node id as the link's `node-id`, and every save of the link (`agora join`, `enroll`, pairing) removes it first.
//! - `renew-state`: what the hub last said (`join_again`, `superseded`, `too_frequent`, `rejected`, `hub_old`), when it was asked, for
//!   `agora doctor`. It is only information and a rest period: nothing here is ever decided for good. A refusal that only a new join
//!   answers is asked again about once a day, and a success clears it.
//!
//! The order of an install (see [`install`]) keeps the folder usable at every point: after each step either the old pair or the new
//! pair can be read as a whole, or [`recover`] makes it so (it runs under the folder's guard whenever the link is loaded). When it
//! cannot write (a full disk) [`usable`] still names a pair that matches, from memory, so the uplink never dials with a certificate and
//! a key that do not belong together.
//!
//! The clock is never trusted to give a verdict: whether the certificate has ended is worked out again at every look from its own end
//! and the clock, and nothing about it is written down, so a clock that jumps forward and back costs nothing.
//!
//! Nothing here holds a lock that agents or the supervisor use, and nothing waits on the network while holding the folder's guard.
//! Agents never depend on any of it: a renewal that cannot finish leaves the node as it was.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::cert_renew::{
    check_renewed, due, fingerprint, pair_matches, subject_name, validity, Pending, Refusal,
};
use crate::uplink_client::{CertSource, HubLink, LinkError, RenewError};

/// The fresh key and request of a renewal that has not been answered yet.
pub const PENDING: &str = "renew-pending.pem";
/// The pair that was in use before the last renewal, until the new one has connected.
pub const PREVIOUS: &str = "previous.pem";
/// What the hub last said, for `agora doctor`.
pub const STATE: &str = "renew-state";

/// The capability a hub offers when it handles renewals.
pub const CAPABILITY: &str = "enroll.renew";

/// How many answers in a row that can never be right for the request (`Rejected::gives_up`) are asked for again before the request is
/// given up and a new one is made. Answers rejected for time never give it up (ADR-0085 Amendment A).
pub const MAX_REJECTED: u32 = 5;

/// The longest a state may hold a retry back, whatever the file says (a damaged or hostile file must not wedge a machine).
const MAX_HOLD_SECS: i64 = 2 * 86_400;

/// The files the link folder's writers make; a temporary file of theirs is `<name>.tmp-<pid>-<seq>` (`<name>.tmp-<pid>` before 0.3.41).
const NAMES: [&str; 8] = [
    "node-id", "url", "ca.pem", "cert.pem", "key.pem", PENDING, PREVIOUS, STATE,
];

/// Text from the hub (or made from it) as it may be kept, logged or shown: no control characters, at most 200 characters.
pub fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| {
            !c.is_control()
                // Zero-width and direction marks, embeddings, overrides and isolates, and the line and paragraph separators: they
                // can make a line read as something else.
                && !matches!(*c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{2028}' | '\u{2029}' | '\u{FEFF}')
        })
        .take(200)
        .collect()
}

/// The most of a file of ours that is read (the state and the pending request are far smaller): a larger file is not read at all.
const MAX_READ: u64 = 64 * 1024;

/// The text of `path`, unless it is larger than [`MAX_READ`] or not text.
fn read_capped(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut text = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_READ + 1)
        .read_to_string(&mut text)
        .ok()?;
    (u64::try_from(text.len()).ok()? <= MAX_READ).then_some(text)
}

/// What the hub said, kept as one short file next to the certificate it was said about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    /// `join_again`, `superseded`, `too_frequent`, `rejected` or `hub_old`.
    pub code: String,
    /// The `reason` the hub gave (or why the answer was rejected): a stable word, shown only as a parameter.
    pub detail: String,
    /// When the certificate this was said about ends: a state about another certificate (the machine joined again) is ignored.
    pub ends: i64,
    /// Seconds since 1970 before which no new try is made (0: none).
    pub retry_at: i64,
    /// Seconds since 1970 when the hub was last asked.
    pub asked_at: i64,
    /// How many answers in a row could never be right for the request (`rejected` with a reason that gives it up); 0 for one rejected
    /// for time.
    pub count: u32,
}

impl State {
    fn encode(&self) -> String {
        format!(
            "{}\n{}\n{}\n{}\n{}\n{}\n",
            clean(&self.code),
            clean(&self.detail),
            self.ends,
            self.retry_at,
            self.asked_at,
            self.count
        )
    }

    /// Reads what [`Self::encode`] wrote, and the shorter form an earlier build wrote. A file that cannot be read is `None`: it is
    /// then as if nothing was recorded, and the next refusal writes it again.
    fn decode(text: &str) -> Option<Self> {
        let mut l = text.lines();
        let code = clean(l.next()?);
        let detail = clean(l.next()?);
        let ends = l.next()?.trim().parse().ok()?;
        let mut num = || l.next().and_then(|v| v.trim().parse::<i64>().ok());
        let retry_at = num().unwrap_or(0);
        let asked_at = num().unwrap_or(0);
        let count = num().and_then(|n| u32::try_from(n).ok()).unwrap_or(0);
        (!code.is_empty()).then_some(Self {
            code,
            detail,
            ends,
            retry_at,
            asked_at,
            count,
        })
    }
}

/// The state recorded in `dir` for the certificate that ends at `ends` (none for a state about another certificate).
pub fn state(dir: &Path, ends: i64) -> Option<State> {
    let s = State::decode(&read_capped(&dir.join(STATE))?)?;
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
    read_capped(&dir.join(name))
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

/// The pair in `previous.pem`, when it is whole and for the node the link folder is for now (`node-id`): a rejoin of another identity
/// must never be undone by the pair of the one before.
fn previous_pair(dir: &Path) -> Option<(String, String)> {
    let (cert, key) = split_pair(&read(dir, PREVIOUS)?)?;
    let id = read(dir, "node-id")?;
    (pair_matches(&cert, &key) && subject_name(&cert).as_deref() == Some(id.trim()))
        .then_some((cert, key))
}

/// Removes what a writer of this folder left behind when it stopped between creating its temporary file and renaming it: a file named
/// `<one of our files>.tmp-<pid>-<seq>` or `.tmp-<pid>` ([`agora_platform::split_temporary`]), nothing else. It may hold a private key. The caller holds the folder's guard, so no writer is
/// in the middle of one.
fn sweep_temporaries(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some((base, _pid)) = agora_platform::split_temporary(&name) else {
            continue;
        };
        if NAMES.contains(&base) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Puts the new certificate and its key in place of the pair in use, in an order that leaves the folder readable whole after every
/// step: (1) the pair in use is copied to `previous.pem`, (2) the new certificate is written over `cert.pem`, (3) the key of the
/// request over `key.pem`, (4) `renew-pending.pem` is dropped, (5) `renew-state` is dropped. Between 2 and 3 the files do not match:
/// [`recover`] finishes the install from `renew-pending.pem`, which is still there, or goes back to `previous.pem`. Each write is a
/// temporary file renamed over its target, so no step leaves half a file. The caller holds the folder's guard.
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

/// What [`recover`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recovery {
    /// `cert.pem` and `key.pem` already were one pair (or there is no link to look at).
    Whole,
    /// They were not, or a stopped install had left files behind, and the folder is now one whole pair.
    Repaired,
    /// They are not one pair and could not be made one (the folder cannot be written). [`usable`] still names a pair.
    Unrepaired,
}

/// Brings the folder to a state where `cert.pem` and `key.pem` are one pair, after a stop between the steps of [`install`] (the caller
/// holds the guard). Temporary files a stopped writer left are removed first. Then:
/// - the pair matches and the pending key is the one in use: an install that only lacked its clean-up is finished (leftovers removed);
/// - the certificate belongs to the pending key but the key file does not: the key is written and the leftovers dropped; when that
///   cannot be written the folder is rolled BACK to the pair in `previous.pem` (the same node id only);
/// - nothing explains the mismatch and `previous.pem` is whole: that pair is put back.
///
/// It never says `Whole` or `Repaired` for a folder whose files do not match when read again.
pub fn recover(dir: &Path, ops: &dyn Ops) -> Recovery {
    sweep_temporaries(dir);
    let (Some(cert), Some(key)) = (read(dir, "cert.pem"), read(dir, "key.pem")) else {
        return Recovery::Whole;
    };
    let pending = read(dir, PENDING).and_then(|t| Pending::decode(&t));
    let whole = |dir: &Path| match (read(dir, "cert.pem"), read(dir, "key.pem")) {
        (Some(c), Some(k)) => pair_matches(&c, &k),
        _ => false,
    };
    if pair_matches(&cert, &key) {
        if pending
            .as_ref()
            .is_some_and(|p| pair_matches(&cert, &p.key_pem))
        {
            let _ = ops.remove(&dir.join(PENDING));
            let _ = ops.remove(&dir.join(STATE));
            return Recovery::Repaired;
        }
        return Recovery::Whole;
    }
    if let Some(p) = &pending {
        if pair_matches(&cert, &p.key_pem)
            && ops
                .write(&dir.join("key.pem"), format!("{}\n", p.key_pem).as_bytes())
                .is_ok()
            && whole(dir)
        {
            let _ = ops.remove(&dir.join(PENDING));
            let _ = ops.remove(&dir.join(STATE));
            return Recovery::Repaired;
        }
    }
    // Roll back: the certificate first, so that a stop between the two leaves the old pair whole (the key on disk is the old one).
    if let Some((c, k)) = previous_pair(dir) {
        let _ = ops.write(&dir.join("cert.pem"), c.as_bytes());
        let _ = ops.write(&dir.join("key.pem"), format!("{k}\n").as_bytes());
        if whole(dir) {
            return Recovery::Repaired;
        }
    }
    Recovery::Unrepaired
}

/// The certificate and key to use now: the files when they are one pair; else, when only the key could not be written, the new
/// certificate with the pending key (the new pair, from memory); else the previous pair; else nothing. Never two that do not match.
pub fn usable(dir: &Path) -> Option<(String, String)> {
    let cert = read(dir, "cert.pem")?;
    let key = read(dir, "key.pem")?;
    if pair_matches(&cert, &key) {
        return Some((cert, key));
    }
    if let Some(p) = read(dir, PENDING).and_then(|t| Pending::decode(&t)) {
        if pair_matches(&cert, &p.key_pem) {
            return Some((cert, p.key_pem));
        }
    }
    previous_pair(dir)
}

/// How long the timer waits in each case.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// Between looks at the certificate when nothing is due, and between asks of a hub that does not handle renewals (an hour).
    pub check_every: Duration,
    /// Between looks while no hub has said yet whether it handles renewals (it may not be connected yet): a look is a few file reads.
    pub unknown_every: Duration,
    /// After a refusal `too_frequent` and after an answer that did not check out (an hour: the hub's own limit).
    pub too_frequent: Duration,
    /// After a refusal that only a new join answers (a day, and up to a tenth more at random so machines do not ask together).
    pub refused_every: Duration,
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
            refused_every: Duration::from_secs(86_400),
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
    /// The certificate has ended by the clock (nothing is written down about it).
    Ended,
    /// Due, but the hub has not said it handles renewals.
    WaitingForHub,
    /// A new certificate was stored; the next connection uses it.
    Renewed,
    /// The hub refused (or a refusal from earlier still rests): the code of the state recorded.
    Refused(String),
    /// The hub could not be reached, answered something else, or the answer was rejected, or the link folder's guard could not be
    /// had: tried again after a wait.
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
    /// Whether a hub has welcomed this node without it (an older hub), as opposed to no welcome yet.
    hub_without: AtomicBool,
    /// Consecutive failures to reach the hub, for the backoff.
    fails: AtomicU32,
    /// A rest the state file asked for beyond [`MAX_HOLD_SECS`]: the value in the file and the end this process holds it to, counted
    /// from when it first saw it (so a file that says "a thousand years" holds two days, not for ever).
    capped: Mutex<Option<(i64, i64)>>,
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

/// `base` seconds and up to a tenth more.
fn jittered(base: Duration) -> i64 {
    let secs = i64::try_from(base.as_secs()).unwrap_or(i64::MAX / 2);
    let mut b = [0u8; 4];
    let _ = getrandom::getrandom(&mut b);
    secs + i64::from(u32::from_le_bytes(b)) % (secs / 10 + 1)
}

/// What a look found in the folder, read under its guard.
struct Snapshot {
    node_id: String,
    link: HubLink,
    pending: Option<Pending>,
    /// The new certificate is in use (in memory at least) and only its key file or the clean-up is missing: nothing to ask.
    finishing: bool,
}

impl Renewer {
    /// A renewer of `dir` that makes keys with `fresh` (a key and its request, as PEM texts) and takes the folder's guard with `hold`
    /// (dropping what it returns lets go). Uses the real clock, connection and files.
    pub fn new(
        dir: PathBuf,
        fresh: impl Fn() -> Option<(String, String)> + Send + Sync + 'static,
        hold: impl Fn() -> io::Result<Box<dyn std::any::Any + Send>> + Send + Sync + 'static,
    ) -> Arc<Self> {
        Self::with_parts(
            dir,
            Timing::default(),
            fresh,
            hold,
            crate::uplink_client::renew,
            secs_now,
            Arc::new(Real),
        )
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
            hub_without: AtomicBool::new(false),
            fails: AtomicU32::new(0),
            capped: Mutex::new(None),
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
        self.hub_without.store(!offers, Ordering::Release);
    }

    /// Log lines go to the node's log, once per change: a hub that stays away does not repeat itself every minute.
    fn log(&self, line: String) {
        let mut last = self.last_log.lock().unwrap_or_else(|e| e.into_inner());
        if *last != line {
            crate::log_line!("{line}");
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

    /// The link folder's files, read whole under its guard (recovering a stopped install first). `Ok(None)`: no link here, or a file
    /// could not be had. `Err`: the guard could not be had, which is not "nothing to do" (audit F1341).
    fn snapshot(&self) -> io::Result<Option<Snapshot>> {
        let _guard = (self.hold)()?;
        Ok(self.snapshot_held())
    }

    fn snapshot_held(&self) -> Option<Snapshot> {
        recover(&self.dir, self.ops.as_ref());
        let node_id = read(&self.dir, "node-id")?.trim().to_string();
        let (cert_pem, key_pem) = usable(&self.dir)?;
        let link = HubLink {
            url: read(&self.dir, "url")?.trim().to_string(),
            ca_pem: read(&self.dir, "ca.pem")?,
            cert_pem,
            key_pem,
            commands: None,
            mesh_out: None,
            grants: None,
            mesh: None,
            devices: None,
            state_dir: None,
            certs: None,
            spawner: None,
        };
        let mut pending = read(&self.dir, PENDING).and_then(|t| Pending::decode(&t));
        let mut finishing = false;
        if let Some(p) = &pending {
            if pair_matches(&link.cert_pem, &p.key_pem) {
                // The new certificate is the one in use. With the files one pair that is a leftover; otherwise the new key could not
                // be written yet and the pending file is its only copy.
                if read(&self.dir, "key.pem").is_some_and(|k| pair_matches(&link.cert_pem, &k)) {
                    let _ = self.ops.remove(&self.dir.join(PENDING));
                    pending = None;
                } else {
                    finishing = true;
                }
            } else if p.for_cert != fingerprint(&link.cert_pem) {
                // Made for a certificate that is not this one (the machine joined again since): never sent.
                let _ = self.ops.remove(&self.dir.join(PENDING));
                pending = None;
            }
        }
        Some(Snapshot {
            node_id,
            link,
            pending,
            finishing,
        })
    }

    fn write_state(&self, s: &State) {
        let Ok(_guard) = (self.hold)() else {
            return;
        };
        let _ = self.ops.write(&self.dir.join(STATE), s.encode().as_bytes());
    }

    fn clear_state(&self) {
        if let Ok(_guard) = (self.hold)() {
            let _ = self.ops.remove(&self.dir.join(STATE));
        }
    }

    fn drop_pending(&self) {
        if let Ok(_guard) = (self.hold)() {
            let _ = self.ops.remove(&self.dir.join(PENDING));
        }
    }

    /// The end of a rest the state file asks for: as written, unless it is further than [`MAX_HOLD_SECS`] away, then that long from
    /// the first time it was seen.
    fn held_until(&self, retry_at: i64, now: i64) -> i64 {
        if retry_at <= now + MAX_HOLD_SECS {
            return retry_at;
        }
        let mut held = self.capped.lock().unwrap_or_else(|e| e.into_inner());
        match *held {
            Some((file, until)) if file == retry_at => until,
            _ => {
                *held = Some((retry_at, now + MAX_HOLD_SECS));
                now + MAX_HOLD_SECS
            }
        }
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
        let snap = match self.snapshot() {
            Ok(Some(snap)) => snap,
            Ok(None) => return (Outcome::Idle, self.timing.check_every),
            Err(e) => {
                // The folder cannot be looked at, so whether the certificate is due is unknown: a failed look, said once, and tried
                // again with the growing wait (audit F1341: it was a silent "nothing to do").
                self.log(format!(
                    "cert renewal: the link folder could not be locked: {}",
                    e.kind()
                ));
                return (Outcome::Failed, self.backoff());
            }
        };
        let Some((_, ends)) = validity(&snap.link.cert_pem) else {
            return (Outcome::Idle, self.timing.check_every);
        };
        if ends <= now {
            // Past its end the hub refuses the connection itself: nothing to ask. Only joining again helps. This is the clock's word
            // and is never written down: a clock that was wrong for a while must not leave a verdict behind.
            self.log(
                "cert renewal: the certificate has ended by this clock; if the clock is right this machine must join again".into(),
            );
            return (Outcome::Ended, self.timing.check_every);
        }
        if snap.finishing {
            self.log("cert renewal: the new key could not be written yet".into());
            return (Outcome::Failed, self.backoff());
        }
        // What the hub said about this very certificate rests until its time (and never for longer than two days, whatever the file
        // says); then it is asked again.
        if let Some(s) = state(&self.dir, ends) {
            if s.code == "hub_old" {
                if self.hub_offers.load(Ordering::Acquire) {
                    self.clear_state();
                }
            } else {
                let retry_at = self.held_until(s.retry_at, now);
                if retry_at > now {
                    let wait = Duration::from_secs(u64::try_from(retry_at - now).unwrap_or(1));
                    return (Outcome::Refused(s.code), wait.min(self.timing.check_every));
                }
            }
        }
        if snap.pending.is_none() && due(&snap.link.cert_pem, now) != Some(true) {
            return (Outcome::Idle, self.timing.check_every);
        }
        if !self.hub_offers.load(Ordering::Acquire) {
            if self.hub_without.load(Ordering::Acquire) {
                // A hub that answered the handshake without the capability is an older one: asked about once an hour, said once.
                self.write_state(&State {
                    code: "hub_old".into(),
                    detail: String::new(),
                    ends,
                    retry_at: 0,
                    asked_at: now,
                    count: 0,
                });
                self.log("cert renewal: the hub does not support renewal yet".into());
                return (Outcome::WaitingForHub, self.timing.check_every);
            }
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
                let p = Pending::new(&key, &csr).bound(&snap.link.cert_pem);
                // A new request has its own five attempts: what was counted for the one given up does not carry over.
                if state(&self.dir, ends).is_some_and(|s| s.code == "rejected") {
                    self.clear_state();
                }
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
                self.refused(&snap, &clean(&reason), ends, now)
            }
            Err(RenewError::Refused { code, .. }) => {
                self.log(format!("cert renewal: the hub answered {}", clean(&code)));
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
            // The pair in use is kept as it is. The request is kept too, and asked again after an hour: the hub answers it again with
            // the same certificate. After MAX_REJECTED answers in a row that can never be right for it (`Rejected::gives_up`) the
            // request is given up (its key goes with it) and the next due look makes a new one. An answer rejected for time (this
            // machine's clock is far behind) or for gaining nothing never gives it up and breaks the run (ADR-0085 Amendment A): the
            // hub would answer a new request `superseded`, and only the pending key can take the certificate it already issued.
            let count = if why.gives_up() {
                state(&self.dir, ends)
                    .filter(|s| {
                        s.code == "rejected"
                            && crate::cert_renew::Rejected::from_code(&s.detail)
                                .is_some_and(|r| r.gives_up())
                    })
                    .map_or(0, |s| s.count)
                    .saturating_add(1)
            } else {
                0
            };
            let given_up = count >= MAX_REJECTED;
            let wait = if given_up {
                self.drop_pending();
                self.timing.refused_every
            } else {
                self.timing.too_frequent
            };
            self.write_state(&State {
                code: "rejected".into(),
                detail: why.code().into(),
                ends,
                retry_at: now + jittered(wait),
                asked_at: now,
                count,
            });
            self.log(format!(
                "cert renewal: the hub's answer was rejected ({}), {count} in a row{}",
                why.code(),
                if given_up {
                    "; the request is given up"
                } else {
                    ""
                }
            ));
            return (Outcome::Failed, wait);
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
                // A stop in the middle: the next look recovers the folder from the pending file (or rolls back to the previous pair)
                // and sends the same request again.
                self.log(format!(
                    "cert renewal: the new certificate could not be stored: {}",
                    e.kind()
                ));
                (Outcome::Failed, self.backoff())
            }
        }
    }

    /// The hub refused: what it means for this machine. Nothing is final: the state tells the owner and rests the asking for a day.
    fn refused(&self, snap: &Snapshot, reason: &str, ends: i64, now: i64) -> (Outcome, Duration) {
        let rest = |wait: Duration, code: &str| {
            self.write_state(&State {
                code: code.into(),
                detail: reason.into(),
                ends,
                retry_at: now + jittered(wait),
                asked_at: now,
                count: 0,
            });
        };
        match Refusal::from_reason(reason) {
            Refusal::JoinAgain => {
                rest(self.timing.refused_every, "join_again");
                self.drop_pending();
                self.log(format!(
                    "cert renewal: the hub refused ({reason}); node {} must join again",
                    clean(&snap.node_id)
                ));
                (
                    Outcome::Refused("join_again".into()),
                    self.timing.check_every,
                )
            }
            Refusal::Superseded => {
                rest(self.timing.refused_every, "superseded");
                self.drop_pending();
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
                rest(wait, "too_frequent");
                self.log("cert renewal: the hub asks to wait; trying again later".into());
                (Outcome::Refused("too_frequent".into()), wait)
            }
            Refusal::Other(r) => {
                self.log(format!("cert renewal: the hub refused ({})", clean(&r)));
                (Outcome::Failed, self.backoff())
            }
        }
    }
}

/// A short stable description of a link error for the log: no certificate or key text is ever in a [`LinkError`].
fn brief(e: &LinkError) -> String {
    match e {
        LinkError::Refused(c) => clean(c),
        other => clean(&format!("{other:?}")).chars().take(120).collect(),
    }
}

impl CertSource for Renewer {
    fn current(&self) -> Option<(String, String)> {
        let _guard = (self.hold)().ok()?;
        recover(&self.dir, self.ops.as_ref());
        usable(&self.dir)
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

    /// [`issue`] as a hub whose clock is `ahead` seconds past this machine's (`node_now`): the certificate starts 5 minutes before
    /// the hub's now.
    fn issue_ahead(ca: &Ca, csr_pem: &str, node_now: i64, ahead: i64, life: i64) -> String {
        let mut csr = CertificateSigningRequestParams::from_pem(csr_pem).unwrap();
        let p = &mut csr.params;
        p.distinguished_name = dn("m");
        p.is_ca = IsCa::ExplicitNoCa;
        p.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        let start = OffsetDateTime::from_unix_timestamp(node_now + ahead).unwrap();
        p.not_before = start - TimeDuration::minutes(5);
        p.not_after = start + TimeDuration::seconds(life);
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
            refused_every: Duration::from_secs(86_400),
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
        // No hub has welcomed this node yet.
        r.hub_offers.store(false, Ordering::Release);
        r.hub_without.store(false, Ordering::Release);
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
        // After the hour (and the tenth more that keeps machines from asking together): the same request.
        clock.fetch_add(2400, Ordering::Relaxed);
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

    /// Audit F1341: a guard that cannot be had (a read-only folder, another user's lock file, a holder that hangs) is a failed look that
    /// is said and backed off, never a silent "nothing to do" while the certificate runs out.
    #[test]
    fn a_link_folder_that_cannot_be_locked_is_a_failed_look_that_is_logged() {
        let j = joined("noguard");
        let asked = Arc::new(AtomicU32::new(0));
        let guard_ok = Arc::new(AtomicBool::new(false));
        let g = Arc::clone(&guard_ok);
        let r = Renewer::with_parts(
            j.dir.clone(),
            timing(),
            || Some(request()),
            move || {
                if g.load(Ordering::Relaxed) {
                    Ok(Box::new(()) as Box<dyn std::any::Any + Send>)
                } else {
                    Err(io::Error::from(io::ErrorKind::PermissionDenied))
                }
            },
            counting(&asked, away()),
            || real_now() + DUE,
            Arc::new(Real),
        );
        r.set_hub_offers(true);
        let waits: Vec<(Outcome, u64)> = (0..3)
            .map(|_| {
                let (o, w) = r.tick();
                (o, w.as_secs())
            })
            .collect();
        assert_eq!(
            waits,
            [
                (Outcome::Failed, 60),
                (Outcome::Failed, 120),
                (Outcome::Failed, 240)
            ]
        );
        let lines = r.log_lines();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("could not be locked"), "{lines:?}");
        assert_eq!(asked.load(Ordering::Relaxed), 0);
        // The guard comes back: the next look asks the hub.
        guard_ok.store(true, Ordering::Relaxed);
        assert_eq!(r.tick().0, Outcome::Failed);
        assert_eq!(asked.load(Ordering::Relaxed), 1);
        assert_old_pair_kept(&j);
        let _ = std::fs::remove_dir_all(&j.dir);
    }

    #[test]
    fn a_certificate_that_has_ended_by_the_clock_is_not_asked_about_and_nothing_is_written_down() {
        let j = joined("ended");
        let asked = Arc::new(AtomicU32::new(0));
        let r = renewer(&j, 100 * DAY, counting(&asked, away()), Arc::new(Real));
        assert_eq!(r.tick().0, Outcome::Ended);
        assert_eq!(asked.load(Ordering::Relaxed), 0);
        assert!(state(&j.dir, ends(&j)).is_none());
        assert!(!j.dir.join(STATE).exists());
        let _ = std::fs::remove_dir_all(&j.dir);
    }

    /// A clock that jumps forward past the certificate's end and then comes back must cost nothing: no verdict was kept.
    #[test]
    fn a_clock_that_jumps_forward_and_back_does_not_wedge_the_machine() {
        let j = joined("jump");
        let clock = Arc::new(AtomicI64::new(DUE));
        let asked = Arc::new(AtomicU32::new(0));
        let r = renewer_clocked(
            &j,
            Arc::clone(&clock),
            counting(&asked, answering(ca_of(&j), "m", 90 * DAY)),
            Arc::new(Real),
        );
        clock.store(100 * DAY, Ordering::Relaxed);
        for _ in 0..3 {
            assert_eq!(r.tick().0, Outcome::Ended);
        }
        assert_eq!(asked.load(Ordering::Relaxed), 0);
        assert!(!j.dir.join(STATE).exists());
        // The clock is right again: the machine renews as if nothing had happened.
        clock.store(DUE, Ordering::Relaxed);
        assert_eq!(r.tick().0, Outcome::Renewed);
        assert_eq!(asked.load(Ordering::Relaxed), 1);
        assert!(same_pair(&j.dir));
        // A state an earlier build wrote for an ended certificate does not hold it either (and is not a retry-never).
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
        assert_eq!(recover(&j.dir, &Real), Recovery::Repaired);
        assert!(same_pair(&j.dir));
        assert_eq!(read_file(&j.dir, "cert.pem").trim(), j.cert.trim());
        // A whole folder is left alone.
        assert_eq!(recover(&j.dir, &Real), Recovery::Whole);
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

    /// B: no refusal is for good. It rests about a day, is asked again, and a success clears it.
    #[test]
    fn a_refusal_rests_about_a_day_is_asked_again_and_a_success_clears_it() {
        for reason in ["unknown", "superseded", "expired", "too_old", "revoked"] {
            let j = joined(&format!("again-{reason}"));
            let clock = Arc::new(AtomicI64::new(DUE));
            let healed = Arc::new(AtomicBool::new(false));
            let asked = Arc::new(AtomicU32::new(0));
            let ca = ca_of(&j);
            let (h, a) = (Arc::clone(&healed), Arc::clone(&asked));
            let ask: AskFn = Box::new(move |_, csr| {
                a.fetch_add(1, Ordering::Relaxed);
                if h.load(Ordering::Relaxed) {
                    Ok(issue(&ca, csr, "m", 90 * DAY))
                } else {
                    Err(RenewError::Refused {
                        code: "enroll.renew_refused".into(),
                        reason: reason.into(),
                    })
                }
            });
            let r = renewer_clocked(&j, Arc::clone(&clock), ask, Arc::new(Real));
            assert!(matches!(r.tick().0, Outcome::Refused(_)), "{reason}");
            let s = state(&j.dir, ends(&j)).unwrap();
            assert!(
                s.asked_at > 0 && s.retry_at >= s.asked_at + 86_400,
                "{reason}"
            );
            assert!(
                s.retry_at <= s.asked_at + 86_400 + 8_641,
                "{reason}: jitter is a tenth"
            );
            // Within the day: not asked, the owner is still told.
            clock.fetch_add(3 * 3600, Ordering::Relaxed);
            assert!(matches!(r.tick().0, Outcome::Refused(_)));
            assert_eq!(asked.load(Ordering::Relaxed), 1, "{reason}");
            // A day and a tenth later: asked again (still refused: still told, rested again).
            clock.fetch_add(2 * DAY, Ordering::Relaxed);
            assert!(matches!(r.tick().0, Outcome::Refused(_)));
            assert_eq!(asked.load(Ordering::Relaxed), 2, "{reason}");
            // The hub was at fault and is fine now: the next ask succeeds and nothing of the refusal is left.
            healed.store(true, Ordering::Relaxed);
            clock.fetch_add(2 * DAY, Ordering::Relaxed);
            assert_eq!(r.tick().0, Outcome::Renewed, "{reason}");
            assert!(!j.dir.join(STATE).exists(), "{reason}");
            assert!(state(&j.dir, validity(&read_file(&j.dir, "cert.pem")).unwrap().1).is_none());
            let _ = std::fs::remove_dir_all(&j.dir);
        }
    }

    /// A state file that cannot be read, or that asks for a rest of a thousand years, holds nothing back for long.
    #[test]
    fn a_damaged_or_hostile_state_file_never_wedges_the_asking() {
        let j = joined("state");
        let clock = Arc::new(AtomicI64::new(DUE));
        let asked = Arc::new(AtomicU32::new(0));
        let r = renewer_clocked(
            &j,
            Arc::clone(&clock),
            counting(&asked, away()),
            Arc::new(Real),
        );
        for junk in [
            "".to_string(),
            "join_again".to_string(),
            "join_again\nunknown\nnot a number\n".to_string(),
            "\u{0}\u{1}\u{2}".to_string(),
        ] {
            std::fs::write(j.dir.join(STATE), junk).unwrap();
            let before = asked.load(Ordering::Relaxed);
            r.tick();
            assert_eq!(
                asked.load(Ordering::Relaxed),
                before + 1,
                "a damaged file held the asking back"
            );
            r.fails.store(0, Ordering::Release);
        }
        // A rest of a thousand years is cut to two days.
        let e = ends(&j);
        std::fs::write(
            j.dir.join(STATE),
            format!(
                "join_again\nunknown\n{e}\n{}\n1\n0\n",
                real_now() + 1000 * 365 * DAY
            ),
        )
        .unwrap();
        let before = asked.load(Ordering::Relaxed);
        assert!(matches!(r.tick().0, Outcome::Refused(_)));
        assert_eq!(asked.load(Ordering::Relaxed), before);
        clock.fetch_add(3 * DAY, Ordering::Relaxed);
        assert_eq!(r.tick().0, Outcome::Failed);
        assert_eq!(asked.load(Ordering::Relaxed), before + 1);
        // The shorter form an earlier build wrote is read too.
        std::fs::write(j.dir.join(STATE), format!("rejected\nkey\n{e}\n0\n")).unwrap();
        assert_eq!(state(&j.dir, e).unwrap().code, "rejected");
        let _ = std::fs::remove_dir_all(&j.dir);
    }

    /// Text from the hub is cut and stripped before it is kept or logged.
    #[test]
    fn a_reason_from_the_hub_is_cut_and_stripped_of_control_characters() {
        let j = joined("sanitize");
        let nasty = format!("un\nknown\u{1b}[31m{}", "x".repeat(500));
        let r = renewer(
            &j,
            DUE,
            Box::new(move |_, _| {
                Err(RenewError::Refused {
                    code: "enroll.renew_refused".into(),
                    reason: nasty.clone(),
                })
            }),
            Arc::new(Real),
        );
        r.tick();
        assert!(!r.log_lines().is_empty());
        for l in r.log_lines() {
            assert!(!l.chars().any(char::is_control), "{l:?}");
            assert!(l.chars().count() < 400, "{}", l.len());
        }
        // A state file that carries such text (written by anything) is read cut and clean, in the one line it has.
        let e = ends(&j);
        std::fs::write(
            j.dir.join(STATE),
            format!("join_again\n{}\u{1b}[2J\n{e}\n0\n1\n0\n", "y".repeat(900)),
        )
        .unwrap();
        let s = state(&j.dir, e).unwrap();
        assert!(s.detail.chars().count() <= 200);
        assert!(!s.detail.chars().any(char::is_control));
        let _ = std::fs::remove_dir_all(&j.dir);
    }

    /// E: an answer that never checks out is asked for five times, then the request is given up and a new one is made.
    #[test]
    fn answers_that_never_check_out_are_given_up_after_five_and_a_new_request_is_made() {
        let j = joined("giveup");
        let clock = Arc::new(AtomicI64::new(DUE));
        let csrs = Arc::new(Mutex::new(Vec::<String>::new()));
        let (c2, ca) = (Arc::clone(&csrs), ca_of(&j));
        let ask: AskFn = Box::new(move |_, csr| {
            c2.lock().unwrap().push(csr.to_string());
            // Always for another node.
            Ok(issue(&ca, csr, "someone-else", 90 * DAY))
        });
        let r = renewer_clocked(&j, Arc::clone(&clock), ask, Arc::new(Real));
        for n in 1..=5u32 {
            assert_eq!(r.tick().0, Outcome::Failed);
            let s = state(&j.dir, ends(&j)).unwrap();
            assert_eq!((s.code.as_str(), s.count), ("rejected", n));
            // Until its time nothing is asked.
            assert_eq!(r.tick().0, Outcome::Refused("rejected".into()));
            clock.fetch_add(2 * 3600, Ordering::Relaxed);
        }
        assert_eq!(csrs.lock().unwrap().len(), 5);
        {
            let c = csrs.lock().unwrap();
            assert!(
                c.iter().all(|x| *x == c[0]),
                "the same request is asked for again"
            );
        }
        assert!(
            !pending_exists(&j.dir),
            "the request was kept after five rejected answers"
        );
        let s = state(&j.dir, ends(&j)).unwrap();
        assert!(s.retry_at > real_now() + clock.load(Ordering::Relaxed) + 20 * 3600);
        // A day later the next due look makes a NEW request.
        clock.fetch_add(2 * DAY, Ordering::Relaxed);
        assert_eq!(r.tick().0, Outcome::Failed);
        let c = csrs.lock().unwrap();
        assert_eq!(c.len(), 6);
        assert_ne!(c[5], c[0]);
        drop(c);
        // The new request starts again from one, and is not given up before its own fifth rejected answer.
        assert_eq!(state(&j.dir, ends(&j)).unwrap().count, 1);
        assert!(pending_exists(&j.dir));
        for n in 2..=4u32 {
            clock.fetch_add(2 * 3600, Ordering::Relaxed);
            assert_eq!(r.tick().0, Outcome::Failed);
            assert_eq!(state(&j.dir, ends(&j)).unwrap().count, n);
            assert!(pending_exists(&j.dir), "given up after {n}");
        }
        let _ = std::fs::remove_dir_all(&j.dir);
    }

    /// ADR-0085 Amendment A (F1340): this machine's clock is 6 hours behind the hub's. The hub's answer starts in this machine's
    /// future; it is taken at once.
    #[test]
    fn a_clock_six_hours_behind_renews_at_once() {
        let j = joined("behind6");
        let ca = ca_of(&j);
        let node_now = real_now() + DUE;
        let ask: AskFn =
            Box::new(move |_, csr| Ok(issue_ahead(&ca, csr, node_now, 6 * 3600, 90 * DAY)));
        let r = renewer(&j, DUE, ask, Arc::new(Real));
        assert_eq!(r.tick().0, Outcome::Renewed);
        assert!(!pending_exists(&j.dir));
        let _ = std::fs::remove_dir_all(&j.dir);
    }

    /// ADR-0085 Amendment A (F1340): 60 hours behind is past the bound (and stays past it while the test's clock runs 16 hours). The answer is rejected for `validity`, but the request (and
    /// so the only key for the certificate the hub issued) is kept through more than five answers, and the same answer is taken once
    /// the clock is right; asking anew would only be answered `superseded`.
    #[test]
    fn a_clock_far_behind_keeps_the_request_and_renews_once_the_clock_is_right() {
        let j = joined("behind60");
        let ca = ca_of(&j);
        let clock = Arc::new(AtomicI64::new(DUE));
        let issued = Arc::new(Mutex::new(None::<String>));
        let csrs = Arc::new(Mutex::new(Vec::<String>::new()));
        let hub_now = real_now() + DUE + 60 * 3600;
        let (i2, c2) = (Arc::clone(&issued), Arc::clone(&csrs));
        // The hub issues once, then answers the same request with the same certificate.
        let ask: AskFn = Box::new(move |_, csr| {
            c2.lock().unwrap().push(csr.to_string());
            let mut i = i2.lock().unwrap();
            Ok(
                i.get_or_insert_with(|| issue_ahead(&ca, csr, hub_now, 0, 90 * DAY))
                    .clone(),
            )
        });
        let r = renewer_clocked(&j, Arc::clone(&clock), ask, Arc::new(Real));
        for n in 0..8 {
            assert_eq!(r.tick().0, Outcome::Failed, "answer {n}");
            let s = state(&j.dir, ends(&j)).unwrap();
            assert_eq!(
                (s.code.as_str(), s.detail.as_str(), s.count),
                ("rejected", "validity", 0)
            );
            assert!(
                pending_exists(&j.dir),
                "the request was given up after {n} answers"
            );
            clock.fetch_add(2 * 3600, Ordering::Relaxed);
        }
        {
            let c = csrs.lock().unwrap();
            assert!(
                c.iter().all(|x| *x == c[0]),
                "the same request is asked for again"
            );
        }
        // The clock is set right: the hub's start is now in this machine's past.
        clock.store(DUE + 60 * 3600, Ordering::Relaxed);
        assert_eq!(r.tick().0, Outcome::Renewed);
        assert!(!pending_exists(&j.dir));
        let _ = std::fs::remove_dir_all(&j.dir);
    }

    /// An older hub: back off to once an hour, say so once, tell the doctor; a hub that does offer it clears that.
    #[test]
    fn an_older_hub_is_asked_about_once_an_hour_said_once_and_cleared_when_a_newer_one_answers() {
        let j = joined("oldhub");
        let asked = Arc::new(AtomicU32::new(0));
        let r = renewer(&j, DUE, counting(&asked, away()), Arc::new(Real));
        r.welcomed(&["terminal.raw".to_string()]);
        for _ in 0..4 {
            assert_eq!(
                r.tick(),
                (Outcome::WaitingForHub, Duration::from_secs(1000))
            );
        }
        assert_eq!(asked.load(Ordering::Relaxed), 0);
        assert_eq!(state(&j.dir, ends(&j)).unwrap().code, "hub_old");
        let said: Vec<_> = r
            .log_lines()
            .into_iter()
            .filter(|l| l.contains("does not support renewal"))
            .collect();
        assert_eq!(said.len(), 1, "{said:?}");
        r.welcomed(&[CAPABILITY.to_string()]);
        assert_eq!(r.tick().0, Outcome::Failed);
        assert!(!j.dir.join(STATE).exists());
        assert_eq!(asked.load(Ordering::Relaxed), 1);
        let _ = std::fs::remove_dir_all(&j.dir);
    }

    /// D: a request made for a certificate that is no longer the one in use (the machine joined again) is never sent.
    #[test]
    fn a_pending_request_from_before_a_rejoin_is_dropped_and_a_new_one_is_made() {
        let j = joined("stale");
        let seen: Seen = Arc::default();
        let r = renewer(&j, DUE, watching(&j.dir, &seen), Arc::new(Real));
        assert_eq!(r.tick().0, Outcome::Failed);
        assert!(pending_exists(&j.dir));
        // The machine joins again: another certificate and key, the old files for the earlier identity left in the folder.
        let (key2, csr2) = request();
        std::fs::write(
            j.dir.join("cert.pem"),
            issue(&ca_of(&j), &csr2, "m", 60 * DAY),
        )
        .unwrap();
        std::fs::write(j.dir.join("key.pem"), key2).unwrap();
        let r2 = renewer(&j, DUE, watching(&j.dir, &seen), Arc::new(Real));
        assert_eq!(r2.tick().0, Outcome::Failed);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_ne!(
            seen[0].0, seen[1].0,
            "a request made for the earlier certificate was sent"
        );
        // And the one sent is the one now kept, for the certificate now in use.
        let p = Pending::decode(&read_file(&j.dir, PENDING)).unwrap();
        assert_eq!(p.csr_pem, seen[1].0.trim());
        assert_eq!(p.for_cert, fingerprint(&read_file(&j.dir, "cert.pem")));
        let _ = std::fs::remove_dir_all(&j.dir);
    }

    /// D: the pair in `previous.pem` is never put back for another node id than the link's.
    #[test]
    fn a_previous_pair_of_another_identity_is_never_put_back() {
        let j = joined("otheridentity");
        // A previous pair for node "other" (a revoked identity this folder used to be), and a mismatched current pair.
        let (okey, ocsr) = request();
        let ocert = issue(&ca_of(&j), &ocsr, "other", 60 * DAY);
        std::fs::write(
            j.dir.join(PREVIOUS),
            format!("{}\n{}\n", ocert.trim(), okey.trim()),
        )
        .unwrap();
        let (key2, _) = request();
        std::fs::write(j.dir.join("key.pem"), key2).unwrap();
        let before = (read_file(&j.dir, "cert.pem"), read_file(&j.dir, "key.pem"));
        assert_eq!(recover(&j.dir, &Real), Recovery::Unrepaired);
        assert_eq!(
            (read_file(&j.dir, "cert.pem"), read_file(&j.dir, "key.pem")),
            before
        );
        assert_eq!(usable(&j.dir), None);
        // No `node-id` (a save that stopped half way) restores nothing either.
        std::fs::write(
            j.dir.join(PREVIOUS),
            format!("{}\n{}\n", j.cert.trim(), j.key.trim()),
        )
        .unwrap();
        std::fs::remove_file(j.dir.join("node-id")).unwrap();
        assert_eq!(recover(&j.dir, &Real), Recovery::Unrepaired);
        let _ = std::fs::remove_dir_all(&j.dir);
    }

    /// F: temporary files a stopped writer left (they may hold a key) are removed, and only those.
    #[test]
    fn leftover_temporary_files_of_the_folders_writers_are_removed_and_nothing_else() {
        let j = joined("sweep");
        for f in [
            "cert.pem.tmp-123",
            "key.pem.tmp-9",
            "renew-pending.pem.tmp-4242",
            "previous.pem.tmp-1",
            "renew-state.tmp-7",
            // write_private's name from 0.3.41 on.
            "key.pem.tmp-9-0",
            "renew-pending.pem.tmp-4242-17",
        ] {
            std::fs::write(j.dir.join(f), "KEY").unwrap();
        }
        for f in [
            "cert.pem.tmp-",
            "cert.pem.tmp-x1",
            "cert.pem.tmp-12x",
            "key.pem.tmp-9-",
            "notes.tmp-1",
            "mine.pem",
            "lock",
        ] {
            std::fs::write(j.dir.join(f), "keep").unwrap();
        }
        // A folder named like a temporary file is left alone; a link named like one may go, never what it points at.
        std::fs::create_dir(j.dir.join("url.tmp-77")).unwrap();
        std::fs::write(j.dir.join("url.tmp-77").join("inside"), "keep").unwrap();
        let target = folder("sweep-target");
        std::fs::write(target.join("secret"), "keep").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&target, j.dir.join("ca.pem.tmp-88")).unwrap();
            std::os::unix::fs::symlink(target.join("secret"), j.dir.join("node-id.tmp-5")).unwrap();
        }
        assert_eq!(recover(&j.dir, &Real), Recovery::Whole);
        assert_eq!(
            files(&j.dir),
            [
                "ca.pem",
                "cert.pem",
                "cert.pem.tmp-",
                "cert.pem.tmp-12x",
                "cert.pem.tmp-x1",
                "key.pem",
                "key.pem.tmp-9-",
                "lock",
                "mine.pem",
                "node-id",
                "notes.tmp-1",
                "url",
                "url.tmp-77"
            ]
        );
        assert_eq!(read_file(&target, "secret"), "keep");
        assert_eq!(read_file(&j.dir.join("url.tmp-77"), "inside"), "keep");
        let _ = std::fs::remove_dir_all(&target);
        let _ = std::fs::remove_dir_all(&j.dir);
    }

    /// Fails every operation from the n-th on (a full disk), until healed.
    struct Disk {
        from: u32,
        seen: AtomicU32,
        healed: AtomicBool,
    }

    impl Disk {
        fn full_from(n: u32) -> Arc<Self> {
            Arc::new(Self {
                from: n,
                seen: AtomicU32::new(0),
                healed: AtomicBool::new(false),
            })
        }

        fn step(&self) -> io::Result<()> {
            let n = self.seen.fetch_add(1, Ordering::AcqRel);
            if n >= self.from && !self.healed.load(Ordering::Acquire) {
                return Err(io::Error::other("disk full"));
            }
            Ok(())
        }
    }

    impl Ops for Disk {
        fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
            self.step()?;
            Real.write(path, bytes)
        }

        fn remove(&self, path: &Path) -> io::Result<()> {
            self.step()?;
            Real.remove(path)
        }
    }

    /// C: the disk stays broken from any step of an install on, through the restarts: the node always has one MATCHING pair to dial with.
    /// When the disk is healed the renewal finishes with exactly one new certificate.
    #[test]
    fn a_disk_that_fails_from_any_step_on_never_leaves_a_certificate_and_key_that_do_not_match() {
        for from in 0..8u32 {
            let j = joined(&format!("full{from}"));
            let made = Arc::new(Mutex::new(Vec::<String>::new()));
            let disk = Disk::full_from(from);
            let lives: Vec<Arc<Renewer>> = (0..3)
                .map(|_| {
                    renewer(
                        &j,
                        DUE,
                        idempotent(ca_of(&j), &made),
                        Arc::clone(&disk) as Arc<dyn Ops>,
                    )
                })
                .collect();
            for (n, life) in lives.iter().enumerate() {
                let _ = life.tick();
                let (c, k) = life
                    .current()
                    .unwrap_or_else(|| panic!("from {from}, life {n}: no pair to dial with"));
                assert!(
                    pair_matches(&c, &k),
                    "from {from}, life {n}: the pair does not match"
                );
                // What recover says is true: it never reports a whole folder that is not.
                let r = recover(&j.dir, disk.as_ref());
                if r != Recovery::Unrepaired {
                    assert!(
                        same_pair(&j.dir),
                        "from {from}, life {n}: {r:?} for a mismatched folder"
                    );
                }
                assert!(usable(&j.dir).is_some_and(|(c, k)| pair_matches(&c, &k)));
            }
            // The disk is fixed; the next start finishes it.
            disk.healed.store(true, Ordering::Release);
            let last = renewer(&j, DUE, idempotent(ca_of(&j), &made), Arc::new(Real));
            assert!(last.current().is_some());
            let _ = last.tick();
            let _ = last.tick();
            assert!(same_pair(&j.dir), "from {from}");
            let made = made.lock().unwrap();
            assert_eq!(made.len(), 1, "from {from}: not exactly one certificate");
            assert_eq!(
                read_file(&j.dir, "cert.pem").trim(),
                made[0].trim(),
                "from {from}"
            );
            assert!(!pending_exists(&j.dir), "from {from}");
            let _ = std::fs::remove_dir_all(&j.dir);
        }
    }

    /// Fails every write of one file, for good.
    struct NoWrites(&'static str);

    impl Ops for NoWrites {
        fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
            if path.file_name().is_some_and(|n| n == self.0) {
                return Err(io::Error::other("cannot write this file"));
            }
            Real.write(path, bytes)
        }

        fn remove(&self, path: &Path) -> io::Result<()> {
            Real.remove(path)
        }
    }

    /// C: the key cannot be written for good: the folder goes back to the pair in use, the node keeps dialling with it, and nothing loops
    /// in the folder (each try leaves one whole pair).
    #[test]
    fn a_key_that_cannot_be_written_rolls_back_to_the_pair_in_use() {
        let j = joined("nokey");
        let made = Arc::new(Mutex::new(Vec::<String>::new()));
        let ops: Arc<dyn Ops> = Arc::new(NoWrites("key.pem"));
        for _ in 0..3 {
            let r = renewer(&j, DUE, idempotent(ca_of(&j), &made), Arc::clone(&ops));
            assert_eq!(r.tick().0, Outcome::Failed);
            let (c, k) = r.current().unwrap();
            assert!(pair_matches(&c, &k));
            assert!(same_pair(&j.dir), "the folder is not one pair");
            assert_eq!(read_file(&j.dir, "cert.pem").trim(), j.cert.trim());
            assert_eq!(read_file(&j.dir, "key.pem").trim(), j.key.trim());
        }
        assert_eq!(made.lock().unwrap().len(), 1);
        // The key file is writable again: the renewal finishes.
        let r = renewer(&j, DUE, idempotent(ca_of(&j), &made), Arc::new(Real));
        assert_eq!(r.tick().0, Outcome::Renewed);
        assert!(same_pair(&j.dir));
        let _ = std::fs::remove_dir_all(&j.dir);
    }

    /// The new certificate is in place but its key could not be written, and there is no previous pair to go back to: the pending
    /// file holds the key, so the new pair is what the node dials with (from memory), and it is not a loop of asking.
    #[test]
    fn a_new_certificate_whose_key_was_not_written_is_used_with_the_pending_key() {
        let j = joined("memory");
        let (key2, csr2) = request();
        let new_cert = issue(&ca_of(&j), &csr2, "m", 90 * DAY);
        std::fs::write(j.dir.join("cert.pem"), &new_cert).unwrap();
        std::fs::write(
            j.dir.join(PENDING),
            Pending::new(&key2, &csr2).bound(&j.cert).encode(),
        )
        .unwrap();
        assert!(!same_pair(&j.dir));
        let (c, k) = usable(&j.dir).unwrap();
        assert_eq!(c, new_cert);
        assert!(pair_matches(&c, &k));
        let asked = Arc::new(AtomicU32::new(0));
        let r = renewer(
            &j,
            DUE,
            counting(&asked, away()),
            Arc::new(NoWrites("key.pem")),
        );
        assert_eq!(r.current(), Some((c, k)));
        assert_eq!(r.tick().0, Outcome::Failed);
        assert_eq!(
            asked.load(Ordering::Relaxed),
            0,
            "asked while only the key file is missing"
        );
        assert!(
            pending_exists(&j.dir),
            "the only copy of the new key was dropped"
        );
        let _ = std::fs::remove_dir_all(&j.dir);
    }

    #[test]
    fn hub_text_loses_direction_marks_and_zero_width_characters() {
        let text = "ab\u{202E}cd\u{2066}e\u{200B}f\u{200F}g\u{2028}h\u{FEFF}i\u{202A}j";
        assert_eq!(clean(text), "abcdefghij");
        assert_eq!(clean("plain text, ünïcode ok"), "plain text, ünïcode ok");
    }

    #[test]
    fn a_state_or_pending_file_larger_than_the_cap_is_not_read() {
        let j = joined("cap");
        let e = ends(&j);
        let line = format!("join_again\nunknown\n{e}\n0\n1\n0\n");
        std::fs::write(j.dir.join(STATE), &line).unwrap();
        assert!(state(&j.dir, e).is_some());
        let mut big = line.clone();
        big.push_str(&"#".repeat(70 * 1024));
        std::fs::write(j.dir.join(STATE), &big).unwrap();
        assert!(state(&j.dir, e).is_none());
        let (k, c) = request();
        let p = Pending::new(&k, &c).bound(&j.cert);
        let mut text = p.encode();
        std::fs::write(j.dir.join(PENDING), &text).unwrap();
        assert!(read(&j.dir, PENDING)
            .and_then(|t| Pending::decode(&t))
            .is_some());
        text.push_str(&" ".repeat(70 * 1024));
        std::fs::write(j.dir.join(PENDING), text).unwrap();
        assert!(read(&j.dir, PENDING).is_none());
        let _ = std::fs::remove_dir_all(&j.dir);
    }
}
