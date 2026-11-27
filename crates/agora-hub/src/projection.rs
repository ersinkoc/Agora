//! The hub's current view of the world, derived from the event log (docs/architecture.md: state is separate
//! from runtime). Pure data, no I/O: `apply` folds one event in, so the view can be rebuilt from the log at any
//! time and is identical however it was built. Clients get this as the snapshot, then live events.
//!
//! Ids are local to the machine that made them (`agent-1` exists on every node), so every entity is keyed by
//! `<producer>/<local id>` in this view (`laptop/agent-1`). Node ids are limited to `[A-Za-z0-9._-]` by the
//! certificate authority, so `/` never appears inside one and the hub can split a key to route a command back to
//! the right node. Project ids are machine-independent already (normalized remote + root commit) and stay as is, except that an
//! older node's id for a repository on a case-insensitive host is folded as a newer node now spells it (ADR-0115, [`fold_event`]).
//!
//! Unknown or out-of-order facts are tolerated (a worktree event for an unseen checkout still records the
//! worktree); the log is the truth, this is a cache.

use std::collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque};
use std::sync::Arc;

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
    pub(crate) run_id: Option<[u8; 16]>,
    pub(crate) started_seq: u64,
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
    /// ADR-0119 decision 10: a helper another machine asked for: its parent's machine and that machine's agent (empty otherwise).
    pub parent_node_id: String,
    pub parent_node_agent_id: String,
    /// ADR-0059: the launch options it was started with, and whether it acts without asking (the badge).
    pub launch_options: Vec<String>,
    pub skips_permissions: bool,
    /// ADR-0060: the node found this agent interrupted (running when the node stopped). Cleared by its next `AgentStarted`.
    pub interrupted: Option<InterruptedView>,
    /// ADR-0101: the "always" answers the agent's process holds, as `(approval id, action)`, newest last, at most
    /// [`MAX_ALWAYS_ALLOWED`]. Cleared when it ends, is interrupted or starts again.
    pub always_allowed: Vec<(String, String)>,
    /// ADR-0094: the kind of the program a person runs by hand in this plain terminal (`AgentRunsInside`); empty when none. Only a
    /// running agent has one (a node sends it for its `shell` agents); cleared by its next `AgentStarted`, its end, its interruption and its node's start.
    pub runs_inside: String,
}

/// ADR-0094: a kind id the hub keeps from `AgentRunsInside`: a manifest's id, at most 64 lowercase letters, digits, `-`, `_` or `.`.
fn runs_inside_kind(kind: &str) -> bool {
    !kind.is_empty()
        && kind.len() <= 64
        && kind.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_' | b'.')
        })
}

/// ADR-0101: how many "always" answers the hub lists per agent (the newest are kept).
pub const MAX_ALWAYS_ALLOWED: usize = 16;

/// #1150: how many decided approvals of one machine the view remembers (see [`View::resolved_approvals`]); as many as a node's
/// announcement restates (`agora_node::daemon::RESOLVED_KEPT`).
pub const RESOLVED_APPROVALS_KEPT: usize = 256;

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
    /// ADR-0101: `ApprovalRequested.scopes`, `always_label` and `always_option_id`, relayed as the node said them.
    pub scopes: Vec<i32>,
    pub always_label: String,
    pub always_option_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskView {
    pub(crate) board_revision: Option<u64>,
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
    // Shared with subscription snapshots. Delivery events are the only writers.
    pub(crate) handoff_deliveries:
        Arc<BTreeMap<String, (u64, agora_protocol::v1::TaskHandoffDeliveryChanged)>>,
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
    /// #1150: producer → the local ids of its last [`RESOLVED_APPROVALS_KEPT`] decided approvals, oldest first. A request a node
    /// records after its decision (a re-announcement that copied it before the decision was taken, or a request recorded after it
    /// was queued and decided) brings no card back. Folded from the log like everything else, so a rebuilt view holds the same ids;
    /// shared, so a clone of the view under the hub lock copies one pointer.
    pub resolved_approvals: Arc<BTreeMap<String, VecDeque<String>>>,
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
    /// ADR-0103: the newest mesh messages per project, for a window's snapshot. Shared, so a clone of the view under the hub lock copies
    /// one pointer; folded in by [`View::apply_at`] only (a message needs its log position).
    pub messages: Arc<MessageRings>,
    /// ADR-0115: `<producer>/<folded project id>` → the spelling that producer gave it, when that is not the folded one: an older
    /// node's current spelling (from its latest `CheckoutDiscovered` or `WorkspaceRegistered`, which also clears it once the node
    /// spells it folded), so a command about the project reaches that node in the spelling it knows ([`View::spelled_for`]); and an
    /// older hub's spellings of a shared board's project (each one its `TaskChanged` named, kept, keyed `<hub>/<folded>\0<spelling>`),
    /// so every board it stored is found ([`View::spellings_of`]). The view itself holds folded ids only.
    pub spellings: BTreeMap<String, String>,
}

/// ADR-0115: the project id an event carries, if it carries one.
fn project_id_of(body: &Body) -> Option<&String> {
    match body {
        Body::CheckoutDiscovered(e) => Some(&e.project_id),
        Body::AgentStarted(e) => Some(&e.project_id),
        Body::AgentInterrupted(e) => Some(&e.project_id),
        Body::TaskChanged(e) => Some(&e.project_id),
        Body::TaskDispatched(e) => Some(&e.project_id),
        Body::WorkspaceRegistered(e) => Some(&e.project_id),
        _ => None,
    }
}

/// ADR-0115: `ev` with its project id folded ([`agora_core::fold_project_id`]) when folding changes it: what an older node sends for a
/// repository on a case-insensitive host then names the project a newer node names. `None`: nothing to fold (most events, and every
/// event of a newer node), so the caller keeps the event it has. The stored event is never rewritten; this is applied as it is read.
pub fn fold_event(ev: &Event) -> Option<Event> {
    let id = project_id_of(ev.body.as_ref()?)?;
    let std::borrow::Cow::Owned(folded) = agora_core::fold_project_id(id) else {
        return None;
    };
    let mut ev = ev.clone();
    match &mut ev.body {
        Some(Body::CheckoutDiscovered(e)) => e.project_id = folded,
        Some(Body::AgentStarted(e)) => e.project_id = folded,
        Some(Body::AgentInterrupted(e)) => e.project_id = folded,
        Some(Body::TaskChanged(e)) => e.project_id = folded,
        Some(Body::TaskDispatched(e)) => e.project_id = folded,
        Some(Body::WorkspaceRegistered(e)) => e.project_id = folded,
        _ => return None,
    }
    Some(ev)
}

/// ADR-0103: the newest messages the hub keeps per project ring, and their most encoded bytes (whichever is smaller); past either the
/// oldest goes. A message larger than the byte cap alone is not kept. Hub constants, not policy (`docs/specs/protocol.md`).
pub const RING_MESSAGES: usize = 50;
pub const RING_BYTES: usize = 256 * 1024;
/// ADR-0103: the most encoded bytes over all rings; past it the oldest message of any ring goes.
pub const RINGS_TOTAL_BYTES: usize = 8 * 1024 * 1024;
/// ADR-0103: what one window's snapshot carries at most: the newest 50 it may see (the web's `MAX_MESSAGES`), then fewer until they
/// encode to at most 256 KiB together.
pub const WINDOW_MESSAGES: usize = 50;
pub const WINDOW_BYTES: usize = 256 * 1024;

/// ADR-0103: one stored `MessagePosted` as the hub keeps it for snapshots.
#[derive(Debug, Clone, PartialEq)]
pub struct KeptMessage {
    /// Its log position.
    pub pos: u64,
    /// The stored event, sent as it is.
    pub event: Event,
    /// What it adds to a `Snapshot` as one `recent_messages` element (tag, length and the `PositionedEvent`).
    pub size: usize,
    /// Placed by its sender's own agent (`<producer>/<from>` is an agent the hub knows): the copy the live rule (`Scope::admit`) gives
    /// a member of the sender's project. False for a copy placed by its recipient, which only the owner gets.
    pub by_sender: bool,
    /// The sender's hub-wide key: `from` when the node already qualified it, `<producer>/<from>` otherwise. For leaving out forgotten
    /// machines and agents only, never for placing.
    pub sender: String,
}

