//! ADR-0088 Amendment A, step 3: the hub's preview origins. Each forward the hub carries gets one port of a small pool, listened on
//! with the hub's own certificate, while it is carried. The opener's window asks for an entry ticket (`POST /api/preview/enter`);
//! the frame trades it on the forward's port for a cookie with the forward token; every later connection on that port carries one
//! request, whose head [`crate::preview_gate`] checks and rewrites before a byte reaches the node, and whose answer goes back with
//! `Connection: close`, after which nothing more from the browser is relayed (point 8). The bytes travel on the node's link as a
//! hub-originated `PreviewConnect` ([`crate::router::Router::connect_preview`]), under the node's credit rules.
//!
//! Revocation: a forward that ends, a person who signs out and a token past its time end live connections too, upgraded WebSockets
//! included: every connection checks its token again whenever the hub revokes anything, and every 30 s.
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agora_protocol::v1;
use prost::Message;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::watch;

use crate::preview_gate::{self as gate, Carried, Gate, HeadError, Partition, PortPool};
use crate::router::Router;
use crate::tls::TlsSlot;

/// The pool's size when none is given: the 16 ports after the hub's own (point 1).
pub const DEFAULT_POOL: u16 = 16;
/// The entry path a preview origin answers itself.
pub const ENTER_PATH: &str = "/.agora-preview/enter";
/// Live connections on one preview port at once (the node holds 32 per forward; this bounds the hub's sockets).
const CONNECTIONS_PER_PORT: usize = 64;
/// How long a browser may take to finish its TLS handshake and send its request head.
const HEAD_WAIT: Duration = Duration::from_secs(10);
/// How often a live connection checks its token again without a revocation (a token's 8 hours end within this).
const RECHECK: Duration = Duration::from_secs(30);

const PREVIEW_DATA: u8 = 20;
const PREVIEW_CREDIT: u8 = 21;
const PREVIEW_END: u8 = 22;

/// A forward as its node announced it (`PreviewChanged`, open).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Forward {
    /// The device that signed its open (`owner` for one opened on the node's control socket).
    pub creator: String,
    /// Its port on the node's loopback.
    pub node_port: u16,
    /// When it ends, Unix ms.
    pub ends_ms: u64,
}

/// Why the hub gives no entry ticket: an HTTP status and a stable code (with its `reason`), never words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub status: u16,
    pub code: &'static str,
    pub reason: &'static str,
}

const fn refused(status: u16, code: &'static str, reason: &'static str) -> Refused {
    Refused {
        status,
        code,
        reason,
    }
}

struct Config {
    ip: IpAddr,
    hub_port: u16,
    slot: TlsSlot,
}

#[derive(Default)]
struct Inner {
    config: Option<Config>,
    pool: Option<PortPool>,
    gate: Gate,
    forwards: HashMap<(String, String), Forward>,
    carried: HashMap<u16, Carried>,
    listeners: HashMap<u16, tokio::task::AbortHandle>,
}

/// The hub's preview origins (one per hub).
pub struct PreviewNet {
    inner: Mutex<Inner>,
    router: Arc<Router>,
    /// Bumped at every revocation; every live connection then checks its token again.
    epoch: watch::Sender<u64>,
}

fn now_ms() -> u64 {
    crate::server::now_ms()
}

