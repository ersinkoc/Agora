//! The mesh broker (docs/specs/mesh-tools.md). Everything an agent can ask of the mesh, as deterministic functions
//! over in-memory state.
//!
//! Safety rules from the spec, enforced here:
//! - **Mesh content is untrusted data.** Every delivered message carries provenance (sender, kind, node, project) set
//!   by the broker from the registry, never by the sender, and nothing here can approve anything.
//! - **Agents cannot impersonate each other:** every call takes the caller's id from the node-bound connection; a body
//!   cannot name another sender.
//! - **Rate limits** per agent (messages per window, open asks) stop two agents from looping forever.
//! - **Bodies are size-limited;** large content is passed by reference (`refs`).
//! - **Everything is auditable:** each state change queues a protocol event ([`Mesh::take_events`]).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use agora_core::{AgentId, LeaseError, LeaseTable, Task, TaskError, TaskId, TaskState};
use agora_protocol::v1::event::Body;
use agora_protocol::v1::{HandoffCreated, LeaseGranted, LeaseReleased, MessagePosted, TaskChanged};
use agora_protocol::{Error, ErrorCode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_body_bytes: usize,
    pub max_refs: usize,
    /// Messages one agent may send per `window_ms`.
    pub messages_per_window: usize,
    pub window_ms: u64,
    pub max_open_asks: usize,
    /// Messages kept per inbox; the oldest are dropped beyond this.
    pub inbox_capacity: usize,
    pub default_ask_timeout_ms: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_body_bytes: 16 * 1024,
            max_refs: 20,
            messages_per_window: 30,
            window_ms: 60_000,
            max_open_asks: 5,
            inbox_capacity: 1000,
            default_ask_timeout_ms: 600_000,
        }
    }
}

/// Who an agent is, as the node knows it. The broker trusts this; senders cannot supply it per message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentInfo {
    pub id: AgentId,
    pub kind: String,
    pub node: String,
    pub project: String,
    pub label: String,
    pub worktree: String,
    pub state: String,
}

/// Provenance stamped on every delivered message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provenance {
    pub from: AgentId,
    pub kind: String,
    pub node: String,
    pub project: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub id: String,
    pub provenance: Provenance,
    /// The agent id or `#channel` it was addressed to.
    pub to: String,
    pub body: String,
    pub refs: Vec<String>,
    pub at_ms: u64,
    pub reply_to: Option<String>,
    /// `message`, `ask`, `reply` or `handoff`.
    pub kind: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeshError {
    /// The caller is not registered (its id).
    UnknownAgent(String),
    RecipientUnknown(String),
    BodyTooLarge,
    TooManyRefs,
    RateLimited,
    TooManyOpenAsks,
    TaskUnknown(String),
    /// The task is not open (claimed or done).
    TaskNotOpen(String),
    /// A dependency of the task is not done yet.
    TaskBlocked {
        task: String,
        depends_on: String,
    },
    /// Only the claiming agent may do this to the task.
    NotTaskOwner(String),
    DependencyUnknown(String),
    /// A lease conflict, with the path that conflicts and who holds it.
    LeaseHeld {
        path: String,
        holder: AgentId,
    },
    LeaseInvalidPath(String),
    /// A lease must last at least a moment.
    LeaseZeroTtl,
    LeaseUnknown(String),
}

impl MeshError {
    /// The protocol error for this failure (codes in `docs/specs/error-codes.md`).
    pub fn to_error(&self, limits: &Limits) -> Error {
        let limit = |n: usize| i64::try_from(n).unwrap_or(i64::MAX);
        match self {
            Self::UnknownAgent(id) => {
                Error::new(ErrorCode::AgentNotFound).with("agent_id", id.as_str())
            }
            Self::RecipientUnknown(to) => {
                Error::new(ErrorCode::MeshRecipientUnknown).with("to", to.as_str())
            }
            Self::BodyTooLarge => {
                Error::new(ErrorCode::MeshBodyTooLarge).with("limit", limit(limits.max_body_bytes))
            }
            Self::TooManyRefs => {
                Error::new(ErrorCode::MeshTooManyRefs).with("limit", limit(limits.max_refs))
            }
            Self::RateLimited => Error::new(ErrorCode::MeshRateLimited),
            Self::TooManyOpenAsks => {
                Error::new(ErrorCode::MeshTooManyAsks).with("limit", limit(limits.max_open_asks))
            }
            Self::TaskUnknown(t) | Self::DependencyUnknown(t) => {
                Error::new(ErrorCode::TaskNotFound).with("task_id", t.as_str())
            }
            Self::TaskNotOpen(t) => Error::new(ErrorCode::TaskNotOpen).with("task_id", t.as_str()),
            Self::TaskBlocked { task, depends_on } => Error::new(ErrorCode::TaskBlocked)
                .with("task_id", task.as_str())
                .with("depends_on", depends_on.as_str()),
            Self::NotTaskOwner(t) => {
                Error::new(ErrorCode::TaskNotOwner).with("task_id", t.as_str())
            }
            Self::LeaseHeld { path, holder } => Error::new(ErrorCode::LeaseHeld)
                .with("path", path.as_str())
                .with("holder_agent_id", holder.as_str())
                // The lease's expiry is not tracked per conflict here; an empty value renders as "later".
                .with("expires_at", ""),
            Self::LeaseInvalidPath(p) => {
                Error::new(ErrorCode::LeaseInvalidPath).with("path", p.as_str())
            }
            Self::LeaseZeroTtl => {
                Error::new(ErrorCode::ProtocolInvalidMessage).with("field", "ttl_s")
            }
            Self::LeaseUnknown(l) => {
                Error::new(ErrorCode::LeaseNotFound).with("lease_id", l.as_str())
            }
        }
    }
}

/// Answer to `ask`, polled by the caller until it is no longer pending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskStatus {
    Pending,
    Answered { reply: String, from: AgentId },
    TimedOut,
}

