//! The node's connection to the hub (ADR-0011): dials `wss://` with the node's client certificate, runs the
//! [`Uplink`] conversation, and reconnects with backoff when the connection drops. Nodes are autonomous: nothing here
//! can stop an agent, and when the hub is unreachable events simply wait in the durable buffer.
//!
//! The hub is trusted only through the CA the node was given at enrollment (pinned), never the system roots.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use agora_protocol::frame::{
    kinds, FIRST_DYNAMIC_STREAM, STREAM_CLOSE, STREAM_COMMANDS, STREAM_CONTROL, STREAM_EVENTS,
};
use agora_protocol::v1::{
    command_request, command_response, CommandRequest, CommandResponse, EventAck, ResumeOk, Welcome,
};
use agora_protocol::ws::{self, Conn, TransportError};
use agora_protocol::Frame;
use prost::Message;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;

use crate::daemon::EventSink;
use crate::uplink::{Replay, Uplink, UplinkError};

/// What this node offers the hub when it dials in. The hub agrees only on what it supports too, so each one must also be in the
/// hub's list (`crates/agora/src/hubcli.rs`, checked there by a test), or the hub refuses every command that needs it.
pub const CAPABILITIES: &[&str] = &[
    "terminal.raw",
    "workspace.directory",
    "board.join",
    "mesh.spawn",
    "directory.list",
    // ADR-0058
    "agent.set_mode",
    "agent.set_model",
    // ADR-0060
    "agent.resume",
    // ADR-0059
    "agent.launch_options",
];

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
    /// The node's own control socket. Commands the hub routes here (ADR-0029) run through it, so they get exactly the
    /// handler, policy and approval rules a local command gets. `None`: such commands are refused.
    pub commands: Option<std::path::PathBuf>,
    /// Where this node's agents' messages for agents on other nodes are carried to the hub (ADR-0031).
    pub mesh_out: Option<Arc<MeshOut>>,
    /// Grants for the commands this node's policy lists in `signed_commands` (ADR-0041). `None`: no routed command is gated.
    pub grants: Option<Arc<crate::grants::Grants>>,
    /// The node's mesh, for the hub's `BoardJoin` (ADR-0039 amendment B), which is answered here and never through the control
    /// socket. `None`: `BoardJoin` is refused.
    pub mesh: Option<Arc<crate::mesh_server::MeshServer>>,
}

type Done = Box<dyn FnOnce(Result<Option<crate::mesh_server::BoardReply>, String>) + Send>;

/// A board tool call (ADR-0039): project, tool, arguments as JSON.
type BoardArgs = (String, String, String);

struct PostJob {
    from_agent: String,
    to: String,
    body: String,
    refs: Vec<String>,
    board: Option<BoardArgs>,
    done: Option<Done>,
}

impl PostJob {
    /// The parts, with the obligation to call `done` handed over.
    fn take(mut self) -> (String, String, String, Vec<String>, Option<BoardArgs>, Done) {
        let done = self.done.take().unwrap_or_else(|| Box::new(|_| {}));
        (
            std::mem::take(&mut self.from_agent),
            std::mem::take(&mut self.to),
            std::mem::take(&mut self.body),
            std::mem::take(&mut self.refs),
            self.board.take(),
            done,
        )
    }
}

// A job that is dropped unsent (the connection ended with it still queued) still tells its sender.
impl Drop for PostJob {
    fn drop(&mut self) {
        if let Some(done) = self.done.take() {
            done(Err("node.offline".into()));
        }
    }
}

/// The uplink's side of [`RemoteLink`](crate::mesh_server::RemoteLink): jobs are handed to whichever hub connection is up
/// and fail at once with `node.offline` when none is. Delivery is at most once; nothing is kept for later.
#[derive(Default)]
pub struct MeshOut {
    jobs: std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedSender<PostJob>>>,
}

