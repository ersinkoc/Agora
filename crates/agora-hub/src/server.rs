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

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agora_protocol::frame::{
    kinds, DEFAULT_MAX_PAYLOAD, FIRST_DYNAMIC_STREAM, STREAM_CLOSE, STREAM_COMMANDS,
    STREAM_CONTROL, STREAM_EVENTS,
};
use agora_protocol::handshake::Support;
use agora_protocol::v1::{self, command_ok, command_request, command_response};
use agora_protocol::ws::{Conn, TransportError};
use agora_protocol::{Error, ErrorCode, Frame};
use futures_util::FutureExt;
use prost::Message;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::ca::Ca;
use crate::enroll::{EnrollError, Enrollment, Lockout};
use crate::hub::Hub;
use crate::projection::View;
use crate::session::{Ingest, NodeSession, SessionError};
use crate::store::{SqliteStore, StoreError};

/// Idle time after which pending acknowledgements are sent, and the longest an event waits for its ack.
const ACK_IDLE: Duration = Duration::from_millis(50);
/// Acknowledge at least every this many events while a node streams without pause.
const ACK_BATCH: u32 = 64;
/// Amortize dispatch when frames are already available, with a bounded turn on the writer.
const WRITE_BATCH: usize = 16;
const WRITE_BATCH_BYTES: usize = 64 << 10;
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
    EventWriter(tokio::task::JoinError),
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
    serve_node_from(conn, None, node_id, hub, support, silence, &|| true).await
}

/// [`serve_node_within`] for as long as `valid` holds: when it stops holding (a revoked certificate), the conversation ends with
/// `auth.device_revoked` and the node is recorded offline, the handshake included. The certificate path uses it with the enrollment
/// record; tests with a switch.
pub async fn serve_node_while<S>(
    conn: Conn<S>,
    node_id: &str,
    hub: &Arc<Hub>,
    support: &Support,
    silence: Duration,
    valid: &(dyn Fn() -> bool + Sync),
) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    serve_node_from(conn, None, node_id, hub, support, silence, valid).await
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
    valid: &(dyn Fn() -> bool + Sync),
) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let result = converse_node(&mut conn, first, node_id, hub, support, silence, valid).await;
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
        // Said in the hub's log: the node sees only a dropped connection (#935: "database is locked" here went unlogged).
        Err(ServeError::Store(e)) => eprintln!("node link ended: {node_id} store: {}", e.0),
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
    valid: &(dyn Fn() -> bool + Sync),
) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut session = NodeSession::new(hub.shared_store(), node_id, support);
    let revoked = || {
        ServeError::Protocol(Error::new(ErrorCode::AuthDeviceRevoked).with("device_id", node_id))
    };

    // The handshake is watched as the conversation is (#558 review): a certificate revoked between Hello and Resume ends it, and a
    // node that stops in the middle is dropped after `silence` instead of holding the connection forever.
    let welcome = {
        let handshake = async {
            let hello_frame = match first {
                Some(f) if (f.stream_id, f.kind) == (STREAM_CONTROL, kinds::HELLO) => f,
                Some(_) => return Err(invalid("frame")),
                None => expect(conn, STREAM_CONTROL, kinds::HELLO).await?,
            };
            let hello =
                v1::Hello::decode(hello_frame.payload.as_slice()).map_err(|_| invalid("hello"))?;
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
            Ok::<_, ServeError>(welcome)
        };
        tokio::select! {
            r = tokio::time::timeout(silence, handshake) => r.map_err(|_| ServeError::Silent)??,
            () = still(valid) => return Err(revoked()),
        }
    };
    // Checked once more right before the node is recorded online.
    if !valid() {
        return Err(revoked());
    }

    // ADR-0070: the connection is open, so the hub records the node online (a first connect or a reconnect alike: nothing the node
    // runs is reset) and, when it ends, offline with why. Both come from this certificate-authenticated connection only.
    let verifies_devices = welcome.capabilities.iter().any(|c| c == "device.trust");
    // #957: a connection whose opening the hub cannot record is ended (the node dials again and is recorded then), never kept live
    // while every view shows the machine offline.
    let (registration, commands, generation) =
        hub.node_link_opened(node_id, welcome.capabilities, now_ms())?;
    let load = registration.load();
    // ADR-0061: a node that verifies endorsements gets every one it may have missed while it was away, and every revocation.
    if verifies_devices {
        give_device_changes(hub, node_id);
    }
    // ADR-0108: the forgets owed to it, oldest first (a node without `agent.forget.local` is never sent one; they wait).
    give_forgets(hub, node_id, hub.forgets().pending(node_id));
    // ADR-0039 amendment B, #1036: board membership looks now, not only at the node's next event. A redial that wrote the node's
    // late end together with its `up` makes it due to hand over its local work, and an idle node that is caught up may send no
    // event for a long time (ADR-0054). The asks go out after the device changes, before anything the connection carries.
    for (node, project) in hub.board_membership(now_ms()) {
        ask_board_join(hub, node, project);
    }
    let mut last_heard = tokio::time::Instant::now();
    let mut events = NodeEvents {
        session: Some(session),
        writing: None,
    };
    // A certificate revoked while the connection is open ends it here, inside the conversation, so the end below still records the
    // node offline (it used to be cancelled from outside, and a revoked machine stayed online in every view).
    let result = tokio::select! {
        r = stream_events(conn, &mut events, hub, node_id, (commands, &load), silence, &mut last_heard) => r,
        () = still(valid) => Err(revoked()),
    };
    // The certificate check can cancel the stream while a write is running. Wait for that event before recording the link closed.
    if events.writing.is_some() {
        let _ = events.settle().await;
    }
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
    // #996: the store refused the end: it is written as soon as the store works, even if nothing else happens on the hub.
    if hub.has_unrecorded() {
        record_later(hub);
    }
    result
}

/// #996: retries the presence facts the store refused until they are written, waiting longer each time (up to 5 s), at most one
/// task per hub. Once they are, the board's membership is worked out again, as when a connection's end is recorded at once. The
/// task holds the hub only during an attempt (a hub dropped meanwhile ends it), and however it ends, a panic included, it lets
/// the next close start another.
fn record_later(hub: &Arc<Hub>) {
    use std::sync::atomic::Ordering;
    /// Clears `recording_later` when the task ends while it still owns it.
    struct Owner {
        hub: std::sync::Weak<Hub>,
        owns: bool,
    }
    impl Drop for Owner {
        fn drop(&mut self) {
            if let (true, Some(hub)) = (self.owns, self.hub.upgrade()) {
                hub.recording_later.store(false, Ordering::Release);
            }
        }
    }
    if hub.recording_later.swap(true, Ordering::AcqRel) {
        return;
    }
    let mut owner = Owner {
        hub: Arc::downgrade(hub),
        owns: true,
    };
    tokio::spawn(async move {
        let mut wait = Duration::from_millis(100);
        loop {
            tokio::time::sleep(wait).await;
            let Some(hub) = owner.hub.upgrade() else {
                return;
            };
            let recorded = {
                let hub = Arc::clone(&hub);
                tokio::task::spawn_blocking(move || hub.record_unrecorded().is_ok())
                    .await
                    .unwrap_or(false)
            };
            if recorded {
                for (node, project) in hub.board_membership(now_ms()) {
                    ask_board_join(&hub, node, project);
                }
                owner.owns = false;
                hub.recording_later.store(false, Ordering::Release);
                // A connection that ended between the write and the line above found this task still running and left its
                // facts to it.
                if !hub.has_unrecorded() || hub.recording_later.swap(true, Ordering::AcqRel) {
                    return;
                }
                owner.owns = true;
                wait = Duration::from_millis(100);
            } else {
                wait = (wait * 2).min(Duration::from_secs(5));
            }
        }
    });
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

#[derive(Default)]
struct PersistedEvents {
    acked: Option<u64>,
    count: u32,
    gap: Option<(u64, VecDeque<Vec<u8>>)>,
}

type WriteResult = (
    NodeSession<Arc<SqliteStore>>,
    Result<PersistedEvents, ServeError>,
);

/// A node waits for its current batch to finish before processing later frames. The session moves into the blocking job
/// with its resync state. A hub-wide permit bounds the jobs sent to Tokio's blocking pool.
struct NodeEvents {
    session: Option<NodeSession<Arc<SqliteStore>>>,
    writing: Option<tokio::task::JoinHandle<WriteResult>>,
}

impl NodeEvents {
    async fn write(
        &mut self,
        hub: &Arc<Hub>,
        batch: VecDeque<Vec<u8>>,
    ) -> Result<PersistedEvents, ServeError> {
        let slot = Arc::clone(&hub.event_writer)
            .acquire_owned()
            .await
            .map_err(|_| StoreError("node event writer is closed".into()))?;
        let mut session = self
            .session
            .take()
            .ok_or_else(|| StoreError("node event session is unavailable".into()))?;
        let hub = Arc::clone(hub);
        self.writing = Some(tokio::task::spawn_blocking(move || {
            let _slot = slot;
            let result = persist_events(&hub, &mut session, batch);
            (session, result)
        }));
        self.settle().await
    }

    async fn settle(&mut self) -> Result<PersistedEvents, ServeError> {
        let writing = self
            .writing
            .as_mut()
            .ok_or_else(|| StoreError("no node event write is pending".into()))?;
        let finished = writing.await;
        self.writing = None;
        let (session, result) = finished.map_err(ServeError::EventWriter)?;
        self.session = Some(session);
        result
    }

    fn snapshot_required(&mut self, at_ms: u64) -> Result<(), ServeError> {
        self.session
            .as_mut()
            .ok_or_else(|| StoreError("node event session is unavailable".into()))?
            .snapshot_required(at_ms)?;
        Ok(())
    }
}

fn persist_events(
    hub: &Arc<Hub>,
    session: &mut NodeSession<Arc<SqliteStore>>,
    mut batch: VecDeque<Vec<u8>>,
) -> Result<PersistedEvents, ServeError> {
    let mut persisted = PersistedEvents::default();
    while let Some(payload) = batch.pop_front() {
        match persist_event(hub, session, &payload)? {
            Ingest::Applied { acked, .. }
            | Ingest::Resynced { acked, .. }
            | Ingest::Duplicate { acked } => {
                persisted.acked = Some(acked);
                persisted.count += 1;
            }
            Ingest::Gap { expected } => {
                // The link must send ResumeOk before processing subsequent frames.
                persisted.gap = Some((expected, batch));
                break;
            }
        }
    }
    Ok(persisted)
}

fn persist_event(
    hub: &Arc<Hub>,
    session: &mut NodeSession<Arc<SqliteStore>>,
    payload: &[u8],
) -> Result<Ingest, ServeError> {
    let outcome = hub.ingest(session, payload)?;
    // ADR-0054: the skipped range is in the event store. Record it in the audit trail as well before acknowledging the event.
    if let Ingest::Resynced {
        from_seq, to_seq, ..
    } = outcome
    {
        let node_id = session.producer();
        eprintln!("node resync: {node_id} skipped {from_seq}..={to_seq}");
        let _ = crate::audit::Audit::new(hub.store()).record(
            now_ms(),
            crate::ca::RESERVED_HUB_ID,
            "node.resynced",
            node_id,
            &format!("{from_seq}..={to_seq}"),
        );
    }
    if matches!(outcome, Ingest::Applied { .. } | Ingest::Resynced { .. }) {
        for (node, project) in hub.board_membership(now_ms()) {
            ask_board_join(hub, node, project);
        }
    }
    Ok(outcome)
}

type ReceivedFrame = Result<Option<Frame>, TransportError>;

#[derive(Default)]
struct NodeFrames {
    events: VecDeque<Vec<u8>>,
    next: Option<ReceivedFrame>,
}

impl NodeFrames {
    async fn recv<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        conn: &mut Conn<S>,
    ) -> ReceivedFrame {
        if let Some(payload) = self.events.pop_front() {
            Ok(Some(frame(STREAM_EVENTS, kinds::EVENT, payload)))
        } else if let Some(next) = self.next.take() {
            next
        } else {
            conn.recv().await
        }
    }

    /// Collect a bounded run of events that can be read immediately. Keep any following control or data frame for the
    /// next iteration, in order. One event above the byte target is still accepted on its own.
    fn batch<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        conn: &mut Conn<S>,
        first: Vec<u8>,
        limit: usize,
    ) -> VecDeque<Vec<u8>> {
        let mut bytes = first.len();
        let mut batch = VecDeque::from([first]);
        while batch.len() < limit && bytes < WRITE_BATCH_BYTES {
            let payload = if let Some(payload) = self.events.pop_front() {
                payload
            } else {
                if self.next.is_some() {
                    break;
                }
                match conn.recv().now_or_never() {
                    Some(Ok(Some(f))) if (f.stream_id, f.kind) == (STREAM_EVENTS, kinds::EVENT) => {
                        f.payload
                    }
                    Some(next) => {
                        self.next = Some(next);
                        break;
                    }
                    None => break,
                }
            };
            if bytes + payload.len() > WRITE_BATCH_BYTES {
                self.events.push_front(payload);
                break;
            }
            bytes += payload.len();
            batch.push_back(payload);
        }
        batch
    }
}

