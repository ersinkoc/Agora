//! The hub's current view of the world, derived from the event log (docs/architecture.md: state is separate
//! from runtime). Pure data, no I/O: `apply` folds one event in, so the view can be rebuilt from the log at any
//! time and is identical however it was built. Clients get this as the snapshot, then live events.
//!
//! Ids are local to the machine that made them (`agent-1` exists on every node), so every entity is keyed by
//! `<producer>/<local id>` in this view (`laptop/agent-1`). Node ids are limited to `[A-Za-z0-9._-]` by the
//! certificate authority, so `/` never appears inside one and the hub can split a key to route a command back to
//! the right node. Project ids are machine-independent already (normalized remote + root commit) and stay as is.
//!
//! Unknown or out-of-order facts are tolerated (a worktree event for an unseen checkout still records the
//! worktree); the log is the truth, this is a cache.

use std::collections::{BTreeMap, BTreeSet};

use agora_protocol::v1::event::Body;
use agora_protocol::v1::{AgentState, Event};
use prost::Message;

use crate::ca::RESERVED_HUB_ID;
use crate::store::{EventStore, StoreError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeView {
    pub hostname: String,
    pub os: String,
    pub build: String,
    /// ADR-0069: the node's executable when its folder is not on the node's PATH (`NodeOnline.binary`); empty otherwise.
    pub binary: String,
    pub online: bool,
    pub agents_running: u32,
    pub terminals_open: u32,
    pub memory_used_bytes: u64,
    /// When the node was last heard from: its latest event (node clock), or the hub's record of its link (ADR-0070, hub clock).
    /// What the window shows as "last seen" for a machine that is offline.
    pub last_event_ms: u64,
    /// The generation of the node's latest `NodeLink { up }` (ADR-0070); 0 when none was seen. A `NodeLink` with a lower one is
    /// about a connection that was already replaced and changes nothing.
    pub link_generation: u64,
    /// ADR-0073: the owner had the hub forget this machine while it was offline. Clients are not shown it, or anything it ran, until
    /// it connects again. Its records are kept (a machine that comes back has its folders again; usage still counts).
    pub forgotten: bool,
}

impl NodeView {
    fn unknown(online: bool) -> Self {
        Self {
            hostname: String::new(),
            os: String::new(),
            build: String::new(),
            binary: String::new(),
            online,
            agents_running: 0,
            terminals_open: 0,
            memory_used_bytes: 0,
            last_event_ms: 0,
            link_generation: 0,
            forgotten: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeView {
    pub checkout_id: String,
    pub path: String,
    pub branch: String,
    pub files_changed: u32,
    pub insertions: u32,
    pub deletions: u32,
    pub head_commit: String,
}

/// ADR-0058: what an agent's adapter reported about its session beyond tokens.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SessionView {
    /// The last `AgentSessionOptions` (its `agent_id` is the node's local id).
    pub options: Option<agora_protocol::v1::AgentSessionOptions>,
    /// ADR-0090: the last `AgentSessionConfig` and `AgentCommands` (local agent id), kept so a window that connects later sees the
    /// model, mode and effort options and the slash commands. Bounded ([`MAX_CONFIG_BYTES`]); gone when the agent ends.
    pub config: Option<agora_protocol::v1::AgentSessionConfig>,
    pub commands: Option<agora_protocol::v1::AgentCommands>,
    /// The context's level and size as last reported; 0 when never reported.
    pub context_tokens: u64,
    pub context_window: u64,
    /// The sum of the cost the agent reported itself, in micro-units of `currency`; `None` when it reported none. A report in
    /// another currency starts the sum again.
    pub cost_micros: Option<u64>,
    pub currency: String,
}

/// ADR-0090: the largest `AgentSessionConfig` or `AgentCommands` the hub keeps, encoded. A node already holds to 64 options of 64
/// choices and 100 commands; this is the hub's own bound, so a node that does not cannot make the view grow. A larger report is not
/// kept (and the one before it is dropped, as it was replaced).
pub const MAX_CONFIG_BYTES: usize = 64 * 1024;
/// ADR-0090: counts the hub accepts (the node's caps).
const MAX_CONFIG_OPTIONS: usize = 64;
const MAX_CONFIG_CHOICES: usize = 64;
const MAX_COMMANDS: usize = 100;

#[derive(Debug, Clone, PartialEq)]
pub struct AgentView {
    pub node_id: String,
    pub project_id: String,
    pub worktree_id: String,
    pub kind: String,
    pub state: AgentState,
    /// True when the state was reported by the agent or an adapter rather than read from the screen.
    pub reported: bool,
    pub session_id: Option<String>,
    pub running: bool,
    pub exit_code: Option<i32>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// The model the agent last reported; empty until it reports one.
    pub model: String,
    /// ADR-0053: the owner had the hub forget this closed agent. It is no longer listed to clients; its tokens and cost still count.
    pub forgotten: bool,
    /// ADR-0058.
    pub session: SessionView,
    /// ADR-0057: the hub-wide key (`<node>/<agent>`) of the agent that started this one; empty when the user did.
    pub parent_agent_id: String,
    /// ADR-0059: the launch options it was started with, and whether it acts without asking (the badge).
    pub launch_options: Vec<String>,
    pub skips_permissions: bool,
    /// ADR-0060: the node found this agent interrupted (running when the node stopped). Cleared by its next `AgentStarted`.
    pub interrupted: Option<InterruptedView>,
}

/// ADR-0060: what `AgentInterrupted` said about an agent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InterruptedView {
    /// Resume continues its own session; false: only Start again (a new session in the same folder).
    pub resumable: bool,
    pub at_ms: u64,
    pub folder: String,
    /// Its process from before the node's restart still runs: Stop it before Resume.
    pub still_running: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalView {
    pub node_id: String,
    pub agent_id: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalView {
    pub agent_id: String,
    pub action: String,
    pub detail: String,
    pub expires_at_ms: u64,
    /// When the machine asked (its clock): with the answer's time, also the machine's, how long the agent waited.
    pub requested_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskView {
    pub project_id: String,
    pub title: String,
    /// `<node>/<agent>`; empty while the task is open.
    pub owner: String,
    pub done: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct View {
    /// Shared-board tasks (ADR-0040), keyed `<project>/<task id>`.
    pub tasks: BTreeMap<String, TaskView>,
    /// Held paths, keyed `<agent>\0<path>` → expiry (ms).
    pub leases: BTreeMap<String, (String, String, u64)>,
    pub workspaces: BTreeMap<String, agora_protocol::v1::WorkspaceRegistered>,
    pub agent_workspaces: BTreeMap<String, String>,
    pub nodes: BTreeMap<String, NodeView>,
    /// checkout id → (node id, project id, path)
    pub checkouts: BTreeMap<String, (String, String, String)>,
    pub worktrees: BTreeMap<String, WorktreeView>,
    pub agents: BTreeMap<String, AgentView>,
    pub terminals: BTreeMap<String, TerminalView>,
    /// Approvals still waiting for an answer.
    pub approvals: BTreeMap<String, ApprovalView>,
    /// ADR-0061: node id → the devices it said it trusts (its last `NodeTrustedDevices`).
    pub trusted_devices: BTreeMap<String, Vec<String>>,
    /// ADR-0073: the position of the latest event that brought a forgotten machine back. A client that last saw an earlier position
    /// may not hold that machine's records (it joined while they were hidden), so it is given a snapshot, not a replay.
    pub remembered_at: u64,
    /// History stored before the session refused cross-node records holds some (T19). The view reads them safely, but a client that
    /// was connected then may have applied one, and replaying the stored bodies would hand them to a client that did not. So a
    /// client whose position is at or before this one (the end of the log when the hub opened over such a record; 0 when there is none, which a position never is) is
    /// given a snapshot, not a replay. Stored events are never rewritten.
    pub distrusted_until: u64,
    /// ADR-0091: the newest `AgentQuotaObserved` per (machine, tool, window), a small bounded table ([`MAX_QUOTA_PER_NODE`] per
    /// machine). It carries no secret: percentage, reset time and labels only. Replayed to a window that connects later.
    pub quota: BTreeMap<(String, String, String), agora_protocol::v1::AgentQuotaObserved>,
}

/// ADR-0091: the most windows the hub keeps per machine (a node holds to 32; this is the hub's own bound). Past it the oldest
/// observation of that machine goes.
pub const MAX_QUOTA_PER_NODE: usize = 64;
/// The longest tool or window label the hub keeps (a longer one is not a label a node would send).
const MAX_QUOTA_LABEL: usize = 64;

/// What a node may say about machines (threat model T19): `Ok(Some(id))` is the node a fact names, which must be the sender;
/// `Ok(None)` is a fact that names none; `Err(field)` is a fact only the hub writes (producer `hub`). Every body is listed, so a new
/// one has to be decided here.
pub(crate) fn named_node(body: &Body) -> Result<Option<&str>, &'static str> {
    Ok(Some(match body {
        // ADR-0070, ADR-0073, ADR-0053: presence, forgetting a machine and forgetting an agent (named by its hub-wide key, which
        // holds its node) are the hub's own facts.
        Body::NodeLink(_) => return Err("node_link"),
        Body::NodeForgotten(_) => return Err("node_forgotten"),
        Body::AgentForgotten(_) => return Err("agent_forgotten"),
        Body::NodeOnline(e) => &e.node_id,
        Body::NodeOffline(e) => &e.node_id,
        Body::NodeResources(e) => &e.node_id,
        // ADR-0091: names its machine, so a node cannot report another machine's limits.
        Body::AgentQuotaObserved(e) => &e.node_id,
        Body::CheckoutDiscovered(e) => &e.node_id,
        Body::AgentStarted(e) => &e.node_id,
        Body::TerminalOpened(e) => &e.node_id,
        Body::WorkspaceRegistered(e) => &e.node_id,
        Body::NodeTrustedDevices(_)
        | Body::WorktreeCreated(_)
        | Body::WorktreeRemoved(_)
        | Body::GitStatusChanged(_)
        | Body::WorkspaceStatusChanged(_)
        | Body::AgentWorkspaceBound(_)
        | Body::AgentStateChanged(_)
        | Body::AgentSessionReported(_)
        | Body::AgentUsageReported(_)
        | Body::AgentExited(_)
        | Body::AgentSessionOptions(_)
        // ADR-0090: stored and relayed like the options above.
        | Body::AgentSessionConfig(_)
        | Body::AgentCommands(_)
        | Body::AgentInterrupted(_)
        | Body::TerminalTitleChanged(_)
        | Body::TerminalClosed(_)
        | Body::ApprovalRequested(_)
        | Body::ApprovalResolved(_)
        | Body::MessagePosted(_)
        | Body::TaskChanged(_)
        | Body::LeaseGranted(_)
        | Body::LeaseReleased(_)
        | Body::HandoffCreated(_)
        | Body::AgentStartRefused(_)
        | Body::PreviewSuggested(_)
        | Body::PreviewChanged(_)
        | Body::FilesChanged(_) => return Ok(None),
    }))
}

/// Whether a stored event is one the session now refuses (T19): a node's record naming another node, or one only the hub writes. The
/// one rule for live ingestion and for history; `View::rebuild` and the replay barrier ask it, so they cannot drift. The hub's own
/// events (producer `hub`) are never forged: they may name any node.
pub(crate) fn is_forged(ev: &Event) -> bool {
    ev.producer_id != RESERVED_HUB_ID
        && match ev.body.as_ref().map(named_node) {
            Some(Err(_)) => true,
            Some(Ok(Some(node))) => node != ev.producer_id,
            _ => false,
        }
}

/// The hub-wide key of an entity a producer named with a local id.
pub fn qualified(producer: &str, local_id: &str) -> String {
    format!("{producer}/{local_id}")
}

/// Splits a hub-wide key into `(producer, local id)`.
pub fn split_key(key: &str) -> Option<(&str, &str)> {
    key.split_once('/')
}

impl View {
    /// ADR-0073: whether `ev` brings a forgotten machine back: its link opening (from the hub, not about a replaced connection) or its
    /// process starting.
    pub fn brings_back(&self, ev: &Event) -> bool {
        let node = match &ev.body {
            Some(Body::NodeLink(e)) if ev.producer_id == RESERVED_HUB_ID && e.up => {
                let replaced = e.reason == agora_protocol::v1::NodeLinkReason::Replaced as i32;
                match self.nodes.get(&e.node_id) {
                    Some(n) if !replaced && e.generation >= n.link_generation => e.node_id.as_str(),
                    _ => return false,
                }
            }
            // A node's start is about the node that sent it, whatever the body names.
            Some(Body::NodeOnline(_)) => ev.producer_id.as_str(),
            _ => return false,
        };
        self.nodes.get(node).is_some_and(|n| n.forgotten)
    }

    /// ADR-0073: a forgotten machine that connects again is listed again, with the agents it still runs. Agents that had closed
    /// before it was forgotten stay forgotten.
    fn remember_node(&mut self, node: &str) {
        let Some(n) = self.nodes.get_mut(node) else {
            return;
        };
        if !std::mem::take(&mut n.forgotten) {
            return;
        }
        for a in self
            .agents
            .values_mut()
            .filter(|a| a.node_id == node && a.running)
        {
            a.forgotten = false;
        }
    }

    /// ADR-0090: the last `AgentSessionConfig` and `AgentCommands` of every running agent, as the events a node sent them in (producer
    /// the node, the agent's local id), for a window that takes a snapshot: the snapshot has no field for them. Each replaces what the
    /// window has, so a window that applies one twice is no worse off. Not for forgotten machines, forgotten or ended agents.
    pub fn session_replay(&self) -> Vec<Event> {
        let shown = self.shown();
        let mut out = Vec::new();
        // ADR-0091: the plan-limit observations of the shown machines, as the nodes sent them (each replaces its window's).
        for q in shown.quota.values() {
            out.push(Event {
                producer_id: q.node_id.clone(),
                seq: 0,
                at_ms: 0,
                body: Some(Body::AgentQuotaObserved(q.clone())),
            });
        }
        for a in shown.agents.values().filter(|a| a.running && !a.forgotten) {
            let one = |body| Event {
                producer_id: a.node_id.clone(),
                seq: 0,
                at_ms: 0,
                body: Some(body),
            };
            if let Some(c) = &a.session.config {
                out.push(one(Body::AgentSessionConfig(c.clone())));
            }
            if let Some(c) = &a.session.commands {
                out.push(one(Body::AgentCommands(c.clone())));
            }
        }
        out
    }

    /// ADR-0073: the view clients are shown. A forgotten machine, and everything keyed under it (`<node>/...`: its folders, worktrees,
    /// terminals, approvals and agents), is left out. Borrowed as it is when no machine is forgotten.
    pub fn shown(&self) -> std::borrow::Cow<'_, View> {
        let hidden: BTreeSet<&str> = self
            .nodes
            .iter()
            .filter(|(_, n)| n.forgotten)
            .map(|(id, _)| id.as_str())
            .collect();
        if hidden.is_empty() {
            return std::borrow::Cow::Borrowed(self);
        }
        let mut v = self.clone();
        let keep = |key: &String| !split_key(key).is_some_and(|(node, _)| hidden.contains(node));
        v.nodes.retain(|id, _| !hidden.contains(id.as_str()));
        v.checkouts
            .retain(|k, c| keep(k) && !hidden.contains(c.0.as_str()));
        v.worktrees.retain(|k, _| keep(k));
        v.workspaces
            .retain(|k, w| keep(k) && !hidden.contains(w.node_id.as_str()));
        v.agent_workspaces.retain(|k, _| keep(k));
        v.terminals.retain(|k, _| keep(k));
        v.approvals.retain(|k, _| keep(k));
        v.agents.retain(|k, _| keep(k));
        v.trusted_devices
            .retain(|id, _| !hidden.contains(id.as_str()));
        v.quota
            .retain(|(node, _, _), _| !hidden.contains(node.as_str()));
        std::borrow::Cow::Owned(v)
    }

    pub fn apply(&mut self, ev: &Event) {
        let producer = ev.producer_id.as_str();
        let q = |id: &str| qualified(&ev.producer_id, id);
        let Some(body) = &ev.body else { return };
        // Any event from a producer proves its node is there.
        if let Some(n) = self.nodes.get_mut(&ev.producer_id) {
            n.last_event_ms = n.last_event_ms.max(ev.at_ms);
        }
        match body {
            Body::NodeTrustedDevices(e) => {
                self.trusted_devices
                    .insert(producer.to_string(), e.device_ids.clone());
            }
            Body::WorkspaceStatusChanged(e) => {
                if let Some(w) = self.workspaces.get_mut(&q(&e.workspace_id)) {
                    w.files_changed = e.files_changed;
                }
            }
            Body::WorkspaceRegistered(e) => {
                let mut w = e.clone();
                w.workspace_id = q(&e.workspace_id);
                w.node_id = producer.to_string();
                self.workspaces.insert(w.workspace_id.clone(), w);
            }
            Body::AgentWorkspaceBound(e) => {
                self.agent_workspaces
                    .insert(q(&e.agent_id), q(&e.workspace_id));
            }

            // The node id is the connection's, never the body's: a node speaks for itself only (T19), and a history stored before
            // the session refused such events is read the same way.
            Body::NodeOnline(e) => {
                // A node announces itself when its process starts, never when only the link to the hub comes back. A resync
                // announcement (ADR-0054) leads with it only when the hub missed that start (ADR-0055). So whatever it had running
                // before is gone (agents and their terminals die with the node) and its waiting approvals are lost.
                for a in self
                    .agents
                    .values_mut()
                    .filter(|a| a.node_id == producer && a.running)
                {
                    a.running = false;
                    a.state = AgentState::Done;
                }
                let prefix = format!("{producer}/");
                self.approvals.retain(|k, _| !k.starts_with(&prefix));
                self.terminals.retain(|k, _| !k.starts_with(&prefix));
                self.remember_node(producer);
                let n = self
                    .nodes
                    .entry(producer.to_string())
                    .or_insert_with(|| NodeView::unknown(true));
                n.hostname = e.hostname.clone();
                n.os = e.os.clone();
                n.build = e.build.clone();
                n.binary = e.binary.clone();
                n.online = true;
                n.last_event_ms = n.last_event_ms.max(ev.at_ms);
            }
            Body::NodeOffline(e) => {
                // The hub writes this for a node whose connection ended (for older clients, ADR-0070): then the body names that
                // node. From a node it is about the node itself, whatever the body says.
                let node = if producer == RESERVED_HUB_ID {
                    e.node_id.as_str()
                } else {
                    producer
                };
                if let Some(n) = self.nodes.get_mut(node) {
                    n.online = false;
                }
            }
            // ADR-0070: the hub's record of a node's connection. Only the hub writes it; one that claims to come from a node is
            // ignored (the session refuses it too). It never touches the node's agents: a link that goes down and up is not a
            // process start.
            Body::NodeLink(e) if producer == RESERVED_HUB_ID => {
                let n = match self.nodes.get_mut(&e.node_id) {
                    Some(n) => n,
                    // A link that opens before the node's first `NodeOnline` arrives: the node is known from here on.
                    None if e.up => self
                        .nodes
                        .entry(e.node_id.clone())
                        .or_insert_with(|| NodeView::unknown(true)),
                    None => return,
                };
                n.last_event_ms = n.last_event_ms.max(e.last_heard_ms);
                // About a connection older than the latest one that opened: stale. A replaced connection's end never makes the
                // node offline either, since the newer connection is up.
                let stale = e.generation < n.link_generation
                    || e.reason == agora_protocol::v1::NodeLinkReason::Replaced as i32;
                if !stale {
                    n.online = e.up;
                    if e.up {
                        n.link_generation = e.generation;
                    }
                }
                if e.up && !stale {
                    self.remember_node(&e.node_id);
                }
            }
            Body::NodeLink(_) => {}
            // ADR-0073: written by the hub only, for a machine that is offline.
            Body::NodeForgotten(e) if producer == RESERVED_HUB_ID => {
                if let Some(n) = self.nodes.get_mut(&e.node_id) {
                    n.forgotten = true;
                }
                for a in self.agents.values_mut().filter(|a| a.node_id == e.node_id) {
                    a.forgotten = true;
                }
            }
            Body::NodeForgotten(_) => {}
            Body::NodeResources(e) => {
                // A node's numbers are its own: the connection's identity decides, never the id in the body (T19).
                if let Some(n) = self.nodes.get_mut(producer) {
                    n.agents_running = e.agents_running;
                    n.terminals_open = e.terminals_open;
                    n.memory_used_bytes = e.memory_used_bytes;
                }
            }
            Body::CheckoutDiscovered(e) => {
                // The checkout is on the node that said so, whatever the body names (T19).
                self.checkouts.insert(
                    q(&e.checkout_id),
                    (producer.to_string(), e.project_id.clone(), e.path.clone()),
                );
            }
            Body::WorktreeCreated(e) => {
                self.worktrees.insert(
                    q(&e.worktree_id),
                    WorktreeView {
                        checkout_id: q(&e.checkout_id),
                        path: e.path.clone(),
                        branch: e.branch.clone(),
                        files_changed: 0,
                        insertions: 0,
                        deletions: 0,
                        head_commit: String::new(),
                    },
                );
            }
            Body::WorktreeRemoved(e) => {
                self.worktrees.remove(&q(&e.worktree_id));
            }
            Body::GitStatusChanged(e) => {
                if let Some(w) = self.worktrees.get_mut(&q(&e.worktree_id)) {
                    w.files_changed = e.files_changed;
                    w.insertions = e.insertions;
                    w.deletions = e.deletions;
                    w.head_commit = e.head_commit.clone();
                }
            }
            Body::AgentStarted(e) => {
                if !e.worktree_id.is_empty() {
                    self.agent_workspaces.remove(&q(&e.agent_id));
                }
                self.agents.insert(
                    q(&e.agent_id),
                    AgentView {
                        // The agent runs on the node that started it: the connection decides, not the body (T19).
                        node_id: producer.to_string(),
                        project_id: e.project_id.clone(),
                        worktree_id: q(&e.worktree_id),
                        kind: e.kind.clone(),
                        state: AgentState::Idle,
                        reported: false,
                        session_id: None,
                        running: true,
                        exit_code: None,
                        input_tokens: 0,
                        output_tokens: 0,
                        model: String::new(),
                        forgotten: false,
                        session: SessionView::default(),
                        parent_agent_id: if e.parent_agent_id.is_empty() {
                            String::new()
                        } else {
                            q(&e.parent_agent_id)
                        },
                        interrupted: None,
                        launch_options: e.launch_options.clone(),
                        skips_permissions: e.skips_permissions,
                    },
                );
            }
            // ADR-0060: kept with the closed agents ("finished"), with what Resume or Start again needs. An agent the hub never saw
            // (its store was reset) is listed from the event alone.
            Body::AgentInterrupted(e) => {
                let a = self
                    .agents
                    .entry(q(&e.agent_id))
                    .or_insert_with(|| AgentView {
                        node_id: producer.to_string(),
                        project_id: e.project_id.clone(),
                        worktree_id: q(""),
                        kind: e.kind.clone(),
                        state: AgentState::Done,
                        reported: false,
                        session_id: None,
                        running: false,
                        exit_code: None,
                        input_tokens: 0,
                        output_tokens: 0,
                        model: String::new(),
                        forgotten: false,
                        session: SessionView::default(),
                        parent_agent_id: if e.parent_agent_id.is_empty() {
                            String::new()
                        } else {
                            q(&e.parent_agent_id)
                        },
                        interrupted: None,
                        launch_options: Vec::new(),
                        skips_permissions: false,
                    });
                a.running = false;
                a.session.config = None;
                a.session.commands = None;
                a.state = AgentState::Done;
                a.interrupted = Some(InterruptedView {
                    resumable: e.resumable,
                    at_ms: e.interrupted_at_ms,
                    folder: e.folder.clone(),
                    still_running: e.still_running,
                });
            }
            Body::AgentSessionOptions(e) => {
                if let Some(a) = self.agents.get_mut(&q(&e.agent_id)) {
                    a.session.options = Some(e.clone());
                }
            }
            Body::AgentStateChanged(e) => {
                if let Some(a) = self.agents.get_mut(&q(&e.agent_id)) {
                    a.state = AgentState::try_from(e.state).unwrap_or(AgentState::Unspecified);
                    a.reported = e.reported;
                }
            }
            Body::AgentSessionReported(e) => {
                if let Some(a) = self.agents.get_mut(&q(&e.agent_id)) {
                    a.session_id = Some(e.session_id.clone());
                }
            }
            Body::AgentUsageReported(e) => {
                if let Some(a) = self.agents.get_mut(&q(&e.agent_id)) {
                    a.input_tokens = a.input_tokens.saturating_add(e.input_tokens);
                    a.output_tokens = a.output_tokens.saturating_add(e.output_tokens);
                    if !e.model.is_empty() {
                        a.model.clone_from(&e.model);
                    }
                    let s = &mut a.session;
                    if e.context_tokens > 0 || e.context_window > 0 {
                        s.context_tokens = e.context_tokens;
                        s.context_window = e.context_window;
                    }
                    if let Some(c) = e.cost_micros.filter(|_| !e.currency.is_empty()) {
                        if s.currency != e.currency {
                            s.currency.clone_from(&e.currency);
                            s.cost_micros = None;
                        }
                        s.cost_micros = Some(s.cost_micros.unwrap_or(0).saturating_add(c));
                    }
                }
            }
            // ADR-0053: the id is the hub-wide key (the hub made this event); history and usage stay. Written by the hub only: one that
            // claims to come from a node is ignored (the session refuses it too), as `NodeForgotten` is.
            Body::AgentForgotten(e) if producer == RESERVED_HUB_ID => {
                if let Some(a) = self.agents.get_mut(&e.agent_id) {
                    a.forgotten = true;
                    a.session.config = None;
                    a.session.commands = None;
                }
            }
            Body::AgentForgotten(_) => {}
            // ADR-0090: the last report of each is kept per agent (replaced, bounded) and replayed to a window that connects later.
            // ADR-0091: the newest observation per (machine, tool, window); bounded per machine, the oldest goes first.
            Body::AgentQuotaObserved(e) => {
                let fits = e.node_id == producer
                    && !e.tool.is_empty()
                    && !e.window.is_empty()
                    && e.tool.len() <= MAX_QUOTA_LABEL
                    && e.window.len() <= MAX_QUOTA_LABEL;
                // The receiving session already cut a time ahead of the hub's clock; this is the second, deterministic bound for
                // history: never more than the event's own time plus a little skew (when it has one).
                let mut e = e.clone();
                if ev.at_ms > 0 {
                    e.observed_at_ms = e
                        .observed_at_ms
                        .min(ev.at_ms.saturating_add(crate::session::QUOTA_SKEW_MS));
                }
                let key = (e.node_id.clone(), e.tool.clone(), e.window.clone());
                if fits
                    && self
                        .quota
                        .get(&key)
                        .is_none_or(|old| e.observed_at_ms >= old.observed_at_ms)
                {
                    let node = e.node_id.clone();
                    self.quota.insert(key, e);
                    // Keys sort by machine first, so one machine's windows are a slice, not a scan of every machine's.
                    let lo = (node.clone(), String::new(), String::new());
                    let mine = self
                        .quota
                        .range(lo..)
                        .take_while(|(k, _)| k.0 == node)
                        .map(|(k, v)| (v.observed_at_ms, k.clone()))
                        .collect::<Vec<_>>();
                    if mine.len() > MAX_QUOTA_PER_NODE {
                        if let Some((_, oldest)) = mine.into_iter().min() {
                            self.quota.remove(&oldest);
                        }
                    }
                }
            }
            Body::AgentSessionConfig(e) => {
                if let Some(a) = self.agents.get_mut(&q(&e.agent_id)) {
                    let fits = e.options.len() <= MAX_CONFIG_OPTIONS
                        && e.options.iter().all(|o| o.choices.len() <= MAX_CONFIG_CHOICES)
                        && e.encoded_len() <= MAX_CONFIG_BYTES;
                    a.session.config = fits.then(|| e.clone());
                }
            }
            Body::AgentCommands(e) => {
                if let Some(a) = self.agents.get_mut(&q(&e.agent_id)) {
                    let fits = e.commands.len() <= MAX_COMMANDS && e.encoded_len() <= MAX_CONFIG_BYTES;
                    // A name that repeats (ignoring a leading "/" and case) is one command: the first wins, so a window never gets two
                    // rows that would act as one.
                    a.session.commands = fits.then(|| {
                        let mut seen = BTreeSet::new();
                        let mut kept = e.clone();
                        kept.commands.retain(|c| {
                            let n = c.name.trim_start_matches('/').to_lowercase();
                            !n.is_empty() && seen.insert(n)
                        });
                        kept
                    });
                }
            }
            Body::AgentExited(e) => {
                if let Some(a) = self.agents.get_mut(&q(&e.agent_id)) {
                    a.session.config = None;
                    a.session.commands = None;
                    a.running = false;
                    a.exit_code = Some(e.exit_code);
                    a.state = AgentState::Done;
                }
            }
            Body::TerminalOpened(e) => {
                self.terminals.insert(
                    q(&e.terminal_id),
                    TerminalView {
                        node_id: producer.to_string(),
                        agent_id: q(&e.agent_id),
                        title: String::new(),
                    },
                );
            }
            Body::TerminalTitleChanged(e) => {
                if let Some(t) = self.terminals.get_mut(&q(&e.terminal_id)) {
                    t.title = e.title.clone();
                }
            }
            Body::TerminalClosed(e) => {
                self.terminals.remove(&q(&e.terminal_id));
            }
            Body::ApprovalRequested(e) => {
                self.approvals.insert(
                    q(&e.approval_id),
                    ApprovalView {
                        agent_id: q(&e.agent_id),
                        action: e.action.clone(),
                        detail: e.detail.clone(),
                        expires_at_ms: e.expires_at_ms,
                        requested_at_ms: ev.at_ms,
                    },
                );
            }
            Body::ApprovalResolved(e) => {
                self.approvals.remove(&q(&e.approval_id));
            }
            // Mesh and file events are not part of this view (the mesh broker and clients consume them).
            Body::TaskChanged(t) => {
                // The hub's own events (the shared board) already carry `<node>/<agent>`; a node's carry its local ids.
                let owner = match (
                    t.claimed_by_agent_id.is_empty(),
                    producer == RESERVED_HUB_ID,
                ) {
                    (true, _) => String::new(),
                    (false, true) => t.claimed_by_agent_id.clone(),
                    (false, false) => qualified(producer, &t.claimed_by_agent_id),
                };
                self.tasks.insert(
                    format!("{}/{}", t.project_id, t.task_id),
                    TaskView {
                        project_id: t.project_id.clone(),
                        title: t.title.clone(),
                        owner,
                        done: t.done,
                    },
                );
            }
            Body::LeaseGranted(l) => {
                let agent = if producer == RESERVED_HUB_ID {
                    l.agent_id.clone()
                } else {
                    qualified(producer, &l.agent_id)
                };
                self.leases.insert(
                    format!("{agent}\0{}", l.path),
                    (agent, l.path.clone(), l.expires_at_ms),
                );
            }
            Body::LeaseReleased(l) => {
                let agent = if producer == RESERVED_HUB_ID {
                    l.agent_id.clone()
                } else {
                    qualified(producer, &l.agent_id)
                };
                self.leases.remove(&format!("{agent}\0{}", l.path));
            }
            // ADR-0057: a refused start changes no view; it is in the store and the audit trail.
            Body::MessagePosted(_)
            | Body::HandoffCreated(_)
            | Body::FilesChanged(_)
            // ADR-0088: the preview events change no view yet (schema only).
            | Body::PreviewSuggested(_)
            | Body::PreviewChanged(_)
            | Body::AgentStartRefused(_) => {}
        }
    }

    /// Rebuild from the whole log. Returns the view and the last position folded in, so a caller can continue
    /// from there with [`EventStore::events_after`].
    pub fn rebuild(store: &impl EventStore) -> Result<(Self, u64), StoreError> {
        let mut view = Self::default();
        let mut pos = 0;
        let mut forged = false;
        loop {
            let batch = store.events_after(pos, 1000)?;
            if batch.is_empty() {
                if forged {
                    view.distrusted_until = pos;
                }
                return Ok((view, pos));
            }
            for e in batch {
                pos = e.pos;
                // A payload that does not decode was stored by a newer build; skip it rather than fail.
                if let Ok(ev) = Event::decode(e.payload.as_slice()) {
                    forged |= is_forged(&ev);
                    if view.brings_back(&ev) {
                        view.remembered_at = pos;
                    }
                    view.apply(&ev);
                }
            }
        }
    }

    /// Agents that need a person: blocked, or with an unanswered approval. Input for the attention strip.
    pub fn needs_attention(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = self
            .agents
            .iter()
            .filter(|(id, a)| {
                a.running
                    && (a.state == AgentState::Blocked
                        || self.approvals.values().any(|p| p.agent_id == **id))
            })
            .map(|(id, _)| id.as_str())
            .collect();
        ids.sort_unstable();
        ids
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::store::SqliteStore;
    use agora_protocol::v1::*;

    /// ADR-0091: a node's plan-limit observation names its own machine, or it is forged; the view keeps nothing of it yet.
    #[test]
    fn a_quota_observation_naming_another_machine_is_forged() {
        let quota = |producer: &str, node: &str| Event {
            producer_id: producer.into(),
            seq: 1,
            body: Some(Body::AgentQuotaObserved(AgentQuotaObserved {
                node_id: node.into(),
                tool: "codex".into(),
                window: "five_hour".into(),
                used_percent: 10,
                ..AgentQuotaObserved::default()
            })),
            ..Event::default()
        };
        assert!(!is_forged(&quota("n1", "n1")));
        assert!(is_forged(&quota("n2", "n1")));
        let mut v = View::default();
        v.apply(&quota("n1", "n1"));
        assert!(v.agents.is_empty());
    }

    fn quota_ev(node: &str, tool: &str, window: &str, pct: u32, at: u64) -> Event {
        Event {
            producer_id: node.into(),
            seq: 1,
            body: Some(Body::AgentQuotaObserved(AgentQuotaObserved {
                node_id: node.into(),
                tool: tool.into(),
                window: window.into(),
                used_percent: pct,
                observed_at_ms: at,
                ..AgentQuotaObserved::default()
            })),
            ..Event::default()
        }
    }

    fn replayed_percent(v: &View) -> Vec<(String, String, u32)> {
        v.session_replay()
            .iter()
            .filter_map(|e| match &e.body {
                Some(Body::AgentQuotaObserved(q)) => {
                    Some((q.node_id.clone(), q.window.clone(), q.used_percent))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_newest_quota_observation_per_machine_tool_and_window_is_kept_and_replayed() {
        let mut v = View::default();
        v.apply(&quota_ev("n1", "codex", "five_hour", 10, 100));
        v.apply(&quota_ev("n1", "codex", "five_hour", 30, 200));
        // An older one (a late relay) does not replace the newer.
        v.apply(&quota_ev("n1", "codex", "five_hour", 5, 150));
        v.apply(&quota_ev("n1", "codex", "seven_day", 50, 100));
        v.apply(&quota_ev("n2", "codex", "five_hour", 90, 100));
        assert_eq!(
            replayed_percent(&v),
            [
                ("n1".to_string(), "five_hour".to_string(), 30),
                ("n1".to_string(), "seven_day".to_string(), 50),
                ("n2".to_string(), "five_hour".to_string(), 90),
            ]
        );
        // A forgotten machine is not replayed; its records stay for when it comes back.
        v.nodes.insert(
            "n2".into(),
            NodeView {
                forgotten: true,
                ..NodeView::unknown(false)
            },
        );
        assert_eq!(replayed_percent(&v).len(), 2);
        assert_eq!(v.quota.len(), 3);
    }

    #[test]
    fn a_future_dated_reading_is_cut_back_and_cannot_win_forever() {
        let at = |t: u64| {
            let mut e = quota_ev("n1", "codex", "five_hour", 10, t);
            e.at_ms = 1_000_000;
            e
        };
        // The session cuts a time ahead of the hub's clock; the stored event carries the cut.
        let future = at(1_000_000 + 86_400_000);
        let cut = crate::session::bound_quota_time(&future, 1_000_000).expect("cut");
        let back = <Event as prost::Message>::decode(cut.as_slice()).unwrap();
        match back.body {
            Some(Body::AgentQuotaObserved(q)) => assert_eq!(q.observed_at_ms, 1_000_000 + 120_000),
            other => panic!("{other:?}"),
        }
        assert!(crate::session::bound_quota_time(&at(1_000_000 + 60_000), 1_000_000).is_none());
        // History is bounded by the event's own time too: a day ahead counts as two minutes ahead.
        let mut v = View::default();
        v.apply(&future);
        assert_eq!(
            v.quota.values().next().unwrap().observed_at_ms,
            1_000_000 + 120_000
        );
        // A node whose clock was corrected backwards is read again: a later honest reading is newer than the cut one.
        let mut honest = quota_ev("n1", "codex", "five_hour", 70, 1_000_000 + 200_000);
        honest.at_ms = 1_000_000 + 200_000;
        v.apply(&honest);
        assert_eq!(v.quota.values().next().unwrap().used_percent, 70);
    }

    #[test]
    fn the_quota_table_is_bounded_per_machine_and_refuses_labels_that_are_not_labels() {
        let mut v = View::default();
        for i in 0..(MAX_QUOTA_PER_NODE as u64 + 10) {
            v.apply(&quota_ev(
                "n1",
                "codex",
                &format!("minutes:{i}"),
                1,
                1000 + i,
            ));
        }
        v.apply(&quota_ev("n2", "codex", "five_hour", 1, 1));
        let n1 = v.quota.keys().filter(|k| k.0 == "n1").count();
        assert_eq!(n1, MAX_QUOTA_PER_NODE);
        assert!(
            !v.quota
                .contains_key(&("n1".into(), "codex".into(), "minutes:0".into())),
            "the oldest went first"
        );
        assert!(
            v.quota
                .contains_key(&("n2".into(), "codex".into(), "five_hour".into())),
            "another machine is not squeezed"
        );
        v.apply(&quota_ev("n3", "codex", &"w".repeat(65), 1, 1));
        v.apply(&quota_ev("n3", "", "five_hour", 1, 1));
        assert!(v.quota.keys().all(|k| k.0 != "n3"));
        // A node speaking for another machine is not kept even if it got this far.
        v.apply(&quota_ev("n3", "codex", "five_hour", 1, 1));
        let mut forged = quota_ev("n3", "codex", "seven_day", 1, 1);
        forged.producer_id = "n4".into();
        v.apply(&forged);
        assert!(!v
            .quota
            .contains_key(&("n3".into(), "codex".into(), "seven_day".into())));
    }

    fn ev(producer: &str, seq: u64, body: Body) -> Event {
        Event {
            producer_id: producer.into(),
            seq,
            at_ms: seq * 10,
            body: Some(body),
        }
    }

    fn started(id: &str) -> Body {
        Body::AgentStarted(AgentStarted {
            agent_id: id.into(),
            project_id: "p".into(),
            node_id: "n1".into(),
            worktree_id: "w".into(),
            kind: "claude-code".into(),
            level: 2,
            parent_agent_id: String::new(),
            launch_options: Vec::new(),
            skips_permissions: false,
        })
    }

    fn state(id: &str, s: AgentState, reported: bool) -> Body {
        Body::AgentStateChanged(AgentStateChanged {
            agent_id: id.into(),
            state: s as i32,
            reported,
        })
    }

    /// Threat model T19: a node speaks for itself only. What a node's events say about another node's id changes nothing about that
    /// node: an agent, a terminal and a checkout are on the node that sent them, and resource numbers are the sender's own.
    #[test]
    fn a_node_cannot_place_things_on_or_change_the_numbers_of_another_node() {
        let online = |id: &str| {
            Body::NodeOnline(NodeOnline {
                node_id: id.into(),
                hostname: id.into(),
                os: "linux".into(),
                build: "b".into(),
                binary: String::new(),
            })
        };
        let mut v = View::default();
        v.apply(&ev("n1", 1, online("n1")));
        v.apply(&ev("n2", 1, online("n2")));
        // n2 says things about n1.
        v.apply(&ev(
            "n2",
            2,
            Body::NodeResources(NodeResources {
                node_id: "n1".into(),
                agents_running: 9,
                terminals_open: 9,
                memory_used_bytes: 9,
            }),
        ));
        assert_eq!(v.nodes["n1"].agents_running, 0, "n1's numbers are n1's own");
        v.apply(&ev("n2", 3, started("a"))); // body names n1
        assert_eq!(
            v.agents["n2/a"].node_id, "n2",
            "the agent runs on the node that started it"
        );
        v.apply(&ev(
            "n2",
            4,
            Body::TerminalOpened(TerminalOpened {
                terminal_id: "t".into(),
                agent_id: "a".into(),
                node_id: "n1".into(),
                ..TerminalOpened::default()
            }),
        ));
        assert_eq!(v.terminals["n2/t"].node_id, "n2");
        v.apply(&ev(
            "n2",
            5,
            Body::CheckoutDiscovered(CheckoutDiscovered {
                checkout_id: "c".into(),
                node_id: "n1".into(),
                project_id: "p".into(),
                path: "/x".into(),
            }),
        ));
        assert_eq!(v.checkouts["n2/c"].0, "n2");
        // The same events from n1 about itself still count.
        v.apply(&ev(
            "n1",
            2,
            Body::NodeResources(NodeResources {
                node_id: "n1".into(),
                agents_running: 2,
                ..NodeResources::default()
            }),
        ));
        assert_eq!(v.nodes["n1"].agents_running, 2);
    }

    /// The same, for the events that end or hide things: a history stored before the session refused them must not act on another node
    /// when it is read again (a hub restart replays the store).
    #[test]
    fn a_stored_forged_start_goodbye_or_forget_changes_nothing_on_another_node() {
        let online = |id: &str| {
            Body::NodeOnline(NodeOnline {
                node_id: id.into(),
                hostname: id.into(),
                os: "linux".into(),
                build: "b".into(),
                binary: String::new(),
            })
        };
        let mut v = View::default();
        v.apply(&ev("n1", 1, online("n1")));
        v.apply(&ev("n2", 1, online("n2")));
        v.apply(&ev("n1", 2, started("a")));
        assert!(v.agents["n1/a"].running);
        // n2 claims n1 started again: n1's agent is still running, and n2's own start counted for n2.
        v.apply(&ev("n2", 2, online("n1")));
        assert!(v.agents["n1/a"].running, "a forged start ended n1's agent");
        assert!(v.nodes["n1"].online);
        // n2 says goodbye for n1: n1 is still online; and for itself it counts.
        v.apply(&ev(
            "n2",
            3,
            Body::NodeOffline(NodeOffline {
                node_id: "n1".into(),
            }),
        ));
        assert!(v.nodes["n1"].online, "a forged goodbye took n1 offline");
        // n2 forgets n1's agent: only the hub may.
        let forget = |producer: &str, seq| {
            ev(
                producer,
                seq,
                Body::AgentForgotten(AgentForgotten {
                    agent_id: "n1/a".into(),
                }),
            )
        };
        v.apply(&forget("n2", 4));
        assert!(
            !v.agents["n1/a"].forgotten,
            "a node hid another node's agent"
        );
        v.apply(&forget(RESERVED_HUB_ID, 1));
        assert!(
            v.agents["n1/a"].forgotten,
            "the hub's own forget still works"
        );
        // A node's own goodbye still counts.
        v.apply(&ev(
            "n2",
            5,
            Body::NodeOffline(NodeOffline {
                node_id: "n2".into(),
            }),
        ));
        assert!(!v.nodes["n2"].online);
    }

    #[test]
    fn a_nodes_binary_off_path_is_kept_from_its_latest_start() {
        // ADR-0069: `NodeOnline.binary` is what the snapshot gives windows that connect later; a later start replaces it.
        let online = |binary: &str| {
            Body::NodeOnline(NodeOnline {
                node_id: "n1".into(),
                hostname: "laptop".into(),
                os: "windows".into(),
                build: "b".into(),
                binary: binary.into(),
            })
        };
        let mut v = View::default();
        v.apply(&ev("n1", 1, online(r"C:\Users\a\Downloads\agora.exe")));
        assert_eq!(v.nodes["n1"].binary, r"C:\Users\a\Downloads\agora.exe");
        let snap = crate::server::snapshot_of(&v, 1);
        assert_eq!(snap.nodes[0].binary, r"C:\Users\a\Downloads\agora.exe");
        v.apply(&ev("n1", 2, online("")));
        assert_eq!(v.nodes["n1"].binary, "");
    }

    #[test]
    fn an_interrupted_agent_stays_closed_until_it_starts_again_and_one_never_seen_is_listed() {
        let online = Body::NodeOnline(NodeOnline {
            node_id: "n1".into(),
            hostname: "laptop".into(),
            os: "linux".into(),
            build: "b".into(),
            binary: String::new(),
        });
        let interrupted = |id: &str, resumable: bool| {
            Body::AgentInterrupted(AgentInterrupted {
                agent_id: id.into(),
                kind: "claude-code".into(),
                project_id: "p".into(),
                folder: "/work/a".into(),
                parent_agent_id: String::new(),
                resumable,
                started_at_ms: 1,
                interrupted_at_ms: 99,
                last_state: AgentState::Working as i32,
                still_running: false,
            })
        };
        let mut v = View::default();
        v.apply(&ev("n1", 1, online.clone()));
        v.apply(&ev("n1", 2, started("a")));
        v.apply(&ev(
            "n1",
            3,
            Body::AgentUsageReported(AgentUsageReported {
                agent_id: "a".into(),
                input_tokens: 5,
                ..Default::default()
            }),
        ));
        // The node stops with the agent running and starts again.
        v.apply(&ev("n1", 4, online));
        v.apply(&ev("n1", 5, interrupted("a", true)));
        let a = &v.agents["n1/a"];
        assert!(!a.running);
        assert_eq!(a.state, AgentState::Done);
        assert_eq!(a.input_tokens, 5);
        assert_eq!(
            a.interrupted,
            Some(InterruptedView {
                resumable: true,
                at_ms: 99,
                folder: "/work/a".into(),
                still_running: false,
            })
        );
        // Said again by a resync: still one agent, still interrupted.
        v.apply(&ev("n1", 6, interrupted("a", true)));
        assert_eq!(v.agents.len(), 1);
        // Resumed: its start clears it.
        v.apply(&ev("n1", 7, started("a")));
        assert!(v.agents["n1/a"].running && v.agents["n1/a"].interrupted.is_none());
        // One the hub never saw start (its store was reset) is listed from the event alone.
        v.apply(&ev("n1", 8, interrupted("b", false)));
        let b = &v.agents["n1/b"];
        assert!(!b.running && b.interrupted.as_ref().is_some_and(|i| !i.resumable));
        assert_eq!(
            (b.kind.as_str(), b.project_id.as_str()),
            ("claude-code", "p")
        );
    }

    fn link(node: &str, up: bool, reason: NodeLinkReason, generation: u64, heard: u64) -> Body {
        Body::NodeLink(NodeLink {
            node_id: node.into(),
            up,
            reason: reason as i32,
            generation,
            last_heard_ms: heard,
        })
    }

    fn node_online(node: &str) -> Body {
        Body::NodeOnline(NodeOnline {
            node_id: node.into(),
            hostname: "h".into(),
            os: "linux".into(),
            build: "b".into(),
            binary: String::new(),
        })
    }

    /// ADR-0070: the hub's `NodeLink` decides `online`, a reconnect touches nothing the node runs, and "last seen" is when the hub
    /// last heard from it.
    #[test]
    fn a_link_that_goes_down_and_up_changes_presence_only() {
        let mut v = View::default();
        v.apply(&ev(
            "hub",
            1,
            link("n1", true, NodeLinkReason::Opened, 1, 5),
        ));
        v.apply(&ev("n1", 1, node_online("n1")));
        v.apply(&ev("n1", 2, started("a1")));
        assert!(v.nodes["n1"].online);
        v.apply(&ev(
            "hub",
            2,
            link("n1", false, NodeLinkReason::Silent, 1, 900),
        ));
        assert!(!v.nodes["n1"].online);
        assert_eq!(v.nodes["n1"].last_event_ms, 900);
        assert!(v.agents["n1/a1"].running, "offline is not finished");
        v.apply(&ev(
            "hub",
            3,
            link("n1", true, NodeLinkReason::Opened, 2, 2000),
        ));
        assert!(v.nodes["n1"].online);
        assert!(v.agents["n1/a1"].running, "a reconnect resets nothing");
        assert_eq!(v.nodes["n1"].link_generation, 2);
    }

    /// A late `up: false` about a connection that a newer one replaced never overrides the newer `up`.
    #[test]
    fn a_late_down_from_a_replaced_connection_is_ignored() {
        let mut v = View::default();
        v.apply(&ev(
            "hub",
            1,
            link("n1", true, NodeLinkReason::Opened, 1, 1),
        ));
        v.apply(&ev(
            "hub",
            2,
            link("n1", true, NodeLinkReason::Opened, 2, 2),
        ));
        v.apply(&ev(
            "hub",
            3,
            link("n1", false, NodeLinkReason::Closed, 1, 3),
        ));
        assert!(v.nodes["n1"].online, "stale generation");
        v.apply(&ev(
            "hub",
            4,
            link("n1", false, NodeLinkReason::Replaced, 2, 4),
        ));
        assert!(v.nodes["n1"].online, "replaced never means offline");
        v.apply(&ev(
            "hub",
            5,
            link("n1", false, NodeLinkReason::Closed, 2, 5),
        ));
        assert!(!v.nodes["n1"].online);
        // The time it was last heard still counts from any of them.
        assert_eq!(v.nodes["n1"].last_event_ms, 5);
    }

    /// Old data (an older hub wrote only `NodeOnline` / `NodeOffline`) and new facts mix: the latest presence fact by position wins.
    #[test]
    fn old_and_new_presence_facts_mix_and_the_latest_wins() {
        let mut v = View::default();
        // Old hub: online from the node's start, offline from the hub.
        v.apply(&ev("n1", 1, node_online("n1")));
        v.apply(&ev(
            "hub",
            1,
            Body::NodeOffline(NodeOffline {
                node_id: "n1".into(),
            }),
        ));
        assert!(!v.nodes["n1"].online);
        // The hub was upgraded: the next connection says up.
        v.apply(&ev(
            "hub",
            2,
            link("n1", true, NodeLinkReason::Opened, 1, 50),
        ));
        assert!(v.nodes["n1"].online);
        // A new hub also writes `NodeOffline` for older clients; it agrees with the `NodeLink` after it.
        v.apply(&ev(
            "hub",
            3,
            Body::NodeOffline(NodeOffline {
                node_id: "n1".into(),
            }),
        ));
        v.apply(&ev(
            "hub",
            4,
            link("n1", false, NodeLinkReason::Closed, 1, 60),
        ));
        assert!(!v.nodes["n1"].online);
        // A process start (the node's own `NodeOnline`) still says online, and resets what it ran (ADR-0055).
        v.apply(&ev("n1", 2, started("a1")));
        v.apply(&ev("n1", 3, node_online("n1")));
        assert!(v.nodes["n1"].online);
        assert!(!v.agents["n1/a1"].running);
    }

    /// Only the hub writes presence: a `NodeLink` that a node produced (the session refuses it too) changes nothing.
    #[test]
    fn a_node_cannot_claim_presence() {
        let mut v = View::default();
        v.apply(&ev(
            "evil",
            1,
            link("victim", true, NodeLinkReason::Opened, 9, 1),
        ));
        assert!(!v.nodes.contains_key("victim"));
        v.apply(&ev(
            "hub",
            1,
            link("victim", true, NodeLinkReason::Opened, 1, 1),
        ));
        v.apply(&ev(
            "evil",
            2,
            link("victim", false, NodeLinkReason::Closed, 1, 2),
        ));
        assert!(v.nodes["victim"].online);
    }

    #[test]
    fn node_lifecycle_and_resources() {
        let mut v = View::default();
        v.apply(&ev(
            "n1",
            1,
            Body::NodeOnline(NodeOnline {
                node_id: "n1".into(),
                hostname: "laptop".into(),
                os: "linux".into(),
                build: "b".into(),
                binary: String::new(),
            }),
        ));
        v.apply(&ev(
            "n1",
            2,
            Body::NodeResources(NodeResources {
                node_id: "n1".into(),
                agents_running: 2,
                terminals_open: 3,
                memory_used_bytes: 9,
            }),
        ));
        assert!(v.nodes["n1"].online);
        assert_eq!(v.nodes["n1"].agents_running, 2);
        assert_eq!(v.nodes["n1"].last_event_ms, 20);
        v.apply(&ev(
            "n1",
            3,
            Body::NodeOffline(NodeOffline {
                node_id: "n1".into(),
            }),
        ));
        assert!(!v.nodes["n1"].online);
    }

    #[test]
    fn a_node_that_starts_again_has_lost_its_agents_and_waiting_approvals() {
        let online = || {
            Body::NodeOnline(NodeOnline {
                node_id: "n1".into(),
                hostname: "laptop".into(),
                os: "linux".into(),
                build: "b".into(),
                binary: String::new(),
            })
        };
        let mut v = View::default();
        v.apply(&ev("n1", 1, online()));
        v.apply(&ev("n1", 2, started("a")));
        v.apply(&ev(
            "n1",
            3,
            Body::ApprovalRequested(ApprovalRequested {
                approval_id: "x".into(),
                agent_id: "a".into(),
                action: "run_command".into(),
                detail: String::new(),
                expires_at_ms: 0,
            }),
        ));
        assert_eq!(v.approvals.len(), 1);
        v.apply(&ev("n1", 4, online()));
        assert!(v.approvals.is_empty(), "the node forgot them");
        assert!(!v.agents["n1/a"].running);
        assert_eq!(v.agents["n1/a"].state, AgentState::Done);
    }

    /// ADR-0054: a node's resync announcement led by `NodeOnline` (a start the hub missed, ADR-0055) ends agents and terminals that
    /// went away inside the lost events, and keeps the ones still running with their current state, session, terminal and approval.
    #[test]
    fn a_resync_announcement_ends_what_went_away_and_keeps_what_still_runs() {
        let online = || {
            Body::NodeOnline(NodeOnline {
                node_id: "n1".into(),
                hostname: "laptop".into(),
                os: "linux".into(),
                build: "b".into(),
                binary: String::new(),
            })
        };
        let terminal = |agent: &str| {
            Body::TerminalOpened(TerminalOpened {
                terminal_id: format!("t-{agent}"),
                node_id: "n1".into(),
                purpose: TerminalPurpose::Agent as i32,
                agent_id: agent.into(),
            })
        };
        let approval = |id: &str, agent: &str| {
            Body::ApprovalRequested(ApprovalRequested {
                approval_id: id.into(),
                agent_id: agent.into(),
                action: "run_command".into(),
                detail: String::new(),
                expires_at_ms: 0,
            })
        };
        let mut v = View::default();
        // What the hub last heard: two agents with terminals, one waiting on an approval; another machine with its own.
        let before = [
            online(),
            started("gone"),
            terminal("gone"),
            approval("x", "gone"),
            started("kept"),
            terminal("kept"),
            state("kept", AgentState::Working, false),
        ];
        for (i, b) in before.into_iter().enumerate() {
            v.apply(&ev("n1", i as u64 + 1, b));
        }
        v.apply(&ev(
            "n2",
            1,
            Body::AgentStarted(AgentStarted {
                agent_id: "other".into(),
                node_id: "n2".into(),
                ..Default::default()
            }),
        ));
        v.apply(&ev("n2", 2, terminal("other")));
        // Lost: `gone` exited, `kept` went blocked and asked for an approval. The announcement says what is true now.
        let announcement = [
            online(),
            started("kept"),
            state("kept", AgentState::Blocked, false),
            Body::AgentSessionReported(AgentSessionReported {
                agent_id: "kept".into(),
                session_id: "s-1".into(),
            }),
            terminal("kept"),
            approval("y", "kept"),
        ];
        for (i, b) in announcement.into_iter().enumerate() {
            v.apply(&ev("n1", 50 + i as u64, b));
        }
        let gone = &v.agents["n1/gone"];
        assert!(!gone.running);
        assert_eq!(gone.state, AgentState::Done);
        let kept = &v.agents["n1/kept"];
        assert!(kept.running);
        assert_eq!(kept.state, AgentState::Blocked);
        assert_eq!(kept.session_id.as_deref(), Some("s-1"));
        assert_eq!(
            v.terminals.keys().collect::<Vec<_>>(),
            ["n1/t-kept", "n2/t-other"],
            "the ended agent's terminal is gone, the other machine's stays"
        );
        assert_eq!(v.approvals.keys().collect::<Vec<_>>(), ["n1/y"]);
        assert!(v.agents["n2/other"].running);
        assert_eq!(v.needs_attention(), ["n1/kept"]);
    }

    #[test]
    fn an_agents_options_context_and_own_cost_are_kept() {
        let mut v = View::default();
        v.apply(&ev("n1", 1, started("a")));
        let options = AgentSessionOptions {
            agent_id: "a".into(),
            modes: vec![SessionOption {
                id: "plan".into(),
                ..SessionOption::default()
            }],
            current_mode: "plan".into(),
            ..AgentSessionOptions::default()
        };
        v.apply(&ev("n1", 2, Body::AgentSessionOptions(options.clone())));
        let usage = |cost: Option<u64>, currency: &str, ctx: u64| {
            Body::AgentUsageReported(AgentUsageReported {
                agent_id: "a".into(),
                cost_micros: cost,
                currency: currency.into(),
                context_tokens: ctx,
                context_window: if ctx > 0 { 1000 } else { 0 },
                ..AgentUsageReported::default()
            })
        };
        v.apply(&ev("n1", 3, usage(Some(10), "USD", 100)));
        v.apply(&ev("n1", 4, usage(Some(5), "USD", 0)));
        let s = &v.agents["n1/a"].session;
        assert_eq!(s.options.as_ref(), Some(&options));
        assert_eq!(
            (s.context_tokens, s.context_window),
            (100, 1000),
            "a report without a level keeps the last one"
        );
        assert_eq!((s.cost_micros, s.currency.as_str()), (Some(15), "USD"));
        v.apply(&ev("n1", 5, usage(Some(7), "EUR", 0)));
        let s = &v.agents["n1/a"].session;
        assert_eq!(
            (s.cost_micros, s.currency.as_str()),
            (Some(7), "EUR"),
            "another currency starts again"
        );
    }

    #[test]
    fn the_last_config_and_commands_are_kept_bounded_and_dropped_with_the_agent() {
        let cfg = |n: usize| {
            Body::AgentSessionConfig(AgentSessionConfig {
                agent_id: "a".into(),
                options: (0..n)
                    .map(|i| SessionConfigOption {
                        id: format!("o{i}"),
                        ..SessionConfigOption::default()
                    })
                    .collect(),
            })
        };
        let cmds = |n: usize| {
            Body::AgentCommands(AgentCommands {
                agent_id: "a".into(),
                commands: (0..n)
                    .map(|i| AgentCommand {
                        name: format!("c{i}"),
                        ..AgentCommand::default()
                    })
                    .collect(),
            })
        };
        let mut v = View::default();
        v.apply(&ev("n1", 1, cfg(2))); // an agent the hub does not know: ignored
        assert!(v.session_replay().is_empty());
        v.apply(&ev("n1", 2, started("a")));
        v.apply(&ev("n1", 3, cfg(2)));
        v.apply(&ev("n1", 4, cfg(3)));
        v.apply(&ev("n1", 5, cmds(2)));
        let r = v.session_replay();
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].producer_id, "n1");
        match r[0].body.as_ref() {
            Some(Body::AgentSessionConfig(c)) => {
                assert_eq!(c.options.len(), 3, "the last report replaces")
            }
            other => panic!("{other:?}"),
        }
        // Over the caps: not kept, and what it replaced is gone too.
        v.apply(&ev("n1", 6, cfg(65)));
        v.apply(&ev("n1", 7, cmds(101)));
        assert!(v.session_replay().is_empty());
        v.apply(&ev("n1", 8, cfg(1)));
        // A repeated name (or one with a leading slash and another case) is kept once, the first.
        v.apply(&ev(
            "n1",
            9,
            Body::AgentCommands(AgentCommands {
                agent_id: "a".into(),
                commands: ["Dup", "/dup", "dup", "other"]
                    .iter()
                    .map(|n| AgentCommand {
                        name: (*n).into(),
                        ..AgentCommand::default()
                    })
                    .collect(),
            }),
        ));
        match v.session_replay().last().and_then(|e| e.body.as_ref()) {
            Some(Body::AgentCommands(c)) => assert_eq!(
                c.commands
                    .iter()
                    .map(|x| x.name.as_str())
                    .collect::<Vec<_>>(),
                ["Dup", "other"]
            ),
            other => panic!("{other:?}"),
        }
        v.apply(&ev("n1", 9, cmds(1)));
        assert_eq!(v.session_replay().len(), 2);
        v.apply(&ev(
            "n1",
            10,
            Body::AgentExited(AgentExited {
                agent_id: "a".into(),
                exit_code: 0,
            }),
        ));
        assert!(v.session_replay().is_empty(), "dropped when the agent ends");
        assert!(v.agents["n1/a"].session.config.is_none());
        // A new run of the id starts empty.
        v.apply(&ev("n1", 11, started("a")));
        v.apply(&ev("n1", 12, cfg(1)));
        assert_eq!(v.session_replay().len(), 1);
        v.apply(&ev("n1", 13, started("a")));
        assert!(v.session_replay().is_empty());
    }

    #[test]
    fn a_helper_names_its_parent_by_hub_wide_key() {
        let mut v = View::default();
        v.apply(&ev("n1", 1, started("a")));
        let mut helper = started("b");
        if let Body::AgentStarted(s) = &mut helper {
            s.parent_agent_id = "a".into();
        }
        v.apply(&ev("n1", 2, helper));
        assert_eq!(v.agents["n1/b"].parent_agent_id, "n1/a");
        assert_eq!(v.agents["n1/a"].parent_agent_id, "");
    }

    #[test]
    fn agent_lifecycle_tracks_state_usage_and_exit() {
        let mut v = View::default();
        v.apply(&ev("n1", 1, started("a")));
        v.apply(&ev("n1", 2, state("a", AgentState::Working, true)));
        assert_eq!(v.agents["n1/a"].state, AgentState::Working);
        assert!(v.agents["n1/a"].reported);
        for _ in 0..2 {
            v.apply(&ev(
                "n1",
                3,
                Body::AgentUsageReported(AgentUsageReported {
                    agent_id: "a".into(),
                    model: "m".into(),
                    input_tokens: 5,
                    output_tokens: 7,
                    ..Default::default()
                }),
            ));
        }
        assert_eq!(
            (
                v.agents["n1/a"].input_tokens,
                v.agents["n1/a"].output_tokens
            ),
            (10, 14)
        );
        v.apply(&ev(
            "n1",
            4,
            Body::AgentExited(AgentExited {
                agent_id: "a".into(),
                exit_code: 3,
            }),
        ));
        let a = &v.agents["n1/a"];
        assert!(!a.running);
        assert_eq!((a.exit_code, a.state), (Some(3), AgentState::Done));
    }

    #[test]
    fn events_for_unknown_entities_are_ignored_not_fatal() {
        let mut v = View::default();
        v.apply(&ev("n1", 1, state("ghost", AgentState::Blocked, false)));
        v.apply(&ev(
            "n1",
            2,
            Body::GitStatusChanged(GitStatusChanged {
                worktree_id: "nope".into(),
                ..Default::default()
            }),
        ));
        v.apply(&Event::default());
        assert_eq!(v, View::default());
    }

    #[test]
    fn attention_lists_blocked_agents_and_agents_with_pending_approvals() {
        let mut v = View::default();
        for id in ["a", "b", "c"] {
            v.apply(&ev("n1", 1, started(id)));
        }
        v.apply(&ev("n1", 2, state("a", AgentState::Blocked, false)));
        v.apply(&ev(
            "n1",
            3,
            Body::ApprovalRequested(ApprovalRequested {
                approval_id: "x".into(),
                agent_id: "b".into(),
                action: "run_command".into(),
                detail: "rm".into(),
                expires_at_ms: 0,
            }),
        ));
        assert_eq!(v.needs_attention(), ["n1/a", "n1/b"]);
        v.apply(&ev(
            "n1",
            4,
            Body::ApprovalResolved(ApprovalResolved {
                approval_id: "x".into(),
                approved: true,
                signer_device_id: "d".into(),
                helper_grant: 0,
                helpers_may_start: false,
            }),
        ));
        assert_eq!(v.needs_attention(), ["n1/a"]);
    }

    #[test]
    fn terminals_and_worktrees_come_and_go() {
        let mut v = View::default();
        v.apply(&ev(
            "n1",
            1,
            Body::TerminalOpened(TerminalOpened {
                terminal_id: "t".into(),
                node_id: "n1".into(),
                purpose: 0,
                agent_id: "a".into(),
            }),
        ));
        v.apply(&ev(
            "n1",
            2,
            Body::TerminalTitleChanged(TerminalTitleChanged {
                terminal_id: "t".into(),
                title: "vim".into(),
            }),
        ));
        assert_eq!(v.terminals["n1/t"].title, "vim");
        v.apply(&ev(
            "n1",
            3,
            Body::TerminalClosed(TerminalClosed {
                terminal_id: "t".into(),
            }),
        ));
        assert!(v.terminals.is_empty());
        v.apply(&ev(
            "n1",
            4,
            Body::WorktreeCreated(WorktreeCreated {
                worktree_id: "w".into(),
                checkout_id: "c".into(),
                path: "/x".into(),
                branch: "agent/a".into(),
            }),
        ));
        v.apply(&ev(
            "n1",
            5,
            Body::GitStatusChanged(GitStatusChanged {
                worktree_id: "w".into(),
                files_changed: 2,
                insertions: 3,
                deletions: 1,
                head_commit: "abc".into(),
            }),
        ));
        assert_eq!(v.worktrees["n1/w"].files_changed, 2);
        v.apply(&ev(
            "n1",
            6,
            Body::WorktreeRemoved(WorktreeRemoved {
                worktree_id: "w".into(),
            }),
        ));
        assert!(v.worktrees.is_empty());
    }

    #[test]
    fn the_same_local_ids_on_two_machines_do_not_collide() {
        let mut v = View::default();
        for node in ["laptop", "server"] {
            v.apply(&ev(node, 1, started("agent-1")));
            v.apply(&ev(
                node,
                2,
                Body::WorktreeCreated(WorktreeCreated {
                    worktree_id: "w".into(),
                    checkout_id: "c".into(),
                    path: "/work".into(),
                    branch: "agent/x".into(),
                }),
            ));
        }
        v.apply(&ev(
            "laptop",
            3,
            state("agent-1", AgentState::Blocked, false),
        ));
        assert_eq!(v.agents.len(), 2);
        assert_eq!(v.worktrees.len(), 2);
        assert_eq!(v.agents["laptop/agent-1"].state, AgentState::Blocked);
        assert_eq!(v.agents["server/agent-1"].state, AgentState::Idle);
        assert_eq!(v.needs_attention(), ["laptop/agent-1"]);
        assert_eq!(split_key("laptop/agent-1"), Some(("laptop", "agent-1")));
    }

    #[test]
    fn rebuild_from_the_store_equals_applying_live() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mut live = View::default();
        let events = [
            ev("n1", 1, started("a")),
            ev("n1", 2, state("a", AgentState::Working, true)),
            // The machine's own start (the shared fixture names n1): no refused record, so the barrier stays down.
            ev("n2", 1, {
                let Body::AgentStarted(mut s) = started("b") else {
                    unreachable!()
                };
                s.node_id = "n2".into();
                Body::AgentStarted(s)
            }),
        ];
        for e in &events {
            store
                .append(&e.producer_id, e.seq, e.at_ms, &e.encode_to_vec())
                .unwrap();
            live.apply(e);
        }
        // A payload from the future that does not decode must not break the rebuild.
        store.append("n3", 1, 0, &[0xff, 0xff]).unwrap();
        let (rebuilt, pos) = View::rebuild(&store).unwrap();
        assert_eq!(rebuilt, live);
        assert_eq!(pos, 4);
    }

    #[test]
    fn board_events_fill_the_view_and_the_snapshot_and_a_rebuilt_view_has_them_too() {
        let mut v = View::default();
        let task = |id: &str, owner: &str, done: bool| {
            Body::TaskChanged(TaskChanged {
                task_id: id.into(),
                project_id: "p".into(),
                title: "t".into(),
                claimed_by_agent_id: owner.into(),
                done,
            })
        };
        // The hub's events carry `<node>/<agent>`; a node's own carry its local ids.
        v.apply(&ev("hub", 1, task("t-1", "alpha/a1", false)));
        v.apply(&ev("beta", 1, task("t-9", "b1", false)));
        v.apply(&ev("hub", 2, task("t-1", "alpha/a1", true)));
        v.apply(&ev(
            "hub",
            3,
            Body::LeaseGranted(LeaseGranted {
                agent_id: "alpha/a1".into(),
                path: "src/x".into(),
                expires_at_ms: 99,
            }),
        ));
        v.apply(&ev(
            "hub",
            4,
            Body::LeaseGranted(LeaseGranted {
                agent_id: "alpha/a1".into(),
                path: "src/y".into(),
                expires_at_ms: 99,
            }),
        ));
        v.apply(&ev(
            "hub",
            5,
            Body::LeaseReleased(LeaseReleased {
                agent_id: "alpha/a1".into(),
                path: "src/y".into(),
            }),
        ));
        assert_eq!(v.tasks["p/t-1"].owner, "alpha/a1");
        assert!(v.tasks["p/t-1"].done);
        assert_eq!(v.tasks["p/t-9"].owner, "beta/b1");
        let snap = crate::server::snapshot_of(&v, 7);
        assert_eq!(snap.tasks.len(), 2);
        assert_eq!(snap.leases.len(), 1);
        assert_eq!(snap.leases[0].path, "src/x");
        assert!(snap.tasks.iter().any(|t| t.task_id == "t-1" && t.done));
    }
}
