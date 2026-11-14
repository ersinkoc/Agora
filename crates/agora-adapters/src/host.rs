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
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
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

/// Calls waiting for an answer. `closed` is set (under the same lock) when the adapter's output ended, so a call
/// that starts after that fails at once instead of waiting for an answer nobody can send.
#[derive(Default)]
struct PendingState {
    map: HashMap<u64, Sender<Result<Value, HostError>>>,
    closed: bool,
}

type Pending = Arc<Mutex<PendingState>>;

/// Requests that may wait for the writer before a call gives up with [`HostError::Timeout`]: the pipe itself holds the
/// first ones, so the queue only fills when the adapter has stopped reading its input.
const WRITE_QUEUE: usize = 64;

pub struct Adapter {
    child: Child,
    /// Lines for the adapter's stdin with the id of the call each one belongs to. A thread of its own writes them, so a
    /// caller never blocks on an adapter that stops reading (the call's timeout holds, and the caller's locks are freed).
    writer: Option<SyncSender<(u64, String)>>,
    pending: Pending,
    events: Receiver<AdapterEvent>,
    next_id: AtomicU64,
    ignored: Arc<AtomicUsize>,
    reader: Option<std::thread::JoinHandle<()>>,
}

/// Reads one line without ever holding more than `MAX_LINE` bytes. `Ok(None)` at end of input; an over-long line
/// comes back as `Ok(Some(None))` after being skipped to its end.
fn read_line_capped<R: BufRead>(r: &mut R) -> io::Result<Option<Option<String>>> {
    let mut buf = Vec::new();
    let mut too_long = false;
    let mut any = false;
    loop {
        let chunk = r.fill_buf()?;
        if chunk.is_empty() {
            break;
        }
        any = true;
        let (take, done) = match chunk.iter().position(|b| *b == b'\n') {
            Some(i) => (i + 1, true),
            None => (chunk.len(), false),
        };
        if !too_long {
            if buf.len() + take > MAX_LINE + 1 {
                too_long = true;
                buf.clear();
            } else {
                buf.extend_from_slice(&chunk[..take]);
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
    Ok(Some(Some(
        String::from_utf8_lossy(&buf).trim_end().to_string(),
    )))
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
            } else if let Some(t) = p.get("toolCall") {
                ChatItem::ToolCall {
                    tool: s(t, "tool"),
                    arguments_json: s(t, "argumentsJson"),
                }
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
        self.child.id()
    }

    pub fn spawn(cfg: &AdapterConfig) -> io::Result<Self> {
        let mut cmd = Command::new(&cfg.program);
        cmd.args(&cfg.args);
        // Confined first: on macOS this rewrites the command, which must happen before the streams are set.
        if let Some(sb) = &cfg.sandbox {
            agora_platform::confine_command(&mut cmd, sb)?;
        }
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
        if let Some(dir) = &cfg.cwd {
            cmd.current_dir(dir);
        }
        let mut child = cmd.spawn()?;
        let (stdin, stdout) = match (child.stdin.take(), child.stdout.take()) {
            (Some(i), Some(o)) => (i, o),
            _ => {
                let _ = child.kill();
                return Err(io::Error::other("adapter pipes missing"));
            }
        };
        let pending: Pending = Arc::default();
        let (writer, lines) = sync_channel(WRITE_QUEUE);
        let spawned = {
            let pending = Arc::clone(&pending);
            std::thread::Builder::new()
                .name("agora-adapter-write".into())
                .spawn(move || write_loop(stdin, &lines, &pending))
        };
        if let Err(e) = spawned {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
        let ignored = Arc::new(AtomicUsize::new(0));
        let (tx, events) = channel();
        let reader = {
            let (pending, ignored) = (Arc::clone(&pending), Arc::clone(&ignored));
            std::thread::Builder::new()
                .name("agora-adapter-read".into())
                .spawn(move || read_loop(stdout, &pending, &ignored, &tx))?
        };
        Ok(Self {
            child,
            writer: Some(writer),
            pending,
            events,
            next_id: AtomicU64::new(1),
            ignored,
            reader: Some(reader),
        })
    }

    /// A request and its answer. Fails with `Exited` if the adapter dies first.
    pub fn call(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, HostError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = channel();
        {
            let mut p = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            if p.closed {
                return Err(HostError::Exited);
            }
            p.map.insert(id, tx);
        }
        let mut line = to_line(&Message::Request {
            id: json!(id),
            method: method.to_string(),
            params,
        });
        line.push('\n');
        // Queued, never written here: a write failure comes back through `pending` like an answer.
        let refused = match &self.writer {
            Some(w) => match w.try_send((id, line)) {
                Ok(()) => None,
                Err(TrySendError::Full(_)) => Some(HostError::Timeout),
                Err(TrySendError::Disconnected(_)) => Some(HostError::Exited),
            },
            None => Some(HostError::Exited),
        };
        if let Some(e) = refused {
            self.pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .map
                .remove(&id);
            return Err(e);
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
        self.call(
            "approval/resolve",
            json!({"sessionId": session, "approvalId": approval_id, "approved": approved}),
            timeout,
        )
        .map(drop)
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

    /// Everything the adapter said since the last call, oldest first. Never blocks.
    pub fn poll(&self) -> Vec<AdapterEvent> {
        std::iter::from_fn(|| self.events.try_recv().ok()).collect()
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
        let _ = self.child.kill();
        let _ = self.child.wait();
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

fn read_loop<R: Read>(
    stdout: R,
    pending: &Pending,
    ignored: &AtomicUsize,
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
    // Output ended: fail everyone still waiting, then say so.
    {
        let mut p = pending.lock().unwrap_or_else(|e| e.into_inner());
        p.closed = true;
        p.map.clear();
    }
    let _ = events.send(AdapterEvent::Exited);
}

/// Writes queued requests in order. When the adapter's input is gone, the call whose line failed and every call queued behind
/// it fail at once (as the old in-line write did) instead of waiting for their timeouts.
fn write_loop(mut stdin: ChildStdin, lines: &Receiver<(u64, String)>, pending: &Pending) {
    while let Ok((id, line)) = lines.recv() {
        if let Err(e) = stdin
            .write_all(line.as_bytes())
            .and_then(|()| stdin.flush())
        {
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
    let tx = pending
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .map
        .remove(&id);
    if let Some(tx) = tx {
        let _ = tx.send(Err(e));
    }
}

fn deliver(pending: &Pending, id: &Value, r: Result<Value, HostError>) {
    let Some(id) = id.as_u64() else { return };
    let tx = pending
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .map
        .remove(&id);
    if let Some(tx) = tx {
        let _ = tx.send(r);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
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
