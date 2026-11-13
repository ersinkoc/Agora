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

use agora_protocol::frame::{
    kinds, FIRST_DYNAMIC_STREAM, STREAM_CLOSE, STREAM_COMMANDS, STREAM_CONTROL, STREAM_EVENTS,
};
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
/// A node connection on which nothing at all arrived for this long is closed and the node recorded offline (`NodeLink { up: false,
/// reason: silent }`, ADR-0070), although the operating system may still call the connection open (a frozen node, a laptop that
/// slept, a NAT that forgot the connection). When the node dials again, the new connection records it online by itself.
/// A node pings every 10 s (`agora_node::uplink_client::PING_EVERY`), so an idle but healthy node is never dropped; the node
/// uses the same deadline for the hub (`SILENCE_DEADLINE`). Two and a half pings: one late ping is not enough.
pub const NODE_SILENCE: Duration = Duration::from_millis(25_000);

#[derive(Debug)]
pub enum ServeError {
    Transport(TransportError),
    Protocol(Error),
    Store(StoreError),
    /// The peer did something this role may not do.
    Refused,
    /// Nothing arrived from the peer for its deadline, or a frame could not be sent for as long (see [`NODE_SILENCE`]).
    Silent,
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
/// connection opened, the hub records the node online, and at the end offline (ADR-0070). When the node broke the protocol, the connection ends with the
/// error's stable code as the close reason, so the node can say why instead of seeing a bare reset; the hub's log says it too.
pub async fn serve_node<S>(
    conn: Conn<S>,
    node_id: &str,
    hub: &Arc<Hub>,
    support: &Support,
) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    serve_node_within(conn, node_id, hub, support, NODE_SILENCE).await
}

/// [`serve_node`] with another silence deadline than [`NODE_SILENCE`] (tests use a short one).
pub async fn serve_node_within<S>(
    conn: Conn<S>,
    node_id: &str,
    hub: &Arc<Hub>,
    support: &Support,
    silence: Duration,
) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    serve_node_from(conn, None, node_id, hub, support, silence).await
}

/// [`serve_node_within`] when the connection's first frame was already read (it must be the node's `Hello`; ADR-0061 reads it to
/// tell a one-shot question from a node's conversation).
async fn serve_node_from<S>(
    mut conn: Conn<S>,
    first: Option<Frame>,
    node_id: &str,
    hub: &Arc<Hub>,
    support: &Support,
    silence: Duration,
) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let result = converse_node(&mut conn, first, node_id, hub, support, silence).await;
    match &result {
        Err(ServeError::Protocol(e)) => {
            // Codes and parameters only: this is the hub's log, not a user's screen.
            eprintln!("node refused: {node_id} {} {:?}", e.code.as_str(), e.params);
            conn.close_with(e.code.as_str()).await;
        }
        // Closed, so a node that was only frozen or asleep reads the end of the stream as soon as it runs again and dials at once.
        // Bounded: a peer that reads nothing must not hold this task. It is recorded offline as `silent` (see `converse_node`).
        Err(ServeError::Silent) => {
            eprintln!("node silent: {node_id}");
            let _ = tokio::time::timeout(Duration::from_secs(1), conn.close()).await;
        }
        _ => {}
    }
    result
}

async fn converse_node<S>(
    conn: &mut Conn<S>,
    first: Option<Frame>,
    node_id: &str,
    hub: &Arc<Hub>,
    support: &Support,
    silence: Duration,
) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut session = NodeSession::new(hub.store(), node_id, support);

    let hello_frame = match first {
        Some(f) if (f.stream_id, f.kind) == (STREAM_CONTROL, kinds::HELLO) => f,
        Some(_) => return Err(invalid("frame")),
        None => expect(conn, STREAM_CONTROL, kinds::HELLO).await?,
    };
    let hello = v1::Hello::decode(hello_frame.payload.as_slice()).map_err(|_| invalid("hello"))?;
    let welcome = session.hello(&hello, now_ms())?;
    conn.send(&frame(
        STREAM_CONTROL,
        kinds::WELCOME,
        welcome.encode_to_vec(),
    ))
    .await?;

    let resume = v1::Resume::decode(
        expect(conn, STREAM_CONTROL, kinds::RESUME)
            .await?
            .payload
            .as_slice(),
    )
    .map_err(|_| invalid("resume"))?;
    let ok = session.resume(&resume)?;
    conn.send(&frame(STREAM_CONTROL, kinds::RESUME_OK, ok.encode_to_vec()))
        .await?;

    // ADR-0070: the connection is open, so the hub records the node online (a first connect or a reconnect alike: nothing the node
    // runs is reset) and, when it ends, offline with why. Both come from this certificate-authenticated connection only.
    let verifies_devices = welcome.capabilities.iter().any(|c| c == "device.trust");
    let (registration, commands, generation) =
        hub.node_link_opened(node_id, welcome.capabilities, now_ms());
    // ADR-0061: a node that verifies endorsements gets every one it may have missed while it was away, and every revocation.
    if verifies_devices {
        give_device_changes(hub, node_id);
    }
    let mut last_heard = tokio::time::Instant::now();
    let result = stream_events(
        conn,
        &mut session,
        hub,
        node_id,
        commands,
        silence,
        &mut last_heard,
    )
    .await;
    // However it ended, the router now knows this node is not connected (commands to it fail with `node.offline` at once), unless
    // a newer connection of the node replaced this one: that one is live, and the node is not offline.
    let now = now_ms();
    let heard_ms = now.saturating_sub(last_heard.elapsed().as_millis() as u64);
    let silent = matches!(result, Err(ServeError::Silent));
    if hub.node_link_closed(registration, generation, silent, heard_ms, now) {
        // Forget it as a member of its projects, so it is asked to hand over its local work when it is back (ADR-0039 amendment
        // B); anyone the change makes due is asked now.
        for (node, project) in hub.board_membership(now) {
            ask_board_join(hub, node, project);
        }
    }
    result
}

/// Sends one frame to a node, but never waits longer than `limit` for it: a node that stopped reading fills the socket's buffer,
/// and a send that waits forever would keep its connection from ever being found dead.
async fn put<S>(conn: &mut Conn<S>, f: &Frame, limit: Duration) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    match tokio::time::timeout(limit, conn.send(f)).await {
        Ok(r) => Ok(r?),
        Err(_) => Err(ServeError::Silent),
    }
}

