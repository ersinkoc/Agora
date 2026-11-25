//! Pairing: the hub's side of "find a hub, compare a number, connected" (ADR-0065, `docs/specs/pairing.md`), as plain state.
//!
//! This module is a **state machine and nothing else**: no sockets, no tasks, no clock of its own (every method takes `now_ms`), no
//! files. The runtime (`pairing_net.rs`) owns connections and the certificate authority and only does what the answers here say.
//! So every rule that decides who may join is tested on plain values.
//!
//! A machine's session is `Awaiting -> Pending -> Approved | Done(denied | expired | closed)` and belongs to **one connection**
//! (section 5a): the `pair_id` is a lookup key, never a capability. At the commit the session freezes the owning connection, the
//! TLS exporter, the hub's leaf hash, the commitment, the source address and the reveal deadline; at the reveal it adds the nonce,
//! the CSR hash, the node id and the approval deadline. Nothing changes afterwards, and a certificate is issued, at most once, for
//! exactly that tuple.
//!
//! Never kept in any log, event or message except the owner's card: the six-digit number. Never in any message at all: the nonces,
//! the commitment, the exporter and the CSR. (`Debug` here is written by hand so that a stray `{:?}` cannot leak them.)

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::net::IpAddr;

use agora_protocol::{Error, ErrorCode};
use ring::hkdf;
use sha2::{Digest, Sha256};

/// How long the switch stays open once opened (section 2).
pub const WINDOW_MS: u64 = 10 * 60_000;
/// A session lives this long from the reveal (section 7).
pub const SESSION_MS: u64 = 5 * 60_000;
/// A reveal must arrive this long after the challenge at the latest.
pub const REVEAL_MS: u64 = 30_000;
/// How long a commitment, an exporter or a nonce is remembered to refuse a repeat (section 5a).
pub const REPLAY_MS: u64 = 10 * 60_000;
/// Pending machine sessions (awaiting a reveal or a decision) at one time.
pub const MAX_PENDING_MACHINES: usize = 4;
/// Pending browser sessions in all, and for one address key.
pub const MAX_PENDING_BROWSERS: usize = 4;
pub const MAX_PENDING_BROWSERS_PER_KEY: usize = 2;
/// Admissions (attempts, refused ones included) from one address key.
pub const KEY_PER_MINUTE: usize = 3;
pub const KEY_PER_10_MINUTES: usize = 20;
/// Admissions for the whole hub, each kind on its own.
pub const HUB_PER_MINUTE: usize = 10;
pub const HUB_PER_10_MINUTES: usize = 60;
/// Failures from one address key that lock it out, and for how long.
pub const KEY_FAILURES: usize = 5;
pub const LOCKOUT_MS: u64 = 60_000;
/// Failures while the switch is open that close it.
pub const WINDOW_FAILURES: u32 = 5;
/// The counter table: keys at most, and how long an idle key is kept.
pub const MAX_KEYS: usize = 1024;
pub const RETAIN_MS: u64 = 10 * 60_000;
/// A push notice for the whole hub at most this often (section 6).
pub const PUSH_EVERY_MS: u64 = 60_000;
/// Refusals are reported (audited) as a rate this often, not one line each (section 8).
pub const REFUSAL_REPORT_MS: u64 = 60_000;
/// Window notices kept for a window that is a moment behind.
const NOTES_KEPT: usize = 64;
/// Longest display text from a machine (bytes).
const TEXT_MAX: usize = 64;

pub type PairId = [u8; 16];

/// A connection, as the runtime numbers them. The session belongs to one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnId(pub u64);

/// Where a message came from: its connection and the address that connection came from.
#[derive(Debug, Clone, Copy)]
pub struct Peer {
    pub conn: ConnId,
    pub addr: IpAddr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Machine,
    Browser,
}

impl Kind {
    fn index(self) -> usize {
        match self {
            Self::Machine => 0,
            Self::Browser => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Approved,
    Denied,
    Expired,
    Closed,
}

/// Why a message was refused. The runtime maps it to a stable error code ([`Refusal::to_error`]); no text is made here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The switch is closed (or the hub cannot pair at all).
    Closed,
    Limit {
        seconds: u64,
    },
    Expired,
    Mismatch,
    Unknown,
    /// A message that is not allowed in the session's state.
    State(&'static str),
    /// A field that is out of bounds.
    Invalid(&'static str),
}

impl Refusal {
    pub fn to_error(&self) -> Error {
        match self {
            Self::Closed => Error::new(ErrorCode::PairClosed),
            Self::Limit { seconds } => Error::new(ErrorCode::PairLimit)
                .with("seconds", i64::try_from(*seconds).unwrap_or(i64::MAX)),
            Self::Expired => Error::new(ErrorCode::PairExpired),
            Self::Mismatch => Error::new(ErrorCode::PairMismatch),
            Self::Unknown => Error::new(ErrorCode::PairUnknown),
            Self::State(state) => Error::new(ErrorCode::PairState).with("state", *state),
            Self::Invalid(field) => {
                Error::new(ErrorCode::ProtocolInvalidMessage).with("field", *field)
            }
        }
    }
}

// ---- the arithmetic: commitment and the number ----

/// `SHA-256("agora/pair/v1/commit" || nonce_n || csr_hash)` (section 5, step 1).
pub fn commitment(nonce_n: &[u8; 16], csr_hash: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"agora/pair/v1/commit");
    h.update(nonce_n);
    h.update(csr_hash);
    h.finalize().into()
}

struct EightBytes;

impl hkdf::KeyType for EightBytes {
    fn len(&self) -> usize {
        8
    }
}

/// The number both sides show (section 5, step 4): `HKDF-SHA256(salt = "agora/pair/v1", ikm = exporter || leaf_hash || csr_hash ||
/// nonce_n || nonce_h, info = "sas")`, its first 8 bytes as a big-endian integer, modulo 10^6.
pub fn sas(
    exporter: &[u8; 32],
    leaf_hash: &[u8; 32],
    csr_hash: &[u8; 32],
    nonce_n: &[u8; 16],
    nonce_h: &[u8; 16],
) -> u32 {
    let mut ikm = Vec::with_capacity(32 * 3 + 16 * 2);
    ikm.extend_from_slice(exporter);
    ikm.extend_from_slice(leaf_hash);
    ikm.extend_from_slice(csr_hash);
    ikm.extend_from_slice(nonce_n);
    ikm.extend_from_slice(nonce_h);
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, b"agora/pair/v1").extract(&ikm);
    let mut out = [0u8; 8];
    // Eight bytes are far below the limit (255 x 32), so expanding cannot fail.
    if let Ok(okm) = prk.expand(&[b"sas"], EightBytes) {
        if okm.fill(&mut out).is_err() {
            out = [0; 8];
        }
    }
    u32::try_from(u64::from_be_bytes(out) % 1_000_000).unwrap_or(0)
}

/// The number as six digits, `482193`.
pub fn digits(number: u32) -> String {
    format!("{number:06}")
}

/// The counter key of an address: the exact address for IPv4, the /64 for IPv6 (section 6).
pub fn address_key(addr: IpAddr) -> IpAddr {
    match addr.to_canonical() {
        IpAddr::V6(v6) => {
            let mut o = v6.octets();
            o[8..].fill(0);
            IpAddr::V6(o.into())
        }
        v4 => v4,
    }
}

// ---- the counters ----

#[derive(Default)]
struct KeyState {
    attempts: VecDeque<u64>,
    failures: VecDeque<u64>,
    locked_until: u64,
    last_seen: u64,
}

/// Admission and failure counters, keyed by `(kind, address key)`, plus one counter for the whole hub. Separate from
/// `enroll::Lockout` on purpose (section 7): that one has no address in its interface, so sharing it would either not limit per
/// address or let pairing noise lock out `agora node enroll` for everyone.
///
/// The table holds at most [`MAX_KEYS`] keys and keeps an idle key [`RETAIN_MS`]; when it is full a new key is **refused**, and a
/// live key is never evicted to make room, so filling the table cannot reset anyone's counters.
#[derive(Default)]
pub struct PairLimits {
    keys: HashMap<(Kind, IpAddr), KeyState>,
    hub: [VecDeque<u64>; 2],
    last_prune: u64,
}

fn seconds_until(ms: u64) -> u64 {
    ms.div_ceil(1000).max(1)
}

impl PairLimits {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of keys held (for tests and for the status).
    pub fn keys(&self) -> usize {
        self.keys.len()
    }

    fn prune(&mut self, now: u64) {
        if now.saturating_sub(self.last_prune) < 1000 && self.last_prune != 0 {
            return;
        }
        self.last_prune = now;
        self.keys
            .retain(|_, k| k.locked_until > now || now.saturating_sub(k.last_seen) < RETAIN_MS);
        for h in &mut self.hub {
            while h
                .front()
                .is_some_and(|t| now.saturating_sub(*t) >= RETAIN_MS)
            {
                h.pop_front();
            }
        }
    }

    /// Counts an attempt to start a request (a refused one counts too) and says whether it may go on. Called before anything is
    /// allocated or anyone is told. `Err(seconds)` is when to try again.
    pub fn admit(&mut self, kind: Kind, key: IpAddr, now: u64) -> Result<(), u64> {
        self.prune(now);
        let k = (kind, key);
        if !self.keys.contains_key(&k) && self.keys.len() >= MAX_KEYS {
            return Err(LOCKOUT_MS / 1000);
        }
        // The address's own limit comes first, and every attempt from it counts, refused ones too: one host that hammers is
        // refused by its own counter and uses up nobody else's share.
        let state = self.keys.entry(k).or_default();
        state.last_seen = now;
        state.attempts.push_back(now);
        while state.attempts.len() > KEY_PER_10_MINUTES + 1 {
            state.attempts.pop_front();
        }
        if state.locked_until > now {
            return Err(seconds_until(state.locked_until - now));
        }
        if let Some(wait) = Self::over(&state.attempts, now, KEY_PER_MINUTE, KEY_PER_10_MINUTES) {
            return Err(wait);
        }
        // Only an attempt that passed its address reaches the hub-wide counter, the last guard: many addresses together are still
        // bounded, but a single address cannot use the hub's share up.
        let hub = &mut self.hub[kind.index()];
        hub.push_back(now);
        while hub.len() > HUB_PER_10_MINUTES + 1 {
            hub.pop_front();
        }
        match Self::over(hub, now, HUB_PER_MINUTE, HUB_PER_10_MINUTES) {
            Some(wait) => Err(wait),
            None => Ok(()),
        }
    }

    /// `Some(seconds)` when more than `per_minute` entries are within the last minute or more than `per_ten` within ten.
    fn over(times: &VecDeque<u64>, now: u64, per_minute: usize, per_ten: usize) -> Option<u64> {
        let within = |span: u64| {
            times
                .iter()
                .filter(|t| now.saturating_sub(**t) < span)
                .copied()
                .collect::<Vec<u64>>()
        };
        let mut wait: Option<u64> = None;
        for (span, limit) in [(60_000, per_minute), (RETAIN_MS, per_ten)] {
            let w = within(span);
            if w.len() > limit {
                // The entry that has to leave the window before one more fits: the retry is an attempt too, so only `limit - 1`
                // of these may still be inside then (F800).
                let leaves = w[w.len() - limit] + span;
                let secs = seconds_until(leaves.saturating_sub(now));
                wait = Some(wait.map_or(secs, |x| x.max(secs)));
            }
        }
        wait
    }

