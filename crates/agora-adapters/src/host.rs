//! Adapter host (L3, docs/specs/adapter-protocol.md): runs an adapter as a child process and speaks newline-delimited
//! JSON-RPC 2.0 with it over stdio. The adapter is untrusted: it never gets hub credentials, its lines are size
//! limited, anything it sends that is malformed or unknown is ignored (and counted), and a crash shows up as an
//! [`AdapterEvent::Exited`] instead of taking the node down. Agents are data, not code: supporting another agent
//! is another adapter process, with no change here.
//!
//! The host is synchronous (threads and channels), like the rest of the node. One reader thread routes responses to
//! the callers waiting for them and turns adapter notifications into [`AdapterEvent`]s for [`Adapter::poll`].

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agora_core::AgentState;
use agora_protocol::jsonrpc::{parse_line, to_line, Message};
use serde_json::{json, Value};

/// An MCP server the agent should connect to (`McpServer` in `schema/agora/v1/adapter.proto`): started by the agent as `command args`,
/// speaking MCP over its stdio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServer {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
}

/// The longest line accepted from an adapter. Longer lines are discarded whole.
pub const MAX_LINE: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct AdapterConfig {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: Option<PathBuf>,
    /// Confine the adapter, and so the agent it hosts (ADR-0047). Spawning fails where that cannot be done.
    pub sandbox: Option<agora_platform::Sandbox>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatItem {
    Text(String),
    ToolCall {
        tool: String,
        arguments_json: String,
    },
    Plan(Vec<String>),
    Diff {
        path: String,
        unified_diff: String,
    },
    /// ADR-0058: something a person should know (a failed turn, the agent exiting), as a stable code and string parameters.
    /// Untrusted like every item: the node checks the code's shape and bounds the parameters before it keeps one.
    Notice {
        code: String,
        params: Vec<(String, String)>,
    },
    /// ADR-0090 (adapter capability `thought`): the agent's thinking. Untrusted text.
    Thought(String),
    /// ADR-0090 (adapter capability `tool_status`): a tool call or a change to one, matched by its id.
    Tool(ToolItem),
    /// ADR-0090 (adapter capability `plan_status`): the whole plan with each step's status; it replaces the earlier plan.
    PlanSteps(Vec<PlanItem>),
}

/// ADR-0090: a tool call, or an update to one (`chat/append` `toolCall` with a `callId`). A field the adapter left out is `None` (or
/// empty) and keeps what the node holds; a field it sent replaces it, as ACP's `tool_call_update` does. All text is untrusted.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToolItem {
    pub call_id: String,
    pub tool: Option<String>,
    pub arguments_json: Option<String>,
    /// ACP `ToolKind` word (`read`, `edit`, ...); empty when not sent.
    pub kind: String,
    /// ACP `ToolCallStatus` word (`pending`, `in_progress`, `completed`, `failed`); empty when not sent.
    pub status: String,
    pub locations: Option<Vec<(String, u32)>>,
    pub output: Option<String>,
}

/// ADR-0090: one step of a plan.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlanItem {
    pub text: String,
    /// `pending`, `in_progress` or `completed`; empty when not sent.
    pub status: String,
    /// `high`, `medium` or `low`; empty when not sent.
    pub priority: String,
}

/// ADR-0090: one option of the agent's session the user may change (`session/config`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConfigOption {
    pub id: String,
    pub name: String,
    pub description: String,
    /// ACP category word: `mode`, `model`, `model_config`, `thought_level`, or another.
    pub category: String,
    /// A boolean; otherwise a select.
    pub boolean: bool,
    pub current: String,
    pub choices: Vec<SessionOption>,
    /// What the adapter says; the node adds its own judgement (`perm`).
    pub skips_permissions: bool,
}

/// ADR-0090: one slash command the agent offers (`session/commands`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AdapterCommand {
    pub name: String,
    pub description: String,
    pub input_hint: String,
}

/// ADR-0058: one mode or model the agent offers.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionOption {
    pub id: String,
    pub name: String,
    pub description: String,
    pub skips_permissions: bool,
}

/// ADR-0058: the modes and models the agent offers and the ones in use (`session/options`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionOptions {
    pub modes: Vec<SessionOption>,
    pub models: Vec<SessionOption>,
    pub current_mode: String,
    pub current_model: String,
}

/// ADR-0058: which of the two `session/set_option` switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionKind {
    Mode,
    Model,
}

impl OptionKind {
    /// The adapter capability that says the adapter takes `session/set_option` for this kind.
    pub fn capability(self) -> &'static str {
        match self {
            Self::Mode => "set_mode",
            Self::Model => "set_model",
        }
    }

    fn wire(self) -> &'static str {
        match self {
            Self::Mode => "SESSION_OPTION_KIND_MODE",
            Self::Model => "SESSION_OPTION_KIND_MODEL",
        }
    }
}

/// What an adapter tells the node (docs/specs/adapter-protocol.md "Methods", adapter → node).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdapterEvent {
    State {
        session_id: String,
        state: AgentState,
        message: String,
    },
    Chat {
        session_id: String,
        item: ChatItem,
    },
    ApprovalRequest {
        session_id: String,
        approval_id: String,
        action: String,
        detail: String,
        /// ADR-0101: the answers the agent offers (empty: ONCE only). The node reads them only from an adapter that declared
        /// `approval_scopes`. A value this build does not know is dropped.
        scopes: Vec<agora_protocol::v1::ApprovalScope>,
        /// ADR-0101: the agent's own name for its always option and that option's exact id (untrusted; the node cleans them).
        always_label: String,
        always_option_id: String,
    },
    Usage {
        session_id: String,
        model: String,
        input_tokens: u64,
        output_tokens: u64,
        cost_micros: Option<u64>,
        currency: String,
        /// ADR-0058: the context's level now and its size; 0 when not reported.
        context_tokens: u64,
        context_window: u64,
    },
    /// ADR-0058: `session/options`.
    Options {
        session_id: String,
        options: SessionOptions,
    },
    SessionReported {
        session_id: String,
        native_session_id: String,
    },
    Log {
        level: String,
        message: String,
    },
    /// ADR-0090: `session/config`, the options the user may change; replaces the last report.
    Config {
        session_id: String,
        options: Vec<ConfigOption>,
    },
    /// ADR-0090: `session/commands`, the slash commands the agent offers; replaces the last list.
    Commands {
        session_id: String,
        commands: Vec<AdapterCommand>,
    },
    /// The adapter's output ended: it exited or closed stdout. Always the last event.
    Exited,
}

#[derive(Debug)]
pub enum HostError {
    Io(io::Error),
    /// No answer within the timeout.
    Timeout,
    /// The adapter exited (or its output ended) before answering.
    Exited,
    /// ADR-0104 O2: the node let the adapter go (it is stopping; the holder keeps the process) while this call waited, after its line
    /// was written (#1119 item 1: a line never written is answered [`Self::Exited`], and is not written after). The answer is not read
    /// here. Callers count the request as given, never as failed (no prompt taken back, no queued one dropped, no decision recorded as
    /// not delivered).
    Detached,
    /// The adapter answered with a JSON-RPC error; `data` is its `{code, params}` if it sent one.
    Rpc {
        code: i64,
        data: Option<Value>,
    },
    /// The answer did not have the shape the protocol requires.
    BadResponse(&'static str),
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Timeout => write!(f, "the adapter did not answer in time"),
            Self::Exited => write!(f, "the adapter exited"),
            Self::Detached => write!(f, "the node let the adapter go before it answered"),
            Self::Rpc { code, .. } => match self.reason() {
                Some((reason, _)) => write!(f, "the adapter returned error {code} ({reason})"),
                None => write!(f, "the adapter returned error {code}"),
            },
            Self::BadResponse(what) => write!(f, "bad response: {what}"),
        }
    }
}

impl std::error::Error for HostError {}

impl HostError {
    /// The adapter's stable reason for an error answer: `data.code` and `data.params` (docs/specs/adapter-protocol.md, "Turn and
    /// exit reasons"), for example `adapter.agent_exited` with the stage, exit code and stderr tail when the agent behind the
    /// bridge exited before `initialize` was answered.
    pub fn reason(&self) -> Option<(&str, &Value)> {
        let Self::Rpc {
            data: Some(data), ..
        } = self
        else {
            return None;
        };
        let code = data.get("code")?.as_str()?;
        Some((code, data.get("params").unwrap_or(&Value::Null)))
    }
}

impl From<io::Error> for HostError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Result of `initialize`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Initialized {
    pub adapter_name: String,
    pub version: String,
    pub agent_kind: String,
    pub capabilities: Vec<String>,
}

/// Calls waiting for an answer. `closed` stops admitting new calls after output ends or a write fails.
#[derive(Default)]
struct PendingState {
    map: HashMap<u64, Waiter>,
    closed: bool,
    /// ADR-0104 O2: set by [`Adapter::let_go`] before the reader is stopped, so the end of output that follows is the node's own
    /// stop. No call is admitted from then on. When the reader ends, a waiting call whose line was written is answered
    /// [`HostError::Detached`]; one whose line is still queued is answered [`HostError::Exited`] and its line is never written; one whose
    /// line is being written is answered by the writer when that write returns (#1119 item 1).
    detached: bool,
    /// Lines the writer has written to the adapter's input (for a process the holder runs: that the holder took), counted under the lock
    /// the writer already takes after each write (#1150: so a test can tell that a call's line was given, and a let-go answers it
    /// [`HostError::Detached`]).
    written: usize,
}

/// A call waiting for its answer, and how far its line got.
struct Waiter {
    answer: Sender<Result<Value, HostError>>,
    line: LineState,
}

/// ADR-0104 O2 (#1119 item 1): how far a waiting call's line got to the adapter's input (for a process the holder runs: to the holder,
/// which passes it on).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineState {
    Queued,
    Writing,
    Written,
}

impl From<Sender<Result<Value, HostError>>> for Waiter {
    fn from(answer: Sender<Result<Value, HostError>>) -> Self {
        Self {
            answer,
            line: LineState::Queued,
        }
    }
}

type Pending = Arc<Mutex<PendingState>>;