impl MeshOut {
    /// A job that cannot be queued is dropped, which fails it with `node.offline`.
    fn queue(&self, job: PostJob) {
        let tx = self.jobs.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(tx) = tx {
            let _ = tx.send(job);
        }
    }

    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn connect(&self) -> tokio::sync::mpsc::UnboundedReceiver<PostJob> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        *self.jobs.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
        rx
    }

    fn disconnect(&self) {
        *self.jobs.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

impl crate::mesh_server::RemoteLink for MeshOut {
    fn post(
        &self,
        from_agent: &str,
        to: &str,
        body: &str,
        refs: &[String],
        done: Box<dyn FnOnce(Result<(), String>) + Send>,
    ) {
        self.queue(PostJob {
            from_agent: from_agent.to_string(),
            to: to.to_string(),
            body: body.to_string(),
            refs: refs.to_vec(),
            board: None,
            done: Some(Box::new(move |r| done(r.map(|_| ())))),
        });
    }

    fn board(
        &self,
        from_agent: &str,
        project: &str,
        tool: &str,
        arguments_json: &str,
        done: Box<dyn FnOnce(Result<crate::mesh_server::BoardReply, String>) + Send>,
    ) {
        self.queue(PostJob {
            from_agent: from_agent.to_string(),
            to: String::new(),
            body: String::new(),
            refs: Vec::new(),
            board: Some((
                project.to_string(),
                tool.to_string(),
                arguments_json.to_string(),
            )),
            done: Some(Box::new(move |r| {
                done(r.and_then(|b| b.ok_or_else(|| "protocol.invalid_message".to_string())))
            })),
        });
    }
}

/// Posts sent on one connection and not yet answered, by the request id this node chose. When the connection ends they
/// all fail with `node.offline`.
#[derive(Default)]
struct Posts {
    next: u64,
    waiting: std::collections::HashMap<String, Done>,
}

impl Drop for Posts {
    fn drop(&mut self) {
        for (_, done) in self.waiting.drain() {
            done(Err("node.offline".into()));
        }
    }
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
        match e {
            // The hub ended the connection saying why (for example `protocol.producer_mismatch`): logged as that code.
            TransportError::Refused(code) => Self::Refused(code),
            e => Self::Transport(e.to_string()),
        }
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
    // Keystrokes and echoes are small frames; do not let Nagle's algorithm hold them back.
    let _ = tcp.set_nodelay(true);
    if tls {
        let name = ServerName::try_from(host.clone()).map_err(|e| LinkError::Bad(e.to_string()))?;
        let stream = connector(link)?
            .connect(name, tcp)
            .await
            .map_err(|e| LinkError::Transport(e.to_string()))?;
        let conn = ws::connect(stream, &link.url).await?;
        converse(
            conn,
            sink,
            stop,
            link.commands.as_deref(),
            link.mesh_out.as_deref(),
            link.grants.clone(),
            link.mesh.clone(),
        )
        .await
    } else {
        let conn = ws::connect(tcp, &link.url).await?;
        converse(
            conn,
            sink,
            stop,
            link.commands.as_deref(),
            link.mesh_out.as_deref(),
            link.grants.clone(),
            link.mesh.clone(),
        )
        .await
    }
}

async fn converse<S>(
    mut conn: Conn<S>,
    sink: &Arc<EventSink>,
    stop: &AtomicBool,
    commands: Option<&std::path::Path>,
    mesh_out: Option<&MeshOut>,
    grants: Option<Arc<crate::grants::Grants>>,
    mesh: Option<Arc<crate::mesh_server::MeshServer>>,
) -> Result<(), LinkError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (answers_tx, mut answers) = tokio::sync::mpsc::unbounded_channel::<CommandResponse>();
    // Terminal frames from relay threads, bounded so a slow hub slows the thread (and, through the local socket,
    // makes the node's own pump resync) instead of growing a queue.
    let (relay_tx, mut relay_rx) = tokio::sync::mpsc::channel::<Frame>(256);
    let mut relays = Relays::default();
    // The worker for commands whose order matters; it ends when this connection (and so `ordered`) is dropped.
    let (ordered, ordered_rx) = std::sync::mpsc::channel::<(
        Option<std::path::PathBuf>,
        CommandRequest,
        tokio::sync::mpsc::UnboundedSender<CommandResponse>,
    )>();
    let ordered_grants = grants.clone();
    std::thread::spawn(move || {
        // One connection to the node's control socket for all keystrokes: a new connection per key would cost more than
        // the key. It is opened on first use and again after any error.
        let mut kept: Option<crate::control::Client> = None;
        while let Ok((socket, req, tx)) = ordered_rx.recv() {
            let _ = tx.send(run_local_on(
                &mut kept,
                socket.as_deref(),
                req,
                ordered_grants.as_deref(),
            ));
        }
    });
    // Messages this node's agents send to agents on other nodes go out on this connection while it is up.
    let mut jobs = mesh_out.map(MeshOut::connect);
    let _disconnect = Disconnect(mesh_out);
    let mut posts = Posts::default();
    let mut up = Uplink::new(Arc::clone(sink), env!("CARGO_PKG_VERSION"), CAPABILITIES);
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
    let replay = up.on_resume_ok(&ok)?;
    send_replay(&mut conn, replay).await?;

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
        let received = tokio::select! {
            r = tokio::time::timeout(TICK, conn.recv()) => r,
            Some(answer) = answers.recv() => {
                conn.send(&frame(STREAM_COMMANDS, 1, answer.encode_to_vec())).await?;
                continue;
            }
            Some(f) = relay_rx.recv() => {
                conn.send(&f).await?;
                continue;
            }
            Some(job) = async { match jobs.as_mut() { Some(j) => j.recv().await, None => std::future::pending().await } } => {
                let (from_agent, to, body, refs, board, done) = job.take();
                posts.next += 1;
                let id = format!("m{}", posts.next);
                let req = CommandRequest {
                    request_id: id.clone(),
                    idempotency_key: String::new(),
                    target_node: String::new(),
                    command: Some(match board {
                        Some((project_id, tool, arguments_json)) => {
                            command_request::Command::BoardCall(agora_protocol::v1::BoardCall {
                                project_id,
                                tool,
                                arguments_json,
                                origin: String::new(),
                                from_agent,
                            })
                        }
                        None => command_request::Command::MeshPost(agora_protocol::v1::MeshPost {
                            to,
                            body,
                            origin: String::new(),
                            from_agent,
                            refs,
                        }),
                    }),
                };
                posts.waiting.insert(id, done);
                conn.send(&frame(STREAM_COMMANDS, 0, req.encode_to_vec())).await?;
                continue;
            }
        };
        match received {
            Err(_) => {}
            Ok(Ok(None)) => return Err(LinkError::Closed),
            Ok(Err(e)) => return Err(e.into()),
            Ok(Ok(Some(f))) => match (f.stream_id, f.kind) {
                (STREAM_EVENTS, kinds::EVENT_ACK) => {
                    if let Ok(a) = EventAck::decode(f.payload.as_slice()) {
                        up.on_ack(a.up_to_seq)?;
                    }
                }
                // A command the hub routes to this node: run it locally, answer when it is done.
                (STREAM_COMMANDS, 0) => {
                    let Ok(req) = CommandRequest::decode(f.payload.as_slice()) else {
                        continue;
                    };
                    // ADR-0039 amendment B: only the hub asks a node to hand over its local board work, so it is answered here,
                    // where the hub's commands enter, and never through the local control socket.
                    if let Some(command_request::Command::BoardJoin(j)) = &req.command {
                        let mesh = mesh.clone();
                        let tx = answers_tx.clone();
                        let (rid, project) = (req.request_id.clone(), j.project_id.clone());
                        tokio::task::spawn_blocking(move || {
                            let _ = tx.send(board_join_answer(mesh.as_deref(), &rid, &project));
                        });
                        continue;
                    }
                    let socket = commands.map(std::path::Path::to_path_buf);
                    let tx = answers_tx.clone();
                    // Keystrokes and resizes must reach the terminal in the order they were typed, so they share one
                    // worker; everything else may run side by side.
                    if matches!(
                        req.command,
                        Some(
                            command_request::Command::TerminalInput(_)
                                | command_request::Command::TerminalResize(_)
                        )
                    ) {
                        let _ = ordered.send((socket, req, tx));
                    } else {
                        let grants = grants.clone();
                        tokio::task::spawn_blocking(move || {
                            let _ = tx.send(run_local(socket.as_deref(), req, grants.as_deref()));
                        });
                    }
                }
                // The hub's answer to a mesh post this node sent.
                (STREAM_COMMANDS, 1) => {
                    if let Ok(r) = CommandResponse::decode(f.payload.as_slice()) {
                        if let Some(done) = posts.waiting.remove(&r.request_id) {
                            done(match r.result {
                                Some(command_response::Result::Err(e)) => Err(e.code),
                                Some(command_response::Result::Ok(ok)) => match ok.result {
                                    Some(agora_protocol::v1::command_ok::Result::BoardResult(
                                        b,
                                    )) => Ok(Some(crate::mesh_server::BoardReply {
                                        shared: b.shared,
                                        result_json: b.result_json,
                                    })),
                                    _ => Ok(None),
                                },
                                None => Ok(None),
                            });
                        }
                    }
                }
                // A terminal subscription: a thread bridges the node's local stream to this connection.
                (stream, 0) if stream >= FIRST_DYNAMIC_STREAM => {
                    let Ok(req) = CommandRequest::decode(f.payload.as_slice()) else {
                        continue;
                    };
                    let alive = Arc::new(AtomicBool::new(true));
                    relays.0.insert(stream, Arc::clone(&alive));
                    let (socket, answers, out) = (
                        commands.map(std::path::Path::to_path_buf),
                        answers_tx.clone(),
                        relay_tx.clone(),
                    );
                    tokio::task::spawn_blocking(move || {
                        relay_terminal(socket.as_deref(), &req, stream, &alive, &answers, &out);
                    });
                }
                (stream, STREAM_CLOSE) if stream >= FIRST_DYNAMIC_STREAM => {
                    if let Some(alive) = relays.0.remove(&stream) {
                        alive.store(false, Ordering::Release);
                    }
                }
                // The hub saw a gap and says where to start again.
                (STREAM_CONTROL, kinds::RESUME_OK) => {
                    if let Ok(r) = ResumeOk::decode(f.payload.as_slice()) {
                        let replay = up.on_gap(r.from_seq)?;
                        send_replay(&mut conn, replay).await?;
                    }
                }
                _ => {}
            },
        }
    }
    conn.close().await;
    Ok(())
}