    /// How long this key is locked out, if it is.
    pub fn locked(&self, kind: Kind, key: IpAddr, now: u64) -> Option<u64> {
        self.keys
            .get(&(kind, key))
            .filter(|k| k.locked_until > now)
            .map(|k| seconds_until(k.locked_until - now))
    }

    /// A failure from this key. The [`KEY_FAILURES`]th locks it out for [`LOCKOUT_MS`]; returns whether it locked now. A key the
    /// table does not hold (and has no room for) was never admitted, so there is nothing to lock.
    pub fn failure(&mut self, kind: Kind, key: IpAddr, now: u64) -> bool {
        let k = (kind, key);
        if !self.keys.contains_key(&k) && self.keys.len() >= MAX_KEYS {
            return false;
        }
        let state = self.keys.entry(k).or_default();
        state.last_seen = now;
        state.failures.push_back(now);
        while state
            .failures
            .front()
            .is_some_and(|t| now.saturating_sub(*t) >= RETAIN_MS)
        {
            state.failures.pop_front();
        }
        if state.failures.len() >= KEY_FAILURES {
            state.failures.clear();
            state.locked_until = now.saturating_add(LOCKOUT_MS);
            return true;
        }
        false
    }
}

// ---- what windows see ----

/// A card: what the owner sees about a request. Its `Debug` leaves the number out.
#[derive(Clone, PartialEq, Eq)]
pub struct Card {
    pub pair_id: PairId,
    pub kind: Kind,
    pub name: String,
    pub os: String,
    pub address: String,
    /// Six digits.
    pub number: String,
    pub expires_ms: u64,
}

impl fmt::Debug for Card {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Card")
            .field("kind", &self.kind)
            .field("name", &self.name)
            .field("number", &"<hidden>")
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Note {
    Requested(Card),
    Decided { pair_id: PairId, outcome: Outcome },
}

/// What the owning connection does after the owner decided.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing yet.
    Waiting,
    /// Issue one certificate for exactly this, and nothing else. Given once.
    Approved(Issue),
    Denied,
    Expired,
    Closed,
}

/// The frozen tuple a certificate may be issued for.
#[derive(PartialEq, Eq)]
pub struct Issue {
    pub node_id: String,
    pub csr_pem: String,
    pub csr_hash: [u8; 32],
    pub replace: bool,
}

impl fmt::Debug for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Issue")
            .field("node_id", &self.node_id)
            .field("replace", &self.replace)
            .finish_non_exhaustive()
    }
}

/// The switch, for `PairStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
    pub open: bool,
    pub closes_at_ms: u64,
    pub pending: u32,
}

/// Refusals counted per `(kind, key)` since the last report (audited as a rate, section 8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefusalRate {
    pub kind: Kind,
    pub address: IpAddr,
    pub count: u32,
}

// ---- the sessions ----

enum Phase {
    Awaiting,
    Pending,
    Approved { replace: bool },
    Done(Outcome),
}

impl Phase {
    fn name(&self) -> &'static str {
        match self {
            Self::Awaiting => "awaiting_reveal",
            Self::Pending => "pending",
            Self::Approved { .. } => "approved",
            Self::Done(Outcome::Approved) => "approved",
            Self::Done(Outcome::Denied) => "denied",
            Self::Done(Outcome::Expired) => "expired",
            Self::Done(Outcome::Closed) => "closed",
        }
    }
}

struct Session {
    conn: ConnId,
    kind: Kind,
    addr: IpAddr,
    key: IpAddr,
    name: String,
    os: String,
    // Frozen at the commit.
    exporter: [u8; 32],
    leaf_hash: [u8; 32],
    commitment: [u8; 32],
    nonce_h: [u8; 16],
    reveal_by: u64,
    // Frozen at the reveal.
    nonce_n: [u8; 16],
    csr_hash: [u8; 32],
    csr_pem: String,
    number: u32,
    deadline: u64,
    phase: Phase,
}

/// What the machine's `PairRequest` carries.
#[derive(Debug, Clone, Copy)]
pub struct Commit<'a> {
    pub commitment: &'a [u8],
    pub name: &'a str,
    pub os: &'a str,
    pub version: &'a str,
}

/// What the machine's `PairReveal` carries, with what the caller worked out from the CSR: `csr_hash` is the SHA-256 of its DER (`None`
/// when it did not parse) and `node_id_ok` says whether the node id passes the rules enrollment applies. Both come from the caller
/// because they need the certificate library, which this module does not use.
#[derive(Clone, Copy)]
pub struct Reveal<'a> {
    pub pair_id: &'a [u8],
    pub nonce_n: &'a [u8],
    pub csr_pem: &'a str,
    pub csr_hash: Option<[u8; 32]>,
    pub node_id_ok: bool,
}

/// Random bytes from the system; a test supplies its own. `false` means the system could not.
pub type Entropy = Box<dyn FnMut(&mut [u8]) -> bool + Send>;

fn system_entropy() -> Entropy {
    Box::new(|b| getrandom::getrandom(b).is_ok())
}

/// The whole pairing state of a hub. Memory only: it ends with the hub (section 7).
pub struct Pairing {
    leaf: Option<[u8; 32]>,
    open_until: Option<u64>,
    window_failures: u32,
    sessions: HashMap<PairId, Session>,
    by_conn: HashMap<ConnId, PairId>,
    /// `(when, what, bytes)` for commitments, exporters and nonces seen in the last [`REPLAY_MS`].
    seen: VecDeque<(u64, u8, Vec<u8>)>,
    limits: PairLimits,
    notes: VecDeque<(u64, Note)>,
    next_seq: u64,
    unannounced: u32,
    last_push: Option<u64>,
    refusals: HashMap<(Kind, IpAddr), u32>,
    last_report: u64,
    /// The node id of each approval a connection has collected and is issuing now: the name is not enrolled until that ends (F811).
    issuing: HashMap<ConnId, String>,
    rng: Entropy,
}

impl fmt::Debug for Pairing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pairing")
            .field("open", &self.open_until.is_some())
            .field("sessions", &self.sessions.len())
            .finish_non_exhaustive()
    }
}

const SEEN_COMMITMENT: u8 = 1;
const SEEN_EXPORTER: u8 = 2;
const SEEN_NONCE: u8 = 3;

/// Text a machine supplies for the owner's card: bounded, with no control or hidden character.
fn clean_text(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= TEXT_MAX
        && !s
            .chars()
            .any(|c| c.is_control() || agora_core::text::is_hidden(c))
}

impl Default for Pairing {
    fn default() -> Self {
        Self::new()
    }
}

impl Pairing {
    pub fn new() -> Self {
        Self::with_entropy(system_entropy())
    }

    pub fn with_entropy(rng: Entropy) -> Self {
        Self {
            leaf: None,
            open_until: None,
            window_failures: 0,
            sessions: HashMap::new(),
            by_conn: HashMap::new(),
            seen: VecDeque::new(),
            limits: PairLimits::new(),
            notes: VecDeque::new(),
            next_seq: 1,
            unannounced: 0,
            last_push: None,
            refusals: HashMap::new(),
            last_report: 0,
            issuing: HashMap::new(),
            rng,
        }
    }

    /// The hub offers pairing once it knows the hash of its own TLS certificate (the one the machine will have seen). Without it
    /// nothing can be paired, and the hub answers `protocol.capability_missing`.
    pub fn set_leaf(&mut self, leaf_hash: [u8; 32]) {
        self.leaf = Some(leaf_hash);
    }

    pub fn enabled(&self) -> bool {
        self.leaf.is_some()
    }

    pub fn limits(&self) -> &PairLimits {
        &self.limits
    }

    // ---- the switch ----

    /// Opens the switch for [`WINDOW_MS`] (the owner's command, audited by the caller). Resets the window's failure budget.
    pub fn open(&mut self, now: u64) -> u64 {
        self.tick(now);
        let until = now.saturating_add(WINDOW_MS);
        self.open_until = Some(until);
        self.window_failures = 0;
        until
    }

    /// Closes the switch and ends every session at once.
    pub fn close(&mut self, now: u64) {
        self.tick(now);
        self.shut(now);
    }

    fn shut(&mut self, now: u64) {
        self.open_until = None;
        let ids: Vec<PairId> = self.sessions.keys().copied().collect();
        for id in ids {
            self.end(&id, Outcome::Closed, now, false);
        }
    }

    pub fn status(&mut self, now: u64) -> Status {
        self.tick(now);
        Status {
            open: self.open_until.is_some(),
            closes_at_ms: self.open_until.unwrap_or(0),
            pending: u32::try_from(
                self.sessions
                    .values()
                    .filter(|s| matches!(s.phase, Phase::Pending))
                    .count(),
            )
            .unwrap_or(u32::MAX),
        }
    }

    // ---- time ----

    /// Moves everything that is past its time: the switch, a reveal nobody sent, a request nobody answered.
    pub fn tick(&mut self, now: u64) {
        while self
            .seen
            .front()
            .is_some_and(|(t, _, _)| now.saturating_sub(*t) >= REPLAY_MS)
        {
            self.seen.pop_front();
        }
        if self.open_until.is_some_and(|u| now >= u) {
            self.shut(now);
        }
        let late: Vec<PairId> = self
            .sessions
            .iter()
            .filter(|(_, s)| match s.phase {
                Phase::Awaiting => now >= s.reveal_by,
                Phase::Pending => now >= s.deadline,
                _ => false,
            })
            .map(|(id, _)| *id)
            .collect();
        for id in late {
            self.end(&id, Outcome::Expired, now, true);
        }
    }

    /// When the connection's session next needs attention (its reveal or approval deadline).
    pub fn deadline_of(&self, conn: ConnId) -> Option<u64> {
        let s = self.sessions.get(self.by_conn.get(&conn)?)?;
        match s.phase {
            Phase::Awaiting => Some(s.reveal_by),
            Phase::Pending => Some(s.deadline),
            _ => None,
        }
    }

    // ---- failures and endings ----

    /// A failure of an admitted session: counted for its address and for the window (5 in one window close the switch).
    fn fail(&mut self, kind: Kind, key: IpAddr, now: u64) {
        self.limits.failure(kind, key, now);
        if self.open_until.is_some() {
            self.window_failures += 1;
            if self.window_failures >= WINDOW_FAILURES {
                // Five failures in one window: the switch closes (the aggregate bound, section 5).
                self.shut(now);
            }
        }
    }

    /// Ends a session that is still undecided. `failure` counts it against its address and the window. Idempotent.
    fn end(&mut self, id: &PairId, outcome: Outcome, now: u64, failure: bool) {
        let Some(s) = self.sessions.get_mut(id) else {
            return;
        };
        let announced = match s.phase {
            Phase::Awaiting => false,
            Phase::Pending | Phase::Approved { .. } => true,
            Phase::Done(_) => return,
        };
        s.phase = Phase::Done(outcome);
        let (kind, key) = (s.kind, s.key);
        if announced {
            self.note(Note::Decided {
                pair_id: *id,
                outcome,
            });
        }
        if failure {
            self.fail(kind, key, now);
        }
    }

