//! What one member or viewer may see and do (ADR-0049): the projects they were granted. A `Scope` follows the event stream a client
//! receives, remembers which project each agent, worktree, terminal and approval belongs to, and so can say for every event and every
//! command whether it is theirs. It is pure (no I/O) so the rules are tested on plain event sequences.
//!
//! Anything it cannot place in a granted project is hidden and refused: a terminal that belongs to no agent, the shared board of a
//! project that spans machines, a command about a whole machine. The owner has no scope and sees everything.
//!
//! A member may also browse, choose and start in folders of a granted project on its machine (ADR-0049 Amendment A, #1049). The hub
//! cannot look at another machine's disk, so a folder is placed by its spelling against the folders the machines reported for each
//! project ([`Scope::command_allowed`]), and the folder the machine then really opened, links followed, is checked again in its answer
//! ([`Scope::confine_answer`]); a start needs no second look, since the machine starts only in the exact real folder it was asked for.

use std::collections::{BTreeMap, BTreeSet};

use agora_protocol::v1::command_request::Command as C;
use agora_protocol::v1::event::Body;
use agora_protocol::v1::{self, Event};

use crate::project_names::workspace_root;
use crate::projection::{qualified, split_key, View};

/// ADR-0069: what anyone but the owner is shown of a machine's own executable (`NodeOnline.binary`): its file name only. No folder is
/// kept, not even the one it is in: a program kept straight in its user's home (`/home/alice/agora`, `C:\Users\alice\agora.exe`)
/// would show the user name there (#598 cross-review), and the hub cannot tell which folder is a home on another machine.
pub fn short_binary(path: &str) -> String {
    path.split(['/', '\\'])
        .rfind(|p| !p.is_empty())
        .unwrap_or_default()
        .to_string()
}

/// [`short_binary`] applied to one event, when it carries a machine's executable; `None` when there is nothing to shorten.
pub fn without_full_binary(ev: &Event) -> Option<Event> {
    match &ev.body {
        Some(Body::NodeOnline(n)) if !n.binary.is_empty() => {
            let mut ev = ev.clone();
            if let Some(Body::NodeOnline(n)) = &mut ev.body {
                n.binary = short_binary(&n.binary);
            }
            Some(ev)
        }
        _ => None,
    }
}

/// A project's hub-wide key on a machine (`node/project`), its id folded (ADR-0115): an older node's spelling of a repository on a
/// case-insensitive host, in an event, a command or a grant made before, is the same project as the folded one the view holds.
fn pq(node: &str, project: &str) -> String {
    qualified(node, &agora_core::fold_project_id(project))
}

#[derive(Debug, Clone, Default)]
pub struct Scope {
    /// Granted projects, `node/project`.
    projects: BTreeSet<String>,
    /// Hub-wide key of a checkout, worktree, agent, terminal or approval → the `node/project` it belongs to.
    checkouts: BTreeMap<String, String>,
    worktrees: BTreeMap<String, String>,
    workspaces: BTreeMap<String, String>,
    agents: BTreeMap<String, String>,
    terminals: BTreeMap<String, String>,
    approvals: BTreeMap<String, String>,
    /// The folder each checkout (`checkout:<key>`) and workspace (`workspace:<key>`) was reported in, with its machine and project.
    folders: BTreeMap<String, Folder>,
}

/// A project's folder on one machine, as that machine reported it (ADR-0049 Amendment A).
#[derive(Debug, Clone)]
struct Folder {
    node: String,
    path: String,
    /// `node/project`.
    project: String,
}

/// The parts of an absolute folder path, spelled as `unix` or Windows paths are: split on `/` alone on Unix (where `\` is an
/// ordinary character of a name), on `/` and `\` on Windows, empty parts dropped (`/a//b/` is `/a/b`). `None` for a relative path or
/// one with a `.` or `..` part, so a path names a folder only by going down from the root; on Windows also for a part that ends in a
/// dot or a space (which Windows drops, so `mine.` would be `mine`) and for a `:` anywhere but a drive (an alternate data stream).
fn path_parts(path: &str, unix: bool) -> Option<Vec<&str>> {
    if path.contains('\0') {
        return None;
    }
    let b = path.as_bytes();
    let absolute = if unix {
        path.starts_with('/')
    } else {
        path.starts_with(r"\\")
            || path.starts_with("//")
            || (b.len() >= 3
                && b[0].is_ascii_alphabetic()
                && b[1] == b':'
                && (b[2] == b'\\' || b[2] == b'/'))
    };
    if !absolute {
        return None;
    }
    let parts: Vec<&str> = if unix {
        path.split('/').filter(|p| !p.is_empty()).collect()
    } else {
        path.split(['/', '\\']).filter(|p| !p.is_empty()).collect()
    };
    // `C:` first, or second after `\\?\`.
    let drive = |i: usize, p: &str| {
        i <= 1 && p.len() == 2 && p.as_bytes()[0].is_ascii_alphabetic() && p.ends_with(':')
    };
    let fine = |(i, p): (usize, &&str)| {
        !matches!(*p, "." | "..")
            && (unix || drive(i, p) || !(p.ends_with(['.', ' ']) || p.contains(':')))
    };
    parts.iter().enumerate().all(fine).then_some(parts)
}

