//! The node's mesh server: one MCP endpoint per local agent (ADR-0008), backed by a node-local broker. Agents on the
//! same machine coordinate with no hub (nodes are autonomous); a hub-side broker for agents on different machines
//! comes later, behind the same tools.
//!
//! Each agent gets its own local socket (owner-only). Whatever connects to it *is* that agent as far as the broker
//! is concerned, so the socket is only ever handed to that agent's process (through its MCP configuration); an agent
//! cannot reach another's socket by guessing a name in a tool call, because no call names a sender.

use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use agora_core::AgentId;
use agora_mesh::broker::{AgentInfo, AskStatus, Limits, Mesh};
use agora_mesh::mcp::{McpConnection, Outcome, BOARD_TOOLS};
use agora_platform::{local_listen_agent, LocalStream};
use agora_protocol::jsonrpc::{to_line, Message};
use agora_protocol::v1::event::Body;
use serde_json::{json, Value};

/// How long an agent's board call waits for the hub before it is treated as unreachable.
const BOARD_WAIT: Duration = Duration::from_secs(10);
/// How long "this project is not shared" is believed before the hub is asked again.
const LOCAL_FOR: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(100);
/// A longer line is a broken or hostile client; the connection is closed.
const MAX_LINE: usize = 1024 * 1024;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// How a mesh message for an agent on another node leaves this node (ADR-0031). The uplink implements it.
pub trait RemoteLink: Send + Sync {
    /// Hands one message to the hub. `done` is called once: `Ok(())` when the target node took it, or the error code
    /// (`node.offline` when there is no hub connection, `node.unknown`, `node.timeout`, `mesh.recipient_unknown`, ...).
    /// Must not block.
    fn post(
        &self,
        from_agent: &str,
        to: &str,
        body: &str,
        refs: &[String],
        done: Box<dyn FnOnce(Result<(), String>) + Send>,
    );

    /// Asks the hub to run a mesh tool on a project's shared board (ADR-0039). `done` is called once. Must not block.
    /// The default says the hub cannot be reached, so a link that does not know boards leaves everything local.
    fn board(
        &self,
        _from_agent: &str,
        _project: &str,
        _tool: &str,
        _arguments_json: &str,
        done: Box<dyn FnOnce(Result<BoardReply, String>) + Send>,
    ) {
        done(Err("node.offline".into()));
    }
}

/// What an agent asked `agent_start` for (ADR-0057), parsed; the node decides everything else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartArgs {
    pub kind: String,
    pub prompt: String,
    pub folder: Option<String>,
    pub worktree: bool,
    pub node: Option<String>,
}

/// The helper a start produced, or why it did not.
pub type StartOutcome = Result<String, agora_protocol::Error>;

/// The node's side of the spawn tools (ADR-0057), implemented by `crate::spawn::Spawner`. Calls must not block: a start waits for the
/// user, so it returns a ticket the connection polls.
pub trait Helpers: Send + Sync {
    /// Checks the request and either starts the helper under a grant or asks the user. The ticket is polled with [`Self::started`].
    fn start(&self, caller: &AgentId, args: &StartArgs) -> Result<u64, agora_protocol::Error>;
    /// `None` while the user has not decided.
    fn started(&self, ticket: u64) -> Option<StartOutcome>;
    /// The connection that asked went away: the ticket is dropped and its request, if still waiting, withdrawn.
    fn abandon(&self, ticket: u64);
    /// The state words of helpers `caller` started (`idle`, `working`, `blocked`, `done`, `unknown`, or `exited` once it ended), one
    /// per name, read from one view of the node.
    fn states(
        &self,
        caller: &AgentId,
        helpers: &[String],
    ) -> Vec<Result<String, agora_protocol::Error>>;
    /// The last `lines` of a helper `caller` started, as the tool result's JSON.
    fn output(
        &self,
        caller: &AgentId,
        helper: &str,
        lines: usize,
    ) -> Result<Value, agora_protocol::Error>;
}

type HelpersSlot = Mutex<Option<Arc<dyn Helpers>>>;

/// The hub's answer to a board call (ADR-0039): not shared (run it locally) or the tool's MCP result as JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardReply {
    pub shared: bool,
    pub result_json: String,
}

type RemoteSlot = Mutex<Option<Arc<dyn RemoteLink>>>;