struct Ask {
    from: AgentId,
    to: AgentId,
    deadline_ms: u64,
    answer: Option<(String, AgentId)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskEntry {
    pub id: TaskId,
    pub project: String,
    pub title: String,
    pub detail: String,
    pub depends_on: Vec<TaskId>,
    pub status: TaskStatus,
    pub owner: Option<AgentId>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Open,
    Claimed,
    Done,
}

struct TaskRec {
    task: Task,
    project: String,
    detail: String,
    depends_on: Vec<TaskId>,
    summary: Option<String>,
}

struct LeaseBatch {
    holder: AgentId,
    project: String,
    paths: Vec<String>,
}

/// A message for an agent on another node, waiting for the node's uplink to carry it to the hub (ADR-0031).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outbound {
    pub message_id: String,
    pub from: AgentId,
    pub to_node: String,
    pub to_agent: String,
    pub body: String,
}

/// `<node>/<agent>` split in two; `None` for a bare agent id, a `#channel` or anything with more or empty parts.
fn split_address(to: &str) -> Option<(&str, &str)> {
    if to.starts_with('#') {
        return None;
    }
    let (node, agent) = to.split_once('/')?;
    (!node.is_empty() && !agent.is_empty() && !agent.contains('/')).then_some((node, agent))
}

pub struct Mesh {
    outbox: Vec<Outbound>,
    limits: Limits,
    agents: BTreeMap<AgentId, AgentInfo>,
    inboxes: BTreeMap<AgentId, VecDeque<Message>>,
    sent: BTreeMap<AgentId, VecDeque<u64>>,
    asks: BTreeMap<String, Ask>,
    tasks: BTreeMap<TaskId, TaskRec>,
    leases: BTreeMap<String, LeaseTable>,
    batches: BTreeMap<String, LeaseBatch>,
    next: u64,
    events: Vec<Body>,
}

impl Mesh {
    pub fn new(limits: Limits) -> Self {
        Self {
            outbox: Vec::new(),
            limits,
            agents: BTreeMap::new(),
            inboxes: BTreeMap::new(),
            sent: BTreeMap::new(),
            asks: BTreeMap::new(),
            tasks: BTreeMap::new(),
            leases: BTreeMap::new(),
            batches: BTreeMap::new(),
            next: 1,
            events: Vec::new(),
        }
    }

    fn id(&mut self, prefix: &str) -> String {
        let n = self.next;
        self.next += 1;
        format!("{prefix}-{n}")
    }

    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// Messages for other nodes queued since the last call, oldest first. The caller hands them to the hub and reports a
    /// failure back with [`Mesh::delivery_failed`].
    pub fn take_outbound(&mut self) -> Vec<Outbound> {
        std::mem::take(&mut self.outbox)
    }

    /// A message from an agent on another node (`origin` is the node the hub says it came from, `from_agent` what that node
    /// calls the sender). It reaches the local agent `to` like any message, with the sender shown as `<origin>/<from_agent>`;
    /// it is untrusted data like every mesh message. Counted against the same per-sender rate limit.
    pub fn deliver_remote(
        &mut self,
        origin: &str,
        from_agent: &str,
        to: &AgentId,
        body: &str,
        now_ms: u64,
    ) -> Result<String, MeshError> {
        if origin.is_empty() || from_agent.is_empty() || from_agent.contains('/') {
            return Err(MeshError::RecipientUnknown(to.as_str().to_string()));
        }
        self.check_content(body, &[])?;
        let Some(recipient) = self.agents.get(to).cloned() else {
            return Err(MeshError::RecipientUnknown(to.as_str().to_string()));
        };
        let sender = AgentId::new(format!("{origin}/{from_agent}"));
        self.check_rate(&sender, now_ms)?;
        let m = Message {
            id: self.id("m"),
            provenance: Provenance {
                from: sender,
                kind: String::new(),
                node: origin.to_string(),
                project: String::new(),
            },
            to: recipient.id.as_str().to_string(),
            body: body.to_string(),
            refs: Vec::new(),
            at_ms: now_ms,
            reply_to: None,
            kind: "message",
        };
        let id = m.id.clone();
        self.deliver(std::slice::from_ref(&recipient.id), &m);
        Ok(id)
    }

    /// Tells `from` that a message it sent to another node did not arrive. It lands in the inbox as a notice (kind
    /// `delivery_failed`, body the error code); delivery is at most once, so nothing is retried.
    pub fn delivery_failed(&mut self, from: &AgentId, to: &str, code: &str, now_ms: u64) {
        let Some(info) = self.agents.get(from).cloned() else {
            return;
        };
        let m = Message {
            id: self.id("m"),
            provenance: Provenance {
                from: AgentId::new(to),
                kind: String::new(),
                node: split_address(to)
                    .map(|(n, _)| n.to_string())
                    .unwrap_or_default(),
                project: String::new(),
            },
            to: info.id.as_str().to_string(),
            body: code.to_string(),
            refs: Vec::new(),
            at_ms: now_ms,
            reply_to: None,
            kind: "delivery_failed",
        };
        let inbox = self.inboxes.entry(info.id).or_default();
        inbox.push_back(m);
        while inbox.len() > self.limits.inbox_capacity {
            inbox.pop_front();
        }
    }

    /// Audit events queued since the last call, oldest first.
    pub fn take_events(&mut self) -> Vec<Body> {
        std::mem::take(&mut self.events)
    }

    // ---- registry ----

    pub fn register(&mut self, info: AgentInfo) {
        self.inboxes.entry(info.id.clone()).or_default();
        self.agents.insert(info.id.clone(), info);
    }

    /// The agent ended: it leaves the registry, loses its leases and open asks, and gives its tasks back.
    pub fn unregister(&mut self, agent: &AgentId, now_ms: u64) {
        self.agents.remove(agent);
        self.inboxes.remove(agent);
        self.sent.remove(agent);
        self.asks.retain(|_, a| &a.from != agent && &a.to != agent);
        let held: Vec<String> = self
            .batches
            .iter()
            .filter(|(_, b)| &b.holder == agent)
            .map(|(id, _)| id.clone())
            .collect();
        for id in held {
            self.release_batch(&id, now_ms);
        }
        let owned: Vec<TaskId> = self
            .tasks
            .iter()
            .filter(|(_, t)| matches!(t.task.state(), TaskState::Claimed(a) if a == agent))
            .map(|(id, _)| id.clone())
            .collect();
        for id in owned {
            if let Some(t) = self.tasks.get_mut(&id) {
                let _ = t.task.release(agent);
            }
            self.task_changed(&id);
        }
    }

