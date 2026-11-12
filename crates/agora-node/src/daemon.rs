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

/// What a node says about itself again when the hub's record of it cannot be continued (see [`EventSink::announce`]).
type Announcer = Box<dyn Fn(&EventSink) + Send>;

/// Facts the node reported that its supervisor does not keep, remembered from the events themselves so a resync announcement
/// (ADR-0054) can say them again: the folders it registered, which agent works in which, and each agent's session.
#[derive(Default, Clone)]
struct Reported {
    workspaces: std::collections::BTreeMap<String, v1::WorkspaceRegistered>,
    bound: std::collections::BTreeMap<String, String>,
    sessions: std::collections::BTreeMap<String, String>,
}

impl Reported {
    fn keeps(body: &Body) -> bool {
        matches!(
            body,
            Body::WorkspaceRegistered(_)
                | Body::WorkspaceStatusChanged(_)
                | Body::AgentWorkspaceBound(_)
                | Body::AgentSessionReported(_)
        )
    }

    fn note(&mut self, body: &Body) {
        match body {
            Body::WorkspaceRegistered(w) => {
                self.workspaces.insert(w.workspace_id.clone(), w.clone());
            }
            Body::WorkspaceStatusChanged(c) => {
                if let Some(w) = self.workspaces.get_mut(&c.workspace_id) {
                    w.files_changed = c.files_changed;
                }
            }
            Body::AgentWorkspaceBound(b) => {
                self.bound
                    .insert(b.agent_id.clone(), b.workspace_id.clone());
            }
            Body::AgentSessionReported(r) => {
                self.sessions
                    .insert(r.agent_id.clone(), r.session_id.clone());
            }
            _ => {}
        }
    }
}

