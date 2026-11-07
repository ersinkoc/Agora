//! The node's connection to the hub (ADR-0011): dials `wss://` with the node's client certificate, runs the
//! [`Uplink`] conversation, and reconnects with backoff when the connection drops. Nodes are autonomous: nothing here
//! can stop an agent, and when the hub is unreachable events simply wait in the durable buffer.
//!
//! The hub is trusted only through the CA the node was given at enrollment (pinned), never the system roots.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use agora_protocol::frame::{kinds, STREAM_CONTROL, STREAM_EVENTS};
use agora_protocol::v1::{EventAck, ResumeOk, Welcome};
use agora_protocol::ws::{self, Conn, TransportError};
use agora_protocol::Frame;
use prost::Message;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;

use crate::daemon::EventSink;
use crate::uplink::{Uplink, UplinkError};

/// Events sent per step; the hub acknowledges after each batch.
const BATCH: usize = 64;
const TICK: Duration = Duration::from_millis(50);
const PING_EVERY: Duration = Duration::from_secs(15);
const BACKOFF_MIN: Duration = Duration::from_millis(500);
const BACKOFF_MAX: Duration = Duration::from_secs(30);

/// Everything needed to dial the hub. PEM texts, as written at enrollment.
#[derive(Clone)]
pub struct HubLink {
    /// `wss://host:port` (or `ws://host:port` for a local test hub; the hub then cannot tell who the node is).
    pub url: String,
    pub ca_pem: String,
    pub cert_pem: String,
    pub key_pem: String,
}

#[derive(Debug)]
pub enum LinkError {
    Bad(String),
    Transport(String),
    Uplink(UplinkError),
    /// The hub closed the connection.
    Closed,
    /// The hub answered with this error code (for enrollment: a wrong, expired or used code is `auth.unauthenticated`).
    Refused(String),
}

impl From<TransportError> for LinkError {
    fn from(e: TransportError) -> Self {
        Self::Transport(e.to_string())
    }
}

impl From<UplinkError> for LinkError {
    fn from(e: UplinkError) -> Self {
        Self::Uplink(e)
    }
}

fn split_url(url: &str) -> Result<(bool, String, String), LinkError> {
    let (tls, rest) = if let Some(r) = url.strip_prefix("wss://") {
        (true, r)
    } else if let Some(r) = url.strip_prefix("ws://") {
        (false, r)
    } else {
        return Err(LinkError::Bad("url must start with wss:// or ws://".into()));
    };
    let host_port = rest.split('/').next().unwrap_or("");
    let host = host_port.rsplit_once(':').map_or(host_port, |(h, _)| h);
    if host.is_empty() {
        return Err(LinkError::Bad("url has no host".into()));
    }
    let with_port = if host_port.contains(':') {
        host_port.to_string()
    } else {
        format!("{host_port}:{}", if tls { 443 } else { 80 })
    };
    Ok((tls, host.to_string(), with_port))
}

fn connector(link: &HubLink) -> Result<tokio_rustls::TlsConnector, LinkError> {
    let bad = |e: &dyn std::fmt::Display| LinkError::Bad(e.to_string());
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut roots = rustls::RootCertStore::empty();
    for c in CertificateDer::pem_slice_iter(link.ca_pem.as_bytes()) {
        roots.add(c.map_err(|e| bad(&e))?).map_err(|e| bad(&e))?;
    }
    let chain: Vec<CertificateDer<'static>> =
        CertificateDer::pem_slice_iter(link.cert_pem.as_bytes())
            .collect::<Result<_, _>>()
            .map_err(|e| bad(&e))?;
    let key = PrivateKeyDer::from_pem_slice(link.key_pem.as_bytes()).map_err(|e| bad(&e))?;
    let cfg = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| bad(&e))?
        .with_root_certificates(roots)
        .with_client_auth_cert(chain, key)
        .map_err(|e| bad(&e))?;
    Ok(tokio_rustls::TlsConnector::from(Arc::new(cfg)))
}

fn frame(stream_id: u32, kind: u8, payload: Vec<u8>) -> Frame {
    Frame {
        stream_id,
        kind,
        seq: None,
        payload,
    }
}