    fn note(&mut self, n: Note) {
        self.notes.push_back((self.next_seq, n));
        self.next_seq += 1;
        while self.notes.len() > NOTES_KEPT {
            self.notes.pop_front();
        }
    }

    fn remember(&mut self, now: u64, what: u8, bytes: &[u8]) {
        self.seen.push_back((now, what, bytes.to_vec()));
    }

    fn was_seen(&self, what: u8, bytes: &[u8]) -> bool {
        self.seen
            .iter()
            .any(|(_, w, b)| *w == what && b.as_slice() == bytes)
    }

    fn random<const N: usize>(&mut self) -> Option<[u8; N]> {
        let mut b = [0u8; N];
        (self.rng)(&mut b).then_some(b)
    }

    fn refuse_counted(&mut self, kind: Kind, key: IpAddr) {
        // At most one counter per live key, so the map is bounded by the table.
        if self.refusals.len() < MAX_KEYS || self.refusals.contains_key(&(kind, key)) {
            let c = self.refusals.entry((kind, key)).or_insert(0);
            *c = c.saturating_add(1);
        }
    }

    /// The refusals since the last report, once a minute (section 8: a rate, not a line each). Empty when it is not time.
    pub fn take_refusal_rates(&mut self, now: u64) -> Vec<RefusalRate> {
        if now.saturating_sub(self.last_report) < REFUSAL_REPORT_MS {
            return Vec::new();
        }
        self.last_report = now;
        let mut out: Vec<RefusalRate> = self
            .refusals
            .drain()
            .map(|((kind, address), count)| RefusalRate {
                kind,
                address,
                count,
            })
            .collect();
        out.sort_by_key(|r| (r.address, r.kind.index()));
        out
    }

    // ---- the machine's messages ----

    /// Step 1. The commitment arrives; the hub checks the switch and the limits **before it allocates anything**, then makes the
    /// session and its challenge. Returns `(pair_id, nonce_h)`.
    pub fn request(
        &mut self,
        now: u64,
        from: Peer,
        exporter: [u8; 32],
        commit: Commit<'_>,
    ) -> Result<(PairId, [u8; 16]), Refusal> {
        self.request_on(now, from, (exporter, None), commit)
    }

    /// [`Pairing::request`] for a connection that knows which certificate it was shown (`leaf_hash`): the session freezes that
    /// one, so a leaf that was replaced a moment ago cannot make the two numbers differ (spec section 5b).
    pub fn request_on(
        &mut self,
        now: u64,
        from: Peer,
        (exporter, seen_leaf): ([u8; 32], Option<[u8; 32]>),
        commit: Commit<'_>,
    ) -> Result<(PairId, [u8; 16]), Refusal> {
        let Commit {
            commitment,
            name,
            os,
            version,
        } = commit;
        self.tick(now);
        let Some(current) = self.leaf else {
            return Err(Refusal::Closed);
        };
        let leaf_hash = seen_leaf.unwrap_or(current);
        if self.open_until.is_none() {
            return Err(Refusal::Closed);
        }
        let kind = Kind::Machine;
        let key = address_key(from.addr);
        // A second request on a connection that has a session is not a retry: it ends that session.
        if let Some(id) = self.by_conn.get(&from.conn).copied() {
            self.end(&id, Outcome::Closed, now, true);
            return Err(Refusal::State("requested"));
        }
        if let Err(seconds) = self.limits.admit(kind, key, now) {
            self.refuse_counted(kind, key);
            return Err(Refusal::Limit { seconds });
        }
        let live = self
            .sessions
            .values()
            .filter(|s| s.kind == kind && matches!(s.phase, Phase::Awaiting | Phase::Pending))
            .count();
        if live >= MAX_PENDING_MACHINES {
            self.refuse_counted(kind, key);
            let soonest = self
                .sessions
                .values()
                .filter(|s| matches!(s.phase, Phase::Awaiting | Phase::Pending))
                .map(|s| match s.phase {
                    Phase::Awaiting => s.reveal_by,
                    _ => s.deadline,
                })
                .min()
                .unwrap_or(now);
            return Err(Refusal::Limit {
                seconds: seconds_until(soonest.saturating_sub(now)),
            });
        }
        if commitment.len() != 32 {
            return Err(Refusal::Invalid("commitment"));
        }
        // The name is the node id: it must already pass the enrolment rules before any card, audit row or limit entry holds it.
        if !crate::ca::valid_node_id(name) {
            return Err(Refusal::Invalid("node_name"));
        }
        if !os.is_empty() && !clean_text(os) {
            return Err(Refusal::Invalid("os"));
        }
        if !version.is_empty() && !clean_text(version) {
            return Err(Refusal::Invalid("version"));
        }
        if self.was_seen(SEEN_COMMITMENT, commitment) || self.was_seen(SEEN_EXPORTER, &exporter) {
            self.fail(kind, key, now);
            return Err(Refusal::Mismatch);
        }
        let (Some(id), Some(nonce_h)) = (self.random::<16>(), self.random::<16>()) else {
            return Err(Refusal::Closed);
        };
        let mut c = [0u8; 32];
        c.copy_from_slice(commitment);
        self.remember(now, SEEN_COMMITMENT, &c);
        self.remember(now, SEEN_EXPORTER, &exporter);
        self.sessions.insert(
            id,
            Session {
                conn: from.conn,
                kind,
                addr: from.addr,
                key,
                name: name.to_string(),
                os: os.to_string(),
                exporter,
                leaf_hash,
                commitment: c,
                nonce_h,
                reveal_by: now.saturating_add(REVEAL_MS),
                nonce_n: [0; 16],
                csr_hash: [0; 32],
                csr_pem: String::new(),
                number: 0,
                deadline: 0,
                phase: Phase::Awaiting,
            },
        );
        self.by_conn.insert(from.conn, id);
        Ok((id, nonce_h))
    }

    /// Step 3. The reveal. `csr_hash` is the SHA-256 of the CSR's DER, or `None` when the CSR did not parse; `node_id_ok` says
    /// whether the node id passes the rules enrollment applies. Both come from the caller because they need the certificate
    /// library, which this module does not use.
    pub fn reveal(&mut self, now: u64, from: Peer, reveal: Reveal<'_>) -> Result<(), Refusal> {
        let Reveal {
            pair_id,
            nonce_n,
            csr_pem,
            csr_hash,
            node_id_ok,
        } = reveal;
        self.tick(now);

        let owned = PairId::try_from(pair_id)
            .ok()
            .filter(|id| self.sessions.get(id).is_some_and(|s| s.conn == from.conn));
        let Some(id) = owned else {
            // Not ours, not known, not a pair id: nothing is gained, and it counts against the address that sent it.
            return Err(self.unknown(now, from));
        };
        let Some(s) = self.sessions.get(&id) else {
            return Err(Refusal::Unknown);
        };
        match s.phase {
            Phase::Awaiting => {}
            Phase::Done(Outcome::Expired) => return Err(Refusal::Expired),
            Phase::Done(Outcome::Closed) => return Err(Refusal::Closed),
            ref p => {
                // A second or replayed reveal, or one out of order: it ends the session and never edits the tuple.
                let state = p.name();
                self.end(&id, Outcome::Closed, now, true);
                return Err(Refusal::State(state));
            }
        }
        let nonce: Option<[u8; 16]> = nonce_n.try_into().ok();
        let (Some(nonce_n), Some(csr_hash)) = (nonce, csr_hash) else {
            let field = if nonce.is_none() {
                "nonce_n"
            } else {
                "csr_pem"
            };
            self.end(&id, Outcome::Closed, now, true);
            return Err(Refusal::Invalid(field));
        };
        if !node_id_ok {
            self.end(&id, Outcome::Closed, now, true);
            return Err(Refusal::Invalid("node_name"));
        }
        let expected = commitment(&nonce_n, &csr_hash);
        // Not a secret (the machine sent it), but compared without an early exit anyway.
        let matches = expected
            .iter()
            .zip(&s.commitment)
            .fold(0u8, |d, (a, b)| d | (a ^ b))
            == 0;
        if !matches || self.was_seen(SEEN_NONCE, &nonce_n) {
            self.end(&id, Outcome::Closed, now, true);
            return Err(Refusal::Mismatch);
        }
        self.remember(now, SEEN_NONCE, &nonce_n);
        let Some(s) = self.sessions.get_mut(&id) else {
            return Err(Refusal::Unknown);
        };
        s.nonce_n = nonce_n;
        s.csr_hash = csr_hash;
        s.csr_pem = csr_pem.to_string();
        s.number = sas(&s.exporter, &s.leaf_hash, &csr_hash, &nonce_n, &s.nonce_h);
        s.deadline = now.saturating_add(SESSION_MS);
        s.phase = Phase::Pending;
        let card = Card {
            pair_id: id,
            kind: s.kind,
            name: s.name.clone(),
            os: s.os.clone(),
            address: s.addr.to_string(),
            number: digits(s.number),
            expires_ms: s.deadline,
        };
        self.unannounced = self.unannounced.saturating_add(1);
        self.note(Note::Requested(card));
        Ok(())
    }

    /// A message naming a `pair_id` its connection does not own (or none at all): `pair.unknown`, no authority gained, and a failure
    /// counted for the address it came from (and not for the window: see the body). While the switch is closed it is `pair.closed`,
    /// counted nowhere.
    pub fn unknown(&mut self, now: u64, from: Peer) -> Refusal {
        self.tick(now);
        // A closed switch refuses every pairing message and allocates nothing (section 2, F801).
        if self.leaf.is_none() || self.open_until.is_none() {
            return Refusal::Closed;
        }
        // Counted against the address only. This message comes from a connection that was never admitted (no limit has been
        // applied to it), so letting it count toward the window's budget would let any host close the owner's window at will.
        self.limits
            .failure(Kind::Machine, address_key(from.addr), now);
        Refusal::Unknown
    }

    /// What the connection that owns a session should do now (section 5a: a result is sent once, on the owning connection). An
    /// approval is handed over once, with the frozen tuple, and the session is gone afterwards.
    pub fn verdict(&mut self, conn: ConnId, now: u64) -> Verdict {
        self.tick(now);
        let Some(id) = self.by_conn.get(&conn).copied() else {
            return Verdict::Closed;
        };
        let Some(s) = self.sessions.get(&id) else {
            self.by_conn.remove(&conn);
            return Verdict::Closed;
        };
        let v = match s.phase {
            Phase::Awaiting | Phase::Pending => return Verdict::Waiting,
            Phase::Approved { replace } => Verdict::Approved(Issue {
                node_id: s.name.clone(),
                csr_pem: s.csr_pem.clone(),
                csr_hash: s.csr_hash,
                replace,
            }),
            Phase::Done(Outcome::Approved) | Phase::Done(Outcome::Closed) => Verdict::Closed,
            Phase::Done(Outcome::Denied) => Verdict::Denied,
            Phase::Done(Outcome::Expired) => Verdict::Expired,
        };
        if let Verdict::Approved(issue) = &v {
            self.issuing.insert(conn, issue.node_id.clone());
        }
        self.sessions.remove(&id);
        self.by_conn.remove(&conn);
        v
    }

