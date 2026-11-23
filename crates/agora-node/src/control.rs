//! The node's local control socket: protocol commands over a local stream (docs/specs/protocol.md §2, §4).
//!
//! Framing is the protocol envelope (`agora_protocol::Frame`). Requests are `CommandRequest` payloads on stream 1
//! (`KIND_REQUEST`); responses are `CommandResponse` payloads on stream 1 (`KIND_RESPONSE`). A terminal stream is
//! opened by the client sending its `TerminalSubscribe` request on a dynamic stream id (3 or higher) of its
//! choosing; after the response, the node writes that terminal's frames on that stream (`KIND_TERM_*`), starting
//! with a snapshot. Input and resize are ordinary commands (`TerminalInput`, `TerminalResize`). The socket is owner-only,
//! so no further authentication is needed locally, except where the system lets a confined agent connect (Linux, ADR-0082): a node
//! that confines agents there requires the control token (`ControlAuth`, first on a connection) from every client; the hub path
//! adds device auth and approvals.
//!
//! Retried requests (same `idempotency_key`) get the stored response and do not repeat their effect.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use agora_core::AgentId;
use agora_platform::{local_connect, local_listen, LocalStream};
use agora_protocol::frame::{DEFAULT_MAX_PAYLOAD, STREAM_CLOSE, STREAM_COMMANDS};
use agora_protocol::v1::{self, command_request::Command as Cmd, command_response::Result as Res};
use agora_protocol::{Decoder, Error, ErrorCode, Frame};
use agora_vt::Size;
use prost::Message;

use crate::approvals::{SkipAsk, SkipKind, SkipRefusal};
use crate::daemon::EventSink;
use crate::git;
use crate::supervisor::{
    Prepared, StartRequest, Submit, Supervisor, SupervisorError, SupervisorEvent,
};
use crate::terminal::{Output, OutputSubscription, Snapshot};

pub const KIND_REQUEST: u8 = 0;
pub const KIND_RESPONSE: u8 = 1;
/// Terminal stream frames, node to client.
pub const KIND_TERM_DATA: u8 = 0;
pub const KIND_TERM_SNAPSHOT: u8 = 1;
/// Payload: cols u16 LE, rows u16 LE.
pub const KIND_TERM_RESIZE: u8 = 2;
/// Payload: exit code i32 LE (negative signal number when killed by a signal, `i32::MIN` if unknown).
pub const KIND_TERM_EXIT: u8 = 3;
/// ADR-0100: history the window asked for (`TerminalSubscribe.history_lines`), ANSI that puts the newest lines into its scrollback;
/// sent only right after the subscription's response, before its first snapshot, never with a resync.
pub const KIND_TERM_HISTORY: u8 = 4;
/// Chat transcript stream (ADR-0038). Payload: an encoded `ChatSnapshot`, sent first.
pub const KIND_CHAT_SNAPSHOT: u8 = 10;
/// Payload: an encoded `ChatEntry`, one for every line added after the snapshot.
pub const KIND_CHAT_ENTRY: u8 = 11;

const POLL: Duration = Duration::from_millis(100);
const IDEMPOTENCY_CACHE: usize = 256;

/// A project registered on this node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectEntry {
    pub id: String,
    pub name: String,
    pub root: PathBuf,
}

/// Everything a connection handler needs.
pub struct ControlContext {
    pub supervisor: Arc<Mutex<Supervisor>>,
    pub projects: Vec<ProjectEntry>,
    pub worktree_root: PathBuf,
    pub node_id: String,
    pub events: Option<Arc<EventSink>>,
    pub mesh: Option<Arc<crate::mesh_server::MeshServer>>,
    pub approvals: Option<Arc<crate::approvals::NodeApprovals>>,
    /// Transcripts of agents without a terminal (ADR-0038).
    pub chat: Arc<crate::chat::ChatBoard>,
    directories: Mutex<crate::directory::Bindings>,
    /// Where the agent manifests live; read again before agents are listed or started.
    manifests_dir: Option<PathBuf>,
    /// ADR-0057: agents that start agents; decisions on their requests come through `ApprovalResolve`.
    spawn: Option<Arc<crate::spawn::Spawner>>,
    /// Set after an agent starts: the node's Git watcher then checks its folder at once instead of on its next round.
    git_wake: Option<Arc<AtomicBool>>,
    /// ADR-0060: the session ledger, which holds the agents `agent.resume` can bring back. `None`: nothing can be resumed.
    ledger: Option<Arc<crate::ledger::Ledger>>,
    /// ADR-0080: the owner's policy `files.show_secret`; off unless the policy file says `true`.
    show_secret: bool,
    /// ADR-0082: when set, a connection must present it (`ControlAuth`) before any other command is answered.
    control_token: Option<(crate::control_token::ControlToken, PathBuf)>,
    names: AtomicU64,
    seen: Mutex<IdempotencyCache>,
    /// Signalled when an attempt claimed in `seen.running` ends (F1360).
    settled: Condvar,
}

#[derive(Default)]
struct IdempotencyCache {
    order: VecDeque<String>,
    responses: HashMap<String, v1::CommandResponse>,
    /// Keys whose first attempt is still running (F1360): a retry on another connection waits for its answer instead of repeating it.
    running: HashSet<String>,
}

/// The right to run the attempt for one idempotency key ([`ControlContext::claim`]); let go when dropped, answered or not.
struct Claim<'a> {
    ctx: &'a ControlContext,
    key: &'a str,
}

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        let mut c = self.ctx.seen.lock().unwrap_or_else(|e| e.into_inner());
        c.running.remove(self.key);
        drop(c);
        self.ctx.settled.notify_all();
    }
}

impl ControlContext {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        supervisor: Arc<Mutex<Supervisor>>,
        projects: Vec<ProjectEntry>,
        worktree_root: PathBuf,
        node_id: String,
        events: Option<Arc<EventSink>>,
        mesh: Option<Arc<crate::mesh_server::MeshServer>>,
        approvals: Option<Arc<crate::approvals::NodeApprovals>>,
        chat: Arc<crate::chat::ChatBoard>,
    ) -> Self {
        Self {
            supervisor,
            projects,
            directories: Mutex::new(crate::directory::Bindings::load(
                worktree_root.join(crate::directory::BINDINGS_FILE),
            )),
            worktree_root,
            node_id,
            events,
            mesh,
            approvals,
            chat,
            names: AtomicU64::new(0),
            manifests_dir: None,
            spawn: None,
            git_wake: None,
            ledger: None,
            show_secret: false,
            control_token: None,
            seen: Mutex::new(IdempotencyCache::default()),
            settled: Condvar::new(),
        }
    }

    /// ADR-0082: requires this token from every connection before any command but `ControlAuth` is answered. [`ControlServer::start`]
    /// writes it to `file` once the socket is bound, and removes the file when the server stops.
    #[must_use]
    pub fn with_control_token(
        mut self,
        token: crate::control_token::ControlToken,
        file: PathBuf,
    ) -> Self {
        self.control_token = Some((token, file));
        self
    }

    /// Wakes the node's Git watcher (`daemon::watch_git`) after an agent starts, so the new folder's changes are counted at once and
    /// not up to a poll later.
    #[must_use]
    pub fn with_git_wake(mut self, wake: Arc<AtomicBool>) -> Self {
        self.git_wake = Some(wake);
        self
    }

    /// ADR-0057: who handles decisions on `agent.start` requests and stopping an agent's helpers.
    #[must_use]
    pub fn with_spawner(mut self, spawn: Arc<crate::spawn::Spawner>) -> Self {
        self.spawn = Some(spawn);
        self
    }

    /// ADR-0060: the session ledger `agent.resume` reads.
    #[must_use]
    pub fn with_ledger(mut self, ledger: Arc<crate::ledger::Ledger>) -> Self {
        self.ledger = Some(ledger);
        self
    }

    /// ADR-0080: secret files are shown (the owner's policy `files.show_secret = true`). Set at start from the policy file only.
    #[must_use]
    pub fn with_show_secret(mut self, shown: bool) -> Self {
        self.show_secret = shown;
        self
    }

    fn wake_git(&self) {
        if let Some(wake) = &self.git_wake {
            wake.store(true, Ordering::Release);
        }
    }

    /// Makes the node read its manifest folder again before it lists or starts agents.
    #[must_use]
    pub fn with_manifests_dir(mut self, dir: PathBuf) -> Self {
        self.manifests_dir = Some(dir);
        self
    }

    /// Registers manifests added or changed since the node started (a kind whose file was deleted stays until restart). A file
    /// that does not parse is skipped; `agora doctor` names it.
    fn reload_manifests(&self) {
        let Some(dir) = &self.manifests_dir else {
            return;
        };
        let (found, _) = crate::daemon::load_manifests(dir);
        let mut sup = self.sup();
        for l in found {
            sup.register_loaded(l);
        }
    }

    fn sup(&self) -> MutexGuard<'_, Supervisor> {
        self.supervisor.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The stored answer for `key`, or the claim to run its attempt. An attempt with the same key that is still running (a retry that
    /// came on another connection before the first answer) is waited for, so its effect is not repeated (F1360).
    fn claim<'a>(&'a self, key: &'a str) -> Result<Claim<'a>, Box<v1::CommandResponse>> {
        let mut c = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(prev) = c.responses.get(key) {
                return Err(Box::new(prev.clone()));
            }
            if c.running.insert(key.to_string()) {
                return Ok(Claim { ctx: self, key });
            }
            c = self.settled.wait(c).unwrap_or_else(|e| e.into_inner());
        }
    }

    fn remember(&self, key: &str, resp: &v1::CommandResponse) {
        let mut c = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        if c.responses.insert(key.to_string(), resp.clone()).is_none() {
            c.order.push_back(key.to_string());
            while c.order.len() > IDEMPOTENCY_CACHE {
                if let Some(old) = c.order.pop_front() {
                    c.responses.remove(&old);
                }
            }
        }
    }
}

pub fn terminal_id_of(agent: &AgentId) -> String {
    format!("t-{}", agent.as_str())
}

pub(crate) fn agent_of_terminal(terminal_id: &str) -> Option<AgentId> {
    terminal_id.strip_prefix("t-").map(AgentId::new)
}

/// ADR-0090: the answer to `AgentPrompt` for an adapter agent: whether the prompt was sent or queued, and how many wait.
fn prompt_result(queued: bool, queue_len: usize) -> v1::CommandOk {
    v1::CommandOk {
        result: Some(v1::command_ok::Result::AgentPrompt(v1::AgentPromptResult {
            queued,
            queue_len: u32::try_from(queue_len).unwrap_or(u32::MAX),
        })),
    }
}

/// ADR-0090: a prompt for an adapter agent, through its adapter: sent now, or queued behind the turn that runs (at most 8, shown as
/// queued until it is sent). Takes the supervisor's lock and lets it go before the adapter's answer is waited for. `(queued,
/// queue_len)`; `queue_len` is 0 when it was sent. `on_queued` gets the queued prompt's tag while the lock is held, so nothing can
/// take the prompt out of the queue before the caller knows its tag.
fn submit_to_adapter(
    ctx: &ControlContext,
    mut sup: MutexGuard<'_, Supervisor>,
    id: &AgentId,
    text: &str,
    on_queued: impl FnOnce(u64),
) -> Result<(bool, usize), Error> {
    let now = crate::daemon::now_ms();
    let submitted = sup
        .adapter_submit(id, text, |_| {
            let tag = ctx.chat.push_queued(id, text.to_string(), now);
            on_queued(tag);
            tag
        })
        .map_err(sup_err);
    drop(sup);
    match submitted? {
        Submit::Queued { queue_len } => Ok((true, queue_len)),
        Submit::Send(call) => {
            // The user's entry is in the conversation BEFORE the agent can answer, so a fast agent's first chunk never comes ahead
            // of the prompt it answers. A prompt that did not go out is taken back.
            let generation = call.generation();
            let entry = ctx.chat.push_user(id, text.to_string(), now);
            match call.run() {
                Ok(()) => Ok((false, 0)),
                Err(e) => {
                    ctx.chat.retract(id, entry);
                    ctx.sup().adapter_send_failed(id, generation);
                    Err(sup_err(e))
                }
            }
        }
    }
}