impl PreviewNet {
    pub fn new(router: Arc<Router>) -> Self {
        Self {
            inner: Mutex::default(),
            router,
            epoch: watch::channel(0).0,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Turns the origins on: the hub listens on `listen` with the certificate in `slot`, and offers previews on `pool` (default the
    /// [`DEFAULT_POOL`] ports after its own). Without this call every entry is refused with `no_origin`.
    pub fn configure(&self, listen: SocketAddr, slot: TlsSlot, pool: Option<(u16, u16)>) {
        let first = listen.port().saturating_add(1);
        let (first, last) = pool.unwrap_or((first, first.saturating_add(DEFAULT_POOL - 1)));
        let mut g = self.lock();
        g.config = Some(Config {
            ip: listen.ip(),
            hub_port: listen.port(),
            slot,
        });
        g.pool = Some(PortPool::new(first, last));
    }

    /// A node's `PreviewChanged`: an open forward is remembered; one that ended is forgotten and, if the hub carries it, its port, its
    /// tickets, its tokens and its live connections end.
    pub fn changed(&self, node: &str, f: &v1::PreviewForward) {
        let key = (node.to_string(), f.forward_id.clone());
        if f.state == v1::PreviewState::Open as i32 {
            let Ok(node_port) = u16::try_from(f.port) else {
                return;
            };
            self.lock().forwards.insert(
                key,
                Forward {
                    creator: f.creator.clone(),
                    node_port,
                    ends_ms: f.expires_at_ms,
                },
            );
        } else {
            self.lock().forwards.remove(&key);
            self.end_forward(node, &f.forward_id);
        }
    }

    /// The forwards a node announced, as the hub knows them (for the window, slice 4).
    pub fn forward(&self, node: &str, forward: &str) -> Option<Forward> {
        self.lock()
            .forwards
            .get(&(node.to_string(), forward.to_string()))
            .cloned()
    }

    fn end_forward(&self, node: &str, forward: &str) {
        let mut g = self.lock();
        let port = g
            .carried
            .iter()
            .find(|(_, c)| c.node == node && c.forward == forward)
            .map(|(p, _)| *p);
        if let Some(port) = port {
            g.carried.remove(&port);
            g.gate.end_port(port);
            if let Some(pool) = g.pool.as_mut() {
                pool.release(port);
            }
            if let Some(task) = g.listeners.remove(&port) {
                task.abort();
            }
            drop(g);
            self.revoked();
        }
    }

    /// A person signed out or was removed: their tickets and tokens end, and so do their live connections.
    pub fn end_user(&self, user: &str) {
        self.lock().gate.end_user(user);
        self.revoked();
    }

    fn revoked(&self) {
        self.epoch.send_modify(|e| *e = e.wrapping_add(1));
    }

    /// An entry ticket for `user` to the forward `forward` on `node`, asked through a request whose `Host` header was `host`.
    /// `device_user` names the person a device belongs to (`None` for an unknown or revoked one). Returns the preview's port and the
    /// ticket.
    pub fn enter(
        self: &Arc<Self>,
        user: &str,
        device_user: impl Fn(&str) -> Option<String>,
        node: &str,
        forward: &str,
        host: &str,
    ) -> Result<(u16, String), Refused> {
        let now = now_ms();
        let f = self
            .forward(node, forward)
            .filter(|f| f.ends_ms > now)
            .ok_or(refused(404, "preview.unknown", ""))?;
        // Only the person whose device opened it (point 2). A forward opened on the node's own control socket has no device: nobody
        // enters it through the hub.
        if f.creator == "owner" || device_user(&f.creator).as_deref() != Some(user) {
            return Err(refused(403, "auth.forbidden", "preview"));
        }
        let mut g = self.lock();
        let Some(config) = g.config.as_ref() else {
            return Err(refused(409, "preview.unavailable", "no_origin"));
        };
        // A window that reached the hub through another port or a proxy cannot reach the hub's other ports either (point 1).
        if host_port(host) != Some(config.hub_port) {
            return Err(refused(409, "preview.unavailable", "no_origin"));
        }
        let ip = config.ip;
        let slot = config.slot.clone();
        let mut bound: Option<std::net::TcpListener> = None;
        let port = g
            .pool
            .as_mut()
            .and_then(|pool| {
                pool.take(node, forward, |p| {
                    match std::net::TcpListener::bind(SocketAddr::new(ip, p)) {
                        Ok(l) => {
                            bound = Some(l);
                            true
                        }
                        Err(_) => false,
                    }
                })
            })
            .ok_or(refused(409, "preview.unavailable", "no_origin"))?;
        if let Some(listener) = bound {
            // A port newly given to this forward: its own cookie tag, its listener.
            let tag = gate::random_hex()
                .map(|t| t[..8].to_string())
                .ok_or(refused(503, "preview.unavailable", "random"))?;
            let started = listener
                .set_nonblocking(true)
                .and_then(|()| tokio::net::TcpListener::from_std(listener));
            let Ok(listener) = started else {
                if let Some(pool) = g.pool.as_mut() {
                    pool.release(port);
                }
                return Err(refused(409, "preview.unavailable", "no_origin"));
            };
            g.carried.insert(
                port,
                Carried {
                    node: node.to_string(),
                    forward: forward.to_string(),
                    user: user.to_string(),
                    partition: Partition {
                        port,
                        tag,
                        node_port: f.node_port,
                    },
                    ends_ms: f.ends_ms,
                },
            );
            let task = tokio::spawn(accept(Arc::clone(self), port, listener, slot));
            g.listeners.insert(port, task.abort_handle());
        }
        let ticket =
            g.gate
                .ticket(port, user, now)
                .ok_or(refused(503, "preview.unavailable", "random"))?;
        Ok((port, ticket))
    }

    /// The person a token presented on `port` belongs to, while both it and the port's forward hold.
    fn check(&self, token: &str, port: u16) -> Option<String> {
        let g = self.lock();
        let carried = g.carried.get(&port)?;
        let user = g.gate.check(token, port, now_ms())?;
        (user == carried.user).then(|| user.to_string())
    }
}

/// The port a `Host` header names (443 when it names none, as a browser sends it for `https`).
fn host_port(host: &str) -> Option<u16> {
    let host = host.trim();
    // `[::1]:7443`, `name:7443`, `name`.
    let after = match host.rfind(']') {
        Some(end) => &host[end + 1..],
        None => host,
    };
    match after.rsplit_once(':') {
        Some((_, p)) if host.starts_with('[') || !p.is_empty() => p.parse().ok(),
        _ if host.starts_with('[') && after.is_empty() => Some(443),
        None => Some(443),
        _ => None,
    }
}

async fn accept(net: Arc<PreviewNet>, port: u16, listener: tokio::net::TcpListener, slot: TlsSlot) {
    let live = Arc::new(tokio::sync::Semaphore::new(CONNECTIONS_PER_PORT));
    loop {
        let Ok((tcp, _peer)) = listener.accept().await else {
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        };
        // More than the port's share at once: the newest is closed before its handshake.
        let Ok(permit) = Arc::clone(&live).try_acquire_owned() else {
            continue;
        };
        let (acceptor, _) = slot.current();
        let net = Arc::clone(&net);
        tokio::spawn(async move {
            let _permit = permit;
            let Ok(Ok(tls)) = tokio::time::timeout(HEAD_WAIT, acceptor.accept(tcp)).await else {
                return;
            };
            serve(&net, port, tls).await;
        });
    }
}

/// A short answer of the hub's own: a status, no body, the connection closed.
async fn answer<S: AsyncWrite + Unpin>(s: &mut S, status: u16) {
    let reason = match status {
        400 => "Bad Request",
        401 => "Unauthorized",
        431 => "Request Header Fields Too Large",
        502 => "Bad Gateway",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: 0\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n"
    );
    let _ = s.write_all(head.as_bytes()).await;
    let _ = s.shutdown().await;
}

/// The ticket in an entry path's query (`?t=<ticket>`), if the path is the entry path.
fn entry_ticket(target: &str) -> Option<&str> {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != ENTER_PATH {
        return None;
    }
    query
        .split('&')
        .find_map(|kv| kv.strip_prefix("t="))
        .filter(|t| t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit()))
}

async fn serve<S: AsyncRead + AsyncWrite + Unpin + Send>(
    net: &Arc<PreviewNet>,
    port: u16,
    mut tls: S,
) {
    // The request head, read whole (and only it) before anything else happens.
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 8192];
    let end = loop {
        match gate::head_end(&buf) {
            Ok(Some(end)) => break end,
            Ok(None) => {}
            Err(HeadError::TooLarge) => return answer(&mut tls, 431).await,
            Err(HeadError::Malformed) => return answer(&mut tls, 400).await,
        }
        match tokio::time::timeout(HEAD_WAIT, tls.read(&mut chunk)).await {
            Ok(Ok(n)) if n > 0 => buf.extend_from_slice(&chunk[..n]),
            _ => return,
        }
    };
    let Some(carried) = net.lock().carried.get(&port).cloned() else {
        return answer(&mut tls, 401).await;
    };
    let request = match gate::rewrite_request(&buf[..end], &carried.partition) {
        Ok(r) => r,
        Err(HeadError::TooLarge) => return answer(&mut tls, 431).await,
        Err(HeadError::Malformed) => return answer(&mut tls, 400).await,
    };
    // The entry: a ticket for this port becomes the forward token, in this port's cookie.
    if let Some(ticket) = entry_ticket(&request.target) {
        let token = net
            .lock()
            .gate
            .enter(ticket, port, now_ms(), carried.ends_ms);
        let Some(token) = token else {
            return answer(&mut tls, 401).await;
        };
        let head = format!(
            "HTTP/1.1 302 Found\r\nLocation: /\r\nSet-Cookie: {}={token}; HttpOnly; Secure; SameSite=Strict; Path=/\r\n\
Referrer-Policy: no-referrer\r\nCache-Control: no-store\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            carried.partition.token_cookie()
        );
        let _ = tls.write_all(head.as_bytes()).await;
        let _ = tls.shutdown().await;
        return;
    }
    let Some(token) = request.token.clone() else {
        return answer(&mut tls, 401).await;
    };
    if net.check(&token, port).is_none() {
        return answer(&mut tls, 401).await;
    }
    let link = match net
        .router
        .connect_preview(&carried.node, &carried.forward)
        .await
    {
        Ok(l) => l,
        Err(_) => return answer(&mut tls, 502).await,
    };
    let mut first = request.head;
    first.extend_from_slice(&buf[end..]);
    relay(
        net,
        port,
        &token,
        &carried,
        link,
        tls,
        first,
        request.upgrade,
    )
    .await;
}

#[derive(PartialEq, Eq)]
enum Phase {
    /// Waiting for the program's final answer head.
    Head,
    /// The final answer went back: its body follows, and nothing more goes from the browser.
    Body,
    /// A WebSocket after `101`: opaque bytes both ways.
    Upgraded,
}

#[allow(clippy::too_many_arguments)]
async fn relay<S: AsyncRead + AsyncWrite + Unpin + Send>(
    net: &Arc<PreviewNet>,
    port: u16,
    token: &str,
    carried: &Carried,
    mut link: crate::router::PreviewLink,
    tls: S,
    first: Vec<u8>,
    upgrade: bool,
) {
    let (mut rd, mut wr) = tokio::io::split(tls);
    let max_chunk = match link.limits.max_chunk_bytes {
        0 => 16_384,
        n => n.clamp(1_024, 65_536) as usize,
    };
    let window = match link.limits.window_bytes {
        0 => 262_144u64,
        n => u64::from(n),
    };
    let mut credit = window; // bytes the node allows us to send
    let mut owed = 0u64; // bytes from the node written to the browser and not yet credited back
    let mut pending = first; // bytes for the node, sent as credit allows
    let mut answer_head: Vec<u8> = Vec::new();
    let mut phase = Phase::Head;
    let mut browser_open = true;
    let mut epoch = net.epoch.subscribe();
    let mut recheck = tokio::time::interval(RECHECK);
    recheck.tick().await;
    let mut chunk = vec![0u8; max_chunk];
    let mut ended_by_node = false;
    'conn: loop {
        // What the browser sent, as far as the node's credit goes.
        while !pending.is_empty() && credit > 0 {
            let n = pending.len().min(max_chunk).min(credit as usize);
            let part: Vec<u8> = pending.drain(..n).collect();
            if !link.send(PREVIEW_DATA, part).await {
                break 'conn;
            }
            credit -= n as u64;
        }
        let may_read = browser_open && pending.is_empty() && credit > 0 && phase != Phase::Body;
        tokio::select! {
            r = rd.read(&mut chunk), if may_read => match r {
                Ok(0) | Err(_) => browser_open = false,
                Ok(n) => pending.extend_from_slice(&chunk[..n]),
            },
            f = link.recv() => {
                let Some(f) = f else { ended_by_node = true; break };
                match f.kind {
                    PREVIEW_DATA => {
                        owed += f.payload.len() as u64;
                        if phase == Phase::Head {
                            answer_head.extend_from_slice(&f.payload);
                            // Every head up to the final answer is read and rewritten; what follows the final head goes on as it is.
                            loop {
                                let end = match gate::head_end(&answer_head) {
                                    Ok(Some(end)) => end,
                                    Ok(None) => break,
                                    Err(_) => break 'conn,
                                };
                                let Ok((status, head)) = gate::rewrite_response(&answer_head[..end], &carried.partition, now_ms(), carried.ends_ms) else {
                                    break 'conn;
                                };
                                if wr.write_all(&head).await.is_err() {
                                    break 'conn;
                                }
                                answer_head.drain(..end);
                                if status == 101 && !upgrade {
                                    // A switch the browser never asked for: nothing of it goes on.
                                    break 'conn;
                                } else if status == 101 {
                                    phase = Phase::Upgraded;
                                } else if status >= 200 {
                                    // The final answer: nothing more from the browser reaches the node (point 8).
                                    phase = Phase::Body;
                                    pending.clear();
                                }
                                if phase != Phase::Head {
                                    let rest = std::mem::take(&mut answer_head);
                                    if !rest.is_empty() && wr.write_all(&rest).await.is_err() {
                                        break 'conn;
                                    }
                                    break;
                                }
                            }
                        } else if wr.write_all(&f.payload).await.is_err() {
                            break 'conn;
                        }
                        if owed >= window / 2 {
                            let grant = v1::PreviewCredit { bytes: u32::try_from(owed).unwrap_or(u32::MAX) };
                            if !link.send(PREVIEW_CREDIT, grant.encode_to_vec()).await {
                                break 'conn;
                            }
                            owed = 0;
                        }
                    }
                    PREVIEW_CREDIT => {
                        if let Ok(c) = v1::PreviewCredit::decode(f.payload.as_slice()) {
                            credit = credit.saturating_add(u64::from(c.bytes));
                        }
                    }
                    PREVIEW_END => { ended_by_node = true; break }
                    _ => {}
                }
            },
            _ = epoch.changed() => {
                if net.check(token, port).is_none() { break }
            },
            _ = recheck.tick() => {
                if net.check(token, port).is_none() { break }
            },
        }
        if !browser_open && phase == Phase::Upgraded && pending.is_empty() {
            break;
        }
    }
    if !ended_by_node {
        let end = v1::PreviewEnd {
            reason: v1::PreviewCloseReason::Closed as i32,
            ..Default::default()
        };
        let _ = link.send(PREVIEW_END, end.encode_to_vec()).await;
    }
    drop(link);
    let _ = wr.flush().await;
    let _ = wr.shutdown().await;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn the_host_header_names_the_port_the_window_came_through() {
        assert_eq!(host_port("hub.example:7443"), Some(7443));
        assert_eq!(host_port("hub.example"), Some(443));
        assert_eq!(host_port("[::1]:7443"), Some(7443));
        assert_eq!(host_port("[::1]"), Some(443));
        assert_eq!(host_port("10.0.0.2:x"), None);
        assert_eq!(host_port("hub.example:"), None);
    }

