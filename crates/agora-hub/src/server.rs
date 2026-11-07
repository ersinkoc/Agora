//! The hub's network service: turns connections into protocol conversations (ADR-0011, ADR-0026, ADR-0027).
//!
//! Three kinds of peer, decided by what the connection proved, never by what it says:
//! - a **node** (TLS client certificate from the hub CA, not revoked): handshake, resume, then events. The producer id
//!   is the certificate's node id. The hub acknowledges what it has stored ([`EventAck`]) after each batch and when idle;
//! - a **client** (a bearer token the [`ClientAuth`] accepts): subscribes and gets a snapshot or the backlog, then live
//!   events;
//! - an **unauthenticated peer on TLS**: may do exactly one thing, enroll (`EnrollRequest` with a one-time code).
//!
//! Anything else is closed. The functions here take an established [`Conn`], so they are tested over plain WebSocket and
//! run over TLS in production.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agora_protocol::frame::{kinds, STREAM_COMMANDS, STREAM_CONTROL, STREAM_EVENTS};
use agora_protocol::handshake::Support;
use agora_protocol::v1::{self, command_ok, command_request, command_response};
use agora_protocol::ws::{Conn, TransportError};
use agora_protocol::{Error, ErrorCode, Frame};
use prost::Message;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::ca::Ca;
use crate::enroll::{EnrollError, Enrollment, Lockout};
use crate::hub::Hub;
use crate::projection::View;
use crate::session::{Ingest, NodeSession, SessionError};
use crate::store::StoreError;

/// Idle time after which pending acknowledgements are sent, and the longest an event waits for its ack.
const ACK_IDLE: Duration = Duration::from_millis(50);
/// Acknowledge at least every this many events while a node streams without pause.
const ACK_BATCH: u32 = 64;
/// How often live events are pushed to a client.
const PUSH: Duration = Duration::from_millis(20);

#[derive(Debug)]
pub enum ServeError {
    Transport(TransportError),
    Protocol(Error),
    Store(StoreError),
    /// The peer did something this role may not do.
    Refused,
}

impl From<TransportError> for ServeError {
    fn from(e: TransportError) -> Self {
        Self::Transport(e)
    }
}

impl From<SessionError> for ServeError {
    fn from(e: SessionError) -> Self {
        match e {
            SessionError::Protocol(e) => Self::Protocol(e),
            SessionError::Store(e) => Self::Store(e),
        }
    }
}

impl From<StoreError> for ServeError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn frame(stream_id: u32, kind: u8, payload: Vec<u8>) -> Frame {
    Frame {
        stream_id,
        kind,
        seq: None,
        payload,
    }
}

fn invalid(field: &str) -> ServeError {
    ServeError::Protocol(Error::new(ErrorCode::ProtocolInvalidMessage).with("field", field))
}

async fn expect<S>(conn: &mut Conn<S>, stream: u32, kind: u8) -> Result<Frame, ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    match conn.recv().await? {
        Some(f) if f.stream_id == stream && f.kind == kind => Ok(f),
        Some(_) => Err(invalid("frame")),
        None => Err(ServeError::Refused),
    }
}

/// The conversation with one authenticated node, until it ends. `node_id` comes from the certificate. When the
/// node had come online, the hub records it offline at the end.
pub async fn serve_node<S>(
    mut conn: Conn<S>,
    node_id: &str,
    hub: &Hub,
    support: &Support,
) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut session = NodeSession::new(hub.store(), node_id, support);

    let hello = v1::Hello::decode(
        expect(&mut conn, STREAM_CONTROL, kinds::HELLO)
            .await?
            .payload
            .as_slice(),
    )
    .map_err(|_| invalid("hello"))?;
    let welcome = session.hello(&hello, now_ms())?;
    conn.send(&frame(
        STREAM_CONTROL,
        kinds::WELCOME,
        welcome.encode_to_vec(),
    ))
    .await?;

    let resume = v1::Resume::decode(
        expect(&mut conn, STREAM_CONTROL, kinds::RESUME)
            .await?
            .payload
            .as_slice(),
    )
    .map_err(|_| invalid("resume"))?;
    let ok = session.resume(&resume)?;
    conn.send(&frame(STREAM_CONTROL, kinds::RESUME_OK, ok.encode_to_vec()))
        .await?;

    let result = stream_events(&mut conn, &mut session, hub).await;
    // However it ended, the hub now knows this node is not connected.
    let _ = hub.node_disconnected(node_id, now_ms());
    result
}