/// ADR-0095 decisions 3 to 5: give open task `d.task_id` of `d.project_id` to agent `d.agent_id` of this node. The task is claimed
/// for the agent where the project's board is ([`MeshServer::dispatch_claim`]); the prompt built from it is not typed over a running
/// turn (a structured agent queues it; a terminal agent in a turn gets it held, one at most, until it is between turns, and every
/// terminal agent gets it as a bracketed paste); a prompt that does not reach the agent gives a task on the node's own broker back;
/// and `TaskDispatched` records it. Not an approval: nothing about the agent's permissions changes.
///
/// [`MeshServer::dispatch_claim`]: crate::mesh_server::MeshServer::dispatch_claim
fn task_dispatch(ctx: &ControlContext, d: &v1::TaskDispatch) -> Result<v1::CommandOk, Error> {
    use crate::task_dispatch::{Given, Held, Slot};
    let unknown = || Error::new(ErrorCode::MeshRecipientUnknown).with("to", d.agent_id.as_str());
    let gone = || Error::new(ErrorCode::AgentNotFound).with("agent_id", d.agent_id.as_str());
    let Some(mesh) = &ctx.mesh else {
        return Err(unknown());
    };
    let id = AgentId::new(d.agent_id.clone());
    let (terminal, skips, paste) = {
        let sup = ctx.sup();
        let view = sup
            .list()
            .into_iter()
            .find(|v| v.id == id && v.running && v.project == d.project_id)
            .ok_or_else(unknown)?;
        let paste = sup
            .terminal(&id)
            .map(crate::terminal::TerminalHandle::bracketed_paste);
        (view.terminal, sup.skips_permissions_now(&id), paste)
    };
    let dispatches = mesh.dispatches();
    // A terminal agent: its program must take a paste (its lines then arrive together, never as several submits), and its one slot is
    // reserved before anything is claimed.
    let token = if terminal {
        match paste {
            None => return Err(gone()),
            Some(false) => return Err(capability_missing("terminal.bracketed_paste")),
            Some(true) => Some(dispatches.reserve(&id, Instant::now())?),
        }
    } else {
        None
    };
    let claim = match mesh.dispatch_claim(&d.project_id, &id, &d.task_id) {
        Ok(c) => c,
        Err(e) => {
            if let Some(t) = token {
                dispatches.cancel(&id, t);
            }
            return Err(e);
        }
    };
    let given = Given {
        agent: id.clone(),
        task_id: d.task_id.clone(),
        shared: claim.shared,
    };
    let sent = if let Some(token) = token {
        // Decided now, after the claim (which may have waited for the hub), from its live screen read now (never the paced
        // detection's last result): a turn that began meanwhile, or a screen no rule names, gets the prompt held, not pasted over it.
        let fresh = ctx.sup().terminal_readiness(&id);
        let slot = match fresh {
            Some((state, detects)) => dispatches.decide(&id, token, state, detects, Instant::now()),
            None => {
                dispatches.cancel(&id, token);
                None
            }
        };
        match slot {
            // The reservation went with the agent (it stopped meanwhile).
            None => Err(gone()),
            Some(Slot::PasteNow) => {
                // The program may have turned bracketed paste off since it was checked: then it is refused as before the claim.
                let control = ctx
                    .sup()
                    .terminal(&id)
                    .map(|h| (h.bracketed_paste(), h.input_handle().ok()));
                match control {
                    Some((true, Some(c))) => crate::task_dispatch::paste_prompt(&c, &claim.prompt)
                        .map(|()| (false, 0))
                        .map_err(|_| gone()),
                    Some((false, _)) => Err(capability_missing("terminal.bracketed_paste")),
                    _ => Err(gone()),
                }
            }
            Some(Slot::Hold) => {
                let held = Held {
                    prompt: claim.prompt.clone(),
                    given: given.clone(),
                };
                if dispatches.fill(token, held) {
                    Ok((true, crate::task_dispatch::MAX_HELD))
                } else {
                    Err(gone())
                }
            }
        }
    } else {
        submit_to_adapter(ctx, ctx.sup(), &id, &claim.prompt, |tag| {
            dispatches.queued(tag, given.clone());
        })
    };
    let (queued, queue_len) = match sent {
        Ok(v) => v,
        Err(e) => {
            mesh.give_back(&given);
            return Err(e);
        }
    };
    let prompt_sha256 = crate::task_dispatch::prompt_sha256(&claim.prompt);
    if let Some(ev) = &ctx.events {
        ev.record_bodies(vec![v1::event::Body::TaskDispatched(v1::TaskDispatched {
            project_id: d.project_id.clone(),
            task_id: d.task_id.clone(),
            agent_id: d.agent_id.clone(),
            by_user: d.by_user.clone(),
            prompt_sha256: prompt_sha256.clone(),
            queued,
            agent_skips_permissions: skips,
        })]);
    }
    Ok(v1::CommandOk {
        result: Some(v1::command_ok::Result::TaskDispatch(
            v1::TaskDispatchResult {
                queued,
                queue_len: u32::try_from(queue_len).unwrap_or(u32::MAX),
                prompt_sha256,
            },
        )),
    })
}

fn ok_empty() -> v1::CommandOk {
    v1::CommandOk {
        result: Some(v1::command_ok::Result::Empty(v1::Empty {})),
    }
}

/// ADR-0090 Amendment A: what a switch of a running agent's mode or config option came to. A call is made (outside the supervisor's lock)
/// and settles the option: a card still waiting for another value of it is withdrawn. `settles` names the option and its twin (F1322: a
/// mode and the config option of category `mode` an agent gives as its mode are one setting). A choice that skips permissions under a
/// policy that allows it is not applied and not refused: the node raises the owner's approval and answers `agent.approval_needed`.
fn switch(
    ctx: &ControlContext,
    agent: &AgentId,
    settles: &[(SkipKind, String)],
    prepared: Result<Prepared, SupervisorError>,
) -> Result<v1::CommandOk, Error> {
    match prepared.map_err(sup_err)? {
        Prepared::Call(call) => {
            call.run().map_err(sup_err)?;
            if let Some(approvals) = &ctx.approvals {
                for (kind, option) in settles {
                    for id in approvals.withdraw_skips(agent, Some((*kind, option))) {
                        if let Some(ev) = &ctx.events {
                            ev.approval_resolved(&id, false, "");
                        }
                    }
                }
            }
            Ok(ok_empty())
        }
        Prepared::Ask(ask) => Err(raise_skip(ctx, ask)),
    }
}

/// Raises the approval for `ask` (or finds the one already waiting) and returns the error that tells the window to show it.
fn raise_skip(ctx: &ControlContext, ask: SkipAsk) -> Error {
    let Some(approvals) = &ctx.approvals else {
        // A node without an approvals queue cannot ask anyone: the switch stays refused, as it was before the approval existed.
        let option = match ask.kind {
            SkipKind::Mode => ask.value.clone(),
            SkipKind::Config => ask.option_id.clone(),
        };
        return SupervisorError::OptionUnconfirmed(ask.agent_kind.clone(), option).to_protocol();
    };
    let unsignable_id = match ask.kind {
        SkipKind::Mode => "mode".to_string(),
        SkipKind::Config => crate::approvals::card_text(&ask.option_id, 64),
    };
    let per_agent = |what: usize| {
        Error::new(ErrorCode::NodeResourceLimit)
            .with("resource", "skip_approvals_agent")
            .with("limit", i64::try_from(what).unwrap_or(i64::MAX))
    };
    let limit = |what: usize| {
        Error::new(ErrorCode::NodeResourceLimit)
            .with("resource", "skip_approvals")
            .with("limit", i64::try_from(what).unwrap_or(i64::MAX))
    };
    match approvals.request_skip(&ctx.node_id, ask, crate::daemon::now_ms()) {
        Ok(raised) => {
            if let Some(ev) = &ctx.events {
                for id in &raised.superseded {
                    ev.approval_resolved(id, false, "");
                }
                if raised.fresh {
                    ev.approval_requested(&raised.pending);
                }
            }
            Error::new(ErrorCode::AgentApprovalNeeded)
                .with("approval_id", raised.pending.id.as_str())
        }
        Err(SkipRefusal::TooManyWaiting) => limit(crate::approvals::MAX_SKIP_PENDING),
        Err(SkipRefusal::TooOften) => limit(crate::approvals::MAX_SKIP_PER_WINDOW),
        Err(SkipRefusal::AgentTooMany) => per_agent(crate::approvals::MAX_SKIP_PER_AGENT),
        Err(SkipRefusal::AgentTooOften) => per_agent(crate::approvals::MAX_SKIP_PER_AGENT_WINDOW),
        // An id or a value the card could not show as it is signed: nothing is raised, and the agent is told it has no such setting.
        Err(SkipRefusal::Unsignable) => {
            Error::new(ErrorCode::AgentOptionUnknown).with("id", unsignable_id.as_str())
        }
        Err(SkipRefusal::NoId) => {
            Error::new(ErrorCode::Internal).with("incident_id", "approval-id")
        }
    }
}

/// The verified decision on a skip request: only `approved` applies anything, and only what the node rendered (`resolution.skip`).
fn decide_skip(
    ctx: &ControlContext,
    r: &v1::ApprovalResolve,
    resolution: &crate::approvals::Resolution,
) -> Result<v1::CommandOk, Error> {
    let approved = resolution.verdict == crate::approvals::Verdict::Approved;
    let announce = |approved: bool, signer: &str| {
        if let Some(ev) = &ctx.events {
            ev.approval_resolved(&r.approval_id, approved, signer);
        }
    };
    let Some(ask) = &resolution.skip else {
        announce(false, "");
        return Err(invalid("approval_id"));
    };
    if approved {
        let call = ctx.sup().prepare_skip_apply(ask);
        if let Err(e) = call.and_then(crate::supervisor::AdapterCall::run) {
            // The decision was verified but the agent is no longer what the card named: nothing was applied, and the card is over.
            announce(false, "");
            return Err(sup_err(e));
        }
    }
    announce(approved, &r.signer_device_id);
    Ok(ok_empty())
}

fn capability_missing(what: &str) -> Error {
    Error::new(ErrorCode::ProtocolCapabilityMissing).with("capability", what)
}

fn invalid(field: &str) -> Error {
    Error::new(ErrorCode::ProtocolInvalidMessage).with("field", field)
}

fn resolve_project<'a>(ctx: &'a ControlContext, wanted: &str) -> Result<&'a ProjectEntry, Error> {
    resolve_in(&ctx.projects, wanted)
}

fn resolve_in<'a>(projects: &'a [ProjectEntry], wanted: &str) -> Result<&'a ProjectEntry, Error> {
    if wanted.is_empty() {
        return match projects {
            [only] => Ok(only),
            _ => Err(invalid("project_id")),
        };
    }
    // An id always wins over a name, and a name that two projects share is refused instead of
    // guessed: the hub checks a member's grants against this exact string.
    if let Some(p) = projects.iter().find(|p| p.id == wanted) {
        return Ok(p);
    }
    let mut named = projects.iter().filter(|p| p.name == wanted);
    match (named.next(), named.next()) {
        (Some(p), None) => Ok(p),
        (Some(_), Some(_)) => Err(invalid("project_id")),
        _ => Err(Error::new(ErrorCode::ProjectNotFound).with("project_id", wanted)),
    }
}

/// ADR-0059: the launch options an agent was started with, as its events carry them.
fn launch_of(v: &crate::supervisor::AgentView) -> crate::launch_options::LaunchChoice {
    crate::launch_options::LaunchChoice {
        ids: v.launch_options.clone(),
        args: Vec::new(),
        skips_permissions: v.skips_permissions,
    }
}

fn sup_err(e: SupervisorError) -> Error {
    e.to_protocol()
}

/// ADR-0101 (Codex review of PR #1007): records an always answer the adapter delivered to the run `run` of `agent`'s process. The
/// caller holds the supervisor's lock (`sup`) for the check and the record together, so a replacement of the process is ordered
/// before the check (which then fails) or after the record. The audit record (`ApprovalResolved`, ALWAYS) is always written; the
/// agent's `AgentAlwaysAllowed` only while that run is still the agent's process. Returns whether it was.
pub(crate) fn record_always_answer(
    sup: &Supervisor,
    events: &crate::daemon::EventSink,
    agent: &AgentId,
    run: Option<u64>,
    approval_id: &str,
    action: &str,
    signer_device_id: &str,
) -> bool {
    let held = run.is_some_and(|run| sup.runs(agent, run));
    events.approval_resolved_always(held.then_some(agent), approval_id, action, signer_device_id);
    held
}

