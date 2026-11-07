//! The node daemon: supervisor + control socket + event buffer, running without any hub.
//!
//! Events go to the durable buffer (`event_buffer`) as protocol `Event`s; with no hub connected they are
//! simply retained (size-capped) until one acknowledges them. Agents never depend on any of this succeeding.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use agora_adapters::Manifest;
use agora_core::{AgentId, AgentState, NodeId};
use agora_protocol::v1::{self, event::Body};
use prost::Message;

use crate::control::{ControlContext, ControlServer, ProjectEntry};
use crate::event_buffer::DiskEventLog;
use crate::git;
use crate::supervisor::{
    AgentView, NodeLimits, RestartPolicy, Supervisor, SupervisorConfig, SupervisorEvent,
};
use crate::terminal::TerminalOptions;

const TICK: Duration = Duration::from_millis(100);
/// Default size cap of the on-disk event buffer.
pub const EVENT_BUFFER_BYTES: u64 = 64 * 1024 * 1024;

/// Writes protocol events to the durable buffer.
pub struct EventSink {
    producer: String,
    log: Mutex<DiskEventLog>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Stable id of a checkout on this node: a hash of its path (the same path always gives the same id).
pub fn checkout_id(root: &Path) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in root.to_string_lossy().bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("co-{h:016x}")
}

fn state_word(s: AgentState) -> &'static str {
    match s {
        AgentState::Working => "working",
        AgentState::Blocked => "blocked",
        AgentState::Idle => "idle",
        AgentState::Done => "done",
        AgentState::Unknown => "unknown",
    }
}

fn proto_state(s: AgentState) -> v1::AgentState {
    match s {
        AgentState::Working => v1::AgentState::Working,
        AgentState::Blocked => v1::AgentState::Blocked,
        AgentState::Idle => v1::AgentState::Idle,
        AgentState::Done => v1::AgentState::Done,
        AgentState::Unknown => v1::AgentState::Unspecified,
    }
}

impl EventSink {
    pub fn open(dir: &Path, producer: &str) -> io::Result<Self> {
        Self::open_with_cap(dir, producer, EVENT_BUFFER_BYTES)
    }

    /// Like [`open`](Self::open) with an explicit size cap for the buffer, in bytes.
    pub fn open_with_cap(dir: &Path, producer: &str, max_bytes: u64) -> io::Result<Self> {
        Ok(Self {
            producer: producer.to_string(),
            log: Mutex::new(DiskEventLog::open(dir, max_bytes)?),
        })
    }

    fn record(&self, body: Body) {
        let mut log = self.log.lock().unwrap_or_else(|e| e.into_inner());
        let ev = v1::Event {
            producer_id: self.producer.clone(),
            seq: log.next_seq(),
            at_ms: now_ms(),
            body: Some(body),
        };
        // A full disk must never stop agents: losing an event is better than losing the node.
        let _ = log.append(&ev.encode_to_vec());
    }

    pub fn agent_started(
        &self,
        agent: &AgentId,
        project: &str,
        node: &str,
        worktree: &str,
        kind: &str,
    ) {
        self.record(Body::AgentStarted(v1::AgentStarted {
            agent_id: agent.as_str().to_string(),
            project_id: project.to_string(),
            node_id: node.to_string(),
            worktree_id: worktree.to_string(),
            kind: kind.to_string(),
            level: v1::IntegrationLevel::L1Manifest as i32,
        }));
    }

    /// Map supervisor events to protocol events. `views` is only used to find nothing today but keeps the
    /// call site honest about needing current state when richer events are added.
    pub fn supervisor_events(&self, events: &[SupervisorEvent], _views: &[AgentView]) {
        for e in events {
            match e {
                SupervisorEvent::StateChanged { agent, state } => {
                    self.record(Body::AgentStateChanged(v1::AgentStateChanged {
                        agent_id: agent.as_str().to_string(),
                        state: proto_state(*state) as i32,
                        reported: false,
                    }));
                }
                SupervisorEvent::Exited { agent, status } => {
                    self.record(Body::AgentExited(v1::AgentExited {
                        agent_id: agent.as_str().to_string(),
                        exit_code: status
                            .code
                            .or_else(|| status.signal.map(|s| -s))
                            .unwrap_or(-1),
                    }));
                    self.record(Body::TerminalClosed(v1::TerminalClosed {
                        terminal_id: crate::control::terminal_id_of(agent),
                    }));
                }
                SupervisorEvent::Stopped { agent } => {
                    // Stopped on request: the exit code is not known (same convention as the terminal exit frame).
                    self.record(Body::AgentExited(v1::AgentExited {
                        agent_id: agent.as_str().to_string(),
                        exit_code: i32::MIN,
                    }));
                    self.record(Body::TerminalClosed(v1::TerminalClosed {
                        terminal_id: crate::control::terminal_id_of(agent),
                    }));
                }
                _ => {}
            }
        }
    }

