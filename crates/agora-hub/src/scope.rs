//! What one member or viewer may see and do (ADR-0049): the projects they were granted. A `Scope` follows the event stream a client
//! receives, remembers which project each agent, worktree, terminal and approval belongs to, and so can say for every event and every
//! command whether it is theirs. It is pure (no I/O) so the rules are tested on plain event sequences.
//!
//! Anything it cannot place in a granted project is hidden and refused: a terminal that belongs to no agent, the shared board of a
//! project that spans machines, a command about a whole machine. The owner has no scope and sees everything.

use std::collections::{BTreeMap, BTreeSet};

use agora_protocol::v1::command_request::Command as C;
use agora_protocol::v1::event::Body;
use agora_protocol::v1::Event;

use crate::projection::{qualified, View};

#[derive(Debug, Clone, Default)]
pub struct Scope {
    /// Granted projects, `node/project`.
    projects: BTreeSet<String>,
    /// Hub-wide key of a checkout, worktree, agent, terminal or approval → the `node/project` it belongs to.
    checkouts: BTreeMap<String, String>,
    worktrees: BTreeMap<String, String>,
    agents: BTreeMap<String, String>,
    terminals: BTreeMap<String, String>,
    approvals: BTreeMap<String, String>,
}

impl Scope {
    /// The scope of `projects` over what the hub already knows.
    pub fn from_view(projects: BTreeSet<String>, view: &View) -> Self {
        let mut s = Self {
            projects,
            ..Self::default()
        };
        for (k, (node, project, _)) in &view.checkouts {
            s.checkouts.insert(k.clone(), qualified(node, project));
        }
        for (k, w) in &view.worktrees {
            if let Some(p) = s.checkouts.get(&w.checkout_id).cloned() {
                s.worktrees.insert(k.clone(), p);
            }
        }
        for (k, a) in &view.agents {
            s.agents
                .insert(k.clone(), qualified(&a.node_id, &a.project_id));
        }
        for (k, t) in &view.terminals {
            if let Some(p) = s.agents.get(&t.agent_id).cloned() {
                s.terminals.insert(k.clone(), p);
            }
        }
        for (k, a) in &view.approvals {
            if let Some(p) = s.agents.get(&a.agent_id).cloned() {
                s.approvals.insert(k.clone(), p);
            }
        }
        s
    }

    fn granted(&self, project: Option<&String>) -> bool {
        project.is_some_and(|p| self.projects.contains(p))
    }

    /// Does the machine host any granted project? Only then does the person see it.
    fn node_visible(&self, node: &str) -> bool {
        let prefix = format!("{node}/");
        self.projects.iter().any(|p| p.starts_with(&prefix))
    }

    /// The view this person gets: only what belongs to a granted project, and machines that host one.
    pub fn filter_view(&self, view: &View) -> View {
        let mut out = View::default();
        for (k, n) in &view.nodes {
            if self.node_visible(k) {
                out.nodes.insert(k.clone(), n.clone());
            }
        }
        for (k, c) in &view.checkouts {
            if self.granted(self.checkouts.get(k)) {
                out.checkouts.insert(k.clone(), c.clone());
            }
        }
        for (k, w) in &view.worktrees {
            if self.granted(self.worktrees.get(k)) {
                out.worktrees.insert(k.clone(), w.clone());
            }
        }
        for (k, a) in &view.agents {
            if self.granted(self.agents.get(k)) {
                out.agents.insert(k.clone(), a.clone());
            }
        }
        for (k, t) in &view.terminals {
            if self.granted(self.terminals.get(k)) {
                out.terminals.insert(k.clone(), t.clone());
            }
        }
        for (k, a) in &view.approvals {
            if self.granted(self.approvals.get(k)) {
                out.approvals.insert(k.clone(), a.clone());
            }
        }
        for (k, l) in &view.leases {
            if self.granted(self.agents.get(&l.0)) {
                out.leases.insert(k.clone(), l.clone());
            }
        }
        out
    }

