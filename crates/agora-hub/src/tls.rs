//! TLS for node connections (ADR-0011, ADR-0015): the hub presents its certificate and *requires* a client
//! certificate issued by its own CA. The node's identity is read from that certificate, never from anything the
//! node says afterwards. Revocation is checked against the device registry by the caller (`Enrollment::is_active`)
//! using the serial in [`NodeIdentity`], so it takes effect on the next connection and for live ones on a sweep.

use std::fmt;
use std::sync::Arc;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::WebPkiClientVerifier;
use rustls::{RootCertStore, ServerConfig};
use tokio_rustls::TlsAcceptor;

#[derive(Debug)]
pub struct TlsError(String);

impl fmt::Display for TlsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tls: {}", self.0)
    }
}

impl std::error::Error for TlsError {}

fn err(e: impl fmt::Display) -> TlsError {
    TlsError(e.to_string())
}

/// Who a connection is, as proven by its client certificate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeIdentity {
    pub node_id: String,
    pub serial_hex: String,
}

/// An acceptor that completes the handshake only for clients holding a certificate from `ca_cert_pem`.
pub fn acceptor(
    server_cert_pem: &str,
    server_key_pem: &str,
    ca_cert_pem: &str,
) -> Result<TlsAcceptor, TlsError> {
    acceptor_with(server_cert_pem, server_key_pem, ca_cert_pem, false)
}

/// Like [`acceptor`]; with `allow_unauthenticated` a peer without a client certificate also completes the handshake
/// (it is a client with a bearer token, or a node that is enrolling), while a certificate that is presented must
/// still verify against the CA. The server layer decides what an unauthenticated peer may do.
pub fn acceptor_with(
    server_cert_pem: &str,
    server_key_pem: &str,
    ca_cert_pem: &str,
    allow_unauthenticated: bool,
) -> Result<TlsAcceptor, TlsError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut roots = RootCertStore::empty();
    for c in CertificateDer::pem_slice_iter(ca_cert_pem.as_bytes()) {
        roots.add(c.map_err(err)?).map_err(err)?;
    }
    let mut vb = WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider.clone());
    if allow_unauthenticated {
        vb = vb.allow_unauthenticated();
    }
    let verifier = vb.build().map_err(err)?;
    let chain: Vec<CertificateDer<'static>> =
        CertificateDer::pem_slice_iter(server_cert_pem.as_bytes())
            .collect::<Result<_, _>>()
            .map_err(err)?;
    let key = PrivateKeyDer::from_pem_slice(server_key_pem.as_bytes()).map_err(err)?;
    let config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(err)?
        .with_client_cert_verifier(verifier)
        .with_single_cert(chain, key)
        .map_err(err)?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}