    fn caller(&self, agent: &AgentId) -> Result<&AgentInfo, MeshError> {
        self.agents
            .get(agent)
            .ok_or_else(|| MeshError::UnknownAgent(agent.as_str().to_string()))
    }

    /// The agent's state as the UI words it (`working`, `blocked`, `idle`, `done`, `unknown`), for `agents_list`.
    pub fn set_state(&mut self, agent: &AgentId, state: &str) {
        if let Some(a) = self.agents.get_mut(agent) {
            if a.state != state {
                a.state = state.to_string();
            }
        }
    }

    /// `agents_list`: peers in a project (all projects when `None`), the caller included.
    pub fn agents_list(&self, project: Option<&str>) -> Vec<AgentInfo> {
        self.agents
            .values()
            .filter(|a| project.is_none_or(|p| a.project == p))
            .cloned()
            .collect()
    }

    // ---- messages ----

    fn check_rate(&mut self, from: &AgentId, now_ms: u64) -> Result<(), MeshError> {
        let window = self.limits.window_ms;
        let q = self.sent.entry(from.clone()).or_default();
        while q
            .front()
            .is_some_and(|t| now_ms.saturating_sub(*t) >= window)
        {
            q.pop_front();
        }
        if q.len() >= self.limits.messages_per_window {
            return Err(MeshError::RateLimited);
        }
        q.push_back(now_ms);
        Ok(())
    }

    fn check_content(&self, body: &str, refs: &[String]) -> Result<(), MeshError> {
        if body.len() > self.limits.max_body_bytes {
            return Err(MeshError::BodyTooLarge);
        }
        if refs.len() > self.limits.max_refs {
            return Err(MeshError::TooManyRefs);
        }
        Ok(())
    }

    /// Recipients for `to`: one agent, or every other agent in the sender's project for `#channel`.
    fn recipients(&self, from: &AgentInfo, to: &str) -> Result<Vec<AgentId>, MeshError> {
        if to.starts_with('#') {
            let r: Vec<AgentId> = self
                .agents
                .values()
                .filter(|a| a.project == from.project && a.id != from.id)
                .map(|a| a.id.clone())
                .collect();
            return Ok(r);
        }
        let id = AgentId::new(to);
        if id == from.id || !self.agents.contains_key(&id) {
            return Err(MeshError::RecipientUnknown(to.to_string()));
        }
        Ok(vec![id])
    }

    fn deliver(&mut self, to: &[AgentId], m: &Message) {
        for r in to {
            let inbox = self.inboxes.entry(r.clone()).or_default();
            inbox.push_back(m.clone());
            while inbox.len() > self.limits.inbox_capacity {
                inbox.pop_front();
            }
        }
        self.events.push(Body::MessagePosted(MessagePosted {
            message_id: m.id.clone(),
            from_agent_id: m.provenance.from.as_str().to_string(),
            to: m.to.clone(),
            body: m.body.clone(),
        }));
    }

    #[allow(clippy::too_many_arguments)]
    fn post(
        &mut self,
        from: &AgentId,
        to: &str,
        body: &str,
        refs: &[String],
        reply_to: Option<&str>,
        kind: &'static str,
        now_ms: u64,
    ) -> Result<(Message, Vec<AgentId>), MeshError> {
        let info = self.caller(from)?.clone();
        self.check_content(body, refs)?;
        // Your own node's name in front is just a longer way to say "here".
        let to = match split_address(to) {
            Some((node, agent)) if node == info.node => agent,
            _ => to,
        };
        if let (Some((node, agent)), "message") = (split_address(to), kind) {
            self.check_rate(from, now_ms)?;
            let m = Message {
                id: self.id("m"),
                provenance: Provenance {
                    from: info.id.clone(),
                    kind: info.kind,
                    node: info.node,
                    project: info.project,
                },
                to: to.to_string(),
                body: body.to_string(),
                refs: refs.to_vec(),
                at_ms: now_ms,
                reply_to: reply_to.map(str::to_string),
                kind,
            };
            self.outbox.push(Outbound {
                message_id: m.id.clone(),
                from: info.id,
                to_node: node.to_string(),
                to_agent: agent.to_string(),
                body: body.to_string(),
            });
            self.events.push(Body::MessagePosted(MessagePosted {
                message_id: m.id.clone(),
                from_agent_id: m.provenance.from.as_str().to_string(),
                to: m.to.clone(),
                body: m.body.clone(),
            }));
            return Ok((m, Vec::new()));
        }
        let recipients = self.recipients(&info, to)?;
        self.check_rate(from, now_ms)?;
        let m = Message {
            id: self.id("m"),
            provenance: Provenance {
                from: info.id,
                kind: info.kind,
                node: info.node,
                project: info.project,
            },
            to: to.to_string(),
            body: body.to_string(),
            refs: refs.to_vec(),
            at_ms: now_ms,
            reply_to: reply_to.map(str::to_string),
            kind,
        };
        self.deliver(&recipients, &m);
        Ok((m, recipients))
    }

    /// `send_message`. A message that replies to an open `ask` addressed to the sender answers it.
    pub fn send_message(
        &mut self,
        from: &AgentId,
        to: &str,
        body: &str,
        refs: &[String],
        reply_to: Option<&str>,
        now_ms: u64,
    ) -> Result<String, MeshError> {
        let answers = reply_to
            .and_then(|r| self.asks.get(r).map(|a| (r.to_string(), a.to == *from)))
            .filter(|(_, mine)| *mine)
            .map(|(r, _)| r);
        let kind = if answers.is_some() {
            "reply"
        } else {
            "message"
        };
        let (m, _) = self.post(from, to, body, refs, reply_to, kind, now_ms)?;
        if let Some(ask_id) = answers {
            if let Some(a) = self.asks.get_mut(&ask_id) {
                if a.answer.is_none() {
                    a.answer = Some((body.to_string(), from.clone()));
                }
            }
        }
        Ok(m.id)
    }

