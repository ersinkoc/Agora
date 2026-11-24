//! Agent supervision: start, stop, resume, and restart agents on this node.
//!
//! The supervisor owns the runtime of every agent (worktree, terminal actor, detector, restart state) and
//! produces plain [`SupervisorEvent`]s. It never talks to a hub and never waits for one: agents keep running
//! when the hub is unreachable and the caller buffers the events (AGENTS.md, nodes are autonomous). Time is
//! passed in, so restart and detection timing is testable without sleeping.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use crate::approvals::{SkipAsk, SkipKind};
use agora_adapters::host::{
    Adapter, AdapterCommand, AdapterConfig, AdapterEvent, Caller, ConfigOption, HostError,
    McpServer, OptionKind, SessionOption, SessionOptions,
};
use agora_adapters::perm::{self, Category};
use agora_adapters::{Detector, Manifest, Pacer};
use agora_core::{Agent, AgentId, AgentState, IntegrationLevel, NodeId, StateSource, WorktreeId};
use agora_platform::{spawn, ExitStatus, ProcessLimits, PtyConfig};
use agora_protocol::{Error, ErrorCode};
use agora_vt::Size;

use crate::git::{self, GitError};
use crate::holder::{HolderHub, HolderSession};
use crate::holder_link::{Held, HeldPipe, LinkError, Refusal as HolderRefusal};
use crate::launch_options::{LaunchChoice, Refusal, Requested, SkipPermissions};
use crate::terminal::{TerminalHandle, TerminalOptions};

/// Restart policy for agents that die on their own (crash, non-zero exit, signal). A user stop is never
/// restarted, and a clean exit (code 0) is a finished agent, not a crash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestartPolicy {
    pub max_restarts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            max_restarts: 5,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
        }
    }
}

impl RestartPolicy {
    /// Delay before restart number `attempt` (0-based): exponential from `base_delay`, capped.
    pub fn delay(&self, attempt: u32) -> Duration {
        self.base_delay
            .saturating_mul(1u32.checked_shl(attempt.min(30)).unwrap_or(u32::MAX))
            .min(self.max_delay)
    }
}

/// Limits a node enforces on itself (docs/design/resource-limits.md). `None` means no limit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NodeLimits {
    /// Agents running (or waiting for a restart) at once.
    pub max_agents: Option<usize>,
    /// Open terminals. Every agent has one terminal; shell terminals will count here too.
    pub max_terminals: Option<usize>,
    /// OS limits applied to every agent process.
    pub process: ProcessLimits,
}

#[derive(Debug, Clone)]
pub struct SupervisorConfig {
    pub node: NodeId,
    pub restart: RestartPolicy,
    pub terminal: TerminalOptions,
    pub limits: NodeLimits,
}

/// What the supervisor tells the node about an agent that is about to launch, so the node can give it a mesh
/// endpoint first.
#[derive(Debug, Clone)]
pub struct Launching {
    pub id: AgentId,
    pub kind: String,
    pub project: String,
    pub node: String,
    pub worktree: PathBuf,
    /// The confined agent's own identity (ADR-0050 Amendment B on Windows), made before its endpoint: the endpoint must admit exactly
    /// this agent. `None` where agents are not confined that way.
    pub identity: Option<String>,
}

/// Called before an agent's process starts. Returns the endpoint to inject into its MCP configuration, or `None`
/// to start it without one (the mesh is an addition; an agent never fails to start because of it).
pub type LaunchHook = Box<dyn Fn(&Launching) -> Option<agora_adapters::inject::Endpoint> + Send>;

/// Undoes the launch hook for an agent whose start then failed (its mesh endpoint would otherwise stay registered).
pub type AbortHook = Box<dyn Fn(&AgentId) + Send>;

/// What injection adds to every (re)launch of one agent.
#[derive(Debug, Clone, Default)]
struct Extras {
    read: Vec<PathBuf>,
    argv: Vec<String>,
    env: Vec<(String, String)>,
    /// The agent's identity under confinement, kept for every relaunch so the process and its mesh endpoint stay matched.
    identity: Option<String>,
    /// The agent's own mesh socket: the only unix socket outside its private temp folder it may reach when offline.
    socket: Option<PathBuf>,
    /// ADR-0059: the arguments of the launch options chosen at start, put right after the launch (or resume) command at every
    /// (re)launch, before the injected ones.
    options: Vec<String>,
}

/// What to start.
#[derive(Debug, Clone)]
pub struct StartRequest {
    /// Manifest kind, e.g. `claude-code`.
    pub kind: String,
    /// A git checkout of the project.
    pub checkout: PathBuf,
    /// Directory under which the agent's own worktree is created.
    pub worktree_root: PathBuf,
    /// Short name used for the worktree directory and the `agent/<name>` branch.
    pub name: String,
    pub base: Option<String>,
}

/// ADR-0060: an interrupted agent as the node's session ledger holds it, for [`Supervisor::restore`].
#[derive(Debug, Clone)]
pub struct Restore {
    pub id: AgentId,
    pub kind: String,
    pub project: agora_core::ProjectId,
    pub folder: PathBuf,
    /// A selected folder (ADR-0052) rather than a worktree the node made for it.
    pub directory: bool,
    pub branch: String,
    /// The agent's own session to continue; `None` starts a new one.
    pub session: Option<String>,
    /// ADR-0060 Amendment A: this resume is the node's own, of an agent that was working: it carries the kind's one "carry on" message
    /// when the kind's manifest has one (`[resume] continue_argv`).
    pub carry_on: bool,
    /// Where its injected configuration goes when it may not be written into the folder (a directory agent in the user's own folder).
    pub private_config: Option<PathBuf>,
}

/// Something that happened to an agent. Plain data, ready to become protocol events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupervisorEvent {
    Started {
        agent: AgentId,
    },
    /// Review of #1103 (B1): the chat items a resumed session replayed before it answered its start (ACP `session/load`), in order.
    /// The node keeps them only for an agent whose transcript is empty (it holds the conversation already otherwise).
    ReplayedChat {
        agent: AgentId,
        items: Vec<agora_adapters::host::ChatItem>,
    },
    /// Native session retained across a resumed/restarted process; copied only on a lifecycle transition.
    SessionReported {
        agent: AgentId,
        session: String,
    },
    StateChanged {
        agent: AgentId,
        state: AgentState,
    },
    Exited {
        agent: AgentId,
        status: ExitStatus,
    },
    Restarting {
        agent: AgentId,
        attempt: u32,
        after: Duration,
    },
    /// The node stopped restarting the agent. `exit_code` (#902): how its last process ended, as reported (`AgentExited`), when the
    /// node knows it; `None` when it does not (its status could not be read, or the last restart did not start).
    GaveUp {
        agent: AgentId,
        exit_code: Option<i32>,
    },
    Stopped {
        agent: AgentId,
    },
    /// `native` is true when the agent's own session was resumed, false for the layout-only fallback
    /// (a fresh process in the same worktree).
    Resumed {
        agent: AgentId,
        native: bool,
    },
    /// Something an L3 adapter reported that is not a state change: chat items, an approval request, usage, a
    /// log line. The node turns these into events and the approvals queue.
    Adapter {
        agent: AgentId,
        event: AdapterEvent,
    },
    /// An adapter process died. Its agent is `unknown` until it is started again.
    AdapterCrashed {
        agent: AgentId,
    },
    /// ADR-0090, #908: a prompt that waited behind a turn became the turn and is being sent now, before anything the agent says in
    /// answer can arrive. `tag` is what the caller gave when it queued it (the id of its chat entry); [`Self::PromptDequeued`]
    /// follows with the outcome.
    PromptSending {
        agent: AgentId,
        tag: u64,
    },
    /// ADR-0090: a prompt that waited behind a turn was sent (`sent`), or could not be (the adapter refused it). `tag` is what the
    /// caller gave when it queued it (the id of its chat entry).
    PromptDequeued {
        agent: AgentId,
        tag: u64,
        sent: bool,
    },
    /// ADR-0090: queued prompts that will never be sent, because their adapter ended (the user dropped them through
    /// `AgentCancelTurn` is answered to the caller instead).
    PromptsDropped {
        agent: AgentId,
        tags: Vec<u64>,
    },
    /// ADR-0093: the holder kept only the newest 1 MiB of what an adapter wrote while no node read it; `bytes` were dropped.
    OutputDropped {
        agent: AgentId,
        bytes: u64,
    },
}

/// ADR-0093: what a node needs to take up an adapter agent the holder kept, beyond what [`Restore`] says: what the adapter told the
/// node when it started (the node cannot ask again), and the agent's last state.
#[derive(Debug, Clone, Default)]
pub struct Reattach {
    /// The adapter's own session id (`session/start`'s answer).
    pub session_id: String,
    /// The capabilities it declared in `initialize`.
    pub capabilities: Vec<String>,
    /// The state the ledger last saw it in.
    pub state: Option<AgentState>,
    /// #932: what it last reported of its session (modes and models, config options, commands), as the last node recorded it. The
    /// adapter is not initialised again and does not report them again by itself, so they are the agent's until it reports new ones:
    /// a mode or option in use that makes it act without asking keeps it shown so, and checked so (`skips_permissions_now`). A newer
    /// report among the lines the last node had already read replaces its kind on take-up (ADR-0104, `adopt_held_pipe`).
    pub options: SessionOptions,
    pub config: Vec<ConfigOption>,
    pub commands: Vec<AdapterCommand>,
}

/// ADR-0090: the most prompts that wait behind a turn, per agent (`agent.queue_full`).
pub const MAX_PROMPT_QUEUE: usize = 8;

/// ADR-0090: the longest prompt that is queued behind a turn (a longer one is refused with `protocol.invalid_message`, field `text`).
/// A prompt sent at once is not bounded here, as before.
pub const MAX_QUEUED_PROMPT_BYTES: usize = 64 * 1024;

/// How long after a cancelled turn the node waits for the adapter to say it ended before it reads the turn as over (an adapter that
/// ignores `session/cancel` must not hold the agent, and what waits behind its turn, for good).
pub const CANCEL_GRACE: Duration = Duration::from_secs(5);

/// A request to an adapter that is made AFTER the supervisor's lock is let go: [`AdapterCall::run`] waits for the adapter's answer
/// (up to ten seconds), so an adapter that does not answer stalls only its own caller, never the node's tick or another agent's
/// command. Taken under the lock (which checks everything that can be checked without the adapter), run without it.
pub struct AdapterCall {
    caller: Caller,
    session: String,
    op: CallOp,
    generation: u64,
    /// ADR-0101 (Codex review of PR #1007): the run of the adapter process this call was made to (`AdapterRich::epoch`).
    epoch: u64,
}

/// What preparing a switch of a running agent's mode or config option came to (ADR-0090 Amendment A).
pub enum Prepared {
    /// Nothing to ask: make this call after the lock is let go.
    Call(AdapterCall),
    /// The choice makes the agent act without asking and the policy allows it: it is NOT applied; the owner's signed decision is.
    Ask(SkipAsk),
}

enum CallOp {
    Prompt {
        text: String,
        from_mesh: bool,
    },
    Cancel,
    SetConfig {
        id: String,
        value: String,
    },
    SetOption {
        kind: OptionKind,
        value: String,
    },
    /// ADR-0101: `scope` only for an adapter that declared `approval_scopes` (`None`: left out, as before).
    Resolve {
        approval: String,
        approved: bool,
        scope: Option<agora_protocol::v1::ApprovalScope>,
    },
}

impl AdapterCall {
    /// The turn (or restart) this call was made in: what the caller reports back about it names this, so a late report is ignored.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The run of the adapter process this call was made to (a restart, a resume, an exit and a stop each begin another): a result
    /// recorded about the agent's process names this, so one that comes back after the process was replaced is not applied to the
    /// replacement ([`Supervisor::runs`]).
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Makes the request and waits for the answer. A request whose answer the node will not read because it let the adapter go while
    /// waiting (it is stopping, ADR-0104 O2: [`HostError::Detached`]) was given to the adapter and counts as made: the caller records
    /// it as sent, as it would after an answer, and never as failed.
    pub fn run(self) -> Result<(), SupervisorError> {
        const WAIT: Duration = Duration::from_secs(10);
        let given = |r: Result<(), HostError>| match r {
            Err(HostError::Detached) => Ok(()),
            other => other,
        };
        let session = self.session.as_str();
        match &self.op {
            CallOp::Prompt { text, from_mesh } => {
                given(self.caller.prompt(session, text, *from_mesh, WAIT))
                    .map_err(|e| SupervisorError::Prompt(e.to_string()))
            }
            CallOp::Cancel => {
                given(self.caller.cancel(session, WAIT)).map_err(call_error("cancel"))
            }
            CallOp::SetConfig { id, value } => {
                given(self.caller.set_config(session, id, value, WAIT))
                    .map_err(call_error("set_config"))
            }
            CallOp::SetOption { kind, value } => {
                given(self.caller.set_option(session, *kind, value, WAIT))
                    .map_err(call_error("set_option"))
            }
            CallOp::Resolve {
                approval,
                approved,
                scope,
            } => given(
                self.caller
                    .resolve_approval_scoped(session, approval, *approved, *scope, WAIT),
            )
            .map_err(|e| SupervisorError::Approval(e.to_string())),
        }
    }
}

/// What [`Supervisor::adapter_submit`] decided about a prompt.
pub enum Submit {
    /// The agent is free: make this call now (outside the lock). If it fails, tell [`Supervisor::adapter_send_failed`].
    Send(AdapterCall),
    /// A turn runs: it waits, with this many waiting in all (this one included).
    Queued { queue_len: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupervisorError {
    UnknownKind(String),
    DirectoryUnsupported(String),
    /// The adapter of this kind came from a registry publisher that was revoked (ADR-0044): `(kind, publisher)`.
    PublisherRevoked(String, String),
    /// The node confines agents and this one cannot be confined here: `(kind, level)`. Nothing was started.
    SandboxUnavailable(String, &'static str),
    /// The node confines agents and a folder this agent would be given cannot be kept closed (ADR-0050 Amendment B): `(kind, level,
    /// reason)`, the reason a stable token (`broad_write_grant`, `label_failed`). Nothing was started.
    SandboxRefused(String, &'static str, &'static str),
    /// The node confines agents and the selected folder is one they may never be launched in (a root, the home folder, a secret
    /// folder, the node's own folder).
    ProtectedDirectory,
    /// F1012: the selected folder is no longer at the path that was checked (it, or a folder above it, was moved or swapped for a link
    /// between the check and the start). Nothing was started.
    FolderMoved,
    /// Windows: an argument to a `.bat`/`.cmd` agent holds a line break or NUL, which `cmd.exe` cannot carry safely. Nothing was started.
    BatchArgumentRefused,
    NotFound(AgentId),
    /// The agent is still running (resume applies to stopped agents).
    StillRunning(AgentId),
    Git(GitError),
    /// Starting the process failed; the text is for logs only.
    Spawn(String),
    /// #1150: an adapter was not given a prompt (it did not answer in time, it exited, or it answered with an error); the text is for
    /// logs only. Not a failed start: the node's log says a prompt failed, and the caller gets `internal` with the reference
    /// `agent.prompt` (no stable code says this yet).
    Prompt(String),
    /// #1150: an adapter was not given the answer to one of its approval requests (it did not answer in time, it exited, or it
    /// answered with an error); the text is for logs only. The caller gets `internal` with the reference `approval.resolve`.
    Approval(String),
    /// #1150: a cancel or a switch (`call`: `cancel`, `set_config`, `set_option`) failed without one of the adapter's stable reasons;
    /// the text is for logs only. The node's log names the call; the caller gets `internal`, as before.
    AdapterCall(&'static str, String),
    /// RM-19 (#903): the start failed and this machine cannot find the kind's program: `(kind, program)`. Nothing was started.
    ProgramMissing(String, String),
    /// ADR-0058: the adapter answered with one of the stable reasons the node passes on (`adapter.agent_exited`,
    /// `adapter.agent_refused`, `adapter.option_refused`), with its parameters already checked and bounded.
    Adapter {
        code: &'static str,
        params: Vec<(&'static str, String)>,
    },
    /// ADR-0058: the agent has no such switch (a terminal agent, or an adapter that did not declare it). The capability's name.
    OptionUnsupported(&'static str),
    /// ADR-0058: the agent did not offer this id; the field (`mode`, `model`).
    OptionUnknown(&'static str),
    /// ADR-0090: the agent did not report this config option or this value of it (`agent.option_unknown`); the id that is not known.
    ConfigUnknown(String),
    /// ADR-0090: eight prompts already wait behind the agent's turn (`agent.queue_full`).
    QueueFull(usize),
    /// ADR-0059: a launch option the node's policy refuses: `(kind, option, policy)`. Nothing was started.
    OptionForbidden(String, String, &'static str),
    /// ADR-0059: a launch option id that is malformed, unknown, repeated or too many: `(kind, option)`.
    OptionInvalid(String, String),
    /// ADR-0059: an option that skips permissions without the user's signed confirmation: `(kind, option)`.
    OptionUnconfirmed(String, String),
    /// ADR-0066: a setting the kind does not declare (or named twice): `(kind, setting)`.
    SettingUnknown(String, String),
    /// ADR-0066: a value the setting does not take: `(kind, setting)`.
    SettingInvalid(String, String),
    /// #1133: the owner's `agents.registry` policy switches this registry kind off: `(kind, rule)`, the rule `registry_off` or
    /// `registry_denied`. Nothing was started.
    RegistryRefused(String, &'static str),
    /// The node is at one of its limits; `resource` is `agents` or `terminals`.
    ResourceLimit {
        resource: &'static str,
        limit: u64,
    },
}

impl SupervisorError {
    pub fn to_protocol(&self) -> Error {
        match self {
            Self::DirectoryUnsupported(k) => Error::new(ErrorCode::AgentModeUnsupported)
                .with("kind", k.as_str())
                .with("mode", "directory"),
            Self::UnknownKind(k) => {
                Error::new(ErrorCode::AgentKindUnknown).with("kind", k.as_str())
            }
            Self::RegistryRefused(k, rule) => Error::new(ErrorCode::NodePolicyDenied)
                .with("node_id", "")
                .with("rule", *rule)
                .with("kind", k.as_str()),
            Self::PublisherRevoked(k, p) => Error::new(ErrorCode::AgentPublisherRevoked)
                .with("kind", k.as_str())
                .with("publisher", p.as_str()),
            Self::SandboxUnavailable(k, l) => Error::new(ErrorCode::AgentSandboxUnavailable)
                .with("kind", k.as_str())
                .with("level", *l)
                .with("reason", "unsupported"),
            Self::SandboxRefused(k, l, r) => Error::new(ErrorCode::AgentSandboxUnavailable)
                .with("kind", k.as_str())
                .with("level", *l)
                .with("reason", *r),
            Self::ProtectedDirectory => {
                Error::new(ErrorCode::ProtocolInvalidMessage).with("field", "directory.protected")
            }
            Self::FolderMoved => {
                Error::new(ErrorCode::ProtocolInvalidMessage).with("field", "directory.path")
            }
            Self::BatchArgumentRefused => {
                Error::new(ErrorCode::ProtocolInvalidMessage).with("field", "argv")
            }
            Self::NotFound(id) => {
                Error::new(ErrorCode::AgentNotFound).with("agent_id", id.as_str())
            }
            Self::StillRunning(id) => {
                Error::new(ErrorCode::AgentBlocked).with("agent_id", id.as_str())
            }
            Self::Git(e) => e.to_protocol(),
            Self::Spawn(text) => {
                // The client gets only the stable code; the cause (an OS error text, never a secret) goes to the node's stderr, which
                // the supervisor's log keeps, so a start that failed is not silent. `{:?}` escapes control characters.
                eprintln!("agent start failed: {text:?}");
                Error::new(ErrorCode::Internal).with("incident_id", "unassigned")
            }
            Self::Prompt(text) => {
                // As for a failed start: only the reference goes to the caller, the cause to the node's log (`{:?}` escapes it).
                eprintln!("agent prompt failed: {text:?}");
                Error::new(ErrorCode::Internal).with("incident_id", "agent.prompt")
            }
            Self::Approval(text) => {
                eprintln!("agent approval answer failed: {text:?}");
                Error::new(ErrorCode::Internal).with("incident_id", "approval.resolve")
            }
            Self::AdapterCall(call, text) => {
                eprintln!("agent {call} failed: {text:?}");
                Error::new(ErrorCode::Internal).with("incident_id", "unassigned")
            }
            Self::ProgramMissing(k, p) => Error::new(ErrorCode::AgentProgramMissing)
                .with("kind", k.as_str())
                .with("program", p.as_str()),
            Self::Adapter { code, params } => params
                .iter()
                .fold(Error::new(ErrorCode::parse(code)), |e, (k, v)| {
                    e.with(k, v.as_str())
                }),
            Self::OptionUnsupported(c) => {
                Error::new(ErrorCode::ProtocolCapabilityMissing).with("capability", *c)
            }
            Self::OptionUnknown(f) => {
                Error::new(ErrorCode::ProtocolInvalidMessage).with("field", *f)
            }
            Self::ConfigUnknown(id) => {
                Error::new(ErrorCode::AgentOptionUnknown).with("id", id.as_str())
            }
            Self::QueueFull(limit) => Error::new(ErrorCode::AgentQueueFull)
                .with("limit", i64::try_from(*limit).unwrap_or(i64::MAX)),
            Self::OptionInvalid(k, o) => Error::new(ErrorCode::AgentOptionInvalid)
                .with("kind", k.as_str())
                .with("option", o.as_str()),
            Self::OptionUnconfirmed(k, o) => Error::new(ErrorCode::AgentOptionUnconfirmed)
                .with("kind", k.as_str())
                .with("option", o.as_str()),
            Self::SettingUnknown(k, st) => Error::new(ErrorCode::AgentSettingUnknown)
                .with("kind", k.as_str())
                .with("setting", st.as_str()),
            Self::SettingInvalid(k, st) => Error::new(ErrorCode::AgentSettingInvalid)
                .with("kind", k.as_str())
                .with("setting", st.as_str()),
            Self::OptionForbidden(k, o, p) => Error::new(ErrorCode::AgentOptionForbidden)
                .with("kind", k.as_str())
                .with("option", o.as_str())
                .with("policy", *p),
            Self::ResourceLimit { resource, limit } => Error::new(ErrorCode::NodeResourceLimit)
                .with("resource", *resource)
                .with("limit", i64::try_from(*limit).unwrap_or(i64::MAX)),
        }
    }
}

impl std::fmt::Display for SupervisorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_protocol())
    }
}

impl std::error::Error for SupervisorError {}

impl From<GitError> for SupervisorError {
    fn from(e: GitError) -> Self {
        Self::Git(e)
    }
}

/// A read-only view of one agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentView {
    pub directory: bool,
    pub terminal: bool,
    pub level: IntegrationLevel,
    pub id: AgentId,
    pub kind: String,
    pub state: AgentState,
    pub worktree: PathBuf,
    pub branch: String,
    pub project: String,
    pub running: bool,
    pub restarts: u32,
    /// ADR-0057: the agent on this node that started this one through the mesh.
    pub parent: Option<AgentId>,
    /// ADR-0059: the launch options it was started with, and whether one of them skips permissions.
    pub launch_options: Vec<String>,
    pub skips_permissions: bool,
}

/// ADR-0095: whether an agent started with `launch_skips` (a launch option that skips permissions) and with these options in use acts
/// without asking now: the launch option, the current mode when it skips, or a config option in use that skips (a boolean that is on,
/// or a select whose current choice skips).
pub fn skips_now(launch_skips: bool, options: &SessionOptions, config: &[ConfigOption]) -> bool {
    let mode = options
        .modes
        .iter()
        .any(|m| m.id == options.current_mode && m.skips_permissions);
    let config = config.iter().any(|o| {
        if o.boolean {
            o.skips_permissions && o.current == "true"
        } else {
            o.choices
                .iter()
                .any(|c| c.id == o.current && c.skips_permissions)
        }
    });
    launch_skips || mode || config
}

/// ADR-0057: whether `program` is a shell, a wrapper or an interpreter, which runs whatever it is given. Compared by its file name,
/// lower-cased, without `.exe`, `.cmd`, `.bat` or `.com` and without a version (`python3.12` is `python`, `pypy3` is `pypy`).
fn runs_anything(program: &str) -> bool {
    const ANY: &[&str] = &[
        "sh",
        "bash",
        "zsh",
        "fish",
        "dash",
        "ksh",
        "csh",
        "tcsh",
        "nu",
        "pwsh",
        "powershell",
        "cmd",
        "busybox",
        "wsl",
        "env",
        "nice",
        "nohup",
        "setsid",
        "timeout",
        "stdbuf",
        "xargs",
        "sudo",
        "doas",
        "su",
        "ssh",
        "docker",
        "podman",
        "python",
        "pypy",
        "perl",
        "ruby",
        "node",
        "deno",
        "bun",
        "npx",
        "uvx",
        "bunx",
        "php",
        "lua",
    ];
    let base = program
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(program)
        .to_ascii_lowercase();
    let base = [".exe", ".cmd", ".bat", ".com"]
        .iter()
        .find_map(|x| base.strip_suffix(x))
        .unwrap_or(&base);
    let name =
        base.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.' || c == '-' || c == '_');
    ANY.contains(&name) || ANY.contains(&base)
}

/// One startable agent kind, as reported by [`Supervisor::kinds`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KindInfo {
    pub kind: String,
    pub display_name: String,
    pub level: IntegrationLevel,
    pub installed: bool,
    /// The program looked for in the node's `PATH` (or the path the manifest names); empty when the manifest names none. Only the
    /// name: never the node's `PATH` itself.
    pub program: String,
    /// ADR-0059: the kind's `[[launch.option]]`s, in the manifest's order.
    pub launch_options: Vec<agora_adapters::manifest::LaunchOption>,
    /// ADR-0066: the kind's `[[launch.setting]]`s, in the manifest's order.
    pub settings: Vec<agora_adapters::manifest::LaunchSetting>,
}

struct KindEntry {
    manifest: Manifest,
    detector: Detector,
    /// ADR-0057: the manifest's `[[ready]]` rules (an empty input box).
    ready: Detector,
    /// ADR-0057: the manifest's `[[typed]]` rules and `input_box` pattern (typed text in the box, nothing else on screen).
    typed: Detector,
}

/// What a terminal agent's live screen shows right now (ADR-0057), read when asked rather than from the paced detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenCheck {
    /// What the kind's rules detect on the live bottom of the screen now (`None`: no rule matches).
    pub state: Option<AgentState>,
    /// The `[[ready]]` rules match: the input box is empty and waiting.
    pub ready: bool,
    /// The `[[typed]]` rules match and the input box (`input_box`) holds exactly the text asked about and nothing else (whitespace
    /// ignored, so a wrapped line still counts). Keys someone else typed into the box make it false.
    pub shows: bool,
}

/// An agent driven through an L3 adapter process instead of a terminal.
/// A prompt waiting behind a turn (ADR-0090). `tag` is the caller's, the id of its chat entry.
struct Queued {
    text: String,
    tag: u64,
}

/// ADR-0090: the outcomes of sends of queued prompts, `(agent, tag, sent, generation)`, written by the threads that made them.
type SentResults = Arc<Mutex<Vec<(AgentId, u64, bool, u64)>>>;

/// ADR-0090: how long an adapter that reports `idle` but never `working` may take before its `idle` counts as the end of the turn
/// (an `idle` sooner than this after a prompt is the stray one of the turn before).
pub const IDLE_ONLY_GRACE: Duration = Duration::from_secs(3);

/// ADR-0090: whether an adapter agent that reports its turns has one open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Turn {
    #[default]
    Idle,
    /// A prompt was sent (or the adapter began a turn by itself) at `since`. `seen_working`: the adapter has said `working` or
    /// `blocked` since, so an `idle` now ends this turn. `idle_seen`: it said `idle` without that (a stray one, or an adapter that
    /// never says `working`): it counts as the end once [`IDLE_ONLY_GRACE`] has passed. `cancel_by`: the turn was cancelled and is
    /// read as over at that time if the adapter has not said so.
    Busy {
        since: Instant,
        seen_working: bool,
        idle_seen: bool,
        cancel_by: Option<Instant>,
    },
}

/// What [`AdapterRich::route`] decided. The number is the turn's generation.
#[derive(Debug, PartialEq, Eq)]
enum Route {
    Send(u64),
    Queued(usize),
}

/// A prompt that became the turn, with the generation of that turn.
type Next = (Queued, u64);

#[derive(Default)]
struct AdapterRich {
    config: Vec<ConfigOption>,
    commands: Vec<AdapterCommand>,
    turn: Turn,
    /// Counts the turns and the restarts of this adapter. Everything that was started in one turn (a send, a cancel) carries the
    /// generation it began in and is ignored when it comes back to another: a late answer cannot touch a newer turn.
    generation: u64,
    /// ADR-0090 Amendment A: counts the runs of this adapter (a restart, a resume): a switch the user approved for one run is never
    /// applied to another.
    epoch: u64,
    queue: VecDeque<Queued>,
}

impl AdapterRich {
    fn busy(&mut self, now: Instant, seen_working: bool) {
        self.generation += 1;
        self.turn = Turn::Busy {
            since: now,
            seen_working,
            idle_seen: false,
            cancel_by: None,
        };
    }

    /// A user's prompt: sent now (the turn becomes busy), or queued behind the turn (also while others wait, to keep their order).
    /// `turns`: the adapter reports its turns; one that does not is never queued for. `tag` runs only for a queued prompt.
    fn route(
        &mut self,
        turns: bool,
        text: &str,
        now: Instant,
        tag: impl FnOnce(usize) -> u64,
    ) -> Result<Route, SupervisorError> {
        if turns && (self.turn != Turn::Idle || !self.queue.is_empty()) {
            if text.len() > MAX_QUEUED_PROMPT_BYTES {
                return Err(SupervisorError::OptionUnknown("text"));
            }
            if self.queue.len() >= MAX_PROMPT_QUEUE {
                return Err(SupervisorError::QueueFull(MAX_PROMPT_QUEUE));
            }
            let len = self.queue.len() + 1;
            self.queue.push_back(Queued {
                text: text.to_string(),
                tag: tag(len),
            });
            return Ok(Route::Queued(len));
        }
        if turns {
            self.busy(now, false);
        }
        Ok(Route::Send(self.generation))
    }

    /// The adapter reported a state. Returns the prompt to send next when this ended the turn. `working` or `blocked` marks the
    /// turn as really running; `idle` ends it after that, and without it only once [`IDLE_ONLY_GRACE`] has passed since the prompt
    /// (a stray or repeated `idle` ends nothing and never sends a prompt into a running turn); `done` and `unknown` end it
    /// whatever came before.
    fn observe(&mut self, state: AgentState, now: Instant) -> Option<Next> {
        match state {
            AgentState::Working | AgentState::Blocked => {
                match &mut self.turn {
                    Turn::Busy { seen_working, .. } => *seen_working = true,
                    // The adapter began a turn of its own (a mesh prompt).
                    Turn::Idle => self.busy(now, true),
                }
                None
            }
            AgentState::Idle => match &mut self.turn {
                Turn::Busy {
                    seen_working: true, ..
                } => self.finish(now),
                Turn::Busy {
                    since, idle_seen, ..
                } => {
                    *idle_seen = true;
                    if now.saturating_duration_since(*since) >= IDLE_ONLY_GRACE {
                        self.finish(now)
                    } else {
                        None
                    }
                }
                Turn::Idle => None,
            },
            _ => self.finish(now),
        }
    }

    /// The turn is over: the next waiting prompt, if any, becomes the turn.
    fn finish(&mut self, now: Instant) -> Option<Next> {
        self.turn = Turn::Idle;
        let next = self.queue.pop_front()?;
        self.busy(now, false);
        Some((next, self.generation))
    }

    /// The outcome of the send of a prompt that became the turn at `generation`. Returns whether it was sent (what its entry shows)
    /// and the prompt to send next. A failed send ends its turn, and only that one: when the generation is no longer the current one
    /// (the adapter restarted, another turn began) the failure changes nothing here.
    fn send_result(&mut self, ok: bool, generation: u64, now: Instant) -> (bool, Option<Next>) {
        if ok {
            return (true, None);
        }
        if generation != self.generation {
            return (false, None);
        }
        let next = match self.turn {
            Turn::Busy {
                seen_working: false,
                ..
            } => self.finish(now),
            _ => None,
        };
        (false, next)
    }

    /// The adapter took `session/cancel` for the turn of `generation`: that turn is read as over at `now + CANCEL_GRACE` if the
    /// adapter has not said so. With `drop_queue` the waiting prompts are returned (their tags) and forgotten. A cancel that comes
    /// back to another turn, or to a restarted adapter, does nothing.
    fn cancelled(&mut self, generation: u64, now: Instant, drop_queue: bool) -> Vec<u64> {
        if generation != self.generation {
            return Vec::new();
        }
        if let Turn::Busy { cancel_by, .. } = &mut self.turn {
            *cancel_by = Some(now + CANCEL_GRACE);
        }
        if drop_queue {
            self.queue.drain(..).map(|q| q.tag).collect()
        } else {
            Vec::new()
        }
    }

    /// A turn that ends by a clock: a cancelled one the adapter did not end in time, or one whose adapter only ever says `idle`.
    /// Returns the prompt to send next.
    fn expire(&mut self, now: Instant) -> Option<Next> {
        match self.turn {
            Turn::Busy {
                cancel_by: Some(t), ..
            } if now >= t => self.finish(now),
            Turn::Busy {
                since,
                seen_working: false,
                idle_seen: true,
                ..
            } if now.saturating_duration_since(since) >= IDLE_ONLY_GRACE => self.finish(now),
            _ => None,
        }
    }

    /// The report of a restarted adapter is the only truth: what the old process said is forgotten, so a choice it offered cannot be
    /// used against the new one, and no prompt waits for a turn that ended with the process. Anything still in flight from the old
    /// process is stale from now on (the generation changed).
    fn forget(&mut self) -> Vec<u64> {
        self.config.clear();
        self.commands.clear();
        self.turn = Turn::Idle;
        self.generation += 1;
        self.epoch += 1;
        self.queue.drain(..).map(|q| q.tag).collect()
    }
}

/// ADR-0101 (Codex review of PR #1007): how the adapter process that runs now was launched, recorded on every launch path. A decision
/// that depends on confinement (`skip_permissions = confined_only`) reads this, never the node's current sandbox setting: an adapter
/// the holder runs is unconfined by design (`launch_held_adapter` takes none with a sandbox), also after a node that confines agents
/// took it up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Confinement {
    /// Started by the node under a sandbox the platform enforces in full.
    Confined,
    /// Started without a sandbox (or with one the platform cannot enforce in full), or run by the holder.
    Unconfined,
    /// Nothing recorded (no process launched yet): read as not confined.
    #[default]
    Unknown,
}

impl Confinement {
    /// Only a launch recorded as confined counts as confined (an unknown one fails closed).
    pub fn confined(self) -> bool {
        self == Self::Confined
    }
}

/// ADR-0101 and #1009: why the `skip_permissions` policy refuses something that makes a RUNNING agent act without asking, judged by
/// how its process was launched (`forbid`: always, `confined_only`: unless the launch was recorded as confined, an unknown one
/// included), or `None` when it allows it. The one rule for "always" ([`always_by_policy`]) and for a switch of its mode, config
/// option or model (and the approved switch applied later); a start is decided before any process exists, by the confinement it
/// will get (`launch_choice`).
pub fn policy_refusal(policy: SkipPermissions, confinement: Confinement) -> Option<&'static str> {
    policy.refusal(confinement.confined())
}