/// Run one command. Terminal subscription is handled by the connection (it needs the stream).
fn execute(ctx: &ControlContext, cmd: &Cmd) -> Result<v1::CommandOk, Error> {
    use v1::command_ok::Result as Ok_;
    if matches!(
        cmd,
        Cmd::AgentKinds(_) | Cmd::AgentStart(_) | Cmd::AgentLaunch(_) | Cmd::AgentResume(_)
    ) {
        ctx.reload_manifests();
    }
    match cmd {
        Cmd::ProjectList(_) => Ok(v1::CommandOk {
            result: Some(Ok_::ProjectList(v1::ProjectListResult {
                projects: ctx
                    .projects
                    .iter()
                    .map(|p| v1::ProjectSummary {
                        project_id: p.id.clone(),
                        name: p.name.clone(),
                    })
                    .collect(),
            })),
        }),
        Cmd::TaskDispatch(d) => task_dispatch(ctx, d),
        // A mesh message another node's agent sent, routed here by the hub with `origin` set by the hub (ADR-0031).
        Cmd::MeshPost(p) => {
            if p.origin.is_empty() {
                return Err(invalid("origin"));
            }
            let Some(mesh) = &ctx.mesh else {
                return Err(Error::new(ErrorCode::MeshRecipientUnknown).with("to", p.to.as_str()));
            };
            mesh.deliver_remote(
                &p.origin,
                &p.from_agent,
                &AgentId::new(&p.to),
                &p.body,
                &p.refs,
            )?;
            Ok(v1::CommandOk {
                result: Some(Ok_::Empty(v1::Empty {})),
            })
        }
        // ADR-0039 amendment B: only the hub asks for this, and the uplink answers it where the hub's commands enter; from the
        // local socket it would hand this node's board work to nobody.
        Cmd::BoardJoin(_) => Err(capability_missing("board.join.local")),
        // Grants are made and judged where the hub's commands enter the node; the local socket has no use for them
        // (the person at this machine is not routed through the hub, ADR-0041).
        Cmd::GrantCreate(_) | Cmd::GrantList(_) | Cmd::GrantRevoke(_) => {
            Err(Error::new(ErrorCode::ProtocolCapabilityMissing).with("capability", "grants.local"))
        }
        Cmd::AgentKinds(_) => {
            let kinds = ctx
                .sup()
                .kinds()
                .into_iter()
                .map(|k| v1::AgentKind {
                    kind: k.kind,
                    display_name: k.display_name,
                    level: match k.level {
                        agora_core::IntegrationLevel::L0Generic => v1::IntegrationLevel::L0Generic,
                        agora_core::IntegrationLevel::L1Manifest => {
                            v1::IntegrationLevel::L1Manifest
                        }
                        agora_core::IntegrationLevel::L2Acp => v1::IntegrationLevel::L2Acp,
                        agora_core::IntegrationLevel::L3Adapter => v1::IntegrationLevel::L3Adapter,
                    } as i32,
                    installed: k.installed,
                    // ADR-0069: a bare file name, never a path (a manifest may name its program by path).
                    program: std::path::Path::new(&k.program)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    settings: k
                        .settings
                        .iter()
                        .map(|st| v1::LaunchSettingSpec {
                            id: st.id.clone(),
                            label: st.label.clone(),
                            values: st.values.clone(),
                            default_value: st.default.clone().unwrap_or_default(),
                        })
                        .collect(),
                    launch_options: k
                        .launch_options
                        .iter()
                        .map(|o| v1::LaunchOption {
                            id: o.id.clone(),
                            label: o.label.clone(),
                            skips_permissions: o.skips_permissions,
                        })
                        .collect(),
                })
                .collect();
            Ok(v1::CommandOk {
                result: Some(Ok_::AgentKinds(v1::AgentKindsResult { kinds })),
            })
        }
        Cmd::AgentList(l) => {
            // Only this node's agents: a node answers for itself (the hub merges across nodes).
            if !l.node_id.is_empty() && l.node_id != ctx.node_id {
                return Err(Error::new(ErrorCode::NodeOffline).with("node_id", l.node_id.as_str()));
            }
            // #1120: the agents an earlier process of this node ran that ended and are still kept (ADR-0060), read before the
            // supervisor's lock is taken (never both at once). Listed as that process listed them: stopped, in their folder.
            let kept = ctx
                .ledger
                .as_ref()
                .map(|l| l.ended(crate::daemon::now_ms()))
                .unwrap_or_default();
            // One lock for the whole answer (the mutex is not reentrant).
            let sup = ctx.sup();
            let views = sup.list();
            let earlier: Vec<v1::AgentSummary> = kept
                .into_iter()
                .filter(|e| !views.iter().any(|v| v.id.as_str() == e.agent_id))
                .map(|e| v1::AgentSummary {
                    state: crate::ledger::proto_state(&e.last_state) as i32,
                    agent_id: e.agent_id,
                    kind: e.kind,
                    worktree_id: e.folder,
                    terminal_id: String::new(),
                    running: false,
                    restarts: 0,
                })
                .collect();
            let mut agents: Vec<v1::AgentSummary> = earlier
                .into_iter()
                .chain(views.into_iter().map(|v| {
                    let has_terminal = sup.terminal(&v.id).is_some();
                    v1::AgentSummary {
                        agent_id: v.id.as_str().to_string(),
                        kind: v.kind,
                        state: match v.state {
                            agora_core::AgentState::Working => v1::AgentState::Working,
                            agora_core::AgentState::Blocked => v1::AgentState::Blocked,
                            agora_core::AgentState::Idle => v1::AgentState::Idle,
                            agora_core::AgentState::Done => v1::AgentState::Done,
                            agora_core::AgentState::Unknown => v1::AgentState::Unspecified,
                        } as i32,
                        worktree_id: v.worktree.to_string_lossy().into_owned(),
                        terminal_id: if has_terminal {
                            terminal_id_of(&v.id)
                        } else {
                            String::new()
                        },
                        running: v.running,
                        restarts: v.restarts,
                    }
                }))
                .collect();
            // One list in id order (`agent-2` before `agent-10`), whether a row comes from the ledger or the supervisor (#1122 review).
            agents.sort_by_key(|a| {
                (
                    crate::ledger::id_number(&a.agent_id).unwrap_or(u64::MAX),
                    a.agent_id.clone(),
                )
            });
            Ok(v1::CommandOk {
                result: Some(Ok_::AgentList(v1::AgentListResult { agents })),
            })
        }
        Cmd::WorkspaceQuery(q) => {
            let path = q
                .workspace_id
                .strip_prefix("directory:")
                .ok_or_else(|| invalid("workspace_id"))?
                .to_string();
            let cmd = match q.operation.as_str() {
                "list" => Cmd::FilesList(v1::FilesList {
                    worktree_id: path,
                    path: q.path.clone(),
                }),
                "read" => Cmd::FilesRead(v1::FilesRead {
                    worktree_id: path,
                    path: q.path.clone(),
                }),
                "diff" => Cmd::GitDiff(v1::GitDiff {
                    worktree_id: path,
                    path: if q.path.is_empty() {
                        ".".into()
                    } else {
                        q.path.clone()
                    },
                }),
                "status" => Cmd::GitStatus(v1::GitStatus { worktree_id: path }),
                "history" => Cmd::GitLog(v1::GitLog {
                    worktree_id: path,
                    limit: q.limit,
                }),
                _ => return Err(invalid("workspace.operation")),
            };
            execute(ctx, &cmd)
        }
        Cmd::DirectoryInspect(d) => {
            let registered = ctx
                .projects
                .iter()
                .any(|p| p.id == d.path || p.name == d.path);
            let path =
                if registered || (!Path::new(&d.path).is_dir() && !d.path.contains(['/', '\\'])) {
                    resolve_project(ctx, &d.path)?
                        .root
                        .to_string_lossy()
                        .into_owned()
                } else {
                    d.path.clone()
                };
            let (mut info, _) =
                crate::directory::inspect(&path, &ctx.node_id, d.create_project_id)?;
            info.previous_project_id = ctx
                .directories
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .previous(&info);
            Ok(v1::CommandOk {
                result: Some(Ok_::DirectoryInfo(info)),
            })
        }
        Cmd::DirectoryList(d) => {
            // The same folders a launch refuses stay out of the listing (ADR-0053); `node_home` is the folder the worktrees are in.
            // The scope is compared with the listed folder by prefix in `std::fs::canonicalize`'s spelling: when it cannot be
            // canonicalized the listing is refused rather than compared with a raw path that would never match (fail closed).
            let canon = |p: &Path| std::fs::canonicalize(p).map_err(|_| invalid("directory.path"));
            let worktrees = canon(&ctx.worktree_root)?;
            let node_home = worktrees
                .parent()
                .map_or_else(|| worktrees.clone(), Path::to_path_buf);
            let home = match crate::usage_files::home_dir() {
                Some(h) => Some(canon(&h)?),
                None => None,
            };
            let scope = crate::directory::ListScope {
                home: home.as_deref(),
                node_home: &node_home,
                worktree_root: &worktrees,
                budget: LIST_BUDGET,
            };
            Ok(v1::CommandOk {
                result: Some(Ok_::DirectoryEntries(crate::directory::list(
                    &d.path, &scope,
                )?)),
            })
        }
        Cmd::AgentLaunch(a) => {
            // F1012: the folder is held from before it is checked, the start goes into the held folder, and a path that leads to
            // another folder after the check (the folder, or one above it, moved or swapped for a link) is refused.
            let held = agora_platform::HeldFolder::open(Path::new(&a.path));
            let (info, project) = crate::directory::inspect(&a.path, &ctx.node_id, false)?;
            if info.path != a.path || info.project_id != a.project_id {
                return Err(invalid("directory.project_id"));
            }
            let folder = held
                .ok()
                .filter(agora_platform::HeldFolder::in_place)
                .ok_or_else(|| invalid("directory.path"))?;
            if a.separate_worktree && info.git_root.is_empty() {
                return Err(invalid("directory.git_root"));
            }
            let mut bindings = ctx.directories.lock().unwrap_or_else(|e| e.into_inner());
            if !bindings.previous(&info).is_empty() && !a.accept_identity_change {
                return Err(invalid("directory.identity_changed"));
            }
            let n = ctx.names.fetch_add(1, Ordering::SeqCst);
            let stamp = crate::grants::now_ms();
            let req = StartRequest {
                kind: a.kind.clone(),
                checkout: PathBuf::from(&info.path),
                worktree_root: ctx.worktree_root.clone(),
                name: format!("{}-{stamp:x}{n}", a.kind),
                base: None,
            };
            // Record only an explicitly confirmed identity. A failed process never owns or removes the existing directory.
            bindings.confirm(&info)?;
            drop(bindings);
            let mut sup = ctx.sup();
            let id = sup
                .start_selected_with_options(
                    &req,
                    project,
                    Path::new(&info.git_root),
                    a.separate_worktree,
                    &crate::launch_options::Requested {
                        ids: a.launch_options.clone(),
                        confirmed: a.confirmed_launch_options.clone(),
                        // ADR-0066: a launch through the hub with settings was refused where the hub's commands enter (its
                        // signature does not cover them yet); here they come from this machine's own socket.
                        settings: a
                            .settings
                            .iter()
                            .map(|st| (st.id.clone(), st.value.clone()))
                            .collect(),
                    },
                    Instant::now(),
                    Some(folder),
                )
                .map_err(sup_err)?;
            let view = sup
                .list()
                .into_iter()
                .find(|v| v.id == id)
                .ok_or_else(|| invalid("agent.id"))?;
            let has_terminal = sup.terminal(&id).is_some();
            let session = sup.session_of(&id);
            let launch = launch_of(&view);
            let path = view.worktree.to_string_lossy().into_owned();
            let workspace_id = format!("directory:{path}");
            drop(sup);
            if let Some(ev) = &ctx.events {
                let git_root = if a.separate_worktree {
                    let root = req.worktree_root.join(&req.name);
                    agora_platform::canonicalize_plain(&root)
                        .unwrap_or(root)
                        .to_string_lossy()
                        .into_owned()
                } else {
                    info.git_root.clone()
                };
                ev.directory_agent_launched_with(
                    &id,
                    &v1::WorkspaceRegistered {
                        workspace_id: workspace_id.clone(),
                        node_id: ctx.node_id.clone(),
                        project_id: info.project_id.clone(),
                        path: path.clone(),
                        git_root,
                        branch: if a.separate_worktree {
                            format!("agent/{}", req.name)
                        } else {
                            info.branch.clone()
                        },
                        managed_worktree: a.separate_worktree,
                        // Counted by the Git watcher, woken below, which sends it as this workspace's status right after; a
                        // `status` here (three git processes, seconds on Windows) only made the start answer later.
                        files_changed: 0,
                    },
                    &a.kind,
                    has_terminal,
                    &launch,
                );
                // ADR-0060: a session id the node chose at launch, recorded right after the start.
                if let Some(session) = &session {
                    ev.session_reported(&id, session);
                }
            }
            ctx.wake_git();
            Ok(v1::CommandOk {
                result: Some(Ok_::AgentLaunch(v1::AgentLaunchResult {
                    agent_id: id.as_str().into(),
                    terminal_id: if has_terminal {
                        terminal_id_of(&id)
                    } else {
                        String::new()
                    },
                    workspace_id,
                    path,
                })),
            })
        }
        Cmd::AgentResume(a) => {
            // A person pressed Resume: they are there and can say what they want, so no "carry on" message (ADR-0060 Amendment A).
            resume_agent(ctx, &a.agent_id, a.new_session, false).map(|r| v1::CommandOk {
                result: Some(Ok_::AgentLaunch(r)),
            })
        }
        Cmd::AgentStart(a) => {
            if !a.worktree_id.is_empty() {
                return Err(capability_missing("agent.start.existing_worktree"));
            }
            let project = resolve_project(ctx, &a.project_id)?;
            let n = ctx.names.fetch_add(1, Ordering::SeqCst);
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let req = StartRequest {
                kind: a.kind.clone(),
                checkout: project.root.clone(),
                worktree_root: ctx.worktree_root.clone(),
                name: format!("{}-{stamp:x}{n}", a.kind),
                base: None,
            };
            let mut sup = ctx.sup();
            let id = sup
                .start_with_options(
                    &req,
                    &crate::launch_options::Requested {
                        ids: a.launch_options.clone(),
                        confirmed: a.confirmed_launch_options.clone(),
                        // ADR-0066: from this machine's own socket only (the hub's path refuses a start with settings).
                        settings: a
                            .settings
                            .iter()
                            .map(|st| (st.id.clone(), st.value.clone()))
                            .collect(),
                    },
                    Instant::now(),
                )
                .map_err(sup_err)?;
            let view = sup.list().into_iter().find(|v| v.id == id);
            let launch = view.as_ref().map(launch_of).unwrap_or_default();
            let worktree = view
                .as_ref()
                .map(|v| v.worktree.to_string_lossy().into_owned())
                .unwrap_or_default();
            let branch = view.as_ref().map(|v| v.branch.clone()).unwrap_or_default();
            if !a.prompt.is_empty() {
                if let Some(t) = sup.terminal(&id) {
                    let _ = t.send_input(format!("{}\r", a.prompt).into_bytes());
                }
            }
            let has_terminal = sup.terminal(&id).is_some();
            let session = sup.session_of(&id);
            drop(sup);
            if let Some(ev) = &ctx.events {
                ev.agent_launched_with(
                    &id,
                    &project.id,
                    &ctx.node_id,
                    &project.root,
                    std::path::Path::new(&worktree),
                    &branch,
                    &a.kind,
                    has_terminal,
                    &launch,
                );
                if let Some(session) = &session {
                    ev.session_reported(&id, session);
                }
            }
            ctx.wake_git();
            Ok(v1::CommandOk {
                result: Some(Ok_::AgentStart(v1::AgentStartResult {
                    terminal_id: if has_terminal {
                        terminal_id_of(&id)
                    } else {
                        String::new()
                    },
                    agent_id: id.as_str().to_string(),
                    worktree_id: worktree,
                })),
            })
        }
        Cmd::AgentStop(a) => {
            let id = AgentId::new(a.agent_id.clone());
            // ADR-0057: its helpers (and theirs) first, when asked; found before the parent stops.
            let helpers = match (&ctx.spawn, a.stop_helpers) {
                (Some(s), true) => s.running_helpers(&id),
                _ => Vec::new(),
            };
            let stopped_now = ctx.sup().stop(&id);
            // ADR-0060: an interrupted agent whose process outlived the node is stopped by ending exactly that process.
            if let (Err(SupervisorError::NotFound(_)), Some(ledger)) = (&stopped_now, &ctx.ledger) {
                // Also when its old process has ended since it was announced still running: the stop then signals nothing and
                // only says it is interrupted without a process, so Resume and Start again are offered again.
                if ledger.get(&a.agent_id).is_some_and(|e| e.interrupted()) {
                    stop_survivor(ctx, ledger, &a.agent_id)?;
                    // Its helpers that outlived the node too, and theirs, when asked (ADR-0057).
                    if a.stop_helpers {
                        let mut todo = vec![a.agent_id.clone()];
                        let mut seen = std::collections::HashSet::new();
                        while let Some(parent) = todo.pop() {
                            for h in ledger.interrupted() {
                                if h.parent_agent_id == parent && seen.insert(h.agent_id.clone()) {
                                    if h.process().is_some() {
                                        stop_survivor(ctx, ledger, &h.agent_id)?;
                                    }
                                    todo.push(h.agent_id);
                                }
                            }
                        }
                    }
                    return Ok(ok_empty());
                }
            }
            let events = stopped_now.map_err(sup_err)?;
            let mut stopped = vec![(id, events)];
            for h in helpers {
                // One that ended meanwhile is simply not stopped again.
                if let Ok(events) = ctx.sup().stop(&h) {
                    stopped.push((h, events));
                }
            }
            for (id, events) in stopped {
                if let Some(mesh) = &ctx.mesh {
                    mesh.unregister_because(&id, "stopped");
                }
                if let Some(s) = &ctx.spawn {
                    s.agent_ended(&id);
                }
                // ADR-0090: what its adapter still asked permission for is withdrawn, and its entries in the conversation say
                // cancelled (the agent left before anyone decided).
                if let Some(approvals) = &ctx.approvals {
                    // ADR-0090 Amendment A: a card to switch this agent to a mode that skips permission questions named its session,
                    // which ends here; no decision may apply it later.
                    for approval in approvals.withdraw_skips(&id, None) {
                        if let Some(ev) = &ctx.events {
                            ev.approval_resolved(&approval, false, "");
                        }
                    }
                    for approval in approvals.cancel_adapter_requests(&id) {
                        ctx.chat.set_permission(
                            &id,
                            &approval,
                            v1::PermissionOutcome::Cancelled,
                            crate::daemon::now_ms(),
                        );
                        if let Some(ev) = &ctx.events {
                            ev.approval_resolved(&approval, false, "");
                        }
                    }
                }
                for e in &events {
                    if let SupervisorEvent::PromptsDropped { agent, tags } = e {
                        ctx.chat.drop_queued(agent, tags, crate::daemon::now_ms());
                        // ADR-0095: a dispatch prompt that waited in the stopped agent's queue gives its task back.
                        if let Some(mesh) = &ctx.mesh {
                            for g in mesh.dispatches().dropped(agent, tags) {
                                mesh.give_back(&g);
                            }
                        }
                    }
                }
                if let Some(ev) = &ctx.events {
                    ev.supervisor_events(&events, &ctx.sup().list());
                }
            }
            Ok(ok_empty())
        }
        Cmd::AgentPrompt(a) => {
            let id = AgentId::new(a.agent_id.clone());
            let sup = ctx.sup();
            if sup.terminal(&id).is_none() {
                // An adapter agent takes prompts through its adapter; the request is part of its conversation.
                let (queued, queue_len) = submit_to_adapter(ctx, sup, &id, &a.text, |_| {})?;
                return Ok(prompt_result(queued, queue_len));
            }
            let control = sup
                .terminal(&id)
                .and_then(|h| h.input_handle().ok())
                .ok_or_else(|| {
                    Error::new(ErrorCode::AgentNotFound).with("agent_id", a.agent_id.as_str())
                })?;
            // The write happens after the supervisor lock is let go (seen with WrongStack: text and Enter must be two writes).
            drop(sup);
            crate::task_dispatch::type_prompt(&control, &a.text).map_err(|_| {
                Error::new(ErrorCode::AgentNotFound).with("agent_id", a.agent_id.as_str())
            })?;
            Ok(ok_empty())
        }
        // ADR-0058: the user's own switch; the supervisor refuses a terminal agent, an adapter without the switch and an id the
        // agent did not offer.
        // The adapter's answer is waited for after the supervisor's lock is let go (`AdapterCall::run`).
        Cmd::AgentSetMode(a) => {
            let id = AgentId::new(a.agent_id.clone());
            let (prepared, twins) = {
                let sup = ctx.sup();
                let prepared =
                    sup.prepare_set_option(&id, agora_adapters::host::OptionKind::Mode, &a.mode);
                (prepared, sup.mode_config_ids(&id))
            };
            let mut settles = vec![(SkipKind::Mode, "mode".to_string())];
            settles.extend(twins.into_iter().map(|c| (SkipKind::Config, c)));
            switch(ctx, &id, &settles, prepared)
        }
        Cmd::AgentSetModel(a) => {
            let id = AgentId::new(a.agent_id.clone());
            let prepared = ctx.sup().prepare_set_option(
                &id,
                agora_adapters::host::OptionKind::Model,
                &a.model,
            );
            switch(ctx, &id, &[], prepared)
        }
        // ADR-0090: the user's own change of an option the agent reported (the reasoning effort is one). The supervisor checks the id and
        // the value against the report, and the node's `skip_permissions` policy against what the choice does.
        Cmd::AgentSetConfig(a) => {
            let id = AgentId::new(a.agent_id.clone());
            let (prepared, twins) = {
                let sup = ctx.sup();
                let prepared = sup.prepare_set_config(&id, &a.config_id, &a.value);
                (prepared, sup.mode_config_ids(&id))
            };
            let mut settles = vec![(SkipKind::Config, a.config_id.clone())];
            if twins.contains(&a.config_id) {
                settles.push((SkipKind::Mode, "mode".to_string()));
            }
            switch(ctx, &id, &settles, prepared)
        }
        // ADR-0090: ends the agent's current turn; the agent and its process live on (`AgentStop` ends them). Permission requests of
        // the turn are withdrawn and shown as cancelled; queued prompts are kept unless `drop_queue` says to throw them away.
        Cmd::AgentCancelTurn(a) => {
            let id = AgentId::new(a.agent_id.clone());
            let call = ctx.sup().prepare_cancel_turn(&id).map_err(sup_err)?;
            let generation = call.generation();
            call.run().map_err(sup_err)?;
            let dropped = ctx
                .sup()
                .adapter_turn_cancelled(&id, generation, a.drop_queue);
            let now = crate::daemon::now_ms();
            ctx.chat.drop_queued(&id, &dropped, now);
            // ADR-0095: a dropped dispatch prompt gives its task back.
            if let Some(mesh) = &ctx.mesh {
                for g in mesh.dispatches().dropped(&id, &dropped) {
                    mesh.give_back(&g);
                }
            }
            if let Some(approvals) = &ctx.approvals {
                for approval in approvals.cancel_adapter_requests(&id) {
                    ctx.chat
                        .set_permission(&id, &approval, v1::PermissionOutcome::Cancelled, now);
                    if let Some(ev) = &ctx.events {
                        ev.approval_resolved(&approval, false, "");
                    }
                }
            }
            Ok(ok_empty())
        }
        Cmd::TerminalInput(t) => {
            // The write happens after the supervisor lock is let go: a program that does not read its input blocks this
            // call, and nothing else (ticks, stop and shutdown keep going).
            let control = with_terminal(ctx, &t.terminal_id, |h| h.input_handle())?;
            control.write_input(&t.data).map_err(|_| {
                Error::new(ErrorCode::AgentNotFound).with("agent_id", t.terminal_id.as_str())
            })?;
            Ok(ok_empty())
        }
        Cmd::TerminalResize(t) => {
            let cols = u16::try_from(t.cols).map_err(|_| invalid("cols"))?;
            let rows = u16::try_from(t.rows).map_err(|_| invalid("rows"))?;
            // A terminal is at most a few thousand cells wide; the emulator allocates the whole grid.
            if cols == 0 || cols > MAX_TERMINAL_COLS {
                return Err(invalid("cols"));
            }
            // F1361: a bad row count is named as such, as one that does not fit u16 is above.
            if rows == 0 || rows > MAX_TERMINAL_ROWS {
                return Err(invalid("rows"));
            }
            with_terminal(ctx, &t.terminal_id, |h| h.resize(Size { cols, rows }))?;
            Ok(ok_empty())
        }
        Cmd::ApprovalResolve(r) => {
            use crate::approvals::ApprovalError;
            let Some(approvals) = &ctx.approvals else {
                return Err(capability_missing("approval.resolve"));
            };
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
            // ADR-0101: the decision is checked against the node's policy (read from its file at node start) and the launch of the
            // agent the request is routed to, as they are now, before the queue is locked (the supervisor's lock is never taken under
            // it). The run checked is kept: the answer goes only to that run (Codex review of PR #1007).
            let (decide, checked_run) = approvals
                .agent_of(&r.approval_id)
                .map(|agent| {
                    let sup = ctx.sup();
                    (sup.approval_decide(&agent), sup.adapter_epoch(&agent))
                })
                .unwrap_or_default();
            let resolution = approvals
                .resolve_decided(r, now, decide)
                .map_err(|e| match e {
                    ApprovalError::Unknown => invalid("approval_id"),
                    ApprovalError::Expired => Error::new(ErrorCode::ApprovalExpired)
                        .with("approval_id", r.approval_id.as_str()),
                    ApprovalError::BadSignature => Error::new(ErrorCode::ApprovalSignatureInvalid)
                        .with("approval_id", r.approval_id.as_str()),
                    ApprovalError::Forbidden => {
                        Error::new(ErrorCode::AuthForbidden).with("action", "approval.resolve")
                    }
                    ApprovalError::InvalidScope => invalid("scope"),
                    ApprovalError::ScopeUnavailable => {
                        Error::new(ErrorCode::ApprovalScopeUnavailable)
                    }
                    ApprovalError::ScopeForbidden => Error::new(ErrorCode::ApprovalScopeForbidden),
                    ApprovalError::AdapterOutdated => {
                        Error::new(ErrorCode::ApprovalAdapterOutdated)
                    }
                })?;
            // ADR-0057: a start request is the node's own, not an adapter's; its decision (verified above) goes to the spawner,
            // which announces it with the grant that was signed.
            if resolution.request.action == crate::approvals::START_ACTION {
                if let Some(s) = &ctx.spawn {
                    s.decided(&resolution.request, r)?;
                }
                return Ok(ok_empty());
            }
            // ADR-0090 Amendment A: a switch to a mode or option that skips permissions is the node's own request, decided over its own
            // domain; its answer never goes to an adapter.
            if resolution.request.action == crate::approvals::SKIP_ACTION {
                return decide_skip(ctx, r, &resolution);
            }
            let (agent, adapter_id, verdict) = (
                resolution.agent,
                resolution.adapter_approval_id,
                resolution.verdict,
            );
            let approved = verdict == crate::approvals::Verdict::Approved;
            let always = resolution.always && approved;
            let call = ctx
                .sup()
                .prepare_resolve_approval_scoped(&agent, &adapter_id, approved, always)
                .and_then(|c| {
                    // The process was replaced since the policy was checked: the request was the former process's.
                    if checked_run.is_some_and(|run| run != c.epoch()) {
                        Err(crate::supervisor::SupervisorError::NotFound(agent.clone()))
                    } else {
                        Ok(c)
                    }
                });
            let run = call
                .as_ref()
                .ok()
                .map(crate::supervisor::AdapterCall::epoch);
            // The adapter answers only once the answer was written to the agent (the ACP bridge waits for its writer), so a success
            // here means the agent was given it.
            if let Err(e) = call.and_then(crate::supervisor::AdapterCall::run) {
                // The decision was verified but the adapter could not be told (it is gone, did not answer, or no longer holds the
                // request, as when an ACP bridge already answered it `cancelled`): the request is over, and the entry in the
                // conversation must not stay pending for good.
                ctx.chat.set_permission(
                    &agent,
                    &r.approval_id,
                    v1::PermissionOutcome::Cancelled,
                    crate::daemon::now_ms(),
                );
                // The request already left the queue: the hub is told as for a cancelled turn (unsigned, not approved), or it
                // would keep showing an approval the node no longer holds.
                if let Some(ev) = &ctx.events {
                    ev.approval_resolved(&r.approval_id, false, "");
                }
                return Err(sup_err(e));
            }
            // ADR-0090: the entry in the conversation shows the verified decision, never anything the agent said.
            ctx.chat.set_permission_as(
                &agent,
                &r.approval_id,
                if approved {
                    v1::PermissionOutcome::Approved
                } else {
                    v1::PermissionOutcome::Denied
                },
                always,
                crate::daemon::now_ms(),
            );
            if let Some(ev) = &ctx.events {
                if always {
                    // ADR-0101: only an always answer the adapter delivered is recorded as one the agent holds, and only for the run it
                    // was delivered to. Checked and recorded under the supervisor's lock (Codex review of PR #1007): a restart, resume,
                    // exit or stop (which begin another run and clear the agent's records with their own events) is ordered either
                    // before the check, which then fails, or after the record. A late answer of a former process is audited as given
                    // (`ApprovalResolved` with ALWAYS) but never marks its replacement.
                    record_always_answer(
                        &ctx.sup(),
                        ev,
                        &agent,
                        run,
                        &r.approval_id,
                        &resolution.request.action,
                        &r.signer_device_id,
                    );
                } else {
                    ev.approval_resolved(&r.approval_id, approved, &r.signer_device_id);
                }
            }
            Ok(ok_empty())
        }
        Cmd::GitStatus(g) => {
            let wt = agent_worktree(ctx, &g.worktree_id)?;
            let st = git::status(&wt).map_err(|e| e.to_protocol())?;
            Ok(v1::CommandOk {
                result: Some(Ok_::GitStatus(v1::GitStatusResult {
                    files_changed: st.files_changed,
                    insertions: st.insertions,
                    deletions: st.deletions,
                    head_commit: st.head_commit,
                })),
            })
        }
        Cmd::GitLog(g) => {
            let wt = agent_worktree(ctx, &g.worktree_id)?;
            let limit = match g.limit {
                0 => 30,
                n => (n as usize).min(200),
            };
            let commits = git::log(&wt, limit)
                .map_err(|e| e.to_protocol())?
                .into_iter()
                .map(|c| v1::GitCommit {
                    id: c.id,
                    subject: c.subject,
                    author: c.author,
                    at_ms: c.at_ms,
                })
                .collect();
            Ok(v1::CommandOk {
                result: Some(Ok_::GitLog(v1::GitLogResult { commits })),
            })
        }
        Cmd::GitDiff(g) => {
            let wt = agent_worktree(ctx, &g.worktree_id)?;
            let path = (!g.path.is_empty()).then_some(g.path.as_str());
            // A path is a path inside the workspace: no absolute paths, no `..`, no git pathspec magic (`:(...)`).
            if path.is_some_and(|p| {
                p.starts_with(['/', ':', '-', '\\'])
                    || p.as_bytes().get(1) == Some(&b':') // a Windows drive (`C:\...`)
                    || p.split(['/', '\\']).any(|c| c == ".." || c == ".git")
            }) {
                return Err(invalid("path"));
            }
            let (mut diff, secret) =
                git::diff_with(&wt, path, ctx.show_secret).map_err(|e| e.to_protocol())?;
            if diff.len() > MAX_DIFF_BYTES {
                let mut end = MAX_DIFF_BYTES;
                while !diff.is_char_boundary(end) {
                    end -= 1;
                }
                diff.truncate(end);
            }
            Ok(v1::CommandOk {
                result: Some(Ok_::GitDiff(v1::GitDiffResult {
                    diff,
                    secret_files: secret
                        .into_iter()
                        .map(|c| v1::SecretFileChange {
                            path: c.path,
                            added: c.added,
                            removed: c.removed,
                        })
                        .collect(),
                })),
            })
        }
        Cmd::FilesList(f) => {
            if is_git_dir(&f.path) {
                return Err(invalid("path"));
            }
            let files = open_files(ctx, &f.worktree_id)?;
            let entries = files
                .list(&f.path)
                .map_err(|e| e.to_protocol())?
                .into_iter()
                .filter(|e| e.name != ".git") // git's own directory (a file in a linked worktree) is not project content
                .map(|e| v1::FileEntry {
                    name: e.name,
                    is_dir: e.is_dir,
                    size_bytes: e.size,
                    secret: e.secret && !ctx.show_secret,
                })
                .collect();
            Ok(v1::CommandOk {
                result: Some(Ok_::FilesList(v1::FilesListResult { entries })),
            })
        }
        Cmd::FilesRead(f) => {
            if is_git_dir(&f.path) {
                return Err(invalid("path"));
            }
            let files = open_files(ctx, &f.worktree_id)?;
            let data = files.read(&f.path).map_err(|e| e.to_protocol())?;
            Ok(v1::CommandOk {
                result: Some(Ok_::FilesRead(v1::FilesReadResult { data })),
            })
        }
        other => Err(capability_missing(command_name(other))),
    }
}

