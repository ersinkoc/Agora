//! Agent supervision: start, stop, resume, and restart agents on this node.
//!
//! The supervisor owns the runtime of every agent (worktree, terminal actor, detector, restart state) and
//! produces plain [`SupervisorEvent`]s. It never talks to a hub and never waits for one: agents keep running
//! when the hub is unreachable and the caller buffers the events (AGENTS.md, nodes are autonomous). Time is
//! passed in, so restart and detection timing is testable without sleeping.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

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
}

/// Called before an agent's process starts. Returns the endpoint to inject into its MCP configuration, or `None`
/// to start it without one (the mesh is an addition; an agent never fails to start because of it).
pub type LaunchHook = Box<dyn Fn(&Launching) -> Option<agora_adapters::inject::Endpoint> + Send>;

/// What injection adds to every (re)launch of one agent.
#[derive(Debug, Clone, Default)]
struct Extras {
    argv: Vec<String>,
    env: Vec<(String, String)>,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupervisorError {
    UnknownKind(String),
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
            Self::UnknownKind(k) => {
                Error::new(ErrorCode::AgentKindUnknown).with("kind", k.as_str())
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
    pub id: AgentId,
    pub kind: String,
    pub state: AgentState,
    pub worktree: PathBuf,
    pub branch: String,
    pub running: bool,
    pub restarts: u32,
}

struct KindEntry {
    manifest: Manifest,
    detector: Detector,
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
}

pub struct Supervisor {
    cfg: SupervisorConfig,
    kinds: BTreeMap<String, KindEntry>,
    agents: BTreeMap<AgentId, Entry>,
    next_id: u64,
    hook: Option<LaunchHook>,
}

impl Supervisor {
    pub fn new(cfg: SupervisorConfig) -> Self {
        Self {
            cfg,
            kinds: BTreeMap::new(),
            agents: BTreeMap::new(),
            next_id: 1,
            hook: None,
        }
    }

    /// Installs the hook that gives each launching agent a mesh endpoint.
    pub fn set_launch_hook(&mut self, hook: LaunchHook) {
        self.hook = Some(hook);
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
        let active = self.active();
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
        let extras = self.inject(&id, &req.kind, &checkout.project.to_string(), &worktree)?;
        let agent = Agent::new(
            id.clone(),
            checkout.project,
            self.cfg.node.clone(),
            Some(WorktreeId::new(worktree.to_string_lossy())),
            req.kind.clone(),
            IntegrationLevel::L1Manifest,
        );
        let terminal = self.launch(&req.kind, &argv, &worktree, &extras)?;
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
            },
        );
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
    ) -> Result<Extras, SupervisorError> {
        let Some(hook) = &self.hook else {
            return Ok(Extras::default());
        };
        // The hook registers the agent with the mesh even when its manifest has no `[mcp]` (it may reach the mesh
        // some other way); only a manifest with `[mcp]` gets anything injected.
        let endpoint = hook(&Launching {
            id: id.clone(),
            kind: kind.to_string(),
            project: project.to_string(),
            node: self.cfg.node.as_str().to_string(),
            worktree: worktree.to_path_buf(),
        });
        let (Some(endpoint), Some(spec)) = (
            endpoint,
            self.kinds.get(kind).and_then(|k| k.manifest.mcp.clone()),
        ) else {
            return Ok(Extras::default());
        };
        let plan = agora_adapters::inject::plan(&spec, worktree, &endpoint)
            .map_err(|e| SupervisorError::Spawn(format!("mcp injection: {e}")))?;
        for (path, contents) in &plan.files {
            write_in_worktree(worktree, path, contents)
                .map_err(|e| SupervisorError::Spawn(format!("mcp injection: {e}")))?;
        }
        Ok(Extras {
            argv: plan.argv_extra,
            env: plan.env,
        })
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
        let mut cfg = PtyConfig::new(argv);
        cfg.cwd = Some(worktree.to_path_buf());
        cfg.cols = self.cfg.terminal.size.cols;
        cfg.rows = self.cfg.terminal.size.rows;
        cfg.limits = self.cfg.limits.process;
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
        let pty = spawn(&cfg).map_err(|e| SupervisorError::Spawn(e.to_string()))?;
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
        let entry = self
            .agents
            .get_mut(id)
            .ok_or_else(|| SupervisorError::NotFound(id.clone()))?;
        let mut events = Vec::new();
        entry.restart_at = None;
        // Dropping the handle kills the process and joins the actor.
        entry.terminal = None;
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
        let extras = self.inject(id, &kind, &project, &worktree)?;
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
        self.agents
            .iter()
            .map(|(id, e)| AgentView {
                id: id.clone(),
                kind: e.agent.kind.clone(),
                state: e.agent.state(),
                worktree: e.worktree.clone(),
                branch: e.branch.clone(),
                running: e.terminal.is_some(),
                restarts: e.restarts,
            })
            .collect()
    }

    /// Terminal size used for new agents (for callers that attach clients).
    pub fn terminal_size(&self) -> Size {
        self.cfg.terminal.size
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
    git::exclude_locally(worktree, rel);
    Ok(())
}
