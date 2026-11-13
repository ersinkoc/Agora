//! Agents that start agents (ADR-0057): the node's side of `agent_start`, `agent_wait` and `agent_output`.
//!
//! What is enforced here, before and after the user decides:
//! - every start needs the user's decision, signed by a trusted device and verified on this node (ADR-0020); the decision covers a
//!   hash of the request as announced, so what starts is what the user read (`approvals::start_signed_message`);
//! - a signed grant may cover more starts of the same kind in the same folder for the asking agent; it ends when that agent exits
//!   or is stopped, and is never given for a folder outside the asking agent's own;
//! - the helper is never wider than the asking agent: its folder is the asker's folder, below it or a worktree of it (any other
//!   folder only by a decision that names it), and on a node that confines agents a helper of another kind may not bring a manifest
//!   the owner trusts to widen its confinement;
//! - a helper cannot start helpers unless the decision that started it allowed it, and a helper's helper never can;
//! - at most [`MAX_HELPERS`] running helpers per agent, one start waiting for the user at a time, [`STARTS_PER_MINUTE`] attempts a
//!   minute, the node's own agent limit, and a first prompt of at most [`MAX_PROMPT`] bytes;
//! - `agent_wait` and `agent_output` answer only for helpers the caller started; output is the bottom of the live screen (never
//!   scrollback, ADR detection rule) or the end of a chat agent's transcript, size-limited.
//!
//! Every refusal before asking is an `AgentStartRefused` event, and every request, decision and start is in the event log.
//! Prompts, kinds and folders come from an agent: they are untrusted data and are only shown, never interpreted.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use agora_core::{AgentId, AgentState};
use agora_protocol::v1;
use agora_protocol::{Error, ErrorCode};
use serde_json::{json, Value};

use crate::approvals::{HelperGrant, NodeApprovals, Pending};
use crate::daemon::EventSink;
use crate::mesh_server::{Helpers, StartArgs, StartOutcome};
use crate::supervisor::{AgentView, StartRequest, Supervisor};

/// Running helpers one agent may have at once.
pub const MAX_HELPERS: usize = 3;
/// `agent_start` calls one agent may make in a minute (refused ones count).
pub const STARTS_PER_MINUTE: usize = 6;
/// The longest first prompt, in bytes; it is shown in full, so a longer one is refused rather than cut.
pub const MAX_PROMPT: usize = 16 * 1024;
/// The most starts one grant may cover.
pub const MAX_GRANT: u32 = 10;
/// The deepest helper: a user-started agent is 0, its helper 1, a helper's helper 2.
pub const MAX_DEPTH: u8 = 2;
/// Most bytes `agent_output` returns.
pub const MAX_OUTPUT_BYTES: usize = 16 * 1024;
/// A helper's first prompt is typed once its screen settles, or after this long.
const PROMPT_FALLBACK: Duration = Duration::from_secs(8);
/// Between the prompt text and its Enter (a program that reads both at once takes them for a paste).
const ENTER_AFTER: Duration = Duration::from_millis(150);

/// Who started a helper, and what the decision allowed it.
#[derive(Debug, Clone)]
struct Lineage {
    parent: AgentId,
    depth: u8,
    may_start: bool,
}

/// A request waiting for the user.
#[derive(Debug, Clone)]
struct Request {
    caller: AgentId,
    kind: String,
    folder: String,
    inside: bool,
    worktree: bool,
    prompt: String,
    depth: u8,
    ticket: u64,
}

struct Prompt {
    agent: AgentId,
    text: String,
    since: Instant,
    typed: Option<Instant>,
}

#[derive(Default)]
struct State {
    requests: HashMap<String, Request>,
    tickets: HashMap<u64, Option<StartOutcome>>,
    next_ticket: u64,
    lineage: HashMap<AgentId, Lineage>,
    attempts: HashMap<AgentId, VecDeque<Instant>>,
    prompts: Vec<Prompt>,
}

pub struct Spawner {
    node_id: String,
    worktree_root: PathBuf,
    sup: Arc<Mutex<Supervisor>>,
    approvals: Arc<NodeApprovals>,
    events: Option<Arc<EventSink>>,
    chat: Arc<crate::chat::ChatBoard>,
    git_wake: Option<Arc<AtomicBool>>,
    names: AtomicU64,
    state: Mutex<State>,
}

