//! Agent supervision: start, stop, resume, and restart agents on this node.
//!
//! The supervisor owns the runtime of every agent (worktree, terminal actor, detector, restart state) and
//! produces plain [`SupervisorEvent`]s. It never talks to a hub and never waits for one: agents keep running
//! when the hub is unreachable and the caller buffers the events (AGENTS.md, nodes are autonomous). Time is
//! passed in, so restart and detection timing is testable without sleeping.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use agora_adapters::host::{Adapter, AdapterConfig, AdapterEvent};
use agora_adapters::{Detector, Manifest, Pacer};
use agora_core::{Agent, AgentId, AgentState, IntegrationLevel, NodeId, StateSource, WorktreeId};
use agora_platform::{spawn, ExitStatus, ProcessLimits, PtyConfig};
use agora_protocol::{Error, ErrorCode};
use agora_vt::Size;

use crate::git::{self, GitError};
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

/// Something that happened to an agent. Plain data, ready to become protocol events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupervisorEvent {
    Started {
        agent: AgentId,
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
    GaveUp {
        agent: AgentId,
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
    NotFound(AgentId),
    /// The agent is still running (resume applies to stopped agents).
    StillRunning(AgentId),
    Git(GitError),
    /// Starting the process failed; the text is for logs only.
    Spawn(String),
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
            Self::NotFound(id) => {
                Error::new(ErrorCode::AgentNotFound).with("agent_id", id.as_str())
            }
            Self::StillRunning(id) => {
                Error::new(ErrorCode::AgentBlocked).with("agent_id", id.as_str())
            }
            Self::Git(e) => e.to_protocol(),
            Self::Spawn(_) => Error::new(ErrorCode::Internal).with("incident_id", "unassigned"),
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
}

/// One startable agent kind, as reported by [`Supervisor::kinds`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KindInfo {
    pub kind: String,
    pub display_name: String,
    pub level: IntegrationLevel,
    pub installed: bool,
}

struct KindEntry {
    manifest: Manifest,
    detector: Detector,
}