/// ADR-0060: ends the process of an interrupted agent that outlived the node (only when it is still the recorded process: one that has
/// ended, or a pid now used by another process, is not signalled), forgets it, and says the agent is interrupted again, now without
/// `still_running`.
fn stop_survivor(
    ctx: &ControlContext,
    ledger: &crate::ledger::Ledger,
    agent_id: &str,
) -> Result<(), Error> {
    if let Some(p) = ledger.get(agent_id).and_then(|e| e.process()) {
        agora_platform::kill_process_if_same(&p)
            .map_err(|_| Error::new(ErrorCode::Internal).with("incident_id", "unassigned"))?;
    }
    ledger.set_process(agent_id, None, 0, crate::daemon::now_ms());
    if let (Some(ev), Some(e)) = (&ctx.events, ledger.get(agent_id)) {
        let resumable = !e.session_id.is_empty() && ctx.sup().resumable(&e.kind);
        ev.agent_interrupted(&e, resumable);
    }
    Ok(())
}

/// ADR-0060: brings back an agent the session ledger holds as interrupted, under the same id, in the same folder and project, with its
/// own session (`new_session` false: the kind's `[resume]` argv or the adapter's session load) or a new one ("Start again"). Refused
/// when the ledger does not hold it as interrupted, when Resume is asked without a session the kind can continue, when the folder is
/// gone or changed identity (ADR-0052), for a helper whose parent does not run (ADR-0057; its grants and approvals are not restored),
/// and by the same confinement and limits as a start. The routed path checks the device signature before this (`Grants`).
/// `carry_on` (Amendment A, only for the node's own automatic resume of an agent that was working): the resume carries the kind's one
/// "carry on" message when its manifest has one; the ledger then records that it was sent.
pub(crate) fn resume_agent(
    ctx: &ControlContext,
    agent_id: &str,
    new_session: bool,
    carry_on: bool,
) -> Result<v1::AgentLaunchResult, Error> {
    let ledger = ctx
        .ledger
        .as_ref()
        .ok_or_else(|| capability_missing("agent.resume"))?;
    let not_interrupted = || Error::new(ErrorCode::AgentNotInterrupted).with("agent_id", agent_id);
    let entry = ledger
        .get(agent_id)
        .filter(crate::ledger::Entry::interrupted)
        .ok_or_else(not_interrupted)?;
    let id = AgentId::new(agent_id);
    let folder = PathBuf::from(&entry.folder);
    let folder_changed = || {
        Error::new(ErrorCode::AgentFolderChanged)
            .with("agent_id", agent_id)
            .with("path", entry.folder.as_str())
    };
    // Its process from before the restart still runs: a second one would continue the same conversation.
    if entry.still_running() {
        return Err(Error::new(ErrorCode::AgentStillRunning).with("agent_id", agent_id));
    }
    let parent = (!entry.parent_agent_id.is_empty()).then(|| AgentId::new(&entry.parent_agent_id));
    {
        let sup = ctx.sup();
        let views = sup.list();
        if views.iter().any(|v| v.id == id) {
            return Err(not_interrupted());
        }
        if let Some(p) = &parent {
            if !views.iter().any(|v| v.id == *p && v.running) {
                return Err(Error::new(ErrorCode::AgentParentNotRunning)
                    .with("agent_id", agent_id)
                    .with("parent_agent_id", p.as_str()));
            }
        }
        // The id goes on a command line for a terminal agent: checked here again, whatever the ledger holds.
        if !new_session
            && (!crate::session_files::argv_safe(&entry.session_id) || !sup.resumable(&entry.kind))
        {
            return Err(Error::new(ErrorCode::AgentSessionMissing).with("agent_id", agent_id));
        }
    }
    // The folder must still be the one the agent worked in: there, a directory, with the same project identity (ADR-0052), and for a
    // selected folder the same canonical path and no unconfirmed change of its identity.
    // F1021: a selected folder is held from before it is checked, and the agent is resumed in the held folder: one that the path no
    // longer leads to after the check (it, or a folder above it, moved or swapped for a link) is refused.
    let held = entry
        .directory
        .then(|| agora_platform::HeldFolder::open(&folder));
    let project = if entry.directory {
        let (info, project) = crate::directory::inspect(&entry.folder, &ctx.node_id, false)
            .map_err(|_| folder_changed())?;
        let confirmed = ctx
            .directories
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .previous(&info)
            .is_empty();
        if info.path != entry.folder || info.project_id != entry.project_id || !confirmed {
            return Err(folder_changed());
        }
        project
    } else {
        let checkout = git::discover_checkout(&folder).map_err(|_| folder_changed())?;
        if checkout.project.to_string() != entry.project_id {
            return Err(folder_changed());
        }
        checkout.project
    };
    let checked = held
        .map(|h| {
            h.ok()
                .filter(agora_platform::HeldFolder::in_place)
                .ok_or_else(folder_changed)
        })
        .transpose()?;
    let restore = crate::supervisor::Restore {
        id: id.clone(),
        kind: entry.kind.clone(),
        project,
        folder: folder.clone(),
        directory: entry.directory,
        branch: entry.branch.clone(),
        session: (!new_session).then(|| entry.session_id.clone()),
        carry_on: carry_on && !new_session,
        private_config: (entry.directory && !entry.managed_worktree).then(|| {
            ctx.worktree_root
                .join(".sessions")
                .join(format!("resume-{agent_id}"))
        }),
    };
    let (events, views, has_terminal, carried) = {
        let mut sup = ctx.sup();
        if !sup.sandbox_policy().enforceable() {
            return Err(SupervisorError::SandboxUnavailable(
                entry.kind.clone(),
                sup.sandbox_policy().level.name(),
            )
            .to_protocol());
        }
        if entry.directory && !entry.managed_worktree {
            sup.refuse_protected(&folder, &ctx.worktree_root)
                .map_err(sup_err)?;
        }
        sup.restore(restore).map_err(sup_err)?;
        if let Some(p) = &parent {
            sup.set_parent(&id, p);
        }
        match sup.resume_in(&id, Instant::now(), checked) {
            Ok(events) => (
                events,
                sup.list(),
                sup.terminal(&id).is_some(),
                sup.carried_on(&id),
            ),
            Err(e) => {
                sup.unrestore(&id);
                return Err(sup_err(e));
            }
        }
    };
    // Said once for this interruption: the ledger remembers it (so it is not sent again), and the node's log says it was sent.
    if carried {
        ledger.mark_continue_sent(agent_id, crate::daemon::now_ms());
        eprintln!("resume: {agent_id} was working when the node stopped; its resume carried the kind's \"carry on\" message");
    }
    if new_session {
        ledger.forget_session(agent_id, crate::daemon::now_ms());
    }
    // A resumed helper may not start helpers of its own: the decision that once allowed it is not restored (ADR-0060, 5).
    if let (Some(p), Some(s)) = (&parent, &ctx.spawn) {
        s.resumed_helper(&id, p);
    }
    let path = views
        .iter()
        .find(|v| v.id == id)
        .map(|v| v.worktree.to_string_lossy().into_owned())
        .unwrap_or_else(|| entry.folder.clone());
    let workspace_id = if entry.directory {
        format!("directory:{path}")
    } else {
        String::new()
    };
    if let Some(ev) = &ctx.events {
        if entry.directory {
            ev.helper_launched(
                &id,
                &v1::WorkspaceRegistered {
                    workspace_id: workspace_id.clone(),
                    node_id: ctx.node_id.clone(),
                    project_id: entry.project_id.clone(),
                    path: path.clone(),
                    git_root: entry.git_root.clone(),
                    branch: entry.branch.clone(),
                    managed_worktree: entry.managed_worktree,
                    files_changed: 0,
                },
                &entry.kind,
                has_terminal,
                &entry.parent_agent_id,
                // The options it was started with (ADR-0059: for its whole life, resumes included), so the badge stays on it.
                &views
                    .iter()
                    .find(|v| v.id == id)
                    .map(|v| crate::launch_options::LaunchChoice {
                        ids: v.launch_options.clone(),
                        args: Vec::new(),
                        skips_permissions: v.skips_permissions,
                    })
                    .unwrap_or_default(),
            );
            // Its start is said above with its folder; the rest (its state, its session) as usual.
            let rest: Vec<SupervisorEvent> = events
                .into_iter()
                .filter(|e| !matches!(e, SupervisorEvent::Resumed { .. }))
                .collect();
            ev.supervisor_events(&rest, &views);
        } else {
            ev.supervisor_events(&events, &views);
        }
    }
    ctx.wake_git();
    Ok(v1::AgentLaunchResult {
        agent_id: agent_id.to_string(),
        terminal_id: if has_terminal {
            terminal_id_of(&id)
        } else {
            String::new()
        },
        workspace_id,
        path,
    })
}