async fn recv_expect<S>(conn: &mut Conn<S>, kind: u8) -> Result<Frame, LinkError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    match tokio::time::timeout(Duration::from_secs(15), conn.recv()).await {
        Ok(Ok(Some(f))) if f.stream_id == STREAM_CONTROL && f.kind == kind => Ok(f),
        Ok(Ok(Some(_))) => Err(LinkError::Transport("unexpected frame".into())),
        Ok(Ok(None)) => Err(LinkError::Closed),
        Ok(Err(e)) => Err(e.into()),
        Err(_) => Err(LinkError::Transport("the hub did not answer".into())),
    }
}

/// One connection's whole life: handshake, then stream events until it ends or `stop` is set.
pub async fn run_once(
    link: &HubLink,
    sink: &Arc<EventSink>,
    stop: &AtomicBool,
) -> Result<(), LinkError> {
    let (tls, host, addr) = split_url(&link.url)?;
    let tcp = TcpStream::connect(&addr)
        .await
        .map_err(|e| LinkError::Transport(e.to_string()))?;
    if tls {
        let name = ServerName::try_from(host.clone()).map_err(|e| LinkError::Bad(e.to_string()))?;
        let stream = connector(link)?
            .connect(name, tcp)
            .await
            .map_err(|e| LinkError::Transport(e.to_string()))?;
        let conn = ws::connect(stream, &link.url).await?;
        converse(conn, sink, stop).await
    } else {
        let conn = ws::connect(tcp, &link.url).await?;
        converse(conn, sink, stop).await
    }
}

async fn converse<S>(
    mut conn: Conn<S>,
    sink: &Arc<EventSink>,
    stop: &AtomicBool,
) -> Result<(), LinkError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut up = Uplink::new(
        Arc::clone(sink),
        env!("CARGO_PKG_VERSION"),
        &["terminal.raw"],
    );
    conn.send(&frame(
        STREAM_CONTROL,
        kinds::HELLO,
        up.hello()?.encode_to_vec(),
    ))
    .await?;
    let welcome = Welcome::decode(
        recv_expect(&mut conn, kinds::WELCOME)
            .await?
            .payload
            .as_slice(),
    )
    .map_err(|_| LinkError::Transport("bad welcome".into()))?;
    up.on_welcome(&welcome)?;
    conn.send(&frame(
        STREAM_CONTROL,
        kinds::RESUME,
        up.resume()?.encode_to_vec(),
    ))
    .await?;
    let ok = ResumeOk::decode(
        recv_expect(&mut conn, kinds::RESUME_OK)
            .await?
            .payload
            .as_slice(),
    )
    .map_err(|_| LinkError::Transport("bad resume".into()))?;
    up.on_resume_ok(&ok)?;

    let mut last_ping = tokio::time::Instant::now();
    while !stop.load(Ordering::Acquire) {
        for payload in up.next_batch(BATCH)? {
            conn.send(&frame(STREAM_EVENTS, kinds::EVENT, payload))
                .await?;
        }
        if last_ping.elapsed() >= PING_EVERY {
            conn.send(&frame(STREAM_CONTROL, kinds::PING, Vec::new()))
                .await?;
            last_ping = tokio::time::Instant::now();
        }
        match tokio::time::timeout(TICK, conn.recv()).await {
            Err(_) => {}
            Ok(Ok(None)) => return Err(LinkError::Closed),
            Ok(Err(e)) => return Err(e.into()),
            Ok(Ok(Some(f))) => match (f.stream_id, f.kind) {
                (STREAM_EVENTS, kinds::EVENT_ACK) => {
                    if let Ok(a) = EventAck::decode(f.payload.as_slice()) {
                        up.on_ack(a.up_to_seq)?;
                    }
                }
                // The hub saw a gap and says where to start again.
                (STREAM_CONTROL, kinds::RESUME_OK) => {
                    if let Ok(r) = ResumeOk::decode(f.payload.as_slice()) {
                        up.on_gap(r.from_seq)?;
                    }
                }
                _ => {}
            },
        }
    }
    conn.close().await;
    Ok(())
}