/// An agent driven through an L3 adapter process instead of a terminal.
struct AdapterEntry {
    agent: Agent,
    adapter: Adapter,
    session_id: String,
    worktree: PathBuf,
    branch: String,
    running: bool,
    /// What starts the adapter process again, and the checkout its session belongs to.
    spec: agora_adapters::manifest::AdapterSpec,
    checkout_root: PathBuf,
    restarts: u32,
    restart_at: Option<Instant>,
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
    adapters: BTreeMap<AgentId, AdapterEntry>,
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
            adapters: BTreeMap::new(),
        }
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

    /// With confinement on, a launch in a folder that would make the confinement meaningless is refused before anything starts.
    fn refuse_protected(
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

    /// A start that failed after the worktree and the mesh endpoint were made: give both back, so repeated failures cannot pile up.
    fn abandon(&self, id: &AgentId, checkout: &std::path::Path, worktree: &std::path::Path) {
        if let Some(h) = &self.abort_hook {
            h(id);
        }
        let _ = git::remove_worktree(checkout, worktree, true);
    }

    /// Make an agent kind available. Replaces a previous manifest of the same kind.
    pub fn register(&mut self, manifest: Manifest) {
        let detector = Detector::new(&manifest);
        self.kinds
            .insert(manifest.kind.clone(), KindEntry { manifest, detector });
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

    /// Create the agent's own worktree and start it there.
    pub fn start(&mut self, req: &StartRequest, now: Instant) -> Result<AgentId, SupervisorError> {
        // Checked first: a refused start must not leave a worktree behind.
        self.check_limits()?;
        let kind = self
            .kinds
            .get(&req.kind)
            .ok_or_else(|| SupervisorError::UnknownKind(req.kind.clone()))?;
        // An agent that cannot be confined is not started unconfined (ADR-0047). Adapter-hosted agents are not confined yet.
        let driver = kind.manifest.driver(&bridge_program());
        if !self.sandbox.enforceable() {
            return Err(SupervisorError::SandboxUnavailable(
                req.kind.clone(),
                self.sandbox.level.name(),
            ));
        }
        if let Some(spec) = driver {
            if let Some(p) = spec.command.first().and_then(|c| {
                agora_adapters::package::revoked_publisher_of(
                    &agora_adapters::package::agora_home(),
                    c,
                )
            }) {
                return Err(SupervisorError::PublisherRevoked(req.kind.clone(), p));
            }
            return self.start_adapter(req, &spec);
        }
        let argv = kind.manifest.process.launch.clone();
        if argv.is_empty() {
            return Err(SupervisorError::UnknownKind(req.kind.clone()));
        }
        let checkout = git::discover_checkout(&req.checkout)?;
        let worktree = req.worktree_root.join(&req.name);
        let branch = format!("agent/{}", req.name);
        git::create_worktree(&checkout.root, &worktree, &branch, req.base.as_deref())?;

        let id = AgentId::new(format!("agent-{}", self.next_id));
        self.next_id += 1;
        let extras = match self.inject(
            &id,
            &req.kind,
            &checkout.project.to_string(),
            &worktree,
            None,
        ) {
            Ok(x) => x,
            Err(e) => {
                self.abandon(&id, &checkout.root, &worktree);
                return Err(e);
            }
        };
        let agent = Agent::new(
            id.clone(),
            checkout.project,
            self.cfg.node.clone(),
            Some(WorktreeId::new(worktree.to_string_lossy())),
            req.kind.clone(),
            IntegrationLevel::L1Manifest,
        );
        let terminal = match self.launch(&req.kind, &argv, &worktree, &extras) {
            Ok(t) => t,
            Err(e) => {
                self.abandon(&id, &checkout.root, &worktree);
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
        if !separate {
            self.refuse_protected(&req.checkout, &req.worktree_root)?;
            return self.start_in_directory(req, project, now);
        }
        self.check_limits()?;
        if !self.sandbox.enforceable() {
            return Err(SupervisorError::SandboxUnavailable(
                req.kind.clone(),
                self.sandbox.level.name(),
            ));
        }
        let target = req.worktree_root.join(&req.name);
        let relative = req
            .checkout
            .strip_prefix(git_root)
            .map_err(|e| SupervisorError::Spawn(e.to_string()))?;
        let branch = format!("agent/{}", req.name);
        git::create_worktree(git_root, &target, &branch, None)?;
        let mut direct = req.clone();
        let result = selected_worktree_directory(&target, relative)
            .map_err(|e| SupervisorError::Spawn(e.to_string()))
            .and_then(|cwd| {
                direct.checkout = cwd;
                self.start_directory_impl(&direct, project, now, false)
            });
        match result {
            Ok(id) => Ok(id),
            Err(e) => {
                let _ = git::remove_worktree(git_root, &target, true);
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
        self.start_directory_impl(req, project, now, true)
    }

    fn start_directory_impl(
        &mut self,
        req: &StartRequest,
        project: agora_core::ProjectId,
        now: Instant,
        private: bool,
    ) -> Result<AgentId, SupervisorError> {
        self.check_limits()?;
        if !self.sandbox.enforceable() {
            return Err(SupervisorError::SandboxUnavailable(
                req.kind.clone(),
                self.sandbox.level.name(),
            ));
        }
        let kind = self
            .kinds
            .get(&req.kind)
            .ok_or_else(|| SupervisorError::UnknownKind(req.kind.clone()))?;
        let driver = kind.manifest.driver(&bridge_program());
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
            let (adapter, session_id) = launch_adapter(
                &spec,
                &root,
                &worktree,
                None,
                self.sandbox_for(&req.kind, &worktree, &spec.command[0]),
            )
            .map_err(SupervisorError::Spawn)?;
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
                    adapter,
                    session_id,
                    worktree,
                    branch: String::new(),
                    running: true,
                    spec,
                    checkout_root: root,
                    restarts: 0,
                    restart_at: None,
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
                Ok(e) => e,
                Err(e) => {
                    if let Some(hook) = &self.abort_hook {
                        hook(&id);
                    }
                    return Err(e);
                }
            };
            let terminal = match self.launch(&req.kind, &argv, &worktree, &extras) {
                Ok(t) => t,
                Err(e) => {
                    if let Some(hook) = &self.abort_hook {
                        hook(&id);
                    }
                    return Err(e);
                }
            };
            let agent = Agent::new(
                id.clone(),
                project,
                self.cfg.node.clone(),
                None,
                req.kind.clone(),
                IntegrationLevel::L1Manifest,
            );
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
                },
            );
        }
        Ok(id)
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
        let Some(hook) = &self.hook else {
            return Ok(none());
        };
        // The hook registers the agent with the mesh even when its manifest has no `[mcp]` (it may reach the mesh
        // some other way); only a manifest with `[mcp]` gets anything injected.
        let endpoint = hook(&Launching {
            id: id.clone(),
            kind: kind.to_string(),
            project: project.to_string(),
            node: self.cfg.node.as_str().to_string(),
            worktree: worktree.to_path_buf(),
            identity: identity.clone(),
        });
        let (Some(endpoint), Some(spec)) = (
            endpoint,
            self.kinds.get(kind).and_then(|k| k.manifest.mcp.clone()),
        ) else {
            return Ok(none());
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
        Ok(Extras {
            read: private_config.map(Path::to_path_buf).into_iter().collect(),
            argv: plan.argv_extra,
            env: plan.env,
            identity,
        })
    }

    /// Starts an agent through its L3 adapter: own worktree, adapter process, `initialize`, `session/start`.
    fn start_adapter(
        &mut self,
        req: &StartRequest,
        spec: &agora_adapters::manifest::AdapterSpec,
    ) -> Result<AgentId, SupervisorError> {
        let checkout = git::discover_checkout(&req.checkout)?;
        let worktree = req.worktree_root.join(&req.name);
        let branch = format!("agent/{}", req.name);
        git::create_worktree(&checkout.root, &worktree, &branch, req.base.as_deref())?;
        if spec.command.is_empty() {
            return Err(SupervisorError::UnknownKind(req.kind.clone()));
        }
        let (adapter, session_id) = launch_adapter(
            spec,
            &checkout.root,
            &worktree,
            None,
            self.sandbox_for(&req.kind, &worktree, &spec.command[0]),
        )
        .map_err(SupervisorError::Spawn)?;
        let id = AgentId::new(format!("agent-{}", self.next_id));
        self.next_id += 1;
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
                adapter,
                session_id,
                worktree,
                branch,
                running: true,
                spec: spec.clone(),
                checkout_root: checkout.root,
                restarts: 0,
                restart_at: None,
            },
        );
        Ok(id)
    }

    /// Drains what adapters said since the last call: states are applied (a report always wins over anything
    /// else), the rest is passed on as events. A dead adapter ends its agent as `unknown`.
    fn poll_adapters(&mut self, now: Instant, events: &mut Vec<SupervisorEvent>) {
        let policy = self.cfg.restart;
        let due: Vec<AgentId> = self
            .adapters
            .iter()
            .filter(|(_, e)| e.restart_at.is_some_and(|t| now >= t))
            .map(|(id, _)| id.clone())
            .collect();
        for id in due {
            self.restart_adapter(&id, now, events);
        }
        for (id, e) in &mut self.adapters {
            if !e.running {
                continue;
            }
            for ev in e.adapter.poll() {
                match ev {
                    AdapterEvent::State { state, .. } => {
                        if e.agent.observe(state, StateSource::Reported) {
                            events.push(SupervisorEvent::StateChanged {
                                agent: id.clone(),
                                state,
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
                        if e.agent.observe(AgentState::Unknown, StateSource::Reported) {
                            events.push(SupervisorEvent::StateChanged {
                                agent: id.clone(),
                                state: AgentState::Unknown,
                            });
                        }
                        events.push(SupervisorEvent::AdapterCrashed { agent: id.clone() });
                        schedule_adapter_restart(id, e, now, &policy, events);
                    }
                    other => events.push(SupervisorEvent::Adapter {
                        agent: id.clone(),
                        event: other,
                    }),
                }
            }
        }
    }

    /// Starts a crashed adapter again and resumes its session where the adapter supports it. A failed attempt
    /// counts and backs off like a crash.
    fn restart_adapter(&mut self, id: &AgentId, now: Instant, events: &mut Vec<SupervisorEvent>) {
        let policy = self.cfg.restart;
        let Some(profile) = self.adapters.get(id).and_then(|e| {
            Some(self.sandbox_for(&e.agent.kind, &e.worktree, e.spec.command.first()?))
        }) else {
            return;
        };
        let Some(e) = self.adapters.get_mut(id) else {
            return;
        };
        e.restart_at = None;
        e.restarts += 1;
        match launch_adapter(
            &e.spec,
            &e.checkout_root,
            &e.worktree,
            e.agent.session.as_deref(),
            profile,
        ) {
            Ok((adapter, session_id)) => {
                e.adapter = adapter;
                e.session_id = session_id;
                e.running = true;
                e.agent.process_restarted();
                events.push(SupervisorEvent::Started { agent: id.clone() });
                events.extend(session_report(&e.agent));
            }
            Err(_) => schedule_adapter_restart(id, e, now, &policy, events),
        }
    }

    fn adapter_of(&self, id: &AgentId) -> Result<&AdapterEntry, SupervisorError> {
        self.adapters
            .get(id)
            .filter(|e| e.running)
            .ok_or_else(|| SupervisorError::NotFound(id.clone()))
    }

    /// Sends text to an adapter agent (user input; mesh content uses `from_mesh`).
    pub fn adapter_prompt(
        &self,
        id: &AgentId,
        text: &str,
        from_mesh: bool,
    ) -> Result<(), SupervisorError> {
        let e = self.adapter_of(id)?;
        e.adapter
            .prompt(&e.session_id, text, from_mesh, Duration::from_secs(10))
            .map_err(|e| SupervisorError::Spawn(e.to_string()))
    }

    /// Delivers a decision the node has already verified (ADR-0020) to the adapter that asked.
    pub fn adapter_resolve_approval(
        &self,
        id: &AgentId,
        approval_id: &str,
        approved: bool,
    ) -> Result<(), SupervisorError> {
        let e = self.adapter_of(id)?;
        e.adapter
            .resolve_approval(
                &e.session_id,
                approval_id,
                approved,
                Duration::from_secs(10),
            )
            .map_err(|e| SupervisorError::Spawn(e.to_string()))
    }

    fn launch(
        &self,
        kind: &str,
        argv: &[String],
        worktree: &std::path::Path,
        extras: &Extras,
    ) -> Result<TerminalHandle, SupervisorError> {
        let mut argv = argv.to_vec();
        argv.extend(extras.argv.iter().cloned());
        let program = argv.first().cloned();
        let mut cfg = PtyConfig::new(argv);
        cfg.cwd = Some(worktree.to_path_buf());
        cfg.cols = self.cfg.terminal.size.cols;
        cfg.rows = self.cfg.terminal.size.rows;
        cfg.limits = self.cfg.limits.process;
        cfg.sandbox = program.and_then(|p| self.sandbox_for(kind, worktree, &p));
        cfg.identity = extras.identity.clone();
        if let Some(profile) = &mut cfg.sandbox {
            profile.read.extend(extras.read.iter().cloned());
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
        if cfg.sandbox.is_some() && agora_platform::PRIVATE_TEMP {
            let temp = crate::sandbox::agent_temp(worktree).display().to_string();
            cfg.env.push(("TMPDIR".into(), temp.clone()));
            cfg.env.push(("TEMP".into(), temp.clone()));
            cfg.env.push(("TMP".into(), temp));
        }
        let pty = spawn(&cfg).map_err(|e| {
            // A folder the confinement cannot keep closed (a broad write grant, ADR-0050 Amendment B, 4) or whose label cannot be set
            // (2) is refused before the agent starts: `agent.sandbox_unavailable` with the reason, not an internal error.
            match agora_platform::sandbox_refusal(&e) {
                Some(why) => SupervisorError::SandboxRefused(
                    kind.to_string(),
                    self.sandbox.level.name(),
                    why.token(),
                ),
                None => SupervisorError::Spawn(e.to_string()),
            }
        })?;
        TerminalHandle::start(pty, self.cfg.terminal)
            .map_err(|e| SupervisorError::Spawn(e.to_string()))
    }

    /// Drive detection, exits, and restarts. Call regularly (a few times per second is plenty); the
    /// per-agent pacer decides when a screen is actually inspected. Returns the events since the last call.
    pub fn tick(&mut self, now: Instant) -> Vec<SupervisorEvent> {
        let mut events = Vec::new();
        let ids: Vec<AgentId> = self.agents.keys().cloned().collect();
        for id in ids {
            self.tick_agent(&id, now, &mut events);
        }
        self.poll_adapters(now, &mut events);
        events
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
                events.push(SupervisorEvent::GaveUp { agent: id.clone() });
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
        let (kind, worktree, session, extras) = (
            entry.agent.kind.clone(),
            entry.worktree.clone(),
            entry.agent.session.clone(),
            entry.extras.clone(),
        );
        let (argv, _native) = self.argv_for_resume(&kind, session.as_deref());
        let result = self.launch(&kind, &argv, &worktree, &extras);
        let Some(entry) = self.agents.get_mut(id) else {
            return;
        };
        entry.restart_at = None;
        entry.restarts += 1;
        match result {
            Ok(t) => {
                entry.terminal = Some(t);
                entry.pacer = Pacer::default();
                entry.next_check = now;
                entry.agent.process_restarted();
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
                    events.push(SupervisorEvent::GaveUp { agent: id.clone() });
                }
            }
        }
    }

    /// Native resume when the manifest has a resume command and the agent reported a session id; otherwise
    /// the plain launch command (layout-only fallback). The bool is true for the native case.
    fn argv_for_resume(&self, kind: &str, session: Option<&str>) -> (Vec<String>, bool) {
        let Some(k) = self.kinds.get(kind) else {
            return (Vec::new(), false);
        };
        match (&k.manifest.resume, session) {
            (Some(r), Some(sid)) => (
                r.argv
                    .iter()
                    .map(|a| a.replace("{session_id}", sid))
                    .collect(),
                true,
            ),
            _ => (k.manifest.process.launch.clone(), false),
        }
    }

    /// Stop an agent for good: kill its process group and cancel any pending restart. The worktree and its
    /// branch are kept (the work is the user's).
    pub fn stop(&mut self, id: &AgentId) -> Result<Vec<SupervisorEvent>, SupervisorError> {
        if let Some(e) = self.adapters.get_mut(id) {
            let mut events = Vec::new();
            e.restart_at = None;
            if e.running {
                let _ = e
                    .adapter
                    .stop_session(&e.session_id, "stop", Duration::from_secs(5));
                e.running = false;
            }
            if e.agent.observe(AgentState::Done, StateSource::Reported) {
                events.push(SupervisorEvent::StateChanged {
                    agent: id.clone(),
                    state: AgentState::Done,
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

    /// Start a stopped (or given-up) agent again in its worktree.
    pub fn resume(
        &mut self,
        id: &AgentId,
        now: Instant,
    ) -> Result<Vec<SupervisorEvent>, SupervisorError> {
        if let Some(entry) = self.adapters.get(id) {
            if entry.running || entry.restart_at.is_some() {
                return Err(SupervisorError::StillRunning(id.clone()));
            }
            self.check_limits()?;
            if !self.sandbox.enforceable() {
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
            let profile = entry
                .spec
                .command
                .first()
                .and_then(|p| self.sandbox_for(&entry.agent.kind, &entry.worktree, p));
            let native = entry.agent.session.is_some();
            let (adapter, session_id) = launch_adapter(
                &entry.spec,
                &entry.checkout_root,
                &entry.worktree,
                entry.agent.session.as_deref(),
                profile,
            )
            .map_err(SupervisorError::Spawn)?;
            let entry = self
                .adapters
                .get_mut(id)
                .ok_or_else(|| SupervisorError::NotFound(id.clone()))?;
            entry.adapter = adapter;
            entry.session_id = session_id;
            entry.running = true;
            entry.restart_at = None;
            entry.restarts = 0;
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
        let (kind, worktree, session, project) = (
            entry.agent.kind.clone(),
            entry.worktree.clone(),
            entry.agent.session.clone(),
            entry.agent.project.to_string(),
        );
        let (argv, native) = self.argv_for_resume(&kind, session.as_deref());
        // A stopped agent left the mesh and its endpoint closed; resuming joins again with a fresh one.
        let config = self.agents.get(id).and_then(|e| e.private_config.clone());
        let extras = self.inject(id, &kind, &project, &worktree, config.as_deref())?;
        let terminal = self.launch(&kind, &argv, &worktree, &extras)?;
        let Some(entry) = self.agents.get_mut(id) else {
            return Err(SupervisorError::NotFound(id.clone()));
        };
        entry.extras = extras;
        entry.terminal = Some(terminal);
        entry.pacer = Pacer::default();
        entry.next_check = now;
        entry.restarts = 0;
        entry.gave_up = false;
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
    pub fn report_session(&mut self, id: &AgentId, session: &str) -> Result<(), SupervisorError> {
        let entry = self
            .agents
            .get_mut(id)
            .ok_or_else(|| SupervisorError::NotFound(id.clone()))?;
        entry.agent.session = Some(session.to_string());
        Ok(())
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
        }));
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    fn usage_target(&self, id: &AgentId) -> Option<crate::usage_files::Target> {
        let e = self.agents.get(id)?;
        let spec = self
            .kinds
            .get(&e.agent.kind)?
            .manifest
            .usage
            .as_ref()?
            .file
            .clone()?;
        Some(crate::usage_files::Target {
            agent: id.clone(),
            worktree: e.worktree.clone(),
            spec,
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
            .map(|k| {
                let m = &k.manifest;
                let acp = m.structured.as_ref().and_then(|s| s.acp.as_ref());
                let (level, program) = match (&m.adapter, acp) {
                    (Some(a), _) => (IntegrationLevel::L3Adapter, a.command.first()),
                    (None, Some(a)) => (IntegrationLevel::L2Acp, a.argv.first()),
                    (None, None) => (IntegrationLevel::L1Manifest, m.process.launch.first()),
                };
                KindInfo {
                    kind: m.kind.clone(),
                    display_name: m.display_name.clone(),
                    level,
                    installed: program.is_some_and(|p| crate::doctor::on_path(p)),
                }
            })
            .collect()
    }

    /// Terminal size used for new agents (for callers that attach clients).
    pub fn terminal_size(&self) -> Size {
        self.cfg.terminal.size
    }
}

/// The program that runs `acp-bridge` for ACP agents: this node's own binary (`agora`), unless `AGORA_ACP_BRIDGE` names another
/// (tests, packaging).
fn bridge_program() -> PathBuf {
    std::env::var_os("AGORA_ACP_BRIDGE")
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok())
        .unwrap_or_else(|| PathBuf::from("agora"))
}

/// Spawns an adapter process, initializes it and starts (or resumes) a session. Returns it with the session id.
fn launch_adapter(
    spec: &agora_adapters::manifest::AdapterSpec,
    checkout_root: &Path,
    worktree: &Path,
    resume: Option<&str>,
    sandbox: Option<agora_platform::Sandbox>,
) -> Result<(Adapter, String), String> {
    const STEP: Duration = Duration::from_secs(10);
    let (program, args) = spec.command.split_first().ok_or("empty adapter command")?;
    let adapter = Adapter::spawn(&AdapterConfig {
        program: program.into(),
        args: args.to_vec(),
        env: spec
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        cwd: Some(worktree.to_path_buf()),
        sandbox,
    })
    .map_err(|e| e.to_string())?;
    let fail = |e: agora_adapters::host::HostError| e.to_string();
    adapter.initialize(&["chat"], "en", STEP).map_err(fail)?;
    let session_id = adapter
        .start_session(
            &checkout_root.to_string_lossy(),
            &worktree.to_string_lossy(),
            resume,
            STEP,
        )
        .map_err(fail)?;
    Ok((adapter, session_id))
}

/// After an adapter died on its own: wait out the backoff and try again, or give up for good.
fn schedule_adapter_restart(
    id: &AgentId,
    e: &mut AdapterEntry,
    now: Instant,
    policy: &RestartPolicy,
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
        events.push(SupervisorEvent::GaveUp { agent: id.clone() });
    }
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