    /// `ask`: sends a question and returns the id to poll with [`Mesh::ask_status`]. The recipient answers with
    /// `send_message(reply_to = <that id>)`.
    pub fn ask(
        &mut self,
        from: &AgentId,
        to: &str,
        question: &str,
        timeout_ms: Option<u64>,
        now_ms: u64,
    ) -> Result<String, MeshError> {
        if to.starts_with('#') {
            return Err(MeshError::RecipientUnknown(to.to_string()));
        }
        self.caller(from)?;
        self.asks
            .retain(|_, a| a.answer.is_none() || &a.from != from);
        let open = self
            .asks
            .values()
            .filter(|a| &a.from == from && a.answer.is_none() && a.deadline_ms > now_ms)
            .count();
        if open >= self.limits.max_open_asks {
            return Err(MeshError::TooManyOpenAsks);
        }
        let (m, recipients) = self.post(from, to, question, &[], None, "ask", now_ms)?;
        let Some(target) = recipients.into_iter().next() else {
            return Err(MeshError::RecipientUnknown(to.to_string()));
        };
        self.asks.insert(
            m.id.clone(),
            Ask {
                from: from.clone(),
                to: target,
                deadline_ms: now_ms
                    .saturating_add(timeout_ms.unwrap_or(self.limits.default_ask_timeout_ms)),
                answer: None,
            },
        );
        Ok(m.id)
    }

    pub fn ask_status(&mut self, ask_id: &str, now_ms: u64) -> AskStatus {
        let Some(a) = self.asks.get(ask_id) else {
            return AskStatus::TimedOut;
        };
        if let Some((reply, from)) = &a.answer {
            let out = AskStatus::Answered {
                reply: reply.clone(),
                from: from.clone(),
            };
            self.asks.remove(ask_id);
            return out;
        }
        if now_ms >= a.deadline_ms {
            self.asks.remove(ask_id);
            return AskStatus::TimedOut;
        }
        AskStatus::Pending
    }