impl Scope {
    /// The scope of `projects` over what the hub already knows.
    pub fn from_view(projects: BTreeSet<String>, view: &View) -> Self {
        // ADR-0115: a grant given in an older spelling is the folded project's.
        let projects = projects
            .into_iter()
            .map(|p| match split_key(&p) {
                Some((node, project)) => pq(node, project),
                None => p,
            })
            .collect();
        let mut s = Self {
            projects,
            ..Self::default()
        };
        // The machine is the one that reported it (the key's producer), never the node id inside the event.
        for (k, (_, project, path)) in &view.checkouts {
            if let Some((node, _)) = split_key(k) {
                s.checkouts.insert(k.clone(), pq(node, project));
                s.add_folder(format!("checkout:{k}"), node, project, path);
            }
        }
        for (k, w) in &view.worktrees {
            if let Some(p) = s.checkouts.get(&w.checkout_id).cloned() {
                s.worktrees.insert(k.clone(), p);
            }
        }
        for (k, w) in &view.workspaces {
            s.workspaces
                .insert(k.clone(), pq(&w.node_id, &w.project_id));
            s.add_folder(
                format!("workspace:{k}"),
                &w.node_id,
                &w.project_id,
                workspace_root(w),
            );
        }
        for (k, a) in &view.agents {
            if let Some((node, _)) = split_key(k) {
                s.agents.insert(k.clone(), pq(node, &a.project_id));
            }
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

    /// Remembers (or, for a known key, replaces) where a checkout or workspace is.
    fn add_folder(&mut self, key: String, node: &str, project: &str, path: &str) {
        self.folders.insert(
            key,
            Folder {
                node: node.into(),
                path: path.into(),
                project: pq(node, project),
            },
        );
    }

    /// The project `path` on `node` belongs to by the folders the machine reported: the project of the nearest reported folder that is
    /// `path` or holds it, as the machine itself places a folder for a device limited to some projects (`known_project`). `None` when
    /// no reported folder holds it, when two projects were reported in that nearest folder, or when the path is no plain absolute path
    /// in that machine's spelling ([`path_parts`]).
    fn project_of_folder(&self, node: &str, path: &str) -> Option<&str> {
        let holding: Vec<(usize, &str)> = self
            .folders
            .values()
            .filter(|f| f.node == node)
            .filter_map(|f| {
                let unix = f.path.starts_with('/');
                let root = path_parts(&f.path, unix)?;
                let at = path_parts(path, unix)?;
                (!root.is_empty() && at.starts_with(&root))
                    .then_some((root.len(), f.project.as_str()))
            })
            .collect();
        let nearest = holding.iter().map(|(depth, _)| *depth).max()?;
        let mut projects = holding
            .into_iter()
            .filter(|(depth, _)| *depth == nearest)
            .map(|(_, p)| p);
        let project = projects.next()?;
        projects.all(|other| other == project).then_some(project)
    }

    /// The granted project (`node/project`) that holds the folder `path` on `node`, if one does.
    fn granted_folder(&self, node: &str, path: &str) -> Option<&str> {
        self.project_of_folder(node, path)
            .filter(|p| self.projects.contains(*p))
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
                let mut n = n.clone();
                n.binary = short_binary(&n.binary);
                out.nodes.insert(k.clone(), n);
            }
        }
        for (k, d) in &view.trusted_devices {
            if self.node_visible(k) {
                out.trusted_devices.insert(k.clone(), d.clone());
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
        for (k, w) in &view.workspaces {
            if self.granted(self.workspaces.get(k)) {
                out.workspaces.insert(k.clone(), w.clone());
            }
        }
        for (agent, workspace) in &view.agent_workspaces {
            if self.granted(self.agents.get(agent)) && self.granted(self.workspaces.get(workspace))
            {
                out.agent_workspaces
                    .insert(agent.clone(), workspace.clone());
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
        // ADR-0103: the messages a member would have got live (`admit`): placed by their sender, in a granted project. A recipient's
        // copy and the messages placed in no project stay the owner's.
        out.messages = std::sync::Arc::new(view.messages.cut(|p| self.projects.contains(p)));
        out
    }

    /// Takes one event of the stream in order, remembers what it teaches, and says whether this person may see it.
    pub fn admit(&mut self, ev: &Event) -> bool {
        let node = ev.producer_id.as_str();
        let q = |id: &str| qualified(node, id);
        let Some(body) = &ev.body else { return false };
        match body {
            Body::WorkspaceStatusChanged(e) => {
                self.granted(self.workspaces.get(&q(&e.workspace_id)))
            }
            Body::WorkspaceRegistered(e) => {
                let p = pq(node, &e.project_id);
                self.workspaces.insert(q(&e.workspace_id), p.clone());
                self.add_folder(
                    format!("workspace:{}", q(&e.workspace_id)),
                    node,
                    &e.project_id,
                    workspace_root(e),
                );
                self.projects.contains(&p)
            }
            Body::AgentWorkspaceBound(e) => self.granted(self.workspaces.get(&q(&e.workspace_id))),

            Body::NodeOnline(e) => self.node_visible(&e.node_id),
            Body::NodeOffline(e) => self.node_visible(&e.node_id),
            Body::NodeLink(e) => self.node_visible(&e.node_id),
            Body::NodeForgotten(e) => self.node_visible(&e.node_id),
            Body::NodeResources(e) => self.node_visible(&e.node_id),
            // ADR-0091 (privacy): a plan limit is the whole machine's use, not a project's. A member of one shared project must not see
            // the owner's plan usage, so a scope (a member's window) never gets it; only an unscoped window does.
            Body::AgentQuotaObserved(_) => false,
            Body::NodeTrustedDevices(_) => self.node_visible(node),
            // A node speaks for itself only: the project is placed on the producer, whatever `node_id` the event names.
            Body::CheckoutDiscovered(e) => {
                let p = pq(node, &e.project_id);
                self.checkouts.insert(q(&e.checkout_id), p.clone());
                self.add_folder(
                    format!("checkout:{}", q(&e.checkout_id)),
                    node,
                    &e.project_id,
                    &e.path,
                );
                self.projects.contains(&p)
            }
            Body::WorktreeCreated(e) => {
                let p = self.checkouts.get(&q(&e.checkout_id)).cloned();
                if let Some(p) = &p {
                    self.worktrees.insert(q(&e.worktree_id), p.clone());
                } else {
                    self.worktrees.remove(&q(&e.worktree_id));
                }
                self.granted(p.as_ref())
            }
            Body::WorktreeRemoved(e) => {
                let p = self.worktrees.remove(&q(&e.worktree_id));
                self.granted(p.as_ref())
            }
            Body::GitStatusChanged(e) => self.granted(self.worktrees.get(&q(&e.worktree_id))),
            Body::FilesChanged(e) => self.granted(self.worktrees.get(&q(&e.worktree_id))),
            // ADR-0088 (schema only): who may see a preview event is decided with the hub step; until then nobody is sent one.
            Body::PreviewSuggested(_) | Body::PreviewChanged(_) => false,
            Body::AgentStarted(e) => {
                let p = pq(node, &e.project_id);
                self.agents.insert(q(&e.agent_id), p.clone());
                self.projects.contains(&p)
            }
            // ADR-0060: the agent may be one the hub never saw start (its store was reset), so it teaches the project too.
            Body::AgentInterrupted(e) => {
                let p = self
                    .agents
                    .entry(q(&e.agent_id))
                    .or_insert_with(|| pq(node, &e.project_id))
                    .clone();
                self.projects.contains(&p)
            }
            Body::AgentStateChanged(e) => self.granted(self.agents.get(&q(&e.agent_id))),
            Body::AgentSessionReported(e) => self.granted(self.agents.get(&q(&e.agent_id))),
            Body::AgentUsageReported(e) => self.granted(self.agents.get(&q(&e.agent_id))),
            Body::AgentSessionOptions(e) => self.granted(self.agents.get(&q(&e.agent_id))),
            // ADR-0090
            Body::AgentSessionConfig(e) => self.granted(self.agents.get(&q(&e.agent_id))),
            Body::AgentCommands(e) => self.granted(self.agents.get(&q(&e.agent_id))),
            // ADR-0101
            Body::AgentAlwaysAllowed(e) => self.granted(self.agents.get(&q(&e.agent_id))),
            // ADR-0094
            Body::AgentRunsInside(e) => self.granted(self.agents.get(&q(&e.agent_id))),
            Body::AgentExited(e) => self.granted(self.agents.get(&q(&e.agent_id))),
            Body::AgentStartRefused(e) => self.granted(self.agents.get(&q(&e.agent_id))),
            // Made by the hub, which names the agent by its hub-wide key already (ADR-0053).
            Body::AgentForgotten(e) => self.granted(self.agents.get(&e.agent_id)),
            Body::TerminalOpened(e) => {
                let p = self.agents.get(&q(&e.agent_id)).cloned();
                if let Some(p) = &p {
                    self.terminals.insert(q(&e.terminal_id), p.clone());
                } else {
                    self.terminals.remove(&q(&e.terminal_id));
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
                } else {
                    self.approvals.remove(&q(&e.approval_id));
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
            // ADR-0095: a dispatch is about one agent of the producer; it is theirs when that agent is.
            Body::TaskDispatched(e) => self.granted(self.agents.get(&q(&e.agent_id))),
        }
    }

    /// ADR-0049 Amendment A: may this member see the machine's answer to a folder listing or a folder's details (the commands
    /// [`Self::answers_confined`] names)? The machine answers with the folder it really opened, links followed, so a link inside the
    /// project that leads out of it is refused here (false: the caller sends `auth.forbidden` instead). A listing whose way up leads
    /// out of the project keeps no way up. A refusal from the machine passes; an answer of any other kind is no folder of theirs.
    pub fn confine_answer(&self, node: &str, resp: &mut v1::CommandResponse) -> bool {
        use v1::command_ok::Result as Ok_;
        use v1::command_response::Result as R;
        match &mut resp.result {
            Some(R::Ok(v1::CommandOk {
                result: Some(Ok_::DirectoryEntries(e)),
            })) => {
                if self.granted_folder(node, &e.path).is_none() {
                    return false;
                }
                if self.granted_folder(node, &e.parent).is_none() {
                    e.parent.clear();
                }
                true
            }
            Some(R::Ok(v1::CommandOk {
                result: Some(Ok_::DirectoryInfo(i)),
            })) => self.granted_folder(node, &i.path).is_some(),
            Some(R::Err(_)) => true,
            _ => false,
        }
    }

    /// The commands whose answer a member gets only through [`Self::confine_answer`].
    pub fn answers_confined(command: &C) -> bool {
        matches!(command, C::DirectoryList(_) | C::DirectoryInspect(_))
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
            // An existing worktree to start in must be the person's too (a node refuses this today; the check is here for when it does not).
            C::WorkspaceQuery(c) => self.granted(self.workspaces.get(&q(&c.workspace_id))),
            C::AgentStart(c) => {
                self.projects.contains(&pq(node, &c.project_id))
                    && (c.worktree_id.is_empty()
                        || self.granted(self.worktrees.get(&q(&c.worktree_id))))
            }
            C::AgentStop(c) => self.granted(self.agents.get(&q(&c.agent_id))),
            C::AgentResume(c) => self.granted(self.agents.get(&q(&c.agent_id))),
            // ADR-0053: named by the hub-wide key, so it is not qualified again.
            C::AgentForget(c) => self.granted(self.agents.get(&c.agent_id)),
            C::AgentPrompt(c) => self.granted(self.agents.get(&q(&c.agent_id))),
            C::AgentSetMode(c) => self.granted(self.agents.get(&q(&c.agent_id))),
            C::AgentSetModel(c) => self.granted(self.agents.get(&q(&c.agent_id))),
            // ADR-0090
            C::AgentSetConfig(c) => self.granted(self.agents.get(&q(&c.agent_id))),
            C::AgentCancelTurn(c) => self.granted(self.agents.get(&q(&c.agent_id))),
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
            C::TaskUpdate(c) => self.projects.contains(&pq(node, &c.project_id)),
            // ADR-0095: the person needs the project and the agent (the node checks that the agent is of that project).
            C::TaskDispatch(c) => {
                self.projects.contains(&pq(node, &c.project_id))
                    && self.granted(self.agents.get(&q(&c.agent_id)))
            }
            // What agents a machine can start is not private to a project, but only for machines the person works on.
            C::AgentKinds(_) => self.node_visible(node),
            // ADR-0113 decision 6: the community theme catalog belongs to no project; every signed-in person may read it (the hub
            // answers it itself and refuses one that names a machine).
            C::ThemeCatalogList(_) | C::ThemeCatalogGet(_) => true,
            // ADR-0049 Amendment A (#1049): folders of a project granted on this machine, and below them; never the home folder (an
            // empty path), a new project identity or a changed one (what a machine also refuses to a device limited to projects). The
            // machine answers with the folder it really opened, which `confine_answer` checks again.
            C::DirectoryList(c) => self.granted_folder(node, &c.path).is_some(),
            C::DirectoryInspect(c) => {
                !c.create_project_id && self.granted_folder(node, &c.path).is_some()
            }
            // A start names the project of its folder: the machine starts only in the exact real folder named (a path through a link
            // is refused there) and only when that folder is of the project named.
            C::AgentLaunch(c) => {
                !c.accept_identity_change
                    && self.granted_folder(node, &c.path) == Some(pq(node, &c.project_id).as_str())
            }
            _ => false,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {

    #[test]
    fn only_the_file_name_of_a_machines_binary_is_shown_to_anyone_but_the_owner() {
        // ADR-0069: any folder may hold the machine's user name, the one it is in too.
        for (path, shown) in [
            ("/home/eda/Downloads/agora", "agora"),
            ("/home/alice/agora", "agora"),
            ("/Users/alice/agora", "agora"),
            (r"C:\Users\alice\agora.exe", "agora.exe"),
            ("agora", "agora"),
            ("", ""),
            ("/opt//agora/", "agora"),
        ] {
            assert_eq!(super::short_binary(path), shown, "{path}");
        }
        let ev = |binary: &str| v1::Event {
            producer_id: "n1".into(),
            body: Some(Body::NodeOnline(v1::NodeOnline {
                node_id: "n1".into(),
                binary: binary.into(),
                ..Default::default()
            })),
            ..Default::default()
        };
        let shown = super::without_full_binary(&ev("/home/eda/bin/agora")).unwrap();
        assert!(matches!(shown.body, Some(Body::NodeOnline(n)) if n.binary == "agora"));
        assert!(super::without_full_binary(&ev("")).is_none());
    }

    use super::*;
    use agora_protocol::v1::{self, command_ok, command_request::Command, command_response};

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
                run_id: Vec::new(),
                agent_id: id.into(),
                project_id: project.into(),
                node_id: node.into(),
                worktree_id: String::new(),
                kind: "k".into(),
                level: 0,
                parent_agent_id: String::new(),
                same_process: false,
                launch_options: Vec::new(),
                skips_permissions: false,
                grant_id: String::new(),
                parent_node_id: String::new(),
                parent_node_agent_id: String::new(),
            }),
        )
    }

    fn scope(granted: &[&str]) -> Scope {
        Scope::from_view(
            granted.iter().map(ToString::to_string).collect(),
            &View::default(),
        )
    }

    /// ADR-0115: a grant given in an older spelling of a GitHub repository, an older node's event in that spelling and a newer node's
    /// folded one are the same project for a member: cut view, live events and commands agree. Another host keeps its case.
    #[test]
    fn a_member_granted_a_project_in_either_case_sees_it_in_both() {
        let root = "0123456789abcdef0123456789abcdef01234567";
        let (cased, folded) = (
            format!("github.com/O/Repo#{root}"),
            format!("github.com/o/repo#{root}"),
        );
        let mut full = View::default();
        for e in [
            checkout("n1", "c1", &cased),
            agent("n1", "a1", &cased),
            agent("n1", "a2", &folded),
        ] {
            full.apply(&e);
        }
        for granted in [format!("n1/{cased}"), format!("n1/{folded}")] {
            let s = Scope::from_view(BTreeSet::from([granted.clone()]), &full);
            let cut = s.filter_view(&full);
            assert!(cut.checkouts.contains_key("n1/c1"), "{granted}");
            assert!(cut.agents.contains_key("n1/a1") && cut.agents.contains_key("n1/a2"));
            let mut live = Scope::from_view(BTreeSet::from([granted.clone()]), &View::default());
            assert!(live.admit(&agent("n1", "a3", &cased)), "{granted}");
            assert!(live.admit(&agent("n1", "a4", &folded)), "{granted}");
            assert!(!live.admit(&agent("n2", "a5", &folded)), "{granted}");
            for project in [&cased, &folded] {
                let update = C::TaskUpdate(v1::TaskUpdate {
                    project_id: project.to_string(),
                    ..Default::default()
                });
                assert!(live.command_allowed("n1", &update), "{granted} {project}");
            }
        }
        let other = format!("git.example.org/O/Repo#{root}");
        let mut s = scope(&[&format!("n1/{other}")]);
        assert!(s.admit(&agent("n1", "a1", &other)));
        assert!(!s.admit(&agent(
            "n1",
            "a2",
            &format!("git.example.org/o/repo#{root}")
        )));
    }

    #[test]
    fn a_member_is_replayed_the_config_and_commands_of_only_the_agents_they_may_see() {
        let mut full = View::default();
        let mut s = scope(&["n1/id:shared"]);
        let cmds = |node: &str, id: &str| {
            ev(
                node,
                Body::AgentCommands(v1::AgentCommands {
                    agent_id: id.into(),
                    commands: vec![v1::AgentCommand {
                        name: "review".into(),
                        ..Default::default()
                    }],
                }),
            )
        };
        for (event, visible) in [
            (agent("n1", "a1", "id:shared"), true),
            (agent("n2", "a2", "id:shared"), false),
            (cmds("n1", "a1"), true),
            (cmds("n2", "a2"), false),
        ] {
            full.apply(&event);
            assert_eq!(s.admit(&event), visible);
        }
        assert_eq!(full.session_replay().len(), 2);
        let replay = s.filter_view(&full).session_replay();
        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0].producer_id, "n1");
    }

    /// ADR-0101: an agent's always answer reaches only the people who may see that agent.
    #[test]
    fn an_always_answer_is_seen_only_with_its_agent() {
        let mut s = scope(&["n1/id:shared"]);
        let always = |node: &str, id: &str| {
            ev(
                node,
                Body::AgentAlwaysAllowed(v1::AgentAlwaysAllowed {
                    agent_id: id.into(),
                    approval_id: "ap-1".into(),
                    action: "run_command".into(),
                    ..Default::default()
                }),
            )
        };
        for (event, visible) in [
            (agent("n1", "a1", "id:shared"), true),
            (agent("n1", "a2", "id:private"), false),
            (always("n1", "a1"), true),
            (always("n1", "a2"), false),
            (always("n1", "never-seen"), false),
        ] {
            assert_eq!(s.admit(&event), visible);
        }
    }

    /// ADR-0094: what runs inside a terminal reaches only the people who may see that agent.
    #[test]
    fn what_runs_inside_a_terminal_is_seen_only_with_its_agent() {
        let mut s = scope(&["n1/id:shared"]);
        let inside = |node: &str, id: &str| {
            ev(
                node,
                Body::AgentRunsInside(v1::AgentRunsInside {
                    agent_id: id.into(),
                    kind: "codex".into(),
                }),
            )
        };
        for (event, visible) in [
            (agent("n1", "a1", "id:shared"), true),
            (agent("n1", "a2", "id:private"), false),
            (inside("n1", "a1"), true),
            (inside("n1", "a2"), false),
            (inside("n1", "never-seen"), false),
        ] {
            assert_eq!(s.admit(&event), visible);
        }
    }

    #[test]
    fn a_guest_member_is_never_given_the_plan_limits_of_a_machine() {
        let mut full = View::default();
        let mut s = scope(&["n1/id:shared"]);
        let obs = ev(
            "n1",
            Body::AgentQuotaObserved(v1::AgentQuotaObserved {
                node_id: "n1".into(),
                tool: "codex".into(),
                window: "five_hour".into(),
                used_percent: 40,
                ..Default::default()
            }),
        );
        full.apply(&obs);
        full.apply(&agent("n1", "a1", "id:shared"));
        // The owner's view replays it; the guest, who may see a project on that very machine, neither live nor on connect.
        assert_eq!(full.session_replay().len(), 1);
        assert!(!s.admit(&obs));
        assert!(s.filter_view(&full).session_replay().is_empty());
    }

    #[test]
    fn portable_identity_does_not_grant_another_nodes_workspace_or_arbitrary_directory_launch() {
        let mut full = View::default();
        let mut s = scope(&["n1/id:shared"]);
        let workspace = |node: &str, id: &str| {
            ev(
                node,
                Body::WorkspaceRegistered(v1::WorkspaceRegistered {
                    workspace_id: id.into(),
                    node_id: "forged-node".into(),
                    project_id: "id:shared".into(),
                    path: "/private".into(),
                    ..Default::default()
                }),
            )
        };
        for (event, visible) in [
            (workspace("n1", "w1"), true),
            (workspace("n2", "w2"), false),
            (agent("n1", "a1", "id:shared"), true),
            (agent("n2", "a2", "id:shared"), false),
            (
                ev(
                    "n1",
                    Body::AgentWorkspaceBound(v1::AgentWorkspaceBound {
                        agent_id: "a1".into(),
                        workspace_id: "w1".into(),
                    }),
                ),
                true,
            ),
            (
                ev(
                    "n2",
                    Body::AgentWorkspaceBound(v1::AgentWorkspaceBound {
                        agent_id: "a2".into(),
                        workspace_id: "w2".into(),
                    }),
                ),
                false,
            ),
        ] {
            full.apply(&event);
            assert_eq!(s.admit(&event), visible);
        }
        let cut = s.filter_view(&full);
        assert_eq!(cut.workspaces.len(), 1);
        assert_eq!(cut.workspaces["n1/w1"].node_id, "n1");
        assert_eq!(cut.agent_workspaces.len(), 1);
        assert_eq!(
            Scope::from_view(["n1/id:shared".to_string()].into(), &full).filter_view(&full),
            cut
        );
        let query = |id: &str| {
            C::WorkspaceQuery(v1::WorkspaceQuery {
                workspace_id: id.into(),
                operation: "read".into(),
                ..Default::default()
            })
        };
        assert!(s.command_allowed("n1", &query("w1")));
        assert!(!s.command_allowed("n2", &query("w2")));
        assert!(!s.command_allowed("n1", &C::DirectoryInspect(v1::DirectoryInspect::default())));
        assert!(!s.command_allowed(
            "n1",
            &C::AgentLaunch(v1::AgentLaunch {
                project_id: "id:shared".into(),
                ..Default::default()
            })
        ));
        assert!(!s.admit(&ev(
            "n2",
            Body::WorkspaceStatusChanged(v1::WorkspaceStatusChanged {
                workspace_id: "w2".into(),
                ..Default::default()
            })
        )));
    }

    #[test]
    fn a_node_cannot_place_its_agent_or_checkout_on_another_machine_by_naming_it() {
        // `n2` says `node_id: "n1"`: what it reports is still `n2`'s, so a member of `n1/mine` neither sees nor commands it.
        let forged = [
            ev(
                "n2",
                Body::AgentStarted(v1::AgentStarted {
                    agent_id: "a".into(),
                    node_id: "n1".into(),
                    project_id: "mine".into(),
                    ..Default::default()
                }),
            ),
            ev(
                "n2",
                Body::CheckoutDiscovered(v1::CheckoutDiscovered {
                    checkout_id: "c".into(),
                    node_id: "n1".into(),
                    project_id: "mine".into(),
                    path: "/p".into(),
                }),
            ),
        ];
        let mut full = View::default();
        let mut s = scope(&["n1/mine"]);
        for e in &forged {
            full.apply(e);
            assert!(!s.admit(e));
        }
        let rebuilt = Scope::from_view(["n1/mine".to_string()].into(), &full);
        for sc in [&s, &rebuilt] {
            assert!(!sc.command_allowed(
                "n2",
                &C::AgentPrompt(v1::AgentPrompt {
                    agent_id: "a".into(),
                    ..Default::default()
                })
            ));
            assert!(!sc.command_allowed(
                "n2",
                &C::WorktreeCreate(v1::WorktreeCreate {
                    checkout_id: "c".into(),
                    ..Default::default()
                })
            ));
        }
        let cut = rebuilt.filter_view(&full);
        assert!(cut.agents.is_empty() && cut.checkouts.is_empty());
        assert_eq!(s.filter_view(&full), cut);
    }

    /// #905: a message between machines is recorded by both nodes. The sender's copy is the member's when the sender is theirs; the
    /// receiver's copy names the sender `<origin>/<agent>` in its producer's words, which no node may say for another node, so it
    /// grants nothing (the window folds the two copies into one row for whoever sees both).
    #[test]
    fn a_message_between_machines_is_the_members_by_the_senders_own_copy_only() {
        let mut s = scope(&["n1/mine"]);
        for e in [
            checkout("n1", "c1", "mine"),
            agent("n1", "a1", "mine"),
            checkout("n2", "c2", "theirs"),
            agent("n2", "b1", "theirs"),
        ] {
            s.admit(&e);
        }
        let posted = |node: &str, from: &str, to: &str| {
            ev(
                node,
                Body::MessagePosted(v1::MessagePosted {
                    message_id: "m-1".into(),
                    from_agent_id: from.into(),
                    to: to.into(),
                    body: "ping".into(),
                }),
            )
        };
        assert!(s.admit(&posted("n1", "a1", "n2/b1")), "the sender's copy");
        assert!(
            !s.admit(&posted("n2", "n1/a1", "b1")),
            "the receiver's copy is not admitted by the name it gives the sender"
        );
        // A member of the receiving project does not see what another project's agent said.
        let mut r = scope(&["n2/theirs"]);
        for e in [
            agent("n1", "a1", "mine"),
            checkout("n2", "c2", "theirs"),
            agent("n2", "b1", "theirs"),
        ] {
            r.admit(&e);
        }
        assert!(!r.admit(&posted("n1", "a1", "n2/b1")));
        assert!(!r.admit(&posted("n2", "n1/a1", "b1")));
    }

    /// ADR-0103 decision 3: a member's snapshot holds exactly the messages the live stream gave them (`admit`), and the owner's every
    /// copy. Run one sequence both ways, for members of either project, and compare.
    #[test]
    fn a_members_recent_messages_are_the_ones_they_would_have_got_live() {
        let posted = |node: &str, from: &str, to: &str, body: &str| {
            ev(
                node,
                Body::MessagePosted(v1::MessagePosted {
                    message_id: "m-1".into(),
                    from_agent_id: from.into(),
                    to: to.into(),
                    body: body.into(),
                }),
            )
        };
        let events = [
            checkout("n1", "c1", "mine"),
            agent("n1", "a1", "mine"),
            agent("n1", "a2", "mine"),
            agent("n1", "x1", "other"),
            checkout("n2", "c2", "theirs"),
            agent("n2", "b1", "theirs"),
            // Within a granted project.
            posted("n1", "a1", "a2", "next door"),
            // Between machines: the sender's copy and the receiver's.
            posted("n1", "a1", "n2/b1", "to theirs"),
            posted("n2", "n1/a1", "b1", "to theirs"),
            posted("n2", "b1", "n1/a1", "to mine"),
            posted("n1", "n2/b1", "a1", "to mine"),
            // A channel message: the sender's local copy, and a copy the hub delivered to the other machine.
            posted("n1", "a1", "#dev", "hello all"),
            posted("n2", "n1/a1", "b1", "[#dev] hello all"),
            // Another project on the same machine, and a sender nobody knows.
            posted("n1", "x1", "a1", "from other"),
            posted("n3", "z", "a1", "stranger"),
        ];
        let mut full = View::default();
        let mut live: BTreeMap<&str, (Scope, Vec<String>)> = ["n1/mine", "n2/theirs", "n1/other"]
            .into_iter()
            .map(|p| (p, (scope(&[p]), Vec::new())))
            .collect();
        for (i, e) in events.iter().enumerate() {
            full.apply_at(i as u64 + 1, e);
            for (s, got) in live.values_mut() {
                if s.admit(e) {
                    if let Some(Body::MessagePosted(m)) = &e.body {
                        got.push(m.body.clone());
                    }
                }
            }
        }
        let bodies = |v: &View| -> Vec<String> {
            crate::server::snapshot_of(v, events.len() as u64)
                .recent_messages
                .into_iter()
                .filter_map(|p| match p.event?.body? {
                    Body::MessagePosted(m) => Some(m.body),
                    _ => None,
                })
                .collect()
        };
        for (project, (_, got_live)) in &live {
            let cut = Scope::from_view([project.to_string()].into(), &full).filter_view(&full);
            assert_eq!(&bodies(&cut), got_live, "{project}");
        }
        assert_eq!(
            live["n1/mine"].1,
            ["next door", "to theirs", "hello all"],
            "the sender's own copies in the granted project"
        );
        assert_eq!(live["n2/theirs"].1, ["to mine"]);
        assert_eq!(live["n1/other"].1, ["from other"]);
        // The owner is sent every copy, the receivers' and the stranger's too.
        assert_eq!(bodies(&full).len(), 9);
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
                    ..Default::default()
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
                    binary: String::new(),
                }),
            ),
            ev(
                "n1",
                Body::NodeOnline(v1::NodeOnline {
                    node_id: "n1".into(),
                    hostname: "h".into(),
                    os: "o".into(),
                    build: "b".into(),
                    binary: String::new(),
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
                    binary: String::new(),
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
                stop_helpers: false,
            })
        };
        assert!(s.command_allowed("n1", &stop("a1")));
        assert!(!s.command_allowed("n1", &stop("a2")));
        assert!(
            !s.command_allowed("n2", &stop("a1")),
            "the same local id on another machine is another agent"
        );
        // Forgetting names the agent by its hub-wide key (ADR-0053): the person's own agent, not another's, on any `target_node`.
        let forget = |id: &str| {
            Command::AgentForget(v1::AgentForget {
                agent_id: id.into(),
            })
        };
        assert!(s.command_allowed("", &forget("n1/a1")));
        assert!(!s.command_allowed("", &forget("n1/a2")));
        assert!(!s.command_allowed("", &forget("a1")));
        let start = |p: &str| {
            Command::AgentStart(v1::AgentStart {
                project_id: p.into(),
                ..Default::default()
            })
        };
        assert!(
            s.command_allowed("n1", &start("mine")) && !s.command_allowed("n1", &start("theirs"))
        );
        let in_worktree = |w: &str| {
            Command::AgentStart(v1::AgentStart {
                project_id: "mine".into(),
                worktree_id: w.into(),
                ..Default::default()
            })
        };
        assert!(
            !s.command_allowed("n1", &in_worktree("someone-elses")),
            "a worktree it cannot place is refused"
        );
        // Anything about a whole machine is the owner's.
        assert!(!s.command_allowed("n1", &Command::NodePolicyGet(v1::NodePolicyGet::default())));
        assert!(!s.command_allowed("n1", &Command::ProjectList(v1::ProjectList::default())));
        assert!(!s.command_allowed("n1", &Command::TerminalOpen(v1::TerminalOpen::default())));
    }

    #[test]
    fn reused_terminal_and_approval_ids_do_not_inherit_stale_grants() {
        let mut projects = BTreeSet::new();
        projects.insert("n1/mine".into());
        let mut s = Scope::from_view(projects, &View::default());
        assert!(s.admit(&ev(
            "n1",
            Body::AgentStarted(v1::AgentStarted {
                agent_id: "a1".into(),
                project_id: "mine".into(),
                ..Default::default()
            }),
        )));
        assert!(s.admit(&ev(
            "n1",
            Body::TerminalOpened(v1::TerminalOpened {
                terminal_id: "t1".into(),
                agent_id: "a1".into(),
                ..Default::default()
            }),
        )));
        assert!(s.admit(&ev(
            "n1",
            Body::ApprovalRequested(v1::ApprovalRequested {
                approval_id: "ap1".into(),
                agent_id: "a1".into(),
                ..Default::default()
            }),
        )));
        // Node restarts: stale terminals and approvals are cleared.
        assert!(s.admit(&ev(
            "n1",
            Body::NodeOnline(v1::NodeOnline {
                node_id: "n1".into(),
                ..Default::default()
            }),
        )));
        assert!(!s.admit(&ev(
            "n1",
            Body::TerminalOpened(v1::TerminalOpened {
                terminal_id: "t1".into(),
                agent_id: String::new(),
                ..Default::default()
            }),
        )));
        assert!(!s.admit(&ev(
            "n1",
            Body::TerminalTitleChanged(v1::TerminalTitleChanged {
                terminal_id: "t1".into(),
                title: "secret".into(),
            }),
        )));
        assert!(!s.command_allowed(
            "n1",
            &Command::TerminalInput(v1::TerminalInput {
                terminal_id: "t1".into(),
                ..Default::default()
            }),
        ));
        assert!(!s.admit(&ev(
            "n1",
            Body::ApprovalRequested(v1::ApprovalRequested {
                approval_id: "ap1".into(),
                agent_id: String::new(),
                ..Default::default()
            }),
        )));
        assert!(!s.admit(&ev(
            "n1",
            Body::ApprovalResolved(v1::ApprovalResolved {
                approval_id: "ap1".into(),
                ..Default::default()
            }),
        )));
    }

    fn checkout_at(node: &str, id: &str, project: &str, path: &str) -> Event {
        ev(
            node,
            Body::CheckoutDiscovered(v1::CheckoutDiscovered {
                checkout_id: id.into(),
                node_id: node.into(),
                project_id: project.into(),
                path: path.into(),
            }),
        )
    }

    fn workspace_at(node: &str, id: &str, project: &str, path: &str, git_root: &str) -> Event {
        ev(
            node,
            Body::WorkspaceRegistered(v1::WorkspaceRegistered {
                workspace_id: id.into(),
                node_id: node.into(),
                project_id: project.into(),
                path: path.into(),
                git_root: git_root.into(),
                ..Default::default()
            }),
        )
    }

    fn list(path: &str) -> Command {
        Command::DirectoryList(v1::DirectoryList {
            path: path.into(),
            authorization: None,
        })
    }

    fn inspect(path: &str, create: bool) -> Command {
        Command::DirectoryInspect(v1::DirectoryInspect {
            path: path.into(),
            create_project_id: create,
            authorization: None,
        })
    }

    fn launch(path: &str, project: &str) -> Command {
        Command::AgentLaunch(v1::AgentLaunch {
            path: path.into(),
            project_id: project.into(),
            kind: "shell".into(),
            ..Default::default()
        })
    }

    /// A member of `n1/mine` (`/srv/mine`, and a repository `/home/eda/web` the hub knows from a start in its `src`) on a hub that also
    /// knows `n1/theirs` (`/srv/theirs`, and `/srv/mine/vendor` inside `mine`) and `n2/mine` (`/srv/mine` on another machine). Built
    /// from the stream and from the view, which must agree.
    fn folder_scopes() -> [Scope; 2] {
        let mut full = View::default();
        let mut s = scope(&["n1/mine"]);
        for e in [
            checkout_at("n1", "c1", "mine", "/srv/mine"),
            checkout_at("n1", "c2", "theirs", "/srv/theirs"),
            checkout_at("n1", "c3", "theirs", "/srv/mine/vendor/"),
            checkout_at("n2", "c1", "mine", "/srv/mine"),
            workspace_at("n1", "w1", "mine", "/home/eda/web/src", "/home/eda/web"),
        ] {
            full.apply(&e);
            s.admit(&e);
        }
        [s, Scope::from_view(["n1/mine".to_string()].into(), &full)]
    }

    #[test]
    fn a_member_chooses_folders_and_starts_agents_inside_a_project_granted_on_that_machine() {
        // ADR-0049 Amendment A (#1049): the project's folders, and any folder below them, on the machine the grant names.
        for s in folder_scopes() {
            for path in [
                "/srv/mine",
                "/srv/mine/",
                "/srv/mine/api",
                "/srv/mine//api/v2",
                "/home/eda/web",
                "/home/eda/web/docs",
            ] {
                assert!(s.command_allowed("n1", &list(path)), "{path}");
                assert!(s.command_allowed("n1", &inspect(path, false)), "{path}");
                assert!(s.command_allowed("n1", &launch(path, "mine")), "{path}");
            }
        }
    }

    #[test]
    fn a_member_is_refused_folders_of_another_project_machine_or_outside_any_project() {
        for s in folder_scopes() {
            for (node, path) in [
                // Another project, beside theirs or inside it (the nearest project folder decides, as the node does).
                ("n1", "/srv/theirs"),
                ("n1", "/srv/theirs/src"),
                ("n1", "/srv/mine/vendor"),
                ("n1", "/srv/mine/vendor/lib"),
                // Outside every project: above it, beside it with a longer name, the home folder (an empty path).
                ("n1", "/srv"),
                ("n1", "/"),
                ("n1", "/srv/mine-old"),
                ("n1", "/home/eda"),
                ("n1", ""),
                // The same folder on another machine, where the grant is not.
                ("n2", "/srv/mine"),
                ("n2", "/srv/mine/api"),
                ("n3", "/srv/mine"),
            ] {
                assert!(!s.command_allowed(node, &list(path)), "{node} {path}");
                assert!(
                    !s.command_allowed(node, &inspect(path, false)),
                    "{node} {path}"
                );
                assert!(
                    !s.command_allowed(node, &launch(path, "mine")),
                    "{node} {path}"
                );
            }
            // A start in their folder names their project, never another one.
            assert!(!s.command_allowed("n1", &launch("/srv/mine/api", "theirs")));
            assert!(!s.command_allowed("n1", &launch("/srv/mine/api", "")));
        }
    }

    #[test]
    fn a_member_cannot_leave_the_project_folder_by_dots_relative_names_or_another_separator() {
        for s in folder_scopes() {
            for path in [
                "/srv/mine/..",
                "/srv/mine/../theirs",
                "/srv/mine/./../theirs",
                "/srv/mine/api/../../theirs",
                "/srv/mine/.",
                "srv/mine",
                "mine",
                "./srv/mine",
                // On Linux `\` is a character of a name: `/srv/mine\..` is a sibling folder of `/srv/mine`, not inside it.
                r"/srv/mine\..\theirs",
                r"/srv/mine\api",
                "/srv/mine/a\0b",
            ] {
                assert!(!s.command_allowed("n1", &list(path)), "{path:?}");
                assert!(!s.command_allowed("n1", &inspect(path, false)), "{path:?}");
                assert!(!s.command_allowed("n1", &launch(path, "mine")), "{path:?}");
            }
            // As a machine's own limits refuse them to a limited device: a new project identity, or one the folder changed.
            assert!(!s.command_allowed("n1", &inspect("/srv/mine", true)));
            assert!(!s.command_allowed(
                "n1",
                &Command::AgentLaunch(v1::AgentLaunch {
                    accept_identity_change: true,
                    ..match launch("/srv/mine", "mine") {
                        Command::AgentLaunch(a) => a,
                        _ => unreachable!(),
                    }
                })
            ));
        }
    }

    #[test]
    fn a_windows_project_folder_is_matched_by_either_separator_and_never_by_dots() {
        let mut s = scope(&["w1/mine"]);
        s.admit(&checkout_at("w1", "c1", "mine", r"C:\Users\eda\mine"));
        for path in [
            r"C:\Users\eda\mine",
            r"C:\Users\eda\mine\api",
            "C:/Users/eda/mine/api",
        ] {
            assert!(s.command_allowed("w1", &list(path)), "{path}");
        }
        for path in [
            r"C:\Users\eda",
            r"C:\Users\eda\mine\..",
            r"C:\Users\eda\mine\..\other",
            r"C:\Users\eda\mine\...",
            r"C:\Users\eda\mine\api.",
            r"C:\Users\eda\mine\api ",
            r"C:\Users\eda\mine\a:stream",
            r"Users\eda\mine",
            r"\Users\eda\mine",
            r"D:\Users\eda\mine",
        ] {
            assert!(!s.command_allowed("w1", &list(path)), "{path}");
        }
    }

    #[test]
    fn a_folder_two_projects_were_reported_in_belongs_to_neither() {
        // The node cannot say which project such a folder is (its identity changed), so neither can the hub.
        let mut s = scope(&["n1/mine"]);
        s.admit(&checkout_at("n1", "c1", "mine", "/srv/app"));
        s.admit(&checkout_at("n1", "c2", "renamed", "/srv/app"));
        assert!(!s.command_allowed("n1", &list("/srv/app/src")));
    }

    #[test]
    fn a_folder_answer_reaches_a_member_only_when_the_folder_the_machine_opened_is_theirs() {
        let ok = |r: command_ok::Result| v1::CommandResponse {
            request_id: "r".into(),
            result: Some(command_response::Result::Ok(v1::CommandOk {
                result: Some(r),
            })),
        };
        let entries = |path: &str, parent: &str| {
            ok(command_ok::Result::DirectoryEntries(v1::DirectoryEntries {
                path: path.into(),
                parent: parent.into(),
                entries: vec![v1::DirectoryEntry {
                    name: "x".into(),
                    is_git: false,
                }],
                truncated: false,
            }))
        };
        let info = |path: &str| {
            ok(command_ok::Result::DirectoryInfo(v1::DirectoryInfo {
                path: path.into(),
                project_id: "mine".into(),
                ..Default::default()
            }))
        };
        for s in folder_scopes() {
            // Inside: as answered, with the way up while it stays inside.
            let mut r = entries("/srv/mine/api", "/srv/mine");
            assert!(s.confine_answer("n1", &mut r));
            assert_eq!(r, entries("/srv/mine/api", "/srv/mine"));
            // At the project's own folder the way up leads out of it, so it is left out.
            let mut r = entries("/srv/mine", "/srv");
            assert!(s.confine_answer("n1", &mut r));
            assert_eq!(r, entries("/srv/mine", ""));
            // A link inside the project that led the machine elsewhere: refused, whatever was asked.
            for mut r in [
                entries("/etc", "/"),
                entries("/srv/theirs", "/srv"),
                entries("/srv/mine/vendor", "/srv/mine"),
                info("/home/eda"),
            ] {
                assert!(!s.confine_answer("n1", &mut r), "{r:?}");
            }
            assert!(!s.confine_answer("n2", &mut entries("/srv/mine", "/srv")));
            let mut r = info("/srv/mine/api");
            assert!(s.confine_answer("n1", &mut r));
            // A refusal from the machine passes as it is; an answer of another kind is not a folder the member may see.
            let mut refused = v1::CommandResponse {
                request_id: "r".into(),
                result: Some(command_response::Result::Err(v1::ErrorInfo {
                    code: "protocol.invalid_message".into(),
                    ..Default::default()
                })),
            };
            assert!(s.confine_answer("n1", &mut refused));
            assert!(!s.confine_answer("n1", &mut ok(command_ok::Result::Empty(v1::Empty {}))));
        }
    }
}