    /// The approval this connection collected was issued or refused: its node id is enrolled or free again, so it no longer holds
    /// it. Called before the result is written, so a machine that stops reading cannot keep the name from every later approval
    /// (Codex review of F811); [`Self::connection_closed`] lets it go too.
    pub fn issued(&mut self, conn: ConnId) {
        self.issuing.remove(&conn);
    }

    /// The connection ended. An undecided session is over (an abort after the challenge is a failure); an approval nobody
    /// collected is void and no certificate is issued.
    pub fn connection_closed(&mut self, conn: ConnId, now: u64) {
        self.issuing.remove(&conn);
        let Some(id) = self.by_conn.remove(&conn) else {
            return;
        };
        let void = self
            .sessions
            .get(&id)
            .is_some_and(|s| matches!(s.phase, Phase::Approved { .. }));
        if void {
            self.note(Note::Decided {
                pair_id: id,
                outcome: Outcome::Closed,
            });
        } else {
            self.end(&id, Outcome::Closed, now, true);
        }
        self.sessions.remove(&id);
    }

    /// A message the connection must not send in this state (anything but the reveal while waiting): it ends the session.
    pub fn violation(&mut self, conn: ConnId, now: u64) -> Refusal {
        self.tick(now);
        let state = match self
            .by_conn
            .get(&conn)
            .copied()
            .and_then(|id| self.sessions.get(&id).map(|s| (id, s.phase.name())))
        {
            Some((id, state)) => {
                self.end(&id, Outcome::Closed, now, true);
                state
            }
            None => "none",
        };
        Refusal::State(state)
    }

    // ---- the owner's decisions ----

    /// The node id a pending request asked for, so the caller can check it against the enrolled machines before approving.
    pub fn pending_node_id(&self, pair_id: &[u8]) -> Option<&str> {
        let s = self.sessions.get(&PairId::try_from(pair_id).ok()?)?;
        matches!(s.phase, Phase::Pending).then_some(s.name.as_str())
    }

    /// Approve: terminal and one-shot. Only a request that is still pending and inside its time, and not while another approval for
    /// the same node id is waiting or being issued: only one of them could get the certificate (`name_in_use`, it stays pending, F811).
    /// An approval closes the switch and ends the other requests (one machine per opening, #942); a refused one changes nothing.
    pub fn approve(&mut self, now: u64, pair_id: &[u8], replace: bool) -> Result<(), Refusal> {
        self.tick(now);
        let id = self.decidable(pair_id)?;
        let name = self.sessions.get(&id).map(|s| s.name.as_str());
        let held =
            self.sessions.values().any(|s| {
                matches!(s.phase, Phase::Approved { .. }) && Some(s.name.as_str()) == name
            }) || self.issuing.values().any(|n| Some(n.as_str()) == name);
        if held {
            return Err(Refusal::State("name_in_use"));
        }
        if let Some(s) = self.sessions.get_mut(&id) {
            s.phase = Phase::Approved { replace };
        }
        self.note(Note::Decided {
            pair_id: id,
            outcome: Outcome::Approved,
        });
        self.close_after_approval(now);
        Ok(())
    }

    /// One machine per opening, whoever approved (#942): an approval closes the switch and ends every request still undecided, as
    /// closing does, but not as a failure for anyone. Unlike [`Self::close`] it leaves every approval alone (this one and any from
    /// an earlier opening that its connection has not collected yet), so each is still handed to its own connection, once.
    fn close_after_approval(&mut self, now: u64) {
        self.open_until = None;
        let undecided: Vec<PairId> = self
            .sessions
            .iter()
            .filter(|(_, s)| matches!(s.phase, Phase::Awaiting | Phase::Pending))
            .map(|(id, _)| *id)
            .collect();
        for id in undecided {
            self.end(&id, Outcome::Closed, now, false);
        }
    }

    /// Deny: terminal. A deny counts as a failure for the address (section 7).
    pub fn deny(&mut self, now: u64, pair_id: &[u8]) -> Result<(), Refusal> {
        self.tick(now);
        let id = self.decidable(pair_id)?;
        self.end(&id, Outcome::Denied, now, true);
        Ok(())
    }

    fn decidable(&self, pair_id: &[u8]) -> Result<PairId, Refusal> {
        let id = PairId::try_from(pair_id).map_err(|_| Refusal::Unknown)?;
        let s = self.sessions.get(&id).ok_or(Refusal::Unknown)?;
        match s.phase {
            Phase::Pending => Ok(id),
            Phase::Done(Outcome::Expired) => Err(Refusal::Expired),
            ref p => Err(Refusal::State(p.name())),
        }
    }

    // ---- what windows and notifiers read ----

    /// The cards waiting now, and the cursor to read later notes from. For a window that has just connected.
    pub fn cards(&mut self, now: u64) -> (Vec<Card>, u64) {
        self.tick(now);
        let mut cards: Vec<Card> = self
            .sessions
            .iter()
            .filter(|(_, s)| matches!(s.phase, Phase::Pending))
            .map(|(id, s)| Card {
                pair_id: *id,
                kind: s.kind,
                name: s.name.clone(),
                os: s.os.clone(),
                address: s.addr.to_string(),
                number: digits(s.number),
                expires_ms: s.deadline,
            })
            .collect();
        cards.sort_by_key(|c| (c.expires_ms, c.pair_id));
        (cards, self.next_seq)
    }

    /// Notes since `cursor`, and the next cursor.
    pub fn notes_after(&mut self, now: u64, cursor: u64) -> (Vec<Note>, u64) {
        self.tick(now);
        let notes = self
            .notes
            .iter()
            .filter(|(seq, _)| *seq >= cursor)
            .map(|(_, n)| n.clone())
            .collect();
        (notes, self.next_seq)
    }

    /// A push notification (ADR-0051) for the requests that arrived since the last one, at most once a minute for the whole hub:
    /// requests that arrive during the cooldown are summed into the next one. Returns how many are waiting to be announced.
    pub fn take_push(&mut self, now: u64) -> Option<u32> {
        if self.unannounced == 0 {
            return None;
        }
        if self
            .last_push
            .is_some_and(|t| now.saturating_sub(t) < PUSH_EVERY_MS)
        {
            return None;
        }
        self.last_push = Some(now);
        Some(std::mem::take(&mut self.unannounced))
    }

    #[cfg(test)]
    fn number_of(&self, pair_id: &PairId) -> Option<String> {
        self.sessions.get(pair_id).map(|s| digits(s.number))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_arguments)]
mod tests {
    use super::*;

    fn counter_entropy() -> Entropy {
        let mut n: u8 = 0;
        Box::new(move |b| {
            for x in b.iter_mut() {
                n = n.wrapping_add(1);
                *x = n;
            }
            true
        })
    }

    fn ip(last: u8) -> IpAddr {
        IpAddr::from([192, 168, 1, last])
    }

    fn peer(conn: u64, last: u8) -> Peer {
        Peer {
            conn: ConnId(conn),
            addr: ip(last),
        }
    }

    const LEAF: [u8; 32] = [7; 32];

    fn open_hub() -> Pairing {
        let mut p = Pairing::with_entropy(counter_entropy());
        p.set_leaf(LEAF);
        p.open(1_000);
        p
    }

    /// What a machine holds: its nonce, its CSR hash and what it sends.
    struct Machine {
        nonce_n: [u8; 16],
        csr_hash: [u8; 32],
        exporter: [u8; 32],
    }

    impl Machine {
        fn new(seed: u8) -> Self {
            Self {
                nonce_n: [seed; 16],
                csr_hash: [seed.wrapping_add(100); 32],
                exporter: [seed.wrapping_add(50); 32],
            }
        }

        fn commit(&self) -> [u8; 32] {
            commitment(&self.nonce_n, &self.csr_hash)
        }
    }

    fn start(
        p: &mut Pairing,
        now: u64,
        from: Peer,
        m: &Machine,
    ) -> Result<(PairId, [u8; 16]), Refusal> {
        p.request(
            now,
            from,
            m.exporter,
            Commit {
                commitment: &m.commit(),
                name: "laptop",
                os: "linux",
                version: "0.3",
            },
        )
    }

    fn reveal(
        p: &mut Pairing,
        now: u64,
        from: Peer,
        id: &PairId,
        m: &Machine,
    ) -> Result<(), Refusal> {
        p.reveal(
            now,
            from,
            Reveal {
                pair_id: id,
                nonce_n: &m.nonce_n,
                csr_pem: "CSR",
                csr_hash: Some(m.csr_hash),
                node_id_ok: true,
            },
        )
    }

    fn pending(p: &mut Pairing, now: u64, conn: u64, last: u8, m: &Machine) -> PairId {
        let from = peer(conn, last);
        let (id, _) = start(p, now, from, m).unwrap();
        reveal(p, now + 10, from, &id, m).unwrap();
        id
    }

    // ---- the arithmetic ----

    /// A known answer, computed outside this code (Python: hkdf-extract/expand with hmac-sha256), so the construction cannot drift
    /// from the spec unnoticed, and the node side has a vector to check against.
    #[test]
    fn the_number_matches_a_vector_computed_independently() {
        let n = sas(&[1; 32], &[2; 32], &[3; 32], &[4; 16], &[5; 16]);
        assert_eq!(digits(n), SAS_VECTOR);
    }

    const SAS_VECTOR: &str = "497259";

    #[test]
    fn every_input_changes_the_number() {
        let base = sas(&[1; 32], &[2; 32], &[3; 32], &[4; 16], &[5; 16]);
        for (i, n) in [
            sas(&[9; 32], &[2; 32], &[3; 32], &[4; 16], &[5; 16]),
            sas(&[1; 32], &[9; 32], &[3; 32], &[4; 16], &[5; 16]),
            sas(&[1; 32], &[2; 32], &[9; 32], &[4; 16], &[5; 16]),
            sas(&[1; 32], &[2; 32], &[3; 32], &[9; 16], &[5; 16]),
            sas(&[1; 32], &[2; 32], &[3; 32], &[4; 16], &[9; 16]),
        ]
        .into_iter()
        .enumerate()
        {
            assert_ne!(n, base, "input {i} did not matter");
        }
        assert!(base < 1_000_000);
    }

    #[test]
    fn the_commitment_binds_the_nonce_and_the_csr() {
        let c = commitment(&[1; 16], &[2; 32]);
        assert_ne!(c, commitment(&[9; 16], &[2; 32]));
        assert_ne!(c, commitment(&[1; 16], &[9; 32]));
        assert_eq!(c, commitment(&[1; 16], &[2; 32]));
    }