impl KeptMessage {
    fn new(pos: u64, ev: &Event, from: &str, by_sender: bool) -> Self {
        use prost::encoding::{encoded_len_varint, key_len};
        let event = ev.encoded_len();
        let positioned = if pos == 0 {
            0
        } else {
            key_len(1) + encoded_len_varint(pos)
        } + key_len(2)
            + encoded_len_varint(event as u64)
            + event;
        Self {
            pos,
            event: ev.clone(),
            size: key_len(12) + encoded_len_varint(positioned as u64) + positioned,
            by_sender,
            sender: if from.contains('/') {
                from.to_string()
            } else {
                qualified(&ev.producer_id, from)
            },
        }
    }
}

/// ADR-0103: the newest messages of one project (or of none), oldest first.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ring {
    pub entries: VecDeque<Arc<KeptMessage>>,
    pub bytes: usize,
}

/// ADR-0103: the hub's tail of mesh messages. A message goes to the ring of its sender's project when the sender is an agent of its
/// producer the hub knows, else to the ring of its recipient's project when that is one, else to `unplaced`, which only the owner reads.
/// The hub never resolves an already-qualified `from` to another machine's agent (T13). Copies are kept as stored; windows pair them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MessageRings {
    /// `<node>/<project>` (the key [`crate::scope::Scope`] grants) → its ring. A ring is removed when it empties.
    pub projects: BTreeMap<Arc<str>, Ring>,
    pub unplaced: Ring,
    /// Every kept message's position → its ring (`None`: `unplaced`), so the oldest over all rings is found without a scan. Keys are
    /// shared, so copying the rings (a message after a snapshot was taken) allocates no string.
    index: BTreeMap<u64, Option<Arc<str>>>,
    /// The encoded bytes over all rings.
    pub bytes: usize,
}

impl MessageRings {
    fn ring_mut(&mut self, key: Option<&Arc<str>>) -> Option<&mut Ring> {
        match key {
            Some(k) => self.projects.get_mut(k),
            None => Some(&mut self.unplaced),
        }
    }

    /// Drops the oldest message of the ring `key`; removes a project ring that empties.
    fn drop_oldest(&mut self, key: Option<&Arc<str>>) {
        let Some(ring) = self.ring_mut(key) else {
            return;
        };
        let Some(old) = ring.entries.pop_front() else {
            return;
        };
        ring.bytes -= old.size;
        let empty = ring.entries.is_empty();
        self.bytes -= old.size;
        self.index.remove(&old.pos);
        if let (Some(k), true) = (key, empty) {
            self.projects.remove(k);
        }
    }

    /// The positions of kept messages sent from a node in `hidden`.
    fn sent_from(&self, hidden: &BTreeSet<&str>) -> Vec<u64> {
        self.projects
            .values()
            .chain(std::iter::once(&self.unplaced))
            .flat_map(|ring| ring.entries.iter())
            .filter(|m| {
                hidden.contains(m.event.producer_id.as_str())
                    || split_key(&m.sender).is_some_and(|(n, _)| hidden.contains(n))
            })
            .map(|m| m.pos)
            .collect()
    }

    fn drop_positions(&mut self, drop_pos: Vec<u64>) {
        for pos in drop_pos {
            if let Some(key) = self.index.remove(&pos) {
                if let Some(ring) = self.ring_mut(key.as_ref()) {
                    if let Some(i) = ring.entries.iter().position(|m| m.pos == pos) {
                        if let Some(old) = ring.entries.remove(i) {
                            ring.bytes -= old.size;
                            self.bytes -= old.size;
                        }
                    }
                }
                if let Some(k) = key {
                    if self.projects.get(&k).is_some_and(|r| r.entries.is_empty()) {
                        self.projects.remove(&k);
                    }
                }
            }
        }
    }

    fn keep(&mut self, key: Option<String>, m: KeptMessage) {
        if m.size > RING_BYTES || self.index.contains_key(&m.pos) {
            return;
        }
        let (pos, size) = (m.pos, m.size);
        // The ring's own key when it has one, so every entry of a ring shares one string.
        let key: Option<Arc<str>> = key.map(|k| match self.projects.get_key_value(k.as_str()) {
            Some((k, _)) => Arc::clone(k),
            None => Arc::from(k),
        });
        let ring = match &key {
            Some(k) => self.projects.entry(Arc::clone(k)).or_default(),
            None => &mut self.unplaced,
        };
        ring.entries.push_back(Arc::new(m));
        ring.bytes += size;
        self.bytes += size;
        self.index.insert(pos, key.clone());
        while self
            .ring_mut(key.as_ref())
            .is_some_and(|r| r.entries.len() > RING_MESSAGES || r.bytes > RING_BYTES)
        {
            self.drop_oldest(key.as_ref());
        }
        while self.bytes > RINGS_TOTAL_BYTES {
            let Some((_, oldest)) = self.index.first_key_value() else {
                break;
            };
            let oldest = oldest.clone();
            self.drop_oldest(oldest.as_ref());
        }
    }

    /// A member's or viewer's rings: the granted projects' messages placed by their sender, the copies the live rule gives them.
    pub fn cut(&self, granted: impl Fn(&str) -> bool) -> Self {
        let mut out = Self::default();
        for (k, ring) in self.projects.iter().filter(|(k, _)| granted(k)) {
            let entries: VecDeque<_> = ring
                .entries
                .iter()
                .filter(|m| m.by_sender)
                .cloned()
                .collect();
            if entries.is_empty() {
                continue;
            }
            let bytes = entries.iter().map(|m| m.size).sum();
            for m in &entries {
                out.index.insert(m.pos, Some(Arc::clone(k)));
            }
            out.bytes += bytes;
            out.projects.insert(Arc::clone(k), Ring { entries, bytes });
        }
        out
    }
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
        Body::TaskHandoffDeliveryChanged(_) => return Err("task_handoff_delivery_changed"),
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
        // ADR-0101: names an agent of the producer.
        | Body::AgentAlwaysAllowed(_)
        // ADR-0094: names an agent of the producer.
        | Body::AgentRunsInside(_)
        | Body::TerminalTitleChanged(_)
        | Body::TerminalClosed(_)
        | Body::ApprovalRequested(_)
        | Body::ApprovalResolved(_)
        | Body::MessagePosted(_)
        | Body::TaskChanged(_)
        | Body::LeaseGranted(_)
        | Body::LeaseReleased(_)
        | Body::HandoffCreated(_)
        // ADR-0095: names an agent of the producer; `by_user` is the node's copy of what the hub wrote into the command (the hub's
        // own audit row of that command is the record of who asked).
        | Body::TaskDispatched(_)
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

impl View {
    /// ADR-0115: keeps the spelling `ev`'s producer gave its project, when it is not the folded one (see [`View::spellings`]).
    fn note_spelling(&mut self, ev: &Event) {
        let Some(body) = &ev.body else { return };
        let current = matches!(
            body,
            Body::CheckoutDiscovered(_) | Body::WorkspaceRegistered(_)
        ) && ev.producer_id != RESERVED_HUB_ID;
        let board = matches!(body, Body::TaskChanged(_)) && ev.producer_id == RESERVED_HUB_ID;
        if !current && !board {
            return;
        }
        let Some(id) = project_id_of(body) else {
            return;
        };
        let folded = agora_core::fold_project_id(id);
        let key = qualified(&ev.producer_id, &folded);
        if folded != id.as_str() {
            // F1821: an older hub stored a board per spelling it saw, so each of its spellings is kept (`<hub>/<folded>\0<spelling>`);
            // a node's is its latest only.
            let key = if board { format!("{key}\0{id}") } else { key };
            self.spellings.insert(key, id.clone());
        } else if current {
            self.spellings.remove(&key);
        }
    }