async fn stream_events<S>(
    conn: &mut Conn<S>,
    session: &mut NodeSession<'_, crate::store::SqliteStore>,
    hub: &Arc<Hub>,
    node_id: &str,
    mut commands: tokio::sync::mpsc::UnboundedReceiver<crate::router::ToNode>,
    silence: Duration,
    last_heard: &mut tokio::time::Instant,
) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // Every frame from the node counts as a sign of life; an idle node sends its pings (every 10 s; 15 s before ADR-0070).
    let mut acked: Option<u64> = None; // highest contiguous seq not yet acknowledged
    let mut since_ack = 0u32;
    // Commands sent to this node and not yet answered, by the request id the hub chose.
    let mut waiting: std::collections::HashMap<
        String,
        tokio::sync::oneshot::Sender<v1::CommandResponse>,
    > = std::collections::HashMap::new();
    // Mesh posts this node sent for another node (ADR-0031): answers come back here from tasks that wait on the target.
    let (posted_tx, mut posted) = tokio::sync::mpsc::unbounded_channel::<v1::CommandResponse>();
    let posts_open = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    // Handoffs the board accepted but the receiving node did not take: undone here, where the hub is at hand.
    let (revert_tx, mut reverts) = tokio::sync::mpsc::unbounded_channel::<crate::board::Handoff>();
    loop {
        if last_heard.elapsed() >= silence {
            return Err(ServeError::Silent);
        }
        let idle;
        let received = tokio::select! {
            r = tokio::time::timeout(ACK_IDLE, conn.recv()) => r,
            Some(answer) = posted.recv() => {
                put(conn, &frame(STREAM_COMMANDS, 1, answer.encode_to_vec()), silence).await?;
                continue;
            }
            Some(handoff) = reverts.recv() => {
                hub.board_revert(&handoff, now_ms());
                continue;
            }
            Some(to_node) = commands.recv() => {
                match to_node {
                    crate::router::ToNode::Command { request, reply, stream } => {
                        waiting.retain(|_, s| !s.is_closed());
                        let payload = request.encode_to_vec();
                        waiting.insert(request.request_id.clone(), reply);
                        put(conn, &frame(stream, 0, payload), silence).await?;
                    }
                    crate::router::ToNode::Close { stream } => {
                        put(conn, &frame(stream, STREAM_CLOSE, Vec::new()), silence).await?;
                    }
                }
                continue;
            }
        };
        match received {
            Ok(Ok(Some(f))) => {
                idle = false;
                *last_heard = tokio::time::Instant::now();
                match (f.stream_id, f.kind) {
                    (STREAM_EVENTS, kinds::EVENT) => match hub.ingest(session, &f.payload)? {
                        Ingest::Applied { acked: a, .. } => {
                            acked = Some(a);
                            since_ack += 1;
                            for (node, project) in hub.board_membership(now_ms()) {
                                ask_board_join(hub, node, project);
                            }
                        }
                        // The node could not replay (ADR-0054): this event is its new base and the numbers before it are lost.
                        // Recorded in the store with the event; said in the log and the audit trail too, so it is never silent.
                        Ingest::Resynced {
                            acked: a,
                            from_seq,
                            to_seq,
                            ..
                        } => {
                            acked = Some(a);
                            since_ack += 1;
                            eprintln!("node resync: {node_id} skipped {from_seq}..={to_seq}");
                            let _ = crate::audit::Audit::new(hub.store()).record(
                                now_ms(),
                                crate::ca::RESERVED_HUB_ID,
                                "node.resynced",
                                node_id,
                                &format!("{from_seq}..={to_seq}"),
                            );
                            for (node, project) in hub.board_membership(now_ms()) {
                                ask_board_join(hub, node, project);
                            }
                        }
                        Ingest::Duplicate { acked: a } => {
                            acked = Some(a);
                            since_ack += 1;
                        }
                        // Not the next event: tell the node where to start again (nothing was stored).
                        Ingest::Gap { expected } => {
                            let ok = v1::ResumeOk { from_seq: expected };
                            put(
                                conn,
                                &frame(STREAM_CONTROL, kinds::RESUME_OK, ok.encode_to_vec()),
                                silence,
                            )
                            .await?;
                        }
                    },
                    // The node cannot replay from where the hub asked (ADR-0054): its next new event is the new base.
                    (STREAM_CONTROL, kinds::SNAPSHOT_REQUIRED) => {
                        session.snapshot_required(now_ms())?
                    }
                    (STREAM_CONTROL, kinds::PING) => {
                        put(
                            conn,
                            &frame(STREAM_CONTROL, kinds::PONG, f.payload),
                            silence,
                        )
                        .await?;
                    }
                    // A node asks the hub to carry one mesh message to an agent on another node (ADR-0031). Nothing else
                    // is accepted from a node on this stream.
                    (STREAM_COMMANDS, 0) => {
                        let posted_tx = posted_tx.clone();
                        let answer = v1::CommandRequest::decode(f.payload.as_slice())
                            .map_err(|_| invalid("request"))?;
                        post_for_node(
                            hub,
                            node_id,
                            answer,
                            &posts_open,
                            posted_tx,
                            revert_tx.clone(),
                        );
                    }
                    // An answer to a routed command: hand it to whoever asked. A garbled answer is dropped, and
                    // the asker's deadline turns it into `node.timeout`.
                    (STREAM_COMMANDS, 1) => {
                        if let Ok(resp) = v1::CommandResponse::decode(f.payload.as_slice()) {
                            if let Some(reply) = waiting.remove(&resp.request_id) {
                                let _ = reply.send(resp);
                            }
                        }
                    }
                    // Terminal frames of a relayed subscription: passed to the client, never stored or decoded.
                    (stream, kind) if stream >= FIRST_DYNAMIC_STREAM => {
                        hub.router().relay_frame(node_id, stream, kind, f.payload);
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
                put(
                    conn,
                    &frame(STREAM_EVENTS, kinds::EVENT_ACK, ack.encode_to_vec()),
                    silence,
                )
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
    /// What the holder of a live token may do (ADR-0049); `None` when it is not one. The default is the owner's, which is what a
    /// hub with one person has always meant.
    fn access(&self, token: &str) -> Option<crate::users::Access> {
        self.check(token).then(|| crate::users::Access {
            role: crate::users::Role::Owner,
            user: "owner".into(),
            projects: None,
            session: String::new(),
        })
    }
}

/// The wire form of the hub's view (ADR-0027). Ids are the hub-wide keys.
pub fn snapshot_of(view: &View, pos: u64) -> v1::Snapshot {
    v1::Snapshot {
        workspaces: view.workspaces.values().cloned().collect(),
        agent_workspaces: view
            .agent_workspaces
            .iter()
            .map(|(agent, workspace)| v1::AgentWorkspaceBound {
                agent_id: agent.clone(),
                workspace_id: workspace.clone(),
            })
            .collect(),
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
                trusted_device_ids: view.trusted_devices.get(id).cloned().unwrap_or_default(),
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
            .filter(|(_, a)| !a.forgotten)
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
                parent_agent_id: a.parent_agent_id.clone(),
                interrupted: a.interrupted.is_some(),
                resumable: a.interrupted.as_ref().is_some_and(|i| i.resumable),
                interrupted_at_ms: a.interrupted.as_ref().map_or(0, |i| i.at_ms),
                folder: a
                    .interrupted
                    .as_ref()
                    .map(|i| i.folder.clone())
                    .unwrap_or_default(),
                still_running: a.interrupted.as_ref().is_some_and(|i| i.still_running),
                launch_options: a.launch_options.clone(),
                skips_permissions: a.skips_permissions,
                ..session_snapshot(&a.session)
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
        tasks: view
            .tasks
            .iter()
            .map(|(key, t)| v1::TaskSnapshot {
                task_id: key
                    .rsplit_once('/')
                    .map_or(key.as_str(), |(_, id)| id)
                    .to_string(),
                project_id: t.project_id.clone(),
                title: t.title.clone(),
                claimed_by_agent_id: t.owner.clone(),
                done: t.done,
            })
            .collect(),
        leases: view
            .leases
            .values()
            .map(|(agent, path, exp)| v1::LeaseSnapshot {
                agent_id: agent.clone(),
                path: path.clone(),
                expires_at_ms: *exp,
            })
            .collect(),
    }
}

/// A signed-in client: waits for `Subscribe`, answers with a snapshot or the backlog, then pushes live events until
/// either side ends. A client that falls too far behind is dropped by the hub and must resubscribe.
pub async fn serve_client<S>(conn: Conn<S>, hub: &Hub, first: Frame) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    serve_client_as(conn, hub, first, &|| {
        Some(crate::users::Access {
            role: crate::users::Role::Owner,
            user: "owner".into(),
            projects: None,
            session: String::new(),
        })
    })
    .await
}

/// [`serve_client`] for a person with a role (ADR-0049). `role` is asked for every command, so a demotion or a removed person takes
/// effect at once; `None` ends the connection.
pub async fn serve_client_as<S>(
    conn: Conn<S>,
    hub: &Hub,
    first: Frame,
    access: &(dyn Fn() -> Option<crate::users::Access> + Sync),
) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    serve_client_while(conn, hub, first, access, &|| true).await
}

/// [`serve_client_as`] that also ends the connection as soon as `valid` turns false (a session that expired, was signed out or
/// revoked). Only the client's part is raced against it, so commands already on its way still get their outcome recorded.
async fn serve_client_while<S>(
    conn: Conn<S>,
    hub: &Hub,
    first: Frame,
    access: &(dyn Fn() -> Option<crate::users::Access> + Sync),
    valid: &(dyn Fn() -> bool + Sync),
) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut inflight: Vec<Inflight> = Vec::new();
    // The connection is moved in, so it is closed when the client's part ends, whichever way it ends.
    let result = tokio::select! {
        r = serve_client_loop(conn, hub, first, access, &mut inflight) => r,
        () = still(valid) => Err(ServeError::Refused),
    };
    finish_unanswered(hub, inflight).await;
    result
}

/// Commands still on their way when the client left (a reload, a closed tab) or was refused: nobody will read their answers, but
/// the audit trail (ADR-0049) says how each one really ended, the node's answer, `node.offline` or `node.timeout`, instead of
/// staying `sent` for good (L17).
async fn finish_unanswered(hub: &Hub, mut inflight: Vec<Inflight>) {
    // answer_ready's swap_remove can put a newer request before an older one. Waiting on the later deadline first would let
    // an older request's late reply queue up and beat its expired timeout when we finally poll it. Visit the earliest deadline first.
    inflight.sort_unstable_by_key(|item| item.deadline);
    for item in inflight {
        let Some(id) = item.audit else { continue };
        let outcome = match tokio::time::timeout_at(item.deadline, item.reply).await {
            Ok(Ok(r)) => match r.result {
                Some(command_response::Result::Err(info)) => info.code,
                _ => "ok".to_string(),
            },
            Ok(Err(_)) => ErrorCode::NodeOffline.as_str().to_string(),
            Err(_) => ErrorCode::NodeTimeout.as_str().to_string(),
        };
        let _ = crate::audit::Audit::new(hub.store()).finish(id, &outcome);
    }
}

async fn serve_client_loop<S>(
    mut conn: Conn<S>,
    hub: &Hub,
    first: Frame,
    access: &(dyn Fn() -> Option<crate::users::Access> + Sync),
    inflight: &mut Vec<Inflight>,
) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if (first.stream_id, first.kind) != (STREAM_EVENTS, kinds::SUBSCRIBE) {
        return Err(invalid("frame"));
    }
    let sub = v1::Subscribe::decode(first.payload.as_slice()).map_err(|_| invalid("subscribe"))?;
    let current = access().ok_or(ServeError::Refused)?;
    // A member or viewer gets a snapshot cut to their projects, whatever position they ask from (a replay from the log would be
    // everyone's), and from then on only the events that are theirs (ADR-0049).
    let restricted = current.projects.is_some();
    let subscription = hub.subscribe(if restricted { None } else { sub.after_pos })?;
    let mut scope = None;
    if let Some((view, pos)) = &subscription.snapshot {
        let body = match &current.projects {
            Some(projects) => {
                let s = crate::scope::Scope::from_view(projects.clone(), view);
                let cut = s.filter_view(view);
                scope = Some(s);
                snapshot_of(&cut, *pos)
            }
            None => snapshot_of(view, *pos),
        };
        conn.send(&frame(STREAM_EVENTS, kinds::SNAPSHOT, body.encode_to_vec()))
            .await?;
    }
    for d in &subscription.backlog {
        send_positioned(&mut conn, d).await?;
    }
    let mut checked = tokio::time::Instant::now();
    let (relays, mut relay_frames) = hub.router().client();
    loop {
        answer_ready(&mut conn, inflight, hub).await?;
        // Terminal frames from nodes, and streams the hub ended.
        while let Ok(rf) = relay_frames.try_recv() {
            conn.send(&frame(rf.stream, rf.kind, rf.payload)).await?;
        }
        for s in relays.take_closed() {
            conn.send(&frame(s, STREAM_CLOSE, Vec::new())).await?;
        }
        // A role or a grant that changed ends this connection; the client reconnects into what it may now see.
        if checked.elapsed() >= Duration::from_secs(1) {
            checked = tokio::time::Instant::now();
            if access().as_ref() != Some(&current) {
                return Err(ServeError::Refused);
            }
        }
        // Push whatever is waiting; a closed channel means the hub dropped this client for being too slow.
        loop {
            match subscription.live.try_recv() {
                Ok(d) => {
                    if scope.as_mut().is_none_or(|s| s.admit(&d.event)) {
                        send_positioned(&mut conn, &d).await?;
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return Ok(()),
            }
        }
        // Watching the connection too: a client that went away ends this task.
        // A terminal frame from a node wakes this loop at once (input and output latency must not wait for the next tick).
        let received = tokio::select! {
            r = tokio::time::timeout(PUSH, conn.recv()) => r,
            Some(rf) = relay_frames.recv() => {
                conn.send(&frame(rf.stream, rf.kind, rf.payload)).await?;
                continue;
            }
        };
        match received {
            Ok(Ok(Some(f))) => match (f.stream_id, f.kind) {
                // A command; a terminal subscription arrives on the dynamic stream the client chose for it.
                (stream, 0) if stream == STREAM_COMMANDS || stream >= FIRST_DYNAMIC_STREAM => {
                    let Some(who) = access() else {
                        return Err(ServeError::Refused);
                    };
                    if who != current {
                        return Err(ServeError::Refused);
                    }
                    forward_command(
                        &mut conn,
                        hub,
                        &relays,
                        inflight,
                        stream,
                        &f.payload,
                        Caller {
                            who: &who,
                            scope: scope.as_ref(),
                        },
                    )
                    .await?;
                }
                (stream, STREAM_CLOSE) if stream >= FIRST_DYNAMIC_STREAM => relays.close(stream),
                // A window checks that the hub is still there (protocol section 1): answered at once, whatever else is queued.
                (STREAM_CONTROL, kinds::PING) => {
                    conn.send(&frame(STREAM_CONTROL, kinds::PONG, f.payload))
                        .await?;
                }
                _ => {}
            },
            Err(_) => {}
            Ok(Ok(None)) => return Ok(()),
            Ok(Err(e)) => return Err(e.into()),
        }
    }
}

/// Longest a client waits for a node's answer to a routed command.
const COMMAND_DEADLINE: Duration = Duration::from_secs(30);
/// Routed commands one client may have waiting at once.
const MAX_INFLIGHT: usize = 256;
/// Mesh posts one node may have waiting at once (ADR-0031).
const MAX_NODE_POSTS: usize = 64;

/// A routed command whose answer has not come back yet.
struct Inflight {
    client_request_id: String,
    node: String,
    /// A terminal subscription: the stream on the node's connection that was reserved for it.
    relay: Option<u32>,
    reply: tokio::sync::oneshot::Receiver<v1::CommandResponse>,
    deadline: tokio::time::Instant,
    /// The audit row that says this command is on its way (ADR-0049); finished when the answer comes.
    audit: Option<i64>,
}

/// Who is asking and what they may reach (ADR-0049).
struct Caller<'a> {
    who: &'a crate::users::Access,
    scope: Option<&'a crate::scope::Scope>,
}

async fn forward_command<S>(
    conn: &mut Conn<S>,
    hub: &Hub,
    relays: &crate::router::ClientRelays,
    inflight: &mut Vec<Inflight>,
    stream_id: u32,
    payload: &[u8],
    caller: Caller<'_>,
) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let req = v1::CommandRequest::decode(payload).map_err(|_| invalid("request"))?;
    let client_request_id = req.request_id.clone();
    let refuse = |e: Error| error_response(&client_request_id, &e);
    let Caller { who, scope } = caller;
    // ADR-0061: a person's own devices, answered (or carried) by the hub; a node decides for itself what it trusts.
    if let Some(resp) = device_command(hub, who, &req) {
        conn.send(&frame(STREAM_COMMANDS, 1, resp.encode_to_vec()))
            .await?;
        return Ok(());
    }
    // ADR-0049: a viewer's commands stop here, before any node hears of them. (The node still decides what its devices may do.)
    let may = req.command.as_ref().is_some_and(|c| {
        (who.role != crate::users::Role::Viewer || crate::users::viewer_may(c))
            && scope.is_none_or(|s| s.command_allowed(&req.target_node, c))
    });
    if !may {
        let action = command_name(&req);
        audit(hub, who, &action, &req.target_node, "auth.forbidden");
        conn.send(&frame(
            STREAM_COMMANDS,
            1,
            refuse(Error::new(ErrorCode::AuthForbidden).with("action", action.as_str()))
                .encode_to_vec(),
        ))
        .await?;
        return Ok(());
    }
    // ADR-0053: the hub forgets a closed agent. It names the agent by its hub-wide key, so it names no node.
    if let Some(command_request::Command::AgentForget(f)) = &req.command {
        let node = f.agent_id.split('/').next().unwrap_or_default().to_string();
        let resp = if req.target_node.is_empty() {
            match hub.forget_agent(&f.agent_id, now_ms()) {
                Ok(()) => {
                    audit(hub, who, "agent.forget", &node, "ok");
                    v1::CommandResponse {
                        request_id: client_request_id,
                        result: Some(command_response::Result::Ok(v1::CommandOk {
                            result: Some(command_ok::Result::Empty(v1::Empty {})),
                        })),
                    }
                }
                Err(e) => {
                    audit(hub, who, "agent.forget", &node, e.code.as_str());
                    refuse(e)
                }
            }
        } else {
            refuse(Error::new(ErrorCode::ProtocolInvalidMessage).with("field", "target_node"))
        };
        conn.send(&frame(STREAM_COMMANDS, 1, resp.encode_to_vec()))
            .await?;
        return Ok(());
    }
    // Answered by the hub itself from its own view and prices (ADR-0033); it names no node.
    if matches!(req.command, Some(command_request::Command::UsageReport(_))) {
        let resp = if req.target_node.is_empty() {
            v1::CommandResponse {
                request_id: client_request_id,
                result: Some(command_response::Result::Ok(v1::CommandOk {
                    result: Some(command_ok::Result::UsageReport(hub.usage_report())),
                })),
            }
        } else {
            refuse(Error::new(ErrorCode::ProtocolInvalidMessage).with("field", "target_node"))
        };
        conn.send(&frame(STREAM_COMMANDS, 1, resp.encode_to_vec()))
            .await?;
        return Ok(());
    }
    let outcome = (|| {
        let node = crate::router::forwardable(&req)?.to_string();
        if !hub.node_known(&node) {
            return Err(Error::new(ErrorCode::NodeUnknown).with("node_id", node.as_str()));
        }
        if inflight.len() >= MAX_INFLIGHT {
            return Err(Error::new(ErrorCode::NodeResourceLimit)
                .with("resource", "requests")
                .with("limit", 256_i64));
        }
        let subscribing = matches!(
            req.command,
            Some(
                command_request::Command::TerminalSubscribe(_)
                    | command_request::Command::ChatSubscribe(_)
            )
        );
        if subscribing != (stream_id >= FIRST_DYNAMIC_STREAM) {
            // A terminal stream needs a stream of its own, and nothing else may use one.
            return Err(Error::new(ErrorCode::ProtocolInvalidMessage).with("field", "stream_id"));
        }
        let (reply, relay) = if subscribing {
            let node_stream = hub.router().open_relay(&node, relays, stream_id);
            match hub.router().send_on(&node, req.clone(), node_stream) {
                Ok(r) => (r, Some(node_stream)),
                Err(e) => {
                    hub.router().drop_relay(&node, node_stream);
                    return Err(e);
                }
            }
        } else {
            (hub.router().send(&node, req.clone())?, None)
        };
        Ok(Inflight {
            client_request_id: client_request_id.clone(),
            node,
            relay,
            reply,
            deadline: tokio::time::Instant::now() + COMMAND_DEADLINE,
            audit: None,
        })
    })();
    match outcome {
        Ok(mut i) => {
            i.audit = audit(hub, who, &audit_action(&req), &i.node, "sent");
            inflight.push(i);
        }
        Err(e) => {
            audit(
                hub,
                who,
                &audit_action(&req),
                &req.target_node,
                e.code.as_str(),
            );
            conn.send(&frame(STREAM_COMMANDS, 1, refuse(e).encode_to_vec()))
                .await?;
        }
    }
    Ok(())
}