fn helper_limit(rule: &str, limit: usize) -> Error {
    Error::new(ErrorCode::MeshHelperLimit)
        .with("rule", rule)
        .with("limit", i64::try_from(limit).unwrap_or(i64::MAX))
}

fn not_found(agent: &str) -> Error {
    Error::new(ErrorCode::AgentNotFound).with("agent_id", agent)
}

fn state_word(v: &AgentView) -> &'static str {
    if !v.running {
        return "exited";
    }
    match v.state {
        AgentState::Working => "working",
        AgentState::Blocked => "blocked",
        AgentState::Idle => "idle",
        AgentState::Done => "done",
        AgentState::Unknown => "unknown",
    }
}

/// `child` is `parent` or below it, both canonical.
fn within(child: &Path, parent: &Path) -> bool {
    child.starts_with(parent)
}

impl Spawner {
    pub fn new(
        node_id: &str,
        worktree_root: &Path,
        sup: Arc<Mutex<Supervisor>>,
        approvals: Arc<NodeApprovals>,
        events: Option<Arc<EventSink>>,
        chat: Arc<crate::chat::ChatBoard>,
    ) -> Self {
        Self {
            node_id: node_id.to_string(),
            worktree_root: worktree_root.to_path_buf(),
            sup,
            approvals,
            events,
            chat,
            git_wake: None,
            names: AtomicU64::new(0),
            state: Mutex::new(State::default()),
        }
    }

    /// Wakes the node's Git watcher after a helper starts (as `ControlContext::with_git_wake`).
    #[must_use]
    pub fn with_git_wake(mut self, wake: Arc<AtomicBool>) -> Self {
        self.git_wake = Some(wake);
        self
    }

