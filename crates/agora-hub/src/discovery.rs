//! Finding a hub on the local network (docs/specs/pairing.md section 3), both halves in one place so they cannot drift: the responder
//! the hub runs **only while it is open for pairing**, and the finder `agora join` runs without an address.
//!
//! `std::net::UdpSocket` only. IPv4 only: a directed or limited broadcast query to [`DISCOVERY_PORT`]. (IPv6 has no broadcast and
//! would need multicast group membership per interface; not built, see section 15 of the spec.)
//!
//! What an answer is: **a candidate address, never trust.** The answer carries the hub's name, its port and the SHA-256 of its CA, all
//! public. Nothing is sent to the hub because it answered and nothing is believed: the machine still does the whole pairing exchange
//! (section 5), and a party that announces itself falsely gets a hub that cannot make the number on the owner's screen match.
//!
//! What the responder never does: answer while the switch is closed (the socket does not exist then), answer a datagram that is not a
//! full-size request (so a reply is never larger than what asked for it, and a spoofed source cannot be used to amplify), answer faster
//! than [`GLOBAL_PER_SECOND`] a second for the whole hub, answer more than [`PER_SOURCE`] searches in [`PER_SOURCE_WINDOW_MS`] from one
//! address (a copy of a search counted already is answered again at most once each [`COPY_INTERVAL_MS`]), or announce an
//! address that is not in the hub's certificate (a machine that dials it could not pass step 6b).

use std::collections::{HashMap, VecDeque};
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use agora_protocol::v1::HubAnnouncement;
use prost::Message;

/// The UDP port a hub that is open for pairing listens on.
pub const DISCOVERY_PORT: u16 = 7446;
/// The first bytes of a request.
pub const PREFIX: &[u8; 12] = b"AGORA-DISCO1";
/// A request is exactly this long at least: 12 bytes of prefix, 16 random bytes, padding. A reply is never longer than this.
pub const REQUEST_LEN: usize = 128;
/// At most this many replies a second for the whole hub (the spec's figure).
pub const GLOBAL_PER_SECOND: usize = 10;
/// At most this many searches (requests with distinct random bytes) from one source address are answered in the window below.
pub const PER_SOURCE: usize = 3;
pub const PER_SOURCE_WINDOW_MS: u64 = 10_000;
/// A copy of a search counted already (the same random bytes from the same source: sent to both broadcast addresses, or repeated
/// because a datagram can be lost) is answered again at most this often, and is not counted again (audit F920).
pub const COPY_INTERVAL_MS: u64 = 1_000;
/// Sources remembered at once (the hub-wide rate lets at most 100 new ones into a window); a full table drops new sources instead of forgetting old ones (a flood from many spoofed addresses
/// cannot reset anyone's count, it can only make discovery miss until the window passes).
const MAX_SOURCES: usize = 32;
/// The finder keeps at most this many hubs; further answers are dropped.
pub const MAX_FOUND: usize = 8;
/// The finder reads at most this many datagrams from one socket in one pass, so a flood cannot hold it past its deadline.
const READS_PER_PASS: usize = 32;
const MAX_NAME_BYTES: usize = 48;
const MAX_VERSION_BYTES: usize = 16;

/// The request a finder sends: prefix, 16 random bytes, zero padding to [`REQUEST_LEN`].
pub fn request(random: [u8; 16]) -> [u8; REQUEST_LEN] {
    let mut r = [0u8; REQUEST_LEN];
    r[..12].copy_from_slice(PREFIX);
    r[12..28].copy_from_slice(&random);
    r
}

/// True for a datagram the responder may answer: the prefix and at least [`REQUEST_LEN`] bytes.
pub fn is_request(datagram: &[u8]) -> bool {
    datagram.len() >= REQUEST_LEN && datagram.starts_with(PREFIX)
}

fn cut(s: &str, max: usize) -> &str {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// The reply, always at most [`REQUEST_LEN`] bytes: the name and version are cut to fit.
pub fn announcement(name: &str, port: u16, ca_fingerprint: [u8; 32], version: &str) -> Vec<u8> {
    HubAnnouncement {
        name: cut(name, MAX_NAME_BYTES).to_string(),
        port: u32::from(port),
        ca_fingerprint: ca_fingerprint.to_vec(),
        accepts_pairing: true,
        version: cut(version, MAX_VERSION_BYTES).to_string(),
    }
    .encode_to_vec()
}

/// A hub that answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub name: String,
    /// The address the answer came from and the hub's port: what `agora join` would be given.
    pub addr: SocketAddr,
    pub ca_fingerprint: [u8; 32],
    pub version: String,
}