    #[test]
    fn only_the_entry_path_with_a_whole_ticket_is_an_entry() {
        let t = "a".repeat(64);
        assert_eq!(
            entry_ticket(&format!("{ENTER_PATH}?t={t}")),
            Some(t.as_str())
        );
        assert_eq!(
            entry_ticket(&format!("{ENTER_PATH}?x=1&t={t}")),
            Some(t.as_str())
        );
        assert_eq!(entry_ticket(&format!("/app{ENTER_PATH}?t={t}")), None);
        assert_eq!(
            entry_ticket(&format!("{ENTER_PATH}?t={}", "a".repeat(63))),
            None
        );
        assert_eq!(
            entry_ticket(&format!("{ENTER_PATH}?t={}", "z".repeat(64))),
            None
        );
        assert_eq!(entry_ticket("/"), None);
    }

    fn forward(creator: &str, ends_ms: u64) -> v1::PreviewForward {
        v1::PreviewForward {
            forward_id: "fwd-1".into(),
            port: 5173,
            creator: creator.into(),
            expires_at_ms: ends_ms,
            state: v1::PreviewState::Open as i32,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn only_the_opener_enters_and_only_through_the_hubs_own_port() {
        let net = Arc::new(PreviewNet::new(Router::new()));
        let devices = |d: &str| match d {
            "web-ada" => Some("ada".to_string()),
            "web-bo" => Some("bo".to_string()),
            _ => None,
        };
        let far = now_ms() + 3_600_000;
        // Not configured: no origin.
        net.changed("mac", &forward("web-ada", far));
        assert_eq!(
            net.enter("ada", devices, "mac", "fwd-1", "h:7443")
                .unwrap_err()
                .reason,
            "no_origin"
        );
        // Configured on a free port of this machine.
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let hub = probe.local_addr().unwrap();
        drop(probe);
        let slot = test_slot();
        net.configure(hub, slot, None);
        let host = format!("localhost:{}", hub.port());
        assert_eq!(
            net.enter("ada", devices, "mac", "fwd-2", &host)
                .unwrap_err()
                .code,
            "preview.unknown"
        );
        assert_eq!(
            net.enter("bo", devices, "mac", "fwd-1", &host)
                .unwrap_err()
                .code,
            "auth.forbidden"
        );
        assert_eq!(
            net.enter("ada", devices, "mac", "fwd-1", "proxy.example")
                .unwrap_err()
                .reason,
            "no_origin"
        );
        let (port, ticket) = net.enter("ada", devices, "mac", "fwd-1", &host).unwrap();
        assert!(port > hub.port() && port <= hub.port() + DEFAULT_POOL);
        assert_eq!(ticket.len(), 64);
        let (again, _) = net.enter("ada", devices, "mac", "fwd-1", &host).unwrap();
        assert_eq!(again, port, "one port per forward");
        // A forward opened on the node's control socket has no device: nobody enters it.
        net.changed(
            "mac",
            &v1::PreviewForward {
                forward_id: "fwd-3".into(),
                ..forward("owner", far)
            },
        );
        assert_eq!(
            net.enter("ada", devices, "mac", "fwd-3", &host)
                .unwrap_err()
                .code,
            "auth.forbidden"
        );
        // The forward ends: its port is free again, and its tickets are gone.
        let tok = net.lock().gate.enter(&ticket, port, now_ms(), far);
        let tok = tok.unwrap();
        assert!(net.check(&tok, port).is_some());
        let mut ended = forward("web-ada", far);
        ended.state = v1::PreviewState::Closed as i32;
        net.changed("mac", &ended);
        assert!(net.check(&tok, port).is_none());
        assert_eq!(
            net.enter("ada", devices, "mac", "fwd-1", &host)
                .unwrap_err()
                .code,
            "preview.unknown"
        );
    }

    fn test_slot() -> TlsSlot {
        test_ca_slot().1
    }

    fn test_ca_slot() -> (crate::ca::Ca, TlsSlot) {
        let ca = crate::ca::Ca::generate("test").unwrap();
        let (cert, key) = ca.issue_server(&["localhost".to_string()]).unwrap();
        let acceptor = crate::tls::acceptor_with(&cert, &key, ca.cert_pem(), true).unwrap();
        (ca, TlsSlot::new(acceptor, [0; 32]))
    }

    fn connector(trust: &crate::ca::Ca) -> tokio_rustls::TlsConnector {
        use rustls::pki_types::{pem::PemObject, CertificateDer};
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut roots = rustls::RootCertStore::empty();
        for c in CertificateDer::pem_slice_iter(trust.cert_pem().as_bytes()) {
            roots.add(c.unwrap()).unwrap();
        }
        let cfg = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
        tokio_rustls::TlsConnector::from(Arc::new(cfg))
    }

    /// One browser request on its own connection: what the hub answered, read until it closed the connection.
    async fn browse(tls: &tokio_rustls::TlsConnector, port: u16, request: &str) -> String {
        let tcp = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        let name = rustls::pki_types::ServerName::try_from("localhost").unwrap();
        let mut s = tls.connect(name, tcp).await.unwrap();
        s.write_all(request.as_bytes()).await.unwrap();
        let mut out = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut out)).await;
        String::from_utf8_lossy(&out).into_owned()
    }