async fn stream_events<S>(
    conn: &mut Conn<S>,
    session: &mut NodeSession<'_, crate::store::SqliteStore>,
    hub: &Hub,
) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut acked: Option<u64> = None; // highest contiguous seq not yet acknowledged
    let mut since_ack = 0u32;
    loop {
        let idle;
        match tokio::time::timeout(ACK_IDLE, conn.recv()).await {
            Ok(Ok(Some(f))) => {
                idle = false;
                match (f.stream_id, f.kind) {
                    (STREAM_EVENTS, kinds::EVENT) => match hub.ingest(session, &f.payload)? {
                        Ingest::Applied { acked: a, .. } | Ingest::Duplicate { acked: a } => {
                            acked = Some(a);
                            since_ack += 1;
                        }
                        // Not the next event: tell the node where to start again (nothing was stored).
                        Ingest::Gap { expected } => {
                            let ok = v1::ResumeOk { from_seq: expected };
                            conn.send(&frame(STREAM_CONTROL, kinds::RESUME_OK, ok.encode_to_vec()))
                                .await?;
                        }
                    },
                    (STREAM_CONTROL, kinds::PING) => {
                        conn.send(&frame(STREAM_CONTROL, kinds::PONG, f.payload))
                            .await?;
                    }
                    // Unknown frames are ignored (newer peers may send more).
                    _ => {}
                }
            }
            Ok(Ok(None)) => return Ok(()),
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => idle = true,
        }
        if let Some(a) = acked {
            if idle || since_ack >= ACK_BATCH {
                let ack = v1::EventAck { up_to_seq: a };
                conn.send(&frame(STREAM_EVENTS, kinds::EVENT_ACK, ack.encode_to_vec()))
                    .await?;
                acked = None;
                since_ack = 0;
            }
        }
    }
}

/// Decides whether a bearer token is a signed-in user.
pub trait ClientAuth {
    fn check(&self, token: &str) -> bool;
}

/// The wire form of the hub's view (ADR-0027). Ids are the hub-wide keys.
pub fn snapshot_of(view: &View, pos: u64) -> v1::Snapshot {
    v1::Snapshot {
        pos,
        nodes: view
            .nodes
            .iter()
            .map(|(id, n)| v1::NodeSnapshot {
                node_id: id.clone(),
                hostname: n.hostname.clone(),
                os: n.os.clone(),
                build: n.build.clone(),
                online: n.online,
                agents_running: n.agents_running,
                terminals_open: n.terminals_open,
                memory_used_bytes: n.memory_used_bytes,
                last_event_ms: n.last_event_ms,
            })
            .collect(),
        checkouts: view
            .checkouts
            .iter()
            .map(|(id, (node, project, path))| v1::CheckoutSnapshot {
                checkout_id: id.clone(),
                node_id: node.clone(),
                project_id: project.clone(),
                path: path.clone(),
            })
            .collect(),
        worktrees: view
            .worktrees
            .iter()
            .map(|(id, w)| v1::WorktreeSnapshot {
                worktree_id: id.clone(),
                checkout_id: w.checkout_id.clone(),
                path: w.path.clone(),
                branch: w.branch.clone(),
                files_changed: w.files_changed,
                insertions: w.insertions,
                deletions: w.deletions,
                head_commit: w.head_commit.clone(),
            })
            .collect(),
        agents: view
            .agents
            .iter()
            .map(|(id, a)| v1::AgentSnapshot {
                agent_id: id.clone(),
                node_id: a.node_id.clone(),
                project_id: a.project_id.clone(),
                worktree_id: a.worktree_id.clone(),
                kind: a.kind.clone(),
                state: a.state as i32,
                reported: a.reported,
                session_id: a.session_id.clone(),
                running: a.running,
                exit_code: a.exit_code,
                input_tokens: a.input_tokens,
                output_tokens: a.output_tokens,
            })
            .collect(),
        terminals: view
            .terminals
            .iter()
            .map(|(id, t)| v1::TerminalSnapshot {
                terminal_id: id.clone(),
                node_id: t.node_id.clone(),
                agent_id: t.agent_id.clone(),
                title: t.title.clone(),
            })
            .collect(),
        approvals: view
            .approvals
            .iter()
            .map(|(id, a)| v1::ApprovalSnapshot {
                approval_id: id.clone(),
                agent_id: a.agent_id.clone(),
                action: a.action.clone(),
                detail: a.detail.clone(),
                expires_at_ms: a.expires_at_ms,
            })
            .collect(),
    }
}