/// Git's own directory (or a file standing in for it in a linked worktree) is bookkeeping, not project content: its config
/// can hold remote URLs with credentials, and a linked worktree's `.git` file names an absolute path elsewhere.
fn is_git_dir(path: &str) -> bool {
    path.split(['/', '\\']).any(|c| c == ".git")
}

/// The largest terminal a client may ask for.
const MAX_TERMINAL_COLS: u16 = 1000;
const MAX_TERMINAL_ROWS: u16 = 1000;

/// Longest one `directory.list` spends reading a folder before it answers with what it has (`truncated`); well inside the
/// routed command deadline.
const LIST_BUDGET: Duration = Duration::from_secs(5);

/// Largest file `files.read` returns.
const MAX_READ_BYTES: u64 = 512 * 1024;

fn open_files(ctx: &ControlContext, worktree_id: &str) -> Result<crate::files::Files, Error> {
    let wt = agent_worktree(ctx, worktree_id)?;
    let files = crate::files::Files::open(&wt, MAX_READ_BYTES)
        .map(|f| f.with_secret_shown(ctx.show_secret))
        .map_err(|e| e.to_protocol())?;
    // Judged again on the root `Files` settled on (its own resolution, which every later read and listing is checked against): a
    // link swapped in after the check above is refused too (audit F1011).
    if !still_in_place(ctx, &wt, files.root()) {
        return Err(invalid("worktree_id"));
    }
    Ok(files)
}

