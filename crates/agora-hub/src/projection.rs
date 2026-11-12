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

use std::collections::BTreeMap;

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
    pub online: bool,
    pub agents_running: u32,
    pub terminals_open: u32,
    pub memory_used_bytes: u64,
    pub last_event_ms: u64,
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

#[derive(Debug, Clone, PartialEq, Eq)]
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
    pub fn apply(&mut self, ev: &Event) {
        let producer = ev.producer_id.as_str();
        let q = |id: &str| qualified(&ev.producer_id, id);
        let Some(body) = &ev.body else { return };
        // Any event from a producer proves its node is there.
        if let Some(n) = self.nodes.get_mut(&ev.producer_id) {
            n.last_event_ms = n.last_event_ms.max(ev.at_ms);
        }
        match body {
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

            Body::NodeOnline(e) => {
                // A node announces itself when its process starts, never when only the link to the hub comes back. So
                // whatever it had running before is gone (agents die with the node) and its waiting approvals are lost.
                for a in self
                    .agents
                    .values_mut()
                    .filter(|a| a.node_id == e.node_id && a.running)
                {
                    a.running = false;
                    a.state = AgentState::Done;
                }
                let prefix = format!("{}/", e.node_id);
                self.approvals.retain(|k, _| !k.starts_with(&prefix));
                let n = self.nodes.entry(e.node_id.clone()).or_insert(NodeView {
                    hostname: String::new(),
                    os: String::new(),
                    build: String::new(),
                    online: true,
                    agents_running: 0,
                    terminals_open: 0,
                    memory_used_bytes: 0,
                    last_event_ms: 0,
                });
                n.hostname = e.hostname.clone();
                n.os = e.os.clone();
                n.build = e.build.clone();
                n.online = true;
                n.last_event_ms = n.last_event_ms.max(ev.at_ms);
            }
            Body::NodeOffline(e) => {
                if let Some(n) = self.nodes.get_mut(&e.node_id) {
                    n.online = false;
                }
            }
            Body::NodeResources(e) => {
                if let Some(n) = self.nodes.get_mut(&e.node_id) {
                    n.agents_running = e.agents_running;
                    n.terminals_open = e.terminals_open;
                    n.memory_used_bytes = e.memory_used_bytes;
                }
            }
            Body::CheckoutDiscovered(e) => {
                self.checkouts.insert(
                    q(&e.checkout_id),
                    (e.node_id.clone(), e.project_id.clone(), e.path.clone()),
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
                        node_id: e.node_id.clone(),
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
                    },
                );
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
                }
            }
            // ADR-0053: the id is the hub-wide key (the hub made this event); history and usage stay.
            Body::AgentForgotten(e) => {
                if let Some(a) = self.agents.get_mut(&e.agent_id) {
                    a.forgotten = true;
                }
            }
            Body::AgentExited(e) => {
                if let Some(a) = self.agents.get_mut(&q(&e.agent_id)) {
                    a.running = false;
                    a.exit_code = Some(e.exit_code);
                    a.state = AgentState::Done;
                }
            }
            Body::TerminalOpened(e) => {
                self.terminals.insert(
                    q(&e.terminal_id),
                    TerminalView {
                        node_id: e.node_id.clone(),
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
            Body::MessagePosted(_) | Body::HandoffCreated(_) | Body::FilesChanged(_) => {}
        }
    }

    /// Rebuild from the whole log. Returns the view and the last position folded in, so a caller can continue
    /// from there with [`EventStore::events_after`].
    pub fn rebuild(store: &impl EventStore) -> Result<(Self, u64), StoreError> {
        let mut view = Self::default();
        let mut pos = 0;
        loop {
            let batch = store.events_after(pos, 1000)?;
            if batch.is_empty() {
                return Ok((view, pos));
            }
            for e in batch {
                pos = e.pos;
                // A payload that does not decode was stored by a newer build; skip it rather than fail.
                if let Ok(ev) = Event::decode(e.payload.as_slice()) {
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
        })
    }

    fn state(id: &str, s: AgentState, reported: bool) -> Body {
        Body::AgentStateChanged(AgentStateChanged {
            agent_id: id.into(),
            state: s as i32,
            reported,
        })
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
            ev("n2", 1, started("b")),
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