/// Sends what the broker queued for other nodes. Called after every tool call, outside the broker's lock; a failure
/// comes back to the sending agent as a notice in its inbox.
fn dispatch_outbound(mesh: &Arc<Mutex<Mesh>>, remote: &RemoteSlot, allowed: &AtomicBool) {
    let out = mesh
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take_outbound();
    if out.is_empty() {
        return;
    }
    let link = remote.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let denied_or_offline = if allowed.load(Ordering::Acquire) {
        "node.offline"
    } else {
        "node.policy_denied"
    };
    for o in out {
        let to = format!("{}/{}", o.to_node, o.to_agent);
        let failed = {
            let (mesh, from, id, to) = (
                Arc::clone(mesh),
                o.from.clone(),
                o.message_id.clone(),
                to.clone(),
            );
            move |code: String| {
                mesh.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .delivery_failed(&from, &id, &to, &code, now_ms());
            }
        };
        match &link {
            Some(l) => {
                let failed = failed.clone();
                l.post(
                    o.from.as_str(),
                    &to,
                    &o.body,
                    &o.refs,
                    Box::new(move |r| {
                        if let Err(code) = r {
                            failed(code);
                        }
                    }),
                );
            }
            None => failed(denied_or_offline.to_string()),
        }
    }
}

/// What this node last learned from the hub about each project (ADR-0039). A project known to be shared fails closed when
/// the hub cannot be reached; one not known to be shared keeps working on the node's own broker.
#[derive(Default)]
struct Shares {
    shared: Mutex<HashMap<String, (bool, Instant)>>,
}

impl Shares {
    fn get(&self, project: &str) -> Option<(bool, Instant)> {
        self.shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(project)
            .copied()
    }

    fn set(&self, project: &str, shared: bool) {
        self.shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(project.to_string(), (shared, Instant::now()));
    }

    /// "Not shared", from an answer to a question asked at `asked`: ignored when something newer (a `BoardJoin`) said it is shared
    /// meanwhile, so a late answer cannot send the project back to the local broker.
    fn set_not_shared_since(&self, project: &str, asked: Instant) {
        let mut g = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(g.get(project), Some((true, at)) if *at >= asked) {
            return;
        }
        g.insert(project.to_string(), (false, Instant::now()));
    }
}

