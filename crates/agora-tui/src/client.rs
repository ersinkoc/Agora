//! The hub client of the terminal console (ADR-0043): sign in with a one-time code, then follow the hub's snapshot. It speaks the
//! same protocol as the browser (ADR-0027) and trusts only the hub's own CA.
//!
//! Every call blocks the calling thread (a small runtime is built per call), so the terminal loop stays plain code.

use std::sync::Arc;
use std::time::Duration;

use crate::device::{approval_message, Device};
use agora_protocol::frame::{kinds, STREAM_COMMANDS, STREAM_EVENTS};
use agora_protocol::v1::{
    command_request, command_response, CommandRequest, CommandResponse, Snapshot,
};
use agora_protocol::ws::{connect_with_token, Conn};
use agora_protocol::Frame;
use prost::Message;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use tokio_rustls::TlsConnector;

/// Where the hub is and which CA signed its certificate.
#[derive(Clone)]
pub struct Hub {
    /// `host:port`.
    pub addr: String,
    pub ca_pem: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ClientError {
    /// The hub could not be reached or spoke something else (connection, TLS, framing).
    Unreachable,
    /// The code or the token was not accepted.
    Refused,
    /// Too many wrong codes: the hub asks to wait.
    TooMany,
}

fn runtime() -> Result<tokio::runtime::Runtime, ClientError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| ClientError::Unreachable)
}

/// The name the hub's certificate must carry: its host, or `localhost` when the address is an IP number (the hub's certificate
/// always names `localhost`).
fn server_name(addr: &str) -> Result<ServerName<'static>, ClientError> {
    let host = addr
        .rsplit_once(':')
        .map_or(addr, |(h, _)| h)
        .trim_matches(['[', ']']);
    let name = if host.parse::<std::net::IpAddr>().is_ok() {
        "localhost"
    } else {
        host
    };
    ServerName::try_from(name.to_string()).map_err(|_| ClientError::Unreachable)
}

async fn tls(hub: &Hub) -> Result<TlsStream<TcpStream>, ClientError> {
    let mut roots = rustls::RootCertStore::empty();
    for c in CertificateDer::pem_slice_iter(hub.ca_pem.as_bytes()) {
        roots
            .add(c.map_err(|_| ClientError::Unreachable)?)
            .map_err(|_| ClientError::Unreachable)?;
    }
    let cfg = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|_| ClientError::Unreachable)?
    .with_root_certificates(roots)
    .with_no_client_auth();
    let tcp = tokio::time::timeout(Duration::from_secs(10), TcpStream::connect(&hub.addr))
        .await
        .map_err(|_| ClientError::Unreachable)?
        .map_err(|_| ClientError::Unreachable)?;
    tokio::time::timeout(
        Duration::from_secs(10),
        TlsConnector::from(Arc::new(cfg)).connect(server_name(&hub.addr)?, tcp),
    )
    .await
    .map_err(|_| ClientError::Unreachable)?
    .map_err(|_| ClientError::Unreachable)
}

