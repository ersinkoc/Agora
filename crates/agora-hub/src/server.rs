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

    let (_registration, commands) = hub.router().register(node_id);
    let result = stream_events(&mut conn, &mut session, hub, node_id, commands).await;
    // However it ended, the hub now knows this node is not connected.
    let _ = hub.node_disconnected(node_id, now_ms());
    result
}

async fn stream_events<S>(
    conn: &mut Conn<S>,
    session: &mut NodeSession<'_, crate::store::SqliteStore>,
    hub: &Hub,
    node_id: &str,
    mut commands: tokio::sync::mpsc::UnboundedReceiver<crate::router::ToNode>,
) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
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
        let idle;
        let received = tokio::select! {
            r = tokio::time::timeout(ACK_IDLE, conn.recv()) => r,
            Some(answer) = posted.recv() => {
                conn.send(&frame(STREAM_COMMANDS, 1, answer.encode_to_vec())).await?;
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
                        conn.send(&frame(stream, 0, payload)).await?;
                    }
                    crate::router::ToNode::Close { stream } => {
                        conn.send(&frame(stream, STREAM_CLOSE, Vec::new())).await?;
                    }
                }
                continue;
            }
        };
        match received {
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
    /// What the holder of a live token may do (ADR-0049); `None` when it is not one. The default is the owner's, which is what a
    /// hub with one person has always meant.
    fn access(&self, token: &str) -> Option<crate::users::Access> {
        self.check(token).then(|| crate::users::Access {
            role: crate::users::Role::Owner,
            user: "owner".into(),
        })
    }
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
        })
    })
    .await
}

/// [`serve_client`] for a person with a role (ADR-0049). `role` is asked for every command, so a demotion or a removed person takes
/// effect at once; `None` ends the connection.
pub async fn serve_client_as<S>(
    mut conn: Conn<S>,
    hub: &Hub,
    first: Frame,
    access: &(dyn Fn() -> Option<crate::users::Access> + Sync),
) -> Result<(), ServeError>
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
    let mut inflight: Vec<Inflight> = Vec::new();
    let (relays, mut relay_frames) = hub.router().client();
    loop {
        answer_ready(&mut conn, &mut inflight, hub).await?;
        // Terminal frames from nodes, and streams the hub ended.
        while let Ok(rf) = relay_frames.try_recv() {
            conn.send(&frame(rf.stream, rf.kind, rf.payload)).await?;
        }
        for s in relays.take_closed() {
            conn.send(&frame(s, STREAM_CLOSE, Vec::new())).await?;
        }
        // Push whatever is waiting; a closed channel means the hub dropped this client for being too slow.
        loop {
            match subscription.live.try_recv() {
                Ok(d) => send_positioned(&mut conn, &d).await?,
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
                    forward_command(
                        &mut conn,
                        hub,
                        &relays,
                        &mut inflight,
                        stream,
                        &f.payload,
                        &who,
                    )
                    .await?;
                }
                (stream, STREAM_CLOSE) if stream >= FIRST_DYNAMIC_STREAM => relays.close(stream),
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

async fn forward_command<S>(
    conn: &mut Conn<S>,
    hub: &Hub,
    relays: &crate::router::ClientRelays,
    inflight: &mut Vec<Inflight>,
    stream_id: u32,
    payload: &[u8],
    who: &crate::users::Access,
) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let req = v1::CommandRequest::decode(payload).map_err(|_| invalid("request"))?;
    let client_request_id = req.request_id.clone();
    let refuse = |e: Error| error_response(&client_request_id, &e);
    // ADR-0049: a viewer's commands stop here, before any node hears of them. (The node still decides what its devices may do.)
    if who.role == crate::users::Role::Viewer
        && !req.command.as_ref().is_some_and(crate::users::viewer_may)
    {
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
            i.audit = audit(hub, who, &command_name(&req), &i.node, "sent");
            inflight.push(i);
        }
        Err(e) => {
            audit(
                hub,
                who,
                &command_name(&req),
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

/// Appends a row to the audit trail. A failure to write is not allowed to take the hub down with it.
fn audit(
    hub: &Hub,
    who: &crate::users::Access,
    action: &str,
    node: &str,
    outcome: &str,
) -> Option<i64> {
    crate::audit::Audit::new(hub.store())
        .record(now_ms(), &who.user, action, node, outcome)
        .ok()
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
            tokio::select! {
                r = serve_client_as(conn, &state.hub, first, &role) => r,
                () = still(|| state.auth.check(token)) => Err(ServeError::Refused),
            }
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
        let Some(tcp) = websocket_only(tcp, state).await? else {
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
    let Some(stream) = websocket_only(stream, state).await? else {
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
            // Revoking a node ends its connection, not only its next one.
            tokio::select! {
                r = serve_node(conn, &id.node_id, &state.hub, &state.support) => r,
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
    state: &ServerState,
) -> Result<Option<crate::web::Rewind<S>>, ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    match crate::web::route(stream, state.assets.as_deref(), state.sessions.as_deref())
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