/// ADR-0101: whether the `skip_permissions` policy lets an agent launched with `confinement` be allowed always (`forbid`: never,
/// `confined_only`: only a launch recorded as confined, `allow`: always), [`policy_refusal`].
pub fn always_by_policy(policy: SkipPermissions, confinement: Confinement) -> bool {
    policy_refusal(policy, confinement).is_none()
}

impl AdapterEntry {
    /// ADR-0101: the adapter declared `approval_scopes`: its requests' `scopes` are read, and it never answers a single Allow with a
    /// remembered option.
    fn strict_scopes(&self) -> bool {
        self.capabilities.iter().any(|c| c == "approval_scopes")
    }

    /// ADR-0101: the process runs the node's ACP bridge (`<agora> acp-bridge -- <agent>`, [`Manifest::driver`]). Read from the command
    /// it was started with, since every adapter agent's `agent.level` is `L3Adapter`.
    fn behind_acp_bridge(&self) -> bool {
        self.spec.command.get(1).map(String::as_str) == Some("acp-bridge")
            && self.spec.command.get(2).map(String::as_str) == Some("--")
    }
}

struct AdapterEntry {
    agent: Agent,
    /// `None` once a user stop has ended the adapter process.
    adapter: Option<Adapter>,
    session_id: String,
    worktree: PathBuf,
    branch: String,
    running: bool,
    /// What starts the adapter process again, and the checkout its session belongs to.
    spec: agora_adapters::manifest::AdapterSpec,
    checkout_root: PathBuf,
    restarts: u32,
    restart_at: Option<Instant>,
    /// What the adapter declared in `initialize` (ADR-0058: `set_mode`, `set_model`).
    capabilities: Vec<String>,
    /// The modes and models the adapter last reported (ADR-0058), checked and bounded.
    options: SessionOptions,
    /// ADR-0090: what the adapter reports beyond modes and models, and the prompts waiting behind its turn.
    rich: AdapterRich,
    /// The agent's own mesh endpoint, passed in `session/start` (`mcp_servers`). Kept across restarts after a crash, as a terminal
    /// agent's is; a stopped agent left the mesh and gets a fresh one when resumed.
    mesh: Option<agora_adapters::inject::Endpoint>,
    /// ADR-0059: the options it was started with (their arguments are already in `spec.command`).
    launch: LaunchChoice,
    /// A selected folder (ADR-0052): its path as its start held it; a restart after a crash holds it again (F1023). `None` for a
    /// worktree the node made.
    folder: Option<PathBuf>,
    /// #933: the generated Git configuration of the adapter process that runs now (ADR-0098), removed when it ends, is stopped or is
    /// replaced by a restart (which writes its own). `None` when it is not confined or nothing runs.
    git_config: Option<crate::gitconfig::LaunchFile>,
    /// ADR-0101: how the adapter process that runs now (or ran last) was launched; set by every launch path.
    confinement: Confinement,
    /// Review of #1103 (B1): what a resumed session said before it answered its start (see `open_session`), read by the next tick
    /// before anything newer.
    replayed: Vec<AdapterEvent>,
}

struct Entry {
    agent: Agent,
    worktree: PathBuf,
    branch: String,
    terminal: Option<TerminalHandle>,
    pacer: Pacer,
    next_check: Instant,
    visible: bool,
    restarts: u32,
    restart_at: Option<Instant>,
    gave_up: bool,
    extras: Extras,
    private_config: Option<PathBuf>,
    /// ADR-0060 Amendment A: the next resume carries the kind's "carry on" message ([`Restore::carry_on`]); said by `restore`, used once.
    carry_on: bool,
    /// The last resume did carry it (read by [`carried_on`](Supervisor::carried_on)).
    carried_on: bool,
    /// When a native resume of this agent's own session last started, until it ended (see `[resume] fresh_argv`).
    native_since: Option<Instant>,
    /// A resume failed at once and the kind has `[resume] fresh_argv`: the next restart starts the session from nothing.
    fresh_next: bool,
    /// ADR-0059: the options it was started with (their arguments are in `extras.options`).
    launch: LaunchChoice,
    /// A selected folder (ADR-0052): its path as its start held it. A restart or resume holds it again and starts in it, or not at all
    /// when the path no longer leads to a folder there (F1020, F1021). `None` for a worktree the node made.
    folder: Option<PathBuf>,
}

pub struct Supervisor {
    cfg: SupervisorConfig,
    kinds: BTreeMap<String, KindEntry>,
    agents: BTreeMap<AgentId, Entry>,
    /// Terminal agents that ended and still owe one last usage read.
    ended_usage: Vec<AgentId>,
    next_id: u64,
    hook: Option<LaunchHook>,
    abort_hook: Option<AbortHook>,
    sandbox: crate::sandbox::SandboxPolicy,
    /// ADR-0084: the secrets this node holds and which kind may receive which.
    credentials: crate::credentials::Credentials,
    adapters: BTreeMap<AgentId, AdapterEntry>,
    /// ADR-0090: the outcome of sends of queued prompts, made on threads of their own (a hung adapter must not stall the tick): `(agent,
    /// tag, sent)`. The next poll reads them.
    sent: SentResults,
    /// ADR-0057: helper -> the agent that started it.
    parents: BTreeMap<AgentId, AgentId>,
    /// ADR-0059: the owner's `skip_permissions` policy.
    skip_permissions: SkipPermissions,
    /// ADR-0092 (#1133): the owner's `agents.registry` policy; a registry kind it switches off is neither listed nor started.
    registry: crate::manifests::RegistryPolicy,
    /// The kinds registered from the ACP registry's converted manifests (by origin, not by name).
    registry_kinds: BTreeSet<String>,
    /// #1150: the folder below which a registry kind that starts through a package runner gets an empty working folder of its own
    /// (`runner_folder`); set by the node (`<data>/runner`). `None` refuses such a start.
    runner_root: Option<PathBuf>,
    /// ADR-0079: the terminal holder agents are started through, when this node has one (Unix); `None` runs every agent in the node.
    holder: Option<std::sync::Arc<HolderHub>>,
    /// Agents taken up from the holder whose launch options came from the ledger without their arguments (`set_kept_launch`): their
    /// next process (a restart after a crash) runs without them, so it is not shown with them.
    kept_launch: BTreeSet<AgentId>,
}

impl Supervisor {
    pub fn new(cfg: SupervisorConfig) -> Self {
        Self {
            cfg,
            kinds: BTreeMap::new(),
            agents: BTreeMap::new(),
            ended_usage: Vec::new(),
            next_id: 1,
            hook: None,
            abort_hook: None,
            sandbox: Default::default(),
            credentials: Default::default(),
            adapters: BTreeMap::new(),
            sent: Arc::default(),
            parents: BTreeMap::new(),
            skip_permissions: SkipPermissions::default(),
            registry: crate::manifests::RegistryPolicy::default(),
            registry_kinds: BTreeSet::new(),
            runner_root: None,
            holder: None,
            kept_launch: BTreeSet::new(),
        }
    }

    /// ADR-0079: starts agents through this holder (and falls back to running them here when it cannot be had). A confined agent is
    /// always run by the node: the holder does not carry a confinement yet.
    pub fn set_holder(&mut self, holder: std::sync::Arc<HolderHub>) {
        self.holder = Some(holder);
    }

    /// The holder this node uses, if any (tests).
    #[doc(hidden)]
    pub fn holder_hub(&self) -> Option<std::sync::Arc<HolderHub>> {
        self.holder.clone()
    }

    /// ADR-0079 decision 9: the agents whose process is the holder's, and the holder's process (pid and start time), for the ledger.
    pub fn held_agents(&self) -> (Vec<AgentId>, Option<(u32, u64)>) {
        let ids = self
            .agents
            .iter()
            .filter(|(_, e)| e.terminal.as_ref().is_some_and(TerminalHandle::is_held))
            .map(|(id, _)| id.clone())
            // ADR-0093: an adapter's process the holder runs.
            .chain(
                self.adapters
                    .iter()
                    .filter(|(_, e)| e.running && e.adapter.as_ref().is_some_and(Adapter::is_held))
                    .map(|(id, _)| id.clone()),
            )
            .collect();
        (ids, self.holder.as_ref().and_then(|h| h.identity()))
    }

    /// ADR-0093: for each adapter agent that runs in the holder, what a later node needs to take it up and cannot ask: the adapter's
    /// own session id and the capabilities it declared. The node writes them to the ledger.
    pub fn held_adapter_states(&self) -> Vec<(AgentId, String, Vec<String>)> {
        self.adapters
            .iter()
            .filter(|(_, e)| e.running && e.adapter.as_ref().is_some_and(Adapter::is_held))
            .map(|(id, e)| (id.clone(), e.session_id.clone(), e.capabilities.clone()))
            .collect()
    }

    /// ADR-0093 decisions 1 and 3: takes an adapter agent the holder kept running while the node was away. Nothing is started and the
    /// adapter is not initialised again (it still has its session): the node talks to the process it left, through the holder.
    /// `pipe` is what the holder kept of its output: lines an earlier node was given (what was still waiting on a person is asked again,
    /// never answered), then lines no node had, which are read as if they had just arrived. On failure nothing is left behind and the
    /// agent stays in the holder (the caller ends it).
    pub fn adopt_held_pipe(
        &mut self,
        r: Restore,
        mut reattach: Reattach,
        pipe: HeldPipe,
        session: &Arc<HolderSession>,
        now: Instant,
    ) -> Result<(), SupervisorError> {
        let _ = now;
        if reattach.session_id.is_empty() {
            return Err(SupervisorError::Spawn("no session".into()));
        }
        let id = r.id.clone();
        let (kind, project, worktree) = (r.kind.clone(), r.project.to_string(), r.folder.clone());
        self.restore(r)?;
        if !self.adapters.contains_key(&id) {
            // A kind that is not hosted by an adapter was not started on pipes.
            self.unrestore(&id);
            return Err(SupervisorError::UnknownKind(kind));
        }
        // ADR-0093 decision 3 and ADR-0104 decision 1: the waiting requests and the newest reports from what the earlier node had.
        let replay = newest_reported(&mut reattach, &pipe.seen);
        let io = session.pipe_io(id.as_str(), pipe.fresh, pipe.dropped, pipe.info.pid);
        let adapter = match Adapter::attach_held(io, replay) {
            Ok(a) => a,
            Err(e) => {
                self.unrestore(&id);
                return Err(SupervisorError::Spawn(e.to_string()));
            }
        };
        // The agent's mesh endpoint was the old node's: made again at the same path, so its configuration keeps working.
        let mesh = self.adapter_mesh(&id, &kind, &project, &worktree);
        let Some(e) = self.adapters.get_mut(&id) else {
            return Err(SupervisorError::NotFound(id));
        };
        e.adapter = Some(adapter);
        // ADR-0101 (Codex review of PR #1007): the holder runs only adapters without a sandbox (`launch_held_adapter`), whatever this
        // node's confinement policy says now.
        e.confinement = Confinement::Unconfined;
        e.session_id = reattach.session_id;
        e.capabilities = reattach.capabilities;
        // Bounded and judged again as a report is (`perm`): the file is the node's own and the replay the holder's, the words in both are
        // the adapter's.
        e.options = bounded_options(reattach.options);
        e.rich.config = bounded_config(reattach.config);
        e.rich.commands = bounded_commands(reattach.commands);
        e.mesh = mesh;
        e.running = true;
        if let Some(state) = reattach.state {
            e.agent.observe(state, StateSource::Reported);
        }
        Ok(())
    }

    /// ADR-0079 decision 4: takes an agent the holder kept running while the node was away: it is put back as `r.id` with its terminal,
    /// which first shows what it printed meanwhile (`held.info.buffer`) and then gets a resize nudge, so full-screen agents redraw.
    /// Nothing is started. On failure nothing is left behind, and the agent stays in the holder.
    pub fn adopt_held(
        &mut self,
        r: Restore,
        held: Held,
        session: &std::sync::Arc<HolderSession>,
        started: Option<SystemTime>,
        now: Instant,
    ) -> Result<(), SupervisorError> {
        let id = r.id.clone();
        let (kind, project, worktree) = (r.kind.clone(), r.project.to_string(), r.folder.clone());
        let Held { info, terminal } = held;
        let Some(master) = terminal else {
            return Err(SupervisorError::Spawn("no terminal".into()));
        };
        let size = self.cfg.terminal.size;
        self.restore(r)?;
        let made = agora_platform::held_pty(master, info.info.pid, session.control_of(id.as_str()))
            .and_then(|pty| TerminalHandle::start_with_output(pty, self.cfg.terminal, &info.buffer))
            // #925: the process started under the earlier node; files it wrote then are still its own.
            .map(|t| match started {
                Some(bound) => t.started_after(bound),
                None => t,
            })
            .map(TerminalHandle::into_held);
        let terminal = match made {
            Ok(t) => t,
            Err(e) => {
                self.unrestore(&id);
                return Err(SupervisorError::Spawn(e.to_string()));
            }
        };
        // The agent's mesh endpoint was the old node's: made again at the same path, so its configuration keeps working.
        if let Some(hook) = &self.hook {
            let _ = hook(&Launching {
                id: id.clone(),
                kind,
                project,
                node: self.cfg.node.as_str().to_string(),
                worktree,
                identity: None,
            });
        }
        // Two resizes so the program gets a window change even when the size did not differ.
        if let Ok(ctl) = terminal.input_handle() {
            let _ = ctl.resize(size.cols, size.rows.saturating_sub(1).max(1));
            let _ = ctl.resize(size.cols, size.rows);
        }
        if let Some(e) = self.agents.get_mut(&id) {
            e.terminal = Some(terminal);
            e.pacer = Pacer::default();
            e.next_check = now;
        }
        Ok(())
    }

    /// ADR-0079 decision 4: an agent taken up from the holder starts at `Unknown`, and an idle program writes nothing for screen detection
    /// to read afresh (a terminal agent that waits for its next prompt may stay silent for hours). Until a screen rule speaks, it shows the
    /// state the ledger last knew. The source stays `Screen`, so a real detection (and any reported state) replaces it on the next check,
    /// and nothing is made up: `Unknown` stays when the ledger knew no state.
    pub fn seed_reattached_state(&mut self, id: &AgentId, last: Option<AgentState>) {
        let Some(state) = last.filter(|s| *s != AgentState::Unknown) else {
            return;
        };
        if let Some(e) = self.agents.get_mut(id) {
            if e.agent.state() == AgentState::Unknown {
                e.agent.observe(state, StateSource::Screen);
            }
        }
    }

    /// ADR-0079/0093 with ADR-0059: an agent taken up from the holder is the process that was started with these launch options (the
    /// ledger's record of its `AgentStarted`), so it is shown with them, and acting without asking when it does (#907). It skips when
    /// the launch said so (`recorded_skips`), when this node's manifest says one of the options does, and when an option is no longer
    /// in the manifest (it cannot be told, so it is shown as skipping). Their arguments are not known again: a restart after a crash
    /// runs without them, as a resume does, and then the agent is shown without them (see `restart`).
    pub fn set_kept_launch(&mut self, id: &AgentId, ids: &[String], recorded_skips: bool) {
        let kind = self
            .agents
            .get(id)
            .map(|e| e.agent.kind.clone())
            .or_else(|| self.adapters.get(id).map(|e| e.agent.kind.clone()));
        let Some(kind) = kind else {
            return;
        };
        let manifest = self.kinds.get(&kind).map(|k| &k.manifest);
        let skips = recorded_skips
            || ids.iter().any(|o| {
                manifest
                    .and_then(|m| m.launch_option(o))
                    .is_none_or(|o| o.skips_permissions)
            });
        let launch = LaunchChoice {
            ids: ids.to_vec(),
            args: Vec::new(),
            skips_permissions: skips,
        };
        if let Some(e) = self.agents.get_mut(id) {
            e.launch = launch;
        } else if let Some(e) = self.adapters.get_mut(id) {
            e.launch = launch;
        }
        if !ids.is_empty() {
            self.kept_launch.insert(id.clone());
        }
    }

    /// #932: what an adapter agent reported of its session, as this node holds it now (bounded and judged, also when it was put back
    /// from the ledger at a take-up), as the events a report gives. Nothing for an agent that is not hosted by an adapter, and nothing
    /// for a part it never reported.
    pub fn session_events(&self, id: &AgentId) -> Vec<SupervisorEvent> {
        let Some(e) = self.adapters.get(id) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let event = |event| SupervisorEvent::Adapter {
            agent: id.clone(),
            event,
        };
        if e.options != SessionOptions::default() {
            out.push(event(AdapterEvent::Options {
                session_id: e.session_id.clone(),
                options: e.options.clone(),
            }));
        }
        if !e.rich.config.is_empty() {
            out.push(event(AdapterEvent::Config {
                session_id: e.session_id.clone(),
                options: e.rich.config.clone(),
            }));
        }
        if !e.rich.commands.is_empty() {
            out.push(event(AdapterEvent::Commands {
                session_id: e.session_id.clone(),
                commands: e.rich.commands.clone(),
            }));
        }
        out
    }

    /// ADR-0060: agent ids start at `next` (the session ledger's), so an id is never given twice across the node's restarts.
    pub fn reserve_ids(&mut self, next: u64) {
        self.next_id = self.next_id.max(next);
    }

    /// ADR-0059: the owner's `skip_permissions` policy (from the node's policy file; the default allows).
    pub fn set_skip_permissions(&mut self, policy: SkipPermissions) {
        self.skip_permissions = policy;
    }

    /// ADR-0092 (#1133): the owner's `agents.registry` policy (from the node's policy file; the default offers them all).
    pub fn set_registry_policy(&mut self, policy: crate::manifests::RegistryPolicy) {
        self.registry = policy;
    }

    /// #1150: where registry kinds that start through a package runner get their empty working folders (the node's `<data>/runner`).
    pub fn set_runner_root(&mut self, root: PathBuf) {
        self.runner_root = Some(std::path::absolute(&root).unwrap_or(root));
    }

    /// #1150 (ADR-0092): the folder the package runner of `kind` is started in, for an ACP registry kind (by origin) that starts
    /// through one (`npx`, `uvx`: [`crate::manifests::fetches_at_start`]); `None` for every other kind, which starts in its worktree
    /// as before. A runner reads package-manager configuration from the folder it starts in and the folders above it (a project's
    /// `.npmrc`, `package.json` and `node_modules`; uv's `uv.toml`, `pyproject.toml` and `.python-version`), which could change what
    /// the pinned package resolves to; from this folder (inside the node's data folder) it reads only the owner's own user and global
    /// configuration. The agent still works in its worktree: ACP gives it the session's folder (`cwd`), which an agent must use
    /// wherever its process was started. Every start gets a new folder ([`crate::runner_folders::new_folder`]); the agent's earlier
    /// ones are removed on another thread, bounded, outside this lock ([`crate::runner_folders::remove_in_background`]). Only a kind
    /// run behind the ACP bridge (no `[adapter]` of its own) and an agent with a plain id are started this way; anything else is
    /// refused.
    fn runner_folder(&self, kind: &str, id: &AgentId) -> Result<Option<PathBuf>, SupervisorError> {
        let Some(k) = self.kinds.get(kind) else {
            return Ok(None);
        };
        if !(self.registry_kinds.contains(kind) && crate::manifests::fetches_at_start(&k.manifest))
        {
            return Ok(None);
        }
        if k.manifest.adapter.is_some() {
            return Err(SupervisorError::Spawn(
                "a package runner of a registry kind runs only behind the ACP bridge".into(),
            ));
        }
        if !crate::transcript::plain_id(id.as_str()) {
            return Err(SupervisorError::Spawn(
                "package runner folder: not a plain agent id".into(),
            ));
        }
        let root = self
            .runner_root
            .as_ref()
            .ok_or_else(|| SupervisorError::Spawn("no folder for package runners".into()))?;
        let dir = crate::runner_folders::new_folder(root, id.as_str())
            .map_err(|e| SupervisorError::Spawn(format!("package runner folder: {e}")))?;
        if let Some(name) = dir.file_name() {
            let earlier = crate::runner_folders::earlier_folders(root, id.as_str(), name);
            crate::runner_folders::remove_in_background(root.clone(), earlier);
        }
        Ok(Some(dir))
    }

    /// #1133: whether a start of `kind` must be signed by a person's device: a kind from the ACP registry started through a package
    /// runner. Read where the hub's commands enter the node. A kind not known yet whose name says it is a registry one (`registry-*`, a
    /// manifest added after the node read its folder) is held to it too: its description is read only after this check (review F6).
    pub fn start_needs_signature(&self, kind: &str) -> bool {
        match self.kinds.get(kind) {
            Some(k) => {
                self.registry_kinds.contains(kind)
                    && crate::manifests::fetches_at_start(&k.manifest)
            }
            None => kind.starts_with(crate::manifests::REGISTRY_PREFIX),
        }
    }

