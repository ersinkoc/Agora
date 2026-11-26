//! How the desktop app reaches a hub (ADR-0048). A webview trusts the system's certificate store, and the hub's certificate comes
//! from its own CA (ADR-0015), so the shell cannot simply open `https://<hub>/`. Instead it runs a tunnel on the loopback interface:
//! the webview loads `http://127.0.0.1:<port>/`, and every connection to that port is carried to the hub over TLS that trusts **only
//! the CA the user confirmed**, never the system store. The tunnel copies bytes and reads none of them: the page, its WebSocket
//! and its sign-in all pass through unchanged, and the tunnel holds no credential of its own.
//!
//! One exception to "reads none": before a connection is carried, the tunnel reads the head of its first request and checks that
//! the request is addressed to the tunnel itself (`Host: 127.0.0.1:<port>`, or `localhost:<port>`). Without that check, any web
//! page in any browser on this machine could reach the hub through the tunnel by DNS rebinding: a name the page controls is pointed
//! at 127.0.0.1, the browser then treats the tunnel as that page's own origin, and since the tunnel strips the TLS that would
//! otherwise expose the wrong name, nothing else stops it. A request that carries an `Origin` (a WebSocket upgrade, a POST) must
//! name the tunnel's own origin too. On the hub's own machine the hub sees the tunnel as loopback, which
//! signs in without a code (threat model T22).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName};
use tokio::io::{copy_bidirectional, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio_rustls::TlsConnector;

/// The most of a request's head the tunnel reads before it decides. A browser's request head is a few hundred bytes to a few KiB
/// (cookies included); a longer one is refused rather than buffered.
const MAX_HEAD: usize = 16 * 1024;

/// What a request addressed to another host gets: no body and no text, since the webview shows its own error page.
const MISDIRECTED: &[u8] =
    b"HTTP/1.1 421 Misdirected Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

/// How long the tunnel waits after a failed accept before it accepts again. The wait keeps a listener that stays ready (a
/// connection still queued while no file handle is free) from turning the loop into a busy one.
const ACCEPT_RETRY: Duration = Duration::from_millis(100);

/// Accepted connections per tunnel, including those still reading a request head or connecting to the hub.
const MAX_CONNECTIONS: usize = 64;

/// Reads from `s` until the end of the first request's head (the blank line), at most [`MAX_HEAD`] bytes. Returns everything read,
/// which can run past the head (the start of a body, a pipelined request), and the head's length; `None` when the connection
/// closes, fails, or sends no head within the limit.
async fn read_head(s: &mut TcpStream) -> Option<(Vec<u8>, usize)> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 2048];
    loop {
        let n = s.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        // Search only the new bytes and the three before them, which is where a blank line split across reads can start.
        let from = buf.len().saturating_sub(3);
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf[from..].windows(4).position(|w| w == b"\r\n\r\n") {
            let end = from + i + 4;
            return (end <= MAX_HEAD).then_some((buf, end));
        }
        if buf.len() > MAX_HEAD {
            return None;
        }
    }
}