    /// `inbox`: messages after `since` (by id), oldest first. Reading does not remove them; `since` moves the cursor.
    pub fn inbox(
        &self,
        agent: &AgentId,
        since: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Message>, MeshError> {
        self.caller(agent)?;
        let Some(inbox) = self.inboxes.get(agent) else {
            return Ok(Vec::new());
        };
        let number = |id: &str| id.rsplit('-').next().and_then(|n| n.parse::<u64>().ok());
        let after = since.and_then(number).unwrap_or(0);
        Ok(inbox
            .iter()
            .filter(|m| number(&m.id).is_some_and(|n| n > after))
            .take(limit.max(1))
            .cloned()
            .collect())
    }

    // ---- tasks ----

    fn task_changed(&mut self, id: &TaskId) {
        if let Some(t) = self.tasks.get(id) {
            let owner = match t.task.state() {
                TaskState::Open => String::new(),
                TaskState::Claimed(a) | TaskState::Done(a) => a.as_str().to_string(),
            };
            self.events.push(Body::TaskChanged(TaskChanged {
                task_id: id.as_str().to_string(),
                project_id: t.project.clone(),
                title: t.task.title.clone(),
                claimed_by_agent_id: owner,
                done: matches!(t.task.state(), TaskState::Done(_)),
            }));
        }
    }

    fn entry(&self, id: &TaskId, t: &TaskRec) -> TaskEntry {
        let (status, owner) = match t.task.state() {
            TaskState::Open => (TaskStatus::Open, None),
            TaskState::Claimed(a) => (TaskStatus::Claimed, Some(a.clone())),
            TaskState::Done(a) => (TaskStatus::Done, Some(a.clone())),
        };
        TaskEntry {
            id: id.clone(),
            project: t.project.clone(),
            title: t.task.title.clone(),
            detail: t.detail.clone(),
            depends_on: t.depends_on.clone(),
            status,
            owner,
            summary: t.summary.clone(),
        }
    }

    pub fn task_create(
        &mut self,
        by: &AgentId,
        title: &str,
        detail: &str,
        depends_on: &[String],
    ) -> Result<TaskId, MeshError> {
        let project = self.caller(by)?.project.clone();
        let deps: Vec<TaskId> = depends_on.iter().map(TaskId::new).collect();
        for d in &deps {
            if self.tasks.get(d).is_none_or(|t| t.project != project) {
                return Err(MeshError::DependencyUnknown(d.as_str().to_string()));
            }
        }
        let id = TaskId::new(self.id("t"));
        self.tasks.insert(
            id.clone(),
            TaskRec {
                task: Task::new(id.clone(), title),
                project,
                detail: detail.to_string(),
                depends_on: deps,
                summary: None,
            },
        );
        self.task_changed(&id);
        Ok(id)
    }

    /// `task_list` for the caller's project.
    pub fn task_list(
        &self,
        by: &AgentId,
        status: Option<TaskStatus>,
    ) -> Result<Vec<TaskEntry>, MeshError> {
        let project = &self.caller(by)?.project;
        Ok(self
            .tasks
            .iter()
            .filter(|(_, t)| &t.project == project)
            .map(|(id, t)| self.entry(id, t))
            .filter(|e| status.is_none_or(|s| e.status == s))
            .collect())
    }

    fn deps_done(&self, task: &str, t: &TaskRec) -> Result<(), MeshError> {
        for d in &t.depends_on {
            let done = self
                .tasks
                .get(d)
                .is_some_and(|x| matches!(x.task.state(), TaskState::Done(_)));
            if !done {
                return Err(MeshError::TaskBlocked {
                    task: task.to_string(),
                    depends_on: d.as_str().to_string(),
                });
            }
        }
        Ok(())
    }

    fn visible_task(&self, by: &AgentId, id: &str) -> Result<TaskId, MeshError> {
        let project = &self.caller(by)?.project;
        let tid = TaskId::new(id);
        match self.tasks.get(&tid) {
            Some(t) if &t.project == project => Ok(tid),
            _ => Err(MeshError::TaskUnknown(id.to_string())),
        }
    }

    pub fn task_claim(&mut self, by: &AgentId, id: &str) -> Result<(), MeshError> {
        let tid = self.visible_task(by, id)?;
        let rec = self
            .tasks
            .get(&tid)
            .ok_or_else(|| MeshError::TaskUnknown(id.into()))?;
        self.deps_done(id, rec)?;
        let rec = self
            .tasks
            .get_mut(&tid)
            .ok_or_else(|| MeshError::TaskUnknown(id.into()))?;
        rec.task
            .claim(by)
            .map_err(|_| MeshError::TaskNotOpen(id.to_string()))?;
        self.task_changed(&tid);
        Ok(())
    }

    pub fn task_complete(
        &mut self,
        by: &AgentId,
        id: &str,
        summary: &str,
    ) -> Result<(), MeshError> {
        let tid = self.visible_task(by, id)?;
        let rec = self
            .tasks
            .get_mut(&tid)
            .ok_or_else(|| MeshError::TaskUnknown(id.into()))?;
        rec.task
            .complete(by)
            .map_err(|_: TaskError| MeshError::NotTaskOwner(id.to_string()))?;
        rec.summary = Some(summary.to_string());
        self.task_changed(&tid);
        Ok(())
    }

    // ---- leases ----

    /// `lease_acquire`: all paths or none. Returns the batch id used by `lease_release`.
    pub fn lease_acquire(
        &mut self,
        by: &AgentId,
        paths: &[String],
        ttl_ms: u64,
        now_ms: u64,
    ) -> Result<String, MeshError> {
        let project = self.caller(by)?.project.clone();
        let table = self.leases.entry(project.clone()).or_default();
        let mut taken: Vec<String> = Vec::new();
        for p in paths {
            match table.acquire(p, by, now_ms, ttl_ms) {
                Ok(l) => taken.push(l.path),
                Err(e) => {
                    for t in &taken {
                        table.release(t, by);
                    }
                    return Err(match e {
                        LeaseError::Conflict { holder } => MeshError::LeaseHeld {
                            path: p.clone(),
                            holder,
                        },
                        LeaseError::InvalidPath => MeshError::LeaseInvalidPath(p.clone()),
                        LeaseError::ZeroTtl => MeshError::LeaseZeroTtl,
                    });
                }
            }
        }
        let id = self.id("l");
        for p in &taken {
            self.events.push(Body::LeaseGranted(LeaseGranted {
                agent_id: by.as_str().to_string(),
                path: p.clone(),
                expires_at_ms: now_ms.saturating_add(ttl_ms),
            }));
        }
        self.batches.insert(
            id.clone(),
            LeaseBatch {
                holder: by.clone(),
                project,
                paths: taken,
            },
        );
        Ok(id)
    }

    fn release_batch(&mut self, id: &str, _now_ms: u64) {
        let Some(b) = self.batches.remove(id) else {
            return;
        };
        if let Some(table) = self.leases.get_mut(&b.project) {
            for p in &b.paths {
                if table.release(p, &b.holder) {
                    self.events.push(Body::LeaseReleased(LeaseReleased {
                        agent_id: b.holder.as_str().to_string(),
                        path: p.clone(),
                    }));
                }
            }
        }
    }

    pub fn lease_release(
        &mut self,
        by: &AgentId,
        lease_id: &str,
        now_ms: u64,
    ) -> Result<(), MeshError> {
        match self.batches.get(lease_id) {
            Some(b) if &b.holder == by => {
                self.release_batch(lease_id, now_ms);
                Ok(())
            }
            _ => Err(MeshError::LeaseUnknown(lease_id.to_string())),
        }
    }

    /// Drops expired leases (and their batches) and records the releases.
    pub fn expire_leases(&mut self, now_ms: u64) {
        let projects: Vec<String> = self.leases.keys().cloned().collect();
        for p in projects {
            let expired = self
                .leases
                .get_mut(&p)
                .map(|t| t.expire(now_ms))
                .unwrap_or_default();
            for l in expired {
                self.events.push(Body::LeaseReleased(LeaseReleased {
                    agent_id: l.holder.as_str().to_string(),
                    path: l.path.clone(),
                }));
                for b in self.batches.values_mut() {
                    if b.holder == l.holder && b.project == p {
                        b.paths.retain(|x| *x != l.path);
                    }
                }
            }
        }
        self.batches.retain(|_, b| !b.paths.is_empty());
    }

    // ---- handoff ----

    /// `handoff`: tells `to` what is being handed over, and moves the task (if one is named) from `from` to `to`.
    /// The task changes owner only if `from` owns it and its dependencies are done; otherwise nothing changes.
    pub fn handoff(
        &mut self,
        from: &AgentId,
        to: &str,
        task_id: Option<&str>,
        summary: &str,
        refs: &[String],
        now_ms: u64,
    ) -> Result<String, MeshError> {
        let info = self.caller(from)?.clone();
        let target = self.recipients(&info, to)?;
        let (Some(target), true) = (target.first().cloned(), !to.starts_with('#')) else {
            return Err(MeshError::RecipientUnknown(to.to_string()));
        };
        let tid = match task_id {
            Some(id) => {
                let tid = self.visible_task(from, id)?;
                let rec = self
                    .tasks
                    .get(&tid)
                    .ok_or_else(|| MeshError::TaskUnknown(id.into()))?;
                if !matches!(rec.task.state(), TaskState::Claimed(a) if a == from) {
                    return Err(MeshError::NotTaskOwner(id.to_string()));
                }
                Some(tid)
            }
            None => None,
        };
        // Deliver first: if the message cannot go out (size, rate), the task must not move.
        let (m, _) = self.post(from, to, summary, refs, None, "handoff", now_ms)?;
        if let Some(tid) = &tid {
            if let Some(rec) = self.tasks.get_mut(tid) {
                let _ = rec.task.release(from);
                let _ = rec.task.claim(&target);
            }
            self.task_changed(tid);
        }
        let hid = self.id("h");
        self.events.push(Body::HandoffCreated(HandoffCreated {
            task_id: tid.map(|t| t.as_str().to_string()).unwrap_or_default(),
            from_agent_id: from.as_str().to_string(),
            to_agent_id: target.as_str().to_string(),
            summary: summary.to_string(),
            refs: refs.to_vec(),
        }));
        let _ = m;
        Ok(hid)
    }

    /// Ids of agents currently registered (for the audit and the UI).
    pub fn agent_ids(&self) -> BTreeSet<AgentId> {
        self.agents.keys().cloned().collect()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn agent(id: &str, project: &str, node: &str) -> AgentInfo {
        AgentInfo {
            id: AgentId::new(id),
            kind: "claude-code".into(),
            node: node.into(),
            project: project.into(),
            label: id.into(),
            worktree: "/wt".into(),
            state: "idle".into(),
        }
    }

    fn mesh() -> Mesh {
        let mut m = Mesh::new(Limits::default());
        m.register(agent("a", "p", "laptop"));
        m.register(agent("b", "p", "server"));
        m.register(agent("c", "other", "laptop"));
        m
    }

    fn id(s: &str) -> AgentId {
        AgentId::new(s)
    }

    #[test]
    fn a_message_arrives_with_provenance_set_by_the_broker() {
        let mut m = mesh();
        m.send_message(&id("a"), "b", "hello", &["src/x.rs".into()], None, 10)
            .unwrap();
        let got = m.inbox(&id("b"), None, 10).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].body, "hello");
        assert_eq!(got[0].refs, ["src/x.rs"]);
        assert_eq!(got[0].provenance.from, id("a"));
        assert_eq!(got[0].provenance.node, "laptop");
        assert_eq!(got[0].provenance.kind, "claude-code");
        // The sender's own inbox is not touched, and nobody in another project sees it.
        assert!(m.inbox(&id("a"), None, 10).unwrap().is_empty());
        assert!(m.inbox(&id("c"), None, 10).unwrap().is_empty());
    }