    /// The node, as far as previews go: answers each `PreviewConnect`, hands what the hub sends to `program`, which says what goes
    /// back (`None`: close the stream once the answer went).
    fn fake_node(
        router: &Arc<Router>,
        program: impl Fn(&[u8]) -> Option<Vec<u8>> + Send + 'static,
    ) -> (
        crate::router::Registration,
        tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    ) {
        use crate::router::ToNode;
        let (reg, mut commands) =
            router.register_with_capabilities("mac", vec!["preview.v1".into()]);
        let mut previews = reg.preview_queue().unwrap();
        let (seen_tx, seen) = tokio::sync::mpsc::unbounded_channel();
        let r = Arc::clone(router);
        tokio::spawn(async move {
            let mut got: HashMap<u32, Vec<u8>> = HashMap::new();
            loop {
                tokio::select! {
                    c = commands.recv() => {
                        let Some(ToNode::Command { request, reply, .. }) = c else { continue };
                        let ok = v1::CommandOk { result: Some(v1::command_ok::Result::PreviewConnect(v1::PreviewConnectResult {
                            max_chunk_bytes: 16384, window_bytes: 262144, ..Default::default() })) };
                        let _ = reply.send(v1::CommandResponse { request_id: request.request_id, result: Some(v1::command_response::Result::Ok(ok)) });
                    }
                    p = previews.recv() => {
                        let Some(p) = p else { break };
                        let _ = seen_tx.send([&[p.kind], p.payload.as_slice()].concat());
                        if p.kind != PREVIEW_DATA { continue; }
                        let buf = got.entry(p.stream).or_default();
                        buf.extend_from_slice(&p.payload);
                        if gate::head_end(buf).ok().flatten().is_some() {
                            let request = std::mem::take(buf);
                            if let Some(answer) = program(&request) {
                                let upgraded = answer.starts_with(b"HTTP/1.1 101");
                                r.relay_frame("mac", p.stream, PREVIEW_DATA, answer);
                                // A program closes its connection after an answer that said `close`; the node then ends the stream.
                                if !upgraded {
                                    r.relay_frame("mac", p.stream, agora_protocol::frame::STREAM_CLOSE, Vec::new());
                                }
                            }
                        }
                    }
                }
            }
        });
        (reg, seen)
    }