    /// Appends already-built events (mesh activity for the audit log).
    pub fn record_bodies(&self, bodies: Vec<Body>) {
        for b in bodies {
            self.record(b);
        }
    }

    /// The node announces itself; the hub marks it online.
    pub fn node_online(&self, node: &str) {
        let host = std::env::var("HOSTNAME")
            .or_else(|_| std::env::var("COMPUTERNAME"))
            .unwrap_or_else(|_| node.to_string());
        self.record(Body::NodeOnline(v1::NodeOnline {
            node_id: node.to_string(),
            hostname: host,
            os: std::env::consts::OS.to_string(),
            build: env!("CARGO_PKG_VERSION").to_string(),
        }));
    }

    /// Recorded on a clean shutdown. A crash records nothing; the hub notices the dropped connection instead.
    pub fn node_offline(&self, node: &str) {
        self.record(Body::NodeOffline(v1::NodeOffline {
            node_id: node.to_string(),
        }));
    }

    pub fn node_resources(&self, node: &str, agents_running: u32, terminals_open: u32) {
        self.record(Body::NodeResources(v1::NodeResources {
            node_id: node.to_string(),
            agents_running,
            terminals_open,
            memory_used_bytes: 0,
        }));
    }

    pub fn checkout_discovered(&self, node: &str, project: &str, root: &Path) {
        self.record(Body::CheckoutDiscovered(v1::CheckoutDiscovered {
            checkout_id: checkout_id(root),
            node_id: node.to_string(),
            project_id: project.to_string(),
            path: root.to_string_lossy().into_owned(),
        }));
    }

    /// An agent is up: its workspace, the agent, and its terminal, in that order so a reader that builds a
    /// view from the events always sees the workspace before the agent that uses it.
    #[allow(clippy::too_many_arguments)]
    pub fn agent_launched(
        &self,
        agent: &AgentId,
        project: &str,
        node: &str,
        checkout: &Path,
        worktree: &Path,
        branch: &str,
        kind: &str,
    ) {
        let worktree_id = worktree.to_string_lossy().into_owned();
        self.record(Body::WorktreeCreated(v1::WorktreeCreated {
            worktree_id: worktree_id.clone(),
            checkout_id: checkout_id(checkout),
            path: worktree_id.clone(),
            branch: branch.to_string(),
        }));
        self.agent_started(agent, project, node, &worktree_id, kind);
        self.record(Body::TerminalOpened(v1::TerminalOpened {
            terminal_id: crate::control::terminal_id_of(agent),
            node_id: node.to_string(),
            purpose: v1::TerminalPurpose::Agent as i32,
            agent_id: agent.as_str().to_string(),
        }));
    }

    /// Highest sequence number the hub has acknowledged (durable).
    pub fn acked(&self) -> u64 {
        self.log.lock().unwrap_or_else(|e| e.into_inner()).acked()
    }

    /// The hub acknowledged everything up to `seq`: the buffer may drop it.
    pub fn ack(&self, seq: u64) -> io::Result<()> {
        self.log.lock().unwrap_or_else(|e| e.into_inner()).ack(seq)
    }

    /// Lowest sequence number still retained.
    pub fn oldest_retained(&self) -> u64 {
        self.log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .oldest_retained()
    }

    /// Encoded events after `after`, oldest first, for the uplink.
    pub fn payloads_after(
        &self,
        after: u64,
    ) -> Result<Vec<Vec<u8>>, crate::event_buffer::ReplayError> {
        self.log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .replay_after(after)
    }

    /// Decoded events after `after` (exclusive), oldest first; what the uplink sends and tests inspect.
    pub fn events_after(&self, after: u64) -> Vec<v1::Event> {
        let log = self.log.lock().unwrap_or_else(|e| e.into_inner());
        log.replay_after(after)
            .unwrap_or_default()
            .iter()
            .filter_map(|b| v1::Event::decode(b.as_slice()).ok())
            .collect()
    }

    pub fn next_seq(&self) -> u64 {
        self.log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .next_seq()
    }
}

/// Where a node keeps things and what it serves.
#[derive(Debug, Clone)]
pub struct NodeConfig {
    pub node_id: String,
    pub socket: PathBuf,
    /// Event buffer and other node state.
    pub data_dir: PathBuf,
    /// Parent directory of agent worktrees.
    pub worktree_root: PathBuf,
    /// Directory of `*.toml` agent manifests.
    pub manifests_dir: PathBuf,
    /// Git checkouts to register as projects.
    pub projects: Vec<PathBuf>,
    pub limits: NodeLimits,
}

#[derive(Debug)]
pub enum NodeError {
    Io(io::Error),
    /// A project path is not inside a git checkout.
    Project(PathBuf),
}

impl std::fmt::Display for NodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Project(p) => write!(f, "not a git checkout: {}", p.display()),
        }
    }
}

impl std::error::Error for NodeError {}