/// Whether an agent's folder, recorded as `recorded` when the agent started, is still that folder, `resolved` being where it leads
/// now (audit F1011). A selected folder is recorded canonical (`directory::inspect`), so it must still resolve to itself; a folder
/// below the node's worktree root must resolve to that root (the node's own, resolved) joined with the rest. Otherwise a folder
/// swapped for a link since the start, by anything that can write beside it (another agent of the project), would lead files, diffs
/// and history wherever the link points, and the secret-file names (ADR-0080) would be judged below the link's target.
fn still_in_place(ctx: &ControlContext, recorded: &Path, resolved: &Path) -> bool {
    match recorded.strip_prefix(&ctx.worktree_root) {
        // Both sides in the system's own resolved spelling: the plain one (`canonicalize_plain`) depends on a path's length on
        // Windows, which the root and the folder below it need not share.
        Ok(rest) => match (
            std::fs::canonicalize(&ctx.worktree_root),
            std::fs::canonicalize(resolved),
        ) {
            (Ok(root), Ok(now)) => now == root.join(rest),
            _ => false,
        },
        // Spelled as `directory::inspect` spelled it.
        Err(_) => agora_platform::canonicalize_plain(resolved).is_ok_and(|now| now == recorded),
    }
}

/// Largest diff one answer carries; a bigger one is cut (the file view is for reading, not for patching).
const MAX_DIFF_BYTES: usize = 512 * 1024;

/// The workspace of one of this node's agents, by the id the node reported it under. Only those: a client cannot
/// point a git command at any other directory, nor at one a link put in the place of an agent's folder ([`still_in_place`]).
fn agent_worktree(ctx: &ControlContext, worktree_id: &str) -> Result<PathBuf, Error> {
    let wt = ctx
        .sup()
        .list()
        .into_iter()
        .find(|v| v.worktree.to_string_lossy() == worktree_id)
        .map(|v| v.worktree)
        .ok_or_else(|| invalid("worktree_id"))?;
    if !still_in_place(ctx, &wt, &wt) {
        return Err(invalid("worktree_id"));
    }
    Ok(wt)
}

fn with_terminal<R>(
    ctx: &ControlContext,
    terminal_id: &str,
    f: impl FnOnce(&crate::terminal::TerminalHandle) -> io::Result<R>,
) -> Result<R, Error> {
    let not_found = || Error::new(ErrorCode::AgentNotFound).with("agent_id", terminal_id);
    let id = agent_of_terminal(terminal_id).ok_or_else(not_found)?;
    let sup = ctx.sup();
    let h = sup.terminal(&id).ok_or_else(not_found)?;
    f(h).map_err(|_| not_found())
}

pub(crate) fn command_name(c: &Cmd) -> &'static str {
    match c {
        Cmd::ProjectList(_) => "project.list",
        Cmd::AgentStart(_) => "agent.start",
        Cmd::DirectoryInspect(_) => "directory.inspect",
        Cmd::DirectoryList(_) => "directory.list",
        Cmd::AgentForget(_) => "agent.forget",
        Cmd::NodeForget(_) => "node.forget",
        Cmd::AgentLaunch(_) => "agent.launch",
        Cmd::AgentResume(_) => "agent.resume",
        Cmd::WorkspaceQuery(_) => "workspace.query",
        Cmd::AgentStop(_) => "agent.stop",
        Cmd::AgentPrompt(_) => "agent.prompt",
        Cmd::AgentSetMode(_) => "agent.set_mode",
        Cmd::AgentSetModel(_) => "agent.set_model",
        Cmd::AgentSetConfig(_) => "agent.set_config",
        Cmd::AgentCancelTurn(_) => "agent.cancel_turn",
        Cmd::TerminalOpen(_) => "terminal.open",
        Cmd::TerminalInput(_) => "terminal.input",
        Cmd::TerminalResize(_) => "terminal.resize",
        Cmd::TerminalSubscribe(_) => "terminal.subscribe",
        Cmd::ChatSubscribe(_) => "chat.subscribe",
        Cmd::FilesList(_) => "files.list",
        Cmd::FilesRead(_) => "files.read",
        Cmd::FilesWrite(_) => "files.write",
        Cmd::FilesWatch(_) => "files.watch",
        Cmd::GitDiff(_) => "git.diff",
        Cmd::GitStatus(_) => "git.status",
        Cmd::GitLog(_) => "git.log",
        Cmd::WorktreeCreate(_) => "worktree.create",
        Cmd::WorktreeRemove(_) => "worktree.remove",
        Cmd::ApprovalResolve(_) => "approval.resolve",
        Cmd::MeshPost(_) => "mesh.post",
        Cmd::AgentList(_) => "agent.list",
        Cmd::AgentKinds(_) => "agent.kinds",
        Cmd::UsageReport(_) => "usage.report",
        Cmd::Enroll(_) => "enroll",
        Cmd::TaskUpdate(_) => "task.update",
        Cmd::TaskDispatch(_) => "task.dispatch",
        Cmd::BoardCall(_) => "board.call",
        Cmd::BoardJoin(_) => "board.join",
        Cmd::NodePolicyGet(_) => "node.policy.get",
        Cmd::GrantCreate(_) => "grant.create",
        Cmd::GrantList(_) => "grant.list",
        Cmd::GrantRevoke(_) => "grant.revoke",
        Cmd::DeviceRegister(_) => "device.register",
        Cmd::DeviceList(_) => "device.list",
        Cmd::DeviceEndorse(_) => "device.endorse",
        Cmd::DeviceRevoke(_) => "device.revoke",
        Cmd::DeviceAskApproval(_) => "device.ask_approval",
        // ADR-0065: the pairing messages exist in the schema; no node code handles them yet, so they are refused as unsupported.
        Cmd::PairOpen(_) => "pair.open",
        Cmd::PairClose(_) => "pair.close",
        Cmd::PairStatus(_) => "pair.status",
        Cmd::PairApprove(_) => "pair.approve",
        Cmd::PairDeny(_) => "pair.deny",
        Cmd::PairRequest(_) => "pair.request",
        Cmd::PairReveal(_) => "pair.reveal",
        // ADR-0088: the preview messages exist in the schema; no node code handles them yet, so they are refused as unsupported.
        Cmd::PreviewOpen(_) => "preview.open",
        Cmd::PreviewClose(_) => "preview.close",
        Cmd::PreviewList(_) => "preview.list",
        // ADR-0085: renewal is asked of the hub, on the machine's own connection; a node never receives it (refused as unsupported).
        Cmd::Renew(_) => "enroll.renew",
        Cmd::PreviewConnect(_) => "preview.connect",
        // ADR-0082: only the local control socket answers it (in `handle_request`); anywhere else it is refused as unsupported.
        Cmd::ControlAuth(_) => "control.auth",
    }
}

fn response(request_id: &str, r: Result<v1::CommandOk, Error>) -> v1::CommandResponse {
    v1::CommandResponse {
        request_id: request_id.to_string(),
        result: Some(match r {
            Ok(ok) => Res::Ok(ok),
            Err(e) => Res::Err(e.to_info()),
        }),
    }
}

/// A running control server. Dropping it stops accepting and removes the socket file.
pub struct ControlServer {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    path: PathBuf,
    /// ADR-0082: the control token file this server wrote, removed when it stops.
    token_file: Option<PathBuf>,
}

impl ControlServer {
    pub fn start(path: &Path, ctx: Arc<ControlContext>) -> io::Result<Self> {
        let listener = local_listen(path)?;
        // Non-blocking so the accept after the readiness wait below never waits (a client that left in between).
        listener.set_nonblocking(true)?;
        // After the bind, which holds the node's lock: a second node on the same home fails there and never replaces the token of the
        // running one. Before the accept loop, so every connection that is answered is checked against the token in the file.
        let token_file = match &ctx.control_token {
            Some((token, file)) => {
                token.write(file)?;
                Some(file.clone())
            }
            None => None,
        };
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("agora-control".into())
            .spawn(move || {
                while !stop2.load(Ordering::Acquire) {
                    // A readiness wait, not a sleep between tries (#958): the hub's routed subscriptions each open a connection, and
                    // a fixed poll made every one of them wait up to `POLL` before it was read. The stop is still seen within `POLL`,
                    // and at once when `Drop` wakes the wait.
                    match listener.accept_timeout(POLL) {
                        // The wake-up connection from `Drop`, or a client that came as the server stops: not served.
                        Ok(Some(_)) if stop2.load(Ordering::Acquire) => break,
                        Ok(Some(stream)) => {
                            let (ctx, stop) = (Arc::clone(&ctx), Arc::clone(&stop2));
                            let _ = std::thread::Builder::new()
                                .name("agora-control-conn".into())
                                .spawn(move || handle_connection(stream, &ctx, &stop));
                        }
                        Ok(None) => {}
                        // A failing accept (no descriptors left) must not spin: back off before the next try.
                        Err(_) => std::thread::sleep(POLL),
                    }
                }
            })?;
        Ok(Self {
            stop,
            thread: Some(thread),
            path: path.to_path_buf(),
            token_file,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            // Best effort: wake the accept wait with a connection of our own, so stopping usually does not wait out `POLL`. Only while
            // the accept thread runs, so something drains the socket's queue (a Unix connect does not wait on a full one). The
            // connect can still take its time: on Windows a busy pipe makes it wait for a free instance up to 5 s (agora-platform's
            // `CONNECT_BUSY_LIMIT`, #974) before it fails; a busy pipe means a client is already there, which wakes the accept anyway.
            // The stop does not depend on it: the accept loop sees the flag within `POLL` either way.
            if !t.is_finished() {
                let _ = local_connect(&self.path);
            }
            let _ = t.join();
        }
        if let Some(f) = &self.token_file {
            let _ = std::fs::remove_file(f);
        }
    }
}

type SharedWriter = Arc<Mutex<LocalStream>>;

fn send(w: &SharedWriter, stream_id: u32, kind: u8, payload: Vec<u8>) -> io::Result<()> {
    let frame = Frame {
        stream_id,
        kind,
        seq: None,
        payload,
    };
    let bytes = frame
        .encode(DEFAULT_MAX_PAYLOAD)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "frame too large"))?;
    let mut g = w.lock().unwrap_or_else(|e| e.into_inner());
    g.write_all(&bytes)
}

fn handle_connection(stream: LocalStream, ctx: &Arc<ControlContext>, stop: &Arc<AtomicBool>) {
    let Ok(reader_side) = stream.try_clone() else {
        return;
    };
    let _ = reader_side.set_read_timeout(Some(POLL));
    let mut reader = reader_side;
    let writer: SharedWriter = Arc::new(Mutex::new(stream));
    let alive = Arc::new(AtomicBool::new(true));
    let mut decoder = Decoder::new(DEFAULT_MAX_PAYLOAD);
    let mut buf = vec![0u8; 16 * 1024];
    // ADR-0082: a node that requires the token answers nothing but `ControlAuth` until it succeeded on this connection.
    let mut authed = ctx.control_token.is_none();
    'conn: while !stop.load(Ordering::Acquire) {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => decoder.push(&buf[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                continue
            }
            Err(_) => break,
        }
        loop {
            match decoder.next_frame() {
                Ok(Some(frame)) => {
                    if frame.kind == KIND_REQUEST
                        && !handle_request(&frame, ctx, &writer, &alive, stop, &mut authed)
                    {
                        break 'conn;
                    }
                }
                Ok(None) => break,
                Err(_) => break 'conn,
            }
        }
    }
    alive.store(false, Ordering::Release);
    let _ = writer.lock().map(|w| w.shutdown());
}