/// Requests that may wait for the writer before a call gives up with [`HostError::Timeout`]: the pipe itself holds the
/// first ones, so the queue only fills when the adapter has stopped reading its input.
const WRITE_QUEUE: usize = 64;

/// ADR-0093: how the process behind an [`Adapter`] is held and ended.
enum Proc {
    /// The node's own child, on pipes the node holds.
    Child(Child),
    /// A process the terminal holder runs (Unix): the node talks to it through the holder, and the holder ends it.
    Held {
        pid: u32,
        control: Arc<dyn agora_platform::HeldControl>,
        /// Ends the reading of its output (an end, a detach).
        stop: Arc<AtomicBool>,
        /// Let go of, not ended: the process keeps running for the next node.
        detached: bool,
        /// Bytes the holder dropped from its output that the node has not been told about yet.
        dropped: Arc<AtomicU64>,
        /// The node has read everything the holder kept (the replay is over): only then is the loss told, so that the notice is
        /// newer than what the replay put in the conversation and a long replay cannot push it out.
        ready: Arc<AtomicBool>,
        /// How the process ended, as `output` last heard it from the holder (#902); `None` while it runs or when it was not heard.
        exit: Arc<Mutex<Option<agora_platform::ExitStatus>>>,
    },
}

/// ADR-0093: the two ends of an adapter process the holder runs, and what ends it.
pub struct HeldIo {
    pub pid: u32,
    pub input: Box<dyn Write + Send>,
    pub output: Box<dyn Read + Send>,
    pub control: Arc<dyn agora_platform::HeldControl>,
    pub stop: Arc<AtomicBool>,
    pub dropped: Arc<AtomicU64>,
    /// Set by `output` once the output the holder kept has been read to its end.
    pub ready: Arc<AtomicBool>,
    /// #902: how the process ended, set by `output` from what the holder says with the output (before the output ends), so the node
    /// knows it without asking the holder again.
    pub exit: Arc<Mutex<Option<agora_platform::ExitStatus>>>,
}

/// How long [`Adapter::detach`] waits for the reader of a held adapter's output to end. A read of the holder waits up to the link's frame
/// time (5 s) when the holder does not answer; a node that stops must not wait that long for each adapter (ADR-0104 O2).
pub const DETACH_WAIT: Duration = Duration::from_secs(2);

pub struct Adapter {
    proc: Proc,
    /// Lines for the adapter's stdin with the id of the call each one belongs to. A thread of its own writes them, so a
    /// caller never blocks on an adapter that stops reading (the call's timeout holds, and the caller's locks are freed).
    writer: Option<SyncSender<(u64, String)>>,
    pending: Pending,
    events: Receiver<AdapterEvent>,
    next_id: Arc<AtomicU64>,
    ignored: Arc<AtomicUsize>,
    /// Events the reader has put in `events` (ADR-0104 O2: so a test can tell that a line was read and is waiting to be taken).
    read: Arc<AtomicUsize>,
    reader: Option<std::thread::JoinHandle<()>>,
}

/// What an [`Adapter`] needs to make a request: see [`Adapter::caller`].
#[derive(Clone)]
pub struct Caller {
    writer: Option<SyncSender<(u64, String)>>,
    pending: Pending,
    next_id: Arc<AtomicU64>,
}

impl Caller {
    /// A request and its answer. Fails with `Exited` if the adapter dies first.
    pub fn call(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, HostError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = channel();
        let mut line = to_line(&Message::Request {
            id: json!(id),
            method: method.to_string(),
            params,
        });
        line.push('\n');
        {
            // Check admission and enqueue under one lock so the writer cannot miss a send
            // while it closes and drains the queue. The send is nonblocking.
            let mut p = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            // ADR-0104 O2: nothing new is sent to an adapter the node is letting go.
            let Some(writer) = self.writer.as_ref().filter(|_| !p.closed && !p.detached) else {
                return Err(HostError::Exited);
            };
            match writer.try_send((id, line)) {
                Ok(()) => {
                    p.map.insert(id, tx.into());
                }
                Err(TrySendError::Full(_)) => return Err(HostError::Timeout),
                Err(TrySendError::Disconnected(_)) => return Err(HostError::Exited),
            }
        }
        let out = match rx.recv_timeout(timeout) {
            Ok(r) => r,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(HostError::Timeout),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(HostError::Exited),
        };
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .map
            .remove(&id);
        out
    }

    pub fn prompt(
        &self,
        session: &str,
        text: &str,
        from_mesh: bool,
        timeout: Duration,
    ) -> Result<(), HostError> {
        self.call(
            "session/prompt",
            json!({"sessionId": session, "text": text,
                   "source": if from_mesh { "INPUT_SOURCE_MESH" } else { "INPUT_SOURCE_USER" }}),
            timeout,
        )
        .map(drop)
    }

    pub fn cancel(&self, session: &str, timeout: Duration) -> Result<(), HostError> {
        self.call("session/cancel", json!({"sessionId": session}), timeout)
            .map(drop)
    }

    /// Delivers a decision the node has already verified (ADR-0020). The adapter never decides for itself.
    pub fn resolve_approval(
        &self,
        session: &str,
        approval_id: &str,
        approved: bool,
        timeout: Duration,
    ) -> Result<(), HostError> {
        self.resolve_approval_scoped(session, approval_id, approved, None, timeout)
    }

    /// ADR-0101: [`Self::resolve_approval`] with the scope the person signed, sent only to an adapter that declared
    /// `approval_scopes` (`None`: the field is left out, as for every adapter before ADR-0101).
    pub fn resolve_approval_scoped(
        &self,
        session: &str,
        approval_id: &str,
        approved: bool,
        scope: Option<agora_protocol::v1::ApprovalScope>,
        timeout: Duration,
    ) -> Result<(), HostError> {
        let mut params =
            json!({"sessionId": session, "approvalId": approval_id, "approved": approved});
        if let Some(scope) = scope {
            params["scope"] = json!(scope_name(scope));
        }
        self.call("approval/resolve", params, timeout).map(drop)
    }

    /// ADR-0058: switches the session to one of the modes or models the adapter reported. The caller checks the adapter declared
    /// [`OptionKind::capability`] first.
    pub fn set_option(
        &self,
        session: &str,
        kind: OptionKind,
        id: &str,
        timeout: Duration,
    ) -> Result<(), HostError> {
        self.call(
            "session/set_option",
            json!({"sessionId": session, "kind": kind.wire(), "id": id}),
            timeout,
        )
        .map(drop)
    }

    /// ADR-0090: sets one of the options the adapter reported in `session/config`. The caller checks the adapter declared the
    /// capability `config`, that the id and value are among what it reported, and the node's skip-permissions policy.
    pub fn set_config(
        &self,
        session: &str,
        config_id: &str,
        value: &str,
        timeout: Duration,
    ) -> Result<(), HostError> {
        self.call(
            "session/set_config",
            json!({"sessionId": session, "configId": config_id, "value": value}),
            timeout,
        )
        .map(drop)
    }
}