/// Sends the answers that have arrived (or timed out) to the client, under the request ids the client chose.
async fn answer_ready<S>(
    conn: &mut Conn<S>,
    inflight: &mut Vec<Inflight>,
    hub: &Hub,
) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    use tokio::sync::oneshot::error::TryRecvError;
    let mut i = 0;
    while i < inflight.len() {
        let item = &mut inflight[i];
        let resp = match item.reply.try_recv() {
            Ok(mut r) => {
                r.request_id.clone_from(&item.client_request_id);
                Some(r)
            }
            // The node's connection ended before it answered.
            Err(TryRecvError::Closed) => Some(error_response(
                &item.client_request_id,
                &Error::new(ErrorCode::NodeOffline).with("node_id", item.node.as_str()),
            )),
            Err(TryRecvError::Empty) if tokio::time::Instant::now() >= item.deadline => {
                Some(error_response(
                    &item.client_request_id,
                    &Error::new(ErrorCode::NodeTimeout).with("node_id", item.node.as_str()),
                ))
            }
            Err(TryRecvError::Empty) => None,
        };
        match resp {
            Some(r) => {
                let done = inflight.swap_remove(i);
                if let Some(id) = done.audit {
                    let outcome = match &r.result {
                        Some(command_response::Result::Err(info)) => info.code.as_str(),
                        _ => "ok",
                    };
                    let _ = crate::audit::Audit::new(hub.store()).finish(id, outcome);
                }
                // A subscription the node refused (or never answered) leaves no relay behind.
                if let (Some(stream), true) = (
                    done.relay,
                    !matches!(r.result, Some(command_response::Result::Ok(_))),
                ) {
                    hub.router().drop_relay(&done.node, stream);
                }
                conn.send(&frame(STREAM_COMMANDS, 1, r.encode_to_vec()))
                    .await?;
            }
            None => i += 1,
        }
    }
    Ok(())
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