    /// The owner's registry policy for `kind`: why it is switched off, for a kind from the registry only.
    fn registry_refusal(&self, kind: &str) -> Option<&'static str> {
        if self.registry_kinds.contains(kind) {
            self.registry.refusal(kind)
        } else {
            None
        }
    }

    /// #1133: a registry kind the owner's policy switches off is not started, by any path.
    fn refuse_registry(&self, kind: &str) -> Result<(), SupervisorError> {
        match self.registry_refusal(kind) {
            Some(rule) => Err(SupervisorError::RegistryRefused(kind.to_string(), rule)),
            None => Ok(()),
        }
    }

    /// ADR-0059: resolves a start's launch options and checks them against the owner's policy, before anything is created.
    /// `confined_only` needs this agent's confinement to be active (the node confines agents and can here: a start that cannot be
    /// confined is refused on its own before this).
    fn launch_choice(&self, kind: &str, req: &Requested) -> Result<LaunchChoice, SupervisorError> {
        if req.is_empty() {
            return Ok(LaunchChoice::default());
        }
        let k = self
            .kinds
            .get(kind)
            .ok_or_else(|| SupervisorError::UnknownKind(kind.to_string()))?;
        let confined =
            self.sandbox.level != crate::sandbox::Level::Off && self.sandbox.enforceable();
        crate::launch_options::choose(&k.manifest, req, self.skip_permissions, confined).map_err(
            |r| match r {
                Refusal::Invalid { option } => {
                    SupervisorError::OptionInvalid(kind.to_string(), option)
                }
                Refusal::Unconfirmed { option } => {
                    SupervisorError::OptionUnconfirmed(kind.to_string(), option)
                }
                Refusal::Forbidden { option, policy } => {
                    SupervisorError::OptionForbidden(kind.to_string(), option, policy)
                }
                Refusal::SettingUnknown { setting } => {
                    SupervisorError::SettingUnknown(kind.to_string(), setting)
                }
                Refusal::SettingInvalid { setting } => {
                    SupervisorError::SettingInvalid(kind.to_string(), setting)
                }
            },
        )
    }

    /// ADR-0066: the settings an agent's `agent_start` asks for, checked against this node's manifest for `kind` (unknown, repeated or
    /// invalid ones are refused, so nothing is asked about nor started) and returned as `(id, label, value)` in the manifest's order,
    /// which is the order the approval card shows, the signature covers and the launch applies. Empty in, empty out.
    pub fn checked_settings(
        &self,
        kind: &str,
        settings: &[(String, String)],
    ) -> Result<Vec<(String, String, String)>, SupervisorError> {
        if settings.is_empty() {
            return Ok(Vec::new());
        }
        self.launch_choice(
            kind,
            &Requested {
                settings: settings.to_vec(),
                ..Requested::default()
            },
        )?;
        let k = self
            .kinds
            .get(kind)
            .ok_or_else(|| SupervisorError::UnknownKind(kind.to_string()))?;
        Ok(k.manifest
            .launch
            .setting
            .iter()
            .filter_map(|st| {
                settings
                    .iter()
                    .find(|(id, _)| *id == st.id)
                    .map(|(id, value)| (id.clone(), st.label.clone(), value.clone()))
            })
            .collect())
    }

    /// ADR-0059 decision 5: the launch options an agent's `agent_start` asks for, checked against this node's manifest for `kind` and
    /// the owner's `skip_permissions` policy before anyone is asked (an undeclared, repeated or malformed id is `agent.option_invalid`,
    /// a policy refusal `agent.option_forbidden`), and returned as `(id, label, skips_permissions)` in the manifest's order: what the
    /// approval card shows and the signature covers. Which options skip permissions is this manifest's word. The user's signed
    /// decision naming them is their confirmation. Empty in, empty out.
    pub fn checked_options(
        &self,
        kind: &str,
        ids: &[String],
    ) -> Result<Vec<(String, String, bool)>, SupervisorError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        self.launch_choice(
            kind,
            &Requested {
                ids: ids.to_vec(),
                confirmed: ids.to_vec(),
                ..Requested::default()
            },
        )?;
        let k = self
            .kinds
            .get(kind)
            .ok_or_else(|| SupervisorError::UnknownKind(kind.to_string()))?;
        Ok(k.manifest
            .launch
            .option
            .iter()
            .filter(|o| ids.contains(&o.id))
            .map(|o| (o.id.clone(), o.label.clone(), o.skips_permissions))
            .collect())
    }

    /// ADR-0057: `child` was started by `parent` (an agent's `agent_start`). Carried in every view of `child`.
    pub fn set_parent(&mut self, child: &AgentId, parent: &AgentId) {
        self.parents.insert(child.clone(), parent.clone());
    }

    /// ADR-0057: a fresh look at a terminal agent's live screen, run on its bottom lines now (never the paced detection's last result):
    /// what its rules detect, whether its `[[ready]]` rules match, and whether its input box holds exactly `text` (`[[typed]]` and
    /// `input_box`). `None` for an agent without a terminal.
    pub fn screen_check(&self, id: &AgentId, text: &str) -> Option<ScreenCheck> {
        let e = self.agents.get(id)?;
        let term = e.terminal.as_ref()?;
        let kind = self.kinds.get(&e.agent.kind)?;
        let squash = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        let needle = squash(text);
        Some(term.inspect(|t| {
            ScreenCheck {
                state: kind.detector.detect(t).map(|d| d.state),
                ready: kind.ready.detect(t).is_some(),
                shows: !needle.is_empty()
                    && kind.typed.detect(t).is_some()
                    && kind
                        .manifest
                        .input_box
                        .as_deref()
                        .and_then(|p| Detector::input_box(p, t))
                        .is_some_and(|in_box| squash(&in_box) == needle),
            }
        }))
    }

    /// ADR-0057: whether a scoped grant ("up to N more without asking") may cover `kind`: only a kind with an agent integration (a
    /// mesh section, an adapter or a structured mode) whose program (`process.launch`, else `structured.acp.argv`, else
    /// `adapter.command`) is not a shell, a wrapper or an interpreter, however many screen rules it has: a grant there would mean "run
    /// any N programs without asking". Unknown kinds are not grantable.
    pub fn grantable(&self, kind: &str) -> bool {
        let Some(k) = self.kinds.get(kind) else {
            return false;
        };
        let m = &k.manifest;
        let integrated = m.mcp.is_some() || m.adapter.is_some() || m.structured.is_some();
        let program = m
            .process
            .launch
            .first()
            .or_else(|| {
                m.structured
                    .as_ref()
                    .and_then(|s| s.acp.as_ref())
                    .and_then(|a| a.argv.first())
            })
            .or_else(|| m.adapter.as_ref().and_then(|a| a.command.first()));
        kind != "shell" && integrated && program.is_some_and(|p| !runs_anything(p))
    }

    /// ADR-0057: the footer and marker texts the kind's `[[typed]]` rules require on screen. A prompt that contains one could stand in
    /// for the real footer, so such a prompt is never typed.
    pub fn typed_markers(&self, kind: &str) -> Vec<String> {
        let Some(k) = self.kinds.get(kind) else {
            return Vec::new();
        };
        k.manifest
            .typed
            .iter()
            .flat_map(|r| {
                r.contains.iter().cloned().chain(
                    r.all
                        .iter()
                        .chain(&r.any)
                        .filter_map(|c| c.contains.clone()),
                )
            })
            .collect()
    }

    /// The node's confinement settings (ADR-0047), for checks that compare two agents' confinement.
    pub fn sandbox_policy(&self) -> &crate::sandbox::SandboxPolicy {
        &self.sandbox
    }

    /// ADR-0084: sets the secrets this node holds and the policy's `credentials.<kind>` lines.
    pub fn set_credentials(&mut self, credentials: crate::credentials::Credentials) {
        self.credentials = credentials;
    }

    /// ADR-0084: every kind's `credential_env`, from the manifests registered.
    pub fn declared_credentials(&self) -> BTreeMap<String, Vec<String>> {
        self.kinds
            .iter()
            .filter_map(|(k, e)| {
                let names = &e.manifest.accounts.as_ref()?.credential_env;
                (!names.is_empty()).then(|| (k.clone(), names.clone()))
            })
            .collect()
    }

    /// ADR-0084: what an agent of `kind` gets and loses in its environment.
    fn credential_plan(&self, kind: &str) -> crate::credentials::Plan {
        self.credentials
            .plan(kind, &self.declared_credentials(), |n| {
                std::env::var(n).ok()
            })
    }

    /// ADR-0084: the variables an adapter of `kind` is given (an adapter starts from a clean environment, so nothing is scrubbed).
    fn credential_vars(&self, kind: &str) -> Vec<(String, String)> {
        self.credential_plan(kind)
            .set
            .into_iter()
            .map(|(n, v)| (n, v.expose().to_string()))
            .collect()
    }

    /// Installs the hook that gives each launching agent a mesh endpoint.
    pub fn set_launch_hook(&mut self, hook: LaunchHook) {
        self.hook = Some(hook);
    }

    /// The confinement of one agent of `kind`: the node's policy plus what the kind's manifest asks, if the owner trusts that kind.
    fn sandbox_for(
        &self,
        kind: &str,
        worktree: &Path,
        program: &str,
    ) -> Option<agora_platform::Sandbox> {
        let needs = self
            .kinds
            .get(kind)
            .and_then(|k| k.manifest.sandbox.as_ref());
        self.sandbox.profile(kind, needs, worktree, program)
    }

    /// The confinement of one adapter agent of `kind` ([`Self::sandbox_for`]), refused as a terminal agent's is
    /// ([`Self::refuse_reachable`]): an adapter and the agent below it run with exactly this profile. #933: with the launch's generated
    /// Git file, which the caller keeps for as long as the adapter it starts runs (dropping it, as a failed start does, removes it).
    fn adapter_sandbox(
        &self,
        kind: &str,
        worktree: &Path,
        program: &str,
    ) -> Result<
        (
            Option<agora_platform::Sandbox>,
            Option<crate::gitconfig::LaunchFile>,
        ),
        SupervisorError,
    > {
        let mut profile = self.sandbox_for(kind, worktree, program);
        let mut file = None;
        if let Some(p) = &mut profile {
            self.refuse_reachable(kind, p)?;
            p.git_config = self.sandbox.git_config(p);
            file = p.git_config.clone().map(crate::gitconfig::LaunchFile::new);
        }
        Ok((profile, file))
    }

    /// ADR-0082, decision 4: where the node requires its control token, the file must lie outside every folder the agent may read or
    /// write, because Landlock cannot say "except" (`Sandbox::deny` is ignored there). ADR-0084: the same for the folder of stored
    /// secrets (only once it holds one): a grant that reaches it would hand a confined agent every secret in it. #1114: and, where
    /// `Sandbox::deny` is ignored, for every path the node keeps closed to agents. Refused, never started. #1150: the paths checked are
    /// handed to the platform (`Sandbox::closed`), which checks each grant again as it opens it for the kernel.
    fn refuse_reachable(
        &self,
        kind: &str,
        profile: &mut agora_platform::Sandbox,
    ) -> Result<(), SupervisorError> {
        let mut closed = self.sandbox.closed_paths();
        if self.sandbox.token_reachable(profile) {
            return Err(SupervisorError::SandboxRefused(
                kind.to_string(),
                self.sandbox.level.name(),
                "control_token_reachable",
            ));
        }
        if let Some(store) = self.credentials.store() {
            if !store.list().is_empty() {
                if self.sandbox.path_reachable(store.dir(), profile) {
                    return Err(SupervisorError::SandboxRefused(
                        kind.to_string(),
                        self.sandbox.level.name(),
                        "secrets_reachable",
                    ));
                }
                if agora_platform::PROTECTED_PATHS_BY_REFUSAL {
                    closed.push(store.dir().to_path_buf());
                }
            }
        }
        // #1114: the same for every path of the node's own (its data folder with the secrets, the ledger and the transcripts, its
        // manifests, its socket folder) where only a refusal keeps them closed (Linux). Checked last, so the two above keep their reason.
        if self.sandbox.protected_reachable(profile).is_some() {
            return Err(SupervisorError::SandboxRefused(
                kind.to_string(),
                self.sandbox.level.name(),
                "protected_path_reachable",
            ));
        }
        profile.closed = closed;
        Ok(())
    }

    /// With confinement on, a launch in a folder that would make the confinement meaningless is refused before anything starts.
    pub(crate) fn refuse_protected(
        &self,
        selected: &Path,
        worktree_root: &Path,
    ) -> Result<(), SupervisorError> {
        if self.sandbox.level == crate::sandbox::Level::Off {
            return Ok(());
        }
        let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        let worktrees = canon(worktree_root);
        let node_home = worktrees
            .parent()
            .map_or_else(|| worktrees.clone(), Path::to_path_buf);
        let home = crate::usage_files::home_dir().map(|h| canon(&h));
        if crate::sandbox::refused_directory(
            &canon(selected),
            home.as_deref(),
            &node_home,
            &worktrees,
        ) {
            return Err(SupervisorError::ProtectedDirectory);
        }
        Ok(())
    }

    /// Sets how agents are confined (ADR-0047); the default is not at all.
    pub fn set_sandbox(&mut self, sandbox: crate::sandbox::SandboxPolicy) {
        self.sandbox = sandbox;
    }

    /// Installs the hook that undoes the launch hook when a start fails after it ran.
    pub fn set_abort_hook(&mut self, hook: AbortHook) {
        self.abort_hook = Some(hook);
    }

    /// Lets go of the launch and abort hooks, for a node that has stopped: they hold the node's mesh, which holds what holds this
    /// supervisor, so in a process that goes on (a test, an embedding) the mesh, and the agents' endpoints it locks, would never be
    /// let go otherwise.
    pub fn clear_hooks(&mut self) {
        self.hook = None;
        self.abort_hook = None;
    }

    /// A start that failed after the worktree and the mesh endpoint were made: give both back, so repeated failures cannot pile up.
    fn abandon(&self, id: &AgentId, worktree: git::StartWorktree) {
        if let Some(h) = &self.abort_hook {
            h(id);
        }
        let _ = worktree.rollback();
    }

    /// Make an agent kind available. Replaces a previous manifest of the same kind.
    pub fn register(&mut self, manifest: Manifest) {
        // A hand-made description of a kind replaces a registry one of the same name: it is no longer the registry's (#1133 F5).
        self.registry_kinds.remove(&manifest.kind);
        self.register_kind(manifest);
    }

    /// [`Self::register`] for a kind converted from the ACP registry (ADR-0092, `manifests::Source::Registry`): the owner's
    /// `agents.registry` policy and the signed-start rule apply to it, by its origin, never by its name alone.
    pub fn register_from_registry(&mut self, manifest: Manifest) {
        self.registry_kinds.insert(manifest.kind.clone());
        self.register_kind(manifest);
    }

    /// [`Self::register`] or [`Self::register_from_registry`], by where the description came from.
    pub fn register_loaded(&mut self, loaded: crate::manifests::Loaded) {
        if loaded.source == crate::manifests::Source::Registry {
            self.register_from_registry(loaded.manifest);
        } else {
            self.register(loaded.manifest);
        }
    }

    fn register_kind(&mut self, manifest: Manifest) {
        let detector = Detector::new(&manifest);
        let ready = Detector::ready(&manifest);
        let typed = Detector::typed(&manifest);
        self.kinds.insert(
            manifest.kind.clone(),
            KindEntry {
                manifest,
                detector,
                ready,
                typed,
            },
        );
    }

    /// ADR-0108: whether the agent runs (in this node or in the holder) or waits for a restart: what a forget must not touch.
    pub fn live(&self, id: &AgentId) -> bool {
        self.agents
            .get(id)
            .is_some_and(|e| e.terminal.is_some() || e.restart_at.is_some())
            || self
                .adapters
                .get(id)
                .is_some_and(|a| a.running || a.restart_at.is_some())
    }

    /// Agents that are running or waiting for a restart.
    fn active(&self) -> usize {
        self.agents
            .values()
            .filter(|e| e.terminal.is_some() || e.restart_at.is_some())
            .count()
    }

    fn check_limits(&self) -> Result<(), SupervisorError> {
        let active = self.active()
            + self
                .adapters
                .values()
                .filter(|a| a.running || a.restart_at.is_some())
                .count();
        for (resource, max) in [
            ("agents", self.cfg.limits.max_agents),
            ("terminals", self.cfg.limits.max_terminals),
        ] {
            if let Some(max) = max {
                if active >= max {
                    return Err(SupervisorError::ResourceLimit {
                        resource,
                        limit: max as u64,
                    });
                }
            }
        }
        Ok(())
    }

    /// Disk used by an agent's worktree.
    pub fn disk_usage(&self, id: &AgentId) -> Result<crate::files::DiskUsage, SupervisorError> {
        let e = self
            .agents
            .get(id)
            .ok_or_else(|| SupervisorError::NotFound(id.clone()))?;
        crate::files::disk_usage(&e.worktree).map_err(|e| SupervisorError::Spawn(e.to_string()))
    }

    /// An agent that cannot be confined is not started unconfined (ADR-0047), and is refused before anything (a worktree) is created.
    /// #897: an agent of a kind started as a plain process (an adapter or ACP bridge) also needs the platform to confine a command,
    /// which Windows cannot yet; before, it failed only at spawn, after its worktree was made, as an internal error.
    fn refuse_unconfinable(&self, kind: &str) -> Result<(), SupervisorError> {
        // Every start path checks this first, so the owner's registry policy is checked with it.
        self.refuse_registry(kind)?;
        let structured = self
            .kinds
            .get(kind)
            .is_some_and(|k| k.manifest.driver(&bridge_program()).is_some());
        let confinable = if structured {
            self.sandbox.enforceable_for_commands()
        } else {
            self.sandbox.enforceable()
        };
        if confinable {
            Ok(())
        } else {
            Err(SupervisorError::SandboxUnavailable(
                kind.to_string(),
                self.sandbox.level.name(),
            ))
        }
    }

    /// Create the agent's own worktree and start it there.
    pub fn start(&mut self, req: &StartRequest, now: Instant) -> Result<AgentId, SupervisorError> {
        self.start_with_options(req, &Requested::default(), now)
    }

    /// [`Self::start`] with the launch options a person chose (ADR-0059). Only the person's own start passes options: a helper an agent
    /// starts (ADR-0057) goes through [`Self::start`] and never gets one.
    pub fn start_with_options(
        &mut self,
        req: &StartRequest,
        options: &Requested,
        now: Instant,
    ) -> Result<AgentId, SupervisorError> {
        // Checked first: a refused start must not leave a worktree behind.
        self.check_limits()?;
        let choice = self.launch_choice(&req.kind, options)?;
        let kind = self
            .kinds
            .get(&req.kind)
            .ok_or_else(|| SupervisorError::UnknownKind(req.kind.clone()))?;
        let driver = kind.manifest.driver(&bridge_program());
        self.refuse_unconfinable(&req.kind)?;
        if let Some(spec) = driver {
            if let Some(p) = spec.command.first().and_then(|c| {
                agora_adapters::package::revoked_publisher_of(
                    &agora_adapters::package::agora_home(),
                    c,
                )
            }) {
                return Err(SupervisorError::PublisherRevoked(req.kind.clone(), p));
            }
            let mut spec = spec;
            spec.command.extend(choice.args.iter().cloned());
            return self.start_adapter(req, &spec, choice);
        }
        let argv = kind.manifest.process.launch.clone();
        if argv.is_empty() {
            return Err(SupervisorError::UnknownKind(req.kind.clone()));
        }
        let checkout = git::discover_checkout(&req.checkout)?;
        let worktree = req.worktree_root.join(&req.name);
        let branch = format!("agent/{}", req.name);
        let created =
            git::create_start_worktree(&checkout.root, &worktree, &branch, req.base.as_deref())?;

        let id = AgentId::new(format!("agent-{}", self.next_id));
        self.next_id += 1;
        let extras = match self.inject(
            &id,
            &req.kind,
            &checkout.project.to_string(),
            &worktree,
            None,
        ) {
            Ok(x) => Extras {
                options: choice.args.clone(),
                ..x
            },
            Err(e) => {
                self.abandon(&id, created);
                return Err(e);
            }
        };
        let mut agent = Agent::new(
            id.clone(),
            checkout.project,
            self.cfg.node.clone(),
            Some(WorktreeId::new(worktree.to_string_lossy())),
            req.kind.clone(),
            IntegrationLevel::L1Manifest,
        );
        let terminal = match self.first_launch(&id, &req.kind, &argv, &worktree, &extras, None) {
            Ok((t, session)) => {
                agent.session = session;
                t
            }
            Err(e) => {
                self.abandon(&id, created);
                return Err(e);
            }
        };
        self.agents.insert(
            id.clone(),
            Entry {
                agent,
                worktree,
                branch,
                terminal: Some(terminal),
                pacer: Pacer::default(),
                next_check: now,
                visible: true,
                restarts: 0,
                restart_at: None,
                gave_up: false,
                extras,
                private_config: None,
                carry_on: false,
                carried_on: false,
                native_since: None,
                fresh_next: false,
                launch: choice,
                folder: None,
            },
        );
        Ok(id)
    }

    /// Explicit directory/worktree launch. Only a worktree created here is rolled back after a failed start.
    pub fn start_selected(
        &mut self,
        req: &StartRequest,
        project: agora_core::ProjectId,
        git_root: &Path,
        separate: bool,
        now: Instant,
    ) -> Result<AgentId, SupervisorError> {
        self.start_selected_with_options(
            req,
            project,
            git_root,
            separate,
            &Requested::default(),
            now,
            None,
        )
    }

    /// [`Self::start_selected`] with the launch options a person chose (ADR-0059); see [`Self::start_with_options`]. `checked` is
    /// the folder `req.checkout` names, held by the caller from before it checked that folder until after (F1012); it is held here
    /// when not given. The agent starts in that folder, never in whatever the path names by then.
    #[allow(clippy::too_many_arguments)] // each is one part of the launch the caller checked
    pub fn start_selected_with_options(
        &mut self,
        req: &StartRequest,
        project: agora_core::ProjectId,
        git_root: &Path,
        separate: bool,
        options: &Requested,
        now: Instant,
        checked: Option<agora_platform::HeldFolder>,
    ) -> Result<AgentId, SupervisorError> {
        // Refused before anything is created.
        self.launch_choice(&req.kind, options)?;
        if !separate {
            self.refuse_protected(&req.checkout, &req.worktree_root)?;
            let folder = held(checked, &req.checkout)?;
            return self.start_directory_impl(req, project, now, true, options, Some(&folder));
        }
        self.check_limits()?;
        self.refuse_unconfinable(&req.kind)?;
        let target = req.worktree_root.join(&req.name);
        let relative = req
            .checkout
            .strip_prefix(git_root)
            .map_err(|e| SupervisorError::Spawn(e.to_string()))?;
        let branch = format!("agent/{}", req.name);
        let folder = held(checked, &req.checkout)?;
        if !folder.in_place() {
            return Err(SupervisorError::FolderMoved);
        }
        let created = git::create_start_worktree(git_root, &target, &branch, None)?;
        let mut direct = req.clone();
        // F1012: the new worktree is made from the checkout by its path; one that moved meanwhile may have been another project's.
        let result = (if folder.in_place() {
            selected_worktree_directory(&target, relative)
                .map_err(|e| SupervisorError::Spawn(e.to_string()))
        } else {
            Err(SupervisorError::FolderMoved)
        })
        .and_then(|cwd| {
            direct.checkout = cwd;
            self.start_directory_impl(&direct, project, now, false, options, None)
        });
        match result {
            Ok(id) => Ok(id),
            Err(e) => {
                let _ = created.rollback();
                Err(e)
            }
        }
    }

    /// Start in a user-owned directory, without creating, resetting or removing Git state.
    pub fn start_in_directory(
        &mut self,
        req: &StartRequest,
        project: agora_core::ProjectId,
        now: Instant,
    ) -> Result<AgentId, SupervisorError> {
        self.start_directory_impl(req, project, now, true, &Requested::default(), None)
    }

    /// `folder`: the selected folder, held since it was checked (F1012); the agent is started in it.
    fn start_directory_impl(
        &mut self,
        req: &StartRequest,
        project: agora_core::ProjectId,
        now: Instant,
        private: bool,
        options: &Requested,
        folder: Option<&agora_platform::HeldFolder>,
    ) -> Result<AgentId, SupervisorError> {
        self.check_limits()?;
        let choice = self.launch_choice(&req.kind, options)?;
        self.refuse_unconfinable(&req.kind)?;
        let kind = self
            .kinds
            .get(&req.kind)
            .ok_or_else(|| SupervisorError::UnknownKind(req.kind.clone()))?;
        let driver = kind.manifest.driver(&bridge_program()).map(|mut spec| {
            spec.command.extend(choice.args.iter().cloned());
            spec
        });
        let argv = kind.manifest.process.launch.clone();
        let id = AgentId::new(format!("agent-{}", self.next_id));
        self.next_id += 1;
        let worktree = req.checkout.clone();
        if let Some(spec) = driver {
            if spec.command.is_empty() {
                return Err(SupervisorError::UnknownKind(req.kind.clone()));
            }
            if let Some(p) = agora_adapters::package::revoked_publisher_of(
                &agora_adapters::package::agora_home(),
                &spec.command[0],
            ) {
                return Err(SupervisorError::PublisherRevoked(req.kind.clone(), p));
            }
            let root =
                git::discover_checkout(&worktree).map_or_else(|_| worktree.clone(), |c| c.root);
            let mesh = self.adapter_mesh(&id, &req.kind, &project.to_string(), &worktree);
            // F1012: an adapter is started, and given its folder, by path; the folder must be where it was held before and after.
            let launched = self
                .adapter_sandbox(&req.kind, &worktree, &spec.command[0])
                .and_then(|(profile, file)| {
                    let runner = self.runner_folder(&req.kind, &id)?;
                    launch_adapter(
                        &id,
                        self.holder.as_ref(),
                        &spec,
                        &root,
                        &worktree,
                        None,
                        profile,
                        mesh.as_ref(),
                        self.credential_vars(&req.kind),
                        folder,
                        file,
                        runner.as_deref(),
                        self.sandbox.enforceable(),
                    )
                })
                .map_err(|e| self.program_missing(&req.kind, e));
            let (adapter, session_id, capabilities, git_config, confinement, _) = match launched {
                Ok(l) => l,
                Err(e) => {
                    if let Some(hook) = &self.abort_hook {
                        hook(&id);
                    }
                    return Err(e);
                }
            };
            let mut agent = Agent::new(
                id.clone(),
                project,
                self.cfg.node.clone(),
                None,
                req.kind.clone(),
                IntegrationLevel::L3Adapter,
            );
            agent.session = Some(session_id.clone());
            self.adapters.insert(
                id.clone(),
                AdapterEntry {
                    agent,
                    adapter: Some(adapter),
                    session_id,
                    worktree,
                    branch: String::new(),
                    running: true,
                    spec,
                    checkout_root: root,
                    restarts: 0,
                    restart_at: None,
                    capabilities,
                    options: SessionOptions::default(),
                    rich: AdapterRich::default(),
                    mesh,
                    launch: choice,
                    folder: folder.map(|f| f.path().to_path_buf()),
                    git_config,
                    confinement,
                    replayed: Vec::new(),
                },
            );
        } else {
            if argv.is_empty() {
                return Err(SupervisorError::UnknownKind(req.kind.clone()));
            }
            let private_config =
                private.then(|| req.worktree_root.join(".sessions").join(&req.name));
            let extras = match self.inject(
                &id,
                &req.kind,
                &project.to_string(),
                &worktree,
                private_config.as_deref(),
            ) {
                Ok(e) => Extras {
                    options: choice.args.clone(),
                    ..e
                },
                Err(e) => {
                    if let Some(hook) = &self.abort_hook {
                        hook(&id);
                    }
                    return Err(e);
                }
            };
            let (terminal, session) =
                match self.first_launch(&id, &req.kind, &argv, &worktree, &extras, folder) {
                    Ok(t) => t,
                    Err(e) => {
                        if let Some(hook) = &self.abort_hook {
                            hook(&id);
                        }
                        return Err(e);
                    }
                };
            let mut agent = Agent::new(
                id.clone(),
                project,
                self.cfg.node.clone(),
                None,
                req.kind.clone(),
                IntegrationLevel::L1Manifest,
            );
            agent.session = session;
            self.agents.insert(
                id.clone(),
                Entry {
                    agent,
                    worktree,
                    branch: String::new(),
                    terminal: Some(terminal),
                    pacer: Pacer::default(),
                    next_check: now,
                    visible: true,
                    restarts: 0,
                    restart_at: None,
                    gave_up: false,
                    extras,
                    private_config,
                    carry_on: false,
                    carried_on: false,
                    native_since: None,
                    fresh_next: false,
                    launch: choice,
                    folder: folder.map(|f| f.path().to_path_buf()),
                },
            );
        }
        Ok(id)
    }

    /// Registers the agent with the mesh through the launch hook and returns the endpoint it is to be given; `None` without a hook or
    /// when the mesh could not serve it (the mesh is an addition: an agent never fails to start because of it).
    fn join_mesh(
        &self,
        id: &AgentId,
        kind: &str,
        project: &str,
        worktree: &Path,
        identity: Option<String>,
    ) -> Option<agora_adapters::inject::Endpoint> {
        let hook = self.hook.as_ref()?;
        hook(&Launching {
            id: id.clone(),
            kind: kind.to_string(),
            project: project.to_string(),
            node: self.cfg.node.as_str().to_string(),
            worktree: worktree.to_path_buf(),
            identity,
        })
    }

    /// The mesh endpoint of an adapter agent, passed to it in `session/start`. No confined identity: the adapter process is not
    /// started with one (on Windows, the only system that has them, a confined adapter cannot be started at all yet).
    fn adapter_mesh(
        &self,
        id: &AgentId,
        kind: &str,
        project: &str,
        worktree: &Path,
    ) -> Option<agora_adapters::inject::Endpoint> {
        self.join_mesh(id, kind, project, worktree, None)
    }

    /// Gives the agent its mesh MCP server as its manifest says (`[mcp]`): asks the hook for an endpoint, writes
    /// the config file inside the worktree, and returns the arguments and variables to add at every launch.
    fn inject(
        &self,
        id: &AgentId,
        kind: &str,
        project: &str,
        worktree: &std::path::Path,
        private_config: Option<&Path>,
    ) -> Result<Extras, SupervisorError> {
        // Made before the endpoint, so the endpoint can admit exactly the process that will hold it.
        let identity = if self.sandbox.level == crate::sandbox::Level::Off {
            None
        } else {
            agora_platform::new_agent_identity()
        };
        let none = || Extras {
            identity: identity.clone(),
            ..Extras::default()
        };
        if self.hook.is_none() {
            return Ok(none());
        }
        // The hook registers the agent with the mesh even when its manifest has no `[mcp]` (it may reach the mesh
        // some other way); only a manifest with `[mcp]` gets anything injected.
        let endpoint = self.join_mesh(id, kind, project, worktree, identity.clone());
        let socket = endpoint.as_ref().map(|e| e.socket.clone());
        let (Some(endpoint), Some(spec)) = (
            endpoint,
            self.kinds.get(kind).and_then(|k| k.manifest.mcp.clone()),
        ) else {
            return Ok(Extras { socket, ..none() });
        };
        let mut plan = agora_adapters::inject::plan(&spec, worktree, &endpoint)
            .map_err(|e| SupervisorError::Spawn(format!("mcp injection: {e}")))?;
        if let Some(config_root) = private_config {
            if !plan.files.is_empty()
                && spec.method == agora_adapters::manifest::McpMethod::ConfigFile
            {
                return Err(SupervisorError::DirectoryUnsupported(kind.to_string()));
            }
            std::fs::create_dir_all(config_root)
                .map_err(|e| SupervisorError::Spawn(e.to_string()))?;
            for (path, contents) in &plan.files {
                let relative = path
                    .strip_prefix(worktree)
                    .map_err(|e| SupervisorError::Spawn(e.to_string()))?;
                let target = config_root.join(relative);
                write_in_worktree(config_root, &target, contents)
                    .map_err(|e| SupervisorError::Spawn(e.to_string()))?;
                let (old, new) = (path.to_string_lossy(), target.to_string_lossy());
                for arg in &mut plan.argv_extra {
                    *arg = arg.replace(old.as_ref(), new.as_ref());
                }
                for (_, value) in &mut plan.env {
                    *value = value.replace(old.as_ref(), new.as_ref());
                }
            }
        } else {
            for (path, contents) in &plan.files {
                write_in_worktree(worktree, path, contents)
                    .map_err(|e| SupervisorError::Spawn(e.to_string()))?;
            }
        }
        // #896: the mesh bridge the agent is told to run is the node's own executable, which may sit in a folder only the user may
        // open (on Windows `%LOCALAPPDATA%\Programs\Agora`). A confined agent may read and run that one file, nothing beside it.
        let read = private_config
            .map(Path::to_path_buf)
            .into_iter()
            .chain(std::iter::once(endpoint.command.clone()))
            .collect();
        Ok(Extras {
            read,
            argv: plan.argv_extra,
            env: plan.env,
            identity,
            socket,
            options: Vec::new(),
        })
    }

    /// Starts an agent through its L3 adapter: own worktree, adapter process, `initialize`, `session/start`.
    fn start_adapter(
        &mut self,
        req: &StartRequest,
        spec: &agora_adapters::manifest::AdapterSpec,
        launch: LaunchChoice,
    ) -> Result<AgentId, SupervisorError> {
        let checkout = git::discover_checkout(&req.checkout)?;
        let worktree = req.worktree_root.join(&req.name);
        let branch = format!("agent/{}", req.name);
        if spec.command.is_empty() {
            return Err(SupervisorError::UnknownKind(req.kind.clone()));
        }
        let created =
            git::create_start_worktree(&checkout.root, &worktree, &branch, req.base.as_deref())?;
        let id = AgentId::new(format!("agent-{}", self.next_id));
        self.next_id += 1;
        let mesh = self.adapter_mesh(&id, &req.kind, &checkout.project.to_string(), &worktree);
        let result = self
            .adapter_sandbox(&req.kind, &worktree, &spec.command[0])
            .and_then(|(profile, file)| {
                let runner = self.runner_folder(&req.kind, &id)?;
                launch_adapter(
                    &id,
                    self.holder.as_ref(),
                    spec,
                    &checkout.root,
                    &worktree,
                    None,
                    profile,
                    mesh.as_ref(),
                    self.credential_vars(&req.kind),
                    None,
                    file,
                    runner.as_deref(),
                    self.sandbox.enforceable(),
                )
            })
            .map_err(|e| self.program_missing(&req.kind, e));
        let (adapter, session_id, capabilities, git_config, confinement, _) = match result {
            Ok(started) => started,
            Err(error) => {
                self.abandon(&id, created);
                return Err(error);
            }
        };
        let mut agent = Agent::new(
            id.clone(),
            checkout.project,
            self.cfg.node.clone(),
            Some(WorktreeId::new(worktree.to_string_lossy())),
            req.kind.clone(),
            IntegrationLevel::L3Adapter,
        );
        agent.session = Some(session_id.clone());
        self.adapters.insert(
            id.clone(),
            AdapterEntry {
                agent,
                adapter: Some(adapter),
                session_id,
                worktree,
                branch,
                running: true,
                spec: spec.clone(),
                checkout_root: checkout.root,
                restarts: 0,
                restart_at: None,
                capabilities,
                options: SessionOptions::default(),
                rich: AdapterRich::default(),
                mesh,
                launch,
                folder: None,
                git_config,
                confinement,
                replayed: Vec::new(),
            },
        );
        Ok(id)
    }

    /// Drains what adapters said since the last call: states are applied (a report always wins over anything
    /// else), the rest is passed on as events. A dead adapter ends its agent as `unknown`. `letting_go` ([`Self::let_go_held`]): only
    /// the adapters the holder runs, with no restart and no queued prompt sent.
    fn poll_adapters(&mut self, now: Instant, events: &mut Vec<SupervisorEvent>, letting_go: bool) {
        let policy = self.cfg.restart;
        let due: Vec<AgentId> = self
            .adapters
            .iter()
            .filter(|(_, e)| !letting_go && e.restart_at.is_some_and(|t| now >= t))
            .map(|(id, _)| id.clone())
            .collect();
        for id in due {
            self.restart_adapter(&id, now, events);
        }
        let sent = Arc::clone(&self.sent);
        // ADR-0090: a queued prompt that could not be sent is dropped (its entry says so) and the next one goes.
        let done: Vec<(AgentId, u64, bool, u64)> =
            std::mem::take(&mut *sent.lock().unwrap_or_else(|e| e.into_inner()));
        for (id, tag, ok, generation) in done {
            // Each tag is reported once, when its send ended: sent, or not (its entry then leaves with a notice).
            match self.adapters.get_mut(&id) {
                Some(e) => {
                    let (was_sent, next) = e.rich.send_result(ok, generation, now);
                    events.push(SupervisorEvent::PromptDequeued {
                        agent: id.clone(),
                        tag,
                        sent: was_sent,
                    });
                    if let Some(next) = next.filter(|_| !letting_go) {
                        begin_send(e, &id, next, &sent, events);
                    }
                }
                None => events.push(SupervisorEvent::PromptDequeued {
                    agent: id.clone(),
                    tag,
                    sent: ok,
                }),
            }
        }
        for (id, e) in &mut self.adapters {
            if letting_go && !e.adapter.as_ref().is_some_and(Adapter::is_held) {
                continue;
            }
            // ADR-0090: a cancelled turn the adapter never ended is over after the grace.
            if let Some(next) = e.rich.expire(now).filter(|_| !letting_go) {
                begin_send(e, id, next, &sent, events);
            }
            // #933: an adapter whose output ended keeps its launch's Git file until its process is collected (a look that never waits;
            // only an ended adapter that still has a file is asked).
            if !e.running
                && e.git_config.is_some()
                && e.adapter
                    .as_mut()
                    .is_none_or(|a| a.exit_status(Duration::ZERO).is_some())
            {
                e.git_config = None;
            }
            let polled = match &e.adapter {
                Some(adapter) if e.running => {
                    // A resumed session's replay first: its chat as one event, the rest as if it had just arrived.
                    let early = std::mem::take(&mut e.replayed);
                    let mut items = Vec::new();
                    let mut rest = Vec::new();
                    for ev in early {
                        match ev {
                            AdapterEvent::Chat { item, .. } => items.push(item),
                            other => rest.push(other),
                        }
                    }
                    if !items.is_empty() {
                        events.push(SupervisorEvent::ReplayedChat {
                            agent: id.clone(),
                            items,
                        });
                    }
                    rest.extend(adapter.poll());
                    rest
                }
                _ => continue,
            };
            // ADR-0093: the holder kept only the newest of what the adapter wrote while no node read it: the person is told.
            let lost = e.adapter.as_ref().map_or(0, Adapter::take_dropped);
            for ev in polled {
                match ev {
                    AdapterEvent::State { state, .. } => {
                        // ADR-0090: an adapter that reports its turns (`turns`) tells when one is open; the next queued prompt goes
                        // out when it ends, one per end.
                        if e.capabilities.iter().any(|c| c == "turns") {
                            if let Some(next) = e.rich.observe(state, now).filter(|_| !letting_go) {
                                begin_send(e, id, next, &sent, events);
                            }
                        }
                        if e.agent.observe(state, StateSource::Reported) {
                            events.push(SupervisorEvent::StateChanged {
                                agent: id.clone(),
                                state,
                            });
                        }
                    }
                    AdapterEvent::Options { options, .. } => {
                        let options = bounded_options(options);
                        if options != e.options {
                            e.options = options.clone();
                            events.push(SupervisorEvent::Adapter {
                                agent: id.clone(),
                                event: AdapterEvent::Options {
                                    session_id: e.session_id.clone(),
                                    options,
                                },
                            });
                        }
                    }
                    AdapterEvent::Config { options, .. } => {
                        let options = bounded_config(options);
                        if options != e.rich.config {
                            e.rich.config = options.clone();
                            events.push(SupervisorEvent::Adapter {
                                agent: id.clone(),
                                event: AdapterEvent::Config {
                                    session_id: e.session_id.clone(),
                                    options,
                                },
                            });
                        }
                    }
                    AdapterEvent::Commands { commands, .. } => {
                        let commands = bounded_commands(commands);
                        if commands != e.rich.commands {
                            e.rich.commands = commands.clone();
                            events.push(SupervisorEvent::Adapter {
                                agent: id.clone(),
                                event: AdapterEvent::Commands {
                                    session_id: e.session_id.clone(),
                                    commands,
                                },
                            });
                        }
                    }
                    AdapterEvent::SessionReported {
                        native_session_id, ..
                    } => {
                        e.agent.session = Some(native_session_id.clone());
                        events.push(SupervisorEvent::Adapter {
                            agent: id.clone(),
                            event: AdapterEvent::SessionReported {
                                session_id: e.session_id.clone(),
                                native_session_id,
                            },
                        });
                    }
                    AdapterEvent::Exited => {
                        e.running = false;
                        e.options = SessionOptions::default();
                        let dropped = e.rich.forget();
                        if !dropped.is_empty() {
                            events.push(SupervisorEvent::PromptsDropped {
                                agent: id.clone(),
                                tags: dropped,
                            });
                        }
                        if e.agent.observe(AgentState::Unknown, StateSource::Reported) {
                            events.push(SupervisorEvent::StateChanged {
                                agent: id.clone(),
                                state: AgentState::Unknown,
                            });
                        }
                        // #902: how its process ended, for the end the node reports if it gives up on it now. A crash path only
                        // (rare), and at most 50 ms under this lock; a held adapter's is what its output's reader heard from the holder, never a question.
                        let status = e
                            .adapter
                            .as_mut()
                            .and_then(|a| a.exit_status(Duration::from_millis(50)));
                        // #933: `Exited` is the end of its output, not of its process: the launch's Git file goes once the process is
                        // collected (here, or by a later poll, or when a restart, a stop or the node's end lets it go).
                        if status.is_some() {
                            e.git_config = None;
                        }
                        events.push(SupervisorEvent::AdapterCrashed { agent: id.clone() });
                        schedule_adapter_restart(id, e, now, &policy, status, events);
                    }
                    other => events.push(SupervisorEvent::Adapter {
                        agent: id.clone(),
                        event: other,
                    }),
                }
            }
            // After what the adapter said, so the notice is the newest entry and a long reply cannot push it out of the conversation.
            if lost > 0 {
                events.push(SupervisorEvent::OutputDropped {
                    agent: id.clone(),
                    bytes: lost,
                });
            }
        }
    }

    /// Starts a crashed adapter again and resumes its session where the adapter supports it. A failed attempt
    /// counts and backs off like a crash.
    fn restart_adapter(&mut self, id: &AgentId, now: Instant, events: &mut Vec<SupervisorEvent>) {
        let policy = self.cfg.restart;
        // F1023: a selected folder is held again; an adapter is started, and given its folder, by path, so the folder must be where
        // it was held right before the start and after it (as at its first start, F1012; `launch_adapter`).
        let held = self
            .adapters
            .get(id)
            .and_then(|e| e.folder.as_deref())
            .map(|p| held_again(None, p))
            .transpose();
        // #933: the new launch's Git file is removed with the attempt when it fails (`launched` is dropped).
        let Some(profile) = self.adapters.get(id).and_then(|e| {
            Some(self.adapter_sandbox(&e.agent.kind, &e.worktree, e.spec.command.first()?))
        }) else {
            return;
        };
        let creds = self
            .adapters
            .get(id)
            .map(|e| self.credential_vars(&e.agent.kind))
            .unwrap_or_default();
        // #1150: a new folder for this start, as at the first (the one the process that ended used is removed off this thread).
        let runner = self
            .adapters
            .get(id)
            .map(|e| self.runner_folder(&e.agent.kind, id))
            .transpose()
            .map(Option::flatten);
        let holder = self.holder.clone();
        let kept_launch = self.kept_launch.contains(id);
        let enforceable = self.sandbox.enforceable();
        let Some(e) = self.adapters.get_mut(id) else {
            return;
        };
        e.restart_at = None;
        e.restarts += 1;
        // The process that ended is let go before the next starts: a held one is collected from the holder, which frees its id.
        e.adapter = None;
        e.git_config = None;
        // A crash keeps the agent in the mesh: the restarted session gets the same endpoint.
        let launched = held.and_then(|held| {
            let (profile, file) = profile?;
            let runner = runner?;
            launch_adapter(
                id,
                holder.as_ref(),
                &e.spec,
                &e.checkout_root,
                &e.worktree,
                e.agent.session.as_deref(),
                profile,
                e.mesh.as_ref(),
                creds,
                held.as_ref(),
                file,
                runner.as_deref(),
                enforceable,
            )
        });
        match launched {
            Ok((adapter, session_id, capabilities, git_config, confinement, replayed)) => {
                e.adapter = Some(adapter);
                // Review of #1103 (B1): what a resumed session replayed before it answered, read before anything newer.
                e.replayed = replayed;
                e.confinement = confinement;
                e.git_config = git_config;
                e.session_id = session_id;
                e.capabilities = capabilities;
                // What the old process reported does not carry over: a mode that skips permissions is checked against what the new
                // one reports.
                e.options = SessionOptions::default();
                e.rich.forget();
                e.running = true;
                e.agent.process_restarted();
                // As for a terminal agent (`restart`): one taken up from the holder restarts without its options' arguments.
                if kept_launch {
                    e.launch = LaunchChoice::default();
                    self.kept_launch.remove(id);
                }
                events.push(SupervisorEvent::Started { agent: id.clone() });
                events.extend(session_report(&e.agent));
            }
            Err(_) => schedule_adapter_restart(id, e, now, &policy, None, events),
        }
    }

    fn adapter_of(&self, id: &AgentId) -> Result<&AdapterEntry, SupervisorError> {
        self.adapters
            .get(id)
            .filter(|e| e.running && e.adapter.is_some())
            .ok_or_else(|| SupervisorError::NotFound(id.clone()))
    }

    fn call_of(&self, id: &AgentId, op: CallOp) -> Result<AdapterCall, SupervisorError> {
        let e = self.adapter_of(id)?;
        Ok(AdapterCall {
            caller: e
                .adapter
                .as_ref()
                .ok_or_else(|| SupervisorError::NotFound(id.clone()))?
                .caller(),
            session: e.session_id.clone(),
            op,
            generation: e.rich.generation,
            epoch: e.rich.epoch,
        })
    }

    /// The `skip_permissions` policy's refusal of a MODEL that makes an agent act without asking (modes and config options are asked
    /// about instead, [`skip_ask`](Self::skip_ask)).
    ///
    /// A launch option needs the user's confirmation (`Requested::confirmed`, signed). A model has no approval card (ADR-0090
    /// Amendment A names modes and config options), so a model that skips is refused with `agent.option_unconfirmed`, after the policy
    /// (`agent.option_forbidden`) has had its say: by the agent's own launch ([`policy_refusal`], #1009), never the node's sandbox
    /// setting now.
    fn skip_refusal(&self, e: &AdapterEntry, option: &str) -> SupervisorError {
        let kind = e.agent.kind.clone();
        match policy_refusal(self.skip_permissions, e.confinement) {
            Some(policy) => SupervisorError::OptionForbidden(kind, option.to_string(), policy),
            None => SupervisorError::OptionUnconfirmed(kind, option.to_string()),
        }
    }

    /// A prompt to an adapter agent without the queue (mesh content uses `from_mesh`). The call is made by [`AdapterCall::run`],
    /// after the lock is let go.
    pub fn prepare_prompt(
        &self,
        id: &AgentId,
        text: &str,
        from_mesh: bool,
    ) -> Result<AdapterCall, SupervisorError> {
        self.call_of(
            id,
            CallOp::Prompt {
                text: text.to_string(),
                from_mesh,
            },
        )
    }

    /// [`prepare_prompt`](Self::prepare_prompt) and run, under the caller's lock: for callers without one to let go (tests).
    pub fn adapter_prompt(
        &self,
        id: &AgentId,
        text: &str,
        from_mesh: bool,
    ) -> Result<(), SupervisorError> {
        self.prepare_prompt(id, text, from_mesh)?.run()
    }

    /// ADR-0090: a user's prompt to an adapter agent: to be sent now ([`Submit::Send`], made outside the lock), or queued behind the
    /// turn that runs (an adapter that reports its turns declares `turns`; one that does not is sent to at once, as before). `tag` is
    /// called only for a prompt that is queued and returns what identifies it later (the id of its chat entry).
    pub fn adapter_submit(
        &mut self,
        id: &AgentId,
        text: &str,
        tag: impl FnOnce(usize) -> u64,
    ) -> Result<Submit, SupervisorError> {
        self.adapter_of(id)?;
        let Some(e) = self.adapters.get_mut(id) else {
            return Err(SupervisorError::NotFound(id.clone()));
        };
        let turns = e.capabilities.iter().any(|c| c == "turns");
        match e.rich.route(turns, text, Instant::now(), tag)? {
            Route::Queued(queue_len) => Ok(Submit::Queued { queue_len }),
            Route::Send(generation) => match self.prepare_prompt(id, text, false) {
                Ok(call) => Ok(Submit::Send(call)),
                Err(err) => {
                    // The prompt will not go out: the turn that was just marked busy never began.
                    self.adapter_send_failed(id, generation);
                    Err(err)
                }
            },
        }
    }

    /// ADR-0090: the call of a [`Submit::Send`] failed: its turn never began, so the agent is free (and what waits goes next, at
    /// the next poll's `idle`: nothing waits when a prompt was sent at once).
    /// `generation` is the call's ([`AdapterCall::generation`]): a failure that comes back after the adapter restarted, or after
    /// another turn began, changes nothing.
    pub fn adapter_send_failed(&mut self, id: &AgentId, generation: u64) {
        if let Some(e) = self.adapters.get_mut(id) {
            let _ = e.rich.send_result(false, generation, Instant::now());
        }
    }

    /// ADR-0090: ends an adapter agent's current turn (the adapter's `session/cancel`); the agent and its process live on. Only an
    /// adapter that declared `cancel` has this; a terminal agent has no turn to cancel. Run the call outside the lock, then
    /// [`adapter_turn_cancelled`](Self::adapter_turn_cancelled).
    pub fn prepare_cancel_turn(&self, id: &AgentId) -> Result<AdapterCall, SupervisorError> {
        const CAPABILITY: &str = "agent.cancel_turn";
        let Some(e) = self.adapters.get(id) else {
            return Err(if self.agents.contains_key(id) {
                SupervisorError::OptionUnsupported(CAPABILITY)
            } else {
                SupervisorError::NotFound(id.clone())
            });
        };
        if !e.running {
            return Err(SupervisorError::NotFound(id.clone()));
        }
        if !e.capabilities.iter().any(|c| c == "cancel") {
            return Err(SupervisorError::OptionUnsupported(CAPABILITY));
        }
        self.call_of(id, CallOp::Cancel)
    }

    /// ADR-0090: the adapter took the cancel. With `drop_queue` the prompts waiting behind the turn are thrown away and their tags
    /// returned; otherwise they are kept and sent in order as the turns end. The turn is read as over after [`CANCEL_GRACE`] if the
    /// adapter does not say so.
    ///
    /// `generation` is the cancel call's ([`AdapterCall::generation`]): one that comes back after the adapter restarted, or after
    /// the turn it was for ended and another began, does nothing (nothing is dropped and no clock is set on the newer turn).
    pub fn adapter_turn_cancelled(
        &mut self,
        id: &AgentId,
        generation: u64,
        drop_queue: bool,
    ) -> Vec<u64> {
        let now = Instant::now();
        self.adapters
            .get_mut(id)
            .map(|e| e.rich.cancelled(generation, now, drop_queue))
            .unwrap_or_default()
    }

    /// ADR-0090: sets one of the config options an adapter agent reported (the reasoning effort is one). The id and the value are
    /// checked against the last report. Whether the choice makes the agent act without asking is the node's own judgement
    /// (`perm`, on top of what the adapter said); one that does is refused by the owner's `skip_permissions` policy
    /// (`agent.option_forbidden`) and otherwise NOT applied: it becomes an approval the owner signs ([`Prepared::Ask`], ADR-0090
    /// Amendment A), whatever the window or the hub said. A change to the value that keeps protection (turning an `auto_approve`
    /// off) is never asked about. The call is made by [`AdapterCall::run`] after the lock is let go.
    pub fn prepare_set_config(
        &self,
        id: &AgentId,
        config_id: &str,
        value: &str,
    ) -> Result<Prepared, SupervisorError> {
        const CAPABILITY: &str = "agent.set_config";
        let Some(e) = self.adapters.get(id) else {
            return Err(if self.agents.contains_key(id) {
                SupervisorError::OptionUnsupported(CAPABILITY)
            } else {
                SupervisorError::NotFound(id.clone())
            });
        };
        if !e.running {
            return Err(SupervisorError::NotFound(id.clone()));
        }
        if !e.capabilities.iter().any(|c| c == "config") {
            return Err(SupervisorError::OptionUnsupported(CAPABILITY));
        }
        let Some(option) = e
            .rich
            .config
            .iter()
            .find(|o| !config_id.is_empty() && o.id == config_id)
        else {
            return Err(SupervisorError::ConfigUnknown(config_id.to_string()));
        };
        let (skips, value_name) = if option.boolean {
            let on = match value {
                "true" => true,
                "false" => false,
                _ => return Err(SupervisorError::ConfigUnknown(value.to_string())),
            };
            let skips = perm::boolean_value_skips(
                Category::from_acp(Some(option.category.as_str())),
                (&option.id, &option.name),
                &option.description,
                option.skips_permissions,
                on,
            );
            (skips, String::new())
        } else {
            let Some(choice) = option
                .choices
                .iter()
                .find(|c| !value.is_empty() && c.id == value)
            else {
                return Err(SupervisorError::ConfigUnknown(value.to_string()));
            };
            (choice.skips_permissions, choice.name.clone())
        };
        if skips {
            return self.skip_ask(
                e,
                SkipKind::Config,
                (config_id, &option.name),
                (value, &value_name),
            );
        }
        self.call_of(
            id,
            CallOp::SetConfig {
                id: config_id.to_string(),
                value: value.to_string(),
            },
        )
        .map(Prepared::Call)
    }

    /// ADR-0090 Amendment A: the choice skips permissions. Under a policy that does not allow it the answer is the refusal of the policy
    /// (`agent.option_forbidden`, nothing is asked); under one that does, the node does not apply it and does not refuse it: it
    /// describes it for the owner's approval ([`Prepared::Ask`]), with the agent, folder, option and value as the node knows them.
    /// The policy reads the agent's own launch ([`policy_refusal`], #1009): an agent the holder runs is never confined, also under a
    /// node that confines now, and one with nothing recorded fails closed.
    fn skip_ask(
        &self,
        e: &AdapterEntry,
        kind: SkipKind,
        option: (&str, &str),
        value: (&str, &str),
    ) -> Result<Prepared, SupervisorError> {
        let (option_id, option_name) = option;
        let (value_id, value_name) = value;
        // A mode's id is the value, as before; a config option's is its own.
        let refused = match kind {
            SkipKind::Mode => value_id,
            SkipKind::Config => option_id,
        };
        if let Some(policy) = policy_refusal(self.skip_permissions, e.confinement) {
            return Err(SupervisorError::OptionForbidden(
                e.agent.kind.clone(),
                refused.to_string(),
                policy,
            ));
        }
        Ok(Prepared::Ask(SkipAsk {
            agent: e.agent.id.clone(),
            agent_kind: e.agent.kind.clone(),
            folder: e
                .folder
                .as_deref()
                .unwrap_or(&e.worktree)
                .to_string_lossy()
                .into_owned(),
            kind,
            option_id: option_id.to_string(),
            option_name: option_name.to_string(),
            value: value_id.to_string(),
            value_name: value_name.to_string(),
            epoch: e.rich.epoch,
        }))
    }

    /// ADR-0090 Amendment A: applies a switch the owner approved, exactly as the node rendered it (`ask` is the node's own record, never
    /// anything the decision carried). Everything that could have changed while the card waited is checked again: the agent still runs
    /// in the run the card named, the agent still offers that option and value, and the policy still allows it. The call is made by
    /// [`AdapterCall::run`] after the lock is let go.
    pub fn prepare_skip_apply(&self, ask: &SkipAsk) -> Result<AdapterCall, SupervisorError> {
        let id = &ask.agent;
        let Some(e) = self.adapters.get(id) else {
            return Err(SupervisorError::NotFound(id.clone()));
        };
        if !e.running || e.rich.epoch != ask.epoch {
            return Err(SupervisorError::NotFound(id.clone()));
        }
        // The policy by the agent's launch as it is now (#1009): a card raised for a run that was confined is not applied to one that
        // is not (a replaced run has another epoch and is refused above).
        if let Some(policy) = policy_refusal(self.skip_permissions, e.confinement) {
            return Err(SupervisorError::OptionForbidden(
                e.agent.kind.clone(),
                // The id `skip_ask` names (#1033): a mode's is the value, a config option's its own.
                match ask.kind {
                    SkipKind::Mode => ask.value.clone(),
                    SkipKind::Config => ask.option_id.clone(),
                },
                policy,
            ));
        }
        match ask.kind {
            SkipKind::Mode => {
                if !e
                    .capabilities
                    .iter()
                    .any(|c| c == OptionKind::Mode.capability())
                {
                    return Err(SupervisorError::OptionUnsupported("agent.set_mode"));
                }
                if !e.options.modes.iter().any(|o| o.id == ask.value) {
                    return Err(SupervisorError::OptionUnknown("mode"));
                }
                self.call_of(
                    id,
                    CallOp::SetOption {
                        kind: OptionKind::Mode,
                        value: ask.value.clone(),
                    },
                )
            }
            SkipKind::Config => {
                if !e.capabilities.iter().any(|c| c == "config") {
                    return Err(SupervisorError::OptionUnsupported("agent.set_config"));
                }
                let offered = e.rich.config.iter().any(|o| {
                    o.id == ask.option_id
                        && if o.boolean {
                            ask.value == "true" || ask.value == "false"
                        } else {
                            o.choices.iter().any(|c| c.id == ask.value)
                        }
                });
                if !offered {
                    return Err(SupervisorError::ConfigUnknown(ask.option_id.clone()));
                }
                self.call_of(
                    id,
                    CallOp::SetConfig {
                        id: ask.option_id.clone(),
                        value: ask.value.clone(),
                    },
                )
            }
        }
    }

    /// ADR-0090 Amendment A: the ids of the select config options of category `mode` an adapter agent reported. An agent that gives its
    /// mode as such an option (ACP config options: the bridge builds the mode list from it and sets a mode through it) has ONE setting
    /// behind `AgentSetMode` and `AgentSetConfig` of that id, so a switch through either settles the waiting cards of both (F1322).
    pub fn mode_config_ids(&self, id: &AgentId) -> Vec<String> {
        self.adapters
            .get(id)
            .map(|e| {
                e.rich
                    .config
                    .iter()
                    .filter(|o| !o.boolean && o.category == "mode")
                    .map(|o| o.id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// [`prepare_set_config`](Self::prepare_set_config) and run, under the caller's lock (tests).
    ///
    /// `Ok(Some(ask))`: the choice skips permissions and waits for the owner's approval; nothing was applied.
    pub fn adapter_set_config(
        &self,
        id: &AgentId,
        config_id: &str,
        value: &str,
    ) -> Result<Option<SkipAsk>, SupervisorError> {
        match self.prepare_set_config(id, config_id, value)? {
            Prepared::Call(call) => call.run().map(|()| None),
            Prepared::Ask(ask) => Ok(Some(ask)),
        }
    }

    /// ADR-0058: switches an adapter agent to one of the modes or models it reported. Only the user's own command reaches this (an
    /// agent has no mesh tool for it); a terminal agent, an adapter that did not declare the switch, and an id the agent did not offer
    /// are refused before the adapter hears of it. A mode that skips permissions is refused as a config option is (see
    /// [`prepare_set_config`](Self::prepare_set_config)). The call is made by [`AdapterCall::run`] after the lock is let go.
    pub fn prepare_set_option(
        &self,
        id: &AgentId,
        kind: OptionKind,
        value: &str,
    ) -> Result<Prepared, SupervisorError> {
        let (capability, field) = match kind {
            OptionKind::Mode => ("agent.set_mode", "mode"),
            OptionKind::Model => ("agent.set_model", "model"),
        };
        let Some(e) = self.adapters.get(id) else {
            return Err(if self.agents.contains_key(id) {
                SupervisorError::OptionUnsupported(capability)
            } else {
                SupervisorError::NotFound(id.clone())
            });
        };
        // A stopped agent keeps the modes it last reported, but it has no session to switch: nothing is applied or asked about for
        // it (F1320: a mode that skips would otherwise raise a card for an agent that ended).
        if !e.running {
            return Err(SupervisorError::NotFound(id.clone()));
        }
        if !e.capabilities.iter().any(|c| c == kind.capability()) {
            return Err(SupervisorError::OptionUnsupported(capability));
        }
        let offered = match kind {
            OptionKind::Mode => &e.options.modes,
            OptionKind::Model => &e.options.models,
        };
        let Some(chosen) = offered.iter().find(|o| !value.is_empty() && o.id == value) else {
            return Err(SupervisorError::OptionUnknown(field));
        };
        // A mode that makes the agent act without asking is a launch option's twin (ADR-0059): the owner's `skip_permissions` policy
        // decides it here, on the node, whatever the window or the hub said, and it needs the user's confirmation too.
        if chosen.skips_permissions {
            // ADR-0090 Amendment A: a mode is asked about (the owner's signed approval applies it); a model that skips stays refused.
            if kind == OptionKind::Mode {
                return self.skip_ask(e, SkipKind::Mode, ("mode", ""), (&chosen.id, &chosen.name));
            }
            return Err(self.skip_refusal(e, &chosen.id));
        }
        self.call_of(
            id,
            CallOp::SetOption {
                kind,
                value: value.to_string(),
            },
        )
        .map(Prepared::Call)
    }

    /// [`prepare_set_option`](Self::prepare_set_option) and run, under the caller's lock (tests).
    ///
    /// `Ok(Some(ask))`: the mode skips permissions and waits for the owner's approval; nothing was applied.
    pub fn adapter_set_option(
        &self,
        id: &AgentId,
        kind: OptionKind,
        value: &str,
    ) -> Result<Option<SkipAsk>, SupervisorError> {
        match self.prepare_set_option(id, kind, value)? {
            Prepared::Call(call) => call.run().map(|()| None),
            Prepared::Ask(ask) => Ok(Some(ask)),
        }
    }

    /// Delivers a decision the node has already verified (ADR-0020) to the adapter that asked: the call to make after the lock is
    /// let go.
    pub fn prepare_resolve_approval(
        &self,
        id: &AgentId,
        approval_id: &str,
        approved: bool,
    ) -> Result<AdapterCall, SupervisorError> {
        self.prepare_resolve_approval_scoped(id, approval_id, approved, false)
    }

    /// ADR-0101: [`prepare_resolve_approval`](Self::prepare_resolve_approval) with the scope the person signed (`always`). An adapter
    /// that declared `approval_scopes` is told ONCE or ALWAYS; any other is told nothing new and is never sent ALWAYS (refused here,
    /// although the node never offers it one).
    pub fn prepare_resolve_approval_scoped(
        &self,
        id: &AgentId,
        approval_id: &str,
        approved: bool,
        always: bool,
    ) -> Result<AdapterCall, SupervisorError> {
        use agora_protocol::v1::ApprovalScope;
        let strict = self.adapter_of(id)?.strict_scopes();
        let scope = match (strict, always) {
            (true, true) => Some(ApprovalScope::Always),
            (true, false) => Some(ApprovalScope::Once),
            (false, false) => None,
            (false, true) => return Err(SupervisorError::NotFound(id.clone())),
        };
        self.call_of(
            id,
            CallOp::Resolve {
                approval: approval_id.to_string(),
                approved,
                scope,
            },
        )
    }

    /// ADR-0101: whether "always" may be offered for (and given to) this agent now: the node's `skip_permissions` policy (`forbid`
    /// never, `confined_only` only for an agent whose process was launched confined, [`always_by_policy`]) and the kind's manifest
    /// (`[structured] allow_always = false`). Codex review of PR #1007: the agent's own launch is read ([`Confinement`]), never the
    /// node's sandbox setting, so a process the holder runs (unconfined) never counts as confined after a node that confines took it
    /// up, and a launch with nothing recorded fails closed.
    fn always_permitted(&self, e: &AdapterEntry) -> bool {
        always_by_policy(self.skip_permissions, e.confinement)
            && self
                .kinds
                .get(&e.agent.kind)
                .and_then(|k| k.manifest.structured.as_ref())
                .is_none_or(agora_adapters::manifest::Structured::offers_always)
    }

    /// ADR-0101: how the node raises a card for this adapter agent's request: whether its adapter declared `approval_scopes`, and
    /// whether "always" may be offered (see [`always_permitted`](Self::always_permitted)). `None` for an agent without an adapter.
    pub fn approval_raise_terms(&self, id: &AgentId) -> Option<(bool, bool)> {
        let e = self.adapters.get(id)?;
        Some((e.strict_scopes(), self.always_permitted(e)))
    }

    /// ADR-0101: the policy a decision on this agent's request is checked under, when the decision arrives: the `skip_permissions`
    /// line the node read from its policy file at start (a change to the file applies from the next node start, as for every policy
    /// line) and the agent's recorded launch ([`Confinement`]). The caller binds the call it then makes to the same run
    /// ([`adapter_epoch`](Self::adapter_epoch)). A single Allow is
    /// refused (Codex review of ADR-0101) only for an agent behind an ACP bridge that did not declare `approval_scopes` (one from
    /// before ADR-0101, kept across a node update, which may answer it with a remembered option) while "always" is not permitted.
    pub fn approval_decide(&self, id: &AgentId) -> crate::approvals::Decide {
        let Some(e) = self.adapters.get(id) else {
            return crate::approvals::Decide::default();
        };
        crate::approvals::Decide::for_adapter(
            self.always_permitted(e),
            e.strict_scopes(),
            e.behind_acp_bridge(),
        )
    }

    /// ADR-0101 (Codex review of PR #1007): the run of this adapter agent's process now (`None` for an agent without an adapter). A
    /// decision checked against the agent's launch is made only to the same run ([`AdapterCall::epoch`]).
    pub fn adapter_epoch(&self, id: &AgentId) -> Option<u64> {
        self.adapters.get(id).map(|e| e.rich.epoch)
    }

    /// ADR-0101 (Codex review of PR #1007): whether the adapter process the call of `epoch` was made to still runs as this agent. A
    /// result about the agent's process (an always answer it now holds) is recorded only while this holds, under the supervisor's
    /// lock, so it is ordered before any replacement: a restart, resume, exit or stop begins another run, and the late result of a
    /// former process never marks its replacement.
    pub fn runs(&self, id: &AgentId, epoch: u64) -> bool {
        self.adapters
            .get(id)
            .is_some_and(|e| e.running && e.rich.epoch == epoch)
    }

    /// The launch recorded for this adapter agent's process (ADR-0101), `None` for an agent without an adapter.
    pub fn confinement(&self, id: &AgentId) -> Option<Confinement> {
        self.adapters.get(id).map(|e| e.confinement)
    }

    /// [`prepare_resolve_approval`](Self::prepare_resolve_approval) and run, under the caller's lock (tests).
    pub fn adapter_resolve_approval(
        &self,
        id: &AgentId,
        approval_id: &str,
        approved: bool,
    ) -> Result<(), SupervisorError> {
        self.prepare_resolve_approval(id, approval_id, approved)?
            .run()
    }

    /// ADR-0060: an agent's first launch. A kind whose manifest has `[session] id_args` gets a session id the node chooses (a random
    /// v4 UUID), returned so the caller reports it before anything else of the agent; restarts and resumes use `[resume]` with it.
    fn first_launch(
        &self,
        id: &AgentId,
        kind: &str,
        argv: &[String],
        worktree: &Path,
        extras: &Extras,
        folder: Option<&agora_platform::HeldFolder>,
    ) -> Result<(TerminalHandle, Option<String>), SupervisorError> {
        let id_args = self
            .kinds
            .get(kind)
            .and_then(|k| k.manifest.session.as_ref())
            .map(|s| s.id_args.clone())
            .unwrap_or_default();
        if id_args.is_empty() {
            return Ok((
                self.launch_in(id, kind, argv, worktree, extras, folder)?,
                None,
            ));
        }
        let session = new_session_id().map_err(|e| SupervisorError::Spawn(e.to_string()))?;
        let mut argv = argv.to_vec();
        argv.extend(id_args.iter().map(|a| a.replace("{session_id}", &session)));
        Ok((
            self.launch_in(id, kind, &argv, worktree, extras, folder)?,
            Some(session),
        ))
    }

    /// A terminal agent's own session id, as chosen at its launch or reported (ADR-0060). `None` for an adapter agent: what it holds
    /// right after its start is the adapter's session, not the agent's own, which its adapter reports itself (`session/reported`).
    pub fn session_of(&self, id: &AgentId) -> Option<String> {
        self.agents.get(id).and_then(|e| e.agent.session.clone())
    }

    /// Every session id the node's agents have (ADR-0060): a session file naming one of these is never another agent's.
    pub fn known_sessions(&self) -> std::collections::HashSet<String> {
        self.agents
            .values()
            .map(|e| &e.agent)
            .chain(self.adapters.values().map(|e| &e.agent))
            .filter_map(|a| a.session.clone())
            .collect()
    }

    /// Terminal agents that run without a known session id and whose kind says which file names it (`[session.file]`, ADR-0060).
    pub fn session_file_targets(&self) -> Vec<crate::session_files::Target> {
        self.agents
            .iter()
            .filter(|(_, e)| e.agent.session.is_none())
            .filter_map(|(id, e)| {
                let terminal = e.terminal.as_ref()?;
                let (launch, started) = (terminal.launch(), terminal.started());
                let spec = self
                    .kinds
                    .get(&e.agent.kind)?
                    .manifest
                    .session
                    .as_ref()?
                    .file
                    .clone()?;
                Some(crate::session_files::Target {
                    agent: id.clone(),
                    launch,
                    started,
                    kind: e.agent.kind.clone(),
                    folder: e.worktree.clone(),
                    spec,
                })
            })
            .collect()
    }

    /// Starts a terminal agent's process, in `folder` when it is given (F1012): `worktree` is that folder's path.
    fn launch_in(
        &self,
        id: &AgentId,
        kind: &str,
        argv: &[String],
        worktree: &std::path::Path,
        extras: &Extras,
        folder: Option<&agora_platform::HeldFolder>,
    ) -> Result<TerminalHandle, SupervisorError> {
        // #925: taken before anything is spawned, so a file written after it may be this process's and one written before never is.
        let before = SystemTime::now();
        self.spawn_terminal(id, kind, argv, worktree, extras, folder)
            .map(|t| t.started_after(before))
    }

    fn spawn_terminal(
        &self,
        id: &AgentId,
        kind: &str,
        argv: &[String],
        worktree: &std::path::Path,
        extras: &Extras,
        folder: Option<&agora_platform::HeldFolder>,
    ) -> Result<TerminalHandle, SupervisorError> {
        let mut argv = argv.to_vec();
        // ADR-0087 item 3: a program `agora agent install` made is found first, then the user's PATH (the node's own, as before).
        if let Some(first) = argv.first_mut() {
            *first = crate::doctor::start_program(&agora_adapters::package::agora_home(), first);
        }
        argv.extend(extras.options.iter().cloned());
        argv.extend(extras.argv.iter().cloned());
        let program = argv.first().cloned();
        let mut cfg = PtyConfig::new(argv);
        cfg.cwd = Some(worktree.to_path_buf());
        cfg.cols = self.cfg.terminal.size.cols;
        cfg.rows = self.cfg.terminal.size.rows;
        cfg.limits = self.cfg.limits.process;
        cfg.sandbox = program.and_then(|p| self.sandbox_for(kind, worktree, &p));
        cfg.identity = extras.identity.clone();
        cfg.env_remove = inherited_session_markers(std::env::vars_os().map(|(k, _)| k));
        if let Some(profile) = &mut cfg.sandbox {
            profile.read.extend(extras.read.iter().cloned());
            own_socket(profile, extras.socket.as_deref());
            // ADR-0083: what points at a socket the user's session offers is not inherited by a confined agent.
            cfg.env_remove.extend(self.sandbox.scrubbed_env());
        }
        // Check the final ordinary grants before writing the file for this launch (ADR-0098). #933: the file lives exactly as long as
        // the launch: a start that fails below drops (removes) it, a started one hands it to the terminal's handle.
        let mut launch_file = None;
        if let Some(profile) = &mut cfg.sandbox {
            self.refuse_reachable(kind, profile)?;
            profile.git_config = self.sandbox.git_config(profile);
            launch_file = profile
                .git_config
                .clone()
                .map(crate::gitconfig::LaunchFile::new);
        }
        if let Some(k) = self.kinds.get(kind) {
            cfg.env = k
                .manifest
                .process
                .env
                .iter()
                .map(|(a, b)| (a.clone(), b.clone()))
                .collect();
        }
        cfg.env.extend(extras.env.iter().cloned());
        // ADR-0084: this kind's listed secrets are set (last, so nothing else sets them), and the other kinds' credential names
        // are left out of what the node inherited. Environment only: never an argument, never a file in the worktree.
        let plan = self.credential_plan(kind);
        cfg.env_remove.extend(plan.scrub);
        for (name, value) in plan.set {
            cfg.env.retain(|(k, _)| k != &name);
            cfg.env.push((name, value.expose().to_string()));
        }
        if cfg.sandbox.is_some() && agora_platform::PRIVATE_TEMP {
            let temp = crate::sandbox::agent_temp(worktree).display().to_string();
            cfg.env.push(("TMPDIR".into(), temp.clone()));
            cfg.env.push(("TEMP".into(), temp.clone()));
            cfg.env.push(("TMP".into(), temp));
        }
        // ADR-0079: an agent the node does not confine is started through the holder when this node has one, and run here when it
        // cannot be (no holder built, it cannot start, the link fails): exactly as before.
        if cfg.sandbox.is_none() {
            if let Some(started) = self.launch_held(id, &cfg, folder) {
                return started;
            }
        }
        let started = match folder {
            Some(f) => agora_platform::spawn_in(&cfg, f),
            None => spawn(&cfg),
        };
        let pty = started.map_err(|e| {
            if agora_platform::folder_moved(&e) {
                return SupervisorError::FolderMoved;
            }
            if agora_platform::batch_argument_refused(&e) {
                return SupervisorError::BatchArgumentRefused;
            }
            // A folder the confinement cannot keep closed (a broad write grant, ADR-0050 Amendment B, 4) or whose label cannot be set
            // (2) is refused before the agent starts: `agent.sandbox_unavailable` with the reason, not an internal error.
            match agora_platform::sandbox_refusal(&e) {
                Some(why) => SupervisorError::SandboxRefused(
                    kind.to_string(),
                    self.sandbox.level.name(),
                    why.token(),
                ),
                None => self.program_missing(kind, SupervisorError::Spawn(e.to_string())),
            }
        })?;
        TerminalHandle::start(pty, self.cfg.terminal)
            .map(|t| t.with_launch_file(launch_file))
            .map_err(|e| SupervisorError::Spawn(e.to_string()))
    }

    /// Starts `cfg` through the holder (ADR-0079). `None` means "not through the holder": the caller runs it itself. The environment is
    /// complete (the holder passes nothing of its own); a selected folder is checked to be in place right before and right after the
    /// start (the holder changes into the path, not into the handle the node holds: weaker than the node's own start, see
    /// `docs/specs/holder-link.md`).
    fn launch_held(
        &self,
        id: &AgentId,
        cfg: &PtyConfig,
        folder: Option<&agora_platform::HeldFolder>,
    ) -> Option<Result<TerminalHandle, SupervisorError>> {
        let hub = self.holder.as_ref()?;
        // An id the holder would refuse (longer than the link carries, #929) is run by the node, never refused: the node's own ids
        // always fit, an id from an older ledger may not.
        if !crate::holder_link::agent_id_fits(id.as_str()) {
            return None;
        }
        let cwd = cfg.cwd.as_ref()?.to_str()?.to_string();
        // A folder that moved is the node path's to report (`FolderMoved`).
        if folder.is_some_and(|f| !f.in_place()) {
            return None;
        }
        // The link carries text. A variable that is not UTF-8 cannot travel, and leaving it out would start the agent with a smaller
        // environment than the node's own start gives it: such an agent is run by the node (`None`).
        let mut env: Vec<(String, String)> = Vec::new();
        for (k, v) in std::env::vars_os() {
            if agora_platform::env_removed(&k, &cfg.env_remove) {
                continue;
            }
            env.push((k.into_string().ok()?, v.into_string().ok()?));
        }
        for (k, v) in &cfg.env {
            env.retain(|(a, _)| a != k);
            env.push((k.clone(), v.clone()));
        }
        let request = crate::holder::StartRequest {
            agent: id.as_str().to_string(),
            argv: cfg.argv.clone(),
            cwd: Some(cwd),
            env,
            env_remove: Vec::new(),
            cols: cfg.cols,
            rows: cfg.rows,
            cpu_seconds: cfg.limits.cpu_seconds,
            memory_bytes: cfg.limits.memory_bytes,
        };
        // A second try with a fresh session: the holder may have ended between the check and the start (it ends with its last agent).
        for _ in 0..2 {
            let session = hub.session()?;
            let (pid, master) = match session.start(&request) {
                Ok(started) => started,
                // It does not know how to start this (a program that is not there, an id in use) or keeps all the agents it can
                // (`MAX_HELD_AGENTS`, #983): the node's own start runs it or says why.
                Err(LinkError::Refused(HolderRefusal::Failed | HolderRefusal::AgentExists, _)) => {
                    return None
                }
                // Refused for another reason: nothing was started.
                Err(LinkError::Refused(..)) => continue,
                // The link failed: the holder may have started the agent all the same. Ask before anything else is done: an agent it
                // has is ended (the node never got its terminal) and only then is another start made, so two copies never run. When
                // that cannot be settled the start fails rather than run a second copy, unless the holder itself is gone.
                Err(e) => {
                    if session.end_agent(id.as_str()).is_ok() || session.ended_after(&e) {
                        continue;
                    }
                    return Some(Err(SupervisorError::Spawn("holder link failed".into())));
                }
            };
            let made = agora_platform::held_pty(master, pid, session.control_of(id.as_str()))
                .and_then(|pty| TerminalHandle::start(pty, self.cfg.terminal))
                .map(TerminalHandle::into_held);
            let terminal = match made {
                Ok(t) => t,
                Err(_) => {
                    // The agent runs in the holder but the node cannot hold its terminal: ended, and the node runs it itself.
                    let _ = session.stop(id.as_str(), true);
                    return None;
                }
            };
            if folder.is_some_and(|f| !f.in_place()) {
                // Dropping the terminal ends the agent (a swapped folder is never the agent's).
                drop(terminal);
                return Some(Err(SupervisorError::FolderMoved));
            }
            return Some(Ok(terminal));
        }
        None
    }

    /// Drive detection, exits, and restarts. Call regularly (a few times per second is plenty); the
    /// per-agent pacer decides when a screen is actually inspected. Returns the events since the last call.
    pub fn tick(&mut self, now: Instant) -> Vec<SupervisorEvent> {
        let mut events = Vec::new();
        let ids: Vec<AgentId> = self.agents.keys().cloned().collect();
        for id in ids {
            self.tick_agent(&id, now, &mut events);
        }
        self.poll_adapters(now, &mut events, false);
        events
    }

    /// ADR-0104 O2: the node is stopping cleanly. Every adapter the holder runs is let go (it runs on for the next node), each reader
    /// is waited for until `until` at most, and what they had read and the node had not taken yet is taken as a tick takes it (states,
    /// options, config and commands applied; chat, usage and the rest passed on), for the node to record before it ends. Nothing is
    /// restarted and no queued prompt is sent. A reader still running at `until` is left (a holder that does not answer): lines the
    /// holder hands it after that are counted as given and lost, as with a kill, and the next node rebuilds only waiting requests and the
    /// newest options, config and commands from them (ADR-0093, ADR-0104 O1). Adapters the node runs itself are left as they are.
    pub fn let_go_held(&mut self, until: Instant) -> Vec<SupervisorEvent> {
        for e in self.adapters.values_mut() {
            e.restart_at = None;
            if let Some(a) = e.adapter.as_mut().filter(|a| a.is_held()) {
                a.let_go();
            }
        }
        for e in self.adapters.values_mut() {
            if let Some(a) = e.adapter.as_mut().filter(|a| a.is_held()) {
                a.reader_ended_by(until);
            }
        }
        // A queued prompt whose send was waiting for its answer has just been told it was given (`HostError::Detached`): its result
        // is taken below with the rest. Each send's thread holds a handle on the results until it has put its own there.
        while Arc::strong_count(&self.sent) > 1 && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(2));
        }
        let mut events = Vec::new();
        self.poll_adapters(Instant::now(), &mut events, true);
        events
    }

    /// ADR-0104 O2: how many events the reader of `id`'s adapter has handed over (0 for none), so a test can tell a line was read.
    pub fn adapter_events_read(&self, id: &AgentId) -> usize {
        self.adapters
            .get(id)
            .and_then(|e| e.adapter.as_ref())
            .map_or(0, Adapter::events_read)
    }

    /// #1150: how many lines the writer of `id`'s adapter has written to its input (0 for none), so a test can tell a call's line was
    /// given before the node stops.
    pub fn adapter_lines_written(&self, id: &AgentId) -> usize {
        self.adapters
            .get(id)
            .and_then(|e| e.adapter.as_ref())
            .map_or(0, Adapter::lines_written)
    }

    fn tick_agent(&mut self, id: &AgentId, now: Instant, events: &mut Vec<SupervisorEvent>) {
        // Pending restart.
        let due = self
            .agents
            .get(id)
            .is_some_and(|e| e.restart_at.is_some_and(|t| now >= t));
        if due {
            self.restart(id, now, events);
            return;
        }
        let Some(entry) = self.agents.get_mut(id) else {
            return;
        };
        let Some(term) = entry.terminal.as_ref() else {
            return;
        };

        let exit = term.exit_status();
        if let Some(status) = exit {
            events.push(SupervisorEvent::Exited {
                agent: id.clone(),
                status,
            });
            entry.terminal = None;
            self.ended_usage.push(id.clone());
            // A resume that fails within moments, for a kind that can start its session from nothing: there was nothing to resume
            // (the tool writes no session until it is spoken to), so the next start is a fresh one under the same id.
            let quick = entry.native_since.take().is_some_and(|t| {
                now.saturating_duration_since(t)
                    < Duration::from_secs(agora_adapters::manifest::FRESH_WINDOW_SECS)
            });
            entry.fresh_next = quick
                && !status.success()
                && self
                    .kinds
                    .get(&entry.agent.kind)
                    .and_then(|k| k.manifest.resume.as_ref())
                    .is_some_and(|r| !r.fresh_argv.is_empty());
            if status.success() {
                set_state(entry, AgentState::Done, StateSource::Reported, events);
            } else if entry.restarts < self.cfg.restart.max_restarts {
                let after = self.cfg.restart.delay(entry.restarts);
                set_state(entry, AgentState::Unknown, StateSource::Reported, events);
                entry.restart_at = Some(now + after);
                events.push(SupervisorEvent::Restarting {
                    agent: id.clone(),
                    attempt: entry.restarts + 1,
                    after,
                });
            } else {
                entry.gave_up = true;
                set_state(entry, AgentState::Unknown, StateSource::Reported, events);
                events.push(SupervisorEvent::GaveUp {
                    agent: id.clone(),
                    exit_code: status.code.or_else(|| status.signal.map(|s| -s)),
                });
            }
            return;
        }

        if now < entry.next_check {
            return;
        }
        let Some(kind) = self.kinds.get(&entry.agent.kind) else {
            return;
        };
        let detected = term.inspect(|t| kind.detector.detect(t)).map(|d| d.state);
        let before = entry.agent.state();
        if let Some(state) = detected {
            entry.agent.observe(state, StateSource::Screen);
        }
        let changed = entry.agent.state() != before;
        if changed {
            events.push(SupervisorEvent::StateChanged {
                agent: id.clone(),
                state: entry.agent.state(),
            });
        }
        entry.pacer.record(changed);
        entry.next_check = now + entry.pacer.interval(entry.visible);
    }

    fn restart(&mut self, id: &AgentId, now: Instant, events: &mut Vec<SupervisorEvent>) {
        let Some(entry) = self.agents.get(id) else {
            return;
        };
        let (kind, worktree, session, extras, folder) = (
            entry.agent.kind.clone(),
            entry.worktree.clone(),
            entry.agent.session.clone(),
            entry.extras.clone(),
            entry.folder.clone(),
        );
        // A restart of a process that died is never a resume by the node after its own stop: no "carry on" message here.
        let fresh_next = entry.fresh_next;
        let kept_launch = self.kept_launch.contains(id);
        let (argv, native, fresh, _) =
            self.argv_for_resume(&kind, session.as_deref(), false, fresh_next);
        // F1020: a selected folder is held again, so a restart never follows a link swapped in for it since the agent started.
        let result = folder
            .as_deref()
            .map(|p| held_again(None, p))
            .transpose()
            .and_then(|f| self.launch_in(id, &kind, &argv, &worktree, &extras, f.as_ref()));
        let Some(entry) = self.agents.get_mut(id) else {
            return;
        };
        entry.restart_at = None;
        entry.restarts += 1;
        let from_nothing = std::mem::take(&mut entry.fresh_next);
        match result {
            Ok(t) => {
                entry.terminal = Some(t);
                entry.native_since = (native && !from_nothing).then_some(now);
                entry.pacer = Pacer::default();
                entry.next_check = now;
                entry.agent.process_restarted();
                // An agent taken up from the holder knows its launch options but not their arguments (`set_kept_launch`): the
                // restarted process runs without them, so it is not shown with them any more.
                if kept_launch {
                    entry.launch = LaunchChoice::default();
                    self.kept_launch.remove(id);
                }
                // A new session when its own could not be continued (ADR-0060): that one is its session now.
                if !native {
                    entry.agent.session = fresh;
                }
                events.push(SupervisorEvent::Started { agent: id.clone() });
                events.extend(session_report(&entry.agent));
            }
            Err(_) => {
                // Could not even start: count it as a failed attempt and back off again.
                if entry.restarts < self.cfg.restart.max_restarts {
                    let after = self.cfg.restart.delay(entry.restarts);
                    entry.restart_at = Some(now + after);
                    events.push(SupervisorEvent::Restarting {
                        agent: id.clone(),
                        attempt: entry.restarts + 1,
                        after,
                    });
                } else {
                    entry.gave_up = true;
                    events.push(SupervisorEvent::GaveUp {
                        agent: id.clone(),
                        exit_code: None,
                    });
                }
            }
        }
    }

    /// Native resume when the manifest has a resume command and the agent reported a session id; otherwise
    /// the plain launch command (layout-only fallback). The bool is true for the native case.
    ///
    /// ADR-0060: only an id [`argv_safe`](crate::session_files::argv_safe) allows goes on the command line (whatever reported it, an id
    /// that could be read as an option never does). Without one, a kind with `[session] id_args` gets a new id the node chooses, as at a
    /// first launch, so it can be resumed later; that new id is the third value (the agent's session from then on).
    fn argv_for_resume(
        &self,
        kind: &str,
        session: Option<&str>,
        carry_on: bool,
        fresh_start: bool,
    ) -> (Vec<String>, bool, Option<String>, bool) {
        let Some(k) = self.kinds.get(kind) else {
            return (Vec::new(), false, None, false);
        };
        let safe = session.filter(|s| crate::session_files::argv_safe(s));
        if let (Some(r), Some(sid)) = (&k.manifest.resume, safe) {
            // `[resume] fresh_argv`: the session did not exist yet; it is started under the id it was meant to have (still native).
            if fresh_start && !r.fresh_argv.is_empty() {
                let argv = r
                    .fresh_argv
                    .iter()
                    .map(|a| a.replace("{session_id}", sid))
                    .collect();
                return (argv, true, None, false);
            }
            // ADR-0060 Amendment A: the node's own resume of a working agent uses the resume command that carries the kind's "carry on"
            // sentence (from its manifest, which checked it: one plain line, no placeholder, not an option) as one argument.
            if let (true, false, Some(text)) = (
                carry_on,
                r.continue_argv.is_empty(),
                r.continue_text.as_deref(),
            ) {
                let argv = r
                    .continue_argv
                    .iter()
                    .map(|a| a.replace("{session_id}", sid).replace("{continue}", text))
                    .collect();
                return (argv, true, None, true);
            }
            let argv = r
                .argv
                .iter()
                .map(|a| a.replace("{session_id}", sid))
                .collect();
            return (argv, true, None, false);
        }
        let mut argv = k.manifest.process.launch.clone();
        let id_args = k
            .manifest
            .session
            .as_ref()
            .map(|s| s.id_args.clone())
            .unwrap_or_default();
        if id_args.is_empty() {
            return (argv, false, None, false);
        }
        match new_session_id() {
            Ok(new) => {
                argv.extend(id_args.iter().map(|a| a.replace("{session_id}", &new)));
                (argv, false, Some(new), false)
            }
            Err(_) => (argv, false, None, false),
        }
    }

    /// Stop an agent for good: kill its process group and cancel any pending restart. The worktree and its
    /// branch are kept (the work is the user's).
    pub fn stop(&mut self, id: &AgentId) -> Result<Vec<SupervisorEvent>, SupervisorError> {
        if let Some(e) = self.adapters.get_mut(id) {
            let mut events = Vec::new();
            e.restart_at = None;
            if e.running {
                // A stop ends the adapter process too (`shutdown`, then kill), as it ends a terminal agent's process.
                if let Some(adapter) = e.adapter.take() {
                    let _ = adapter.stop_session(&e.session_id, "stop", Duration::from_secs(5));
                    adapter.shutdown(Duration::from_secs(1));
                }
                e.running = false;
            }
            // #933: once its process is ended and collected, so is this launch's Git file. An adapter whose output ended earlier but
            // whose process has not been collected keeps it until a later poll sees the process gone.
            if e.adapter
                .as_mut()
                .is_none_or(|a| a.exit_status(Duration::ZERO).is_some())
            {
                e.git_config = None;
            }
            if e.agent.observe(AgentState::Done, StateSource::Reported) {
                events.push(SupervisorEvent::StateChanged {
                    agent: id.clone(),
                    state: AgentState::Done,
                });
            }
            // ADR-0090: the prompts that waited for its turn will never be sent.
            let dropped = e.rich.forget();
            if !dropped.is_empty() {
                events.push(SupervisorEvent::PromptsDropped {
                    agent: id.clone(),
                    tags: dropped,
                });
            }
            events.push(SupervisorEvent::Stopped { agent: id.clone() });
            return Ok(events);
        }
        let entry = self
            .agents
            .get_mut(id)
            .ok_or_else(|| SupervisorError::NotFound(id.clone()))?;
        let mut events = Vec::new();
        entry.restart_at = None;
        // Dropping the handle kills the process and joins the actor.
        entry.terminal = None;
        self.ended_usage.push(id.clone());
        set_state(entry, AgentState::Done, StateSource::Reported, &mut events);
        events.push(SupervisorEvent::Stopped { agent: id.clone() });
        Ok(events)
    }

    /// ADR-0060: whether an agent of `kind` can continue its own session: its manifest has a `[resume]` argv, or it runs through an
    /// adapter (an ACP bridge loads the session when the agent offers `loadSession`, and opens a new one otherwise).
    pub fn resumable(&self, kind: &str) -> bool {
        self.kinds.get(kind).is_some_and(|k| {
            k.manifest.resume.is_some() || k.manifest.driver(&bridge_program()).is_some()
        })
    }

    /// ADR-0060: whether Resume is offered for an agent the ledger holds as interrupted: the node has the agent's own session id and
    /// its kind can continue one. What `AgentInterrupted.resumable` says, wherever the node says it (ADR-0107: also `agent.list`).
    pub fn offers_resume(&self, e: &crate::ledger::Entry) -> bool {
        !e.session_id.is_empty() && self.resumable(&e.kind)
    }

    /// ADR-0060: puts an agent the node's session ledger holds as interrupted back as a stopped agent, so [`resume`](Self::resume)
    /// starts it under the same id, in the same folder and project. Nothing runs yet. `session` is the agent's own session id to
    /// continue, `None` for a new session.
    pub fn restore(&mut self, r: Restore) -> Result<(), SupervisorError> {
        if self.agents.contains_key(&r.id) || self.adapters.contains_key(&r.id) {
            return Err(SupervisorError::StillRunning(r.id));
        }
        let kind = self
            .kinds
            .get(&r.kind)
            .ok_or_else(|| SupervisorError::UnknownKind(r.kind.clone()))?;
        let worktree = (!r.directory).then(|| WorktreeId::new(r.folder.to_string_lossy()));
        if let Some(spec) = kind.manifest.driver(&bridge_program()) {
            let mut agent = Agent::new(
                r.id.clone(),
                r.project,
                self.cfg.node.clone(),
                worktree,
                r.kind.clone(),
                IntegrationLevel::L3Adapter,
            );
            agent.session = r.session;
            let checkout_root =
                git::discover_checkout(&r.folder).map_or_else(|_| r.folder.clone(), |c| c.root);
            self.adapters.insert(
                r.id,
                AdapterEntry {
                    agent,
                    folder: r.directory.then(|| r.folder.clone()),
                    adapter: None,
                    session_id: String::new(),
                    worktree: r.folder,
                    branch: r.branch,
                    running: false,
                    spec,
                    checkout_root,
                    restarts: 0,
                    restart_at: None,
                    capabilities: Vec::new(),
                    options: SessionOptions::default(),
                    rich: AdapterRich::default(),
                    mesh: None,
                    // ADR-0060: a resumed agent gets none of its launch options back (a resumed agent asks before acting again).
                    launch: LaunchChoice::default(),
                    git_config: None,
                    // ADR-0101: no process yet; the launch (or the take-up) that follows records how it runs.
                    confinement: Confinement::Unknown,
                    replayed: Vec::new(),
                },
            );
            return Ok(());
        }
        if kind.manifest.process.launch.is_empty() {
            return Err(SupervisorError::UnknownKind(r.kind));
        }
        let mut agent = Agent::new(
            r.id.clone(),
            r.project,
            self.cfg.node.clone(),
            worktree,
            r.kind,
            IntegrationLevel::L1Manifest,
        );
        agent.session = r.session;
        self.agents.insert(
            r.id,
            Entry {
                agent,
                folder: r.directory.then(|| r.folder.clone()),
                worktree: r.folder,
                branch: r.branch,
                terminal: None,
                pacer: Pacer::default(),
                next_check: Instant::now(),
                visible: true,
                restarts: 0,
                restart_at: None,
                gave_up: false,
                extras: Extras::default(),
                private_config: r.private_config,
                carry_on: r.carry_on,
                carried_on: false,
                native_since: None,
                fresh_next: false,
                // ADR-0060: no launch option comes back with a resume.
                launch: LaunchChoice::default(),
            },
        );
        Ok(())
    }

    /// ADR-0060 Amendment A: whether the agent's last resume carried the kind's "carry on" message.
    pub fn carried_on(&self, id: &AgentId) -> bool {
        self.agents.get(id).is_some_and(|e| e.carried_on)
    }

    /// ADR-0060: takes back a [`restore`](Self::restore) whose resume failed, so the agent stays only in the ledger.
    pub fn unrestore(&mut self, id: &AgentId) {
        if self.agents.get(id).is_some_and(|e| e.terminal.is_none()) {
            self.agents.remove(id);
        }
        if self.adapters.get(id).is_some_and(|e| !e.running) {
            self.adapters.remove(id);
        }
        self.parents.remove(id);
    }

    /// RM-19 (#903): a start that failed without a stable reason of its own (its process could not be started, or an adapter or the
    /// ACP bridge ended before it answered) for a kind whose program this machine cannot find is said as that
    /// (`agent.program_missing`), by the same check that marks the kind "not installed" ([`Self::kinds`]), instead of as an internal
    /// error. The cause still goes to the node's log.
    fn program_missing(&self, kind: &str, e: SupervisorError) -> SupervisorError {
        let SupervisorError::Spawn(text) = &e else {
            return e;
        };
        let Some(program) = self
            .kinds
            .get(kind)
            .and_then(|k| crate::manifests::program(&k.manifest))
        else {
            return e;
        };
        if crate::doctor::locate_in(&agora_adapters::package::agora_home(), program).is_some() {
            return e;
        }
        eprintln!("agent start failed: {text:?}");
        SupervisorError::ProgramMissing(kind.to_string(), program.to_string())
    }

    /// Start a stopped (or given-up) agent again in its worktree.
    pub fn resume(
        &mut self,
        id: &AgentId,
        now: Instant,
    ) -> Result<Vec<SupervisorEvent>, SupervisorError> {
        self.resume_in(id, now, None)
    }

    /// [`Self::resume`]; `checked` is the agent's selected folder, held by the caller from before it checked that folder (F1021). A
    /// selected folder is held here when not given. A terminal agent starts in that folder, never in whatever the path names by then.
    pub fn resume_in(
        &mut self,
        id: &AgentId,
        now: Instant,
        checked: Option<agora_platform::HeldFolder>,
    ) -> Result<Vec<SupervisorEvent>, SupervisorError> {
        if let Some(entry) = self.adapters.get(id) {
            if entry.running || entry.restart_at.is_some() {
                return Err(SupervisorError::StillRunning(id.clone()));
            }
            self.check_limits()?;
            self.refuse_registry(&entry.agent.kind)?;
            // An adapter is a plain process: it needs the platform to confine a command (#897).
            if !self.sandbox.enforceable_for_commands() {
                return Err(SupervisorError::SandboxUnavailable(
                    entry.agent.kind.clone(),
                    self.sandbox.level.name(),
                ));
            }
            if let Some(p) = entry.spec.command.first().and_then(|p| {
                agora_adapters::package::revoked_publisher_of(
                    &agora_adapters::package::agora_home(),
                    p,
                )
            }) {
                return Err(SupervisorError::PublisherRevoked(
                    entry.agent.kind.clone(),
                    p,
                ));
            }
            // F1021 for adapters (Codex review): a selected folder is held before anything is started for it (the mesh endpoint
            // included) and must be where it was held right before the adapter starts and after (`launch_adapter`).
            let folder = entry
                .folder
                .as_deref()
                .map(|p| held_again(checked, p))
                .transpose()?;
            // #933: the launch's Git file goes with a resume that fails (dropped below), and stays with the adapter it starts.
            let (profile, git_config) = match entry.spec.command.first() {
                Some(p) => self.adapter_sandbox(&entry.agent.kind, &entry.worktree, p)?,
                None => (None, None),
            };
            let runner = self.runner_folder(&entry.agent.kind, id)?;
            let native = entry.agent.session.is_some();
            // A stopped (or given-up) agent left the mesh; resuming joins again with a fresh endpoint, given back if the start fails.
            let mesh = self.adapter_mesh(
                id,
                &entry.agent.kind,
                &entry.agent.project.to_string(),
                &entry.worktree,
            );
            let launched = launch_adapter(
                id,
                self.holder.as_ref(),
                &entry.spec,
                &entry.checkout_root,
                &entry.worktree,
                entry.agent.session.as_deref(),
                profile,
                mesh.as_ref(),
                self.credential_vars(&entry.agent.kind),
                folder.as_ref(),
                git_config,
                runner.as_deref(),
                self.sandbox.enforceable(),
            )
            .map_err(|e| self.program_missing(&entry.agent.kind, e));
            let (adapter, session_id, capabilities, git_config, confinement, replayed) =
                match launched {
                    Ok(l) => l,
                    Err(e) => {
                        if let Some(hook) = &self.abort_hook {
                            hook(id);
                        }
                        return Err(e);
                    }
                };
            let entry = self
                .adapters
                .get_mut(id)
                .ok_or_else(|| SupervisorError::NotFound(id.clone()))?;
            entry.adapter = Some(adapter);
            entry.replayed = replayed;
            entry.confinement = confinement;
            entry.git_config = git_config;
            entry.session_id = session_id;
            entry.capabilities = capabilities;
            entry.mesh = mesh;
            entry.running = true;
            entry.restart_at = None;
            entry.restarts = 0;
            // A resume is a new run: nothing approved for the last one carries over (ADR-0090 Amendment A).
            entry.rich.epoch += 1;
            entry.agent.process_restarted();
            let mut events = vec![SupervisorEvent::Resumed {
                agent: id.clone(),
                native,
            }];
            events.extend(session_report(&entry.agent));
            return Ok(events);
        }
        let entry = self
            .agents
            .get(id)
            .ok_or_else(|| SupervisorError::NotFound(id.clone()))?;
        if entry.terminal.is_some() || entry.restart_at.is_some() {
            return Err(SupervisorError::StillRunning(id.clone()));
        }
        self.check_limits()?;
        self.refuse_registry(&entry.agent.kind)?;
        let (kind, worktree, session, project) = (
            entry.agent.kind.clone(),
            entry.worktree.clone(),
            entry.agent.session.clone(),
            entry.agent.project.to_string(),
        );
        // F1021: held before anything is started for it (the mesh endpoint included).
        let folder = entry
            .folder
            .as_deref()
            .map(|p| held_again(checked, p))
            .transpose()?;
        let carry = self.agents.get(id).is_some_and(|e| e.carry_on);
        let (argv, native, fresh, carried) =
            self.argv_for_resume(&kind, session.as_deref(), carry, false);
        // A stopped agent left the mesh and its endpoint closed; resuming joins again with a fresh one.
        let config = self.agents.get(id).and_then(|e| e.private_config.clone());
        // ADR-0059: a resumed agent keeps the options it was started with.
        let options = self
            .agents
            .get(id)
            .map(|e| e.extras.options.clone())
            .unwrap_or_default();
        // A resume that fails after the hook ran gives the endpoint back, as a failed start does.
        let launched = self
            .inject(id, &kind, &project, &worktree, config.as_deref())
            .map(|extras| Extras { options, ..extras })
            .and_then(|extras| {
                let t = self.launch_in(id, &kind, &argv, &worktree, &extras, folder.as_ref())?;
                Ok((t, extras))
            });
        let (terminal, extras) = match launched {
            Ok(t) => t,
            Err(e) => {
                if let Some(hook) = &self.abort_hook {
                    hook(id);
                }
                return Err(e);
            }
        };
        let Some(entry) = self.agents.get_mut(id) else {
            return Err(SupervisorError::NotFound(id.clone()));
        };
        entry.extras = extras;
        entry.terminal = Some(terminal);
        entry.pacer = Pacer::default();
        entry.next_check = now;
        entry.restarts = 0;
        entry.gave_up = false;
        // Used once: a later restart of its process (a crash) never carries the message again.
        entry.carry_on = false;
        entry.carried_on = carried;
        entry.native_since = native.then_some(now);
        if !native {
            entry.agent.session = fresh;
        }
        let mut events = vec![SupervisorEvent::Resumed {
            agent: id.clone(),
            native,
        }];
        // A resumed agent starts from an unknown screen; clear the "done" left by the stop.
        if entry.agent.process_restarted() {
            events.push(SupervisorEvent::StateChanged {
                agent: id.clone(),
                state: AgentState::Unknown,
            });
        }
        events.extend(session_report(&entry.agent));
        Ok(events)
    }

    /// A state the agent itself reported (hook, ACP, adapter). Outranks screen detection.
    pub fn report_state(
        &mut self,
        id: &AgentId,
        state: AgentState,
    ) -> Result<Vec<SupervisorEvent>, SupervisorError> {
        let entry = self
            .agents
            .get_mut(id)
            .ok_or_else(|| SupervisorError::NotFound(id.clone()))?;
        let mut events = Vec::new();
        set_state(entry, state, StateSource::Reported, &mut events);
        Ok(events)
    }

    /// The agent's own session id, used by [`resume`](Self::resume).
    ///
    /// ADR-0060: a terminal agent's id goes on its command line at a restart (`[resume]`), so only an id
    /// [`argv_safe`](crate::session_files::argv_safe) allows is taken (`protocol.invalid_message`, `session_id`, otherwise).
    pub fn report_session(&mut self, id: &AgentId, session: &str) -> Result<(), SupervisorError> {
        if !crate::session_files::argv_safe(session) {
            return Err(SupervisorError::OptionUnknown("session_id"));
        }
        let entry = self
            .agents
            .get_mut(id)
            .ok_or_else(|| SupervisorError::NotFound(id.clone()))?;
        entry.agent.session = Some(session.to_string());
        Ok(())
    }

    /// ADR-0060: the session id found in a session file for the agent's process `launch` ([`session_files::Target`]). Set only while
    /// that process still runs and the agent has no session yet: one found for a process that has since been restarted, resumed or
    /// stopped is dropped (the file is that process's conversation, never the next one's, #925), and so is an id another agent holds
    /// now (one reported while the files were read). Returns whether it was set.
    ///
    /// [`session_files::Target`]: crate::session_files::Target
    pub fn report_found_session(
        &mut self,
        id: &AgentId,
        launch: u64,
        session: &str,
    ) -> Result<bool, SupervisorError> {
        if !crate::session_files::argv_safe(session) {
            return Err(SupervisorError::OptionUnknown("session_id"));
        }
        if !self.agents.contains_key(id) {
            return Err(SupervisorError::NotFound(id.clone()));
        }
        if self.known_sessions().contains(session) {
            return Ok(false);
        }
        let entry = self
            .agents
            .get_mut(id)
            .ok_or_else(|| SupervisorError::NotFound(id.clone()))?;
        let same = entry.terminal.as_ref().map(TerminalHandle::launch) == Some(launch);
        if !same || entry.agent.session.is_some() {
            return Ok(false);
        }
        entry.agent.session = Some(session.to_string());
        Ok(true)
    }

    /// Whether anyone is looking at the agent's terminal; hidden terminals are still checked, less often.
    pub fn set_visible(&mut self, id: &AgentId, visible: bool) {
        if let Some(e) = self.agents.get_mut(id) {
            e.visible = visible;
        }
    }

    pub fn terminal(&self, id: &AgentId) -> Option<&TerminalHandle> {
        self.agents.get(id)?.terminal.as_ref()
    }

    /// The node is stopping: every agent's process ends now (terminal process groups killed, adapters shut down; an agent in the holder
    /// is let go and runs on, ADR-0079), without an end
    /// recorded anywhere, so the session ledger finds them interrupted at the next start (ADR-0060). Nothing is restarted.
    pub fn shutdown(&mut self) {
        for e in self.agents.values_mut() {
            e.restart_at = None;
            // ADR-0079: an agent in the holder keeps running (its terminal is let go, not ended); any other ends with the handle.
            if let Some(t) = &e.terminal {
                t.detach();
            }
            // Dropping the handle kills the process group and joins the actor.
            e.terminal = None;
        }
        // ADR-0079: the link to the holder closes with the node, so that the next node (or this process, in a test) can attach: the
        // holder serves one node at a time. Every terminal's copy of it is gone with its handle above.
        self.holder = None;
        for e in self.adapters.values_mut() {
            e.restart_at = None;
            e.running = false;
            if let Some(mut adapter) = e.adapter.take() {
                // ADR-0093: an adapter's process in the holder keeps running (it is let go, not ended); any other ends.
                if !adapter.detach() {
                    adapter.shutdown(Duration::from_secs(1));
                }
            }
            // #933: an ended adapter's Git file goes with it. One the holder keeps running has none: it is never confined.
            e.git_config = None;
        }
    }

    /// ADR-0060: each running agent's top process (its terminal's program, or its adapter), for the session ledger.
    pub fn agent_pids(&self) -> Vec<(AgentId, u32)> {
        self.agents
            .iter()
            .filter_map(|(id, e)| Some((id.clone(), e.terminal.as_ref()?.pid())))
            .chain(
                self.adapters
                    .iter()
                    .filter(|(_, e)| e.running)
                    .filter_map(|(id, e)| Some((id.clone(), e.adapter.as_ref()?.pid()))),
            )
            .collect()
    }

    /// The process ids at the top of each running agent: its terminal's process, or its adapter process.
    pub fn running_pids(&self) -> Vec<u32> {
        self.agents
            .values()
            .filter_map(|e| e.terminal.as_ref().map(TerminalHandle::pid))
            .chain(
                self.adapters
                    .values()
                    .filter(|e| e.running)
                    .filter_map(|e| e.adapter.as_ref().map(Adapter::pid)),
            )
            .collect()
    }

    /// ADR-0095: whether the agent acts without asking for permission now: a launch option that skips it (ADR-0059), or, for a
    /// structured agent, the mode or a config option in use that does (ADR-0058, ADR-0090 Amendment A; the node's own judgement
    /// added to the adapter's word). See [`skips_now`].
    pub fn skips_permissions_now(&self, id: &AgentId) -> bool {
        if let Some(e) = self.agents.get(id) {
            return e.launch.skips_permissions;
        }
        self.adapters
            .get(id)
            .is_some_and(|e| skips_now(e.launch.skips_permissions, &e.options, &e.rich.config))
    }

    /// ADR-0095: whether a running terminal agent's kind has a screen-detection rule that says idle or done (a kind without one never
    /// reaches a state the node pastes in, so `unknown` is all it can say). `None` for an agent that is not a running terminal agent.
    pub fn terminal_detects(&self, id: &AgentId) -> Option<bool> {
        let e = self.agents.get(id).filter(|e| e.terminal.is_some())?;
        Some(self.kinds.get(&e.agent.kind).is_some_and(|k| {
            k.manifest.rules.iter().any(|r| {
                matches!(
                    r.state,
                    agora_adapters::manifest::RuleState::Idle
                        | agora_adapters::manifest::RuleState::Done
                )
            })
        }))
    }

    /// ADR-0095: a fresh look at a running terminal agent's screen, for a dispatch: its rules run now on the live bottom of the buffer
    /// (never the user-visible viewport, and never the paced detection's last result, which an unmatched screen such as a dialog leaves
    /// as it was): the state they detect (`unknown` when none matches), and [`Self::terminal_detects`]. `None` for an agent that is
    /// not a running terminal agent.
    /// ADR-0095: how long until the agent's screen is looked at again by its own detection pacing (`tick_agent`): a held dispatch
    /// prompt's look at the live screen ([`Self::terminal_readiness`]) waits as long, so it adds no screen reads beyond normal
    /// detection. `None` for an agent that is not a running terminal agent.
    pub fn detection_interval(&self, id: &AgentId) -> Option<Duration> {
        let e = self.agents.get(id).filter(|e| e.terminal.is_some())?;
        Some(e.pacer.interval(e.visible))
    }

    pub fn terminal_readiness(&self, id: &AgentId) -> Option<(AgentState, bool)> {
        let detects = self.terminal_detects(id)?;
        let e = self.agents.get(id)?;
        let term = e.terminal.as_ref()?;
        let kind = self.kinds.get(&e.agent.kind)?;
        let state = term
            .inspect(|t| kind.detector.detect(t))
            .map_or(AgentState::Unknown, |d| d.state);
        Some((state, detects))
    }

    pub fn list(&self) -> Vec<AgentView> {
        let mut out: Vec<AgentView> = self
            .agents
            .iter()
            .map(|(id, e)| AgentView {
                directory: e.agent.worktree.is_none(),
                terminal: e.terminal.is_some(),
                level: e.agent.level,
                id: id.clone(),
                kind: e.agent.kind.clone(),
                state: e.agent.state(),
                worktree: e.worktree.clone(),
                branch: e.branch.clone(),
                project: e.agent.project.to_string(),
                running: e.terminal.is_some(),
                restarts: e.restarts,
                parent: self.parents.get(id).cloned(),
                launch_options: e.launch.ids.clone(),
                skips_permissions: e.launch.skips_permissions,
            })
            .collect();
        out.extend(self.adapters.iter().map(|(id, e)| AgentView {
            directory: e.agent.worktree.is_none(),
            terminal: false,
            level: e.agent.level,
            id: id.clone(),
            kind: e.agent.kind.clone(),
            state: e.agent.state(),
            worktree: e.worktree.clone(),
            branch: e.branch.clone(),
            project: e.agent.project.to_string(),
            running: e.running,
            restarts: e.restarts,
            parent: self.parents.get(id).cloned(),
            launch_options: e.launch.ids.clone(),
            skips_permissions: e.launch.skips_permissions,
        }));
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    fn usage_target(&self, id: &AgentId) -> Option<crate::usage_files::Target> {
        let e = self.agents.get(id)?;
        let manifest = &self.kinds.get(&e.agent.kind)?.manifest;
        let spec = manifest.usage.as_ref()?.file.clone()?;
        Some(crate::usage_files::Target {
            agent: id.clone(),
            worktree: e.worktree.clone(),
            spec,
            tool: manifest.kind.clone(),
            quota: manifest.quota.clone(),
            session: e.agent.session.clone(),
        })
    }

    /// Running terminal agents whose kind names a usage file (ADR-0042), for the node's usage watcher.
    pub fn usage_targets(&self) -> Vec<crate::usage_files::Target> {
        self.agents
            .iter()
            .filter(|(_, e)| e.terminal.is_some())
            .filter_map(|(id, _)| self.usage_target(id))
            .collect()
    }

    /// Agents that ended since the last call, for one final read of their usage file (what they wrote in their last second).
    pub fn take_ended_usage_targets(&mut self) -> Vec<crate::usage_files::Target> {
        std::mem::take(&mut self.ended_usage)
            .iter()
            .filter_map(|id| self.usage_target(id))
            .collect()
    }

    /// The agents this node can start (ADR-0030): kind, name, level and whether the program is installed. Sorted by kind.
    pub fn kinds(&self) -> Vec<KindInfo> {
        self.kinds
            .values()
            // #1133: a registry kind the owner's policy switches off is not offered.
            .filter(|k| self.registry_refusal(&k.manifest.kind).is_none())
            .map(|k| {
                let m = &k.manifest;
                let acp = m.structured.as_ref().and_then(|s| s.acp.as_ref());
                let level = match (&m.adapter, acp) {
                    (Some(_), _) => IntegrationLevel::L3Adapter,
                    (None, Some(_)) => IntegrationLevel::L2Acp,
                    (None, None) => IntegrationLevel::L1Manifest,
                };
                let program = crate::manifests::program(m);
                KindInfo {
                    kind: m.kind.clone(),
                    display_name: m.display_name.clone(),
                    level,
                    installed: program.is_some_and(|p| {
                        crate::doctor::locate_in(&agora_adapters::package::agora_home(), p)
                            .is_some()
                    }),
                    program: program.unwrap_or_default().to_string(),
                    launch_options: m.launch.option.clone(),
                    settings: m.launch.setting.clone(),
                }
            })
            .collect()
    }

    /// Terminal size used for new agents (for callers that attach clients).
    pub fn terminal_size(&self) -> Size {
        self.cfg.terminal.size
    }
}

/// ADR-0060: when the node itself runs inside a Claude Code session (started from one, or from a terminal one opened), it inherits that
/// session's markers (`CLAUDECODE`, `CLAUDE_CODE_CHILD_SESSION`, other `CLAUDE_CODE_*`). An agent started with them takes itself for a
/// child session and Claude Code then saves no transcript, so its session can never be resumed (seen on Claude Code 2.1.319: "Transcript
/// saving is off — inherited CLAUDE_CODE_CHILD_SESSION marker"). Only then are they left out of every terminal agent's environment; a
/// node started normally passes `CLAUDE_CODE_*` settings the user made (`CLAUDE_CODE_USE_BEDROCK`, ...) on as before.
fn inherited_session_markers(names: impl Iterator<Item = std::ffi::OsString>) -> Vec<String> {
    let inside = names
        .filter_map(|n| n.into_string().ok())
        .any(|n| n == "CLAUDECODE" || n == "CLAUDE_CODE_CHILD_SESSION");
    if inside {
        vec!["CLAUDECODE".into(), "CLAUDE_CODE_*".into()]
    } else {
        Vec::new()
    }
}

/// A random version 4 UUID, lower-case.
fn new_session_id() -> Result<String, getrandom::Error> {
    let mut b = [0u8; 16];
    getrandom::getrandom(&mut b)?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    ))
}