/// After the hub said where to start: when the node no longer has those events it tells the hub before sending any more
/// (ADR-0054). A node that can replay sends nothing here.
async fn send_replay<S>(conn: &mut Conn<S>, replay: Replay) -> Result<(), LinkError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if let Replay::Resync {
        from_seq,
        oldest_retained,
    } = replay
    {
        // Codes and numbers only: this goes to the node's log, not to the user's screen.
        eprintln!("hub link: resync, events {from_seq}..{oldest_retained} are gone");
        conn.send(&frame(STREAM_CONTROL, kinds::SNAPSHOT_REQUIRED, Vec::new()))
            .await?;
    }
    Ok(())
}

/// Marks the mesh link down when the connection's conversation ends, however it ends.
struct Disconnect<'a>(Option<&'a MeshOut>);

impl Drop for Disconnect<'_> {
    fn drop(&mut self) {
        if let Some(m) = self.0 {
            m.disconnect();
        }
    }
}

/// The relayed terminals of one connection; when it ends their threads are told to stop.
#[derive(Default)]
struct Relays(std::collections::HashMap<u32, Arc<AtomicBool>>);

impl Drop for Relays {
    fn drop(&mut self) {
        for alive in self.0.values() {
            alive.store(false, Ordering::Release);
        }
    }
}