    #[test]
    fn channels_reach_the_projects_other_agents_only() {
        let mut m = mesh();
        m.register(agent("d", "p", "server"));
        m.send_message(&id("a"), "#dev", "standup", &[], None, 0)
            .unwrap();
        assert_eq!(m.inbox(&id("b"), None, 10).unwrap().len(), 1);
        assert_eq!(m.inbox(&id("d"), None, 10).unwrap().len(), 1);
        assert!(m.inbox(&id("a"), None, 10).unwrap().is_empty());
        assert!(m.inbox(&id("c"), None, 10).unwrap().is_empty());
    }

    #[test]
    fn unknown_recipients_and_unregistered_callers_are_refused() {
        let mut m = mesh();
        let e = m
            .send_message(&id("a"), "nobody", "x", &[], None, 0)
            .unwrap_err();
        assert_eq!(e, MeshError::RecipientUnknown("nobody".into()));
        assert_eq!(
            e.to_error(&Limits::default()).code,
            ErrorCode::MeshRecipientUnknown
        );
        // Not to oneself either.
        assert!(m.send_message(&id("a"), "a", "x", &[], None, 0).is_err());
        assert_eq!(
            m.send_message(&id("ghost"), "b", "x", &[], None, 0)
                .unwrap_err(),
            MeshError::UnknownAgent("ghost".into())
        );
    }

    #[test]
    fn inbox_pages_with_a_cursor() {
        let mut m = mesh();
        for i in 0..5 {
            m.send_message(&id("a"), "b", &format!("m{i}"), &[], None, i)
                .unwrap();
        }
        let first = m.inbox(&id("b"), None, 2).unwrap();
        assert_eq!(
            first.iter().map(|x| x.body.as_str()).collect::<Vec<_>>(),
            ["m0", "m1"]
        );
        let next = m.inbox(&id("b"), Some(&first[1].id), 10).unwrap();
        assert_eq!(
            next.iter().map(|x| x.body.as_str()).collect::<Vec<_>>(),
            ["m2", "m3", "m4"]
        );
    }

    #[test]
    fn size_and_rate_limits_stop_loops() {
        let mut m = Mesh::new(Limits {
            messages_per_window: 3,
            max_body_bytes: 10,
            max_refs: 1,
            ..Limits::default()
        });
        m.register(agent("a", "p", "n"));
        m.register(agent("b", "p", "n"));
        assert_eq!(
            m.send_message(&id("a"), "b", "way too long body", &[], None, 0),
            Err(MeshError::BodyTooLarge)
        );
        assert_eq!(
            m.send_message(&id("a"), "b", "x", &["1".into(), "2".into()], None, 0),
            Err(MeshError::TooManyRefs)
        );
        for t in 0..3 {
            m.send_message(&id("a"), "b", "x", &[], None, t).unwrap();
        }
        assert_eq!(
            m.send_message(&id("a"), "b", "x", &[], None, 3),
            Err(MeshError::RateLimited)
        );
        // The window slides: later, sending works again. Refused messages did not count against the budget.
        m.send_message(&id("a"), "b", "x", &[], None, 60_000)
            .unwrap();
    }

    #[test]
    fn ask_is_answered_by_a_reply_and_times_out_otherwise() {
        let mut m = mesh();
        let ask = m.ask(&id("a"), "b", "which port?", Some(1000), 0).unwrap();
        assert_eq!(m.ask_status(&ask, 10), AskStatus::Pending);
        // The recipient sees the question as a message of kind "ask" and replies to it.
        let q = m.inbox(&id("b"), None, 10).unwrap();
        assert_eq!(q[0].kind, "ask");
        // Someone else replying does not answer it.
        m.register(agent("d", "p", "n"));
        m.send_message(&id("d"), "a", "i know!", &[], Some(&ask), 20)
            .unwrap();
        assert_eq!(m.ask_status(&ask, 30), AskStatus::Pending);
        m.send_message(&id("b"), "a", "8080", &[], Some(&ask), 40)
            .unwrap();
        assert_eq!(
            m.ask_status(&ask, 50),
            AskStatus::Answered {
                reply: "8080".into(),
                from: id("b")
            }
        );
        // An unanswered ask times out.
        let slow = m.ask(&id("a"), "b", "hello?", Some(100), 0).unwrap();
        assert_eq!(m.ask_status(&slow, 100), AskStatus::TimedOut);
    }

    #[test]
    fn open_asks_are_capped() {
        let mut m = Mesh::new(Limits {
            max_open_asks: 2,
            ..Limits::default()
        });
        m.register(agent("a", "p", "n"));
        m.register(agent("b", "p", "n"));
        m.ask(&id("a"), "b", "1", None, 0).unwrap();
        m.ask(&id("a"), "b", "2", None, 0).unwrap();
        assert_eq!(
            m.ask(&id("a"), "b", "3", None, 0),
            Err(MeshError::TooManyOpenAsks)
        );
        assert!(
            m.ask(&id("a"), "#all", "?", None, 0).is_err(),
            "asks go to one agent"
        );
    }