/// Writes protocol events to the durable buffer.
pub struct EventSink {
    producer: String,
    log: Mutex<DiskEventLog>,
    /// Says again what the hub must know about this node, see [`EventSink::announce`].
    announcer: Mutex<Option<Announcer>>,
    /// See [`Reported`]. Never held together with `log`.
    reported: Mutex<Reported>,
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
            reported: Mutex::new(Reported::default()),
        })
    }

    /// What this node says about itself when the hub's record of it cannot be continued (see [`announce`](Self::announce)).
    pub fn set_announcer(&self, f: Announcer) {
        *self.announcer.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
    }

    /// Says again, at the tail of the buffer, what the hub must know about this node now: who it is, its checkouts and folders,
    /// the agents running with their state, session and terminal, and the approvals waiting (ADR-0054 section 3). Used when the
    /// hub cannot have what was recorded before: an empty data directory under a known name, or events lost to the size cap or
    /// to a hub restored from an older backup. Without an announcer (a bare sink in tests) it says nothing.
    ///
    /// Lock order: the announcer runs under the `announcer` lock and takes the supervisor's and the approvals' locks one at a time
    /// to read them, then `reported` and `log` one at a time to write; it never holds two of them together. Nothing that holds the
    /// supervisor's lock ever waits for the `announcer` lock, and `log` is never held while taking another lock.
    pub fn announce(&self) {
        let announcer = self.announcer.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(f) = &*announcer {
            f(self);
        }
    }

    /// The hub has acknowledged numbers this buffer never produced (this machine came back under a known name with an empty data
    /// directory): skip our numbering forward to `next` and say again who we are, since what we recorded before the skip is gone.
    pub fn catch_up_with_hub(&self, next: u64) -> io::Result<()> {
        self.log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .advance_to(next)?;
        self.announce();
        Ok(())
    }

    /// The present part of the announcement (see [`announce`](Self::announce)): the folders this node registered, then every
    /// agent in `views` that is running, in the order a launch reports it, then `waiting` approvals. The callers' locks are
    /// already released: `views` and `waiting` are copies.
    pub fn announce_present(&self, views: &[AgentView], waiting: &[crate::approvals::Pending]) {
        let known = self
            .reported
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        for w in known.workspaces.values() {
            self.record(Body::WorkspaceRegistered(w.clone()));
        }
        for v in views.iter().filter(|v| v.running) {
            let id = v.id.as_str();
            if v.directory {
                if let Some(ws) = known.bound.get(id) {
                    self.record(Body::AgentWorkspaceBound(v1::AgentWorkspaceBound {
                        agent_id: id.to_string(),
                        workspace_id: ws.clone(),
                    }));
                }
            }
            self.agent_up(v, known.sessions.get(id));
        }
        for p in waiting {
            self.approval_requested(p);
        }
    }

    /// An agent in `view` is up: started, its state, its session if known, and its terminal if it has one.
    fn agent_up(&self, view: &AgentView, session: Option<&String>) {
        let agent = view.id.as_str();
        self.record(Body::AgentStarted(v1::AgentStarted {
            agent_id: agent.into(),
            project_id: view.project.clone(),
            node_id: self.producer.clone(),
            kind: view.kind.clone(),
            worktree_id: if view.directory {
                String::new()
            } else {
                view.worktree.to_string_lossy().into_owned()
            },
            level: match view.level {
                agora_core::IntegrationLevel::L0Generic => v1::IntegrationLevel::L0Generic,
                agora_core::IntegrationLevel::L1Manifest => v1::IntegrationLevel::L1Manifest,
                agora_core::IntegrationLevel::L2Acp => v1::IntegrationLevel::L2Acp,
                agora_core::IntegrationLevel::L3Adapter => v1::IntegrationLevel::L3Adapter,
            } as i32,
        }));
        self.record(Body::AgentStateChanged(v1::AgentStateChanged {
            agent_id: agent.into(),
            state: proto_state(view.state) as i32,
            reported: false,
        }));
        if let Some(session) = session {
            self.record(Body::AgentSessionReported(v1::AgentSessionReported {
                agent_id: agent.into(),
                session_id: session.clone(),
            }));
        }
        if view.terminal {
            self.record(Body::TerminalOpened(v1::TerminalOpened {
                terminal_id: crate::control::terminal_id_of(&view.id),
                node_id: self.producer.clone(),
                purpose: v1::TerminalPurpose::Agent as i32,
                agent_id: agent.into(),
            }));
        }
    }

    fn record(&self, body: Body) {
        // Only the few kinds a resync announcement repeats take this lock; every other event goes straight to the log.
        if Reported::keeps(&body) {
            self.reported
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .note(&body);
        }
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

    /// Publish lifecycle and reported events using the supervisor's current process/session facts.
    pub fn supervisor_events(&self, events: &[SupervisorEvent], views: &[AgentView]) {
        for e in events {
            match e {
                SupervisorEvent::Started { agent } | SupervisorEvent::Resumed { agent, .. } => {
                    if let Some(view) = views.iter().find(|v| v.id == *agent) {
                        self.agent_up(view, None);
                    }
                }
                SupervisorEvent::SessionReported { agent, session } => {
                    self.record(Body::AgentSessionReported(v1::AgentSessionReported {
                        agent_id: agent.as_str().into(),
                        session_id: session.clone(),
                    }));
                }
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

    pub fn node_resources(
        &self,
        node: &str,
        agents_running: u32,
        terminals_open: u32,
        memory_used_bytes: u64,
    ) {
        self.record(Body::NodeResources(v1::NodeResources {
            node_id: node.to_string(),
            agents_running,
            terminals_open,
            memory_used_bytes,
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

    pub fn directory_agent_launched(
        &self,
        agent: &AgentId,
        workspace: &v1::WorkspaceRegistered,
        kind: &str,
        terminal: bool,
    ) {
        self.record(Body::WorkspaceRegistered(workspace.clone()));
        self.record(Body::AgentWorkspaceBound(v1::AgentWorkspaceBound {
            agent_id: agent.as_str().into(),
            workspace_id: workspace.workspace_id.clone(),
        }));
        self.agent_started(agent, &workspace.project_id, &workspace.node_id, "", kind);
        if terminal {
            self.record(Body::TerminalOpened(v1::TerminalOpened {
                terminal_id: crate::control::terminal_id_of(agent),
                node_id: workspace.node_id.clone(),
                purpose: v1::TerminalPurpose::Agent as i32,
                agent_id: agent.as_str().into(),
            }));
        }
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
    /// Routed commands that need a user-signed grant (`signed_commands = a, b*`; ADR-0041). Empty: none do.
    pub signed_commands: crate::grants::SignedCommands,
    /// How agents are confined (`sandbox`, `sandbox.read`, `sandbox.write`; ADR-0047). Default: not at all.
    pub sandbox: crate::sandbox::SandboxPolicy,
    /// What each trusted device may sign (`device.<id>.projects = a, b`; ADR-0049). A device not named is not limited.
    pub device_projects: std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            mesh_remote: true,
            signed_commands: crate::grants::SignedCommands::default(),
            sandbox: crate::sandbox::SandboxPolicy::default(),
            device_projects: std::collections::BTreeMap::new(),
        }
    }
}

/// The `sandbox.read` / `sandbox.write` entries exactly as written, each with its key, so `agora doctor` can say which ones the node
/// drops (relative, `..`, a `~` with no home folder) or cannot use yet (missing). Same line rules as [`read_policy`].
pub fn policy_sandbox_entries(path: &std::path::Path) -> Vec<(&'static str, String)> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let key = match k.trim() {
            "sandbox.read" => "sandbox.read",
            "sandbox.write" => "sandbox.write",
            _ => continue,
        };
        out.extend(
            v.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| (key, s.to_string())),
        );
    }
    out
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
            match k.trim() {
                "mesh.remote" => p.mesh_remote = v.trim() == "true",
                "signed_commands" => {
                    p.signed_commands = crate::grants::SignedCommands(
                        v.split(',')
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_string)
                            .collect(),
                    );
                }
                key if key.starts_with("device.") && key.ends_with(".projects") => {
                    // A named device with a value that names nothing is a device limited to nothing: a typo never widens.
                    let id = &key["device.".len()..key.len() - ".projects".len()];
                    if !id.is_empty() {
                        p.device_projects.insert(
                            id.to_string(),
                            v.split(',')
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                                .map(str::to_string)
                                .collect(),
                        );
                    }
                }
                "sandbox" => p.sandbox.level = crate::sandbox::Level::parse(v),
                "sandbox.read" => p.sandbox.read = paths(v),
                "sandbox.write" => p.sandbox.write = paths(v),
                "sandbox.trust_manifest" => {
                    p.sandbox.trusted_kinds = v
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect();
                }
                _ => {}
            }
        }
    }
    p
}

