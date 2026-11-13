//! The node daemon: supervisor + control socket + event buffer, running without any hub.
//!
//! Events go to the durable buffer (`event_buffer`) as protocol `Event`s; with no hub connected they are
//! simply retained (size-capped) until one acknowledges them. Agents never depend on any of this succeeding.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
/// In the event buffer's folder: the producer (node id) whose events the log holds.
const PRODUCER_FILE: &str = "producer";
/// Default size cap of the on-disk event buffer.
pub const EVENT_BUFFER_BYTES: u64 = 64 * 1024 * 1024;

/// How an announcement begins (ADR-0055).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Announce {
    /// With `NodeOnline`: the hub will never see this process's start, so everything it knew of the node before is reset and only
    /// what runs now is said again (ADR-0054 section 3).
    Restart,
    /// Without `NodeOnline`: every agent of this process is stated once, running or ended, with the approvals that wait and those
    /// resolved recently (ADR-0055 section 2).
    Present,
}

/// What a node says about itself again when the hub's record of it cannot be continued (see [`EventSink::announce`] and
/// [`EventSink::resync`]). The announcer records `NodeOnline` too, for [`Announce::Restart`] only.
type Announcer = Box<dyn Fn(&EventSink, Announce) + Send>;

/// How many resolved approvals a [`Announce::Present`] announcement says again (ADR-0055).
pub const RESOLVED_KEPT: usize = 256;

/// Facts the node reported that its supervisor does not keep, remembered from the events themselves so a resync announcement
/// (ADR-0054, ADR-0055) can say them again: the folders it registered, which agent works in which, each agent's session, terminal
/// and last exit code, and the last [`RESOLVED_KEPT`] resolved approvals.
#[derive(Default, Clone)]
struct Reported {
    workspaces: std::collections::BTreeMap<String, v1::WorkspaceRegistered>,
    bound: std::collections::BTreeMap<String, String>,
    sessions: std::collections::BTreeMap<String, String>,
    exits: std::collections::BTreeMap<String, i32>,
    terminals: std::collections::BTreeSet<String>,
    resolved: std::collections::VecDeque<v1::ApprovalResolved>,
}

