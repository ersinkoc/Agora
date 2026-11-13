//! The node's local control socket: protocol commands over a local stream (docs/specs/protocol.md §2, §4).
//!
//! Framing is the protocol envelope (`agora_protocol::Frame`). Requests are `CommandRequest` payloads on stream 1
//! (`KIND_REQUEST`); responses are `CommandResponse` payloads on stream 1 (`KIND_RESPONSE`). A terminal stream is
//! opened by the client sending its `TerminalSubscribe` request on a dynamic stream id (3 or higher) of its
//! choosing; after the response, the node writes that terminal's frames on that stream (`KIND_TERM_*`), starting
//! with a snapshot. Input and resize are ordinary commands (`TerminalInput`, `TerminalResize`). The socket is owner-only,
//! so no further authentication is needed locally; the hub path adds device auth and approvals.
//!
//! Retried requests (same `idempotency_key`) get the stored response and do not repeat their effect.

use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use agora_core::AgentId;
use agora_platform::{local_connect, local_listen, LocalStream};
use agora_protocol::frame::{DEFAULT_MAX_PAYLOAD, STREAM_COMMANDS};
use agora_protocol::v1::{self, command_request::Command as Cmd, command_response::Result as Res};
use agora_protocol::{Decoder, Error, ErrorCode, Frame};
use agora_vt::Size;
use prost::Message;

use crate::daemon::EventSink;
use crate::git;
use crate::supervisor::{StartRequest, Supervisor, SupervisorError};
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
    /// Set after an agent starts: the node's Git watcher then checks its folder at once instead of on its next round.
    git_wake: Option<Arc<AtomicBool>>,
    names: AtomicU64,
    seen: Mutex<IdempotencyCache>,
}

