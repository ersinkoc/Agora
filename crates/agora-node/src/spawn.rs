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
//! - at most [`MAX_HELPERS`] running helpers per agent, one start in progress or waiting for the user per agent at a time (a slot
//!   reserved from the first check to the end of the start, so concurrent calls cannot pass the limits together),
//!   [`STARTS_PER_MINUTE`] attempts a minute, the node's own agent limit, and a first prompt of at most [`MAX_PROMPT`] bytes with
//!   no control characters, no bidirectional or zero-width characters and no leading `!`, `/` or `#` (shell escapes, slash and
//!   memory commands in common agents);
//! - the first prompt is typed only into a helper whose screen is idle, and Enter only while it is not blocked: a trust or
//!   permission dialog is never answered by it. Unless typed within [`PROMPT_WAIT`] it is left for the user (`agent.blocked`);
//! - `agent_wait` and `agent_output` answer only for helpers the caller started; output is the bottom of the live screen (never
//!   scrollback, ADR detection rule) or the end of a chat agent's transcript, size-limited.
//!
//! Every refusal before asking is an `AgentStartRefused` event (one per minute for rate refusals, agent-written fields cut to
//! [`MAX_FIELD`]), every helper's first prompt is a `MessagePosted` from its parent (the text, or its hash when long), and every
//! request, decision and start is in the event log.
//! Prompts, kinds and folders come from an agent: they are untrusted data and are only shown, never interpreted.

use std::collections::{HashMap, HashSet, VecDeque};
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
use crate::supervisor::{AgentView, ScreenCheck, StartRequest, Supervisor};

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
/// The longest agent-written text (kind, folder) an `AgentStartRefused` event carries.
pub const MAX_FIELD: usize = 256;
/// A prompt up to this size is recorded as it is; a longer one by its hash.
pub const MAX_RECORDED_PROMPT: usize = 4 * 1024;
/// How long a new helper's screen may take to become idle before its first prompt is left for the user.
pub const PROMPT_WAIT: Duration = Duration::from_secs(120);
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
    /// A scoped grant may cover this kind (not a shell or a plain terminal program, see `Supervisor::grantable`).
    grantable: bool,
    ticket: u64,
}

struct Prompt {
    agent: AgentId,
    parent: AgentId,
    kind: String,
    text: String,
    since: Instant,
    typed: Option<Instant>,
    agreed: Agreed,
}

/// What to do with a helper's first prompt now (see [`prompt_step`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Wait,
    Type,
    Enter,
    /// It never became idle, or turned blocked after the text was typed: leave it for the user.
    GiveUp,
    /// The helper is gone.
    Drop,
}

/// What the node knows of a new helper right now, for [`prompt_step`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Seen {
    /// It ended.
    Gone,
    /// A terminal agent: a fresh look at its live screen (`Supervisor::screen_check`).
    Screen(ScreenCheck),
    /// An agent without a terminal (a chat agent), by the state it reported.
    Reported(AgentState),
}

/// Consecutive fresh looks that agreed, for [`prompt_step`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Agreed {
    ready: u8,
    typed: u8,
}

/// Fresh looks in a row that must agree before the node types, and again before it presses Enter.
const LOOKS: u8 = 2;
/// How long typed text may take to show in the box before the node gives up on sending it.
const TYPE_SETTLE: Duration = Duration::from_secs(1);