    /// Takes one event of the stream in order, remembers what it teaches, and says whether this person may see it.
    pub fn admit(&mut self, ev: &Event) -> bool {
        let node = ev.producer_id.as_str();
        let q = |id: &str| qualified(node, id);
        let Some(body) = &ev.body else { return false };
        match body {
            Body::NodeOnline(e) => self.node_visible(&e.node_id),
            Body::NodeOffline(e) => self.node_visible(&e.node_id),
            Body::NodeResources(e) => self.node_visible(&e.node_id),
            Body::CheckoutDiscovered(e) => {
                let p = qualified(&e.node_id, &e.project_id);
                self.checkouts.insert(q(&e.checkout_id), p.clone());
                self.projects.contains(&p)
            }
            Body::WorktreeCreated(e) => {
                let p = self.checkouts.get(&q(&e.checkout_id)).cloned();
                if let Some(p) = &p {
                    self.worktrees.insert(q(&e.worktree_id), p.clone());
                }
                self.granted(p.as_ref())
            }
            Body::WorktreeRemoved(e) => {
                let p = self.worktrees.remove(&q(&e.worktree_id));
                self.granted(p.as_ref())
            }
            Body::GitStatusChanged(e) => self.granted(self.worktrees.get(&q(&e.worktree_id))),
            Body::FilesChanged(e) => self.granted(self.worktrees.get(&q(&e.worktree_id))),
            Body::AgentStarted(e) => {
                let p = qualified(&e.node_id, &e.project_id);
                self.agents.insert(q(&e.agent_id), p.clone());
                self.projects.contains(&p)
            }
            Body::AgentStateChanged(e) => self.granted(self.agents.get(&q(&e.agent_id))),
            Body::AgentSessionReported(e) => self.granted(self.agents.get(&q(&e.agent_id))),
            Body::AgentUsageReported(e) => self.granted(self.agents.get(&q(&e.agent_id))),
            Body::AgentExited(e) => self.granted(self.agents.get(&q(&e.agent_id))),
            Body::TerminalOpened(e) => {
                let p = self.agents.get(&q(&e.agent_id)).cloned();
                if let Some(p) = &p {
                    self.terminals.insert(q(&e.terminal_id), p.clone());
                }
                self.granted(p.as_ref())
            }
            Body::TerminalTitleChanged(e) => self.granted(self.terminals.get(&q(&e.terminal_id))),
            Body::TerminalClosed(e) => {
                let p = self.terminals.remove(&q(&e.terminal_id));
                self.granted(p.as_ref())
            }
            Body::ApprovalRequested(e) => {
                let p = self.agents.get(&q(&e.agent_id)).cloned();
                if let Some(p) = &p {
                    self.approvals.insert(q(&e.approval_id), p.clone());
                }
                self.granted(p.as_ref())
            }
            Body::ApprovalResolved(e) => {
                let p = self.approvals.remove(&q(&e.approval_id));
                self.granted(p.as_ref())
            }
            // What an agent says to another can be about anything: it is theirs only when the sender is theirs.
            Body::MessagePosted(e) => self.granted(self.agents.get(&q(&e.from_agent_id))),
            Body::HandoffCreated(e) => {
                self.granted(self.agents.get(&q(&e.from_agent_id)))
                    && self.granted(self.agents.get(&q(&e.to_agent_id)))
            }
            // Leases name hub-wide agent keys when the hub wrote them, local ones when a node did.
            Body::LeaseGranted(e) => self.lease_visible(node, &e.agent_id),
            Body::LeaseReleased(e) => self.lease_visible(node, &e.agent_id),
            // The shared board spans machines, so no single grant covers it.
            Body::TaskChanged(_) => false,
        }
    }

    fn lease_visible(&self, producer: &str, agent: &str) -> bool {
        let key = if producer == crate::ca::RESERVED_HUB_ID {
            agent.to_string()
        } else {
            qualified(producer, agent)
        };
        self.granted(self.agents.get(&key))
    }