    #[test]
    fn tasks_follow_dependencies_and_owners() {
        let mut m = mesh();
        let t1 = m
            .task_create(&id("a"), "write parser", "details", &[])
            .unwrap();
        let t2 = m
            .task_create(&id("a"), "write tests", "", &[t1.as_str().to_string()])
            .unwrap();
        // A dependency must exist in the same project.
        assert!(matches!(
            m.task_create(&id("a"), "x", "", &["t-999".into()]),
            Err(MeshError::DependencyUnknown(_))
        ));
        assert_eq!(
            m.task_list(&id("b"), Some(TaskStatus::Open)).unwrap().len(),
            2
        );
        // t2 is blocked until t1 is done.
        assert_eq!(
            m.task_claim(&id("b"), t2.as_str()),
            Err(MeshError::TaskBlocked {
                task: t2.as_str().into(),
                depends_on: t1.as_str().into()
            })
        );
        m.task_claim(&id("a"), t1.as_str()).unwrap();
        assert_eq!(
            m.task_claim(&id("b"), t1.as_str()),
            Err(MeshError::TaskNotOpen(t1.as_str().into()))
        );
        assert_eq!(
            m.task_complete(&id("b"), t1.as_str(), "no"),
            Err(MeshError::NotTaskOwner(t1.as_str().into()))
        );
        m.task_complete(&id("a"), t1.as_str(), "parser done")
            .unwrap();
        m.task_claim(&id("b"), t2.as_str()).unwrap();
        let done = m.task_list(&id("a"), Some(TaskStatus::Done)).unwrap();
        assert_eq!(done[0].summary.as_deref(), Some("parser done"));
        // Another project cannot see or touch these tasks.
        assert!(m.task_list(&id("c"), None).unwrap().is_empty());
        assert!(matches!(
            m.task_claim(&id("c"), t1.as_str()),
            Err(MeshError::TaskUnknown(_))
        ));
    }

    #[test]
    fn leases_are_all_or_nothing_and_conflicts_name_the_holder() {
        let mut m = mesh();
        let l = m
            .lease_acquire(&id("a"), &["src/a.rs".into(), "src/b".into()], 1000, 0)
            .unwrap();
        let err = m
            .lease_acquire(
                &id("b"),
                &["docs/x.md".into(), "src/b/deep.rs".into()],
                1000,
                10,
            )
            .unwrap_err();
        assert_eq!(
            err,
            MeshError::LeaseHeld {
                path: "src/b/deep.rs".into(),
                holder: id("a")
            }
        );
        assert_eq!(err.to_error(&Limits::default()).code, ErrorCode::LeaseHeld);
        // Nothing of the failed batch was kept: docs/x.md is free.
        m.lease_acquire(&id("b"), &["docs/x.md".into()], 1000, 20)
            .unwrap();
        assert_eq!(
            m.lease_release(&id("b"), &l, 30),
            Err(MeshError::LeaseUnknown(l.clone())),
            "not the holder"
        );
        m.lease_release(&id("a"), &l, 30).unwrap();
        m.lease_acquire(&id("b"), &["src/b/deep.rs".into()], 1000, 40)
            .unwrap();
        // Leases are per project.
        m.lease_acquire(&id("c"), &["src/a.rs".into()], 1000, 50)
            .unwrap();
    }

    #[test]
    fn expired_leases_free_the_path_and_are_recorded() {
        let mut m = mesh();
        m.lease_acquire(&id("a"), &["f.rs".into()], 100, 0).unwrap();
        m.take_events();
        m.expire_leases(100);
        assert!(matches!(
            m.take_events().as_slice(),
            [Body::LeaseReleased(_)]
        ));
        m.lease_acquire(&id("b"), &["f.rs".into()], 100, 100)
            .unwrap();
    }

    #[test]
    fn handoff_moves_the_task_and_tells_the_receiver() {
        let mut m = mesh();
        let t = m.task_create(&id("a"), "refactor", "", &[]).unwrap();
        m.task_claim(&id("a"), t.as_str()).unwrap();
        m.handoff(
            &id("a"),
            "b",
            Some(t.as_str()),
            "parser is done, wire it up",
            &["commit:abc".into()],
            5,
        )
        .unwrap();
        let e = m.task_list(&id("a"), None).unwrap().remove(0);
        assert_eq!((e.status, e.owner), (TaskStatus::Claimed, Some(id("b"))));
        let msg = m.inbox(&id("b"), None, 10).unwrap();
        assert_eq!(
            (msg[0].kind, msg[0].body.as_str()),
            ("handoff", "parser is done, wire it up")
        );
        // Only the owner can hand a task off.
        assert_eq!(
            m.handoff(&id("a"), "b", Some(t.as_str()), "again", &[], 6),
            Err(MeshError::NotTaskOwner(t.as_str().into()))
        );
        // A handoff whose message cannot be sent moves nothing.
        let t2 = m.task_create(&id("a"), "other", "", &[]).unwrap();
        m.task_claim(&id("a"), t2.as_str()).unwrap();
        let huge = "x".repeat(Limits::default().max_body_bytes + 1);
        assert_eq!(
            m.handoff(&id("a"), "b", Some(t2.as_str()), &huge, &[], 7),
            Err(MeshError::BodyTooLarge)
        );
        let still = m.task_list(&id("a"), Some(TaskStatus::Claimed)).unwrap();
        assert!(still.iter().any(|x| x.id == t2 && x.owner == Some(id("a"))));
    }

    #[test]
    fn an_agent_that_ends_gives_everything_back() {
        let mut m = mesh();
        let t = m.task_create(&id("a"), "t", "", &[]).unwrap();
        m.task_claim(&id("a"), t.as_str()).unwrap();
        m.lease_acquire(&id("a"), &["x.rs".into()], 10_000, 0)
            .unwrap();
        let ask = m.ask(&id("b"), "a", "?", None, 0).unwrap();
        m.unregister(&id("a"), 10);
        assert_eq!(
            m.task_list(&id("b"), Some(TaskStatus::Open)).unwrap().len(),
            1
        );
        m.lease_acquire(&id("b"), &["x.rs".into()], 100, 20)
            .unwrap();
        assert_eq!(
            m.ask_status(&ask, 30),
            AskStatus::TimedOut,
            "asks involving a dead agent are dropped"
        );
        assert!(!m.agent_ids().contains(&id("a")));
    }