/// The first prompt is typed only when [`LOOKS`] fresh looks in a row at the screen show the kind's empty input box (`[[ready]]`) and
/// an idle state. Enter follows only when [`LOOKS`] later fresh looks in a row show the box holding exactly the typed text with no
/// dialog or menu on screen (`[[typed]]`, `input_box`); a blocked screen, keys someone else typed, or text that did not arrive within
/// [`TYPE_SETTLE`] leave it for the user. A chat agent gets it when it reports idle. Pure, so the rule is tested without a terminal.
fn prompt_step(seen: Seen, typed: Option<Duration>, waited: Duration, agreed: &mut Agreed) -> Step {
    match (seen, typed) {
        (Seen::Gone, _) => Step::Drop,
        (Seen::Screen(_), Some(after)) if after < ENTER_AFTER => Step::Wait,
        (Seen::Screen(c), Some(after)) => {
            if c.state == Some(AgentState::Blocked) {
                return Step::GiveUp;
            }
            if c.state == Some(AgentState::Idle) && c.shows {
                agreed.typed += 1;
                return if agreed.typed >= LOOKS {
                    Step::Enter
                } else {
                    Step::Wait
                };
            }
            agreed.typed = 0;
            if after < TYPE_SETTLE {
                Step::Wait
            } else {
                Step::GiveUp
            }
        }
        (Seen::Screen(c), None) => {
            if c.state == Some(AgentState::Idle) && c.ready {
                agreed.ready += 1;
                if agreed.ready >= LOOKS {
                    return Step::Type;
                }
                return Step::Wait;
            }
            agreed.ready = 0;
            if waited >= PROMPT_WAIT {
                Step::GiveUp
            } else {
                Step::Wait
            }
        }
        (Seen::Reported(AgentState::Idle), None) => Step::Type,
        (Seen::Reported(_), Some(_)) => Step::Drop,
        (Seen::Reported(_), None) if waited >= PROMPT_WAIT => Step::GiveUp,
        (Seen::Reported(_), None) => Step::Wait,
    }
}

#[derive(Default)]
struct State {
    requests: HashMap<String, Request>,
    tickets: HashMap<u64, Option<StartOutcome>>,
    next_ticket: u64,
    lineage: HashMap<AgentId, Lineage>,
    attempts: HashMap<AgentId, VecDeque<Instant>>,
    /// When a rate refusal of each agent was last recorded.
    rate_noted: HashMap<AgentId, Instant>,
    /// Agents with a start in progress or waiting for the user: the slot is reserved before the limits are checked and kept until
    /// the start ends, so two calls cannot both pass them.
    busy: HashSet<AgentId>,
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

fn invalid(field: &str) -> Error {
    Error::new(ErrorCode::ProtocolInvalidMessage).with("field", field)
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

/// Whether the folder as inspected (the path that is shown, signed and launched) is the asker's folder or below it.
fn placed_inside(inspected: &str, mine: &Path) -> bool {
    within(Path::new(inspected), mine)
}

/// See `agora_core::text::is_hidden`: invisible characters could make the approval card show something other than what would run.
fn invisible(c: char) -> bool {
    agora_core::text::is_hidden(c)
}

/// The prompt rules that need nothing but the text.
fn check_prompt(prompt: &str) -> Result<(), Error> {
    if prompt.len() > MAX_PROMPT {
        return Err(helper_limit("prompt", MAX_PROMPT));
    }
    // Control characters would be keys the user never saw (escape sequences, Enter in the middle, Ctrl-C) when typed into a
    // terminal: only line breaks and tabs are allowed. Invisible characters could make the card lie.
    if prompt
        .chars()
        .any(|c| (c.is_control() && c != '\n' && c != '\t') || invisible(c))
    {
        return Err(invalid("prompt"));
    }
    // Common agents read a leading `!` as a shell command, `/` as one of their own commands, `#` as a memory edit, `&` as a background
    // send, `$` as a skill or shell form and `?` (in an empty box) as the shortcuts key.
    if matches!(
        prompt.trim_start().chars().next(),
        Some('!' | '/' | '#' | '&' | '$' | '?')
    ) {
        return Err(invalid("prompt"));
    }
    // A last word starting with `@` opens a file picker in common agents, which would take the Enter meant to send the prompt.
    if prompt
        .split_whitespace()
        .last()
        .is_some_and(|w| w.starts_with('@'))
    {
        return Err(invalid("prompt"));
    }
    Ok(())
}

/// `s` cut to at most `max` bytes at a character boundary.
fn cut(s: &str, max: usize) -> &str {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// How a prompt is recorded: as it is when small, else by its hash.
fn recorded(prompt: &str) -> String {
    if prompt.len() <= MAX_RECORDED_PROMPT {
        return prompt.to_string();
    }
    let d = ring::digest::digest(&ring::digest::SHA256, prompt.as_bytes());
    let hex: String = d.as_ref().iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

/// The caller's reserved slot: released when dropped (also on a panic) unless handed on to a waiting request.
struct Slot<'a> {
    spawner: &'a Spawner,
    caller: AgentId,
    kept: bool,
}

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        if !self.kept {
            self.spawner.release(&self.caller);
        }
    }
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

    /// Records a refusal (agent-written fields cut short) and hands it back.
    fn refused(&self, caller: &AgentId, kind: &str, folder: &str, e: Error) -> Error {
        if let Some(ev) = &self.events {
            ev.agent_start_refused(
                caller,
                cut(kind, MAX_FIELD),
                cut(folder, MAX_FIELD),
                e.code.as_str(),
            );
        }
        e
    }

    fn release(&self, caller: &AgentId) {
        self.st().busy.remove(caller);
    }

    fn new_ticket(st: &mut State, outcome: Option<StartOutcome>) -> u64 {
        st.next_ticket += 1;
        let t = st.next_ticket;
        st.tickets.insert(t, outcome);
        t
    }

    /// The rate, then the slot. `Ok(())` reserves the caller's slot; `Err((error, record))` says whether to record the refusal.
    fn reserve(&self, caller: &AgentId) -> Result<(), (Error, bool)> {
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
            // One record per minute: a looping agent must not fill the event log with refusals.
            let record = st
                .rate_noted
                .get(caller)
                .is_none_or(|t| now.duration_since(*t) >= Duration::from_secs(60));
            if record {
                st.rate_noted.insert(caller.clone(), now);
            }
            return Err((Error::new(ErrorCode::MeshRateLimited), record));
        }
        seen.push_back(now);
        if !st.busy.insert(caller.clone()) {
            return Err((helper_limit("pending", 1), true));
        }
        Ok(())
    }