    #[tokio::test]
    async fn a_preview_goes_through_its_own_origin_with_only_its_own_cookies() {
        let router = Router::new();
        let net = Arc::new(PreviewNet::new(Arc::clone(&router)));
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let hub = probe.local_addr().unwrap();
        drop(probe);
        let (ca, slot) = test_ca_slot();
        net.configure(hub, slot, None);
        let far = now_ms() + 3_600_000;
        net.changed("mac", &forward("web-ada", far));
        let (_node, mut seen) = fake_node(&router, |req| {
            let head = String::from_utf8_lossy(req);
            assert!(head.contains("Host: localhost:5173\r\n"), "{head}");
            assert!(
                !head.contains("agora_preview"),
                "the token never reaches the program: {head}"
            );
            assert!(head.ends_with("Connection: close\r\n\r\n"), "{head}");
            let cookie = if head.contains("Cookie: theme=dark\r\n") {
                "seen"
            } else {
                "none"
            };
            Some(format!("HTTP/1.1 200 OK\r\nConnection: keep-alive\r\nSet-Cookie: session=abc\r\nX-Cookie: {cookie}\r\nContent-Length: 2\r\n\r\nhi").into_bytes())
        });
        let devices = |d: &str| (d == "web-ada").then(|| "ada".to_string());
        let host = format!("localhost:{}", hub.port());
        let (port, ticket) = net.enter("ada", devices, "mac", "fwd-1", &host).unwrap();
        let tls = connector(&ca);
        // Without the cookie: refused, nothing reaches the node.
        let no = browse(&tls, port, "GET / HTTP/1.1\r\nHost: x\r\n\r\n").await;
        assert!(no.starts_with("HTTP/1.1 401"), "{no}");
        // The entry: the ticket becomes this port's cookie, once.
        let entered = browse(
            &tls,
            port,
            &format!("GET {ENTER_PATH}?t={ticket} HTTP/1.1\r\nHost: x\r\n\r\n"),
        )
        .await;
        assert!(entered.starts_with("HTTP/1.1 302"), "{entered}");
        assert!(entered.contains("Referrer-Policy: no-referrer"));
        let cookie_line = entered
            .lines()
            .find(|l| l.starts_with("Set-Cookie: "))
            .unwrap();
        assert!(
            cookie_line.contains("HttpOnly; Secure; SameSite=Strict; Path=/"),
            "{cookie_line}"
        );
        let token = cookie_line
            .trim_start_matches("Set-Cookie: ")
            .split(';')
            .next()
            .unwrap()
            .to_string();
        assert!(token.starts_with(&format!("agora_preview_{port}=")));
        let again = browse(
            &tls,
            port,
            &format!("GET {ENTER_PATH}?t={ticket} HTTP/1.1\r\nHost: x\r\n\r\n"),
        )
        .await;
        assert!(again.starts_with("HTTP/1.1 401"), "single use: {again}");
        // A request with the cookie: rewritten on the way in and out.
        let prefix = net.lock().carried.get(&port).unwrap().partition.prefix();
        let page = browse(&tls, port, &format!(
            "GET /app HTTP/1.1\r\nHost: x\r\nCookie: {token}; {prefix}theme=dark; agora_preview_9999=other; p1-x_y=z\r\n\r\n")).await;
        assert!(page.starts_with("HTTP/1.1 200 OK\r\n"), "{page}");
        assert!(
            page.contains("X-Cookie: seen"),
            "its own cookie went, unprefixed: {page}"
        );
        assert!(
            page.contains(&format!("Set-Cookie: {prefix}session=abc; Max-Age=")),
            "{page}"
        );
        assert!(
            page.contains("Connection: close\r\n") && !page.contains("keep-alive"),
            "{page}"
        );
        assert!(page.ends_with("\r\n\r\nhi"), "{page}");
        // The program closed after its answer: the browser's connection ended at once, and nothing more went to the node.
        let at = std::time::Instant::now();
        let page2 = browse(
            &tls,
            port,
            &format!("GET /two HTTP/1.1\r\nHost: x\r\nCookie: {token}\r\n\r\n"),
        )
        .await;
        assert!(
            page2.starts_with("HTTP/1.1 200") && at.elapsed() < Duration::from_secs(2),
            "{page2}"
        );
        let mut kinds = Vec::new();
        while let Ok(Some(f)) = tokio::time::timeout(Duration::from_millis(300), seen.recv()).await
        {
            kinds.push(f[0]);
        }
        assert!(
            kinds.iter().all(|k| *k == PREVIEW_DATA),
            "only the requests went: {kinds:?}"
        );
    }