/// The program that runs `acp-bridge` for ACP agents: this node's own binary (`agora`), unless `AGORA_ACP_BRIDGE` names another
/// (tests, packaging).
fn bridge_program() -> PathBuf {
    std::env::var_os("AGORA_ACP_BRIDGE")
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok())
        .unwrap_or_else(|| PathBuf::from("agora"))
}

/// The most modes or models kept per agent, and the longest id, name and description (ADR-0058). Adapters are untrusted: an option
/// with a longer id is dropped (a cut id would name another option), longer text is cut.
const OPTIONS_MAX: usize = 200;
const OPTION_ID_MAX: usize = 200;
const OPTION_NAME_MAX: usize = 200;
const OPTION_DESCRIPTION_MAX: usize = 500;
/// The longest parameter value of an adapter's reason passed on in an error (the bridge's stderr tail is at most about 400 bytes).
const REASON_PARAM_MAX: usize = 600;

fn clip(s: &str, max: usize) -> String {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// ADR-0093 decision 3 and ADR-0104 decision 1: what a node taking up a held adapter reads from the lines the earlier node had already
/// been given (`seen`), in one pass. The requests still waiting on the person are returned, to be asked again. The newest
/// `session/options`, `session/config` and `session/commands` of the recorded session replace the kept file's copy of that kind (a kind
/// with no report there keeps the file's). Only the adapter's own reports set these, and each replaces the last, so the newest one the
/// holder still has is at least as new as the file's, which misses one the earlier node read and did not record before it stopped
/// (#1080). They are bounded and judged afterwards, with the rest of `reattach`, as a report is.
fn newest_reported(reattach: &mut Reattach, seen: &[Vec<u8>]) -> Vec<AdapterEvent> {
    let replay = agora_adapters::host::replay(seen, &reattach.session_id);
    if let Some(options) = replay.options {
        reattach.options = options;
    }
    if let Some(config) = replay.config {
        reattach.config = config;
    }
    if let Some(commands) = replay.commands {
        reattach.commands = commands;
    }
    replay.approvals
}

/// ADR-0058: an adapter's report, checked and bounded. A current id that is not in its list is dropped too.
fn bounded_options(o: SessionOptions) -> SessionOptions {
    let list = |l: Vec<SessionOption>, modes: bool| -> Vec<SessionOption> {
        let mut seen = std::collections::BTreeSet::new();
        l.into_iter()
            .filter(|o| !o.id.is_empty() && o.id.len() <= OPTION_ID_MAX)
            .filter(|o| seen.insert(o.id.clone()))
            .take(OPTIONS_MAX)
            .map(|o| {
                let name = clip(&o.name, OPTION_NAME_MAX);
                let description = clip(&o.description, OPTION_DESCRIPTION_MAX);
                // The adapter's word and the node's own rule (`perm`): either flags it, neither ever clears the other. A model is
                // flagged only by the strong words (a mode that is not clearly one that asks is flagged too).
                let (category, option) = if modes {
                    (Category::Mode, ("mode", "mode"))
                } else {
                    (Category::Model, ("model", "model"))
                };
                SessionOption {
                    skips_permissions: o.skips_permissions
                        || perm::value_skips(category, option, (&o.id, &name, &description)),
                    name,
                    description,
                    id: o.id,
                }
            })
            .collect()
    };
    let (modes, models) = (list(o.modes, true), list(o.models, false));
    let keep = |id: String, l: &[SessionOption]| {
        if l.iter().any(|o| o.id == id) {
            id
        } else {
            String::new()
        }
    };
    SessionOptions {
        current_mode: keep(o.current_mode, &modes),
        current_model: keep(o.current_model, &models),
        modes,
        models,
    }
}

/// ADR-0090: a queued prompt becomes the turn. The send itself is made on a thread of its own, so a hung adapter cannot stall the tick
/// (which holds the supervisor's lock); its outcome comes back through `sent`, with the turn's generation, and the next poll reports
/// it (once) and, when it failed, goes on with the next prompt.
fn begin_send(
    e: &AdapterEntry,
    id: &AgentId,
    next: Next,
    sent: &SentResults,
    events: &mut Vec<SupervisorEvent>,
) {
    let (next, generation) = next;
    // #908: said before the send starts, so the conversation marks where this turn begins ahead of anything the agent answers.
    events.push(SupervisorEvent::PromptSending {
        agent: id.clone(),
        tag: next.tag,
    });
    let caller = e
        .adapter
        .as_ref()
        .filter(|_| e.running)
        .map(Adapter::caller);
    let push = |results: &SentResults, ok: bool| {
        results.lock().unwrap_or_else(|e| e.into_inner()).push((
            id.clone(),
            next.tag,
            ok,
            generation,
        ));
    };
    let Some(caller) = caller else {
        return push(sent, false);
    };
    let call = AdapterCall {
        caller,
        session: e.session_id.clone(),
        op: CallOp::Prompt {
            text: next.text.clone(),
            from_mesh: false,
        },
        generation,
        epoch: e.rich.epoch,
    };
    let (results, agent, tag) = (Arc::clone(sent), id.clone(), next.tag);
    let spawned = std::thread::Builder::new()
        .name("agora-queued-prompt".into())
        .spawn(move || {
            let ok = match call.run() {
                Ok(()) => true,
                Err(e) => {
                    log_call_failure("prompt", &agent, &e);
                    false
                }
            };
            results
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push((agent, tag, ok, generation));
        });
    if spawned.is_err() {
        push(sent, false);
    }
}

/// #1150: one line in the node's log for a call to `agent`'s adapter that failed where nobody is answered with the error (`what`:
/// `prompt` for a queued prompt or a helper's first prompt, `approval refusal` for a request the node refuses on its own). Never the
/// prompt's or the request's text, which may hold secrets: only the cause.
pub(crate) fn log_call_failure(what: &str, agent: &AgentId, e: &SupervisorError) {
    match e {
        SupervisorError::Prompt(cause)
        | SupervisorError::Approval(cause)
        | SupervisorError::AdapterCall(_, cause) => {
            eprintln!("agent {what} failed: {}: {cause:?}", agent.as_str())
        }
        other => eprintln!("agent {what} failed: {}: {other:?}", agent.as_str()),
    }
}

/// ADR-0090: the most config options kept per agent, choices per option, commands, and the longest text of each.
const CONFIG_OPTIONS_MAX: usize = 64;
const CONFIG_CHOICES_MAX: usize = 64;
const COMMANDS_MAX: usize = 100;
const COMMAND_NAME_MAX: usize = 100;
const COMMAND_HINT_MAX: usize = 200;
const CONFIG_CATEGORY_MAX: usize = 40;

/// ADR-0090: an adapter's config options, checked and bounded: at most 64 options of 64 choices, an empty, overlong or repeated id
/// dropped (a cut id would name another option), text cut. `skips_permissions` is the adapter's word or the node's own rule
/// (`perm`): an unknown value of a permission option counts as skipping, so the unknown case is never read as safe and the
/// window asks first.
fn bounded_config(options: Vec<ConfigOption>) -> Vec<ConfigOption> {
    let mut seen = std::collections::BTreeSet::new();
    options
        .into_iter()
        .filter(|o| !o.id.is_empty() && o.id.len() <= OPTION_ID_MAX)
        .filter(|o| seen.insert(o.id.clone()))
        .take(CONFIG_OPTIONS_MAX)
        .map(|o| {
            let name = clip(&o.name, OPTION_NAME_MAX);
            let description = clip(&o.description, OPTION_DESCRIPTION_MAX);
            let category = Category::from_acp(Some(o.category.as_str()));
            let mut seen = std::collections::BTreeSet::new();
            let choices: Vec<SessionOption> = if o.boolean {
                Vec::new()
            } else {
                o.choices
                    .into_iter()
                    .filter(|c| !c.id.is_empty() && c.id.len() <= OPTION_ID_MAX)
                    .filter(|c| seen.insert(c.id.clone()))
                    .take(CONFIG_CHOICES_MAX)
                    .map(|c| {
                        let (cname, cdesc) = (
                            clip(&c.name, OPTION_NAME_MAX),
                            clip(&c.description, OPTION_DESCRIPTION_MAX),
                        );
                        SessionOption {
                            skips_permissions: c.skips_permissions
                                || perm::value_skips(
                                    category,
                                    (&o.id, &name),
                                    (&c.id, &cname, &cdesc),
                                ),
                            name: cname,
                            description: cdesc,
                            id: c.id,
                        }
                    })
                    .collect()
            };
            // A select's current value that is not one of its choices is dropped; a boolean's is `true` or `false`.
            let current = if o.boolean {
                if o.current == "true" { "true" } else { "false" }.to_string()
            } else if choices.iter().any(|c| c.id == o.current) {
                o.current
            } else {
                String::new()
            };
            ConfigOption {
                skips_permissions: o.boolean
                    && (o.skips_permissions
                        || perm::boolean_skips(category, (&o.id, &name), &description)),
                category: clip(&o.category, CONFIG_CATEGORY_MAX),
                id: o.id,
                name,
                description,
                boolean: o.boolean,
                current,
                choices,
            }
        })
        .collect()
}

/// ADR-0090: an adapter's slash commands, bounded (100, a nameless one dropped, text cut).
fn bounded_commands(commands: Vec<AdapterCommand>) -> Vec<AdapterCommand> {
    commands
        .into_iter()
        .filter(|c| !c.name.is_empty() && c.name.len() <= COMMAND_NAME_MAX)
        .take(COMMANDS_MAX)
        .map(|c| AdapterCommand {
            description: clip(&c.description, OPTION_DESCRIPTION_MAX),
            input_hint: clip(&c.input_hint, COMMAND_HINT_MAX),
            name: c.name,
        })
        .collect()
}

/// An adapter's error answer as a supervisor error: one of the stable reasons the node passes on (ADR-0058), with only the
/// parameters that code has, as strings, bounded; anything else is internal (the text goes to logs only).
fn adapter_error(e: HostError) -> SupervisorError {
    adapter_reason(e).unwrap_or_else(|e| SupervisorError::Spawn(e.to_string()))
}

/// #1150: [`adapter_error`] for a call made to a running agent (`call`: `cancel`, `set_config`, `set_option`): what has no stable
/// reason is internal as before, but the node's log names the call instead of a failed start.
fn call_error(call: &'static str) -> impl Fn(HostError) -> SupervisorError {
    move |e| adapter_reason(e).unwrap_or_else(|e| SupervisorError::AdapterCall(call, e.to_string()))
}

/// One of the stable reasons the node passes on, or the error itself when it has none.
fn adapter_reason(e: HostError) -> Result<SupervisorError, HostError> {
    if let Some((code, params)) = e.reason() {
        let known = [
            agora_protocol::ErrorCode::AdapterAgentExited,
            agora_protocol::ErrorCode::AdapterAgentRefused,
            agora_protocol::ErrorCode::AdapterOptionRefused,
        ];
        if let Some(c) = known.iter().find(|c| c.as_str() == code) {
            let code = match c {
                agora_protocol::ErrorCode::AdapterAgentExited => "adapter.agent_exited",
                agora_protocol::ErrorCode::AdapterAgentRefused => "adapter.agent_refused",
                _ => "adapter.option_refused",
            };
            let params = c
                .params()
                .iter()
                .map(|k| {
                    let v = params.get(*k).and_then(serde_json::Value::as_str);
                    // The one parameter an adapter may leave out: an agent killed by a signal has no exit code.
                    let v = match (v, *k) {
                        (Some(v), _) => clip(v, REASON_PARAM_MAX),
                        (None, "exit_code") => "?".to_string(),
                        (None, _) => String::new(),
                    };
                    (*k, v)
                })
                .collect();
            return Ok(SupervisorError::Adapter { code, params });
        }
    }
    Err(e)
}

/// F1012: the folder a launch checked (`checkout`, as the check spelled it): the one the caller held, or held now when it gave none.
/// A path that no longer leads to it (a link swapped in since the check) is refused.
fn held(
    checked: Option<agora_platform::HeldFolder>,
    checkout: &Path,
) -> Result<agora_platform::HeldFolder, SupervisorError> {
    if let Some(folder) = checked {
        return if folder.path() == checkout {
            Ok(folder)
        } else {
            Err(SupervisorError::FolderMoved)
        };
    }
    // Held where the path leads now, in the spelling a check would record (a caller that gave none need not have spelled it so: a
    // temporary folder under `/var` on macOS, a short name on Windows).
    agora_platform::canonicalize_plain(checkout)
        .and_then(|plain| agora_platform::HeldFolder::open(&plain))
        .map_err(|e| {
            let gone = matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            );
            if gone || agora_platform::folder_moved(&e) {
                SupervisorError::FolderMoved
            } else {
                SupervisorError::Spawn(e.to_string())
            }
        })
}