/// Text from the network is untrusted: no control character or character in the shared hidden-character policy may make one name
/// look like another.
fn plain(s: &str) -> bool {
    !s.is_empty()
        && !s
            .chars()
            .any(|c| c.is_control() || agora_core::text::is_hidden(c))
}

/// Reads an answer, or `None` for anything that is not one (too long, not a message, not accepting, no name, an unusable port or
/// fingerprint, text that is not plain).
pub fn parse_announcement(datagram: &[u8], from: IpAddr) -> Option<Found> {
    if datagram.len() > REQUEST_LEN {
        return None;
    }
    let a = HubAnnouncement::decode(datagram).ok()?;
    let port = u16::try_from(a.port).ok().filter(|p| *p != 0)?;
    let fp: [u8; 32] = a.ca_fingerprint.as_slice().try_into().ok()?;
    if !a.accepts_pairing || !plain(&a.name) || a.name.len() > MAX_NAME_BYTES {
        return None;
    }
    if a.version.len() > MAX_VERSION_BYTES || (!a.version.is_empty() && !plain(&a.version)) {
        return None;
    }
    Some(Found {
        name: a.name,
        addr: SocketAddr::new(from, port),
        ca_fingerprint: fp,
        version: a.version,
    })
}

/// The fingerprint as `agora hub fingerprint` prints it: upper-case hex pairs separated by colons.
pub fn fingerprint_text(fp: &[u8; 32]) -> String {
    fp.iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// The short form a person compares: the first 12 bytes (96 bits) of the CA's SHA-256 as 24 hex digits in 6 groups of 4
/// (`566A D067 41A8 B995 CD07 C464`).
/// `agora pair open` and `agora pair status` print the same on the hub's screen, and `agora join` shows it for each hub it found. It is
/// for human comparison; the machine itself compares all 32 bytes.
pub fn short_fingerprint(fp: &[u8; 32]) -> String {
    fp[..12]
        .chunks(2)
        .map(|c| format!("{:02X}{:02X}", c[0], c[1]))
        .collect::<Vec<_>>()
        .join(" ")
}

// ---- rate limits (pure: the caller gives the time) ----

/// One search counted for a source: when it was counted, its random bytes (none for a caller that gives none), and when a reply to
/// it was last sent ([`ReplyLimits::answered`]; none while every try got silence, its address not in the certificate yet).
struct Counted {
    at: u64,
    nonce: Option<[u8; 16]>,
    last: Option<u64>,
}

/// Per source: the searches counted in the window.
type Sent = VecDeque<Counted>;

#[derive(Default)]
pub struct ReplyLimits {
    global: VecDeque<u64>,
    sources: HashMap<IpAddr, Sent>,
}

impl ReplyLimits {
    pub fn new() -> Self {
        Self::default()
    }

    /// True when a reply to `source` may be sent at `now_ms`; counts it when so.
    pub fn allow(&mut self, source: IpAddr, now_ms: u64) -> bool {
        self.allow_request(source, None, now_ms)
    }

    /// [`Self::allow`] for a request carrying the random bytes `nonce`. The source's budget counts searches, not datagrams: a copy of
    /// a search counted already in the window (the query sent to the limited and the directed broadcast address, or sent again because
    /// a datagram can be lost) is not counted again, and is answered again at most once each [`COPY_INTERVAL_MS`]. So one search
    /// spends one of the source's [`PER_SOURCE`], a reply that was lost is made good by a later copy, and a search made again at once
    /// is still answered (audit F920). The hub-wide [`GLOBAL_PER_SECOND`] counts every reply, copies too.
    pub fn allow_request(&mut self, source: IpAddr, nonce: Option<[u8; 16]>, now_ms: u64) -> bool {
        while self
            .global
            .front()
            .is_some_and(|t| now_ms.saturating_sub(*t) >= 1_000)
        {
            self.global.pop_front();
        }
        if self.global.len() >= GLOBAL_PER_SECOND {
            return false;
        }
        if !self.sources.contains_key(&source) && self.sources.len() >= MAX_SOURCES {
            self.sources.retain(|_, q| {
                q.back()
                    .is_some_and(|c| now_ms.saturating_sub(c.at) < PER_SOURCE_WINDOW_MS)
            });
            if self.sources.len() >= MAX_SOURCES {
                return false;
            }
        }
        let q = self.sources.entry(source).or_default();
        while q
            .front()
            .is_some_and(|c| now_ms.saturating_sub(c.at) >= PER_SOURCE_WINDOW_MS)
        {
            q.pop_front();
        }
        if let Some(c) = q.iter_mut().find(|c| nonce.is_some() && c.nonce == nonce) {
            if c.last
                .is_some_and(|t| now_ms.saturating_sub(t) < COPY_INTERVAL_MS)
            {
                return false;
            }
            self.global.push_back(now_ms);
            return true;
        }
        if q.len() >= PER_SOURCE {
            return false;
        }
        q.push_back(Counted {
            at: now_ms,
            nonce,
            last: None,
        });
        self.global.push_back(now_ms);
        true
    }

    /// A reply to the search with the random bytes `nonce` from `source` was sent at `now_ms`: its copies wait
    /// [`COPY_INTERVAL_MS`] from now.
    pub fn answered(&mut self, source: IpAddr, nonce: [u8; 16], now_ms: u64) {
        if let Some(c) = self
            .sources
            .get_mut(&source)
            .and_then(|q| q.iter_mut().find(|c| c.nonce == Some(nonce)))
        {
            c.last = Some(now_ms);
        }
    }
}

// ---- the responder ----

/// What the responder needs to know, all of it public.
pub struct Config {
    /// The UDP port to listen on ([`DISCOVERY_PORT`]; a test uses another).
    pub port: u16,
    /// The local address to bind (`0.0.0.0` for the network; a test binds loopback).
    pub bind: Ipv4Addr,
    /// The hub's name as shown to a person (its host name).
    pub name: Box<dyn Fn() -> String + Send + Sync>,
    /// The port of the hub's `wss://` listener.
    pub hub_port: u16,
    /// SHA-256 of the CA certificate's DER.
    pub ca_fingerprint: [u8; 32],
    pub version: &'static str,
    /// The addresses in the hub's certificate. The responder answers a query only from an address in this list.
    pub addresses: Box<dyn Fn() -> Vec<IpAddr> + Send + Sync>,
}

struct Running {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
    local: SocketAddr,
}

pub struct Responder {
    cfg: Config,
    limits: Mutex<ReplyLimits>,
    started: Instant,
    running: Mutex<Option<Running>>,
}

/// The local address the system would send from to reach `peer`: a UDP socket "connected" to it sends nothing and only picks a route.
fn local_toward(peer: SocketAddr) -> Option<IpAddr> {
    let any = match peer {
        SocketAddr::V4(_) => SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)),
        SocketAddr::V6(_) => SocketAddr::from((std::net::Ipv6Addr::UNSPECIFIED, 0)),
    };
    let s = UdpSocket::bind(any).ok()?;
    s.connect(peer).ok()?;
    s.local_addr().ok().map(|a| a.ip())
}