#[derive(Default)]
struct IdempotencyCache {
    order: VecDeque<String>,
    responses: HashMap<String, v1::CommandResponse>,
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
                worktree_root.join(".directory-bindings.json"),
            )),
            worktree_root,
            node_id,
            events,
            mesh,
            approvals,
            chat,
            names: AtomicU64::new(0),
            manifests_dir: None,
            git_wake: None,
            seen: Mutex::new(IdempotencyCache::default()),
        }
    }

    /// Wakes the node's Git watcher (`daemon::watch_git`) after an agent starts, so the new folder's changes are counted at once and
    /// not up to a poll later.
    #[must_use]
    pub fn with_git_wake(mut self, wake: Arc<AtomicBool>) -> Self {
        self.git_wake = Some(wake);
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
        for m in found {
            sup.register(m);
        }
    }

    fn sup(&self) -> MutexGuard<'_, Supervisor> {
        self.supervisor.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn remembered(&self, key: &str) -> Option<v1::CommandResponse> {
        let c = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        c.responses.get(key).cloned()
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

fn ok_empty() -> v1::CommandOk {
    v1::CommandOk {
        result: Some(v1::command_ok::Result::Empty(v1::Empty {})),
    }
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

fn sup_err(e: SupervisorError) -> Error {
    e.to_protocol()
}

/// Run one command. Terminal subscription is handled by the connection (it needs the stream).
fn execute(ctx: &ControlContext, cmd: &Cmd) -> Result<v1::CommandOk, Error> {
    use v1::command_ok::Result as Ok_;
    if matches!(
        cmd,
        Cmd::AgentKinds(_) | Cmd::AgentStart(_) | Cmd::AgentLaunch(_)
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
            // One lock for the whole answer (the mutex is not reentrant).
            let sup = ctx.sup();
            let agents = sup
                .list()
                .into_iter()
                .map(|v| {
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
                })
                .collect();
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
            let (info, project) = crate::directory::inspect(&a.path, &ctx.node_id, false)?;
            if info.path != a.path || info.project_id != a.project_id {
                return Err(invalid("directory.project_id"));
            }
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
                .start_selected(
                    &req,
                    project,
                    Path::new(&info.git_root),
                    a.separate_worktree,
                    Instant::now(),
                )
                .map_err(sup_err)?;
            let view = sup
                .list()
                .into_iter()
                .find(|v| v.id == id)
                .ok_or_else(|| invalid("agent.id"))?;
            let has_terminal = sup.terminal(&id).is_some();
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
                ev.directory_agent_launched(
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
                );
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
            let id = sup.start(&req, Instant::now()).map_err(sup_err)?;
            let view = sup.list().into_iter().find(|v| v.id == id);
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
            drop(sup);
            if let Some(ev) = &ctx.events {
                ev.agent_launched(
                    &id,
                    &project.id,
                    &ctx.node_id,
                    &project.root,
                    std::path::Path::new(&worktree),
                    &branch,
                    &a.kind,
                    has_terminal,
                );
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
            let events = ctx.sup().stop(&id).map_err(sup_err)?;
            if let Some(mesh) = &ctx.mesh {
                mesh.unregister(&id);
            }
            if let Some(ev) = &ctx.events {
                ev.supervisor_events(&events, &ctx.sup().list());
            }
            Ok(ok_empty())
        }
        Cmd::AgentPrompt(a) => {
            let id = AgentId::new(a.agent_id.clone());
            let sup = ctx.sup();
            if sup.terminal(&id).is_none() {
                // An adapter agent takes prompts through its adapter; the request is part of its conversation.
                return sup
                    .adapter_prompt(&id, &a.text, false)
                    .map(|()| {
                        ctx.chat.push(
                            &id,
                            v1::ChatRole::User,
                            v1::ChatKind::Text,
                            a.text.clone(),
                            String::new(),
                            crate::daemon::now_ms(),
                        );
                        ok_empty()
                    })
                    .map_err(sup_err);
            }
            let control = sup
                .terminal(&id)
                .and_then(|h| h.input_handle().ok())
                .ok_or_else(|| {
                    Error::new(ErrorCode::AgentNotFound).with("agent_id", a.agent_id.as_str())
                })?;
            // The write happens after the supervisor lock is let go. The text and the Enter key are two writes: a terminal
            // program that reads both in one go takes them for a paste and keeps the Enter as a newline in its input box
            // (seen with WrongStack, whose prompt then never ran).
            drop(sup);
            let gone =
                |_| Error::new(ErrorCode::AgentNotFound).with("agent_id", a.agent_id.as_str());
            control.write_input(a.text.as_bytes()).map_err(gone)?;
            std::thread::sleep(std::time::Duration::from_millis(150));
            control.write_input(b"\r").map_err(gone)?;
            Ok(ok_empty())
        }
        // ADR-0058: the user's own switch; the supervisor refuses a terminal agent, an adapter without the switch and an id the
        // agent did not offer.
        Cmd::AgentSetMode(a) => ctx
            .sup()
            .adapter_set_option(
                &AgentId::new(a.agent_id.clone()),
                agora_adapters::host::OptionKind::Mode,
                &a.mode,
            )
            .map(|()| ok_empty())
            .map_err(sup_err),
        Cmd::AgentSetModel(a) => ctx
            .sup()
            .adapter_set_option(
                &AgentId::new(a.agent_id.clone()),
                agora_adapters::host::OptionKind::Model,
                &a.model,
            )
            .map(|()| ok_empty())
            .map_err(sup_err),
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
            if cols == 0 || rows == 0 || cols > MAX_TERMINAL_COLS || rows > MAX_TERMINAL_ROWS {
                return Err(invalid("cols"));
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
            let (agent, adapter_id, verdict) = approvals.resolve(r, now).map_err(|e| match e {
                ApprovalError::Unknown => invalid("approval_id"),
                ApprovalError::Expired => Error::new(ErrorCode::ApprovalExpired)
                    .with("approval_id", r.approval_id.as_str()),
                ApprovalError::BadSignature => Error::new(ErrorCode::ApprovalSignatureInvalid)
                    .with("approval_id", r.approval_id.as_str()),
                ApprovalError::Forbidden => {
                    Error::new(ErrorCode::AuthForbidden).with("action", "approval.resolve")
                }
            })?;
            let approved = verdict == crate::approvals::Verdict::Approved;
            ctx.sup()
                .adapter_resolve_approval(&agent, &adapter_id, approved)
                .map_err(sup_err)?;
            if let Some(ev) = &ctx.events {
                ev.approval_resolved(&r.approval_id, approved, &r.signer_device_id);
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
            let mut diff = git::diff(&wt, path).map_err(|e| e.to_protocol())?;
            if diff.len() > MAX_DIFF_BYTES {
                let mut end = MAX_DIFF_BYTES;
                while !diff.is_char_boundary(end) {
                    end -= 1;
                }
                diff.truncate(end);
            }
            Ok(v1::CommandOk {
                result: Some(Ok_::GitDiff(v1::GitDiffResult { diff })),
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
    crate::files::Files::open(&wt, MAX_READ_BYTES).map_err(|e| e.to_protocol())
}

/// Largest diff one answer carries; a bigger one is cut (the file view is for reading, not for patching).
const MAX_DIFF_BYTES: usize = 512 * 1024;

/// The workspace of one of this node's agents, by the id the node reported it under. Only those: a client cannot
/// point a git command at any other directory.
fn agent_worktree(ctx: &ControlContext, worktree_id: &str) -> Result<PathBuf, Error> {
    ctx.sup()
        .list()
        .into_iter()
        .find(|v| v.worktree.to_string_lossy() == worktree_id)
        .map(|v| v.worktree)
        .ok_or_else(|| invalid("worktree_id"))
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
        Cmd::AgentLaunch(_) => "agent.launch",
        Cmd::WorkspaceQuery(_) => "workspace.query",
        Cmd::AgentStop(_) => "agent.stop",
        Cmd::AgentPrompt(_) => "agent.prompt",
        Cmd::AgentSetMode(_) => "agent.set_mode",
        Cmd::AgentSetModel(_) => "agent.set_model",
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
        Cmd::BoardCall(_) => "board.call",
        Cmd::BoardJoin(_) => "board.join",
        Cmd::NodePolicyGet(_) => "node.policy.get",
        Cmd::GrantCreate(_) => "grant.create",
        Cmd::GrantList(_) => "grant.list",
        Cmd::GrantRevoke(_) => "grant.revoke",
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
}

impl ControlServer {
    pub fn start(path: &Path, ctx: Arc<ControlContext>) -> io::Result<Self> {
        let listener = local_listen(path)?;
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("agora-control".into())
            .spawn(move || {
                while !stop2.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok(stream) => {
                            let (ctx, stop) = (Arc::clone(&ctx), Arc::clone(&stop2));
                            let _ = std::thread::Builder::new()
                                .name("agora-control-conn".into())
                                .spawn(move || handle_connection(stream, &ctx, &stop));
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(POLL),
                        Err(_) => std::thread::sleep(POLL),
                    }
                }
            })?;
        Ok(Self {
            stop,
            thread: Some(thread),
            path: path.to_path_buf(),
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
            let _ = t.join();
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
                    if frame.kind == KIND_REQUEST {
                        handle_request(&frame, ctx, &writer, &alive, stop);
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
) {
    let Ok(req) = v1::CommandRequest::decode(frame.payload.as_slice()) else {
        return;
    };
    if !req.idempotency_key.is_empty() {
        if let Some(mut prev) = ctx.remembered(&req.idempotency_key) {
            // Same result, addressed to this attempt: the client matches responses by request id.
            prev.request_id.clone_from(&req.request_id);
            let _ = send(writer, STREAM_COMMANDS, KIND_RESPONSE, prev.encode_to_vec());
            return;
        }
    }
    let Some(cmd) = &req.command else {
        let resp = response(&req.request_id, Err(invalid("command")));
        let _ = send(writer, STREAM_COMMANDS, KIND_RESPONSE, resp.encode_to_vec());
        return;
    };

    // Terminal subscriptions need the stream the request arrived on.
    if let Cmd::TerminalSubscribe(sub) = cmd {
        let outcome = subscribe(ctx, sub, frame.stream_id);
        let resp = response(
            &req.request_id,
            outcome.as_ref().map(|_| ok_empty()).map_err(Clone::clone),
        );
        let _ = send(writer, STREAM_COMMANDS, KIND_RESPONSE, resp.encode_to_vec());
        if let Ok((agent, snap, sub)) = outcome {
            let (ctx, writer, alive, stop) = (
                Arc::clone(ctx),
                Arc::clone(writer),
                Arc::clone(alive),
                Arc::clone(stop),
            );
            let stream_id = frame.stream_id;
            let _ = std::thread::Builder::new()
                .name("agora-control-term".into())
                .spawn(move || pump(&ctx, &writer, stream_id, &agent, &snap, &sub, &alive, &stop));
        }
        return;
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
        if let Ok((entries, rx)) = outcome {
            let (writer, alive, stop) = (Arc::clone(writer), Arc::clone(alive), Arc::clone(stop));
            let stream_id = frame.stream_id;
            let _ = std::thread::Builder::new()
                .name("agora-control-chat".into())
                .spawn(move || pump_chat(&writer, stream_id, entries, &rx, &alive, &stop));
        }
        return;
    }

    let resp = response(&req.request_id, execute(ctx, cmd));
    if !req.idempotency_key.is_empty() {
        ctx.remember(&req.idempotency_key, &resp);
    }
    let _ = send(writer, STREAM_COMMANDS, KIND_RESPONSE, resp.encode_to_vec());
}

fn subscribe(
    ctx: &ControlContext,
    sub: &v1::TerminalSubscribe,
    stream_id: u32,
) -> Result<(AgentId, Snapshot, OutputSubscription), Error> {
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
    let (snap, subscription) = handle.subscribe();
    drop(guard);
    Ok((id, snap, subscription))
}

fn exit_payload(st: &agora_platform::ExitStatus) -> Vec<u8> {
    let code = st
        .code
        .or_else(|| st.signal.map(|s| -s))
        .unwrap_or(i32::MIN);
    code.to_le_bytes().to_vec()
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
    snap: &Snapshot,
    sub: &OutputSubscription,
    alive: &AtomicBool,
    stop: &AtomicBool,
) {
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
            Ok(Output::Lagged) => {
                let snap = ctx.sup().terminal(agent).map(|h| h.resync(sub));
                match snap {
                    Some(s) => repaint(&s),
                    None => return,
                }
            }
            Err(RecvTimeoutError::Timeout) => Ok(()),
            Err(RecvTimeoutError::Disconnected) => return,
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
    rx: &std::sync::mpsc::Receiver<v1::ChatEntry>,
    alive: &AtomicBool,
    stop: &AtomicBool,
) {
    let snapshot = v1::ChatSnapshot { entries };
    if send(writer, stream, KIND_CHAT_SNAPSHOT, snapshot.encode_to_vec()).is_err() {
        return;
    }
    while alive.load(Ordering::Acquire) && !stop.load(Ordering::Acquire) {
        match rx.recv_timeout(POLL) {
            Ok(e) => {
                if send(writer, stream, KIND_CHAT_ENTRY, e.encode_to_vec()).is_err() {
                    return;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
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
    pub fn connect(path: &Path) -> io::Result<Self> {
        let stream = local_connect(path)?;
        Ok(Self {
            stream,
            decoder: Decoder::new(DEFAULT_MAX_PAYLOAD),
            next_request: 1,
            pending: VecDeque::new(),
        })
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