/// Whether a request head is addressed to the tunnel on `port`: exactly one `Host` field, whose value is `127.0.0.1:<port>` or
/// `localhost:<port>` (the port may be left out only when it is 80, as browsers do), and a request target in origin form (`/…`, or
/// `*`). Anything else (another name, another port, `localhost.`, no `Host`, two of them, a head that is not UTF-8) is refused. An
/// absolute target (`GET http://other/ HTTP/1.1`) is refused too, since a server takes the host from it rather than from `Host`
/// (RFC 9112 3.2.2); a browser sends that form only to a proxy.
///
/// Only the first request on a connection is checked. A browser keeps a connection for the origin it opened it for, and does not
/// pipeline, so a page on another name never shares the webview's connection; a local program could send another request with
/// another `Host` after the first, but such a program can send the right one anyway, so this check is about browsers only.
fn addressed_to_tunnel(head: &[u8], port: u16) -> bool {
    let Ok(text) = std::str::from_utf8(head) else {
        return false;
    };
    let target = text
        .split("\r\n")
        .next()
        .and_then(|line| line.split(' ').nth(1));
    if !matches!(target, Some(t) if t.starts_with('/') || t == "*") {
        return false;
    }
    // The fields follow the request line and end at the blank line. A field name has no space before its colon (RFC 9112 5.1),
    // and a line starting with a space continues the previous field, so neither can count as `Host`.
    let field = |wanted: &'static str| {
        text.split("\r\n")
            .skip(1)
            .take_while(|l| !l.is_empty())
            .filter_map(move |l| {
                let (name, value) = l.split_once(':')?;
                name.eq_ignore_ascii_case(wanted).then(|| value.trim())
            })
    };
    let mut hosts = field("host");
    let (Some(host), None) = (hosts.next(), hosts.next()) else {
        return false;
    };
    let (name, given_port) = match host.rsplit_once(':') {
        Some((name, p)) => (name, p.parse::<u16>().ok()),
        None => (host, Some(80)),
    };
    if !(given_port == Some(port) && is_own_name(name)) {
        return false;
    }
    // A browser adds `Origin` to a WebSocket upgrade, a POST and any cross-origin request. When one is there it must be this
    // tunnel's own origin (`http://127.0.0.1:<port>` or `http://localhost:<port>`): a page that reached the tunnel under its own
    // name has another origin, and `null` (a sandboxed frame, a `file:` page) is refused too. No `Origin` is fine (a navigation,
    // a same-origin GET, a program that is not a browser).
    let mut origins = field("origin");
    match (origins.next(), origins.next()) {
        (None, _) => true,
        (Some(o), None) => o.strip_prefix("http://").is_some_and(|h| {
            h.rsplit_once(':')
                .is_some_and(|(n, p)| p.parse::<u16>().ok() == Some(port) && is_own_name(n))
                || (port == 80 && is_own_name(h))
        }),
        (Some(_), Some(_)) => false,
    }
}

/// Whether `name` is one of the two names the tunnel answers to.
fn is_own_name(name: &str) -> bool {
    name == "127.0.0.1" || name.eq_ignore_ascii_case("localhost")
}

/// Connects to `hub` (`host:port`) the way `agora_protocol::dial::connect` does, kept here so the desktop shell does not link the
/// protocol crate: a loopback name (`localhost`) tries IPv4 first and IPv6 only after it, because on Windows a refused `::1` costs
/// about two seconds before the next address; any other name keeps the resolver's order.
async fn dial(hub: &str) -> std::io::Result<tokio::net::TcpStream> {
    let mut addrs: Vec<SocketAddr> = tokio::net::lookup_host(hub).await?.collect();
    if addrs.iter().all(|a| a.ip().is_loopback()) {
        addrs.sort_by_key(SocketAddr::is_ipv6);
    }
    let mut last = std::io::Error::new(std::io::ErrorKind::NotFound, "no address");
    for addr in addrs {
        match tokio::net::TcpStream::connect(addr).await {
            Ok(s) => return Ok(s),
            Err(e) => last = e,
        }
    }
    Err(last)
}

#[derive(Debug, PartialEq, Eq)]
pub enum TunnelError {
    /// The CA is not a readable PEM certificate.
    BadCa,
    /// The hub address is not `host:port`.
    BadAddress,
    /// The loopback port could not be opened.
    Listen,
    /// Nothing answered at the hub address in time.
    Unreachable,
    /// The hub answered, but its certificate is not signed by the pinned CA for this address (or the TLS handshake failed).
    Untrusted,
}

/// The one certificate in the CA text. A hub's CA is a single certificate (ADR-0015); text with more than one is refused, since the
/// fingerprint the user compares names one certificate, and the tunnel must trust exactly the one it names.
fn the_ca(ca_pem: &str) -> Result<CertificateDer<'static>, TunnelError> {
    let mut certs = CertificateDer::pem_slice_iter(ca_pem.as_bytes());
    let Some(Ok(der)) = certs.next() else {
        return Err(TunnelError::BadCa);
    };
    // The same certificate again (a file saved twice over) still names one CA; another one, or an unreadable one, is refused.
    for other in certs {
        match other {
            Ok(o) if o == der => {}
            _ => return Err(TunnelError::BadCa),
        }
    }
    Ok(der)
}

/// The SHA-256 of the CA's certificate, as upper-case hex pairs separated by colons: what the user compares on first use.
pub fn fingerprint(ca_pem: &str) -> Result<String, TunnelError> {
    let der = the_ca(ca_pem)?;
    let d = ring::digest::digest(&ring::digest::SHA256, der.as_ref());
    Ok(d.as_ref()
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":"))
}