/// A call to one of the board tools: on a shared project the hub's board answers; `None` means "use the local broker".
fn board_reply(
    conn: &McpConnection,
    line: &str,
    mesh: &Arc<Mutex<Mesh>>,
    remote: &RemoteSlot,
    shares: &Shares,
) -> Option<Message> {
    if !conn.initialized() {
        return None;
    }
    let Ok(Message::Request { id, method, params }) = agora_protocol::jsonrpc::parse_line(line)
    else {
        return None;
    };
    let name = params.get("name").and_then(Value::as_str)?;
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    if !args.is_object() {
        return None;
    }
    // A message to a `#channel` of a shared project goes to every agent of the project, so it is the board's too.
    let channel = name == "send_message"
        && args
            .get("to")
            .and_then(Value::as_str)
            .is_some_and(|t| t.starts_with('#'));
    if method != "tools/call" || !(channel || BOARD_TOOLS.contains(&name)) {
        return None;
    }
    let project = mesh
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .agent_info(conn.agent())?
        .project
        .clone();
    let known = shares.get(&project);
    let unavailable = || {
        let text =
            json!({"error": {"code": "board.unavailable", "params": {"project_id": project}}});
        Message::Response {
            id: id.clone(),
            result: json!({"content": [{"type": "text", "text": text.to_string()}], "isError": true}),
        }
    };
    // No link (the node never had a hub, or `mesh.remote` is off): a project known to be shared must not fall back to local
    // state, or two machines could each believe they hold the same path.
    let Some(link) = remote.lock().unwrap_or_else(|e| e.into_inner()).clone() else {
        return matches!(known, Some((true, _))).then(unavailable);
    };
    if matches!(known, Some((false, at)) if at.elapsed() < LOCAL_FOR) {
        return None;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let asked = Instant::now();
    link.board(
        conn.agent().as_str(),
        &project,
        name,
        &args.to_string(),
        Box::new(move |r| {
            let _ = tx.send(r);
        }),
    );
    match rx.recv_timeout(BOARD_WAIT) {
        Ok(Ok(r)) if r.shared => {
            shares.set(&project, true);
            let result = serde_json::from_str(&r.result_json).ok();
            Some(match result {
                Some(result) => Message::Response {
                    id: id.clone(),
                    result,
                },
                None => unavailable(),
            })
        }
        Ok(Ok(_)) => {
            shares.set_not_shared_since(&project, asked);
            None
        }
        _ => matches!(known, Some((true, _))).then(unavailable),
    }
}

struct AgentServer {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    path: PathBuf,
}

pub struct MeshServer {
    /// ADR-0057: answers `agent_start`, `agent_wait` and `agent_output`; without it those tools are not listed.
    helpers: Arc<HelpersSlot>,
    /// Whether messages may cross to and from other nodes (node policy `mesh.remote`, ADR-0031).
    remote_allowed: Arc<AtomicBool>,
    remote: Arc<RemoteSlot>,
    shares: Arc<Shares>,
    mesh: Arc<Mutex<Mesh>>,
    dir: PathBuf,
    agents: Mutex<HashMap<AgentId, AgentServer>>,
    stop: Arc<AtomicBool>,
    ticker: Option<JoinHandle<()>>,
}

impl MeshServer {
    /// `dir` holds the per-agent sockets. Keep its path short: Unix socket paths are limited to about 100 bytes.
    pub fn new(dir: &Path, limits: Limits) -> io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let mesh = Arc::new(Mutex::new(Mesh::new(limits)));
        let stop = Arc::new(AtomicBool::new(false));
        // Leases expire on a clock, not on a call: sweep so a path frees up even if nobody asks about it.
        let ticker = {
            let (mesh, stop) = (Arc::clone(&mesh), Arc::clone(&stop));
            std::thread::Builder::new()
                .name("agora-mesh-tick".into())
                .spawn(move || {
                    while !stop.load(Ordering::Acquire) {
                        mesh.lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .expire_leases(now_ms());
                        std::thread::sleep(Duration::from_millis(500));
                    }
                })?
        };
        Ok(Self {
            helpers: Arc::new(Mutex::new(None)),
            remote_allowed: Arc::new(AtomicBool::new(true)),
            remote: Arc::new(Mutex::new(None)),
            shares: Arc::new(Shares::default()),
            mesh,
            dir: dir.to_path_buf(),
            agents: Mutex::new(HashMap::new()),
            stop,
            ticker: Some(ticker),
        })
    }

    /// ADR-0057: who answers the spawn tools. Connections made after this list them.
    pub fn set_helpers(&self, helpers: Arc<dyn Helpers>) {
        *self.helpers.lock().unwrap_or_else(|e| e.into_inner()) = Some(helpers);
    }

    /// Where messages for agents on other nodes go (the hub uplink). Without one they fail with `node.offline`.
    pub fn set_remote(&self, link: Arc<dyn RemoteLink>) {
        if !self.remote_allowed.load(Ordering::Acquire) {
            return;
        }
        *self.remote.lock().unwrap_or_else(|e| e.into_inner()) = Some(link);
    }

    /// Node policy `mesh.remote`: when false, messages to other nodes fail with `node.policy_denied` and messages from
    /// them are refused. Local messages are not affected.
    pub fn set_remote_allowed(&self, allowed: bool) {
        self.remote_allowed.store(allowed, Ordering::Release);
        if !allowed {
            *self.remote.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
    }

    /// A message from an agent on another node, as the hub vouched for it (ADR-0031).
    pub fn deliver_remote(
        &self,
        origin: &str,
        from_agent: &str,
        to: &AgentId,
        body: &str,
        refs: &[String],
    ) -> Result<(), agora_protocol::Error> {
        if !self.remote_allowed.load(Ordering::Acquire) {
            return Err(
                agora_protocol::Error::new(agora_protocol::ErrorCode::NodePolicyDenied)
                    .with("node_id", "")
                    .with("rule", "mesh.remote"),
            );
        }
        let mut m = self.mesh.lock().unwrap_or_else(|e| e.into_inner());
        let limits = m.limits();
        m.deliver_remote(origin, from_agent, to, body, refs, now_ms())
            .map(|_| ())
            .map_err(|e| e.to_error(&limits))
    }

    /// ADR-0039 amendment B: the hub says `project` became shared. Hands over this node's unfinished work for it (it is no longer
    /// kept here) and from now on asks the hub for the project's board calls instead of trusting an older "not shared".
    pub fn board_join(
        &self,
        project: &str,
    ) -> Result<agora_protocol::v1::BoardJoinResult, agora_protocol::Error> {
        use agora_protocol::v1::{BoardJoinLease, BoardJoinResult, BoardJoinTask};
        // `mesh.remote = false`: this node's mesh does not reach other nodes, so its work does not go to a shared board either.
        if !self.remote_allowed.load(Ordering::Acquire) {
            return Err(
                agora_protocol::Error::new(agora_protocol::ErrorCode::NodePolicyDenied)
                    .with("node_id", "")
                    .with("rule", "mesh.remote"),
            );
        }
        // Shared first: a board call from now on goes to the hub, so nothing new is made locally after the work is taken.
        self.shares.set(project, true);
        let now = now_ms();
        let (tasks, leases, any_agent) = {
            let mut mesh = self.mesh.lock().unwrap_or_else(|e| e.into_inner());
            let (tasks, leases) = mesh.take_unfinished(project, now);
            // The board records a creator for every task; the original one is not kept, so any agent of the project stands in.
            let any_agent = mesh
                .agents_list(Some(project))
                .first()
                .map(|a| a.id.as_str().to_string())
                .unwrap_or_default();
            (tasks, leases, any_agent)
        };
        Ok(BoardJoinResult {
            leases: leases
                .into_iter()
                .map(|(path, holder, expires_at_ms)| BoardJoinLease {
                    agent_id: holder.as_str().to_string(),
                    path,
                    expires_at_ms,
                })
                .collect(),
            tasks: tasks
                .into_iter()
                .map(|t| {
                    let owner = t.owner.map(|o| o.as_str().to_string()).unwrap_or_default();
                    BoardJoinTask {
                        task_id: t.id.as_str().to_string(),
                        title: t.title,
                        detail: t.detail,
                        creator_agent_id: if owner.is_empty() {
                            any_agent.clone()
                        } else {
                            owner.clone()
                        },
                        owner_agent_id: owner,
                    }
                })
                .collect(),
        })
    }

    pub fn mesh(&self) -> Arc<Mutex<Mesh>> {
        Arc::clone(&self.mesh)
    }

    pub fn socket_path(&self, agent: &AgentId) -> PathBuf {
        self.dir.join(format!("{}.sock", agent.as_str()))
    }

    /// Adds the agent to the mesh and starts serving its endpoint. Returns the socket path to put in its MCP
    /// configuration.
    pub fn register(&self, info: AgentInfo) -> io::Result<PathBuf> {
        self.register_confined(info, None)
    }

    /// Like [`Self::register`] for an agent that will run confined with `identity` (ADR-0050 Amendment B on Windows): its endpoint
    /// admits exactly that identity among confined processes, so one agent cannot speak on another's connection.
    pub fn register_confined(
        &self,
        info: AgentInfo,
        identity: Option<&str>,
    ) -> io::Result<PathBuf> {
        let id = info.id.clone();
        let path = self.socket_path(&id);
        let listener = local_listen_agent(&path, identity)?;
        listener.set_nonblocking(true)?;
        self.mesh
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .register(info);
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (mesh, stop, agent, remote, allowed, shares, helpers) = (
                Arc::clone(&self.mesh),
                Arc::clone(&stop),
                id.clone(),
                Arc::clone(&self.remote),
                Arc::clone(&self.remote_allowed),
                Arc::clone(&self.shares),
                Arc::clone(&self.helpers),
            );
            std::thread::Builder::new()
                .name("agora-mesh-agent".into())
                .spawn(move || {
                    let mut conns: Vec<JoinHandle<()>> = Vec::new();
                    while !stop.load(Ordering::Acquire) {
                        match listener.accept() {
                            Ok(stream) => {
                                let (mesh, stop, agent, remote, allowed, shares) = (
                                    Arc::clone(&mesh),
                                    Arc::clone(&stop),
                                    agent.clone(),
                                    Arc::clone(&remote),
                                    Arc::clone(&allowed),
                                    Arc::clone(&shares),
                                );
                                let helpers =
                                    helpers.lock().unwrap_or_else(|e| e.into_inner()).clone();
                                if let Ok(h) = std::thread::Builder::new()
                                    .name("agora-mesh-conn".into())
                                    .spawn(move || {
                                        serve(
                                            stream,
                                            &agent,
                                            &mesh,
                                            &Links {
                                                remote: &remote,
                                                allowed: &allowed,
                                                shares: &shares,
                                                helpers,
                                            },
                                            &stop,
                                        )
                                    })
                                {
                                    conns.push(h);
                                }
                            }
                            Err(_) => std::thread::sleep(POLL),
                        }
                        conns.retain(|h| !h.is_finished());
                    }
                    for h in conns {
                        let _ = h.join();
                    }
                })?
        };
        self.agents
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                id,
                AgentServer {
                    stop,
                    thread: Some(thread),
                    path: path.clone(),
                },
            );
        Ok(path)
    }

    /// The agent ended: it leaves the mesh (its tasks, leases and asks are given back) and its endpoint closes.
    pub fn unregister(&self, agent: &AgentId) {
        let server = self
            .agents
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(agent);
        if let Some(mut s) = server {
            s.stop.store(true, Ordering::Release);
            if let Some(t) = s.thread.take() {
                let _ = t.join();
            }
            let _ = std::fs::remove_file(&s.path);
        }
        self.mesh
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .unregister(agent, now_ms());
    }

    pub fn set_state(&self, agent: &AgentId, state: &str) {
        self.mesh
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .set_state(agent, state);
    }

    /// Audit events produced by mesh activity since the last call.
    pub fn take_events(&self) -> Vec<Body> {
        self.mesh
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take_events()
    }

    pub fn is_registered(&self, agent: &AgentId) -> bool {
        self.agents
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(agent)
    }
}