    fn st(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn sup(&self) -> MutexGuard<'_, Supervisor> {
        self.sup.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn refused(&self, caller: &AgentId, kind: &str, folder: &str, e: Error) -> Error {
        if let Some(ev) = &self.events {
            ev.agent_start_refused(caller, kind, folder, e.code.as_str());
        }
        e
    }

    fn new_ticket(st: &mut State, outcome: Option<StartOutcome>) -> u64 {
        st.next_ticket += 1;
        let t = st.next_ticket;
        st.tickets.insert(t, outcome);
        t
    }

    /// Everything decided before the user is asked. Returns the request (without its ticket) or the refusal.
    fn check(&self, caller: &AgentId, a: &StartArgs) -> Result<Request, Error> {
        if a.node
            .as_deref()
            .is_some_and(|n| !n.is_empty() && n != self.node_id)
        {
            // ADR-0057 allows another machine only when the approval names it; starting there is not built yet, so it is refused.
            return Err(Error::new(ErrorCode::ProtocolCapabilityMissing)
                .with("capability", "mesh.spawn.remote"));
        }
        {
            let mut st = self.st();
            let now = Instant::now();
            let seen = st.attempts.entry(caller.clone()).or_default();
            while seen
                .front()
                .is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(60))
            {
                seen.pop_front();
            }
            if seen.len() >= STARTS_PER_MINUTE {
                return Err(Error::new(ErrorCode::MeshRateLimited));
            }
            seen.push_back(now);
        }
        if a.prompt.len() > MAX_PROMPT {
            return Err(helper_limit("prompt", MAX_PROMPT));
        }
        // Control characters would be keys the user never saw (escape sequences, Enter in the middle, Ctrl-C) when typed into a
        // terminal, and could hide text in the request: only line breaks and tabs are allowed.
        if a.prompt
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
        {
            return Err(Error::new(ErrorCode::ProtocolInvalidMessage).with("field", "prompt"));
        }
        let (views, kinds, confined, trusted) = {
            let sup = self.sup();
            let p = sup.sandbox_policy();
            (
                sup.list(),
                sup.kinds(),
                p.level != crate::sandbox::Level::Off,
                p.trusted_kinds.clone(),
            )
        };
        let me = views
            .iter()
            .find(|v| v.id == *caller && v.running)
            .ok_or_else(|| not_found(caller.as_str()))?;
        let depth = {
            let st = self.st();
            let mine = st.lineage.get(caller);
            if let Some(l) = mine {
                // A helper asks only if the decision that started it allowed it, and a helper's helper never does.
                if !l.may_start || l.depth >= MAX_DEPTH {
                    return Err(helper_limit("depth", usize::from(MAX_DEPTH)));
                }
            }
            let running = views
                .iter()
                .filter(|v| v.running && st.lineage.get(&v.id).is_some_and(|l| l.parent == *caller))
                .count();
            if running >= MAX_HELPERS {
                return Err(helper_limit("helpers", MAX_HELPERS));
            }
            if st.requests.values().any(|r| r.caller == *caller) {
                return Err(helper_limit("pending", 1));
            }
            mine.map_or(1, |l| l.depth + 1)
        };
        if !kinds.iter().any(|k| k.kind == a.kind) {
            return Err(Error::new(ErrorCode::AgentKindUnknown).with("kind", a.kind.as_str()));
        }
        // The ceiling on confinement, the same on every platform: with agents confined, a helper of another kind gets the node's
        // profile plus what its manifest asks only if the owner trusts that manifest, which would make it wider than the asker.
        if confined && a.kind != me.kind && trusted.contains(&a.kind) {
            return Err(Error::new(ErrorCode::NodePolicyDenied)
                .with("node_id", self.node_id.as_str())
                .with("rule", "sandbox.trust_manifest"));
        }
        let mine = agora_platform::canonicalize_plain(&me.worktree)
            .map_err(|_| not_found(caller.as_str()))?;
        let wanted = a
            .folder
            .as_deref()
            .filter(|f| !f.is_empty())
            .map_or_else(|| mine.clone(), PathBuf::from);
        let folder = agora_platform::canonicalize_plain(&wanted).map_err(|_| {
            Error::new(ErrorCode::MeshFolderNotAllowed)
                .with("folder", wanted.to_string_lossy().as_ref())
        })?;
        // The folder rules of ADR-0052 (refused folders, identity) apply as to any launch.
        let (info, _) = crate::directory::inspect(&folder.to_string_lossy(), &self.node_id, false)?;
        if a.worktree && info.git_root.is_empty() {
            return Err(
                Error::new(ErrorCode::ProtocolInvalidMessage).with("field", "directory.git_root")
            );
        }
        let inside = within(&folder, &mine);
        // A helper below the user's agent may go outside the asker's folder only by a decision naming it; a helper's helper, whose
        // decision the user gave to its parent's chain, stays inside.
        if !inside && depth > 1 {
            return Err(
                Error::new(ErrorCode::MeshFolderNotAllowed).with("folder", info.path.as_str())
            );
        }
        Ok(Request {
            caller: caller.clone(),
            kind: a.kind.clone(),
            folder: info.path,
            inside,
            worktree: a.worktree,
            prompt: a.prompt.clone(),
            depth,
            ticket: 0,
        })
    }

    fn detail(&self, r: &Request, caller_kind: &str) -> String {
        json!({
            "node": self.node_id,
            "agent": r.caller.as_str(),
            "agent_kind": caller_kind,
            "kind": r.kind,
            "folder": r.folder,
            "inside": r.inside,
            "worktree": r.worktree,
            "depth": r.depth,
            "prompt": r.prompt,
        })
        .to_string()
    }