/// A signed-in client: waits for `Subscribe`, answers with a snapshot or the backlog, then pushes live events until
/// either side ends. A client that falls too far behind is dropped by the hub and must resubscribe.
pub async fn serve_client<S>(mut conn: Conn<S>, hub: &Hub, first: Frame) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if (first.stream_id, first.kind) != (STREAM_EVENTS, kinds::SUBSCRIBE) {
        return Err(invalid("frame"));
    }
    let sub = v1::Subscribe::decode(first.payload.as_slice()).map_err(|_| invalid("subscribe"))?;
    let subscription = hub.subscribe(sub.after_pos)?;
    if let Some((view, pos)) = &subscription.snapshot {
        conn.send(&frame(
            STREAM_EVENTS,
            kinds::SNAPSHOT,
            snapshot_of(view, *pos).encode_to_vec(),
        ))
        .await?;
    }
    for d in &subscription.backlog {
        send_positioned(&mut conn, d).await?;
    }
    loop {
        // Push whatever is waiting; a closed channel means the hub dropped this client for being too slow.
        loop {
            match subscription.live.try_recv() {
                Ok(d) => send_positioned(&mut conn, &d).await?,
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return Ok(()),
            }
        }
        // Watching the connection too: a client that went away ends this task.
        match tokio::time::timeout(PUSH, conn.recv()).await {
            Ok(Ok(Some(_))) | Err(_) => {}
            Ok(Ok(None)) => return Ok(()),
            Ok(Err(e)) => return Err(e.into()),
        }
    }
}

async fn send_positioned<S>(conn: &mut Conn<S>, d: &crate::hub::Delivered) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let p = v1::PositionedEvent {
        pos: d.pos,
        event: Some(d.event.clone()),
    };
    conn.send(&frame(
        STREAM_EVENTS,
        kinds::POSITIONED_EVENT,
        p.encode_to_vec(),
    ))
    .await?;
    Ok(())
}

fn error_response(request_id: &str, e: &Error) -> v1::CommandResponse {
    v1::CommandResponse {
        request_id: request_id.to_string(),
        result: Some(command_response::Result::Err(e.to_info())),
    }
}

/// An unauthenticated peer: one `EnrollRequest`, one answer. The code is single use, short lived and rate limited
/// ([`Lockout`], shared by all connections); a refusal never says whether the code was wrong or expired.
pub async fn serve_enroll<S>(
    mut conn: Conn<S>,
    hub: &Hub,
    ca: &Ca,
    lockout: &Arc<Lockout>,
    first: Frame,
) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if (first.stream_id, first.kind) != (STREAM_COMMANDS, 0) {
        return Err(invalid("frame"));
    }
    let req =
        v1::CommandRequest::decode(first.payload.as_slice()).map_err(|_| invalid("request"))?;
    let Some(command_request::Command::Enroll(e)) = req.command else {
        // Nothing but enrollment is available without a certificate.
        let resp = error_response(&req.request_id, &Error::new(ErrorCode::AuthUnauthenticated));
        conn.send(&frame(STREAM_COMMANDS, 1, resp.encode_to_vec()))
            .await?;
        return Err(ServeError::Refused);
    };
    let mut enrollment = Enrollment::with_lockout(hub.store(), Arc::clone(lockout));
    let result = enrollment.redeem(ca, &e.code, &e.node_id, &e.csr_pem, now_ms());
    let resp = match result {
        Ok(issued) => v1::CommandResponse {
            request_id: req.request_id.clone(),
            result: Some(command_response::Result::Ok(v1::CommandOk {
                result: Some(command_ok::Result::Enroll(v1::EnrollResponse {
                    cert_pem: issued.cert_pem,
                    ca_pem: ca.cert_pem().to_string(),
                    not_after_ms: now_ms() + crate::ca::DEVICE_CERT_DAYS as u64 * 86_400_000,
                })),
            })),
        },
        Err(EnrollError::LockedOut { .. }) => error_response(
            &req.request_id,
            &Error::new(ErrorCode::AuthForbidden).with("action", "enroll"),
        ),
        // Wrong, expired, used, or a request the CA refused: the same answer.
        Err(_) => error_response(&req.request_id, &Error::new(ErrorCode::AuthUnauthenticated)),
    };
    conn.send(&frame(STREAM_COMMANDS, 1, resp.encode_to_vec()))
        .await?;
    conn.close().await;
    Ok(())
}

