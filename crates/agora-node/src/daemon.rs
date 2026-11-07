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
        Ok(Self {
            producer: producer.to_string(),
            log: Mutex::new(DiskEventLog::open(dir, EVENT_BUFFER_BYTES)?),
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
                }
                _ => {}
            }
        }
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

        let supervisor = Arc::new(Mutex::new(sup));
        let ctx = Arc::new(ControlContext::new(
            Arc::clone(&supervisor),
            projects,
            cfg.worktree_root.clone(),
            cfg.node_id.clone(),
            Some(Arc::clone(&events)),
        ));
        let server = ControlServer::start(&cfg.socket, ctx)?;

        let stop = Arc::new(AtomicBool::new(false));
        let ticker = {
            let (sup, ev, stop) = (
                Arc::clone(&supervisor),
                Arc::clone(&events),
                Arc::clone(&stop),
            );
            std::thread::Builder::new()
                .name("agora-supervise".into())
                .spawn(move || {
                    while !stop.load(Ordering::Acquire) {
                        let (events, views) = {
                            let mut s = sup.lock().unwrap_or_else(|e| e.into_inner());
                            (s.tick(Instant::now()), s.list())
                        };
                        if !events.is_empty() {
                            ev.supervisor_events(&events, &views);
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
        // Agents die with the node's supervisor (terminal handles kill their process groups on drop).
    }
}