    /// ADR-0115: `project` as `node` spells it: the spelling it last reported for that project when an older node spelled it in
    /// another case; otherwise `project` itself.
    pub fn spelled_for<'a>(&'a self, node: &str, project: &'a str) -> &'a str {
        let folded = agora_core::fold_project_id(project);
        self.spellings
            .get(&qualified(node, &folded))
            .map_or(project, String::as_str)
    }

    /// ADR-0115: every other spelling a producer gave the (folded) `project`, in key order (by producer), for finding what was stored
    /// under an older spelling.
    pub fn spellings_of<'a>(&'a self, project: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.spellings
            .iter()
            .filter(move |(k, _)| {
                split_key(k)
                    .is_some_and(|(_, p)| p.split_once('\0').map_or(p, |(p, _)| p) == project)
            })
            .map(|(_, raw)| raw.as_str())
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
        // The rings are shared with the view; they are copied only when a forgotten node's message must go.
        let gone = v.messages.sent_from(&hidden);
        if !gone.is_empty() {
            Arc::make_mut(&mut v.messages).drop_positions(gone);
        }
        std::borrow::Cow::Owned(v)
    }

    /// #1150: whether `producer`'s approval `approval_id` was decided (one of its last [`RESOLVED_APPROVALS_KEPT`] decisions): a
    /// request for it stated now is late, and brings no card, notice or audit row.
    pub fn approval_decided(&self, producer: &str, approval_id: &str) -> bool {
        self.resolved_approvals
            .get(producer)
            .is_some_and(|ids| ids.iter().any(|id| id == approval_id))
    }

    /// [`View::apply`] for an event at log position `pos`, which also keeps a `MessagePosted` for snapshots (ADR-0103). The hub's
    /// publish and its rebuild from the log both come here, so a rebuilt view has the same tail as the live one.
    pub fn apply_at(&mut self, pos: u64, ev: &Event) {
        self.apply(ev);
        if let Some(Body::MessagePosted(m)) = &ev.body {
            self.keep_message(pos, ev, &m.from_agent_id, &m.to);
        }
    }

    /// ADR-0103: places one message by what its producer can vouch for, its own agents only: the sender's (`<producer>/<from>`), else
    /// the recipient's (`<producer>/<to>`), else none.
    fn keep_message(&mut self, pos: u64, ev: &Event, from: &str, to: &str) {
        let project = |agent: &str| {
            let key = qualified(&ev.producer_id, agent);
            self.agents
                .get(&key)
                .map(|a| qualified(&ev.producer_id, &a.project_id))
        };
        let (key, by_sender) = match project(from) {
            Some(p) => (Some(p), true),
            None => (project(to), false),
        };
        let m = KeptMessage::new(pos, ev, from, by_sender);
        Arc::make_mut(&mut self.messages).keep(key, m);
    }

    /// ADR-0103: the messages a window is sent with this view's snapshot: from every ring it holds (a member's cut view holds only
    /// theirs), without a sender under a forgotten machine, a producer that is a forgotten machine or a sender that is a forgotten
    /// agent; the newest [`WINDOW_MESSAGES`], then fewer until they come to at most [`WINDOW_BYTES`]; oldest first.
    pub fn recent_messages(&self) -> Vec<Arc<KeptMessage>> {
        let forgotten_node = |n: &str| self.nodes.get(n).is_some_and(|n| n.forgotten);
        let hidden = |m: &KeptMessage| {
            forgotten_node(&m.event.producer_id)
                || split_key(&m.sender).is_some_and(|(n, _)| forgotten_node(n))
                || self.agents.get(&m.sender).is_some_and(|a| a.forgotten)
        };
        let rings: Vec<&Ring> = self
            .messages
            .projects
            .values()
            .chain(std::iter::once(&self.messages.unplaced))
            .filter(|r| !r.entries.is_empty())
            .collect();
        // Newest first over all rings: each ring is in log order, so a merge from their ends.
        let mut next: Vec<usize> = rings.iter().map(|r| r.entries.len()).collect();
        let mut heads: BinaryHeap<(u64, usize)> = rings
            .iter()
            .enumerate()
            .filter_map(|(i, r)| r.entries.back().map(|m| (m.pos, i)))
            .collect();
        let (mut out, mut bytes) = (Vec::new(), 0usize);
        while let Some((_, i)) = heads.pop() {
            next[i] -= 1;
            let m = &rings[i].entries[next[i]];
            if next[i] > 0 {
                heads.push((rings[i].entries[next[i] - 1].pos, i));
            }
            if hidden(m) {
                continue;
            }
            if out.len() == WINDOW_MESSAGES || bytes + m.size > WINDOW_BYTES {
                break;
            }
            bytes += m.size;
            out.push(Arc::clone(m));
        }
        out.reverse();
        out
    }