async fn stream_events<S>(
    conn: &mut Conn<S>,
    events: &mut NodeEvents,
    hub: &Arc<Hub>,
    node_id: &str,
    // The commands queued for this node, and the connection's count of them (`GET /api/metrics`).
    (mut commands, load): (
        tokio::sync::mpsc::UnboundedReceiver<crate::router::ToNode>,
        &crate::router::LinkLoad,
    ),
    silence: Duration,
    last_heard: &mut tokio::time::Instant,
) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // Every frame from the node counts as a sign of life; an idle node sends its pings (every 10 s; 15 s before ADR-0070).
    let mut acked: Option<u64> = None; // highest contiguous seq not yet acknowledged
    let mut since_ack = 0u32;
    let mut frames = NodeFrames::default();
    // Commands sent to this node and not yet answered, by the request id the hub chose.
    let mut waiting: std::collections::HashMap<
        String,
        tokio::sync::oneshot::Sender<v1::CommandResponse>,
    > = std::collections::HashMap::new();
    // Mesh posts this node sent for another node (ADR-0031): answers come back here from tasks that wait on the target.
    let (posted_tx, mut posted) = tokio::sync::mpsc::unbounded_channel::<v1::CommandResponse>();
    let posts_open = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    loop {
        if last_heard.elapsed() >= silence {
            return Err(ServeError::Silent);
        }
        let idle;
        let received = tokio::select! {
            r = tokio::time::timeout(ACK_IDLE, frames.recv(conn)) => r,
            Some(answer) = posted.recv() => {
                put(conn, &frame(STREAM_COMMANDS, 1, answer.encode_to_vec()), silence).await?;
                continue;
            }
            Some(to_node) = commands.recv() => {
                match to_node {
                    crate::router::ToNode::Command { request, reply, stream } => {
                        load.queued.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                        waiting.retain(|_, s| !s.is_closed());
                        let payload = request.encode_to_vec();
                        waiting.insert(request.request_id.clone(), reply);
                        load.unanswered.store(waiting.len(), std::sync::atomic::Ordering::Relaxed);
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
                    (STREAM_EVENTS, kinds::EVENT) => {
                        let limit = WRITE_BATCH.min((ACK_BATCH - since_ack) as usize);
                        let batch = frames.batch(conn, f.payload, limit);
                        let persisted = events.write(hub, batch).await?;
                        if persisted.acked.is_some() {
                            acked = persisted.acked;
                            since_ack += persisted.count;
                        }
                        // Not the next event: tell the node where to start again (nothing was stored).
                        if let Some((expected, mut remaining)) = persisted.gap {
                            if !remaining.is_empty() {
                                remaining.append(&mut frames.events);
                                frames.events = remaining;
                            }
                            let ok = v1::ResumeOk { from_seq: expected };
                            put(
                                conn,
                                &frame(STREAM_CONTROL, kinds::RESUME_OK, ok.encode_to_vec()),
                                silence,
                            )
                            .await?;
                        }
                    }
                    // The node cannot replay from where the hub asked (ADR-0054): its next new event is the new base.
                    (STREAM_CONTROL, kinds::SNAPSHOT_REQUIRED) => {
                        events.snapshot_required(now_ms())?
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
                        post_for_node(hub, node_id, answer, &posts_open, posted_tx);
                    }
                    // An answer to a routed command: hand it to whoever asked. A garbled answer is dropped, and
                    // the asker's deadline turns it into `node.timeout`.
                    (STREAM_COMMANDS, 1) => {
                        if let Ok(resp) = v1::CommandResponse::decode(f.payload.as_slice()) {
                            if let Some(reply) = waiting.remove(&resp.request_id) {
                                let _ = reply.send(resp);
                            }
                            load.unanswered
                                .store(waiting.len(), std::sync::atomic::Ordering::Relaxed);
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

/// The protocol generation a window speaks: windows send no `Hello`, and the hub's own is `schema/agora/v1`.
const CLIENT_GENERATION: u32 = 1;

/// ADR-0106: the `Welcome` a window gets first on its connection, naming the role of the person it signed in as. Nothing was negotiated
/// (a window sends no `Hello`), so it carries no capabilities and no peer id. A hub not in team mode answers for its one person, the
/// owner ([`ClientAuth::access`]). A hint for the window only: every command is still checked against `access`.
pub(crate) fn client_welcome(access: &crate::users::Access, now_ms: u64) -> v1::Welcome {
    use crate::users::Role;
    let role = match access.role {
        Role::Owner => v1::PersonRole::Owner,
        Role::Member => v1::PersonRole::Member,
        Role::Viewer => v1::PersonRole::Viewer,
    };
    let mut welcome = v1::Welcome {
        chosen_generation: CLIENT_GENERATION,
        server_time_ms: now_ms,
        ..v1::Welcome::default()
    };
    welcome.set_person_role(role);
    welcome
}

/// The wire form of the hub's view (ADR-0027). Ids are the hub-wide keys.
pub fn snapshot_of(view: &View, pos: u64) -> v1::Snapshot {
    snapshot_for_client(view, pos, true)
}

fn snapshot_for_client(view: &View, pos: u64, owner: bool) -> v1::Snapshot {
    // ADR-0103: keep the original view for the history filter, which needs to see forgotten machines and agents.
    let recent_messages = view.recent_messages();
    // ADR-0073: forgotten machines and what they ran are not shown.
    let shown = view.shown();
    let view: &View = &shown;
    let mut snapshot = v1::Snapshot {
        recent_messages: Vec::new(),
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
                binary: if owner {
                    n.binary.clone()
                } else {
                    crate::scope::short_binary(&n.binary)
                },
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
                // ADR-0101: only a running agent holds always answers.
                always_allowed: if a.running {
                    a.always_allowed
                        .iter()
                        .map(|(_, action)| action.clone())
                        .collect()
                } else {
                    Vec::new()
                },
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
                scopes: a.scopes.clone(),
                always_label: a.always_label.clone(),
                always_option_id: a.always_option_id.clone(),
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
    };
    if !recent_messages.is_empty() {
        let mut remaining = DEFAULT_MAX_PAYLOAD.saturating_sub(snapshot.encoded_len());
        // Each size includes the repeated field's tag and length. Keep the newest messages that fit, then send them oldest first.
        for message in recent_messages.iter().rev() {
            if message.size > remaining {
                break;
            }
            remaining -= message.size;
            snapshot.recent_messages.push(v1::PositionedEvent {
                pos: message.pos,
                event: Some(message.event.clone()),
            });
        }
        snapshot.recent_messages.reverse();
    }
    snapshot
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
        let audit = crate::audit::Audit::new(hub.store());
        let _ = match tokio::time::timeout_at(item.deadline, item.reply).await {
            Ok(Ok(r)) => audit.finish_response(id, &r, item.fresh_command),
            Ok(Err(_)) => audit.finish(id, ErrorCode::NodeOffline.as_str()),
            Err(_) => audit.finish(id, ErrorCode::NodeTimeout.as_str()),
        };
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
    // ADR-0106: the first frame a window gets says the role of the person it signed in as, for this connection (a change of role or
    // grants ends it, below). A window sends no `Hello`, so this `Welcome` comes unasked, before the snapshot; an older window ignores
    // a control frame it does not use.
    conn.send(&frame(
        STREAM_CONTROL,
        kinds::WELCOME,
        client_welcome(&current, now_ms()).encode_to_vec(),
    ))
    .await?;
    // A member or viewer gets a snapshot cut to their projects, whatever position they ask from (a replay from the log would be
    // everyone's), and from then on only the events that are theirs (ADR-0049).
    let restricted = current.projects.is_some();
    // ADR-0069: only the owner sees the full path of a machine's own executable; everyone else its file name.
    let owner = current.role == crate::users::Role::Owner;
    let mut subscription = hub.subscribe(if restricted { None } else { sub.after_pos })?;
    // Taken out of the subscription, which lives as long as the connection, and dropped once it is sent: the view it holds shares the
    // hub's message rings (ADR-0103), and a window that kept it would keep messages the hub has since let go, and make the hub copy the
    // rings at its next message.
    let snapshot = subscription.snapshot.take();
    let mut scope = None;
    if let Some((view, pos)) = &snapshot {
        let body = match &current.projects {
            Some(projects) => {
                let s = crate::scope::Scope::from_view(projects.clone(), view);
                let cut = s.filter_view(view);
                scope = Some(s);
                snapshot_of(&cut, *pos)
            }
            None => snapshot_for_client(view, *pos, owner),
        };
        conn.send(&frame(STREAM_EVENTS, kinds::SNAPSHOT, body.encode_to_vec()))
            .await?;
    }
    // ADR-0090: the snapshot has no field for an agent's config options and slash commands; the last report of each follows it as
    // the event the node sent, at the snapshot's position (a window applies these two, which replace, at its own position). Only
    // what the window may see: the cut view of a member, the shown view of the owner.
    if let Some((view, pos)) = &snapshot {
        let replay = match &scope {
            Some(s) => s.filter_view(view).session_replay(),
            None => view.session_replay(),
        };
        for event in replay {
            send_positioned(&mut conn, &crate::hub::Delivered { pos: *pos, event }).await?;
        }
    }
    drop(snapshot);
    for d in &subscription.backlog {
        send_shown(&mut conn, d, owner).await?;
    }
    let mut checked = tokio::time::Instant::now();
    let (relays, mut relay_frames) = hub.router().client();
    // ADR-0065: only the owner is told who is asking to join (the card carries the number); memory only, never the event log.
    let mut pair_feed = None;
    if owner && hub.pairing().enabled() {
        let (feed, cards) = crate::pairing_net::WindowFeed::start(hub);
        for f in cards {
            conn.send(&f).await?;
        }
        pair_feed = Some(feed);
    }
    loop {
        answer_ready(&mut conn, inflight, hub, scope.as_ref()).await?;
        if let Some(feed) = pair_feed.as_mut() {
            for f in feed.poll(hub) {
                conn.send(&f).await?;
            }
        }
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
                        send_shown(&mut conn, &d, owner).await?;
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
            // A node answered: sent at the top of the loop, now (`answer_ready`).
            () = an_answer(inflight) => continue,
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
    /// The answer [`an_answer`] already took from `reply` (a oneshot gives it once), for [`answer_ready`] to send.
    answered: Option<Result<v1::CommandResponse, tokio::sync::oneshot::error::RecvError>>,
    deadline: tokio::time::Instant,
    /// The audit row that says this command is on its way (ADR-0049); finished when the answer comes.
    audit: Option<i64>,
    /// A cached reply to a caller-supplied idempotency key cannot vouch for the current request's signer.
    fresh_command: bool,
    /// ADR-0049 Amendment A: a member's folder listing or folder details, by the command's schema name. The answer reaches them only
    /// when the folder the machine opened is inside their projects (`Scope::confine_answer`), otherwise `auth.forbidden` naming it.
    confined: Option<String>,
}

/// Resolves when a command on its way has its answer (or lost its node), keeping it in [`Inflight::answered`]: a client gets an
/// answer as soon as the node gives it. Before, answers waited for the loop's next tick ([`PUSH`]): every command's round trip was
/// at least 20 ms (architecture review R5 measurement, `tests/flood_latency.rs`).
fn an_answer(inflight: &mut [Inflight]) -> impl std::future::Future<Output = ()> + '_ {
    std::future::poll_fn(move |cx| {
        let mut any = false;
        for i in inflight.iter_mut().filter(|i| i.answered.is_none()) {
            if let std::task::Poll::Ready(r) =
                std::future::Future::poll(std::pin::Pin::new(&mut i.reply), cx)
            {
                i.answered = Some(r);
                any = true;
            }
        }
        if any {
            std::task::Poll::Ready(())
        } else {
            std::task::Poll::Pending
        }
    })
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
    let mut req = v1::CommandRequest::decode(payload).map_err(|_| invalid("request"))?;
    let client_request_id = req.request_id.clone();
    let refuse = |e: Error| error_response(&client_request_id, &e);
    let Caller { who, scope } = caller;
    // ADR-0065: the owner's pairing commands are the hub's own; a machine's pairing messages are not a window's to send.
    if let Some(resp) = crate::pairing_net::owner_command(hub, who, &req) {
        conn.send(&frame(STREAM_COMMANDS, 1, resp.encode_to_vec()))
            .await?;
        return Ok(());
    }
    // ADR-0061: a person's own devices, answered (or carried) by the hub; a node decides for itself what it trusts.
    if let Some(resp) = device_command(hub, who, &req) {
        conn.send(&frame(STREAM_COMMANDS, 1, resp.encode_to_vec()))
            .await?;
        return Ok(());
    }
    // Who may do what is decided apart from the transport (R9, `dispatch.rs`); here only what the answer says is done.
    let ok_empty = |request_id: String| v1::CommandResponse {
        request_id,
        result: Some(command_response::Result::Ok(v1::CommandOk {
            result: Some(command_ok::Result::Empty(v1::Empty {})),
        })),
    };
    let resp = match crate::dispatch::route(who, scope, &req) {
        crate::dispatch::Route::Forward => None,
        crate::dispatch::Route::Forbidden { action } => {
            audit_command(hub, who, &req, &action, "auth.forbidden");
            Some(refuse(
                Error::new(ErrorCode::AuthForbidden).with("action", action.as_str()),
            ))
        }
        crate::dispatch::Route::TargetNotAllowed => Some(refuse(
            Error::new(ErrorCode::ProtocolInvalidMessage).with("field", "target_node"),
        )),
        // ADR-0053: the hub forgets a closed agent. It names the agent by its hub-wide key, so it names no node.
        crate::dispatch::Route::ForgetAgent { agent_id, .. } => {
            let (target, _) = hub.audit_target(&req);
            let result = hub.forget_agent(&agent_id, now_ms());
            let outcome = result.as_ref().map_or_else(|e| e.code.as_str(), |_| "ok");
            let _ = crate::audit::Audit::new(hub.store()).record_target(
                &who.session,
                now_ms(),
                &who.user,
                "agent.forget",
                outcome,
                &target,
            );
            Some(match result {
                Ok((node, local)) => {
                    // ADR-0108: passed on to its node now when it is connected and able; otherwise when it next connects. A key with
                    // no node's own id (no `/`) names nothing a node keeps.
                    if !local.is_empty() {
                        give_forgets(hub, &node, vec![local]);
                    }
                    ok_empty(client_request_id.clone())
                }
                Err(e) => refuse(e),
            })
        }
        // ADR-0073: the hub forgets an offline machine (owner only, decided in `dispatch`).
        crate::dispatch::Route::ForgetNodeForbidden { node_id } => {
            audit(hub, who, "node.forget", &node_id, "auth.forbidden");
            Some(refuse(
                Error::new(ErrorCode::AuthForbidden).with("action", "node.forget"),
            ))
        }
        crate::dispatch::Route::ForgetNode {
            node_id,
            refuse_return,
        } => Some(match hub.forget_node(&node_id, refuse_return, now_ms()) {
            Ok(()) => {
                let outcome = if refuse_return {
                    "ok refused_return"
                } else {
                    "ok"
                };
                audit(hub, who, "node.forget", &node_id, outcome);
                ok_empty(client_request_id.clone())
            }
            Err(e) => {
                audit(hub, who, "node.forget", &node_id, e.code.as_str());
                refuse(e)
            }
        }),
        // Answered by the hub itself from its own view and prices (ADR-0033); it names no node.
        crate::dispatch::Route::UsageReport => Some(v1::CommandResponse {
            request_id: client_request_id.clone(),
            result: Some(command_response::Result::Ok(v1::CommandOk {
                result: Some(command_ok::Result::UsageReport(hub.usage_report())),
            })),
        }),
    };
    if let Some(resp) = resp {
        conn.send(&frame(STREAM_COMMANDS, 1, resp.encode_to_vec()))
            .await?;
        return Ok(());
    }
    // ADR-0095: a dispatch names the person who asks, and a person gives at most a few a minute.
    if let Err(e) = crate::dispatch::prepare_dispatch(
        who,
        &mut req,
        hub.dispatches(),
        std::time::Instant::now(),
    ) {
        audit_command(hub, who, &req, &audit_action(&req), e.code.as_str());
        conn.send(&frame(STREAM_COMMANDS, 1, refuse(e).encode_to_vec()))
            .await?;
        return Ok(());
    }
    // Shared terminal size: the node gets the smallest size of the windows watching the terminal, and only when it changes.
    if let Some(command_request::Command::TerminalResize(t)) = req.command.as_mut() {
        if !req.target_node.is_empty() {
            use crate::router::Resize;
            match hub
                .router()
                .resize(relays, &req.target_node, &t.terminal_id, t.cols, t.rows)
            {
                Resize::Forward((cols, rows)) => {
                    t.cols = cols.into();
                    t.rows = rows.into();
                }
                Resize::Unchanged => {
                    audit_command(hub, who, &req, &audit_action(&req), "ok");
                    let resp = v1::CommandResponse {
                        request_id: client_request_id,
                        result: Some(command_response::Result::Ok(v1::CommandOk {
                            result: Some(command_ok::Result::Empty(v1::Empty {})),
                        })),
                    };
                    conn.send(&frame(STREAM_COMMANDS, 1, resp.encode_to_vec()))
                        .await?;
                    return Ok(());
                }
                Resize::PassThrough => {}
            }
        }
    }
    // An answer may remove its approval from the hub projection. Capture the target before forwarding.
    let (audit_target, known) = hub.audit_target(&req);
    let outcome = (|| {
        let node = crate::router::forwardable(&req)?.to_string();
        if !known {
            return Err(Error::new(ErrorCode::NodeUnknown).with("node_id", node.as_str()));
        }
        if inflight.len() >= MAX_INFLIGHT {
            return Err(Error::new(ErrorCode::NodeResourceLimit)
                .with("resource", "requests")
                .with("limit", 256_i64));
        }
        // ADR-0088 (schema only): the hub has no preview relay yet, so a stream-opening preview command is refused as a missing
        // capability, not as a misplaced command.
        if matches!(
            req.command,
            Some(command_request::Command::PreviewConnect(_))
        ) {
            return Err(
                Error::new(ErrorCode::ProtocolCapabilityMissing).with("capability", "preview.v1")
            );
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
            let terminal = match &req.command {
                Some(command_request::Command::TerminalSubscribe(t)) => {
                    Some(t.terminal_id.as_str())
                }
                _ => None,
            };
            let node_stream = hub.router().open_relay(&node, relays, stream_id, terminal);
            match hub
                .router()
                .send_on(&node, hub.spelled_for(&node, req.clone()), node_stream)
            {
                Ok(r) => (r, Some(node_stream)),
                Err(e) => {
                    hub.router().drop_relay(&node, node_stream);
                    return Err(e);
                }
            }
        } else {
            // ADR-0115: a project in the spelling that node knows it by.
            (
                hub.router()
                    .send(&node, hub.spelled_for(&node, req.clone()))?,
                None,
            )
        };
        Ok(Inflight {
            client_request_id: client_request_id.clone(),
            node,
            relay,
            reply,
            answered: None,
            deadline: tokio::time::Instant::now() + COMMAND_DEADLINE,
            audit: None,
            fresh_command: req.idempotency_key.is_empty(),
            confined: scope
                .is_some()
                .then_some(req.command.as_ref())
                .flatten()
                .filter(|c| crate::scope::Scope::answers_confined(c))
                .map(|_| command_name(&req)),
        })
    })();
    match outcome {
        Ok(mut i) => {
            i.audit = crate::audit::Audit::new(hub.store())
                .record_target(
                    &who.session,
                    now_ms(),
                    &who.user,
                    &audit_action(&req),
                    "sent",
                    &audit_target,
                )
                .ok();
            inflight.push(i);
        }
        Err(e) => {
            let _ = crate::audit::Audit::new(hub.store()).record_target(
                &who.session,
                now_ms(),
                &who.user,
                &audit_action(&req),
                e.code.as_str(),
                &audit_target,
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
    scope: Option<&crate::scope::Scope>,
) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    use tokio::sync::oneshot::error::TryRecvError;
    let mut i = 0;
    while i < inflight.len() {
        let item = &mut inflight[i];
        let got = match item.answered.take() {
            Some(Ok(r)) => Ok(r),
            Some(Err(_)) => Err(TryRecvError::Closed),
            None => item.reply.try_recv(),
        };
        let resp = match got {
            Ok(mut r) => {
                r.request_id.clone_from(&item.client_request_id);
                // A member's folder answer: only a folder inside their projects (a scope is always there when this is set).
                if let Some(action) = &item.confined {
                    if !scope.is_some_and(|s| s.confine_answer(&item.node, &mut r)) {
                        r = error_response(
                            &item.client_request_id,
                            &Error::new(ErrorCode::AuthForbidden).with("action", action.as_str()),
                        );
                    }
                }
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
                    let _ = crate::audit::Audit::new(hub.store()).finish_response(
                        id,
                        &r,
                        done.fresh_command,
                    );
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

/// [`send_positioned`], with what only the owner may see taken out for anyone else (ADR-0069: a machine's full executable path).
async fn send_shown<S>(
    conn: &mut Conn<S>,
    d: &crate::hub::Delivered,
    owner: bool,
) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // ADR-0115: a window gets an older node's project id folded, as the snapshot has it.
    let folded = crate::projection::fold_event(&d.event);
    let event = folded.as_ref().unwrap_or(&d.event);
    match (owner, crate::scope::without_full_binary(event)) {
        (false, Some(event)) => {
            send_positioned(conn, &crate::hub::Delivered { pos: d.pos, event }).await
        }
        _ => match folded {
            Some(event) => {
                send_positioned(conn, &crate::hub::Delivered { pos: d.pos, event }).await
            }
            None => send_positioned(conn, d).await,
        },
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

/// A node's `MeshPost` for an agent on another node: forwarded to that node like a client command (ADR-0029) with `origin`
/// set from the connection, whatever the node wrote. The answer (or an error) is sent on `answers`.
fn post_for_node(
    hub: &Arc<Hub>,
    node_id: &str,
    req: v1::CommandRequest,
    open: &Arc<std::sync::atomic::AtomicUsize>,
    answers: tokio::sync::mpsc::UnboundedSender<v1::CommandResponse>,
) {
    use std::sync::atomic::Ordering;
    let rid = req.request_id.clone();
    let refuse = |e: Error| {
        let _ = answers.send(error_response(&rid, &e));
    };
    let post = match req.command {
        Some(command_request::Command::MeshPost(post)) => post,
        Some(command_request::Command::BoardCall(call)) => {
            return board_for_node(hub, node_id, &rid, &call, open, answers);
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
    // ADR-0115: an older node hands over the work it holds under its own spelling of the project.
    let req = hub.spelled_for(&node, req);
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

/// ADR-0108: passes the person's forgets of `agents` (the node's own ids) on to `node`, one after another in one task, each after the
/// node answered the one before: the node's queue is never filled with them. An answer that says the node decided settles that
/// forget (`ok`: removed or unknown; `agent.running`; refused by its grants or policy; an invalid id). Amendment A: one that says it
/// could not decide now ([`forget_failed_for_now`]) keeps it, counted, for the node's next connection (dropped at the bound,
/// `forgets::MAX_TRANSIENT`); after a stopping node's `node.offline` the rest wait for that connection too. A node that is not
/// connected, does not offer `agent.forget.local`, or whose connection ends first is sent the rest when it next connects. Each answer
/// is audited under the hub's own id with its code as the outcome, and the agent and its project in their own columns.
fn give_forgets(hub: &Hub, node: &str, mut agents: Vec<String>) {
    agents.retain(|a| !a.is_empty());
    if agents.is_empty() || !hub.router().is_connected(node) {
        return;
    }
    let (router, forgets, store) = (
        Arc::clone(hub.router()),
        Arc::clone(hub.forgets()),
        hub.shared_store(),
    );
    let node = node.to_string();
    // Each agent with its project, for its audit row (read now: the task does not hold the hub).
    let agents: Vec<(String, String)> = agents
        .into_iter()
        .map(|a| {
            let project = hub.agent_project(&node, &a);
            (a, project)
        })
        .collect();
    tokio::spawn(async move {
        for (agent, project) in agents {
            let req = v1::CommandRequest {
                request_id: String::new(),
                idempotency_key: String::new(),
                target_node: String::new(),
                command: Some(command_request::Command::AgentForget(v1::AgentForget {
                    agent_id: agent.clone(),
                })),
            };
            let Ok(reply) = router.send(&node, req) else {
                return;
            };
            // No deadline: the reply ends when the node answers or its connection goes (the router drops the sender).
            let Ok(resp) = reply.await else {
                return;
            };
            let outcome = match &resp.result {
                Some(command_response::Result::Err(e)) => e.code.clone(),
                _ => "ok".to_string(),
            };
            let passing = forget_failed_for_now(&outcome);
            if passing {
                let _ = forgets.failed_for_now(&node, &agent);
            } else {
                let _ = forgets.answered(&node, &agent);
            }
            // The outcome column holds the code alone; the agent and its project have columns of their own.
            let _ = crate::audit::Audit::new(&store).record_target(
                "",
                now_ms(),
                crate::ca::RESERVED_HUB_ID,
                "agent.forget.local",
                &outcome,
                &crate::audit::Target::of_agent(&node, &agent, &project),
            );
            // The node is stopping: it would answer the rest the same way.
            if outcome == ErrorCode::NodeOffline.as_str() {
                return;
            }
        }
    });
}

/// ADR-0108 amendment A: the codes a node answers a forget with when it could not decide now: it is stopping (`node.offline`), the
/// removal or the uplink's own wait ran out (`node.timeout`), or something failed (`internal`). Every other answer is a decision.
fn forget_failed_for_now(code: &str) -> bool {
    [
        ErrorCode::NodeOffline,
        ErrorCode::NodeTimeout,
        ErrorCode::Internal,
    ]
    .iter()
    .any(|c| c.as_str() == code)
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
    hub: &Arc<Hub>,
    node_id: &str,
    rid: &str,
    call: &v1::BoardCall,
    open: &Arc<std::sync::atomic::AtomicUsize>,
    answers: tokio::sync::mpsc::UnboundedSender<v1::CommandResponse>,
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
    // ADR-0115 (#1150): what restoring the board owes agents (a task of an older board given a new id), whatever the call's answer.
    for n in &a.notices {
        board_notice(hub, n);
    }
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
        Verdict::Done { result, is_error } => bounded_board_result(result.to_string(), is_error),
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
            hub.board_revert(&handoff, now_ms());
            let _ = answers.send(error_response(rid, &e));
            return;
        }
    };
    open.fetch_add(1, Ordering::Relaxed);
    let (hub, open, rid, node) = (
        Arc::clone(hub),
        Arc::clone(open),
        rid.to_string(),
        handoff.to_node.clone(),
    );
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
            hub.board_revert(&handoff, now_ms());
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

/// The largest board tool result the hub sends a node (well inside one frame, `DEFAULT_MAX_PAYLOAD`, with room for the envelope).
pub(crate) const MAX_BOARD_RESULT: usize = agora_protocol::frame::DEFAULT_MAX_PAYLOAD - 64 * 1024;

/// A board tool result as it may be sent: one too big for a frame (a large board's whole `task_list`) becomes the tool error
/// `mesh.body_too_large` (`limit`), so the answer never ends the node's connection.
fn bounded_board_result(result: String, is_error: bool) -> (String, bool) {
    if result.len() <= MAX_BOARD_RESULT {
        return (result, is_error);
    }
    let e = Error::new(ErrorCode::MeshBodyTooLarge).with("limit", MAX_BOARD_RESULT as i64);
    (agora_mesh::mcp::protocol_error_result(&e).to_string(), true)
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
    let devices = crate::devices::Devices::new(hub.store());
    for node in hub.router().nodes_with("device.trust") {
        // An endorsement goes only to machines its device's person may act on (as the device list does); a revocation, which can
        // only narrow, goes everywhere.
        let relevant = match &command {
            command_request::Command::DeviceEndorse(d) => d.endorsement.as_ref().is_some_and(|e| {
                devices
                    .list_for_node(&node)
                    .is_ok_and(|l| l.iter().any(|x| x.device_id == e.device_id))
            }),
            _ => true,
        };
        if relevant && (nodes.is_empty() || nodes.contains(&node)) {
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
    let relevant: Vec<String> = crate::devices::Devices::new(hub.store())
        .list_for_node(node_id)
        .map(|l| l.into_iter().map(|d| d.device_id).collect())
        .unwrap_or_default();
    let commands = endorsements
        .into_iter()
        .filter(|e| e.nodes.is_empty() || e.nodes.iter().any(|n| n == node_id))
        .filter(|e| relevant.contains(&e.device_id))
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
    ca: Option<&Ca>,
    id: &crate::tls::NodeIdentity,
    first: Frame,
) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let node_id = id.node_id.as_str();
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
        // ADR-0085: renewal, only here (the connection's certificate verified and was found active before this function ran).
        Some(command_request::Command::Renew(r)) => match ca {
            Some(ca) => renew_certificate(hub, ca, id, &r, &req.request_id),
            // A hub without a CA issues nothing.
            None => error_response(
                &req.request_id,
                &Error::new(ErrorCode::ProtocolCapabilityMissing)
                    .with("capability", "enroll.renew"),
            ),
        },
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

/// ADR-0085: answers a machine's `RenewRequest`. The refusal reasons are `enroll.renew_refused`'s; a CSR that does not parse is an invalid
/// message; a store that cannot be read or written is `internal` (the machine keeps its certificate and asks again).
fn renew_certificate(
    hub: &Hub,
    ca: &Ca,
    id: &crate::tls::NodeIdentity,
    r: &v1::RenewRequest,
    request_id: &str,
) -> v1::CommandResponse {
    use crate::enroll::{RenewError, RenewFrom};
    let now = now_ms();
    let from = RenewFrom {
        serial_hex: &id.serial_hex,
        node_id: &id.node_id,
        not_after_secs: id.not_after_secs,
    };
    match Enrollment::new(hub.store()).renew(ca, &from, &r.csr_pem, now) {
        Ok(issued) => {
            // From the certificate itself: a certificate sent again keeps its own end.
            let not_after_ms = crate::ca::cert_not_after_secs(&issued.cert_pem)
                .ok()
                .and_then(|s| u64::try_from(s).ok())
                .map_or_else(
                    || now.saturating_add(ca.lifetime_ms()),
                    |s| s.saturating_mul(1000),
                );
            v1::CommandResponse {
                request_id: request_id.to_string(),
                result: Some(command_response::Result::Ok(v1::CommandOk {
                    result: Some(command_ok::Result::Renew(v1::RenewResponse {
                        cert_pem: issued.cert_pem,
                        not_after_ms,
                    })),
                })),
            }
        }
        Err(RenewError::Refused(why)) => error_response(
            request_id,
            &Error::new(ErrorCode::EnrollRenewRefused).with("reason", why.reason()),
        ),
        Err(RenewError::Ca(_)) => error_response(
            request_id,
            &Error::new(ErrorCode::ProtocolInvalidMessage).with("field", "csr_pem"),
        ),
        Err(RenewError::Store(_)) => error_response(
            request_id,
            &Error::new(ErrorCode::Internal).with("incident_id", "enroll.renew"),
        ),
    }
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

fn audit_command(
    hub: &Hub,
    who: &crate::users::Access,
    req: &v1::CommandRequest,
    action: &str,
    outcome: &str,
) {
    let (target, _) = hub.audit_target(req);
    let _ = crate::audit::Audit::new(hub.store()).record_target(
        &who.session,
        now_ms(),
        &who.user,
        action,
        outcome,
        &target,
    );
}

/// The audit row's action: the command's schema name, with the launch options a start chose (ADR-0059: `AgentLaunch[skip-permissions]`).
fn audit_action(req: &v1::CommandRequest) -> String {
    let options = match &req.command {
        Some(command_request::Command::AgentLaunch(a)) => a.launch_options.as_slice(),
        Some(command_request::Command::AgentStart(a)) => a.launch_options.as_slice(),
        _ => &[],
    };
    // ADR-0066: and the settings, `AgentLaunch[model=x,effort=high]`, so the audit trail shows what an agent was asked to run with.
    let settings = match &req.command {
        Some(command_request::Command::AgentLaunch(a)) => a.settings.as_slice(),
        Some(command_request::Command::AgentStart(a)) => a.settings.as_slice(),
        _ => &[],
    };
    let name = command_name(req);
    // The audit text is bounded here, before the node validates anything (what is sent to the node is not touched): at most
    // `AUDIT_MAX_CHOSEN` entries, each cut to `AUDIT_MAX_ENTRY` bytes, with a mark where something was left out.
    let mut chosen: Vec<String> = options
        .iter()
        .map(String::as_str)
        .map(clip_for_audit)
        .chain(
            settings
                .iter()
                .map(|st| format!("{}={}", clip_for_audit(&st.id), clip_for_audit(&st.value))),
        )
        .take(AUDIT_MAX_CHOSEN)
        .collect();
    let total = options.len() + settings.len();
    if total > AUDIT_MAX_CHOSEN {
        chosen.push(format!("+{}", total - AUDIT_MAX_CHOSEN));
    }
    if chosen.is_empty() {
        name
    } else {
        format!("{name}[{}]", chosen.join(","))
    }
}

/// Most launch options and settings written into one audit row (the manifest's own maximum of settings is 8; this is generous).
const AUDIT_MAX_CHOSEN: usize = 16;
/// Longest one id, value or option written into an audit row, in bytes.
const AUDIT_MAX_ENTRY: usize = 128;

/// `text` cut to [`AUDIT_MAX_ENTRY`] bytes at a character boundary, with `~` where it was cut.
fn clip_for_audit(text: &str) -> String {
    if text.len() <= AUDIT_MAX_ENTRY {
        return text.to_string();
    }
    let mut end = AUDIT_MAX_ENTRY;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}~", &text[..end])
}

use crate::dispatch::command_name;

pub(crate) fn error_response(request_id: &str, e: &Error) -> v1::CommandResponse {
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
                    not_after_ms: now_ms() + ca.lifetime_ms(),
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
    conn: Conn<S>,
    token: Option<&str>,
    state: &ServerState,
) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    serve_unauthenticated_with(conn, token, state, None).await
}

/// [`serve_unauthenticated`] for a connection whose TLS layer also knows where it came from and its pairing exporter (TLS 1.3 only):
/// only such a connection can pair (ADR-0065).
pub async fn serve_unauthenticated_with<S>(
    conn: Conn<S>,
    token: Option<&str>,
    state: &ServerState,
    pairing: Option<crate::pairing_net::PairPeer>,
) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    serve_unauthenticated_within(conn, token, state, pairing, NODE_SILENCE).await
}

/// [`serve_unauthenticated_with`] where the first frame must arrive within `first_frame`: a peer that says nothing is dropped as a
/// silent node is (ADR-0070, audit F880).
async fn serve_unauthenticated_within<S>(
    mut conn: Conn<S>,
    token: Option<&str>,
    state: &ServerState,
    pairing: Option<crate::pairing_net::PairPeer>,
    first_frame: Duration,
) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let first = match tokio::time::timeout(first_frame, conn.recv()).await {
        Ok(r) => r?,
        Err(_) => return Err(ServeError::Silent),
    };
    let Some(first) = first else {
        return Ok(());
    };
    match (first.stream_id, first.kind) {
        (STREAM_COMMANDS, 0) => {
            // A pairing request is told apart from an enrollment by its first command, nothing else.
            let pairs = matches!(
                v1::CommandRequest::decode(first.payload.as_slice())
                    .ok()
                    .and_then(|r| r.command),
                Some(
                    command_request::Command::PairRequest(_)
                        | command_request::Command::PairReveal(_)
                )
            );
            if pairs {
                return match pairing {
                    Some(peer) => {
                        crate::pairing_net::serve_pairing(
                            conn,
                            &state.hub,
                            state.ca.as_ref(),
                            first,
                            peer,
                        )
                        .await
                    }
                    // No TLS 1.3 binding to bring: pairing is not possible on this connection.
                    None => Err(ServeError::Refused),
                };
            }
            match &state.ca {
                Some(ca) => serve_enroll(conn, &state.hub, ca, &state.lockout, first).await,
                None => Err(ServeError::Refused),
            }
        }
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
    let _ = handle(&state, tcp, tls, None).await;
}

/// `leaf_hash` is the hash of the certificate `tls` presents, when the caller knows it (a [`crate::tls::TlsSlot`] does).
async fn handle(
    state: &ServerState,
    tcp: tokio::net::TcpStream,
    tls: Option<tokio_rustls::TlsAcceptor>,
    leaf_hash: Option<[u8; 32]>,
) -> Result<(), ServeError> {
    handle_within(state, tcp, tls, leaf_hash, NODE_SILENCE).await
}

/// [`handle`] where each step of opening a connection (the TLS handshake, the request head, the WebSocket upgrade, an
/// unauthenticated peer's first frame) must be done within `opening`; a peer that stalls in any of them is dropped as a silent
/// node is (ADR-0070, audit F880). The conversation after that keeps its own deadlines.
async fn handle_within(
    state: &ServerState,
    tcp: tokio::net::TcpStream,
    tls: Option<tokio_rustls::TlsAcceptor>,
    leaf_hash: Option<[u8; 32]>,
    opening: Duration,
) -> Result<(), ServeError> {
    // Who is on the other end: a sign-in from another machine needs the screen password (ADR-0067), the hub's own does not.
    let peer = tcp.peer_addr().ok().map(|a| a.ip());
    let Some(acceptor) = tls else {
        // Without TLS nothing can prove it is a node: only the token path exists (development and tests).
        let Some(tcp) = websocket_only(tcp, peer, state, opening).await? else {
            return Ok(());
        };
        let (conn, token) = upgrade(tcp, opening).await?;
        return serve_unauthenticated_within(conn, token.as_deref(), state, None, opening).await;
    };
    let stream = tokio::time::timeout(opening, acceptor.accept(tcp))
        .await
        .map_err(|_| ServeError::Silent)?
        .map_err(|e| ServeError::Transport(TransportError::Ws(e.to_string())))?;
    let identity = crate::tls::identity(stream.get_ref().1.peer_certificates());
    // ADR-0065: a pairing machine presents no certificate; what binds its number to this TLS session is read here.
    let pairing = match (&identity, peer) {
        (None, Some(addr)) => crate::tls::pairing_exporter(stream.get_ref().1).map(|exporter| {
            crate::pairing_net::PairPeer {
                addr,
                exporter,
                leaf_hash,
            }
        }),
        _ => None,
    };
    let Some(stream) = websocket_only(stream, peer, state, opening).await? else {
        return Ok(());
    };
    let (conn, token) = upgrade(stream, opening).await?;
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
            // ADR-0085: the first connection with a certificate starts the week after which the machine's older certificates are revoked.
            // Failing to record it must not stop a machine that is allowed in.
            if let Err(e) = Enrollment::new(state.hub.store()).note_connected(
                &id.node_id,
                &id.serial_hex,
                now_ms(),
            ) {
                eprintln!("node {}: could not note its certificate: {e:?}", id.node_id);
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
                return serve_node_query(conn, &state.hub, state.ca.as_ref(), &id, first).await;
            }
            // Revoking a node ends its connection, not only its next one (checked inside the conversation, so it ends as any other).
            // While the connection is open, only a record that says "revoked" ends it: a store that cannot be read right now is
            // logged and the connection kept (#558 review), where the check at connect time above refuses on any doubt.
            let active = || match Enrollment::new(state.hub.store()).is_active(&id.serial_hex) {
                Ok(active) => active,
                Err(_) => {
                    eprintln!(
                        "node {}: enrollment record unreadable; connection kept",
                        id.node_id
                    );
                    true
                }
            };
            serve_node_from(
                conn,
                Some(first),
                &id.node_id,
                &state.hub,
                &state.support,
                NODE_SILENCE,
                &active,
            )
            .await
        }
        None => serve_unauthenticated_within(conn, token.as_deref(), state, pairing, opening).await,
    }
}

/// The WebSocket upgrade of `stream`, done within `opening` (audit F880).
async fn upgrade<S>(stream: S, opening: Duration) -> Result<(Conn<S>, Option<String>), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    tokio::time::timeout(opening, agora_protocol::ws::accept_with_token(stream))
        .await
        .map_err(|_| ServeError::Silent)?
        .map_err(ServeError::from)
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
    read: Duration,
) -> Result<Option<crate::web::Rewind<S>>, ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    match crate::web::route_within(
        stream,
        peer,
        state.assets.as_deref(),
        state.sessions.as_deref(),
        read,
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
    run_with(listener, state, move || tls.clone().map(|a| (a, None))).await;
}

/// Like [`run`], with a TLS acceptor that can be replaced while the hub runs (a new leaf from the same CA, spec section 5b). Each
/// connection takes the slot's current acceptor when it is accepted and keeps it.
pub async fn run_slot(
    listener: tokio::net::TcpListener,
    state: Arc<ServerState>,
    slot: crate::tls::TlsSlot,
) {
    run_with(listener, state, move || {
        let (acceptor, hash) = slot.current();
        Some((acceptor, Some(hash)))
    })
    .await;
}

type Chosen = Option<(tokio_rustls::TlsAcceptor, Option<[u8; 32]>)>;

async fn run_with(
    listener: tokio::net::TcpListener,
    state: Arc<ServerState>,
    tls: impl Fn() -> Chosen,
) {
    while let Ok((tcp, _)) = listener.accept().await {
        // Small frames (a keystroke, an echo) must leave at once: without this, Nagle's algorithm plus the peer's delayed
        // ACK holds one back for up to 40 ms.
        let _ = tcp.set_nodelay(true);
        let (acceptor, leaf_hash) = match tls() {
            Some((a, h)) => (Some(a), h),
            None => (None, None),
        };
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            let _ = handle(&state, tcp, acceptor, leaf_hash).await;
        });
    }
}

#[cfg(test)]
mod snapshot_budget_tests {
    use super::*;
    use crate::scope::Scope;
    use v1::event::Body;

    const SNAPSHOT_POS: u64 = 500;

    fn record(view: &mut View, pos: u64, node: &str, body: Body) -> v1::PositionedEvent {
        let event = v1::Event {
            producer_id: node.into(),
            body: Some(body),
            ..Default::default()
        };
        view.apply_at(pos, &event);
        v1::PositionedEvent {
            pos,
            event: Some(event),
        }
    }

    fn fixture(binary: &str) -> View {
        let mut view = View::default();
        record(
            &mut view,
            1,
            "n1",
            Body::NodeOnline(v1::NodeOnline {
                node_id: "n1".into(),
                binary: binary.into(),
                ..Default::default()
            }),
        );
        record(
            &mut view,
            2,
            "n1",
            Body::AgentStarted(v1::AgentStarted {
                node_id: "n1".into(),
                agent_id: "sender".into(),
                project_id: "p".into(),
                ..Default::default()
            }),
        );
        record(
            &mut view,
            3,
            "n1",
            Body::ApprovalRequested(v1::ApprovalRequested {
                approval_id: "pad".into(),
                agent_id: "sender".into(),
                // Keep the padding in the same protobuf length-varint range through the 1 MiB boundary.
                detail: "x".repeat(512 * 1024),
                ..Default::default()
            }),
        );
        view
    }

    fn say(view: &mut View, pos: u64, node: &str, from: &str, body: &str) -> v1::PositionedEvent {
        record(
            view,
            pos,
            node,
            Body::MessagePosted(v1::MessagePosted {
                message_id: format!("m-{pos}"),
                from_agent_id: from.into(),
                to: "sender".into(),
                body: body.into(),
            }),
        )
    }

    fn wire_size(message: &v1::PositionedEvent) -> usize {
        v1::Snapshot {
            recent_messages: vec![message.clone()],
            ..Default::default()
        }
        .encoded_len()
    }

    fn padding(view: &mut View) -> &mut String {
        let Some(approval) = view.approvals.get_mut("n1/pad") else {
            panic!("fixture approval is missing");
        };
        &mut approval.detail
    }

    fn leave_space(view: &mut View, bytes: usize, snapshot: impl Fn(&View) -> v1::Snapshot) {
        let mut base = View {
            messages: Arc::default(),
            ..view.clone()
        };
        let additional = DEFAULT_MAX_PAYLOAD - bytes - snapshot(&base).encoded_len();
        let detail = "x".repeat(additional);
        padding(view).push_str(&detail);
        padding(&mut base).push_str(&detail);
        assert_eq!(snapshot(&base).encoded_len(), DEFAULT_MAX_PAYLOAD - bytes);
    }

    #[test]
    fn snapshot_history_counts_the_complete_protobuf_element_at_the_boundary() {
        let mut view = fixture("");
        let message = say(&mut view, 128, "n1", "sender", &"a".repeat(256));
        let snapshot = |view: &View| snapshot_of(view, SNAPSHOT_POS);
        leave_space(&mut view, wire_size(&message), snapshot);

        let exact = snapshot(&view);
        assert_eq!(exact.encoded_len(), DEFAULT_MAX_PAYLOAD);
        assert_eq!(exact.recent_messages, [message]);

        padding(&mut view).push('x');
        let short = snapshot(&view);
        assert!(short.recent_messages.is_empty());
        assert!(short.encoded_len() <= DEFAULT_MAX_PAYLOAD);
    }

    #[test]
    fn snapshot_history_stops_at_the_first_message_that_does_not_fit() {
        let mut view = fixture("");
        let old = say(&mut view, 128, "n1", "sender", "older");
        say(&mut view, 129, "n1", "sender", &"x".repeat(16 * 1024));
        let newest = say(&mut view, 130, "n1", "sender", "newest");
        let snapshot = |view: &View| snapshot_of(view, SNAPSHOT_POS);
        leave_space(&mut view, wire_size(&old) + wire_size(&newest), snapshot);

        let result = snapshot(&view);
        assert!(result.encoded_len() <= DEFAULT_MAX_PAYLOAD);
        assert_eq!(result.recent_messages, [newest]);
    }

    #[test]
    fn a_member_budgets_only_its_project_and_sender_copies() {
        let mut view = fixture("/opt/releases/agora");
        record(
            &mut view,
            4,
            "n1",
            Body::AgentStarted(v1::AgentStarted {
                node_id: "n1".into(),
                agent_id: "other".into(),
                project_id: "private".into(),
                ..Default::default()
            }),
        );
        let visible = say(
            &mut view,
            128,
            "n1",
            "sender",
            &"shared project ".repeat(10),
        );
        say(&mut view, 129, "n1", "other", &"p".repeat(1024));
        say(&mut view, 130, "n1", "n2/remote", &"r".repeat(1024));
        let member = |view: &View| {
            let scope = Scope::from_view(["n1/p".into()].into(), view);
            snapshot_of(&scope.filter_view(view), SNAPSHOT_POS)
        };
        leave_space(&mut view, wire_size(&visible), member);
        assert!(snapshot_of(&view, SNAPSHOT_POS).recent_messages.is_empty());

        let result = member(&view);
        assert_eq!(result.encoded_len(), DEFAULT_MAX_PAYLOAD);
        assert_eq!(result.recent_messages, [visible]);
        assert_eq!(result.nodes[0].binary, "agora");
        assert_eq!(result.agents.len(), 1);
        assert_eq!(result.agents[0].agent_id, "n1/sender");
    }

    #[test]
    fn the_shortened_binary_path_frees_budget_for_a_non_owner() {
        let mut view = fixture("/opt/a-machine-specific-installation-directory/agora");
        let message = say(&mut view, 128, "n1", "sender", &"hello ".repeat(40));
        let non_owner = |view: &View| snapshot_for_client(view, SNAPSHOT_POS, false);
        leave_space(&mut view, wire_size(&message), non_owner);

        let result = non_owner(&view);
        assert_eq!(result.encoded_len(), DEFAULT_MAX_PAYLOAD);
        assert_eq!(result.nodes[0].binary, "agora");
        assert_eq!(result.recent_messages, [message]);
        let owner = snapshot_of(&view, SNAPSHOT_POS);
        assert!(owner.encoded_len() <= DEFAULT_MAX_PAYLOAD);
        assert!(owner.recent_messages.is_empty());
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod event_writer_tests {
    use super::*;
    use crate::store::EventStore;
    use tokio::io::DuplexStream;
    use tokio::sync::oneshot;

    fn support() -> Support {
        Support {
            generations: vec![1],
            capabilities: vec![],
            required: vec![],
        }
    }

    fn hello() -> v1::Hello {
        v1::Hello {
            role: v1::Role::Node as i32,
            protocol_generations: vec![1],
            ..Default::default()
        }
    }

    fn online(node: &str, seq: u64) -> Vec<u8> {
        v1::Event {
            producer_id: node.into(),
            seq,
            at_ms: 1,
            body: Some(v1::event::Body::NodeOnline(v1::NodeOnline {
                node_id: node.into(),
                ..Default::default()
            })),
        }
        .encode_to_vec()
    }

    async fn recv(node: &mut Conn<DuplexStream>) -> Frame {
        tokio::time::timeout(Duration::from_secs(2), node.recv())
            .await
            .expect("node reply")
            .unwrap()
            .expect("open node link")
    }

    async fn ping(node: &mut Conn<DuplexStream>) {
        let payload = b"ping".to_vec();
        node.send(&frame(STREAM_CONTROL, kinds::PING, payload.clone()))
            .await
            .unwrap();
        let reply = recv(node).await;
        assert_eq!((reply.stream_id, reply.kind), (STREAM_CONTROL, kinds::PONG));
        assert_eq!(reply.payload, payload);
    }

    async fn connect(
        hub: &Arc<Hub>,
        node_id: &'static str,
    ) -> (
        Conn<DuplexStream>,
        tokio::task::JoinHandle<Result<(), ServeError>>,
    ) {
        let (mut node, link) = open_link(hub, node_id, &[]).await;
        // This comes after the hub has recorded the link open, which also writes to SQLite.
        ping(&mut node).await;
        (node, link)
    }

    /// A node connection through `Hello` and an empty `Resume` (the node has nothing to send), both sides offering `caps`.
    async fn open_link(
        hub: &Arc<Hub>,
        node_id: &'static str,
        caps: &[&str],
    ) -> (
        Conn<DuplexStream>,
        tokio::task::JoinHandle<Result<(), ServeError>>,
    ) {
        let caps: Vec<String> = caps.iter().map(|c| (*c).to_string()).collect();
        let (client, server) = tokio::io::duplex(8192);
        let hub = Arc::clone(hub);
        let offered = caps.clone();
        let link = tokio::spawn(async move {
            let (conn, _) = agora_protocol::ws::accept_with_token(server).await.unwrap();
            let support = Support {
                capabilities: offered,
                ..support()
            };
            serve_node(conn, node_id, &hub, &support).await
        });
        let mut node = agora_protocol::ws::connect(client, "ws://localhost/")
            .await
            .unwrap();
        let hello = v1::Hello {
            capabilities: caps,
            ..hello()
        };
        node.send(&frame(STREAM_CONTROL, kinds::HELLO, hello.encode_to_vec()))
            .await
            .unwrap();
        assert_eq!(recv(&mut node).await.kind, kinds::WELCOME);
        node.send(&frame(STREAM_CONTROL, kinds::RESUME, Vec::new()))
            .await
            .unwrap();
        assert_eq!(recv(&mut node).await.kind, kinds::RESUME_OK);
        (node, link)
    }

    async fn hold_store(
        hub: &Arc<Hub>,
    ) -> (std::sync::mpsc::Sender<()>, std::thread::JoinHandle<()>) {
        let (release, held) = std::sync::mpsc::channel();
        let (locked, ready) = oneshot::channel();
        let hub = Arc::clone(hub);
        let holder = std::thread::spawn(move || {
            let _store = hub.store().lock().unwrap();
            locked.send(()).unwrap();
            // A test failure must release the connection mutex.
            let _ = held.recv_timeout(Duration::from_secs(5));
        });
        ready.await.unwrap();
        (release, holder)
    }

    #[tokio::test]
    async fn a_node_waiting_for_sqlite_does_not_delay_another_nodes_ping() {
        let hub = Arc::new(Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap());
        let (mut sender, sending) = connect(&hub, "sender").await;
        let (mut peer, peering) = connect(&hub, "peer").await;
        let (release, holder) = hold_store(&hub).await;

        sender
            .send(&frame(STREAM_EVENTS, kinds::EVENT, online("sender", 1)))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while hub.event_writer.available_permits() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("event write admitted");

        ping(&mut peer).await;
        // The write is still queued on the store while the other connection serves its ping.
        assert_eq!(hub.event_writer.available_permits(), 0);
        release.send(()).expect("store still held");
        let ack = recv(&mut sender).await;
        assert_eq!(ack.kind, kinds::EVENT_ACK);
        assert_eq!(
            v1::EventAck::decode(ack.payload.as_slice())
                .unwrap()
                .up_to_seq,
            1
        );
        holder.join().unwrap();
        sender.close().await;
        peer.close().await;
        sending.await.unwrap().unwrap();
        peering.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn a_batch_preserves_gap_replies_and_resync_control_frames() {
        let hub = Arc::new(Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap());
        let (mut node, link) = connect(&hub, "n").await;
        // The gap has to be reported before the later valid events in the same burst are processed.
        for seq in [2, 1, 2, 4] {
            node.send(&frame(STREAM_EVENTS, kinds::EVENT, online("n", seq)))
                .await
                .unwrap();
        }
        node.send(&frame(STREAM_CONTROL, kinds::SNAPSHOT_REQUIRED, Vec::new()))
            .await
            .unwrap();
        node.send(&frame(STREAM_EVENTS, kinds::EVENT, online("n", 4)))
            .await
            .unwrap();

        for expected in [1, 3] {
            let reply = recv(&mut node).await;
            assert_eq!(reply.kind, kinds::RESUME_OK);
            assert_eq!(
                v1::ResumeOk::decode(reply.payload.as_slice())
                    .unwrap()
                    .from_seq,
                expected
            );
        }
        let reply = recv(&mut node).await;
        assert_eq!(reply.kind, kinds::EVENT_ACK);
        assert_eq!(
            v1::EventAck::decode(reply.payload.as_slice())
                .unwrap()
                .up_to_seq,
            4
        );
        let seqs: Vec<_> = hub
            .store()
            .events_after(0, 20)
            .unwrap()
            .into_iter()
            .filter(|event| event.producer == "n")
            .map(|event| event.seq)
            .collect();
        assert_eq!(seqs, [1, 2, 4]);
        assert_eq!(hub.store().resync_count().unwrap(), 1);
        node.close().await;
        link.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn a_cancelled_wait_can_still_settle_its_admitted_event_before_link_cleanup() {
        let hub = Arc::new(Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap());
        let mut session = NodeSession::new(hub.shared_store(), "n", &support());
        session.hello(&hello(), 1).unwrap();
        session.resume(&v1::Resume::default()).unwrap();
        let (registration, _, generation) = hub.node_link_opened("n", vec![], 1).unwrap();
        let (release, holder) = hold_store(&hub).await;
        let mut events = NodeEvents {
            session: Some(session),
            writing: None,
        };
        let mut waiting = Box::pin(events.write(&hub, VecDeque::from([online("n", 1)])));
        tokio::select! {
            biased;
            _ = &mut waiting => panic!("write returned while the store is held"),
            () = std::future::ready(()) => {}
        }
        drop(waiting);

        assert!(events.writing.is_some());
        assert_eq!(hub.event_writer.available_permits(), 0);
        release.send(()).unwrap();
        assert_eq!(events.settle().await.unwrap().acked, Some(1));
        holder.join().unwrap();
        assert!(hub.node_link_closed(registration, generation, false, 1, 2));
        assert_eq!(hub.store().acked("n").unwrap(), 1);
        assert!(!hub.view().0.nodes["n"].online);
    }

    #[tokio::test]
    async fn handoff_reverts_when_giver_disconnects_before_receiver_rejects() {
        let hub = Arc::new(Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap());
        let (mut alpha, alpha_link) = connect(&hub, "alpha").await;
        let (mut beta, beta_link) = connect(&hub, "beta").await;

        let register = |node: &str, seq: u64, body| {
            v1::Event {
                producer_id: node.into(),
                seq,
                at_ms: 1000 + seq,
                body: Some(body),
            }
            .encode_to_vec()
        };
        for (conn, node) in [(&mut alpha, "alpha"), (&mut beta, "beta")] {
            for (seq, body) in [
                (
                    1,
                    v1::event::Body::NodeOnline(v1::NodeOnline {
                        node_id: node.into(),
                        hostname: node.into(),
                        ..Default::default()
                    }),
                ),
                (
                    2,
                    v1::event::Body::WorkspaceRegistered(v1::WorkspaceRegistered {
                        workspace_id: "w".into(),
                        node_id: node.into(),
                        project_id: "p".into(),
                        path: "/proj".into(),
                        ..Default::default()
                    }),
                ),
                (
                    3,
                    v1::event::Body::AgentStarted(v1::AgentStarted {
                        agent_id: "a".into(),
                        node_id: node.into(),
                        project_id: "p".into(),
                        kind: "shell".into(),
                        ..Default::default()
                    }),
                ),
            ] {
                conn.send(&frame(
                    STREAM_EVENTS,
                    kinds::EVENT,
                    register(node, seq, body),
                ))
                .await
                .unwrap();
            }
            loop {
                let ack = recv(conn).await;
                if ack.kind == kinds::EVENT_ACK
                    && v1::EventAck::decode(ack.payload.as_slice())
                        .unwrap()
                        .up_to_seq
                        >= 3
                {
                    break;
                }
            }
        }

        let board = |node: &str, tool: &str, args: serde_json::Value| -> serde_json::Value {
            let call = v1::BoardCall {
                origin: node.into(),
                from_agent: "a".into(),
                project_id: "p".into(),
                tool: tool.into(),
                arguments_json: args.to_string(),
            };
            match hub.board_call(node, &call, 2000).verdict {
                crate::board::Verdict::Done { result, is_error } => {
                    assert!(!is_error);
                    serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
                }
                other => panic!("unexpected verdict: {other:?}"),
            }
        };
        let task_id = board(
            "alpha",
            "task_create",
            serde_json::json!({"title": "t1", "depends_on": []}),
        )["task_id"]
            .as_str()
            .unwrap()
            .to_string();
        board(
            "alpha",
            "task_claim",
            serde_json::json!({"task_id": task_id}),
        );

        let req = v1::CommandRequest {
            request_id: "handoff-1".into(),
            idempotency_key: String::new(),
            target_node: String::new(),
            command: Some(command_request::Command::BoardCall(v1::BoardCall {
                origin: "alpha".into(),
                from_agent: "a".into(),
                project_id: "p".into(),
                tool: "handoff".into(),
                arguments_json: serde_json::json!({
                    "to": "beta/a",
                    "task_id": task_id,
                    "summary": "s",
                    "refs": []
                })
                .to_string(),
            })),
        };
        alpha
            .send(&frame(STREAM_COMMANDS, 0, req.encode_to_vec()))
            .await
            .unwrap();
        let forwarded = loop {
            let f = recv(&mut beta).await;
            if f.stream_id == STREAM_COMMANDS && f.kind == 0 {
                break v1::CommandRequest::decode(f.payload.as_slice()).unwrap();
            }
        };

        // Giver disconnects before receiver replies with delivery failure.
        alpha.close().await;
        let _ = alpha_link.await;

        let err_resp = v1::CommandResponse {
            request_id: forwarded.request_id,
            result: Some(command_response::Result::Err(v1::ErrorInfo {
                code: "agent.not_found".into(),
                ..Default::default()
            })),
        };
        beta.send(&frame(STREAM_COMMANDS, 1, err_resp.encode_to_vec()))
            .await
            .unwrap();

        let mut reverted = false;
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(10)).await;
            let tasks = board("beta", "task_list", serde_json::json!({}));
            if tasks[0]["owner"] == "alpha/a" {
                reverted = true;
                break;
            }
        }
        beta.close().await;
        let _ = beta_link.await;
        assert!(reverted, "handoff must revert after giver disconnects");
    }

    /// #996: a machine whose connection ends while the store refuses writes is recorded offline once the store works again, with
    /// no reconnect, no restart and nothing else happening on the hub. Before, the failed `NodeLink { up: false }` was dropped and
    /// the machine that had left stayed online in every view until it came back or the hub restarted.
    #[tokio::test]
    async fn a_machine_that_leaves_while_the_store_refuses_writes_is_shown_offline_once_it_works() {
        let path = std::env::temp_dir().join(format!("agora-996-{}.db", std::process::id()));
        let remove = |path: &std::path::Path| {
            for ext in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{ext}", path.display()));
            }
        };
        remove(&path);
        let hub = Arc::new(Hub::open(SqliteStore::open(&path).unwrap()).unwrap());
        let (mut node, link) = connect(&hub, "n").await;
        assert!(hub.view().0.nodes["n"].online);
        let other = rusqlite::Connection::open(&path).unwrap();
        other
            .execute_batch("CREATE TRIGGER no_events BEFORE INSERT ON events BEGIN SELECT RAISE(ABORT, 'full'); END;")
            .unwrap();
        node.close().await;
        link.await.unwrap().unwrap();
        assert!(!hub.router().is_connected("n"));
        assert!(hub.has_unrecorded());
        // The retry fails at least once while the store still refuses.
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(hub.has_unrecorded());
        other.execute_batch("DROP TRIGGER no_events;").unwrap();
        let offline = tokio::time::timeout(Duration::from_secs(10), async {
            while hub.view().0.nodes["n"].online {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        // The retry task lets go once it is done, so a later close can start another.
        let released = tokio::time::timeout(Duration::from_secs(5), async {
            while hub
                .recording_later
                .load(std::sync::atomic::Ordering::Acquire)
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        let rebuilt = crate::projection::View::rebuild(hub.store()).unwrap().0;
        drop(hub);
        remove(&path);
        assert!(offline.is_ok(), "the machine that left stayed online");
        assert!(released.is_ok(), "the retry task kept its claim");
        assert!(!rebuilt.nodes["n"].online);
    }
    /// #1036 (Codex review): a machine whose end the store refused, and whose redial then writes that end and its new `up`
    /// together, is asked to hand over its local board work (`BoardJoin`) by the redial itself, even when it sends no event (an
    /// idle node that is caught up only resumes and answers pings, ADR-0054) and no retry task is left to look at membership
    /// (the redial wrote the end before the closing connection checked for one). Before, the ask waited for the node's next event.
    #[tokio::test]
    async fn an_idle_machine_whose_redial_writes_its_end_is_asked_to_rejoin_the_board() {
        let path = std::env::temp_dir().join(format!("agora-1036-{}.db", std::process::id()));
        let remove = |path: &std::path::Path| {
            for ext in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{ext}", path.display()));
            }
        };
        remove(&path);
        let hub = Arc::new(Hub::open(SqliteStore::open(&path).unwrap()).unwrap());
        let mut links = Vec::new();
        for node in ["alpha", "beta", "gamma"] {
            let (mut conn, link) = open_link(&hub, node, &["board.join"]).await;
            for (seq, body) in [
                (
                    1,
                    v1::event::Body::NodeOnline(v1::NodeOnline {
                        node_id: node.into(),
                        ..Default::default()
                    }),
                ),
                (
                    2,
                    v1::event::Body::AgentStarted(v1::AgentStarted {
                        agent_id: "a".into(),
                        node_id: node.into(),
                        project_id: "p".into(),
                        kind: "shell".into(),
                        ..Default::default()
                    }),
                ),
            ] {
                let event = v1::Event {
                    producer_id: node.into(),
                    seq,
                    at_ms: seq,
                    body: Some(body),
                };
                conn.send(&frame(STREAM_EVENTS, kinds::EVENT, event.encode_to_vec()))
                    .await
                    .unwrap();
            }
            // Other frames (the `BoardJoin` asks of the project becoming shared) are left unanswered.
            loop {
                let f = recv(&mut conn).await;
                if f.kind == kinds::EVENT_ACK
                    && v1::EventAck::decode(f.payload.as_slice())
                        .unwrap()
                        .up_to_seq
                        >= 2
                {
                    break;
                }
            }
            links.push((conn, link));
        }
        assert!(hub.board_membership(now_ms()).is_empty());
        for node in ["alpha", "beta", "gamma"] {
            hub.board_join_settled(node, "p");
        }

        // No retry task will look at membership: as when the redial wrote the end before the closing connection checked.
        hub.recording_later
            .store(true, std::sync::atomic::Ordering::Release);
        let other = rusqlite::Connection::open(&path).unwrap();
        other
            .execute_batch("CREATE TRIGGER no_events BEFORE INSERT ON events BEGIN SELECT RAISE(ABORT, 'full'); END;")
            .unwrap();
        let (mut alpha, alpha_link) = links.remove(0);
        alpha.close().await;
        alpha_link.await.unwrap().unwrap();
        assert!(hub.has_unrecorded());
        assert!(hub.view().0.nodes["alpha"].online);
        other.execute_batch("DROP TRIGGER no_events;").unwrap();

        // The redial: resume and a ping, no event.
        let (mut alpha, alpha_link) = open_link(&hub, "alpha", &["board.join"]).await;
        assert!(!hub.has_unrecorded());
        alpha
            .send(&frame(STREAM_CONTROL, kinds::PING, b"ping".to_vec()))
            .await
            .unwrap();
        let (mut ponged, mut asked) = (false, Vec::new());
        let seen = tokio::time::timeout(Duration::from_secs(2), async {
            while !(ponged && !asked.is_empty()) {
                let f = alpha.recv().await.unwrap().expect("open node link");
                if (f.stream_id, f.kind) == (STREAM_CONTROL, kinds::PONG) {
                    ponged = true;
                } else if (f.stream_id, f.kind) == (STREAM_COMMANDS, 0) {
                    if let Some(command_request::Command::BoardJoin(j)) =
                        v1::CommandRequest::decode(f.payload.as_slice())
                            .unwrap()
                            .command
                    {
                        asked.push(j.project_id);
                    }
                }
            }
        })
        .await;
        let left_over = hub.board_membership(now_ms());
        hub.recording_later
            .store(false, std::sync::atomic::Ordering::Release);
        alpha.close().await;
        let _ = alpha_link.await;
        for (mut conn, link) in links {
            conn.close().await;
            let _ = link.await;
        }
        drop(hub);
        remove(&path);
        assert!(ponged, "the redialled node got no pong");
        assert!(
            seen.is_ok(),
            "the redialled node was not asked to rejoin the board"
        );
        assert_eq!(asked, ["p"]);
        assert!(left_over.is_empty(), "{left_over:?}");
    }
}

#[cfg(test)]
mod audit_cleanup_tests {
    use super::*;

    /// Codex review of #883: a board answer too big for one frame (a large board's whole `task_list`) becomes a tool error instead of a
    /// frame that would end the node's connection.
    #[test]
    fn a_board_answer_too_big_for_a_frame_becomes_a_tool_error() {
        let small = "[]".to_string();
        assert_eq!(bounded_board_result(small.clone(), false), (small, false));
        let (text, is_error) = bounded_board_result("x".repeat(MAX_BOARD_RESULT + 1), false);
        assert!(is_error);
        assert!(text.len() < 1024, "{}", text.len());
        assert!(text.contains("mesh.body_too_large"), "{text}");
        let ok = "y".repeat(MAX_BOARD_RESULT);
        assert_eq!(bounded_board_result(ok.clone(), false), (ok, false));
    }

    /// Review F7: the audit text is bounded in count and in the length of each entry, with a mark, and the request is not touched.
    #[test]
    fn the_audit_text_of_a_launch_is_bounded_and_marked() {
        let many = |n: usize, id: &str, value: &str| v1::CommandRequest {
            command: Some(command_request::Command::AgentLaunch(v1::AgentLaunch {
                settings: (0..n)
                    .map(|_| v1::LaunchSetting {
                        id: id.into(),
                        value: value.into(),
                    })
                    .collect(),
                ..Default::default()
            })),
            ..Default::default()
        };
        let req = many(40, &"i".repeat(500), &"\u{e9}".repeat(300));
        let text = audit_action(&req);
        assert!(text.len() < 16 * (2 * 129 + 3) + 64, "{}", text.len());
        assert!(text.contains(&format!("{}~=", "i".repeat(128))), "{text}");
        assert!(text.ends_with(",+24]"), "{text}");
        assert_eq!(text.matches('~').count(), 32, "{text}");
        // Nothing of the request itself was cut.
        let Some(command_request::Command::AgentLaunch(a)) = &req.command else {
            unreachable!()
        };
        assert_eq!((a.settings.len(), a.settings[0].id.len()), (40, 500));
        // A small one is written as it is.
        assert_eq!(audit_action(&many(1, "model", "x")), "AgentLaunch[model=x]");
    }

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
            fresh_command: true,
            reply,
            answered: None,
            deadline,
            confined: None,
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod opening_tests {
    use super::*;
    use agora_protocol::frame::{kinds, STREAM_EVENTS};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    const OPENING: Duration = Duration::from_millis(300);
    const BOUND: Duration = Duration::from_secs(5);

    struct Tokens;
    impl ClientAuth for Tokens {
        fn check(&self, token: &str) -> bool {
            token == "good-token"
        }
    }

    async fn serve() -> std::net::SocketAddr {
        let state = Arc::new(ServerState {
            hub: Arc::new(
                crate::hub::Hub::open(crate::store::SqliteStore::open_in_memory().unwrap())
                    .unwrap(),
            ),
            support: Support {
                generations: vec![1],
                capabilities: vec![],
                required: vec![],
            },
            ca: None,
            lockout: Arc::new(crate::enroll::Lockout::new()),
            auth: Box::new(Tokens),
            assets: None,
            sessions: None,
        });
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((tcp, _)) = l.accept().await {
                let st = Arc::clone(&state);
                tokio::spawn(async move {
                    let _ = handle_within(&st, tcp, None, None, OPENING).await;
                });
            }
        });
        addr
    }

    async fn ended(mut s: TcpStream) -> bool {
        let mut buf = [0u8; 64];
        matches!(
            tokio::time::timeout(BOUND, s.read(&mut buf)).await,
            Ok(Ok(0)) | Ok(Err(_))
        )
    }

    /// Audit F880: a peer that opens a connection and then says nothing, or drips its request head a byte at a time, was held
    /// with no deadline; now each step of the opening has one. A client that is prompt keeps its conversation past it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_stalled_opening_is_dropped_and_a_prompt_client_is_kept() {
        let addr = serve().await;
        assert!(ended(TcpStream::connect(addr).await.unwrap()).await);
        let s = TcpStream::connect(addr).await.unwrap();
        let mut ws = agora_protocol::ws::connect(s, &format!("ws://{addr}/"))
            .await
            .unwrap();
        assert!(matches!(
            tokio::time::timeout(BOUND, ws.recv()).await,
            Ok(Ok(None)) | Ok(Err(_))
        ));
        let (mut r, mut w) = TcpStream::connect(addr).await.unwrap().into_split();
        let drip = tokio::spawn(async move {
            for byte in
                b"GET / HTTP/1.1\r\nHost: a-name-long-enough-to-outlast-the-deadline-xxxxxxxx"
            {
                if w.write_all(&[*byte]).await.is_err() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        });
        let mut buf = [0u8; 64];
        assert!(matches!(
            tokio::time::timeout(BOUND, r.read(&mut buf)).await,
            Ok(Ok(0)) | Ok(Err(_))
        ));
        assert!(!drip.is_finished());
        drip.abort();

        let s = TcpStream::connect(addr).await.unwrap();
        let mut good =
            agora_protocol::ws::connect_with_token(s, &format!("ws://{addr}/"), "good-token")
                .await
                .unwrap();
        good.send(&agora_protocol::Frame {
            stream_id: STREAM_EVENTS,
            kind: kinds::SUBSCRIBE,
            seq: None,
            payload: v1::Subscribe::default().encode_to_vec(),
        })
        .await
        .unwrap();
        // ADR-0106: the role first, then the snapshot.
        let welcome = tokio::time::timeout(BOUND, good.recv()).await;
        assert!(matches!(welcome, Ok(Ok(Some(f))) if f.kind == kinds::WELCOME));
        let snap = tokio::time::timeout(BOUND, good.recv()).await;
        assert!(matches!(snap, Ok(Ok(Some(f))) if f.kind == kinds::SNAPSHOT));
        assert!(tokio::time::timeout(OPENING * 4, good.recv())
            .await
            .is_err());
    }
}