/// A node's `MeshPost` for an agent on another node: forwarded to that node like a client command (ADR-0029) with `origin`
/// set from the connection, whatever the node wrote. The answer (or an error) is sent on `answers`.
fn post_for_node(
    hub: &Hub,
    node_id: &str,
    req: v1::CommandRequest,
    open: &Arc<std::sync::atomic::AtomicUsize>,
    answers: tokio::sync::mpsc::UnboundedSender<v1::CommandResponse>,
    reverts: tokio::sync::mpsc::UnboundedSender<crate::board::Handoff>,
) {
    use std::sync::atomic::Ordering;
    let rid = req.request_id.clone();
    let refuse = |e: Error| {
        let _ = answers.send(error_response(&rid, &e));
    };
    let post = match req.command {
        Some(command_request::Command::MeshPost(post)) => post,
        Some(command_request::Command::BoardCall(call)) => {
            return board_for_node(hub, node_id, &rid, &call, open, answers, reverts);
        }
        _ => {
            return refuse(Error::new(ErrorCode::ProtocolInvalidMessage).with("field", "command"));
        }
    };
    let target = post
        .to
        .split_once('/')
        .filter(|(n, a)| !n.is_empty() && !a.is_empty() && !a.contains('/') && *n != node_id);
    let Some((target, agent)) = target else {
        return refuse(Error::new(ErrorCode::ProtocolInvalidMessage).with("field", "to"));
    };
    if !hub.node_known(target) {
        return refuse(Error::new(ErrorCode::NodeUnknown).with("node_id", target));
    }
    if open.load(Ordering::Relaxed) >= MAX_NODE_POSTS {
        return refuse(
            Error::new(ErrorCode::NodeResourceLimit)
                .with("resource", "requests")
                .with("limit", MAX_NODE_POSTS as i64),
        );
    }
    let forwarded = v1::CommandRequest {
        request_id: String::new(),
        idempotency_key: String::new(),
        target_node: String::new(),
        command: Some(command_request::Command::MeshPost(v1::MeshPost {
            to: agent.to_string(),
            body: post.body.clone(),
            // The hub's word, not the node's: a node cannot speak for another node.
            origin: node_id.to_string(),
            from_agent: post.from_agent.clone(),
            refs: post.refs.clone(),
        })),
    };
    let reply = match hub.router().send(target, forwarded) {
        Ok(r) => r,
        Err(e) => return refuse(e),
    };
    open.fetch_add(1, Ordering::Relaxed);
    let open = Arc::clone(open);
    let target = target.to_string();
    tokio::spawn(async move {
        let resp = match tokio::time::timeout(COMMAND_DEADLINE, reply).await {
            Ok(Ok(mut r)) => {
                r.request_id.clone_from(&rid);
                r
            }
            Ok(Err(_)) => error_response(
                &rid,
                &Error::new(ErrorCode::NodeOffline).with("node_id", target.as_str()),
            ),
            Err(_) => error_response(
                &rid,
                &Error::new(ErrorCode::NodeTimeout).with("node_id", target.as_str()),
            ),
        };
        open.fetch_sub(1, Ordering::Relaxed);
        let _ = answers.send(resp);
    });
}