    #[test]
    fn the_numbers_use_the_exporter_two_sessions_with_one_leaf_and_nonces_differ() {
        let mut p = open_hub();
        let m1 = Machine::new(1);
        let mut m2 = Machine::new(1);
        m2.exporter = [200; 32];
        let a = pending(&mut p, 2_000, 1, 10, &m1);
        // Same nonce_n and csr, so the commitment repeats: the replay rule refuses it, which is the point of that rule. Another
        // hub state with fresh entropy shows the exporter on its own:
        let mut q = open_hub();
        let b = pending(&mut q, 2_000, 1, 10, &m2);
        assert_ne!(p.number_of(&a), q.number_of(&b));
    }

    /// Spec section 5b: a machine's connection was shown leaf A; the hub re-issued to leaf B (`set_leaf`) before the request landed. The
    /// session must freeze A, the certificate the machine's own connection saw, or the two numbers would differ.
    #[test]
    fn a_session_keeps_the_leaf_its_own_connection_was_shown_when_the_leaf_is_replaced_before_the_request(
    ) {
        const A: [u8; 32] = [0xA1; 32];
        const B: [u8; 32] = [0xB2; 32];
        let mut p = Pairing::with_entropy(counter_entropy());
        p.set_leaf(A);
        p.open(1_000);
        // The connection is accepted with leaf A; then the hub switches to leaf B.
        p.set_leaf(B);
        let m = Machine::new(1);
        let from = peer(1, 10);
        let (id, nonce_h) = p
            .request_on(
                2_000,
                from,
                (m.exporter, Some(A)),
                Commit {
                    commitment: &m.commit(),
                    name: "laptop",
                    os: "linux",
                    version: "0.3",
                },
            )
            .unwrap();
        reveal(&mut p, 2_010, from, &id, &m).unwrap();
        let with_a = digits(sas(&m.exporter, &A, &m.csr_hash, &m.nonce_n, &nonce_h));
        let with_b = digits(sas(&m.exporter, &B, &m.csr_hash, &m.nonce_n, &nonce_h));
        assert_ne!(with_a, with_b);
        assert_eq!(p.number_of(&id), Some(with_a));
        // A connection that did not say which leaf it saw gets the hub's current one.
        let m2 = Machine::new(2);
        let from2 = peer(2, 11);
        let (id2, nonce_h2) = start(&mut p, 2_100, from2, &m2).unwrap();
        reveal(&mut p, 2_110, from2, &id2, &m2).unwrap();
        assert_eq!(
            p.number_of(&id2),
            Some(digits(sas(
                &m2.exporter,
                &B,
                &m2.csr_hash,
                &m2.nonce_n,
                &nonce_h2
            )))
        );
    }

    #[test]
    fn an_address_key_is_the_ipv4_address_or_the_ipv6_slash_64() {
        let a: IpAddr = "2001:db8::1:2:3:4".parse().unwrap();
        let b: IpAddr = "2001:db8::ffff:2:3:4".parse().unwrap();
        let c: IpAddr = "2001:db9::1".parse().unwrap();
        assert_eq!(address_key(a), address_key(b));
        assert_ne!(address_key(a), address_key(c));
        assert_eq!(address_key(ip(5)), ip(5));
        // An IPv4 address that arrives mapped into IPv6 counts as that IPv4 address.
        assert_eq!(address_key("::ffff:192.168.1.5".parse().unwrap()), ip(5));
    }

    // ---- the handshake ----

