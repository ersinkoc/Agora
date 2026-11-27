//! ADR-0088 build step 2: the node's table of preview forwards (`docs/specs/preview.md`).
//!
//! A forward is a named, time-limited permission to reach one port on this machine's loopback (`127.0.0.1` or `::1`) from the window.
//! This module holds the forwards and enforces the rules the hub can never widen: the port is 1024..=65535 and none of the node's own
//! ports (the hub on this machine, the hub link, the panel), it answers on the loopback when it is opened, at most [`MAX_FORWARDS`] are
//! open, and each ends at its expiry, when closed, or when the node stops (forwards are never restored).
//!
//! Who may open one is decided before this module is reached: the owner on the node's control socket, or a person whose device signed
//! a `preview-action-v1` open the node verified ([`crate::grants::Grants::preview_action`]). An agent can only raise a suggestion.

use std::collections::{BTreeMap, VecDeque};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use agora_protocol::v1;
use agora_protocol::{Error, ErrorCode};

/// At most this many forwards are open on one node (`preview.limit`).
pub const MAX_FORWARDS: usize = 8;
/// `expires_seconds` 0 means this: 8 hours.
pub const DEFAULT_SECONDS: u32 = 28_800;
/// The longest a forward lasts, whatever the open asked for: 24 hours.
pub const MAX_SECONDS: u32 = 86_400;
/// Ports below this are refused (`privileged`); the owner's policy may name some later (ADR-0088 decision 3, not built).
pub const FIRST_UNPRIVILEGED_PORT: u32 = 1024;
/// How long the open waits for the port to answer on each loopback address.
const PROBE: Duration = Duration::from_millis(500);
/// How many ended forwards are remembered, so a late connect hears `preview.expired` rather than `preview.unknown`.
const REMEMBERED: usize = 32;

/// Where one of the node's own ports is written. Read again at every open: the hub or the panel on this machine may start after the
/// node, or move to another port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortSource {
    /// A file holding a socket address a server on this machine listens on (`127.0.0.1:7443`, `0.0.0.0:7443`): the hub's `listen`.
    Listen(PathBuf),
    /// A file holding a port number alone: the panel's `panel-port`.
    Port(PathBuf),
    /// A file holding the hub's address as the node dials it (`wss://host:port`, the link's `url`): its port counts when the host is
    /// this machine, or when the node cannot tell.
    HubUrl(PathBuf),
}

/// The sources for a home laid out the default way: `<home>/hub/listen`, `<home>/link/url` and `<home>/link/panel-port`.
pub fn default_sources(home: &std::path::Path) -> Vec<PortSource> {
    vec![
        PortSource::Listen(home.join("hub").join("listen")),
        PortSource::HubUrl(home.join("link").join("url")),
        PortSource::Port(home.join("link").join("panel-port")),
    ]
}

/// What an open, a close or the clock changed, for `PreviewChanged` (the indicator and the audit trail: the port, never the content).
pub type Changed = Vec<v1::PreviewForward>;

/// One forward the table holds.
#[derive(Debug, Clone)]
struct Forward {
    port: u16,
    opened_at_ms: u64,
    expires_at_ms: u64,
    creator: String,
}

impl Forward {
    fn view(
        &self,
        id: &str,
        state: v1::PreviewState,
        reason: v1::PreviewCloseReason,
    ) -> v1::PreviewForward {
        v1::PreviewForward {
            forward_id: id.to_string(),
            port: u32::from(self.port),
            opened_at_ms: self.opened_at_ms,
            expires_at_ms: self.expires_at_ms,
            creator: self.creator.clone(),
            state: state as i32,
            close_reason: reason as i32,
        }
    }
}

#[derive(Default)]
struct Table {
    open: BTreeMap<String, Forward>,
    /// Ids that ended by expiry, newest last.
    expired: VecDeque<String>,
    /// The number of the last id handed out (`fwd-<n>`); ids are not reused while the node runs.
    last: u64,
}