/// `sandbox.read` / `sandbox.write`: absolute paths or `~/` below the home folder; a relative or `..` path is dropped, never guessed at.
fn paths(v: &str) -> Vec<PathBuf> {
    v.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(crate::sandbox::expand)
        .collect()
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
    /// User-signed grants for commands the hub routes here (ADR-0041).
    pub grants: Arc<crate::grants::Grants>,
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
        sup.set_sandbox(read_policy(&cfg.data_dir.join(POLICY_FILE)).sandbox);
        let (found, manifest_problems) = load_manifests(&cfg.manifests_dir);
        for m in found {
            sup.register(m);
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
        let known_checkouts: Vec<(String, PathBuf)> = projects
            .iter()
            .map(|p| (p.id.clone(), p.root.clone()))
            .collect();

        // Every agent that launches joins the mesh and, if its manifest has `[mcp]`, is handed its own endpoint.
        {
            let for_abort = Arc::clone(&mesh);
            let mesh = Arc::clone(&mesh);
            sup.set_launch_hook(Box::new(move |l| {
                let socket = mesh
                    .register_confined(
                        agora_mesh::broker::AgentInfo {
                            id: l.id.clone(),
                            kind: l.kind.clone(),
                            node: l.node.clone(),
                            project: l.project.clone(),
                            label: l.id.as_str().to_string(),
                            worktree: l.worktree.to_string_lossy().into_owned(),
                            state: "idle".into(),
                        },
                        l.identity.as_deref(),
                    )
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
        let policy_now = read_policy(&cfg.data_dir.join(POLICY_FILE));
        // Limited devices only mean something if the commands they could sign are the ones that need a signature.
        let signed =
            if policy_now.signed_commands.0.is_empty() && !policy_now.device_projects.is_empty() {
                crate::grants::SignedCommands::team_default()
            } else {
                policy_now.signed_commands
            };
        let limits = Arc::new(crate::limits::DeviceLimits::new(policy_now.device_projects));
        approvals.set_limits(Arc::clone(&limits));
        let grants = Arc::new(
            crate::grants::Grants::new(&cfg.node_id, Arc::clone(&approvals), signed)
                .with_limits(Arc::clone(&limits))
                .with_directory_nonce_file(cfg.data_dir.join("directory-nonces.json")),
        );

        let supervisor = Arc::new(Mutex::new(sup));
        {
            let sup = Arc::clone(&supervisor);
            let sup2 = Arc::clone(&sup);
            grants.set_project_resolver(Box::new(move |agent| {
                let s = sup.lock().unwrap_or_else(|e| e.into_inner());
                s.list()
                    .into_iter()
                    .find(|v| v.id.as_str() == agent)
                    .map(|v| v.project)
            }));
            limits.set_project_resolver(Box::new(move |agent| {
                let s = sup2.lock().unwrap_or_else(|e| e.into_inner());
                s.list()
                    .into_iter()
                    .find(|v| v.id.as_str() == agent)
                    .map(|v| v.project)
            }));
        }
        {
            // What this node says when the hub's record of it cannot be continued (ADR-0054): who it is, its checkouts, and its
            // present. Each lock is taken alone and released before the next, and before anything is recorded (see `announce`).
            let (node, sup, approvals) = (
                cfg.node_id.clone(),
                Arc::clone(&supervisor),
                Arc::clone(&approvals),
            );
            events.set_announcer(Box::new(move |ev| {
                let views = sup.lock().unwrap_or_else(|e| e.into_inner()).list();
                let waiting = approvals.waiting();
                ev.node_online(&node);
                for (id, root) in &known_checkouts {
                    ev.checkout_discovered(&node, id, root);
                }
                ev.announce_present(&views, &waiting);
            }));
        }
        let chat = Arc::new(crate::chat::ChatBoard::new());
        let git_wake = Arc::new(AtomicBool::new(false));
        let ctx = Arc::new(
            ControlContext::new(
                Arc::clone(&supervisor),
                projects,
                cfg.worktree_root.clone(),
                cfg.node_id.clone(),
                Some(Arc::clone(&events)),
                Some(Arc::clone(&mesh)),
                Some(Arc::clone(&approvals)),
                Arc::clone(&chat),
            )
            .with_manifests_dir(cfg.manifests_dir.clone())
            .with_git_wake(Arc::clone(&git_wake)),
        );
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
                    let mut reported_memory = 0u64;
                    let mut memory_checked = Instant::now();
                    let mut usage = crate::usage_files::UsageWatcher::default();
                    while !stop.load(Ordering::Acquire) {
                        let (events, views, usage_targets, ended) = {
                            let mut s = sup.lock().unwrap_or_else(|e| e.into_inner());
                            let events = s.tick(Instant::now());
                            (
                                events,
                                s.list(),
                                s.usage_targets(),
                                s.take_ended_usage_targets(),
                            )
                        };
                        // Usage that terminal agents wrote to their own files (ADR-0042); the files are read outside the lock.
                        let mut used = usage.finish(Instant::now(), &ended);
                        used.extend(usage.poll(Instant::now(), &usage_targets));
                        if !used.is_empty() {
                            ev.record_bodies(
                                used.into_iter()
                                    .map(|(agent, s)| {
                                        Body::AgentUsageReported(v1::AgentUsageReported {
                                            agent_id: agent.as_str().to_string(),
                                            model: s.model,
                                            input_tokens: s.input,
                                            output_tokens: s.output,
                                        })
                                    })
                                    .collect(),
                            );
                        }
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
                        // Resources are reported when they change, not on a timer, so an idle node is silent: when the number of
                        // running agents changes, and when their memory moved a lot since the last report (checked every
                        // `MEMORY_CHECK`, only while agents run).
                        let running = u32::try_from(views.iter().filter(|v| v.running).count())
                            .unwrap_or(u32::MAX);
                        let count_changed = (running, running) != reported;
                        if count_changed
                            || (running > 0 && memory_checked.elapsed() >= MEMORY_CHECK)
                        {
                            memory_checked = Instant::now();
                            let pids = sup.lock().unwrap_or_else(|e| e.into_inner()).running_pids();
                            let memory = agents_memory(&pids);
                            if count_changed || memory_moved(reported_memory, memory) {
                                reported = (running, running);
                                reported_memory = memory;
                                ev.node_resources(&node_id, running, running, memory);
                            }
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
                .spawn(move || watch_git(&sup, &ev, &stop, &git_wake))?
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
            grants,
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
/// How often the memory of running agents is measured between changes in their number.
const MEMORY_CHECK: Duration = Duration::from_secs(30);

/// Resident memory of every process at or below `roots` (each running agent's own process tree), counted once each. A process that
/// cannot be read (gone, or not ours to read) counts as nothing.
fn agents_memory(roots: &[u32]) -> u64 {
    let mut seen = std::collections::HashSet::new();
    let mut total = 0u64;
    for root in roots {
        let Ok(tree) = agora_platform::process_tree(*root) else {
            continue;
        };
        for p in tree {
            if seen.insert(p.pid) {
                total += agora_platform::process_stats(p.pid).map_or(0, |s| s.rss_bytes);
            }
        }
    }
    total
}

/// Whether memory moved enough since the last report to send another: by more than a quarter, or by more than 256 MiB.
fn memory_moved(reported: u64, now: u64) -> bool {
    let moved = reported.abs_diff(now);
    moved > reported / 4 || moved > 256 << 20
}

/// `wake` (set when an agent starts) ends the wait between rounds early, so a new folder's changes are counted at once.
fn watch_git(sup: &Mutex<Supervisor>, ev: &EventSink, stop: &AtomicBool, wake: &AtomicBool) {
    let mut last: std::collections::HashMap<(PathBuf, bool), (u32, u32, u32, String)> =
        std::collections::HashMap::new();
    while !stop.load(Ordering::Acquire) {
        let views = sup.lock().unwrap_or_else(|e| e.into_inner()).list();
        last.retain(|(path, _), _| views.iter().any(|v| &v.worktree == path));
        // Shared sessions have one Git status call per directory. A legacy and a new binding each need their event.
        let mut paths = std::collections::BTreeMap::<&Path, (bool, bool)>::new();
        for v in views.iter().filter(|v| v.running) {
            let modes = paths.entry(&v.worktree).or_default();
            if v.directory {
                modes.0 = true;
            } else {
                modes.1 = true;
            }
        }
        for (path, (directory, legacy)) in paths {
            if stop.load(Ordering::Acquire) {
                return;
            }
            let Ok(st) = git::status(path) else {
                continue;
            };
            let now = (
                st.files_changed,
                st.insertions,
                st.deletions,
                st.head_commit.clone(),
            );
            for (enabled, is_directory) in [(directory, true), (legacy, false)] {
                if !enabled {
                    continue;
                }
                let key = (path.to_path_buf(), is_directory);
                if last.get(&key) == Some(&now) {
                    continue;
                }
                last.insert(key, now.clone());
                if is_directory {
                    ev.record(Body::WorkspaceStatusChanged(v1::WorkspaceStatusChanged {
                        workspace_id: format!("directory:{}", path.to_string_lossy()),
                        files_changed: st.files_changed,
                        insertions: st.insertions,
                        deletions: st.deletions,
                        head_commit: st.head_commit.clone(),
                    }));
                } else {
                    ev.git_status_changed(&path.to_string_lossy(), &st);
                }
            }
        }
        let until = Instant::now() + GIT_POLL;
        while Instant::now() < until
            && !stop.load(Ordering::Acquire)
            && !wake.swap(false, Ordering::AcqRel)
        {
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

/// Every `*.toml` agent manifest in `dir`, in file-name order, and each file that could not be read or parsed with the reason.
/// The node reads them at start and again before it lists or starts agents, so a manifest added later is seen without a restart.
pub fn load_manifests(dir: &Path) -> (Vec<Manifest>, Vec<(PathBuf, String)>) {
    let mut found = Vec::new();
    let mut problems = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
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
                Ok(m) => found.push(m),
                Err(e) => problems.push((p, e)),
            }
        }
    }
    (found, problems)
}

#[cfg(test)]
#[allow(clippy::expect_used)] // expect is allowed in tests
mod memory_tests {
    use super::{agents_memory, memory_moved};

    #[test]
    fn memory_is_reported_again_only_when_it_moved_a_lot() {
        assert!(!memory_moved(0, 0));
        assert!(
            memory_moved(0, 1 << 20),
            "the first measurement after nothing"
        );
        let gib = 1u64 << 30;
        assert!(!memory_moved(gib, gib + (gib / 5)), "20% up");
        assert!(memory_moved(gib, gib + (gib / 3)), "33% up");
        assert!(memory_moved(gib, gib / 2), "half down");
        let big = 8 * gib;
        assert!(memory_moved(big, big + (300 << 20)), "300 MiB on 8 GiB");
        assert!(!memory_moved(big, big + (200 << 20)), "200 MiB on 8 GiB");
    }

    #[test]
    fn an_agents_memory_counts_its_whole_process_tree() {
        // A shell with a child below it, as an agent with the tools it started. On Windows a native one: Git for Windows' `sh`
        // starts `sleep` through a helper that exits, so `sleep`'s parent is gone and no tree leads to it.
        let mut shell = if std::env::consts::OS == "windows" {
            std::process::Command::new("cmd")
                .args(["/c", "ping -n 30 127.0.0.1 >nul"])
                .spawn()
        } else {
            std::process::Command::new("sh")
                .args(["-c", "sleep 30 & wait"])
                .spawn()
        }
        .expect("spawn");
        let pid = shell.id();
        // The child shows up once the shell has started it, which takes a while on a busy machine (windows' fix).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let tree = loop {
            let tree = agora_platform::process_tree(pid).expect("tree");
            if tree.len() >= 2 || std::time::Instant::now() > deadline {
                break tree;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        };
        let measured = agents_memory(&[pid]);
        // Counted once, however often it is named.
        let again = agents_memory(&[pid, pid]);
        let own = agora_platform::process_stats(pid)
            .map(|s| s.rss_bytes)
            .expect("stats");
        let _ = shell.kill();
        let _ = shell.wait();
        assert!(tree.len() >= 2, "the shell and its child: {tree:?}");
        assert!(
            measured > own,
            "the child is counted too: {measured} vs {own}"
        );
        assert!(
            again.abs_diff(measured) < measured / 2,
            "{again} vs {measured}"
        );
        assert_eq!(
            agents_memory(&[u32::MAX - 1]),
            0,
            "a process that does not exist"
        );
    }
}
