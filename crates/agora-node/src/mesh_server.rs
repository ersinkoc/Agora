//! The node's mesh server: one MCP endpoint per local agent (ADR-0008), backed by a node-local broker. Agents on the
//! same machine coordinate with no hub (nodes are autonomous); a hub-side broker for agents on different machines
//! comes later, behind the same tools.
//!
//! Each agent gets its own local socket (owner-only). Whatever connects to it *is* that agent as far as the broker
//! is concerned, so the socket is only ever handed to that agent's process (through its MCP configuration); an agent
//! cannot reach another's socket by guessing a name in a tool call, because no call names a sender.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agora_core::AgentId;
use agora_mesh::broker::{AgentInfo, AskStatus, Limits, Mesh};
use agora_mesh::mcp::{McpConnection, Outcome};
use agora_platform::{local_listen, LocalStream};
use agora_protocol::jsonrpc::{to_line, Message};
use agora_protocol::v1::event::Body;
use serde_json::Value;

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
        done: Box<dyn FnOnce(Result<(), String>) + Send>,
    );
}

type RemoteSlot = Mutex<Option<Arc<dyn RemoteLink>>>;

/// Sends what the broker queued for other nodes. Called after every tool call, outside the broker's lock; a failure
/// comes back to the sending agent as a notice in its inbox.
fn dispatch_outbound(mesh: &Arc<Mutex<Mesh>>, remote: &RemoteSlot) {
    let out = mesh
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take_outbound();
    if out.is_empty() {
        return;
    }
    let link = remote.lock().unwrap_or_else(|e| e.into_inner()).clone();
    for o in out {
        let to = format!("{}/{}", o.to_node, o.to_agent);
        let failed = {
            let (mesh, from, to) = (Arc::clone(mesh), o.from.clone(), to.clone());
            move |code: String| {
                mesh.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .delivery_failed(&from, &to, &code, now_ms());
            }
        };
        match &link {
            Some(l) => {
                let failed = failed.clone();
                l.post(
                    o.from.as_str(),
                    &to,
                    &o.body,
                    Box::new(move |r| {
                        if let Err(code) = r {
                            failed(code);
                        }
                    }),
                );
            }
            None => failed("node.offline".to_string()),
        }
    }
}

struct AgentServer {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    path: PathBuf,
}

pub struct MeshServer {
    remote: Arc<RemoteSlot>,
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
            remote: Arc::new(Mutex::new(None)),
            mesh,
            dir: dir.to_path_buf(),
            agents: Mutex::new(HashMap::new()),
            stop,
            ticker: Some(ticker),
        })
    }

    /// Where messages for agents on other nodes go (the hub uplink). Without one they fail with `node.offline`.
    pub fn set_remote(&self, link: Arc<dyn RemoteLink>) {
        *self.remote.lock().unwrap_or_else(|e| e.into_inner()) = Some(link);
    }

    /// A message from an agent on another node, as the hub vouched for it (ADR-0031).
    pub fn deliver_remote(
        &self,
        origin: &str,
        from_agent: &str,
        to: &AgentId,
        body: &str,
    ) -> Result<(), agora_protocol::Error> {
        let mut m = self.mesh.lock().unwrap_or_else(|e| e.into_inner());
        let limits = m.limits();
        m.deliver_remote(origin, from_agent, to, body, now_ms())
            .map(|_| ())
            .map_err(|e| e.to_error(&limits))
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
        let id = info.id.clone();
        let path = self.socket_path(&id);
        let listener = local_listen(&path)?;
        listener.set_nonblocking(true)?;
        self.mesh
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .register(info);
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (mesh, stop, agent, remote) = (
                Arc::clone(&self.mesh),
                Arc::clone(&stop),
                id.clone(),
                Arc::clone(&self.remote),
            );
            std::thread::Builder::new()
                .name("agora-mesh-agent".into())
                .spawn(move || {
                    let mut conns: Vec<JoinHandle<()>> = Vec::new();
                    while !stop.load(Ordering::Acquire) {
                        match listener.accept() {
                            Ok(stream) => {
                                let (mesh, stop, agent, remote) = (
                                    Arc::clone(&mesh),
                                    Arc::clone(&stop),
                                    agent.clone(),
                                    Arc::clone(&remote),
                                );
                                if let Ok(h) = std::thread::Builder::new()
                                    .name("agora-mesh-conn".into())
                                    .spawn(move || serve(stream, &agent, &mesh, &remote, &stop))
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

/// One connection: newline-delimited JSON-RPC in, replies out. Questions that wait for an answer are polled each
/// time the read times out, so a pending `ask` never blocks the agent's other calls.
fn serve(
    stream: LocalStream,
    agent: &AgentId,
    mesh: &Arc<Mutex<Mesh>>,
    remote: &RemoteSlot,
    stop: &AtomicBool,
) {
    let Ok(read_half) = stream.try_clone() else {
        return;
    };
    let _ = read_half.set_read_timeout(Some(POLL));
    let mut reader = BufReader::new(read_half);
    let mut writer = stream;
    let mut conn = McpConnection::new(agent.clone());
    let mut line: Vec<u8> = Vec::new();
    // (request id, ask id, deadline) of asks still waiting for an answer.
    let mut waiting: Vec<(Value, String, u64)> = Vec::new();
    while !stop.load(Ordering::Acquire) {
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => break,
            Ok(_) if line.ends_with(b"\n") => {
                let text = String::from_utf8_lossy(&line).trim().to_string();
                line.clear();
                if text.is_empty() {
                    continue;
                }
                let outcome = {
                    let mut m = mesh.lock().unwrap_or_else(|e| e.into_inner());
                    conn.handle(&text, &mut m, now_ms())
                };
                dispatch_outbound(mesh, remote);
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
        let mut still = Vec::new();
        for (rid, ask_id, deadline) in waiting.drain(..) {
            let (status, limits) = {
                let mut m = mesh.lock().unwrap_or_else(|e| e.into_inner());
                (m.ask_status(&ask_id, now_ms()), m.limits())
            };
            if status == AskStatus::Pending {
                still.push((rid, ask_id, deadline));
            } else if send(&mut writer, &conn.finish_ask(&rid, &status, &limits)).is_err() {
                return;
            }
        }
        waiting = still;
    }
    let _ = writer.shutdown();
}