/// The node's forwards. One per node, shared by the control socket and (through it) the hub's commands.
pub struct Previews {
    table: Mutex<Table>,
    sources: Mutex<Vec<PortSource>>,
}

impl Previews {
    pub fn new(sources: Vec<PortSource>) -> Self {
        Self {
            table: Mutex::new(Table::default()),
            sources: Mutex::new(sources),
        }
    }

    /// Adds places where the node's own ports are written (the binary knows its home's real layout).
    pub fn add_sources(&self, more: impl IntoIterator<Item = PortSource>) {
        let mut s = self.sources.lock().unwrap_or_else(|e| e.into_inner());
        for src in more {
            if !s.contains(&src) {
                s.push(src);
            }
        }
    }

    fn table(&self) -> std::sync::MutexGuard<'_, Table> {
        self.table.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Opens a forward to `port` for `seconds` (0: [`DEFAULT_SECONDS`]; capped at [`MAX_SECONDS`]) by `creator` (a device id, or
    /// `owner`). Every rule is checked here, whoever asks. The forward and what changed (the forwards that expired meanwhile, then
    /// the new one).
    pub fn open(
        &self,
        port: u32,
        seconds: u32,
        creator: &str,
        now_ms: u64,
    ) -> Result<(v1::PreviewForward, Changed), Error> {
        let refused = |reason: &str| {
            Error::new(ErrorCode::PreviewPortRefused)
                .with("port", i64::from(port))
                .with("reason", reason)
        };
        let Some(p) = u16::try_from(port).ok().filter(|p| *p != 0) else {
            return Err(refused("invalid"));
        };
        if port < FIRST_UNPRIVILEGED_PORT {
            return Err(refused("privileged"));
        }
        if self.own_ports().contains(&p) {
            return Err(refused("node_port"));
        }
        // Before the table's lock: a probe waits up to a second, and a list or a close must not wait for it.
        if !answers_on_loopback(p) {
            return Err(refused("not_loopback"));
        }
        let mut changed = self.sweep(now_ms);
        let mut t = self.table();
        if t.open.len() >= MAX_FORWARDS {
            return Err(Error::new(ErrorCode::PreviewLimit).with("limit", MAX_FORWARDS as i64));
        }
        let seconds = match seconds {
            0 => DEFAULT_SECONDS,
            s => s.min(MAX_SECONDS),
        };
        t.last += 1;
        let id = format!("fwd-{}", t.last);
        let f = Forward {
            port: p,
            opened_at_ms: now_ms,
            expires_at_ms: now_ms.saturating_add(u64::from(seconds) * 1000),
            creator: creator.to_string(),
        };
        let view = f.view(
            &id,
            v1::PreviewState::Open,
            v1::PreviewCloseReason::Unspecified,
        );
        t.open.insert(id, f);
        changed.push(view.clone());
        Ok((view, changed))
    }

    /// Closes one forward, or with `all` every open one ("Close all previews"). `all` with an id, or neither, is
    /// `protocol.invalid_message {field: forward_id}`; an id that is not open (never opened, closed, expired) is `preview.unknown`.
    pub fn close(&self, forward_id: &str, all: bool, now_ms: u64) -> Result<Changed, Error> {
        if all == !forward_id.is_empty() {
            return Err(Error::new(ErrorCode::ProtocolInvalidMessage).with("field", "forward_id"));
        }
        let mut changed = self.sweep(now_ms);
        let mut t = self.table();
        let ids: Vec<String> = if all {
            t.open.keys().cloned().collect()
        } else if t.open.contains_key(forward_id) {
            vec![forward_id.to_string()]
        } else {
            return Err(Error::new(ErrorCode::PreviewUnknown).with("forward_id", forward_id));
        };
        for id in ids {
            if let Some(f) = t.open.remove(&id) {
                changed.push(f.view(
                    &id,
                    v1::PreviewState::Closed,
                    v1::PreviewCloseReason::Closed,
                ));
            }
        }
        Ok(changed)
    }

    /// The open forwards, by id, and what expired since the last look.
    pub fn list(&self, now_ms: u64) -> (Vec<v1::PreviewForward>, Changed) {
        let changed = self.sweep(now_ms);
        let t = self.table();
        let open = t
            .open
            .iter()
            .map(|(id, f)| {
                f.view(
                    id,
                    v1::PreviewState::Open,
                    v1::PreviewCloseReason::Unspecified,
                )
            })
            .collect();
        (open, changed)
    }

    /// The port of an open forward, for a connection to it: `preview.expired` when its expiry passed, `preview.unknown` otherwise.
    pub fn target(&self, forward_id: &str, now_ms: u64) -> Result<(u16, Changed), Error> {
        let changed = self.sweep(now_ms);
        let t = self.table();
        if let Some(f) = t.open.get(forward_id) {
            return Ok((f.port, changed));
        }
        let code = if t.expired.iter().any(|id| id == forward_id) {
            ErrorCode::PreviewExpired
        } else {
            ErrorCode::PreviewUnknown
        };
        Err(Error::new(code).with("forward_id", forward_id))
    }

    /// Ends every forward whose expiry has passed; what ended. The node calls it on a timer so an expiry is announced even when
    /// nobody asks.
    pub fn sweep(&self, now_ms: u64) -> Changed {
        let mut t = self.table();
        let gone: Vec<String> = t
            .open
            .iter()
            .filter(|(_, f)| f.expires_at_ms <= now_ms)
            .map(|(id, _)| id.clone())
            .collect();
        let mut changed = Vec::new();
        for id in gone {
            if let Some(f) = t.open.remove(&id) {
                changed.push(f.view(
                    &id,
                    v1::PreviewState::Expired,
                    v1::PreviewCloseReason::Expired,
                ));
                t.expired.push_back(id);
                if t.expired.len() > REMEMBERED {
                    t.expired.pop_front();
                }
            }
        }
        changed
    }

    /// Ends every open forward because the node stops; what ended (`NODE_STOPPED`).
    pub fn stop_all(&self) -> Changed {
        let mut t = self.table();
        std::mem::take(&mut t.open)
            .into_iter()
            .map(|(id, f)| {
                f.view(
                    &id,
                    v1::PreviewState::Closed,
                    v1::PreviewCloseReason::NodeStopped,
                )
            })
            .collect()
    }

    /// The node's own ports, read now from every source. A source that cannot be read names no port (the file is not there when the
    /// hub or the panel does not run on this machine).
    fn own_ports(&self) -> Vec<u16> {
        let sources = self
            .sources
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let mut out = Vec::new();
        for s in &sources {
            match s {
                PortSource::Listen(path) => {
                    if let Some(a) =
                        read_small(path).and_then(|t| t.trim().parse::<SocketAddr>().ok())
                    {
                        out.push(a.port());
                    }
                }
                PortSource::Port(path) => {
                    if let Some(p) = read_small(path).and_then(|t| t.trim().parse::<u16>().ok()) {
                        out.push(p);
                    }
                }
                PortSource::HubUrl(path) => {
                    if let Some((host, port)) = read_small(path).as_deref().and_then(url_host_port)
                    {
                        if !surely_elsewhere(&host, port) {
                            out.push(port);
                        }
                    }
                }
            }
        }
        out
    }
}

/// A file of at most 4 KiB as text; `None` when it is missing, larger, or not text.
fn read_small(path: &std::path::Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > 4096 {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// The host and port of `wss://host:port/...` (`ws://` too; a bracketed IPv6 host loses its brackets). The port defaults to 443
/// for `wss` and 80 for `ws`.
fn url_host_port(url: &str) -> Option<(String, u16)> {
    let url = url.trim();
    let (scheme, rest) = url.split_once("://")?;
    let default = match scheme {
        "wss" | "https" => 443,
        "ws" | "http" => 80,
        _ => return None,
    };
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit_once('@').map_or(authority, |(_, a)| a);
    if let Some(v6) = authority.strip_prefix('[') {
        let (host, after) = v6.split_once(']')?;
        let port = match after.strip_prefix(':') {
            Some(p) => p.parse().ok()?,
            None if after.is_empty() => default,
            None => return None,
        };
        return Some((host.to_string(), port));
    }
    match authority.rsplit_once(':') {
        Some((host, p)) => Some((host.to_string(), p.parse().ok()?)),
        None => Some((authority.to_string(), default)),
    }
}

/// Whether `host` is certainly not this machine: it resolves, and none of its addresses is a loopback address or one of this machine's
/// own. When the node cannot tell (no answer from the resolver, the interfaces cannot be listed) the port counts as the node's own.
fn surely_elsewhere(host: &str, port: u16) -> bool {
    if host.eq_ignore_ascii_case("localhost") || host.is_empty() {
        return false;
    }
    let Ok(own) = agora_platform::interface_addresses() else {
        return false;
    };
    let addrs: Vec<IpAddr> = match host.parse::<IpAddr>() {
        Ok(ip) => vec![ip],
        Err(_) => match (host, port).to_socket_addrs() {
            Ok(a) => a.map(|a| a.ip()).collect(),
            Err(_) => return false,
        },
    };
    !addrs.is_empty()
        && addrs.iter().all(|ip| {
            let ip = canonical(*ip);
            !ip.is_loopback() && !ip.is_unspecified() && !own.iter().any(|o| canonical(o.ip) == ip)
        })
}

/// An IPv4-mapped IPv6 address as the IPv4 address it is.
fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 => v4,
    }
}

/// The loopback addresses a connection to `port` tries, in the order `localhost` resolves on this machine. Only the two literal
/// loopback addresses are ever used: a resolver that answers `localhost` with anything else changes nothing but the order.
pub fn loopback_order(port: u16) -> [SocketAddr; 2] {
    let v4 = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let v6 = SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), port);
    let v6_first = ("localhost", port)
        .to_socket_addrs()
        .ok()
        .and_then(|mut a| a.find(|a| a.ip().is_loopback()))
        .is_some_and(|a| a.is_ipv6());
    if v6_first {
        [v6, v4]
    } else {
        [v4, v6]
    }
}