/// Trades a one-time sign-in code (`agora hub login`) for a session token.
pub fn sign_in(hub: &Hub, code: &str) -> Result<String, ClientError> {
    runtime()?.block_on(async {
        let mut s = tls(hub).await?;
        let body = serde_json::json!({ "code": code }).to_string();
        let host = hub.addr.clone();
        let req = format!(
            "POST /api/session HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        s.write_all(req.as_bytes()).await.map_err(|_| ClientError::Unreachable)?;
        let mut raw = Vec::new();
        // The hub closes after answering; a missing close_notify after the answer is not an error here.
        let _ = tokio::time::timeout(Duration::from_secs(10), s.read_to_end(&mut raw)).await;
        let text = String::from_utf8_lossy(&raw);
        let status = text.split_whitespace().nth(1).unwrap_or("");
        match status {
            "200" => {
                let json = text.split_once("\r\n\r\n").map_or("", |(_, b)| b);
                serde_json::from_str::<serde_json::Value>(json)
                    .ok()
                    .and_then(|v| v.get("token").and_then(|t| t.as_str()).map(str::to_string))
                    .ok_or(ClientError::Unreachable)
            }
            "401" => Err(ClientError::Refused),
            "429" => Err(ClientError::TooMany),
            _ => Err(ClientError::Unreachable),
        }
    })
}

/// A live connection: one snapshot, then the positions of the events that follow.
pub struct Session {
    rt: tokio::runtime::Runtime,
    conn: Conn<TlsStream<TcpStream>>,
    requests: u64,
    /// An event arrived while a command was waiting for its answer.
    changed: bool,
}

/// What the hub sent next.
pub enum Update {
    Snapshot(Box<Snapshot>),
    /// An event happened somewhere; the console asks for a fresh snapshot.
    Changed,
    /// Nothing within the wait.
    Quiet,
    Closed,
}

impl Session {
    /// Connects and subscribes; the first update is the snapshot.
    pub fn connect(hub: &Hub, token: &str) -> Result<Session, ClientError> {
        let rt = runtime()?;
        let conn = rt.block_on(async {
            let stream = tls(hub).await?;
            let host = hub
                .addr
                .rsplit_once(':')
                .map_or(hub.addr.as_str(), |(h, _)| h);
            let mut conn = connect_with_token(stream, &format!("wss://{host}/"), token)
                .await
                .map_err(|_| ClientError::Refused)?;
            let sub = agora_protocol::v1::Subscribe { after_pos: None }.encode_to_vec();
            conn.send(&Frame {
                stream_id: STREAM_EVENTS,
                kind: kinds::SUBSCRIBE,
                seq: None,
                payload: sub,
            })
            .await
            .map_err(|_| ClientError::Unreachable)?;
            Ok::<_, ClientError>(conn)
        })?;
        Ok(Session {
            rt,
            conn,
            requests: 0,
            changed: false,
        })
    }

    /// Waits up to `wait` for the hub to say something.
    pub fn next(&mut self, wait: Duration) -> Update {
        if std::mem::take(&mut self.changed) {
            return Update::Changed;
        }
        let conn = &mut self.conn;
        self.rt.block_on(async {
            match tokio::time::timeout(wait, conn.recv()).await {
                Err(_) => Update::Quiet,
                Ok(Ok(Some(f))) if f.stream_id == STREAM_EVENTS && f.kind == kinds::SNAPSHOT => {
                    match Snapshot::decode(f.payload.as_slice()) {
                        Ok(s) => Update::Snapshot(Box::new(s)),
                        Err(_) => Update::Closed,
                    }
                }
                Ok(Ok(Some(f)))
                    if f.stream_id == STREAM_EVENTS && f.kind == kinds::POSITIONED_EVENT =>
                {
                    Update::Changed
                }
                Ok(Ok(Some(_))) => Update::Quiet,
                Ok(_) => Update::Closed,
            }
        })
    }

    /// Runs a command on a machine through the hub (ADR-0029) and waits for its answer. `Err` carries the error code the machine or
    /// the hub answered with (`approval.signature_invalid`, `node.offline`, ...), or `client.unreachable` when the link failed.
    pub fn command(
        &mut self,
        node: &str,
        command: command_request::Command,
        wait: Duration,
    ) -> Result<(), String> {
        self.requests += 1;
        let id = format!("t{}", self.requests);
        let req = CommandRequest {
            request_id: id.clone(),
            idempotency_key: String::new(),
            target_node: node.to_string(),
            command: Some(command),
        };
        let conn = &mut self.conn;
        let changed = &mut self.changed;
        self.rt.block_on(async {
            conn.send(&Frame {
                stream_id: STREAM_COMMANDS,
                kind: 0,
                seq: None,
                payload: req.encode_to_vec(),
            })
            .await
            .map_err(|_| "client.unreachable".to_string())?;
            let end = tokio::time::Instant::now() + wait;
            loop {
                let left = end.saturating_duration_since(tokio::time::Instant::now());
                let f = match tokio::time::timeout(left, conn.recv()).await {
                    Ok(Ok(Some(f))) => f,
                    Err(_) => return Err("node.timeout".to_string()),
                    Ok(_) => return Err("client.unreachable".to_string()),
                };
                if (f.stream_id, f.kind) == (STREAM_COMMANDS, 1) {
                    let Ok(r) = CommandResponse::decode(f.payload.as_slice()) else {
                        continue;
                    };
                    if r.request_id != id {
                        continue;
                    }
                    return match r.result {
                        Some(command_response::Result::Err(e)) => Err(e.code),
                        _ => Ok(()),
                    };
                }
                if f.stream_id == STREAM_EVENTS && f.kind == kinds::POSITIONED_EVENT {
                    *changed = true;
                }
            }
        })
    }

    /// Answers the first approval waiting in `snapshot` with this device's signature (ADR-0028). `Ok(true)` when there was one and the
    /// machine took the answer, `Ok(false)` when nothing was waiting, `Err` with the error code otherwise
    /// (`approval.signature_invalid` means the machine does not trust this device yet).
    pub fn resolve_first_approval(
        &mut self,
        snapshot: &Snapshot,
        device: &Device,
        approved: bool,
    ) -> Result<bool, String> {
        let Some(a) = snapshot.approvals.first() else {
            return Ok(false);
        };
        // Hub-wide ids are `<node>/<local id>`; the machine knows the local one.
        let (node, local) = a
            .approval_id
            .split_once('/')
            .unwrap_or(("", a.approval_id.as_str()));
        let signature = device
            .sign(&approval_message(node, local, approved))
            .ok_or_else(|| "client.sign".to_string())?;
        let cmd = command_request::Command::ApprovalResolve(agora_protocol::v1::ApprovalResolve {
            approval_id: local.to_string(),
            approved,
            signer_device_id: device.id.clone(),
            signature,
        });
        self.command(node, cmd, Duration::from_secs(15))
            .map(|()| true)
    }
}