impl From<io::Error> for NodeError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// A running node. Dropping it stops the control socket and the supervision loop (agents are killed).
pub struct Node {
    // Field order matters for drop: stop serving before the supervisor goes away.
    server: Option<ControlServer>,
    stop: Arc<AtomicBool>,
    ticker: Option<JoinHandle<()>>,
    pub supervisor: Arc<Mutex<Supervisor>>,
    pub events: Arc<EventSink>,
    /// Per-agent mesh endpoints (messages, tasks, leases between the agents on this node).
    pub mesh: Arc<crate::mesh_server::MeshServer>,
    node_id: String,
    /// Manifests that could not be loaded, as `(path, reason)` for the log.
    pub manifest_problems: Vec<(PathBuf, String)>,
}

impl Node {
    pub fn start(cfg: &NodeConfig) -> Result<Self, NodeError> {
        std::fs::create_dir_all(&cfg.data_dir)?;
        std::fs::create_dir_all(&cfg.worktree_root)?;
        let events = Arc::new(EventSink::open(&cfg.data_dir.join("events"), &cfg.node_id)?);

        let mut sup = Supervisor::new(SupervisorConfig {
            node: NodeId::new(cfg.node_id.clone()),
            restart: RestartPolicy::default(),
            terminal: TerminalOptions::default(),
            limits: cfg.limits,
        });
        let mut manifest_problems = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&cfg.manifests_dir) {
            let mut paths: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "toml"))
                .collect();
            paths.sort();
            for p in paths {
                match std::fs::read_to_string(&p)
                    .map_err(|e| e.to_string())
                    .and_then(|s| Manifest::parse(&s).map_err(|e| e.to_string()))
                {
                    Ok(m) => sup.register(m),
                    Err(e) => manifest_problems.push((p, e)),
                }
            }
        }

        let mut projects = Vec::new();
        for p in &cfg.projects {
            let info = git::discover_checkout(p).map_err(|_| NodeError::Project(p.clone()))?;
            let name = info.root.file_name().map_or_else(
                || "project".to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            projects.push(ProjectEntry {
                id: info.project.to_string(),
                name,
                root: info.root,
            });
        }

        events.node_online(&cfg.node_id);
        for p in &projects {
            events.checkout_discovered(&cfg.node_id, &p.id, &p.root);
        }

        // Short path on purpose: Unix socket paths are limited to about 100 bytes.
        let mesh_dir = cfg
            .socket
            .parent()
            .map_or_else(|| PathBuf::from("m"), |p| p.join("m"));
        let mesh = Arc::new(crate::mesh_server::MeshServer::new(
            &mesh_dir,
            agora_mesh::broker::Limits::default(),
        )?);

        let supervisor = Arc::new(Mutex::new(sup));
        let ctx = Arc::new(ControlContext::new(
            Arc::clone(&supervisor),
            projects,
            cfg.worktree_root.clone(),
            cfg.node_id.clone(),
            Some(Arc::clone(&events)),
            Some(Arc::clone(&mesh)),
        ));
        let server = ControlServer::start(&cfg.socket, ctx)?;

        let stop = Arc::new(AtomicBool::new(false));
        let ticker = {
            let (sup, ev, stop, node_id, mesh) = (
                Arc::clone(&supervisor),
                Arc::clone(&events),
                Arc::clone(&stop),
                cfg.node_id.clone(),
                Arc::clone(&mesh),
            );
            std::thread::Builder::new()
                .name("agora-supervise".into())
                .spawn(move || {
                    let mut reported = (u32::MAX, u32::MAX);
                    while !stop.load(Ordering::Acquire) {
                        let (events, views) = {
                            let mut s = sup.lock().unwrap_or_else(|e| e.into_inner());
                            (s.tick(Instant::now()), s.list())
                        };
                        if !events.is_empty() {
                            ev.supervisor_events(&events, &views);
                        }
                        // Agents that ended on their own leave the mesh; the others keep their state current.
                        for e in &events {
                            if let SupervisorEvent::Exited { agent, .. } = e {
                                mesh.unregister(agent);
                            }
                        }
                        for v in views.iter().filter(|v| v.running) {
                            mesh.set_state(&v.id, state_word(v.state));
                        }
                        ev.record_bodies(mesh.take_events());
                        // Resources are reported when they change, not on a timer, so an idle node is silent.
                        let running = u32::try_from(views.iter().filter(|v| v.running).count())
                            .unwrap_or(u32::MAX);
                        if (running, running) != reported {
                            reported = (running, running);
                            ev.node_resources(&node_id, running, running);
                        }
                        std::thread::sleep(TICK);
                    }
                })?
        };
        Ok(Self {
            server: Some(server),
            stop,
            ticker: Some(ticker),
            supervisor,
            events,
            mesh,
            node_id: cfg.node_id.clone(),
            manifest_problems,
        })
    }

    pub fn socket(&self) -> Option<&Path> {
        self.server.as_ref().map(ControlServer::path)
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        drop(self.server.take());
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.ticker.take() {
            let _ = t.join();
        }
        self.events.node_offline(&self.node_id);
        // Agents die with the node's supervisor (terminal handles kill their process groups on drop).
    }
}