impl Reported {
    fn keeps(body: &Body) -> bool {
        matches!(
            body,
            Body::WorkspaceRegistered(_)
                | Body::WorkspaceStatusChanged(_)
                | Body::AgentWorkspaceBound(_)
                | Body::AgentSessionReported(_)
                | Body::AgentExited(_)
                | Body::TerminalOpened(_)
                | Body::ApprovalResolved(_)
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
            Body::AgentExited(x) => {
                self.exits.insert(x.agent_id.clone(), x.exit_code);
            }
            Body::TerminalOpened(t) if !t.agent_id.is_empty() => {
                self.terminals.insert(t.agent_id.clone());
            }
            // Said again by an announcement, it is already here: the ring keeps its order and never holds one twice.
            Body::ApprovalResolved(r)
                if !self.resolved.iter().any(|k| k.approval_id == r.approval_id) =>
            {
                if self.resolved.len() == RESOLVED_KEPT {
                    self.resolved.pop_front();
                }
                self.resolved.push_back(r.clone());
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
    /// The `seq` of the last `NodeOnline` recorded, 0 for none (ADR-0055: a resync says `NodeOnline` only when the hub lost it).
    last_online: AtomicU64,
    /// See [`EventSink::set_aside`].
    set_aside: Option<SetAside>,
}

/// An event log found at start whose unacknowledged events name another producer (for example `local`, from runs before the node
/// was enrolled). It was renamed, never deleted, and is not sent to the hub.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetAside {
    /// The producer its events name.
    pub old_id: String,
    /// Where the old log is now.
    pub path: PathBuf,
}

/// ADR-0058: an adapter's currency as an upper-case three-letter code, or `None` for anything else.
fn currency_code(c: &str) -> Option<String> {
    (c.len() == 3 && c.bytes().all(|b| b.is_ascii_alphabetic())).then(|| c.to_ascii_uppercase())
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
    ///
    /// One log holds the events of one producer. When events not yet acknowledged name another producer (the node ran as
    /// `local` before it was enrolled, or under another name), they must never be replayed: the hub refuses events that name
    /// another node than the certificate. The old log is set aside (renamed, never deleted, see [`DiskEventLog::set_aside`]) and
    /// a fresh one starts; [`set_aside`](Self::set_aside) says so.
    pub fn open_with_cap(dir: &Path, producer: &str, max_bytes: u64) -> io::Result<Self> {
        let mut log = DiskEventLog::open(dir, max_bytes)?;
        // Whose log this is: the `producer` file says so (its events may all be acknowledged and compacted away, but the numbering
        // still belongs to that name); a log from before the file existed is judged by any event it still holds.
        let recorded = fs::read_to_string(dir.join(PRODUCER_FILE))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s != producer);
        let foreign = match recorded {
            Some(old) => Some(old),
            None => log
                .find_retained(|p| v1::Event::decode(p).is_ok_and(|e| e.producer_id != producer))?
                .and_then(|p| v1::Event::decode(p.as_slice()).ok())
                .map(|e| e.producer_id),
        };
        let mut set_aside = None;
        if let Some(old_id) = foreign.filter(|_| !log.is_pristine()) {
            let secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let path = log.set_aside(&old_id, secs)?;
            log = DiskEventLog::open(dir, max_bytes)?;
            set_aside = Some(SetAside { old_id, path });
        }
        let tmp = dir.join(format!("{PRODUCER_FILE}.tmp"));
        fs::write(&tmp, producer)?;
        fs::rename(&tmp, dir.join(PRODUCER_FILE))?;
        Ok(Self {
            producer: producer.to_string(),
            log: Mutex::new(log),
            announcer: Mutex::new(None),
            reported: Mutex::new(Reported::default()),
            last_online: AtomicU64::new(0),
            set_aside,
        })
    }

    /// The log this sink found at open and set aside because its events name another producer, if it did.
    pub fn set_aside(&self) -> Option<&SetAside> {
        self.set_aside.as_ref()
    }

    /// What this node says about itself when the hub's record of it cannot be continued (see [`announce`](Self::announce)).
    pub fn set_announcer(&self, f: Announcer) {
        *self.announcer.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
    }

    /// Says again, at the tail of the buffer, what the hub must know about this node now, led by `NodeOnline`: who it is, its
    /// checkouts and folders, the agents running with their state, session and terminal, and the approvals waiting (ADR-0054
    /// section 3, [`Announce::Restart`]). Used when the hub cannot have what was recorded before: an empty data directory under a
    /// known name, or (see [`resync`](Self::resync)) a start of this process the hub lost. Without an announcer (a bare sink in
    /// tests) it says nothing.
    ///
    /// Lock order: the announcer runs under the `announcer` lock and takes the supervisor's and the approvals' locks one at a time
    /// to read them, then `reported` and `log` one at a time to write; it never holds two of them together. Nothing that holds the
    /// supervisor's lock ever waits for the `announcer` lock, and `log` is never held while taking another lock.
    pub fn announce(&self) {
        self.announce_as(Announce::Restart);
    }

    fn announce_as(&self, how: Announce) {
        let announcer = self.announcer.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(f) = &*announcer {
            f(self, how);
        }
    }

    /// The hub cannot have `from_seq..oldest_retained` (ADR-0054): say what is true now at the tail (ADR-0055). The announcement
    /// begins with `NodeOnline` only when this process's start is in that lost range; otherwise it states facts and leaves what the
    /// hub knows of running agents (and their work on a shared board) in place. If the announcement itself pushes the start out of
    /// the buffer under the size cap, a full one led by `NodeOnline` follows.
    pub fn resync(&self, from_seq: u64, oldest_retained: u64) {
        let lost = |oldest: u64| {
            let start = self.last_online.load(Ordering::Acquire);
            start != 0 && start >= from_seq && start < oldest
        };
        if lost(oldest_retained) {
            self.announce_as(Announce::Restart);
            return;
        }
        self.announce_as(Announce::Present);
        if lost(self.oldest_retained()) {
            self.announce_as(Announce::Restart);
        }
    }

    /// The `seq` of the last `NodeOnline` this sink recorded, 0 for none.
    pub fn last_online(&self) -> u64 {
        self.last_online.load(Ordering::Acquire)
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

    /// The present part of an announcement (see [`announce`](Self::announce) and [`resync`](Self::resync)): the folders this node
    /// registered, then every agent in `views` that is running, in the order a launch reports it, then `waiting` approvals. For
    /// [`Announce::Present`] also every agent in `views` that ended (`AgentExited` with the last exit code recorded for it, or the
    /// stop convention `i32::MIN`, and `TerminalClosed` if it had a terminal) and the last [`RESOLVED_KEPT`] resolved approvals.
    /// The callers' locks are already released: `views` and `waiting` are copies.
    pub fn announce_present(
        &self,
        views: &[AgentView],
        waiting: &[crate::approvals::Pending],
        how: Announce,
    ) {
        let known = self
            .reported
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        for w in known.workspaces.values() {
            self.record(Body::WorkspaceRegistered(w.clone()));
        }
        for v in views {
            let id = v.id.as_str();
            if !v.running {
                if how == Announce::Present {
                    self.record(Body::AgentExited(v1::AgentExited {
                        agent_id: id.to_string(),
                        exit_code: known.exits.get(id).copied().unwrap_or(i32::MIN),
                    }));
                    if known.terminals.contains(id) {
                        self.record(Body::TerminalClosed(v1::TerminalClosed {
                            terminal_id: crate::control::terminal_id_of(&v.id),
                        }));
                    }
                }
                continue;
            }
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
        if how == Announce::Present {
            for r in known.resolved {
                self.record(Body::ApprovalResolved(r));
            }
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
            parent_agent_id: view
                .parent
                .as_ref()
                .map(|p| p.as_str().to_string())
                .unwrap_or_default(),
            launch_options: Vec::new(),
            skips_permissions: false,
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
        let online = matches!(body, Body::NodeOnline(_));
        let mut log = self.log.lock().unwrap_or_else(|e| e.into_inner());
        let ev = v1::Event {
            producer_id: self.producer.clone(),
            seq: log.next_seq(),
            at_ms: now_ms(),
            body: Some(body),
        };
        // A full disk must never stop agents: losing an event is better than losing the node.
        if let (Ok(appended), true) = (log.append(&ev.encode_to_vec()), online) {
            self.last_online.store(appended.seq, Ordering::Release);
        }
    }

    pub fn agent_started(
        &self,
        agent: &AgentId,
        project: &str,
        node: &str,
        worktree: &str,
        kind: &str,
    ) {
        self.agent_started_by(agent, project, node, worktree, kind, "");
    }

    /// [`Self::agent_started`] for a helper another agent started (ADR-0057); `parent` is empty when the user started it.
    pub fn agent_started_by(
        &self,
        agent: &AgentId,
        project: &str,
        node: &str,
        worktree: &str,
        kind: &str,
        parent: &str,
    ) {
        self.record(Body::AgentStarted(v1::AgentStarted {
            agent_id: agent.as_str().to_string(),
            project_id: project.to_string(),
            node_id: node.to_string(),
            worktree_id: worktree.to_string(),
            kind: kind.to_string(),
            level: v1::IntegrationLevel::L1Manifest as i32,
            parent_agent_id: parent.to_string(),
            launch_options: Vec::new(),
            skips_permissions: false,
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
                        cost_micros,
                        currency,
                        context_tokens,
                        context_window,
                        ..
                    } => self.record(Body::AgentUsageReported(v1::AgentUsageReported {
                        agent_id: agent.as_str().to_string(),
                        model: model.clone(),
                        input_tokens: *input_tokens,
                        output_tokens: *output_tokens,
                        // ADR-0058: the agent's own cost, only with a currency the hub can tell apart (three letters).
                        cost_micros: cost_micros.filter(|_| currency_code(currency).is_some()),
                        currency: currency_code(currency).unwrap_or_default(),
                        context_tokens: *context_tokens,
                        context_window: *context_window,
                    })),
                    // ADR-0058: the supervisor passes these on checked and bounded, and only when they changed.
                    AdapterEvent::Options { options, .. } => {
                        let list = |l: &[agora_adapters::host::SessionOption]| {
                            l.iter()
                                .map(|o| v1::SessionOption {
                                    id: o.id.clone(),
                                    name: o.name.clone(),
                                    description: o.description.clone(),
                                    skips_permissions: o.skips_permissions,
                                })
                                .collect()
                        };
                        self.record(Body::AgentSessionOptions(v1::AgentSessionOptions {
                            agent_id: agent.as_str().to_string(),
                            modes: list(&options.modes),
                            models: list(&options.models),
                            current_mode: options.current_mode.clone(),
                            current_model: options.current_model.clone(),
                        }));
                    }
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
        self.start_resolved(approval_id, approved, signer_device_id, 0, false);
    }

    /// The decision on an `agent.start` request (ADR-0057), with the grant that was signed.
    pub fn start_resolved(
        &self,
        approval_id: &str,
        approved: bool,
        signer_device_id: &str,
        helper_grant: u32,
        helpers_may_start: bool,
    ) {
        self.record(Body::ApprovalResolved(v1::ApprovalResolved {
            approval_id: approval_id.to_string(),
            approved,
            signer_device_id: signer_device_id.to_string(),
            helper_grant,
            helpers_may_start,
        }));
    }

    /// The node refused an agent's `agent_start` before asking anyone (ADR-0057).
    pub fn agent_start_refused(&self, agent: &AgentId, kind: &str, folder: &str, code: &str) {
        self.record(Body::AgentStartRefused(v1::AgentStartRefused {
            agent_id: agent.as_str().to_string(),
            kind: kind.to_string(),
            folder: folder.to_string(),
            code: code.to_string(),
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
        self.helper_launched(agent, workspace, kind, terminal, "");
    }

    /// [`Self::directory_agent_launched`] for a helper `parent` started (ADR-0057); `parent` empty means the user.
    pub fn helper_launched(
        &self,
        agent: &AgentId,
        workspace: &v1::WorkspaceRegistered,
        kind: &str,
        terminal: bool,
        parent: &str,
    ) {
        self.record(Body::WorkspaceRegistered(workspace.clone()));
        self.record(Body::AgentWorkspaceBound(v1::AgentWorkspaceBound {
            agent_id: agent.as_str().into(),
            workspace_id: workspace.workspace_id.clone(),
        }));
        self.agent_started_by(
            agent,
            &workspace.project_id,
            &workspace.node_id,
            "",
            kind,
            parent,
        );
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

    /// Like [`payloads_after`](Self::payloads_after), at most `max` of them: one uplink batch.
    pub fn payloads_after_max(
        &self,
        after: u64,
        max: usize,
    ) -> Result<Vec<Vec<u8>>, crate::event_buffer::ReplayError> {
        self.log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .replay_after_max(after, max)
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
    /// Agents that start agents (ADR-0057).
    pub spawner: Arc<crate::spawn::Spawner>,
    node_id: String,
    /// Manifests that could not be loaded, as `(path, reason)` for the log.
    pub manifest_problems: Vec<(PathBuf, String)>,
    /// The event log found at start that named another node and was set aside instead of being sent to the hub, for the log.
    pub events_set_aside: Option<SetAside>,
}

impl Node {
    pub fn start(cfg: &NodeConfig) -> Result<Self, NodeError> {
        Self::start_with_event_cap(cfg, EVENT_BUFFER_BYTES)
    }

    /// Like [`start`](Self::start) with an explicit size cap for the event buffer, in bytes (tests use a small one to lose events
    /// the way a long offline burst does).
    pub fn start_with_event_cap(cfg: &NodeConfig, max_bytes: u64) -> Result<Self, NodeError> {
        std::fs::create_dir_all(&cfg.data_dir)?;
        std::fs::create_dir_all(&cfg.worktree_root)?;
        let events = Arc::new(EventSink::open_with_cap(
            &cfg.data_dir.join("events"),
            &cfg.node_id,
            max_bytes,
        )?);

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
            // What this node says when the hub's record of it cannot be continued (ADR-0054, ADR-0055): who it is (only when the hub
            // lost this process's start), its checkouts, and its present. Each lock is taken alone and released before the next, and
            // before anything is recorded (see `announce`).
            let (node, sup, approvals) = (
                cfg.node_id.clone(),
                Arc::clone(&supervisor),
                Arc::clone(&approvals),
            );
            events.set_announcer(Box::new(move |ev, how| {
                let views = sup.lock().unwrap_or_else(|e| e.into_inner()).list();
                let waiting = approvals.waiting();
                if how == Announce::Restart {
                    ev.node_online(&node);
                }
                for (id, root) in &known_checkouts {
                    ev.checkout_discovered(&node, id, root);
                }
                ev.announce_present(&views, &waiting, how);
            }));
        }
        let chat = Arc::new(crate::chat::ChatBoard::new());
        let git_wake = Arc::new(AtomicBool::new(false));
        // ADR-0057: agents may ask to start helpers; every start needs a decision a trusted device signed.
        let spawner = Arc::new(
            crate::spawn::Spawner::new(
                &cfg.node_id,
                &cfg.worktree_root,
                Arc::clone(&supervisor),
                Arc::clone(&approvals),
                Some(Arc::clone(&events)),
                Arc::clone(&chat),
            )
            .with_git_wake(Arc::clone(&git_wake)),
        );
        mesh.set_helpers(Arc::clone(&spawner) as Arc<dyn crate::mesh_server::Helpers>);
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
            .with_git_wake(Arc::clone(&git_wake))
            .with_spawner(Arc::clone(&spawner)),
        );
        let server = ControlServer::start(&cfg.socket, ctx)?;

        let stop = Arc::new(AtomicBool::new(false));
        let ticker = {
            let (sup, ev, stop, node_id, mesh, approvals, chat, spawner) = (
                Arc::clone(&supervisor),
                Arc::clone(&events),
                Arc::clone(&stop),
                cfg.node_id.clone(),
                Arc::clone(&mesh),
                Arc::clone(&approvals),
                Arc::clone(&chat),
                Arc::clone(&spawner),
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
                                            ..Default::default()
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
                            // ADR-0057: an agent's helper grants and waiting start end when it exits or is stopped.
                            if let SupervisorEvent::Exited { agent, .. }
                            | SupervisorEvent::GaveUp { agent }
                            | SupervisorEvent::Stopped { agent } = e
                            {
                                spawner.agent_ended(agent);
                            }
                        }
                        spawner.tick();
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
                                    // The spawner first: a start the agent waited for is withdrawn and its slot released, before
                                    // the queue drops the request.
                                    spawner.agent_ended(agent);
                                    approvals.cancel_agent(agent);
                                }
                                _ => {}
                            }
                        }
                        for (node_approval, agent, adapter_id) in approvals.expire(now) {
                            if spawner.expired(&node_approval) {
                                continue;
                            }
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
        let events_set_aside = events.set_aside().cloned();
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
            spawner,
            node_id: cfg.node_id.clone(),
            manifest_problems,
            events_set_aside,
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod announce_tests {
    use super::*;

    fn sink(name: &str) -> EventSink {
        let dir =
            std::env::temp_dir().join(format!("agora-announce-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        EventSink::open(&dir, "n1").unwrap()
    }

    fn view(id: &str, running: bool, terminal: bool) -> AgentView {
        AgentView {
            directory: false,
            terminal: terminal && running,
            level: agora_core::IntegrationLevel::L1Manifest,
            id: AgentId::new(id),
            kind: "probe".into(),
            state: AgentState::Idle,
            worktree: PathBuf::from(format!("/wt/{id}")),
            branch: format!("agent/{id}"),
            project: "p".into(),
            running,
            restarts: 0,
            parent: None,
        }
    }

    fn said(s: &EventSink, mark: u64) -> Vec<Body> {
        s.events_after(mark)
            .into_iter()
            .filter_map(|e| e.body)
            .collect()
    }

    fn launched(s: &EventSink, id: &str, terminal: bool) {
        s.agent_launched(
            &AgentId::new(id),
            "p",
            "n1",
            Path::new("/work/p"),
            Path::new(&format!("/wt/{id}")),
            &format!("agent/{id}"),
            "probe",
            terminal,
        );
    }

    /// ADR-0055 section 2: without `NodeOnline` every agent of this process is stated once. An ended one gets `AgentExited` with
    /// the last exit code recorded for it (the stop convention `i32::MIN` when none was) and `TerminalClosed` if it had a terminal;
    /// a running one is announced as before. With `NodeOnline` (ADR-0054) ended agents are not mentioned.
    #[test]
    fn ended_agents_are_stated_with_their_recorded_exit_and_terminal() {
        let s = sink("ended");
        launched(&s, "crashed", true);
        launched(&s, "stopped", true);
        launched(&s, "chat", false);
        launched(&s, "kept", true);
        let views = [
            view("crashed", false, true),
            view("stopped", false, true),
            view("chat", false, false),
            view("never", false, false),
            view("kept", true, true),
        ];
        s.supervisor_events(
            &[
                SupervisorEvent::Exited {
                    agent: AgentId::new("crashed"),
                    status: agora_platform::ExitStatus {
                        code: Some(3),
                        signal: None,
                    },
                },
                SupervisorEvent::Exited {
                    agent: AgentId::new("chat"),
                    status: agora_platform::ExitStatus {
                        code: None,
                        signal: Some(9),
                    },
                },
                SupervisorEvent::Stopped {
                    agent: AgentId::new("stopped"),
                },
            ],
            &views,
        );
        let mark = s.next_seq() - 1;
        s.announce_present(&views, &[], Announce::Present);
        let present = said(&s, mark);
        let exits: Vec<(&str, i32)> = present
            .iter()
            .filter_map(|b| match b {
                Body::AgentExited(x) => Some((x.agent_id.as_str(), x.exit_code)),
                _ => None,
            })
            .collect();
        assert_eq!(
            exits,
            [
                ("crashed", 3),
                ("stopped", i32::MIN),
                ("chat", -9),
                ("never", i32::MIN)
            ]
        );
        let closed: Vec<&str> = present
            .iter()
            .filter_map(|b| match b {
                Body::TerminalClosed(t) => Some(t.terminal_id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            closed,
            ["t-crashed", "t-stopped"],
            "only those with a terminal"
        );
        let started: Vec<&str> = present
            .iter()
            .filter_map(|b| match b {
                Body::AgentStarted(a) => Some(a.agent_id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(started, ["kept"]);
        assert!(!present.iter().any(|b| matches!(b, Body::NodeOnline(_))));

        let mark = s.next_seq() - 1;
        s.announce_present(&views, &[], Announce::Restart);
        assert!(!said(&s, mark)
            .iter()
            .any(|b| matches!(b, Body::AgentExited(_) | Body::TerminalClosed(_))));
    }

    /// ADR-0055 section 2: the last 256 resolved approvals are said again, oldest first, so one answered inside the lost window
    /// does not stay listed as waiting. Saying them again does not change what is kept.
    #[test]
    fn the_last_resolved_approvals_are_said_again_and_the_ring_is_bounded() {
        let s = sink("resolved");
        for i in 0..300 {
            s.approval_resolved(&format!("ap-{i}"), i % 2 == 0, "dev");
        }
        let restated = |how| {
            let mark = s.next_seq() - 1;
            s.announce_present(&[], &[], how);
            said(&s, mark)
                .into_iter()
                .filter_map(|b| match b {
                    Body::ApprovalResolved(r) => Some(r),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let first = restated(Announce::Present);
        assert_eq!(first.len(), RESOLVED_KEPT);
        let ids: Vec<String> = first.iter().map(|r| r.approval_id.clone()).collect();
        assert_eq!(
            ids,
            (44..300).map(|i| format!("ap-{i}")).collect::<Vec<_>>()
        );
        assert!(first.iter().all(|r| r.signer_device_id == "dev"
            && r.approved == r.approval_id[3..].parse::<u32>().unwrap().is_multiple_of(2)));
        assert_eq!(restated(Announce::Present), first, "the ring did not move");
        assert!(restated(Announce::Restart).is_empty());
    }
}
