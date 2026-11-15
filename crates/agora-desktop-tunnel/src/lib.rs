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
//! otherwise expose the wrong name, nothing else stops it. On the hub's own machine the hub sees the tunnel as loopback, which
//! signs in without a code (threat model T22).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName};
use tokio::io::{copy_bidirectional, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsConnector;

/// The most of a request's head the tunnel reads before it decides. A browser's request head is a few hundred bytes to a few KiB
/// (cookies included); a longer one is refused rather than buffered.
const MAX_HEAD: usize = 16 * 1024;

/// What a request addressed to another host gets: no body and no text, since the webview shows its own error page.
const MISDIRECTED: &[u8] =
    b"HTTP/1.1 421 Misdirected Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

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
/// `localhost:<port>` (the port may be left out only when it is 80, as browsers do). Anything else (another name, another port,
/// no `Host`, two of them, a head that is not UTF-8) is refused.
///
/// Only the first request on a connection is checked. A browser keeps a connection for the origin it opened it for, so a page on
/// another name never shares the webview's connection; a local program could send another `Host` later, but such a program can
/// send the right one anyway, so this check is about browsers only.
fn addressed_to_tunnel(head: &[u8], port: u16) -> bool {
    let Ok(text) = std::str::from_utf8(head) else {
        return false;
    };
    // The fields follow the request line and end at the blank line. A field name has no space before its colon (RFC 9112 5.1),
    // and a line starting with a space continues the previous field, so neither can count as `Host`.
    let mut hosts = text
        .split("\r\n")
        .skip(1)
        .take_while(|l| !l.is_empty())
        .filter_map(|l| {
            let (name, value) = l.split_once(':')?;
            name.eq_ignore_ascii_case("host").then(|| value.trim())
        });
    let (Some(host), None) = (hosts.next(), hosts.next()) else {
        return false;
    };
    let (name, given_port) = match host.rsplit_once(':') {
        Some((name, p)) => (name, p.parse::<u16>().ok()),
        None => (host, Some(80)),
    };
    given_port == Some(port) && (name == "127.0.0.1" || name.eq_ignore_ascii_case("localhost"))
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

/// The SHA-256 of the CA's certificate, as upper-case hex pairs separated by colons: what the user compares on first use.
pub fn fingerprint(ca_pem: &str) -> Result<String, TunnelError> {
    let der = CertificateDer::pem_slice_iter(ca_pem.as_bytes())
        .next()
        .ok_or(TunnelError::BadCa)?
        .map_err(|_| TunnelError::BadCa)?;
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
    let mut roots = rustls::RootCertStore::empty();
    for c in CertificateDer::pem_slice_iter(ca_pem.as_bytes()) {
        roots
            .add(c.map_err(|_| TunnelError::BadCa)?)
            .map_err(|_| TunnelError::BadCa)?;
    }
    if roots.is_empty() {
        return Err(TunnelError::BadCa);
    }
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

/// A running tunnel. Dropping it closes the loopback port; connections already open finish on their own.
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
    let name = server_name(hub)?;
    let tls = connector(ca_pem)?;
    // Only the loopback interface: the tunnel is for this machine's webview, never for the network.
    let listener = TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|_| TunnelError::Listen)?;
    let local = listener.local_addr().map_err(|_| TunnelError::Listen)?;
    let hub = hub.to_string();
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut inbound, _)) = listener.accept().await else {
                break;
            };
            let (hub, name, tls) = (hub.clone(), name.clone(), tls.clone());
            tokio::spawn(async move {
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
        // Port 80 may be left out, as browsers do.
        assert!(addressed_to_tunnel(
            b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n",
            80
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