/// Everything the connection handler needs.
pub struct ServerState {
    pub hub: Arc<Hub>,
    pub support: Support,
    /// Present when this hub enrolls nodes.
    pub ca: Option<Ca>,
    pub lockout: Arc<Lockout>,
    pub auth: Box<dyn ClientAuth + Send + Sync>,
}

/// A peer with no certificate: its first frame says what it wants, and the answer is yes only for what it proved.
/// An enrollment needs nothing but the code; a subscription needs a bearer token the hub accepts.
pub async fn serve_unauthenticated<S>(
    mut conn: Conn<S>,
    token: Option<&str>,
    state: &ServerState,
) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let Some(first) = conn.recv().await? else {
        return Ok(());
    };
    match (first.stream_id, first.kind) {
        (STREAM_COMMANDS, 0) => match &state.ca {
            Some(ca) => serve_enroll(conn, &state.hub, ca, &state.lockout, first).await,
            None => Err(ServeError::Refused),
        },
        (STREAM_EVENTS, kinds::SUBSCRIBE) if token.is_some_and(|t| state.auth.check(t)) => {
            serve_client(conn, &state.hub, first).await
        }
        _ => Err(ServeError::Refused),
    }
}

/// One accepted TCP connection, with TLS when `tls` is given. A client certificate that verifies and is not revoked
/// makes it a node (the certificate names it); no certificate makes it an unauthenticated peer.
pub async fn handle_connection(
    state: Arc<ServerState>,
    tcp: tokio::net::TcpStream,
    tls: Option<tokio_rustls::TlsAcceptor>,
) {
    let _ = handle(&state, tcp, tls).await;
}

async fn handle(
    state: &ServerState,
    tcp: tokio::net::TcpStream,
    tls: Option<tokio_rustls::TlsAcceptor>,
) -> Result<(), ServeError> {
    let Some(acceptor) = tls else {
        // Without TLS nothing can prove it is a node: only the token path exists (development and tests).
        let (conn, token) = agora_protocol::ws::accept_with_token(tcp).await?;
        return serve_unauthenticated(conn, token.as_deref(), state).await;
    };
    let stream = acceptor
        .accept(tcp)
        .await
        .map_err(|e| ServeError::Transport(TransportError::Ws(e.to_string())))?;
    let identity = crate::tls::identity(stream.get_ref().1.peer_certificates());
    let (conn, token) = agora_protocol::ws::accept_with_token(stream).await?;
    match identity {
        Some(id) => {
            let active = Enrollment::new(state.hub.store())
                .is_active(&id.serial_hex)
                .unwrap_or(false);
            if !active {
                return Err(ServeError::Protocol(
                    Error::new(ErrorCode::AuthDeviceRevoked).with("device_id", id.node_id.as_str()),
                ));
            }
            serve_node(conn, &id.node_id, &state.hub, &state.support).await
        }
        None => serve_unauthenticated(conn, token.as_deref(), state).await,
    }
}

/// Accepts connections until the listener fails; each runs on its own task.
pub async fn run(
    listener: tokio::net::TcpListener,
    state: Arc<ServerState>,
    tls: Option<tokio_rustls::TlsAcceptor>,
) {
    while let Ok((tcp, _)) = listener.accept().await {
        tokio::spawn(handle_connection(Arc::clone(&state), tcp, tls.clone()));
    }
}