impl Responder {
    pub fn new(cfg: Config) -> Arc<Self> {
        Arc::new(Self {
            cfg,
            limits: Mutex::new(ReplyLimits::new()),
            started: Instant::now(),
            running: Mutex::new(None),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Running>> {
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The socket exists exactly while `open` is true: it is bound when the switch opens and closed (and the thread ended) before this
    /// returns when it closes. A hub that cannot bind the port (another hub on this machine has it) simply stays silent; the error is
    /// returned for the caller to say once.
    pub fn sync(self: &Arc<Self>, open: bool) -> io::Result<()> {
        let mut running = self.lock();
        if !open {
            if let Some(r) = running.take() {
                r.stop.store(true, Ordering::SeqCst);
                let _ = r.thread.join();
            }
            return Ok(());
        }
        if running.is_some() {
            return Ok(());
        }
        let socket = UdpSocket::bind(SocketAddr::from((self.cfg.bind, self.cfg.port)))?;
        socket.set_read_timeout(Some(Duration::from_millis(100)))?;
        let local = socket.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let me = Arc::clone(self);
        let flag = Arc::clone(&stop);
        let thread = std::thread::spawn(move || me.serve(&socket, &flag));
        *running = Some(Running {
            stop,
            thread,
            local,
        });
        Ok(())
    }

    /// Where the socket is bound while the switch is open, else `None`.
    pub fn bound(&self) -> Option<SocketAddr> {
        self.lock().as_ref().map(|r| r.local)
    }

    fn serve(&self, socket: &UdpSocket, stop: &AtomicBool) {
        let mut buf = [0u8; 2048];
        while !stop.load(Ordering::SeqCst) {
            match socket.recv_from(&mut buf) {
                Ok((n, src)) => {
                    if let Some(reply) = self.answer(&buf[..n], src) {
                        let _ = socket.send_to(&reply, src);
                    }
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock
                            | io::ErrorKind::TimedOut
                            | io::ErrorKind::Interrupted
                    ) => {}
                // A reply that bounced (ICMP "unreachable" shows up as an error on some systems): not a reason to stop.
                Err(e) if e.kind() == io::ErrorKind::ConnectionReset => {}
                Err(_) => std::thread::sleep(Duration::from_millis(100)),
            }
        }
    }

    /// The reply to one datagram, or `None` (silence). Pure but for the clock and the route lookup.
    pub fn answer(&self, datagram: &[u8], src: SocketAddr) -> Option<Vec<u8>> {
        if !is_request(datagram) {
            return None;
        }
        // Rate limits before any work that costs something.
        let now = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let nonce: Option<[u8; 16]> = datagram[12..28].try_into().ok();
        if !self
            .limits
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .allow_request(src.ip(), nonce, now)
        {
            return None;
        }
        // Only an address the certificate names may be announced: the finder dials the address the reply comes from, which is the one
        // the system sends from toward it.
        let from = local_toward(src)?;
        if !(self.cfg.addresses)().contains(&from) {
            return None;
        }
        let reply = announcement(
            &(self.cfg.name)(),
            self.cfg.hub_port,
            self.cfg.ca_fingerprint,
            self.cfg.version,
        );
        // The reply is never larger than the request (and the request is at least REQUEST_LEN): never an amplifier.
        if reply.len() > REQUEST_LEN || reply.len() > datagram.len() {
            return None;
        }
        if let Some(n) = nonce {
            self.limits
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .answered(src.ip(), n, now);
        }
        Some(reply)
    }
}

impl Drop for Responder {
    fn drop(&mut self) {
        if let Some(r) = self
            .running
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            r.stop.store(true, Ordering::SeqCst);
            let _ = r.thread.join();
        }
    }
}

// ---- the finder ----

/// Asks from every local address (with the targets it should ask: its own network's broadcast address and the limited broadcast
/// address) and collects answers for `wait`. A socket is bound to each local address (the port is chosen by the system) so that a
/// query leaves from the interface it is meant for, and is allowed to broadcast. The same query (one search, one set of random bytes)
/// is sent again every half second until `wait` ends (a datagram can be lost, and another hub may answer later): a hub counts a search
/// once and answers its copies at most once a second, so the repeats do not spend its per-source budget (audit F920). A hub that
/// answered more than once, from one address or several, is listed once. Nothing is sent to a hub except the query.
pub fn find(
    queries: &[(Ipv4Addr, Vec<SocketAddr>)],
    wait: Duration,
    random: [u8; 16],
) -> Vec<Found> {
    let sockets: Vec<(UdpSocket, &[SocketAddr])> = queries
        .iter()
        .filter_map(|(ip, targets)| {
            let s = UdpSocket::bind(SocketAddr::from((*ip, 0))).ok()?;
            s.set_broadcast(true).ok()?;
            s.set_nonblocking(true).ok()?;
            Some((s, targets.as_slice()))
        })
        .collect();
    let q = request(random);
    let deadline = Instant::now() + wait;
    let mut next_send = Instant::now();
    let mut found: Vec<Found> = Vec::new();
    let mut buf = [0u8; 2048];
    'listen: while Instant::now() < deadline {
        if Instant::now() >= next_send {
            for (s, targets) in &sockets {
                for t in *targets {
                    let _ = s.send_to(&q, t);
                }
            }
            next_send = Instant::now() + Duration::from_millis(500);
        }
        for (s, _) in &sockets {
            // A bounded number of reads per socket per pass, and the deadline is looked at inside the loop: a flood of answers can
            // neither keep this from finishing on time nor grow the list past MAX_FOUND.
            for _ in 0..READS_PER_PASS {
                if Instant::now() >= deadline {
                    break 'listen;
                }
                let Ok((n, src)) = s.recv_from(&mut buf) else {
                    break;
                };
                let Some(f) = parse_announcement(&buf[..n], src.ip()) else {
                    continue;
                };
                // The same hub answers from each of its addresses (a LAN card and a VPN, say): it is one hub, listed once, at the
                // first address that answered.
                if found.len() < MAX_FOUND && !found.iter().any(|g| same_hub(g, &f)) {
                    found.push(f);
                }
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    found
}

/// Two answers from one hub: the same CA, name and port.
fn same_hub(a: &Found, b: &Found) -> bool {
    (&a.ca_fingerprint, &a.name, a.addr.port()) == (&b.ca_fingerprint, &b.name, b.addr.port())
}

/// The directed broadcast address of `ip/prefix_len` (`192.168.1.255` for `192.168.1.20/24`), or `None` for a prefix with no room for
/// hosts (/31 and /32) or none given.
pub fn directed_broadcast(ip: Ipv4Addr, prefix_len: u8) -> Option<Ipv4Addr> {
    if prefix_len == 0 || prefix_len > 30 {
        return None;
    }
    let mask = u32::MAX << (32 - u32::from(prefix_len));
    Some(Ipv4Addr::from(u32::from(ip) | !mask))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const FP: [u8; 32] = [7; 32];

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn the_longest_possible_reply_fits_in_a_request() {
        let long = "n".repeat(500);
        for port in [1u16, 7443, 65535] {
            let r = announcement(&long, port, [255; 32], &"v".repeat(100));
            assert!(r.len() <= REQUEST_LEN, "{}", r.len());
        }
        // Multi-byte names are cut on a character boundary.
        let r = announcement(&"é".repeat(100), 7443, FP, "0.3.16");
        let f = parse_announcement(&r, ip("10.0.0.1")).unwrap();
        assert!(f.name.chars().all(|c| c == 'é') && f.name.len() <= MAX_NAME_BYTES);
    }

    #[test]
    fn an_announcement_round_trips_and_carries_nothing_secret() {
        let r = announcement("box", 7443, FP, "0.3.16");
        let f = parse_announcement(&r, ip("192.168.1.20")).unwrap();
        assert_eq!(
            f,
            Found {
                name: "box".into(),
                addr: "192.168.1.20:7443".parse().unwrap(),
                ca_fingerprint: FP,
                version: "0.3.16".into()
            }
        );
        assert_eq!(fingerprint_text(&FP), vec!["07"; 32].join(":"));
    }

    #[test]
    fn what_is_not_an_announcement_is_ignored() {
        let ok = |a: HubAnnouncement| parse_announcement(&a.encode_to_vec(), ip("10.0.0.1"));
        let base = HubAnnouncement {
            name: "box".into(),
            port: 7443,
            ca_fingerprint: FP.to_vec(),
            accepts_pairing: true,
            version: "1".into(),
        };
        assert!(ok(base.clone()).is_some());
        assert!(ok(HubAnnouncement {
            accepts_pairing: false,
            ..base.clone()
        })
        .is_none());
        assert!(ok(HubAnnouncement {
            port: 0,
            ..base.clone()
        })
        .is_none());
        assert!(ok(HubAnnouncement {
            port: 70_000,
            ..base.clone()
        })
        .is_none());
        assert!(ok(HubAnnouncement {
            ca_fingerprint: vec![1; 31],
            ..base.clone()
        })
        .is_none());
        assert!(ok(HubAnnouncement {
            name: String::new(),
            ..base.clone()
        })
        .is_none());
        assert!(ok(HubAnnouncement {
            name: "a\nb".into(),
            ..base.clone()
        })
        .is_none());
        assert!(ok(HubAnnouncement {
            name: "a\u{202E}b".into(),
            ..base.clone()
        })
        .is_none());
        assert!(ok(HubAnnouncement {
            name: "x".repeat(49),
            ..base
        })
        .is_none());
        assert!(parse_announcement(&[0xff; 20], ip("10.0.0.1")).is_none());
        assert!(parse_announcement(&[0u8; 129], ip("10.0.0.1")).is_none());
    }

    #[test]
    fn only_a_full_size_request_with_the_prefix_is_a_request() {
        let q = request([1; 16]);
        assert!(is_request(&q));
        assert!(is_request(&[&q[..], &[0u8; 100]].concat()));
        assert!(!is_request(&q[..127]));
        assert!(!is_request(&q[..1]));
        assert!(!is_request(&[]));
        let mut other = q;
        other[0] = b'X';
        assert!(!is_request(&other));
        assert!(!is_request(&[0u8; 128]));
    }

    #[test]
    fn the_whole_hub_answers_ten_a_second() {
        let mut l = ReplyLimits::new();
        let sent = (0..100)
            .filter(|i| {
                l.allow(
                    IpAddr::from([10, 0, (*i / 250) as u8, (*i % 250) as u8]),
                    5_000 + *i as u64,
                )
            })
            .count();
        assert_eq!(sent, GLOBAL_PER_SECOND);
        // A second later there is room again.
        assert!(l.allow(ip("10.9.9.9"), 6_200));
    }

    #[test]
    fn one_address_gets_three_replies_in_ten_seconds() {
        let mut l = ReplyLimits::new();
        let a = ip("10.0.0.5");
        assert!(l.allow(a, 0));
        assert!(l.allow(a, 1_100));
        assert!(l.allow(a, 2_200));
        assert!(!l.allow(a, 3_300));
        assert!(!l.allow(a, 9_999));
        // Another address is unaffected, and the first one is served again when its window has passed.
        assert!(l.allow(ip("10.0.0.6"), 3_400));
        assert!(l.allow(a, 10_000));
    }

    #[test]
    fn a_full_table_drops_new_sources_and_never_resets_old_ones() {
        let mut l = ReplyLimits::new();
        let mut t = 0;
        // Fill the table without ever hitting the hub-wide rate (a few a second).
        for i in 0..MAX_SOURCES {
            t += 150;
            assert!(
                l.allow(IpAddr::from([10, 1, (i / 250) as u8, (i % 250) as u8]), t),
                "{i}"
            );
        }
        t += 150;
        assert!(
            !l.allow(ip("10.2.0.1"), t),
            "a new source finds the table full"
        );
        // The oldest source still has its count (one used): it is not evicted to make room, and its limit still holds.
        let first = IpAddr::from([10, 1, 0, 0]);
        assert!(l.allow(first, t + 150));
        assert!(l.allow(first, t + 300));
        assert!(!l.allow(first, t + 450));
        // When the window passes the table empties and new sources are served.
        assert!(l.allow(ip("10.2.0.1"), t + PER_SOURCE_WINDOW_MS + 500));
    }

    #[test]
    fn invisible_and_look_alike_characters_are_not_plain_text() {
        for c in [
            '\u{2060}',
            '\u{2061}',
            '\u{2064}',
            '\u{00AD}',
            '\u{061C}',
            '\u{034F}',
            '\u{2800}',
            '\u{3164}',
            '\u{FE0F}',
            '\u{FFA0}',
            '\u{E0000}',
            '\u{E0041}',
            '\u{E007F}',
            '\u{200B}',
            '\u{202E}',
            '\u{FEFF}',
            '\u{7}',
        ] {
            assert!(!plain(&format!("a{c}b")), "{c:?}");
        }
        assert!(plain("Büro-Rechner 2"));
        assert!(plain("masaüstü"));
    }

    #[test]
    fn the_short_fingerprint_is_six_groups_of_four() {
        let mut fp = [0u8; 32];
        fp[..12].copy_from_slice(&[
            0x56, 0x6A, 0xD0, 0x67, 0x41, 0xA8, 0xB9, 0x95, 0xCD, 0x07, 0xC4, 0x64,
        ]);
        fp[12] = 0xFF;
        assert_eq!(short_fingerprint(&fp), "566A D067 41A8 B995 CD07 C464");
    }

    #[test]
    fn a_flood_of_answers_is_bounded_in_count_and_time() {
        // A fake hub that, once asked, answers with thousands of different well-formed hubs as fast as it can for longer than the
        // finder listens.
        let hub = UdpSocket::bind("127.0.0.1:0").unwrap();
        hub.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let at = hub.local_addr().unwrap();
        let flooder = std::thread::spawn(move || {
            let mut b = [0u8; 256];
            let Ok((_, asker)) = hub.recv_from(&mut b) else {
                return;
            };
            let end = Instant::now() + Duration::from_millis(1_500);
            let mut i = 0u32;
            while Instant::now() < end {
                i += 1;
                let a = announcement(&format!("rogue{i}"), 7443, [(i % 251) as u8; 32], "1");
                let _ = hub.send_to(&a, asker);
            }
        });
        let started = Instant::now();
        let found = find(
            &[(Ipv4Addr::LOCALHOST, vec![at])],
            Duration::from_millis(1_000),
            [9; 16],
        );
        let took = started.elapsed();
        flooder.join().unwrap();
        assert!(found.len() <= MAX_FOUND, "{}", found.len());
        assert_eq!(
            found.len(),
            MAX_FOUND,
            "the flood filled the list to its cap"
        );
        assert!(
            took < Duration::from_millis(1_400),
            "finished late: {took:?}"
        );
    }

    #[test]
    fn broadcast_addresses() {
        let b = |s: &str, p| directed_broadcast(s.parse().unwrap(), p);
        assert_eq!(
            b("192.168.1.20", 24),
            Some("192.168.1.255".parse().unwrap())
        );
        assert_eq!(b("10.1.2.3", 8), Some("10.255.255.255".parse().unwrap()));
        assert_eq!(b("10.1.2.3", 30), Some("10.1.2.3".parse().unwrap()));
        assert_eq!(b("10.1.2.3", 31), None);
        assert_eq!(b("10.1.2.3", 32), None);
        assert_eq!(b("10.1.2.3", 0), None);
    }

    fn responder(port: u16, addresses: Vec<IpAddr>) -> Arc<Responder> {
        Responder::new(Config {
            port,
            bind: Ipv4Addr::LOCALHOST,
            name: Box::new(|| "testhub".into()),
            hub_port: 7443,
            ca_fingerprint: FP,
            version: "9.9.9",
            addresses: Box::new(move || addresses.clone()),
        })
    }

    #[test]
    fn a_query_from_an_address_not_in_the_certificate_gets_silence() {
        let r = responder(0, vec![ip("192.168.9.9")]);
        assert!(r
            .answer(&request([1; 16]), "127.0.0.1:5555".parse().unwrap())
            .is_none());
        let r = responder(0, vec![ip("127.0.0.1")]);
        assert!(r
            .answer(&request([1; 16]), "127.0.0.1:5555".parse().unwrap())
            .is_some());
    }

    #[test]
    fn no_reply_is_larger_than_its_request_whatever_the_request() {
        let r = responder(0, vec![ip("127.0.0.1")]);
        for len in [0usize, 1, 11, 12, 27, 100, 127] {
            let mut d = request([2; 16]).to_vec();
            d.truncate(len);
            assert!(
                r.answer(&d, "127.0.0.1:5555".parse().unwrap()).is_none(),
                "{len}"
            );
        }
        for len in [128usize, 129, 600] {
            let mut d = request([len as u8; 16]).to_vec();
            d.resize(len, 0);
            let reply = r
                .answer(&d, SocketAddr::from(([127, 0, 0, 1], 6000 + len as u16)))
                .unwrap();
            assert!(
                reply.len() <= d.len() && reply.len() <= REQUEST_LEN,
                "{len}"
            );
        }
    }

    // ---- real sockets, on loopback ----

    #[test]
    fn the_socket_exists_only_while_the_switch_is_open() {
        let r = responder(0, vec![ip("127.0.0.1")]);
        assert!(r.bound().is_none());
        r.sync(true).unwrap();
        let at = r.bound().unwrap();
        let found = find(
            &[(Ipv4Addr::LOCALHOST, vec![at])],
            Duration::from_millis(900),
            [3; 16],
        );
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].name, "testhub");
        assert_eq!(found[0].addr, SocketAddr::from(([127, 0, 0, 1], 7443)));
        assert_eq!(found[0].ca_fingerprint, FP);
        // Closed: nothing is listening, so nothing answers, and the port is free again.
        r.sync(false).unwrap();
        assert!(r.bound().is_none());
        let found = find(
            &[(Ipv4Addr::LOCALHOST, vec![at])],
            Duration::from_millis(700),
            [4; 16],
        );
        assert!(found.is_empty(), "{found:?}");
        UdpSocket::bind(at).unwrap();
        // And it opens again.
        r.sync(true).unwrap();
        assert!(r.bound().is_some());
        r.sync(true).unwrap();
        r.sync(false).unwrap();
    }

    #[test]
    fn a_one_byte_query_and_a_wrong_prefix_get_no_answer_on_the_wire() {
        let r = responder(0, vec![ip("127.0.0.1")]);
        r.sync(true).unwrap();
        let at = r.bound().unwrap();
        let c = UdpSocket::bind("127.0.0.1:0").unwrap();
        c.set_read_timeout(Some(Duration::from_millis(400)))
            .unwrap();
        let mut wrong = request([5; 16]);
        wrong[3] = b'X';
        for d in [&[1u8][..], &request([5; 16])[..127], &wrong[..]] {
            c.send_to(d, at).unwrap();
            let mut b = [0u8; 2048];
            assert!(c.recv_from(&mut b).is_err(), "answered {} bytes", d.len());
        }
        // A real request still works afterwards, and its reply is no larger than it.
        c.send_to(&request([5; 16]), at).unwrap();
        let mut b = [0u8; 2048];
        let (n, _) = c.recv_from(&mut b).unwrap();
        assert!(n <= REQUEST_LEN);
        r.sync(false).unwrap();
    }

    #[test]
    fn a_flood_from_one_address_is_answered_three_times() {
        let r = responder(0, vec![ip("127.0.0.1")]);
        r.sync(true).unwrap();
        let at = r.bound().unwrap();
        let c = UdpSocket::bind("127.0.0.1:0").unwrap();
        c.set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        for i in 0..50u8 {
            c.send_to(&request([i; 16]), at).unwrap();
        }
        let mut b = [0u8; 2048];
        let mut got = 0;
        while c.recv_from(&mut b).is_ok() {
            got += 1;
        }
        assert_eq!(got, PER_SOURCE);
        r.sync(false).unwrap();
    }

    /// Audit F920 (and its Codex review): the per-source budget counts searches, not datagrams. A search's copies (the other broadcast
    /// address, the half-second repeats) are not counted and are answered again at most once a second, so a lost reply is made good;
    /// the budget still holds for distinct searches, and the hub-wide limit counts the copies too.
    #[test]
    fn a_search_is_counted_once_and_its_copies_are_answered_at_most_once_a_second() {
        let s = |b: u8| Some([b; 16]);
        // As the responder does: a reply that is allowed is sent, and recorded as sent.
        let ask = |l: &mut ReplyLimits, src: IpAddr, b: u8, t: u64| {
            let ok = l.allow_request(src, s(b), t);
            if ok {
                l.answered(src, [b; 16], t);
            }
            ok
        };
        let mut l = ReplyLimits::new();
        let a = ip("10.0.0.5");
        assert!(ask(&mut l, a, 1, 0));
        assert!(
            !ask(&mut l, a, 1, 10),
            "the copy to the other broadcast address"
        );
        assert!(!ask(&mut l, a, 1, 500), "a repeat within the second");
        assert!(
            ask(&mut l, a, 1, 1_000),
            "a repeat a second later (the reply may have been lost)"
        );
        assert!(ask(&mut l, a, 2, 1_100));
        assert!(ask(&mut l, a, 3, 1_200));
        assert!(!ask(&mut l, a, 4, 1_300), "a fourth search in the window");
        assert!(ask(&mut l, a, 1, 2_000), "copies are never counted");
        assert!(
            ask(&mut l, ip("10.0.0.6"), 1, 2_000),
            "another source is its own"
        );
        assert!(ask(&mut l, a, 4, 10_000), "the window passed");
        // A search that got silence (nothing sent, its address not in the certificate yet) may be tried again at once.
        assert!(l.allow_request(ip("10.0.0.7"), s(9), 3_000));
        assert!(l.allow_request(ip("10.0.0.7"), s(9), 3_010));
        // Copies count toward the hub-wide limit: ten sources' copies fill a second, and a new search waits.
        let mut l = ReplyLimits::new();
        let src = |i: u8| IpAddr::from([10, 1, 0, i]);
        for i in 0..GLOBAL_PER_SECOND as u8 {
            assert!(ask(&mut l, src(i), 1, 0));
        }
        for i in 0..GLOBAL_PER_SECOND as u8 {
            assert!(ask(&mut l, src(i), 1, 1_000));
        }
        assert!(!ask(&mut l, src(200), 2, 1_000));
    }

    /// Audit F920 on the wire: as `agora join` asks (one query, sent to two targets that both reach the hub, repeated), three searches
    /// in a row each find the hub; and the finder keeps asking after one hub answered, so a second hub whose first reply was lost
    /// (a fake that drops the first query and answers the next) is listed too (Codex review).
    #[test]
    fn a_search_made_again_at_once_finds_the_hub_and_a_late_second_hub_is_listed() {
        let r = responder(0, vec![ip("127.0.0.1")]);
        r.sync(true).unwrap();
        let at = r.bound().unwrap();
        for seed in 1..=3u8 {
            let found = find(
                &[(Ipv4Addr::LOCALHOST, vec![at, at])],
                Duration::from_millis(700),
                [seed; 16],
            );
            assert_eq!(found.len(), 1, "search {seed}: {found:?}");
        }
        // A fourth search inside the window is the budget at work; the second part asks a hub of its own.
        r.sync(false).unwrap();
        let r = responder(0, vec![ip("127.0.0.1")]);
        r.sync(true).unwrap();
        let at = r.bound().unwrap();
        let late = UdpSocket::bind("127.0.0.1:0").unwrap();
        let late_at = late.local_addr().unwrap();
        // Bounded: a finder that stops asking fails this test instead of hanging it.
        late.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let fake = std::thread::spawn(move || {
            let mut b = [0u8; 2048];
            if late.recv_from(&mut b).is_err() {
                return;
            }
            if let Ok((_, asker)) = late.recv_from(&mut b) {
                let _ = late.send_to(&announcement("late", 7443, [9; 32], "1"), asker);
            }
        });
        let found = find(
            &[(Ipv4Addr::LOCALHOST, vec![at, late_at])],
            Duration::from_millis(1_500),
            [42; 16],
        );
        fake.join().unwrap();
        let names: Vec<&str> = found.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(found.len(), 2, "{names:?}");
        assert!(names.contains(&"late"), "{names:?}");
        r.sync(false).unwrap();
    }
}