impl Drop for MeshServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.ticker.take() {
            let _ = t.join();
        }
        let ids: Vec<AgentId> = self
            .agents
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect();
        for id in ids {
            self.unregister(&id);
        }
    }
}

fn send(w: &mut LocalStream, msg: &Message) -> io::Result<()> {
    let mut line = to_line(msg);
    line.push('\n');
    w.write_all(line.as_bytes())?;
    w.flush()
}

/// Ask IDs owned by one transport, independent of its temporary polling queue. An early return while draining that queue
/// still cancels everything this connection can no longer consume, without affecting the agent's other connections.
struct ConnectionAsks<'a> {
    mesh: &'a Mutex<Mesh>,
    agent: &'a AgentId,
    ids: HashSet<String>,
}

impl Drop for ConnectionAsks<'_> {
    fn drop(&mut self) {
        if !self.ids.is_empty() {
            let mut mesh = self.mesh.lock().unwrap_or_else(|e| e.into_inner());
            for id in &self.ids {
                mesh.cancel_ask(self.agent, id);
            }
        }
    }
}

type WaitingAsk = (Value, String, u64);

fn poll_asks(
    waiting: &mut Vec<WaitingAsk>,
    owned: &mut ConnectionAsks<'_>,
    conn: &McpConnection,
    mut reply: impl FnMut(&Message) -> io::Result<()>,
) -> io::Result<()> {
    let mut still = Vec::new();
    for (rid, ask_id, deadline) in waiting.drain(..) {
        let (status, limits) = {
            let mut m = owned.mesh.lock().unwrap_or_else(|e| e.into_inner());
            (m.ask_status(&ask_id, now_ms()), m.limits())
        };
        if status == AskStatus::Pending {
            still.push((rid, ask_id, deadline));
        } else {
            owned.ids.remove(&ask_id); // ask_status already consumed or expired it
            reply(&conn.finish_ask(&rid, &status, &limits))?;
        }
    }
    *waiting = still;
    Ok(())
}