    /// Starts the helper the user (or a grant) allowed. Limits are checked again: things changed while the user read.
    fn launch(&self, r: &Request, may_start: bool) -> StartOutcome {
        let running = {
            let st = self.st();
            self.sup()
                .list()
                .iter()
                .filter(|v| {
                    v.running && st.lineage.get(&v.id).is_some_and(|l| l.parent == r.caller)
                })
                .count()
        };
        if running >= MAX_HELPERS {
            return Err(helper_limit("helpers", MAX_HELPERS));
        }
        let (info, project) = crate::directory::inspect(&r.folder, &self.node_id, false)?;
        // The folder that was shown is the folder used: a path that now resolves elsewhere is not followed.
        if info.path != r.folder {
            return Err(
                Error::new(ErrorCode::MeshFolderNotAllowed).with("folder", r.folder.as_str())
            );
        }
        let n = self.names.fetch_add(1, Ordering::SeqCst);
        let stamp = crate::grants::now_ms();
        let req = StartRequest {
            kind: r.kind.clone(),
            checkout: PathBuf::from(&info.path),
            worktree_root: self.worktree_root.clone(),
            name: format!("{}-{stamp:x}h{n}", r.kind),
            base: None,
        };
        let mut sup = self.sup();
        let id = sup
            .start_selected(
                &req,
                project,
                Path::new(&info.git_root),
                r.worktree,
                Instant::now(),
            )
            .map_err(|e| e.to_protocol())?;
        sup.set_parent(&id, &r.caller);
        let view = sup.list().into_iter().find(|v| v.id == id);
        let has_terminal = sup.terminal(&id).is_some();
        drop(sup);
        self.st().lineage.insert(
            id.clone(),
            Lineage {
                parent: r.caller.clone(),
                depth: r.depth,
                may_start: may_start && r.depth < MAX_DEPTH,
            },
        );
        let path = view
            .as_ref()
            .map(|v| v.worktree.to_string_lossy().into_owned())
            .unwrap_or_default();
        if let Some(ev) = &self.events {
            let git_root = if r.worktree {
                let root = req.worktree_root.join(&req.name);
                agora_platform::canonicalize_plain(&root)
                    .unwrap_or(root)
                    .to_string_lossy()
                    .into_owned()
            } else {
                info.git_root.clone()
            };
            ev.helper_launched(
                &id,
                &v1::WorkspaceRegistered {
                    workspace_id: format!("directory:{path}"),
                    node_id: self.node_id.clone(),
                    project_id: info.project_id.clone(),
                    path,
                    git_root,
                    branch: if r.worktree {
                        format!("agent/{}", req.name)
                    } else {
                        info.branch.clone()
                    },
                    managed_worktree: r.worktree,
                    files_changed: 0,
                },
                &r.kind,
                has_terminal,
                r.caller.as_str(),
            );
        }
        if let Some(w) = &self.git_wake {
            w.store(true, Ordering::Release);
        }
        if !r.prompt.is_empty() {
            self.st().prompts.push(Prompt {
                agent: id.clone(),
                text: r.prompt.clone(),
                since: Instant::now(),
                typed: None,
            });
        }
        Ok(id.as_str().to_string())
    }

    /// The user's verified decision on an `agent.start` request (called after `NodeApprovals::resolve_request` succeeded).
    pub fn decided(&self, request: &Pending, r: &v1::ApprovalResolve) {
        let Some(req) = self.st().requests.remove(&request.id) else {
            return;
        };
        let (outcome, grant, nested) = if r.approved {
            // A grant and permission to go deeper are honoured only inside the asker's own folder.
            let grant = if req.inside {
                r.helper_grant.min(MAX_GRANT)
            } else {
                0
            };
            let nested = req.inside && r.helpers_may_start;
            (self.launch(&req, nested), grant, nested)
        } else {
            (
                Err(Error::new(ErrorCode::MeshStartDenied).with("approval_id", request.id.as_str())),
                0,
                false,
            )
        };
        if outcome.is_ok() && grant > 1 {
            self.approvals.add_helper_grant(HelperGrant {
                id: request.id.clone(),
                agent: req.caller.clone(),
                kind: req.kind.clone(),
                folder: req.folder.clone(),
                remaining: grant - 1,
                helpers_may_start: nested,
                signer_device_id: r.signer_device_id.clone(),
            });
        }
        if let Some(ev) = &self.events {
            ev.start_resolved(&request.id, r.approved, &r.signer_device_id, grant, nested);
        }
        if let Some(t) = self.st().tickets.get_mut(&req.ticket) {
            *t = Some(outcome);
        }
    }

    /// A request that ran out of time; `false` if it was not a start request.
    pub fn expired(&self, approval_id: &str) -> bool {
        let Some(req) = self.st().requests.remove(approval_id) else {
            return false;
        };
        if let Some(ev) = &self.events {
            ev.approval_resolved(approval_id, false, "");
        }
        if let Some(t) = self.st().tickets.get_mut(&req.ticket) {
            *t = Some(Err(
                Error::new(ErrorCode::ApprovalExpired).with("approval_id", approval_id)
            ));
        }
        true
    }