/// F1020, F1021: a started agent's selected folder held again by the path its start held it at (`checked`, when the caller held it
/// for its check). Never resolved first: a path that no longer leads to a folder there (one swapped for a link) is refused.
fn held_again(
    checked: Option<agora_platform::HeldFolder>,
    path: &Path,
) -> Result<agora_platform::HeldFolder, SupervisorError> {
    if let Some(folder) = checked {
        return if folder.path() == path {
            Ok(folder)
        } else {
            Err(SupervisorError::FolderMoved)
        };
    }
    agora_platform::HeldFolder::open(path).map_err(|e| {
        let gone = matches!(
            e.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
        );
        if gone || agora_platform::folder_moved(&e) {
            SupervisorError::FolderMoved
        } else {
            SupervisorError::Spawn(e.to_string())
        }
    })
}

/// What [`launch_adapter`] started: the adapter, its session id, the capabilities it declared, its launch's Git file and how it was
/// launched (ADR-0101).
type Launched = (
    Adapter,
    String,
    Vec<String>,
    Option<crate::gitconfig::LaunchFile>,
    Confinement,
    Vec<AdapterEvent>,
);

/// Spawns an adapter process, initializes it and starts (or resumes) a session. Returns it with the session id and the capabilities
/// it declared. An adapter's stable reason for failing (ADR-0058) becomes [`SupervisorError::Adapter`].
///
/// `folder`: the selected folder the start holds (F1012, F1023). An adapter is started, and given its folder, by path, so the path
/// must lead to the held folder right before the adapter is started and again once its session is open; otherwise nothing is started,
/// or the adapter is ended ([`discard_adapter`]), and the start is refused ([`SupervisorError::FolderMoved`]).
///
/// ADR-0101: also returns how the process was launched: [`Confinement::Confined`] only for one the node started itself under `sandbox`
/// while the node's policy is `enforceable` (as every other confinement check reads it); one the holder runs never is.
#[allow(clippy::too_many_arguments)] // each is one part of the start the caller made
fn launch_adapter(
    agent: &AgentId,
    holder: Option<&Arc<HolderHub>>,
    spec: &agora_adapters::manifest::AdapterSpec,
    checkout_root: &Path,
    worktree: &Path,
    resume: Option<&str>,
    mut sandbox: Option<agora_platform::Sandbox>,
    mesh: Option<&agora_adapters::inject::Endpoint>,
    credentials: Vec<(String, String)>,
    folder: Option<&agora_platform::HeldFolder>,
    git_config: Option<crate::gitconfig::LaunchFile>,
    runner: Option<&Path>,
    enforceable: bool,
) -> Result<Launched, SupervisorError> {
    let (program, args) = spec
        .command
        .split_first()
        .ok_or_else(|| SupervisorError::Spawn("empty adapter command".into()))?;
    let mut args = args.to_vec();
    // #1150: a registry kind's package runner starts in its own empty folder (`Supervisor::runner_folder`), never in the worktree;
    // the bridge stays in the worktree and tells the agent its session's folder. Only the ACP bridge can do that: a registry kind
    // whose runner is its own adapter command (the conversion never writes one) is not started.
    if let Some(dir) = runner {
        let dir_text = dir
            .to_str()
            .ok_or_else(|| SupervisorError::Spawn("package runner folder is not UTF-8".into()))?;
        if Path::new(program) != bridge_program()
            || args.first().map(String::as_str) != Some("acp-bridge")
        {
            return Err(SupervisorError::Spawn(
                "a package runner of a registry kind runs only behind the ACP bridge".into(),
            ));
        }
        args.splice(
            1..1,
            [
                agora_adapters::acp::RUNNER_CWD_FLAG.to_string(),
                dir_text.to_string(),
            ],
        );
        // Confined: the folder is opened as the worktree is (on macOS it lies below the node's closed home, `reopen`).
        if let Some(profile) = &mut sandbox {
            profile.write.push(dir.to_path_buf());
            profile.reopen.push(dir.to_path_buf());
        }
    }
    let mut env: Vec<(String, String)> = spec
        .env
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    // ADR-0084: the secrets listed for this kind (the adapter's environment is otherwise clean, so no other kind's reach it).
    env.retain(|(k, _)| !credentials.iter().any(|(n, _)| n == k));
    env.extend(credentials);
    // The agent's mesh server runs below the adapter, inside its confinement: offline, it may reach its own mesh socket and no other.
    if let Some(profile) = &mut sandbox {
        own_socket(profile, mesh.map(|m| m.socket.as_path()));
    }
    // A confined adapter works in its own private temporary folder, as a confined terminal agent does.
    if sandbox.is_some() && agora_platform::PRIVATE_TEMP {
        let temp = crate::sandbox::agent_temp(worktree).display().to_string();
        env.push(("TMPDIR".into(), temp.clone()));
        env.push(("TEMP".into(), temp.clone()));
        env.push(("TMP".into(), temp));
    }
    let moved = || folder.is_some_and(|f| !f.in_place());
    // Codex review of F1023: checked last before the start, with everything for it made.
    if moved() {
        return Err(SupervisorError::FolderMoved);
    }
    let config = AdapterConfig {
        program: program.into(),
        args,
        env,
        cwd: Some(worktree.to_path_buf()),
        sandbox,
    };
    // ADR-0093: an adapter the node does not confine is run by the holder when this node has one (so it outlives the node), and by the
    // node itself when it cannot be: exactly as before.
    let (adapter, confinement) = match launch_held_adapter(agent, &config, holder) {
        // The holder takes only an adapter without a sandbox.
        Some(started) => (started?, Confinement::Unconfined),
        None => {
            let confined = config.sandbox.is_some() && enforceable;
            let adapter =
                Adapter::spawn(&config).map_err(|e| SupervisorError::Spawn(e.to_string()))?;
            let confinement = if confined {
                Confinement::Confined
            } else {
                Confinement::Unconfined
            };
            (adapter, confinement)
        }
    };
    match open_session(&adapter, checkout_root, worktree, resume, mesh) {
        Ok(_) if moved() => {
            discard_adapter(adapter, git_config);
            Err(SupervisorError::FolderMoved)
        }
        Ok((session_id, capabilities, replayed)) => Ok((
            adapter,
            session_id,
            capabilities,
            git_config,
            confinement,
            replayed,
        )),
        Err(e) => {
            discard_adapter(adapter, git_config);
            Err(e)
        }
    }
}