/// What a connection reaches besides its broker.
struct Links<'a> {
    remote: &'a RemoteSlot,
    allowed: &'a AtomicBool,
    shares: &'a Shares,
    helpers: Option<Arc<dyn Helpers>>,
}

/// The longest `agent_wait`, and the one used when none is given.
const WAIT_MAX_MS: u64 = 3_600_000;
const WAIT_DEFAULT_MS: u64 = 600_000;
/// `agent_wait` calls one connection may have open at once.
const MAX_WAITS: usize = 8;

/// A spawn tool call that waits: a start until the user decides, or a wait until the helper's state.
enum SpawnWait {
    Start {
        request_id: Value,
        ticket: u64,
    },
    State {
        request_id: Value,
        helper: String,
        wanted: String,
        deadline_ms: u64,
    },
}

/// Tickets of starts this connection is waiting for; dropped with it, so a request nobody will read is withdrawn.
struct ConnectionStarts {
    helpers: Option<Arc<dyn Helpers>>,
    tickets: HashSet<u64>,
}

impl Drop for ConnectionStarts {
    fn drop(&mut self) {
        if let Some(h) = &self.helpers {
            for t in &self.tickets {
                h.abandon(*t);
            }
        }
    }
}

fn tool_reply(id: &Value, result: Value) -> Message {
    Message::Response {
        id: id.clone(),
        result,
    }
}

fn reached(state: &str, wanted: &str) -> bool {
    // A wait ends at the state asked for, or when the helper exited (it will reach nothing else).
    state == wanted || state == "exited"
}