/// ADR-0039 amendment B: asks `node` to hand over its local work for `project`, which just became shared, and puts the answer on
/// the board whenever it comes: the node let the work go when it answered, so a late answer is still applied (only the hold on
/// grants ends after `JOIN_WAIT_MS`). A node that is gone or too old to know `BoardJoin` is simply not waited for.
fn ask_board_join(hub: &Arc<Hub>, node: String, project: String) {
    let req = v1::CommandRequest {
        request_id: String::new(),
        idempotency_key: String::new(),
        target_node: String::new(),
        command: Some(command_request::Command::BoardJoin(v1::BoardJoin {
            project_id: project.clone(),
        })),
    };
    let reply = match hub.router().send(&node, req) {
        Ok(r) => r,
        Err(_) => {
            hub.board_join_settled(&node, &project);
            return;
        }
    };
    let hub = Arc::clone(hub);
    tokio::spawn(async move {
        // No deadline: the reply ends when the node answers or its connection goes (the router drops the sender).
        match reply.await {
            Ok(v1::CommandResponse {
                result:
                    Some(command_response::Result::Ok(v1::CommandOk {
                        result: Some(v1::command_ok::Result::BoardJoin(work)),
                    })),
                ..
            }) => {
                for n in hub.board_joined(&node, &project, &work, now_ms()) {
                    board_notice(&hub, &n);
                }
            }
            _ => hub.board_join_settled(&node, &project),
        }
    });
}