fn handle_request(
    frame: &Frame,
    ctx: &Arc<ControlContext>,
    writer: &SharedWriter,
    alive: &Arc<AtomicBool>,
    stop: &Arc<AtomicBool>,
    authed: &mut bool,
) -> bool {
    let Ok(req) = v1::CommandRequest::decode(frame.payload.as_slice()) else {
        return true;
    };
    // ADR-0082. Before anything else, so not even a stored answer (`idempotency_key`) reaches a connection that has not proved itself.
    if let Some(Cmd::ControlAuth(a)) = &req.command {
        let ok = ctx
            .control_token
            .as_ref()
            .is_none_or(|(t, _)| t.verify(&a.token));
        let result = if ok {
            Ok(ok_empty())
        } else {
            Err(Error::new(ErrorCode::AuthUnauthenticated))
        };
        let resp = response(&req.request_id, result);
        let _ = send(writer, STREAM_COMMANDS, KIND_RESPONSE, resp.encode_to_vec());
        *authed = ok;
        // A wrong token ends the connection: no second guess on it.
        return ok;
    }
    if !*authed {
        let resp = response(
            &req.request_id,
            Err(Error::new(ErrorCode::AuthUnauthenticated)),
        );
        let _ = send(writer, STREAM_COMMANDS, KIND_RESPONSE, resp.encode_to_vec());
        return true;
    }
    // F1360: held until this attempt's answer is stored, so a retry with the same key waits for it instead of running again.
    let _claim = if req.idempotency_key.is_empty() {
        None
    } else {
        match ctx.claim(&req.idempotency_key) {
            Ok(claim) => Some(claim),
            Err(mut prev) => {
                // Same result, addressed to this attempt: the client matches responses by request id.
                prev.request_id.clone_from(&req.request_id);
                let _ = send(writer, STREAM_COMMANDS, KIND_RESPONSE, prev.encode_to_vec());
                return true;
            }
        }
    };
    let Some(cmd) = &req.command else {
        let resp = response(&req.request_id, Err(invalid("command")));
        let _ = send(writer, STREAM_COMMANDS, KIND_RESPONSE, resp.encode_to_vec());
        return true;
    };

    // Terminal subscriptions need the stream the request arrived on.
    if let Cmd::TerminalSubscribe(sub) = cmd {
        let outcome = subscribe(ctx, sub, frame.stream_id);
        let resp = response(
            &req.request_id,
            outcome.as_ref().map(|_| ok_empty()).map_err(Clone::clone),
        );
        let _ = send(writer, STREAM_COMMANDS, KIND_RESPONSE, resp.encode_to_vec());
        if let Ok(Subscribed {
            agent,
            history,
            snap,
            sub,
        }) = outcome
        {
            let (ctx, writer, alive, stop) = (
                Arc::clone(ctx),
                Arc::clone(writer),
                Arc::clone(alive),
                Arc::clone(stop),
            );
            let stream_id = frame.stream_id;
            let _ = std::thread::Builder::new()
                .name("agora-control-term".into())
                .spawn(move || {
                    pump(
                        &ctx, &writer, stream_id, &agent, history, &snap, &sub, &alive, &stop,
                    )
                });
        }
        return true;
    }

    // A chat transcript: the snapshot, then every new entry, on the stream the request arrived on.
    if let Cmd::ChatSubscribe(sub) = cmd {
        let agent = AgentId::new(sub.agent_id.clone());
        let known = ctx.sup().list().iter().any(|v| v.id == agent);
        let outcome = if known {
            Ok(ctx.chat.follow(&agent))
        } else {
            Err(Error::new(ErrorCode::AgentNotFound).with("agent_id", sub.agent_id.as_str()))
        };
        let resp = response(
            &req.request_id,
            outcome.as_ref().map(|_| ok_empty()).map_err(Clone::clone),
        );
        let _ = send(writer, STREAM_COMMANDS, KIND_RESPONSE, resp.encode_to_vec());
        if let Ok((entries, subscription)) = outcome {
            let (writer, alive, stop) = (Arc::clone(writer), Arc::clone(alive), Arc::clone(stop));
            let stream_id = frame.stream_id;
            let _ = std::thread::Builder::new()
                .name("agora-control-chat".into())
                .spawn(move || pump_chat(&writer, stream_id, entries, subscription, &alive, &stop));
        }
        return true;
    }

    let resp = response(&req.request_id, execute(ctx, cmd));
    if !req.idempotency_key.is_empty() {
        ctx.remember(&req.idempotency_key, &resp);
    }
    let _ = send(writer, STREAM_COMMANDS, KIND_RESPONSE, resp.encode_to_vec());
    true
}

/// An accepted terminal subscription: whose terminal, the history frames the window asked for (ADR-0100), the snapshot they end at,
/// and the live output after it.
struct Subscribed {
    agent: AgentId,
    history: Vec<Vec<u8>>,
    snap: Snapshot,
    sub: OutputSubscription,
}

fn subscribe(
    ctx: &ControlContext,
    sub: &v1::TerminalSubscribe,
    stream_id: u32,
) -> Result<Subscribed, Error> {
    if stream_id < agora_protocol::frame::FIRST_DYNAMIC_STREAM {
        return Err(invalid("stream_id"));
    }
    if !sub.format.is_empty() && sub.format != "terminal.raw" {
        return Err(capability_missing(&sub.format));
    }
    let id = agent_of_terminal(&sub.terminal_id).ok_or_else(|| {
        Error::new(ErrorCode::AgentNotFound).with("agent_id", sub.terminal_id.as_str())
    })?;
    let guard = ctx.sup();
    let handle = guard.terminal(&id).ok_or_else(|| {
        Error::new(ErrorCode::AgentNotFound).with("agent_id", sub.terminal_id.as_str())
    })?;
    // ADR-0100: the window's count, capped by what the terminal keeps (and by the replay's byte cap).
    // Subscribed after the supervisor's lock is let go: copying and formatting a history must not hold up other commands.
    let lines = usize::try_from(sub.history_lines).unwrap_or(usize::MAX);
    let watch = handle.watch();
    drop(guard);
    let (history, snap, subscription) = watch.subscribe_with_history(lines);
    Ok(Subscribed {
        agent: id,
        history,
        snap,
        sub: subscription,
    })
}

fn exit_payload(st: &agora_platform::ExitStatus) -> Vec<u8> {
    let code = st
        .code
        .or_else(|| st.signal.map(|s| -s))
        .unwrap_or(i32::MIN);
    code.to_le_bytes().to_vec()
}

/// The exit frame of a terminal that ended in a way this subscriber did not see.
fn unknown_exit_payload() -> Vec<u8> {
    i32::MIN.to_le_bytes().to_vec()
}

/// A terminal size as the resize frame carries it: columns, then rows, each a little-endian u16.
fn size_payload(s: agora_vt::Size) -> Vec<u8> {
    let mut p = s.cols.to_le_bytes().to_vec();
    p.extend_from_slice(&s.rows.to_le_bytes());
    p
}

#[allow(clippy::too_many_arguments)]
fn pump(
    ctx: &ControlContext,
    writer: &SharedWriter,
    stream: u32,
    agent: &AgentId,
    history: Vec<Vec<u8>>,
    snap: &Snapshot,
    sub: &OutputSubscription,
    alive: &AtomicBool,
    stop: &AtomicBool,
) {
    // ADR-0100: the history the window asked for comes first, then the snapshot it ends at; each frame is let go once sent.
    for frame in history {
        if send(writer, stream, KIND_TERM_HISTORY, frame).is_err() {
            return;
        }
    }
    // Each repaint is followed by the size it was made at, so a client that paints the screen itself (rather than resizing the
    // agent to fit) knows how wide the program thinks its lines are.
    let repaint = |snap: &Snapshot| {
        send(writer, stream, KIND_TERM_SNAPSHOT, snap.to_ansi())
            .and_then(|()| send(writer, stream, KIND_TERM_RESIZE, size_payload(snap.size)))
    };
    if repaint(snap).is_err() {
        return;
    }
    while alive.load(Ordering::Acquire) && !stop.load(Ordering::Acquire) {
        let sent = match sub.recv_timeout(POLL) {
            Ok(Output::Data(b)) => send(writer, stream, KIND_TERM_DATA, b.to_vec()),
            Ok(Output::Resize(s)) => send(writer, stream, KIND_TERM_RESIZE, size_payload(s)),
            Ok(Output::Exited(st)) => {
                let _ = send(writer, stream, KIND_TERM_EXIT, exit_payload(&st));
                return;
            }
            // What was dropped can include the exit: a terminal that has ended says so after its last snapshot, and one that is
            // already gone (or a subscription that ends without its exit) gets the unknown exit, so the client still hears it ended.
            Ok(Output::Lagged) => {
                let snap = ctx
                    .sup()
                    .terminal(agent)
                    .map(|h| (h.resync(sub), h.exit_status()));
                match snap {
                    Some((s, None)) => repaint(&s),
                    Some((s, Some(st))) => {
                        let _ = repaint(&s)
                            .and_then(|()| send(writer, stream, KIND_TERM_EXIT, exit_payload(&st)));
                        return;
                    }
                    None => {
                        let _ = send(writer, stream, KIND_TERM_EXIT, unknown_exit_payload());
                        return;
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => Ok(()),
            Err(RecvTimeoutError::Disconnected) => {
                let _ = send(writer, stream, KIND_TERM_EXIT, unknown_exit_payload());
                return;
            }
        };
        if sent.is_err() {
            return;
        }
    }
}

fn pump_chat(
    writer: &SharedWriter,
    stream: u32,
    entries: Vec<v1::ChatEntry>,
    subscription: crate::chat::ChatSubscription,
    alive: &AtomicBool,
    stop: &AtomicBool,
) {
    let snapshot = v1::ChatSnapshot { entries };
    if send(writer, stream, KIND_CHAT_SNAPSHOT, snapshot.encode_to_vec()).is_err() {
        return;
    }
    while alive.load(Ordering::Acquire) && !stop.load(Ordering::Acquire) {
        match subscription.recv_timeout(POLL) {
            Ok(e) => {
                if send(writer, stream, KIND_CHAT_ENTRY, e.encode_to_vec()).is_err() {
                    return;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            // The follower was dropped: the transcript changed faster than this client read it (ADR-0090). The stream is ended
            // explicitly (a close frame), never left silent, so the client (and the uplink relay in front of it) subscribes again and
            // gets the ring as a fresh snapshot instead of keeping a transcript with a hole in it.
            Err(RecvTimeoutError::Disconnected) => {
                let _ = send(writer, stream, STREAM_CLOSE, Vec::new());
                return;
            }
        }
    }
}

/// A blocking client for the control socket (used by the CLI and tests).
pub struct Client {
    stream: LocalStream,
    decoder: Decoder,
    next_request: u64,
    /// Frames that arrived while waiting for a response (terminal output).
    pub pending: VecDeque<Frame>,
}

impl Client {
    /// Opens the node's control socket. When the token file beside the socket holds a token (a node that confines agents on Linux
    /// requires it, ADR-0082) it is presented first, so every caller keeps its own contract; a node that does not know the command (an
    /// older one) or does not require the token carries on. `PermissionDenied` when the node refuses the token.
    pub fn connect(path: &Path) -> io::Result<Self> {
        let mut c = Self::open(path)?;
        let mut tried = None;
        loop {
            let Some(token) =
                crate::control_token::read_token(&crate::control_token::token_path(path))
            else {
                return Ok(c);
            };
            if tried.as_ref() == Some(&token) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "the node refused the control token",
                ));
            }
            match c.call(Cmd::ControlAuth(v1::ControlAuth {
                token: token.clone(),
            }))? {
                Ok(_) => return Ok(c),
                Err(e) if e.code == ErrorCode::AuthUnauthenticated.as_str() => {
                    // A node starting at this moment may not have replaced the file yet: once more, on a new connection, if the
                    // file changed since.
                    tried = Some(token);
                    c = Self::open(path)?;
                }
                // A node that does not know the command: no token needed.
                Err(_) => return Ok(c),
            }
        }
    }

    /// Opens the socket and presents no token, as a client that does not know the token (an old one, a script, a confined agent) does. A
    /// node that requires the token answers every command but `ControlAuth` with `auth.unauthenticated`; tests use this to prove it.
    pub fn connect_unauthenticated(path: &Path) -> io::Result<Self> {
        Self::open(path)
    }

    fn open(path: &Path) -> io::Result<Self> {
        let stream = local_connect(path)?;
        Ok(Self {
            stream,
            decoder: Decoder::new(DEFAULT_MAX_PAYLOAD),
            next_request: 1,
            pending: VecDeque::new(),
        })
    }

    /// The connection itself, for a caller that carries its bytes on (`agora node stdio`). Whatever was proved at connect stays proved.
    pub fn into_stream(self) -> LocalStream {
        self.stream
    }

    /// A second handle for writing from another thread (for example forwarding keystrokes).
    pub fn writer(&self) -> io::Result<RequestWriter> {
        Ok(RequestWriter {
            stream: Arc::new(Mutex::new(self.stream.try_clone()?)),
            next_request: Arc::new(AtomicU64::new(1_000_000)),
        })
    }

    /// Send a command on the commands stream and wait for its response.
    pub fn call(&mut self, cmd: Cmd) -> io::Result<Result<v1::CommandOk, v1::ErrorInfo>> {
        self.call_on(STREAM_COMMANDS, cmd)
    }

    /// Like [`call`](Self::call), sending the request on `stream_id` (opens a terminal stream for
    /// `TerminalSubscribe`).
    pub fn call_on(
        &mut self,
        stream_id: u32,
        cmd: Cmd,
    ) -> io::Result<Result<v1::CommandOk, v1::ErrorInfo>> {
        self.call_full(stream_id, "", cmd)
    }

    /// Like [`call`](Self::call) with an idempotency key: a retry with the same key gets the first response
    /// and does not repeat the effect.
    pub fn call_with_key(
        &mut self,
        key: &str,
        cmd: Cmd,
    ) -> io::Result<Result<v1::CommandOk, v1::ErrorInfo>> {
        self.call_full(STREAM_COMMANDS, key, cmd)
    }

    fn call_full(
        &mut self,
        stream_id: u32,
        key: &str,
        cmd: Cmd,
    ) -> io::Result<Result<v1::CommandOk, v1::ErrorInfo>> {
        let id = self.next_request;
        self.next_request += 1;
        let req = v1::CommandRequest {
            request_id: id.to_string(),
            idempotency_key: key.to_string(),
            command: Some(cmd),
            target_node: String::new(),
        };
        self.send_request(stream_id, &req)?;
        loop {
            // From the wire, not `pending`: frames set aside below must not be read back in this loop.
            let frame = self.read_wire_frame(None)?.ok_or_else(|| {
                io::Error::new(io::ErrorKind::UnexpectedEof, "node closed the connection")
            })?;
            if frame.stream_id == STREAM_COMMANDS && frame.kind == KIND_RESPONSE {
                let resp = v1::CommandResponse::decode(frame.payload.as_slice())
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
                if resp.request_id == id.to_string() {
                    return match resp.result {
                        Some(Res::Ok(ok)) => Ok(Ok(ok)),
                        Some(Res::Err(e)) => Ok(Err(e)),
                        None => Err(io::Error::new(io::ErrorKind::InvalidData, "empty response")),
                    };
                }
            } else {
                self.pending.push_back(frame);
            }
        }
    }

    /// Sends a request as it is (keeping its request id and idempotency key) and waits for its answer, up to `limit`.
    pub fn request(
        &mut self,
        req: &v1::CommandRequest,
        limit: Duration,
    ) -> io::Result<v1::CommandResponse> {
        self.request_on(STREAM_COMMANDS, req, limit)
    }

    /// Like [`request`](Self::request), sending on `stream_id` (a dynamic stream for `TerminalSubscribe`). Frames that
    /// arrive before the answer are kept in `pending`.
    pub fn request_on(
        &mut self,
        stream_id: u32,
        req: &v1::CommandRequest,
        limit: Duration,
    ) -> io::Result<v1::CommandResponse> {
        self.send_request(stream_id, req)?;
        let end = std::time::Instant::now() + limit;
        loop {
            let left = end.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "no answer in time"));
            }
            let Some(frame) = self.read_wire_frame(Some(left))? else {
                continue;
            };
            if frame.stream_id == STREAM_COMMANDS && frame.kind == KIND_RESPONSE {
                let resp = v1::CommandResponse::decode(frame.payload.as_slice())
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
                if resp.request_id == req.request_id {
                    return Ok(resp);
                }
            } else {
                self.pending.push_back(frame);
            }
        }
    }

    fn send_request(&mut self, stream_id: u32, req: &v1::CommandRequest) -> io::Result<()> {
        let bytes = Frame {
            stream_id,
            kind: KIND_REQUEST,
            seq: None,
            payload: req.encode_to_vec(),
        }
        .encode(DEFAULT_MAX_PAYLOAD)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "request too large"))?;
        self.stream.write_all(&bytes)
    }

    /// Next frame: from the pending queue, else from the socket. `Ok(None)` when the timeout passed;
    /// `Err(UnexpectedEof)` when the node closed the connection.
    pub fn read_frame(&mut self, timeout: Option<Duration>) -> io::Result<Option<Frame>> {
        if let Some(f) = self.pending.pop_front() {
            return Ok(Some(f));
        }
        self.read_wire_frame(timeout)
    }

    /// Next frame from the socket only (never from `pending`).
    fn read_wire_frame(&mut self, timeout: Option<Duration>) -> io::Result<Option<Frame>> {
        self.stream.set_read_timeout(timeout)?;
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            match self.decoder.next_frame() {
                Ok(Some(f)) => return Ok(Some(f)),
                Ok(None) => {}
                Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidData, e.to_string())),
            }
            match self.stream.read(&mut buf) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "node closed the connection",
                    ))
                }
                Ok(n) => self.decoder.push(&buf[..n]),
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    return Ok(None)
                }
                Err(e) => return Err(e),
            }
        }
    }
}