/// ADR-0057: a call to one of the spawn tools. `None` when the line is not one (or no helpers are offered), so the broker handles it.
fn spawn_call(
    conn: &McpConnection,
    line: &str,
    helpers: &Arc<dyn Helpers>,
    waits: &mut Vec<SpawnWait>,
    starts: &mut ConnectionStarts,
) -> Option<Option<Message>> {
    if !conn.initialized() {
        return None;
    }
    let Ok(Message::Request { id, method, params }) = agora_protocol::jsonrpc::parse_line(line)
    else {
        return None;
    };
    let name = params.get("name").and_then(Value::as_str)?;
    if method != "tools/call" || !agora_mesh::mcp::SPAWN_TOOLS.contains(&name) {
        return None;
    }
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    let bad = || {
        Some(Some(Message::ErrorResponse {
            id: id.clone(),
            code: agora_mesh::mcp::invalid_params_error(),
            data: None,
        }))
    };
    if !args.is_object() {
        return bad();
    }
    let s = |k: &str| args.get(k).and_then(Value::as_str);
    let opt = |k: &str| -> Result<Option<String>, ()> {
        match args.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(v)) => Ok(Some(v.clone())),
            Some(_) => Err(()),
        }
    };
    let me = conn.agent();
    let err = |e: agora_protocol::Error| {
        Some(Some(tool_reply(
            &id,
            agora_mesh::mcp::protocol_error_result(&e),
        )))
    };
    match name {
        "agent_start" => {
            let (Some(kind), Ok(prompt), Ok(folder), Ok(node)) =
                (s("kind"), opt("prompt"), opt("folder"), opt("node"))
            else {
                return bad();
            };
            let worktree = match args.get("worktree") {
                None | Some(Value::Null) => false,
                Some(Value::Bool(b)) => *b,
                Some(_) => return bad(),
            };
            let a = StartArgs {
                kind: kind.to_string(),
                prompt: prompt.unwrap_or_default(),
                folder,
                worktree,
                node,
            };
            match helpers.start(me, &a) {
                Ok(ticket) => {
                    starts.tickets.insert(ticket);
                    waits.push(SpawnWait::Start {
                        request_id: id,
                        ticket,
                    });
                    Some(None)
                }
                Err(e) => err(e),
            }
        }
        "agent_wait" => {
            let (Some(helper), Some(wanted)) = (s("agent_id"), s("state")) else {
                return bad();
            };
            if !["idle", "working", "blocked", "done", "exited"].contains(&wanted) {
                return bad();
            }
            let timeout = match args.get("timeout_s") {
                None | Some(Value::Null) => WAIT_DEFAULT_MS,
                Some(v) => match v.as_f64() {
                    Some(n) if n.is_finite() && n >= 0.0 => ((n * 1000.0) as u64).min(WAIT_MAX_MS),
                    _ => return bad(),
                },
            };
            let open = waits
                .iter()
                .filter(|w| matches!(w, SpawnWait::State { .. }))
                .count();
            if open >= MAX_WAITS {
                return err(
                    agora_protocol::Error::new(agora_protocol::ErrorCode::MeshTooManyAsks)
                        .with("limit", MAX_WAITS as i64),
                );
            }
            let state = helpers
                .states(me, &[helper.to_string()])
                .pop()
                .unwrap_or_else(|| {
                    Err(agora_protocol::Error::new(
                        agora_protocol::ErrorCode::Internal,
                    ))
                });
            match state {
                Ok(state) if reached(&state, wanted) => Some(Some(tool_reply(
                    &id,
                    agora_mesh::mcp::ok_tool_result(&json!({"agent_id": helper, "state": state})),
                ))),
                Ok(_) => {
                    waits.push(SpawnWait::State {
                        request_id: id,
                        helper: helper.to_string(),
                        wanted: wanted.to_string(),
                        deadline_ms: now_ms().saturating_add(timeout),
                    });
                    Some(None)
                }
                Err(e) => err(e),
            }
        }
        "agent_output" => {
            let Some(helper) = s("agent_id") else {
                return bad();
            };
            let lines = match args.get("lines") {
                None | Some(Value::Null) => 40,
                Some(v) => match v.as_u64() {
                    Some(n) => usize::try_from(n).unwrap_or(usize::MAX).clamp(1, 200),
                    None => return bad(),
                },
            };
            match helpers.output(me, helper, lines) {
                Ok(v) => Some(Some(tool_reply(&id, agora_mesh::mcp::ok_tool_result(&v)))),
                Err(e) => err(e),
            }
        }
        _ => None,
    }
}

/// Answers spawn calls whose start was decided, whose helper reached the state, or whose time ran out.
fn poll_spawn(
    waits: &mut Vec<SpawnWait>,
    starts: &mut ConnectionStarts,
    caller: &AgentId,
    mut reply: impl FnMut(&Message) -> io::Result<()>,
) -> io::Result<()> {
    let Some(helpers) = starts.helpers.clone() else {
        return Ok(());
    };
    // Every waiting helper's state from one view of the node, not one per wait.
    let names: Vec<String> = waits
        .iter()
        .filter_map(|w| match w {
            SpawnWait::State { helper, .. } => Some(helper.clone()),
            SpawnWait::Start { .. } => None,
        })
        .collect();
    let mut states = if names.is_empty() {
        Vec::new()
    } else {
        helpers.states(caller, &names)
    }
    .into_iter();
    let mut still = Vec::new();
    for w in waits.drain(..) {
        match w {
            SpawnWait::Start { request_id, ticket } => match helpers.started(ticket) {
                None => still.push(SpawnWait::Start { request_id, ticket }),
                Some(outcome) => {
                    starts.tickets.remove(&ticket);
                    let result = match outcome {
                        Ok(agent) => agora_mesh::mcp::ok_tool_result(&json!({"agent_id": agent})),
                        Err(e) => agora_mesh::mcp::protocol_error_result(&e),
                    };
                    reply(&tool_reply(&request_id, result))?;
                }
            },
            SpawnWait::State {
                request_id,
                helper,
                wanted,
                deadline_ms,
            } => {
                let state = states.next().unwrap_or_else(|| {
                    Err(agora_protocol::Error::new(
                        agora_protocol::ErrorCode::Internal,
                    ))
                });
                let result = match state {
                    Ok(state) if reached(&state, &wanted) => Some(agora_mesh::mcp::ok_tool_result(
                        &json!({"agent_id": helper, "state": state}),
                    )),
                    Ok(_) if now_ms() >= deadline_ms => {
                        Some(agora_mesh::mcp::protocol_error_result(
                            &agora_protocol::Error::new(agora_protocol::ErrorCode::MeshTimeout),
                        ))
                    }
                    Ok(_) => None,
                    Err(e) => Some(agora_mesh::mcp::protocol_error_result(&e)),
                };
                match result {
                    Some(r) => reply(&tool_reply(&request_id, r))?,
                    None => still.push(SpawnWait::State {
                        request_id,
                        helper,
                        wanted,
                        deadline_ms,
                    }),
                }
            }
        }
    }
    *waits = still;
    Ok(())
}

