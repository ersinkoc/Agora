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

use agora_adapters::host::AdapterEvent;
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
/// How often each agent's workspace is checked for uncommitted changes.
const GIT_POLL: Duration = Duration::from_secs(5);
/// Default size cap of the on-disk event buffer.
pub const EVENT_BUFFER_BYTES: u64 = 64 * 1024 * 1024;

/// Writes protocol events to the durable buffer.
pub struct EventSink {
    producer: String,
    log: Mutex<DiskEventLog>,
    /// Says again what the hub must know about this node (online, its checkouts), see [`EventSink::announce`].
    announcer: Mutex<Option<Box<dyn Fn(&EventSink) + Send>>>,
}

pub(crate) fn now_ms() -> u64 {
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
            announcer: Mutex::new(None),
        })
    }

    /// What this node says about itself when the hub turns out to be ahead of it (see [`announce`](Self::announce)).
    pub fn set_announcer(&self, f: Box<dyn Fn(&EventSink) + Send>) {
        *self.announcer.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
    }

    /// The hub has acknowledged numbers this buffer never produced (this machine came back under a known name with an empty data
    /// directory): skip our numbering forward to `next` and say again who we are, since what we recorded before the skip is gone.
    pub fn catch_up_with_hub(&self, next: u64) -> io::Result<()> {
        self.log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .advance_to(next)?;
        let announcer = self.announcer.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(f) = &*announcer {
            f(self);
        }
        Ok(())
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
                SupervisorEvent::Adapter { agent, event } => match event {
                    AdapterEvent::Usage {
                        model,
                        input_tokens,
                        output_tokens,
                        ..
                    } => self.record(Body::AgentUsageReported(v1::AgentUsageReported {
                        agent_id: agent.as_str().to_string(),
                        model: model.clone(),
                        input_tokens: *input_tokens,
                        output_tokens: *output_tokens,
                    })),
                    AdapterEvent::SessionReported {
                        native_session_id, ..
                    } => self.record(Body::AgentSessionReported(v1::AgentSessionReported {
                        agent_id: agent.as_str().to_string(),
                        session_id: native_session_id.clone(),
                    })),
                    // Approval requests go through the node's approvals queue (see `Node`), chat and log lines
                    // are not part of the event log.
                    _ => {}
                },
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

    pub fn approval_requested(&self, p: &crate::approvals::Pending) {
        self.record(Body::ApprovalRequested(v1::ApprovalRequested {
            approval_id: p.id.clone(),
            agent_id: p.agent_id.clone(),
            action: p.action.clone(),
            detail: p.detail.clone(),
            expires_at_ms: p.expires_ms,
        }));
    }

    pub fn approval_resolved(&self, approval_id: &str, approved: bool, signer_device_id: &str) {
        self.record(Body::ApprovalResolved(v1::ApprovalResolved {
            approval_id: approval_id.to_string(),
            approved,
            signer_device_id: signer_device_id.to_string(),
        }));
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

    pub fn git_status_changed(&self, worktree_id: &str, st: &git::GitStatus) {
        self.record(Body::GitStatusChanged(v1::GitStatusChanged {
            worktree_id: worktree_id.to_string(),
            files_changed: st.files_changed,
            insertions: st.insertions,
            deletions: st.deletions,
            head_commit: st.head_commit.clone(),
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
        has_terminal: bool,
    ) {
        let worktree_id = worktree.to_string_lossy().into_owned();
        self.record(Body::WorktreeCreated(v1::WorktreeCreated {
            worktree_id: worktree_id.clone(),
            checkout_id: checkout_id(checkout),
            path: worktree_id.clone(),
            branch: branch.to_string(),
        }));
        self.agent_started(agent, project, node, &worktree_id, kind);
        if !has_terminal {
            return; // an adapter agent has no terminal
        }
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

/// Name of the node's policy file in its data directory (written by the machine's owner; the hub cannot change it).
pub const POLICY_FILE: &str = "policy";

/// What the owner allows on this node. A missing file allows the defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    /// Messages may cross to and from agents on other nodes (`mesh.remote`, default true; ADR-0031).
    pub mesh_remote: bool,
}

impl Default for Policy {
    fn default() -> Self {
        Self { mesh_remote: true }
    }
}

/// Reads `key = value` lines (blank lines and `#` comments skipped). A known key with a value that is not `true` or `false`
/// counts as `false`: a typo must never widen what the node allows. Unknown keys are ignored.
pub fn read_policy(path: &std::path::Path) -> Policy {
    let mut p = Policy::default();
    let Ok(text) = std::fs::read_to_string(path) else {
        return p;
    };
    for line in text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        if let Some((k, v)) = line.split_once('=') {
            if k.trim() == "mesh.remote" {
                p.mesh_remote = v.trim() == "true";
            }
        }
    }
    p
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
    git_watch: Option<JoinHandle<()>>,
    pub supervisor: Arc<Mutex<Supervisor>>,
    pub events: Arc<EventSink>,
    /// Per-agent mesh endpoints (messages, tasks, leases between the agents on this node).
    pub mesh: Arc<crate::mesh_server::MeshServer>,
    /// Sensitive actions waiting for a user's signed decision (ADR-0020).
    pub approvals: Arc<crate::approvals::NodeApprovals>,
    node_id: String,
    /// Manifests that could not be loaded, as `(path, reason)` for the log.
    pub manifest_problems: Vec<(PathBuf, String)>,
}

impl Node {
    pub fn start(cfg: &NodeConfig) -> Result<Self, NodeError> {
        std::fs::create_dir_all(&cfg.data_dir)?;
        std::fs::create_dir_all(&cfg.worktree_root)?;
        let events = Arc::new(EventSink::open(&cfg.data_dir.join("events"), &cfg.node_id)?);

        // Short path on purpose: Unix socket paths are limited to about 100 bytes.
        let mesh_dir = cfg
            .socket
            .parent()
            .map_or_else(|| PathBuf::from("m"), |p| p.join("m"));
        let mesh = Arc::new(crate::mesh_server::MeshServer::new(
            &mesh_dir,
            agora_mesh::broker::Limits::default(),
        )?);
        mesh.set_remote_allowed(read_policy(&cfg.data_dir.join(POLICY_FILE)).mesh_remote);

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
        {
            let node = cfg.node_id.clone();
            let known: Vec<(String, PathBuf)> = projects
                .iter()
                .map(|p| (p.id.clone(), p.root.clone()))
                .collect();
            events.set_announcer(Box::new(move |ev| {
                ev.node_online(&node);
                for (id, root) in &known {
                    ev.checkout_discovered(&node, id, root);
                }
            }));
        }

        // Every agent that launches joins the mesh and, if its manifest has `[mcp]`, is handed its own endpoint.
        {
            let for_abort = Arc::clone(&mesh);
            let mesh = Arc::clone(&mesh);
            sup.set_launch_hook(Box::new(move |l| {
                let socket = mesh
                    .register(agora_mesh::broker::AgentInfo {
                        id: l.id.clone(),
                        kind: l.kind.clone(),
                        node: l.node.clone(),
                        project: l.project.clone(),
                        label: l.id.as_str().to_string(),
                        worktree: l.worktree.to_string_lossy().into_owned(),
                        state: "idle".into(),
                    })
                    .ok()?;
                let command = std::env::current_exe().ok()?;
                Some(agora_adapters::inject::Endpoint { command, socket })
            }));
            sup.set_abort_hook(Box::new(move |id| for_abort.unregister(id)));
        }

        // Only devices the owner listed in `<data>/trusted-devices` (`agora node trust`) can answer an approval; with
        // none listed every request waits and expires as denied.
        let (verifier, _skipped) = crate::approvals::P256Verifier::load(
            &cfg.data_dir.join(crate::approvals::TRUSTED_DEVICES_FILE),
        );
        let approvals = Arc::new(crate::approvals::NodeApprovals::new(
            &cfg.node_id,
            Box::new(verifier),
        ));
        approvals.follow_trust_file(&cfg.data_dir.join(crate::approvals::TRUSTED_DEVICES_FILE));

        let supervisor = Arc::new(Mutex::new(sup));
        let chat = Arc::new(crate::chat::ChatBoard::new());
        let ctx = Arc::new(ControlContext::new(
            Arc::clone(&supervisor),
            projects,
            cfg.worktree_root.clone(),
            cfg.node_id.clone(),
            Some(Arc::clone(&events)),
            Some(Arc::clone(&mesh)),
            Some(Arc::clone(&approvals)),
            Arc::clone(&chat),
        ));
        let server = ControlServer::start(&cfg.socket, ctx)?;

        let stop = Arc::new(AtomicBool::new(false));
        let ticker = {
            let (sup, ev, stop, node_id, mesh, approvals, chat) = (
                Arc::clone(&supervisor),
                Arc::clone(&events),
                Arc::clone(&stop),
                cfg.node_id.clone(),
                Arc::clone(&mesh),
                Arc::clone(&approvals),
                Arc::clone(&chat),
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
                        // An agent that will be restarted keeps its endpoint (the config it was launched with still
                        // points at it); one that finished or was given up on leaves the mesh.
                        for e in &events {
                            match e {
                                SupervisorEvent::Exited { agent, status } if status.success() => {
                                    mesh.unregister(agent);
                                }
                                SupervisorEvent::GaveUp { agent } => mesh.unregister(agent),
                                _ => {}
                            }
                        }
                        for v in views.iter().filter(|v| v.running) {
                            mesh.set_state(&v.id, state_word(v.state));
                        }
                        ev.record_bodies(mesh.take_events());
                        // Adapter approval requests enter the queue and are announced; stopped or dead agents leave
                        // nothing waiting; requests that ran out of time are answered as denied.
                        let now = now_ms();
                        for e in &events {
                            match e {
                                // What a chat agent says goes to its transcript (ADR-0038).
                                SupervisorEvent::Adapter {
                                    agent,
                                    event: AdapterEvent::Chat { item, .. },
                                } => chat.push_item(agent, item, now),
                                SupervisorEvent::Adapter {
                                    agent,
                                    event:
                                        AdapterEvent::ApprovalRequest {
                                            approval_id,
                                            action,
                                            detail,
                                            ..
                                        },
                                } => {
                                    if let Some(p) =
                                        approvals.request(agent, approval_id, action, detail, now)
                                    {
                                        ev.approval_requested(&p);
                                    } else {
                                        // No safe id: refuse rather than let the action through.
                                        let _ = sup
                                            .lock()
                                            .unwrap_or_else(|e| e.into_inner())
                                            .adapter_resolve_approval(agent, approval_id, false);
                                    }
                                }
                                SupervisorEvent::Stopped { agent }
                                | SupervisorEvent::AdapterCrashed { agent } => {
                                    approvals.cancel_agent(agent);
                                }
                                _ => {}
                            }
                        }
                        for (node_approval, agent, adapter_id) in approvals.expire(now) {
                            let _ = sup
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .adapter_resolve_approval(&agent, &adapter_id, false);
                            ev.approval_resolved(&node_approval, false, "");
                        }
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
        let git_watch = {
            let (sup, ev, stop) = (
                Arc::clone(&supervisor),
                Arc::clone(&events),
                Arc::clone(&stop),
            );
            std::thread::Builder::new()
                .name("agora-git".into())
                .spawn(move || watch_git(&sup, &ev, &stop))?
        };
        Ok(Self {
            server: Some(server),
            stop,
            ticker: Some(ticker),
            git_watch: Some(git_watch),
            supervisor,
            events,
            mesh,
            approvals,
            node_id: cfg.node_id.clone(),
            manifest_problems,
        })
    }

    pub fn socket(&self) -> Option<&Path> {
        self.server.as_ref().map(ControlServer::path)
    }
}

/// Reports each agent workspace's uncommitted changes whenever they change (`GitStatusChanged`), checked every
/// [`GIT_POLL`] on its own thread so slow git calls never delay the supervisor's ticks. The first check of a
/// workspace is reported too, so a client that connects later sees the current state from the event log.
fn watch_git(sup: &Mutex<Supervisor>, ev: &EventSink, stop: &AtomicBool) {
    let mut last: std::collections::HashMap<PathBuf, (u32, u32, u32, String)> =
        std::collections::HashMap::new();
    while !stop.load(Ordering::Acquire) {
        let views = sup.lock().unwrap_or_else(|e| e.into_inner()).list();
        last.retain(|path, _| views.iter().any(|v| &v.worktree == path));
        for v in views.iter().filter(|v| v.running) {
            if stop.load(Ordering::Acquire) {
                return;
            }
            let Ok(st) = git::status(&v.worktree) else {
                continue;
            };
            let now = (
                st.files_changed,
                st.insertions,
                st.deletions,
                st.head_commit.clone(),
            );
            if last.get(&v.worktree) != Some(&now) {
                last.insert(v.worktree.clone(), now);
                ev.git_status_changed(&v.worktree.to_string_lossy(), &st);
            }
        }
        let until = Instant::now() + GIT_POLL;
        while Instant::now() < until && !stop.load(Ordering::Acquire) {
            std::thread::sleep(TICK);
        }
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        drop(self.server.take());
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.ticker.take() {
            let _ = t.join();
        }
        if let Some(t) = self.git_watch.take() {
            let _ = t.join();
        }
        self.events.node_offline(&self.node_id);
        // Agents die with the node's supervisor (terminal handles kill their process groups on drop).
    }
}