/// Whether something accepts a connection on `port` on either loopback address.
fn answers_on_loopback(port: u16) -> bool {
    loopback_order(port)
        .iter()
        .any(|a| TcpStream::connect_timeout(a, PROBE).is_ok())
}

/// ADR-0088: what a device signs for a `PreviewOpen` sent through the hub: the domain `preview-action-v1`, one NUL byte, then the
/// node, `open`, the port, the requested seconds (0 for the default, as the request carries it), the nonce and the signature's expiry,
/// each as `F(x)` = decimal byte length, `:`, `x`. Shared vectors: `schema/fixtures/signing/preview-action-v1.json`.
pub fn signed_message(
    node: &str,
    port: u32,
    seconds: u32,
    nonce: &str,
    expires_at_ms: u64,
) -> Vec<u8> {
    let mut out = b"preview-action-v1\0".to_vec();
    for part in [
        node,
        "open",
        &port.to_string(),
        &seconds.to_string(),
        nonce,
        &expires_at_ms.to_string(),
    ] {
        out.extend_from_slice(part.len().to_string().as_bytes());
        out.push(b':');
        out.extend_from_slice(part.as_bytes());
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn listener() -> (TcpListener, u16) {
        let l = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let p = l.local_addr().unwrap().port();
        (l, p)
    }

    fn code(e: &Error) -> &str {
        e.code.as_str()
    }

    fn reason(e: &Error) -> Option<&str> {
        match e.params.get("reason") {
            Some(agora_protocol::ParamValue::Str(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    #[test]
    fn signed_bytes_follow_the_shared_vectors() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../schema/fixtures/signing/preview-action-v1.json");
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let cases = doc["cases"].as_array().unwrap();
        assert!(cases.len() >= 3);
        for c in cases {
            let bytes = signed_message(
                c["node"].as_str().unwrap(),
                u32::try_from(c["port"].as_u64().unwrap()).unwrap(),
                u32::try_from(c["expires_seconds"].as_u64().unwrap()).unwrap(),
                c["nonce"].as_str().unwrap(),
                c["expires"].as_u64().unwrap(),
            );
            let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(hex, c["hex"].as_str().unwrap(), "{}", c["name"]);
        }
    }

    #[test]
    fn an_open_names_its_port_and_creator_and_lasts_eight_hours_unless_asked() {
        let (_l, port) = listener();
        let p = Previews::new(Vec::new());
        let (f, changed) = p.open(u32::from(port), 0, "owner", 1_000).unwrap();
        assert_eq!(f.forward_id, "fwd-1");
        assert_eq!((f.port, f.creator.as_str()), (u32::from(port), "owner"));
        assert_eq!(f.expires_at_ms, 1_000 + 28_800_000);
        assert_eq!(f.state, v1::PreviewState::Open as i32);
        assert_eq!(changed, vec![f.clone()]);
        let (g, _) = p.open(u32::from(port), 999_999, "dev-1", 2_000).unwrap();
        assert_eq!(g.forward_id, "fwd-2");
        assert_eq!(g.expires_at_ms, 2_000 + 86_400_000, "capped at a day");
        assert_eq!(p.list(3_000).0.len(), 2);
    }

    #[test]
    fn ports_out_of_range_privileged_or_silent_are_refused() {
        let p = Previews::new(Vec::new());
        for (port, why) in [
            (0, "invalid"),
            (70_000, "invalid"),
            (22, "privileged"),
            (1023, "privileged"),
        ] {
            let e = p.open(port, 0, "owner", 1).unwrap_err();
            assert_eq!(code(&e), "preview.port_refused");
            assert_eq!(reason(&e), Some(why), "{port}");
        }
        // A port nobody listens on: bound, then let go.
        let (l, silent) = listener();
        drop(l);
        let e = p.open(u32::from(silent), 0, "owner", 1).unwrap_err();
        assert_eq!(reason(&e), Some("not_loopback"));
        assert!(p.list(1).0.is_empty());
    }

    #[test]
    fn the_nodes_own_ports_are_refused_whatever_listens_there() {
        let dir = std::env::temp_dir().join(format!("agora-preview-own-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("hub")).unwrap();
        std::fs::create_dir_all(dir.join("link")).unwrap();
        let (_hub, hub) = listener();
        let (_panel, panel) = listener();
        let (_link, link) = listener();
        let (_app, app) = listener();
        std::fs::write(dir.join("hub/listen"), format!("0.0.0.0:{hub}\n")).unwrap();
        std::fs::write(dir.join("link/panel-port"), format!("{panel}")).unwrap();
        std::fs::write(dir.join("link/url"), format!("wss://127.0.0.1:{link}")).unwrap();
        let p = Previews::new(default_sources(&dir));
        for port in [hub, panel, link] {
            let e = p.open(u32::from(port), 0, "owner", 1).unwrap_err();
            assert_eq!(reason(&e), Some("node_port"), "{port}");
        }
        assert!(
            p.open(u32::from(app), 0, "owner", 1).is_ok(),
            "any other listening port opens"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_hub_address_counts_unless_it_is_surely_another_machine() {
        assert_eq!(
            url_host_port("wss://hub.example:7443"),
            Some(("hub.example".into(), 7443))
        );
        assert_eq!(
            url_host_port("wss://[::1]:9000/x"),
            Some(("::1".into(), 9000))
        );
        assert_eq!(url_host_port("wss://box"), Some(("box".into(), 443)));
        assert_eq!(url_host_port("ftp://box:1"), None);
        assert!(!surely_elsewhere("localhost", 7443));
        assert!(!surely_elsewhere("127.0.0.1", 7443));
        assert!(!surely_elsewhere("::1", 7443));
        assert!(
            !surely_elsewhere("name.that-does-not-resolve.invalid", 7443),
            "unsure counts as own"
        );
        // TEST-NET-3, never an address of this machine.
        assert!(surely_elsewhere("203.0.113.9", 7443));
    }

    #[test]
    fn at_most_eight_are_open_and_closing_frees_a_place() {
        let (_l, port) = listener();
        let p = Previews::new(Vec::new());
        for _ in 0..MAX_FORWARDS {
            p.open(u32::from(port), 0, "owner", 1).unwrap();
        }
        let e = p.open(u32::from(port), 0, "owner", 1).unwrap_err();
        assert_eq!(code(&e), "preview.limit");
        let changed = p.close("fwd-3", false, 2).unwrap();
        assert_eq!(changed.len(), 1);
        assert_eq!(
            changed[0].close_reason,
            v1::PreviewCloseReason::Closed as i32
        );
        assert_eq!(
            p.open(u32::from(port), 0, "owner", 3).unwrap().0.forward_id,
            "fwd-9",
            "ids are not reused"
        );
        let all = p.close("", true, 4).unwrap();
        assert_eq!(all.len(), MAX_FORWARDS);
        assert!(p.list(5).0.is_empty());
    }

    #[test]
    fn close_needs_exactly_one_of_an_id_and_all() {
        let p = Previews::new(Vec::new());
        assert_eq!(
            code(&p.close("", false, 1).unwrap_err()),
            "protocol.invalid_message"
        );
        assert_eq!(
            code(&p.close("fwd-1", true, 1).unwrap_err()),
            "protocol.invalid_message"
        );
        assert_eq!(
            code(&p.close("fwd-1", false, 1).unwrap_err()),
            "preview.unknown"
        );
        assert!(
            p.close("", true, 1).unwrap().is_empty(),
            "closing all of none is fine"
        );
    }

    #[test]
    fn a_forward_ends_at_its_expiry_and_a_late_connect_hears_so() {
        let (_l, port) = listener();
        let p = Previews::new(Vec::new());
        let (f, _) = p.open(u32::from(port), 60, "owner", 1_000).unwrap();
        assert_eq!(p.target(&f.forward_id, 60_999).unwrap().0, port);
        let ended = p.sweep(61_000);
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0].state, v1::PreviewState::Expired as i32);
        assert!(p.sweep(61_001).is_empty(), "announced once");
        assert_eq!(
            code(&p.target(&f.forward_id, 61_001).unwrap_err()),
            "preview.expired"
        );
        assert_eq!(
            code(&p.target("fwd-77", 61_001).unwrap_err()),
            "preview.unknown"
        );
        assert_eq!(
            code(&p.close(&f.forward_id, false, 61_001).unwrap_err()),
            "preview.unknown"
        );
    }

    #[test]
    fn a_stopping_node_ends_every_forward() {
        let (_l, port) = listener();
        let p = Previews::new(Vec::new());
        p.open(u32::from(port), 0, "owner", 1).unwrap();
        let ended = p.stop_all();
        assert_eq!(
            ended[0].close_reason,
            v1::PreviewCloseReason::NodeStopped as i32
        );
        assert!(p.list(2).0.is_empty());
    }

    #[test]
    fn only_the_two_loopback_literals_are_ever_tried() {
        let order = loopback_order(3000);
        assert!(order
            .iter()
            .all(|a| a.ip().is_loopback() && a.port() == 3000));
        assert_ne!(order[0].ip(), order[1].ip());
    }
}