/// One connection: newline-delimited JSON-RPC in, replies out. Questions that wait for an answer are polled each
/// time the read times out, so a pending `ask` never blocks the agent's other calls.
fn serve(
    stream: LocalStream,
    agent: &AgentId,
    mesh: &Arc<Mutex<Mesh>>,
    links: &Links<'_>,
    stop: &AtomicBool,
) {
    let (remote, allowed, shares) = (links.remote, links.allowed, links.shares);
    let Ok(read_half) = stream.try_clone() else {
        return;
    };
    let _ = read_half.set_read_timeout(Some(POLL));
    let mut reader = BufReader::new(read_half);
    let mut writer = stream;
    let mut conn = McpConnection::new(agent.clone());
    if links.helpers.is_some() {
        conn.enable_spawn();
    }
    let mut spawn_waits: Vec<SpawnWait> = Vec::new();
    let mut starts = ConnectionStarts {
        helpers: links.helpers.clone(),
        tickets: HashSet::new(),
    };
    let mut owned = ConnectionAsks {
        mesh,
        agent,
        ids: HashSet::new(),
    };
    let mut line: Vec<u8> = Vec::new();
    // (request id, ask id, deadline) of asks still waiting for an answer.
    let mut waiting: Vec<WaitingAsk> = Vec::new();
    while !stop.load(Ordering::Acquire) {
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => break,
            Ok(_) if line.ends_with(b"\n") => {
                let text = String::from_utf8_lossy(&line).trim().to_string();
                line.clear();
                if text.is_empty() {
                    continue;
                }
                // ADR-0057: starting and following helpers is the node's, not the broker's.
                if let Some(h) = &links.helpers {
                    if let Some(answer) = spawn_call(&conn, &text, h, &mut spawn_waits, &mut starts)
                    {
                        if let Some(msg) = answer {
                            if send(&mut writer, &msg).is_err() {
                                break;
                            }
                        }
                        continue;
                    }
                }
                // A shared project's tasks, leases and handoffs are decided on the hub's board (ADR-0039).
                if let Some(msg) = board_reply(&conn, &text, mesh, remote, shares) {
                    if send(&mut writer, &msg).is_err() {
                        break;
                    }
                    continue;
                }
                let outcome = {
                    let mut m = mesh.lock().unwrap_or_else(|e| e.into_inner());
                    conn.handle(&text, &mut m, now_ms())
                };
                if let Outcome::Waiting { ask_id, .. } = &outcome {
                    owned.ids.insert(ask_id.clone());
                }
                dispatch_outbound(mesh, remote, allowed);
                match outcome {
                    Outcome::Reply(msg) => {
                        if send(&mut writer, &msg).is_err() {
                            break;
                        }
                    }
                    Outcome::Silent => {}
                    Outcome::Waiting {
                        request_id,
                        ask_id,
                        deadline_ms,
                    } => {
                        waiting.push((request_id, ask_id, deadline_ms));
                    }
                }
            }
            // A partial line (the read timed out mid-line): keep what arrived and wait for the rest.
            Ok(_) => {}
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break,
        }
        if line.len() > MAX_LINE {
            break;
        }
        // Answer asks whose reply arrived or whose time ran out.
        if poll_asks(&mut waiting, &mut owned, &conn, |msg| {
            send(&mut writer, msg)
        })
        .is_err()
        {
            return;
        }
        if poll_spawn(&mut spawn_waits, &mut starts, agent, |msg| {
            send(&mut writer, msg)
        })
        .is_err()
        {
            return;
        }
    }
    let _ = writer.shutdown();
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn ask_fixture() -> (Arc<Mutex<Mesh>>, AgentId, AgentId) {
        let mut mesh = Mesh::new(Limits::default());
        let (a, b) = (AgentId::new("a"), AgentId::new("b"));
        for id in [&a, &b] {
            mesh.register(AgentInfo {
                id: id.clone(),
                kind: "fixture".into(),
                node: "local".into(),
                project: "p".into(),
                label: String::new(),
                worktree: String::new(),
                state: "idle".into(),
            });
        }
        (Arc::new(Mutex::new(mesh)), a, b)
    }

    #[test]
    fn a_disconnected_connection_discards_an_answer_it_never_polled() {
        let (mesh, a, b) = ask_fixture();
        let now = now_ms();
        let (completed, other) = {
            let mut m = mesh.lock().unwrap();
            let completed = m.ask(&a, "b", "finished?", None, now).unwrap();
            let other = m.ask(&a, "b", "other connection", None, now).unwrap();
            m.send_message(&b, "a", "done", &[], Some(&completed), now)
                .unwrap();
            (completed, other)
        };
        {
            let _connection = ConnectionAsks {
                mesh: &mesh,
                agent: &a,
                ids: HashSet::from([completed.clone()]),
            };
        }
        let mut m = mesh.lock().unwrap();
        assert_eq!(m.ask_status(&completed, 0), AskStatus::TimedOut);
        assert_eq!(m.ask_status(&other, 0), AskStatus::Pending);
    }

    #[test]
    fn a_write_error_mid_poll_cancels_both_retained_and_unvisited_asks() {
        let (mesh, a, b) = ask_fixture();
        let now = now_ms();
        let (before, completed, after, other) = {
            let mut m = mesh.lock().unwrap();
            let before = m.ask(&a, "b", "before", None, now).unwrap();
            let completed = m.ask(&a, "b", "completed", None, now).unwrap();
            m.send_message(&b, "a", "done", &[], Some(&completed), now)
                .unwrap();
            let after = m.ask(&a, "b", "after", None, now).unwrap();
            let other = m.ask(&a, "b", "other connection", None, now).unwrap();
            (before, completed, after, other)
        };
        {
            let mut owned = ConnectionAsks {
                mesh: &mesh,
                agent: &a,
                ids: HashSet::from([before.clone(), completed.clone(), after.clone()]),
            };
            let mut waiting = vec![
                (json!(1), before.clone(), u64::MAX),
                (json!(2), completed.clone(), u64::MAX),
                (json!(3), after.clone(), u64::MAX),
            ];
            let conn = McpConnection::new(a.clone());
            let mut writes = 0;
            let error = poll_asks(&mut waiting, &mut owned, &conn, |_| {
                writes += 1;
                Err(io::ErrorKind::BrokenPipe.into())
            })
            .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
            assert_eq!(writes, 1);
        }
        let mut m = mesh.lock().unwrap();
        for id in [&before, &completed, &after] {
            assert_eq!(m.ask_status(id, 0), AskStatus::TimedOut);
        }
        // A second connection of the same agent still owns its request and can consume the reply normally.
        assert_eq!(m.ask_status(&other, 0), AskStatus::Pending);
        m.send_message(&b, "a", "kept", &[], Some(&other), now)
            .unwrap();
        assert_eq!(
            m.ask_status(&other, 0),
            AskStatus::Answered {
                reply: "kept".into(),
                from: b
            }
        );
    }

    #[test]
    fn a_late_not_shared_answer_does_not_undo_a_join() {
        let shares = Shares::default();
        let asked = Instant::now();
        shares.set("p", true); // a BoardJoin, after the question was asked
        shares.set_not_shared_since("p", asked);
        assert!(matches!(shares.get("p"), Some((true, _))));
        // An answer to a question asked after the join counts.
        let later = Instant::now();
        shares.set_not_shared_since("p", later);
        assert!(matches!(shares.get("p"), Some((false, _))));
    }

    #[test]
    fn board_join_respects_mesh_remote_and_hands_over_only_unfinished_work() {
        let dir = std::env::temp_dir().join(format!("agora-bj-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let server = MeshServer::new(&dir, Limits::default()).unwrap();
        {
            let mesh = server.mesh();
            let mut m = mesh.lock().unwrap();
            m.register(AgentInfo {
                id: AgentId::new("agent-1"),
                kind: "shell".into(),
                node: "n".into(),
                project: "p".into(),
                label: "agent-1".into(),
                worktree: "/w".into(),
                state: "idle".into(),
            });
            m.task_create(&AgentId::new("agent-1"), "open", "", &[])
                .unwrap();
            m.lease_acquire(
                &AgentId::new("agent-1"),
                &["README.md".into()],
                600_000,
                now_ms(),
            )
            .unwrap();
        }
        server.set_remote_allowed(false);
        let e = server.board_join("p").unwrap_err();
        assert_eq!(e.code, agora_protocol::ErrorCode::NodePolicyDenied);
        // Refused: nothing was taken and the project is not marked shared.
        assert!(server.shares.get("p").is_none());
        server.set_remote_allowed(true);
        let work = server.board_join("p").unwrap();
        assert_eq!(work.tasks.len(), 1);
        assert_eq!(work.tasks[0].creator_agent_id, "agent-1");
        assert_eq!(work.leases.len(), 1);
        assert!(matches!(server.shares.get("p"), Some((true, _))));
        // Handed over once: a second ask finds nothing left.
        let again = server.board_join("p").unwrap();
        assert!(again.tasks.is_empty() && again.leases.is_empty());
        drop(server);
        let _ = std::fs::remove_dir_all(dir);
    }
}