/// The first time a structured agent is started by the node itself because the holder speaks only link version 1 (built before
/// ADR-0093): one log line with a stable code, so that a person reading the log can see why a node restart ends such agents.
fn log_old_holder_once(holder_pid: u32) {
    static LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        eprintln!(
            "holder.link_v1: structured agent runs in the node, holder_pid={holder_pid} holder_link_version=1"
        );
    }
}

/// ADR-0093: starts `cfg` through the holder, on pipes. `None` means "not through the holder": the caller runs it itself (a confined
/// adapter, no holder or an older one, a program or id the holder refuses, an id longer than the link carries, a variable that is not
/// text). The environment is complete (the holder passes nothing of its own).
fn launch_held_adapter(
    agent: &AgentId,
    cfg: &AdapterConfig,
    holder: Option<&Arc<HolderHub>>,
) -> Option<Result<Adapter, SupervisorError>> {
    let hub = holder?;
    if cfg.sandbox.is_some() || !crate::holder_link::agent_id_fits(agent.as_str()) {
        return None;
    }
    let cwd = cfg.cwd.as_ref()?.to_str()?.to_string();
    let env = Adapter::held_env(cfg)?;
    let mut argv = vec![cfg.program.to_str()?.to_string()];
    argv.extend(cfg.args.iter().cloned());
    let request = crate::holder::StartPipeRequest {
        agent: agent.as_str().to_string(),
        argv,
        cwd: Some(cwd),
        env,
    };
    // A second try with a fresh session: the holder may have ended between the check and the start (it ends with its last agent).
    for _ in 0..2 {
        let session = hub.session()?;
        if !session.pipes {
            log_old_holder_once(session.pid);
            return None;
        }
        let pid = match session.start_pipe(&request) {
            Ok(pid) => pid,
            // It does not know how to start this (a program that is not there) or keeps all the agents it can (`MAX_HELD_AGENTS`,
            // #983): the node's own start runs it or says why.
            Err(LinkError::Refused(HolderRefusal::Failed, _)) => return None,
            // An earlier run of this agent is still in the holder (its end was not collected): it is ended, then started again.
            Err(LinkError::Refused(HolderRefusal::AgentExists, _)) => {
                if session.end_agent(agent.as_str()).is_ok() {
                    continue;
                }
                return None;
            }
            Err(LinkError::Refused(..)) => continue,
            // The link failed: the holder may have started it all the same. Ask before anything else is done: one it has is ended and
            // only then is another start made, so two copies never run; when that cannot be settled the start fails, unless the holder
            // itself is gone.
            Err(e) => {
                if session.end_agent(agent.as_str()).is_ok() || session.ended_after(&e) {
                    continue;
                }
                return Some(Err(SupervisorError::Spawn("holder link failed".into())));
            }
        };
        let io = session.pipe_io(agent.as_str(), Vec::new(), 0, pid);
        return match Adapter::attach_held(io, Vec::new()) {
            Ok(adapter) => Some(Ok(adapter)),
            Err(_) => {
                // It runs in the holder but the node cannot read it: ended, and the node runs it itself.
                let _ = session.stop(agent.as_str(), true);
                None
            }
        };
    }
    None
}

