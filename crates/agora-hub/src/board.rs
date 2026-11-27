//! The shared board of a project (ADR-0039): tasks, path leases and handoffs for a project that has checkouts on two or
//! more online nodes. The hub is the single writer, so a claim or a lease is decided in one place.
//!
//! A board is the same broker a node runs ([`agora_mesh::broker::Mesh`]) with agents named `<node>/<agent>`, so there is one
//! implementation of the rules. The board's agents are not kept by hand: before each call they are made to match the hub's
//! view (running agents of that project), so an agent that ended gives its tasks and leases back.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use agora_core::{AgentId, LeaseTable};
use agora_mesh::broker::{AgentInfo, Limits, Mesh};
use agora_mesh::mcp::{
    board_tool, channel_text, protocol_error_result, send_message_args, SendMessageArgs,
    BOARD_TOOLS,
};
use agora_protocol::v1::event::Body;
use agora_protocol::{Error, ErrorCode};
use serde_json::{json, Value};

use crate::projection::View;

/// A handoff the hub must now pass to the receiving agent's node, after the board accepted it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handoff {
    pub project_id: String,
    pub task_id: Option<String>,
    pub to_node: String,
    pub to_agent: String,
    pub from_node: String,
    pub from_agent: String,
    pub body: String,
    pub refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// The project is not on two online nodes: nothing was done; the node runs the tool on its own broker.
    NotShared,
    /// The tool ran on the board: its MCP result and whether it is a tool error.
    Done { result: Value, is_error: bool },
    /// The call itself is malformed (unknown tool, bad arguments).
    Invalid(Error),
}

/// A `#channel` message on a shared project: one copy for every other running agent of the project, on any node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fanout {
    pub from_node: String,
    pub from_agent: String,
    /// `(node, agent)` of each receiver.
    pub targets: Vec<(String, String)>,
    pub body: String,
    pub refs: Vec<String>,
}

#[derive(Debug)]
pub struct Answer {
    pub fanout: Option<Fanout>,
    pub verdict: Verdict,
    /// Audit events the call produced, to be published as hub events.
    pub events: Vec<Body>,
    pub handoff: Option<Handoff>,
    /// Board notices for agents to send with the answer (ADR-0115: the owners of tasks a merge of older boards gave new ids, #1150).
    pub notices: Vec<Notice>,
}

fn answer(verdict: Verdict) -> Answer {
    Answer {
        verdict,
        events: Vec::new(),
        handoff: None,
        fanout: None,
        notices: Vec::new(),
    }
}

/// Known `(node, project)` memberships from checkouts, selected workspaces and still-running agents.
/// Facts can repeat and include offline nodes.
pub fn project_memberships(view: &View) -> impl Iterator<Item = (&str, &str)> {
    view.checkouts
        .values()
        .map(|(node, project, _)| (node.as_str(), project.as_str()))
        .chain(
            view.workspaces
                .values()
                .map(|workspace| (workspace.node_id.as_str(), workspace.project_id.as_str())),
        )
        .chain(view.agents.iter().filter_map(|(id, agent)| {
            if !agent.running {
                return None;
            }
            // Running sessions keep their original project when folder metadata changes. The qualified key names the
            // authenticated producer; the payload's node_id must not manufacture another node's membership.
            id.split_once('/')
                .map(|(node, _)| (node, agent.project_id.as_str()))
        }))
}

/// The distinct online nodes with a checkout, selected workspace or running session of `project`.
pub fn nodes_with(view: &View, project: &str) -> Vec<String> {
    let mut nodes: Vec<String> = project_memberships(view)
        .filter(|(node, p)| *p == project && view.nodes.get(*node).is_some_and(|v| v.online))
        .map(|(node, _)| node.to_string())
        .collect();
    nodes.sort();
    nodes.dedup();
    nodes
}

/// How long lease and claim calls on a project that just became shared wait for its nodes to hand over their local work
/// (ADR-0039 amendment B). A node that answers later still hands it over.
pub const JOIN_WAIT_MS: u64 = 5_000;
/// At most this many tasks and leases are taken from one node's answer, and a handed-over lease runs at most this long: the answer
/// is a node's word, and it must not fill the board or hold a path forever.
pub const MAX_JOIN_TASKS: usize = agora_mesh::broker::MAX_HANDOVER_TASKS;
pub const MAX_JOIN_LEASES: usize = agora_mesh::broker::MAX_HANDOVER_LEASES;
pub const MAX_JOIN_LEASE_MS: u64 = 60 * 60 * 1000;
/// Who the board names as a handed-over task's creator when the node's own agent is not running any more.
pub const BOARD_AGENT: &str = "hub/board";

/// A message the hub sends an agent about the board (ADR-0039 amendment B). The body is a small JSON object of stable codes, not
/// prose: `{"board": "task_moved", "from": ..., "to": ...}`, `{"board": "lease_refused", "path": ..., "holder": ...}`,
/// `{"board": "lease_conflict", "path": ..., "other": ...}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub to_node: String,
    pub to_agent: String,
    pub body: String,
}

/// A [`Notice`] for the agent `to` names (`<node>/<agent>`); none for an id without a node.
fn notice_to(to: &str, body: Value) -> Option<Notice> {
    let (to_node, to_agent) = to.split_once('/')?;
    Some(Notice {
        to_node: to_node.to_string(),
        to_agent: to_agent.to_string(),
        body: body.to_string(),
    })
}

/// What [`Boards::merge_snapshots`] made of several boards (ADR-0115, #1150).
pub struct Merged {
    /// The merged board, as [`Boards::snapshot`] text.
    pub bytes: Vec<u8>,
    /// For the owner of each task given a new id: `task_moved`; for the holders of a path two boards' leases held: `lease_refused`
    /// to the one whose lease was dropped, `lease_conflict` to the one who keeps it (as after a join).
    pub notices: Vec<Notice>,
    /// Each task given a new id, and each task that kept an id another board also had: a window keys a task by project and id, so
    /// it showed the boards' tasks with one id as one, and each is stated again.
    pub restated: BTreeSet<String>,
}

#[derive(Default)]
struct Joins {
    /// The online nodes with a checkout of each project at the last look.
    members: BTreeMap<String, BTreeSet<String>>,
    /// Projects whose nodes are handing over local work: until when calls wait, and the nodes not yet answered.
    pending: BTreeMap<String, (u64, BTreeSet<String>)>,
}

/// A project's board at one moment (see [`Boards::checkpoint`]).
pub struct Checkpoint(Option<Mesh>);

/// Whether a board tool can change the board (everything but reading the task list).
pub fn changes_board(tool: &str) -> bool {
    BOARD_TOOLS.contains(&tool) && tool != "task_list"
}

#[derive(Default)]
pub struct Boards {
    by_project: Mutex<BTreeMap<String, Mesh>>,
    joins: Mutex<Joins>,
    /// Nodes whose process started again while this hub runs: a board restored later still names their dead agents.
    restarted: Mutex<BTreeSet<String>>,
}

fn tool_error(code: ErrorCode, key: &str, value: &str) -> Value {
    json!({
        "content": [{"type": "text", "text": json!({"error": {"code": code.as_str(), "params": {key: value}}}).to_string()}],
        "isError": true,
    })
}

impl Boards {
    pub fn new() -> Self {
        Self::default()
    }

    /// Makes the board's agents match the running agents of the project in `view`.
    fn sync(mesh: &mut Mesh, view: &View, project: &str, now_ms: u64) {
        let wanted: BTreeMap<&String, &crate::projection::AgentView> = view
            .agents
            .iter()
            .filter(|(_, a)| a.project_id == project && a.running)
            .collect();
        for (key, a) in &wanted {
            let id = AgentId::new(key.as_str());
            if !mesh.agent_ids().contains(&id) {
                mesh.register(AgentInfo {
                    id,
                    kind: a.kind.clone(),
                    node: a.node_id.clone(),
                    project: project.to_string(),
                    label: key.to_string(),
                    worktree: String::new(),
                    state: "idle".into(),
                });
            }
        }
        for id in mesh.agent_ids() {
            if !wanted.contains_key(&id.as_str().to_string()) {
                mesh.unregister(&id, now_ms);
            }
        }
    }

    /// `send_message` to `#channel`: the same limits as a local message, then one copy per other running agent.
    fn channel(view: &View, node: &str, agent: &str, project: &str, args: &Value) -> Answer {
        let limits = Limits::default();
        let invalid = |f: &str| {
            answer(Verdict::Invalid(
                Error::new(ErrorCode::ProtocolInvalidMessage).with("field", f),
            ))
        };
        // As the broker reads them (the node checks first; this is the last guard).
        let Some(SendMessageArgs { to, body, refs, .. }) = send_message_args(args) else {
            return invalid("arguments");
        };
        if !to.starts_with('#') || to.len() < 2 {
            return invalid("to");
        }
        // Receivers get the channel's name in front of the text and refuse a body over the limit, so that text is what is checked,
        // before any copy goes out.
        let body = channel_text(to, body);
        let done = |result: Value, is_error: bool| Verdict::Done { result, is_error };
        if let Err(e) = limits.check_content(&body, &refs) {
            return answer(done(protocol_error_result(&e.to_error(&limits)), true));
        }
        let me = format!("{node}/{agent}");
        let mut targets: Vec<(String, String)> = view
            .agents
            .iter()
            .filter(|(k, a)| {
                a.project_id == project
                    && a.running
                    && **k != me
                    && view.nodes.get(&a.node_id).is_some_and(|n| n.online)
            })
            .filter_map(|(k, a)| {
                k.strip_prefix(&format!("{}/", a.node_id))
                    .map(|local| (a.node_id.clone(), local.to_string()))
            })
            .collect();
        targets.sort();
        let text = json!({"delivered_to": targets.len()});
        let mut a = answer(done(
            json!({"content": [{"type": "text", "text": text.to_string()}], "isError": false}),
            false,
        ));
        // The channel's name stays in front of the text: receivers get it as a message from the sender, like any cross-node one.
        a.fanout = Some(Fanout {
            from_node: node.to_string(),
            from_agent: agent.to_string(),
            targets,
            body,
            refs,
        });
        a
    }

    /// `workspace_status` on a shared project: the hub's facts about every running agent of the project on an online node (kind,
    /// machine, state, the board task it claimed, the paths it holds, who started it), the board's open tasks and live leases, and how
    /// many machines are online. Read only; the calling node merges it with what it knows of its own agents.
    fn status(&self, view: &View, project: &str, now_ms: u64) -> Answer {
        let mut guard = self.by_project.lock().unwrap_or_else(|e| e.into_inner());
        let mesh = guard
            .entry(project.to_string())
            .or_insert_with(|| Mesh::new(Limits::default()));
        Self::sync(mesh, view, project, now_ms);
        mesh.release_orphans();
        mesh.expire_leases(now_ms);
        let agents: Vec<Value> = view
            .agents
            .iter()
            .filter(|(_, a)| {
                a.project_id == project
                    && a.running
                    && view.nodes.get(&a.node_id).is_some_and(|n| n.online)
            })
            .take(agora_mesh::awareness::MAX_AGENTS)
            .map(|(key, a)| {
                let id = AgentId::new(key.as_str());
                json!({
                    "id": key, "kind": a.kind, "machine": a.node_id, "state": state_word(a.state),
                    "holds": mesh.holds(&id, now_ms), "task": mesh.claimed_task(&id),
                    "parent": (!a.parent_agent_id.is_empty()).then_some(a.parent_agent_id.as_str()),
                    "workspace": folder_of(view, key, a),
                })
            })
            .collect();
        let open_tasks: Vec<Value> = mesh
            .export_tasks()
            .1
            .iter()
            .filter(|t| t.project == project && t.status != agora_mesh::broker::TaskStatus::Done)
            .take(agora_mesh::awareness::MAX_TASKS)
            .map(|t| {
                json!({"id": t.id.as_str(), "title": cut(&t.title, 512),
                       "owner": t.owner.as_ref().map(AgentId::as_str)})
            })
            .collect();
        let leases: Vec<Value> = mesh
            .live_leases(project, now_ms)
            .iter()
            .take(agora_mesh::awareness::MAX_LEASES)
            .map(|(path, holder, expires)| {
                json!({"path": cut(path, 512), "owner": holder.as_str(),
                       "ttl": expires.saturating_sub(now_ms).div_ceil(1000)})
            })
            .collect();
        let text = json!({
            "project": project, "machines_online": nodes_with(view, project).len(),
            "agents": agents, "open_tasks": open_tasks, "leases": leases,
        });
        let mut a = answer(Verdict::Done {
            result: json!({"content": [{"type": "text", "text": text.to_string()}], "isError": false}),
            is_error: false,
        });
        a.events = mesh.take_events();
        a
    }

    /// A copy of a project's board as it is now, to put back with [`restore`](Self::restore) when a change cannot be stored.
    pub fn checkpoint(&self, project: &str) -> Checkpoint {
        let g = self.by_project.lock().unwrap_or_else(|e| e.into_inner());
        Checkpoint(g.get(project).cloned())
    }

    /// Puts a project's board back as [`checkpoint`](Self::checkpoint) saw it (no board then: none now).
    pub fn restore(&self, project: &str, saved: Checkpoint) {
        let mut g = self.by_project.lock().unwrap_or_else(|e| e.into_inner());
        match saved.0 {
            Some(mesh) => {
                g.insert(project.to_string(), mesh);
            }
            None => {
                g.remove(project);
            }
        }
    }