/// The identity in the client certificate of an established connection. `None` when the peer sent none (the
/// verifier above already refuses that, so this is a second line of defence) or the name is unusable.
pub fn identity(peer_certs: Option<&[CertificateDer<'_>]>) -> Option<NodeIdentity> {
    let der = peer_certs?.first()?;
    let (_, cert) = x509_parser::parse_x509_certificate(der).ok()?;
    let node_id = cert
        .subject()
        .iter_common_name()
        .next()?
        .as_str()
        .ok()?
        .to_string();
    Some(NodeIdentity {
        node_id,
        serial_hex: cert.raw_serial_as_string().replace(':', ""),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::ca::{new_device_request, Ca};
    use rustls::ClientConfig;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio_rustls::TlsConnector;

    struct Hub {
        ca: Ca,
        acceptor: TlsAcceptor,
    }

    fn hub() -> Hub {
        let ca = Ca::generate("hub ca").unwrap();
        let (cert, key) = ca.issue_server(&["localhost".to_string()]).unwrap();
        let acceptor = acceptor(&cert, &key, ca.cert_pem()).unwrap();
        Hub { ca, acceptor }
    }

    /// Connector trusting `trust`, presenting a client certificate for `node_id` from `issuer` (if any).
    fn connector(trust: &Ca, client: Option<(&Ca, &str)>) -> TlsConnector {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut roots = RootCertStore::empty();
        for c in CertificateDer::pem_slice_iter(trust.cert_pem().as_bytes()) {
            roots.add(c.unwrap()).unwrap();
        }
        let b = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots);
        let cfg = match client {
            None => b.with_no_client_auth(),
            Some((issuer, id)) => {
                let (key, csr) = new_device_request().unwrap();
                let cert = issuer.issue_node(id, &csr).unwrap().cert_pem;
                let chain = CertificateDer::pem_slice_iter(cert.as_bytes())
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap();
                b.with_client_auth_cert(
                    chain,
                    PrivateKeyDer::from_pem_slice(key.as_bytes()).unwrap(),
                )
                .unwrap()
            }
        };
        TlsConnector::from(Arc::new(cfg))
    }

    /// Runs one connection; returns the server's view (identity) or the failure.
    async fn connect(hub: &Hub, connector: TlsConnector) -> Result<Option<NodeIdentity>, String> {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        let acceptor = hub.acceptor.clone();
        let server = tokio::spawn(async move {
            let (s, _) = l.accept().await.unwrap();
            let mut tls = acceptor.accept(s).await.map_err(|e| e.to_string())?;
            let id = identity(tls.get_ref().1.peer_certificates());
            let mut buf = [0u8; 4];
            tls.read_exact(&mut buf).await.map_err(|e| e.to_string())?;
            tls.write_all(b"pong").await.map_err(|e| e.to_string())?;
            tls.flush().await.map_err(|e| e.to_string())?;
            Ok::<_, String>(id)
        });
        let client = async {
            let s = TcpStream::connect(addr).await.map_err(|e| e.to_string())?;
            let name = rustls::pki_types::ServerName::try_from("localhost").unwrap();
            let mut tls = connector
                .connect(name, s)
                .await
                .map_err(|e| e.to_string())?;
            tls.write_all(b"ping").await.map_err(|e| e.to_string())?;
            let mut buf = [0u8; 4];
            tls.read_exact(&mut buf).await.map_err(|e| e.to_string())?;
            Ok::<_, String>(())
        };
        let (s, c) = tokio::join!(server, client);
        let s = s.unwrap();
        match (s, c) {
            (Ok(id), Ok(())) => Ok(id),
            (Err(e), _) | (_, Err(e)) => Err(e),
        }
    }

    #[tokio::test]
    async fn an_enrolled_node_connects_and_is_identified_by_its_certificate() {
        let hub = hub();
        let c = connector(&hub.ca, Some((&hub.ca, "laptop-1")));
        let id = connect(&hub, c).await.unwrap().unwrap();
        assert_eq!(id.node_id, "laptop-1");
        assert!(!id.serial_hex.is_empty());
    }

    #[tokio::test]
    async fn a_client_without_a_certificate_is_refused() {
        let hub = hub();
        assert!(connect(&hub, connector(&hub.ca, None)).await.is_err());
    }

    #[tokio::test]
    async fn a_certificate_from_another_authority_is_refused() {
        let hub = hub();
        let rogue = Ca::generate("rogue").unwrap();
        let c = connector(&hub.ca, Some((&rogue, "laptop-1")));
        assert!(connect(&hub, c).await.is_err());
    }

    #[tokio::test]
    async fn a_node_that_does_not_trust_the_hub_ca_refuses_the_hub() {
        let hub = hub();
        let other = Ca::generate("someone else").unwrap();
        let c = connector(&other, Some((&hub.ca, "laptop-1")));
        assert!(connect(&hub, c).await.is_err());
    }

    #[test]
    fn identity_needs_a_certificate_with_a_name() {
        assert_eq!(identity(None), None);
        assert_eq!(identity(Some(&[])), None);
    }

    #[test]
    fn a_server_certificate_needs_a_name() {
        let ca = Ca::generate("ca").unwrap();
        assert!(ca.issue_server(&[]).is_err());
    }
}