/// Sends one board notice to its agent as a mesh message from the hub (best effort, like any mesh post).
fn board_notice(hub: &Hub, n: &crate::board::Notice) {
    let post = v1::CommandRequest {
        request_id: String::new(),
        idempotency_key: String::new(),
        target_node: String::new(),
        command: Some(command_request::Command::MeshPost(v1::MeshPost {
            to: n.to_agent.clone(),
            body: n.body.clone(),
            origin: crate::ca::RESERVED_HUB_ID.to_string(),
            from_agent: "board".to_string(),
            refs: Vec::new(),
        })),
    };
    let _ = hub.router().send(&n.to_node, post);
}

/// A node's `BoardCall` (ADR-0039): the board decides, the answer goes back to the node. A handoff is also passed to the
/// receiving agent's node, as a mesh message from the giver, and the node's answer waits for that.
fn board_for_node(
    hub: &Hub,
    node_id: &str,
    rid: &str,
    call: &v1::BoardCall,
    open: &Arc<std::sync::atomic::AtomicUsize>,
    answers: tokio::sync::mpsc::UnboundedSender<v1::CommandResponse>,
    reverts: tokio::sync::mpsc::UnboundedSender<crate::board::Handoff>,
) {
    use crate::board::Verdict;
    use std::sync::atomic::Ordering;
    // A handoff waits for the receiving node, so it counts against the same cap as a mesh post.
    if call.tool == "handoff" && open.load(Ordering::Relaxed) >= MAX_NODE_POSTS {
        let e = Error::new(ErrorCode::NodeResourceLimit)
            .with("resource", "requests")
            .with("limit", MAX_NODE_POSTS as i64);
        let _ = answers.send(error_response(rid, &e));
        return;
    }
    let a = hub.board_call(node_id, call, now_ms());
    let ok = |shared: bool, result: String, is_error: bool| v1::CommandResponse {
        request_id: rid.to_string(),
        result: Some(command_response::Result::Ok(v1::CommandOk {
            result: Some(v1::command_ok::Result::BoardResult(v1::BoardResult {
                shared,
                result_json: result,
                is_error,
            })),
        })),
    };
    let (result, is_error) = match a.verdict {
        Verdict::NotShared => {
            let _ = answers.send(ok(false, String::new(), false));
            return;
        }
        Verdict::Invalid(e) => {
            let _ = answers.send(error_response(rid, &e));
            return;
        }
        Verdict::Done { result, is_error } => (result.to_string(), is_error),
    };
    if let Some(f) = a.fanout {
        // Best effort, like any mesh post: a receiver that is gone just does not get its copy.
        for (to_node, to_agent) in f.targets {
            let post = v1::CommandRequest {
                request_id: String::new(),
                idempotency_key: String::new(),
                target_node: String::new(),
                command: Some(command_request::Command::MeshPost(v1::MeshPost {
                    to: to_agent,
                    body: f.body.clone(),
                    origin: f.from_node.clone(),
                    from_agent: f.from_agent.clone(),
                    refs: f.refs.clone(),
                })),
            };
            let _ = hub.router().send(&to_node, post);
        }
        let _ = answers.send(ok(true, result, is_error));
        return;
    }
    let Some(handoff) = a.handoff else {
        let _ = answers.send(ok(true, result, is_error));
        return;
    };
    let h = &handoff;
    // The message for the receiver: sent like a node's own `MeshPost`, with `origin` from the hub.
    let forwarded = v1::CommandRequest {
        request_id: String::new(),
        idempotency_key: String::new(),
        target_node: String::new(),
        command: Some(command_request::Command::MeshPost(v1::MeshPost {
            to: h.to_agent.clone(),
            body: h.body.clone(),
            origin: h.from_node.clone(),
            from_agent: h.from_agent.clone(),
            refs: h.refs.clone(),
        })),
    };
    let reply = match hub.router().send(&h.to_node, forwarded) {
        Ok(r) => r,
        Err(e) => {
            let _ = reverts.send(handoff);
            let _ = answers.send(error_response(rid, &e));
            return;
        }
    };
    open.fetch_add(1, Ordering::Relaxed);
    let (open, rid, node) = (Arc::clone(open), rid.to_string(), handoff.to_node.clone());
    tokio::spawn(async move {
        // The board has accepted the handoff; whether the message arrived only changes what the giver is told.
        let delivered = matches!(
            tokio::time::timeout(COMMAND_DEADLINE, reply).await,
            Ok(Ok(v1::CommandResponse {
                result: Some(command_response::Result::Ok(_)),
                ..
            }))
        );
        open.fetch_sub(1, Ordering::Relaxed);
        if !delivered {
            let _ = reverts.send(handoff);
        }
        let resp = if delivered {
            v1::CommandResponse {
                request_id: rid,
                result: Some(command_response::Result::Ok(v1::CommandOk {
                    result: Some(v1::command_ok::Result::BoardResult(v1::BoardResult {
                        shared: true,
                        result_json: result,
                        is_error,
                    })),
                })),
            }
        } else {
            error_response(
                &rid,
                &Error::new(ErrorCode::NodeOffline).with("node_id", node.as_str()),
            )
        };
        let _ = answers.send(resp);
    });
}

/// ADR-0058: the snapshot fields that come from an agent's reported session (options, context, its own cost).
fn session_snapshot(s: &crate::projection::SessionView) -> v1::AgentSnapshot {
    let o = s.options.clone().unwrap_or_default();
    v1::AgentSnapshot {
        modes: o.modes,
        models: o.models,
        current_mode: o.current_mode,
        current_model: o.current_model,
        context_tokens: s.context_tokens,
        context_window: s.context_window,
        cost_micros: s.cost_micros,
        currency: s.currency.clone(),
        ..v1::AgentSnapshot::default()
    }
}