    /// May this person send `command` to `node`? Only commands that name something in a granted project; everything about a whole
    /// machine, or that lists across machines, is the owner's.
    pub fn command_allowed(&self, node: &str, command: &C) -> bool {
        let q = |id: &str| qualified(node, id);
        match command {
            C::AgentStart(c) => self.projects.contains(&qualified(node, &c.project_id)),
            C::AgentStop(c) => self.granted(self.agents.get(&q(&c.agent_id))),
            C::AgentPrompt(c) => self.granted(self.agents.get(&q(&c.agent_id))),
            C::AgentSetMode(c) => self.granted(self.agents.get(&q(&c.agent_id))),
            C::ChatSubscribe(c) => self.granted(self.agents.get(&q(&c.agent_id))),
            C::TerminalInput(c) => self.granted(self.terminals.get(&q(&c.terminal_id))),
            C::TerminalResize(c) => self.granted(self.terminals.get(&q(&c.terminal_id))),
            C::TerminalSubscribe(c) => self.granted(self.terminals.get(&q(&c.terminal_id))),
            C::FilesList(c) => self.granted(self.worktrees.get(&q(&c.worktree_id))),
            C::FilesRead(c) => self.granted(self.worktrees.get(&q(&c.worktree_id))),
            C::FilesWrite(c) => self.granted(self.worktrees.get(&q(&c.worktree_id))),
            C::FilesWatch(c) => self.granted(self.worktrees.get(&q(&c.worktree_id))),
            C::GitDiff(c) => self.granted(self.worktrees.get(&q(&c.worktree_id))),
            C::GitStatus(c) => self.granted(self.worktrees.get(&q(&c.worktree_id))),
            C::GitLog(c) => self.granted(self.worktrees.get(&q(&c.worktree_id))),
            C::WorktreeCreate(c) => self.granted(self.checkouts.get(&q(&c.checkout_id))),
            C::WorktreeRemove(c) => self.granted(self.worktrees.get(&q(&c.worktree_id))),
            C::ApprovalResolve(c) => self.granted(self.approvals.get(&q(&c.approval_id))),
            C::TaskUpdate(c) => self.projects.contains(&qualified(node, &c.project_id)),
            // What agents a machine can start is not private to a project, but only for machines the person works on.
            C::AgentKinds(_) => self.node_visible(node),
            _ => false,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use agora_protocol::v1::{self, command_request::Command};

    fn ev(node: &str, body: Body) -> Event {
        Event {
            producer_id: node.into(),
            seq: 1,
            at_ms: 1,
            body: Some(body),
        }
    }

    fn checkout(node: &str, id: &str, project: &str) -> Event {
        ev(
            node,
            Body::CheckoutDiscovered(v1::CheckoutDiscovered {
                checkout_id: id.into(),
                node_id: node.into(),
                project_id: project.into(),
                path: "/p".into(),
            }),
        )
    }

    fn agent(node: &str, id: &str, project: &str) -> Event {
        ev(
            node,
            Body::AgentStarted(v1::AgentStarted {
                agent_id: id.into(),
                project_id: project.into(),
                node_id: node.into(),
                worktree_id: String::new(),
                kind: "k".into(),
                level: 0,
            }),
        )
    }

    fn scope(granted: &[&str]) -> Scope {
        Scope::from_view(
            granted.iter().map(ToString::to_string).collect(),
            &View::default(),
        )
    }

    #[test]
    fn a_member_sees_what_belongs_to_a_granted_project_and_nothing_of_another() {
        let mut s = scope(&["n1/mine"]);
        let seen: Vec<bool> = [
            checkout("n1", "c1", "mine"),
            checkout("n1", "c2", "theirs"),
            agent("n1", "a1", "mine"),
            agent("n1", "a2", "theirs"),
            ev(
                "n1",
                Body::AgentStateChanged(v1::AgentStateChanged {
                    agent_id: "a1".into(),
                    state: 0,
                    reported: false,
                }),
            ),
            ev(
                "n1",
                Body::AgentStateChanged(v1::AgentStateChanged {
                    agent_id: "a2".into(),
                    state: 0,
                    reported: false,
                }),
            ),
            ev(
                "n1",
                Body::TerminalOpened(v1::TerminalOpened {
                    terminal_id: "t1".into(),
                    node_id: "n1".into(),
                    purpose: 0,
                    agent_id: "a1".into(),
                }),
            ),
            ev(
                "n1",
                Body::TerminalOpened(v1::TerminalOpened {
                    terminal_id: "t2".into(),
                    node_id: "n1".into(),
                    purpose: 0,
                    agent_id: "a2".into(),
                }),
            ),
            // A terminal that belongs to no agent is nobody's but the owner's.
            ev(
                "n1",
                Body::TerminalOpened(v1::TerminalOpened {
                    terminal_id: "t3".into(),
                    node_id: "n1".into(),
                    purpose: 0,
                    agent_id: String::new(),
                }),
            ),
            ev(
                "n1",
                Body::ApprovalRequested(v1::ApprovalRequested {
                    approval_id: "p1".into(),
                    agent_id: "a2".into(),
                    action: "x".into(),
                    detail: "secret".into(),
                    expires_at_ms: 0,
                }),
            ),
            ev(
                "n1",
                Body::MessagePosted(v1::MessagePosted {
                    message_id: "m".into(),
                    from_agent_id: "a2".into(),
                    to: "a1".into(),
                    body: "b".into(),
                }),
            ),
            // Another machine that hosts none of their projects does not even show up.
            ev(
                "n2",
                Body::NodeOnline(v1::NodeOnline {
                    node_id: "n2".into(),
                    hostname: "h".into(),
                    os: "o".into(),
                    build: "b".into(),
                }),
            ),
            ev(
                "n1",
                Body::NodeOnline(v1::NodeOnline {
                    node_id: "n1".into(),
                    hostname: "h".into(),
                    os: "o".into(),
                    build: "b".into(),
                }),
            ),
        ]
        .iter()
        .map(|e| s.admit(e))
        .collect();
        assert_eq!(
            seen,
            [
                true, false, true, false, true, false, true, false, false, false, false, false,
                true
            ]
        );
        // What a hidden thing closes stays hidden, and what is theirs is closed visibly.
        assert!(!s.admit(&ev(
            "n1",
            Body::TerminalClosed(v1::TerminalClosed {
                terminal_id: "t2".into()
            })
        )));
        assert!(s.admit(&ev(
            "n1",
            Body::TerminalClosed(v1::TerminalClosed {
                terminal_id: "t1".into()
            })
        )));
    }

    #[test]
    fn the_snapshot_is_cut_to_the_granted_projects_and_commands_follow_the_same_line() {
        let mut full = View::default();
        let mut s = scope(&["n1/mine"]);
        for e in [
            checkout("n1", "c1", "mine"),
            checkout("n1", "c2", "theirs"),
            agent("n1", "a1", "mine"),
            agent("n1", "a2", "theirs"),
            ev(
                "n1",
                Body::NodeOnline(v1::NodeOnline {
                    node_id: "n1".into(),
                    hostname: "h".into(),
                    os: "o".into(),
                    build: "b".into(),
                }),
            ),
        ] {
            full.apply(&e);
            s.admit(&e);
        }
        let cut = s.filter_view(&full);
        assert_eq!(cut.agents.keys().collect::<Vec<_>>(), ["n1/a1"]);
        assert_eq!(cut.checkouts.keys().collect::<Vec<_>>(), ["n1/c1"]);
        assert_eq!(cut.nodes.len(), 1);
        // The same cut, rebuilt from the full view instead of from the stream, is the same scope.
        let rebuilt = Scope::from_view(["n1/mine".to_string()].into(), &full);
        assert_eq!(rebuilt.filter_view(&full), cut);

        let stop = |id: &str| {
            Command::AgentStop(v1::AgentStop {
                agent_id: id.into(),
            })
        };
        assert!(s.command_allowed("n1", &stop("a1")));
        assert!(!s.command_allowed("n1", &stop("a2")));
        assert!(
            !s.command_allowed("n2", &stop("a1")),
            "the same local id on another machine is another agent"
        );
        let start = |p: &str| {
            Command::AgentStart(v1::AgentStart {
                project_id: p.into(),
                ..Default::default()
            })
        };
        assert!(
            s.command_allowed("n1", &start("mine")) && !s.command_allowed("n1", &start("theirs"))
        );
        // Anything about a whole machine is the owner's.
        assert!(!s.command_allowed("n1", &Command::NodePolicyGet(v1::NodePolicyGet::default())));
        assert!(!s.command_allowed("n1", &Command::ProjectList(v1::ProjectList::default())));
        assert!(!s.command_allowed("n1", &Command::TerminalOpen(v1::TerminalOpen::default())));
    }
}