    /// Everything decided before the user is asked, with the caller's slot reserved. Returns the request (without its ticket).
    fn check(&self, caller: &AgentId, a: &StartArgs) -> Result<Request, Error> {
        let (views, kinds, confined, trusted, grantable, markers) = {
            let sup = self.sup();
            let p = sup.sandbox_policy();
            (
                sup.list(),
                sup.kinds(),
                p.level != crate::sandbox::Level::Off,
                p.trusted_kinds.clone(),
                sup.grantable(&a.kind),
                sup.typed_markers(&a.kind),
            )
        };
        // The texts the kind's screen must show around typed text (its footer): a prompt that contains one could stand in for it when
        // a menu replaced the real one, so it is refused.
        let lower = a.prompt.to_lowercase();
        if markers
            .iter()
            .any(|m| !m.is_empty() && lower.contains(&m.to_lowercase()))
        {
            return Err(invalid("prompt"));
        }
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
            } else if me.parent.is_some() {
                // The supervisor knows it as a helper but this record of what its decision allowed is gone: it may not ask.
                return Err(helper_limit("depth", usize::from(MAX_DEPTH)));
            }
            // The slot is held, so no other start of this caller can add a helper before this one ends.
            if Self::running_children(&st, &views, caller) >= MAX_HELPERS {
                return Err(helper_limit("helpers", MAX_HELPERS));
            }
            mine.map_or(1, |l| l.depth + 1)
        };
        if !kinds.iter().any(|k| k.kind == a.kind) {
            return Err(
                Error::new(ErrorCode::AgentKindUnknown).with("kind", cut(&a.kind, MAX_FIELD))
            );
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
        let folder_refused =
            |f: &str| Error::new(ErrorCode::MeshFolderNotAllowed).with("folder", cut(f, MAX_FIELD));
        let wanted = match a.folder.as_deref().filter(|f| !f.is_empty()) {
            None => mine.clone(),
            Some(f) if f.chars().any(|c| c.is_control() || invisible(c)) => {
                return Err(folder_refused(f))
            }
            // A relative folder is below the asker's own, never below the node's working folder.
            Some(f) => mine.join(f),
        };
        let folder = agora_platform::canonicalize_plain(&wanted)
            .map_err(|_| folder_refused(&wanted.to_string_lossy()))?;
        // The folder rules of ADR-0052 (refused folders, identity) apply as to any launch.
        let (info, _) = crate::directory::inspect(&folder.to_string_lossy(), &self.node_id, false)?;
        // The resolved path is what is shown, signed and launched: it must be as plain as the one asked for.
        if info.path.chars().any(|c| c.is_control() || invisible(c)) {
            return Err(folder_refused(&info.path));
        }
        if a.worktree && info.git_root.is_empty() {
            return Err(invalid("directory.git_root"));
        }
        // From the one resolution that is shown, signed and launched (`info.path`), never from an earlier one: a folder swapped for a
        // link between two resolutions cannot carry `inside` to somewhere else.
        let inside = placed_inside(&info.path, &mine);
        // A helper below the user's agent may go outside the asker's folder only by a decision naming it; a helper's helper, whose
        // decision the user gave to its parent's chain, stays inside.
        if !inside && depth > 1 {
            return Err(folder_refused(&info.path));
        }
        Ok(Request {
            caller: caller.clone(),
            kind: a.kind.clone(),
            folder: info.path,
            inside,
            worktree: a.worktree,
            prompt: a.prompt.clone(),
            depth,
            grantable,
            ticket: 0,
        })
    }

    /// Running helpers of `caller`, by the supervisor's parent links (which every view carries) or this record.
    fn running_children(st: &State, views: &[AgentView], caller: &AgentId) -> usize {
        views
            .iter()
            .filter(|v| {
                v.running
                    && (v.parent.as_ref() == Some(caller)
                        || st.lineage.get(&v.id).is_some_and(|l| l.parent == *caller))
            })
            .count()
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
            "grant_allowed": r.inside && r.grantable,
            "prompt": r.prompt,
        })
        .to_string()
    }

    /// Starts the helper the user (or a grant) allowed. Called with the caller's slot held; limits are checked again, since things
    /// changed while the user read.
    fn launch(&self, r: &Request, may_start: bool) -> StartOutcome {
        let views = self.sup().list();
        // The asker may have ended while the user read: a helper is never started for an agent that is gone.
        if !views.iter().any(|v| v.id == r.caller && v.running) {
            return Err(not_found(r.caller.as_str()));
        }
        if Self::running_children(&self.st(), &views, &r.caller) >= MAX_HELPERS {
            return Err(helper_limit("helpers", MAX_HELPERS));
        }
        let (info, project) = crate::directory::inspect(&r.folder, &self.node_id, false)?;
        // The folder that was shown is the folder used: a path that now resolves elsewhere is not followed.
        if info.path != r.folder {
            return Err(Error::new(ErrorCode::MeshFolderNotAllowed)
                .with("folder", cut(&r.folder, MAX_FIELD)));
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
        let session = sup.session_of(&id);
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
            // ADR-0060: a session id the node chose at launch.
            if let Some(session) = &session {
                ev.session_reported(&id, session);
            }
            // The helper's task, from its parent, in the log: every start's prompt, granted ones included (which nobody was shown).
            if !r.prompt.is_empty() {
                ev.record_bodies(vec![v1::event::Body::MessagePosted(v1::MessagePosted {
                    message_id: format!("start-{}", id.as_str()),
                    from_agent_id: r.caller.as_str().to_string(),
                    to: id.as_str().to_string(),
                    body: recorded(&r.prompt),
                })]);
            }
        }
        if let Some(w) = &self.git_wake {
            w.store(true, Ordering::Release);
        }
        if !r.prompt.is_empty() {
            self.st().prompts.push(Prompt {
                agent: id.clone(),
                parent: r.caller.clone(),
                kind: r.kind.clone(),
                text: r.prompt.clone(),
                since: Instant::now(),
                typed: None,
                agreed: Agreed::default(),
            });
        }
        Ok(id.as_str().to_string())
    }

    /// The user's verified decision on an `agent.start` request (called after `NodeApprovals::resolve_request` succeeded). An error
    /// means the request was no longer waiting (its agent stopped waiting): nothing starts, and the asker was already told.
    pub fn decided(&self, request: &Pending, r: &v1::ApprovalResolve) -> Result<(), Error> {
        let Some(req) = self.st().requests.remove(&request.id) else {
            return Err(
                Error::new(ErrorCode::MeshStartDenied).with("approval_id", request.id.as_str())
            );
        };
        // The request's slot is released however this ends (a panic included).
        let _slot = Slot {
            spawner: self,
            caller: req.caller.clone(),
            kept: false,
        };
        let (outcome, grant, nested) = if r.approved {
            // A grant and permission to go deeper are honoured only inside the asker's own folder, and a grant never for a shell or
            // a plain terminal program (it would mean "run any N programs without asking").
            let grant = if req.inside && req.grantable {
                r.helper_grant.min(MAX_GRANT)
            } else {
                0
            };
            let nested = req.inside && req.grantable && r.helpers_may_start;
            let outcome = self.launch(&req, nested);
            if outcome.is_ok() {
                (outcome, grant, nested)
            } else {
                // Approved but not started: no grant was given, and the failure is on record.
                (outcome, 0, false)
            }
        } else {
            (
                Err(Error::new(ErrorCode::MeshStartDenied).with("approval_id", request.id.as_str())),
                0,
                false,
            )
        };
        if grant > 1 {
            self.approvals.add_helper_grant(HelperGrant {
                id: request.id.clone(),
                agent: req.caller.clone(),
                kind: req.kind.clone(),
                folder: req.folder.clone(),
                worktree: req.worktree,
                remaining: grant - 1,
                helpers_may_start: nested,
                signer_device_id: r.signer_device_id.clone(),
            });
        }
        if let Some(ev) = &self.events {
            ev.start_resolved(&request.id, r.approved, &r.signer_device_id, grant, nested);
        }
        if let (true, Err(e)) = (r.approved, &outcome) {
            self.refused(&req.caller, &req.kind, &req.folder, e.clone());
        }
        if let Some(t) = self.st().tickets.get_mut(&req.ticket) {
            *t = Some(outcome);
        }
        Ok(())
    }

    /// A request that ran out of time; `false` if it was not a start request.
    pub fn expired(&self, approval_id: &str) -> bool {
        let Some(req) = self.st().requests.remove(approval_id) else {
            return false;
        };
        if let Some(ev) = &self.events {
            ev.approval_resolved(approval_id, false, "");
        }
        let mut st = self.st();
        st.busy.remove(&req.caller);
        if let Some(t) = st.tickets.get_mut(&req.ticket) {
            *t = Some(Err(
                Error::new(ErrorCode::ApprovalExpired).with("approval_id", approval_id)
            ));
        }
        true
    }

    /// The agent exited (a crash the supervisor restarts included) or was stopped: its grants end and a start it was waiting for is
    /// withdrawn. What it is (a helper of whom, how deep, what its decision allowed) is kept: an agent can run again under the same id
    /// (a restart, a resume), and the supervisor, which never forgets an agent while the node runs, keeps its parent link too.
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
            st.busy.remove(agent);
            st.prompts.retain(|p| p.agent != *agent);
            // Old attempt times leave the rate window by themselves; an agent that crashes and restarts keeps its count.
            ids
        };
        for id in withdrawn {
            self.approvals.withdraw(&id);
            if let Some(ev) = &self.events {
                ev.approval_resolved(&id, false, "");
            }
        }
    }

    /// ADR-0060: `helper` was resumed under `parent` after the node restarted. It is `parent`'s helper again (stopped with it, listed
    /// as its own), but the decision that started it is not restored: it may not ask to start helpers, and no grant of its parent's
    /// covers anything until a new decision is signed.
    pub fn resumed_helper(&self, helper: &AgentId, parent: &AgentId) {
        self.st().lineage.insert(
            helper.clone(),
            Lineage {
                parent: parent.clone(),
                depth: MAX_DEPTH,
                may_start: false,
            },
        );
    }

    /// The helpers `agent` started, and theirs, that still run (for "stop its helpers too").
    pub fn running_helpers(&self, agent: &AgentId) -> Vec<AgentId> {
        let views = self.sup().list();
        let st = self.st();
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut todo = vec![agent.clone()];
        while let Some(p) = todo.pop() {
            for (child, l) in &st.lineage {
                if l.parent == p && seen.insert(child.clone()) {
                    todo.push(child.clone());
                    if views.iter().any(|v| v.id == *child && v.running) {
                        out.push(child.clone());
                    }
                }
            }
        }
        out
    }

    /// Types each new helper's first prompt when a fresh look at its screen allows it (see [`prompt_step`]). Called on the node's tick.
    pub fn tick(&self) {
        let mut type_now: Vec<(AgentId, String)> = Vec::new();
        let mut enter_now: Vec<AgentId> = Vec::new();
        let mut gave_up: Vec<(AgentId, String, AgentId)> = Vec::new();
        {
            let mut st = self.st();
            if st.prompts.is_empty() {
                return;
            }
            let sup = self.sup();
            let views = sup.list();
            st.prompts.retain_mut(|p| {
                let view = views.iter().find(|v| v.id == p.agent && v.running);
                let seen = match view {
                    None => Seen::Gone,
                    Some(v) if v.terminal => match sup.screen_check(&p.agent, &p.text) {
                        Some(c) => Seen::Screen(c),
                        None => Seen::Gone,
                    },
                    Some(v) => Seen::Reported(v.state),
                };
                match prompt_step(
                    seen,
                    p.typed.map(|t| t.elapsed()),
                    p.since.elapsed(),
                    &mut p.agreed,
                ) {
                    Step::Wait => true,
                    Step::Drop => false,
                    Step::GiveUp => {
                        gave_up.push((p.parent.clone(), p.kind.clone(), p.agent.clone()));
                        false
                    }
                    Step::Enter => {
                        enter_now.push(p.agent.clone());
                        false
                    }
                    Step::Type => {
                        type_now.push((p.agent.clone(), p.text.clone()));
                        // A terminal gets Enter on a later tick, after another look; a chat agent takes the whole request at once.
                        if matches!(seen, Seen::Screen(_)) {
                            p.typed = Some(Instant::now());
                            return true;
                        }
                        false
                    }
                }
            });
        }
        for (agent, text) in type_now {
            let sup = self.sup();
            match sup.terminal(&agent).and_then(|t| t.input_handle().ok()) {
                Some(control) => {
                    drop(sup);
                    // In a terminal a line break would be Enter mid-prompt: it is typed as a space, so the prompt goes as one request.
                    let _ = control.write_input(text.replace(['\n', '\t'], " ").as_bytes());
                }
                None if sup.adapter_prompt(&agent, &text, false).is_ok() => {
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
        for agent in enter_now {
            let control = self
                .sup()
                .terminal(&agent)
                .and_then(|t| t.input_handle().ok());
            if let Some(control) = control {
                let _ = control.write_input(b"\r");
            }
        }
        // Not typed (or typed but not sent): the user finds it on the helper's screen; the log says why.
        for (parent, kind, helper) in gave_up {
            if let Some(ev) = &self.events {
                ev.agent_start_refused(
                    &parent,
                    cut(&kind, MAX_FIELD),
                    helper.as_str(),
                    ErrorCode::AgentBlocked.as_str(),
                );
            }
        }
    }

    fn helper_of(st: &State, caller: &AgentId, helper: &str) -> Result<AgentId, Error> {
        let id = AgentId::new(helper);
        match st.lineage.get(&id) {
            Some(l) if l.parent == *caller => Ok(id),
            // Another agent's helper, or no agent at all: the same answer, so a caller learns nothing about agents it did not start.
            _ => Err(not_found(cut(helper, MAX_FIELD))),
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
        let refuse = |e: Error| self.refused(caller, &a.kind, &folder_shown, e);
        if a.node
            .as_deref()
            .is_some_and(|n| !n.is_empty() && n != self.node_id)
        {
            // ADR-0057 allows another machine only when the approval names it; starting there is not built yet, so it is refused.
            return Err(refuse(
                Error::new(ErrorCode::ProtocolCapabilityMissing)
                    .with("capability", "mesh.spawn.remote"),
            ));
        }
        if let Err((e, record)) = self.reserve(caller) {
            return Err(if record { refuse(e) } else { e });
        }
        // From here the caller's slot is held: every way out (a panic included) releases it, or hands it to the waiting request.
        let mut slot = Slot {
            spawner: self,
            caller: caller.clone(),
            kept: false,
        };
        let mut req = match check_prompt(&a.prompt).and_then(|()| self.check(caller, a)) {
            Ok(r) => r,
            Err(e) => return Err(refuse(e)),
        };
        // A grant covers this start without asking: same asker, kind, folder and worktree choice, inside its own folder.
        if req.inside && req.grantable {
            if let Some(nested) =
                self.approvals
                    .use_helper_grant(caller, &req.kind, &req.folder, req.worktree)
            {
                let outcome = self.launch(&req, nested);
                if let Err(e) = &outcome {
                    self.refused(caller, &req.kind, &req.folder, e.clone());
                }
                drop(slot);
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
        // The waiting request holds the slot now; its decision, expiry or withdrawal releases it.
        slot.kept = true;
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
            let found = st
                .requests
                .iter()
                .find(|(_, r)| r.ticket == ticket)
                .map(|(id, r)| (id.clone(), r.caller.clone()));
            if let Some((id, caller)) = &found {
                st.requests.remove(id);
                st.busy.remove(caller);
            }
            found.map(|(id, _)| id)
        };
        // Nobody will read the answer: the request leaves the queue and is recorded as withdrawn. A decision that raced it finds no
        // request and is answered with `mesh.start_denied` (nothing starts).
        if let Some(id) = gone {
            self.approvals.withdraw(&id);
            if let Some(ev) = &self.events {
                ev.approval_resolved(&id, false, "");
            }
        }
    }

    fn states(&self, caller: &AgentId, helpers: &[String]) -> Vec<Result<String, Error>> {
        let ids: Vec<Result<AgentId, Error>> = {
            let st = self.st();
            helpers
                .iter()
                .map(|h| Self::helper_of(&st, caller, h))
                .collect()
        };
        // One view of the node for all of them.
        let views = self.sup().list();
        ids.into_iter()
            .map(|id| {
                id.map(|id| {
                    views
                        .iter()
                        .find(|v| v.id == id)
                        .map_or("exited", state_word)
                        .to_string()
                })
            })
            .collect()
    }

    fn output(&self, caller: &AgentId, helper: &str, lines: usize) -> Result<Value, Error> {
        let id = Self::helper_of(&self.st(), caller, helper)?;
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
    fn the_first_prompt_waits_for_an_empty_box_and_never_answers_a_dialog_or_a_menu() {
        let s = Duration::from_secs;
        let screen = |state, ready, shows| {
            Seen::Screen(ScreenCheck {
                state,
                ready,
                shows,
            })
        };
        let idle = Some(AgentState::Idle);
        let step = |seen, typed, waited, a: &mut Agreed| prompt_step(seen, typed, waited, a);
        // Typing needs two looks in a row with a fresh idle state and the empty input box; a menu row an idle rule matches is not one.
        let mut a = Agreed::default();
        assert_eq!(
            step(screen(idle, true, false), None, s(1), &mut a),
            Step::Wait
        );
        assert_eq!(
            step(screen(idle, false, false), None, s(1), &mut a),
            Step::Wait
        );
        assert_eq!(
            step(screen(idle, true, false), None, s(1), &mut a),
            Step::Wait
        );
        assert_eq!(
            step(screen(idle, true, false), None, s(1), &mut a),
            Step::Type
        );
        let mut a = Agreed::default();
        assert_eq!(
            step(
                screen(Some(AgentState::Blocked), false, false),
                None,
                PROMPT_WAIT,
                &mut a
            ),
            Step::GiveUp
        );
        assert_eq!(
            step(screen(None, false, false), None, PROMPT_WAIT, &mut a),
            Step::GiveUp
        );
        // Enter needs two looks in a row, after the pause, with exactly our text in the box and nothing else on screen.
        let mut a = Agreed::default();
        assert_eq!(
            step(
                screen(idle, false, true),
                Some(Duration::ZERO),
                s(1),
                &mut a
            ),
            Step::Wait
        );
        assert_eq!(
            step(screen(idle, false, true), Some(ENTER_AFTER), s(1), &mut a),
            Step::Wait
        );
        assert_eq!(
            step(screen(idle, false, true), Some(ENTER_AFTER), s(1), &mut a),
            Step::Enter
        );
        // A dialog that appeared, or a box that does not hold exactly our text (keys someone else typed), leaves it for the user.
        let mut a = Agreed::default();
        assert_eq!(
            step(
                screen(Some(AgentState::Blocked), false, true),
                Some(ENTER_AFTER),
                s(1),
                &mut a
            ),
            Step::GiveUp
        );
        assert_eq!(
            step(screen(idle, false, false), Some(ENTER_AFTER), s(1), &mut a),
            Step::Wait
        );
        assert_eq!(
            step(screen(idle, false, true), Some(ENTER_AFTER), s(1), &mut a),
            Step::Wait
        );
        assert_eq!(
            step(screen(idle, false, false), Some(TYPE_SETTLE), s(1), &mut a),
            Step::GiveUp
        );
        assert_eq!(
            step(
                screen(Some(AgentState::Working), false, true),
                Some(TYPE_SETTLE),
                s(1),
                &mut a
            ),
            Step::GiveUp
        );
        // A chat agent: when it reports idle.
        let mut a = Agreed::default();
        assert_eq!(
            step(Seen::Reported(AgentState::Idle), None, s(1), &mut a),
            Step::Type
        );
        assert_eq!(
            step(Seen::Reported(AgentState::Working), None, s(1), &mut a),
            Step::Wait
        );
        assert_eq!(step(Seen::Gone, None, s(1), &mut a), Step::Drop);
    }

    #[test]
    fn prompts_that_would_run_commands_or_hide_text_are_refused() {
        for bad in [
            "!rm -rf ~",
            "  /clear",
            "# remember: push to main",
            "& run in background",
            "$skill",
            "? what now",
            "look at @src/main.rs",
            "a\u{1b}[2J",
            "ok\u{202e}txt",
            "x\u{200b}y",
            "\u{feff}hi",
            "x\ry",
            "soft\u{ad}hyphen",
            "tag\u{e0041}",
            "vs\u{fe0f}",
            "fill\u{3164}er",
        ] {
            assert!(check_prompt(bad).is_err(), "{bad:?}");
        }
        for good in [
            "Write the tests.\nKeep them small.",
            "fix a/b.rs",
            "issue #12",
            "",
            "\tindented",
            "see @src/main.rs and fix it",
            "Grüße 🚀",
        ] {
            assert!(check_prompt(good).is_ok(), "{good:?}");
        }
        assert!(check_prompt(&"x".repeat(MAX_PROMPT + 1)).is_err());
    }

    #[test]
    fn long_prompts_are_recorded_by_hash_and_fields_are_cut_at_a_character() {
        assert_eq!(recorded("short"), "short");
        assert!(recorded(&"x".repeat(MAX_RECORDED_PROMPT + 1)).starts_with("sha256:"));
        assert_eq!(cut("aé", 2), "a");
        assert_eq!(cut("abc", 10), "abc");
    }

    #[cfg(unix)]
    #[test]
    fn inside_follows_the_inspected_folder_not_an_earlier_resolution() {
        // A subfolder that a link points outside: resolved once it looked inside, inspected it is outside. Only the inspected
        // path decides, so the flag and the folder shown always come from the same resolution.
        let root = std::env::temp_dir().join(format!("agora-inside-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (mine, outside) = (root.join("mine"), root.join("outside"));
        std::fs::create_dir_all(mine.join("sub")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let mine = agora_platform::canonicalize_plain(&mine).unwrap();
        let before = agora_platform::canonicalize_plain(&mine.join("sub")).unwrap();
        assert!(within(&before, &mine));
        std::fs::remove_dir(mine.join("sub")).unwrap();
        std::os::unix::fs::symlink(&outside, mine.join("sub")).unwrap();
        let (info, _) =
            crate::directory::inspect(&mine.join("sub").to_string_lossy(), "n", false).unwrap();
        assert!(!placed_inside(&info.path, &mine), "{}", info.path);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn within_compares_whole_components() {
        assert!(within(Path::new("/a/b/c"), Path::new("/a/b")));
        assert!(within(Path::new("/a/b"), Path::new("/a/b")));
        assert!(!within(Path::new("/a/bc"), Path::new("/a/b")));
        assert!(!within(Path::new("/a"), Path::new("/a/b")));
    }
}