    #[tokio::test]
    async fn a_websocket_ends_when_its_person_signs_out() {
        let router = Router::new();
        let net = Arc::new(PreviewNet::new(Arc::clone(&router)));
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let hub = probe.local_addr().unwrap();
        drop(probe);
        let (ca, slot) = test_ca_slot();
        net.configure(hub, slot, None);
        net.changed("mac", &forward("web-ada", now_ms() + 3_600_000));
        let (_node, _seen) = fake_node(&router, |_| {
            Some(b"HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n".to_vec())
        });
        let devices = |d: &str| (d == "web-ada").then(|| "ada".to_string());
        let (port, ticket) = net
            .enter(
                "ada",
                devices,
                "mac",
                "fwd-1",
                &format!("localhost:{}", hub.port()),
            )
            .unwrap();
        let tls = connector(&ca);
        let entered = browse(
            &tls,
            port,
            &format!("GET {ENTER_PATH}?t={ticket} HTTP/1.1\r\nHost: x\r\n\r\n"),
        )
        .await;
        let token = entered
            .lines()
            .find(|l| l.starts_with("Set-Cookie: "))
            .unwrap()
            .trim_start_matches("Set-Cookie: ")
            .split(';')
            .next()
            .unwrap()
            .to_string();
        let tcp = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        let mut s = tls
            .connect(
                rustls::pki_types::ServerName::try_from("localhost").unwrap(),
                tcp,
            )
            .await
            .unwrap();
        s.write_all(format!("GET /ws HTTP/1.1\r\nHost: x\r\nCookie: {token}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n").as_bytes()).await.unwrap();
        let mut head = vec![0u8; 4096];
        let n = s.read(&mut head).await.unwrap();
        assert!(
            String::from_utf8_lossy(&head[..n]).starts_with("HTTP/1.1 101"),
            "upgraded"
        );
        net.end_user("ada");
        let mut rest = Vec::new();
        let ended = tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut rest)).await;
        assert!(ended.is_ok(), "the upgraded connection ends at once");
    }
}