/// Fire-and-forget request sender for a [`Client`] connection (responses are read by the client).
pub struct RequestWriter {
    stream: Arc<Mutex<LocalStream>>,
    next_request: Arc<AtomicU64>,
}

impl RequestWriter {
    pub fn send(&self, cmd: Cmd) -> io::Result<()> {
        let id = self.next_request.fetch_add(1, Ordering::SeqCst);
        let req = v1::CommandRequest {
            request_id: id.to_string(),
            idempotency_key: String::new(),
            command: Some(cmd),
            target_node: String::new(),
        };
        let bytes = Frame {
            stream_id: STREAM_COMMANDS,
            kind: KIND_REQUEST,
            seq: None,
            payload: req.encode_to_vec(),
        }
        .encode(DEFAULT_MAX_PAYLOAD)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "request too large"))?;
        let mut g = self.stream.lock().unwrap_or_else(|e| e.into_inner());
        g.write_all(&bytes)
    }
}

#[cfg(test)]
mod resolve_tests {
    use super::*;

    fn entries(list: &[(&str, &str)]) -> Vec<ProjectEntry> {
        list.iter()
            .map(|(id, name)| ProjectEntry {
                id: (*id).into(),
                name: (*name).into(),
                root: PathBuf::from("/x"),
            })
            .collect()
    }

    #[test]
    fn an_id_beats_a_name_and_a_shared_name_is_refused() {
        let c = entries(&[("a", "b"), ("b", "a"), ("c", "same"), ("d", "same")]);
        assert_eq!(resolve_in(&c, "a").map(|p| p.id.as_str()).ok(), Some("a"));
        assert_eq!(resolve_in(&c, "b").map(|p| p.id.as_str()).ok(), Some("b"));
        assert!(resolve_in(&c, "same").is_err());
        assert!(resolve_in(&c, "nope").is_err());
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod idempotency_tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "git {args:?}");
    }

    /// Whether `sh` (the test agent) can be found on `PATH`, as in `crates/agora/tests/cli.rs`.
    fn sh_on_path() -> bool {
        std::env::var_os("PATH").is_some_and(|p| {
            std::env::split_paths(&p).any(|d| d.join("sh").is_file() || d.join("sh.exe").is_file())
        })
    }

    fn agent_of(r: io::Result<Result<v1::CommandOk, v1::ErrorInfo>>) -> String {
        match r {
            Ok(Ok(v1::CommandOk {
                result: Some(v1::command_ok::Result::AgentStart(s)),
            })) => s.agent_id,
            other => panic!("not a start: {other:?}"),
        }
    }

    #[test]
    fn a_retry_that_overlaps_its_first_attempt_waits_for_its_answer() {
        // Audit F1360: the cache was checked before and filled after the command ran, so a retry on another connection while the first
        // attempt still ran (a client that timed out and reconnected) ran it again. Gated: the supervisor lock holds the first attempt
        // inside `execute` (it has taken its name number, `names`); the retry is sent; then the lock is let go.
        if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
            return;
        }
        // The test agent is `sh` (#972): Git for Windows puts it on PATH only with its `usr\bin`.
        if !sh_on_path() {
            eprintln!("skipped: no `sh` on PATH (on Windows, add Git's usr\\bin to PATH)");
            return;
        }
        let dir = std::env::temp_dir().join(format!("agora-ctl-idem-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let repo = dir.join("proj");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "x").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        let mut sup = Supervisor::new(crate::supervisor::SupervisorConfig {
            node: agora_core::NodeId::new("n"),
            restart: crate::supervisor::RestartPolicy::default(),
            terminal: crate::terminal::TerminalOptions::default(),
            limits: crate::supervisor::NodeLimits::default(),
        });
        sup.register(
            agora_adapters::Manifest::parse(
                "kind = \"shell\"\ndisplay_name = \"S\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"sh\"]\n",
            )
            .unwrap(),
        );
        let ctx = Arc::new(ControlContext::new(
            Arc::new(Mutex::new(sup)),
            vec![ProjectEntry {
                id: "p".into(),
                name: "proj".into(),
                root: repo,
            }],
            dir.join("worktrees"),
            "n".into(),
            None,
            None,
            None,
            Arc::new(crate::chat::ChatBoard::new()),
        ));
        let sock = dir.join("c.sock");
        let server = ControlServer::start(&sock, Arc::clone(&ctx)).unwrap();
        let attempt = |sock: PathBuf| {
            std::thread::spawn(move || {
                let mut c = Client::connect(&sock).unwrap();
                let start = Cmd::AgentStart(v1::AgentStart {
                    kind: "shell".into(),
                    ..Default::default()
                });
                c.call_with_key("k", start)
            })
        };
        let names_reach = |want: u64, limit: Duration| {
            let end = Instant::now() + limit;
            while ctx.names.load(Ordering::SeqCst) < want {
                if Instant::now() >= end {
                    return false;
                }
                std::thread::yield_now();
            }
            true
        };
        let held = ctx.supervisor.lock().unwrap();
        let first = attempt(sock.clone());
        assert!(
            names_reach(1, Duration::from_secs(20)),
            "the first attempt never ran"
        );
        let retry = attempt(sock.clone());
        // Unfixed, the retry reaches `execute` too; fixed, it waits for the first answer and this times out.
        let _ = names_reach(2, Duration::from_secs(3));
        drop(held);
        let (a, b) = (
            agent_of(first.join().unwrap()),
            agent_of(retry.join().unwrap()),
        );
        let started = ctx.sup().list();
        for v in &started {
            let _ = ctx.sup().stop(&v.id);
        }
        drop(server);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(started.len(), 1, "the retry started a second agent");
        assert_eq!(a, b, "both attempts get the one answer");
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod accept_tests {
    use super::*;

    fn server(name: &str) -> (ControlServer, PathBuf) {
        // Short: Unix socket paths are limited to about 100 bytes.
        let dir = std::env::temp_dir().join(format!("agora-ctl-acc-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sup = Supervisor::new(crate::supervisor::SupervisorConfig {
            node: agora_core::NodeId::new("n"),
            restart: crate::supervisor::RestartPolicy::default(),
            terminal: crate::terminal::TerminalOptions::default(),
            limits: crate::supervisor::NodeLimits::default(),
        });
        let ctx = Arc::new(ControlContext::new(
            Arc::new(Mutex::new(sup)),
            Vec::new(),
            dir.join("worktrees"),
            "n".into(),
            None,
            None,
            None,
            Arc::new(crate::chat::ChatBoard::new()),
        ));
        let sock = dir.join("c.sock");
        (ControlServer::start(&sock, ctx).unwrap(), dir)
    }

    fn list(c: &mut Client) {
        let r = c
            .call(Cmd::ProjectList(v1::ProjectList::default()))
            .unwrap();
        assert!(r.is_ok(), "{r:?}");
    }

    fn p95(mut v: Vec<Duration>) -> Duration {
        v.sort();
        v[(v.len() * 95).div_ceil(100) - 1]
    }

    #[test]
    fn a_new_connection_is_served_without_waiting_for_a_poll() {
        // #958: the accept loop slept 100 ms after every empty try, so a command on a new connection (the uplink opens one for each
        // subscription the hub routes) waited 0-100 ms before it was read: p95 about 95 ms. Measured against the same command on a
        // connection already open, so a slow runner moves both and only the accept wait is left in the difference.
        if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
            return;
        }
        let (server, dir) = server("new");
        let sock = server.path().to_path_buf();
        let mut kept = Client::connect(&sock).unwrap();
        list(&mut kept);
        let (mut open, mut new) = (Vec::new(), Vec::new());
        for _ in 0..60 {
            let t = Instant::now();
            list(&mut kept);
            open.push(t.elapsed());
            let t = Instant::now();
            let mut c = Client::connect(&sock).unwrap();
            list(&mut c);
            new.push(t.elapsed());
        }
        let (open, new) = (p95(open), p95(new));
        drop(kept);
        drop(server);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            new < open + Duration::from_millis(40),
            "a new connection p95 {new:?}, an open one p95 {open:?}"
        );
    }

    #[test]
    fn stopping_the_server_ends_its_accept_thread_at_once() {
        if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
            return;
        }
        let mut took = Vec::new();
        for round in 0..10 {
            let (server, dir) = server(&format!("stop{round}"));
            let sock = server.path().to_path_buf();
            list(&mut Client::connect(&sock).unwrap());
            let t = Instant::now();
            drop(server);
            took.push(t.elapsed());
            // The thread has ended and dropped the listener: nothing answers any more.
            assert!(
                Client::connect(&sock).is_err(),
                "a stopped server still accepts"
            );
            let _ = std::fs::remove_dir_all(&dir);
        }
        // Woken rather than waiting out the poll (that took 0-100 ms, about 50 ms in the middle; woken it is under a millisecond
        // here). Generous for a loaded runner.
        let mut sorted = took.clone();
        sorted.sort();
        assert!(
            sorted[sorted.len() / 2] < Duration::from_millis(50),
            "{took:?}"
        );
        assert!(p95(took.clone()) < Duration::from_secs(2), "{took:?}");
    }
}
