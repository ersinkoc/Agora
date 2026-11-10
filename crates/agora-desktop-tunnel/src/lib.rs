//! How the desktop app reaches a hub (ADR-0048). A webview trusts the system's certificate store, and the hub's certificate comes
//! from its own CA (ADR-0015), so the shell cannot simply open `https://<hub>/`. Instead it runs a tunnel on the loopback interface:
//! the webview loads `http://127.0.0.1:<port>/`, and every connection to that port is carried to the hub over TLS that trusts **only
//! the CA the user confirmed**, never the system store. The tunnel copies bytes and reads none of them: the page, its WebSocket
//! and its sign-in all pass through unchanged, and the tunnel holds no credential of its own.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName};
use tokio::io::copy_bidirectional;
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsConnector;

#[derive(Debug, PartialEq, Eq)]
pub enum TunnelError {
    /// The CA is not a readable PEM certificate.
    BadCa,
    /// The hub address is not `host:port`.
    BadAddress,
    /// The loopback port could not be opened.
    Listen,
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

/// The name the hub's certificate must carry: its host, or `localhost` when the address is an IP number (the hub's own certificate
/// names `localhost`).
fn server_name(addr: &str) -> Result<ServerName<'static>, TunnelError> {
    let (host, port) = addr.rsplit_once(':').ok_or(TunnelError::BadAddress)?;
    if host.is_empty() || port.parse::<u16>().is_err() {
        return Err(TunnelError::BadAddress);
    }
    let host = host.trim_matches(['[', ']']);
    let name = if host.parse::<std::net::IpAddr>().is_ok() {
        "localhost"
    } else {
        host
    };
    ServerName::try_from(name.to_string()).map_err(|_| TunnelError::BadAddress)
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
    let cfg = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|_| TunnelError::BadCa)?
    .with_root_certificates(roots)
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
                // A hub that is down, slow, or that does not present a certificate from the pinned CA ends this one connection and
                // nothing else; the webview shows its own error page.
                let Ok(Ok(tcp)) =
                    tokio::time::timeout(Duration::from_secs(10), TcpStream::connect(&hub)).await
                else {
                    return;
                };
                let Ok(Ok(mut outbound)) =
                    tokio::time::timeout(Duration::from_secs(10), tls.connect(name, tcp)).await
                else {
                    return;
                };
                let _ = copy_bidirectional(&mut inbound, &mut outbound).await;
            });
        }
    });
    Ok(Tunnel { local, task })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use agora_hub::ca::Ca;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A TLS server that answers one line with `hello <line>`, signed by `ca`.
    async fn hub(ca: &Ca) -> SocketAddr {
        let (cert, key) = ca.issue_server(&["localhost".to_string()]).unwrap();
        let acceptor = agora_hub::tls::acceptor_with(&cert, &key, ca.cert_pem(), true).unwrap();
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let (s, _) = l.accept().await.unwrap();
                let a = acceptor.clone();
                tokio::spawn(async move {
                    let Ok(mut s) = a.accept(s).await else { return };
                    let mut buf = [0u8; 64];
                    let n = s.read(&mut buf).await.unwrap_or(0);
                    let _ = s
                        .write_all(
                            format!("hello {}", String::from_utf8_lossy(&buf[..n])).as_bytes(),
                        )
                        .await;
                    let _ = s.shutdown().await;
                });
            }
        });
        addr
    }

    async fn ask(local: SocketAddr, line: &str) -> String {
        let mut c = TcpStream::connect(local).await.unwrap();
        c.write_all(line.as_bytes()).await.unwrap();
        let mut out = String::new();
        let _ = tokio::time::timeout(Duration::from_secs(5), c.read_to_string(&mut out)).await;
        out
    }

    #[tokio::test]
    async fn the_page_reaches_the_hub_through_the_pinned_tunnel_and_only_on_loopback() {
        let ca = Ca::generate("hub CA").unwrap();
        let addr = hub(&ca).await;
        let t = open(&addr.to_string(), ca.cert_pem(), 0).await.unwrap();
        assert!(t.local.ip().is_loopback());
        assert_eq!(ask(t.local, "page").await, "hello page");
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