/// The name the hub's certificate must carry: its host name, or its IP number. A hub started without `--name` has a certificate for
/// `localhost` only; for an IP address, [`PinnedVerifier`] also accepts that one.
fn server_name(addr: &str) -> Result<ServerName<'static>, TunnelError> {
    let (host, port) = addr.rsplit_once(':').ok_or(TunnelError::BadAddress)?;
    if host.is_empty() || port.parse::<u16>().is_err() {
        return Err(TunnelError::BadAddress);
    }
    let host = host.trim_matches(['[', ']']);
    ServerName::try_from(host.to_string()).map_err(|_| TunnelError::BadAddress)
}

/// Checks the hub's certificate against the pinned CA only. For an address given as an IP number, a certificate that names that IP
/// (a hub started with `--name <ip>`) or `localhost` (a hub started without `--name`) is accepted; either way it must chain to the
/// pinned CA, which signs nothing but this hub's certificates.
#[derive(Debug)]
struct PinnedVerifier(Arc<rustls::client::WebPkiServerVerifier>);

impl rustls::client::danger::ServerCertVerifier for PinnedVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp: &[u8],
        now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        let first = self
            .0
            .verify_server_cert(end_entity, intermediates, server_name, ocsp, now);
        match (&first, server_name) {
            (Err(_), ServerName::IpAddress(_)) => {
                let localhost = ServerName::try_from("localhost")
                    .map_err(|_| rustls::Error::General("localhost is a valid name".to_string()))?;
                self.0
                    .verify_server_cert(end_entity, intermediates, &localhost, ocsp, now)
                    .or(first)
            }
            _ => first,
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.supported_verify_schemes()
    }
}

fn connector(ca_pem: &str) -> Result<TlsConnector, TunnelError> {
    // Exactly the certificate the fingerprint names, never a second one carried along in the same text.
    let mut roots = rustls::RootCertStore::empty();
    roots.add(the_ca(ca_pem)?).map_err(|_| TunnelError::BadCa)?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = rustls::client::WebPkiServerVerifier::builder_with_provider(
        Arc::new(roots),
        provider.clone(),
    )
    .build()
    .map_err(|_| TunnelError::BadCa)?;
    let cfg = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|_| TunnelError::BadCa)?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinnedVerifier(verifier)))
        .with_no_client_auth();
    Ok(TlsConnector::from(Arc::new(cfg)))
}