    /// The agent exited or was stopped: its grants end, and a start it was waiting for is withdrawn.
    pub fn agent_ended(&self, agent: &AgentId) {
        self.approvals.end_helper_grants(agent);
        let withdrawn: Vec<String> = {
            let mut st = self.st();
            let ids: Vec<String> = st
                .requests
                .iter()
                .filter(|(_, r)| r.caller == *agent)
                .map(|(id, _)| id.clone())
                .collect();
            for id in &ids {
                if let Some(r) = st.requests.remove(id) {
                    st.tickets.remove(&r.ticket);
                }
            }
            st.prompts.retain(|p| p.agent != *agent);
            ids
        };
        for id in withdrawn {
            self.approvals.withdraw(&id);
            if let Some(ev) = &self.events {
                ev.approval_resolved(&id, false, "");
            }
        }
    }

    /// The helpers `agent` started, and theirs, that still run (for "stop its helpers too").
    pub fn running_helpers(&self, agent: &AgentId) -> Vec<AgentId> {
        let views = self.sup().list();
        let st = self.st();
        let mut out = Vec::new();
        let mut todo = vec![agent.clone()];
        while let Some(p) = todo.pop() {
            for (child, l) in &st.lineage {
                if l.parent == p && !out.contains(child) {
                    todo.push(child.clone());
                    if views.iter().any(|v| v.id == *child && v.running) {
                        out.push(child.clone());
                    }
                }
            }
        }
        out
    }

    /// Types each new helper's first prompt once its screen settled (or after a while). Called on the node's tick.
    pub fn tick(&self) {
        let due: Vec<(AgentId, String, bool)> = {
            let mut st = self.st();
            if st.prompts.is_empty() {
                return;
            }
            let views = self.sup().list();
            let mut due = Vec::new();
            st.prompts.retain_mut(|p| {
                let Some(v) = views.iter().find(|v| v.id == p.agent && v.running) else {
                    return false;
                };
                match p.typed {
                    Some(at) if at.elapsed() >= ENTER_AFTER => {
                        due.push((p.agent.clone(), String::new(), true));
                        false
                    }
                    Some(_) => true,
                    None => {
                        let settled = matches!(v.state, AgentState::Idle | AgentState::Blocked);
                        if settled || p.since.elapsed() >= PROMPT_FALLBACK {
                            due.push((p.agent.clone(), p.text.clone(), false));
                            if v.terminal {
                                p.typed = Some(Instant::now());
                                return true;
                            }
                            return false;
                        }
                        true
                    }
                }
            });
            due
        };
        for (agent, text, enter) in due {
            let sup = self.sup();
            match sup.terminal(&agent).and_then(|t| t.input_handle().ok()) {
                Some(control) => {
                    drop(sup);
                    // In a terminal a line break would be Enter mid-prompt: it is typed as a space, so the prompt goes as one request.
                    let typed = text.replace(['\n', '\t'], " ");
                    let bytes: &[u8] = if enter { b"\r" } else { typed.as_bytes() };
                    let _ = control.write_input(bytes);
                }
                None if !enter && sup.adapter_prompt(&agent, &text, false).is_ok() => {
                    drop(sup);
                    self.chat.push(
                        &agent,
                        v1::ChatRole::User,
                        v1::ChatKind::Text,
                        text,
                        String::new(),
                        crate::daemon::now_ms(),
                    );
                }
                None => {}
            }
        }
    }

    fn helper_of(&self, caller: &AgentId, helper: &str) -> Result<AgentId, Error> {
        let id = AgentId::new(helper);
        match self.st().lineage.get(&id) {
            Some(l) if l.parent == *caller => Ok(id),
            // Another agent's helper, or no agent at all: the same answer, so a caller learns nothing about agents it did not start.
            _ => Err(not_found(helper)),
        }
    }
}

/// The last `max` bytes of `lines` joined, cut at a line boundary.
fn tail_within(lines: Vec<String>, max: usize) -> Vec<String> {
    let mut out = VecDeque::new();
    let mut size = 0;
    for l in lines.into_iter().rev() {
        size += l.len() + 1;
        if size > max {
            break;
        }
        out.push_front(l);
    }
    out.into()
}