/// ADR-0061: the device commands a client sends, answered here; `None` for any other command. Registering and listing are open to
/// every signed-in person for their own devices; a viewer, who cannot act on a machine, cannot endorse or revoke.
fn device_command(
    hub: &Hub,
    who: &crate::users::Access,
    req: &v1::CommandRequest,
) -> Option<v1::CommandResponse> {
    use command_request::Command as C;
    let devices = crate::devices::Devices::new(hub.store());
    let now = now_ms();
    let empty = || command_ok::Result::Empty(v1::Empty {});
    let viewer = who.role == crate::users::Role::Viewer;
    let aimed = !req.target_node.is_empty();
    let (action, result): (&str, Result<command_ok::Result, Error>) = match &req.command {
        // Nothing is registered, kept or carried for a request that names a node: device commands are the hub's.
        Some(
            C::DeviceRegister(_)
            | C::DeviceList(_)
            | C::DeviceAskApproval(_)
            | C::DeviceEndorse(_)
            | C::DeviceRevoke(_),
        ) if aimed => (
            "",
            Err(Error::new(ErrorCode::ProtocolInvalidMessage).with("field", "target_node")),
        ),
        Some(C::DeviceRegister(r)) => ("", devices.register(&who.user, r, now).map(|()| empty())),
        Some(C::DeviceList(_)) => (
            "",
            devices
                .list_for(&who.user)
                .map(|devices| command_ok::Result::DeviceList(v1::DeviceListResult { devices })),
        ),
        Some(C::DeviceAskApproval(a)) => (
            "device.ask_approval",
            devices.ask_approval(&who.user, a, now).map(|()| empty()),
        ),
        Some(C::DeviceEndorse(_) | C::DeviceRevoke(_)) if viewer => (
            "device.endorse",
            Err(Error::new(ErrorCode::AuthForbidden).with("action", command_name(req).as_str())),
        ),
        Some(C::DeviceEndorse(d)) => (
            "device.endorse",
            match &d.endorsement {
                None => {
                    Err(Error::new(ErrorCode::ProtocolInvalidMessage).with("field", "endorsement"))
                }
                Some(e) => devices.record_endorsement(&who.user, e, now).map(|()| {
                    carry_device_change(hub, C::DeviceEndorse(d.clone()), &e.nodes);
                    empty()
                }),
            },
        ),
        Some(C::DeviceRevoke(d)) => (
            "device.revoke",
            match &d.revocation {
                None => {
                    Err(Error::new(ErrorCode::ProtocolInvalidMessage).with("field", "revocation"))
                }
                Some(r) => devices.record_revocation(&who.user, r, now).map(|()| {
                    carry_device_change(hub, C::DeviceRevoke(d.clone()), &[]);
                    empty()
                }),
            },
        ),
        _ => return None,
    };
    if !action.is_empty() {
        let outcome = match &result {
            Ok(_) => "ok",
            Err(e) => e.code.as_str(),
        };
        audit(hub, who, action, "", outcome);
    }
    Some(match result {
        Ok(ok) => v1::CommandResponse {
            request_id: req.request_id.clone(),
            result: Some(command_response::Result::Ok(v1::CommandOk {
                result: Some(ok),
            })),
        },
        Err(e) => error_response(&req.request_id, &e),
    })
}

/// ADR-0061: hands an endorsement or revocation to every connected node that verifies them (for an endorsement that names machines,
/// only those). A node that is not connected gets it when it connects ([`give_device_changes`]). The answers are not waited for: what
/// a node did shows in its next `NodeTrustedDevices`, and the node keeps its own record of every one it refused.
fn carry_device_change(hub: &Hub, command: command_request::Command, nodes: &[String]) {
    for node in hub.router().nodes_with("device.trust") {
        if nodes.is_empty() || nodes.contains(&node) {
            let req = v1::CommandRequest {
                request_id: String::new(),
                idempotency_key: String::new(),
                target_node: String::new(),
                command: Some(command.clone()),
            };
            let _ = hub.router().send(&node, req);
        }
    }
}

/// ADR-0061: everything a node that just connected may have missed: endorsements that have not expired (for it), then revocations.
/// Applying one twice changes nothing on the node.
fn give_device_changes(hub: &Hub, node_id: &str) {
    let Ok((endorsements, revocations)) =
        crate::devices::Devices::new(hub.store()).for_machines(now_ms())
    else {
        return;
    };
    let commands = endorsements
        .into_iter()
        .filter(|e| e.nodes.is_empty() || e.nodes.iter().any(|n| n == node_id))
        .map(|e| {
            command_request::Command::DeviceEndorse(v1::DeviceEndorse {
                endorsement: Some(e),
            })
        })
        .chain(revocations.into_iter().map(|r| {
            command_request::Command::DeviceRevoke(v1::DeviceRevoke {
                revocation: Some(r),
            })
        }));
    for command in commands {
        let req = v1::CommandRequest {
            request_id: String::new(),
            idempotency_key: String::new(),
            target_node: String::new(),
            command: Some(command),
        };
        let _ = hub.router().send(node_id, req);
    }
}

/// ADR-0061: a node's one question on a connection of its own (made with its certificate, before any `Hello`): `DeviceList`, so the
/// owner at the machine can choose which registered devices it trusts (`agora node trust --from-hub`) while the node's own connection
/// stays as it is. One answer, then the connection ends; nothing else is accepted this way.
async fn serve_node_query<S>(
    mut conn: Conn<S>,
    hub: &Hub,
    node_id: &str,
    first: Frame,
) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let req =
        v1::CommandRequest::decode(first.payload.as_slice()).map_err(|_| invalid("request"))?;
    let resp = match req.command {
        Some(command_request::Command::DeviceList(_)) => {
            match crate::devices::Devices::new(hub.store()).list_for_node(node_id) {
                Ok(devices) => v1::CommandResponse {
                    request_id: req.request_id.clone(),
                    result: Some(command_response::Result::Ok(v1::CommandOk {
                        result: Some(command_ok::Result::DeviceList(v1::DeviceListResult {
                            devices,
                        })),
                    })),
                },
                Err(e) => error_response(&req.request_id, &e),
            }
        }
        _ => error_response(
            &req.request_id,
            &Error::new(ErrorCode::ProtocolInvalidMessage).with("field", "command"),
        ),
    };
    conn.send(&frame(STREAM_COMMANDS, 1, resp.encode_to_vec()))
        .await?;
    conn.close().await;
    Ok(())
}

/// Appends a row to the audit trail. A failure to write is not allowed to take the hub down with it.
fn audit(
    hub: &Hub,
    who: &crate::users::Access,
    action: &str,
    node: &str,
    outcome: &str,
) -> Option<i64> {
    crate::audit::Audit::new(hub.store())
        .record_in(&who.session, now_ms(), &who.user, action, node, outcome)
        .ok()
}

/// The audit row's action: the command's schema name, with the launch options a start chose (ADR-0059: `AgentLaunch[skip-permissions]`).
fn audit_action(req: &v1::CommandRequest) -> String {
    let options = match &req.command {
        Some(command_request::Command::AgentLaunch(a)) => a.launch_options.as_slice(),
        Some(command_request::Command::AgentStart(a)) => a.launch_options.as_slice(),
        _ => &[],
    };
    let name = command_name(req);
    if options.is_empty() {
        name
    } else {
        format!("{name}[{}]", options.join(","))
    }
}