/// Bridges one terminal subscription: opens the stream on the node's control socket, answers the hub's request,
/// then copies the terminal's frames (snapshot, output, resize, exit) to the hub on `stream` until the hub closes it,
/// the terminal exits or the connection ends. Blocking: runs on its own thread.
fn relay_terminal(
    socket: Option<&std::path::Path>,
    req: &CommandRequest,
    stream: u32,
    alive: &AtomicBool,
    answers: &tokio::sync::mpsc::UnboundedSender<CommandResponse>,
    out: &tokio::sync::mpsc::Sender<Frame>,
) {
    use agora_protocol::{Error, ErrorCode};
    let refuse = |e: Error| {
        let _ = answers.send(CommandResponse {
            request_id: req.request_id.clone(),
            result: Some(command_response::Result::Err(e.to_info())),
        });
    };
    let Some(socket) = socket else {
        return refuse(
            Error::new(ErrorCode::NodePolicyDenied)
                .with("node_id", "")
                .with("rule", "commands.routed"),
        );
    };
    let Ok(mut local) = crate::control::Client::connect(socket) else {
        return refuse(Error::new(ErrorCode::NodeTimeout).with("node_id", ""));
    };
    let Ok(resp) = local.request_on(stream, req, ROUTED_DEADLINE) else {
        return refuse(Error::new(ErrorCode::NodeTimeout).with("node_id", ""));
    };
    let accepted = matches!(resp.result, Some(command_response::Result::Ok(_)));
    let _ = answers.send(resp);
    while accepted && alive.load(Ordering::Acquire) {
        match local.read_frame(Some(Duration::from_millis(200))) {
            Ok(Some(f)) if f.stream_id == stream => {
                let exited = f.kind == 3;
                if out.blocking_send(f).is_err() || exited {
                    return;
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    // The local stream ended without an exit frame (the node closed it): tell the hub so the client can resubscribe.
    if alive.load(Ordering::Acquire) && accepted {
        let _ = out.blocking_send(frame(stream, STREAM_CLOSE, Vec::new()));
    }
}

/// The node's answer to the hub's `BoardJoin`: its unfinished local work for the project, or why it gives none.
fn board_join_answer(
    mesh: Option<&crate::mesh_server::MeshServer>,
    request_id: &str,
    project: &str,
) -> CommandResponse {
    use agora_protocol::{Error, ErrorCode};
    let result = match mesh {
        _ if project.is_empty() => {
            Err(Error::new(ErrorCode::ProtocolInvalidMessage).with("field", "project_id"))
        }
        None => {
            Err(Error::new(ErrorCode::ProtocolCapabilityMissing).with("capability", "board.join"))
        }
        Some(m) => m.board_join(project),
    };
    CommandResponse {
        request_id: request_id.to_string(),
        result: Some(match result {
            Ok(work) => command_response::Result::Ok(agora_protocol::v1::CommandOk {
                result: Some(agora_protocol::v1::command_ok::Result::BoardJoin(work)),
            }),
            Err(e) => command_response::Result::Err(e.to_info()),
        }),
    }
}

/// Runs a routed command on this node's control socket and returns its answer (an error answer when it cannot).
/// Terminal streams are not relayed by this path (they need a stream of their own), so they are refused.
fn run_local(
    socket: Option<&std::path::Path>,
    req: CommandRequest,
    grants: Option<&crate::grants::Grants>,
) -> CommandResponse {
    run_local_on(&mut None, socket, req, grants)
}

/// Like [`run_local`], over `kept` when it holds a live connection, and leaving the connection there for the next call.
fn run_local_on(
    kept: &mut Option<crate::control::Client>,
    socket: Option<&std::path::Path>,
    req: CommandRequest,
    grants: Option<&crate::grants::Grants>,
) -> CommandResponse {
    use agora_protocol::{Error, ErrorCode};
    let fail = |e: Error| CommandResponse {
        request_id: req.request_id.clone(),
        result: Some(command_response::Result::Err(e.to_info())),
    };
    // ADR-0059: launch options come through the hub only on a signed `AgentLaunch` (its signature covers them); a routed
    // `AgentStart` carries no signature, so one with options is refused whatever grant is live (a grant never covers one).
    if let Some(command_request::Command::AgentStart(a)) = &req.command {
        if let Some(option) = a.launch_options.first() {
            return fail(
                Error::new(ErrorCode::AgentOptionForbidden)
                    .with("kind", a.kind.as_str())
                    .with("option", option.as_str())
                    .with("policy", "signature"),
            );
        }
    }
    // ADR-0041: grants are made and judged here, where the hub's commands enter the node. A command the policy lists
    // runs only under a live grant; the local socket never passes through this function.
    if grants.is_none()
        && matches!(
            req.command,
            Some(
                command_request::Command::DirectoryInspect(_)
                    | command_request::Command::DirectoryList(_)
                    | command_request::Command::AgentLaunch(_)
                    | command_request::Command::AgentResume(_)
            )
        )
    {
        return fail(
            Error::new(ErrorCode::NodePolicyDenied)
                .with("rule", "directory.authorization")
                .with("node_id", ""),
        );
    }
    if let Some(g) = grants {
        let now = crate::grants::now_ms();
        let done = |r: Result<agora_protocol::v1::command_ok::Result, Error>| CommandResponse {
            request_id: req.request_id.clone(),
            result: Some(match r {
                Ok(ok) => {
                    command_response::Result::Ok(agora_protocol::v1::CommandOk { result: Some(ok) })
                }
                Err(e) => command_response::Result::Err(e.to_info()),
            }),
        };
        use agora_protocol::v1::command_ok::Result as Ok_;
        match &req.command {
            Some(
                c @ (command_request::Command::DirectoryInspect(_)
                | command_request::Command::DirectoryList(_)
                | command_request::Command::AgentLaunch(_)
                | command_request::Command::AgentResume(_)),
            ) => {
                if let Err(e) = g.directory_action(c, now) {
                    return fail(e);
                }
            }
            Some(command_request::Command::GrantCreate(c)) => {
                return done(
                    g.create(c, now)
                        .map(|()| Ok_::Empty(agora_protocol::v1::Empty {})),
                );
            }
            Some(command_request::Command::GrantList(_)) => {
                return done(Ok(Ok_::GrantList(agora_protocol::v1::GrantListResult {
                    grants: g.list(now),
                })));
            }
            Some(command_request::Command::GrantRevoke(r)) => {
                g.revoke(&r.grant_id);
                return done(Ok(Ok_::Empty(agora_protocol::v1::Empty {})));
            }
            Some(c) => {
                if let Err(e) = g.gate(c, now) {
                    return fail(e);
                }
            }
            None => {}
        }
    }
    if matches!(
        req.command,
        Some(
            command_request::Command::TerminalSubscribe(_)
                | command_request::Command::ChatSubscribe(_)
        )
    ) {
        return fail(
            Error::new(ErrorCode::ProtocolCapabilityMissing).with("capability", "terminal.relay"),
        );
    }
    let Some(socket) = socket else {
        return fail(
            Error::new(ErrorCode::NodePolicyDenied)
                .with("node_id", "")
                .with("rule", "commands.routed"),
        );
    };
    // A kept connection may have gone stale since the last key; one fresh attempt follows its failure.
    for attempt in 0..2 {
        let client = match kept.take() {
            Some(c) if attempt == 0 => Ok(c),
            _ => crate::control::Client::connect(socket),
        };
        if let Ok(mut c) = client {
            if let Ok(resp) = c.request(&req, ROUTED_DEADLINE) {
                *kept = Some(c);
                return resp;
            }
        }
    }
    fail(Error::new(ErrorCode::NodeTimeout).with("node_id", ""))
}

/// Longest a routed command may take on the node before the hub is told it timed out.
const ROUTED_DEADLINE: Duration = Duration::from_secs(25);

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
                    let mut last = String::new();
                    while !stop.load(Ordering::Acquire) {
                        let started = tokio::time::Instant::now();
                        let r = run_once(&link, &sink, &stop).await;
                        if stop.load(Ordering::Acquire) {
                            break;
                        }
                        // Why the hub could not be reached, once per change, so a node that never connects is not silent. Codes and
                        // transport detail only: this goes to the node's log, not to the user's screen.
                        if let Err(e) = &r {
                            let now = format!("{e:?}");
                            if now != last {
                                eprintln!("hub link: {now}");
                                last = now;
                            }
                        } else {
                            last.clear();
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
            target_node: String::new(),
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

    /// ADR-0059: a routed `AgentStart` that chose launch options carries no signature over them, so it is refused where the hub's
    /// commands enter the node, with or without grants, and even under a live grant for `agent.start` (a grant never covers one).
    #[test]
    fn a_routed_start_with_launch_options_is_refused_even_under_a_grant() {
        struct Fake;
        impl crate::approvals::Verifier for Fake {
            fn verify(&self, device: &str, message: &[u8], signature: &[u8]) -> bool {
                device == "dev" && signature == message
            }
        }
        let approvals = std::sync::Arc::new(crate::approvals::NodeApprovals::new(
            "node-a",
            Box::new(Fake),
        ));
        let grants = crate::grants::Grants::new(
            "node-a",
            approvals,
            crate::grants::SignedCommands(vec!["agent.start".into()]),
        );
        let now = crate::grants::now_ms();
        let commands = vec!["agent.start".to_string()];
        let expires = now + 60_000;
        let message = crate::grants::signed_message("node-a", "g", "node", &commands, expires);
        grants
            .create(
                &agora_protocol::v1::GrantCreate {
                    grant_id: "g".into(),
                    scope: "node".into(),
                    commands,
                    expires_at_ms: expires,
                    signer_device_id: "dev".into(),
                    signature: message,
                },
                now,
            )
            .unwrap();
        let start = |options: Vec<String>| CommandRequest {
            request_id: "r".into(),
            command: Some(command_request::Command::AgentStart(
                agora_protocol::v1::AgentStart {
                    kind: "codex".into(),
                    launch_options: options,
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        for g in [None, Some(&grants)] {
            let resp = run_local(None, start(vec!["skip-permissions".into()]), g);
            let Some(command_response::Result::Err(e)) = resp.result else {
                panic!("not refused: {resp:?}");
            };
            assert_eq!(e.code, "agent.option_forbidden");
            assert!(e.params.contains_key("policy"));
        }
        // Without options the grant lets it through to the node (here: no control socket, so `commands.routed`).
        let resp = run_local(None, start(Vec::new()), Some(&grants));
        let Some(command_response::Result::Err(e)) = resp.result else {
            panic!("{resp:?}");
        };
        assert_eq!(e.code, "node.policy_denied");
    }
}