/// Ends an adapter that is not kept (its start failed or was refused after the process began): the processes below it first, then the
/// adapter itself, whose last wait (for its output to close) runs on a thread of its own. A process below it that outlived the kill
/// would keep that output open, and the caller (which holds the supervisor) must not wait for it (Codex review of F1023).
///
/// #933: its launch's Git file (`git_config`) is removed only once the adapter is ended and collected, on that same thread.
fn discard_adapter(adapter: Adapter, git_config: Option<crate::gitconfig::LaunchFile>) {
    // Unix: every process found below the adapter, each only while it is still the process that was found (ADR-0060). Elsewhere the
    // adapter alone is ended, as by any stop.
    let tree = |root| agora_platform::process_tree(root).unwrap_or_default();
    let below: Vec<agora_platform::ProcessIdentity> = tree(adapter.pid())
        .iter()
        .skip(1)
        .filter_map(|p| agora_platform::process_identity(p.pid).ok())
        .collect();
    // Each is ended only while it is still the process found (its identity carries its start time, so a pid given to another
    // process meanwhile is left alone); one the adapter's exit reparented in between is still ended.
    for p in &below {
        let _ = agora_platform::kill_process_if_same(p);
    }
    // When no thread can be started the adapter is ended here, as before.
    let _ = std::thread::Builder::new()
        .name("agora-adapter-end".into())
        .spawn(move || {
            drop(adapter);
            drop(git_config);
        });
}

/// The adapter's `initialize` and its session's start (or resume): the session id, the capabilities it declared and, for a resume,
/// everything the adapter said before it answered the session's start. Review of #1103 (B1): an agent that loads its session replays
/// the conversation then (ACP `session/load`, passed on by the bridge as chat), and the adapter's reader hands every line on in order,
/// so all of that is waiting in its events when the answer is in. The tick reads them first and reports their chat items as one
/// [`SupervisorEvent::ReplayedChat`], for the node to keep only when its transcript does not already hold the conversation.
fn open_session(
    adapter: &Adapter,
    checkout_root: &Path,
    worktree: &Path,
    resume: Option<&str>,
    mesh: Option<&agora_adapters::inject::Endpoint>,
) -> Result<(String, Vec<String>, Vec<AdapterEvent>), SupervisorError> {
    // A real agent can be slow to answer: opencode 2.x took 2 to 5 s to initialize and 9.5 to 12 s to open a session (seen on Windows), so
    // the old 10 s ended such a start as a bare `internal` error.
    const INIT: Duration = Duration::from_secs(30);
    const SESSION: Duration = Duration::from_secs(60);
    let init = adapter
        .initialize(&["chat", "chat.rich"], "en", INIT)
        .map_err(adapter_error)?;
    // The agent's own mesh server, described as for a terminal agent (`inject`): `agora mesh stdio --socket <its socket>`.
    let servers: Vec<McpServer> = mesh
        .map(|m| McpServer {
            name: "agora-mesh".into(),
            command: m.command.to_string_lossy().into_owned(),
            args: vec![
                "mesh".into(),
                "stdio".into(),
                "--socket".into(),
                m.socket.to_string_lossy().into_owned(),
            ],
        })
        .into_iter()
        .collect();
    let session_id = adapter
        .start_session_with(
            &checkout_root.to_string_lossy(),
            &worktree.to_string_lossy(),
            resume,
            &servers,
            SESSION,
        )
        .map_err(adapter_error)?;
    let replayed = if resume.is_some() {
        adapter.poll()
    } else {
        Vec::new()
    };
    Ok((session_id, init.capabilities, replayed))
}

/// The agent's own mesh socket joins the unix sockets a confined agent may reach (#475, ADR-0083): it lies in a folder of the node's,
/// which is closed to every agent but this one's socket. Other agents' sockets and the node's are not added.
fn own_socket(profile: &mut agora_platform::Sandbox, socket: Option<&Path>) {
    profile.sockets.extend(socket.map(Path::to_path_buf));
}

/// After an adapter died on its own: wait out the backoff and try again, or give up for good. `last` is how its process ended, when
/// the node could read it (`None` for a restart that could not start).
///
/// #902 (RM-13): giving up ends the agent: it is reported as exited ([`gave_up_status`]) before [`SupervisorEvent::GaveUp`], as a
/// terminal agent's last crash is, so a window shows it ended with an error and takes no more prompts instead of keeping it unknown.
fn schedule_adapter_restart(
    id: &AgentId,
    e: &mut AdapterEntry,
    now: Instant,
    policy: &RestartPolicy,
    last: Option<ExitStatus>,
    events: &mut Vec<SupervisorEvent>,
) {
    if e.restarts < policy.max_restarts {
        let after = policy.delay(e.restarts);
        e.restart_at = Some(now + after);
        events.push(SupervisorEvent::Restarting {
            agent: id.clone(),
            attempt: e.restarts + 1,
            after,
        });
    } else {
        let (status, known) = gave_up_status(last);
        events.push(SupervisorEvent::Exited {
            agent: id.clone(),
            status,
        });
        events.push(SupervisorEvent::GaveUp {
            agent: id.clone(),
            exit_code: known,
        });
    }
}

/// #902: the end reported for an adapter the node gave up on, and the exit code it may say (`None`: not known). An adapter does not end
/// on its own, so this end is always a failure and must read as one wherever an end is shown, whatever ended the last process:
/// - its own non-zero exit code is kept;
/// - a signal is reported as an exit code the way a shell reports it, 128 + the signal (SIGKILL 137), because a window reads only some
///   signals (a crash) as a failure and the others (a stop) as a plain end; this mapping is only for an agent the node gave up on, so an
///   agent a person ends keeps being shown as before;
/// - exit code 1 goes on the wire when the status is not known or was 0 (so it is never a success), and is not said as its exit code.
fn gave_up_status(last: Option<ExitStatus>) -> (ExitStatus, Option<i32>) {
    let code = match last {
        Some(ExitStatus {
            signal: Some(sig), ..
        }) => Some(sig.saturating_add(128)),
        Some(ExitStatus { code: Some(c), .. }) if c != 0 => Some(c),
        _ => None,
    };
    (
        ExitStatus {
            code: Some(code.unwrap_or(1)),
            signal: None,
        },
        code,
    )
}

fn set_state(
    entry: &mut Entry,
    state: AgentState,
    source: StateSource,
    events: &mut Vec<SupervisorEvent>,
) {
    if entry.agent.observe(state, source) {
        events.push(SupervisorEvent::StateChanged {
            agent: entry.agent.id.clone(),
            state,
        });
    }
}

/// Writes `contents` at `path`, which must be inside `worktree`, without ever following a symlink: a repository can
/// contain `.mcp.json -> ~/.ssh/authorized_keys`, and writing "through" it would let a checkout overwrite the user's
/// files. Every directory between the worktree and the file must be a real directory (created if missing), and an
/// existing file is replaced, not written into.
fn write_in_worktree(
    worktree: &std::path::Path,
    path: &std::path::Path,
    contents: &str,
) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind, Write};
    let rel = path
        .strip_prefix(worktree)
        .map_err(|_| Error::new(ErrorKind::InvalidInput, "path is outside the worktree"))?;
    let mut dir = worktree.to_path_buf();
    let parts: Vec<_> = rel.components().collect();
    let Some((file, dirs)) = parts.split_last() else {
        return Err(Error::new(ErrorKind::InvalidInput, "empty path"));
    };
    for d in dirs {
        dir.push(d);
        match std::fs::symlink_metadata(&dir) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    "a directory on the path is a symlink",
                ));
            }
            Ok(m) if !m.is_dir() => {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    "a path component is not a directory",
                ));
            }
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => std::fs::create_dir(&dir)?,
            Err(e) => return Err(e),
        }
    }
    let target = dir.join(file);
    match std::fs::symlink_metadata(&target) {
        // Replace whatever is there (a symlink is removed, not followed).
        Ok(_) => std::fs::remove_file(&target)?,
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target)?;
    f.write_all(contents.as_bytes())?;
    // Keep the injected file out of `git status`: it is the node's, not the agent's work.
    // Exclude patterns are anchored at the Git root, even when the session starts in a nested folder.
    if let Ok(checkout) = git::discover_checkout(worktree) {
        if let (Ok(root), Ok(file)) = (
            agora_platform::canonicalize_plain(&checkout.root),
            agora_platform::canonicalize_plain(&target),
        ) {
            if let Ok(relative) = file.strip_prefix(&root) {
                git::exclude_locally(&root, relative);
            }
        }
    }
    Ok(())
}

/// Recreate an untracked selected folder only inside the new worktree; never follow restored symlinks.
fn selected_worktree_directory(root: &Path, relative: &Path) -> std::io::Result<PathBuf> {
    use std::io::{Error, ErrorKind};
    let root = agora_platform::canonicalize_plain(root)?;
    let mut directory = root.clone();
    for component in relative.components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "invalid worktree subdirectory",
            ));
        }
        directory.push(component);
        match std::fs::symlink_metadata(&directory) {
            Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    "worktree subdirectory is not a real directory",
                ))
            }
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => std::fs::create_dir(&directory)?,
            Err(e) => return Err(e),
        }
    }
    let directory = agora_platform::canonicalize_plain(&directory)?;
    if !directory.starts_with(root) {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "worktree subdirectory leaves its root",
        ));
    }
    Ok(directory)
}