    pub fn apply(&mut self, ev: &Event) {
        // ADR-0115: an older node's project id for a repository on a case-insensitive host is read folded, and its spelling kept.
        self.note_spelling(ev);
        let folded = fold_event(ev);
        let ev = folded.as_ref().unwrap_or(ev);
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
                    // ADR-0101: an always answer lives in the agent's process.
                    a.always_allowed.clear();
                    // ADR-0094: the node looks again at what runs inside a terminal it took up.
                    a.runs_inside.clear();
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
                // #1150: its decisions are not remembered for it (a bounded aid against late requests, not history).
                if self.resolved_approvals.contains_key(&e.node_id) {
                    Arc::make_mut(&mut self.resolved_approvals).remove(&e.node_id);
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
                // ADR-0104 (#930): the node re-states a process it announced before under this id (taken up from the holder after
                // its restart, or in a resync). What the hub summed for it stays: tokens, model, context level and its own cost. Only
                // for that node's own agent (the key is qualified by the producer); a new start, a resume, a restart after a crash and
                // every `AgentStarted` of an older node start again from 0. The rest starts again as for any start (the node
                // re-states options, config, commands and always answers).
                let (input_tokens, output_tokens, model, session) = match self
                    .agents
                    .get_mut(&q(&e.agent_id))
                    .filter(|a| e.same_process && a.node_id == producer)
                {
                    Some(a) => (
                        a.input_tokens,
                        a.output_tokens,
                        std::mem::take(&mut a.model),
                        SessionView {
                            context_tokens: a.session.context_tokens,
                            context_window: a.session.context_window,
                            cost_micros: a.session.cost_micros,
                            currency: std::mem::take(&mut a.session.currency),
                            ..SessionView::default()
                        },
                    ),
                    None => (0, 0, String::new(), SessionView::default()),
                };
                self.agents.insert(
                    q(&e.agent_id),
                    AgentView {
                        run_id: e.run_id.as_slice().try_into().ok(),
                        started_seq: ev.seq,
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
                        input_tokens,
                        output_tokens,
                        model,
                        forgotten: false,
                        session,
                        parent_agent_id: if e.parent_agent_id.is_empty() {
                            String::new()
                        } else {
                            q(&e.parent_agent_id)
                        },
                        parent_node_id: e.parent_node_id.clone(),
                        parent_node_agent_id: e.parent_node_agent_id.clone(),
                        interrupted: None,
                        launch_options: e.launch_options.clone(),
                        skips_permissions: e.skips_permissions,
                        always_allowed: Vec::new(),
                        runs_inside: String::new(),
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
                        run_id: None,
                        started_seq: 0,
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
                        parent_node_id: e.parent_node_id.clone(),
                        parent_node_agent_id: e.parent_node_agent_id.clone(),
                        interrupted: None,
                        launch_options: Vec::new(),
                        skips_permissions: false,
                        always_allowed: Vec::new(),
                        runs_inside: String::new(),
                    });
                a.running = false;
                a.always_allowed.clear();
                a.runs_inside.clear();
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
                    a.always_allowed.clear();
                    a.runs_inside.clear();
                }
            }
            // ADR-0094: what a running plain terminal runs inside now (the node sends it for its `shell` agents only). A kind that is
            // not a plain kind id is not kept.
            Body::AgentRunsInside(e) => {
                if let Some(a) = self
                    .agents
                    .get_mut(&q(&e.agent_id))
                    .filter(|a| a.running)
                {
                    if e.kind.is_empty() {
                        a.runs_inside.clear();
                    } else if runs_inside_kind(&e.kind) {
                        a.runs_inside = e.kind.clone();
                    }
                }
            }
            // ADR-0101: an agent that runs holds one more "always" answer (a repeat of one it holds, as a resync says it again, changes
            // nothing).
            Body::AgentAlwaysAllowed(e) => {
                if let Some(a) = self
                    .agents
                    .get_mut(&q(&e.agent_id))
                    .filter(|a| a.running)
                {
                    if !a.always_allowed.iter().any(|(id, _)| *id == e.approval_id) {
                        if a.always_allowed.len() == MAX_ALWAYS_ALLOWED {
                            a.always_allowed.remove(0);
                        }
                        a.always_allowed
                            .push((e.approval_id.clone(), e.action.clone()));
                    }
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
            Body::ApprovalRequested(e) if self.approval_decided(producer, &e.approval_id) => {}
            Body::ApprovalRequested(e) => {
                self.approvals.insert(
                    q(&e.approval_id),
                    ApprovalView {
                        agent_id: q(&e.agent_id),
                        action: e.action.clone(),
                        detail: e.detail.clone(),
                        expires_at_ms: e.expires_at_ms,
                        requested_at_ms: ev.at_ms,
                        scopes: e.scopes.clone(),
                        always_label: e.always_label.clone(),
                        always_option_id: e.always_option_id.clone(),
                    },
                );
            }
            Body::ApprovalResolved(e) => {
                self.approvals.remove(&q(&e.approval_id));
                let known = self.approval_decided(producer, &e.approval_id);
                // A decision the node says again (an announcement restates recent ones) is kept once, where it first came.
                if !known {
                    let ids = Arc::make_mut(&mut self.resolved_approvals)
                        .entry(producer.to_string())
                        .or_default();
                    if ids.len() == RESOLVED_APPROVALS_KEPT {
                        ids.pop_front();
                    }
                    ids.push_back(e.approval_id.clone());
                }
            }
            // Mesh and file events are not part of this view (the mesh broker and clients consume them).
            Body::TaskHandoffDeliveryChanged(status) if producer == RESERVED_HUB_ID => {
                let rows = Arc::make_mut(&mut self.handoff_deliveries);
                rows.insert(status.delivery_id.clone(), (ev.seq, status.clone()));
                if crate::handoff::is_settled(status) {
                    let settled = || rows.values().filter(|(_, status)| crate::handoff::is_settled(status));
                    if settled().nth(crate::handoff::MAX_SETTLED).is_some() {
                        let oldest = settled()
                            .min_by_key(|(seq, _)| *seq)
                            .map(|(_, status)| status.delivery_id.clone());
                        if let Some(id) = oldest {
                            rows.remove(&id);
                        }
                    }
                }
            }
            Body::TaskHandoffDeliveryChanged(_) => {}
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
                let key = format!("{}/{}", t.project_id, t.task_id);
                let board_revision = if producer == RESERVED_HUB_ID {
                    Some(ev.seq)
                } else {
                    self.tasks.get(&key).and_then(|task| task.board_revision)
                };
                self.tasks.insert(
                    key,
                    TaskView {
                        board_revision,
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
            // ADR-0057: a refused start changes no view; it is in the store and the audit trail. ADR-0103: a message is kept by
            // `apply_at`, which knows its log position.
            Body::MessagePosted(_)
            | Body::HandoffCreated(_)
            // ADR-0095: the audit record changes no view; the task's move is its `TaskChanged`.
            | Body::TaskDispatched(_)
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
                    view.apply_at(pos, &ev);
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
            run_id: Vec::new(),
            agent_id: id.into(),
            project_id: "p".into(),
            node_id: "n1".into(),
            worktree_id: "w".into(),
            kind: "claude-code".into(),
            level: 2,
            parent_agent_id: String::new(),
            same_process: false,
            launch_options: Vec::new(),
            skips_permissions: false,
            grant_id: String::new(),
            parent_node_id: String::new(),
            parent_node_agent_id: String::new(),
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

    /// ADR-0115: what a window is sent and the view reads: an older node's id for a GitHub repository folded, every other event and
    /// id as it is (no copy).
    #[test]
    fn an_older_nodes_project_id_is_read_folded_and_nothing_else_is_touched() {
        let root = "0123456789abcdef0123456789abcdef01234567";
        let cased = ev(
            "old",
            1,
            Body::TaskDispatched(agora_protocol::v1::TaskDispatched {
                project_id: format!("github.com/O/R#{root}"),
                task_id: "t".into(),
                agent_id: "a".into(),
                ..Default::default()
            }),
        );
        let folded = fold_event(&cased).unwrap();
        assert!(matches!(
            &folded.body,
            Some(Body::TaskDispatched(d)) if d.project_id == format!("github.com/o/r#{root}") && d.task_id == "t"
        ));
        assert_eq!((folded.producer_id.as_str(), folded.seq), ("old", 1));
        assert!(fold_event(&folded).is_none());
        assert!(fold_event(&ev("old", 2, started("a"))).is_none());
        assert!(fold_event(&ev("old", 2, state("a", AgentState::Working, true))).is_none());
        let other = ev(
            "old",
            3,
            Body::CheckoutDiscovered(agora_protocol::v1::CheckoutDiscovered {
                project_id: format!("git.example.org/O/R#{root}"),
                ..Default::default()
            }),
        );
        assert!(fold_event(&other).is_none());
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
                parent_node_id: String::new(),
                parent_node_agent_id: String::new(),
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
                ..Default::default()
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
                ..Default::default()
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

    /// ADR-0119 decision 10: a helper another machine asked for names its parent there, and no local parent.
    #[test]
    fn a_helper_from_another_machine_keeps_its_parents_machine() {
        let mut v = View::default();
        let mut s = match started("h") {
            Body::AgentStarted(s) => s,
            _ => unreachable!(),
        };
        s.parent_node_id = "mac".into();
        s.parent_node_agent_id = "agent-3".into();
        v.apply(&ev("n1", 1, Body::AgentStarted(s)));
        let a = &v.agents["n1/h"];
        assert_eq!(
            (
                a.parent_agent_id.as_str(),
                a.parent_node_id.as_str(),
                a.parent_node_agent_id.as_str()
            ),
            ("", "mac", "agent-3")
        );
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

    /// ADR-0104 (#930): what a kept agent had used, as the hub summed it before its node restarted.
    fn used_before(v: &mut View, producer: &str, id: &str) {
        v.apply(&ev(producer, 1, started_again(producer, id, false)));
        v.apply(&ev(
            producer,
            2,
            Body::AgentSessionOptions(AgentSessionOptions {
                agent_id: id.into(),
                current_mode: "plan".into(),
                ..AgentSessionOptions::default()
            }),
        ));
        v.apply(&ev(
            producer,
            3,
            Body::AgentUsageReported(AgentUsageReported {
                agent_id: id.into(),
                model: "m".into(),
                input_tokens: 5,
                output_tokens: 7,
                cost_micros: Some(10),
                currency: "USD".into(),
                context_tokens: 100,
                context_window: 1000,
            }),
        ));
    }

    fn started_again(producer: &str, id: &str, same_process: bool) -> Body {
        let Body::AgentStarted(mut s) = started(id) else {
            unreachable!()
        };
        s.node_id = producer.into();
        s.same_process = same_process;
        Body::AgentStarted(s)
    }

    type Counters<'a> = (u64, u64, &'a str, u64, u64, Option<u64>, &'a str);

    fn counters(a: &AgentView) -> Counters<'_> {
        let s = &a.session;
        (
            a.input_tokens,
            a.output_tokens,
            a.model.as_str(),
            s.context_tokens,
            s.context_window,
            s.cost_micros,
            s.currency.as_str(),
        )
    }

    const KEPT: Counters<'static> = (5, 7, "m", 100, 1000, Some(10), "USD");
    const ZERO: Counters<'static> = (0, 0, "", 0, 0, None, "");

    /// ADR-0104 decision 2 (#930): a node that takes a kept agent up after its restart says `NodeOnline`, then the agent's
    /// `AgentStarted` with `same_process`. The hub keeps its tokens, model, context level and own cost (the cost report reads the same
    /// numbers), and what the node re-states (options, config, commands, always answers) starts again as for any start. Usage reported
    /// after the take-up adds to the kept totals. A view rebuilt from the log (a hub restart) has the same totals.
    #[test]
    fn a_kept_agent_taken_up_after_its_node_restarts_keeps_its_counters() {
        let online = Body::NodeOnline(NodeOnline {
            node_id: "n1".into(),
            hostname: "n1".into(),
            os: "linux".into(),
            build: "b".into(),
            binary: String::new(),
        });
        let log = [
            ev("n1", 4, online),
            ev("n1", 5, started_again("n1", "a", true)),
            ev(
                "n1",
                6,
                Body::AgentUsageReported(AgentUsageReported {
                    agent_id: "a".into(),
                    input_tokens: 1,
                    output_tokens: 2,
                    cost_micros: Some(3),
                    currency: "USD".into(),
                    ..Default::default()
                }),
            ),
        ];
        let mut v = View::default();
        used_before(&mut v, "n1", "a");
        v.apply(&log[0]);
        assert!(!v.agents["n1/a"].running);
        v.apply(&log[1]);
        let a = &v.agents["n1/a"];
        assert!(a.running);
        assert_eq!(counters(a), KEPT);
        assert_eq!(a.session.options, None, "the node re-states its options");
        v.apply(&log[2]);
        assert_eq!(
            counters(&v.agents["n1/a"]),
            (6, 9, "m", 100, 1000, Some(13), "USD")
        );
        let mut rebuilt = View::default();
        used_before(&mut rebuilt, "n1", "a");
        for e in &log {
            rebuilt.apply(e);
        }
        assert_eq!(rebuilt.agents["n1/a"], v.agents["n1/a"]);
    }

    /// ADR-0104 decision 2: a resync re-states every running agent with `same_process`, and a hub that only missed events keeps their
    /// counters (no `NodeOnline` before it).
    #[test]
    fn a_resync_restatement_keeps_the_counters_of_an_agent_that_never_stopped() {
        let mut v = View::default();
        used_before(&mut v, "n1", "a");
        v.apply(&ev("n1", 9, started_again("n1", "a", true)));
        assert!(v.agents["n1/a"].running);
        assert_eq!(counters(&v.agents["n1/a"]), KEPT);
    }

    /// ADR-0104 decision 2: a new start, a resume or a restart after a crash of a reused id is a new process and starts again from 0,
    /// and so does every `AgentStarted` of a node that predates the field (absent reads as false). One that says `same_process`
    /// keeps only what the hub holds for that node's own agent: nothing for an id it never listed.
    #[test]
    fn a_new_process_under_a_reused_id_starts_its_counters_again() {
        let mut v = View::default();
        used_before(&mut v, "n1", "a");
        v.apply(&ev("n1", 4, started_again("n1", "a", false)));
        assert_eq!(counters(&v.agents["n1/a"]), ZERO);

        let mut v = View::default();
        v.apply(&ev("n1", 1, started_again("n1", "b", true)));
        assert_eq!(counters(&v.agents["n1/b"]), ZERO);
        assert!(v.agents["n1/b"].running);

        // Another node's agent of the same local id is another agent (T19): its counters are not taken.
        let mut v = View::default();
        used_before(&mut v, "n1", "a");
        v.apply(&ev("n2", 1, started_again("n2", "a", true)));
        assert_eq!(counters(&v.agents["n2/a"]), ZERO);
        assert_eq!(counters(&v.agents["n1/a"]), KEPT);
    }

    fn always(agent: &str, approval: &str, action: &str) -> Body {
        Body::AgentAlwaysAllowed(AgentAlwaysAllowed {
            agent_id: agent.into(),
            approval_id: approval.into(),
            action: action.into(),
            signer_device_id: "phone".into(),
            at_ms: 1,
        })
    }

    fn held(v: &View) -> Vec<String> {
        crate::server::snapshot_of(v, 0)
            .agents
            .into_iter()
            .find(|a| a.agent_id == "n1/a")
            .map(|a| a.always_allowed)
            .unwrap_or_default()
    }

    /// ADR-0101: the request's offer reaches the window's snapshot as the node said it.
    #[test]
    fn an_approvals_offer_is_relayed_into_the_snapshot() {
        let mut v = View::default();
        v.apply(&ev("n1", 1, started("a")));
        v.apply(&ev(
            "n1",
            2,
            Body::ApprovalRequested(ApprovalRequested {
                approval_id: "ap-1".into(),
                agent_id: "a".into(),
                action: "run_command".into(),
                detail: "git status".into(),
                expires_at_ms: 99,
                scopes: vec![ApprovalScope::Once as i32, ApprovalScope::Always as i32],
                always_label: "Always allow git".into(),
                always_option_id: "aa".into(),
            }),
        ));
        let snap = crate::server::snapshot_of(&v, 2);
        let a = &snap.approvals[0];
        assert_eq!(a.agent_id, "n1/a");
        assert_eq!(
            a.scopes,
            vec![ApprovalScope::Once as i32, ApprovalScope::Always as i32]
        );
        assert_eq!(
            (a.always_label.as_str(), a.always_option_id.as_str()),
            ("Always allow git", "aa")
        );
    }

    /// ADR-0094: what runs inside a plain terminal, from `AgentRunsInside`: kept for a running agent of the producer, only as a plain
    /// kind id; cleared by an empty kind, a start (the restated mark that follows brings it back), the end, an interruption and the
    /// node's start.
    #[test]
    fn what_runs_inside_a_terminal_is_kept_while_it_runs_and_cleared_with_its_process() {
        let inside = |agent: &str, kind: &str| {
            Body::AgentRunsInside(AgentRunsInside {
                agent_id: agent.into(),
                kind: kind.into(),
            })
        };
        let shown = |v: &View| {
            v.agents
                .get("n1/a")
                .map(|a| a.runs_inside.clone())
                .unwrap_or_default()
        };
        let mut v = View::default();
        v.apply(&ev("n1", 1, started("a")));
        v.apply(&ev("n1", 2, inside("a", "claude-code")));
        assert_eq!(shown(&v), "claude-code");
        // Not a plain kind id: not kept, the mark stays as it was.
        for bad in ["Claude", "claude code", "x\u{1b}[31m", &"k".repeat(65)] {
            v.apply(&ev("n1", 3, inside("a", bad)));
            assert_eq!(shown(&v), "claude-code", "{bad:?}");
        }
        // Another node cannot mark this agent.
        v.apply(&ev("n2", 4, inside("a", "codex")));
        assert_eq!(shown(&v), "claude-code");
        // An empty kind: the program ended.
        v.apply(&ev("n1", 5, inside("a", "")));
        assert_eq!(shown(&v), "");
        // A start clears; a restated mark brings it back.
        v.apply(&ev("n1", 6, inside("a", "codex")));
        v.apply(&ev("n1", 7, started("a")));
        assert_eq!(shown(&v), "");
        v.apply(&ev("n1", 8, inside("a", "codex")));
        assert_eq!(shown(&v), "codex");
        // The end clears, and an ended agent takes none.
        v.apply(&ev(
            "n1",
            9,
            Body::AgentExited(AgentExited {
                agent_id: "a".into(),
                exit_code: 0,
            }),
        ));
        assert_eq!(shown(&v), "");
        v.apply(&ev("n1", 10, inside("a", "codex")));
        assert_eq!(shown(&v), "");
        // The node's start clears.
        v.apply(&ev("n1", 11, started("a")));
        v.apply(&ev("n1", 12, inside("a", "codex")));
        v.apply(&ev("n1", 13, node_online("n1")));
        assert_eq!(shown(&v), "");
        // An interruption clears.
        v.apply(&ev("n1", 14, started("a")));
        v.apply(&ev("n1", 15, inside("a", "codex")));
        v.apply(&ev(
            "n1",
            16,
            Body::AgentInterrupted(AgentInterrupted {
                agent_id: "a".into(),
                ..Default::default()
            }),
        ));
        assert_eq!(shown(&v), "");
    }

    /// ADR-0101: an agent's always answers, from `AgentAlwaysAllowed` (a repeat of one changes nothing, at most 16, newest kept),
    /// cleared when it ends or starts again; a resync that says its start again and then its answers keeps them.
    #[test]
    fn an_agents_always_answers_are_derived_and_cleared_with_its_process() {
        let mut v = View::default();
        v.apply(&ev("n1", 1, started("a")));
        v.apply(&ev("n1", 2, always("a", "ap-1", "run_command")));
        v.apply(&ev("n1", 3, always("a", "ap-1", "run_command")));
        v.apply(&ev("n1", 4, always("a", "ap-2", "write_file")));
        assert_eq!(held(&v), ["run_command", "write_file"]);
        // Another node cannot add to this agent (its events name its own agents).
        v.apply(&ev("n2", 5, always("a", "ap-3", "rm -rf")));
        assert_eq!(held(&v), ["run_command", "write_file"]);
        // Bounded: the newest 16.
        for i in 0..20 {
            v.apply(&ev(
                "n1",
                10 + i,
                always("a", &format!("x-{i}"), &format!("act-{i}")),
            ));
        }
        let h = held(&v);
        assert_eq!(h.len(), MAX_ALWAYS_ALLOWED);
        assert_eq!(h.first().map(String::as_str), Some("act-4"));
        assert_eq!(h.last().map(String::as_str), Some("act-19"));
        // A start (a new process, or a resync's restatement) clears; the restated answers that follow bring them back.
        v.apply(&ev("n1", 40, started("a")));
        assert!(held(&v).is_empty());
        v.apply(&ev("n1", 41, always("a", "ap-1", "run_command")));
        assert_eq!(held(&v), ["run_command"]);
        // The end clears, and an ended agent takes none.
        v.apply(&ev(
            "n1",
            42,
            Body::AgentExited(AgentExited {
                agent_id: "a".into(),
                exit_code: 0,
            }),
        ));
        assert!(held(&v).is_empty());
        v.apply(&ev("n1", 43, always("a", "ap-9", "late")));
        assert!(held(&v).is_empty());
        // A node's start (its agents died with it) clears too.
        v.apply(&ev("n1", 47, started("a")));
        v.apply(&ev("n1", 48, always("a", "ap-1", "run_command")));
        v.apply(&ev("n1", 49, node_online("n1")));
        assert!(held(&v).is_empty());
        // An interruption clears too (a resumed agent's answers are not replayed).
        v.apply(&ev("n1", 44, started("a")));
        v.apply(&ev("n1", 45, always("a", "ap-1", "run_command")));
        v.apply(&ev(
            "n1",
            46,
            Body::AgentInterrupted(AgentInterrupted {
                agent_id: "a".into(),
                ..Default::default()
            }),
        ));
        assert!(held(&v).is_empty());
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
                ..Default::default()
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
                ..Default::default()
            }),
        ));
        assert_eq!(v.needs_attention(), ["n1/a"]);
    }

    fn asked(id: &str) -> Body {
        Body::ApprovalRequested(ApprovalRequested {
            approval_id: id.into(),
            agent_id: "a".into(),
            action: "run_command".into(),
            ..Default::default()
        })
    }

    fn decided(id: &str) -> Body {
        Body::ApprovalResolved(ApprovalResolved {
            approval_id: id.into(),
            approved: true,
            signer_device_id: "d".into(),
            ..Default::default()
        })
    }

    /// #1150 (reviews of #1216 and #1223): a node can record a request after its decision (a re-announcement that copied it before
    /// the decision, or a request recorded after it was queued and decided). Its card never comes back, live or rebuilt from the log;
    /// the same id from another machine is another request, and the ids remembered are bounded per machine.
    #[test]
    fn a_request_stated_after_its_decision_brings_no_card_back() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mut live = View::default();
        let mut events = vec![
            ev("n1", 1, started("a")),
            ev("n1", 2, asked("x")),
            ev("n1", 3, decided("x")),
            ev("n1", 4, asked("x")),
            ev("n2", 1, decided("y")),
            ev("n1", 5, asked("y")),
        ];
        // A decision the node says again (an announcement restates recent ones) is remembered once.
        let first = RESOLVED_APPROVALS_KEPT as u64;
        for i in 0..first {
            events.push(ev("n1", 6 + i, decided(&format!("r-{i}"))));
        }
        events.push(ev("n1", 6 + first, decided("r-0")));
        events.push(ev("n1", 7 + first, asked("r-0")));
        for e in &events {
            store
                .append(&e.producer_id, e.seq, e.at_ms, &e.encode_to_vec())
                .unwrap();
            live.apply(e);
        }
        assert_eq!(
            live.approvals.keys().collect::<Vec<_>>(),
            ["n1/y"],
            "only the request no decision of its machine answered"
        );
        assert_eq!(live.resolved_approvals["n1"].len(), RESOLVED_APPROVALS_KEPT);
        assert_eq!(live.resolved_approvals["n2"].len(), 1);
        let (rebuilt, _) = View::rebuild(&store).unwrap();
        assert_eq!(rebuilt, live);
        // Past the bound, the oldest decision is forgotten first (`x` went when `r-0`..`r-{N-1}` came).
        let mut v = live.clone();
        v.apply(&ev("n1", 8 + first, decided("one-more")));
        assert_eq!(v.resolved_approvals["n1"].len(), RESOLVED_APPROVALS_KEPT);
        v.apply(&ev("n1", 9 + first, asked("r-0")));
        assert!(v.approvals.contains_key("n1/r-0"), "r-0 was the oldest");
        v.apply(&ev("n1", 10 + first, asked("r-1")));
        assert!(!v.approvals.contains_key("n1/r-1"));
        // Review of #1234: a forgotten machine's decisions are not kept for it; only the hub can forget one.
        let forgot = |node: &str| {
            Body::NodeForgotten(NodeForgotten {
                node_id: node.into(),
                refused_return: false,
            })
        };
        v.apply(&ev("n1", 11 + first, forgot("n2")));
        assert!(v.resolved_approvals.contains_key("n2"));
        v.apply(&ev(RESERVED_HUB_ID, 1, forgot("n2")));
        assert!(!v.resolved_approvals.contains_key("n2"));
        assert!(v.resolved_approvals.contains_key("n1"));
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

    /// ADR-0103 test helpers: a view fed with positions, as the hub's publish and rebuild feed it.
    struct Fed {
        view: View,
        pos: u64,
    }

    impl Fed {
        fn new() -> Self {
            Self {
                view: View::default(),
                pos: 0,
            }
        }

        fn push(&mut self, e: Event) -> u64 {
            self.pos += 1;
            self.view.apply_at(self.pos, &e);
            self.pos
        }

        fn agent(&mut self, node: &str, id: &str, project: &str) {
            self.push(ev(
                node,
                1,
                Body::AgentStarted(AgentStarted {
                    agent_id: id.into(),
                    project_id: project.into(),
                    node_id: node.into(),
                    kind: "k".into(),
                    ..AgentStarted::default()
                }),
            ));
        }

        fn say(&mut self, node: &str, from: &str, to: &str, body: &str) -> u64 {
            self.push(message(node, from, to, body))
        }
    }

    fn message(node: &str, from: &str, to: &str, body: &str) -> Event {
        ev(
            node,
            7,
            Body::MessagePosted(MessagePosted {
                message_id: "m-1".into(),
                from_agent_id: from.into(),
                to: to.into(),
                body: body.into(),
            }),
        )
    }

    /// The window's messages as (position, body).
    fn sent(v: &View) -> Vec<(u64, String)> {
        crate::server::snapshot_of(v, u64::MAX)
            .recent_messages
            .into_iter()
            .map(|p| match p.event.and_then(|e| e.body) {
                Some(Body::MessagePosted(m)) => (p.pos, m.body),
                other => panic!("{other:?}"),
            })
            .collect()
    }

    fn ring_bodies(r: &Ring) -> Vec<(String, bool)> {
        r.entries
            .iter()
            .map(|m| match &m.event.body {
                Some(Body::MessagePosted(p)) => (p.body.clone(), m.by_sender),
                other => panic!("{other:?}"),
            })
            .collect()
    }

    /// ADR-0103 decision 2: a message is placed by what its producer can vouch for, its own agents: the sender's project, else the
    /// recipient's, else no project. An already-qualified `from` is never resolved to another machine's agent.
    #[test]
    fn a_message_goes_to_the_ring_its_producer_can_vouch_for() {
        let mut f = Fed::new();
        f.agent("n1", "a", "p");
        f.agent("n2", "b", "q");
        // The sender's own copy of a message between machines, and the receiver's.
        f.say("n1", "a", "n2/b", "to b");
        f.say("n2", "n1/a", "b", "to b");
        // A `#channel` copy the hub delivered: recorded by the receiver, `from` already qualified.
        f.say("n2", "n1/a", "b", "[#dev] all");
        // A node that names another machine's agent as the sender, to nobody it hosts: no project.
        f.say("n2", "n1/a", "nobody", "forged");
        // Neither end known: no project.
        f.say("n3", "x", "y", "strangers");
        let rings = &f.view.messages;
        assert_eq!(rings.projects.len(), 2, "{:?}", rings.projects.keys());
        assert_eq!(
            ring_bodies(&rings.projects["n1/p"]),
            [("to b".to_string(), true)]
        );
        assert_eq!(
            ring_bodies(&rings.projects["n2/q"]),
            [
                ("to b".to_string(), false),
                ("[#dev] all".to_string(), false)
            ]
        );
        assert_eq!(
            ring_bodies(&rings.unplaced),
            [
                ("forged".to_string(), false),
                ("strangers".to_string(), false)
            ]
        );
        // The owner is sent every copy, in log order; the window pairs them.
        assert_eq!(
            sent(&f.view)
                .iter()
                .map(|(_, b)| b.as_str())
                .collect::<Vec<_>>(),
            ["to b", "to b", "[#dev] all", "forged", "strangers"]
        );
        // `apply` alone (no position) keeps nothing.
        let mut plain = View::default();
        plain.apply(&message("n3", "x", "y", "no position"));
        assert!(plain.messages.unplaced.entries.is_empty());
    }

    /// ADR-0103: 51 messages in one project keep the newest 50, and a window is sent those, oldest first.
    #[test]
    fn a_ring_and_a_window_keep_the_newest_fifty_messages() {
        let mut f = Fed::new();
        f.agent("n1", "a", "p");
        let first = f.say("n1", "a", "b", "0");
        for i in 1..=50 {
            f.say("n1", "a", "b", &i.to_string());
        }
        let ring = &f.view.messages.projects["n1/p"];
        assert_eq!(ring.entries.len(), RING_MESSAGES);
        assert_ne!(ring.entries[0].pos, first);
        let got = sent(&f.view);
        assert_eq!(got.len(), WINDOW_MESSAGES);
        assert_eq!(got[0].1, "1");
        assert_eq!(got[49].1, "50");
        assert!(got.windows(2).all(|w| w[0].0 < w[1].0), "oldest first");
    }

    /// ADR-0103: a ring holds at most 256 KiB, a window is sent at most 256 KiB (as many of the newest as fit), and one message larger
    /// than a ring alone is not kept and pushes nothing out. 17 bodies of 16 KiB (a node's largest) are more than fit: with their
    /// envelopes 15 do, not 16.
    #[test]
    fn a_ring_and_a_window_keep_at_most_256_kib() {
        let mut f = Fed::new();
        f.agent("n1", "a", "p");
        for i in 0..17 {
            f.say(
                "n1",
                "a",
                "b",
                &format!("{i:02}{}", "x".repeat(16 * 1024 - 2)),
            );
        }
        let ring = &f.view.messages.projects["n1/p"];
        assert!(ring.bytes <= RING_BYTES);
        assert_eq!(
            ring.bytes,
            ring.entries.iter().map(|m| m.size).sum::<usize>()
        );
        assert_eq!(ring.entries.len(), 15);
        let snap = crate::server::snapshot_of(&f.view, f.pos);
        let without = agora_protocol::v1::Snapshot {
            recent_messages: Vec::new(),
            ..snap.clone()
        };
        let added = snap.encoded_len() - without.encoded_len();
        assert!(added <= WINDOW_BYTES, "{added}");
        assert_eq!(
            added, ring.bytes,
            "the sizes kept are what the snapshot carries"
        );
        assert_eq!(snap.recent_messages.len(), 15);
        assert!(matches!(
            snap.recent_messages[0].event.as_ref().and_then(|e| e.body.as_ref()),
            Some(Body::MessagePosted(m)) if m.body.starts_with("02")
        ));
        // Too large for a ring alone: dropped, and the ring is as it was.
        let before = ring.clone();
        f.say("n1", "a", "b", &"y".repeat(RING_BYTES));
        assert_eq!(f.view.messages.projects["n1/p"], before);
    }

    #[test]
    fn recent_team_chat_uses_only_the_remaining_snapshot_frame_budget() {
        use agora_protocol::frame::DEFAULT_MAX_PAYLOAD;
        use agora_protocol::v1::{ApprovalRequested, PositionedEvent};

        let mut f = Fed::new();
        for i in 0..500 {
            let node = format!("machine-{}", i / 50);
            let agent = format!("agent-{i}");
            f.agent(&node, &agent, &format!("project-{}", i / 10));
            if i < 450 {
                f.push(ev(
                    &node,
                    1,
                    Body::ApprovalRequested(ApprovalRequested {
                        agent_id: agent,
                        approval_id: format!("approval-{i}"),
                        action: "review".into(),
                        detail: "Example pending review detail. ".repeat(64),
                        expires_at_ms: 60_000,
                        ..Default::default()
                    }),
                ));
            }
        }
        let no_history = crate::server::snapshot_of(&f.view, f.pos);
        assert!(no_history.encoded_len() < DEFAULT_MAX_PAYLOAD);
        println!("CONTROL OK: fleet state fits without history");

        let body = "Example team update. ".repeat(560);
        for _ in 0..20 {
            f.say("machine-0", "agent-0", "agent-1", &body);
        }
        let eligible = f.view.recent_messages();
        assert_eq!(eligible.len(), 20);
        let snapshot = crate::server::snapshot_of(&f.view, f.pos);
        let actual = snapshot.encoded_len();
        if actual > DEFAULT_MAX_PAYLOAD {
            println!("PROBLEM CONFIRMED: expected <= {DEFAULT_MAX_PAYLOAD}, actual {actual}");
        }
        assert!(
            actual <= DEFAULT_MAX_PAYLOAD,
            "snapshot uses {actual} bytes"
        );
        let history = &snapshot.recent_messages;
        assert!(!history.is_empty());
        assert!(history.len() < eligible.len());
        let first = eligible.len() - history.len();
        assert!(history
            .iter()
            .zip(&eligible[first..])
            .all(|(a, b)| a.pos == b.pos));

        let mut one_more = snapshot.clone();
        let previous = &eligible[first - 1];
        one_more.recent_messages.insert(
            0,
            PositionedEvent {
                pos: previous.pos,
                event: Some(previous.event.clone()),
            },
        );
        assert!(one_more.encoded_len() > DEFAULT_MAX_PAYLOAD);
        assert_eq!(snapshot.agents, no_history.agents);
        assert_eq!(snapshot.approvals, no_history.approvals);

        let padding = DEFAULT_MAX_PAYLOAD - no_history.encoded_len() - 8;
        f.view
            .approvals
            .values_mut()
            .next()
            .expect("approval")
            .detail
            .push_str(&" ".repeat(padding));
        let almost_full = crate::server::snapshot_of(&f.view, f.pos);
        assert!(almost_full.encoded_len() <= DEFAULT_MAX_PAYLOAD);
        assert!(almost_full.recent_messages.is_empty());
    }

    /// ADR-0103: a busy project does not push a quiet one's messages out of its ring.
    #[test]
    fn a_busy_project_does_not_push_a_quiet_ones_messages_out() {
        let mut f = Fed::new();
        f.agent("n1", "a", "busy");
        f.agent("n1", "q", "quiet");
        f.say("n1", "q", "a", "still here");
        for i in 0..200 {
            f.say("n1", "a", "q", &i.to_string());
        }
        assert_eq!(
            ring_bodies(&f.view.messages.projects["n1/quiet"]),
            [("still here".to_string(), true)]
        );
        assert_eq!(f.view.messages.projects["n1/busy"].entries.len(), 50);
    }

    /// ADR-0103: all rings together hold at most 8 MiB; the oldest message of any ring goes first and a ring that empties is removed.
    #[test]
    fn all_rings_together_keep_at_most_8_mib() {
        let mut f = Fed::new();
        let body = "z".repeat(16 * 1024);
        for p in 0..40 {
            f.agent("n1", &format!("a{p}"), &format!("p{p:02}"));
        }
        for p in 0..40 {
            for _ in 0..15 {
                f.say("n1", &format!("a{p}"), "x", &body);
            }
        }
        let rings = &f.view.messages;
        assert!(rings.bytes <= RINGS_TOTAL_BYTES, "{}", rings.bytes);
        let sum: usize = rings.projects.values().map(|r| r.bytes).sum();
        assert_eq!(rings.bytes, sum + rings.unplaced.bytes);
        assert!(rings.bytes > RINGS_TOTAL_BYTES - RING_BYTES);
        // The first projects lost their messages, the last kept all of theirs.
        assert!(!rings.projects.contains_key("n1/p00"));
        assert_eq!(rings.projects["n1/p39"].entries.len(), 15);
        assert!(rings.projects.values().all(|r| !r.entries.is_empty()));
    }

    /// ADR-0103 decision 3 and ADR-0073, ADR-0053: the owner is sent no message whose sender is under a forgotten machine, recorded by
    /// a forgotten machine, or a forgotten agent. They are kept, so a machine that comes back has them again.
    #[test]
    fn forgotten_machines_and_agents_leave_their_messages_out_of_the_snapshot_not_the_ring() {
        let mut f = Fed::new();
        let online = |n: &str| {
            Body::NodeOnline(NodeOnline {
                node_id: n.into(),
                hostname: n.into(),
                ..NodeOnline::default()
            })
        };
        f.push(ev("n1", 1, online("n1")));
        f.push(ev("n2", 1, online("n2")));
        f.agent("n1", "a", "p");
        f.agent("n2", "b", "q");
        f.agent("n2", "gone", "q");
        f.say("n1", "a", "n2/b", "from n1");
        f.say("n2", "n1/a", "b", "copy on n2");
        f.say("n2", "b", "n1/a", "from n2");
        f.say("n2", "gone", "b", "from gone");
        assert_eq!(sent(&f.view).len(), 4);
        f.push(ev(
            "n2",
            2,
            Body::AgentExited(AgentExited {
                agent_id: "gone".into(),
                exit_code: 0,
            }),
        ));
        f.push(ev(
            RESERVED_HUB_ID,
            1,
            Body::AgentForgotten(AgentForgotten {
                agent_id: "n2/gone".into(),
            }),
        ));
        assert_eq!(
            sent(&f.view)
                .iter()
                .map(|(_, b)| b.as_str())
                .collect::<Vec<_>>(),
            ["from n1", "copy on n2", "from n2"]
        );
        // n1 goes offline and is forgotten: its sender's messages go, wherever they were recorded.
        f.push(ev(
            RESERVED_HUB_ID,
            2,
            Body::NodeOffline(NodeOffline {
                node_id: "n1".into(),
            }),
        ));
        f.push(ev(
            RESERVED_HUB_ID,
            3,
            Body::NodeForgotten(NodeForgotten {
                node_id: "n1".into(),
                ..NodeForgotten::default()
            }),
        ));
        assert_eq!(
            sent(&f.view)
                .iter()
                .map(|(_, b)| b.as_str())
                .collect::<Vec<_>>(),
            ["from n2"]
        );
        assert_eq!(f.view.messages.projects["n1/p"].entries.len(), 1);
        // Its link comes back with its agent still running: so do their messages.
        f.push(ev(
            RESERVED_HUB_ID,
            4,
            Body::NodeLink(NodeLink {
                node_id: "n1".into(),
                up: true,
                generation: 1,
                ..NodeLink::default()
            }),
        ));
        assert_eq!(sent(&f.view).len(), 3);
    }

    /// ADR-0103: a hub that rebuilds its view from the log at start has the same tail as the live one, and sends the same snapshot.
    #[test]
    fn a_rebuilt_view_has_the_same_messages_as_the_live_one() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mut f = Fed::new();
        let mut events = vec![
            ev("n1", 1, started("a")),
            ev("n2", 1, {
                let Body::AgentStarted(mut s) = started("b") else {
                    unreachable!()
                };
                s.node_id = "n2".into();
                s.project_id = "q".into();
                Body::AgentStarted(s)
            }),
        ];
        for i in 0..60 {
            events.push(message("n1", "a", "n2/b", &format!("ping {i}")));
            events.push(message("n2", "n1/a", "b", &format!("ping {i}")));
        }
        events.push(message("n3", "x", "y", "nobody's"));
        let mut seqs = BTreeMap::<String, u64>::new();
        for e in &mut events {
            let seq = seqs.entry(e.producer_id.clone()).or_default();
            *seq += 1;
            e.seq = *seq;
            let stored = store
                .append(&e.producer_id, e.seq, e.at_ms, &e.encode_to_vec())
                .unwrap();
            assert_eq!(stored, crate::store::Append::Applied { pos: f.pos + 1 });
            f.push(e.clone());
        }
        let (rebuilt, pos) = View::rebuild(&store).unwrap();
        assert_eq!(pos, f.pos);
        assert_eq!(rebuilt.messages, f.view.messages);
        assert_eq!(rebuilt, f.view);
        assert_eq!(
            crate::server::snapshot_of(&rebuilt, pos),
            crate::server::snapshot_of(&f.view, pos)
        );
        // The last event is a message: its position is the snapshot's own, and it is sent.
        let last = crate::server::snapshot_of(&rebuilt, pos);
        assert_eq!(last.recent_messages.last().map(|p| p.pos), Some(pos));
    }