/// A running tunnel. Dropping it stops the listener and cancels its active connections.
pub struct Tunnel {
    /// The loopback address the webview loads.
    pub local: SocketAddr,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Opens a tunnel on `127.0.0.1:port` (0 picks a free one) to the hub at `hub` (`host:port`), trusting only `ca_pem`. Must be called
/// inside a Tokio runtime.
pub async fn open(hub: &str, ca_pem: &str, port: u16) -> Result<Tunnel, TunnelError> {
    open_with_limit(hub, ca_pem, port, MAX_CONNECTIONS).await
}

async fn open_with_limit(
    hub: &str,
    ca_pem: &str,
    port: u16,
    connection_limit: usize,
) -> Result<Tunnel, TunnelError> {
    let name = server_name(hub)?;
    let tls = connector(ca_pem)?;
    // Only the loopback interface: the tunnel is for this machine's webview, never for the network. Bound exclusively, so no other
    // program can bind the same port beside it while it runs (threat model T27; Windows lets a `SO_REUSEADDR` binder share a port
    // that tokio bound).
    let listener = agora_platform::bind_loopback_exclusive(port)
        .and_then(|l| {
            l.set_nonblocking(true)?;
            TcpListener::from_std(l)
        })
        .map_err(|_| TunnelError::Listen)?;
    let local = listener.local_addr().map_err(|_| TunnelError::Listen)?;
    let hub = hub.to_string();
    let task = tokio::spawn(async move {
        // The listener owns the connections. Aborting it drops the set and cancels both ends of each connection.
        let mut connections = JoinSet::new();
        loop {
            while connections.try_join_next().is_some() {}
            if connections.len() == connection_limit {
                // Leave pending connections in the OS listen queue until a slot becomes available.
                let _ = connections.join_next().await;
            }
            // A failed accept passes (no free file handle while other connections hold them, a connection reset before it was
            // taken), so it ends neither the loop nor the port: the shell still holds the tunnel as open, and a closed port would
            // turn every later page load away and leave the fixed port free for another program to take (threat model T27).
            let mut inbound = match listener.accept().await {
                Ok((s, _)) => s,
                Err(_) => {
                    tokio::time::sleep(ACCEPT_RETRY).await;
                    continue;
                }
            };
            let (hub, name, tls) = (hub.clone(), name.clone(), tls.clone());
            connections.spawn(async move {
                // The first request must be addressed to this tunnel (DNS rebinding, see the crate docs), checked before the hub is
                // dialed, so a refused page never opens a connection to it. A head that does not arrive in time ends the connection.
                let Ok(Some((first, head_len))) =
                    tokio::time::timeout(Duration::from_secs(10), read_head(&mut inbound)).await
                else {
                    return;
                };
                if !addressed_to_tunnel(&first[..head_len], local.port()) {
                    let _ = inbound.write_all(MISDIRECTED).await;
                    let _ = inbound.shutdown().await;
                    return;
                }
                // A hub that is down, slow, or that does not present a certificate from the pinned CA ends this one connection and
                // nothing else; the webview shows its own error page.
                let Ok(Ok(tcp)) = tokio::time::timeout(Duration::from_secs(10), dial(&hub)).await
                else {
                    return;
                };
                let Ok(Ok(mut outbound)) =
                    tokio::time::timeout(Duration::from_secs(10), tls.connect(name, tcp)).await
                else {
                    return;
                };
                // The bytes already read go first, unchanged; from then on the tunnel copies without reading.
                if outbound.write_all(&first).await.is_err() {
                    return;
                }
                let _ = copy_bidirectional(&mut inbound, &mut outbound).await;
            });
        }
    });
    Ok(Tunnel { local, task })
}

/// One TLS handshake with the hub at `hub`, trusting only `ca_pem`, then the connection is closed. The shell calls it before it
/// remembers a hub, so a wrong certificate or address is reported on the connect screen instead of being saved: `open` alone
/// cannot tell, because it checks each connection only when the page makes one.
pub async fn check(hub: &str, ca_pem: &str) -> Result<(), TunnelError> {
    let name = server_name(hub)?;
    let tls = connector(ca_pem)?;
    let tcp = match tokio::time::timeout(Duration::from_secs(10), dial(hub)).await {
        Ok(Ok(tcp)) => tcp,
        _ => return Err(TunnelError::Unreachable),
    };
    match tokio::time::timeout(Duration::from_secs(10), tls.connect(name, tcp)).await {
        Ok(Ok(mut s)) => {
            let _ = tokio::io::AsyncWriteExt::shutdown(&mut s).await;
            Ok(())
        }
        Ok(Err(_)) => Err(TunnelError::Untrusted),
        Err(_) => Err(TunnelError::Unreachable),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use agora_hub::ca::Ca;

    #[tokio::test]
    async fn localhost_reaches_an_ipv4_hub_first() {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        let s = dial(&format!("localhost:{port}")).await.unwrap();
        assert!(s.peer_addr().unwrap().is_ipv4());
    }

    /// A TLS server that answers a request with `hello <its request line>`, signed by `ca`, for `localhost`.
    async fn hub(ca: &Ca) -> SocketAddr {
        hub_named(ca, "localhost").await
    }

    /// The same, with a certificate for `name` only.
    async fn hub_named(ca: &Ca, name: &str) -> SocketAddr {
        let (cert, key) = ca.issue_server(&[name.to_string()]).unwrap();
        let acceptor = agora_hub::tls::acceptor_with(&cert, &key, ca.cert_pem(), true).unwrap();
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let (s, _) = l.accept().await.unwrap();
                let a = acceptor.clone();
                tokio::spawn(async move {
                    let Ok(mut s) = a.accept(s).await else { return };
                    let mut buf = [0u8; 1024];
                    let n = s.read(&mut buf).await.unwrap_or(0);
                    let got = String::from_utf8_lossy(&buf[..n]);
                    let line = got.split("\r\n").next().unwrap_or_default();
                    let _ = s.write_all(format!("hello {line}").as_bytes()).await;
                    let _ = s.shutdown().await;
                });
            }
        });
        addr
    }

    /// Sends `GET /<path>` through the tunnel the way the webview does (`Host: 127.0.0.1:<port>`) and returns the answer.
    async fn ask(local: SocketAddr, path: &str) -> String {
        ask_as(local, &local.to_string(), path).await
    }

    /// The same with the given `Host`, written in one piece.
    async fn ask_as(local: SocketAddr, host: &str, path: &str) -> String {
        send(
            local,
            &[&format!("GET /{path} HTTP/1.1\r\nHost: {host}\r\n\r\n")],
        )
        .await
    }

    /// Writes `parts` to the tunnel one after another (a head can arrive in pieces) and returns everything it answers.
    async fn send(local: SocketAddr, parts: &[&str]) -> String {
        let mut c = TcpStream::connect(local).await.unwrap();
        for p in parts {
            c.write_all(p.as_bytes()).await.unwrap();
            c.flush().await.unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let mut out = String::new();
        let _ = tokio::time::timeout(Duration::from_secs(5), c.read_to_string(&mut out)).await;
        out
    }

    const PAGE: &str = "hello GET /page HTTP/1.1";
    const REFUSED: &str =
        "HTTP/1.1 421 Misdirected Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

    #[tokio::test]
    async fn a_page_on_another_name_is_refused_before_the_hub_is_dialed() {
        let ca = Ca::generate("hub CA").unwrap();
        // A hub that counts its connections, so a refusal can be shown to reach nothing.
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let hub_addr = l.local_addr().unwrap();
        let reached = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = reached.clone();
        tokio::spawn(async move {
            while let Ok((s, _)) = l.accept().await {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                drop(s);
            }
        });
        let t = open(&hub_addr.to_string(), ca.cert_pem(), 0).await.unwrap();
        let port = t.local.port();
        // DNS rebinding: the browser sends the attacker's name with the tunnel's port.
        for host in [
            format!("rebind.example:{port}"),
            format!("127.0.0.1:{}", port.wrapping_add(1)),
            format!("127.0.0.2:{port}"),
            format!("[::1]:{port}"),
            "127.0.0.1".to_string(),
        ] {
            assert_eq!(ask_as(t.local, &host, "page").await, REFUSED, "{host}");
        }
        // No Host at all, and two of them.
        assert_eq!(send(t.local, &["GET / HTTP/1.1\r\n\r\n"]).await, REFUSED);
        let two = format!(
            "GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nHost: rebind.example:{port}\r\n\r\n"
        );
        assert_eq!(send(t.local, &[&two]).await, REFUSED);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(reached.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn the_webviews_own_address_gets_through_even_when_its_head_arrives_in_pieces() {
        let ca = Ca::generate("hub CA").unwrap();
        let addr = hub(&ca).await;
        let t = open(&addr.to_string(), ca.cert_pem(), 0).await.unwrap();
        let port = t.local.port();
        assert_eq!(
            ask_as(t.local, &format!("localhost:{port}"), "page").await,
            PAGE
        );
        assert_eq!(
            ask_as(t.local, &format!("LocalHost:{port}"), "page").await,
            PAGE
        );
        // The blank line split across writes, and the field name in another case.
        let parts = [
            "GET /page HTTP/1.1\r\nhost: ",
            &format!("127.0.0.1:{port}\r"),
            "\n\r",
            "\n",
        ];
        assert_eq!(send(t.local, &parts).await, PAGE);
    }

    #[tokio::test]
    async fn the_tunnel_admits_a_waiting_connection_when_a_slot_opens() {
        let ca = Ca::generate("hub CA").unwrap();
        let (cert, key) = ca.issue_server(&["localhost".to_string()]).unwrap();
        let acceptor = agora_hub::tls::acceptor_with(&cert, &key, ca.cert_pem(), true).unwrap();
        let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = upstream.local_addr().unwrap();
        let t = open_with_limit(&addr.to_string(), ca.cert_pem(), 0, 1)
            .await
            .unwrap();
        let request = format!("GET /page HTTP/1.1\r\nHost: {}\r\n\r\n", t.local);

        let mut first = TcpStream::connect(t.local).await.unwrap();
        first.write_all(request.as_bytes()).await.unwrap();
        let (socket, _) = tokio::time::timeout(Duration::from_secs(5), upstream.accept())
            .await
            .unwrap()
            .unwrap();
        let mut first_hub = acceptor.accept(socket).await.unwrap();
        let mut received = vec![0; request.len()];
        first_hub.read_exact(&mut received).await.unwrap();
        assert_eq!(received, request.as_bytes());

        let mut second = TcpStream::connect(t.local).await.unwrap();
        second.write_all(request.as_bytes()).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), upstream.accept())
                .await
                .is_err(),
            "the waiting connection reached the hub while the first held the only slot"
        );

        first.shutdown().await.unwrap();
        first_hub.shutdown().await.unwrap();
        let (socket, _) = tokio::time::timeout(Duration::from_secs(5), upstream.accept())
            .await
            .unwrap()
            .unwrap();
        let mut second_hub = acceptor.accept(socket).await.unwrap();
        second_hub.read_exact(&mut received).await.unwrap();
        assert_eq!(received, request.as_bytes());
    }

    /// The page's WebSocket: after the hub answers the upgrade, bytes flow both ways while the tunnel is open.
    #[tokio::test]
    async fn a_websocket_carries_bytes_both_ways_and_closes_when_its_tunnel_drops() {
        let ca = Ca::generate("hub CA").unwrap();
        let (cert, key) = ca.issue_server(&["localhost".to_string()]).unwrap();
        let acceptor = agora_hub::tls::acceptor_with(&cert, &key, ca.cert_pem(), true).unwrap();
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        let hub_task = tokio::spawn(async move {
            let (s, _) = l.accept().await.unwrap();
            let mut s = acceptor.accept(s).await.unwrap();
            // Read the upgrade head, answer 101, then echo every frame back with a prefix.
            let mut head = Vec::new();
            let mut b = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                s.read_exact(&mut b).await.unwrap();
                head.push(b[0]);
            }
            assert!(String::from_utf8_lossy(&head).contains("Upgrade: websocket"));
            s.write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n")
                .await
                .unwrap();
            let mut buf = [0u8; 64];
            loop {
                let n = s.read(&mut buf).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                let mut out = b"echo:".to_vec();
                out.extend_from_slice(&buf[..n]);
                s.write_all(&out).await.unwrap();
            }
        });
        let t = open(&addr.to_string(), ca.cert_pem(), 0).await.unwrap();
        let mut c = TcpStream::connect(t.local).await.unwrap();
        let up = format!("GET /ws HTTP/1.1\r\nHost: {}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n", t.local);
        c.write_all(up.as_bytes()).await.unwrap();
        let mut got = Vec::new();
        let mut b = [0u8; 1];
        while !got.ends_with(b"\r\n\r\n") {
            c.read_exact(&mut b).await.unwrap();
            got.push(b[0]);
        }
        assert!(got.starts_with(b"HTTP/1.1 101 "));
        for frame in ["one", "two"] {
            c.write_all(frame.as_bytes()).await.unwrap();
            let mut buf = [0u8; 16];
            let n = tokio::time::timeout(Duration::from_secs(5), c.read(&mut buf))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(&buf[..n], format!("echo:{frame}").as_bytes());
        }
        drop(t);
        let end = tokio::time::timeout(Duration::from_secs(5), c.read(&mut b))
            .await
            .expect("dropping the tunnel should close the webview socket");
        match end {
            Ok(n) => assert_eq!(n, 0),
            Err(error) => assert!(matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
            )),
        }
        tokio::time::timeout(Duration::from_secs(5), hub_task)
            .await
            .expect("dropping the tunnel should also close its hub socket")
            .unwrap();
    }

    #[tokio::test]
    async fn a_head_too_long_or_never_finished_gets_nothing() {
        let ca = Ca::generate("hub CA").unwrap();
        let addr = hub(&ca).await;
        let t = open(&addr.to_string(), ca.cert_pem(), 0).await.unwrap();
        let long = format!(
            "GET / HTTP/1.1\r\nX-Pad: {}\r\nHost: {}\r\n\r\n",
            "a".repeat(MAX_HEAD),
            t.local
        );
        assert_eq!(send(t.local, &[&long]).await, "");
        // A connection that closes its sending half before the blank line.
        let mut c = TcpStream::connect(t.local).await.unwrap();
        c.write_all(b"GET / HTTP/1.1\r\nHost: ").await.unwrap();
        c.shutdown().await.unwrap();
        let mut out = String::new();
        let _ = tokio::time::timeout(Duration::from_secs(5), c.read_to_string(&mut out)).await;
        assert_eq!(out, "");
    }

    #[test]
    fn only_a_single_host_naming_the_tunnel_counts() {
        let ok = |h: &str| addressed_to_tunnel(h.as_bytes(), 7000);
        assert!(ok("GET / HTTP/1.1\r\nHost: 127.0.0.1:7000\r\n\r\n"));
        assert!(ok(
            "GET / HTTP/1.1\r\nUser-Agent: x\r\nHOST:localhost:7000 \r\n\r\n"
        ));
        assert!(!ok("GET / HTTP/1.1\r\nHost: rebind.example:7000\r\n\r\n"));
        assert!(!ok("GET / HTTP/1.1\r\nHost: 127.0.0.1:7001\r\n\r\n"));
        assert!(!ok("GET / HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n"));
        assert!(!ok("GET / HTTP/1.1\r\nHost: 127.0.0.1:\r\n\r\n"));
        // A space before the colon, or a continuation line, is not a Host field.
        assert!(!ok("GET / HTTP/1.1\r\nHost : 127.0.0.1:7000\r\n\r\n"));
        assert!(!ok(
            "GET / HTTP/1.1\r\nX-A: b\r\n Host: 127.0.0.1:7000\r\n\r\n"
        ));
        // A Host in the body (after the blank line) does not count; the request line is not a field.
        assert!(!ok(
            "GET / HTTP/1.1\r\nX-A: b\r\n\r\nHost: 127.0.0.1:7000\r\n"
        ));
        assert!(!ok("Host: 127.0.0.1:7000\r\n\r\n"));
        assert!(!addressed_to_tunnel(
            b"GET / HTTP/1.1\r\nHost: 127.0.0.1:7000\xff\r\n\r\n",
            7000
        ));
        // A trailing dot is another name to a browser (another origin), and an absolute or authority-form target overrides `Host`.
        assert!(!ok("GET / HTTP/1.1\r\nHost: localhost.:7000\r\n\r\n"));
        assert!(!ok(
            "GET http://rebind.example:7000/ HTTP/1.1\r\nHost: 127.0.0.1:7000\r\n\r\n"
        ));
        assert!(!ok(
            "CONNECT rebind.example:443 HTTP/1.1\r\nHost: 127.0.0.1:7000\r\n\r\n"
        ));
        assert!(!ok("GET\r\nHost: 127.0.0.1:7000\r\n\r\n"));
        assert!(ok("OPTIONS * HTTP/1.1\r\nHost: 127.0.0.1:7000\r\n\r\n"));
        // Port 80 may be left out, as browsers do.
        assert!(addressed_to_tunnel(
            b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n",
            80
        ));
    }

    /// DNS rebinding and cross-site requests: an `Origin` that is not the tunnel's own is refused, a missing one is not.
    #[test]
    fn an_origin_must_be_the_tunnels_own_when_there_is_one() {
        let ok = |o: &str| {
            addressed_to_tunnel(
                format!("GET /ws HTTP/1.1\r\nHost: 127.0.0.1:7000\r\n{o}\r\n").as_bytes(),
                7000,
            )
        };
        assert!(ok(""));
        assert!(ok("Origin: http://127.0.0.1:7000\r\n"));
        assert!(ok("origin: http://LocalHost:7000\r\n"));
        assert!(!ok("Origin: http://evil.example:7000\r\n"));
        assert!(!ok("Origin: https://127.0.0.1:7000\r\n"));
        assert!(!ok("Origin: http://127.0.0.1:7001\r\n"));
        assert!(!ok("Origin: http://127.0.0.1\r\n"));
        assert!(!ok("Origin: http://127.0.0.1:7000.evil.example\r\n"));
        assert!(!ok("Origin: null\r\n"));
        assert!(!ok("Origin:\r\n"));
        assert!(!ok(
            "Origin: http://127.0.0.1:7000\r\nOrigin: http://evil.example:7000\r\n"
        ));
    }

    #[tokio::test]
    async fn the_page_reaches_the_hub_through_the_pinned_tunnel_and_only_on_loopback() {
        let ca = Ca::generate("hub CA").unwrap();
        let addr = hub(&ca).await;
        let t = open(&addr.to_string(), ca.cert_pem(), 0).await.unwrap();
        assert!(t.local.ip().is_loopback());
        assert_eq!(ask(t.local, "page").await, PAGE);
        // Dropping the tunnel closes the port.
        let local = t.local;
        drop(t);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(TcpStream::connect(local).await.is_err());
    }

    #[tokio::test]
    async fn a_hub_whose_certificate_is_not_from_the_pinned_ca_gets_nothing_through() {
        let real = Ca::generate("hub CA").unwrap();
        let impostor = Ca::generate("other CA").unwrap();
        let addr = hub(&impostor).await;
        // The user pinned `real`; the server at the address is signed by someone else.
        let t = open(&addr.to_string(), real.cert_pem(), 0).await.unwrap();
        assert_eq!(ask(t.local, "page").await, "");
    }

    #[tokio::test]
    async fn check_accepts_the_pinned_hub_and_names_what_is_wrong_otherwise() {
        let real = Ca::generate("hub CA").unwrap();
        let impostor = Ca::generate("other CA").unwrap();
        let good = hub(&real).await.to_string();
        let bad = hub(&impostor).await.to_string();
        assert_eq!(check(&good, real.cert_pem()).await, Ok(()));
        assert_eq!(
            check(&bad, real.cert_pem()).await,
            Err(TunnelError::Untrusted)
        );
        // A port nobody listens on: bind one, note it, close it.
        let closed = {
            let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
            l.local_addr().unwrap().to_string()
        };
        assert_eq!(
            check(&closed, real.cert_pem()).await,
            Err(TunnelError::Unreachable)
        );
        assert_eq!(check(&good, "not a pem").await, Err(TunnelError::BadCa));
    }

    #[tokio::test]
    async fn an_ip_address_accepts_a_certificate_for_that_ip_or_for_localhost_and_nothing_else() {
        let ca = Ca::generate("hub CA").unwrap();
        // A hub started with `--name 127.0.0.1` (a LAN hub names its IP) and one started without `--name`.
        let by_ip = hub_named(&ca, "127.0.0.1").await.to_string();
        let by_localhost = hub(&ca).await.to_string();
        assert_eq!(check(&by_ip, ca.cert_pem()).await, Ok(()));
        assert_eq!(check(&by_localhost, ca.cert_pem()).await, Ok(()));
        let t = open(&by_ip, ca.cert_pem(), 0).await.unwrap();
        assert_eq!(ask(t.local, "page").await, PAGE);
        // A certificate for another name, even from the pinned CA, is refused.
        let other = hub_named(&ca, "hub.example.com").await.to_string();
        assert_eq!(
            check(&other, ca.cert_pem()).await,
            Err(TunnelError::Untrusted)
        );
    }

    /// Audit F1051: the fingerprint the user confirms names one certificate, so a CA text carrying a second one (the real CA first,
    /// another after it, which shows the real fingerprint) is refused everywhere instead of trusting both.
    #[tokio::test]
    async fn a_ca_text_with_a_second_certificate_is_refused_since_the_fingerprint_names_one() {
        let real = Ca::generate("hub CA").unwrap();
        let other = Ca::generate("other CA").unwrap();
        let other_hub = hub(&other).await;
        for pasted in [
            format!("{}{}", real.cert_pem(), other.cert_pem()),
            format!("{}{}", other.cert_pem(), real.cert_pem()),
        ] {
            assert_eq!(fingerprint(&pasted), Err(TunnelError::BadCa));
            assert_eq!(
                check(&other_hub.to_string(), &pasted).await,
                Err(TunnelError::BadCa)
            );
            assert_eq!(
                open(&other_hub.to_string(), &pasted, 0).await.err(),
                Some(TunnelError::BadCa)
            );
        }
        // The same CA twice is still one CA.
        let twice = format!("{}{}", real.cert_pem(), real.cert_pem());
        assert_eq!(fingerprint(&twice), fingerprint(real.cert_pem()));
        // The real CA alone still trusts the real CA only.
        let real_hub = hub(&real).await;
        assert_eq!(check(&real_hub.to_string(), real.cert_pem()).await, Ok(()));
        assert_eq!(
            check(&other_hub.to_string(), real.cert_pem()).await,
            Err(TunnelError::Untrusted)
        );
    }

    #[test]
    fn the_fingerprint_is_the_certificates_sha256_and_bad_input_is_refused() {
        let ca = Ca::generate("hub CA").unwrap();
        let f = fingerprint(ca.cert_pem()).unwrap();
        assert_eq!(f.len(), 32 * 3 - 1);
        assert!(f
            .chars()
            .all(|c| c == ':' || c.is_ascii_digit() || ('A'..='F').contains(&c)));
        assert_eq!(f, fingerprint(ca.cert_pem()).unwrap());
        assert_ne!(
            f,
            fingerprint(Ca::generate("other").unwrap().cert_pem()).unwrap()
        );
        assert_eq!(fingerprint("not a pem"), Err(TunnelError::BadCa));
        assert_eq!(server_name("nohost").err(), Some(TunnelError::BadAddress));
        assert_eq!(
            server_name("host:notaport").err(),
            Some(TunnelError::BadAddress)
        );
        assert!(
            server_name("hub.example.com:7443").is_ok() && server_name("127.0.0.1:7443").is_ok()
        );
    }
}