    #[test]
    fn every_change_is_audited() {
        let mut m = mesh();
        m.send_message(&id("a"), "b", "hi", &[], None, 0).unwrap();
        let t = m.task_create(&id("a"), "t", "", &[]).unwrap();
        m.task_claim(&id("a"), t.as_str()).unwrap();
        m.lease_acquire(&id("a"), &["f".into()], 10, 0).unwrap();
        m.handoff(&id("a"), "b", Some(t.as_str()), "s", &[], 1)
            .unwrap();
        let kinds: Vec<&str> = m
            .take_events()
            .iter()
            .map(|b| match b {
                Body::MessagePosted(_) => "message",
                Body::TaskChanged(_) => "task",
                Body::LeaseGranted(_) => "lease",
                Body::HandoffCreated(_) => "handoff",
                _ => "other",
            })
            .collect();
        assert_eq!(
            kinds,
            ["message", "task", "task", "lease", "message", "task", "handoff"]
        );
        assert!(m.take_events().is_empty());
    }

    #[test]
    fn agents_list_filters_by_project() {
        let m = mesh();
        assert_eq!(m.agents_list(Some("p")).len(), 2);
        assert_eq!(m.agents_list(None).len(), 3);
    }

    mod remote {
        use super::*;

        fn info(id: &str, node: &str) -> AgentInfo {
            AgentInfo {
                id: AgentId::new(id),
                kind: "shell".into(),
                node: node.into(),
                project: "p".into(),
                label: String::new(),
                worktree: String::new(),
                state: "idle".into(),
            }
        }

        fn mesh() -> Mesh {
            let mut m = Mesh::new(Limits::default());
            m.register(info("a1", "n1"));
            m.register(info("a2", "n1"));
            m
        }

        #[test]
        fn a_message_to_another_node_goes_to_the_outbox_not_an_inbox() {
            let mut m = mesh();
            let id = m
                .send_message(&AgentId::new("a1"), "n2/b7", "hi", &[], None, 1)
                .unwrap();
            let out = m.take_outbound();
            assert_eq!(out.len(), 1);
            assert_eq!(
                (
                    out[0].to_node.as_str(),
                    out[0].to_agent.as_str(),
                    out[0].body.as_str()
                ),
                ("n2", "b7", "hi")
            );
            assert_eq!(out[0].message_id, id);
            assert!(m.take_outbound().is_empty());
            assert!(m.inbox(&AgentId::new("a2"), None, 10).unwrap().is_empty());
        }

        #[test]
        fn your_own_node_name_in_front_means_here() {
            let mut m = mesh();
            m.send_message(&AgentId::new("a1"), "n1/a2", "hi", &[], None, 1)
                .unwrap();
            assert!(m.take_outbound().is_empty());
            assert_eq!(m.inbox(&AgentId::new("a2"), None, 10).unwrap().len(), 1);
        }

        #[test]
        fn asks_handoffs_and_odd_addresses_stay_refused() {
            let mut m = mesh();
            let a1 = AgentId::new("a1");
            assert!(matches!(
                m.ask(&a1, "n2/b7", "q?", None, 1),
                Err(MeshError::RecipientUnknown(_))
            ));
            assert!(matches!(
                m.handoff(&a1, "n2/b7", None, "s", &[], 1),
                Err(MeshError::RecipientUnknown(_))
            ));
            for odd in ["n2/", "/b7", "n2/b7/x", "n2/b7/"] {
                assert!(
                    m.send_message(&a1, odd, "hi", &[], None, 1).is_err(),
                    "{odd}"
                );
            }
            assert!(
                m.take_outbound().is_empty(),
                "nothing queued by a refused call"
            );
        }

        #[test]
        fn remote_sends_count_against_the_rate_limit_and_the_size_limit() {
            let mut m = mesh();
            let a1 = AgentId::new("a1");
            let big = "x".repeat(m.limits().max_body_bytes + 1);
            assert!(matches!(
                m.send_message(&a1, "n2/b", &big, &[], None, 1),
                Err(MeshError::BodyTooLarge)
            ));
            let n = m.limits().messages_per_window;
            for _ in 0..n {
                m.send_message(&a1, "n2/b", "x", &[], None, 5).unwrap();
            }
            assert!(matches!(
                m.send_message(&a1, "n2/b", "x", &[], None, 5),
                Err(MeshError::RateLimited)
            ));
        }

        #[test]
        fn an_inbound_message_shows_where_it_came_from_and_can_be_answered() {
            let mut m = mesh();
            let id = m
                .deliver_remote("n2", "b7", &AgentId::new("a1"), "hello", 9)
                .unwrap();
            let got = m.inbox(&AgentId::new("a1"), None, 10).unwrap();
            assert_eq!(got.len(), 1);
            assert_eq!(
                (
                    got[0].id.as_str(),
                    got[0].provenance.from.as_str(),
                    got[0].provenance.node.as_str()
                ),
                (id.as_str(), "n2/b7", "n2")
            );
            // Answering is a send to that address: it leaves through the outbox.
            m.send_message(&AgentId::new("a1"), "n2/b7", "hi back", &[], None, 10)
                .unwrap();
            assert_eq!(m.take_outbound().len(), 1);
            assert!(!m.take_events().is_empty());
        }

        #[test]
        fn inbound_for_nobody_or_from_nobody_is_refused() {
            let mut m = mesh();
            assert!(m
                .deliver_remote("n2", "b7", &AgentId::new("ghost"), "x", 1)
                .is_err());
            assert!(m
                .deliver_remote("", "b7", &AgentId::new("a1"), "x", 1)
                .is_err());
            assert!(m
                .deliver_remote("n2", "", &AgentId::new("a1"), "x", 1)
                .is_err());
            assert!(m
                .deliver_remote("n2", "b/7", &AgentId::new("a1"), "x", 1)
                .is_err());
            let big = "x".repeat(m.limits().max_body_bytes + 1);
            assert!(m
                .deliver_remote("n2", "b7", &AgentId::new("a1"), &big, 1)
                .is_err());
        }

        #[test]
        fn a_failed_delivery_reaches_the_sender_as_a_notice() {
            let mut m = mesh();
            m.delivery_failed(&AgentId::new("a1"), "n2/b7", "node.offline", 3);
            let got = m.inbox(&AgentId::new("a1"), None, 10).unwrap();
            assert_eq!(
                (got[0].kind, got[0].body.as_str()),
                ("delivery_failed", "node.offline")
            );
            m.delivery_failed(&AgentId::new("gone"), "n2/b7", "x", 3); // an agent that left: nothing happens
        }
    }
}