    #[test]
    fn the_full_machine_flow_ends_with_one_issuance_for_the_frozen_tuple() {
        let mut p = open_hub();
        let m = Machine::new(1);
        let id = pending(&mut p, 2_000, 1, 10, &m);
        // The number is what both sides compute from what each saw.
        let (_, nonce_h) = (id, p.sessions[&id].nonce_h);
        let expected = digits(sas(&m.exporter, &LEAF, &m.csr_hash, &m.nonce_n, &nonce_h));
        let (cards, _) = p.cards(2_100);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].number, expected);
        assert_eq!(cards[0].name, "laptop");
        assert_eq!(cards[0].address, "192.168.1.10");
        assert_eq!(p.verdict(ConnId(1), 2_100), Verdict::Waiting);
        p.approve(2_200, &id, false).unwrap();
        let Verdict::Approved(issue) = p.verdict(ConnId(1), 2_300) else {
            panic!("approved")
        };
        assert_eq!(issue.node_id, "laptop");
        assert_eq!(issue.csr_hash, m.csr_hash);
        assert!(!issue.replace);
        // Once: the session is gone, so there is nothing to issue a second time.
        assert_eq!(p.verdict(ConnId(1), 2_400), Verdict::Closed);
        assert_eq!(p.approve(2_500, &id, false), Err(Refusal::Unknown));
    }

    #[test]
    fn a_request_before_the_switch_is_open_is_refused_and_counted_nowhere() {
        let mut p = Pairing::with_entropy(counter_entropy());
        p.set_leaf(LEAF);
        let m = Machine::new(1);
        assert_eq!(
            start(&mut p, 5, peer(1, 10), &m).unwrap_err(),
            Refusal::Closed
        );
        assert_eq!(p.limits().keys(), 0);
        // Without a leaf the hub cannot pair at all.
        let mut q = Pairing::with_entropy(counter_entropy());
        q.open(0);
        assert!(!q.enabled());
        assert_eq!(
            start(&mut q, 5, peer(1, 10), &m).unwrap_err(),
            Refusal::Closed
        );
    }

    #[test]
    fn the_switch_closes_by_itself_and_ends_every_session() {
        let mut p = open_hub();
        let m = Machine::new(1);
        let id = pending(&mut p, 2_000, 1, 10, &m);
        assert!(p.status(2_000).open);
        assert_eq!(p.status(2_000).pending, 1);
        let after = 1_000 + WINDOW_MS;
        assert!(!p.status(after).open);
        assert_eq!(p.status(after).closes_at_ms, 0);
        assert_eq!(p.verdict(ConnId(1), after), Verdict::Closed);
        assert_eq!(p.approve(after, &id, false), Err(Refusal::Unknown));
        let (notes, _) = p.notes_after(after, 0);
        assert!(matches!(
            notes.last(),
            Some(Note::Decided {
                outcome: Outcome::Closed,
                ..
            })
        ));
    }

    #[test]
    fn closing_ends_every_session_at_once() {
        let mut p = open_hub();
        let a = pending(&mut p, 2_000, 1, 10, &Machine::new(1));
        let b = pending(&mut p, 2_000, 2, 11, &Machine::new(2));
        p.close(2_100);
        assert!(!p.status(2_100).open);
        for (conn, id) in [(1, a), (2, b)] {
            assert_eq!(p.verdict(ConnId(conn), 2_100), Verdict::Closed);
            assert_eq!(p.approve(2_100, &id, false), Err(Refusal::Unknown));
        }
        // A closing is not a failure for anyone.
        assert!(p.limits().locked(Kind::Machine, ip(10), 2_100).is_none());
    }

    #[test]
    fn a_reveal_before_the_commit_has_no_session_to_land_in() {
        let mut p = open_hub();
        let m = Machine::new(1);
        assert_eq!(
            reveal(&mut p, 2_000, peer(1, 10), &[0; 16], &m).unwrap_err(),
            Refusal::Unknown
        );
        // ...and a pair id of the wrong length is the same.
        assert_eq!(
            p.reveal(
                2_000,
                peer(1, 10),
                Reveal {
                    pair_id: &[0; 3],
                    nonce_n: &m.nonce_n,
                    csr_pem: "c",
                    csr_hash: Some(m.csr_hash),
                    node_id_ok: true
                }
            )
            .unwrap_err(),
            Refusal::Unknown
        );
    }

    #[test]
    fn a_second_connection_naming_a_pair_id_gets_unknown_and_changes_nothing() {
        let mut p = open_hub();
        let m = Machine::new(1);
        let (id, _) = start(&mut p, 2_000, peer(1, 10), &m).unwrap();
        let thief = Machine::new(9);
        assert_eq!(
            reveal(&mut p, 2_010, peer(2, 99), &id, &thief).unwrap_err(),
            Refusal::Unknown
        );
        // The thief's address was charged a failure; the owner's session is untouched and still completes.
        assert_eq!(p.sessions[&id].conn, ConnId(1));
        reveal(&mut p, 2_020, peer(1, 10), &id, &m).unwrap();
        assert_eq!(p.status(2_030).pending, 1);
        // The thief cannot approve-by-connection either: a verdict is for the owning connection only.
        assert_eq!(p.verdict(ConnId(2), 2_040), Verdict::Closed);
        assert_eq!(p.verdict(ConnId(1), 2_050), Verdict::Waiting);
    }

    #[test]
    fn a_duplicate_or_replayed_reveal_ends_the_session_and_does_not_edit_it() {
        let mut p = open_hub();
        let m = Machine::new(1);
        let from = peer(1, 10);
        let (id, _) = start(&mut p, 2_000, from, &m).unwrap();
        reveal(&mut p, 2_010, from, &id, &m).unwrap();
        let number = p.number_of(&id).unwrap();
        // The same reveal again, then another one with a different nonce: both are `pair.state`.
        assert_eq!(
            reveal(&mut p, 2_020, from, &id, &m).unwrap_err(),
            Refusal::State("pending")
        );
        assert_eq!(p.verdict(ConnId(1), 2_030), Verdict::Closed);
        assert_eq!(p.number_of(&id), None);
        let _ = number;
        // It counted as a failure, and nothing can be approved any more.
        assert_eq!(p.approve(2_040, &id, false), Err(Refusal::Unknown));
    }

    #[test]
    fn a_reveal_with_another_nonce_or_csr_is_a_mismatch_and_ends_the_session() {
        let m = Machine::new(1);
        for tamper in 0..2 {
            let mut p = open_hub();
            let from = peer(1, 10);
            let (id, _) = start(&mut p, 2_000, from, &m).unwrap();
            let r = if tamper == 0 {
                p.reveal(
                    2_010,
                    from,
                    Reveal {
                        pair_id: &id,
                        nonce_n: &[77; 16],
                        csr_pem: "c",
                        csr_hash: Some(m.csr_hash),
                        node_id_ok: true,
                    },
                )
            } else {
                p.reveal(
                    2_010,
                    from,
                    Reveal {
                        pair_id: &id,
                        nonce_n: &m.nonce_n,
                        csr_pem: "c",
                        csr_hash: Some([1; 32]),
                        node_id_ok: true,
                    },
                )
            };
            assert_eq!(r.unwrap_err(), Refusal::Mismatch);
            assert_eq!(p.verdict(ConnId(1), 2_020), Verdict::Closed);
        }
    }

    #[test]
    fn a_bad_nonce_length_an_unparsable_csr_and_a_bad_node_id_end_the_session() {
        let m = Machine::new(1);
        type Case<'a> = (&'a [u8], Option<[u8; 32]>, bool, &'a str);
        let cases: [Case<'_>; 3] = [
            (&[1; 15], Some(m.csr_hash), true, "nonce_n"),
            (&[1; 16], None, true, "csr_pem"),
            (&[1; 16], Some(m.csr_hash), false, "node_name"),
        ];
        for (nonce, csr, ok, field) in cases {
            let mut p = open_hub();
            let from = peer(1, 10);
            let (id, _) = start(&mut p, 2_000, from, &m).unwrap();
            assert_eq!(
                p.reveal(
                    2_010,
                    from,
                    Reveal {
                        pair_id: &id,
                        nonce_n: nonce,
                        csr_pem: "c",
                        csr_hash: csr,
                        node_id_ok: ok
                    }
                )
                .unwrap_err(),
                Refusal::Invalid(field)
            );
            assert_eq!(p.verdict(ConnId(1), 2_020), Verdict::Closed);
        }
    }

    #[test]
    fn a_second_request_on_a_connection_ends_the_first_session() {
        let mut p = open_hub();
        let from = peer(1, 10);
        let (id, _) = start(&mut p, 2_000, from, &Machine::new(1)).unwrap();
        assert_eq!(
            start(&mut p, 2_010, from, &Machine::new(2)).unwrap_err(),
            Refusal::State("requested")
        );
        assert_eq!(
            reveal(&mut p, 2_020, from, &id, &Machine::new(1)).unwrap_err(),
            Refusal::Closed
        );
    }

    #[test]
    fn a_replayed_commitment_or_exporter_is_refused_for_ten_minutes_then_forgotten() {
        let mut p = open_hub();
        let m = Machine::new(1);
        let (_, _) = start(&mut p, 2_000, peer(1, 10), &m).unwrap();
        // Same commitment, new connection and address.
        assert_eq!(
            start(&mut p, 2_100, peer(2, 11), &m).unwrap_err(),
            Refusal::Mismatch
        );
        // Same exporter, another commitment.
        let mut other = Machine::new(2);
        other.exporter = m.exporter;
        assert_eq!(
            start(&mut p, 2_200, peer(3, 12), &other).unwrap_err(),
            Refusal::Mismatch
        );
        // After the memory runs out it is a new request (the switch is reopened because it also ran out).
        let later = 2_000 + REPLAY_MS + 10;
        p.open(later);
        assert!(start(&mut p, later, peer(4, 13), &m).is_ok());
    }

    #[test]
    fn a_nonce_is_never_accepted_twice() {
        let mut p = open_hub();
        // Two machines with the same nonce_n but different CSRs: the second reveal repeats a nonce.
        let a = Machine::new(1);
        let mut b = Machine::new(2);
        b.nonce_n = a.nonce_n;
        pending(&mut p, 2_000, 1, 10, &a);
        let from = peer(2, 11);
        let (id, _) = start(&mut p, 2_100, from, &b).unwrap();
        assert_eq!(
            reveal(&mut p, 2_110, from, &id, &b).unwrap_err(),
            Refusal::Mismatch
        );
    }

    #[test]
    fn a_reveal_that_comes_late_is_expired() {
        let mut p = open_hub();
        let m = Machine::new(1);
        let from = peer(1, 10);
        let (id, _) = start(&mut p, 2_000, from, &m).unwrap();
        assert_eq!(
            reveal(&mut p, 2_000 + REVEAL_MS, from, &id, &m).unwrap_err(),
            Refusal::Expired
        );
        assert_eq!(p.verdict(ConnId(1), 2_000 + REVEAL_MS), Verdict::Expired);
    }

    #[test]
    fn a_request_nobody_answers_expires_and_nothing_is_issued() {
        let mut p = open_hub();
        let id = pending(&mut p, 2_000, 1, 10, &Machine::new(1));
        let end = 2_010 + SESSION_MS;
        assert_eq!(p.deadline_of(ConnId(1)), Some(end));
        assert_eq!(p.approve(end - 1, &[0; 16], false), Err(Refusal::Unknown));
        assert_eq!(p.verdict(ConnId(1), end - 1), Verdict::Waiting);
        assert_eq!(p.approve(end, &id, false), Err(Refusal::Expired));
        assert_eq!(p.verdict(ConnId(1), end), Verdict::Expired);
        let (notes, _) = p.notes_after(end, 0);
        assert!(notes.iter().any(|n| matches!(
            n,
            Note::Decided {
                outcome: Outcome::Expired,
                ..
            }
        )));
    }

    #[test]
    fn approve_twice_issues_one_certificate_and_approve_after_deny_none() {
        let mut p = open_hub();
        let id = pending(&mut p, 2_000, 1, 10, &Machine::new(1));
        p.approve(2_100, &id, false).unwrap();
        assert_eq!(p.approve(2_101, &id, true), Err(Refusal::State("approved")));
        assert!(matches!(p.verdict(ConnId(1), 2_102), Verdict::Approved(_)));
        assert_eq!(p.verdict(ConnId(1), 2_103), Verdict::Closed);

        // The approval closed the switch (#942): the next request comes after the owner opened it again.
        p.open(2_900);
        let id = pending(&mut p, 3_000, 2, 11, &Machine::new(2));
        p.deny(3_100, &id).unwrap();
        assert_eq!(p.approve(3_101, &id, false), Err(Refusal::State("denied")));
        assert_eq!(p.deny(3_102, &id), Err(Refusal::State("denied")));
        assert_eq!(p.verdict(ConnId(2), 3_103), Verdict::Denied);
        assert_eq!(p.verdict(ConnId(2), 3_104), Verdict::Closed);
    }

    /// #942 (RM-07): one machine per opening is the hub's rule, whoever approves (a window, `agora pair open`). The approval closes the
    /// switch and ends every other request still undecided (not a failure for anyone), and the approved one is still collected by
    /// its own connection, once.
    #[test]
    fn an_approval_closes_the_switch_ends_the_other_requests_and_keeps_its_own() {
        let mut p = open_hub();
        let a = pending(&mut p, 2_000, 1, 10, &Machine::new(1));
        let b = pending(&mut p, 2_000, 2, 11, &Machine::new(2));
        let (c, _) = start(&mut p, 2_050, peer(3, 12), &Machine::new(3)).unwrap();
        let (_, cursor) = p.notes_after(2_060, 0);
        p.approve(2_100, &a, false).unwrap();
        let s = p.status(2_100);
        assert!(!s.open, "the switch stayed open after an approval");
        assert_eq!((s.closes_at_ms, s.pending), (0, 0));
        // The others are over: closed, nothing to approve, and their connections are told the switch closed.
        assert_eq!(p.approve(2_110, &b, false), Err(Refusal::State("closed")));
        assert_eq!(p.verdict(ConnId(2), 2_120), Verdict::Closed);
        assert_eq!(p.verdict(ConnId(3), 2_120), Verdict::Closed);
        assert_eq!(p.approve(2_130, &c, false), Err(Refusal::Unknown));
        // The windows are told: the approval, then the other card ended (the one that never revealed had no card).
        let (notes, _) = p.notes_after(2_140, cursor);
        assert_eq!(
            notes,
            vec![
                Note::Decided {
                    pair_id: a,
                    outcome: Outcome::Approved
                },
                Note::Decided {
                    pair_id: b,
                    outcome: Outcome::Closed
                },
            ]
        );
        // A new machine is refused until the owner opens again.
        assert_eq!(
            start(&mut p, 2_150, peer(4, 13), &Machine::new(4)).unwrap_err(),
            Refusal::Closed
        );
        // The approved request is still handed to its own connection, once.
        let Verdict::Approved(issue) = p.verdict(ConnId(1), 2_200) else {
            panic!("the approval was lost when the switch closed")
        };
        assert_eq!(issue.node_id, "laptop");
        assert_eq!(p.verdict(ConnId(1), 2_300), Verdict::Closed);
        // Closing after an approval is no failure: nobody is locked out, and the owner can open again.
        for last in [10, 11, 12] {
            assert!(p.limits().locked(Kind::Machine, ip(last), 2_400).is_none());
        }
        p.open(2_500);
        assert!(p.status(2_500).open);
    }

    /// A refused approval (`name_in_use` while another approval for that name is in flight) is no approval: the switch stays open and
    /// the request stays pending.
    #[test]
    fn a_refused_approval_leaves_the_switch_open() {
        let mut p = open_hub();
        let a = pending(&mut p, 2_000, 1, 10, &Machine::new(1));
        p.approve(2_100, &a, false).unwrap();
        p.open(2_200);
        let b = pending(&mut p, 2_300, 2, 11, &Machine::new(2));
        assert_eq!(
            p.approve(2_400, &b, false),
            Err(Refusal::State("name_in_use"))
        );
        assert!(p.status(2_400).open);
        assert_eq!(p.pending_node_id(&b), Some("laptop"));
    }

    #[test]
    fn a_request_that_has_not_revealed_cannot_be_approved() {
        let mut p = open_hub();
        let (id, _) = start(&mut p, 2_000, peer(1, 10), &Machine::new(1)).unwrap();
        assert_eq!(
            p.approve(2_001, &id, false),
            Err(Refusal::State("awaiting_reveal"))
        );
        assert_eq!(p.deny(2_001, &id), Err(Refusal::State("awaiting_reveal")));
        assert!(p.pending_node_id(&id).is_none());
    }

    #[test]
    fn replace_is_carried_to_the_issue() {
        let mut p = open_hub();
        let id = pending(&mut p, 2_000, 1, 10, &Machine::new(1));
        assert_eq!(p.pending_node_id(&id), Some("laptop"));
        p.approve(2_100, &id, true).unwrap();
        let Verdict::Approved(i) = p.verdict(ConnId(1), 2_200) else {
            panic!()
        };
        assert!(i.replace);
    }

    #[test]
    fn a_reconnecting_machine_cannot_reclaim_an_approval() {
        let mut p = open_hub();
        let m = Machine::new(1);
        let id = pending(&mut p, 2_000, 1, 10, &m);
        p.approve(2_100, &id, false).unwrap();
        // The owning connection dies before it collected the approval: void.
        p.connection_closed(ConnId(1), 2_150);
        assert_eq!(p.verdict(ConnId(1), 2_160), Verdict::Closed);
        // The approval closed the switch (#942); the owner opens it again for the retry.
        p.open(2_165);
        // A new connection naming the same pair id has nothing; it must start again with fresh nonces.
        assert_eq!(
            reveal(&mut p, 2_170, peer(2, 10), &id, &m).unwrap_err(),
            Refusal::Unknown
        );
        assert_eq!(
            start(&mut p, 2_180, peer(2, 10), &m).unwrap_err(),
            Refusal::Mismatch,
            "the old commitment is not accepted again"
        );
    }

    #[test]
    fn a_connection_that_ends_while_pending_closes_the_card_and_counts_as_an_abort() {
        let mut p = open_hub();
        let id = pending(&mut p, 2_000, 1, 10, &Machine::new(1));
        p.connection_closed(ConnId(1), 2_100);
        assert_eq!(p.approve(2_200, &id, false), Err(Refusal::Unknown));
        assert_eq!(p.status(2_200).pending, 0);
        let (notes, _) = p.notes_after(2_200, 0);
        assert!(matches!(
            notes.last(),
            Some(Note::Decided {
                outcome: Outcome::Closed,
                ..
            })
        ));
        // Closing a connection twice is harmless.
        p.connection_closed(ConnId(1), 2_300);
    }

    #[test]
    fn any_other_message_while_waiting_is_pair_state_and_ends_the_session() {
        let mut p = open_hub();
        pending(&mut p, 2_000, 1, 10, &Machine::new(1));
        assert_eq!(p.violation(ConnId(1), 2_100), Refusal::State("pending"));
        assert_eq!(p.verdict(ConnId(1), 2_101), Verdict::Closed);
        assert_eq!(p.violation(ConnId(7), 2_102), Refusal::State("none"));
    }

    // ---- limits ----

    #[test]
    fn the_fourth_request_in_a_minute_from_one_address_is_refused_with_a_wait() {
        let mut p = open_hub();
        for i in 0..3 {
            let from = peer(10 + i, 10);
            let m = Machine::new(i as u8 + 1);
            let (id, _) = start(&mut p, 2_000 + i * 100, from, &m).unwrap();
            // Finish each so the pending cap does not interfere.
            p.connection_closed(from.conn, 2_050 + i * 100);
            let _ = id;
        }
        let r = start(&mut p, 2_400, peer(20, 10), &Machine::new(9)).unwrap_err();
        let Refusal::Limit { seconds } = r else {
            panic!("{r:?}")
        };
        assert!((1..=60).contains(&seconds), "{seconds}");
        // Another address is unaffected.
        assert!(start(&mut p, 2_410, peer(21, 11), &Machine::new(8)).is_ok());
    }

    #[test]
    fn twenty_in_ten_minutes_is_the_most_from_one_address() {
        let mut limits = PairLimits::new();
        let mut now = 0;
        let mut admitted = 0;
        // One every 25 s keeps the minute rate under 3 but not the 10-minute rate.
        for _ in 0..40 {
            if limits.admit(Kind::Machine, ip(1), now).is_ok() {
                admitted += 1;
            }
            now += 25_000;
        }
        // 40 attempts over ~16 minutes: no window ever allowed more than 20 (refused ones count as attempts).
        assert!(admitted <= 20, "{admitted}");
        assert!(admitted >= 10, "{admitted}");
    }

    #[test]
    fn the_hub_wide_rate_bounds_many_addresses() {
        let mut limits = PairLimits::new();
        let ok = (0..200u32)
            .filter(|i| {
                let addr = IpAddr::from([10, 0, (i / 250) as u8, (i % 250) as u8]);
                limits
                    .admit(Kind::Machine, addr, 5_000 + u64::from(*i))
                    .is_ok()
            })
            .count();
        assert_eq!(ok, HUB_PER_MINUTE);
        // Browsers are counted on their own.
        assert!(limits.admit(Kind::Browser, ip(1), 5_300).is_ok());
    }

    #[test]
    fn five_failures_lock_an_address_for_a_minute_and_only_that_address() {
        let mut limits = PairLimits::new();
        assert!(limits.admit(Kind::Machine, ip(1), 0).is_ok());
        for i in 0..4 {
            assert!(!limits.failure(Kind::Machine, ip(1), i));
        }
        assert!(limits.failure(Kind::Machine, ip(1), 5));
        assert_eq!(limits.locked(Kind::Machine, ip(1), 6), Some(60));
        assert!(limits.admit(Kind::Machine, ip(1), 7).is_err());
        assert!(limits.admit(Kind::Machine, ip(2), 7).is_ok());
        assert!(limits
            .admit(Kind::Machine, ip(1), 5 + LOCKOUT_MS + 61_000)
            .is_ok());
    }

    #[test]
    fn a_full_table_refuses_new_keys_and_never_evicts_a_live_one() {
        // The rates alone keep the table far below its size (60 admissions in ten minutes for the whole hub), so it is the
        // backstop: fill it directly, with failures from many addresses at one instant.
        let mut limits = PairLimits::new();
        let now = 1_000u64;
        for i in 0..MAX_KEYS as u32 {
            let addr = IpAddr::from([10, (i >> 16) as u8, (i >> 8) as u8, i as u8]);
            limits.failure(Kind::Machine, addr, now);
        }
        assert_eq!(limits.keys(), MAX_KEYS);
        // A key with failures stays when new keys arrive.
        let first = IpAddr::from([10, 0, 0, 0]);
        for _ in 0..3 {
            limits.failure(Kind::Machine, first, now);
        }
        let new = IpAddr::from([172, 16, 0, 1]);
        assert!(limits.admit(Kind::Machine, new, now).is_err());
        assert_eq!(limits.keys(), MAX_KEYS);
        // Its counters were not reset: one more failure (the fifth) locks it.
        assert!(limits.failure(Kind::Machine, first, now));
    }

    #[test]
    fn idle_keys_are_forgotten_after_ten_minutes() {
        let mut limits = PairLimits::new();
        limits.admit(Kind::Machine, ip(1), 1_000).unwrap();
        assert_eq!(limits.keys(), 1);
        limits
            .admit(Kind::Machine, ip(2), 1_000 + RETAIN_MS + 5_000)
            .unwrap();
        assert_eq!(limits.keys(), 1);
    }

    #[test]
    fn the_pending_machine_cap_is_four_for_the_hub() {
        let mut p = open_hub();
        for i in 0..4u8 {
            start(
                &mut p,
                2_000 + u64::from(i),
                peer(u64::from(i), 10 + i),
                &Machine::new(i + 1),
            )
            .unwrap();
        }
        let r = start(&mut p, 2_100, peer(9, 50), &Machine::new(50)).unwrap_err();
        assert!(matches!(r, Refusal::Limit { .. }), "{r:?}");
        // A slot frees when one ends.
        p.connection_closed(ConnId(0), 2_200);
        assert!(start(&mut p, 2_300, peer(9, 51), &Machine::new(51)).is_ok());
    }

    #[test]
    fn five_failures_in_a_window_close_the_switch() {
        let mut p = open_hub();
        for i in 0..4u8 {
            let from = peer(u64::from(i), 10 + i);
            let m = Machine::new(i + 1);
            let (id, _) = start(&mut p, 2_000, from, &m).unwrap();
            // A reveal with a wrong nonce: a mismatch is a failure.
            let _ = p.reveal(
                2_010,
                from,
                Reveal {
                    pair_id: &id,
                    nonce_n: &[99; 16],
                    csr_pem: "c",
                    csr_hash: Some(m.csr_hash),
                    node_id_ok: true,
                },
            );
            p.connection_closed(from.conn, 2_020);
        }
        // The first four failures from four addresses left the switch open...
        assert!(p.status(2_030).open);
        let from = peer(40, 40);
        let (id, _) = start(&mut p, 2_040, from, &Machine::new(40)).unwrap();
        let _ = p.reveal(
            2_050,
            from,
            Reveal {
                pair_id: &id,
                nonce_n: &[99; 16],
                csr_pem: "c",
                csr_hash: Some([1; 32]),
                node_id_ok: true,
            },
        );
        // ...the fifth closes it.
        assert!(!p.status(2_060).open);
        assert_eq!(
            start(&mut p, 2_070, peer(50, 50), &Machine::new(50)).unwrap_err(),
            Refusal::Closed
        );
        // Opening again gives a fresh budget.
        p.open(2_080);
        assert!(p.status(2_080).open);
    }

    #[test]
    fn a_deny_counts_as_a_failure() {
        let mut p = open_hub();
        for i in 0..4u8 {
            let id = pending(&mut p, 2_000, u64::from(i), 10 + i, &Machine::new(i + 1));
            p.deny(2_100, &id).unwrap();
        }
        assert!(p.status(2_110).open);
        let id = pending(&mut p, 2_200, 9, 60, &Machine::new(60));
        p.deny(2_300, &id).unwrap();
        assert!(!p.status(2_310).open);
    }

    #[test]
    fn ten_thousand_requests_from_one_address_leave_one_pending_and_one_notice_a_minute() {
        let mut p = open_hub();
        let legit = Machine::new(1);
        let id = pending(&mut p, 2_000, 1, 10, &legit);
        let mut admitted = 0;
        let mut pushes = 0;
        if p.take_push(2_020).is_some() {
            pushes += 1;
        }
        for i in 0..10_000u64 {
            let now = 3_000 + i * 5;
            let m = Machine::new(((i % 200) as u8).wrapping_add(2));
            let from = peer(1_000 + i, 10);
            if let Ok((nid, _)) = start(&mut p, now, from, &m) {
                admitted += 1;
                let _ = nid;
                p.connection_closed(from.conn, now);
            }
            if p.take_push(now).is_some() {
                pushes += 1;
            }
        }
        // ~50 s of attack: at most 3 admissions a minute from the address (the legitimate one used one already).
        assert!(admitted <= 3, "{admitted}");
        assert!(pushes <= 1, "{pushes}");
        // The legitimate request is untouched and can still be approved.
        assert!(p.limits().keys() <= 2);
        let _ = p.refusal_count_for_test();
        assert_eq!(p.verdict(ConnId(1), 60_000), Verdict::Waiting);
        p.approve(60_010, &id, false).unwrap();
    }

    #[test]
    fn refusals_are_reported_as_a_rate_once_a_minute() {
        let mut p = open_hub();
        for i in 0..50u64 {
            let _ = start(
                &mut p,
                2_000 + i,
                peer(i, 10),
                &Machine::new((i % 250) as u8 + 1),
            );
        }
        assert!(p.take_refusal_rates(2_100).is_empty() || p.last_report > 0);
        // Nothing reported again within the minute; the next report carries the counts, then resets.
        for i in 0..20u64 {
            let _ = start(
                &mut p,
                3_000 + i,
                peer(100 + i, 10),
                &Machine::new((i % 250) as u8 + 100),
            );
        }
        assert!(p.take_refusal_rates(3_100).is_empty());
        let report = p.take_refusal_rates(2_100 + REFUSAL_REPORT_MS);
        assert_eq!(report.len(), 1, "one address key, one line");
        assert!(report[0].count >= 10, "{report:?}");
        assert!(p
            .take_refusal_rates(2_100 + 2 * REFUSAL_REPORT_MS)
            .is_empty());
    }

    #[test]
    fn pushes_are_coalesced_to_one_a_minute_and_summed() {
        let mut p = open_hub();
        pending(&mut p, 2_000, 1, 10, &Machine::new(1));
        assert_eq!(p.take_push(2_100), Some(1));
        pending(&mut p, 3_000, 2, 11, &Machine::new(2));
        pending(&mut p, 4_000, 3, 12, &Machine::new(3));
        assert_eq!(p.take_push(5_000), None);
        assert_eq!(p.take_push(2_100 + PUSH_EVERY_MS - 1), None);
        assert_eq!(p.take_push(2_100 + PUSH_EVERY_MS), Some(2));
        assert_eq!(p.take_push(2_100 + 2 * PUSH_EVERY_MS), None);
    }

    // ---- what windows read ----

    #[test]
    fn a_window_reads_the_cards_and_then_the_notes_after_its_cursor() {
        let mut p = open_hub();
        let a = pending(&mut p, 2_000, 1, 10, &Machine::new(1));
        let (cards, cursor) = p.cards(2_100);
        assert_eq!(cards.len(), 1);
        let b = pending(&mut p, 2_200, 2, 11, &Machine::new(2));
        p.deny(2_300, &a).unwrap();
        let (notes, next) = p.notes_after(2_400, cursor);
        assert_eq!(notes.len(), 2);
        assert!(matches!(&notes[0], Note::Requested(c) if c.pair_id == b));
        assert!(
            matches!(&notes[1], Note::Decided { pair_id, outcome: Outcome::Denied } if *pair_id == a)
        );
        assert!(p.notes_after(2_500, next).0.is_empty());
    }

    #[test]
    fn the_notes_are_bounded() {
        let mut p = open_hub();
        for i in 0..200u64 {
            p.note(Note::Decided {
                pair_id: [0; 16],
                outcome: Outcome::Closed,
            });
            let _ = i;
        }
        assert_eq!(p.notes_after(0, 0).0.len(), NOTES_KEPT);
    }

    #[test]
    fn debug_output_never_shows_the_number_the_nonces_or_the_csr() {
        let mut p = open_hub();
        let id = pending(&mut p, 2_000, 1, 10, &Machine::new(1));
        let number = p.number_of(&id).unwrap();
        let (cards, _) = p.cards(2_100);
        let text = format!("{p:?} {:?} {:?}", cards[0], p.notes_after(0, 0));
        assert!(!text.contains(&number), "{text}");
        p.approve(2_200, &id, false).unwrap();
        let Verdict::Approved(issue) = p.verdict(ConnId(1), 2_300) else {
            panic!()
        };
        assert!(!format!("{issue:?}").contains("CSR"));
    }

    #[test]
    fn text_that_reorders_or_hides_is_refused_in_every_field_the_card_shows() {
        let m = Machine::new(1);
        for bad in [
            "\u{202E}gnp",
            "a\u{200B}b",
            "a\u{200F}b",
            "a\u{2066}b",
            "a\u{2069}b",
            "\u{FEFF}a",
            "a\u{202A}b",
        ] {
            for (name, os, version, field) in [
                (bad, "linux", "v", "node_name"),
                ("ok", bad, "v", "os"),
                ("ok", "linux", bad, "version"),
            ] {
                let mut p = open_hub();
                let c = m.commit();
                let commit = Commit {
                    commitment: &c,
                    name,
                    os,
                    version,
                };
                assert_eq!(
                    p.request(2_000, peer(1, 10), m.exporter, commit)
                        .unwrap_err(),
                    Refusal::Invalid(field),
                    "{bad:?} in {field}"
                );
            }
        }
        // Ordinary non-ASCII text is fine for the fields that are not the node id.
        let mut p = open_hub();
        let c = m.commit();
        let commit = Commit {
            commitment: &c,
            name: "ok",
            os: "Windows 11 (t\u{fc}rk\u{e7}e)",
            version: "0.3",
        };
        assert!(p.request(2_000, peer(1, 10), m.exporter, commit).is_ok());
    }

    #[test]
    fn one_host_hammering_does_not_use_up_the_hubs_share_for_everyone_else() {
        let mut limits = PairLimits::new();
        let mut host_ok = 0;
        for i in 0..200u64 {
            if limits.admit(Kind::Machine, ip(1), 5_000 + i).is_ok() {
                host_ok += 1;
            }
        }
        // The address's own limit holds: 3 a minute, whatever it sends.
        assert_eq!(host_ok, KEY_PER_MINUTE);
        // Different addresses are still admitted (the hub-wide cap counted only what passed an address's limit).
        for last in 2..=8u8 {
            assert!(
                limits.admit(Kind::Machine, ip(last), 5_300).is_ok(),
                "address {last} was refused after one host hammered"
            );
        }
        // The hub-wide cap is still the last guard for many addresses together.
        let ok = (20..120u8)
            .filter(|last| limits.admit(Kind::Machine, ip(*last), 5_400).is_ok())
            .count();
        assert!(ok <= HUB_PER_MINUTE, "{ok}");
    }

    #[test]
    fn a_host_that_sends_a_reveal_first_cannot_close_the_owners_window() {
        let mut p = open_hub();
        // Many connections, never admitted, each naming a pair id they cannot own.
        for i in 0..50u64 {
            let r = p.unknown(2_000 + i, peer(100 + i, 66));
            assert_eq!(r, Refusal::Unknown);
        }
        assert!(p.status(2_100).open, "the window was closed by strangers");
        // It does count against that address's own limiter, which locks it out.
        assert!(p.limits().locked(Kind::Machine, ip(66), 2_100).is_some());
        // And a real machine from another address still pairs.
        let id = pending(&mut p, 3_000, 1, 10, &Machine::new(1));
        p.approve(3_100, &id, false).unwrap();
        // A real session's failures still count for the window (the 5-failure close is unchanged).
        let mut q = open_hub();
        for i in 0..4u8 {
            let m = Machine::new(i + 1);
            let from = peer(u64::from(i), 10 + i);
            let (id, _) = start(&mut q, 2_000, from, &m).unwrap();
            let _ = q.reveal(
                2_010,
                from,
                Reveal {
                    pair_id: &id,
                    nonce_n: &[99; 16],
                    csr_pem: "c",
                    csr_hash: Some(m.csr_hash),
                    node_id_ok: true,
                },
            );
        }
        assert!(q.status(2_030).open);
    }

    /// F800: the refused attempt counts too, so the stated wait must leave room for the retry itself.
    #[test]
    fn a_retry_after_the_stated_wait_is_admitted() {
        let history = || {
            let mut limits = PairLimits::new();
            for t in [10_000, 11_000, 12_000] {
                limits.admit(Kind::Machine, ip(1), t).unwrap();
            }
            let wait = limits.admit(Kind::Machine, ip(1), 13_000).unwrap_err();
            (limits, 13_000 + wait * 1000)
        };
        let (mut limits, at) = history();
        assert_eq!(limits.admit(Kind::Machine, ip(1), at), Ok(()));
        // ...and the wait is not generous: a second earlier is still refused.
        let (mut limits, at) = history();
        assert!(limits.admit(Kind::Machine, ip(1), at - 1000).is_err());
        // The hub-wide counter: the eleventh address in a minute waits, and one more at the stated time is admitted.
        let mut limits = PairLimits::new();
        for i in 0..10u8 {
            limits
                .admit(Kind::Machine, ip(100 + i), 70_000 + u64::from(i))
                .unwrap();
        }
        let wait = limits.admit(Kind::Machine, ip(120), 70_100).unwrap_err();
        assert_eq!(
            limits.admit(Kind::Machine, ip(121), 70_100 + wait * 1000),
            Ok(())
        );
    }

    /// F801: a closed switch refuses a reveal-first with `pair.closed` and counts nothing, so nobody can lock an address out or fill
    /// the counter table before the owner opens it.
    #[test]
    fn a_reveal_first_while_closed_is_closed_and_counted_nowhere() {
        let mut p = Pairing::with_entropy(counter_entropy());
        p.set_leaf(LEAF);
        for i in 0..5u64 {
            assert_eq!(p.unknown(1_000 + i, peer(100 + i, 10)), Refusal::Closed);
        }
        assert_eq!(p.limits().keys(), 0);
        p.open(2_000);
        assert!(start(&mut p, 2_100, peer(1, 10), &Machine::new(1)).is_ok());
    }

    /// F811: two requests for one node id. While one approval waits or is being issued, the other cannot be approved (only one can
    /// get the certificate, and the windows would be told "approved" for both); it stays pending until that connection ends. Since
    /// an approval closes the switch (#942), the second request is one made after the owner opened it again.
    #[test]
    fn a_second_approval_for_a_node_id_in_flight_is_name_in_use() {
        let mut p = open_hub();
        let a = pending(&mut p, 2_000, 1, 10, &Machine::new(1));
        p.approve(2_050, &a, false).unwrap();
        p.open(2_060);
        let b = pending(&mut p, 2_100, 2, 11, &Machine::new(2));
        assert_eq!(
            p.approve(2_300, &b, true),
            Err(Refusal::State("name_in_use"))
        );
        assert!(matches!(p.verdict(ConnId(1), 2_400), Verdict::Approved(_)));
        assert_eq!(
            p.approve(2_500, &b, true),
            Err(Refusal::State("name_in_use"))
        );
        assert_eq!(p.pending_node_id(&b), Some("laptop"));
        p.connection_closed(ConnId(1), 2_600);
        assert_eq!(p.approve(2_700, &b, true), Ok(()));
    }

    /// Codex review of F811: once the approval is issued (or refused), the name is let go even while that connection stays open, as
    /// one whose machine stopped reading the result would; `connection_closed` afterwards changes nothing.
    #[test]
    fn an_issued_approval_lets_its_node_id_go_while_its_connection_stays_open() {
        let mut p = open_hub();
        let a = pending(&mut p, 2_000, 1, 10, &Machine::new(1));
        p.approve(2_050, &a, false).unwrap();
        p.open(2_060);
        let b = pending(&mut p, 2_100, 2, 11, &Machine::new(2));
        assert!(matches!(p.verdict(ConnId(1), 2_300), Verdict::Approved(_)));
        p.issued(ConnId(1));
        assert_eq!(p.approve(2_400, &b, true), Ok(()));
        p.connection_closed(ConnId(1), 2_500);
        assert!(matches!(p.verdict(ConnId(2), 2_600), Verdict::Approved(_)));
    }

    #[test]
    fn the_text_a_machine_sends_is_bounded() {
        let m = Machine::new(1);
        let long = "x".repeat(TEXT_MAX + 1);
        for (name, os, field) in [
            ("", "linux", "node_name"),
            (long.as_str(), "linux", "node_name"),
            ("a\nb", "linux", "node_name"),
            ("ok", long.as_str(), "os"),
            ("ok", "lin\u{2800}ux", "os"),
        ] {
            let mut p = open_hub();
            let c = m.commit();
            let commit = Commit {
                commitment: &c,
                name,
                os,
                version: "v",
            };
            let r = p.request(2_000, peer(1, 10), m.exporter, commit);
            assert_eq!(r.unwrap_err(), Refusal::Invalid(field), "{name:?}");
        }
        let mut p = open_hub();
        let commit = Commit {
            commitment: &[0; 5],
            name: "ok",
            os: "linux",
            version: "v",
        };
        let r = p.request(2_000, peer(1, 10), m.exporter, commit);
        assert_eq!(r.unwrap_err(), Refusal::Invalid("commitment"));
    }

    #[test]
    fn refusals_become_stable_codes_with_parameters() {
        assert_eq!(Refusal::Closed.to_error().code, ErrorCode::PairClosed);
        assert_eq!(Refusal::Unknown.to_error().code, ErrorCode::PairUnknown);
        assert_eq!(Refusal::Mismatch.to_error().code, ErrorCode::PairMismatch);
        assert_eq!(Refusal::Expired.to_error().code, ErrorCode::PairExpired);
        let e = Refusal::Limit { seconds: 7 }.to_error();
        assert_eq!(e.code, ErrorCode::PairLimit);
        assert!(e.params.contains_key("seconds"));
        let e = Refusal::State("pending").to_error();
        assert_eq!(e.code, ErrorCode::PairState);
        assert!(e.params.contains_key("state"));
        assert_eq!(
            Refusal::Invalid("x").to_error().code,
            ErrorCode::ProtocolInvalidMessage
        );
    }

    impl Pairing {
        fn refusal_count_for_test(&self) -> u32 {
            self.refusals.values().sum()
        }
    }
}