impl Helpers for Spawner {
    fn start(&self, caller: &AgentId, a: &StartArgs) -> Result<u64, Error> {
        let folder_shown = a.folder.clone().unwrap_or_default();
        let mut req = match self.check(caller, a) {
            Ok(r) => r,
            Err(e) => return Err(self.refused(caller, &a.kind, &folder_shown, e)),
        };
        // A grant covers this start without asking: same asker, kind and folder, inside its own folder.
        if req.inside {
            if let Some(nested) = self
                .approvals
                .use_helper_grant(caller, &req.kind, &req.folder)
            {
                let outcome = self.launch(&req, nested);
                return Ok(Self::new_ticket(&mut self.st(), Some(outcome)));
            }
        }
        let caller_kind = self
            .sup()
            .list()
            .into_iter()
            .find(|v| v.id == *caller)
            .map(|v| v.kind)
            .unwrap_or_default();
        let detail = self.detail(&req, &caller_kind);
        let Some(p) = self
            .approvals
            .request_start(caller, &detail, crate::daemon::now_ms())
        else {
            return Err(Error::new(ErrorCode::Internal).with("incident_id", "approval-id"));
        };
        let mut st = self.st();
        req.ticket = Self::new_ticket(&mut st, None);
        let ticket = req.ticket;
        st.requests.insert(p.id.clone(), req);
        drop(st);
        if let Some(ev) = &self.events {
            ev.approval_requested(&p);
        }
        Ok(ticket)
    }

    fn started(&self, ticket: u64) -> Option<StartOutcome> {
        let mut st = self.st();
        match st.tickets.get(&ticket) {
            Some(Some(_)) => st.tickets.remove(&ticket).flatten(),
            Some(None) => None,
            // Withdrawn (its agent ended): nothing will come.
            None => Some(Err(
                Error::new(ErrorCode::MeshStartDenied).with("approval_id", "")
            )),
        }
    }

    fn abandon(&self, ticket: u64) {
        let gone: Option<String> = {
            let mut st = self.st();
            st.tickets.remove(&ticket);
            let id = st
                .requests
                .iter()
                .find(|(_, r)| r.ticket == ticket)
                .map(|(id, _)| id.clone());
            if let Some(id) = &id {
                st.requests.remove(id);
            }
            id
        };
        // Nobody will read the answer: the request leaves the queue, and a late decision on it is refused.
        if let Some(id) = gone {
            self.approvals.withdraw(&id);
            if let Some(ev) = &self.events {
                ev.approval_resolved(&id, false, "");
            }
        }
    }

    fn state(&self, caller: &AgentId, helper: &str) -> Result<String, Error> {
        let id = self.helper_of(caller, helper)?;
        Ok(self
            .sup()
            .list()
            .iter()
            .find(|v| v.id == id)
            .map_or("exited", state_word)
            .to_string())
    }

    fn output(&self, caller: &AgentId, helper: &str, lines: usize) -> Result<Value, Error> {
        let id = self.helper_of(caller, helper)?;
        let screen = {
            let sup = self.sup();
            sup.terminal(&id)
                .map(|t| t.inspect(|term| term.bottom_non_empty_lines(lines)))
        };
        let (source, text) = match screen {
            Some(text) => ("screen", text),
            None => (
                "transcript",
                self.chat
                    .snapshot(&id)
                    .into_iter()
                    .rev()
                    .take(lines)
                    .rev()
                    .flat_map(|e| e.text.lines().map(str::to_string).collect::<Vec<_>>())
                    .collect::<Vec<_>>(),
            ),
        };
        let start = text.len().saturating_sub(lines);
        let text = tail_within(text[start..].to_vec(), MAX_OUTPUT_BYTES);
        Ok(json!({"agent_id": helper, "source": source, "lines": text}))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn output_is_cut_from_the_top_at_a_line() {
        let lines: Vec<String> = (0..10).map(|i| format!("line {i}")).collect();
        let cut = tail_within(lines, 20);
        assert_eq!(cut, vec!["line 8", "line 9"]);
    }

    #[test]
    fn within_compares_whole_components() {
        assert!(within(Path::new("/a/b/c"), Path::new("/a/b")));
        assert!(within(Path::new("/a/b"), Path::new("/a/b")));
        assert!(!within(Path::new("/a/bc"), Path::new("/a/b")));
        assert!(!within(Path::new("/a"), Path::new("/a/b")));
    }
}
