//! The shared board of a project (ADR-0039): tasks, path leases and handoffs for a project that has checkouts on two or
//! more online nodes. The hub is the single writer, so a claim or a lease is decided in one place.
//!
//! A board is the same broker a node runs ([`agora_mesh::broker::Mesh`]) with agents named `<node>/<agent>`, so there is one
//! implementation of the rules. The board's agents are not kept by hand: before each call they are made to match the hub's
//! view (running agents of that project), so an agent that ended gives its tasks and leases back.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use agora_core::AgentId;
use agora_mesh::broker::{AgentInfo, Limits, Mesh};
use agora_mesh::mcp::{board_tool, BOARD_TOOLS};
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
}

fn answer(verdict: Verdict) -> Answer {
    Answer {
        verdict,
        events: Vec::new(),
        handoff: None,
        fanout: None,
    }
}

/// Known `(node, project)` memberships from checkouts, selected workspaces and still-running agents.
/// Facts can repeat and include offline nodes: transition discovery must still see a project after all its nodes disconnect.
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

#[derive(Default)]
struct Joins {
    /// The online nodes with a checkout of each project at the last look.
    members: BTreeMap<String, BTreeSet<String>>,
    /// Projects whose nodes are handing over local work: until when calls wait, and the nodes not yet answered.
    pending: BTreeMap<String, (u64, BTreeSet<String>)>,
}

#[derive(Default)]
pub struct Boards {
    by_project: Mutex<BTreeMap<String, Mesh>>,
    joins: Mutex<Joins>,
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
        let (Some(to), Some(body)) = (
            args.get("to").and_then(Value::as_str),
            args.get("body").and_then(Value::as_str),
        ) else {
            return invalid("body");
        };
        if !to.starts_with('#') || to.len() < 2 {
            return invalid("to");
        }
        let refs: Vec<String> = args
            .get("refs")
            .and_then(Value::as_array)
            .map(|r| {
                r.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let done = |result: Value, is_error: bool| Verdict::Done { result, is_error };
        let refuse = |code: ErrorCode, limit: usize| {
            let text = json!({"error": {"code": code.as_str(), "params": {"limit": limit}}});
            done(
                json!({"content": [{"type": "text", "text": text.to_string()}], "isError": true}),
                true,
            )
        };
        if body.len() > limits.max_body_bytes {
            return answer(refuse(ErrorCode::MeshBodyTooLarge, limits.max_body_bytes));
        }
        if refs.len() > limits.max_refs {
            return answer(refuse(ErrorCode::MeshTooManyRefs, limits.max_refs));
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
            body: format!("[{to}] {body}"),
            refs,
        });
        a
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

    /// A node's process started again: every agent it had is gone, even if a new one will carry the same local id (`agent-1`). Gives
    /// their tasks and leases back now, in every board in memory, and returns each project with the events that produced.
    pub fn node_restarted(&self, node: &str, now_ms: u64) -> Vec<(String, Vec<Body>)> {
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

    /// Restores a board from [`snapshot`](Self::snapshot) text. A damaged snapshot is ignored: the board starts empty.
    pub fn load(&self, project: &str, bytes: &[u8], now_ms: u64) {
        use agora_mesh::broker::{TaskEntry, TaskStatus};
        let Ok(v) = serde_json::from_slice::<Value>(bytes) else {
            return;
        };
        let text = |t: &Value, k: &str| t[k].as_str().unwrap_or("").to_string();
        let tasks: Vec<TaskEntry> = v["tasks"]
            .as_array()
            .map(|a| {
                a.iter()
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
                    .collect()
            })
            .unwrap_or_default();
        let mut g = self.by_project.lock().unwrap_or_else(|e| e.into_inner());
        let mesh = g
            .entry(project.to_string())
            .or_insert_with(|| Mesh::new(Limits::default()));
        mesh.import_tasks(v["next"].as_u64().unwrap_or(1), &tasks);
        let leases: Vec<(String, AgentId, u64)> = v["leases"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|l| {
                        Some((
                            l["path"].as_str()?.to_string(),
                            AgentId::new(l["holder"].as_str()?),
                            l["expires_at_ms"].as_u64()?,
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        mesh.import_leases(project, &leases, now_ms);
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
        if !channel && !BOARD_TOOLS.contains(&tool) {
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
        }
    }
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
        let mut projects: BTreeSet<String> = project_memberships(view)
            .map(|(_, project)| project.to_string())
            .collect();
        let mut j = self.joins.lock().unwrap_or_else(|e| e.into_inner());
        // A reassigned folder and its last exiting agent can remove every current fact for a project.
        projects.extend(j.members.keys().cloned());
        let mut out = Vec::new();
        for project in projects {
            let now: BTreeSet<String> = nodes_with(view, &project).into_iter().collect();
            let before = j.members.get(&project).cloned().unwrap_or_default();
            let join: Vec<String> = if now.len() < 2 {
                Vec::new()
            } else if before.len() < 2 {
                now.iter().cloned().collect()
            } else {
                now.difference(&before).cloned().collect()
            };
            j.members.insert(project.clone(), now);
            if join.is_empty() {
                continue;
            }
            let entry = j
                .pending
                .entry(project.clone())
                .or_insert_with(|| (0, BTreeSet::new()));
            // A hold that is still running keeps its end: joins one after another never hold grants longer than one wait.
            if entry.0 <= now_ms {
                entry.0 = now_ms + JOIN_WAIT_MS;
            }
            entry.1.extend(join.iter().cloned());
            out.extend(join.into_iter().map(|n| (n, project.clone())));
        }
        out
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
        let j = self.joins.lock().unwrap_or_else(|e| e.into_inner());
        j.pending
            .get(project)
            .is_some_and(|(until, nodes)| !nodes.is_empty() && now_ms < *until)
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

    fn view(online: &[&str]) -> View {
        let mut v = View::default();
        for (i, n) in ["alpha", "beta", "gamma"].iter().enumerate() {
            v.nodes.insert(
                (*n).to_string(),
                NodeView {
                    hostname: (*n).into(),
                    os: String::new(),
                    build: String::new(),
                    online: online.contains(n),
                    agents_running: 1,
                    terminals_open: 0,
                    memory_used_bytes: 0,
                    last_event_ms: 0,
                },
            );
            v.checkouts
                .insert(format!("c{i}"), ((*n).into(), "p".into(), "/p".into()));
            v.agents.insert(
                format!("{n}/a"),
                AgentView {
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
}