/// Session identifiers are lifecycle data; ordinary supervisor ticks/listing need no extra allocation.
fn session_report(agent: &Agent) -> Option<SupervisorEvent> {
    agent
        .session
        .as_ref()
        .map(|session| SupervisorEvent::SessionReported {
            agent: agent.id.clone(),
            session: session.clone(),
        })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod adapter_option_tests {
    use super::*;

    /// #896: an agent given the mesh may read (and so run) the mesh bridge it is told to start, the node's own executable, which may
    /// sit in a folder only the user may open; that file is added to what it may read, not its folder.
    #[test]
    fn an_agent_with_the_mesh_may_read_the_bridge_program_and_only_that_file() {
        let root = std::env::temp_dir().join(format!("agora-sup-bridge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (worktree, programs) = (root.join("work"), root.join("programs"));
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::create_dir_all(&programs).unwrap();
        let bridge = programs.join("agora-bridge");
        let mut sup = Supervisor::new(SupervisorConfig {
            node: NodeId::new("n"),
            restart: RestartPolicy::default(),
            terminal: TerminalOptions::default(),
            limits: NodeLimits::default(),
        });
        sup.register(
            agora_adapters::Manifest::parse(
                "kind = \"meshy\"\ndisplay_name = \"M\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"claude\"]\n\
                 [mcp]\nmethod = \"env\"\n[mcp.env]\nAGORA_MCP = \"{mcp_command} mesh stdio\"\n",
            )
            .unwrap(),
        );
        let (command, socket) = (bridge.clone(), worktree.join("mesh.sock"));
        sup.set_launch_hook(Box::new(move |_| {
            Some(agora_adapters::inject::Endpoint {
                command: command.clone(),
                socket: socket.clone(),
            })
        }));
        let extras = sup
            .inject(&AgentId::new("agent-1"), "meshy", "p", &worktree, None)
            .unwrap();
        assert_eq!(
            extras.read,
            vec![bridge],
            "the bridge file itself, not its folder"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// #902: an adapter the node gave up on always ends as a failure: its own failing code is kept, a signal (a crash, or SIGKILL from
    /// an out-of-memory killer) becomes the positive code a shell reports (never the negative value a window reads as a stop), and a
    /// status that is not known or says success goes out as exit code 1 without being said as its code.
    #[test]
    fn an_adapter_given_up_on_ends_as_a_failure() {
        let st = |code, signal| ExitStatus { code, signal };
        let wire = |code| st(Some(code), None);
        assert_eq!(gave_up_status(Some(st(Some(3), None))), (wire(3), Some(3)));
        assert_eq!(
            gave_up_status(Some(st(None, Some(9)))),
            (wire(137), Some(137))
        );
        assert_eq!(
            gave_up_status(Some(st(None, Some(15)))),
            (wire(143), Some(143))
        );
        assert_eq!(
            gave_up_status(Some(st(None, Some(11)))),
            (wire(139), Some(139))
        );
        assert_eq!(gave_up_status(Some(st(Some(0), None))), (wire(1), None));
        assert_eq!(gave_up_status(Some(st(None, None))), (wire(1), None));
        assert_eq!(gave_up_status(None), (wire(1), None));
        for (status, _) in [
            gave_up_status(Some(st(None, Some(9)))),
            gave_up_status(None),
        ] {
            assert!(!status.success() && status.code.is_some_and(|c| c > 0));
        }
    }

    /// ADR-0095: `TaskDispatched.agent_skips_permissions` is true for a launch option that skips, the current mode that skips, a boolean
    /// option that skips and is on, or a select whose current choice skips; never for one that is offered but not in use.
    #[test]
    fn an_agent_skips_permissions_by_its_launch_its_mode_or_an_option_in_use() {
        let opt = |id: &str, skips: bool| SessionOption {
            id: id.into(),
            skips_permissions: skips,
            ..SessionOption::default()
        };
        let mut options = SessionOptions {
            modes: vec![opt("build", false), opt("bypass", true)],
            current_mode: "build".into(),
            ..SessionOptions::default()
        };
        let boolean = |current: &str| ConfigOption {
            id: "auto_approve".into(),
            boolean: true,
            current: current.into(),
            skips_permissions: true,
            ..ConfigOption::default()
        };
        let select = |current: &str| ConfigOption {
            id: "approvals".into(),
            current: current.into(),
            choices: vec![opt("ask", false), opt("turbo", true)],
            ..ConfigOption::default()
        };
        assert!(!skips_now(false, &options, &[]));
        assert!(skips_now(true, &options, &[]));
        assert!(!skips_now(
            false,
            &options,
            &[boolean("false"), select("ask")]
        ));
        assert!(skips_now(false, &options, &[boolean("true")]));
        assert!(skips_now(false, &options, &[select("turbo")]));
        options.current_mode = "bypass".into();
        assert!(skips_now(false, &options, &[]));
        // A mode no longer offered (the id does not match one of the list) does not count.
        options.current_mode = String::new();
        assert!(!skips_now(false, &options, &[]));
    }

    #[test]
    fn claude_code_session_markers_are_left_out_only_when_the_node_runs_inside_a_session() {
        let names = |l: &[&str]| {
            l.iter()
                .map(std::ffi::OsString::from)
                .collect::<Vec<_>>()
                .into_iter()
        };
        assert!(inherited_session_markers(names(&["PATH", "CLAUDE_CODE_USE_BEDROCK"])).is_empty());
        for marker in ["CLAUDECODE", "CLAUDE_CODE_CHILD_SESSION"] {
            let removed = inherited_session_markers(names(&["PATH", marker]));
            assert!(agora_platform::env_removed("CLAUDECODE".as_ref(), &removed));
            assert!(agora_platform::env_removed(
                "CLAUDE_CODE_CHILD_SESSION".as_ref(),
                &removed
            ));
            assert!(agora_platform::env_removed(
                "CLAUDE_CODE_ENTRYPOINT".as_ref(),
                &removed
            ));
            assert!(!agora_platform::env_removed("PATH".as_ref(), &removed));
            assert!(!agora_platform::env_removed(
                "CLAUDE_CONFIG_DIR".as_ref(),
                &removed
            ));
        }
    }

    #[test]
    fn a_grant_needs_an_agent_program_from_launch_acp_or_adapter() {
        let mut sup = Supervisor::new(SupervisorConfig {
            node: NodeId::new("n"),
            restart: RestartPolicy::default(),
            terminal: TerminalOptions::default(),
            limits: NodeLimits::default(),
        });
        let head = |k: &str| {
            format!("kind = \"{k}\"\ndisplay_name = \"K\"\nversion = \"1\"\nmin_engine = 1\n")
        };
        let mcp = "[mcp]\nmethod = \"cli_flag\"\nflag = [\"--mcp\", \"{config_file}\"]\n";
        let cases = [
            (
                "agent",
                format!("{}[process]\nlaunch = [\"claude\"]\n{mcp}", head("agent")),
                true,
            ),
            (
                "bare",
                format!("{}[process]\nlaunch = [\"claude\"]\n", head("bare")),
                false,
            ),
            (
                "pyl",
                format!(
                    "{}[process]\nlaunch = [\"/usr/bin/python3.12\", \"x.py\"]\n{mcp}",
                    head("pyl")
                ),
                false,
            ),
            (
                "acp-ok",
                format!(
                    "{}[structured]\nacp = {{ argv = [\"opencode\", \"acp\"] }}\n",
                    head("acp-ok")
                ),
                true,
            ),
            (
                "acp-py",
                format!(
                    "{}[structured]\nacp = {{ argv = [\"python3\", \"a.py\"] }}\n",
                    head("acp-py")
                ),
                false,
            ),
            (
                "acp-env",
                format!(
                    "{}[structured]\nacp = {{ argv = [\"env\", \"-S\", \"x\"] }}\n",
                    head("acp-env")
                ),
                false,
            ),
            (
                "ad-ok",
                format!(
                    "{}[adapter]\ncommand = [\"agora-adapter-x\"]\n",
                    head("ad-ok")
                ),
                true,
            ),
            (
                "ad-bash",
                format!(
                    "{}[adapter]\ncommand = [\"C:\\\\Tools\\\\BASH.EXE\", \"-c\"]\n",
                    head("ad-bash")
                ),
                false,
            ),
            (
                "ad-npx",
                format!(
                    "{}[adapter]\ncommand = [\"npx\", \"pkg\"]\n",
                    head("ad-npx")
                ),
                false,
            ),
        ];
        for (kind, src, _) in &cases {
            sup.register(Manifest::parse(src).unwrap_or_else(|e| panic!("{kind}: {e:?}")));
        }
        for (kind, _, want) in &cases {
            assert_eq!(sup.grantable(kind), *want, "{kind}");
        }
        for p in [
            "pypy3", "PWSH.exe", "wsl.exe", "uvx", "bunx", "nohup", "setsid", "xargs", "stdbuf",
            "cmd.cmd", "node22",
        ] {
            assert!(runs_anything(p), "{p}");
        }
        for p in ["claude", "codex", "wstack", "opencode"] {
            assert!(!runs_anything(p), "{p}");
        }
    }

    /// ADR-0101, Codex review of PR #1007: "always" (and, behind an ACP bridge that did not declare `approval_scopes`, a single Allow)
    /// under `skip_permissions = confined_only` follows how the agent's own process was launched, never the node's sandbox setting
    /// now: an adapter the holder runs is unconfined (also once a node that confines took it up), and a launch with nothing recorded
    /// fails closed.
    #[test]
    fn always_follows_the_agents_own_launch_never_the_nodes_sandbox_setting() {
        use crate::launch_options::SkipPermissions as S;
        use Confinement as C;
        for (policy, confinement, want) in [
            (S::Allow, C::Confined, true),
            (S::Allow, C::Unconfined, true),
            (S::Allow, C::Unknown, true),
            (S::ConfinedOnly, C::Confined, true),
            (S::ConfinedOnly, C::Unconfined, false),
            (S::ConfinedOnly, C::Unknown, false),
            (S::Forbid, C::Confined, false),
            (S::Forbid, C::Unconfined, false),
            (S::Forbid, C::Unknown, false),
        ] {
            assert_eq!(
                always_by_policy(policy, confinement),
                want,
                "{policy:?} {confinement:?}"
            );
        }

        let mut sup = Supervisor::new(SupervisorConfig {
            node: NodeId::new("n"),
            restart: RestartPolicy::default(),
            terminal: TerminalOptions::default(),
            limits: NodeLimits::default(),
        });
        sup.register(
            Manifest::parse(
                "kind = \"acp\"\ndisplay_name = \"A\"\nversion = \"1\"\nmin_engine = 1\n[structured]\nacp = { argv = [\"opencode\", \"acp\"] }\n",
            )
            .unwrap(),
        );
        // The node confines agents now (the old check read only this), and its policy allows "always" only for a confined agent.
        sup.set_sandbox(crate::sandbox::SandboxPolicy {
            level: crate::sandbox::Level::Worktree,
            ..Default::default()
        });
        sup.set_skip_permissions(S::ConfinedOnly);
        let id = AgentId::new("agent-1");
        sup.restore(Restore {
            id: id.clone(),
            kind: "acp".into(),
            project: agora_core::ProjectId::new(None, &"a".repeat(40)).unwrap(),
            folder: std::env::temp_dir(),
            directory: true,
            branch: String::new(),
            session: None,
            carry_on: false,
            private_config: None,
        })
        .unwrap();
        assert!(sup.adapters[&id].behind_acp_bridge());
        let terms = |sup: &Supervisor| {
            let decide = sup.approval_decide(&id);
            (sup.approval_raise_terms(&id), decide.always, decide.once)
        };
        // Restored, nothing launched yet: unknown, refused.
        assert_eq!(sup.confinement(&id), Some(C::Unknown));
        assert_eq!(terms(&sup), (Some((false, false)), false, false));
        // Taken up from the holder (unconfined by design) by this confining node: neither "always" nor, behind a bridge from before
        // ADR-0101, a single Allow.
        sup.adapters.get_mut(&id).unwrap().confinement = C::Unconfined;
        assert_eq!(terms(&sup), (Some((false, false)), false, false));
        sup.adapters.get_mut(&id).unwrap().capabilities = vec!["approval_scopes".into()];
        assert_eq!(terms(&sup), (Some((true, false)), false, true));
        // Launched confined by this node: allowed.
        sup.adapters.get_mut(&id).unwrap().confinement = C::Confined;
        assert_eq!(terms(&sup), (Some((true, true)), true, true));
        sup.adapters.get_mut(&id).unwrap().capabilities.clear();
        assert_eq!(terms(&sup), (Some((false, true)), false, true));
    }

    /// #1009: a running adapter agent under `skip_permissions = confined_only` that offers a mode (`bypass`), a model (`yolo`) and a
    /// config value (`approvals` = `turbo`) that make it act without asking, beside ones that ask. No adapter process: nothing is
    /// called (a call that passed every check ends in `call_of`'s `NotFound`).
    fn switch_rig() -> (Supervisor, AgentId) {
        let mut sup = Supervisor::new(SupervisorConfig {
            node: NodeId::new("n"),
            restart: RestartPolicy::default(),
            terminal: TerminalOptions::default(),
            limits: NodeLimits::default(),
        });
        sup.register(
            Manifest::parse(
                "kind = \"acp\"\ndisplay_name = \"A\"\nversion = \"1\"\nmin_engine = 1\n[structured]\nacp = { argv = [\"opencode\", \"acp\"] }\n",
            )
            .unwrap(),
        );
        sup.set_skip_permissions(crate::launch_options::SkipPermissions::ConfinedOnly);
        let id = AgentId::new("agent-1");
        sup.restore(Restore {
            id: id.clone(),
            kind: "acp".into(),
            project: agora_core::ProjectId::new(None, &"a".repeat(40)).unwrap(),
            folder: std::env::temp_dir(),
            directory: true,
            branch: String::new(),
            session: None,
            carry_on: false,
            private_config: None,
        })
        .unwrap();
        let opt = |id: &str, skips: bool| SessionOption {
            id: id.into(),
            skips_permissions: skips,
            ..SessionOption::default()
        };
        let e = sup.adapters.get_mut(&id).unwrap();
        e.running = true;
        e.capabilities = vec!["set_mode".into(), "set_model".into(), "config".into()];
        e.options = SessionOptions {
            modes: vec![opt("default", false), opt("bypass", true)],
            models: vec![opt("careful", false), opt("yolo", true)],
            current_mode: "default".into(),
            current_model: "careful".into(),
        };
        e.rich.config = vec![ConfigOption {
            id: "approvals".into(),
            current: "ask".into(),
            choices: vec![opt("ask", false), opt("turbo", true)],
            ..ConfigOption::default()
        }];
        (sup, id)
    }

    /// #1009: the agent's recorded launch and the node's sandbox setting now. The refused cases are an agent the holder runs
    /// (unconfined) and one with nothing recorded, each under a node that confines now (what the check read before: it said
    /// confined); the allowed case is a launch recorded as confined under a node that no longer confines (the check before said not).
    fn launched(
        sup: &mut Supervisor,
        id: &AgentId,
        confinement: Confinement,
        level: crate::sandbox::Level,
    ) {
        sup.adapters.get_mut(id).unwrap().confinement = confinement;
        sup.set_sandbox(crate::sandbox::SandboxPolicy {
            level,
            ..Default::default()
        });
    }

    const NOT_CONFINED: [Confinement; 2] = [Confinement::Unconfined, Confinement::Unknown];

    fn forbidden_by_confined_only(r: &SupervisorError, option: &str) -> bool {
        matches!(r, SupervisorError::OptionForbidden(k, o, "confined_only") if k == "acp" && o == option)
    }

    /// #1009: `AgentSetMode` of a mode that skips follows `confined_only` by the agent's own launch (as "always" does, ADR-0101
    /// section 7 item 7), never the node's sandbox setting: refused for an agent the holder runs or one with nothing recorded, asked
    /// about for one launched confined. A mode that asks is never held to the policy.
    #[test]
    fn a_mode_switch_under_confined_only_follows_the_agents_own_launch() {
        use crate::sandbox::Level;
        let (mut sup, id) = switch_rig();
        for confinement in NOT_CONFINED {
            launched(&mut sup, &id, confinement, Level::Worktree);
            match sup.prepare_set_option(&id, OptionKind::Mode, "bypass") {
                Err(e) => assert!(
                    forbidden_by_confined_only(&e, "bypass"),
                    "{confinement:?}: {e:?}"
                ),
                Ok(Prepared::Ask(_)) => {
                    panic!("{confinement:?}: a mode that skips was asked about, not refused")
                }
                Ok(Prepared::Call(_)) => panic!("{confinement:?}: a mode that skips was applied"),
            }
            assert!(!matches!(
                sup.prepare_set_option(&id, OptionKind::Mode, "default"),
                Err(SupervisorError::OptionForbidden(..)) | Ok(Prepared::Ask(_))
            ));
        }
        launched(&mut sup, &id, Confinement::Confined, Level::Off);
        match sup.prepare_set_option(&id, OptionKind::Mode, "bypass") {
            Ok(Prepared::Ask(ask)) => {
                assert_eq!((ask.kind, ask.value.as_str()), (SkipKind::Mode, "bypass"))
            }
            Ok(Prepared::Call(_)) => panic!("a mode that skips was applied"),
            Err(e) => panic!("a confined agent's mode was refused: {e:?}"),
        }
    }

    /// #1009: `AgentSetConfig` of a value that skips, as the mode switch above; a value that asks is never held to the policy.
    #[test]
    fn a_config_switch_under_confined_only_follows_the_agents_own_launch() {
        use crate::sandbox::Level;
        let (mut sup, id) = switch_rig();
        for confinement in NOT_CONFINED {
            launched(&mut sup, &id, confinement, Level::Worktree);
            match sup.prepare_set_config(&id, "approvals", "turbo") {
                Err(e) => assert!(
                    forbidden_by_confined_only(&e, "approvals"),
                    "{confinement:?}: {e:?}"
                ),
                Ok(Prepared::Ask(_)) => panic!(
                    "{confinement:?}: a config value that skips was asked about, not refused"
                ),
                Ok(Prepared::Call(_)) => {
                    panic!("{confinement:?}: a config value that skips was applied")
                }
            }
            assert!(!matches!(
                sup.prepare_set_config(&id, "approvals", "ask"),
                Err(SupervisorError::OptionForbidden(..)) | Ok(Prepared::Ask(_))
            ));
        }
        launched(&mut sup, &id, Confinement::Confined, Level::Off);
        match sup.prepare_set_config(&id, "approvals", "turbo") {
            Ok(Prepared::Ask(ask)) => assert_eq!(
                (ask.kind, ask.option_id.as_str(), ask.value.as_str()),
                (SkipKind::Config, "approvals", "turbo")
            ),
            Ok(Prepared::Call(_)) => panic!("a config value that skips was applied"),
            Err(e) => panic!("a confined agent's config value was refused: {e:?}"),
        }
    }

    /// #1009: `AgentSetModel` of a model that skips: refused by `confined_only` for an agent the holder runs or one with nothing
    /// recorded; for one launched confined the policy says nothing and the model stays unconfirmed (it has no approval card).
    #[test]
    fn a_model_switch_under_confined_only_follows_the_agents_own_launch() {
        use crate::sandbox::Level;
        let (mut sup, id) = switch_rig();
        for confinement in NOT_CONFINED {
            launched(&mut sup, &id, confinement, Level::Worktree);
            match sup.prepare_set_option(&id, OptionKind::Model, "yolo") {
                Err(e) => assert!(
                    forbidden_by_confined_only(&e, "yolo"),
                    "{confinement:?}: {e:?}"
                ),
                Ok(_) => panic!("{confinement:?}: a model that skips was not refused"),
            }
        }
        launched(&mut sup, &id, Confinement::Confined, Level::Off);
        match sup.prepare_set_option(&id, OptionKind::Model, "yolo") {
            Err(SupervisorError::OptionUnconfirmed(k, o)) if k == "acp" && o == "yolo" => {}
            Err(e) => panic!("a confined agent's model was refused by the policy: {e:?}"),
            Ok(_) => panic!("a model that skips was applied"),
        }
    }

    /// #1009: an approved mode or config switch is checked again when the approval arrives, by the agent's launch as it is then: refused
    /// for an agent the holder runs or one with nothing recorded, applied (here: on to the call) for one launched confined.
    #[test]
    fn an_approved_switch_under_confined_only_follows_the_agents_own_launch() {
        use crate::sandbox::Level;
        let (mut sup, id) = switch_rig();
        let epoch = sup.adapters[&id].rich.epoch;
        let ask = |kind: SkipKind, option_id: &str, value: &str| SkipAsk {
            agent: id.clone(),
            agent_kind: "acp".into(),
            folder: String::new(),
            kind,
            option_id: option_id.into(),
            option_name: String::new(),
            value: value.into(),
            value_name: String::new(),
            epoch,
        };
        // With the id the refusal names: a mode's id, a config option's own (#1033).
        let asks = [
            (ask(SkipKind::Mode, "mode", "bypass"), "bypass"),
            (ask(SkipKind::Config, "approvals", "turbo"), "approvals"),
        ];
        for confinement in NOT_CONFINED {
            launched(&mut sup, &id, confinement, Level::Worktree);
            for (ask, refused) in &asks {
                match sup.prepare_skip_apply(ask) {
                    Err(e) => assert!(
                        forbidden_by_confined_only(&e, refused),
                        "{confinement:?} {:?}: {e:?}",
                        ask.kind
                    ),
                    Ok(_) => panic!(
                        "{confinement:?} {:?}: an approved switch that skips was applied",
                        ask.kind
                    ),
                }
            }
        }
        launched(&mut sup, &id, Confinement::Confined, Level::Off);
        for (ask, _) in &asks {
            match sup.prepare_skip_apply(ask) {
                Err(SupervisorError::NotFound(_)) => {}
                Err(e) => panic!(
                    "{:?}: a confined agent's approved switch was refused: {e:?}",
                    ask.kind
                ),
                Ok(_) => panic!("an entry without an adapter process was called"),
            }
        }
    }

    /// ADR-0101, Codex review of PR #1007: a result about the agent's process is applied only to the run the call was made to: a
    /// restart, a resume, an exit or a stop begins another, and the former run's late result does not count for it.
    #[test]
    fn a_late_result_names_the_run_it_was_made_to() {
        let mut sup = Supervisor::new(SupervisorConfig {
            node: NodeId::new("n"),
            restart: RestartPolicy::default(),
            terminal: TerminalOptions::default(),
            limits: NodeLimits::default(),
        });
        sup.register(
            Manifest::parse(
                "kind = \"acp\"\ndisplay_name = \"A\"\nversion = \"1\"\nmin_engine = 1\n[structured]\nacp = { argv = [\"opencode\", \"acp\"] }\n",
            )
            .unwrap(),
        );
        let id = AgentId::new("agent-1");
        sup.restore(Restore {
            id: id.clone(),
            kind: "acp".into(),
            project: agora_core::ProjectId::new(None, &"a".repeat(40)).unwrap(),
            folder: std::env::temp_dir(),
            directory: true,
            branch: String::new(),
            session: None,
            carry_on: false,
            private_config: None,
        })
        .unwrap();
        let run = sup.adapter_epoch(&id).unwrap();
        // No process runs (restored only): nothing is applied.
        assert!(!sup.runs(&id, run));
        assert!(!sup.runs(&AgentId::new("agent-9"), run));
        // Its process runs: the result applies, also after a turn began meanwhile (a turn changes the call's `generation`, not its run).
        sup.adapters.get_mut(&id).unwrap().running = true;
        assert!(sup.runs(&id, run));
        let before = sup.adapters[&id].rich.generation;
        sup.adapters
            .get_mut(&id)
            .unwrap()
            .rich
            .busy(Instant::now(), false);
        assert_ne!(sup.adapters[&id].rich.generation, before);
        assert!(sup.runs(&id, run));
        // A restart (or an exit, a stop) forgets the former process: its run is over.
        let _ = sup.adapters.get_mut(&id).unwrap().rich.forget();
        assert_ne!(sup.adapter_epoch(&id), Some(run));
        assert!(!sup.runs(&id, run));
        // A resume, likewise.
        let now = sup.adapter_epoch(&id).unwrap();
        assert!(sup.runs(&id, now));
        sup.adapters.get_mut(&id).unwrap().rich.epoch += 1;
        assert!(!sup.runs(&id, now));
        // An exit leaves it not running.
        sup.adapters.get_mut(&id).unwrap().running = false;
        assert!(!sup.runs(&id, now + 1));

        // How the node records an always answer delivered to a run: the agent's mark only while that run is its process. A restart
        // while the answer was on its way: the answer is audited as given, its replacement is not marked.
        let dir = std::env::temp_dir().join(format!("agora-always-run-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sink = crate::daemon::EventSink::open(&dir, "n1").unwrap();
        let marked = |sink: &crate::daemon::EventSink, approval: &str| {
            let bodies: Vec<_> = sink
                .events_after(0)
                .into_iter()
                .filter_map(|e| e.body)
                .collect();
            let audited = bodies.iter().any(|b| matches!(b, agora_protocol::v1::event::Body::ApprovalResolved(r)
                if r.approval_id == approval && r.scope == agora_protocol::v1::ApprovalScope::Always as i32));
            let mark = bodies.iter().any(|b| {
                matches!(b, agora_protocol::v1::event::Body::AgentAlwaysAllowed(a)
                if a.approval_id == approval && a.agent_id == "agent-1")
            });
            (audited, mark)
        };
        sup.adapters.get_mut(&id).unwrap().running = true;
        let run = sup.adapter_epoch(&id);
        assert!(crate::control::record_always_answer(
            &sup,
            &sink,
            &id,
            run,
            "ap-1",
            "run_command",
            "phone"
        ));
        assert_eq!(marked(&sink, "ap-1"), (true, true));
        let former = sup.adapter_epoch(&id);
        let _ = sup.adapters.get_mut(&id).unwrap().rich.forget();
        assert!(!crate::control::record_always_answer(
            &sup,
            &sink,
            &id,
            former,
            "ap-2",
            "run_command",
            "phone"
        ));
        assert_eq!(marked(&sink, "ap-2"), (true, false));
        // A call that never reached a run marks nothing either.
        assert!(!crate::control::record_always_answer(
            &sup,
            &sink,
            &id,
            None,
            "ap-3",
            "run_command",
            "phone"
        ));
        assert_eq!(marked(&sink, "ap-3"), (true, false));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn opt(id: &str, skips: bool) -> SessionOption {
        SessionOption {
            id: id.into(),
            name: "n".repeat(300),
            skips_permissions: skips,
            ..SessionOption::default()
        }
    }

    #[test]
    fn reported_options_are_bounded_and_flagged_by_the_adapter_or_the_nodes_own_rule() {
        let mut models: Vec<SessionOption> =
            (0..250).map(|i| opt(&format!("m{i}"), true)).collect();
        models.insert(0, opt(&"x".repeat(OPTION_ID_MAX + 1), false));
        models.push(opt("m1", false));
        let o = bounded_options(SessionOptions {
            modes: vec![
                opt("plan", false),
                opt("yolo", true),
                opt("", false),
                opt("plan", true),
            ],
            models,
            current_mode: "yolo".into(),
            current_model: "gone".into(),
        });
        // `plan` was reported twice: the first one counts (and the adapter's later claim on it does not).
        assert_eq!(
            o.modes
                .iter()
                .map(|m| (m.id.as_str(), m.skips_permissions))
                .collect::<Vec<_>>(),
            [("plan", false), ("yolo", true)],
            "empty and repeated ids are dropped"
        );
        assert_eq!(o.models.len(), OPTIONS_MAX);
        // The adapter's flag is kept for a model too (it can only add protection); text is cut.
        assert!(o
            .models
            .iter()
            .all(|m| m.skips_permissions && m.name.len() <= OPTION_NAME_MAX));
        assert_eq!(
            (o.current_mode.as_str(), o.current_model.as_str()),
            ("yolo", "")
        );
    }

    /// ADR-0090: the node judges by its own rule (`perm`) what an adapter did not flag: a mode it cannot place is flagged, a model that
    /// merely is called `auto` is not, an explicit bypass in a model's name is.
    #[test]
    fn the_node_flags_by_its_own_rule_what_the_adapter_did_not() {
        let named = |id: &str| SessionOption {
            id: id.into(),
            name: id.into(),
            ..SessionOption::default()
        };
        let o = bounded_options(SessionOptions {
            modes: ["default", "plan", "turbo", "bypassPermissions"]
                .map(named)
                .to_vec(),
            models: ["auto", "claude-x", "yolo-fast"].map(named).to_vec(),
            ..SessionOptions::default()
        });
        let flags = |l: &[SessionOption]| l.iter().map(|m| m.skips_permissions).collect::<Vec<_>>();
        assert_eq!(flags(&o.modes), [false, false, true, true]);
        assert_eq!(flags(&o.models), [false, false, true]);
    }

    fn config(id: &str, category: &str, current: &str, choices: &[&str]) -> ConfigOption {
        ConfigOption {
            id: id.into(),
            name: id.into(),
            category: category.into(),
            current: current.into(),
            choices: choices
                .iter()
                .map(|c| SessionOption {
                    id: (*c).into(),
                    name: (*c).into(),
                    ..SessionOption::default()
                })
                .collect(),
            ..ConfigOption::default()
        }
    }

    /// ADR-0090: 64 options of 64 choices, ids that do not fit dropped, a current value that is not a choice dropped, and the node's
    /// own flag: an unknown value of a permission option is flagged, a reasoning level is not; a boolean about approvals is flagged.
    #[test]
    fn reported_config_is_bounded_and_flagged_by_the_nodes_own_rule() {
        let many: Vec<ConfigOption> = (0..100)
            .map(|i| config(&format!("o{i}"), "thought_level", "c0", &["c0"]))
            .collect();
        assert_eq!(bounded_config(many).len(), CONFIG_OPTIONS_MAX);
        let choices: Vec<String> = (0..100).map(|i| format!("c{i}")).collect();
        let refs: Vec<&str> = choices.iter().map(String::as_str).collect();
        let o = bounded_config(vec![
            config("effort", "thought_level", "c1", &refs),
            config(&"x".repeat(OPTION_ID_MAX + 1), "mode", "", &[]),
            config("", "mode", "", &[]),
            config("effort", "thought_level", "c2", &["c2"]),
        ]);
        assert_eq!(
            o.len(),
            1,
            "an overlong, an empty and a repeated id are dropped"
        );
        assert_eq!(o[0].choices.len(), CONFIG_CHOICES_MAX);
        assert_eq!(o[0].current, "c1");
        let o = bounded_config(vec![config("effort", "thought_level", "gone", &["low"])]);
        assert_eq!(
            o[0].current, "",
            "a current value that is not a choice is dropped"
        );

        let o = bounded_config(vec![
            config(
                "approvals",
                "mode",
                "default",
                &["default", "turbo", "plan", "bypass"],
            ),
            config("effort", "thought_level", "low", &["low", "turbo"]),
            ConfigOption {
                boolean: true,
                current: "true".into(),
                ..config("auto_approve", "_custom", "", &[])
            },
            ConfigOption {
                boolean: true,
                current: "maybe".into(),
                ..config("verbose", "_custom", "", &[])
            },
        ]);
        let flags = |c: &ConfigOption| {
            c.choices
                .iter()
                .map(|x| x.skips_permissions)
                .collect::<Vec<_>>()
        };
        assert_eq!(flags(&o[0]), [false, true, false, true]);
        assert_eq!(
            flags(&o[1]),
            [false, false],
            "a reasoning level is not a permission"
        );
        assert!(o[2].skips_permissions && o[2].current == "true");
        assert!(!o[3].skips_permissions && o[3].current == "false");
        // An adapter's own flag is never cleared by the node.
        let mut flagged = config("effort", "thought_level", "low", &["low"]);
        flagged.choices[0].skips_permissions = true;
        assert!(bounded_config(vec![flagged])[0].choices[0].skips_permissions);
    }

    fn queued(r: &mut AdapterRich, text: &str, tag: u64) -> Result<Route, SupervisorError> {
        r.route(true, text, Instant::now(), |_| tag)
    }

    /// The generation of the turn that is open.
    fn gen(r: &AdapterRich) -> u64 {
        r.generation
    }

    fn is_busy(r: &AdapterRich) -> bool {
        matches!(r.turn, Turn::Busy { .. })
    }

    fn tag(n: Option<Next>) -> Option<u64> {
        n.map(|(q, _)| q.tag)
    }

    /// The first prompt goes at once; later ones wait in order, at most eight, each of at most 64 KiB; an adapter that does not
    /// report its turns is never queued for.
    #[test]
    fn the_queue_takes_eight_in_order_and_only_for_an_adapter_that_reports_its_turns() {
        let mut r = AdapterRich::default();
        assert!(matches!(queued(&mut r, "a", 0), Ok(Route::Send(_))));
        assert!(is_busy(&r));
        for n in 1..=8u64 {
            assert_eq!(queued(&mut r, "q", n), Ok(Route::Queued(n as usize)));
        }
        assert_eq!(
            queued(&mut r, "q", 9),
            Err(SupervisorError::QueueFull(MAX_PROMPT_QUEUE))
        );
        let big = "x".repeat(MAX_QUEUED_PROMPT_BYTES + 1);
        let mut other = AdapterRich::default();
        assert!(matches!(queued(&mut other, "a", 0), Ok(Route::Send(_))));
        assert_eq!(
            queued(&mut other, &big, 1),
            Err(SupervisorError::OptionUnknown("text"))
        );
        assert!(other.queue.is_empty());
        assert_eq!(
            queued(&mut other, &"x".repeat(MAX_QUEUED_PROMPT_BYTES), 2),
            Ok(Route::Queued(1))
        );
        // No `turns`: always sent, never queued, however busy it looks.
        let mut plain = AdapterRich::default();
        for _ in 0..20 {
            assert!(matches!(
                plain.route(false, "x", Instant::now(), |_| 0),
                Ok(Route::Send(_))
            ));
        }
        assert_eq!(plain.turn, Turn::Idle);
    }

    /// (a) A prompt that fails to send leaves nothing stuck: the turn it never began is over and the next prompt goes.
    #[test]
    fn a_prompt_that_fails_to_send_does_not_hold_the_queue() {
        let now = Instant::now();
        let mut r = AdapterRich::default();
        let Ok(Route::Send(g0)) = queued(&mut r, "a", 0) else {
            panic!()
        };
        let _ = queued(&mut r, "b", 1);
        let _ = queued(&mut r, "c", 2);
        // `a` was sent and failed: `b` becomes the turn; it fails too: `c`; it fails: nothing waits, the agent is free.
        let (sent, next) = r.send_result(false, g0, now);
        assert!(!sent);
        let (b, g1) = next.unwrap();
        assert_eq!(b.tag, 1);
        let (_, next) = r.send_result(false, g1, now);
        let (c, g2) = next.unwrap();
        assert_eq!(c.tag, 2);
        let (_, next) = r.send_result(false, g2, now);
        assert!(next.is_none());
        assert_eq!(r.turn, Turn::Idle);
        assert!(
            matches!(queued(&mut r, "d", 3), Ok(Route::Send(_))),
            "later prompts are not left waiting for an idle that never comes"
        );
        // A failure is only about a turn that has not begun.
        r.observe(AgentState::Working, now);
        let _ = queued(&mut r, "e", 4);
        assert!(r.send_result(false, gen(&r), now).1.is_none());
        assert_eq!(r.queue.len(), 1);
        // A send that succeeded changes nothing.
        let (sent, next) = r.send_result(true, 0, now);
        assert!(sent && next.is_none());
    }

    /// (b) An adapter that says `done` or `unknown` instead of `idle` ends the turn too.
    #[test]
    fn done_or_unknown_ends_a_turn_like_idle() {
        let now = Instant::now();
        for ending in [AgentState::Done, AgentState::Unknown] {
            let mut r = AdapterRich::default();
            let _ = queued(&mut r, "a", 0);
            let _ = queued(&mut r, "b", 1);
            // Even before it said `working`: it will not say `idle` any more.
            assert_eq!(tag(r.observe(ending, now)), Some(1), "{ending:?}");
            assert!(is_busy(&r));
            assert!(r.observe(ending, now).is_none());
            assert_eq!(r.turn, Turn::Idle);
        }
    }

    /// (c) A stray `idle`, or a second one, ends nothing and never sends a prompt into a running turn.
    #[test]
    fn a_stray_idle_does_not_pop_a_prompt_into_a_running_turn() {
        let now = Instant::now();
        let mut r = AdapterRich::default();
        let _ = queued(&mut r, "a", 0);
        let _ = queued(&mut r, "b", 1);
        let _ = queued(&mut r, "c", 2);
        // The startup `idle` that arrives after the prompt was sent: before any `working`.
        assert!(r.observe(AgentState::Idle, now).is_none());
        assert_eq!(r.queue.len(), 2);
        assert!(r.observe(AgentState::Working, now).is_none());
        assert_eq!(
            tag(r.observe(AgentState::Idle, now)),
            Some(1),
            "the turn ended: `b` goes"
        );
        // `b` is the turn now. A second `idle` (a duplicate of the last one) must not release `c`.
        assert!(r.observe(AgentState::Idle, now).is_none());
        assert_eq!(r.queue.len(), 1);
        assert!(r.observe(AgentState::Working, now).is_none());
        assert!(r.observe(AgentState::Blocked, now).is_none());
        assert_eq!(tag(r.observe(AgentState::Idle, now)), Some(2));
        assert!(r.observe(AgentState::Working, now).is_none());
        assert!(r.observe(AgentState::Idle, now).is_none());
        assert_eq!(r.turn, Turn::Idle);
        // An idle agent that gets an `idle` stays idle.
        assert!(r.observe(AgentState::Idle, now).is_none());
    }

    /// An adapter that only ever says `idle` (never `working`) does not hold the agent for good: its `idle` ends the turn once the
    /// grace has passed since the prompt, whether it comes late or came early and the clock catches up.
    #[test]
    fn an_adapter_that_never_says_working_is_not_stuck_busy() {
        let t0 = Instant::now();
        // The `idle` comes after the grace.
        let mut r = AdapterRich::default();
        let _ = queued(&mut r, "a", 0);
        let _ = queued(&mut r, "b", 1);
        assert!(r
            .observe(
                AgentState::Idle,
                t0 + IDLE_ONLY_GRACE - Duration::from_millis(1)
            )
            .is_none());
        // It was early: nothing else will come, so the clock ends it.
        assert!(r.expire(t0 + Duration::from_secs(1)).is_none());
        assert_eq!(
            tag(r.expire(t0 + IDLE_ONLY_GRACE + Duration::from_secs(1))),
            Some(1)
        );
        // A late `idle` ends it directly.
        let mut r = AdapterRich::default();
        let _ = queued(&mut r, "a", 0);
        let _ = queued(&mut r, "b", 1);
        assert_eq!(
            tag(r.observe(
                AgentState::Idle,
                t0 + IDLE_ONLY_GRACE + Duration::from_secs(1)
            )),
            Some(1)
        );
        // Without any `idle` it is not read as over (only a cancel's grace or `done` can).
        let mut r = AdapterRich::default();
        let _ = queued(&mut r, "a", 0);
        assert!(r.expire(t0 + Duration::from_secs(3600)).is_none());
        assert!(is_busy(&r));
    }

    /// (d) An adapter that ignores `session/cancel` does not hold the agent for good: the turn is read as over after the grace, and
    /// what waits goes (or, with `drop_queue`, is gone).
    #[test]
    fn a_cancelled_turn_the_adapter_never_ends_is_over_after_the_grace() {
        let t0 = Instant::now();
        let mut r = AdapterRich::default();
        let _ = queued(&mut r, "a", 0);
        r.observe(AgentState::Working, t0);
        let _ = queued(&mut r, "b", 1);
        let g = gen(&r);
        assert!(r.cancelled(g, t0, false).is_empty(), "the queue is kept");
        assert!(r
            .expire(t0 + CANCEL_GRACE - Duration::from_millis(1))
            .is_none());
        assert_eq!(tag(r.expire(t0 + CANCEL_GRACE)), Some(1));
        // Not cancelled: never expires.
        assert!(r.expire(t0 + CANCEL_GRACE * 100).is_none());
        // With drop_queue the tags come back and the turn is free after the grace.
        let mut r = AdapterRich::default();
        let _ = queued(&mut r, "a", 0);
        let _ = queued(&mut r, "b", 5);
        let _ = queued(&mut r, "c", 6);
        assert_eq!(r.cancelled(gen(&r), t0, true), [5, 6]);
        assert!(r.expire(t0 + CANCEL_GRACE).is_none());
        assert_eq!(r.turn, Turn::Idle);
        // An adapter that does end the cancelled turn (the normal case) needs no grace.
        let mut r = AdapterRich::default();
        let _ = queued(&mut r, "a", 0);
        r.observe(AgentState::Working, t0);
        let _ = queued(&mut r, "b", 1);
        r.cancelled(gen(&r), t0, false);
        assert_eq!(tag(r.observe(AgentState::Idle, t0)), Some(1));
    }

    /// A late answer cannot corrupt a newer turn: a send that fails after the adapter restarted, and a cancel that comes back after
    /// a restart or after its turn ended, change nothing; and a tag is reported once.
    #[test]
    fn late_results_of_an_old_turn_or_process_change_nothing() {
        let t0 = Instant::now();
        // Restart during a send: the prompt that was being sent fails with the old process.
        let mut r = AdapterRich::default();
        let _ = queued(&mut r, "a", 0);
        let _ = queued(&mut r, "b", 1);
        let (b, gb) = {
            r.observe(AgentState::Working, t0);
            r.observe(AgentState::Idle, t0).unwrap()
        };
        assert_eq!(b.tag, 1);
        assert_eq!(
            r.forget(),
            Vec::<u64>::new(),
            "`b` is in flight, not queued"
        );
        let _ = queued(&mut r, "c", 2); // the new process's first prompt
        let (c_turn, g_new) = (is_busy(&r), gen(&r));
        assert!(c_turn && g_new != gb);
        let (sent, next) = r.send_result(false, gb, t0);
        assert!(!sent, "its entry is dropped");
        assert!(
            next.is_none() && is_busy(&r) && gen(&r) == g_new,
            "the new turn is untouched"
        );
        // Cancel after a restart: the old cancel call returns to the new process.
        let mut r = AdapterRich::default();
        let _ = queued(&mut r, "a", 0);
        let old = gen(&r);
        r.forget();
        let _ = queued(&mut r, "b", 1);
        let _ = queued(&mut r, "c", 2);
        assert!(r.cancelled(old, t0, true).is_empty(), "nothing is dropped");
        assert_eq!(r.queue.len(), 1);
        assert!(
            r.expire(t0 + CANCEL_GRACE * 10).is_none(),
            "and no clock was set on the new turn"
        );
        // Cancel for a turn that has since ended and another began.
        let mut r = AdapterRich::default();
        let _ = queued(&mut r, "a", 0);
        r.observe(AgentState::Working, t0);
        let _ = queued(&mut r, "b", 1);
        let old = gen(&r);
        assert_eq!(tag(r.observe(AgentState::Idle, t0)), Some(1));
        assert!(r.cancelled(old, t0, false).is_empty());
        assert!(r.expire(t0 + CANCEL_GRACE * 10).is_none());
    }

    /// ADR-0104 decision 1 (#1080): the kept file has what the earlier node recorded; the holder's `seen` lines have the reports it
    /// read after that and did not record before it stopped. On take-up the newest report of each kind in `seen` wins, a kind with no
    /// report there keeps the file's, and the request still waiting is returned as before (ADR-0093 decision 3).
    #[test]
    fn on_take_up_the_newest_reports_the_earlier_node_had_win_over_the_kept_file() {
        let line = |method: &str, params: serde_json::Value| {
            serde_json::json!({"jsonrpc": "2.0", "method": method, "params": params})
                .to_string()
                .into_bytes()
        };
        let modes = serde_json::json!([{"id": "default", "name": "Ask first"},
                                       {"id": "bypassPermissions", "name": "Bypass Permissions", "skipsPermissions": true}]);
        let mode = |current: &str| SessionOptions {
            modes: vec![
                SessionOption {
                    id: "default".into(),
                    name: "Ask first".into(),
                    ..SessionOption::default()
                },
                SessionOption {
                    id: "bypassPermissions".into(),
                    name: "Bypass Permissions".into(),
                    skips_permissions: true,
                    ..SessionOption::default()
                },
            ],
            current_mode: current.into(),
            ..SessionOptions::default()
        };
        let kept_commands = vec![AdapterCommand {
            name: "review".into(),
            ..AdapterCommand::default()
        }];
        // What the earlier node recorded: the start's mode, no config options yet, one command.
        let mut r = Reattach {
            session_id: "s-1".into(),
            options: mode("default"),
            config: Vec::new(),
            commands: kept_commands.clone(),
            ..Reattach::default()
        };
        let seen = vec![
            line(
                "session/options",
                serde_json::json!({"sessionId": "s-1", "modes": modes, "currentMode": "default"}),
            ),
            line(
                "approval/request",
                serde_json::json!({"sessionId": "s-1", "approvalId": "ap-1", "action": "run", "detail": "ls"}),
            ),
            line(
                "session/options",
                serde_json::json!({"sessionId": "s-1", "modes": modes, "currentMode": "bypassPermissions"}),
            ),
            line(
                "session/config",
                serde_json::json!({"sessionId": "s-1", "options": [{"id": "auto_approve", "name": "Auto approve",
                                   "type": "boolean", "current": "true"}]}),
            ),
        ];
        let waiting = newest_reported(&mut r, &seen);
        assert_eq!(waiting.len(), 1, "the waiting request is asked again");
        assert_eq!(r.options, mode("bypassPermissions"), "the newer mode won");
        assert_eq!(
            r.config
                .iter()
                .map(|o| (o.id.as_str(), o.current.as_str()))
                .collect::<Vec<_>>(),
            [("auto_approve", "true")],
            "the config options the earlier node read and did not record"
        );
        assert_eq!(r.commands, kept_commands, "no report in seen: the file's");
        // Judged as a report is once put back: the mode in use skips.
        let options = bounded_options(r.options.clone());
        assert!(options
            .modes
            .iter()
            .any(|m| m.id == options.current_mode && m.skips_permissions));
        // Nothing newer in seen: the kept file stands.
        let mut r = Reattach {
            session_id: "s-1".into(),
            options: mode("bypassPermissions"),
            commands: kept_commands.clone(),
            ..Reattach::default()
        };
        assert!(newest_reported(&mut r, &[]).is_empty());
        assert_eq!(r.options, mode("bypassPermissions"));
        assert_eq!(r.commands, kept_commands);
    }

    /// ADR-0090: what a restarted adapter's old process reported is forgotten (a stale mode list could decide a choice against the
    /// new process), and the prompts that waited for its turn are handed back to be dropped.
    #[test]
    fn a_restarted_adapters_old_reports_and_queue_are_forgotten() {
        let mut r = AdapterRich {
            config: bounded_config(vec![config("effort", "thought_level", "low", &["low"])]),
            commands: vec![AdapterCommand {
                name: "review".into(),
                ..AdapterCommand::default()
            }],
            ..AdapterRich::default()
        };
        r.busy(Instant::now(), true);
        r.queue.push_back(Queued {
            text: "a".into(),
            tag: 7,
        });
        r.queue.push_back(Queued {
            text: "b".into(),
            tag: 9,
        });
        assert_eq!(r.forget(), [7, 9]);
        assert!(r.config.is_empty() && r.commands.is_empty() && r.queue.is_empty());
        assert_eq!(r.turn, Turn::Idle);
        assert!(r.forget().is_empty());
    }

    #[test]
    fn reported_commands_are_bounded() {
        let c = |n: &str| AdapterCommand {
            name: n.into(),
            description: "d".repeat(900),
            input_hint: "h".repeat(900),
        };
        let mut list: Vec<AdapterCommand> = (0..150).map(|i| c(&format!("c{i}"))).collect();
        list.insert(0, c(""));
        list.insert(1, c(&"n".repeat(COMMAND_NAME_MAX + 1)));
        let out = bounded_commands(list);
        assert_eq!(out.len(), COMMANDS_MAX);
        assert_eq!(out[0].name, "c0");
        assert!(out
            .iter()
            .all(|c| c.description.len() <= OPTION_DESCRIPTION_MAX
                && c.input_hint.len() <= COMMAND_HINT_MAX));
    }

    #[test]
    fn an_adapters_reason_is_passed_on_only_for_known_codes_with_their_own_params() {
        let exited = HostError::Rpc {
            code: -32000,
            data: Some(serde_json::json!({"code": "adapter.agent_exited",
                "params": {"stage": "initialize", "stderr_tail": "x".repeat(2000), "extra": "no"}})),
        };
        let e = adapter_error(exited).to_protocol();
        assert_eq!(e.code, ErrorCode::AdapterAgentExited);
        let s = |k: &str| match e.params.get(k) {
            Some(agora_protocol::ParamValue::Str(s)) => s.clone(),
            other => panic!("{k}: {other:?}"),
        };
        assert_eq!(
            (s("stage").as_str(), s("exit_code").as_str()),
            ("initialize", "?")
        );
        assert_eq!(s("stderr_tail").len(), REASON_PARAM_MAX);
        assert!(!e.params.contains_key("extra"));
        let other = HostError::Rpc {
            code: -32000,
            data: Some(serde_json::json!({"code": "auth.forbidden", "params": {}})),
        };
        assert_eq!(adapter_error(other).to_protocol().code, ErrorCode::Internal);
        assert_eq!(
            adapter_error(HostError::Timeout).to_protocol().code,
            ErrorCode::Internal
        );
    }

    /// #1150: a cancel or a switch that fails without a stable reason is a failed call of that name, never a failed start (whose
    /// label the start steps keep); the caller's answer is unchanged. A known reason is passed on as for a start.
    #[test]
    fn a_failed_call_to_a_running_adapter_is_named_and_a_failed_start_stays_a_start() {
        assert!(matches!(
            call_error("cancel")(HostError::Timeout),
            SupervisorError::AdapterCall("cancel", _)
        ));
        let e = call_error("set_config")(HostError::Exited).to_protocol();
        assert_eq!(e.code, ErrorCode::Internal);
        assert_eq!(
            e.params.get("incident_id"),
            Some(&agora_protocol::ParamValue::Str("unassigned".into()))
        );
        let refused = HostError::Rpc {
            code: -32000,
            data: Some(
                serde_json::json!({"code": "adapter.option_refused", "params": {"id": "plan"}}),
            ),
        };
        assert_eq!(
            call_error("set_option")(refused).to_protocol().code,
            ErrorCode::AdapterOptionRefused
        );
        assert!(matches!(
            adapter_error(HostError::Timeout),
            SupervisorError::Spawn(_)
        ));
    }

    /// #1033: a switch that skips permissions refused by the policy names the same option when it is asked (`skip_ask`) and when an
    /// approved card is applied after the policy changed (`prepare_skip_apply`): a mode's id (the value; never the word `mode`), and a
    /// config option's own id.
    #[test]
    fn a_refused_switch_names_the_mode_id_or_the_config_id_when_asked_and_when_applied() {
        use crate::launch_options::SkipPermissions as S;
        let (mut sup, id) = switch_rig();
        let option = |r: Result<Prepared, SupervisorError>| match r {
            Err(err) => {
                let err = err.to_protocol();
                assert_eq!(err.code, ErrorCode::AgentOptionForbidden);
                match err.params.get("option") {
                    Some(agora_protocol::ParamValue::Str(s)) => s.clone(),
                    other => panic!("option: {other:?}"),
                }
            }
            Ok(_) => panic!("not refused"),
        };
        let ask = |r: Result<Prepared, SupervisorError>| match r {
            Ok(Prepared::Ask(ask)) => ask,
            _ => panic!("not asked"),
        };

        sup.set_skip_permissions(S::Allow);
        let mode = ask(sup.prepare_set_option(&id, OptionKind::Mode, "bypass"));
        let approvals = ask(sup.prepare_set_config(&id, "approvals", "turbo"));
        sup.set_skip_permissions(S::Forbid);
        assert_eq!(
            option(sup.prepare_set_option(&id, OptionKind::Mode, "bypass")),
            "bypass"
        );
        assert_eq!(
            option(sup.prepare_skip_apply(&mode).map(Prepared::Call)),
            "bypass",
            "the approved mode, by its id"
        );
        assert_eq!(
            option(sup.prepare_set_config(&id, "approvals", "turbo")),
            "approvals"
        );
        assert_eq!(
            option(sup.prepare_skip_apply(&approvals).map(Prepared::Call)),
            "approvals"
        );
    }

    /// #1150: the paths the start check keeps out of every grant (the control token, the node's own paths, its data folder and, once
    /// it holds a secret, its folder of secrets) go to the platform with the profile, which checks each grant again as it opens it.
    /// Only where a refusal is what keeps them closed (Linux); elsewhere nothing is added.
    #[test]
    fn the_paths_the_start_check_keeps_closed_go_to_the_platform() {
        let base = std::env::temp_dir().join(format!("agora-1150-closed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (home, data) = (base.join("node"), base.join("node/data"));
        std::fs::create_dir_all(&data).unwrap();
        let token = home.join("control.token");
        let mut sup = Supervisor::new(SupervisorConfig {
            node: NodeId::new("n"),
            restart: RestartPolicy::default(),
            terminal: TerminalOptions::default(),
            limits: NodeLimits::default(),
        });
        sup.set_sandbox(crate::sandbox::SandboxPolicy {
            level: crate::sandbox::Level::Worktree,
            protect: vec![home.clone()],
            data_dir: Some(data.clone()),
            control_token: Some(token.clone()),
            ..Default::default()
        });
        let store = crate::credentials::SecretStore::in_data(&data);
        sup.set_credentials(crate::credentials::Credentials::new(
            store.clone(),
            BTreeMap::new(),
        ));
        let work = base.join("work");
        let closed = |sup: &Supervisor| {
            let mut profile = agora_platform::Sandbox::new(vec![], vec![work.clone()]);
            sup.refuse_reachable("k", &mut profile).unwrap();
            profile.closed
        };
        let checked = vec![token.clone(), home.clone(), data.clone()];
        let want = |v: Vec<PathBuf>| {
            if agora_platform::PROTECTED_PATHS_BY_REFUSAL {
                v
            } else {
                Vec::new()
            }
        };
        assert_eq!(closed(&sup), want(checked.clone()));
        store
            .set("A_TOKEN", &crate::credentials::Secret::new("v").unwrap())
            .unwrap();
        let mut with_secrets = checked;
        with_secrets.push(store.dir().to_path_buf());
        assert_eq!(closed(&sup), want(with_secrets));
        let _ = std::fs::remove_dir_all(&base);
    }
}