/// The command's schema name (`AgentStop`), for an error that names what was refused.
fn command_name(req: &v1::CommandRequest) -> String {
    let dbg = format!("{:?}", req.command);
    let inner = dbg.strip_prefix("Some(").unwrap_or(&dbg);
    inner.split('(').next().unwrap_or("").to_string()
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
                    // ADR-0061: candidates only; the owner at the machine says which ones it trusts.
                    devices: crate::devices::Devices::new(hub.store())
                        .list_for_node(&e.node_id)
                        .unwrap_or_default(),
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
    /// The web app, served over HTTP on the same port when present.
    pub assets: Option<Arc<dyn crate::web::Assets>>,
    /// Answers `POST /api/session` (sign-in with a one-time code) when present.
    pub sessions: Option<Arc<dyn crate::web::SessionApi>>,
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
            // The token was good when the connection opened; an expired or revoked session must not keep a live one.
            let token = token.unwrap_or_default();
            let role = || state.auth.access(token);
            serve_client_while(conn, &state.hub, first, &role, &|| state.auth.check(token)).await
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
    // Who is on the other end: a sign-in from another machine needs the screen password (ADR-0067), the hub's own does not.
    let peer = tcp.peer_addr().ok().map(|a| a.ip());
    let Some(acceptor) = tls else {
        // Without TLS nothing can prove it is a node: only the token path exists (development and tests).
        let Some(tcp) = websocket_only(tcp, peer, state).await? else {
            return Ok(());
        };
        let (conn, token) = agora_protocol::ws::accept_with_token(tcp).await?;
        return serve_unauthenticated(conn, token.as_deref(), state).await;
    };
    let stream = acceptor
        .accept(tcp)
        .await
        .map_err(|e| ServeError::Transport(TransportError::Ws(e.to_string())))?;
    let identity = crate::tls::identity(stream.get_ref().1.peer_certificates());
    let Some(stream) = websocket_only(stream, peer, state).await? else {
        return Ok(());
    };
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
            let mut conn = conn;
            // The first frame decides; a peer that says nothing is dropped like a silent node (ADR-0070).
            let first = match tokio::time::timeout(NODE_SILENCE, conn.recv()).await {
                Ok(r) => r?,
                Err(_) => return Err(ServeError::Silent),
            };
            let Some(first) = first else {
                return Ok(());
            };
            // ADR-0061: a one-shot question on a connection of its own.
            if (first.stream_id, first.kind) == (STREAM_COMMANDS, 0) {
                return serve_node_query(conn, &state.hub, &id.node_id, first).await;
            }
            // Revoking a node ends its connection, not only its next one.
            tokio::select! {
                r = serve_node_from(conn, Some(first), &id.node_id, &state.hub, &state.support, NODE_SILENCE) => r,
                () = still(|| Enrollment::new(state.hub.store()).is_active(&id.serial_hex).unwrap_or(false)) => {
                    Err(ServeError::Protocol(
                        Error::new(ErrorCode::AuthDeviceRevoked).with("device_id", id.node_id.as_str()),
                    ))
                }
            }
        }
        None => serve_unauthenticated(conn, token.as_deref(), state).await,
    }
}

/// How often a live connection is checked against its credential (10 s; `AGORA_RECHECK_MS` shortens it for tests).
fn recheck() -> Duration {
    std::env::var("AGORA_RECHECK_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .map_or(Duration::from_secs(10), Duration::from_millis)
}

/// Completes when `valid` first says no. Checked every [`RECHECK`].
async fn still(valid: impl Fn() -> bool) {
    loop {
        tokio::time::sleep(recheck()).await;
        if !valid() {
            return;
        }
    }
}

/// Answers plain HTTP requests from the assets; returns the stream (replaying what was read) when it is a WebSocket.
async fn websocket_only<S>(
    stream: S,
    peer: Option<std::net::IpAddr>,
    state: &ServerState,
) -> Result<Option<crate::web::Rewind<S>>, ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    match crate::web::route(
        stream,
        peer,
        state.assets.as_deref(),
        state.sessions.as_deref(),
    )
    .await
    .map_err(|e| ServeError::Transport(TransportError::Ws(e.to_string())))?
    {
        crate::web::First::WebSocket(s) => Ok(Some(s)),
        crate::web::First::Answered => Ok(None),
    }
}

/// Accepts connections until the listener fails; each runs on its own task.
pub async fn run(
    listener: tokio::net::TcpListener,
    state: Arc<ServerState>,
    tls: Option<tokio_rustls::TlsAcceptor>,
) {
    while let Ok((tcp, _)) = listener.accept().await {
        // Small frames (a keystroke, an echo) must leave at once: without this, Nagle's algorithm plus the peer's delayed
        // ACK holds one back for up to 40 ms.
        let _ = tcp.set_nodelay(true);
        tokio::spawn(handle_connection(Arc::clone(&state), tcp, tls.clone()));
    }
}

#[cfg(test)]
mod audit_cleanup_tests {
    use super::*;

    #[tokio::test]
    async fn an_older_command_must_time_out_while_cleanup_waits_for_a_newer_one(
    ) -> Result<(), crate::store::StoreError> {
        let hub = Hub::open(crate::store::SqliteStore::open_in_memory()?)?;
        let audit = crate::audit::Audit::new(hub.store());
        let older_id = audit.record(0, "owner", "AgentList", "n", "sent")?;
        let newer_id = audit.record(1, "owner", "AgentList", "n", "sent")?;
        let now = tokio::time::Instant::now();
        let older_deadline = now + Duration::from_millis(10);
        let (older_send, older_reply) = tokio::sync::oneshot::channel();
        let (newer_send, newer_reply) = tokio::sync::oneshot::channel();
        let pending = |audit, reply, deadline| Inflight {
            client_request_id: String::new(),
            node: "n".into(),
            relay: None,
            audit: Some(audit),
            reply,
            deadline,
        };
        // answer_ready uses swap_remove: a completed first request moves the newest one in front of an older pending request.
        let inflight = vec![
            pending(newer_id, newer_reply, now + Duration::from_secs(1)),
            pending(older_id, older_reply, older_deadline),
        ];
        let send_late = async {
            tokio::time::sleep_until(older_deadline + Duration::from_millis(50)).await;
            assert!(tokio::time::Instant::now() > older_deadline);
            let ok = || v1::CommandResponse {
                request_id: String::new(),
                result: Some(command_response::Result::Ok(v1::CommandOk::default())),
            };
            let _ = older_send.send(ok());
            let _ = newer_send.send(ok());
        };
        // Poll cleanup first even if a busy test runner wakes after both timers: the timely timeout must precede the late reply.
        tokio::join!(biased; finish_unanswered(&hub, inflight), send_late);
        assert_eq!(audit.outcome(newer_id).as_deref(), Some("ok"));
        assert_eq!(audit.outcome(older_id).as_deref(), Some("node.timeout"));
        Ok(())
    }
}