/// A running uplink thread. Dropping it stops the connection (agents are not affected).
pub struct UplinkHandle {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for UplinkHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Starts connecting to the hub in the background and keeps doing so, with exponential backoff, until dropped.
pub fn spawn(sink: Arc<EventSink>, link: HubLink) -> std::io::Result<UplinkHandle> {
    let stop = Arc::new(AtomicBool::new(false));
    let thread = {
        let stop = Arc::clone(&stop);
        std::thread::Builder::new()
            .name("agora-uplink".into())
            .spawn(move || {
                let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    return;
                };
                rt.block_on(async {
                    let mut backoff = BACKOFF_MIN;
                    while !stop.load(Ordering::Acquire) {
                        let started = tokio::time::Instant::now();
                        let r = run_once(&link, &sink, &stop).await;
                        if stop.load(Ordering::Acquire) {
                            break;
                        }
                        // A connection that lasted a while was healthy: start the backoff over.
                        if started.elapsed() > Duration::from_secs(30) || r.is_ok() {
                            backoff = BACKOFF_MIN;
                        }
                        let mut waited = Duration::ZERO;
                        while waited < backoff && !stop.load(Ordering::Acquire) {
                            tokio::time::sleep(Duration::from_millis(100)).await;
                            waited += Duration::from_millis(100);
                        }
                        backoff = (backoff * 2).min(BACKOFF_MAX);
                    }
                });
            })?
    };
    Ok(UplinkHandle {
        stop,
        thread: Some(thread),
    })
}

/// Enrolls this node with a hub: presents the one-time `code` and a CSR over TLS that trusts only `ca_pem`, and
/// returns the hub's answer (the certificate for the id the hub approved, and the CA to keep pinning). Blocks the
/// calling thread; enrollment happens once, from the CLI.
pub fn enroll(
    url: &str,
    ca_pem: &str,
    code: &str,
    node_id: &str,
    csr_pem: &str,
) -> Result<agora_protocol::v1::EnrollResponse, LinkError> {
    use agora_protocol::v1::{
        command_ok, command_request, command_response, CommandRequest, CommandResponse,
        EnrollRequest,
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| LinkError::Transport(e.to_string()))?;
    rt.block_on(async {
        let (tls, host, addr) = split_url(url)?;
        if !tls {
            return Err(LinkError::Bad("enrollment needs wss://".into()));
        }
        let bad = |e: &dyn std::fmt::Display| LinkError::Bad(e.to_string());
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut roots = rustls::RootCertStore::empty();
        for c in CertificateDer::pem_slice_iter(ca_pem.as_bytes()) {
            roots.add(c.map_err(|e| bad(&e))?).map_err(|e| bad(&e))?;
        }
        let cfg = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| bad(&e))?
            .with_root_certificates(roots)
            .with_no_client_auth();
        let tcp = TcpStream::connect(&addr)
            .await
            .map_err(|e| LinkError::Transport(e.to_string()))?;
        let name = ServerName::try_from(host).map_err(|e| bad(&e))?;
        let stream = tokio_rustls::TlsConnector::from(Arc::new(cfg))
            .connect(name, tcp)
            .await
            .map_err(|e| LinkError::Transport(e.to_string()))?;
        let mut conn = ws::connect(stream, url).await?;
        let req = CommandRequest {
            request_id: "enroll".into(),
            idempotency_key: String::new(),
            command: Some(command_request::Command::Enroll(EnrollRequest {
                code: code.into(),
                node_id: node_id.into(),
                csr_pem: csr_pem.into(),
            })),
        };
        conn.send(&frame(
            agora_protocol::frame::STREAM_COMMANDS,
            0,
            req.encode_to_vec(),
        ))
        .await?;
        let f = match tokio::time::timeout(Duration::from_secs(15), conn.recv()).await {
            Ok(Ok(Some(f))) => f,
            Ok(Ok(None)) => return Err(LinkError::Closed),
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => return Err(LinkError::Transport("the hub did not answer".into())),
        };
        let resp = CommandResponse::decode(f.payload.as_slice())
            .map_err(|_| LinkError::Transport("bad response".into()))?;
        match resp.result {
            Some(command_response::Result::Ok(ok)) => match ok.result {
                Some(command_ok::Result::Enroll(e)) => Ok(e),
                _ => Err(LinkError::Transport("unexpected answer".into())),
            },
            Some(command_response::Result::Err(e)) => Err(LinkError::Refused(e.code)),
            None => Err(LinkError::Transport("empty answer".into())),
        }
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn urls_split_into_scheme_host_and_address() {
        assert_eq!(
            split_url("wss://hub.example:7443/x").unwrap(),
            (true, "hub.example".into(), "hub.example:7443".into())
        );
        assert_eq!(
            split_url("ws://127.0.0.1:9").unwrap(),
            (false, "127.0.0.1".into(), "127.0.0.1:9".into())
        );
        assert_eq!(
            split_url("wss://hub").unwrap(),
            (true, "hub".into(), "hub:443".into())
        );
        assert!(split_url("http://x").is_err());
        assert!(split_url("wss://").is_err());
    }
}