/// Reads one line without ever holding more than `MAX_LINE` bytes. `Ok(None)` at end of input; an over-long line
/// comes back as `Ok(Some(None))` after being skipped to its end.
fn read_line_capped<R: BufRead>(r: &mut R) -> io::Result<Option<Option<String>>> {
    let mut buf = Vec::new();
    let mut too_long = false;
    let mut any = false;
    loop {
        let chunk = match r.fill_buf() {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if chunk.is_empty() {
            break;
        }
        any = true;
        let (take, done) = match chunk.iter().position(|b| *b == b'\n') {
            Some(i) => (i + 1, true),
            None => (chunk.len(), false),
        };
        if !too_long {
            let content_len = take - usize::from(done);
            if buf.len() + content_len > MAX_LINE {
                too_long = true;
                buf.clear();
            } else {
                buf.extend_from_slice(&chunk[..content_len]);
            }
        }
        r.consume(take);
        if done {
            break;
        }
    }
    if !any {
        return Ok(None);
    }
    if too_long {
        return Ok(Some(None));
    }
    Ok(Some(
        String::from_utf8(buf)
            .ok()
            .map(|line| line.trim_end().to_string()),
    ))
}

fn s(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// proto3 JSON writes 64-bit integers as strings; accept either.
fn u(v: &Value, key: &str) -> u64 {
    match v.get(key) {
        Some(Value::String(x)) => x.parse().unwrap_or(0),
        Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
        _ => 0,
    }
}

/// ADR-0101: `ApprovalRequestParams.scopes` as proto3 JSON (enum names or numbers). UNSPECIFIED and unknown values are dropped, and
/// each scope is kept once.
fn approval_scopes(v: Option<&Value>) -> Vec<agora_protocol::v1::ApprovalScope> {
    use agora_protocol::v1::ApprovalScope;
    let mut out = Vec::new();
    for x in v.and_then(Value::as_array).into_iter().flatten() {
        let scope = match x {
            Value::String(n) => ApprovalScope::from_str_name(n),
            Value::Number(n) => n
                .as_i64()
                .and_then(|n| i32::try_from(n).ok())
                .and_then(|n| ApprovalScope::try_from(n).ok()),
            _ => None,
        };
        if let Some(scope) = scope.filter(|s| *s != ApprovalScope::Unspecified) {
            if !out.contains(&scope) {
                out.push(scope);
            }
        }
    }
    out
}

/// The proto3 JSON name of a scope for `approval/resolve`.
fn scope_name(scope: agora_protocol::v1::ApprovalScope) -> &'static str {
    scope.as_str_name()
}

/// An `AgentState` as proto3 JSON: the enum name or its number. Anything else is `Unknown`.
fn agent_state(v: Option<&Value>) -> AgentState {
    match v {
        Some(Value::String(n)) => match n.as_str() {
            "AGENT_STATE_WORKING" => AgentState::Working,
            "AGENT_STATE_BLOCKED" => AgentState::Blocked,
            "AGENT_STATE_IDLE" => AgentState::Idle,
            "AGENT_STATE_DONE" => AgentState::Done,
            _ => AgentState::Unknown,
        },
        Some(Value::Number(n)) => match n.as_u64() {
            Some(1) => AgentState::Working,
            Some(2) => AgentState::Blocked,
            Some(3) => AgentState::Idle,
            Some(4) => AgentState::Done,
            _ => AgentState::Unknown,
        },
        _ => AgentState::Unknown,
    }
}

fn options(v: Option<&Value>) -> Vec<SessionOption> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter(|o| o.is_object())
                .map(|o| SessionOption {
                    id: s(o, "id"),
                    name: s(o, "name"),
                    description: s(o, "description"),
                    skips_permissions: o
                        .get("skipsPermissions")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Notification → event. `None` for methods this host does not know: adapters must be free to send more than the
/// node understands (forward compatibility), so that is not an error.
fn to_event(method: &str, p: &Value) -> Option<AdapterEvent> {
    Some(match method {
        "state/update" => AdapterEvent::State {
            session_id: s(p, "sessionId"),
            state: agent_state(p.get("state")),
            message: s(p, "message"),
        },
        "chat/append" => {
            let item = if let Some(t) = p.get("text") {
                ChatItem::Text(s(t, "text"))
            } else if let Some(t) = p.get("thought") {
                ChatItem::Thought(s(t, "text"))
            } else if let Some(t) = p.get("toolCall") {
                match t.get("callId").and_then(Value::as_str) {
                    // ADR-0090: a call with an id is one entry the node updates in place.
                    Some(call_id) if !call_id.is_empty() => {
                        let opt = |k: &str| t.get(k).and_then(Value::as_str).map(str::to_string);
                        ChatItem::Tool(ToolItem {
                            call_id: call_id.to_string(),
                            tool: opt("tool"),
                            arguments_json: opt("argumentsJson"),
                            kind: s(t, "kind"),
                            status: s(t, "status"),
                            locations: t.get("locations").and_then(Value::as_array).map(|a| {
                                a.iter()
                                    .filter_map(|l| {
                                        Some((
                                            l.get("path")?.as_str()?.to_string(),
                                            u32::try_from(u(l, "line")).unwrap_or(0),
                                        ))
                                    })
                                    .collect()
                            }),
                            output: opt("output"),
                        })
                    }
                    _ => ChatItem::ToolCall {
                        tool: s(t, "tool"),
                        arguments_json: s(t, "argumentsJson"),
                    },
                }
            } else if let Some(entries) = p.pointer("/plan/entries").and_then(Value::as_array) {
                ChatItem::PlanSteps(
                    entries
                        .iter()
                        .filter(|e| e.is_object())
                        .map(|e| PlanItem {
                            text: s(e, "text"),
                            status: s(e, "status"),
                            priority: s(e, "priority"),
                        })
                        .collect(),
                )
            } else if let Some(t) = p.get("plan") {
                ChatItem::Plan(
                    t.get("steps")
                        .and_then(Value::as_array)
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default(),
                )
            } else if let Some(t) = p.get("diff") {
                ChatItem::Diff {
                    path: s(t, "path"),
                    unified_diff: s(t, "unifiedDiff"),
                }
            } else {
                let t = p.get("notice")?;
                ChatItem::Notice {
                    code: s(t, "code"),
                    params: t
                        .get("params")
                        .and_then(Value::as_object)
                        .map(|m| {
                            m.iter()
                                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                                .collect()
                        })
                        .unwrap_or_default(),
                }
            };
            AdapterEvent::Chat {
                session_id: s(p, "sessionId"),
                item,
            }
        }
        "approval/request" => AdapterEvent::ApprovalRequest {
            session_id: s(p, "sessionId"),
            approval_id: s(p, "approvalId"),
            action: s(p, "action"),
            detail: s(p, "detail"),
            scopes: approval_scopes(p.get("scopes")),
            always_label: s(p, "alwaysLabel"),
            always_option_id: s(p, "alwaysOptionId"),
        },
        "usage/report" => AdapterEvent::Usage {
            session_id: s(p, "sessionId"),
            model: s(p, "model"),
            input_tokens: u(p, "inputTokens"),
            output_tokens: u(p, "outputTokens"),
            cost_micros: p.get("costMicros").map(|_| u(p, "costMicros")),
            currency: s(p, "currency"),
            context_tokens: u(p, "contextTokens"),
            context_window: u(p, "contextWindow"),
        },
        "session/options" => AdapterEvent::Options {
            session_id: s(p, "sessionId"),
            options: SessionOptions {
                modes: options(p.get("modes")),
                models: options(p.get("models")),
                current_mode: s(p, "currentMode"),
                current_model: s(p, "currentModel"),
            },
        },
        "session/config" => AdapterEvent::Config {
            session_id: s(p, "sessionId"),
            options: p
                .get("options")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter(|o| o.is_object())
                        .map(|o| ConfigOption {
                            id: s(o, "id"),
                            name: s(o, "name"),
                            description: s(o, "description"),
                            category: s(o, "category"),
                            boolean: s(o, "type") == "boolean",
                            current: s(o, "current"),
                            choices: options(o.get("choices")),
                            skips_permissions: o
                                .get("skipsPermissions")
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        },
        "session/commands" => AdapterEvent::Commands {
            session_id: s(p, "sessionId"),
            commands: p
                .get("commands")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter(|c| c.is_object())
                        .map(|c| AdapterCommand {
                            name: s(c, "name"),
                            description: s(c, "description"),
                            input_hint: s(c, "inputHint"),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        },
        "session/reported" => AdapterEvent::SessionReported {
            session_id: s(p, "sessionId"),
            native_session_id: s(p, "nativeSessionId"),
        },
        "log" => AdapterEvent::Log {
            level: s(p, "level"),
            message: s(p, "message"),
        },
        _ => return None,
    })
}

/// Whether an environment variable is passed on to an adapter: `PATH`, the user's home and standard folders, the language and
/// terminal settings and the Windows system folders, never anything that can hold a secret (API keys, tokens, `SSH_AUTH_SOCK`) and never a temporary or runtime folder (a confined
/// agent must not be pointed at the node's own temp folder or at the folder with the node's sockets).
fn inheritable(name: &str, confined: bool) -> bool {
    const NAMES: &[&str] = &[
        "PATH",
        "PATHEXT",
        "HOME",
        "USER",
        "LOGNAME",
        "SHELL",
        "LANG",
        "LANGUAGE",
        "TERM",
        "TZ",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_CACHE_HOME",
        "XDG_STATE_HOME",
        "USERPROFILE",
        "USERNAME",
        "USERDOMAIN",
        "APPDATA",
        "LOCALAPPDATA",
        "PROGRAMDATA",
        "HOMEDRIVE",
        "HOMEPATH",
        "SYSTEMROOT",
        "SYSTEMDRIVE",
        "WINDIR",
        "COMSPEC",
        "COMPUTERNAME",
        "PROGRAMFILES",
        "PROGRAMFILES(X86)",
        "PROGRAMW6432",
        "COMMONPROGRAMFILES",
        "NUMBER_OF_PROCESSORS",
        "PROCESSOR_ARCHITECTURE",
        "OS",
    ];
    let upper = name.to_ascii_uppercase();
    // The temporary folder: an agent that is not confined keeps the user's (without it Windows puts temp files in the profile
    // root); a confined one gets its own private one from the node.
    if !confined && matches!(upper.as_str(), "TMPDIR" | "TEMP" | "TMP") {
        return true;
    }
    NAMES.contains(&upper.as_str()) || upper.starts_with("LC_")
}

impl Adapter {
    /// The adapter process's id (the agent it hosts runs below it).
    pub fn pid(&self) -> u32 {
        match &self.proc {
            Proc::Child(c) => c.id(),
            Proc::Held { pid, .. } => *pid,
        }
    }

    /// #902: how the process ended, once its output has: its exit code, or on Unix the signal that ended it. Returns within `wait`
    /// (it is asked under the node's supervisor lock); `None` when it has not ended by then or its status is not known.
    ///
    /// A process the holder runs is never asked about over the holder link here (a list with its own reply timeout, a release, the
    /// link's lock could take far longer than `wait`): the status is the one the output's reader already heard from the holder with
    /// the output, or unknown. Reading it changes nothing for the end of this value (a later drop kills and collects as before).
    pub fn exit_status(&mut self, wait: Duration) -> Option<agora_platform::ExitStatus> {
        let end = std::time::Instant::now() + wait;
        match &mut self.proc {
            Proc::Child(c) => loop {
                let read = c.try_wait().map(|s| {
                    s.map(|s| agora_platform::ExitStatus {
                        code: s.code(),
                        signal: agora_platform::exit_signal(&s),
                    })
                });
                match read {
                    Ok(Some(status)) => return Some(status),
                    Ok(None) if std::time::Instant::now() < end => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    _ => return None,
                }
            },
            Proc::Held { exit, .. } => exit.lock().ok().and_then(|s| *s),
        }
    }

    /// ADR-0093: whether the process is the holder's.
    pub fn is_held(&self) -> bool {
        matches!(self.proc, Proc::Held { .. })
    }

    /// ADR-0093: bytes the holder dropped from the adapter's output since this was last asked (it kept only the newest 1 MiB while no
    /// node read it). Always 0 for a process the node runs itself.
    pub fn take_dropped(&self) -> u64 {
        match &self.proc {
            Proc::Held { dropped, ready, .. } if ready.load(Ordering::Acquire) => {
                dropped.swap(0, Ordering::AcqRel)
            }
            Proc::Held { .. } => 0,
            Proc::Child(_) => 0,
        }
    }

    /// ADR-0093: lets go of a process the holder runs: it keeps running, for the next node. The node stops reading and writing
    /// (what it had asked for and not yet had is the holder's to keep). `false` for a process the node runs itself, which is left as
    /// it was (it ends with this value, as before). The reader is waited for at most [`DETACH_WAIT`] ([`Self::let_go`] and
    /// [`Self::reader_ended_by`]); what it read before it ended stays to be taken with [`Self::poll`].
    pub fn detach(&mut self) -> bool {
        if !self.let_go() {
            return false;
        }
        self.reader_ended_by(std::time::Instant::now() + DETACH_WAIT);
        true
    }

    /// ADR-0104 O2: the first half of [`Self::detach`], so that a node letting go of many adapters waits for all their readers within one
    /// time: the reader is told to stop and nothing more is written. It still hands over the lines it already had from the holder
    /// (which the holder counts as given), then ends; their events stay to be taken with [`Self::poll`]. `false` for a process the
    /// node runs itself, which is left as it was.
    pub fn let_go(&mut self) -> bool {
        let Proc::Held { stop, detached, .. } = &mut self.proc else {
            return false;
        };
        *detached = true;
        // Before the reader stops: the end of output it then reaches is this let-go, not the adapter's (the waiting calls are
        // answered `Detached`).
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .detached = true;
        stop.store(true, Ordering::Release);
        self.writer = None;
        true
    }

    /// ADR-0104 O2: waits until `until` at most for the reader to end. `true` when it ended (or there was none). A reader still
    /// running then (a holder that does not answer: a read waits up to the link's frame time) is left to end on its own, never
    /// waited for again: a hung read must not keep a node from stopping.
    pub fn reader_ended_by(&mut self, until: std::time::Instant) -> bool {
        let Some(t) = self.reader.take() else {
            return true;
        };
        while !t.is_finished() && std::time::Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        if t.is_finished() {
            let _ = t.join();
            true
        } else {
            false
        }
    }

    /// ADR-0104 O2: how many events the reader has handed over so far (taken or not), so that a test can tell a line was read.
    pub fn events_read(&self) -> usize {
        self.read.load(Ordering::Acquire)
    }

    /// #1150: how many lines the writer has written to the adapter's input so far (for a process the holder runs: that the holder took),
    /// so that a test can tell a call's line was given (a let-go then answers it [`HostError::Detached`], never [`HostError::Exited`]).
    pub fn lines_written(&self) -> usize {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .written
    }

    /// ADR-0093: an adapter whose process the holder runs, over the link the node has to it. `replay` are events the node rebuilt from
    /// what the holder kept (a request still waiting for the person); they come first, before anything the process says from now.
    pub fn attach_held(io: HeldIo, replay: Vec<AdapterEvent>) -> io::Result<Self> {
        let (tx, events) = channel();
        for e in replay {
            let _ = tx.send(e);
        }
        let mut adapter = Self {
            proc: Proc::Held {
                pid: io.pid,
                control: io.control,
                stop: Arc::clone(&io.stop),
                detached: false,
                dropped: io.dropped,
                ready: io.ready,
                exit: io.exit,
            },
            writer: None,
            pending: Arc::default(),
            events,
            next_id: Arc::new(AtomicU64::new(1)),
            ignored: Arc::new(AtomicUsize::new(0)),
            read: Arc::new(AtomicUsize::new(0)),
            reader: None,
        };
        let (writer, lines) = sync_channel(WRITE_QUEUE);
        let pending = Arc::clone(&adapter.pending);
        let input = io.input;
        std::thread::Builder::new()
            .name("agora-adapter-write".into())
            .spawn(move || write_loop(input, &lines, &pending))?;
        adapter.writer = Some(writer);
        let (pending, ignored, read, output) = (
            Arc::clone(&adapter.pending),
            Arc::clone(&adapter.ignored),
            Arc::clone(&adapter.read),
            io.output,
        );
        adapter.reader = Some(
            std::thread::Builder::new()
                .name("agora-adapter-read".into())
                .spawn(move || read_loop(output, &pending, &ignored, &read, &tx))?,
        );
        Ok(adapter)
    }

    /// ADR-0093: the complete environment [`Self::spawn`] gives the process (the node's `inheritable` variables plus the config's), as
    /// text, for a holder that passes on nothing of its own. `None` when a variable is not UTF-8: the link carries text, so the node
    /// then runs the adapter itself, never with a smaller environment.
    pub fn held_env(cfg: &AdapterConfig) -> Option<Vec<(String, String)>> {
        let mut env: Vec<(String, String)> = Vec::new();
        for (name, value) in std::env::vars_os() {
            if name
                .to_str()
                .is_some_and(|n| inheritable(n, cfg.sandbox.is_some()))
            {
                env.push((name.into_string().ok()?, value.into_string().ok()?));
            }
        }
        for (k, v) in &cfg.env {
            env.retain(|(n, _)| n != k);
            env.push((k.clone(), v.clone()));
        }
        Some(env)
    }

    pub fn spawn(cfg: &AdapterConfig) -> io::Result<Self> {
        // A Windows `.cmd` shim (`npx`) is started by its full path; any other name, and every name elsewhere, is unchanged.
        let child_path = cfg
            .env
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("PATH"))
            .map(|(_, v)| std::ffi::OsStr::new(v.as_str()));
        let program = cfg.program.to_str().map_or_else(
            || cfg.program.clone(),
            |p| agora_platform::program_path(p, child_path),
        );
        let mut cmd = Command::new(&program);
        cmd.args(&cfg.args);
        // Confined first: on macOS this rewrites the command, which must happen before the streams are set.
        let git_config = cfg
            .sandbox
            .as_ref()
            .map(|sb| agora_platform::confine_command(&mut cmd, sb))
            .transpose()?
            .flatten();
        // The node has no console: on Windows the adapter (and the agent it starts, which shares its console) would otherwise keep a
        // window of its own open for the agent's whole life.
        agora_platform::no_console_window(&mut cmd);
        // It runs as long as the agent: it must not keep a pipe another thread of the node was making as it started (#1011).
        agora_platform::keep_only_standard_streams(&mut cmd);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // The adapter's own stderr is diagnostics for whoever runs the node; it is not parsed.
            .stderr(Stdio::inherit());
        // An adapter starts with a clean environment plus what the node chose: it never inherits the node's
        // secrets by accident. PATH and the variables that say where the user's own folders are (`inheritable`) are kept, so
        // the agent it wraps finds its own configuration and login the way it does when the user starts it.
        cmd.env_clear();
        for (name, value) in std::env::vars_os() {
            if name
                .to_str()
                .is_some_and(|n| inheritable(n, cfg.sandbox.is_some()))
            {
                cmd.env(name, value);
            }
        }
        cmd.envs(cfg.env.iter().map(|(k, v)| (k, v)));
        if let Some(file) = git_config {
            cmd.envs(agora_platform::git_config_env(&file));
        }
        if let Some(dir) = &cfg.cwd {
            cmd.current_dir(dir);
        }
        Self::attach_child(cmd.spawn()?, |stdout, pending, ignored, read, tx| {
            std::thread::Builder::new()
                .name("agora-adapter-read".into())
                .spawn(move || read_loop(stdout, &pending, &ignored, &read, &tx))
        })
    }

    fn attach_child(
        child: Child,
        start_reader: impl FnOnce(
            ChildStdout,
            Pending,
            Arc<AtomicUsize>,
            Arc<AtomicUsize>,
            Sender<AdapterEvent>,
        ) -> io::Result<std::thread::JoinHandle<()>>,
    ) -> io::Result<Self> {
        let (tx, events) = channel();
        // Own the child before starting workers so early errors use the same cleanup as a normal drop.
        let mut adapter = Self {
            proc: Proc::Child(child),
            writer: None,
            pending: Arc::default(),
            events,
            next_id: Arc::new(AtomicU64::new(1)),
            ignored: Arc::new(AtomicUsize::new(0)),
            read: Arc::new(AtomicUsize::new(0)),
            reader: None,
        };
        let Proc::Child(child) = &mut adapter.proc else {
            return Err(io::Error::other("adapter pipes missing"));
        };
        let (stdin, stdout) = child
            .stdin
            .take()
            .zip(child.stdout.take())
            .ok_or_else(|| io::Error::other("adapter pipes missing"))?;
        let (writer, lines) = sync_channel(WRITE_QUEUE);
        let pending = Arc::clone(&adapter.pending);
        std::thread::Builder::new()
            .name("agora-adapter-write".into())
            .spawn(move || write_loop(stdin, &lines, &pending))?;
        adapter.writer = Some(writer);
        adapter.reader = Some(start_reader(
            stdout,
            Arc::clone(&adapter.pending),
            Arc::clone(&adapter.ignored),
            Arc::clone(&adapter.read),
            tx,
        )?);
        Ok(adapter)
    }

    /// A request and its answer. Fails with `Exited` if the adapter dies first.
    pub fn call(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, HostError> {
        self.caller().call(method, params, timeout)
    }

    /// A handle that makes requests without borrowing the adapter: the node takes one under its supervisor lock, lets the lock go,
    /// and only then waits for the answer, so an adapter that does not answer never stalls anything but its own caller.
    pub fn caller(&self) -> Caller {
        Caller {
            writer: self.writer.clone(),
            pending: Arc::clone(&self.pending),
            next_id: Arc::clone(&self.next_id),
        }
    }

    pub fn initialize(
        &self,
        node_capabilities: &[&str],
        locale: &str,
        timeout: Duration,
    ) -> Result<Initialized, HostError> {
        let r = self.call(
            "initialize",
            json!({"protocolGeneration": 1, "nodeCapabilities": node_capabilities, "locale": locale}),
            timeout,
        )?;
        if !r.is_object() {
            return Err(HostError::BadResponse("initialize result"));
        }
        Ok(Initialized {
            adapter_name: s(&r, "adapterName"),
            version: s(&r, "version"),
            agent_kind: s(&r, "agentKind"),
            capabilities: r
                .get("capabilities")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
        })
    }

    /// Starts a session; returns its id.
    pub fn start_session(
        &self,
        project_root: &str,
        worktree: &str,
        resume_session_id: Option<&str>,
        timeout: Duration,
    ) -> Result<String, HostError> {
        self.start_session_with(project_root, worktree, resume_session_id, &[], timeout)
    }

    /// Starts a session and hands the agent `mcp_servers` (`SessionStartParams.mcp_servers`: the node passes the agent's own mesh
    /// server there); returns its id.
    pub fn start_session_with(
        &self,
        project_root: &str,
        worktree: &str,
        resume_session_id: Option<&str>,
        mcp_servers: &[McpServer],
        timeout: Duration,
    ) -> Result<String, HostError> {
        let mut params = json!({"projectRoot": project_root, "worktree": worktree});
        if !mcp_servers.is_empty() {
            params["mcpServers"] = mcp_servers
                .iter()
                .map(|m| json!({"name": m.name, "command": m.command, "args": m.args}))
                .collect();
        }
        if let Some(r) = resume_session_id {
            params["resumeSessionId"] = json!(r);
        }
        let r = self.call("session/start", params, timeout)?;
        let id = s(&r, "sessionId");
        if id.is_empty() {
            return Err(HostError::BadResponse("session/start without sessionId"));
        }
        Ok(id)
    }

    pub fn prompt(
        &self,
        session: &str,
        text: &str,
        from_mesh: bool,
        timeout: Duration,
    ) -> Result<(), HostError> {
        self.caller().prompt(session, text, from_mesh, timeout)
    }

    pub fn cancel(&self, session: &str, timeout: Duration) -> Result<(), HostError> {
        self.caller().cancel(session, timeout)
    }

    /// Delivers a decision the node has already verified (ADR-0020). The adapter never decides for itself.
    pub fn resolve_approval(
        &self,
        session: &str,
        approval_id: &str,
        approved: bool,
        timeout: Duration,
    ) -> Result<(), HostError> {
        self.caller()
            .resolve_approval(session, approval_id, approved, timeout)
    }

    /// ADR-0101: [`Caller::resolve_approval_scoped`].
    pub fn resolve_approval_scoped(
        &self,
        session: &str,
        approval_id: &str,
        approved: bool,
        scope: Option<agora_protocol::v1::ApprovalScope>,
        timeout: Duration,
    ) -> Result<(), HostError> {
        self.caller()
            .resolve_approval_scoped(session, approval_id, approved, scope, timeout)
    }

    /// ADR-0058: switches the session to one of the modes or models the adapter reported. The caller checks the adapter declared
    /// [`OptionKind::capability`] first.
    pub fn set_option(
        &self,
        session: &str,
        kind: OptionKind,
        id: &str,
        timeout: Duration,
    ) -> Result<(), HostError> {
        self.caller().set_option(session, kind, id, timeout)
    }

    /// ADR-0090: sets one of the options the adapter reported in `session/config`. The caller checks the adapter declared the
    /// capability `config`, that the id and value are among what it reported, and the node's skip-permissions policy.
    pub fn set_config(
        &self,
        session: &str,
        config_id: &str,
        value: &str,
        timeout: Duration,
    ) -> Result<(), HostError> {
        self.caller().set_config(session, config_id, value, timeout)
    }

    pub fn stop_session(
        &self,
        session: &str,
        reason: &str,
        timeout: Duration,
    ) -> Result<(), HostError> {
        self.call(
            "session/stop",
            json!({"sessionId": session, "reason": reason}),
            timeout,
        )
        .map(drop)
    }

    /// Everything the adapter said since the last call, oldest first. Never blocks. After [`Self::detach`] or [`Self::let_go`], the end
    /// of the output that the node's own stop caused is not an `Exited`: the process runs on in the holder. A real end read just before
    /// the let-go is dropped too (a stopping node would only schedule a restart it never makes): the holder still has the ended process,
    /// so the next node does not take it up, the holder lets it go, and the agent is offered for Resume (#1119 item 5, tested in
    /// `hold_node_pipes`).
    pub fn poll(&self) -> Vec<AdapterEvent> {
        let detached = matches!(self.proc, Proc::Held { detached: true, .. });
        std::iter::from_fn(|| self.events.try_recv().ok())
            .filter(|e| !(detached && *e == AdapterEvent::Exited))
            .collect()
    }

    /// Waits up to `timeout` for the next event.
    pub fn next_event(&self, timeout: Duration) -> Option<AdapterEvent> {
        self.events.recv_timeout(timeout).ok()
    }

    /// Lines the adapter sent that were not valid protocol (bad JSON, bad envelope, too long) and were ignored.
    pub fn ignored_lines(&self) -> usize {
        self.ignored.load(Ordering::Relaxed)
    }

    /// Asks the adapter to shut down, then makes sure it is gone.
    pub fn shutdown(mut self, grace: Duration) {
        let _ = self.call("shutdown", Value::Null, grace);
        self.kill_and_reap();
    }

    fn kill_and_reap(&mut self) {
        // Closing stdin lets a well-behaved adapter finish; kill covers the rest. The writer ends once its queue is closed
        // and its pipe is gone; it is not waited for (a descendant still holding the pipe must not hang the caller).
        self.writer = None;
        match &mut self.proc {
            Proc::Child(child) => {
                let _ = child.kill();
                let _ = child.wait();
            }
            Proc::Held {
                control,
                stop,
                detached,
                ..
            } => {
                if !*detached {
                    // Ended in the holder, and collected there (so it forgets the agent and can end with its last).
                    let _ = control.signal(agora_platform::HeldSignal::Kill);
                    let end = std::time::Instant::now() + Duration::from_secs(3);
                    while matches!(control.exit_status(), Ok(None))
                        && std::time::Instant::now() < end
                    {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                }
                stop.store(true, Ordering::Release);
            }
        }
        if let Some(t) = self.reader.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Adapter {
    fn drop(&mut self) {
        self.kill_and_reap();
    }
}

/// ADR-0093 decision 3: what a holder kept of an adapter's output that an earlier node had already taken tells what was still waiting on
/// the person: the approval requests with no sign after them that the agent went on (a state other than `blocked`). Of the rest, only
/// the reports that replace the last are taken ([`replay`], ADR-0104), so a request is asked again, never answered, and nothing shows
/// twice.
pub fn pending_approvals(lines: &[Vec<u8>]) -> Vec<AdapterEvent> {
    replay(lines, "").approvals
}

/// What a node taking up a held adapter reads from the lines an earlier node had already been given (the holder's `seen`), in one pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Replay {
    /// ADR-0093 decision 3: the requests still waiting on the person (see [`pending_approvals`]).
    pub approvals: Vec<AdapterEvent>,
    /// ADR-0104 decision 1: the newest `session/options`, `session/config` and `session/commands` of the session, as the adapter wrote
    /// them (not yet bounded or judged). `None` when the lines hold no report of that kind. Each report replaces the last, so the newest
    /// one is the agent's whatever the earlier node made of it; chat, usage and state append and are not taken.
    pub options: Option<SessionOptions>,
    pub config: Option<Vec<ConfigOption>>,
    pub commands: Option<Vec<AdapterCommand>>,
}

/// Reads `lines` once for [`Replay`]. A report is the session's when it names `session_id` or no session (as an update without one is
/// read live); one that names another session is not taken.
pub fn replay(lines: &[Vec<u8>], session_id: &str) -> Replay {
    let mut out = Replay::default();
    let ours = |sid: &str| sid.is_empty() || sid == session_id;
    for raw in lines {
        let Ok(text) = std::str::from_utf8(raw) else {
            continue;
        };
        let Ok(Message::Notification { method, params }) = parse_line(text) else {
            continue;
        };
        match method.as_str() {
            "approval/request" => {
                if let Some(e @ AdapterEvent::ApprovalRequest { .. }) = to_event(&method, &params) {
                    out.approvals.push(e);
                }
            }
            "state/update" if agent_state(params.get("state")) != AgentState::Blocked => {
                out.approvals.clear();
            }
            "session/options" | "session/config" | "session/commands" => {
                match to_event(&method, &params) {
                    Some(AdapterEvent::Options {
                        session_id,
                        options,
                    }) if ours(&session_id) => out.options = Some(options),
                    Some(AdapterEvent::Config {
                        session_id,
                        options,
                    }) if ours(&session_id) => out.config = Some(options),
                    Some(AdapterEvent::Commands {
                        session_id,
                        commands,
                    }) if ours(&session_id) => out.commands = Some(commands),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    out
}

fn read_loop<R: Read>(
    stdout: R,
    pending: &Pending,
    ignored: &AtomicUsize,
    read: &AtomicUsize,
    events: &Sender<AdapterEvent>,
) {
    let mut r = BufReader::new(stdout);
    while let Ok(Some(line)) = read_line_capped(&mut r) {
        let Some(line) = line else {
            ignored.fetch_add(1, Ordering::Relaxed);
            continue;
        };
        if line.is_empty() {
            continue;
        }
        match parse_line(&line) {
            Ok(Message::Response { id, result }) => deliver(pending, &id, Ok(result)),
            Ok(Message::ErrorResponse { id, code, data }) => {
                deliver(pending, &id, Err(HostError::Rpc { code, data }));
            }
            Ok(Message::Notification { method, params }) => match to_event(&method, &params) {
                Some(e) => {
                    if events.send(e).is_err() {
                        break;
                    }
                    read.fetch_add(1, Ordering::Release);
                }
                None => {
                    ignored.fetch_add(1, Ordering::Relaxed);
                }
            },
            // The method table has no adapter → node request today. Ignoring one (and counting it) is the safe
            // reading of "tolerate unknown methods"; the adapter's own call simply gets no answer.
            Ok(Message::Request { .. }) | Err(_) => {
                ignored.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    // Output ended: fail everyone still waiting, then say so. When the node let the adapter go (ADR-0104 O2), a call whose line was
    // written was given and only its answer will not be read; one whose line was not is not delivered, and the writer now skips it; one
    // whose line is being written is the writer's to answer (#1119 item 1).
    {
        let mut p = pending.lock().unwrap_or_else(|e| e.into_inner());
        p.closed = true;
        if p.detached {
            p.map.retain(|_, w| match w.line {
                LineState::Writing => true,
                LineState::Written => {
                    let _ = w.answer.send(Err(HostError::Detached));
                    false
                }
                LineState::Queued => {
                    let _ = w.answer.send(Err(HostError::Exited));
                    false
                }
            });
        } else {
            p.map.clear();
        }
    }
    let _ = events.send(AdapterEvent::Exited);
}

/// Writes queued requests in order. When the adapter's input is gone, the call whose line failed and every call queued behind
/// it fail at once (as the old in-line write did) instead of waiting for their timeouts.
fn write_loop<W: Write>(mut stdin: W, lines: &Receiver<(u64, String)>, pending: &Pending) {
    while let Ok((id, line)) = lines.recv() {
        // A caller can time out while its line is still queued, and a line still queued when the reader of an adapter the node let go
        // ended was answered as not delivered (ADR-0104 O2, #1119 item 1): neither is written. Never hold this lock during I/O.
        let active = {
            let mut p = pending.lock().unwrap_or_else(|e| e.into_inner());
            p.map
                .get_mut(&id)
                .map(|w| w.line = LineState::Writing)
                .is_some()
        };
        if !active {
            continue;
        }
        let wrote = stdin
            .write_all(line.as_bytes())
            .and_then(|()| stdin.flush());
        if wrote.is_ok() {
            let mut p = pending.lock().unwrap_or_else(|e| e.into_inner());
            p.written += 1;
            if p.closed && p.detached {
                // The reader of an adapter the node let go ended while this line was written: given, its answer is not read.
                if let Some(w) = p.map.remove(&id) {
                    let _ = w.answer.send(Err(HostError::Detached));
                }
            } else if let Some(w) = p.map.get_mut(&id) {
                w.line = LineState::Written;
            }
        }
        if let Err(e) = wrote {
            pending.lock().unwrap_or_else(|e| e.into_inner()).closed = true;
            let err = if e.kind() == io::ErrorKind::BrokenPipe {
                HostError::Exited
            } else {
                HostError::Io(e)
            };
            fail(pending, id, err);
            for (id, _) in lines.try_iter() {
                fail(pending, id, HostError::Exited);
            }
            return;
        }
    }
}

fn fail(pending: &Pending, id: u64, e: HostError) {
    let w = pending
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .map
        .remove(&id);
    if let Some(w) = w {
        let _ = w.answer.send(Err(e));
    }
}

fn deliver(pending: &Pending, id: &Value, r: Result<Value, HostError>) {
    let Some(id) = id.as_u64() else { return };
    let w = pending
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .map
        .remove(&id);
    if let Some(w) = w {
        let _ = w.answer.send(r);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    /// ADR-0101: the offer is read from names or numbers; UNSPECIFIED, unknown values and repeats are dropped.
    #[test]
    fn an_approval_request_carries_the_offer() {
        use agora_protocol::v1::ApprovalScope as S;
        let p = serde_json::json!({"sessionId": "s", "approvalId": "a", "action": "run", "detail": "ls",
            "scopes": ["APPROVAL_SCOPE_ONCE", 2, "APPROVAL_SCOPE_BOGUS", 0, 9, "APPROVAL_SCOPE_ONCE"],
            "alwaysLabel": "Always", "alwaysOptionId": "aa"});
        let Some(super::AdapterEvent::ApprovalRequest {
            scopes,
            always_label,
            always_option_id,
            ..
        }) = super::to_event("approval/request", &p)
        else {
            panic!("no approval request");
        };
        assert_eq!(scopes, vec![S::Once, S::Always]);
        assert_eq!(
            (always_label.as_str(), always_option_id.as_str()),
            ("Always", "aa")
        );
        // An adapter before ADR-0101 sends none of them.
        let old = serde_json::json!({"sessionId": "s", "approvalId": "a", "action": "run", "detail": "ls"});
        let Some(super::AdapterEvent::ApprovalRequest {
            scopes,
            always_label,
            ..
        }) = super::to_event("approval/request", &old)
        else {
            panic!("no approval request");
        };
        assert!(scopes.is_empty() && always_label.is_empty());
    }

    #[test]
    fn an_adapter_gets_the_users_folders_and_never_a_secret() {
        for ok in [
            "PATH",
            "Path",
            "HOME",
            "USERPROFILE",
            "APPDATA",
            "LocalAppData",
            "XDG_CONFIG_HOME",
            "LANG",
            "LC_ALL",
            "SystemRoot",
        ] {
            assert!(super::inheritable(ok, true), "{ok}");
            assert!(super::inheritable(ok, false), "{ok}");
        }
        // ADR-0060: a parent Claude Code session's markers never reach an adapter (it would save no transcript): not in the list.
        for marker in [
            "CLAUDECODE",
            "CLAUDE_CODE_CHILD_SESSION",
            "CLAUDE_CODE_ENTRYPOINT",
            "CLAUDE_CODE_SSE_PORT",
        ] {
            assert!(!super::inheritable(marker, true), "{marker}");
            assert!(!super::inheritable(marker, false), "{marker}");
        }
        for secret in [
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "GITHUB_TOKEN",
            "AWS_SECRET_ACCESS_KEY",
            "SSH_AUTH_SOCK",
            "AGORA_HUB_TOKEN",
            "DATABASE_URL",
            "NPM_TOKEN",
            "HOME_SECRET",
            "LCX",
            "",
            "TMPDIR",
            "TEMP",
            "TMP",
            "XDG_RUNTIME_DIR",
            "HTTPS_PROXY",
        ] {
            assert!(!super::inheritable(secret, true), "{secret}");
            assert!(
                !super::inheritable(secret, false) || ["TMPDIR", "TEMP", "TMP"].contains(&secret),
                "{secret}"
            );
        }
        for temp in ["TMPDIR", "Temp", "TMP"] {
            assert!(super::inheritable(temp, false), "{temp}");
            assert!(
                !super::inheritable(temp, true),
                "{temp}: a confined agent gets the node's private temp, not the user's"
            );
        }
    }

    use super::*;
    use std::io::Cursor;

    /// A holder's control for tests: the process runs, and nothing is asked of it.
    struct Running;

    impl agora_platform::HeldControl for Running {
        fn signal(&self, _: agora_platform::HeldSignal) -> io::Result<()> {
            Ok(())
        }
        fn exit_status(&self) -> io::Result<Option<agora_platform::ExitStatus>> {
            Ok(None)
        }
    }

    fn held(output: Box<dyn Read + Send>, stop: Arc<AtomicBool>) -> Adapter {
        Adapter::attach_held(
            HeldIo {
                pid: 1,
                input: Box::new(io::sink()),
                output,
                control: Arc::new(Running),
                stop,
                dropped: Arc::default(),
                ready: Arc::default(),
                exit: Arc::default(),
            },
            Vec::new(),
        )
        .unwrap()
    }

    /// A held adapter's output as the node's link reads it: `lines` first, then nothing until the stop (then its end).
    struct Lines(Cursor<Vec<u8>>, Arc<AtomicBool>);

    impl Read for Lines {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            loop {
                let n = self.0.read(out)?;
                if n > 0 || self.1.load(Ordering::Acquire) {
                    return Ok(n);
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }

    /// ADR-0104 O2: what the reader handed over before a detach stays to be taken, and the end of the output that the detach caused is
    /// not an `Exited` (the process runs on in the holder).
    #[test]
    fn a_detached_held_adapter_keeps_what_it_read_and_is_not_said_to_have_exited() {
        let stop = Arc::new(AtomicBool::new(false));
        let lines = b"{\"jsonrpc\":\"2.0\",\"method\":\"chat/append\",\"params\":{\"sessionId\":\"s\",\"text\":{\"text\":\"hi\"}}}\n\
                      {\"jsonrpc\":\"2.0\",\"method\":\"state/update\",\"params\":{\"sessionId\":\"s\",\"state\":\"AGENT_STATE_IDLE\"}}\n";
        let mut a = held(
            Box::new(Lines(Cursor::new(lines.to_vec()), Arc::clone(&stop))),
            stop,
        );
        let end = std::time::Instant::now() + Duration::from_secs(10);
        while a.events_read() < 2 {
            assert!(std::time::Instant::now() < end, "the lines were not read");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(a.detach());
        let events = a.poll();
        assert_eq!(events.len(), 2, "{events:?}");
        assert!(matches!(events[0], AdapterEvent::Chat { .. }), "{events:?}");
        assert!(matches!(
            events[1],
            AdapterEvent::State {
                state: AgentState::Idle,
                ..
            }
        ));
        assert!(a.poll().is_empty());
    }

    /// ADR-0104 O2: a call still waiting for its answer when the node lets the adapter go was given (its line is written) and is
    /// answered `Detached`, never `Exited`, which callers read as not delivered.
    #[test]
    fn a_call_waiting_when_the_node_lets_go_is_answered_detached_and_its_line_is_written() {
        #[derive(Clone, Default)]
        struct Input(Arc<Mutex<Vec<u8>>>);
        impl Write for Input {
            fn write(&mut self, b: &[u8]) -> io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(b);
                Ok(b.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let stop = Arc::new(AtomicBool::new(false));
        let input = Input::default();
        let mut a = Adapter::attach_held(
            HeldIo {
                pid: 1,
                input: Box::new(input.clone()),
                output: Box::new(Lines(Cursor::new(Vec::new()), Arc::clone(&stop))),
                control: Arc::new(Running),
                stop,
                dropped: Arc::default(),
                ready: Arc::default(),
                exit: Arc::default(),
            },
            Vec::new(),
        )
        .unwrap();
        let caller = a.caller();
        let waiting = std::thread::spawn(move || {
            caller.call(
                "session/prompt",
                json!({"text": "x"}),
                Duration::from_secs(10),
            )
        });
        let end = std::time::Instant::now() + Duration::from_secs(10);
        while !String::from_utf8_lossy(&input.0.lock().unwrap()).contains("session/prompt") {
            assert!(std::time::Instant::now() < end, "the line was not written");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(a.detach());
        assert!(matches!(waiting.join().unwrap(), Err(HostError::Detached)));
        // A call made after the let-go is not given.
        assert!(matches!(
            a.caller()
                .call("session/cancel", json!({}), Duration::from_secs(1)),
            Err(HostError::Exited)
        ));
    }

    /// The input of a held adapter whose first write waits until the test says how it ends (`true`: written, `false`: the link broke).
    #[derive(Clone)]
    struct Gated {
        written: Arc<Mutex<Vec<u8>>>,
        entered: Arc<Mutex<Option<Sender<()>>>>,
        outcome: Arc<Mutex<Receiver<bool>>>,
    }

    impl Write for Gated {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            if let Some(entered) = self.entered.lock().unwrap().take() {
                let _ = entered.send(());
                if !self.outcome.lock().unwrap().recv().unwrap_or(false) {
                    return Err(io::ErrorKind::BrokenPipe.into());
                }
            }
            self.written.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// A held adapter on a [`Gated`] input: a prompt whose write waits, and a cancel queued behind it. Then the node lets it go and its
    /// reader ends. The adapter, the input's bytes, how to end the waiting write, and the two calls' answers.
    #[allow(clippy::type_complexity)]
    fn let_go_while_writing() -> (
        Adapter,
        Arc<Mutex<Vec<u8>>>,
        Sender<bool>,
        std::thread::JoinHandle<Result<Value, HostError>>,
        std::thread::JoinHandle<Result<Value, HostError>>,
    ) {
        let stop = Arc::new(AtomicBool::new(false));
        let (entered, is_writing) = channel();
        let (end_write, outcome) = channel();
        let input = Gated {
            written: Arc::default(),
            entered: Arc::new(Mutex::new(Some(entered))),
            outcome: Arc::new(Mutex::new(outcome)),
        };
        let mut a = Adapter::attach_held(
            HeldIo {
                pid: 1,
                input: Box::new(input.clone()),
                output: Box::new(Lines(Cursor::new(Vec::new()), Arc::clone(&stop))),
                control: Arc::new(Running),
                stop,
                dropped: Arc::default(),
                ready: Arc::default(),
                exit: Arc::default(),
            },
            Vec::new(),
        )
        .unwrap();
        let call = |method: &'static str| {
            let caller = a.caller();
            std::thread::spawn(move || caller.call(method, json!({}), Duration::from_secs(10)))
        };
        let prompt = call("session/prompt");
        is_writing.recv_timeout(Duration::from_secs(10)).unwrap();
        let cancel = call("session/cancel");
        let end = std::time::Instant::now() + Duration::from_secs(10);
        while a.pending.lock().unwrap().map.len() < 2 {
            assert!(std::time::Instant::now() < end, "the cancel was not queued");
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(a.let_go());
        assert!(a.reader_ended_by(std::time::Instant::now() + DETACH_WAIT));
        (a, input.written, end_write, prompt, cancel)
    }

    /// #1119 item 1: when the reader of an adapter the node let go ends, a call whose line is still queued is answered `Exited` (not
    /// delivered) at once and its line is never written; one whose line is being written is answered when that write returns:
    /// `Detached` (given) once written.
    #[test]
    fn a_line_still_queued_at_the_let_go_is_not_delivered_and_never_written() {
        let (_a, written, end_write, prompt, cancel) = let_go_while_writing();
        assert!(matches!(cancel.join().unwrap(), Err(HostError::Exited)));
        end_write.send(true).unwrap();
        assert!(matches!(prompt.join().unwrap(), Err(HostError::Detached)));
        // The writer goes on to the queued cancel, and skips it.
        std::thread::sleep(Duration::from_millis(100));
        let text = String::from_utf8_lossy(&written.lock().unwrap()).into_owned();
        assert!(text.contains("session/prompt"), "{text}");
        assert!(!text.contains("session/cancel"), "{text}");
    }

    /// #1119 item 1: a call whose line was being written when the reader of an adapter the node let go ended, and whose write then
    /// failed, was not delivered: `Exited`, never `Detached` (which callers record as sent).
    #[test]
    fn a_line_whose_write_fails_after_the_let_go_is_not_delivered() {
        let (_a, written, end_write, prompt, cancel) = let_go_while_writing();
        assert!(matches!(cancel.join().unwrap(), Err(HostError::Exited)));
        end_write.send(false).unwrap();
        assert!(matches!(prompt.join().unwrap(), Err(HostError::Exited)));
        assert!(written.lock().unwrap().is_empty());
    }

    /// ADR-0104 O2: a read of the holder that does not come back (a holder that does not answer) never keeps a detach, and so a
    /// node's stop, waiting longer than [`DETACH_WAIT`].
    #[test]
    fn a_detach_whose_read_hangs_returns_in_bounded_time() {
        struct Hung(std::sync::mpsc::Receiver<()>);
        impl Read for Hung {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                let _ = self.0.recv();
                Ok(0)
            }
        }
        let (release, hung) = std::sync::mpsc::channel();
        let mut a = held(Box::new(Hung(hung)), Arc::new(AtomicBool::new(false)));
        let began = std::time::Instant::now();
        assert!(a.detach());
        let took = began.elapsed();
        assert!(
            took >= DETACH_WAIT && took < DETACH_WAIT + Duration::from_secs(1),
            "{took:?}"
        );
        assert!(a.poll().is_empty());
        drop(a);
        drop(release);
    }

    #[test]
    fn startup_cleanup_child() {
        if std::env::var_os("AGORA_TEST_STARTUP_CLEANUP_CHILD").is_some() {
            // Keep the fake alive independently of its pipes. The timeout bounds a failed test.
            let (_hold, release) = channel::<()>();
            let _ = release.recv_timeout(Duration::from_secs(30));
        }
    }

    #[test]
    fn a_reader_start_error_reaps_the_spawned_adapter() {
        if agora_platform::process_identity(std::process::id())
            .is_err_and(|e| e.kind() == io::ErrorKind::Unsupported)
        {
            return;
        }
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "host::tests::startup_cleanup_child"])
            .env("AGORA_TEST_STARTUP_CLEANUP_CHILD", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let identity = agora_platform::process_identity(child.id()).unwrap();
        let result = Adapter::attach_child(child, |_, _, _, _, _| {
            assert!(agora_platform::process_still_running(&identity));
            Err(io::Error::from(io::ErrorKind::WouldBlock))
        });
        let alive = agora_platform::process_still_running(&identity);
        if alive {
            let _ = agora_platform::kill_process_if_same(&identity);
        }
        assert!(!alive, "the child is still present after startup failed");
        assert!(matches!(result, Err(e) if e.kind() == io::ErrorKind::WouldBlock));
    }

    fn line(method: &str, params: Value) -> Vec<u8> {
        json!({"jsonrpc": "2.0", "method": method, "params": params})
            .to_string()
            .into_bytes()
    }

    /// ADR-0093 decision 3: of the lines an earlier node had, only a request with no sign after it that the agent went on is asked
    /// again; no other line makes a request, and what is not a message is ignored.
    #[test]
    fn only_a_request_the_agent_has_not_moved_past_is_rebuilt_from_the_lines_an_earlier_node_had() {
        let blocked = || {
            line(
                "state/update",
                json!({"sessionId": "s", "state": "AGENT_STATE_BLOCKED"}),
            )
        };
        let ask = |id: &str| {
            line(
                "approval/request",
                json!({"sessionId": "s", "approvalId": id, "action": "run", "detail": "ls"}),
            )
        };
        let working = line(
            "state/update",
            json!({"sessionId": "s", "state": "AGENT_STATE_WORKING"}),
        );
        let chat = line(
            "chat/append",
            json!({"sessionId": "s", "text": {"text": "hello"}}),
        );
        let ids = |lines: &[Vec<u8>]| -> Vec<String> {
            pending_approvals(lines)
                .into_iter()
                .filter_map(|e| match e {
                    AdapterEvent::ApprovalRequest { approval_id, .. } => Some(approval_id),
                    _ => None,
                })
                .collect()
        };
        // Waiting: asked, and nothing since but words.
        assert_eq!(
            ids(&[blocked(), ask("a1"), chat.clone(), b"not json".to_vec()]),
            ["a1"]
        );
        // Two waiting at once are both asked again.
        assert_eq!(
            ids(&[blocked(), ask("a1"), blocked(), ask("a2")]),
            ["a1", "a2"]
        );
        // The agent went on after it: it was answered (or given up), so it is not asked again.
        assert!(ids(&[blocked(), ask("a1"), working.clone()]).is_empty());
        // One answered, a later one waiting.
        assert_eq!(
            ids(&[blocked(), ask("a1"), working, blocked(), ask("a2")]),
            ["a2"]
        );
        // Words are not requests, a request is not read from anything else, and bytes that are not text are skipped.
        assert!(ids(&[chat, vec![0xff, 0xfe]]).is_empty());
        assert!(ids(&[]).is_empty());
    }

    /// ADR-0104 decision 1: of the lines an earlier node had, the newest report of each kind that replaces the last is taken, for the
    /// agent's session only; a kind with no report is left as `None`, and nothing that appends is taken.
    #[test]
    fn the_newest_options_config_and_commands_of_the_session_are_read_from_the_lines_an_earlier_node_had(
    ) {
        let options = |sid: &str, mode: &str| {
            line(
                "session/options",
                json!({"sessionId": sid, "modes": [{"id": "default", "name": "Ask"}, {"id": "yolo", "name": "Yolo"}],
                       "models": [], "currentMode": mode, "currentModel": ""}),
            )
        };
        let config = |sid: &str, current: &str| {
            line(
                "session/config",
                json!({"sessionId": sid, "options": [{"id": "auto_approve", "name": "Auto approve", "type": "boolean",
                       "current": current}]}),
            )
        };
        let chat = line(
            "chat/append",
            json!({"sessionId": "s", "text": {"text": "hello"}}),
        );
        let r = replay(
            &[
                options("s", "default"),
                config("s", "false"),
                chat.clone(),
                b"not json".to_vec(),
                vec![0xff, 0xfe],
                options("s", "yolo"),
                config("s", "true"),
                // Another session's report is not this agent's.
                options("other", "default"),
                config("other", "false"),
            ],
            "s",
        );
        assert_eq!(
            r.options.as_ref().map(|o| o.current_mode.as_str()),
            Some("yolo")
        );
        assert_eq!(r.options.as_ref().map(|o| o.modes.len()), Some(2));
        assert_eq!(
            r.config.as_ref().and_then(|c| c.first()).map(|o| (
                o.id.as_str(),
                o.current.as_str(),
                o.boolean
            )),
            Some(("auto_approve", "true", true))
        );
        // No report of the commands: left to what the node kept.
        assert_eq!(r.commands, None);
        assert!(r.approvals.is_empty());
        // The newest report replaces, even when it is empty; one with no session id is the session's.
        let r = replay(
            &[
                line(
                    "session/commands",
                    json!({"sessionId": "s", "commands": [{"name": "review"}]}),
                ),
                line("session/commands", json!({"commands": []})),
            ],
            "s",
        );
        assert_eq!(r.commands, Some(Vec::new()));
        assert_eq!(r.options, None);
        assert_eq!(r.config, None);
        // Nothing to read, nothing taken.
        assert_eq!(replay(&[chat], "s"), Replay::default());
    }

    #[test]
    fn a_queued_request_is_skipped_after_its_caller_times_out() {
        struct GatedWriter {
            entered: Option<Sender<()>>,
            release: Receiver<()>,
            output: Vec<u8>,
        }

        impl Write for GatedWriter {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if let Some(entered) = self.entered.take() {
                    entered.send(()).unwrap();
                    self.release.recv_timeout(Duration::from_secs(10)).unwrap();
                }
                self.output.write(bytes)
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let pending: Pending = Arc::default();
        let (requests, lines) = channel();
        let (entered, blocked) = channel();
        let (resume, release) = channel();
        let mut responses = Vec::new();
        for (id, line) in [(1, "first\n"), (2, "expired\n"), (3, "last\n")] {
            let (reply, response) = channel();
            pending.lock().unwrap().map.insert(id, reply.into());
            responses.push(response);
            requests.send((id, line.into())).unwrap();
        }
        drop(requests);
        let writer_pending = Arc::clone(&pending);
        let worker = std::thread::spawn(move || {
            let mut writer = GatedWriter {
                entered: Some(entered),
                release,
                output: Vec::new(),
            };
            write_loop(&mut writer, &lines, &writer_pending);
            writer.output
        });
        blocked.recv_timeout(Duration::from_secs(10)).unwrap();
        // Adapter::call removes its pending entry before returning Timeout.
        pending.lock().unwrap().map.remove(&2);
        resume.send(()).unwrap();
        assert_eq!(worker.join().unwrap(), b"first\nlast\n");
    }

    #[test]
    fn a_write_failure_closes_admission_before_writer_retirement() {
        struct BrokenWriter(Sender<()>, Receiver<()>);

        impl Write for BrokenWriter {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::ErrorKind::BrokenPipe.into())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        impl Drop for BrokenWriter {
            fn drop(&mut self) {
                self.0.send(()).unwrap();
                self.1.recv_timeout(Duration::from_secs(10)).unwrap();
            }
        }

        let pending: Pending = Arc::default();
        let (requests, lines) = sync_channel(2);
        let (retiring, drained) = channel();
        let (resume, release) = channel();
        let responses = [1, 2, 3].map(|id| {
            let (reply, response) = channel();
            pending.lock().unwrap().map.insert(id, reply.into());
            if id > 1 {
                requests.try_send((id, format!("{id}\n"))).unwrap();
            }
            response
        });
        let writer_pending = Arc::clone(&pending);
        let worker = std::thread::spawn(move || {
            write_loop(BrokenWriter(retiring, release), &lines, &writer_pending);
        });
        drained.recv_timeout(Duration::from_secs(10)).unwrap();
        // The failed write and queue drain are complete, but the receiver still exists.
        let closed = pending.lock().unwrap().closed;
        resume.send(()).unwrap();
        worker.join().unwrap();
        assert!(closed);
        let [sent, failed, queued] = responses;
        for response in [failed, queued] {
            assert!(matches!(response.try_recv(), Ok(Err(HostError::Exited))));
        }
        deliver(&pending, &json!(1), Ok(json!("answer")));
        assert_eq!(sent.try_recv().unwrap().unwrap(), "answer");
    }

    #[test]
    fn long_lines_are_skipped_whole_and_the_next_line_is_read() {
        let mut data = vec![b'x'; MAX_LINE + 10];
        data.push(b'\n');
        data.extend_from_slice(b"ok\n");
        let mut r = Cursor::new(data);
        assert_eq!(read_line_capped(&mut r).unwrap(), Some(None));
        assert_eq!(read_line_capped(&mut r).unwrap(), Some(Some("ok".into())));
        assert_eq!(read_line_capped(&mut r).unwrap(), None);
    }

    #[test]
    fn a_last_line_without_a_newline_is_still_read() {
        let mut r = Cursor::new(b"abc".to_vec());
        assert_eq!(read_line_capped(&mut r).unwrap(), Some(Some("abc".into())));
    }

    #[test]
    fn an_interrupted_read_keeps_the_final_response() {
        struct InterruptOnce<R>(bool, R);

        impl<R: Read> Read for InterruptOnce<R> {
            fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
                if !std::mem::replace(&mut self.0, true) {
                    return Err(io::ErrorKind::Interrupted.into());
                }
                self.1.read(bytes)
            }
        }

        let pending: Pending = Arc::default();
        let (reply, response) = channel();
        pending.lock().unwrap().map.insert(1, reply.into());
        let (events, output) = channel();
        let head = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":";
        let tail = b"\"done\"}";
        let input = Cursor::new(head).chain(InterruptOnce(false, Cursor::new(tail)));
        read_loop(
            input,
            &pending,
            &AtomicUsize::new(0),
            &AtomicUsize::new(0),
            &events,
        );
        assert_eq!(response.try_recv().unwrap().unwrap(), json!("done"));
        assert_eq!(output.try_recv().unwrap(), AdapterEvent::Exited);
        assert!(output.try_recv().is_err());
    }

    #[test]
    fn the_line_cap_applies_to_the_final_unterminated_line() {
        let mut data = vec![b'x'; MAX_LINE];
        let line = read_line_capped(&mut Cursor::new(data.as_slice())).unwrap();
        assert_eq!(line.unwrap().unwrap().len(), MAX_LINE);
        data.push(b'x');
        assert_eq!(
            read_line_capped(&mut Cursor::new(data.as_slice())).unwrap(),
            Some(None)
        );
    }

    #[test]
    fn invalid_utf8_is_ignored_without_settling_a_request_or_ending_the_stream() {
        let pending: Pending = Arc::default();
        let (answer, response) = channel();
        pending.lock().unwrap().map.insert(1, answer.into());
        let ignored = AtomicUsize::new(0);
        let (events, output) = channel();
        let input = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"text\":\"\xff\"}}\n\
                      {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"text\":\"valid\"}}\n";
        read_loop(
            Cursor::new(input),
            &pending,
            &ignored,
            &AtomicUsize::new(0),
            &events,
        );
        assert_eq!(response.recv().unwrap().unwrap(), json!({"text": "valid"}));
        assert_eq!(ignored.load(Ordering::Relaxed), 1);
        assert_eq!(output.try_recv().unwrap(), AdapterEvent::Exited);
        assert!(output.try_recv().is_err());
    }

    #[test]
    fn invalid_utf8_at_eof_is_discarded_and_a_valid_replacement_character_is_kept() {
        let mut invalid = Cursor::new(b"\xe2\x82".to_vec());
        assert_eq!(read_line_capped(&mut invalid).unwrap(), Some(None));
        assert_eq!(read_line_capped(&mut invalid).unwrap(), None);
        let mut valid = Cursor::new("valid �\r\n".as_bytes());
        assert_eq!(
            read_line_capped(&mut valid).unwrap(),
            Some(Some("valid �".into()))
        );
    }

    #[test]
    fn an_error_answer_carries_its_reason() {
        let e = HostError::Rpc {
            code: -32000,
            data: Some(json!({"code": "adapter.agent_exited", "params": {"stage": "initialize"}})),
        };
        let (code, params) = e.reason().unwrap();
        assert_eq!(
            (code, params["stage"].as_str()),
            ("adapter.agent_exited", Some("initialize"))
        );
        assert!(e.to_string().contains("adapter.agent_exited"));
        let bare = HostError::Rpc {
            code: -32000,
            data: None,
        };
        assert!(bare.reason().is_none());
        assert!(HostError::Exited.reason().is_none());
    }

    #[test]
    fn states_accept_names_numbers_and_garbage() {
        assert_eq!(
            agent_state(Some(&json!("AGENT_STATE_BLOCKED"))),
            AgentState::Blocked
        );
        assert_eq!(agent_state(Some(&json!(1))), AgentState::Working);
        assert_eq!(agent_state(Some(&json!("nope"))), AgentState::Unknown);
        assert_eq!(agent_state(None), AgentState::Unknown);
    }

    #[test]
    fn notifications_become_events_and_unknown_ones_do_not() {
        let e = to_event(
            "usage/report",
            &json!({"sessionId": "s", "model": "m", "inputTokens": "7", "outputTokens": 9, "costMicros": "5", "currency": "USD"}),
        );
        assert_eq!(
            e,
            Some(AdapterEvent::Usage {
                session_id: "s".into(),
                model: "m".into(),
                input_tokens: 7,
                output_tokens: 9,
                cost_micros: Some(5),
                currency: "USD".into(),
                context_tokens: 0,
                context_window: 0,
            })
        );
        let e = to_event(
            "chat/append",
            &json!({"sessionId": "s", "notice": {"code": "adapter.turn_empty", "params": {"a": "1", "b": 2}}}),
        );
        assert_eq!(
            e,
            Some(AdapterEvent::Chat {
                session_id: "s".into(),
                item: ChatItem::Notice {
                    code: "adapter.turn_empty".into(),
                    params: vec![("a".into(), "1".into())]
                }
            })
        );
        let e = to_event(
            "session/options",
            &json!({"sessionId": "s", "modes": [{"id": "yolo", "name": "Yolo", "skipsPermissions": true}, 3],
                    "models": [{"id": "m"}], "currentMode": "yolo", "currentModel": "m"}),
        );
        let Some(AdapterEvent::Options { options, .. }) = e else {
            panic!("{e:?}")
        };
        assert!(options.modes.len() == 1 && options.modes[0].skips_permissions);
        assert_eq!(
            (options.models[0].id.as_str(), options.current_mode.as_str()),
            ("m", "yolo")
        );
        assert!(to_event("future/method", &json!({})).is_none());
        assert!(to_event("chat/append", &json!({"sessionId": "s"})).is_none());
    }
}