    #[test]
    fn shown_view_recent_messages_excludes_forgotten_nodes() {
        let mut v = View::default();
        v.apply_at(
            1,
            &ev(
                RESERVED_HUB_ID,
                1,
                Body::NodeLink(NodeLink {
                    node_id: "n2".into(),
                    up: true,
                    generation: 1,
                    ..NodeLink::default()
                }),
            ),
        );
        v.apply_at(2, &message("n2", "a2", "all", "ok"));
        v.apply_at(
            3,
            &ev(
                RESERVED_HUB_ID,
                1,
                Body::NodeLink(NodeLink {
                    node_id: "n1".into(),
                    up: true,
                    generation: 1,
                    ..NodeLink::default()
                }),
            ),
        );
        v.apply_at(4, &message("n1", "a1", "all", "hidden"));
        v.apply_at(
            5,
            &ev(
                RESERVED_HUB_ID,
                2,
                Body::NodeOffline(NodeOffline {
                    node_id: "n1".into(),
                }),
            ),
        );
        v.apply_at(
            6,
            &ev(
                RESERVED_HUB_ID,
                3,
                Body::NodeForgotten(NodeForgotten {
                    node_id: "n1".into(),
                    ..NodeForgotten::default()
                }),
            ),
        );

        let shown = v.shown();
        let msgs = shown.recent_messages();
        assert!(msgs.iter().any(|m| m.event.producer_id == "n2"));
        assert!(!msgs.iter().any(|m| m.event.producer_id == "n1"));
    }
}