    /// Undoes a handoff whose message could not be delivered: the task goes back to the giver. Returns the audit events.
    pub fn revert_handoff(&self, h: &Handoff) -> Vec<Body> {
        let Some(task) = &h.task_id else {
            return Vec::new();
        };
        let mut g = self.by_project.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mesh) = g.get_mut(&h.project_id) else {
            return Vec::new();
        };
        let giver = AgentId::new(format!("{}/{}", h.from_node, h.from_agent));
        let receiver = AgentId::new(format!("{}/{}", h.to_node, h.to_agent));
        mesh.reassign_task(task, &receiver, &giver);
        mesh.take_events()
    }

    /// Compensate a tracked delivery only while the board still holds the receiver's claim.
    pub(crate) fn revert_tracked(
        &self,
        pending: &crate::handoff::Pending,
        view: &View,
        now_ms: u64,
    ) -> Vec<Body> {
        let delivery = &pending.delivery;
        let project = &pending.board_project;
        let mut boards = self.by_project.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mesh) = boards.get_mut(project) else {
            return Vec::new();
        };
        let receiver = AgentId::new(format!("{}/{}", pending.receiver_node, delivery.agent_id));
        let still_claimed = mesh.export_tasks().1.iter().any(|task| {
            task.id.as_str() == delivery.task_id
                && task.status == agora_mesh::broker::TaskStatus::Claimed
                && task.owner.as_ref() == Some(&receiver)
        });
        if !still_claimed {
            return Vec::new();
        }
        let giver = format!("{}/{}", delivery.sender_node, delivery.sender_agent_id);
        let giver = view
            .agents
            .get(&giver)
            .filter(|a| {
                a.running
                    && a.project_id == *project
                    && match a.run_id {
                        Some(run) => run.as_slice() == pending.giver_run,
                        None => {
                            pending.giver_run.is_empty()
                                && a.started_seq == pending.giver_started_seq
                        }
                    }
            })
            .map_or("", |_| giver.as_str());
        Self::sync(mesh, view, project, now_ms);
        mesh.reassign_task(&delivery.task_id, &receiver, &AgentId::new(giver));
        mesh.take_events()
    }

    /// A node's process started again: every agent it had is gone, even if a new one will carry the same local id (`agent-1`). Gives
    /// their tasks and leases back now, in every board in memory, and returns each project with the events that produced.
    pub fn node_restarted(&self, node: &str, now_ms: u64) -> Vec<(String, Vec<Body>)> {
        self.restarted
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(node.to_string());
        let prefix = format!("{node}/");
        let mut g = self.by_project.lock().unwrap_or_else(|e| e.into_inner());
        let mut out = Vec::new();
        for (project, mesh) in g.iter_mut() {
            let gone: Vec<AgentId> = mesh
                .agent_ids()
                .into_iter()
                .filter(|id| id.as_str().starts_with(&prefix))
                .collect();
            for id in &gone {
                mesh.unregister(id, now_ms);
            }
            let events = mesh.take_events();
            if !gone.is_empty() || !events.is_empty() {
                out.push((project.clone(), events));
            }
        }
        out
    }

    /// True while the project's board has a task that is not done or a lease that has not ended.
    fn holds_unfinished_work(&self, project: &str, now_ms: u64) -> bool {
        use agora_mesh::broker::TaskStatus;
        let g = self.by_project.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mesh) = g.get(project) else {
            return false;
        };
        let (_, tasks) = mesh.export_tasks();
        tasks
            .iter()
            .any(|t| t.project == project && t.status != TaskStatus::Done)
            || mesh
                .export_leases(project, now_ms)
                .iter()
                .any(|(_, _, expires)| *expires > now_ms)
    }

    pub fn has(&self, project: &str) -> bool {
        self.by_project
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(project)
    }

    /// The project's tasks as stored text (ADR-0039: the board is part of the hub's SQLite file and its backup).
    pub fn snapshot(&self, project: &str, now_ms: u64) -> Option<Vec<u8>> {
        use agora_mesh::broker::TaskStatus;
        let g = self.by_project.lock().unwrap_or_else(|e| e.into_inner());
        let mesh = g.get(project)?;
        let (next, tasks) = mesh.export_tasks();
        let leases: Vec<Value> = mesh
            .export_leases(project, now_ms)
            .iter()
            .map(|(path, holder, exp)| json!({"path": path, "holder": holder.as_str(), "expires_at_ms": exp}))
            .collect();
        let tasks: Vec<Value> = tasks
            .iter()
            .filter(|t| t.project == project)
            .map(|t| {
                json!({
                    "id": t.id.as_str(), "title": t.title, "detail": t.detail,
                    "depends_on": t.depends_on.iter().map(agora_core::TaskId::as_str).collect::<Vec<_>>(),
                    "done": t.status == TaskStatus::Done,
                    "owner": t.owner.as_ref().map(AgentId::as_str),
                    "summary": t.summary,
                })
            })
            .collect();
        Some(
            json!({"next": next, "tasks": tasks, "leases": leases})
                .to_string()
                .into_bytes(),
        )
    }

    /// ADR-0115: one [`snapshot`](Self::snapshot) text from several of one project's boards (an older hub kept one per spelling of a
    /// GitHub, GitLab, Bitbucket or Codeberg path), to [`load`](Self::load) as that project's board. The first is taken as it is; a
    /// task of a later one whose id is already taken gets a new one above every id of all of them (its own `depends_on` follows, and
    /// its owner is told, as `task_moved` after a join, unless the task is done), and where several boards' live leases hold one
    /// path, or paths one of which is inside the other (`src` and `src/a.rs`), one holder keeps it, the others' leases are passed on
    /// as ended and both are told (see the lease rules below; `running` says whether an agent runs in the project).
    /// A dependency on a task its own board no longer has stays unmet: when another board's task has that id, it is given an id of
    /// its own that no task has or will have, instead of naming that task (#1150). A damaged one is left out.
    pub fn merge_snapshots(
        &self,
        parts: &[Vec<u8>],
        now_ms: u64,
        running: impl Fn(&str) -> bool,
    ) -> Merged {
        let boards: Vec<Value> = parts
            .iter()
            .filter_map(|b| serde_json::from_slice::<Value>(b).ok())
            .filter(Value::is_object)
            .collect();
        let tasks_of =
            |b: &Value| -> Vec<Value> { b["tasks"].as_array().cloned().unwrap_or_default() };
        let id_of = |t: &Value| t["id"].as_str().unwrap_or("").to_string();
        let task_number = |id: &str| id.strip_prefix("t-").and_then(|n| n.parse::<u64>().ok());
        // Above every id any of them names, a dependency's included: a new id never meets one.
        let mut next = boards
            .iter()
            .map(|b| b["next"].as_u64().unwrap_or(1))
            .chain(boards.iter().flat_map(|b| {
                tasks_of(b)
                    .into_iter()
                    .flat_map(|t| {
                        let deps = t["depends_on"].as_array().cloned().unwrap_or_default();
                        std::iter::once(t["id"].clone()).chain(deps)
                    })
                    .filter_map(|id| id.as_str().and_then(task_number))
                    .map(|n| n + 1)
            }))
            .max()
            .unwrap_or(1);
        // Each board's new ids, and every id the merged board holds.
        let mut taken = BTreeSet::<String>::new();
        let mut renames = Vec::<BTreeMap<String, String>>::new();
        let mut restated = BTreeSet::<String>::new();
        let mut notices = Vec::new();
        for board in &boards {
            let mine = tasks_of(board);
            let mut renamed = BTreeMap::<String, String>::new();
            for t in &mine {
                let id = id_of(t);
                if taken.contains(&id) {
                    let to = format!("t-{next}");
                    next += 1;
                    let done = t["done"].as_bool().unwrap_or(false);
                    if let Some(owner) = t["owner"].as_str().filter(|_| !done) {
                        notices.extend(notice_to(
                            owner,
                            json!({"board": "task_moved", "from": id, "to": to}),
                        ));
                    }
                    restated.insert(id.clone());
                    restated.insert(to.clone());
                    renamed.insert(id, to);
                }
            }
            for t in &mine {
                let id = id_of(t);
                taken.insert(renamed.get(&id).cloned().unwrap_or(id));
            }
            renames.push(renamed);
        }
        let mut tasks = Vec::<Value>::new();
        let mut all_leases = Vec::<Value>::new();
        for (board, renamed) in boards.iter().zip(&renames) {
            let mine = tasks_of(board);
            let own: BTreeSet<String> = mine.iter().map(id_of).collect();
            let mut unmet = BTreeMap::<String, String>::new();
            for mut t in mine {
                let rename = |id: &str| renamed.get(id).cloned().unwrap_or_else(|| id.to_string());
                t["id"] = Value::String(rename(&id_of(&t)));
                if let Some(deps) = t["depends_on"].as_array_mut() {
                    for d in deps.iter_mut() {
                        let Some(x) = d.as_str() else {
                            continue;
                        };
                        let to = if own.contains(x) {
                            rename(x)
                        } else if taken.contains(x) {
                            // Another board's task has this id: an id no task has keeps it unmet, as it was on its own board.
                            unmet
                                .entry(x.to_string())
                                .or_insert_with(|| {
                                    next += 1;
                                    format!("t-{}", next - 1)
                                })
                                .clone()
                        } else {
                            x.to_string()
                        };
                        *d = Value::String(to);
                    }
                }
                tasks.push(t);
            }
            all_leases.extend(board["leases"].as_array().cloned().unwrap_or_default());
        }
        // Leases (#1150). A live lease whose holder runs keeps its path before one whose holder is gone (not running in the project,
        // or on a node that started again since this hub opened: `load` and the next `sync` free those), first board first. Every
        // other holder's live lease on that path, or on a path above or below it, is passed on as ended, so `load` says it released
        // (a window still shows the older hub's grant) and both holders are told, as when a join finds the path held. An ended lease
        // is passed on too, for the same reason, unless its holder has a live lease on that path (its release would end that lease
        // in the window's view).
        let restarted = self
            .restarted
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let gone = |holder: &str| {
            !running(holder)
                || holder
                    .split_once('/')
                    .is_some_and(|(node, _)| restarted.contains(node))
        };
        let fields = |l: &Value| -> Option<(String, String, u64)> {
            Some((
                l["path"].as_str()?.to_string(),
                l["holder"].as_str()?.to_string(),
                l["expires_at_ms"].as_u64()?,
            ))
        };
        let live = |l: &&Value| fields(l).is_some_and(|(_, _, expires)| expires > now_ms);
        // Which live lease keeps its path is decided as `load` grants them: a lease conflicts with another holder's on the same path
        // or one above or below it (`src` and `src/a.rs`), never with its own holder's. `refused` names, for each lease that loses,
        // the holder that keeps the path it overlaps. A path the board cannot take is left to `load`, as before.
        let mut table = agora_core::LeaseTable::new();
        let mut refused = BTreeMap::<(String, String), String>::new();
        for pass_gone in [false, true] {
            for (path, holder, expires) in all_leases.iter().filter(live).filter_map(fields) {
                if gone(&holder) != pass_gone
                    || refused.contains_key(&(path.clone(), holder.clone()))
                {
                    continue;
                }
                let by = AgentId::new(&holder);
                if let Err(agora_core::LeaseError::Conflict { holder: kept, .. }) =
                    table.acquire(&path, &by, now_ms, expires - now_ms)
                {
                    refused.insert((path, holder), kept.as_str().to_string());
                }
            }
        }
        let mut leases = Vec::<Value>::new();
        // Live ones first: a holder's kept or refused lease is passed on once, and its ended one then adds nothing.
        let mut passed = BTreeSet::<(String, String)>::new();
        let (live_ones, ended_ones): (Vec<&Value>, Vec<&Value>) = all_leases.iter().partition(live);
        for l in live_ones.into_iter().chain(ended_ones) {
            let Some((path, holder, expires)) = fields(l) else {
                continue;
            };
            if !passed.insert((path.clone(), holder.clone())) {
                continue;
            }
            // An ended lease whose holder keeps a live one on that path was passed on above.
            if expires <= now_ms {
                leases.push(l.clone());
                continue;
            }
            let Some(kept) = refused.get(&(path.clone(), holder.clone())).cloned() else {
                leases.push(l.clone());
                continue;
            };
            let mut ended = l.clone();
            ended["expires_at_ms"] = json!(0);
            leases.push(ended);
            notices.extend(notice_to(
                &holder,
                json!({"board": "lease_refused", "path": path, "holder": kept}),
            ));
            notices.extend(notice_to(
                &kept,
                json!({"board": "lease_conflict", "path": path, "other": holder}),
            ));
        }
        Merged {
            bytes: json!({"next": next, "tasks": tasks, "leases": leases})
                .to_string()
                .into_bytes(),
            notices,
            restated,
        }
    }

    /// ADR-0115 (#1150): `TaskChanged` for each of `ids` the project's board holds, as the board says it when the task changes: what
    /// a window must be told again after a merge.
    pub fn task_changes(&self, project: &str, ids: &BTreeSet<String>) -> Vec<Body> {
        use agora_mesh::broker::TaskStatus;
        let g = self.by_project.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mesh) = g.get(project) else {
            return Vec::new();
        };
        let (_, tasks) = mesh.export_tasks();
        tasks
            .iter()
            .filter(|t| t.project == project && ids.contains(t.id.as_str()))
            .map(|t| {
                Body::TaskChanged(agora_protocol::v1::TaskChanged {
                    task_id: t.id.as_str().to_string(),
                    project_id: project.to_string(),
                    title: t.title.clone(),
                    claimed_by_agent_id: t
                        .owner
                        .as_ref()
                        .map(|o| o.as_str().to_string())
                        .unwrap_or_default(),
                    done: t.status == TaskStatus::Done,
                })
            })
            .collect()
    }

    /// Restores a board from [`snapshot`](Self::snapshot) text. A damaged snapshot is ignored: the board starts empty.
    pub fn load(&self, project: &str, bytes: &[u8], now_ms: u64) {
        use agora_mesh::broker::{TaskEntry, TaskStatus};
        let Ok(v) = serde_json::from_slice::<Value>(bytes) else {
            return;
        };
        let Some(task_values) = v["tasks"].as_array() else {
            return;
        };
        let qualified_agent = |id: &str| {
            id.split_once('/').is_some_and(|(node, agent)| {
                !node.is_empty() && !agent.is_empty() && !agent.contains('/')
            })
        };
        for task in task_values {
            if task["id"].as_str().is_none()
                || task["title"].as_str().is_none()
                || task["detail"].as_str().is_none()
                || (!task["summary"].is_null() && task["summary"].as_str().is_none())
            {
                return;
            }
            let Some(dependencies) = task["depends_on"].as_array() else {
                return;
            };
            if dependencies
                .iter()
                .any(|dependency| !dependency.is_string())
                || task["done"].as_bool().is_none()
            {
                return;
            }
            if let Some(owner) = task["owner"].as_str() {
                if !qualified_agent(owner) {
                    return;
                }
            } else if !task["owner"].is_null() {
                return;
            }
        }
        let text = |t: &Value, k: &str| t[k].as_str().unwrap_or("").to_string();
        let tasks: Vec<TaskEntry> = task_values
            .iter()
            .map(|t| {
                let owner = t["owner"].as_str().map(AgentId::new);
                let done = t["done"].as_bool().unwrap_or(false);
                TaskEntry {
                    id: agora_core::TaskId::new(text(t, "id")),
                    project: project.to_string(),
                    title: text(t, "title"),
                    detail: text(t, "detail"),
                    depends_on: t["depends_on"]
                        .as_array()
                        .map(|d| {
                            d.iter()
                                .filter_map(|x| x.as_str().map(agora_core::TaskId::new))
                                .collect()
                        })
                        .unwrap_or_default(),
                    status: match (&owner, done) {
                        (None, _) => TaskStatus::Open,
                        (Some(_), false) => TaskStatus::Claimed,
                        (Some(_), true) => TaskStatus::Done,
                    },
                    owner,
                    summary: t["summary"].as_str().map(str::to_string),
                }
            })
            .collect();
        let Some(next) = v["next"].as_u64() else {
            return;
        };
        if next == 0 || next == u64::MAX {
            return;
        }
        let mut ids = BTreeMap::new();
        for (value, task) in task_values.iter().zip(&tasks) {
            let Some(number) = task
                .id
                .as_str()
                .strip_prefix("t-")
                .and_then(|value| value.parse::<u64>().ok())
            else {
                return;
            };
            if number == 0 || number >= next || ids.insert(task.id.clone(), number).is_some() {
                return;
            }
            if value["done"].as_bool() == Some(true) && task.owner.is_none() {
                return;
            }
        }
        // A dependency names a task of this board, or an id below `next` that no task has: a merge (ADR-0115) points a dependency
        // whose task went to another id there, so the task keeps waiting, and no new task can ever take that id. Anything else (an
        // id at or above `next`, which a later task would get, or not a task id at all) refuses the board, and so does a cycle.
        let placeholder = |dependency: &agora_core::TaskId| {
            dependency
                .as_str()
                .strip_prefix("t-")
                .and_then(|value| value.parse::<u64>().ok())
                .is_some_and(|number| number > 0 && number < next)
        };
        let by_id: BTreeMap<&agora_core::TaskId, &TaskEntry> =
            tasks.iter().map(|task| (&task.id, task)).collect();
        if tasks.iter().any(|task| {
            task.depends_on
                .iter()
                .any(|dependency| !ids.contains_key(dependency) && !placeholder(dependency))
        }) {
            return;
        }
        // Depth-first, without recursion: 0 not seen, 1 on the current path, 2 done.
        let mut mark: BTreeMap<&agora_core::TaskId, u8> = BTreeMap::new();
        for start in &tasks {
            if mark.get(&start.id).copied().unwrap_or(0) != 0 {
                continue;
            }
            let mut stack: Vec<(&agora_core::TaskId, usize)> = vec![(&start.id, 0)];
            mark.insert(&start.id, 1);
            while let Some((id, at)) = stack.pop() {
                let deps = by_id
                    .get(id)
                    .map_or(&[][..], |task| task.depends_on.as_slice());
                if let Some(next_dep) = deps.get(at) {
                    stack.push((id, at + 1));
                    if !by_id.contains_key(next_dep) {
                        continue;
                    }
                    match mark.get(next_dep).copied().unwrap_or(0) {
                        0 => {
                            mark.insert(next_dep, 1);
                            stack.push((next_dep, 0));
                        }
                        1 => return,
                        _ => {}
                    }
                } else {
                    mark.insert(id, 2);
                }
            }
        }
        let Some(lease_values) = v["leases"].as_array() else {
            return;
        };
        let mut lease_check = LeaseTable::new();
        let mut restored_leases = Vec::with_capacity(lease_values.len());
        for lease in lease_values {
            let (Some(path), Some(holder), Some(expires)) = (
                lease["path"].as_str(),
                lease["holder"].as_str(),
                lease["expires_at_ms"].as_u64(),
            ) else {
                return;
            };
            if !qualified_agent(holder) {
                return;
            }
            let holder = AgentId::new(holder);
            // Live leases may not overlap another holder's, nor repeat. An ended one (`expires_at_ms` 0 included) is released below
            // whatever it overlaps: a merge (ADR-0115) passes a lease that lost its path on as ended, so loading says it was released.
            if expires > now_ms {
                let before = lease_check.all().len();
                if lease_check
                    .acquire(path, &holder, now_ms, expires - now_ms)
                    .is_err()
                    || lease_check.all().len() != before + 1
                {
                    return;
                }
            }
            restored_leases.push((path.to_string(), holder, expires));
        }
        let Some(live_lease_count) = u64::try_from(
            restored_leases
                .iter()
                .filter(|(_, _, expires)| *expires > now_ms)
                .count(),
        )
        .ok() else {
            return;
        };
        if next.checked_add(live_lease_count).is_none() {
            return;
        }
        let restarted = self
            .restarted
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let mut g = self.by_project.lock().unwrap_or_else(|e| e.into_inner());
        let mesh = g
            .entry(project.to_string())
            .or_insert_with(|| Mesh::new(Limits::default()));
        mesh.import_tasks(next, &tasks);
        let (leases, ended): (Vec<(String, AgentId, u64)>, Vec<_>) = restored_leases
            .into_iter()
            .partition(|(_, _, expires)| *expires > now_ms);
        // A restored lease is granted again to its holder, so it belongs to a grant: when the holder ends, `sync` (or a restart of
        // its node) frees it like any other. A holder that is not registered yet is, until the next `sync` keeps or removes it.
        for (path, holder, expires) in leases {
            if !mesh.agent_ids().contains(&holder) {
                mesh.register(AgentInfo {
                    id: holder.clone(),
                    kind: String::new(),
                    node: holder.as_str().split('/').next().unwrap_or("").to_string(),
                    project: project.to_string(),
                    label: holder.as_str().to_string(),
                    worktree: String::new(),
                    state: "idle".into(),
                });
            }
            let _ = mesh.lease_acquire(
                &holder,
                std::slice::from_ref(&path),
                expires - now_ms,
                now_ms,
            );
        }
        // Those leases were granted before the hub stopped: not news. A board is loaded once, before any call, so nothing else waits.
        let _ = mesh.take_events();
        mesh.import_leases(project, &ended, now_ms);
        // A node that started again since this hub opened lost every agent it had when the board was saved: its claims and leases
        // go back now, or a new agent with a reused id (`agent-1`) would inherit them.
        let dead: BTreeSet<AgentId> = tasks
            .iter()
            .filter_map(|t| t.owner.clone())
            .chain(mesh.agent_ids())
            .filter(|id| {
                id.as_str()
                    .split_once('/')
                    .is_some_and(|(node, _)| restarted.contains(node))
            })
            .collect();
        for id in &dead {
            mesh.unregister(id, now_ms);
        }
    }

    #[allow(clippy::too_many_arguments)]
    /// Runs one board tool for `agent` of `node` (both as the hub knows them: `node` from the connection's certificate).
    pub fn call(
        &self,
        view: &View,
        node: &str,
        agent: &str,
        project: &str,
        tool: &str,
        arguments_json: &str,
        now_ms: u64,
    ) -> Answer {
        let bad = |field: &str| {
            answer(Verdict::Invalid(
                Error::new(ErrorCode::ProtocolInvalidMessage).with("field", field),
            ))
        };
        let channel = tool == "send_message";
        let status = tool == "workspace_status";
        if !channel && !status && !BOARD_TOOLS.contains(&tool) {
            return bad("tool");
        }
        let Ok(mut args) = serde_json::from_str::<Value>(arguments_json) else {
            return bad("arguments_json");
        };
        if !args.is_object() {
            return bad("arguments_json");
        }
        let nodes = nodes_with(view, project);
        // A node can only speak for a project represented by its checkout, workspace or running session. A board is in use while two nodes share the project, and (ADR-0039
        // amendment A) while it still holds a task that is not done or a lease that has not ended: a node that drops must not make
        // the other one fall back to a local broker that knows nothing of the hub's tasks.
        if !nodes.iter().any(|n| n == node)
            || (nodes.len() < 2 && !self.holds_unfinished_work(project, now_ms))
        {
            return answer(Verdict::NotShared);
        }
        if channel {
            return Self::channel(view, node, agent, project, &args);
        }
        if status {
            return self.status(view, project, now_ms);
        }
        // ADR-0039 amendment B: while the project's nodes are handing over their local work, nothing is granted against work the
        // board has not seen yet.
        if matches!(tool, "lease_acquire" | "task_claim") && self.joining(project, now_ms) {
            return answer(Verdict::Done {
                result: tool_error(ErrorCode::BoardUnavailable, "project_id", project),
                is_error: true,
            });
        }
        let caller = format!("{node}/{agent}");
        let mut guard = self.by_project.lock().unwrap_or_else(|e| e.into_inner());
        let mesh = guard
            .entry(project.to_string())
            .or_insert_with(|| Mesh::new(Limits::default()));
        Self::sync(mesh, view, project, now_ms);
        mesh.release_orphans();
        mesh.expire_leases(now_ms);

        let mut target: Option<(String, String)> = None;
        if tool == "handoff" {
            let Some(to) = args.get("to").and_then(Value::as_str) else {
                return bad("to");
            };
            // A bare name is an agent on the caller's own node.
            let qualified = if to.contains('/') {
                to.to_string()
            } else {
                format!("{node}/{to}")
            };
            if let Some((n, a)) = qualified.split_once('/') {
                target = Some((n.to_string(), a.to_string()));
            }
            // Fail before anything changes: a handoff to a node that cannot take the message must not move the task.
            if let Some((n, _)) = &target {
                if !view.nodes.get(n).is_some_and(|v| v.online) {
                    let r = tool_error(ErrorCode::NodeOffline, "node_id", n);
                    return answer(Verdict::Done {
                        result: r,
                        is_error: true,
                    });
                }
            }
            args["to"] = json!(qualified);
        }

        let Some(result) = board_tool(&AgentId::new(&caller), tool, &args, mesh, now_ms) else {
            return bad("arguments_json");
        };
        let is_error = result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let events = mesh.take_events();
        let handoff = match (&target, is_error, tool) {
            (Some((to_node, to_agent)), false, "handoff") => Some(Handoff {
                project_id: project.to_string(),
                task_id: args
                    .get("task_id")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                to_node: to_node.clone(),
                to_agent: to_agent.clone(),
                from_node: node.to_string(),
                from_agent: agent.to_string(),
                body: args
                    .get("summary")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                refs: args
                    .get("refs")
                    .and_then(Value::as_array)
                    .map(|r| {
                        r.iter()
                            .filter_map(|x| x.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
            }),
            _ => None,
        };
        Answer {
            verdict: Verdict::Done { result, is_error },
            events,
            handoff,
            fanout: None,
            notices: Vec::new(),
        }
    }
}

/// The word an agent's state is in an agent's answer (the UI word).
fn state_word(s: agora_protocol::v1::AgentState) -> &'static str {
    use agora_protocol::v1::AgentState;
    match s {
        AgentState::Working => "working",
        AgentState::Blocked => "blocked",
        AgentState::Idle => "idle",
        AgentState::Done => "done",
        AgentState::Unspecified => "unknown",
    }
}

/// The folder an agent runs in on its machine: its worktree's, or for an agent started in a folder (a workspace, ADR-0052, as every
/// helper is, ADR-0057), which has no worktree, that workspace's (F1820). Empty when the hub knows neither.
fn folder_of<'a>(view: &'a View, key: &str, a: &'a crate::projection::AgentView) -> &'a str {
    if let Some(w) = view.worktrees.get(&a.worktree_id) {
        return &w.path;
    }
    view.agent_workspaces
        .get(key)
        .and_then(|w| view.workspaces.get(w))
        .map_or("", |w| w.path.as_str())
}

/// ADR-0112: at most this many machines and agents in a `fleet_status` answer; past either the answer says `partial: true`.
pub const FLEET_MAX_MACHINES: usize = 64;
pub const FLEET_MAX_AGENTS: usize = 200;
/// The longest machine id, agent id, kind, project and folder in a `fleet_status` answer (bytes).
const FLEET_ID_MAX: usize = 160;
const FLEET_PATH_MAX: usize = 512;
/// ADR-0112: the most bytes the rows of one `fleet_status` answer take, serialized; past it the answer is cut and says `partial: true`.
/// The answer travels as a JSON string inside the MCP result, where each `"` or `\` of it becomes two bytes (control characters are
/// cleaned out first), so twice this, with room for the wrapping, stays under the hub's limit for a board result.
pub const FLEET_MAX_BYTES: usize = 384 * 1024;
const _: () = assert!(2 * FLEET_MAX_BYTES + 1024 < crate::server::MAX_BOARD_RESULT);

/// ADR-0112: `fleet_status`, the hub's read-only view for agents of every machine it knows and every running agent on an online
/// machine. Only facts the hub projects from node events: never a task, a label, `doing`, a message or a file name. A machine the
/// owner forgot is left out, as it is for windows. Who may see it is the caller's to decide ([`crate::hub::Hub::board_call`]).
/// Every text is put on one line without control or invisible characters (bidi marks included) and cut, as the node does
/// (`agora_mesh::awareness::clean`); at most [`FLEET_MAX_MACHINES`] machines, [`FLEET_MAX_AGENTS`] agents and [`FLEET_MAX_BYTES`].
///
/// Amendment A: only machines in `agreed` (their latest link agreed on `mesh.fleet_status.v1`) are listed, and only their agents. A
/// machine whose policy says `mesh.fleet = false`, or an older node, does not agree, so it and its agents are left out.
pub fn fleet(view: &View, agreed: &BTreeSet<String>) -> Value {
    fleet_within(view, agreed, FLEET_MAX_BYTES)
}

fn fleet_within(view: &View, agreed: &BTreeSet<String>, max_bytes: usize) -> Value {
    use agora_mesh::awareness::clean;
    let mut partial = false;
    let mut room = max_bytes;
    // Takes `row` while it fits in what is left; a row that does not fit ends its list.
    let mut fits = |row: &Value| {
        let n = row.to_string().len() + 1;
        let ok = n <= room;
        if ok {
            room -= n;
        }
        ok
    };
    let mut machines: Vec<Value> = Vec::new();
    for (id, n) in view
        .nodes
        .iter()
        .filter(|(id, n)| !n.forgotten && agreed.contains(*id))
    {
        let row = json!({
            "id": clean(id, FLEET_ID_MAX), "online": n.online, "os": clean(&n.os, 64),
            "build": clean(&n.build, 64), "agents_running": n.agents_running,
            "last_seen_ms": n.last_event_ms,
        });
        if machines.len() == FLEET_MAX_MACHINES || !fits(&row) {
            partial = true;
            break;
        }
        machines.push(row);
    }
    let mut agents: Vec<Value> = Vec::new();
    for (key, a) in &view.agents {
        // The machine is the authenticated producer named in the hub-wide key, never the payload's `node_id`.
        let Some((node, _)) = key.split_once('/') else {
            continue;
        };
        if !a.running
            || a.forgotten
            || !agreed.contains(node)
            || !view
                .nodes
                .get(node)
                .is_some_and(|n| n.online && !n.forgotten)
        {
            continue;
        }
        let parent =
            (!a.parent_agent_id.is_empty()).then(|| clean(&a.parent_agent_id, FLEET_ID_MAX));
        let row = json!({
            "agent_id": clean(key, FLEET_ID_MAX), "kind": clean(&a.kind, 64), "machine": clean(node, FLEET_ID_MAX),
            "project": clean(&a.project_id, FLEET_ID_MAX), "state": state_word(a.state),
            "workspace": clean(folder_of(view, key, a), FLEET_PATH_MAX),
            "parent": parent,
        });
        if agents.len() == FLEET_MAX_AGENTS || !fits(&row) {
            partial = true;
            break;
        }
        agents.push(row);
    }
    json!({"machines": machines, "agents": agents, "partial": partial})
}

/// [`fleet`] as the MCP tool result a `BoardCall` answers with.
pub fn fleet_answer(view: &View, agreed: &BTreeSet<String>) -> Answer {
    answer(Verdict::Done {
        result: json!({"content": [{"type": "text", "text": fleet(view, agreed).to_string()}], "isError": false}),
        is_error: false,
    })
}

/// ADR-0112: the refusal of `fleet_status` on a hub with people besides the owner (no parameters).
pub fn fleet_refused() -> Answer {
    let text = json!({"error": {"code": ErrorCode::BoardNotAllowed.as_str(), "params": {}}});
    answer(Verdict::Done {
        result: json!({"content": [{"type": "text", "text": text.to_string()}], "isError": true}),
        is_error: true,
    })
}

/// `s` cut to at most `max` bytes on a character boundary.
fn cut(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

impl Boards {
    /// ADR-0039 amendment B. Compares the online nodes with a checkout of each project with the last look and returns the
    /// `(node, project)` pairs that must now hand over their local work: every such node when a project becomes shared (reaches
    /// two nodes), and a node that joins an already shared project. Calls that grant something then wait (see [`JOIN_WAIT_MS`]).
    pub fn membership(&self, view: &View, now_ms: u64) -> Vec<(String, String)> {
        let mut members = BTreeMap::<String, BTreeSet<String>>::new();
        for (node, project) in project_memberships(view) {
            if view.nodes.get(node).is_some_and(|n| n.online) {
                members
                    .entry(project.to_string())
                    .or_default()
                    .insert(node.to_string());
            }
        }
        let mut j = self.joins.lock().unwrap_or_else(|e| e.into_inner());
        let mut out = Vec::new();
        for (project, nodes) in &members {
            if nodes.len() < 2 {
                continue;
            }
            let join: Vec<String> =
                if let Some(before) = j.members.get(project).filter(|p| p.len() >= 2) {
                    nodes.difference(before).cloned().collect()
                } else {
                    nodes.iter().cloned().collect()
                };
            if join.is_empty() {
                continue;
            }
            let entry = j
                .pending
                .entry(project.clone())
                .or_insert_with(|| (0, BTreeSet::new()));
            // A hold that is still running keeps its end: joins one after another never hold grants longer than one wait.
            if entry.0 <= now_ms {
                entry.0 = now_ms.saturating_add(JOIN_WAIT_MS);
            }
            entry.1.extend(join.iter().cloned());
            out.extend(join.into_iter().map(|n| (n, project.clone())));
        }
        // A project can lose its last fact when a folder is reassigned and its last agent ends.
        j.members = members;
        out
    }

    /// #1036: `nodes` went offline since the last look (see [`Boards::membership`]): they are no longer remembered as members of
    /// any project, as a look while they were offline would have recorded, so each is asked again when it is back.
    pub fn forget_members(&self, nodes: &BTreeSet<String>) {
        if nodes.is_empty() {
            return;
        }
        let mut j = self.joins.lock().unwrap_or_else(|e| e.into_inner());
        for members in j.members.values_mut() {
            members.retain(|n| !nodes.contains(n));
        }
    }

    /// `node` will not answer for `project` (it is offline, or too old to know `BoardJoin`): calls stop waiting for it.
    pub fn join_settled(&self, node: &str, project: &str) {
        let mut j = self.joins.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, nodes)) = j.pending.get_mut(project) {
            nodes.remove(node);
            if nodes.is_empty() {
                j.pending.remove(project);
            }
        }
    }

    /// Registers [`BOARD_AGENT`] on a project's board so it can be named as a creator (the next sync removes it; tasks stay).
    fn board_agent(mesh: &mut Mesh, project: &str) {
        let id = AgentId::new(BOARD_AGENT);
        if !mesh.agent_ids().contains(&id) {
            mesh.register(AgentInfo {
                id,
                kind: "board".into(),
                node: crate::ca::RESERVED_HUB_ID.into(),
                project: project.to_string(),
                label: BOARD_AGENT.into(),
                worktree: String::new(),
                state: "idle".into(),
            });
        }
    }

    fn joining(&self, project: &str, now_ms: u64) -> bool {
        let mut j = self.joins.lock().unwrap_or_else(|e| e.into_inner());
        if j.pending
            .get(project)
            .is_some_and(|(until, _)| now_ms >= *until)
        {
            j.pending.remove(project);
            return false;
        }
        j.pending
            .get(project)
            .is_some_and(|(_, nodes)| !nodes.is_empty())
    }

    /// Puts what `node` handed over for `project` on the board (ADR-0039 amendment B): each lease for its holder, refused (and both
    /// holders told) where the board already has a lease on an overlapping path; each task created anew and claimed again by its
    /// owner, who is told the new id. Returns the board's audit events and the notices to send.
    pub fn joined(
        &self,
        view: &View,
        node: &str,
        project: &str,
        work: &agora_protocol::v1::BoardJoinResult,
        now_ms: u64,
    ) -> (Vec<Body>, Vec<Notice>) {
        self.join_settled(node, project);
        let mut notices = Vec::new();
        let notice = |to: &str, body: Value| -> Option<Notice> {
            let (to_node, to_agent) = to.split_once('/')?;
            Some(Notice {
                to_node: to_node.to_string(),
                to_agent: to_agent.to_string(),
                body: body.to_string(),
            })
        };
        let mut guard = self.by_project.lock().unwrap_or_else(|e| e.into_inner());
        let mesh = guard
            .entry(project.to_string())
            .or_insert_with(|| Mesh::new(Limits::default()));
        Self::sync(mesh, view, project, now_ms);
        mesh.expire_leases(now_ms);
        let max_text = mesh.limits().max_body_bytes;
        for l in work.leases.iter().take(MAX_JOIN_LEASES) {
            if l.expires_at_ms <= now_ms {
                continue;
            }
            let holder = format!("{node}/{}", l.agent_id);
            match mesh.lease_acquire(
                &AgentId::new(&holder),
                std::slice::from_ref(&l.path),
                (l.expires_at_ms - now_ms).min(MAX_JOIN_LEASE_MS),
                now_ms,
            ) {
                Ok(_) => {}
                Err(agora_mesh::broker::MeshError::LeaseHeld { holder: other, .. }) => {
                    let other = other.as_str().to_string();
                    notices.extend(notice(
                        &holder,
                        json!({"board": "lease_refused", "path": l.path, "holder": other}),
                    ));
                    notices.extend(notice(
                        &other,
                        json!({"board": "lease_conflict", "path": l.path, "other": holder}),
                    ));
                }
                // The holder is not running any more (or the path is not one the board takes): nothing to keep.
                Err(_) => {}
            }
        }
        let running: BTreeSet<AgentId> = mesh.agent_ids().into_iter().collect();
        for t in work.tasks.iter().take(MAX_JOIN_TASKS) {
            let (title, detail) = (cut(&t.title, max_text), cut(&t.detail, max_text));
            // The node already let the task go, so it must land: when its creator is not running (it ended, or the hub has not
            // seen it yet) the board itself is named as the creator.
            let creator = AgentId::new(format!("{node}/{}", t.creator_agent_id));
            let creator = if running.contains(&creator) {
                creator
            } else {
                Self::board_agent(mesh, project);
                AgentId::new(BOARD_AGENT)
            };
            let Ok(id) = mesh.task_create(&creator, title, detail, &[]) else {
                continue;
            };
            let owner = AgentId::new(format!("{node}/{}", t.owner_agent_id));
            // A task whose owner ended is open on the board, as it would be after any agent ends.
            let told = if !t.owner_agent_id.is_empty() && running.contains(&owner) {
                let _ = mesh.task_claim(&owner, id.as_str());
                Some(owner)
            } else {
                (creator.as_str() != BOARD_AGENT).then_some(creator)
            };
            if let Some(told) = told {
                notices.extend(notice(
                    told.as_str(),
                    json!({"board": "task_moved", "from": t.task_id, "to": id.as_str()}),
                ));
            }
        }
        (mesh.take_events(), notices)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::projection::{AgentView, NodeView};
    use agora_protocol::v1::AgentState;

    /// #1150 (ADR-0115 note 4): a dependency on a task its own board no longer has stays unmet after a merge. It never names another
    /// board's task with that id, whichever board came first; its new id is one no task has, and new tasks never get it.
    #[test]
    fn a_merged_dependency_never_names_another_boards_task() {
        let board = |next: u64, tasks: Value| {
            json!({"next": next, "tasks": tasks, "leases": []})
                .to_string()
                .into_bytes()
        };
        let task = |id: &str, title: &str, deps: &[&str]| json!({"id": id, "title": title, "detail": "", "depends_on": deps, "done": false, "owner": null, "summary": ""});
        // The later board's `t-3` waits for its own `t-1`, gone; the first board's `t-5` waits for its own `t-2`, gone, which the
        // later board has (and keeps: no earlier board had it).
        let first = board(
            6,
            json!([
                task("t-1", "first one", &[]),
                task("t-5", "first five", &["t-2"])
            ]),
        );
        let later = board(
            4,
            json!([
                task("t-2", "later two", &[]),
                task("t-3", "later three", &["t-1"])
            ]),
        );
        let m = Boards::default().merge_snapshots(&[first, later], 0, |_| true);
        let v: Value = serde_json::from_slice(&m.bytes).unwrap();
        let tasks = v["tasks"].as_array().unwrap();
        let by_title = |t: &str| tasks.iter().find(|x| x["title"] == t).unwrap();
        let ids: BTreeSet<&str> = tasks.iter().map(|t| t["id"].as_str().unwrap()).collect();
        assert_eq!(by_title("later two")["id"], "t-2");
        assert_eq!(by_title("first one")["id"], "t-1");
        let next = v["next"].as_u64().unwrap();
        for (title, gone) in [("later three", "t-1"), ("first five", "t-2")] {
            let dep = by_title(title)["depends_on"][0]
                .as_str()
                .unwrap()
                .to_string();
            assert_ne!(dep, gone, "{title} names another board's task: {v}");
            assert!(!ids.contains(dep.as_str()), "{title} waits for a task: {v}");
            let n: u64 = dep.strip_prefix("t-").unwrap().parse().unwrap();
            assert!(n < next, "a new task could get {dep}: {v}");
        }
        assert!(
            m.notices.is_empty() && m.restated.is_empty(),
            "no id was taken twice"
        );
        // Loaded, both still wait.
        let b = Boards::default();
        b.load("p", &m.bytes, 0);
        let entries = b.by_project.lock().unwrap()["p"].export_tasks().1;
        for title in ["later three", "first five"] {
            let e = entries.iter().find(|e| e.title == title).unwrap();
            assert!(e.depends_on.iter().all(|d| !ids.contains(d.as_str())));
        }
    }

    fn merge_lease(path: &str, holder: &str, expires: u64) -> Value {
        json!({"path": path, "holder": holder, "expires_at_ms": expires})
    }

    fn lease_board(leases: Value) -> Vec<u8> {
        json!({"next": 1, "tasks": [], "leases": leases})
            .to_string()
            .into_bytes()
    }

    /// A merged board's leases as `(path, holder, live)`, and the `(path, holder)` loading it says were released.
    type MergedLeases = (Vec<(String, String, bool)>, Vec<(String, String)>);

    /// The merged board's leases, and what loading it says was released (see [`MergedLeases`]).
    fn merged_leases(b: &Boards, m: &Merged, now: u64) -> MergedLeases {
        let v: Value = serde_json::from_slice(&m.bytes).unwrap();
        let leases = v["leases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| {
                (
                    l["path"].as_str().unwrap().to_string(),
                    l["holder"].as_str().unwrap().to_string(),
                    l["expires_at_ms"].as_u64().unwrap() > now,
                )
            })
            .collect();
        b.load("p", &m.bytes, now);
        let released = b
            .by_project
            .lock()
            .unwrap()
            .get_mut("p")
            .unwrap()
            .take_events()
            .into_iter()
            .filter_map(|e| match e {
                Body::LeaseReleased(r) => Some((r.path, r.agent_id)),
                _ => None,
            })
            .collect();
        (leases, released)
    }

    fn told(node: &str, agent: &str, body: Value) -> Notice {
        Notice {
            to_node: node.into(),
            to_agent: agent.into(),
            body: body.to_string(),
        }
    }

    /// #1150 (ADR-0115 note 4, first half): when two boards' live leases hold one path, the first board's holder keeps it and both
    /// are told (`lease_refused`, `lease_conflict`), as after a join; the other lease is passed on as ended, so loading says it was
    /// released. An ended lease never keeps a path from a live one.
    #[test]
    fn a_merged_lease_on_a_held_path_tells_both_holders() {
        let first = lease_board(json!([
            merge_lease("src/a", "n1/x", 500),
            merge_lease("src/b", "n1/x", 50)
        ]));
        let later = lease_board(json!([
            merge_lease("src/a", "n2/y", 500),
            merge_lease("src/b", "n2/y", 500),
            merge_lease("src/a", "n1/x", 500)
        ]));
        let b = Boards::default();
        let m = b.merge_snapshots(&[first, later], 100, |_| true);
        let (leases, released) = merged_leases(&b, &m, 100);
        let live: Vec<(String, String)> = leases
            .iter()
            .filter(|l| l.2)
            .map(|l| (l.0.clone(), l.1.clone()))
            .collect();
        assert_eq!(
            live,
            [
                ("src/a".to_string(), "n1/x".to_string()),
                ("src/b".into(), "n2/y".into())
            ]
        );
        assert_eq!(
            m.notices,
            [
                told(
                    "n2",
                    "y",
                    json!({"board": "lease_refused", "path": "src/a", "holder": "n1/x"})
                ),
                told(
                    "n1",
                    "x",
                    json!({"board": "lease_conflict", "path": "src/a", "other": "n2/y"})
                ),
            ]
        );
        let mut released = released;
        released.sort();
        assert_eq!(
            released,
            [
                ("src/a".to_string(), "n2/y".to_string()),
                ("src/b".into(), "n1/x".into())
            ]
        );
    }

    /// #1150: different paths that overlap (a folder and a file in it) are one path to a lease. When two boards' live leases hold
    /// such paths for different holders, the rule for one path applies: the running holder keeps its lease, the other one's is passed
    /// on as ended (loading says it was released, so the window stops showing it) and both are told. One holder's own overlapping
    /// leases, and paths that only share a prefix (`lib/a.rs` and `lib/ab.rs`), stay.
    #[test]
    fn a_merged_lease_on_an_overlapping_path_tells_both_holders() {
        let first = lease_board(json!([
            merge_lease("src", "n1/gone", 500),
            merge_lease("lib/a.rs", "n1/x", 500)
        ]));
        let later = lease_board(json!([
            merge_lease("src/a.rs", "n2/y", 500),
            merge_lease("lib", "n2/y", 500),
            merge_lease("lib/ab.rs", "n2/z", 500),
            merge_lease("lib/a.rs/x", "n1/x", 500)
        ]));
        let b = Boards::default();
        let m = b.merge_snapshots(&[first, later], 100, |h| h != "n1/gone");
        let (leases, released) = merged_leases(&b, &m, 100);
        let mut live: Vec<(String, String)> = leases
            .iter()
            .filter(|l| l.2)
            .map(|l| (l.0.clone(), l.1.clone()))
            .collect();
        live.sort();
        assert_eq!(
            live,
            [
                ("lib/a.rs".to_string(), "n1/x".to_string()),
                ("lib/a.rs/x".into(), "n1/x".into()),
                ("lib/ab.rs".into(), "n2/z".into()),
                ("src/a.rs".into(), "n2/y".into()),
            ]
        );
        let mut released = released;
        released.sort();
        assert_eq!(
            released,
            [
                ("lib".to_string(), "n2/y".to_string()),
                ("src".into(), "n1/gone".into())
            ]
        );
        let mut held: Vec<(String, String)> = b.by_project.lock().unwrap()["p"]
            .export_leases("p", 100)
            .into_iter()
            .map(|l| (l.0, l.1.as_str().to_string()))
            .collect();
        held.sort();
        assert_eq!(held, live, "every kept lease is held after loading");
        let mut notices = m.notices.clone();
        notices.sort_by(|a, b| (&a.to_agent, &a.body).cmp(&(&b.to_agent, &b.body)));
        assert_eq!(
            notices,
            [
                told(
                    "n1",
                    "gone",
                    json!({"board": "lease_refused", "path": "src", "holder": "n2/y"})
                ),
                told(
                    "n1",
                    "x",
                    json!({"board": "lease_conflict", "path": "lib", "other": "n2/y"})
                ),
                told(
                    "n2",
                    "y",
                    json!({"board": "lease_conflict", "path": "src", "other": "n1/gone"})
                ),
                told(
                    "n2",
                    "y",
                    json!({"board": "lease_refused", "path": "lib", "holder": "n1/x"})
                ),
            ]
        );
    }

    /// #1150 (review of #1223, H1): an ended lease of any merged board, the folded board's own included, is passed on, so loading says
    /// it was released and a window that rebuilt its view from the log stops showing it held; but not when the holder that keeps
    /// the path is its own (its release would end the kept lease in that view).
    #[test]
    fn a_merge_passes_ended_leases_on_to_be_released() {
        let own = lease_board(json!([
            merge_lease("src/old", "n1/x", 50),
            merge_lease("src/c", "n1/x", 50)
        ]));
        let other = lease_board(json!([merge_lease("src/c", "n1/x", 500)]));
        let b = Boards::default();
        let m = b.merge_snapshots(&[own, other], 100, |_| true);
        let (leases, released) = merged_leases(&b, &m, 100);
        assert_eq!(released, [("src/old".to_string(), "n1/x".to_string())]);
        assert!(leases.contains(&("src/c".into(), "n1/x".into(), true)));
        assert!(m.notices.is_empty());
    }

    /// #1150 (review of #1223, L2): a lease whose holder is gone (not running in the project, or on a node that started again since
    /// the hub opened) does not keep a path from a later board's running holder: `load` would free it and leave the path to no one.
    #[test]
    fn a_merged_lease_of_a_gone_holder_gives_way_to_a_running_one() {
        for (first_holder, running) in [("n1/x", false), ("n3/z", true)] {
            let first = lease_board(json!([merge_lease("src/a", first_holder, 500)]));
            let later = lease_board(json!([merge_lease("src/a", "n2/y", 500)]));
            let b2 = Boards::default();
            b2.node_restarted("n3", 0);
            let m = b2.merge_snapshots(&[first, later], 100, |h| running || h != first_holder);
            let (leases, released) = merged_leases(&b2, &m, 100);
            assert!(
                leases.contains(&("src/a".into(), "n2/y".into(), true)),
                "{first_holder}: {leases:?}"
            );
            assert_eq!(released, [("src/a".to_string(), first_holder.to_string())]);
            let held = b2.by_project.lock().unwrap()["p"].export_leases("p", 100);
            assert_eq!(held.len(), 1);
            assert_eq!(held[0].1.as_str(), "n2/y");
        }
    }

    /// #1150 (review of #1223, L1): the owner of a done task that got a new id is not told (it no longer works on it).
    #[test]
    fn a_done_task_moved_by_a_merge_tells_no_one() {
        let task = |title: &str, done: bool| json!({"id": "t-1", "title": title, "detail": "", "depends_on": [], "done": done, "owner": "n2/y", "summary": ""});
        let board = |t: Value| {
            json!({"next": 2, "tasks": [t], "leases": []})
                .to_string()
                .into_bytes()
        };
        let b = Boards::default();
        let m = b.merge_snapshots(
            &[
                board(task("first", false)),
                board(task("later, done", true)),
            ],
            0,
            |_| true,
        );
        assert!(m.notices.is_empty(), "{:?}", m.notices);
        assert_eq!(m.restated.len(), 2);
    }

    fn view(online: &[&str]) -> View {
        let mut v = View::default();
        for (i, n) in ["alpha", "beta", "gamma"].iter().enumerate() {
            v.nodes.insert(
                (*n).to_string(),
                NodeView {
                    hostname: (*n).into(),
                    os: String::new(),
                    build: String::new(),
                    binary: String::new(),
                    online: online.contains(n),
                    agents_running: 1,
                    terminals_open: 0,
                    memory_used_bytes: 0,
                    last_event_ms: 0,
                    link_generation: 0,
                    forgotten: false,
                },
            );
            v.checkouts
                .insert(format!("c{i}"), ((*n).into(), "p".into(), "/p".into()));
            v.agents.insert(
                format!("{n}/a"),
                AgentView {
                    run_id: None,
                    started_seq: 0,
                    node_id: (*n).into(),
                    project_id: "p".into(),
                    worktree_id: String::new(),
                    kind: "shell".into(),
                    state: AgentState::Idle,
                    reported: false,
                    session_id: None,
                    running: true,
                    exit_code: None,
                    input_tokens: 0,
                    output_tokens: 0,
                    model: String::new(),
                    forgotten: false,
                    session: crate::projection::SessionView::default(),
                    parent_agent_id: String::new(),
                    interrupted: None,
                    launch_options: Vec::new(),
                    skips_permissions: false,
                    always_allowed: Vec::new(),
                    runs_inside: String::new(),
                },
            );
        }
        v
    }

    fn text(a: &Answer) -> String {
        match &a.verdict {
            Verdict::Done { result, .. } => result["content"][0]["text"]
                .as_str()
                .unwrap_or("")
                .to_string(),
            other => format!("{other:?}"),
        }
    }

    #[test]
    fn a_project_that_becomes_shared_asks_its_nodes_to_join_and_holds_grants_until_they_answer() {
        let b = Boards::new();
        // One node: nothing to join.
        assert!(b.membership(&view(&["alpha"]), 1_000).is_empty());
        // A second node comes online: both are asked.
        let v = view(&["alpha", "beta"]);
        let mut asked = b.membership(&v, 2_000);
        asked.sort();
        assert_eq!(
            asked,
            [
                ("alpha".to_string(), "p".to_string()),
                ("beta".to_string(), "p".to_string())
            ]
        );
        // Nothing changed since: nobody is asked again.
        assert!(b.membership(&v, 2_100).is_empty());
        // While they hand over their work, a lease or a claim is not granted; a list still answers.
        let lease = run(
            &b,
            &v,
            "beta",
            "lease_acquire",
            json!({"paths": ["README.md"], "ttl_s": 60}),
            2_200,
        );
        assert!(
            text(&lease).contains("board.unavailable"),
            "{}",
            text(&lease)
        );
        assert!(!text(&run(&b, &v, "beta", "task_list", json!({}), 2_200))
            .contains("board.unavailable"));
        // Both answered: grants go through.
        b.join_settled("alpha", "p");
        b.join_settled("beta", "p");
        let lease = run(
            &b,
            &v,
            "beta",
            "lease_acquire",
            json!({"paths": ["README.md"], "ttl_s": 60}),
            2_300,
        );
        assert!(text(&lease).contains("lease_id"), "{}", text(&lease));
        // A third node joining an already shared project is asked alone.
        assert_eq!(
            b.membership(&view(&["alpha", "beta", "gamma"]), 3_000),
            [("gamma".to_string(), "p".to_string())]
        );
    }

    #[test]
    fn a_project_that_loses_all_members_asks_both_nodes_when_shared_again() {
        let boards = Boards::new();
        let shared = view(&["alpha", "beta"]);
        let expected = [
            ("alpha".to_string(), "p".to_string()),
            ("beta".to_string(), "p".to_string()),
        ];
        assert_eq!(boards.membership(&shared, 1_000), expected);
        boards.join_settled("alpha", "p");
        boards.join_settled("beta", "p");
        assert!(boards.membership(&View::default(), 2_000).is_empty());
        assert_eq!(boards.membership(&shared, 3_000), expected);
    }

    #[test]
    fn a_node_that_never_answers_holds_grants_only_for_the_join_wait() {
        let b = Boards::new();
        let v = view(&["alpha", "beta"]);
        b.membership(&view(&["alpha"]), 0);
        b.membership(&v, 1_000);
        let early = run(
            &b,
            &v,
            "alpha",
            "lease_acquire",
            json!({"paths": ["x"], "ttl_s": 60}),
            1_000 + JOIN_WAIT_MS - 1,
        );
        assert!(text(&early).contains("board.unavailable"));
        let late = run(
            &b,
            &v,
            "alpha",
            "lease_acquire",
            json!({"paths": ["x"], "ttl_s": 60}),
            1_000 + JOIN_WAIT_MS,
        );
        assert!(text(&late).contains("lease_id"), "{}", text(&late));
    }

    #[test]
    fn f415_join_wait_creation_does_not_overflow_the_clock() {
        let control = Boards::new();
        let shared = view(&["alpha", "beta"]);
        assert_eq!(control.membership(&shared, 1_000).len(), 2);
        println!("CONTROL OK");

        let target = Boards::new();
        let actual = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            target.membership(&shared, u64::MAX - JOIN_WAIT_MS + 1)
        }));
        println!("EXPECTED: membership succeeds");
        println!(
            "ACTUAL: {}",
            if actual.is_ok() {
                "succeeded"
            } else {
                "panicked"
            }
        );
        if actual.is_ok() {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert!(matches!(actual, Ok(pairs) if pairs.len() == 2));
    }

    #[test]
    fn verifies_f415_join_wait_clock_boundaries() {
        for now in [0, u64::MAX - JOIN_WAIT_MS, u64::MAX - JOIN_WAIT_MS + 1] {
            let boards = Boards::new();
            let shared = view(&["alpha", "beta"]);
            assert_eq!(boards.membership(&shared, now).len(), 2);
            assert!(boards.membership(&shared, now).is_empty());
        }
        println!("FIX VERIFIED");
    }

    #[test]
    fn f393_an_ended_join_wait_cannot_return_after_clock_rollback() {
        let b = Boards::new();
        let v = view(&["alpha", "beta"]);
        b.membership(&view(&["alpha"]), 0);
        b.membership(&v, 1_000);
        let early = run(
            &b,
            &v,
            "alpha",
            "lease_acquire",
            json!({"paths": ["early"], "ttl_s": 60}),
            1_000 + JOIN_WAIT_MS - 1,
        );
        assert!(text(&early).contains("board.unavailable"));
        println!("CONTROL OK");

        let ended = run(
            &b,
            &v,
            "alpha",
            "lease_acquire",
            json!({"paths": ["ended"], "ttl_s": 60}),
            1_000 + JOIN_WAIT_MS,
        );
        assert!(text(&ended).contains("lease_id"), "{}", text(&ended));
        let actual = run(
            &b,
            &v,
            "alpha",
            "lease_acquire",
            json!({"paths": ["rollback"], "ttl_s": 60}),
            1_001,
        );
        let granted = text(&actual).contains("lease_id");
        println!("EXPECTED: true");
        println!("ACTUAL: {granted}");
        if granted {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert!(granted, "{}", text(&actual));
    }

    #[test]
    fn verifies_f393_join_wait_deadline_boundaries() {
        fn pending() -> (Boards, View) {
            let boards = Boards::new();
            let shared = view(&["alpha", "beta"]);
            boards.membership(&view(&["alpha"]), 0);
            boards.membership(&shared, 1_000);
            (boards, shared)
        }
        let acquire = |boards: &Boards, shared: &View, path: &str, now| {
            text(&run(
                boards,
                shared,
                "alpha",
                "lease_acquire",
                json!({"paths": [path], "ttl_s": 60}),
                now,
            ))
        };

        let (active, shared) = pending();
        assert!(
            acquire(&active, &shared, "active", 1_000 + JOIN_WAIT_MS - 1)
                .contains("board.unavailable")
        );

        let (exact, shared) = pending();
        assert!(acquire(&exact, &shared, "exact", 1_000 + JOIN_WAIT_MS).contains("lease_id"));
        assert!(acquire(&exact, &shared, "exact-rollback", 1_001).contains("lease_id"));

        let (late, shared) = pending();
        assert!(acquire(&late, &shared, "late", 1_000 + JOIN_WAIT_MS + 500).contains("lease_id"));
        assert!(acquire(&late, &shared, "late-rollback", 1_002).contains("lease_id"));

        let (settled, shared) = pending();
        settled.join_settled("alpha", "p");
        settled.join_settled("beta", "p");
        assert!(acquire(&settled, &shared, "settled", 1_001).contains("lease_id"));
        println!("FIX VERIFIED");
    }

    #[test]
    fn handed_over_work_lands_on_the_board_and_a_conflicting_lease_is_refused_with_both_holders_told(
    ) {
        use agora_protocol::v1::{BoardJoinLease, BoardJoinResult, BoardJoinTask};
        let b = Boards::new();
        let v = view(&["alpha", "beta"]);
        b.membership(&view(&["alpha"]), 0);
        b.membership(&v, 1_000);
        b.join_settled("beta", "p");
        // beta already holds README.md on the board (it answered first).
        b.join_settled("alpha", "p");
        let held = run(
            &b,
            &v,
            "beta",
            "lease_acquire",
            json!({"paths": ["README.md"], "ttl_s": 600}),
            1_100,
        );
        assert!(text(&held).contains("lease_id"));
        let work = BoardJoinResult {
            leases: vec![
                BoardJoinLease {
                    agent_id: "a".into(),
                    path: "README.md".into(),
                    expires_at_ms: 500_000,
                },
                BoardJoinLease {
                    agent_id: "a".into(),
                    path: "docs".into(),
                    expires_at_ms: 500_000,
                },
                // Already over: dropped.
                BoardJoinLease {
                    agent_id: "a".into(),
                    path: "old".into(),
                    expires_at_ms: 1_000,
                },
            ],
            tasks: vec![BoardJoinTask {
                task_id: "t-1".into(),
                title: "made alone".into(),
                detail: "d".into(),
                owner_agent_id: "a".into(),
                creator_agent_id: "a".into(),
            }],
        };
        let (events, notices) = b.joined(&v, "alpha", "p", &work, 1_200);
        assert!(!events.is_empty());
        let bodies: Vec<(String, Value)> = notices
            .iter()
            .map(|n| {
                (
                    format!("{}/{}", n.to_node, n.to_agent),
                    serde_json::from_str(&n.body).unwrap(),
                )
            })
            .collect();
        assert!(bodies.contains(&(
            "alpha/a".into(),
            json!({"board": "lease_refused", "path": "README.md", "holder": "beta/a"})
        )));
        assert!(bodies.contains(&(
            "beta/a".into(),
            json!({"board": "lease_conflict", "path": "README.md", "other": "alpha/a"})
        )));
        let moved = bodies
            .iter()
            .find(|(_, b)| b["board"] == "task_moved")
            .unwrap();
        assert_eq!(moved.0, "alpha/a");
        assert_eq!(moved.1["from"], "t-1");
        // On the board: the task, claimed by its owner under the new id; `docs` held by alpha; README.md still beta's only.
        let list = text(&run(&b, &v, "beta", "task_list", json!({}), 1_300));
        assert!(
            list.contains("made alone") && list.contains("alpha/a"),
            "{list}"
        );
        assert!(list.contains(moved.1["to"].as_str().unwrap()));
        let docs = run(
            &b,
            &v,
            "beta",
            "lease_acquire",
            json!({"paths": ["docs/x.md"], "ttl_s": 60}),
            1_300,
        );
        assert!(text(&docs).contains("alpha/a"), "{}", text(&docs));
        let readme = run(
            &b,
            &v,
            "alpha",
            "lease_acquire",
            json!({"paths": ["README.md"], "ttl_s": 60}),
            1_300,
        );
        assert!(text(&readme).contains("beta/a"), "{}", text(&readme));
    }

    #[test]
    fn a_task_whose_agents_ended_still_lands_and_a_handed_over_lease_is_capped() {
        use agora_protocol::v1::{BoardJoinLease, BoardJoinResult, BoardJoinTask};
        let b = Boards::new();
        let v = view(&["alpha", "beta"]);
        b.membership(&view(&["alpha"]), 0);
        b.membership(&v, 1_000);
        let work = BoardJoinResult {
            leases: vec![BoardJoinLease {
                agent_id: "a".into(),
                path: "far".into(),
                expires_at_ms: u64::MAX,
            }],
            tasks: vec![BoardJoinTask {
                task_id: "t-9".into(),
                title: "x".repeat(100_000),
                detail: String::new(),
                // Neither runs on alpha any more.
                owner_agent_id: "gone".into(),
                creator_agent_id: "gone".into(),
            }],
        };
        let (_, notices) = b.joined(&v, "alpha", "p", &work, 2_000);
        // Nobody to tell: the owner ended; the task is on the board, open, made by the board, its title cut.
        assert!(notices.is_empty(), "{notices:?}");
        let list: Value =
            serde_json::from_str(&text(&run(&b, &v, "beta", "task_list", json!({}), 2_100)))
                .unwrap();
        let t = &list[0];
        assert_eq!(t["status"], "open");
        assert!(t["title"].as_str().unwrap().len() < 100_000);
        // The lease runs at most MAX_JOIN_LEASE_MS (beta answers too, so grants are decided again).
        b.join_settled("beta", "p");
        let held = text(&run(
            &b,
            &v,
            "beta",
            "lease_acquire",
            json!({"paths": ["far"], "ttl_s": 1}),
            2_100,
        ));
        let e: Value = serde_json::from_str(&held).unwrap();
        assert_eq!(
            e["error"]["params"]["expires_at"],
            json!(2_000 + MAX_JOIN_LEASE_MS),
            "{held}"
        );
    }

    #[test]
    fn joins_one_after_another_do_not_hold_grants_longer_than_one_wait() {
        let b = Boards::new();
        b.membership(&view(&["alpha"]), 0);
        b.membership(&view(&["alpha", "beta"]), 1_000);
        // gamma joins while alpha and beta still have not answered: the hold keeps its first end.
        let v = view(&["alpha", "beta", "gamma"]);
        assert_eq!(
            b.membership(&v, 4_000),
            [("gamma".to_string(), "p".to_string())]
        );
        let at_end = run(
            &b,
            &v,
            "alpha",
            "lease_acquire",
            json!({"paths": ["y"], "ttl_s": 60}),
            1_000 + JOIN_WAIT_MS,
        );
        assert!(text(&at_end).contains("lease_id"), "{}", text(&at_end));
    }

    fn run(b: &Boards, v: &View, node: &str, tool: &str, args: Value, now: u64) -> Answer {
        b.call(v, node, "a", "p", tool, &args.to_string(), now)
    }

    fn code(a: &Answer) -> String {
        let Verdict::Done { result, .. } = &a.verdict else {
            panic!("{:?}", a.verdict)
        };
        let text = result["content"][0]["text"].as_str().unwrap();
        serde_json::from_str::<Value>(text).unwrap()["error"]["code"]
            .as_str()
            .unwrap_or("")
            .to_string()
    }

    fn handed_off_task() -> (Boards, View, String, Handoff) {
        let boards = Boards::new();
        let shared = view(&["alpha", "beta", "gamma"]);
        let made = run(
            &boards,
            &shared,
            "alpha",
            "task_create",
            json!({"title": "target", "depends_on": []}),
            1,
        );
        let Verdict::Done { result, .. } = made.verdict else {
            panic!()
        };
        let id = agora_mesh::mcp::tool_result_value(&result).unwrap()["task_id"]
            .as_str()
            .unwrap()
            .to_string();
        run(
            &boards,
            &shared,
            "alpha",
            "task_claim",
            json!({"task_id": id}),
            2,
        );
        let handoff = run(
            &boards,
            &shared,
            "alpha",
            "handoff",
            json!({"to": "beta/a", "task_id": id, "summary": "s", "refs": []}),
            3,
        )
        .handoff
        .unwrap();
        (boards, shared, id, handoff)
    }

    fn saved_task<'a>(snapshot: &'a Value, id: &str) -> &'a Value {
        snapshot["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|task| task["id"] == id)
            .unwrap()
    }

    #[test]
    fn workspace_status_lists_the_projects_agents_on_online_nodes_with_their_board_work() {
        let b = Boards::new();
        let mut v = view(&["alpha", "beta"]);
        v.agents.get_mut("beta/a").unwrap().state = AgentState::Working;
        v.agents.get_mut("beta/a").unwrap().parent_agent_id = "beta/boss".into();
        // Not shared: the node answers from its own broker.
        let single = view(&["alpha"]);
        assert_eq!(
            run(&b, &single, "alpha", "workspace_status", json!({}), 1).verdict,
            Verdict::NotShared
        );
        let t = run(
            &b,
            &v,
            "beta",
            "task_create",
            json!({"title": "ship it"}),
            1,
        );
        let tid = serde_json::from_str::<Value>(&text(&t)).unwrap()["task_id"].clone();
        run(&b, &v, "beta", "task_claim", json!({"task_id": tid}), 2);
        run(
            &b,
            &v,
            "beta",
            "lease_acquire",
            json!({"paths": ["src/a.rs"], "ttl_s": 60}),
            3,
        );
        let a = run(&b, &v, "alpha", "workspace_status", json!({}), 1_000);
        let st: Value = serde_json::from_str(&text(&a)).unwrap();
        assert_eq!(st["machines_online"], 2);
        let agents = st["agents"].as_array().unwrap();
        // gamma is offline: its agent is not listed.
        assert_eq!(agents.len(), 2, "{st}");
        let beta = agents.iter().find(|r| r["id"] == "beta/a").unwrap();
        assert_eq!(beta["state"], "working");
        assert_eq!(beta["task"], "ship it");
        assert_eq!(beta["holds"], 1);
        assert_eq!(beta["parent"], "beta/boss");
        assert_eq!(st["open_tasks"][0]["owner"], "beta/a");
        assert_eq!(st["leases"][0]["path"], "src/a.rs");
        assert_eq!(st["leases"][0]["ttl"], 60);
    }

    #[test]
    fn one_online_node_is_not_a_shared_project_and_a_node_cannot_use_a_project_it_has_no_checkout_of(
    ) {
        let b = Boards::new();
        let v = view(&["alpha"]);
        let a = run(&b, &v, "alpha", "task_list", json!({}), 1);
        assert_eq!(a.verdict, Verdict::NotShared);
        let v = view(&["alpha", "beta"]);
        let a = b.call(&v, "gamma", "a", "p", "task_list", "{}", 1);
        assert_eq!(a.verdict, Verdict::NotShared);
        let a = b.call(&v, "alpha", "a", "p", "send_message", "{}", 1);
        assert!(matches!(a.verdict, Verdict::Invalid(_)));
        let a = b.call(&v, "alpha", "a", "p", "task_list", "[1]", 1);
        assert!(matches!(a.verdict, Verdict::Invalid(_)));
    }

    /// Codex review of #793 (F1221 on the hub): a shared project's `#channel` has the broker's content rules. Refs together over the
    /// body limit are `mesh.body_too_large` with nothing fanned out (the receiving nodes would refuse every copy, unreported); refs
    /// exactly at the limit go out; a ref that is not a string is an invalid call, not silently dropped.
    #[test]
    fn a_shared_channel_refuses_refs_over_the_body_limit_before_fanout() {
        let b = Boards::new();
        let v = view(&["alpha", "beta"]);
        let max = Limits::default().max_body_bytes;
        let refs = |total: usize| json!(["x".repeat(total - max / 2), "y".repeat(max / 2)]);
        let over = run(
            &b,
            &v,
            "alpha",
            "send_message",
            json!({"to": "#all", "body": "hi", "refs": refs(max + 1)}),
            1,
        );
        assert_eq!(code(&over), "mesh.body_too_large", "{}", text(&over));
        assert!(over.fanout.is_none());
        let at = run(
            &b,
            &v,
            "alpha",
            "send_message",
            json!({"to": "#all", "body": "hi", "refs": refs(max)}),
            2,
        );
        assert_eq!(code(&at), "", "{}", text(&at));
        let f = at.fanout.expect("refs at the limit are sent");
        assert_eq!(f.refs.iter().map(String::len).sum::<usize>(), max);
        assert_eq!(f.targets, [("beta".to_string(), "a".to_string())]);
        let not_text = run(
            &b,
            &v,
            "alpha",
            "send_message",
            json!({"to": "#all", "body": "hi", "refs": ["ok", 1]}),
            3,
        );
        assert!(
            matches!(not_text.verdict, Verdict::Invalid(_)),
            "{:?}",
            not_text.verdict
        );
        assert!(not_text.fanout.is_none());
        // The hub is the last guard for the same argument types as the broker: a `reply_to` that is not a string is invalid.
        let bad_reply = run(
            &b,
            &v,
            "alpha",
            "send_message",
            json!({"to": "#all", "body": "hi", "reply_to": 5}),
            4,
        );
        assert!(
            matches!(bad_reply.verdict, Verdict::Invalid(_)),
            "{:?}",
            bad_reply.verdict
        );
        assert!(bad_reply.fanout.is_none());
    }

    /// Codex review of #793: receivers get `[#channel] body`, and each refuses a body over the limit, so the hub checks that text,
    /// not the raw body: exactly at the limit it goes out, one byte over it is `mesh.body_too_large` with nothing fanned out.
    #[test]
    fn a_shared_channel_checks_the_body_receivers_will_see() {
        let b = Boards::new();
        let v = view(&["alpha", "beta"]);
        let max = Limits::default().max_body_bytes;
        let room = max - "[#all] ".len();
        let at = run(
            &b,
            &v,
            "alpha",
            "send_message",
            json!({"to": "#all", "body": "x".repeat(room)}),
            1,
        );
        assert_eq!(code(&at), "", "{}", text(&at));
        assert_eq!(at.fanout.expect("at the limit it is sent").body.len(), max);
        let over = run(
            &b,
            &v,
            "alpha",
            "send_message",
            json!({"to": "#all", "body": "x".repeat(room + 1)}),
            2,
        );
        assert_eq!(code(&over), "mesh.body_too_large", "{}", text(&over));
        assert!(over.fanout.is_none());
    }

    #[test]
    fn a_handoff_to_an_agent_on_the_callers_own_node_is_delivered() {
        let b = Boards::new();
        let mut v = view(&["alpha", "beta"]);
        // A second agent on alpha, as sync registers it: the hub-wide id names the agent.
        let twin = v.agents.get("alpha/a").unwrap().clone();
        v.agents.insert("alpha/b".to_string(), twin);
        let a = run(
            &b,
            &v,
            "alpha",
            "handoff",
            json!({"to": "alpha/b", "summary": "s", "refs": []}),
            1,
        );
        assert_eq!(code(&a), "", "{}", text(&a));
        let h = a.handoff.expect("the handoff must be accepted");
        assert_eq!((h.to_node.as_str(), h.to_agent.as_str()), ("alpha", "b"));
    }

    #[test]
    fn the_board_stays_in_use_after_a_node_drops_while_it_holds_unfinished_work() {
        // ADR-0039 amendment A. `alpha` and `beta` share the project and `alpha` makes a task; `beta` goes offline.
        let b = Boards::new();
        let both = view(&["alpha", "beta"]);
        let made = run(
            &b,
            &both,
            "alpha",
            "task_create",
            json!({"title": "t", "depends_on": []}),
            1,
        );
        let Verdict::Done { result, .. } = made.verdict else {
            panic!()
        };
        let first = agora_mesh::mcp::tool_result_value(&result).unwrap()["task_id"]
            .as_str()
            .unwrap()
            .to_string();
        let alone = view(&["alpha"]);
        // Still the hub's board: the task is listed, a new one gets a new id, a handoff to the offline node says so.
        let listed = run(&b, &alone, "alpha", "task_list", json!({}), 2);
        assert!(
            matches!(listed.verdict, Verdict::Done { .. }),
            "{:?}",
            listed.verdict
        );
        let again = run(
            &b,
            &alone,
            "alpha",
            "task_create",
            json!({"title": "u", "depends_on": []}),
            3,
        );
        let Verdict::Done { result, .. } = again.verdict else {
            panic!("{:?}", again.verdict)
        };
        let second = agora_mesh::mcp::tool_result_value(&result).unwrap()["task_id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_ne!(first, second);
        run(
            &b,
            &alone,
            "alpha",
            "task_claim",
            json!({"task_id": first}),
            4,
        );
        let h = run(
            &b,
            &alone,
            "alpha",
            "handoff",
            json!({"to": "beta/a", "task_id": first, "summary": "s", "refs": []}),
            5,
        );
        assert_eq!(code(&h), "node.offline");
        // Everything done and no lease: one node alone goes back to its local broker.
        run(
            &b,
            &alone,
            "alpha",
            "task_complete",
            json!({"task_id": first, "summary": "d"}),
            6,
        );
        run(
            &b,
            &alone,
            "alpha",
            "task_claim",
            json!({"task_id": second}),
            7,
        );
        run(
            &b,
            &alone,
            "alpha",
            "task_complete",
            json!({"task_id": second, "summary": "d"}),
            8,
        );
        assert_eq!(
            run(&b, &alone, "alpha", "task_list", json!({}), 9).verdict,
            Verdict::NotShared
        );
    }

    #[test]
    fn a_lease_ends_at_its_ttl_even_if_nobody_releases_it() {
        let b = Boards::new();
        let v = view(&["alpha", "beta"]);
        let args = json!({"paths": ["x"], "ttl_s": 10});
        assert_ne!(
            code(&run(&b, &v, "alpha", "lease_acquire", args.clone(), 1_000)),
            "lease.held"
        );
        assert_eq!(
            code(&run(&b, &v, "beta", "lease_acquire", args.clone(), 5_000)),
            "lease.held"
        );
        let a = run(&b, &v, "beta", "lease_acquire", args, 12_000);
        assert_eq!(code(&a), "");
        assert!(a.events.iter().any(|e| matches!(e, Body::LeaseReleased(_))));
    }

    #[test]
    fn an_agent_that_ended_gives_its_task_back() {
        let b = Boards::new();
        let mut v = view(&["alpha", "beta"]);
        let made = run(
            &b,
            &v,
            "alpha",
            "task_create",
            json!({"title": "t", "depends_on": []}),
            1,
        );
        let Verdict::Done { result, .. } = made.verdict else {
            panic!()
        };
        let id = agora_mesh::mcp::tool_result_value(&result).unwrap()["task_id"]
            .as_str()
            .unwrap()
            .to_string();
        run(&b, &v, "alpha", "task_claim", json!({"task_id": id}), 2);
        v.agents.get_mut("alpha/a").unwrap().running = false;
        let a = run(&b, &v, "beta", "task_claim", json!({"task_id": id}), 3);
        assert_eq!(code(&a), "");
    }

    #[test]
    fn a_restarted_node_does_not_inherit_the_claims_of_its_dead_agents() {
        // After a crash the node starts again and its first agent is `a` again: the claim of the one that died must not carry over.
        let b = Boards::new();
        let v = view(&["alpha", "beta"]);
        let made = run(
            &b,
            &v,
            "alpha",
            "task_create",
            json!({"title": "t", "depends_on": []}),
            1,
        );
        let Verdict::Done { result, .. } = made.verdict else {
            panic!()
        };
        let id = agora_mesh::mcp::tool_result_value(&result).unwrap()["task_id"]
            .as_str()
            .unwrap()
            .to_string();
        run(&b, &v, "alpha", "task_claim", json!({"task_id": id}), 2);
        assert_eq!(
            code(&run(
                &b,
                &v,
                "beta",
                "task_claim",
                json!({"task_id": id}),
                3
            )),
            "task.not_open"
        );
        let released = b.node_restarted("alpha", 4);
        assert_eq!(released.len(), 1);
        assert!(!released[0].1.is_empty());
        // `alpha/a` runs again in the view (the new agent), yet the task is open for anyone.
        assert_eq!(
            code(&run(
                &b,
                &v,
                "beta",
                "task_claim",
                json!({"task_id": id}),
                5
            )),
            ""
        );
        // A node that has no agent on any board changes nothing.
        assert!(b.node_restarted("delta", 6).is_empty());
    }

    #[test]
    fn a_handoff_to_an_offline_node_changes_nothing() {
        let b = Boards::new();
        let v = view(&["alpha", "beta"]);
        let made = run(
            &b,
            &v,
            "alpha",
            "task_create",
            json!({"title": "t", "depends_on": []}),
            1,
        );
        let Verdict::Done { result, .. } = made.verdict else {
            panic!()
        };
        let id = agora_mesh::mcp::tool_result_value(&result).unwrap()["task_id"]
            .as_str()
            .unwrap()
            .to_string();
        run(&b, &v, "alpha", "task_claim", json!({"task_id": id}), 2);
        // gamma has a checkout and an agent but is offline.
        let a = run(
            &b,
            &v,
            "alpha",
            "handoff",
            json!({"to": "gamma/a", "task_id": id, "summary": "s", "refs": []}),
            3,
        );
        assert_eq!(code(&a), "node.offline");
        assert!(a.handoff.is_none());
        // Still alpha's: beta cannot take it.
        assert_eq!(
            code(&run(
                &b,
                &v,
                "beta",
                "task_claim",
                json!({"task_id": id}),
                4
            )),
            "task.not_open"
        );
    }

    #[test]
    fn tasks_survive_a_restart_and_a_claim_by_an_agent_that_is_gone_is_given_back() {
        let b = Boards::new();
        let v = view(&["alpha", "beta"]);
        let made = run(
            &b,
            &v,
            "alpha",
            "task_create",
            json!({"title": "keep me", "detail": "d", "depends_on": []}),
            1,
        );
        let Verdict::Done { result, .. } = made.verdict else {
            panic!()
        };
        let id = agora_mesh::mcp::tool_result_value(&result).unwrap()["task_id"]
            .as_str()
            .unwrap()
            .to_string();
        run(&b, &v, "alpha", "task_claim", json!({"task_id": id}), 2);
        let saved = b.snapshot("p", 3).unwrap();

        // A new hub process: same view, empty memory, then the stored board.
        let b2 = Boards::new();
        b2.load("p", &saved, 3);
        let a = run(&b2, &v, "beta", "task_list", json!({}), 3);
        let Verdict::Done { result, .. } = a.verdict else {
            panic!()
        };
        let list = agora_mesh::mcp::tool_result_value(&result).unwrap();
        assert_eq!(
            (list[0]["title"].as_str(), list[0]["owner"].as_str()),
            (Some("keep me"), Some("alpha/a"))
        );
        // New ids do not collide with restored ones.
        let n = run(
            &b2,
            &v,
            "beta",
            "task_create",
            json!({"title": "new", "depends_on": []}),
            4,
        );
        let Verdict::Done { result, .. } = n.verdict else {
            panic!()
        };
        assert_ne!(
            agora_mesh::mcp::tool_result_value(&result).unwrap()["task_id"]
                .as_str()
                .unwrap(),
            id
        );

        // The owner is no longer running after the restart: its claim is given back.
        let b3 = Boards::new();
        b3.load("p", &saved, 5);
        let mut gone = view(&["alpha", "beta"]);
        gone.agents.remove("alpha/a");
        assert_eq!(
            code(&run(
                &b3,
                &gone,
                "beta",
                "task_claim",
                json!({"task_id": id}),
                5
            )),
            ""
        );

        // Garbage in the store never takes the hub down.
        Boards::new().load("p", b"not json", 1);
    }

    #[test]
    fn an_unexpired_lease_survives_a_restart_and_an_expired_one_does_not() {
        let b = Boards::new();
        let v = view(&["alpha", "beta"]);
        run(
            &b,
            &v,
            "alpha",
            "lease_acquire",
            json!({"paths": ["keep"], "ttl_s": 100}),
            1_000,
        );
        run(
            &b,
            &v,
            "alpha",
            "lease_acquire",
            json!({"paths": ["gone"], "ttl_s": 5}),
            1_000,
        );
        let saved = b.snapshot("p", 2_000).unwrap();
        let b2 = Boards::new();
        // The hub is back at t = 10 s: "gone" ended at 6 s, "keep" lasts until 101 s.
        b2.load("p", &saved, 10_000);
        assert_eq!(
            code(&run(
                &b2,
                &v,
                "beta",
                "lease_acquire",
                json!({"paths": ["keep"], "ttl_s": 10}),
                10_001
            )),
            "lease.held"
        );
        assert_eq!(
            code(&run(
                &b2,
                &v,
                "beta",
                "lease_acquire",
                json!({"paths": ["gone"], "ttl_s": 10}),
                10_001
            )),
            ""
        );
    }

    #[test]
    fn a_restored_lease_is_freed_when_its_holder_ends() {
        // A lease restored from the store belongs to a grant again: its holder's end frees it, as it frees a live one.
        let b = Boards::new();
        let v = view(&["alpha", "beta"]);
        let args = json!({"paths": ["src/x.rs"], "ttl_s": 3600});
        run(&b, &v, "alpha", "lease_acquire", args.clone(), 1_000);
        let saved = b.snapshot("p", 1_000).unwrap();
        let mut gone = v.clone();
        gone.agents.get_mut("alpha/a").unwrap().running = false;
        for seen_running_first in [true, false] {
            let b2 = Boards::new();
            b2.load("p", &saved, 2_000);
            if seen_running_first {
                let held = run(&b2, &v, "beta", "lease_acquire", args.clone(), 2_100);
                assert_eq!(code(&held), "lease.held");
                assert!(!held
                    .events
                    .iter()
                    .any(|e| matches!(e, Body::LeaseGranted(_))));
            }
            let a = run(&b2, &gone, "beta", "lease_acquire", args.clone(), 3_000);
            assert_eq!(code(&a), "", "seen_running_first={seen_running_first}");
            assert!(a
                .events
                .iter()
                .any(|e| matches!(e, Body::LeaseReleased(l) if l.agent_id == "alpha/a")));
        }
    }

    #[test]
    fn a_board_restored_after_its_node_restarted_does_not_hand_dead_agents_work_to_reused_ids() {
        // The hub reopens, alpha's process starts again (its first agent is `a` again), and only then is the board restored.
        let b = Boards::new();
        let v = view(&["alpha", "beta"]);
        let mut ids = Vec::new();
        for node in ["alpha", "beta"] {
            let made = run(&b, &v, node, "task_create", json!({"title": node}), 1);
            let Verdict::Done { result, .. } = made.verdict else {
                panic!()
            };
            let id = agora_mesh::mcp::tool_result_value(&result).unwrap()["task_id"]
                .as_str()
                .unwrap()
                .to_string();
            run(&b, &v, node, "task_claim", json!({"task_id": id}), 2);
            ids.push(id);
        }
        run(
            &b,
            &v,
            "alpha",
            "lease_acquire",
            json!({"paths": ["src"], "ttl_s": 3600}),
            3,
        );
        let saved = b.snapshot("p", 3).unwrap();
        let b2 = Boards::new();
        assert!(b2.node_restarted("alpha", 10).is_empty());
        b2.load("p", &saved, 20);
        // The new alpha/a does not own the dead one's task; it is open, and its path is free.
        assert_eq!(
            code(&run(
                &b2,
                &v,
                "alpha",
                "task_complete",
                json!({"task_id": ids[0], "summary": "x"}),
                30
            )),
            "task.not_owner"
        );
        assert_eq!(
            code(&run(
                &b2,
                &v,
                "beta",
                "task_claim",
                json!({"task_id": ids[0]}),
                31
            )),
            "",
        );
        assert_eq!(
            code(&run(
                &b2,
                &v,
                "beta",
                "lease_acquire",
                json!({"paths": ["src/x.rs"], "ttl_s": 60}),
                32
            )),
            ""
        );
        // beta did not restart: its claim stays.
        assert_eq!(
            code(&run(
                &b2,
                &v,
                "alpha",
                "task_claim",
                json!({"task_id": ids[1]}),
                33
            )),
            "task.not_open"
        );
    }

    #[test]
    fn a_handoff_that_could_not_be_delivered_is_undone() {
        let b = Boards::new();
        let v = view(&["alpha", "beta"]);
        let made = run(
            &b,
            &v,
            "alpha",
            "task_create",
            json!({"title": "t", "depends_on": []}),
            1,
        );
        let Verdict::Done { result, .. } = made.verdict else {
            panic!()
        };
        let id = agora_mesh::mcp::tool_result_value(&result).unwrap()["task_id"]
            .as_str()
            .unwrap()
            .to_string();
        run(&b, &v, "alpha", "task_claim", json!({"task_id": id}), 2);
        let a = run(
            &b,
            &v,
            "alpha",
            "handoff",
            json!({"to": "beta/a", "task_id": id, "summary": "s", "refs": []}),
            3,
        );
        let h = a.handoff.unwrap();
        assert_eq!(
            code(&run(
                &b,
                &v,
                "alpha",
                "task_complete",
                json!({"task_id": id, "summary": "x"}),
                4
            )),
            "task.not_owner"
        );
        let events = b.revert_handoff(&h);
        assert!(!events.is_empty());
        // Back with the giver, who can finish it; the receiver cannot.
        assert_eq!(
            code(&run(
                &b,
                &v,
                "beta",
                "task_complete",
                json!({"task_id": id, "summary": "x"}),
                5
            )),
            "task.not_owner"
        );
        assert_eq!(
            code(&run(
                &b,
                &v,
                "alpha",
                "task_complete",
                json!({"task_id": id, "summary": "x"}),
                6
            )),
            ""
        );
    }

    /// Every machine of `v`, as if each one's link agreed on the hub-wide view.
    fn every(v: &View) -> BTreeSet<String> {
        v.nodes.keys().cloned().collect()
    }

    /// ADR-0112 Amendment A: a machine whose link did not agree on `mesh.fleet_status.v1` (policy `mesh.fleet = false`, or an older
    /// node) is left out with its agents, whether it is online or not.
    #[test]
    fn fleet_status_leaves_out_machines_that_did_not_agree() {
        let v = view(&["alpha", "beta", "gamma"]);
        let agreed: BTreeSet<String> = ["alpha", "gamma"].iter().map(|s| s.to_string()).collect();
        let f = fleet(&v, &agreed);
        let ids = |key: &str, field: &str| -> Vec<String> {
            f[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r[field].as_str().unwrap().to_string())
                .collect()
        };
        assert_eq!(ids("machines", "id"), ["alpha", "gamma"]);
        assert_eq!(ids("agents", "agent_id"), ["alpha/a", "gamma/a"]);
        assert!(!f.to_string().contains("beta"), "{f}");
        let none = fleet(&v, &BTreeSet::new());
        assert_eq!(
            none,
            json!({"machines": [], "agents": [], "partial": false})
        );
    }

    /// ADR-0112: `fleet_status` lists every machine the hub knows (offline ones too) and every running agent on an online machine,
    /// in every project, with exactly the agreed fields: nothing an agent wrote, no task, label, `doing`, message or file name.
    #[test]
    fn fleet_status_lists_every_machine_and_running_agent_with_only_the_agreed_fields() {
        let mut v = view(&["alpha", "beta"]);
        v.nodes.get_mut("alpha").unwrap().os = "linux".into();
        v.nodes.get_mut("alpha").unwrap().build = "0.3.40".into();
        v.nodes.get_mut("alpha").unwrap().last_event_ms = 77;
        let beta = v.agents.get_mut("beta/a").unwrap();
        beta.project_id = "other".into();
        beta.state = AgentState::Working;
        beta.parent_agent_id = "beta/boss".into();
        beta.worktree_id = "w1".into();
        v.worktrees.insert(
            "w1".into(),
            crate::projection::WorktreeView {
                checkout_id: String::new(),
                path: "/home/me/other".into(),
                branch: String::new(),
                files_changed: 0,
                insertions: 0,
                deletions: 0,
                head_commit: String::new(),
            },
        );
        // A machine the owner forgot, and an agent that ended: neither is listed.
        v.nodes.get_mut("gamma").unwrap().forgotten = true;
        let mut ended = v.agents["alpha/a"].clone();
        ended.running = false;
        v.agents.insert("alpha/old".into(), ended);
        // Board work that names a task: never in the answer.
        let b = Boards::new();
        run(
            &b,
            &v,
            "alpha",
            "task_create",
            json!({"title": "SECRET-TITLE"}),
            1,
        );

        let f = fleet(&v, &every(&v));
        let machines = f["machines"].as_array().unwrap();
        let ids: Vec<&str> = machines.iter().map(|m| m["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["alpha", "beta"], "{f}");
        assert_eq!(
            machines[0],
            json!({"id": "alpha", "online": true, "os": "linux", "build": "0.3.40", "agents_running": 1, "last_seen_ms": 77})
        );
        let agents = f["agents"].as_array().unwrap();
        let ids: Vec<&str> = agents
            .iter()
            .map(|a| a["agent_id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["alpha/a", "beta/a"], "{f}");
        assert_eq!(
            agents[1],
            json!({"agent_id": "beta/a", "kind": "shell", "machine": "beta", "project": "other", "state": "working",
                   "workspace": "/home/me/other", "parent": "beta/boss"})
        );
        for a in agents {
            let mut keys: Vec<&str> = a.as_object().unwrap().keys().map(String::as_str).collect();
            keys.sort_unstable();
            assert_eq!(
                keys,
                [
                    "agent_id",
                    "kind",
                    "machine",
                    "parent",
                    "project",
                    "state",
                    "workspace"
                ]
            );
        }
        assert_eq!(f["partial"], false);
        let text = f.to_string();
        for absent in ["SECRET-TITLE", "doing", "label", "task", "gamma"] {
            assert!(!text.contains(absent), "{absent} in {text}");
        }
        // An agent on an offline machine is not listed; the machine still is.
        let v = view(&["alpha"]);
        let f = fleet(&v, &every(&v));
        assert_eq!(f["machines"].as_array().unwrap().len(), 3);
        assert_eq!(f["agents"].as_array().unwrap().len(), 1);
    }

    /// ADR-0112: at most 64 machines and 200 agents; a cut list says `partial: true`.
    #[test]
    fn fleet_status_is_cut_at_its_caps_and_says_so() {
        let mut v = view(&["alpha"]);
        let template = v.agents["alpha/a"].clone();
        for i in 0..FLEET_MAX_AGENTS {
            v.agents.insert(format!("alpha/x{i:03}"), template.clone());
        }
        let f = fleet(&v, &every(&v));
        assert_eq!(f["agents"].as_array().unwrap().len(), FLEET_MAX_AGENTS);
        assert_eq!(f["partial"], true);
        // Exactly at the cap is whole.
        v.agents.remove("alpha/a");
        assert_eq!(fleet(&v, &every(&v))["partial"], false);

        let mut v = view(&["alpha"]);
        let node = v.nodes["alpha"].clone();
        for i in 0..FLEET_MAX_MACHINES {
            v.nodes.insert(format!("m{i:03}"), node.clone());
        }
        let f = fleet(&v, &every(&v));
        assert_eq!(f["machines"].as_array().unwrap().len(), FLEET_MAX_MACHINES);
        assert_eq!(f["partial"], true);
    }

    /// F4 of the #1205 review: the hub cleans what nodes reported (control characters, bidi and other invisible marks) and keeps a
    /// full answer well inside its limit for a board result; a byte budget that is reached cuts the answer and says `partial: true`.
    #[test]
    fn fleet_status_text_is_cleaned_and_the_answer_stays_under_the_board_limit() {
        let mut v = view(&["alpha"]);
        let dirty = |n: usize| "a\u{1b}[31m\"\\\u{202e}\u{200b}\n\t".repeat(n).to_string();
        for node in v.nodes.values_mut() {
            node.os = dirty(50);
            node.build = dirty(50);
        }
        let template = v.agents["alpha/a"].clone();
        for i in 0..FLEET_MAX_AGENTS {
            let mut a = template.clone();
            a.kind = dirty(200);
            a.project_id = dirty(200);
            a.parent_agent_id = dirty(200);
            a.worktree_id = "w".into();
            v.agents.insert(format!("alpha/x{i:03}{}", dirty(30)), a);
        }
        v.worktrees.insert(
            "w".into(),
            crate::projection::WorktreeView {
                checkout_id: String::new(),
                path: dirty(500),
                branch: String::new(),
                files_changed: 0,
                insertions: 0,
                deletions: 0,
                head_commit: String::new(),
            },
        );
        let a = fleet_answer(&v, &every(&v));
        let Verdict::Done { result, .. } = &a.verdict else {
            panic!()
        };
        let wire = result.to_string();
        assert!(
            wire.len() < crate::server::MAX_BOARD_RESULT,
            "{}",
            wire.len()
        );
        let text = result["content"][0]["text"].as_str().unwrap();
        let f: Value = serde_json::from_str(text).unwrap();
        assert_eq!(f["agents"].as_array().unwrap().len(), FLEET_MAX_AGENTS);
        // Every text value, not the serialized answer (which escapes control characters anyway).
        fn texts(v: &Value, out: &mut Vec<String>) {
            match v {
                Value::String(s) => out.push(s.clone()),
                Value::Array(a) => a.iter().for_each(|x| texts(x, out)),
                Value::Object(o) => o.values().for_each(|x| texts(x, out)),
                _ => {}
            }
        }
        let mut all = Vec::new();
        texts(&f, &mut all);
        assert!(all.len() > 1000);
        for t in &all {
            assert!(
                !t.chars()
                    .any(|c| c.is_control() || agora_core::text::is_hidden(c)),
                "{t:?}"
            );
        }
        assert!(f["agents"][0]["kind"].as_str().unwrap().len() <= 64);
        assert!(f["agents"][0]["workspace"].as_str().unwrap().len() <= FLEET_PATH_MAX);

        // A small budget cuts the answer and says so.
        let cut = fleet_within(&v, &every(&v), 4096);
        assert_eq!(cut["partial"], true);
        assert!(
            cut.to_string().len() < 4096 + 64,
            "{}",
            cut.to_string().len()
        );
        assert!(cut["agents"].as_array().unwrap().len() < FLEET_MAX_AGENTS);
    }

    /// F1820: an agent started in a folder (a workspace the node registered, ADR-0052, as every helper is, ADR-0057) has no worktree;
    /// its folder is the workspace's. `fleet_status` and `workspace_status` give it as for an agent in a worktree, never empty.
    #[test]
    fn an_agent_in_a_folder_has_that_folder_in_the_hubs_answers() {
        use agora_protocol::v1::{self as v1, Event};
        let ev = |seq: u64, body: Body| Event {
            producer_id: "alpha".into(),
            seq,
            at_ms: seq,
            body: Some(body),
        };
        let mut v = View::default();
        v.apply(&ev(
            1,
            Body::NodeOnline(v1::NodeOnline {
                node_id: "alpha".into(),
                hostname: "alpha".into(),
                ..Default::default()
            }),
        ));
        v.apply(&ev(
            2,
            Body::WorktreeCreated(v1::WorktreeCreated {
                worktree_id: "w1".into(),
                checkout_id: "c1".into(),
                path: "/src/tree".into(),
                branch: "b".into(),
            }),
        ));
        v.apply(&ev(
            3,
            Body::AgentStarted(v1::AgentStarted {
                agent_id: "tree".into(),
                project_id: "p".into(),
                node_id: "alpha".into(),
                worktree_id: "w1".into(),
                kind: "claude".into(),
                ..Default::default()
            }),
        ));
        // As the node records a start in a folder: the workspace, the binding, then the start without a worktree.
        v.apply(&ev(
            4,
            Body::WorkspaceRegistered(v1::WorkspaceRegistered {
                workspace_id: "ws1".into(),
                node_id: "alpha".into(),
                project_id: "p".into(),
                path: "/home/me/folder".into(),
                ..Default::default()
            }),
        ));
        v.apply(&ev(
            5,
            Body::AgentWorkspaceBound(v1::AgentWorkspaceBound {
                agent_id: "helper".into(),
                workspace_id: "ws1".into(),
            }),
        ));
        v.apply(&ev(
            6,
            Body::AgentStarted(v1::AgentStarted {
                agent_id: "helper".into(),
                project_id: "p".into(),
                node_id: "alpha".into(),
                kind: "claude".into(),
                ..Default::default()
            }),
        ));
        let f = fleet(&v, &every(&v));
        let fleet_row = |id: &str| {
            f["agents"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["agent_id"] == id)
                .cloned()
                .unwrap()
        };
        assert_eq!(fleet_row("alpha/tree")["workspace"], "/src/tree");
        assert_eq!(fleet_row("alpha/helper")["workspace"], "/home/me/folder");
        let Verdict::Done { result, .. } = Boards::new().status(&v, "p", 10).verdict else {
            panic!("workspace_status is answered");
        };
        let st: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        let status_row = |id: &str| {
            st["agents"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["id"] == id)
                .cloned()
                .unwrap()
        };
        assert_eq!(status_row("alpha/tree")["workspace"], "/src/tree");
        assert_eq!(status_row("alpha/helper")["workspace"], "/home/me/folder");
    }

    #[test]
    fn f397_a_failed_handoff_does_not_return_a_task_to_a_giver_that_ended() {
        let b = Boards::new();
        let v = view(&["alpha", "beta"]);
        let made = run(
            &b,
            &v,
            "alpha",
            "task_create",
            json!({"title": "target", "depends_on": []}),
            1,
        );
        let Verdict::Done { result, .. } = made.verdict else {
            panic!()
        };
        let id = agora_mesh::mcp::tool_result_value(&result).unwrap()["task_id"]
            .as_str()
            .unwrap()
            .to_string();
        run(&b, &v, "alpha", "task_claim", json!({"task_id": id}), 2);
        let handoff = run(
            &b,
            &v,
            "alpha",
            "handoff",
            json!({"to": "beta/a", "task_id": id, "summary": "s", "refs": []}),
            3,
        )
        .handoff
        .unwrap();
        let mut gone = v.clone();
        gone.agents.get_mut("alpha/a").unwrap().running = false;
        run(&b, &gone, "beta", "task_list", json!({}), 4);
        println!("CONTROL OK");

        b.revert_handoff(&handoff);
        let saved: Value = serde_json::from_slice(&b.snapshot("p", 5).unwrap()).unwrap();
        let actual = saved["tasks"][0]["owner"].as_str();
        println!("EXPECTED: None");
        println!("ACTUAL: {actual:?}");
        if actual.is_none() {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert_eq!(actual, None);
    }

    #[test]
    fn f398_a_failed_handoff_returns_an_open_task_when_only_the_receiver_ended() {
        let b = Boards::new();
        let v = view(&["alpha", "beta"]);
        let made = run(
            &b,
            &v,
            "alpha",
            "task_create",
            json!({"title": "target", "depends_on": []}),
            1,
        );
        let Verdict::Done { result, .. } = made.verdict else {
            panic!()
        };
        let id = agora_mesh::mcp::tool_result_value(&result).unwrap()["task_id"]
            .as_str()
            .unwrap()
            .to_string();
        run(&b, &v, "alpha", "task_claim", json!({"task_id": id}), 2);
        let handoff = run(
            &b,
            &v,
            "alpha",
            "handoff",
            json!({"to": "beta/a", "task_id": id, "summary": "s", "refs": []}),
            3,
        )
        .handoff
        .unwrap();
        let mut gone = v.clone();
        gone.agents.get_mut("beta/a").unwrap().running = false;
        run(&b, &gone, "alpha", "task_list", json!({}), 4);
        let before: Value = serde_json::from_slice(&b.snapshot("p", 4).unwrap()).unwrap();
        assert!(before["tasks"][0]["owner"].is_null());
        println!("CONTROL OK");

        b.revert_handoff(&handoff);
        let saved: Value = serde_json::from_slice(&b.snapshot("p", 5).unwrap()).unwrap();
        let actual = saved["tasks"][0]["owner"].as_str();
        println!("EXPECTED: Some(\"alpha/a\")");
        println!("ACTUAL: {actual:?}");
        if actual == Some("alpha/a") {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert_eq!(actual, Some("alpha/a"));
    }

    #[test]
    fn f399_a_snapshot_whose_next_id_collides_with_a_task_is_ignored() {
        let correct = Boards::new();
        let snapshot = json!({
            "next": 2,
            "tasks": [{"id": "t-1", "title": "kept", "detail": "", "depends_on": [],
                       "done": false, "owner": null, "summary": null}],
            "leases": []
        });
        correct.load("p", snapshot.to_string().as_bytes(), 0);
        let listed = run(
            &correct,
            &view(&["alpha", "beta"]),
            "alpha",
            "task_list",
            json!({}),
            1,
        );
        let list: Value = serde_json::from_str(&text(&listed)).unwrap();
        assert_eq!(list.as_array().unwrap().len(), 1);
        println!("CONTROL OK");

        let damaged = Boards::new();
        let snapshot = json!({
            "next": 1,
            "tasks": [{"id": "t-1", "title": "must not load", "detail": "", "depends_on": [],
                       "done": false, "owner": null, "summary": null}],
            "leases": []
        });
        damaged.load("p", snapshot.to_string().as_bytes(), 0);
        let listed = run(
            &damaged,
            &view(&["alpha", "beta"]),
            "alpha",
            "task_list",
            json!({}),
            1,
        );
        let actual: Value = serde_json::from_str(&text(&listed)).unwrap();
        println!("EXPECTED: []");
        println!("ACTUAL: {actual}");
        if actual.as_array().is_some_and(Vec::is_empty) {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert_eq!(actual, json!([]));
    }

    #[test]
    fn verifies_f397_failed_handoff_giver_liveness_boundaries() {
        let (normal, _, id, handoff) = handed_off_task();
        normal.revert_handoff(&handoff);
        let snapshot: Value = serde_json::from_slice(&normal.snapshot("p", 4).unwrap()).unwrap();
        assert_eq!(saved_task(&snapshot, &id)["owner"], "alpha/a");

        let (giver_gone, mut view, id, handoff) = handed_off_task();
        view.agents.get_mut("alpha/a").unwrap().running = false;
        run(&giver_gone, &view, "beta", "task_list", json!({}), 4);
        giver_gone.revert_handoff(&handoff);
        let snapshot: Value =
            serde_json::from_slice(&giver_gone.snapshot("p", 5).unwrap()).unwrap();
        assert!(saved_task(&snapshot, &id)["owner"].is_null());

        let (both_gone, mut view, id, handoff) = handed_off_task();
        view.agents.get_mut("alpha/a").unwrap().running = false;
        view.agents.get_mut("beta/a").unwrap().running = false;
        run(&both_gone, &view, "gamma", "task_list", json!({}), 4);
        both_gone.revert_handoff(&handoff);
        let snapshot: Value = serde_json::from_slice(&both_gone.snapshot("p", 5).unwrap()).unwrap();
        assert!(saved_task(&snapshot, &id)["owner"].is_null());
        println!("FIX VERIFIED");
    }

    #[test]
    fn verifies_f398_failed_handoff_receiver_race_boundaries() {
        let (receiver_gone, mut view, id, handoff) = handed_off_task();
        view.agents.get_mut("beta/a").unwrap().running = false;
        run(&receiver_gone, &view, "alpha", "task_list", json!({}), 4);
        receiver_gone.revert_handoff(&handoff);
        let snapshot: Value =
            serde_json::from_slice(&receiver_gone.snapshot("p", 5).unwrap()).unwrap();
        assert_eq!(saved_task(&snapshot, &id)["owner"], "alpha/a");

        let (claimed_elsewhere, mut view, id, handoff) = handed_off_task();
        view.agents.get_mut("beta/a").unwrap().running = false;
        run(
            &claimed_elsewhere,
            &view,
            "gamma",
            "task_claim",
            json!({"task_id": id}),
            4,
        );
        claimed_elsewhere.revert_handoff(&handoff);
        let snapshot: Value =
            serde_json::from_slice(&claimed_elsewhere.snapshot("p", 5).unwrap()).unwrap();
        assert_eq!(saved_task(&snapshot, &id)["owner"], "gamma/a");

        let (completed, view, id, handoff) = handed_off_task();
        run(
            &completed,
            &view,
            "beta",
            "task_complete",
            json!({"task_id": id, "summary": "done"}),
            4,
        );
        completed.revert_handoff(&handoff);
        let snapshot: Value = serde_json::from_slice(&completed.snapshot("p", 5).unwrap()).unwrap();
        assert_eq!(saved_task(&snapshot, &id)["owner"], "beta/a");
        assert_eq!(saved_task(&snapshot, &id)["done"], true);
        println!("FIX VERIFIED");
    }

    #[test]
    fn verifies_f399_snapshot_task_id_boundaries() {
        let load = |snapshot: Value| {
            let boards = Boards::new();
            boards.load("p", snapshot.to_string().as_bytes(), 0);
            let listed = run(
                &boards,
                &view(&["alpha", "beta"]),
                "alpha",
                "task_list",
                json!({}),
                1,
            );
            serde_json::from_str::<Value>(&text(&listed)).unwrap()
        };
        let task = |id: &str| {
            json!({"id": id, "title": id, "detail": "", "depends_on": [],
                   "done": false, "owner": null, "summary": null})
        };

        assert_eq!(
            load(json!({"next": 3, "tasks": [task("t-1"), task("t-2")], "leases": []}))
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            load(json!({"next": 2, "tasks": [task("t-2")], "leases": []})),
            json!([])
        );
        assert_eq!(
            load(json!({"next": 2, "tasks": [task("t-1"), task("t-1")], "leases": []})),
            json!([])
        );
        assert_eq!(
            load(json!({"next": 2, "tasks": [task("other")], "leases": []})),
            json!([])
        );
        println!("FIX VERIFIED");
    }

    #[test]
    fn f400_a_snapshot_counter_at_u64_max_is_ignored_before_id_allocation_panics() {
        let valid = Boards::new();
        valid.load(
            "p",
            json!({"next": 1, "tasks": [], "leases": []})
                .to_string()
                .as_bytes(),
            0,
        );
        let control = run(
            &valid,
            &view(&["alpha", "beta"]),
            "alpha",
            "task_create",
            json!({"title": "control", "depends_on": []}),
            1,
        );
        assert_eq!(
            agora_mesh::mcp::tool_result_value(match &control.verdict {
                Verdict::Done { result, .. } => result,
                _ => panic!(),
            })
            .unwrap()["task_id"],
            "t-1"
        );
        println!("CONTROL OK");

        let damaged = Boards::new();
        damaged.load(
            "p",
            json!({"next": u64::MAX, "tasks": [], "leases": []})
                .to_string()
                .as_bytes(),
            0,
        );
        let actual = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run(
                &damaged,
                &view(&["alpha", "beta"]),
                "alpha",
                "task_create",
                json!({"title": "target", "depends_on": []}),
                1,
            )
        }));
        println!("EXPECTED: Ok(t-1)");
        println!("ACTUAL: {}", if actual.is_ok() { "Ok" } else { "panic" });
        if actual.is_ok() {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert!(actual.is_ok());
    }

    #[test]
    fn f401_a_done_snapshot_task_without_an_owner_is_not_reopened() {
        let task = |owner: Value| {
            json!({"id": "t-1", "title": "task", "detail": "", "depends_on": [],
                   "done": true, "owner": owner, "summary": "done"})
        };
        let valid = Boards::new();
        valid.load(
            "p",
            json!({"next": 2, "tasks": [task(json!("alpha/a"))], "leases": []})
                .to_string()
                .as_bytes(),
            0,
        );
        let saved: Value = serde_json::from_slice(&valid.snapshot("p", 0).unwrap()).unwrap();
        assert_eq!(saved["tasks"][0]["done"], true);
        println!("CONTROL OK");

        let damaged = Boards::new();
        damaged.load(
            "p",
            json!({"next": 2, "tasks": [task(Value::Null)], "leases": []})
                .to_string()
                .as_bytes(),
            0,
        );
        let actual = damaged.snapshot("p", 0).map_or(0, |bytes| {
            serde_json::from_slice::<Value>(&bytes).unwrap()["tasks"]
                .as_array()
                .unwrap()
                .len()
        });
        println!("EXPECTED: 0 tasks");
        println!("ACTUAL: {actual} tasks");
        if actual == 0 {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert_eq!(actual, 0);
    }

    #[test]
    fn f402_a_snapshot_task_with_an_unknown_dependency_is_ignored() {
        let task = |dependency: &str| {
            json!({"id": "t-2", "title": "task", "detail": "", "depends_on": [dependency],
                   "done": false, "owner": null, "summary": null})
        };
        let valid = Boards::new();
        valid.load(
            "p",
            json!({
                "next": 3,
                "tasks": [
                    {"id": "t-1", "title": "dependency", "detail": "", "depends_on": [],
                     "done": true, "owner": "alpha/a", "summary": "done"},
                    task("t-1")
                ],
                "leases": []
            })
            .to_string()
            .as_bytes(),
            0,
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&valid.snapshot("p", 0).unwrap()).unwrap()["tasks"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        println!("CONTROL OK");

        let damaged = Boards::new();
        damaged.load(
            "p",
            // `t-3` is not below `next`: a later task would take that id. (An id below `next` that names no task is a merge's
            // waiting placeholder, ADR-0115, and loads.)
            json!({"next": 3, "tasks": [task("t-3")], "leases": []})
                .to_string()
                .as_bytes(),
            0,
        );
        let actual = damaged.snapshot("p", 0).map_or(0, |bytes| {
            serde_json::from_slice::<Value>(&bytes).unwrap()["tasks"]
                .as_array()
                .unwrap()
                .len()
        });
        println!("EXPECTED: 0 tasks");
        println!("ACTUAL: {actual} tasks");
        if actual == 0 {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert_eq!(actual, 0);
    }

    #[test]
    fn f403_a_snapshot_with_a_dependency_cycle_is_ignored() {
        let task = |id: &str, dependency: &str| {
            json!({"id": id, "title": id, "detail": "", "depends_on": [dependency],
                   "done": false, "owner": null, "summary": null})
        };
        let valid = Boards::new();
        valid.load(
            "p",
            json!({
                "next": 3,
                "tasks": [
                    {"id": "t-1", "title": "first", "detail": "", "depends_on": [],
                     "done": false, "owner": null, "summary": null},
                    task("t-2", "t-1")
                ],
                "leases": []
            })
            .to_string()
            .as_bytes(),
            0,
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&valid.snapshot("p", 0).unwrap()).unwrap()["tasks"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        println!("CONTROL OK");

        let damaged = Boards::new();
        damaged.load(
            "p",
            json!({"next": 3, "tasks": [task("t-1", "t-2"), task("t-2", "t-1")], "leases": []})
                .to_string()
                .as_bytes(),
            0,
        );
        let actual = damaged.snapshot("p", 0).map_or(0, |bytes| {
            serde_json::from_slice::<Value>(&bytes).unwrap()["tasks"]
                .as_array()
                .unwrap()
                .len()
        });
        println!("EXPECTED: 0 tasks");
        println!("ACTUAL: {actual} tasks");
        if actual == 0 {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert_eq!(actual, 0);
    }

    #[test]
    fn f404_a_snapshot_with_conflicting_live_leases_is_ignored_atomically() {
        let lease = |path: &str, holder: &str| json!({"path": path, "holder": holder, "expires_at_ms": 10_000});
        let valid = Boards::new();
        valid.load(
            "p",
            json!({
                "next": 1,
                "tasks": [],
                "leases": [lease("src/a", "alpha/a"), lease("src/b", "beta/a")]
            })
            .to_string()
            .as_bytes(),
            0,
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&valid.snapshot("p", 0).unwrap()).unwrap()["leases"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        println!("CONTROL OK");

        let damaged = Boards::new();
        damaged.load(
            "p",
            json!({
                "next": 1,
                "tasks": [],
                "leases": [lease("src", "alpha/a"), lease("src/lib", "beta/a")]
            })
            .to_string()
            .as_bytes(),
            0,
        );
        let actual = damaged.snapshot("p", 0).map_or(0, |bytes| {
            serde_json::from_slice::<Value>(&bytes).unwrap()["leases"]
                .as_array()
                .unwrap()
                .len()
        });
        println!("EXPECTED: 0 leases");
        println!("ACTUAL: {actual} leases");
        if actual == 0 {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert_eq!(actual, 0);
    }

    #[test]
    fn verifies_f400_snapshot_counter_allocation_boundaries() {
        let create_after = |next: u64| {
            let boards = Boards::new();
            boards.load(
                "p",
                json!({"next": next, "tasks": [], "leases": []})
                    .to_string()
                    .as_bytes(),
                0,
            );
            let answer = run(
                &boards,
                &view(&["alpha", "beta"]),
                "alpha",
                "task_create",
                json!({"title": "task", "depends_on": []}),
                1,
            );
            let Verdict::Done { result, .. } = answer.verdict else {
                panic!()
            };
            agora_mesh::mcp::tool_result_value(&result).unwrap()["task_id"]
                .as_str()
                .unwrap()
                .to_string()
        };

        assert_eq!(create_after(1), "t-1");
        assert_eq!(create_after(42), "t-42");
        assert_eq!(create_after(u64::MAX - 1), format!("t-{}", u64::MAX - 1));
        assert_eq!(create_after(0), "t-1");
        assert_eq!(create_after(u64::MAX), "t-1");
        println!("FIX VERIFIED");
    }

    #[test]
    fn verifies_f401_snapshot_task_terminal_state_boundaries() {
        let load = |done: bool, owner: Value| {
            let boards = Boards::new();
            boards.load(
                "p",
                json!({
                    "next": 2,
                    "tasks": [{"id": "t-1", "title": "task", "detail": "", "depends_on": [],
                               "done": done, "owner": owner, "summary": null}],
                    "leases": []
                })
                .to_string()
                .as_bytes(),
                0,
            );
            boards
                .snapshot("p", 0)
                .map(|bytes| serde_json::from_slice::<Value>(&bytes).unwrap()["tasks"].clone())
        };

        let open = load(false, Value::Null).unwrap();
        assert_eq!(open[0]["done"], false);
        assert!(open[0]["owner"].is_null());
        let claimed = load(false, json!("alpha/a")).unwrap();
        assert_eq!(claimed[0]["done"], false);
        assert_eq!(claimed[0]["owner"], "alpha/a");
        let done = load(true, json!("alpha/a")).unwrap();
        assert_eq!(done[0]["done"], true);
        assert_eq!(done[0]["owner"], "alpha/a");
        assert!(load(true, Value::Null).is_none());
        println!("FIX VERIFIED");
    }

    #[test]
    fn verifies_f402_snapshot_dependency_existence_boundaries() {
        let load = |tasks: Value| {
            let boards = Boards::new();
            boards.load(
                "p",
                json!({"next": 4, "tasks": tasks, "leases": []})
                    .to_string()
                    .as_bytes(),
                0,
            );
            boards.snapshot("p", 0).map_or(0, |bytes| {
                serde_json::from_slice::<Value>(&bytes).unwrap()["tasks"]
                    .as_array()
                    .unwrap()
                    .len()
            })
        };
        let task = |id: &str, dependencies: Value| {
            json!({"id": id, "title": id, "detail": "", "depends_on": dependencies,
                   "done": false, "owner": null, "summary": null})
        };

        assert_eq!(load(json!([task("t-1", json!([]))])), 1);
        assert_eq!(
            load(json!([
                task("t-1", json!([])),
                task("t-2", json!(["t-1"])),
                task("t-3", json!(["t-1", "t-2"]))
            ])),
            3
        );
        // An id below `next` that no task has is a merge's waiting placeholder (ADR-0115): the task loads and keeps waiting.
        assert_eq!(load(json!([task("t-2", json!(["t-1"]))])), 1);
        assert_eq!(load(json!([task("t-1", json!(["t-9"]))])), 0);
        assert_eq!(load(json!([task("t-1", json!(["t-4"]))])), 0);
        assert_eq!(load(json!([task("t-1", json!(["x"]))])), 0);
        println!("FIX VERIFIED");
    }

    #[test]
    fn verifies_f403_snapshot_dependency_order_boundaries() {
        let load = |tasks: Value| {
            let boards = Boards::new();
            boards.load(
                "p",
                json!({"next": 4, "tasks": tasks, "leases": []})
                    .to_string()
                    .as_bytes(),
                0,
            );
            boards.snapshot("p", 0).is_some()
        };
        let task = |id: &str, dependencies: Value| {
            json!({"id": id, "title": id, "detail": "", "depends_on": dependencies,
                   "done": false, "owner": null, "summary": null})
        };

        assert!(load(json!([
            task("t-1", json!([])),
            task("t-2", json!(["t-1"])),
            task("t-3", json!(["t-2"]))
        ])));
        assert!(!load(json!([task("t-1", json!(["t-1"]))])));
        assert!(!load(json!([
            task("t-1", json!(["t-2"])),
            task("t-2", json!(["t-1"]))
        ])));
        // A merge (ADR-0115) can renumber a task above one that waits for it: no cycle, so it loads.
        assert!(load(json!([
            task("t-1", json!(["t-2"])),
            task("t-2", json!([]))
        ])));
        assert!(!load(json!([
            task("t-1", json!(["t-3"])),
            task("t-2", json!(["t-1"])),
            task("t-3", json!(["t-2"]))
        ])));
        println!("FIX VERIFIED");
    }

    #[test]
    fn verifies_f404_snapshot_lease_consistency_boundaries() {
        let load = |leases: Value| {
            let boards = Boards::new();
            boards.load(
                "p",
                json!({"next": 1, "tasks": [], "leases": leases})
                    .to_string()
                    .as_bytes(),
                0,
            );
            boards.snapshot("p", 0).map_or(0, |bytes| {
                serde_json::from_slice::<Value>(&bytes).unwrap()["leases"]
                    .as_array()
                    .unwrap()
                    .len()
            })
        };
        let lease = |path: &str, holder: &str| json!({"path": path, "holder": holder, "expires_at_ms": 10_000});

        assert_eq!(
            load(json!([lease("src/a", "alpha/a"), lease("src/b", "beta/a")])),
            2
        );
        assert_eq!(
            load(json!([
                lease("src", "alpha/a"),
                lease("src/lib", "alpha/a")
            ])),
            2
        );
        assert_eq!(
            load(json!([lease("src", "alpha/a"), lease("src/lib", "beta/a")])),
            0
        );
        assert_eq!(load(json!([lease("../outside", "alpha/a")])), 0);
        println!("FIX VERIFIED");
    }

    #[test]
    fn f405_a_non_array_dependency_field_does_not_silently_lose_dependencies() {
        let task = |id: &str, dependencies: Value| {
            json!({"id": id, "title": id, "detail": "", "depends_on": dependencies,
                   "done": false, "owner": null, "summary": null})
        };
        let load = |dependencies: Value| {
            let boards = Boards::new();
            boards.load(
                "p",
                json!({
                    "next": 3,
                    "tasks": [task("t-1", json!([])), task("t-2", dependencies)],
                    "leases": []
                })
                .to_string()
                .as_bytes(),
                0,
            );
            boards.snapshot("p", 0).map_or(0, |bytes| {
                serde_json::from_slice::<Value>(&bytes).unwrap()["tasks"]
                    .as_array()
                    .unwrap()
                    .len()
            })
        };
        assert_eq!(load(json!(["t-1"])), 2);
        println!("CONTROL OK");

        let actual = load(json!("t-1"));
        println!("EXPECTED: 0 tasks");
        println!("ACTUAL: {actual} tasks");
        if actual == 0 {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert_eq!(actual, 0);
    }

    #[test]
    fn f406_a_non_boolean_done_field_does_not_reopen_a_completed_task() {
        let load = |done: Value| {
            let boards = Boards::new();
            boards.load(
                "p",
                json!({
                    "next": 2,
                    "tasks": [{"id": "t-1", "title": "task", "detail": "", "depends_on": [],
                               "done": done, "owner": "alpha/a", "summary": "done"}],
                    "leases": []
                })
                .to_string()
                .as_bytes(),
                0,
            );
            boards
                .snapshot("p", 0)
                .map(|bytes| serde_json::from_slice::<Value>(&bytes).unwrap()["tasks"].clone())
        };
        assert_eq!(load(json!(true)).unwrap()[0]["done"], true);
        println!("CONTROL OK");

        let actual = load(json!("true"));
        println!("EXPECTED: None");
        println!("ACTUAL: {actual:?}");
        if actual.is_none() {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert!(actual.is_none());
    }

    #[test]
    fn f407_duplicate_snapshot_leases_are_not_partially_collapsed() {
        let lease =
            |path: &str| json!({"path": path, "holder": "alpha/a", "expires_at_ms": 10_000});
        let load = |leases: Value| {
            let boards = Boards::new();
            boards.load(
                "p",
                json!({"next": 1, "tasks": [], "leases": leases})
                    .to_string()
                    .as_bytes(),
                0,
            );
            boards.snapshot("p", 0).map_or(0, |bytes| {
                serde_json::from_slice::<Value>(&bytes).unwrap()["leases"]
                    .as_array()
                    .unwrap()
                    .len()
            })
        };
        assert_eq!(load(json!([lease("src/a"), lease("src/b")])), 2);
        println!("CONTROL OK");

        let actual = load(json!([lease("src/a"), lease("src/a")]));
        println!("EXPECTED: 0 leases");
        println!("ACTUAL: {actual} leases");
        if actual == 0 {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert_eq!(actual, 0);
    }

    #[test]
    fn f408_a_snapshot_task_owner_must_be_a_hub_qualified_agent() {
        let load = |owner: &str| {
            let boards = Boards::new();
            boards.load(
                "p",
                json!({
                    "next": 2,
                    "tasks": [{"id": "t-1", "title": "task", "detail": "", "depends_on": [],
                               "done": false, "owner": owner, "summary": null}],
                    "leases": []
                })
                .to_string()
                .as_bytes(),
                0,
            );
            boards.snapshot("p", 0).map_or(0, |bytes| {
                serde_json::from_slice::<Value>(&bytes).unwrap()["tasks"]
                    .as_array()
                    .unwrap()
                    .len()
            })
        };
        assert_eq!(load("alpha/a"), 1);
        println!("CONTROL OK");

        let actual = load("alpha");
        println!("EXPECTED: 0 tasks");
        println!("ACTUAL: {actual} tasks");
        if actual == 0 {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert_eq!(actual, 0);
    }

    #[test]
    fn f409_a_snapshot_lease_holder_must_be_a_hub_qualified_agent() {
        let load = |holder: &str| {
            let boards = Boards::new();
            boards.load(
                "p",
                json!({
                    "next": 1,
                    "tasks": [],
                    "leases": [{"path": "src", "holder": holder, "expires_at_ms": 10_000}]
                })
                .to_string()
                .as_bytes(),
                0,
            );
            boards.snapshot("p", 0).map_or(0, |bytes| {
                serde_json::from_slice::<Value>(&bytes).unwrap()["leases"]
                    .as_array()
                    .unwrap()
                    .len()
            })
        };
        assert_eq!(load("alpha/a"), 1);
        println!("CONTROL OK");

        let actual = load("alpha");
        println!("EXPECTED: 0 leases");
        println!("ACTUAL: {actual} leases");
        if actual == 0 {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert_eq!(actual, 0);
    }

    #[test]
    fn verifies_f405_snapshot_dependency_field_type_boundaries() {
        let task = |id: &str, dependencies: Value| {
            json!({"id": id, "title": id, "detail": "", "depends_on": dependencies,
                   "done": false, "owner": null, "summary": null})
        };
        let load = |dependencies: Value| {
            let boards = Boards::new();
            boards.load(
                "p",
                json!({
                    "next": 3,
                    "tasks": [task("t-1", json!([])), task("t-2", dependencies)],
                    "leases": []
                })
                .to_string()
                .as_bytes(),
                0,
            );
            boards.snapshot("p", 0).is_some()
        };

        assert!(load(json!([])));
        assert!(load(json!(["t-1"])));
        assert!(!load(json!("t-1")));
        assert!(!load(Value::Null));
        assert!(!load(json!(["t-1", 2])));
        println!("FIX VERIFIED");
    }

    #[test]
    fn verifies_f406_snapshot_done_field_type_boundaries() {
        let load = |done: Value, owner: Value| {
            let boards = Boards::new();
            boards.load(
                "p",
                json!({
                    "next": 2,
                    "tasks": [{"id": "t-1", "title": "task", "detail": "", "depends_on": [],
                               "done": done, "owner": owner, "summary": null}],
                    "leases": []
                })
                .to_string()
                .as_bytes(),
                0,
            );
            boards.snapshot("p", 0).is_some()
        };

        assert!(load(json!(false), Value::Null));
        assert!(load(json!(false), json!("alpha/a")));
        assert!(load(json!(true), json!("alpha/a")));
        assert!(!load(json!("false"), json!("alpha/a")));
        assert!(!load(json!(0), json!("alpha/a")));
        assert!(!load(Value::Null, json!("alpha/a")));
        println!("FIX VERIFIED");
    }

    #[test]
    fn verifies_f407_snapshot_duplicate_lease_boundaries() {
        let load = |paths: &[&str]| {
            let boards = Boards::new();
            let leases: Vec<Value> = paths
                .iter()
                .map(|path| json!({"path": path, "holder": "alpha/a", "expires_at_ms": 10_000}))
                .collect();
            boards.load(
                "p",
                json!({"next": 1, "tasks": [], "leases": leases})
                    .to_string()
                    .as_bytes(),
                0,
            );
            boards.snapshot("p", 0).map_or(0, |bytes| {
                serde_json::from_slice::<Value>(&bytes).unwrap()["leases"]
                    .as_array()
                    .unwrap()
                    .len()
            })
        };

        assert_eq!(load(&["src/a"]), 1);
        assert_eq!(load(&["src/a", "src/b"]), 2);
        assert_eq!(load(&["src", "src/lib"]), 2);
        assert_eq!(load(&["src/a", "src/a"]), 0);
        assert_eq!(load(&["src/a", "./src/a"]), 0);
        println!("FIX VERIFIED");
    }

    #[test]
    fn verifies_f408_snapshot_task_owner_identity_boundaries() {
        let load = |owner: Value| {
            let boards = Boards::new();
            boards.load(
                "p",
                json!({
                    "next": 2,
                    "tasks": [{"id": "t-1", "title": "task", "detail": "", "depends_on": [],
                               "done": false, "owner": owner, "summary": null}],
                    "leases": []
                })
                .to_string()
                .as_bytes(),
                0,
            );
            boards.snapshot("p", 0).is_some()
        };

        assert!(load(Value::Null));
        assert!(load(json!("alpha/a")));
        assert!(load(json!(BOARD_AGENT)));
        assert!(!load(json!("alpha")));
        assert!(!load(json!("/a")));
        assert!(!load(json!("alpha/")));
        assert!(!load(json!("alpha/a/extra")));
        println!("FIX VERIFIED");
    }

    #[test]
    fn verifies_f409_snapshot_lease_holder_identity_boundaries() {
        let load = |holder: Value| {
            let boards = Boards::new();
            boards.load(
                "p",
                json!({
                    "next": 1,
                    "tasks": [],
                    "leases": [{"path": "src", "holder": holder, "expires_at_ms": 10_000}]
                })
                .to_string()
                .as_bytes(),
                0,
            );
            boards.snapshot("p", 0).is_some()
        };

        assert!(load(json!("alpha/a")));
        assert!(load(json!(BOARD_AGENT)));
        assert!(!load(json!("alpha")));
        assert!(!load(json!("/a")));
        assert!(!load(json!("alpha/")));
        assert!(!load(json!("alpha/a/extra")));
        assert!(!load(json!(7)));
        println!("FIX VERIFIED");
    }

    #[test]
    fn f410_a_non_string_snapshot_title_does_not_silently_become_empty() {
        let load = |title: Value| {
            let boards = Boards::new();
            boards.load(
                "p",
                json!({
                    "next": 2,
                    "tasks": [{"id": "t-1", "title": title, "detail": "", "depends_on": [],
                               "done": false, "owner": null, "summary": null}],
                    "leases": []
                })
                .to_string()
                .as_bytes(),
                0,
            );
            boards
                .snapshot("p", 0)
                .map(|bytes| serde_json::from_slice::<Value>(&bytes).unwrap()["tasks"].clone())
        };
        assert_eq!(load(json!("title")).unwrap()[0]["title"], "title");
        println!("CONTROL OK");

        let actual = load(json!(7));
        println!("EXPECTED: None");
        println!("ACTUAL: {actual:?}");
        if actual.is_none() {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert!(actual.is_none());
    }

    #[test]
    fn f411_a_non_string_snapshot_detail_does_not_silently_become_empty() {
        let load = |detail: Value| {
            let boards = Boards::new();
            boards.load(
                "p",
                json!({
                    "next": 2,
                    "tasks": [{"id": "t-1", "title": "task", "detail": detail, "depends_on": [],
                               "done": false, "owner": null, "summary": null}],
                    "leases": []
                })
                .to_string()
                .as_bytes(),
                0,
            );
            boards
                .snapshot("p", 0)
                .map(|bytes| serde_json::from_slice::<Value>(&bytes).unwrap()["tasks"].clone())
        };
        assert_eq!(load(json!("detail")).unwrap()[0]["detail"], "detail");
        println!("CONTROL OK");

        let actual = load(json!(["detail"]));
        println!("EXPECTED: None");
        println!("ACTUAL: {actual:?}");
        if actual.is_none() {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert!(actual.is_none());
    }

    #[test]
    fn f412_a_non_string_snapshot_summary_does_not_silently_disappear() {
        let load = |summary: Value| {
            let boards = Boards::new();
            boards.load(
                "p",
                json!({
                    "next": 2,
                    "tasks": [{"id": "t-1", "title": "task", "detail": "", "depends_on": [],
                               "done": false, "owner": "alpha/a", "summary": summary}],
                    "leases": []
                })
                .to_string()
                .as_bytes(),
                0,
            );
            boards
                .snapshot("p", 0)
                .map(|bytes| serde_json::from_slice::<Value>(&bytes).unwrap()["tasks"].clone())
        };
        assert_eq!(load(json!("handoff")).unwrap()[0]["summary"], "handoff");
        println!("CONTROL OK");

        let actual = load(json!({"text": "handoff"}));
        println!("EXPECTED: None");
        println!("ACTUAL: {actual:?}");
        if actual.is_none() {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert!(actual.is_none());
    }

    #[test]
    fn f413_a_snapshot_with_no_id_counter_is_ignored() {
        let lease = json!({"path": "src", "holder": "alpha/a", "expires_at_ms": 10_000});
        let valid = Boards::new();
        valid.load(
            "p",
            json!({"next": 1, "tasks": [], "leases": [lease.clone()]})
                .to_string()
                .as_bytes(),
            0,
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&valid.snapshot("p", 0).unwrap()).unwrap()["leases"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        println!("CONTROL OK");

        let damaged = Boards::new();
        damaged.load(
            "p",
            json!({"tasks": [], "leases": [lease]})
                .to_string()
                .as_bytes(),
            0,
        );
        let actual = damaged.snapshot("p", 0);
        println!("EXPECTED: None");
        println!("ACTUAL: {}", if actual.is_some() { "Some" } else { "None" });
        if actual.is_none() {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert!(actual.is_none());
    }

    #[test]
    fn f414_restore_reserves_enough_id_space_for_every_live_lease() {
        let leases = json!([
            {"path": "src/a", "holder": "alpha/a", "expires_at_ms": 10_000},
            {"path": "src/b", "holder": "alpha/a", "expires_at_ms": 10_000}
        ]);
        let valid = Boards::new();
        valid.load(
            "p",
            json!({"next": u64::MAX - 3, "tasks": [], "leases": leases.clone()})
                .to_string()
                .as_bytes(),
            0,
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&valid.snapshot("p", 0).unwrap()).unwrap()["leases"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        println!("CONTROL OK");

        let damaged = Boards::new();
        let actual = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            damaged.load(
                "p",
                json!({"next": u64::MAX - 1, "tasks": [], "leases": leases})
                    .to_string()
                    .as_bytes(),
                0,
            );
        }));
        println!("EXPECTED: Ok and ignored");
        println!("ACTUAL: {}", if actual.is_ok() { "Ok" } else { "panic" });
        if actual.is_ok() && damaged.snapshot("p", 0).is_none() {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert!(actual.is_ok());
        assert!(damaged.snapshot("p", 0).is_none());
    }

    #[test]
    fn verifies_f410_snapshot_title_type_boundaries() {
        let load = |title: Value| {
            let boards = Boards::new();
            boards.load(
                "p",
                json!({
                    "next": 2,
                    "tasks": [{"id": "t-1", "title": title, "detail": "", "depends_on": [],
                               "done": false, "owner": null, "summary": null}],
                    "leases": []
                })
                .to_string()
                .as_bytes(),
                0,
            );
            boards.snapshot("p", 0).is_some()
        };
        assert!(load(json!("")));
        assert!(load(json!("task")));
        assert!(load(json!("görev")));
        assert!(!load(json!(7)));
        assert!(!load(Value::Null));
        assert!(!load(json!(["task"])));
        println!("FIX VERIFIED");
    }

    #[test]
    fn verifies_f411_snapshot_detail_type_boundaries() {
        let load = |detail: Value| {
            let boards = Boards::new();
            boards.load(
                "p",
                json!({
                    "next": 2,
                    "tasks": [{"id": "t-1", "title": "task", "detail": detail, "depends_on": [],
                               "done": false, "owner": null, "summary": null}],
                    "leases": []
                })
                .to_string()
                .as_bytes(),
                0,
            );
            boards.snapshot("p", 0).is_some()
        };
        assert!(load(json!("")));
        assert!(load(json!("detail")));
        assert!(load(json!("satır\niki")));
        assert!(!load(json!(false)));
        assert!(!load(Value::Null));
        assert!(!load(json!({"detail": "text"})));
        println!("FIX VERIFIED");
    }

    #[test]
    fn verifies_f412_snapshot_summary_type_boundaries() {
        let load = |summary: Value| {
            let boards = Boards::new();
            boards.load(
                "p",
                json!({
                    "next": 2,
                    "tasks": [{"id": "t-1", "title": "task", "detail": "", "depends_on": [],
                               "done": false, "owner": "alpha/a", "summary": summary}],
                    "leases": []
                })
                .to_string()
                .as_bytes(),
                0,
            );
            boards.snapshot("p", 0).is_some()
        };
        assert!(load(Value::Null));
        assert!(load(json!("")));
        assert!(load(json!("handoff")));
        assert!(!load(json!(1)));
        assert!(!load(json!(true)));
        assert!(!load(json!({"text": "handoff"})));
        println!("FIX VERIFIED");
    }

    #[test]
    fn verifies_f413_snapshot_counter_type_boundaries() {
        let load = |next: Option<Value>| {
            let boards = Boards::new();
            let mut snapshot = json!({"tasks": [], "leases": []});
            if let Some(next) = next {
                snapshot["next"] = next;
            }
            boards.load("p", snapshot.to_string().as_bytes(), 0);
            boards.snapshot("p", 0).is_some()
        };
        assert!(load(Some(json!(1))));
        assert!(load(Some(json!(42))));
        assert!(load(Some(json!(u64::MAX - 1))));
        assert!(!load(None));
        assert!(!load(Some(json!("1"))));
        assert!(!load(Some(json!(-1))));
        println!("FIX VERIFIED");
    }

    #[test]
    fn verifies_f414_live_lease_id_capacity_boundaries() {
        let load = |next: u64, expires: &[u64], now: u64| {
            let boards = Boards::new();
            let leases: Vec<Value> = expires
                .iter()
                .enumerate()
                .map(|(index, expires)| {
                    json!({"path": format!("src/{index}"), "holder": "alpha/a",
                           "expires_at_ms": expires})
                })
                .collect();
            boards.load(
                "p",
                json!({"next": next, "tasks": [], "leases": leases})
                    .to_string()
                    .as_bytes(),
                now,
            );
            boards.snapshot("p", now).is_some()
        };
        assert!(load(u64::MAX - 3, &[10_000, 10_000], 0));
        assert!(load(u64::MAX - 2, &[10_000, 10_000], 0));
        assert!(load(u64::MAX - 1, &[10_000], 0));
        assert!(!load(u64::MAX - 1, &[10_000, 10_000], 0));
        assert!(load(u64::MAX - 1, &[1, 2], 10));
        println!("FIX VERIFIED");
    }
}
