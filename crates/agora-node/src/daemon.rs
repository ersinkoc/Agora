//! The node daemon: supervisor + control socket + event buffer, running without any hub.
//!
//! Events go to the durable buffer (`event_buffer`) as protocol `Event`s; with no hub connected they are
//! simply retained (size-capped) until one acknowledges them. Agents never depend on any of this succeeding.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use agora_adapters::host::AdapterEvent;
use agora_core::{AgentId, AgentState, NodeId};
use agora_protocol::v1::{self, event::Body};
use prost::Message;

use crate::control::{ControlContext, ControlServer, ProjectEntry};
use crate::event_buffer::DiskEventLog;
use crate::git;
use crate::supervisor::{
    AgentView, NodeLimits, RestartPolicy, Supervisor, SupervisorConfig, SupervisorEvent,
};
use crate::terminal::TerminalOptions;

const TICK: Duration = Duration::from_millis(100);
/// How often each agent's workspace is checked for uncommitted changes.
const GIT_POLL: Duration = Duration::from_secs(5);
/// In the event buffer's folder: the producer (node id) whose events the log holds.
const PRODUCER_FILE: &str = "producer";
/// Default size cap of the on-disk event buffer.
pub const EVENT_BUFFER_BYTES: u64 = 64 * 1024 * 1024;

/// How an announcement begins (ADR-0055).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Announce {
    /// With `NodeOnline`: the hub will never see this process's start, so everything it knew of the node before is reset and only
    /// what runs now is said again (ADR-0054 section 3).
    Restart,
    /// Without `NodeOnline`: every agent of this process is stated once, running or ended, with the approvals that wait and those
    /// resolved recently (ADR-0055 section 2).
    Present,
}

/// What a node says about itself again when the hub's record of it cannot be continued (see [`EventSink::announce`] and
/// [`EventSink::resync`]). The announcer records `NodeOnline` too, for [`Announce::Restart`] only.
type Announcer = Box<dyn Fn(&EventSink, Announce) + Send>;

/// The node's announcer (see [`EventSink::announce`]): who it is (only when the hub lost this process's start), its checkouts, and
/// its present. The supervisor's and the ledger's locks are each taken alone, to copy what they hold, and released before anything
/// is recorded. The waiting approvals are not copied: they are recorded under the approval queue's lock
/// ([`NodeApprovals::announce_waiting`](crate::approvals::NodeApprovals::announce_waiting)), so a request decided while the
/// announcement runs is never stated again after its `ApprovalResolved` (#1150, review of #1216, N3). `before_record` runs once the
/// copies are taken and before the first record (a test's pause; nothing in the node).
fn node_announcer(
    node: String,
    sup: Arc<Mutex<Supervisor>>,
    approvals: Arc<crate::approvals::NodeApprovals>,
    ledger: Arc<crate::ledger::Ledger>,
    known_checkouts: Vec<(String, PathBuf)>,
    before_record: Box<dyn Fn() + Send>,
) -> Announcer {
    Box::new(move |ev, how| {
        let views = sup.lock().unwrap_or_else(|e| e.into_inner()).list();
        // ADR-0060: the agents still offered to resume, said again after the others.
        let interrupted: Vec<(crate::ledger::Entry, bool)> = {
            let found = ledger.interrupted();
            let s = sup.lock().unwrap_or_else(|e| e.into_inner());
            found
                .into_iter()
                .map(|e| {
                    let resumable = s.offers_resume(&e);
                    (e, resumable)
                })
                .collect()
        };
        before_record();
        if how == Announce::Restart {
            ev.node_online_with_runs(&node, views.iter());
        }
        for (id, root) in &known_checkouts {
            ev.checkout_discovered(&node, id, root);
        }
        ev.announce_present(&views, &[], how);
        approvals.announce_waiting(|p| ev.approval_requested(p));
        for (e, resumable) in &interrupted {
            ev.agent_interrupted(e, *resumable);
        }
    })
}

/// How many resolved approvals a [`Announce::Present`] announcement says again (ADR-0055).
pub const RESOLVED_KEPT: usize = 256;

/// Facts the node reported that its supervisor does not keep, remembered from the events themselves so a resync announcement
/// (ADR-0054, ADR-0055) can say them again: the folders it registered, which agent works in which, each agent's session, terminal
/// and last exit code, and the last [`RESOLVED_KEPT`] resolved approvals.
#[derive(Default, Clone)]
struct Reported {
    workspaces: std::collections::BTreeMap<String, v1::WorkspaceRegistered>,
    bound: std::collections::BTreeMap<String, String>,
    sessions: std::collections::BTreeMap<String, String>,
    exits: std::collections::BTreeMap<String, i32>,
    terminals: std::collections::BTreeSet<String>,
    resolved: std::collections::VecDeque<v1::ApprovalResolved>,
}

impl Reported {
    fn keeps(body: &Body) -> bool {
        matches!(
            body,
            Body::WorkspaceRegistered(_)
                | Body::WorkspaceStatusChanged(_)
                | Body::AgentWorkspaceBound(_)
                | Body::AgentSessionReported(_)
                | Body::AgentExited(_)
                | Body::TerminalOpened(_)
                | Body::ApprovalResolved(_)
        )
    }

    fn note(&mut self, body: &Body) {
        match body {
            Body::WorkspaceRegistered(w) => {
                self.workspaces.insert(w.workspace_id.clone(), w.clone());
            }
            Body::WorkspaceStatusChanged(c) => {
                if let Some(w) = self.workspaces.get_mut(&c.workspace_id) {
                    w.files_changed = c.files_changed;
                }
            }
            Body::AgentWorkspaceBound(b) => {
                self.bound
                    .insert(b.agent_id.clone(), b.workspace_id.clone());
            }
            Body::AgentSessionReported(r) => {
                self.sessions
                    .insert(r.agent_id.clone(), r.session_id.clone());
            }
            Body::AgentExited(x) => {
                self.exits.insert(x.agent_id.clone(), x.exit_code);
            }
            Body::TerminalOpened(t) if !t.agent_id.is_empty() => {
                self.terminals.insert(t.agent_id.clone());
            }
            // Said again by an announcement, it is already here: the ring keeps its order and never holds one twice.
            Body::ApprovalResolved(r)
                if !self.resolved.iter().any(|k| k.approval_id == r.approval_id) =>
            {
                if self.resolved.len() == RESOLVED_KEPT {
                    self.resolved.pop_front();
                }
                self.resolved.push_back(r.clone());
            }
            _ => {}
        }
    }
}

/// ADR-0119 decision 10: a helper's parent on another machine (the hub-written origin and that machine's agent), and the standing grant
/// it started under (empty when it was asked).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteParent {
    pub node: String,
    pub agent: String,
    pub grant_id: String,
}

/// Writes protocol events to the durable buffer.
pub struct EventSink {
    producer: String,
    log: Mutex<DiskEventLog>,
    /// Says again what the hub must know about this node, see [`EventSink::announce`].
    announcer: Mutex<Option<Announcer>>,
    /// See [`Reported`]. Never held together with `log`.
    reported: Mutex<Reported>,
    /// The `seq` of the last `NodeOnline` recorded, 0 for none (ADR-0055: a resync says `NodeOnline` only when the hub lost it).
    last_online: AtomicU64,
    /// See [`EventSink::set_aside`].
    set_aside: Option<SetAside>,
    /// ADR-0060: learns from every event recorded (see [`crate::ledger::Ledger::observe`]). Never held with another lock.
    ledger: std::sync::OnceLock<Arc<crate::ledger::Ledger>>,
    /// Usage reports gathered before they are logged ([`crate::usage_merge`]). Never held with `log`.
    usage: Mutex<crate::usage_merge::UsageMerge>,
    /// #1147 (audit of #1157): the node's secret values, replaced in the agent's own words an event carries (see `redact_body`).
    secrets: std::sync::OnceLock<Arc<crate::transcript::Store>>,
    /// F2041: the devices this node last said it trusts, said again by every announcement. Held while that is recorded, so an
    /// announcement never records an older set after a newer one; `log` is taken under it, never the other way round.
    trusted: Mutex<Option<v1::NodeTrustedDevices>>,
}

impl Drop for EventSink {
    /// When the sink is dropped in-process, the usage still being gathered is logged. A node that stops cleanly logs it before
    /// (`Node::drop`, which `agora node` runs on `SIGTERM`, ADR-0104 O2); one ended outright (a crash, `SIGKILL`, `TerminateProcess`)
    /// does not unwind, so up to [`crate::usage_merge::WINDOW_MS`] of each agent's usage is lost then (#522 review).
    fn drop(&mut self) {
        self.flush_all_usage();
    }
}

/// An event log found at start whose unacknowledged events name another producer (for example `local`, from runs before the node
/// was enrolled). It was renamed, never deleted, and is not sent to the hub.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetAside {
    /// The producer its events name.
    pub old_id: String,
    /// Where the old log is now.
    pub path: PathBuf,
}

/// ADR-0058: an adapter's currency as an upper-case three-letter code, or `None` for anything else.
fn currency_code(c: &str) -> Option<String> {
    (c.len() == 3 && c.bytes().all(|b| b.is_ascii_alphabetic())).then(|| c.to_ascii_uppercase())
}

/// ADR-0069: this node's executable, only when the folder it is in is not on its PATH (so a client can say to run `agora` from there);
/// empty otherwise, or when it cannot be told.
fn binary_off_path() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|exe| {
            off_path(
                &exe,
                std::env::var_os("PATH").as_deref(),
                std::env::current_dir().ok().as_deref(),
            )
        })
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// `exe` when its folder is none of the folders in `path_var`, compared as the file system resolves them (links, `.`, case on
/// Windows); `None` when it is on PATH or has no folder. An empty entry is the current folder `cwd`, as a shell reads it
/// (`PATH=:/usr/bin`).
fn off_path(exe: &Path, path_var: Option<&std::ffi::OsStr>, cwd: Option<&Path>) -> Option<PathBuf> {
    let dir = exe.parent()?;
    let resolve =
        |p: &Path| agora_platform::canonicalize_plain(p).unwrap_or_else(|_| p.to_path_buf());
    let dir = resolve(dir);
    let on_path = path_var.is_some_and(|v| {
        std::env::split_paths(v).any(|d| {
            let d = if d.as_os_str().is_empty() {
                match cwd {
                    Some(c) => c.to_path_buf(),
                    None => return false,
                }
            } else {
                d
            };
            resolve(&d) == dir
        })
    });
    (!on_path).then(|| exe.to_path_buf())
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Stable id of a checkout on this node: a hash of its path (the same path always gives the same id).
pub fn checkout_id(root: &Path) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in root.to_string_lossy().bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("co-{h:016x}")
}

/// The kind of a chat agent's item in the fixed words other agents see (`agora_mesh::awareness`); `None` for an item that says
/// nothing about what it is doing (a notice).
fn activity_word(item: &agora_adapters::host::ChatItem) -> Option<&'static str> {
    use agora_adapters::host::ChatItem;
    match item {
        ChatItem::Text(_) => Some("writing a reply"),
        ChatItem::ToolCall { tool, .. } => Some(agora_mesh::awareness::tool_word(tool)),
        ChatItem::Plan(_) => Some("planning"),
        ChatItem::Diff { .. } => Some("editing files"),
        ChatItem::Notice { .. } => None,
        // ADR-0090: thinking is never routed to the mesh, not even as a word; a tool call with an id says what kind of tool it is.
        ChatItem::Thought(_) => None,
        ChatItem::Tool(t) => t.tool.as_deref().map(agora_mesh::awareness::tool_word),
        ChatItem::PlanSteps(_) => Some("planning"),
    }
}

/// ADR-0090: a config option as the protocol carries it. The category word is the agent's: one this build does not know is
/// `UNSPECIFIED` (a name starting with `_` is the agent's own, `CUSTOM`).
fn config_option(o: &agora_adapters::host::ConfigOption) -> v1::SessionConfigOption {
    use v1::SessionConfigCategory as C;
    v1::SessionConfigOption {
        id: o.id.clone(),
        name: o.name.clone(),
        description: o.description.clone(),
        category: match o.category.as_str() {
            "mode" => C::Mode,
            "model" => C::Model,
            "model_config" => C::ModelConfig,
            "thought_level" => C::ThoughtLevel,
            c if c.starts_with('_') => C::Custom,
            _ => C::Unspecified,
        } as i32,
        r#type: if o.boolean {
            v1::SessionConfigType::Boolean
        } else {
            v1::SessionConfigType::Select
        } as i32,
        current: o.current.clone(),
        choices: o
            .choices
            .iter()
            .map(|c| v1::SessionOption {
                id: c.id.clone(),
                name: c.name.clone(),
                description: c.description.clone(),
                skips_permissions: c.skips_permissions,
            })
            .collect(),
        skips_permissions: o.skips_permissions,
    }
}

/// How a process's end is reported (`AgentExited.exit_code`): its exit code, or on Unix the signal that ended it as a negative number;
/// -1 when neither is known.
fn exit_code(status: &agora_platform::ExitStatus) -> i32 {
    status
        .code
        .or_else(|| status.signal.map(|s| -s))
        .unwrap_or(-1)
}

/// A chat agent (an adapter's or an ACP agent's, ADR-0038), not one in a terminal. By its level: `AgentView::terminal` says only whether
/// a terminal is open now, which is false for a terminal agent that just ended too.
fn is_chat(views: &[AgentView], agent: &AgentId) -> bool {
    views.iter().any(|v| {
        v.id == *agent
            && matches!(
                v.level,
                agora_core::IntegrationLevel::L2Acp | agora_core::IntegrationLevel::L3Adapter
            )
    })
}

/// #902 (RM-13): a chat agent whose adapter ended on its own: its conversation says that the node starts it again (`adapter.restarting`,
/// `attempt`), or that it gave up on it (`adapter.gave_up`, with `restarts`, and `exit_code` only when the node knows how the last
/// process ended). Nothing for a terminal agent (its screen says it) or for any other event.
fn restart_notice(
    chat: &crate::chat::ChatBoard,
    e: &SupervisorEvent,
    views: &[AgentView],
    now: u64,
) {
    match e {
        SupervisorEvent::Restarting { agent, attempt, .. } if is_chat(views, agent) => {
            chat.restart_notice(
                agent,
                "adapter.restarting",
                &[("attempt", attempt.to_string())],
                now,
            );
        }
        SupervisorEvent::GaveUp { agent, exit_code } if is_chat(views, agent) => {
            let restarts = views
                .iter()
                .find(|v| v.id == *agent)
                .map_or(0, |v| v.restarts);
            let mut params = vec![("restarts", restarts.to_string())];
            params.extend(exit_code.map(|c| ("exit_code", c.to_string())));
            chat.restart_notice(agent, "adapter.gave_up", &params, now);
        }
        _ => {}
    }
}

fn state_word(s: AgentState) -> &'static str {
    match s {
        AgentState::Working => "working",
        AgentState::Blocked => "blocked",
        AgentState::Idle => "idle",
        AgentState::Done => "done",
        AgentState::Unknown => "unknown",
    }
}

fn proto_state(s: AgentState) -> v1::AgentState {
    match s {
        AgentState::Working => v1::AgentState::Working,
        AgentState::Blocked => v1::AgentState::Blocked,
        AgentState::Idle => v1::AgentState::Idle,
        AgentState::Done => v1::AgentState::Done,
        AgentState::Unknown => v1::AgentState::Unspecified,
    }
}

/// ADR-0123: `(id, value)` launch settings as `AgentStarted` carries them.
fn settings_of(settings: &[(String, String)]) -> Vec<v1::LaunchSetting> {
    settings
        .iter()
        .map(|(id, value)| v1::LaunchSetting {
            id: id.clone(),
            value: value.clone(),
        })
        .collect()
}

impl EventSink {
    pub fn open(dir: &Path, producer: &str) -> io::Result<Self> {
        Self::open_with_cap(dir, producer, EVENT_BUFFER_BYTES)
    }

    /// Like [`open`](Self::open) with an explicit size cap for the buffer, in bytes.
    ///
    /// One log holds the events of one producer. When events not yet acknowledged name another producer (the node ran as
    /// `local` before it was enrolled, or under another name), they must never be replayed: the hub refuses events that name
    /// another node than the certificate. The old log is set aside (renamed, never deleted, see [`DiskEventLog::set_aside`]) and
    /// a fresh one starts; [`set_aside`](Self::set_aside) says so.
    pub fn open_with_cap(dir: &Path, producer: &str, max_bytes: u64) -> io::Result<Self> {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        // Shutdown safety: a log that cannot be read is set aside, so the node still starts (operational log, not UI text).
        let (mut log, unreadable) = DiskEventLog::open_or_set_aside(dir, max_bytes, secs)?;
        if let Some(path) = unreadable {
            crate::log_line!(
                "events: the event log could not be read; set aside as {} and started again",
                path.display()
            );
        }
        // Whose log this is: the `producer` file says so (its events may all be acknowledged and compacted away, but the numbering
        // still belongs to that name); a log from before the file existed is judged by any event it still holds.
        let recorded = fs::read_to_string(dir.join(PRODUCER_FILE))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s != producer);
        let foreign = match recorded {
            Some(old) => Some(old),
            None => log
                .find_retained(|p| v1::Event::decode(p).is_ok_and(|e| e.producer_id != producer))?
                .and_then(|p| v1::Event::decode(p.as_slice()).ok())
                .map(|e| e.producer_id),
        };
        let mut set_aside = None;
        if let Some(old_id) = foreign.filter(|_| !log.is_pristine()) {
            let path = log.set_aside(&old_id, secs)?;
            log = DiskEventLog::open(dir, max_bytes)?;
            set_aside = Some(SetAside { old_id, path });
        }
        // Replaced whole and synced: a power loss never leaves it empty.
        agora_platform::write_private(&dir.join(PRODUCER_FILE), producer.as_bytes())?;
        Ok(Self {
            producer: producer.to_string(),
            log: Mutex::new(log),
            announcer: Mutex::new(None),
            reported: Mutex::new(Reported::default()),
            last_online: AtomicU64::new(0),
            set_aside,
            ledger: std::sync::OnceLock::new(),
            usage: Mutex::new(crate::usage_merge::UsageMerge::default()),
            secrets: std::sync::OnceLock::new(),
            trusted: Mutex::new(None),
        })
    }

    /// Shutdown safety: makes the events recorded so far last through a power loss (each append only reaches the system). The node calls
    /// it when it stops, as an OS shutdown or reboot may follow.
    pub fn sync(&self) -> io::Result<()> {
        self.log.lock().unwrap_or_else(|e| e.into_inner()).sync()
    }

    /// #1147 (audit of #1157): the store whose secret values are replaced in the agent's words an event carries. Set once.
    pub fn set_secrets(&self, store: Arc<crate::transcript::Store>) {
        let _ = self.secrets.set(store);
    }

    /// #1147 (audit of #1157): the text an agent chose in `body`, with the node's secret values replaced (whole, and pieces of 4 bytes
    /// or more at the text's ends), so the hub never records them: a mesh message's body, a task's title, a handoff's summary, and the
    /// names, descriptions and hints of the agent's modes, options and commands. Ids are kept (they are answered back). An approval's
    /// action and detail are replaced before the request is queued (the always signature binds what the card shows), not here.
    fn redact_body(&self, body: Body) -> Body {
        let Some(store) = self.secrets.get() else {
            return body;
        };
        let sec = store.current();
        if sec.is_empty() {
            return body;
        }
        let r = |s: &mut String| *s = crate::transcript::redact_text(s, &sec);
        let mut body = body;
        match &mut body {
            Body::MessagePosted(m) => r(&mut m.body),
            Body::TaskChanged(t) => r(&mut t.title),
            Body::HandoffCreated(h) => {
                r(&mut h.summary);
                h.refs.iter_mut().for_each(r);
            }
            Body::AgentSessionOptions(o) => {
                for x in o.modes.iter_mut().chain(o.models.iter_mut()) {
                    r(&mut x.name);
                    r(&mut x.description);
                }
            }
            Body::AgentSessionConfig(c) => {
                for o in &mut c.options {
                    r(&mut o.name);
                    r(&mut o.description);
                    for x in &mut o.choices {
                        r(&mut x.name);
                        r(&mut x.description);
                    }
                }
            }
            Body::AgentCommands(c) => {
                for x in &mut c.commands {
                    r(&mut x.description);
                    r(&mut x.input_hint);
                }
            }
            _ => {}
        }
        body
    }

    /// ADR-0060: the session ledger that learns from what this sink records. Set once; a second call is ignored.
    pub fn set_ledger(&self, ledger: Arc<crate::ledger::Ledger>) {
        let _ = self.ledger.set(ledger);
    }

    /// The log this sink found at open and set aside because its events name another producer, if it did.
    pub fn set_aside(&self) -> Option<&SetAside> {
        self.set_aside.as_ref()
    }

    /// What this node says about itself when the hub's record of it cannot be continued (see [`announce`](Self::announce)).
    pub fn set_announcer(&self, f: Announcer) {
        *self.announcer.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
    }

    /// Says again, at the tail of the buffer, what the hub must know about this node now, led by `NodeOnline`: who it is, its
    /// checkouts and folders, the agents running with their state, session and terminal, and the approvals waiting (ADR-0054
    /// section 3, [`Announce::Restart`]). Used when the hub cannot have what was recorded before: an empty data directory under a
    /// known name, or (see [`resync`](Self::resync)) a start of this process the hub lost. Without an announcer (a bare sink in
    /// tests) it says nothing.
    ///
    /// Lock order: the announcer runs under the `announcer` lock and takes the supervisor's lock alone to read it, then `reported`
    /// and `log` one at a time to write. The waiting approvals are recorded under the approval queue's lock (announcer, queue, then
    /// the sink's own locks: the order is spawner, routes, queue, event sink, and nothing in this sink takes an approvals lock), so
    /// a request decided meanwhile is never restated after its `ApprovalResolved`. Nothing that holds the supervisor's or an
    /// approvals lock ever waits for the `announcer` lock, and `log` is never held while taking another lock.
    pub fn announce(&self) {
        self.announce_as(Announce::Restart);
    }

    fn announce_as(&self, how: Announce) {
        let announcer = self.announcer.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(f) = &*announcer {
            f(self, how);
        }
    }

    /// The hub cannot have `from_seq..oldest_retained` (ADR-0054): say what is true now at the tail (ADR-0055). The announcement
    /// begins with `NodeOnline` only when this process's start is in that lost range; otherwise it states facts and leaves what the
    /// hub knows of running agents (and their work on a shared board) in place. If the announcement itself pushes the start out of
    /// the buffer under the size cap, a full one led by `NodeOnline` follows.
    pub fn resync(&self, from_seq: u64, oldest_retained: u64) {
        let lost = |oldest: u64| {
            let start = self.last_online.load(Ordering::Acquire);
            start != 0 && start >= from_seq && start < oldest
        };
        if lost(oldest_retained) {
            self.announce_as(Announce::Restart);
            return;
        }
        self.announce_as(Announce::Present);
        if lost(self.oldest_retained()) {
            self.announce_as(Announce::Restart);
        }
    }

    /// The `seq` of the last `NodeOnline` this sink recorded, 0 for none.
    pub fn last_online(&self) -> u64 {
        self.last_online.load(Ordering::Acquire)
    }

    /// The hub has acknowledged numbers this buffer never produced (this machine came back under a known name with an empty data
    /// directory): skip our numbering forward to `next` and say again who we are, since what we recorded before the skip is gone.
    pub fn catch_up_with_hub(&self, next: u64) -> io::Result<()> {
        self.log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .advance_to(next)?;
        self.announce();
        Ok(())
    }

    /// The present part of an announcement (see [`announce`](Self::announce) and [`resync`](Self::resync)): the folders this node
    /// registered, the devices it last said it trusts (F2041), then every agent in `views` that is running, in the order a launch
    /// reports it, then `waiting` approvals. For [`Announce::Present`] also every agent in `views` that ended (`AgentExited` with the last exit code recorded for it, or the
    /// stop convention `i32::MIN`, and `TerminalClosed` if it had a terminal) and the last [`RESOLVED_KEPT`] resolved approvals.
    /// The callers' locks are already released: `views` and `waiting` are copies. The node passes no `waiting` and records its
    /// waiting approvals after this, under the queue's lock (see `node_announcer`): a copy can hold a request decided since.
    pub fn announce_present(
        &self,
        views: &[AgentView],
        waiting: &[crate::approvals::Pending],
        how: Announce,
    ) {
        let known = self
            .reported
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        for w in known.workspaces.values() {
            self.restate(Body::WorkspaceRegistered(w.clone()));
        }
        // F2041: the devices this machine trusts. The hub keeps the last set it got (a `NodeOnline` does not clear it), and the node says
        // it again only when it changes: one lost with the rest would stay wrong in every window until then.
        {
            let last = self.trusted.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(t) = &*last {
                self.restate(Body::NodeTrustedDevices(t.clone()));
            }
        }
        for v in views {
            let id = v.id.as_str();
            if !v.running {
                if how == Announce::Present {
                    self.restate(Body::AgentExited(v1::AgentExited {
                        agent_id: id.to_string(),
                        exit_code: known.exits.get(id).copied().unwrap_or(i32::MIN),
                    }));
                    if known.terminals.contains(id) {
                        self.restate(Body::TerminalClosed(v1::TerminalClosed {
                            terminal_id: crate::control::terminal_id_of(&v.id),
                        }));
                    }
                }
                continue;
            }
            if v.directory {
                if let Some(ws) = known.bound.get(id) {
                    self.restate(Body::AgentWorkspaceBound(v1::AgentWorkspaceBound {
                        agent_id: id.to_string(),
                        workspace_id: ws.clone(),
                    }));
                }
            }
            // ADR-0104: this node announced the process that runs now under this id before (its start, a resume or a take-up), so the
            // hub keeps the counters it summed for it.
            self.agent_up(v, known.sessions.get(id), false, true);
            // `AgentStarted` leaves the hub with no session options: what an adapter agent last reported of its session follows it
            // (#932: a kept agent's adapter does not report it again by itself).
            if let Some(ledger) = self.ledger.get() {
                for b in ledger.kept_session(id).bodies() {
                    self.restate(b);
                }
            }
        }
        if how == Announce::Present {
            for r in known.resolved {
                self.restate(Body::ApprovalResolved(r));
            }
        }
        for p in waiting {
            self.approval_requested(p);
        }
    }

    /// ADR-0079/0093: an agent the holder kept running, taken up by this node at its start. The node's `NodeOnline` told the hub that
    /// everything it ran before is gone, so the agent is said up again after it (started, its state, its session if known, its terminal
    /// if it has one), as an announcement says it. Re-stated: the ledger already holds the agent and took it back (`Ledger::reclaim`).
    /// What an adapter agent last reported of its session follows from the supervisor (`Supervisor::session_events`, #932).
    /// ADR-0104: its `AgentStarted` says `same_process`, so the hub keeps the counters it summed for it before the node restarted.
    pub fn agent_taken_up(&self, view: &AgentView, session: Option<&String>) {
        self.agent_up(view, session, false, true);
    }

    /// An agent in `view` is up: started, its state, its session if known, and its terminal if it has one. `learn` false: an
    /// announcement re-stating it from a copy of the supervisor's list (see [`restate`](Self::restate)). `same_process` (ADR-0104):
    /// this re-states a process this node announced before under this id (a take-up or an announcement); never for a new start, a
    /// resume or a restart after a crash, whose counters start again from 0.
    fn agent_up(
        &self,
        view: &AgentView,
        session: Option<&String>,
        learn: bool,
        same_process: bool,
    ) {
        let agent = view.id.as_str();
        let record = |body| self.record_as(body, learn);
        // ADR-0119 decision 10: a helper another machine asked for keeps naming it through a restart, a resume or a take-up (the
        // ledger kept it from its first start). The ledger's lock is taken alone.
        let (parent_node_id, parent_node_agent_id) = self
            .ledger
            .get()
            .and_then(|l| l.get(agent))
            .map(|e| (e.parent_node_id, e.parent_node_agent_id))
            .unwrap_or_default();
        record(Body::AgentStarted(v1::AgentStarted {
            launch_settings: settings_of(&view.launch_settings),
            run_id: view.run_id.map(Vec::from).unwrap_or_default(),
            agent_id: agent.into(),
            project_id: view.project.clone(),
            node_id: self.producer.clone(),
            kind: view.kind.clone(),
            worktree_id: if view.directory {
                String::new()
            } else {
                view.worktree.to_string_lossy().into_owned()
            },
            level: match view.level {
                agora_core::IntegrationLevel::L0Generic => v1::IntegrationLevel::L0Generic,
                agora_core::IntegrationLevel::L1Manifest => v1::IntegrationLevel::L1Manifest,
                agora_core::IntegrationLevel::L2Acp => v1::IntegrationLevel::L2Acp,
                agora_core::IntegrationLevel::L3Adapter => v1::IntegrationLevel::L3Adapter,
            } as i32,
            parent_agent_id: view
                .parent
                .as_ref()
                .map(|p| p.as_str().to_string())
                .unwrap_or_default(),
            same_process,
            launch_options: view.launch_options.clone(),
            skips_permissions: view.skips_permissions,
            grant_id: String::new(),
            parent_node_id,
            parent_node_agent_id,
        }));
        record(Body::AgentStateChanged(v1::AgentStateChanged {
            agent_id: agent.into(),
            state: proto_state(view.state) as i32,
            reported: view.reported,
        }));
        if let Some(session) = session {
            record(Body::AgentSessionReported(v1::AgentSessionReported {
                agent_id: agent.into(),
                session_id: session.clone(),
            }));
        }
        // ADR-0094: the hub cleared what runs inside a plain terminal on the `AgentStarted` above; said again while it runs.
        if let Some(kind) = &view.runs_inside {
            record(Body::AgentRunsInside(v1::AgentRunsInside {
                agent_id: agent.into(),
                kind: kind.clone(),
            }));
        }
        if view.terminal {
            record(Body::TerminalOpened(v1::TerminalOpened {
                terminal_id: crate::control::terminal_id_of(&view.id),
                node_id: self.producer.clone(),
                purpose: v1::TerminalPurpose::Agent as i32,
                agent_id: agent.into(),
            }));
        }
    }

    fn record(&self, body: Body) {
        self.record_as(body, true);
    }

    /// F870: an announcement re-states what this node recorded before, from a copy of the supervisor's list taken before it records
    /// (the supervision tick records meanwhile). It goes to the log for the hub, but the session ledger, which learned every fact it
    /// re-states when it happened, does not learn from it: a stale copy would end a restarted agent there, or bring back a finished one.
    fn restate(&self, body: Body) {
        self.record_as(body, false);
    }

    fn record_as(&self, body: Body, learn: bool) {
        let body = self.redact_body(body);
        let ended = match &body {
            // Gathered for a few seconds and logged as one (see `crate::usage_merge`).
            Body::AgentUsageReported(u) => {
                let back = self
                    .usage
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .add(now_ms(), u.clone());
                if let Some(u) = back {
                    self.write(Body::AgentUsageReported(u));
                }
                return;
            }
            Body::AgentExited(e) => Some(e.agent_id.clone()),
            Body::AgentInterrupted(e) => Some(e.agent_id.clone()),
            _ => None,
        };
        // An agent that ended: what it used is logged before its end.
        if let Some(agent) = ended {
            let gathered = self
                .usage
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take_agent(&agent);
            for u in gathered {
                self.write(Body::AgentUsageReported(u));
            }
        }
        self.write_as(body, learn);
    }

    /// Logs the usage reports whose gathering window has passed. Called on every supervision tick.
    pub fn flush_usage(&self) {
        let due = self
            .usage
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .due(now_ms());
        for u in due {
            self.write(Body::AgentUsageReported(u));
        }
    }

    /// Logs all the usage still being gathered, due or not (a node that stops, ADR-0104 O2).
    pub fn flush_all_usage(&self) {
        let rest = self
            .usage
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take_all();
        for u in rest {
            self.write(Body::AgentUsageReported(u));
        }
    }

    fn write(&self, body: Body) {
        self.write_as(body, true);
    }

    fn write_as(&self, body: Body, learn: bool) {
        if let Some(ledger) = self.ledger.get().filter(|_| learn) {
            ledger.observe(&body, now_ms());
        }
        // Only the few kinds a resync announcement repeats take this lock; every other event goes straight to the log.
        if Reported::keeps(&body) {
            self.reported
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .note(&body);
        }
        let online = matches!(body, Body::NodeOnline(_));
        let mut log = self.log.lock().unwrap_or_else(|e| e.into_inner());
        let ev = v1::Event {
            producer_id: self.producer.clone(),
            seq: log.next_seq(),
            at_ms: now_ms(),
            body: Some(body),
        };
        // A full disk must never stop agents: losing an event is better than losing the node.
        if let (Ok(appended), true) = (log.append(&ev.encode_to_vec()), online) {
            self.last_online.store(appended.seq, Ordering::Release);
        }
    }

    pub fn agent_started(
        &self,
        agent: &AgentId,
        project: &str,
        node: &str,
        worktree: &str,
        kind: &str,
    ) {
        self.agent_started_by(agent, project, node, worktree, kind, "");
    }

    /// [`Self::agent_started`] for a helper another agent started (ADR-0057); `parent` is empty when the user started it.
    pub fn agent_started_by(
        &self,
        agent: &AgentId,
        project: &str,
        node: &str,
        worktree: &str,
        kind: &str,
        parent: &str,
    ) {
        let none = crate::launch_options::LaunchChoice::default();
        self.agent_started_with(
            agent, project, node, worktree, kind, parent, &none, None, None,
        );
    }

    /// [`Self::agent_started_by`] with the launch options the agent was started with (ADR-0059).
    #[allow(clippy::too_many_arguments)]
    fn agent_started_with(
        &self,
        agent: &AgentId,
        project: &str,
        node: &str,
        worktree: &str,
        kind: &str,
        parent: &str,
        launch: &crate::launch_options::LaunchChoice,
        run_id: Option<[u8; 16]>,
        remote: Option<&RemoteParent>,
    ) {
        self.record(Body::AgentStarted(v1::AgentStarted {
            launch_settings: settings_of(&launch.settings),
            run_id: run_id.map(Vec::from).unwrap_or_default(),
            agent_id: agent.as_str().to_string(),
            project_id: project.to_string(),
            node_id: node.to_string(),
            worktree_id: worktree.to_string(),
            kind: kind.to_string(),
            level: v1::IntegrationLevel::L1Manifest as i32,
            parent_agent_id: parent.to_string(),
            same_process: false,
            launch_options: launch.ids.clone(),
            skips_permissions: launch.skips_permissions,
            grant_id: remote.map(|r| r.grant_id.clone()).unwrap_or_default(),
            parent_node_id: remote.map(|r| r.node.clone()).unwrap_or_default(),
            parent_node_agent_id: remote.map(|r| r.agent.clone()).unwrap_or_default(),
        }));
    }

    /// Publish lifecycle and reported events using the supervisor's current process/session facts.
    pub fn supervisor_events(&self, events: &[SupervisorEvent], views: &[AgentView]) {
        for e in events {
            match e {
                SupervisorEvent::Started { agent } | SupervisorEvent::Resumed { agent, .. } => {
                    if let Some(view) = views.iter().find(|v| v.id == *agent) {
                        self.agent_up(view, None, true, false);
                    }
                }
                SupervisorEvent::SessionReported { agent, session } => {
                    self.record(Body::AgentSessionReported(v1::AgentSessionReported {
                        agent_id: agent.as_str().into(),
                        session_id: session.clone(),
                    }));
                }
                // ADR-0094: the kind of the program a person runs by hand in a plain terminal, or that it ended.
                SupervisorEvent::RunsInside { agent, kind } => {
                    self.record(Body::AgentRunsInside(v1::AgentRunsInside {
                        agent_id: agent.as_str().into(),
                        kind: kind.clone().unwrap_or_default(),
                    }));
                }
                SupervisorEvent::StateChanged {
                    agent,
                    state,
                    reported,
                } => {
                    self.record(Body::AgentStateChanged(v1::AgentStateChanged {
                        agent_id: agent.as_str().to_string(),
                        state: proto_state(*state) as i32,
                        reported: *reported,
                    }));
                }
                SupervisorEvent::Exited { agent, status } => {
                    self.record(Body::AgentExited(v1::AgentExited {
                        agent_id: agent.as_str().to_string(),
                        exit_code: exit_code(status),
                    }));
                    // #902: a chat agent the node gave up on never had a terminal.
                    if !is_chat(views, agent) {
                        self.record(Body::TerminalClosed(v1::TerminalClosed {
                            terminal_id: crate::control::terminal_id_of(agent),
                        }));
                    }
                }
                SupervisorEvent::Adapter { agent, event } => match event {
                    AdapterEvent::Usage {
                        model,
                        input_tokens,
                        output_tokens,
                        cost_micros,
                        currency,
                        context_tokens,
                        context_window,
                        ..
                    } => self.record(Body::AgentUsageReported(v1::AgentUsageReported {
                        agent_id: agent.as_str().to_string(),
                        model: model.clone(),
                        input_tokens: *input_tokens,
                        output_tokens: *output_tokens,
                        // ADR-0058: the agent's own cost, only with a currency the hub can tell apart (three letters).
                        cost_micros: cost_micros.filter(|_| currency_code(currency).is_some()),
                        currency: currency_code(currency).unwrap_or_default(),
                        context_tokens: *context_tokens,
                        context_window: *context_window,
                    })),
                    // ADR-0058: the supervisor passes these on checked and bounded, and only when they changed.
                    AdapterEvent::Options { options, .. } => {
                        let list = |l: &[agora_adapters::host::SessionOption]| {
                            l.iter()
                                .map(|o| v1::SessionOption {
                                    id: o.id.clone(),
                                    name: o.name.clone(),
                                    description: o.description.clone(),
                                    skips_permissions: o.skips_permissions,
                                })
                                .collect()
                        };
                        self.record(Body::AgentSessionOptions(v1::AgentSessionOptions {
                            agent_id: agent.as_str().to_string(),
                            modes: list(&options.modes),
                            models: list(&options.models),
                            current_mode: options.current_mode.clone(),
                            current_model: options.current_model.clone(),
                        }));
                    }
                    // ADR-0090: the config options and the commands, checked and bounded by the supervisor, only when they changed.
                    AdapterEvent::Config { options, .. } => {
                        self.record(Body::AgentSessionConfig(v1::AgentSessionConfig {
                            agent_id: agent.as_str().to_string(),
                            options: options.iter().map(config_option).collect(),
                        }));
                    }
                    AdapterEvent::Commands { commands, .. } => {
                        self.record(Body::AgentCommands(v1::AgentCommands {
                            agent_id: agent.as_str().to_string(),
                            commands: commands
                                .iter()
                                .map(|c| v1::AgentCommand {
                                    name: c.name.clone(),
                                    description: c.description.clone(),
                                    input_hint: c.input_hint.clone(),
                                })
                                .collect(),
                        }));
                    }
                    AdapterEvent::SessionReported {
                        native_session_id, ..
                    } => self.record(Body::AgentSessionReported(v1::AgentSessionReported {
                        agent_id: agent.as_str().to_string(),
                        session_id: native_session_id.clone(),
                    })),
                    // Approval requests go through the node's approvals queue (see `Node`), chat and log lines
                    // are not part of the event log.
                    _ => {}
                },
                SupervisorEvent::Stopped { agent } => {
                    // Stopped on request: the exit code is not known (same convention as the terminal exit frame).
                    self.record(Body::AgentExited(v1::AgentExited {
                        agent_id: agent.as_str().to_string(),
                        exit_code: crate::ledger::STOPPED_ON_REQUEST,
                    }));
                    self.record(Body::TerminalClosed(v1::TerminalClosed {
                        terminal_id: crate::control::terminal_id_of(agent),
                    }));
                }
                _ => {}
            }
        }
    }

    pub fn approval_requested(&self, p: &crate::approvals::Pending) {
        self.record(Body::ApprovalRequested(v1::ApprovalRequested {
            approval_id: p.id.clone(),
            agent_id: p.agent_id.clone(),
            action: p.action.clone(),
            detail: p.detail.clone(),
            expires_at_ms: p.expires_ms,
            scopes: p.offer.scopes(),
            always_label: p
                .offer
                .always
                .as_ref()
                .map(|a| a.label.clone())
                .unwrap_or_default(),
            always_option_id: p
                .offer
                .always
                .as_ref()
                .map(|a| a.option_id.clone())
                .unwrap_or_default(),
        }));
    }

    pub fn approval_resolved(&self, approval_id: &str, approved: bool, signer_device_id: &str) {
        self.start_resolved(approval_id, approved, signer_device_id, 0, false);
    }

    /// ADR-0101: a person's decision on an adapter's request was delivered "always": the audit record says so, and, when `holder` is
    /// the agent whose current process was given it, the agent's `AgentAlwaysAllowed` follows (the hub derives the "Asks less" chip
    /// from it; the session ledger keeps it for a kept agent). `None`: the process it was delivered to was replaced meanwhile (Codex
    /// review of PR #1007), so nothing is said about the agent's current process.
    pub fn approval_resolved_always(
        &self,
        holder: Option<&AgentId>,
        approval_id: &str,
        action: &str,
        signer_device_id: &str,
    ) {
        self.record(Body::ApprovalResolved(v1::ApprovalResolved {
            approval_id: approval_id.to_string(),
            approved: true,
            signer_device_id: signer_device_id.to_string(),
            scope: v1::ApprovalScope::Always as i32,
            ..Default::default()
        }));
        let Some(agent) = holder else {
            return;
        };
        self.record(Body::AgentAlwaysAllowed(v1::AgentAlwaysAllowed {
            agent_id: agent.as_str().to_string(),
            approval_id: approval_id.to_string(),
            action: crate::approvals::card_text(action, crate::approvals::MAX_ALWAYS_LABEL_CHARS),
            signer_device_id: signer_device_id.to_string(),
            at_ms: now_ms(),
        }));
    }

    /// The decision on an `agent.start` request (ADR-0057), with the grant that was signed.
    pub fn start_resolved(
        &self,
        approval_id: &str,
        approved: bool,
        signer_device_id: &str,
        helper_grant: u32,
        helpers_may_start: bool,
    ) {
        self.record(Body::ApprovalResolved(v1::ApprovalResolved {
            approval_id: approval_id.to_string(),
            approved,
            signer_device_id: signer_device_id.to_string(),
            helper_grant,
            helpers_may_start,
            // ADR-0101: a person's decision is a single answer here; a request nobody decided has no scope.
            scope: if signer_device_id.is_empty() {
                v1::ApprovalScope::Unspecified as i32
            } else {
                v1::ApprovalScope::Once as i32
            },
        }));
    }

    /// The agent's own session id (ADR-0060: chosen by the node at its first launch, or read from its session file).
    pub fn session_reported(&self, agent: &AgentId, session: &str) {
        self.record(Body::AgentSessionReported(v1::AgentSessionReported {
            agent_id: agent.as_str().to_string(),
            session_id: session.to_string(),
        }));
    }

    /// ADR-0060: an agent the ledger holds as interrupted (it ran when the node last stopped). `resumable`: the node has its own session
    /// id and its kind can continue one.
    pub fn agent_interrupted(&self, e: &crate::ledger::Entry, resumable: bool) {
        self.record(Body::AgentInterrupted(e.interrupted_event(resumable)));
    }

    /// The node refused an agent's `agent_start` before asking anyone (ADR-0057).
    pub fn agent_start_refused(&self, agent: &AgentId, kind: &str, folder: &str, code: &str) {
        self.record(Body::AgentStartRefused(v1::AgentStartRefused {
            agent_id: agent.as_str().to_string(),
            kind: kind.to_string(),
            folder: folder.to_string(),
            code: code.to_string(),
        }));
    }

    /// Appends already-built events (mesh activity for the audit log).
    pub fn record_bodies(&self, bodies: Vec<Body>) {
        for b in bodies {
            self.record(b);
        }
    }

    /// The node announces itself; the hub marks it online.
    pub fn node_online(&self, node: &str) {
        self.node_online_with_runs(node, std::iter::empty());
    }

    fn node_online_with_runs<'a>(&self, node: &str, views: impl Iterator<Item = &'a AgentView>) {
        let retained_agent_runs = views
            .filter(|v| {
                v.running && v.id.as_str().len() <= agora_protocol::MAX_RETAINED_AGENT_ID_BYTES
            })
            .filter_map(|v| {
                Some(v1::RetainedAgentRun {
                    agent_id: v.id.as_str().to_string(),
                    run_id: v.run_id?.to_vec(),
                })
            })
            .take(agora_protocol::MAX_RETAINED_AGENT_RUNS)
            .collect();
        let host = std::env::var("HOSTNAME")
            .or_else(|_| std::env::var("COMPUTERNAME"))
            .unwrap_or_else(|_| node.to_string());
        self.record(Body::NodeOnline(v1::NodeOnline {
            node_id: node.to_string(),
            hostname: host,
            os: std::env::consts::OS.to_string(),
            build: env!("CARGO_PKG_VERSION").to_string(),
            binary: binary_off_path(),
            retained_agent_runs,
        }));
    }

    /// ADR-0061: the devices this node trusts now (ids only, sorted).
    pub fn node_trusted_devices(&self, device_ids: Vec<String>) {
        let mut last = self.trusted.lock().unwrap_or_else(|e| e.into_inner());
        let said = v1::NodeTrustedDevices { device_ids };
        *last = Some(said.clone());
        self.record(Body::NodeTrustedDevices(said));
    }

    /// ADR-0088: a preview forward opened or ended (the port and who opened it, never what went through it).
    pub fn preview_changed(&self, forward: v1::PreviewForward) {
        self.record(Body::PreviewChanged(v1::PreviewChanged {
            forward: Some(forward),
        }));
    }

    /// Recorded on a clean shutdown. A crash records nothing; the hub notices the dropped connection instead.
    pub fn node_offline(&self, node: &str) {
        self.record(Body::NodeOffline(v1::NodeOffline {
            node_id: node.to_string(),
        }));
    }

    pub fn node_resources(
        &self,
        node: &str,
        agents_running: u32,
        terminals_open: u32,
        memory_used_bytes: u64,
    ) {
        self.record(Body::NodeResources(v1::NodeResources {
            node_id: node.to_string(),
            agents_running,
            terminals_open,
            memory_used_bytes,
        }));
    }

    pub fn git_status_changed(&self, worktree_id: &str, st: &git::GitStatus) {
        self.record(Body::GitStatusChanged(v1::GitStatusChanged {
            worktree_id: worktree_id.to_string(),
            files_changed: st.files_changed,
            insertions: st.insertions,
            deletions: st.deletions,
            head_commit: st.head_commit.clone(),
        }));
    }

    pub fn checkout_discovered(&self, node: &str, project: &str, root: &Path) {
        self.record(Body::CheckoutDiscovered(v1::CheckoutDiscovered {
            checkout_id: checkout_id(root),
            node_id: node.to_string(),
            project_id: project.to_string(),
            path: root.to_string_lossy().into_owned(),
        }));
    }

    /// An agent is up: its workspace, the agent, and its terminal, in that order so a reader that builds a
    /// view from the events always sees the workspace before the agent that uses it.
    #[allow(clippy::too_many_arguments)]
    pub fn agent_launched(
        &self,
        agent: &AgentId,
        project: &str,
        node: &str,
        checkout: &Path,
        worktree: &Path,
        branch: &str,
        kind: &str,
        has_terminal: bool,
    ) {
        let none = crate::launch_options::LaunchChoice::default();
        self.agent_launched_with(
            agent,
            project,
            node,
            checkout,
            worktree,
            branch,
            kind,
            has_terminal,
            &none,
            None,
        );
    }

    /// [`Self::agent_launched`] with the launch options the agent was started with (ADR-0059).
    #[allow(clippy::too_many_arguments)]
    pub fn agent_launched_with(
        &self,
        agent: &AgentId,
        project: &str,
        node: &str,
        checkout: &Path,
        worktree: &Path,
        branch: &str,
        kind: &str,
        has_terminal: bool,
        launch: &crate::launch_options::LaunchChoice,
        run_id: Option<[u8; 16]>,
    ) {
        let worktree_id = worktree.to_string_lossy().into_owned();
        self.record(Body::WorktreeCreated(v1::WorktreeCreated {
            worktree_id: worktree_id.clone(),
            checkout_id: checkout_id(checkout),
            path: worktree_id.clone(),
            branch: branch.to_string(),
        }));
        self.agent_started_with(
            agent,
            project,
            node,
            &worktree_id,
            kind,
            "",
            launch,
            run_id,
            None,
        );
        if !has_terminal {
            return; // an adapter agent has no terminal
        }
        self.record(Body::TerminalOpened(v1::TerminalOpened {
            terminal_id: crate::control::terminal_id_of(agent),
            node_id: node.to_string(),
            purpose: v1::TerminalPurpose::Agent as i32,
            agent_id: agent.as_str().to_string(),
        }));
    }

    pub fn directory_agent_launched(
        &self,
        agent: &AgentId,
        workspace: &v1::WorkspaceRegistered,
        kind: &str,
        terminal: bool,
    ) {
        let none = crate::launch_options::LaunchChoice::default();
        self.helper_launched(agent, workspace, kind, terminal, "", &none, None);
    }

    /// [`Self::directory_agent_launched`] with the launch options the agent was started with (ADR-0059).
    pub fn directory_agent_launched_with(
        &self,
        agent: &AgentId,
        workspace: &v1::WorkspaceRegistered,
        kind: &str,
        terminal: bool,
        launch: &crate::launch_options::LaunchChoice,
        run_id: Option<[u8; 16]>,
    ) {
        self.workspace_agent_launched(agent, workspace, kind, terminal, "", launch, run_id, None);
    }

    /// [`Self::directory_agent_launched`] for a helper `parent` started (ADR-0057); `parent` empty means the user.
    #[allow(clippy::too_many_arguments)]
    pub fn helper_launched(
        &self,
        agent: &AgentId,
        workspace: &v1::WorkspaceRegistered,
        kind: &str,
        terminal: bool,
        parent: &str,
        launch: &crate::launch_options::LaunchChoice,
        run_id: Option<[u8; 16]>,
    ) {
        // ADR-0059 decision 5: a helper has launch options only when the user's signed decision named them (never inherited); the
        // event carries them, so every client shows the "does not ask" badge on it.
        self.workspace_agent_launched(
            agent, workspace, kind, terminal, parent, launch, run_id, None,
        );
    }

    /// ADR-0119: a helper another machine asked for: as [`Self::helper_launched`], with its parent on that machine and the grant it
    /// started under (if any) on its `AgentStarted`, and no local parent.
    #[allow(clippy::too_many_arguments)]
    pub fn remote_helper_launched(
        &self,
        agent: &AgentId,
        workspace: &v1::WorkspaceRegistered,
        kind: &str,
        terminal: bool,
        remote: &RemoteParent,
        launch: &crate::launch_options::LaunchChoice,
        run_id: Option<[u8; 16]>,
    ) {
        self.workspace_agent_launched(
            agent,
            workspace,
            kind,
            terminal,
            "",
            launch,
            run_id,
            Some(remote),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn workspace_agent_launched(
        &self,
        agent: &AgentId,
        workspace: &v1::WorkspaceRegistered,
        kind: &str,
        terminal: bool,
        parent: &str,
        launch: &crate::launch_options::LaunchChoice,
        run_id: Option<[u8; 16]>,
        remote: Option<&RemoteParent>,
    ) {
        self.record(Body::WorkspaceRegistered(workspace.clone()));
        self.record(Body::AgentWorkspaceBound(v1::AgentWorkspaceBound {
            agent_id: agent.as_str().into(),
            workspace_id: workspace.workspace_id.clone(),
        }));
        self.agent_started_with(
            agent,
            &workspace.project_id,
            &workspace.node_id,
            "",
            kind,
            parent,
            launch,
            run_id,
            remote,
        );
        if terminal {
            self.record(Body::TerminalOpened(v1::TerminalOpened {
                terminal_id: crate::control::terminal_id_of(agent),
                node_id: workspace.node_id.clone(),
                purpose: v1::TerminalPurpose::Agent as i32,
                agent_id: agent.as_str().into(),
            }));
        }
    }

    /// Highest sequence number the hub has acknowledged (durable).
    pub fn acked(&self) -> u64 {
        self.log.lock().unwrap_or_else(|e| e.into_inner()).acked()
    }

    /// The hub acknowledged everything up to `seq`: the buffer may drop it.
    pub fn ack(&self, seq: u64) -> io::Result<()> {
        self.log.lock().unwrap_or_else(|e| e.into_inner()).ack(seq)
    }

    /// Lowest sequence number still retained.
    pub fn oldest_retained(&self) -> u64 {
        self.log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .oldest_retained()
    }

    /// Encoded events after `after`, oldest first, for the uplink.
    pub fn payloads_after(
        &self,
        after: u64,
    ) -> Result<Vec<Vec<u8>>, crate::event_buffer::ReplayError> {
        self.log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .replay_after(after)
    }

    /// Like [`payloads_after`](Self::payloads_after), at most `max` of them: one uplink batch.
    pub fn payloads_after_max(
        &self,
        after: u64,
        max: usize,
    ) -> Result<Vec<Vec<u8>>, crate::event_buffer::ReplayError> {
        self.log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .replay_after_max(after, max)
    }

    /// Decoded events after `after` (exclusive), oldest first; what the uplink sends and tests inspect.
    pub fn events_after(&self, after: u64) -> Vec<v1::Event> {
        let log = self.log.lock().unwrap_or_else(|e| e.into_inner());
        log.replay_after(after)
            .unwrap_or_default()
            .iter()
            .filter_map(|b| v1::Event::decode(b.as_slice()).ok())
            .collect()
    }

    pub fn next_seq(&self) -> u64 {
        self.log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .next_seq()
    }
}

/// Where a node keeps things and what it serves.
#[derive(Debug, Clone)]
pub struct NodeConfig {
    pub node_id: String,
    pub socket: PathBuf,
    /// Event buffer and other node state.
    pub data_dir: PathBuf,
    /// Parent directory of agent worktrees.
    pub worktree_root: PathBuf,
    /// Directory of `*.toml` agent manifests.
    pub manifests_dir: PathBuf,
    /// Git checkouts to register as projects.
    pub projects: Vec<PathBuf>,
    pub limits: NodeLimits,
    /// ADR-0062 item 4: at start, bring back every interrupted agent that can continue its own session, whatever its kind and folder
    /// (`agora node run --resume auto`, which `agora` passes unless the owner said no). The policy file's `resume = auto` with its
    /// kinds and folders works as before.
    pub resume_all: bool,
}

/// What no confined agent may read, write or connect to, whatever its profile opens (#475, and the security reviews of #477).
/// When the node has a home of its own (data beside its worktrees or beside its control socket, as `agora node` lays it out, also
/// with `--worktrees` elsewhere) that is the **whole home**: data (trusted devices, policy, ledger), identity, hub, logs, locks,
/// manifests, adapters, publishers, the registry's keys, the control socket and the agents' mesh sockets, and anything added there
/// later. The profile opens the agent's own worktree and mesh socket again (an allow-list, not a list of what to close that has to
/// grow). Otherwise what is known to be the node's: its data, its control socket and the mesh sockets' folder, and its manifests.
/// Relative paths are made absolute first (a relative home would otherwise resolve to nothing and close nothing).
pub fn node_private_paths(cfg: &NodeConfig) -> Vec<PathBuf> {
    let abs = |p: &Path| std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let (data, worktrees, socket, manifests) = (
        abs(&cfg.data_dir),
        abs(&cfg.worktree_root),
        abs(&cfg.socket),
        abs(&cfg.manifests_dir),
    );
    // Only a folder that is the node's own: never the user's home or a folder above it (`--data ~/x --socket ~/y` would close all
    // of `~` and drop every owner write entry), never a root, never one holding a project the node serves.
    let user_home = crate::usage_files::home_dir().map(|h| abs(&h));
    let home = data.parent().filter(|h| {
        (Some(*h) == worktrees.parent() || Some(*h) == socket.parent())
            && h.parent().is_some()
            && !user_home.as_ref().is_some_and(|u| u.starts_with(h))
            && !cfg.projects.iter().any(|p| abs(p).starts_with(h))
    });
    if let Some(h) = home {
        let mut out = vec![h.to_path_buf()];
        for p in [
            Some(socket.clone()),
            socket.parent().map(|d| d.join("m")),
            Some(manifests.clone()),
        ]
        .into_iter()
        .flatten()
        {
            if !p.starts_with(h) {
                out.push(p);
            }
        }
        return out;
    }
    let mut out = vec![data, socket.clone(), manifests];
    if let Some(dir) = socket.parent() {
        out.push(dir.join("m"));
    }
    out
}

/// ADR-0062 item 4: the most agents a node brings back by itself at a time; the others wait for the next round.
pub const AUTO_RESUME_MAX: usize = 8;
/// ADR-0062 item 4: the most agents a node brings back by itself at one start; any more stay offered (Resume in the window).
pub const AUTO_RESUME_TOTAL: usize = 16;
/// ADR-0062 item 4: an agent the node brought back by itself less than this long ago is not brought back by itself again (a crash
/// loop restarts the node at most 5 times in 10 minutes).
pub const AUTO_RESUME_COOLDOWN_MS: u64 = 10 * 60 * 1000;
/// ADR-0062 item 4: the time between two rounds of agents brought back by themselves.
pub const AUTO_RESUME_GAP: Duration = Duration::from_secs(5);

/// Name of the node's policy file in its data directory (written by the machine's owner; the hub cannot change it).
pub const POLICY_FILE: &str = "policy";

/// What the owner allows on this node. A missing file allows the defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    /// Messages may cross to and from agents on other nodes (`mesh.remote`, default true; ADR-0031).
    pub mesh_remote: bool,
    /// Agents on other machines may ask this node to start helpers (`mesh.remote_helpers`, default true; ADR-0119), only while
    /// `mesh.remote` is on too. Each start still needs a person's signed decision here, or a standing grant one created.
    pub remote_helpers: bool,
    /// Agents may see every machine and agent of the hub (`mesh.fleet`, default true; ADR-0112): the node offers `machines_list` and
    /// `agents_list {scope: "hub"}` and asks the hub for its view only when this allows it.
    pub mesh_fleet: bool,
    /// Routed commands that need a user-signed grant (`signed_commands = a, b*`; ADR-0041). Empty: none do.
    pub signed_commands: crate::grants::SignedCommands,
    /// How agents are confined (`sandbox`, `sandbox.read`, `sandbox.write`; ADR-0047). Default: not at all.
    pub sandbox: crate::sandbox::SandboxPolicy,
    /// What each trusted device may sign (`device.<id>.projects = a, b`; ADR-0049). A device not named is not limited.
    pub device_projects: std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
    /// Whether an agent may start with a launch option that skips permissions (`skip_permissions = forbid | allow | confined_only`;
    /// ADR-0059). Default: allow.
    pub skip_permissions: crate::launch_options::SkipPermissions,
    /// Lines of an existing policy file the node did not understand (`unreadable` when the file could not be read). Any of them sets
    /// `skip_permissions` to `forbid`: a typo (`skip_permission = forbid`) must never leave the default `allow` in place.
    pub problems: Vec<String>,
    /// What happens to interrupted agents after the node starts (`resume`, `resume.kinds`, `resume.folders`; ADR-0060).
    pub resume: ResumePolicy,
    /// Whether the file browser and the review screen show secret files (`files.show_secret = true`; ADR-0080). Default: no. Only the
    /// word `true` turns it on.
    pub show_secret: bool,
    /// ADR-0084: which stored secrets (or environment variables) of the node an agent of a kind receives
    /// (`credentials.<kind> = NAME, ...`). Only the owner's policy file sets it; a line that is malformed lists nothing.
    pub credentials: std::collections::BTreeMap<String, Vec<String>>,
    /// ADR-0084: the kinds whose `credentials.<kind>` line was malformed (and so passes nothing), for `agora doctor`.
    pub credentials_malformed: Vec<String>,
    /// ADR-0092 (#1133): whether the ACP registry's kinds are offered here (`agents.registry = on | off`, default on) and which of
    /// them are not (`agents.registry.deny = registry-a, registry-b`). A value other than `on` or `off` switches them off: a typo never
    /// widens. An unreadable policy file switches them off too.
    pub registry: crate::manifests::RegistryPolicy,
    /// ADR-0089 decision 2: how much of each structured agent's chat history the node keeps on its disk (`chat.history_max_bytes`,
    /// at most the default 32 MiB, 0 for none) and for how long after it was last written (`chat.history_days`, at most the default 30).
    pub history: crate::transcript::Limits,
    /// ADR-0064: the workspace clean-up (`workspaces.clean = auto | off`, `workspaces.grace_hours`, `workspaces.max_gb`,
    /// `workspaces.min_free_gb`).
    pub workspaces: crate::workspaces::Settings,
}

/// ADR-0060: `resume = ask | auto`. `ask` (the default, and any other value) offers Resume in the window and does nothing by itself;
/// `auto` resumes, as the node starts, the interrupted agents that can continue their own session, of a kind in `resume.kinds`, in a
/// folder at or below one in `resume.folders`. Both lists must name something: `auto` with an empty list resumes nothing.
/// ADR-0062 item 4: a `resume` line always wins over `agora node run --resume auto`: `ask` (or any word but `auto`) stops it, and
/// `auto` keeps it to the kinds and folders listed. Only a policy without a `resume` line leaves the choice to the flag.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResumePolicy {
    /// The policy file has a `resume` line.
    pub set: bool,
    pub auto: bool,
    pub kinds: Vec<String>,
    pub folders: Vec<PathBuf>,
    /// ADR-0060 Amendment A: `resume.continue = off` (or any word but `on`) keeps the node from giving a working agent its one "carry
    /// on" message after an automatic resume. On by default, wherever automatic resume is on. Only the owner's policy file sets it.
    pub no_carry_on: bool,
}

impl ResumePolicy {
    /// Whether `auto` covers an agent of `kind` in `folder`.
    pub fn covers(&self, kind: &str, folder: &Path) -> bool {
        let folder =
            agora_platform::canonicalize_plain(folder).unwrap_or_else(|_| folder.to_path_buf());
        self.auto
            && self.kinds.iter().any(|k| k == kind)
            && self.folders.iter().any(|f| {
                let listed = agora_platform::canonicalize_plain(f).unwrap_or_else(|_| f.clone());
                folder.starts_with(&listed)
            })
    }
}

impl Policy {
    /// The routed commands this node gates behind a device-signed grant (ADR-0041), as the node applies them when it starts: the
    /// `signed_commands` line, or, without one, the team default when some device is limited to projects (ADR-0049), since limited
    /// devices only mean something if the commands they could sign are the ones that need a signature. Empty: none is gated.
    pub fn effective_signed_commands(&self) -> crate::grants::SignedCommands {
        if self.signed_commands.0.is_empty() && !self.device_projects.is_empty() {
            crate::grants::SignedCommands::team_default()
        } else {
            self.signed_commands.clone()
        }
    }
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            mesh_remote: true,
            remote_helpers: true,
            mesh_fleet: true,
            signed_commands: crate::grants::SignedCommands::default(),
            sandbox: crate::sandbox::SandboxPolicy::default(),
            device_projects: std::collections::BTreeMap::new(),
            resume: ResumePolicy::default(),
            skip_permissions: crate::launch_options::SkipPermissions::default(),
            show_secret: false,
            credentials: std::collections::BTreeMap::new(),
            credentials_malformed: Vec::new(),
            problems: Vec::new(),
            registry: crate::manifests::RegistryPolicy::default(),
            history: crate::transcript::Limits::default(),
            workspaces: crate::workspaces::Settings::default(),
        }
    }
}

/// The `sandbox.read` / `sandbox.write` entries exactly as written, each with its key, so `agora doctor` can say which ones the node
/// drops (relative, `..`, a `~` with no home folder) or cannot use yet (missing). Same line rules as [`read_policy`].
pub fn policy_sandbox_entries(path: &std::path::Path) -> Vec<(&'static str, String)> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let key = match k.trim() {
            "sandbox.read" => "sandbox.read",
            "sandbox.write" => "sandbox.write",
            "sandbox.sockets" => "sandbox.sockets",
            _ => continue,
        };
        out.extend(
            v.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| (key, s.to_string())),
        );
    }
    out
}

/// Reads `key = value` lines (blank lines and `#` comments skipped). A known key with a value that is not `true` or `false`
/// counts as `false`: a typo must never widen what the node allows. Unknown keys are ignored.
/// Makes one of the node's own folders (its data, its agents' mesh sockets) exist and closes it to every other user (issue #877): the
/// event buffer, the session ledger, `hub-link` and the sockets inside are created under the umask, so on a Mac (every account in
/// group `staff`) or a volume whose default ACL lets other users in (Windows `D:\`) others could read the buffered events and the
/// ledger. An existing folder is closed too, so what an older version left is closed at the next start. On Unix taking the group and
/// other bits off the folder closes everything below it; on Windows the owner-only ACL is passed on to what inherits. Nothing is
/// given to the owner, and a folder reached through a link (a `--data` or `--socket` pointed elsewhere, perhaps at a shared folder),
/// one shared on purpose (sticky or other-writable) or one that cannot be changed (another user's) is written to the node's log and
/// used as it is: the node keeps running its agents (nodes are autonomous).
fn private_node_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    if let Err(e) = agora_platform::close_to_others(dir) {
        crate::log_line!("{}: not closed to other users: {e}", dir.display());
    }
    Ok(())
}

/// A policy value without the quotes an owner may put around it (`"off"`, `'off'`), and without spaces around it.
fn unquote(v: &str) -> &str {
    let v = v.trim();
    v.strip_prefix('"')
        .and_then(|x| x.strip_suffix('"'))
        .or_else(|| v.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')))
        .unwrap_or(v)
        .trim()
}

pub fn read_policy(path: &std::path::Path) -> Policy {
    let mut p = Policy::default();
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        // No file: the defaults, `skip_permissions = allow` included (the maintainer's decision, ADR-0059).
        Err(e) if e.kind() == io::ErrorKind::NotFound => return p,
        Err(_) => {
            p.problems.push("unreadable".into());
            p.skip_permissions = crate::launch_options::SkipPermissions::Forbid;
            // The stricter reading here too: a policy the owner wrote and the node cannot read may say `resume.continue = off`, so no
            // "carry on" message is typed into an agent (ADR-0060 Amendment A).
            p.resume.no_carry_on = true;
            // ...and may switch the registry's kinds off (#1133).
            p.registry.off = true;
            return p;
        }
    };
    for line in text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let Some((k, v)) = line.split_once('=') else {
            p.problems.push(line.chars().take(80).collect());
            continue;
        };
        {
            match k.trim() {
                "mesh.remote" => p.mesh_remote = v.trim() == "true",
                "mesh.remote_helpers" => p.remote_helpers = v.trim() == "true",
                "mesh.fleet" => p.mesh_fleet = v.trim() == "true",
                "files.show_secret" => p.show_secret = v.trim() == "true",
                // ADR-0064: a value it does not understand keeps the default (and `workspaces.clean` off) and is a line not understood.
                key if key.starts_with("workspaces.") => {
                    if p.workspaces.take(key, v) != Some(true) {
                        p.problems.push(line.chars().take(80).collect());
                    }
                }
                // ADR-0089: a value that is not a number keeps the default and is a line the node did not understand.
                "chat.history_max_bytes" => match crate::transcript::Limits::parse_max_bytes(v) {
                    Some(b) => p.history.max_bytes = b,
                    None => p.problems.push(line.chars().take(80).collect()),
                },
                "chat.history_days" => match crate::transcript::Limits::parse_days(v) {
                    Some(d) => p.history.days = d,
                    None => p.problems.push(line.chars().take(80).collect()),
                },
                "signed_commands" => {
                    p.signed_commands = crate::grants::SignedCommands(
                        v.split(',')
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_string)
                            .collect(),
                    );
                }
                key if key
                    .strip_prefix("device.")
                    .and_then(|s| s.strip_suffix(".projects"))
                    .is_some() =>
                {
                    // A named device with a value that names nothing is a device limited to nothing: a typo never widens.
                    if let Some(id) = key
                        .strip_prefix("device.")
                        .and_then(|s| s.strip_suffix(".projects"))
                        .filter(|id| !id.is_empty())
                    {
                        p.device_projects.insert(
                            id.to_string(),
                            v.split(',')
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                                .map(str::to_string)
                                .collect(),
                        );
                    }
                }
                "resume" => {
                    p.resume.set = true;
                    p.resume.auto = v.trim() == "auto";
                }
                "resume.kinds" => {
                    p.resume.kinds = v
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect();
                }
                "resume.folders" => p.resume.folders = paths(v),
                "resume.continue" => p.resume.no_carry_on = v.trim() != "on",
                "sandbox" => p.sandbox.level = crate::sandbox::Level::parse(v),
                "skip_permissions" => {
                    // `"forbid"` as the ADR writes it means the same as `forbid`.
                    let v = v.trim();
                    let v = v
                        .strip_prefix('"')
                        .and_then(|x| x.strip_suffix('"'))
                        .unwrap_or(v);
                    p.skip_permissions = crate::launch_options::SkipPermissions::parse(v);
                }
                "sandbox.read" => p.sandbox.read = paths(v),
                "sandbox.write" => p.sandbox.write = paths(v),
                "sandbox.sockets" => p.sandbox.sockets = paths(v),
                "sandbox.pass_env" => p.sandbox.pass_env = crate::sandbox::parse_pass_env(v),
                key if key.starts_with("credentials.") => {
                    // ADR-0084: a kind with a malformed line is a kind with nothing listed; a typo never widens.
                    let kind = &key["credentials.".len()..];
                    let kind_ok = !kind.is_empty()
                        && kind
                            .chars()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
                    match crate::credentials::parse_listed(v) {
                        Some(names) if kind_ok => {
                            p.credentials.insert(kind.to_string(), names);
                            p.credentials_malformed.retain(|k| k != kind);
                        }
                        _ => {
                            p.credentials.remove(kind);
                            if kind_ok {
                                p.credentials_malformed.push(kind.to_string());
                            } else {
                                p.problems.push(key.chars().take(80).collect());
                            }
                        }
                    }
                }
                "agents.registry" => match unquote(v) {
                    "on" => p.registry.off = false,
                    "off" => p.registry.off = true,
                    _ => {
                        p.registry.off = true;
                        p.problems.push(line.chars().take(80).collect());
                    }
                },
                // Every deny line adds to the list (a second line never undoes the first), each name quoted or not. A name that is
                // not one of this program's registry kinds is a policy problem: it may be a typo of one the owner meant to switch off.
                "agents.registry.deny" => {
                    for name in v.split(',').map(unquote).filter(|s| !s.is_empty()) {
                        if !crate::manifests::is_registry_kind_name(name) {
                            p.problems.push(
                                format!("agents.registry.deny {name}")
                                    .chars()
                                    .take(80)
                                    .collect(),
                            );
                        }
                        p.registry.deny.insert(name.to_string());
                    }
                }
                "sandbox.trust_manifest" => {
                    p.sandbox.trusted_kinds = v
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect();
                }
                other => p.problems.push(other.chars().take(80).collect()),
            }
        }
    }
    if !p.problems.is_empty() {
        p.skip_permissions = crate::launch_options::SkipPermissions::Forbid;
    }
    p
}

/// `sandbox.read` / `sandbox.write`: absolute paths or `~/` below the home folder; a relative or `..` path is dropped, never guessed at.
fn paths(v: &str) -> Vec<PathBuf> {
    v.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(crate::sandbox::expand)
        .collect()
}

#[derive(Debug)]
pub enum NodeError {
    Io(io::Error),
    /// A project path is not inside a git checkout.
    Project(PathBuf),
}

impl std::fmt::Display for NodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Project(p) => write!(f, "not a git checkout: {}", p.display()),
        }
    }
}

impl std::error::Error for NodeError {}

impl From<io::Error> for NodeError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// A running node. Dropping it stops the control socket and the supervision loop (agents are killed).
pub struct Node {
    // Field order matters for drop: stop serving before the supervisor goes away.
    server: Option<ControlServer>,
    stop: Arc<AtomicBool>,
    ticker: Option<JoinHandle<()>>,
    /// ADR-0062 item 4: brings back, a round at a time, the agents still waiting after the first round.
    resume_wait: Option<JoinHandle<()>>,
    git_watch: Option<JoinHandle<()>>,
    /// ADR-0064: the workspace clean-up.
    workspace_clean: Option<JoinHandle<()>>,
    pub supervisor: Arc<Mutex<Supervisor>>,
    pub events: Arc<EventSink>,
    /// Per-agent mesh endpoints (messages, tasks, leases between the agents on this node).
    pub mesh: Arc<crate::mesh_server::MeshServer>,
    /// Sensitive actions waiting for a user's signed decision (ADR-0020).
    pub approvals: Arc<crate::approvals::NodeApprovals>,
    /// User-signed grants for commands the hub routes here (ADR-0041).
    pub grants: Arc<crate::grants::Grants>,
    /// Agents that start agents (ADR-0057).
    pub spawner: Arc<crate::spawn::Spawner>,
    /// The conversations of the agents without a terminal (ADR-0038, ADR-0090).
    pub chat: Arc<crate::chat::ChatBoard>,
    /// The devices this machine trusts and the endorsements and revocations the hub carries (ADR-0061).
    pub devices: Arc<crate::device_trust::DeviceTrust>,
    devices_watch: Option<JoinHandle<()>>,
    /// ADR-0088: the preview forwards (the binary adds the places its own ports are written, [`crate::preview::Previews::add_sources`]).
    pub previews: Arc<crate::preview::Previews>,
    /// ADR-0088: ends each forward at its expiry and says so.
    preview_watch: Option<JoinHandle<()>>,
    /// ADR-0089: writes the chat history of structured agents (`crate::transcript`).
    transcript_writer: Option<JoinHandle<()>>,
    node_id: String,
    /// Manifests that could not be loaded, as `(path, reason)` for the log.
    pub manifest_problems: Vec<(PathBuf, String)>,
    /// Policy file lines the node did not understand (see [`Policy::problems`]), for the log: they forbid launch options that skip
    /// permissions until they are fixed.
    pub policy_problems: Vec<String>,
    /// The event log found at start that named another node and was set aside instead of being sent to the hub, for the log.
    pub events_set_aside: Option<SetAside>,
    /// ADR-0060: the session ledger.
    pub ledger: Arc<crate::ledger::Ledger>,
    /// See [`Node::crash`].
    crashed: bool,
    /// See [`Node::stop_keeping_agents`].
    keep_agents: bool,
    /// The control commands' context, for the stop to wait for the commands it lets finish (ADR-0104 O2).
    commands: Arc<ControlContext>,
    /// F2040: the lock on [`HOME_LOCK_FILE`] that makes the data folder this node's; last, so it is let go after everything else.
    _home_lock: fs::File,
}

/// In the node's data folder: the file a running node holds locked (F2040), so a second node on the same folder fails before it acts.
/// Not the front door's `run/node.lock`, which names the node's supervisor and is not taken by `agora node run` itself.
const HOME_LOCK_FILE: &str = "data.lock";

/// F2040: how long a start waits for [`HOME_LOCK_FILE`]. A process forked from one whose node held it keeps that lock until it runs its
/// own program (a moment), so a node started right after another stopped may find it still held; a running node holds it for good.
const HOME_LOCK_WAIT: Duration = Duration::from_secs(1);

/// F2040: locks [`HOME_LOCK_FILE`] in `data_dir` for this node; `AddrInUse` while another node holds it (as a second bind of the control
/// socket fails).
fn own_data_dir(data_dir: &Path) -> io::Result<fs::File> {
    let file = agora_platform::open_lock_private(&data_dir.join(HOME_LOCK_FILE))?;
    let until = Instant::now() + HOME_LOCK_WAIT;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(fs::TryLockError::WouldBlock) if Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(10));
            }
            // Operational text, not UI text.
            Err(fs::TryLockError::WouldBlock) => {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    "the node's data folder is in use by another node",
                ))
            }
            Err(fs::TryLockError::Error(e)) => return Err(e),
        }
    }
}

impl Node {
    pub fn start(cfg: &NodeConfig) -> Result<Self, NodeError> {
        Self::start_with_event_cap(cfg, EVENT_BUFFER_BYTES)
    }

    /// Like [`start`](Self::start) with an explicit size cap for the event buffer, in bytes (tests use a small one to lose events
    /// the way a long offline burst does).
    pub fn start_with_event_cap(cfg: &NodeConfig, max_bytes: u64) -> Result<Self, NodeError> {
        Self::start_inner(cfg, max_bytes, None)
    }

    /// [`start`](Self::start) with a terminal holder (ADR-0079): agents are started through it, and the ones it kept running while no
    /// node was there are taken up again. `None` is [`start`](Self::start) (and what a node on a system without the holder does).
    pub fn start_with_holder(
        cfg: &NodeConfig,
        holder: Option<crate::holder::HolderSetup>,
    ) -> Result<Self, NodeError> {
        Self::start_inner(cfg, EVENT_BUFFER_BYTES, holder)
    }

    fn start_inner(
        cfg: &NodeConfig,
        max_bytes: u64,
        holder: Option<crate::holder::HolderSetup>,
    ) -> Result<Self, NodeError> {
        private_node_dir(&cfg.data_dir)?;
        // F2040: this node owns its data folder before it reads or writes anything in it (the ledger, the event log, the mesh's files)
        // or brings an agent back: a second node on the same home fails here, never after it acted for the running one.
        let home_lock = own_data_dir(&cfg.data_dir)?;
        std::fs::create_dir_all(&cfg.worktree_root)?;
        let events = Arc::new(EventSink::open_with_cap(
            &cfg.data_dir.join("events"),
            &cfg.node_id,
            max_bytes,
        )?);
        // ADR-0060: what this node knows of its agents across its own restarts; learns from every event recorded from here on.
        let ledger = Arc::new(crate::ledger::Ledger::open(&cfg.data_dir, now_ms())?);
        // Shutdown safety: an agent's end seen once a stop request reached this process (an OS shutdown signals the agents too) is not an end.
        // Only a process that catches stop requests (`agora node`) ever has one.
        ledger.stops_when(agora_platform::stop_requested);
        events.set_ledger(Arc::clone(&ledger));
        // ADR-0089, #1147: the store of chat transcripts, and the node's secret values it reads, which every event that carries the
        // agent's own words has replaced (from the first event recorded, the agents taken up included).
        let policy = read_policy(&cfg.data_dir.join(POLICY_FILE));
        let transcripts = Arc::new(
            crate::transcript::Store::new(
                &cfg.data_dir,
                policy.history,
                Some(crate::credentials::SecretStore::in_data(&cfg.data_dir)),
            )
            .with_env_secrets(policy.credentials.values().flatten().map(String::as_str)),
        );
        events.set_secrets(Arc::clone(&transcripts));

        // Short path on purpose: Unix socket paths are limited to about 100 bytes.
        let mesh_dir = cfg
            .socket
            .parent()
            .map_or_else(|| PathBuf::from("m"), |p| p.join("m"));
        private_node_dir(&mesh_dir)?;
        let mesh = Arc::new(crate::mesh_server::MeshServer::new(
            &mesh_dir,
            agora_mesh::broker::Limits::default(),
        )?);
        mesh.set_remote_allowed(policy.mesh_remote);
        mesh.set_fleet_allowed(policy.mesh_fleet);
        // ADR-0109: the durable inbox opens first, so the reservation below starts above every restored record's number.
        if let Err(e) = mesh.open_handoffs(&cfg.data_dir) {
            crate::log_line!("mesh: durable handoffs unavailable: {e}");
        }
        // ADR-0039: before agents are taken up, so each gets back the shared-board leases the last run renewed for it.
        if let Err(e) = mesh.open_lease_renewals(&cfg.data_dir) {
            crate::log_line!("mesh: lease renewals are not kept across a restart: {e}");
        }
        // #965: ids never start over below what an agent kept across this restart still holds (its `inbox` cursor). A failed
        // reservation does not stop the node: its agents run on, and messages are refused until the mesh's ticker reserves.
        mesh.reserve_ids(&cfg.data_dir);

        let mut sup = Supervisor::new(SupervisorConfig {
            node: NodeId::new(cfg.node_id.clone()),
            restart: RestartPolicy::default(),
            terminal: TerminalOptions::default(),
            limits: cfg.limits,
        });
        let mut sandbox = read_policy(&cfg.data_dir.join(POLICY_FILE)).sandbox;
        sandbox.protect = node_private_paths(cfg);
        sandbox.data_dir = Some(cfg.data_dir.clone());
        // ADR-0082: where a confined agent could otherwise connect to the control socket (Linux), the socket needs a token the agent's
        // profile must not reach. Made here, written when the socket is bound.
        let control_token = if agora_platform::CONTROL_TOKEN_REQUIRED
            && sandbox.level != crate::sandbox::Level::Off
        {
            let file = std::path::absolute(crate::control_token::token_path(&cfg.socket))
                .unwrap_or_else(|_| crate::control_token::token_path(&cfg.socket));
            sandbox.control_token = Some(file.clone());
            Some((crate::control_token::ControlToken::generate()?, file))
        } else {
            None
        };
        sup.set_sandbox(sandbox);
        // #1150: registry kinds that start through a package runner (`npx`, `uvx`) start it in an empty folder of their own here, not
        // in the project, whose package-manager configuration it would otherwise read.
        sup.set_runner_root(cfg.data_dir.join("runner"));
        // ADR-0064: the worktrees this node makes are recorded, capped and cleaned up after their agents.
        let workspaces = Arc::new(crate::workspaces::Workspaces::open(
            &cfg.data_dir,
            &cfg.worktree_root,
        )?);
        workspaces.set_settings(read_policy(&cfg.data_dir.join(POLICY_FILE)).workspaces);
        sup.set_workspaces(Arc::clone(&workspaces));
        // ADR-0084: the secrets this node holds, and which kind may receive which (the node's own policy; the hub cannot set it).
        sup.set_credentials(crate::credentials::Credentials::new(
            crate::credentials::SecretStore::in_data(&cfg.data_dir),
            read_policy(&cfg.data_dir.join(POLICY_FILE)).credentials,
        ));
        sup.reserve_ids(ledger.next_id());
        sup.set_skip_permissions(read_policy(&cfg.data_dir.join(POLICY_FILE)).skip_permissions);
        sup.set_registry_policy(read_policy(&cfg.data_dir.join(POLICY_FILE)).registry);
        let (found, manifest_problems) = load_manifests(&cfg.manifests_dir);
        for l in found {
            sup.register_loaded(l);
        }

        let mut projects = Vec::new();
        for p in &cfg.projects {
            let info = git::discover_checkout(p).map_err(|_| NodeError::Project(p.clone()))?;
            let name = info.root.file_name().map_or_else(
                || "project".to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            projects.push(ProjectEntry {
                id: info.project.to_string(),
                name,
                root: info.root,
            });
        }

        // Every agent that launches joins the mesh and, if its manifest has `[mcp]`, is handed its own endpoint. Set before the holder's
        // agents are taken up: each joins the mesh again at the same path through this hook, so its running bridge reconnects (#906).
        {
            let for_abort = Arc::clone(&mesh);
            let for_runs = Arc::clone(&mesh);
            let mesh = Arc::clone(&mesh);
            sup.set_run_hook(Box::new(move |id, project, run| {
                for_runs.bind_handoff_run(id, project, run)
            }));
            sup.set_launch_hook(Box::new(move |l| {
                let socket = mesh
                    .register_confined(
                        agora_mesh::broker::AgentInfo {
                            id: l.id.clone(),
                            kind: l.kind.clone(),
                            node: l.node.clone(),
                            project: l.project.clone(),
                            label: l.id.as_str().to_string(),
                            worktree: l.worktree.to_string_lossy().into_owned(),
                            state: "idle".into(),
                        },
                        l.identity.as_deref(),
                    )
                    // #1093: the agent still starts, outside the mesh; the node's log says so (operational log, not UI text).
                    .inspect_err(|e| {
                        crate::log_line!(
                            "mesh: {} could not get its endpoint and runs outside the mesh: {e}",
                            l.id.as_str()
                        );
                    })
                    .ok()?;
                let command = std::env::current_exe()
                    .inspect_err(|e| {
                        crate::log_line!(
                            "mesh: {} gets no endpoint (this program's path is unknown): {e}",
                            l.id.as_str()
                        );
                    })
                    .ok()?;
                Some(agora_adapters::inject::Endpoint { command, socket })
            }));
            sup.set_abort_hook(Box::new(move |id| for_abort.unregister(id)));
        }

        // ADR-0079: agents the holder kept running while no node was there are this node's again, before anything is called
        // interrupted. A holder that cannot be had changes nothing: every agent runs in the node.
        let mut taken_up = Vec::new();
        if let Some(setup) = holder {
            let hub = Arc::new(crate::holder::HolderHub::new(setup));
            taken_up = take_up_held(&hub, &mut sup, &ledger, cfg);
            sup.set_holder(hub);
        }
        // #932: only an agent just taken up has a session to show again; every other kept session is an earlier process's (also when
        // there is no holder to attach to).
        {
            let keep: Vec<&str> = taken_up.iter().map(|(id, _)| id.as_str()).collect();
            ledger.forget_kept_except(&keep);
        }
        // ADR-0039: every agent the holder kept has joined the mesh again and got its renewed leases back; the rest are not taken up.
        mesh.forget_unadopted_lease_renewals();
        let views = sup.list();
        let kept: Vec<_> = taken_up
            .iter()
            .filter_map(|(id, session)| {
                views
                    .iter()
                    .find(|v| v.id == *id && v.running)
                    .map(|v| (id, session, v))
            })
            .collect();
        events.node_online_with_runs(&cfg.node_id, kept.iter().map(|(_, _, v)| *v));
        for p in &projects {
            events.checkout_discovered(&cfg.node_id, &p.id, &p.root);
        }
        // `NodeOnline` tells the hub that whatever this node ran before is gone; the agents taken up from the holder are not, so each is
        // said up again after it, as an announcement does (#907).
        for (id, session, v) in kept {
            events.agent_taken_up(v, session.as_ref());
            // #932: its modes, settings and commands as the supervisor holds them, judged again as a report is, so what the
            // window shows matches `skips_permissions_now`.
            events.supervisor_events(&sup.session_events(id), &views);
            // ADR-0101: the kept process still holds its always answers; the hub cleared them on the `AgentStarted` above.
            for a in ledger.kept_session(id.as_str()).always {
                events.restate(Body::AgentAlwaysAllowed(a));
            }
        }
        // ADR-0060: what was running when the node last stopped, offered again (Resume, or Start again in the same folder).
        for e in ledger.interrupted() {
            events.agent_interrupted(&e, sup.offers_resume(&e));
        }
        let known_checkouts: Vec<(String, PathBuf)> = projects
            .iter()
            .map(|p| (p.id.clone(), p.root.clone()))
            .collect();

        // Only devices the owner listed in `<data>/trusted-devices` (`agora node trust`) can answer an approval; with
        // none listed every request waits and expires as denied.
        let (verifier, _skipped) = crate::approvals::P256Verifier::load(
            &cfg.data_dir.join(crate::approvals::TRUSTED_DEVICES_FILE),
        );
        let approvals = Arc::new(crate::approvals::NodeApprovals::new(
            &cfg.node_id,
            Box::new(verifier),
        ));
        approvals.follow_trust_file(&cfg.data_dir.join(crate::approvals::TRUSTED_DEVICES_FILE));
        let devices = Arc::new(crate::device_trust::DeviceTrust::new(
            &cfg.node_id,
            &cfg.data_dir.join(crate::approvals::TRUSTED_DEVICES_FILE),
        ));
        approvals.set_device_trust(Arc::clone(&devices));
        let policy_now = read_policy(&cfg.data_dir.join(POLICY_FILE));
        let signed = policy_now.effective_signed_commands();
        devices.set_policy_limits(policy_now.device_projects.clone());
        let limits = Arc::new(crate::limits::DeviceLimits::new(policy_now.device_projects));
        // ADR-0061: a device endorsed by a limited device is limited the same way (kept in the trusted-devices file).
        limits.follow_trust_file(&cfg.data_dir.join(crate::approvals::TRUSTED_DEVICES_FILE));
        approvals.set_limits(Arc::clone(&limits));
        let grants = Arc::new(
            crate::grants::Grants::new(&cfg.node_id, Arc::clone(&approvals), signed)
                .with_limits(Arc::clone(&limits))
                .with_directory_nonce_file(cfg.data_dir.join("directory-nonces.json")),
        );

        let supervisor = Arc::new(Mutex::new(sup));
        {
            // #1133: a registry kind started through a package runner is started from the hub only by a signed launch.
            let sup = Arc::clone(&supervisor);
            grants.set_signed_start_check(Box::new(move |kind| {
                sup.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .start_needs_signature(kind)
            }));
        }
        {
            let sup = Arc::clone(&supervisor);
            let sup2 = Arc::clone(&sup);
            grants.set_project_resolver(Box::new(move |agent| {
                let s = sup.lock().unwrap_or_else(|e| e.into_inner());
                s.list()
                    .into_iter()
                    .find(|v| v.id.as_str() == agent)
                    .map(|v| v.project)
            }));
            limits.set_project_resolver(Box::new(move |agent| {
                let s = sup2.lock().unwrap_or_else(|e| e.into_inner());
                s.list()
                    .into_iter()
                    .find(|v| v.id.as_str() == agent)
                    .map(|v| v.project)
            }));
        }
        {
            // ADR-0049/0053: where a directory action of a device limited to some projects lands, by the node's own records only: the
            // folders launches confirmed (the same file the launch writes), the registered checkouts, and the ledger for a resume.
            let bindings = cfg.worktree_root.join(crate::directory::BINDINGS_FILE);
            let known: Vec<(String, String, PathBuf)> = projects
                .iter()
                .map(|p| (p.id.clone(), p.name.clone(), p.root.clone()))
                .collect();
            let ledger = Arc::clone(&ledger);
            grants.set_place_resolver(Box::new(move |place| match place {
                crate::grants::Place::Folder(path) => {
                    crate::directory::known_project(&bindings, &known, path)
                }
                crate::grants::Place::Agent(agent) => ledger.get(agent).map(|e| e.project_id),
            }));
        }
        // What this node says when the hub's record of it cannot be continued (ADR-0054, ADR-0055): see `node_announcer`.
        events.set_announcer(node_announcer(
            cfg.node_id.clone(),
            Arc::clone(&supervisor),
            Arc::clone(&approvals),
            Arc::clone(&ledger),
            known_checkouts,
            Box::new(|| {}),
        ));
        // ADR-0089, ADR-0104 C1: the chat history of structured agents on this node's disk; the agents taken up from the holder get
        // theirs back before anything they wrote meanwhile is read (that happens in the supervision tick, which starts below).
        let chat = Arc::new(crate::chat::ChatBoard::saving(transcripts));
        for (id, _) in &taken_up {
            chat.seed_from_disk(id, now_ms());
        }
        let git_wake = Arc::new(AtomicBool::new(false));
        // The node's stop flag: its threads end on it, and its control commands and helper starts are refused once it is set (ADR-0104
        // O2).
        let stop = Arc::new(AtomicBool::new(false));
        // ADR-0057: agents may ask to start helpers; every start needs a decision a trusted device signed.
        let spawner = Arc::new(
            crate::spawn::Spawner::new(
                &cfg.node_id,
                &cfg.worktree_root,
                Arc::clone(&supervisor),
                Arc::clone(&approvals),
                Some(Arc::clone(&events)),
                Arc::clone(&chat),
            )
            .with_git_wake(Arc::clone(&git_wake))
            .with_stopping(Arc::clone(&stop)),
        );
        // ADR-0119: starts other machines ask for, as the owner's policy allows.
        spawner.set_remote_helpers(policy.remote_helpers && policy.mesh_remote);
        mesh.set_helpers(Arc::clone(&spawner) as Arc<dyn crate::mesh_server::Helpers>);
        // ADR-0088: the node's own ports, where a home laid out the default way writes them (`<home>/hub/listen`, `<home>/link/url`,
        // `<home>/link/panel-port`, home being the data folder's parent).
        let previews = Arc::new(crate::preview::Previews::new(
            cfg.data_dir
                .parent()
                .map(crate::preview::default_sources)
                .unwrap_or_default(),
        ));
        let mut ctx = ControlContext::new(
            Arc::clone(&supervisor),
            projects,
            cfg.worktree_root.clone(),
            cfg.node_id.clone(),
            Some(Arc::clone(&events)),
            Some(Arc::clone(&mesh)),
            Some(Arc::clone(&approvals)),
            Arc::clone(&chat),
        )
        .with_manifests_dir(cfg.manifests_dir.clone())
        .with_git_wake(Arc::clone(&git_wake))
        .with_spawner(Arc::clone(&spawner))
        .with_ledger(Arc::clone(&ledger))
        .with_show_secret(read_policy(&cfg.data_dir.join(POLICY_FILE)).show_secret)
        .with_previews(Arc::clone(&previews))
        .with_stopping(Arc::clone(&stop));
        if let Some((token, file)) = control_token {
            ctx = ctx.with_control_token(token, file);
        }
        let ctx = Arc::new(ctx);
        // ADR-0060: `resume = auto` brings back, without anyone asking, the interrupted agents of the kinds and folders the owner listed
        // that can continue their own session. In id order, so a helper comes after its parent (a helper whose parent did not come
        // back stays offered). A refusal leaves the agent offered as before.
        // ADR-0062 item 4: `agora node run --resume auto` brings back every such agent, whatever its kind and folder, unless the
        // policy file has a `resume` line, which always wins (the stricter one). At most AUTO_RESUME_TOTAL at a start, none brought
        // back by itself in the last AUTO_RESUME_COOLDOWN_MS, and AUTO_RESUME_MAX at a time so a machine coming back does not start
        // everything at once: the first round here, before the socket answers, the others every AUTO_RESUME_GAP after (each still
        // offered until then; one resumed or started again by hand meanwhile is refused and skipped).
        let resume = read_policy(&cfg.data_dir.join(POLICY_FILE)).resume;
        let carry_on = !resume.no_carry_on;
        let started_ms = now_ms();
        let mut waiting: std::collections::VecDeque<String> = ledger
            .interrupted()
            .into_iter()
            .filter(|e| {
                let wanted = if resume.set {
                    resume.covers(&e.kind, Path::new(&e.folder))
                } else {
                    cfg.resume_all
                };
                wanted
                    && !e.session_id.is_empty()
                    && started_ms.saturating_sub(e.auto_resumed_at_ms) >= AUTO_RESUME_COOLDOWN_MS
            })
            .map(|e| e.agent_id)
            .take(AUTO_RESUME_TOTAL)
            .collect();
        resume_round(&ctx, &ledger, &mut waiting, carry_on);
        let resume_ctx = Arc::clone(&ctx);
        let commands = Arc::clone(&ctx);
        let server = ControlServer::start(&cfg.socket, ctx)?;

        let ticker = {
            let (sup, ev, stop, node_id, mesh, approvals, chat, spawner) = (
                Arc::clone(&supervisor),
                Arc::clone(&events),
                Arc::clone(&stop),
                cfg.node_id.clone(),
                Arc::clone(&mesh),
                Arc::clone(&approvals),
                Arc::clone(&chat),
                Arc::clone(&spawner),
            );
            let ledger_tick = Arc::clone(&ledger);
            std::thread::Builder::new()
                .name("agora-supervise".into())
                .spawn(move || {
                    let mut reported = (u32::MAX, u32::MAX);
                    let mut reported_memory = 0u64;
                    let mut memory_checked = Instant::now();
                    let mut usage = crate::usage_files::UsageWatcher::default();
                    // ADR-0091: the plan windows a tool reports in the same files, sent when they change.
                    let mut quota = crate::usage_quota::QuotaGate::new(node_id.as_str());
                    let mut sessions = crate::session_files::SessionWatcher::default();
                    // ADR-0060: the process each agent runs as, as last written to the ledger.
                    let mut processes: std::collections::HashMap<AgentId, u32> = Default::default();
                    while !stop.load(Ordering::Acquire) {
                        // ADR-0095: when the list below is taken (under the lock, after the tick): a dispatch made after this is not
                        // dropped by a list that predates it.
                        let views_at = Instant::now();
                        let Some((
                            events,
                            views,
                            usage_targets,
                            ended,
                            session_targets,
                            pids,
                            held,
                            adapter_states,
                        )) = ({
                            let mut s = sup.lock().unwrap_or_else(|e| e.into_inner());
                            // ADR-0104 O2: once the node is stopping, what the adapters said is the stop's to take (`Node::drop`), all
                            // of it and in one place, never half by a last tick.
                            if stop.load(Ordering::Acquire) {
                                None
                            } else {
                                let events = s.tick(Instant::now());
                                Some((
                                    events,
                                    s.list(),
                                    s.usage_targets(),
                                    s.take_ended_usage_targets(),
                                    s.session_file_targets(),
                                    s.agent_pids(),
                                    s.held_agents(),
                                    s.held_adapter_states(),
                                ))
                            }
                        })
                        else {
                            break;
                        };
                        // ADR-0079 decision 9: which holder each running agent is in, so the next node attaches instead of calling
                        // it interrupted. An agent the node runs itself records none (also after a fallback).
                        for v in views.iter().filter(|v| v.running) {
                            let holder = held.1.filter(|_| held.0.contains(&v.id));
                            ledger_tick.set_holder(v.id.as_str(), holder, now_ms());
                        }
                        // ADR-0093: what a later node needs to take up an adapter agent the holder runs (it cannot ask the adapter again).
                        for (agent, session, capabilities) in &adapter_states {
                            ledger_tick.set_adapter_state(
                                agent.as_str(),
                                session,
                                capabilities,
                                now_ms(),
                            );
                        }
                        // ADR-0060: a new process (a start, a restart, a resume) is written to the ledger with its start time, so one
                        // that outlives a crashed node is recognised at the next start. Read only when the pid changed.
                        // #925: with when each process discovery waits for was spawned, so a node that takes it up from the holder later
                        // reads its files from the same start.
                        let starts: std::collections::HashMap<AgentId, u64> = session_targets
                            .iter()
                            .filter_map(|t| Some((t.agent.clone(), ms_after(t.started?))))
                            .collect();
                        record_new_processes(
                            &ledger_tick,
                            &mut processes,
                            &pids,
                            &starts,
                            agora_platform::process_identity,
                            now_ms(),
                        );
                        // ADR-0060: the session ids terminal agents write to their own files (`[session.file]`), read outside the lock.
                        // Polled with no targets too, so what was kept for a process that ended is let go at once (#925).
                        let known = if session_targets.is_empty() {
                            Default::default()
                        } else {
                            sup.lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .known_sessions()
                        };
                        for (agent, id) in sessions.poll(Instant::now(), &session_targets, &known) {
                            // #925: set only for the process it was found for (the lock was let go meanwhile; a restart or a resume
                            // may have replaced it).
                            let Some(launch) = session_targets
                                .iter()
                                .find(|t| t.agent == agent)
                                .map(|t| t.launch)
                            else {
                                continue;
                            };
                            let set = sup
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .report_found_session(&agent, launch, &id);
                            if matches!(set, Ok(true)) {
                                ev.session_reported(&agent, &id);
                            }
                        }
                        // Usage that terminal agents wrote to their own files (ADR-0042); the files are read outside the lock.
                        let mut used = usage.finish(Instant::now(), &ended);
                        used.extend(usage.poll(Instant::now(), &usage_targets));
                        if !used.is_empty() {
                            ev.record_bodies(
                                used.into_iter()
                                    .map(|(agent, s)| {
                                        Body::AgentUsageReported(v1::AgentUsageReported {
                                            agent_id: agent.as_str().to_string(),
                                            model: s.model,
                                            input_tokens: s.input,
                                            output_tokens: s.output,
                                            ..Default::default()
                                        })
                                    })
                                    .collect(),
                            );
                        }
                        let (at, at_ms) = (Instant::now(), now_ms());
                        let mut limits: Vec<_> = usage
                            .take_quota()
                            .iter()
                            .filter_map(|(tool, raw)| quota.offer(at, at_ms, tool, raw))
                            .collect();
                        limits.extend(quota.due(at));
                        if !limits.is_empty() {
                            ev.record_bodies(
                                limits.into_iter().map(Body::AgentQuotaObserved).collect(),
                            );
                        }
                        if !events.is_empty() {
                            ev.supervisor_events(&events, &views);
                        }
                        // Agents that ended on their own leave the mesh; the others keep their state current.
                        // An agent that will be restarted keeps its endpoint (the config it was launched with still
                        // points at it); one that finished or was given up on leaves the mesh.
                        for e in &events {
                            match e {
                                SupervisorEvent::Exited { agent, status } if status.success() => {
                                    mesh.unregister_because(agent, "finished");
                                }
                                SupervisorEvent::GaveUp { agent, .. } => {
                                    mesh.unregister_because(agent, "crashed");
                                }
                                _ => {}
                            }
                            // ADR-0057: an agent's helper grants and waiting start end when it exits or is stopped.
                            if let SupervisorEvent::Exited { agent, .. }
                            | SupervisorEvent::GaveUp { agent, .. }
                            | SupervisorEvent::Stopped { agent } = e
                            {
                                spawner.agent_ended(agent);
                            }
                            // ADR-0090 Amendment A: a card to switch an agent to a mode that skips permission questions names this agent's
                            // session; when it ends, crashes or is stopped (P2-16: also for what it had in use), no decision may apply it
                            // later.
                            if let SupervisorEvent::Exited { agent, .. }
                            | SupervisorEvent::GaveUp { agent, .. }
                            | SupervisorEvent::Stopped { agent }
                            | SupervisorEvent::SkipRefused { agent, .. }
                            | SupervisorEvent::AdapterCrashed { agent } = e
                            {
                                for id in approvals.withdraw_skips(agent, None) {
                                    ev.approval_resolved(&id, false, "");
                                }
                            }
                        }
                        // ADR-0095: every dispatch of an agent no longer running (stopped by a person or its parent, ended, crashed)
                        // is dropped and its task on this node's broker given back; a held prompt of a terminal agent between turns
                        // is pasted on a thread of its own (the paste pauses). A paste that cannot start, or that fails, gives the task
                        // back too (the shared board has no release).
                        if !mesh.dispatches().is_empty() {
                            let seen: Vec<crate::task_dispatch::Seen> = views
                                .iter()
                                .map(|v| crate::task_dispatch::Seen {
                                    id: v.id.clone(),
                                    running: v.running,
                                    terminal: v.terminal,
                                })
                                .collect();
                            let back = mesh.dispatches().tick(
                                &seen,
                                views_at,
                                Instant::now(),
                                // ADR-0095: the live screen of an agent with a held prompt, read now, and paced like the agent's own
                                // detection, so a held prompt adds no screen reads beyond it.
                                |a| {
                                    let s = sup.lock().unwrap_or_else(|e| e.into_inner());
                                    let ready =
                                        s.terminal_readiness(a).is_some_and(|(state, detects)| {
                                            crate::task_dispatch::between_turns(state, detects)
                                        });
                                    (ready, s.detection_interval(a).unwrap_or(Duration::ZERO))
                                },
                                |h| deliver_held(&sup, &mesh, h),
                            );
                            for g in &back {
                                mesh.give_back(g);
                            }
                        }
                        spawner.tick();
                        // ADR-0090: versions of chat entries held back to keep to one per 150 ms go out once they are due.
                        chat.flush_due(now_ms());
                        for v in views.iter().filter(|v| v.running) {
                            mesh.observe(&v.id, state_word(v.state), v.parent.as_ref());
                        }
                        ev.record_bodies(mesh.take_events());
                        // Adapter approval requests enter the queue and are announced; stopped or dead agents leave
                        // nothing waiting; requests that ran out of time are answered as denied.
                        let now = now_ms();
                        for e in &events {
                            match e {
                                SupervisorEvent::Adapter {
                                    agent,
                                    event:
                                        AdapterEvent::ApprovalRequest {
                                            approval_id,
                                            action,
                                            detail,
                                            scopes,
                                            always_label,
                                            always_option_id,
                                            ..
                                        },
                                } => {
                                    mesh.set_activity(agent, "asking the user");
                                    // ADR-0101: which answers the card offers, after the policy (read under the supervisor's lock,
                                    // let go before the queue's is taken).
                                    let terms = sup
                                        .lock()
                                        .unwrap_or_else(|e| e.into_inner())
                                        .approval_raise_terms(agent);
                                    let (strict, permitted) = terms.unwrap_or((false, false));
                                    // #1147 (audit of #1157): the agent's name for its "always" option, as the card (and the always
                                    // signature over it) shows it, with the node's secret values replaced; its id is kept.
                                    let raise = crate::approvals::raise_for(
                                        strict,
                                        scopes,
                                        &chat.redact_text(always_label),
                                        always_option_id,
                                        permitted,
                                    );
                                    let offer = match raise {
                                        crate::approvals::Raise::Card(offer) => Some(offer),
                                        refused => {
                                            // No card a person could allow: the request is said and withdrawn at once (as one the
                                            // agent stopped waiting for), the adapter is answered denied, and the person is told why.
                                            if let Some(p) = approvals.request(
                                                agent,
                                                approval_id,
                                                &chat.redact_text(action),
                                                &chat.redact_text(detail),
                                                now,
                                            ) {
                                                ev.approval_requested(&p);
                                                approvals.withdraw(&p.id);
                                                ev.approval_resolved(&p.id, false, "");
                                            }
                                            let call = sup
                                                .lock()
                                                .unwrap_or_else(|e| e.into_inner())
                                                .prepare_resolve_approval(
                                                    agent,
                                                    approval_id,
                                                    false,
                                                );
                                            if let Err(e) = call.and_then(|c| c.run()) {
                                                crate::supervisor::log_call_failure(
                                                    "approval refusal",
                                                    agent,
                                                    &e,
                                                );
                                            }
                                            let code = if refused
                                                == crate::approvals::Raise::AlwaysOnlyForbidden
                                            {
                                                "approval.always_only_forbidden"
                                            } else {
                                                "adapter.permission_unanswerable"
                                            };
                                            chat.refused_request(agent, code, action, now);
                                            None
                                        }
                                    };
                                    let Some(offer) = offer else {
                                        continue;
                                    };
                                    // #1147 (audit of #1157): the card (the queue, its event, the window) shows the agent's action and
                                    // detail with the node's secret values replaced; the always signature binds those words.
                                    if let Some(p) = approvals.request_offered(
                                        agent,
                                        approval_id,
                                        &chat.redact_text(action),
                                        &chat.redact_text(detail),
                                        now,
                                        offer,
                                    ) {
                                        // ADR-0090: the request also appears in the conversation. The entry points at the id the node
                                        // minted (`p.id`), never at the adapter's own.
                                        chat.push_permission(agent, &p.id, action, now);
                                        ev.approval_requested(&p);
                                    } else {
                                        // No safe id: refuse rather than let the action through.
                                        // The adapter's answer is waited for after the lock is let go.
                                        let call = sup
                                            .lock()
                                            .unwrap_or_else(|e| e.into_inner())
                                            .prepare_resolve_approval(agent, approval_id, false);
                                        if let Err(e) = call.and_then(|c| c.run()) {
                                            crate::supervisor::log_call_failure(
                                                "approval refusal",
                                                agent,
                                                &e,
                                            );
                                        }
                                    }
                                }
                                SupervisorEvent::Restarting { .. }
                                | SupervisorEvent::GaveUp { .. } => {
                                    if let SupervisorEvent::GaveUp { agent, .. } = e {
                                        chat.set_working(agent, false);
                                    }
                                    restart_notice(&chat, e, &views, now);
                                }
                                // P2-16: the node stopped an agent whose mode or config option in use skips permissions under a
                                // policy that refuses it. As a stop asked for (`AgentStop`): it leaves the mesh and what its adapter
                                // asked is withdrawn (the `Stopped` that follows does the rest); its conversation says why.
                                SupervisorEvent::SkipRefused {
                                    agent,
                                    policy,
                                    setting,
                                } => {
                                    mesh.unregister_because(agent, "stopped");
                                    for approval in approvals.cancel_adapter_requests(agent) {
                                        chat.set_permission(
                                            agent,
                                            &approval,
                                            v1::PermissionOutcome::Cancelled,
                                            now,
                                        );
                                        ev.approval_resolved(&approval, false, "");
                                    }
                                    chat.skip_refused(agent, policy, setting, now);
                                }
                                SupervisorEvent::Stopped { agent }
                                | SupervisorEvent::AdapterCrashed { agent } => {
                                    chat.set_working(agent, false);
                                    // The spawner first: a start the agent waited for is withdrawn and its slot released, before
                                    // the queue drops the request.
                                    spawner.agent_ended(agent);
                                    // ADR-0090: what was still waiting for a decision is shown as cancelled, from the queue's own state.
                                    for id in approvals.cancel_agent(agent) {
                                        chat.set_permission(
                                            agent,
                                            &id,
                                            v1::PermissionOutcome::Cancelled,
                                            now,
                                        );
                                    }
                                }
                                other => chat_event(&chat, &mesh, other, now),
                            }
                        }
                        // Review of #1103, round 3: the state each agent is in, as a backstop for a change no event carried (the
                        // transcript writer holds an agent's newest words only while it works).
                        for v in &views {
                            chat.set_working_if_known(
                                &v.id,
                                v.running && v.state == agora_core::AgentState::Working,
                            );
                        }
                        for (node_approval, agent, adapter_id) in approvals.expire(now) {
                            // ADR-0090 Amendment A: nobody decided a switch in time: nothing is applied, and no adapter asked.
                            if approvals.forget_skip(&node_approval) {
                                ev.approval_resolved(&node_approval, false, "");
                                continue;
                            }
                            if spawner.expired(&node_approval) {
                                continue;
                            }
                            // ADR-0119: a card for another machine's start; its asker hears `approval.expired`.
                            if spawner.remote_expired(&node_approval) {
                                continue;
                            }
                            let call = sup
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .prepare_resolve_approval(&agent, &adapter_id, false);
                            // Security review of PR #1190, N3: the agent is told on a thread of its own, so an adapter that does not
                            // answer never holds the tick (and every other agent's events) for its wait.
                            if let Ok(call) = call {
                                let refused = agent.clone();
                                let told = std::thread::Builder::new()
                                    .name("agora-approval-expired".into())
                                    .spawn(move || {
                                        if let Err(e) = call.run() {
                                            crate::supervisor::log_call_failure(
                                                "approval refusal",
                                                &refused,
                                                &e,
                                            );
                                        }
                                    });
                                if let Err(e) = told {
                                    crate::log_line!(
                                        "approval refusal not sent: {}: {e}",
                                        agent.as_str()
                                    );
                                }
                            }
                            // ADR-0090: nobody decided in time. ADR-0110 (S2): unless an answer call of it still runs; that call
                            // records it (with the answer, if it reached the agent).
                            if approvals.flight_ended(&node_approval) {
                                chat.set_permission(
                                    &agent,
                                    &node_approval,
                                    v1::PermissionOutcome::Cancelled,
                                    now,
                                );
                                ev.approval_resolved(&node_approval, false, "");
                            }
                        }
                        // Resources are reported when they change, not on a timer, so an idle node is silent: when the number of
                        // running agents changes, and when their memory moved a lot since the last report (checked every
                        // `MEMORY_CHECK`, only while agents run).
                        let running = u32::try_from(views.iter().filter(|v| v.running).count())
                            .unwrap_or(u32::MAX);
                        let count_changed = (running, running) != reported;
                        if count_changed
                            || (running > 0 && memory_checked.elapsed() >= MEMORY_CHECK)
                        {
                            memory_checked = Instant::now();
                            let pids = sup.lock().unwrap_or_else(|e| e.into_inner()).running_pids();
                            let memory = agents_memory(&pids);
                            if count_changed || memory_moved(reported_memory, memory) {
                                reported = (running, running);
                                reported_memory = memory;
                                ev.node_resources(&node_id, running, running, memory);
                            }
                        }
                        ev.flush_usage();
                        std::thread::sleep(TICK);
                    }
                })?
        };
        let workspace_clean = {
            let (sup, ev, stop, ledger, workspaces) = (
                Arc::clone(&supervisor),
                Arc::clone(&events),
                Arc::clone(&stop),
                Arc::clone(&ledger),
                Arc::clone(&workspaces),
            );
            let root = cfg.worktree_root.clone();
            std::thread::Builder::new()
                .name("agora-workspaces".into())
                .spawn(move || clean_workspaces(&sup, &ev, &ledger, &workspaces, &root, &stop))?
        };
        let git_watch = {
            let (sup, ev, stop, mesh) = (
                Arc::clone(&supervisor),
                Arc::clone(&events),
                Arc::clone(&stop),
                Arc::clone(&mesh),
            );
            std::thread::Builder::new()
                .name("agora-git".into())
                .spawn(move || watch_git(&sup, &ev, &mesh, &stop, &git_wake))?
        };
        // ADR-0061: what this machine trusts, said at start and whenever it changes (the owner's CLI, an endorsement, a revocation, an
        // expiry), so a window can tell whether this machine trusts its device. The file is small and read every 2 s off every hot path.
        let devices_watch = {
            let (ev, stop, devices) =
                (Arc::clone(&events), Arc::clone(&stop), Arc::clone(&devices));
            std::thread::Builder::new()
                .name("agora-devices".into())
                .spawn(move || {
                    let mut said: Option<Vec<String>> = None;
                    while !stop.load(Ordering::Acquire) {
                        let now = devices.trusted_ids(crate::grants::now_ms());
                        if said.as_ref() != Some(&now) {
                            ev.node_trusted_devices(now.clone());
                            said = Some(now);
                        }
                        for _ in 0..20 {
                            if stop.load(Ordering::Acquire) {
                                return;
                            }
                            std::thread::sleep(Duration::from_millis(100));
                        }
                    }
                })?
        };
        // ADR-0088: a forward ends at its expiry even when nobody asks about it; the window's indicator learns it from the event.
        let preview_watch = {
            let (ev, stop, previews) = (
                Arc::clone(&events),
                Arc::clone(&stop),
                Arc::clone(&previews),
            );
            std::thread::Builder::new()
                .name("agora-preview".into())
                .spawn(move || {
                    while !stop.load(Ordering::Acquire) {
                        for f in previews.sweep(now_ms()) {
                            ev.preview_changed(f);
                        }
                        for _ in 0..10 {
                            if stop.load(Ordering::Acquire) {
                                return;
                            }
                            std::thread::sleep(Duration::from_millis(100));
                        }
                    }
                })?
        };
        // ADR-0089: the transcript writer. A node that cannot start it runs its agents all the same; their history is then not saved.
        let transcript_writer = {
            let (sup, ledger, stopping) = (
                Arc::clone(&supervisor),
                Arc::clone(&ledger),
                Arc::clone(&stop),
            );
            crate::transcript::spawn_writer(
                Arc::clone(&chat),
                // ADR-0089 decision 2 (review of #1103, N6): a running agent's file and one offered for Resume are kept; an ended one
                // ages from when it ended (the ledger's last write for it), one the ledger no longer knows from its file.
                Box::new(move |names: &[String]| {
                    use crate::transcript::Kept;
                    // #1119 item 2: a launch holds the supervisor for up to 30 s, and the stop joins this writer: once the node is
                    // stopping the lock is not waited for, and every file is kept (this sweep removes nothing).
                    let Some(sup) = lock_unless(&sup, &stopping) else {
                        return names.iter().map(|n| (n.clone(), Kept::Keep)).collect();
                    };
                    let running: std::collections::HashSet<String> = sup
                        .list()
                        .into_iter()
                        .filter(|v| v.running)
                        .map(|v| v.id.as_str().to_string())
                        .collect();
                    drop(sup);
                    names
                        .iter()
                        .filter_map(|n| {
                            if running.contains(n) {
                                return Some((n.clone(), Kept::Keep));
                            }
                            let e = ledger.get(n)?;
                            let kept = if e.ended {
                                Kept::EndedAt(e.updated_at_ms)
                            } else {
                                // Interrupted (offered for Resume), or still running as far as the ledger knows.
                                Kept::Keep
                            };
                            Some((n.clone(), kept))
                        })
                        .collect()
                }),
                Arc::clone(&stop),
            )
            .unwrap_or_else(|e| {
                // Operational log, not UI text.
                crate::log_line!("transcripts: the writer did not start: {e}");
                None
            })
        };
        // Started last, after every step that can fail: a node that did not start leaves nothing resuming behind it.
        let resume_wait = if waiting.is_empty() {
            None
        } else {
            let (ledger, stop) = (Arc::clone(&ledger), Arc::clone(&stop));
            Some(
                std::thread::Builder::new()
                    .name("agora-resume".into())
                    .spawn(move || {
                        while !waiting.is_empty() {
                            let until = Instant::now() + AUTO_RESUME_GAP;
                            while Instant::now() < until {
                                if stop.load(Ordering::Acquire) {
                                    return;
                                }
                                std::thread::sleep(TICK);
                            }
                            resume_round(&resume_ctx, &ledger, &mut waiting, carry_on);
                        }
                    })?,
            )
        };
        let events_set_aside = events.set_aside().cloned();
        Ok(Self {
            server: Some(server),
            stop,
            ticker: Some(ticker),
            resume_wait,
            git_watch: Some(git_watch),
            workspace_clean: Some(workspace_clean),
            supervisor,
            events,
            mesh,
            approvals,
            grants,
            spawner,
            chat: Arc::clone(&chat),
            devices,
            devices_watch: Some(devices_watch),
            previews,
            preview_watch: Some(preview_watch),
            transcript_writer,
            node_id: cfg.node_id.clone(),
            manifest_problems,
            policy_problems: read_policy(&cfg.data_dir.join(POLICY_FILE)).problems,
            events_set_aside,
            ledger,
            crashed: false,
            keep_agents: false,
            commands,
            _home_lock: home_lock,
        })
    }

    /// What a node killed without warning (`kill -9`) leaves behind, for tests (ADR-0060): it stops serving and supervising and
    /// records nothing more, and its agents' processes are not stopped (on Unix a terminal agent leads its own process group and
    /// outlives the node that way). They are left running for good.
    pub fn crash(mut self) {
        self.crashed = true;
    }

    /// ADR-0104 O2: the stop of `agora node` on a stop request. As dropping the node (what the held adapters said is recorded, the
    /// conversation, the usage and the ledger are written, the held adapters are let go), but no agent is ended and nothing more is
    /// said: the agents the node runs itself are left as a node ended from outside leaves them (ADR-0060: a terminal agent in its own
    /// session runs on and is found still running at the next start; an adapter sees its input end), and the hub learns that the
    /// node went from its connection, as before. Only the drop of a node in-process (tests, an embedding) ends its own agents.
    pub fn stop_keeping_agents(mut self) {
        self.keep_agents = true;
    }

    /// Tells the node's threads that it is stopping, as dropping it does first: the supervision loop takes nothing more from the
    /// agents, and the stop that follows ([`Drop`]) records what they said since (ADR-0104 O2). For a test that holds the supervisor
    /// while it stops the node, so the loop cannot take what is waiting before the stop does.
    pub fn begin_stop(&self) {
        self.stop.store(true, Ordering::Release);
    }

    pub fn socket(&self) -> Option<&Path> {
        self.server.as_ref().map(ControlServer::path)
    }
}

/// How often the memory of running agents is measured between changes in their number.
const MEMORY_CHECK: Duration = Duration::from_secs(30);

/// Resident memory of every process at or below `roots` (each running agent's own process tree), counted once each. A process that
/// cannot be read (gone, or not ours to read) counts as nothing.
fn agents_memory(roots: &[u32]) -> u64 {
    let mut seen = std::collections::HashSet::new();
    let mut total = 0u64;
    for root in roots {
        let Ok(tree) = agora_platform::process_tree(*root) else {
            continue;
        };
        for p in tree {
            if seen.insert(p.pid) {
                total += agora_platform::process_stats(p.pid).map_or(0, |s| s.rss_bytes);
            }
        }
    }
    total
}

/// Whether memory moved enough since the last report to send another: by more than a quarter, or by more than 256 MiB.
fn memory_moved(reported: u64, now: u64) -> bool {
    let moved = reported.abs_diff(now);
    moved > reported / 4 || moved > 256 << 20
}

/// Reports each agent workspace's uncommitted changes whenever they change (`GitStatusChanged`), checked every
/// [`GIT_POLL`] on its own thread so slow git calls never delay the supervisor's ticks. The first check of a
/// workspace is reported too, so a client that connects later sees the current state from the event log.
/// `wake` (set when an agent starts) ends the wait between rounds early, so a new folder's changes are counted at once.
/// It also makes every folder's count be said again: a start registers its folder with no count, even one already counted.
fn watch_git(
    sup: &Mutex<Supervisor>,
    ev: &EventSink,
    mesh: &crate::mesh_server::MeshServer,
    stop: &AtomicBool,
    wake: &AtomicBool,
) {
    let mut last: std::collections::HashMap<(PathBuf, bool), (u32, u32, u32, String)> =
        std::collections::HashMap::new();
    while !stop.load(Ordering::Acquire) {
        let views = sup.lock().unwrap_or_else(|e| e.into_inner()).list();
        last.retain(|(path, _), _| views.iter().any(|v| &v.worktree == path));
        // Shared sessions have one Git status call per directory. A legacy and a new binding each need their event.
        let mut paths = std::collections::BTreeMap::<&Path, (bool, bool)>::new();
        for v in views.iter().filter(|v| v.running) {
            let modes = paths.entry(&v.worktree).or_default();
            if v.directory {
                modes.0 = true;
            } else {
                modes.1 = true;
            }
        }
        for (path, (directory, legacy)) in paths {
            if stop.load(Ordering::Acquire) {
                return;
            }
            let Ok(st) = git::status(path) else {
                continue;
            };
            // Agents see how many files are changed in each other's workspace (`workspace_status`).
            mesh.set_files_changed(path, st.files_changed);
            let now = (
                st.files_changed,
                st.insertions,
                st.deletions,
                st.head_commit.clone(),
            );
            for (enabled, is_directory) in [(directory, true), (legacy, false)] {
                if !enabled {
                    continue;
                }
                let key = (path.to_path_buf(), is_directory);
                if last.get(&key) == Some(&now) {
                    continue;
                }
                last.insert(key, now.clone());
                if is_directory {
                    ev.record(Body::WorkspaceStatusChanged(v1::WorkspaceStatusChanged {
                        workspace_id: format!("directory:{}", path.to_string_lossy()),
                        files_changed: st.files_changed,
                        insertions: st.insertions,
                        deletions: st.deletions,
                        head_commit: st.head_commit.clone(),
                    }));
                } else {
                    ev.git_status_changed(&path.to_string_lossy(), &st);
                }
            }
        }
        let until = Instant::now() + GIT_POLL;
        while Instant::now() < until && !stop.load(Ordering::Acquire) {
            if wake.swap(false, Ordering::AcqRel) {
                // A start registers its folder again with no count (#1017), also a folder another agent shares, whose changes are
                // the same: every folder's count is said again, so it follows that registration for the hub and every window.
                last.clear();
                break;
            }
            std::thread::sleep(TICK);
        }
    }
}

/// ADR-0062 item 4: resumes waiting agents, in order, until AUTO_RESUME_MAX came back or none wait. A refusal (the agent was resumed or
/// started again by hand meanwhile, or its folder is gone) skips it and leaves it offered.
fn resume_round(
    ctx: &ControlContext,
    ledger: &crate::ledger::Ledger,
    waiting: &mut std::collections::VecDeque<String>,
    carry_on: bool,
) {
    let mut brought = 0;
    while brought < AUTO_RESUME_MAX {
        let Some(id) = waiting.pop_front() else {
            return;
        };
        // ADR-0060 Amendment A: an agent that was working gets its kind's one "carry on" message with its resume, if the owner's policy
        // allows it and it has not had one for this interruption; the message itself comes from the kind's manifest.
        let carry = carry_on && ledger.wants_continue(&id);
        // #1119 item 3: as a command is, counted while it runs and not begun once the node is stopping (the agent stays offered).
        let Some(_running) = ctx.admit() else {
            return;
        };
        if crate::control::resume_agent(ctx, &id, false, carry).is_ok() {
            ledger.mark_auto_resumed(&id, now_ms());
            brought += 1;
        }
    }
}

impl Drop for Node {
    /// A clean stop (ADR-0104 O2). Control commands are refused from the first step on. Every adapter the holder runs is let go first
    /// (a call still waiting on one, in the supervision loop or a command, returns at once and counts as given), then the supervision
    /// loop is waited for, and what the node had read from those adapters and not recorded yet is recorded as the loop would (states,
    /// options, usage and conversation; a request for the person is asked again by the next node from the holder's replay, ADR-0093).
    /// Then the conversation's last batch, the usage still being gathered and the ledger's kept files are written. Waiting for the
    /// supervisor's lock is bounded by [`LOCK_WAIT`], for the readers by [`DRAIN_WAIT`] and for the loop by [`LOOP_WAIT`]; the rest is
    /// local work. Agents the holder runs keep running. With [`Node::stop_keeping_agents`] (what `agora node` does on `SIGTERM`) that
    /// is all, and no other thread is waited for; otherwise the node's other threads are joined and its own agents end with it.
    ///
    /// #1119 item 2: a start or a resume launches its adapter under the supervisor's lock (its `initialize` may take 30 s). When the
    /// lock cannot be had within [`LOCK_WAIT`], what does not need it is written at once (the conversation, the usage, the ledger as it
    /// stands), and the lock is tried again until [`LOCK_RETRY_UNTIL`] after the stop began. Still busy then: `agora node`'s stop lets
    /// nothing go (its held adapters see the link close, as at a kill, and what their readers had not handed over is the holder's to
    /// keep); a node dropped in-process waits for the lock, as its agents must end.
    fn drop(&mut self) {
        // Before the control server ends (it can take up to its poll time): a command that comes meanwhile is refused, not run.
        self.stop.store(true, Ordering::Release);
        // Shutdown safety: an agent's end seen from here on is the stop's doing, never an end in the ledger.
        self.ledger.stopping();
        drop(self.server.take());
        if self.crashed {
            self.join_threads();
            // Nothing is said, recorded or stopped: the supervisor, which owns the agents' processes, is never dropped.
            std::mem::forget(Arc::clone(&self.supervisor));
            return;
        }
        // ADR-0088: forwards never outlive the node; said before the event log is written below.
        for f in self.previews.stop_all() {
            self.events.preview_changed(f);
        }
        // ADR-0039: the lease renewals as they are now, before agents leave the mesh below (the holder keeps them running).
        self.mesh.freeze_lease_renewals();
        let began = Instant::now();
        // Shutdown safety: `agora node`'s stop may be an OS shutdown's, which may not wait for the whole deadline: the usage, the ledger and
        // the event log are written first, and again below with what the drain adds. The conversation keeps its one last batch after
        // the drain (its held-back words are taken whole only then), as before.
        if self.keep_agents {
            self.events.flush_all_usage();
            let _ = self.ledger.flush_all();
            let _ = self.events.sync();
        }
        let supervisor = Arc::clone(&self.supervisor);
        let mut sup = lock_by(&supervisor, began + LOCK_WAIT);
        let missed = sup.is_none();
        if missed {
            self.write_held();
            sup = lock_by(&supervisor, began + LOCK_RETRY_UNTIL);
        }
        // In-process, the agents must end: the lock is waited for, and the stop has no deadline.
        let waited = sup.is_none() && !self.keep_agents;
        if waited {
            sup = Some(supervisor.lock().unwrap_or_else(|e| e.into_inner()));
        }
        // ADR-0104 O2: what the held adapters said that the supervision loop had not taken (it never ticks once the stop is set). The
        // readers get DRAIN_WAIT from when the lock was had, within LOCK_WAIT + DRAIN_WAIT of the stop's start unless it was waited for.
        let drain = sup.map(|mut s| {
            let mut until = Instant::now() + DRAIN_WAIT;
            if !waited {
                until = until.min(began + LOCK_WAIT + DRAIN_WAIT);
            }
            let drained = s.let_go_held(until);
            (drained, s.list())
        });
        // The loop's own last round and the commands already running go first: their events are older than the drained ones, and a
        // call they waited on a held adapter for has just come back as given. Either may wait on an adapter the node runs itself (a
        // call of up to 10 s), so the stop waits for them only so long; not at all when the lock was never had (they wait for it).
        if drain.is_some() {
            let until = Instant::now() + LOOP_WAIT;
            while Instant::now() < until
                && (self.commands.commands_running() > 0
                    || self.ticker.as_ref().is_some_and(|t| !t.is_finished()))
            {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        if let Some(t) = self.ticker.take() {
            if t.is_finished() || !self.keep_agents {
                let _ = t.join();
            }
        }
        if let Some((drained, views)) = &drain {
            if !drained.is_empty() {
                self.events.supervisor_events(drained, views);
                let now = now_ms();
                for e in drained {
                    chat_event(&self.chat, &self.mesh, e, now);
                }
            }
            // Before anything that could take long, as a stop has a deadline (written once already when the lock was busy: again, with
            // what came since).
            self.write_held();
        }
        if self.keep_agents {
            // The process ends right after: the other threads (a Git or device watcher may wait on a slow command) are not waited for,
            // and as with `crash` the supervisor, which owns the agents' processes, is never dropped, so none is ended.
            std::mem::forget(supervisor);
            return;
        }
        self.join_threads();
        self.events.node_offline(&self.node_id);
        // The agents end with the node, now (other parts of the node may still hold the supervisor, so its own drop is not relied on).
        // Its hooks go too: they hold the mesh, which (through the helpers) holds the supervisor, so in a process that goes on the
        // agents' mesh endpoints would stay locked and a next node there could not serve them.
        {
            let mut sup = self.supervisor.lock().unwrap_or_else(|e| e.into_inner());
            sup.shutdown();
            sup.clear_hooks();
        }
        // The agents' last states. They ended without an end in the ledger: at the next start they are interrupted (ADR-0060). And
        // what the kept agents last reported of their session (#932).
        let _ = self.ledger.flush_all();
    }
}

impl Node {
    /// ADR-0104 O2: the conversation's last batch (its writer ends first, so the two never write at once), then the usage still being
    /// gathered, then the ledger and the kept session files, then the event log synced to disk: none needs the supervisor's
    /// lock, and each is quick local work. Can be called again (with what came since).
    fn write_held(&mut self) {
        if let Some(t) = self.transcript_writer.take() {
            if let Some(w) = self.chat.wake() {
                w.notify();
            }
            let _ = t.join();
        }
        crate::transcript::flush(&self.chat);
        self.events.flush_all_usage();
        let _ = self.ledger.flush_all();
        let _ = self.events.sync();
    }

    /// Joins the node's threads (each ends on the stop flag); the transcript writer is woken first.
    fn join_threads(&mut self) {
        for t in [
            self.ticker.take(),
            self.resume_wait.take(),
            self.git_watch.take(),
            self.workspace_clean.take(),
            self.devices_watch.take(),
            self.preview_watch.take(),
        ]
        .into_iter()
        .flatten()
        {
            let _ = t.join();
        }
        if let Some(t) = self.transcript_writer.take() {
            if let Some(w) = self.chat.wake() {
                w.notify();
            }
            let _ = t.join();
        }
    }
}

/// `m` locked, waiting until `until` at most (`None` when it is still held then). A lock poisoned by a panic is taken as it is, as
/// everywhere in the node.
fn lock_by<T>(m: &Mutex<T>, until: Instant) -> Option<std::sync::MutexGuard<'_, T>> {
    loop {
        match m.try_lock() {
            Ok(g) => return Some(g),
            Err(std::sync::TryLockError::Poisoned(e)) => return Some(e.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) if Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(std::sync::TryLockError::WouldBlock) => return None,
        }
    }
}

/// ADR-0064: the first pass a minute after the node started, then every hour, or at once when a start was refused for the cap.
const WORKSPACE_FIRST: Duration = Duration::from_secs(60);
const WORKSPACE_EVERY: Duration = Duration::from_secs(60 * 60);

/// ADR-0064: the node's own workspace clean-up (`crate::workspaces`). Who is where comes from the supervisor (agents that run or wait
/// to be restarted) and the session ledger (agents interrupted and still offered count as in use; ended ones with when they ended),
/// asked again for each workspace once it is claimed. Each removal is said to the hub as `WorktreeRemoved`.
fn clean_workspaces(
    sup: &Mutex<Supervisor>,
    ev: &EventSink,
    ledger: &crate::ledger::Ledger,
    workspaces: &crate::workspaces::Workspaces,
    root: &Path,
    stop: &AtomicBool,
) {
    let users = || {
        // Not waited for once the node stops: then every workspace counts as in use (nothing is removed).
        let Some(s) = lock_unless(sup, stop) else {
            return crate::workspaces::Users::unknown();
        };
        let busy = s.busy_folders();
        drop(s);
        let mut u = crate::workspaces::Users::from_ledger(&ledger.entries());
        for f in busy {
            u.busy(&f);
        }
        u
    };
    let mut next = Instant::now() + WORKSPACE_FIRST;
    while !stop.load(Ordering::Acquire) {
        if Instant::now() < next && !workspaces.take_wake() {
            std::thread::sleep(TICK);
            continue;
        }
        let how = crate::workspaces::Clean {
            by_node: true,
            older_than_ms: None,
            dry_run: false,
        };
        for o in workspaces.clean(&users, now_ms(), how, stop) {
            if o.action != crate::workspaces::Action::Removed {
                continue;
            }
            // Said for the place the node made it at (`WorktreeCreated`), and for its canonical spelling when that differs.
            let mut ids = vec![o.path.to_string_lossy().into_owned()];
            if let Some(name) = o.path.file_name() {
                let made = root.join(name).to_string_lossy().into_owned();
                if made != ids[0] {
                    ids.push(made);
                }
            }
            for worktree_id in ids {
                ev.record(Body::WorktreeRemoved(v1::WorktreeRemoved { worktree_id }));
            }
        }
        next = Instant::now() + WORKSPACE_EVERY;
    }
}

/// `m` locked, waiting for it as long as `stop` is not set (`None` once it is and the lock is still held).
fn lock_unless<'a, T>(m: &'a Mutex<T>, stop: &AtomicBool) -> Option<std::sync::MutexGuard<'a, T>> {
    loop {
        match m.try_lock() {
            Ok(g) => return Some(g),
            Err(std::sync::TryLockError::Poisoned(e)) => return Some(e.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) if !stop.load(Ordering::Acquire) => {
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(std::sync::TryLockError::WouldBlock) => return None,
        }
    }
}

/// #1119 item 2: how long a node that stops waits at first for the supervisor's lock, which a start or a resume holds while it launches
/// an adapter (up to 30 s). Past it, what does not need the lock is written at once.
pub const LOCK_WAIT: Duration = Duration::from_secs(1);

/// #1119 item 2: until when, from the start of a stop, the supervisor's lock is tried again after [`LOCK_WAIT`] (and what does not need
/// it was written). The readers then still get at least `LOCK_WAIT + DRAIN_WAIT - LOCK_RETRY_UNTIL` (1 s), and the loop [`LOOP_WAIT`]:
/// 4 s in all at most, under `agora node`'s 5 s stop deadline.
pub const LOCK_RETRY_UNTIL: Duration = Duration::from_secs(2);

/// ADR-0104 O2: how long a node that stops waits at most for its supervision loop's last round, after it let the held adapters go.
pub const LOOP_WAIT: Duration = Duration::from_secs(1);

/// ADR-0104 O2: how long a node that stops waits at most for the readers of the adapters the holder runs to hand over what they read.
/// A read of a holder that does not answer waits up to the link's frame time (5 s); the stop does not. Lines the holder hands such a
/// reader after the wait count as given there and are lost, as with a kill. Under `agora node`'s whole stop deadline
/// (`crates/agora/src/main.rs`).
pub const DRAIN_WAIT: Duration = Duration::from_secs(2);

/// What an adapter's event changes in its conversation (and the mesh's view of it), as the supervision loop and a clean stop's drain
/// (ADR-0104 O2) both record it. Approval requests, ends and restarts are the loop's own.
fn chat_event(
    chat: &crate::chat::ChatBoard,
    mesh: &crate::mesh_server::MeshServer,
    e: &SupervisorEvent,
    now: u64,
) {
    match e {
        // What a chat agent says goes to its transcript (ADR-0038).
        SupervisorEvent::Adapter {
            agent,
            event: AdapterEvent::Chat { item, cuts, .. },
        } => {
            // Other agents see the kind of this item in fixed words (`workspace_status`), never its text.
            if let Some(word) = activity_word(item) {
                mesh.set_activity(agent, word);
            }
            // ADR-0111: the supervisor kept the places only from an adapter that declared `chat.cuts`.
            chat.push_item_cut(agent, item, cuts, now);
        }
        // #908: a queued prompt became the turn: its entry takes the time it went (the wall clock now, not the tick's, which is older
        // than what this tick already said).
        SupervisorEvent::PromptSending { agent, tag } => {
            chat.sending(agent, *tag, crate::daemon::now_ms());
        }
        // ADR-0090: a queued prompt went out (or could not), or was lost with its adapter.
        SupervisorEvent::PromptDequeued { agent, tag, sent } => {
            if *sent {
                chat.unqueue(agent, *tag, now);
            } else {
                chat.drop_queued(agent, &[*tag], now);
            }
            // ADR-0095: a dispatch prompt that could not be sent gives its task back.
            if let Some(g) = mesh.dispatches().dequeued(agent, *tag, *sent) {
                mesh.give_back(&g);
            }
        }
        // ADR-0110 decision 6: a queued prompt whose adapter did not answer in time stays, marked as not confirmed. A dispatch prompt
        // gives its task back as for one that was not sent (today's handling).
        SupervisorEvent::PromptUnconfirmed {
            agent,
            tag,
            kind,
            waited_ms,
        } => {
            chat.unconfirmed(agent, *tag, kind, *waited_ms, now);
            if let Some(g) = mesh.dispatches().dequeued(agent, *tag, false) {
                mesh.give_back(&g);
            }
        }
        SupervisorEvent::PromptsDropped { agent, tags } => {
            chat.drop_queued(agent, tags, now);
            for g in mesh.dispatches().dropped(agent, tags) {
                mesh.give_back(&g);
            }
        }
        SupervisorEvent::OutputDropped { agent, bytes } => {
            chat.output_dropped(agent, *bytes, now);
        }
        // Review of #1103 (B1): a resumed session's replay of its conversation is shown only when this node holds none of it (its file
        // was purged, never written, or the agent never spoke here).
        SupervisorEvent::ReplayedChat { agent, items } => {
            chat.replayed(agent, items, now);
        }
        // Review of #1103, round 3: the transcript writer holds the agent's newest words back only while it is in a turn; leaving one
        // (or ending) closes them.
        SupervisorEvent::StateChanged { agent, state, .. } => {
            chat.set_working(agent, *state == agora_core::AgentState::Working);
        }
        SupervisorEvent::Exited { agent, .. } => {
            chat.set_working(agent, false);
        }
        _ => {}
    }
}

/// ADR-0095: pastes a held dispatch prompt into its terminal agent on a thread of its own (the paste pauses); false when it cannot start
/// (no terminal, or the program turned bracketed paste off since the prompt was held, which is logged). A paste that fails on its
/// thread gives the task back there.
fn deliver_held(
    sup: &Mutex<Supervisor>,
    mesh: &Arc<crate::mesh_server::MeshServer>,
    h: &crate::task_dispatch::Held,
) -> bool {
    let control = {
        let s = sup.lock().unwrap_or_else(|e| e.into_inner());
        s.terminal(&h.given.agent)
            .map(|t| (t.bracketed_paste(), t.input_handle().ok()))
    };
    let c = match control {
        Some((true, Some(c))) => c,
        Some((false, _)) => {
            // Operational log, not UI text.
            crate::log_line!(
                "task dispatch: {} turned bracketed paste off; the held prompt for task {} is dropped and the task given back",
                h.given.agent.as_str(),
                h.given.task_id
            );
            return false;
        }
        _ => return false,
    };
    let (mesh, h) = (Arc::clone(mesh), h.clone());
    std::thread::Builder::new()
        .name("agora-dispatch-paste".into())
        .spawn(move || {
            if crate::task_dispatch::paste_prompt(&c, &h.prompt).is_err() {
                mesh.give_back(&h.given);
            }
        })
        .is_ok()
}

/// The node's agent kinds: the built-in manifests, then every `*.toml` in `dir` (a file there replaces the built-in kind of the same
/// name or adds one; see `crate::manifests`), and each file that could not be read or parsed with the reason.
/// The node reads them at start and again before it lists or starts agents, so a manifest added later is seen without a restart.
pub fn load_manifests(dir: &Path) -> (Vec<crate::manifests::Loaded>, Vec<(PathBuf, String)>) {
    let (loaded, problems) = crate::manifests::load(dir);
    for (kind, file) in crate::manifests::shadowing(&loaded) {
        crate::log_line!(
            "manifests: {} replaces the built-in kind {kind}; newer built-in features of it (such as plan limits) are not used",
            file.display()
        );
    }
    (loaded, problems)
}

/// The state a ledger entry last knew (its `last_state` word), or `None` for one that is not a state worth showing again.
fn ledger_state(word: &str) -> Option<agora_core::AgentState> {
    match word {
        "working" => Some(agora_core::AgentState::Working),
        "blocked" => Some(agora_core::AgentState::Blocked),
        "idle" => Some(agora_core::AgentState::Idle),
        "done" => Some(agora_core::AgentState::Done),
        _ => None,
    }
}

/// ADR-0079 decisions 4 and 9: attaches to the node's holder if one runs, and puts back every agent it kept running that the ledger
/// says it holds (the same holder, by pid and start time). Whatever is not put back is **ended through the holder**, never left
/// attached and unread: the attach made the holder stop reading each terminal and forget its buffer, and the node drops what it does
/// not take, so a running agent left like that would block on a full terminal for good. That is an agent that ended while no node was
/// there (released; its entry stays interrupted, offered for resume), one the ledger does not know or names another holder, one whose
/// folder or project is gone, and one that could not be taken up. When one cannot be ended the holder is not used for new agents.
/// An agent in a worktree the node made is taken up in the project the ledger recorded for it (T8b).
/// Returns the agents taken up, each with the session id the ledger knew, for the events that say them up again.
fn take_up_held(
    hub: &crate::holder::HolderHub,
    sup: &mut Supervisor,
    ledger: &crate::ledger::Ledger,
    cfg: &NodeConfig,
) -> Vec<(AgentId, Option<String>)> {
    use crate::holder_link::AgentState as Held;
    let Some((session, held, pipes)) = hub.attach_existing() else {
        return Vec::new();
    };
    let mut taken_up = Vec::new();
    let mut ended = Vec::new();
    for h in held {
        let id = h.info.info.agent.clone();
        let running = matches!(h.info.info.state, Held::Running);
        let entry = ledger.get(&id);
        let theirs = entry
            .as_ref()
            .is_some_and(|e| (e.holder_pid, e.holder_started) == (session.pid, session.start));
        let adoptable = entry.filter(|e| running && theirs && e.interrupted());
        let Some(entry) = adoptable else {
            ended.push(h);
            continue;
        };
        let folder = PathBuf::from(&entry.folder);
        let project = if entry.directory {
            crate::directory::inspect(&entry.folder, &cfg.node_id, false)
                .ok()
                .map(|(_, p)| p)
        } else {
            // T8b: in a worktree the node made, the project the node recorded before the agent ran, never one computed again from a
            // HEAD the agent (or another agent of the repository) can move; a running agent is not ended over a branch that moved.
            entry
                .recorded_project()
                .filter(|_| folder.is_dir())
                .or_else(|| git::discover_checkout(&folder).ok().map(|c| c.project))
        };
        let Some(project) = project else {
            ended.push(h);
            continue;
        };
        let restore = crate::supervisor::Restore {
            id: AgentId::new(id.clone()),
            kind: entry.kind.clone(),
            project,
            folder,
            directory: entry.directory,
            branch: entry.branch.clone(),
            session: (!entry.session_id.is_empty()).then(|| entry.session_id.clone()),
            carry_on: false,
            private_config: (entry.directory && !entry.managed_worktree).then(|| {
                cfg.worktree_root
                    .join(".sessions")
                    .join(format!("resume-{id}"))
            }),
        };
        // A failed take-up consumes the terminal: the agent is ended by name below, like the others.
        if sup
            .adopt_held(
                restore,
                h,
                &session,
                held_since(entry.process_since_ms),
                Instant::now(),
            )
            .is_err()
        {
            ended.push(crate::holder_link::Held {
                info: crate::holder_link::TerminalInfo {
                    info: crate::holder_link::AgentInfo {
                        agent: id,
                        pid: 0,
                        state: Held::Running,
                    },
                    buffer_dropped: false,
                    buffer: Vec::new(),
                },
                terminal: None,
            });
            continue;
        }
        sup.seed_reattached_state(&AgentId::new(id.clone()), ledger_state(&entry.last_state));
        sup.set_kept_launch(
            &AgentId::new(id.clone()),
            &entry.launch_options,
            entry.skips_permissions,
            &entry.launch_settings,
        );
        if !entry.parent_agent_id.is_empty() {
            sup.set_parent(
                &AgentId::new(id.clone()),
                &AgentId::new(entry.parent_agent_id.clone()),
            );
        }
        ledger.reclaim(&id, (session.pid, session.start), now_ms());
        let known = (!entry.session_id.is_empty()).then(|| entry.session_id.clone());
        taken_up.push((AgentId::new(id), known));
    }
    let mut ended_pipes = Vec::new();
    for p in pipes {
        let id = p.info.agent.clone();
        let running = matches!(p.info.state, Held::Running);
        let entry = ledger.get(&id);
        let theirs = entry
            .as_ref()
            .is_some_and(|e| (e.holder_pid, e.holder_started) == (session.pid, session.start));
        let adoptable = entry.filter(|e| running && theirs && e.interrupted());
        // A pipe agent this node does not take up is ended through the holder, like a terminal: an agent nobody reads is one nobody
        // answers.
        let gone = |id: &str| crate::holder_link::HeldPipe {
            info: crate::holder_link::AgentInfo {
                agent: id.to_string(),
                pid: 0,
                state: Held::Running,
            },
            dropped: 0,
            eof: false,
            seen: Vec::new(),
            fresh: Vec::new(),
        };
        let Some(entry) = adoptable else {
            ended_pipes.push(p);
            continue;
        };
        let folder = PathBuf::from(&entry.folder);
        let project = if entry.directory {
            crate::directory::inspect(&entry.folder, &cfg.node_id, false)
                .ok()
                .map(|(_, p)| p)
        } else {
            // T8b: in a worktree the node made, the project the node recorded before the agent ran, never one computed again from a
            // HEAD the agent (or another agent of the repository) can move; a running agent is not ended over a branch that moved.
            entry
                .recorded_project()
                .filter(|_| folder.is_dir())
                .or_else(|| git::discover_checkout(&folder).ok().map(|c| c.project))
        };
        let Some(project) = project else {
            ended_pipes.push(p);
            continue;
        };
        let restore = crate::supervisor::Restore {
            id: AgentId::new(id.clone()),
            kind: entry.kind.clone(),
            project,
            folder,
            directory: entry.directory,
            branch: entry.branch.clone(),
            session: (!entry.session_id.is_empty()).then(|| entry.session_id.clone()),
            carry_on: false,
            private_config: None,
        };
        let kept = ledger.kept_session(&id);
        let (options, config, commands) = kept_session_parts(&kept);
        let reattach = crate::supervisor::Reattach {
            session_id: entry.adapter_session.clone(),
            capabilities: entry.adapter_capabilities.clone(),
            state: ledger_state(&entry.last_state),
            options,
            config,
            commands,
        };
        if sup
            .adopt_held_pipe(restore, reattach, p, &session, Instant::now())
            .is_err()
        {
            ended_pipes.push(gone(&id));
            continue;
        }
        sup.set_kept_launch(
            &AgentId::new(id.clone()),
            &entry.launch_options,
            entry.skips_permissions,
            &entry.launch_settings,
        );
        if !entry.parent_agent_id.is_empty() {
            sup.set_parent(
                &AgentId::new(id.clone()),
                &AgentId::new(entry.parent_agent_id.clone()),
            );
        }
        ledger.reclaim(&id, (session.pid, session.start), now_ms());
        let known = (!entry.session_id.is_empty()).then(|| entry.session_id.clone());
        taken_up.push((AgentId::new(id), known));
    }
    if !crate::holder::HolderHub::end_unadopted_all(&session, ended, ended_pipes) {
        hub.retire_session();
    }
    taken_up
}

/// #932: a kept session as the supervisor holds it (the reverse of what the node records for an adapter's reports). A config option's
/// category the protocol names only as custom comes back as `_custom`, which is judged the same way.
fn kept_session_parts(
    kept: &crate::ledger::KeptSession,
) -> (
    agora_adapters::host::SessionOptions,
    Vec<agora_adapters::host::ConfigOption>,
    Vec<agora_adapters::host::AdapterCommand>,
) {
    use agora_adapters::host::{AdapterCommand, ConfigOption, SessionOption, SessionOptions};
    use v1::SessionConfigCategory as C;
    let list = |l: &[v1::SessionOption]| -> Vec<SessionOption> {
        l.iter()
            .map(|o| SessionOption {
                id: o.id.clone(),
                name: o.name.clone(),
                description: o.description.clone(),
                skips_permissions: o.skips_permissions,
            })
            .collect()
    };
    let options = kept
        .options
        .as_ref()
        .map(|o| SessionOptions {
            modes: list(&o.modes),
            models: list(&o.models),
            current_mode: o.current_mode.clone(),
            current_model: o.current_model.clone(),
        })
        .unwrap_or_default();
    let config = kept
        .config
        .as_ref()
        .map(|c| {
            c.options
                .iter()
                .map(|o| ConfigOption {
                    id: o.id.clone(),
                    name: o.name.clone(),
                    description: o.description.clone(),
                    category: match C::try_from(o.category).unwrap_or(C::Unspecified) {
                        C::Mode => "mode",
                        C::Model => "model",
                        C::ModelConfig => "model_config",
                        C::ThoughtLevel => "thought_level",
                        C::Custom => "_custom",
                        C::Unspecified => "",
                    }
                    .to_string(),
                    boolean: o.r#type == v1::SessionConfigType::Boolean as i32,
                    current: o.current.clone(),
                    choices: list(&o.choices),
                    skips_permissions: o.skips_permissions,
                })
                .collect()
        })
        .unwrap_or_default();
    let commands = kept
        .commands
        .as_ref()
        .map(|c| {
            c.commands
                .iter()
                .map(|c| AdapterCommand {
                    name: c.name.clone(),
                    description: c.description.clone(),
                    input_hint: c.input_hint.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    (options, config, commands)
}

/// #925: a time before an agent taken up from the holder started, as the earlier node recorded it (`Entry::process_since_ms`, 0: unknown).
fn held_since(ms: u64) -> Option<SystemTime> {
    (ms != 0).then(|| UNIX_EPOCH + Duration::from_millis(ms))
}

/// `t` in ms since the epoch, rounded up: never earlier than `t`, so a bound stays a bound.
fn ms_after(t: SystemTime) -> u64 {
    let d = t.duration_since(UNIX_EPOCH).unwrap_or_default();
    u64::try_from(d.as_nanos().div_ceil(1_000_000)).unwrap_or(u64::MAX)
}

/// ADR-0060: writes the process of every agent whose pid is new to `recorded` into the ledger, and remembers it only once the ledger
/// has taken it. Two things make a write not happen yet, and both are tried again on the next tick: the process cannot be read (it
/// is just starting), and the ledger has no entry for the agent (the supervisor starts the process and releases its lock before the
/// node records `AgentStarted`, which makes the entry: a tick between the two used to remember the pid as written, so the ledger
/// never got it and a survivor of a crashed node was not recognised; seen on slow macOS runners).
/// #925: `starts` holds, per agent, a time before its process was spawned (ms, rounded up): written with the process in one write, so a
/// node that takes the agent up from the holder never finds a later time recorded as its start. An agent not in it is recorded with
/// none (0).
fn record_new_processes(
    ledger: &crate::ledger::Ledger,
    recorded: &mut std::collections::HashMap<AgentId, u32>,
    pids: &[(AgentId, u32)],
    starts: &std::collections::HashMap<AgentId, u64>,
    identity: impl Fn(u32) -> io::Result<agora_platform::ProcessIdentity>,
    now_ms: u64,
) {
    for (agent, pid) in pids {
        if recorded.get(agent) != Some(pid) {
            if let Ok(identity) = identity(*pid) {
                let since = starts.get(agent).copied().unwrap_or(0);
                if ledger.set_process(agent.as_str(), Some(identity), since, now_ms) {
                    recorded.insert(agent.clone(), *pid);
                }
            }
        }
    }
    recorded.retain(|a, _| pids.iter().any(|(p, _)| p == a));
}

#[cfg(test)]
#[allow(clippy::expect_used)] // expect is allowed in tests
mod memory_tests {
    use super::{agents_memory, memory_moved};

    #[test]
    fn memory_is_reported_again_only_when_it_moved_a_lot() {
        assert!(!memory_moved(0, 0));
        assert!(
            memory_moved(0, 1 << 20),
            "the first measurement after nothing"
        );
        let gib = 1u64 << 30;
        assert!(!memory_moved(gib, gib + (gib / 5)), "20% up");
        assert!(memory_moved(gib, gib + (gib / 3)), "33% up");
        assert!(memory_moved(gib, gib / 2), "half down");
        let big = 8 * gib;
        assert!(memory_moved(big, big + (300 << 20)), "300 MiB on 8 GiB");
        assert!(!memory_moved(big, big + (200 << 20)), "200 MiB on 8 GiB");
    }

    #[test]
    fn an_agents_memory_counts_its_whole_process_tree() {
        // A shell with a child below it, as an agent with the tools it started. On Windows a native one: Git for Windows' `sh`
        // starts `sleep` through a helper that exits, so `sleep`'s parent is gone and no tree leads to it.
        let mut shell = if std::env::consts::OS == "windows" {
            std::process::Command::new("cmd")
                .args(["/c", "ping -n 30 127.0.0.1 >nul"])
                .spawn()
        } else {
            std::process::Command::new("sh")
                .args(["-c", "sleep 30 & wait"])
                .spawn()
        }
        .expect("spawn");
        let pid = shell.id();
        // The child shows up once the shell has started it, which takes a while on a busy machine (windows' fix).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let tree = loop {
            let tree = agora_platform::process_tree(pid).expect("tree");
            if tree.len() >= 2 || std::time::Instant::now() > deadline {
                break tree;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        };
        let measured = agents_memory(&[pid]);
        // Counted once, however often it is named.
        let again = agents_memory(&[pid, pid]);
        let own = agora_platform::process_stats(pid)
            .map(|s| s.rss_bytes)
            .expect("stats");
        let _ = shell.kill();
        let _ = shell.wait();
        assert!(tree.len() >= 2, "the shell and its child: {tree:?}");
        assert!(
            measured > own,
            "the child is counted too: {measured} vs {own}"
        );
        assert!(
            again.abs_diff(measured) < measured / 2,
            "{again} vs {measured}"
        );
        assert_eq!(
            agents_memory(&[u32::MAX - 1]),
            0,
            "a process that does not exist"
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod announce_tests {
    use super::*;

    fn sink(name: &str) -> EventSink {
        let dir =
            std::env::temp_dir().join(format!("agora-announce-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        EventSink::open(&dir, "n1").unwrap()
    }

    fn view(id: &str, running: bool, terminal: bool) -> AgentView {
        AgentView {
            launch_settings: Vec::new(),
            directory: false,
            terminal: terminal && running,
            level: agora_core::IntegrationLevel::L1Manifest,
            id: AgentId::new(id),
            kind: "probe".into(),
            state: AgentState::Idle,
            reported: false,
            worktree: PathBuf::from(format!("/wt/{id}")),
            branch: format!("agent/{id}"),
            project: "p".into(),
            running,
            restarts: 0,
            run_id: None,
            parent: None,
            launch_options: Vec::new(),
            skips_permissions: false,
            runs_inside: None,
        }
    }

    fn said(s: &EventSink, mark: u64) -> Vec<Body> {
        s.events_after(mark)
            .into_iter()
            .filter_map(|e| e.body)
            .collect()
    }

    fn launched(s: &EventSink, id: &str, terminal: bool) {
        s.agent_launched(
            &AgentId::new(id),
            "p",
            "n1",
            Path::new("/work/p"),
            Path::new(&format!("/wt/{id}")),
            &format!("agent/{id}"),
            "probe",
            terminal,
        );
    }

    /// #1150 (review of #1216, N3): a request decided while an announcement runs (after it read what to say, before it records) is
    /// never stated again after its `ApprovalResolved`, whatever kind it is; a request still waiting is stated. The announcer is the
    /// one the node installs, paused where it has read the supervisor and the ledger.
    #[test]
    fn a_request_decided_during_an_announcement_is_not_restated_after_its_decision() {
        use std::sync::mpsc;
        let dir = std::env::temp_dir().join(format!("agora-announce-{}-race", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sink = Arc::new(EventSink::open(&dir.join("events"), "n1").unwrap());
        let sup = Arc::new(Mutex::new(Supervisor::new(SupervisorConfig {
            node: NodeId::new("n1"),
            restart: RestartPolicy::default(),
            terminal: TerminalOptions::default(),
            limits: crate::supervisor::NodeLimits::default(),
        })));
        let approvals = Arc::new(crate::approvals::NodeApprovals::new(
            "n1",
            Box::new(crate::approvals::DenyAll),
        ));
        let ledger = Arc::new(crate::ledger::Ledger::open(&dir, 1).unwrap());
        let agent = AgentId::new("agent-1");
        let decided = approvals
            .request(&agent, "a-1", "shell.exec", "rm -rf build", now_ms())
            .unwrap();
        let waits = approvals
            .request(&agent, "a-2", "file.write", "notes.md", now_ms())
            .unwrap();
        sink.approval_requested(&decided);
        sink.approval_requested(&waits);
        let (reached_tx, reached_rx) = mpsc::channel();
        let (go_tx, go_rx) = mpsc::channel::<()>();
        let go_rx = Mutex::new(go_rx);
        sink.set_announcer(node_announcer(
            "n1".into(),
            sup,
            Arc::clone(&approvals),
            ledger,
            Vec::new(),
            Box::new(move || {
                let _ = reached_tx.send(());
                let _ = go_rx.lock().unwrap().recv();
            }),
        ));
        let mark = sink.next_seq() - 1;
        let announcing = {
            let sink = Arc::clone(&sink);
            std::thread::spawn(move || sink.announce())
        };
        reached_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the announcement reached its pause");
        // A decision while the announcement is paused: its request leaves the queue, then its resolution is recorded.
        assert!(approvals.take_pending(&decided.id));
        sink.approval_resolved(&decided.id, true, "dev-1");
        go_tx.send(()).unwrap();
        announcing.join().unwrap();
        let said = said(&sink, mark);
        let resolved_at = said
            .iter()
            .position(|b| matches!(b, Body::ApprovalResolved(r) if r.approval_id == decided.id))
            .expect("the decision is recorded");
        let restated: Vec<usize> = said
            .iter()
            .enumerate()
            .filter(|(_, b)| matches!(b, Body::ApprovalRequested(r) if r.approval_id == decided.id))
            .map(|(i, _)| i)
            .collect();
        assert!(
            restated.iter().all(|i| *i < resolved_at),
            "the decided request was stated again after its decision: {said:?}"
        );
        assert!(
            said.iter()
                .any(|b| matches!(b, Body::ApprovalRequested(r) if r.approval_id == waits.id)),
            "the request still waiting is stated: {said:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An agent taken up from the holder, or said again by an announcement, is stated with where its state came from.
    #[test]
    fn a_restated_agent_says_where_its_state_came_from() {
        let s = sink("reported");
        let mut own = view("own", true, false);
        own.reported = true;
        let mut seen = view("seen", true, true);
        seen.state = AgentState::Working;
        let mark = s.next_seq() - 1;
        s.agent_taken_up(&own, None);
        s.announce_present(&[seen], &[], Announce::Present);
        let states: Vec<(String, bool)> = said(&s, mark)
            .into_iter()
            .filter_map(|b| match b {
                Body::AgentStateChanged(c) => Some((c.agent_id, c.reported)),
                _ => None,
            })
            .collect();
        assert_eq!(states, [("own".into(), true), ("seen".into(), false)]);
    }

    /// ADR-0055 section 2: without `NodeOnline` every agent of this process is stated once. An ended one gets `AgentExited` with
    /// the last exit code recorded for it (the stop convention `i32::MIN` when none was) and `TerminalClosed` if it had a terminal;
    /// a running one is announced as before. With `NodeOnline` (ADR-0054) ended agents are not mentioned.
    #[test]
    fn ended_agents_are_stated_with_their_recorded_exit_and_terminal() {
        let s = sink("ended");
        launched(&s, "crashed", true);
        launched(&s, "stopped", true);
        launched(&s, "chat", false);
        launched(&s, "kept", true);
        let views = [
            view("crashed", false, true),
            view("stopped", false, true),
            view("chat", false, false),
            view("never", false, false),
            view("kept", true, true),
        ];
        s.supervisor_events(
            &[
                SupervisorEvent::Exited {
                    agent: AgentId::new("crashed"),
                    status: agora_platform::ExitStatus {
                        code: Some(3),
                        signal: None,
                    },
                },
                SupervisorEvent::Exited {
                    agent: AgentId::new("chat"),
                    status: agora_platform::ExitStatus {
                        code: None,
                        signal: Some(9),
                    },
                },
                SupervisorEvent::Stopped {
                    agent: AgentId::new("stopped"),
                },
            ],
            &views,
        );
        let mark = s.next_seq() - 1;
        s.announce_present(&views, &[], Announce::Present);
        let present = said(&s, mark);
        let exits: Vec<(&str, i32)> = present
            .iter()
            .filter_map(|b| match b {
                Body::AgentExited(x) => Some((x.agent_id.as_str(), x.exit_code)),
                _ => None,
            })
            .collect();
        assert_eq!(
            exits,
            [
                ("crashed", 3),
                ("stopped", i32::MIN),
                ("chat", -9),
                ("never", i32::MIN)
            ]
        );
        let closed: Vec<&str> = present
            .iter()
            .filter_map(|b| match b {
                Body::TerminalClosed(t) => Some(t.terminal_id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            closed,
            ["t-crashed", "t-stopped"],
            "only those with a terminal"
        );
        let started: Vec<&str> = present
            .iter()
            .filter_map(|b| match b {
                Body::AgentStarted(a) => Some(a.agent_id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(started, ["kept"]);
        assert!(!present.iter().any(|b| matches!(b, Body::NodeOnline(_))));

        let mark = s.next_seq() - 1;
        s.announce_present(&views, &[], Announce::Restart);
        assert!(!said(&s, mark)
            .iter()
            .any(|b| matches!(b, Body::AgentExited(_) | Body::TerminalClosed(_))));
    }

    /// ADR-0104 decision 2 (#930): `AgentStarted.same_process` is set where the node re-states a process it announced before under the
    /// id (a take-up from the holder, an announcement of either kind) and never for a new start or a resume, whose counters start
    /// again.
    #[test]
    fn only_a_restatement_of_the_same_process_says_so() {
        let s = sink("same-process");
        let starts = |mark: u64| -> Vec<(String, bool)> {
            said(&s, mark)
                .into_iter()
                .filter_map(|b| match b {
                    Body::AgentStarted(a) => Some((a.agent_id, a.same_process)),
                    _ => None,
                })
                .collect()
        };
        let views = [view("a", true, false), view("b", true, false)];
        let mark = s.next_seq() - 1;
        launched(&s, "a", false);
        s.agent_started(&AgentId::new("b"), "p", "n1", "/wt/b", "probe");
        s.supervisor_events(
            &[
                SupervisorEvent::Started {
                    agent: AgentId::new("a"),
                },
                SupervisorEvent::Resumed {
                    agent: AgentId::new("b"),
                    native: true,
                },
            ],
            &views,
        );
        assert!(
            starts(mark).iter().all(|(_, same)| !same),
            "{:?}",
            starts(mark)
        );

        let mark = s.next_seq() - 1;
        s.agent_taken_up(&views[0], None);
        assert_eq!(starts(mark), [("a".to_string(), true)]);
        for how in [Announce::Present, Announce::Restart] {
            let mark = s.next_seq() - 1;
            s.announce_present(&views, &[], how);
            assert_eq!(
                starts(mark),
                [("a".to_string(), true), ("b".to_string(), true)]
            );
        }
    }

    /// #902: an adapter's agent the node gave up on is reported as exited without a `TerminalClosed` (it never had a terminal); a terminal
    /// agent that just ended (its view no longer has a terminal open) still gets one.
    #[test]
    fn a_chat_agent_given_up_on_closes_no_terminal() {
        let s = sink("gave-up-terminal");
        let mut chat = view("chat", false, false);
        chat.level = agora_core::IntegrationLevel::L3Adapter;
        let views = [chat, view("term", false, true)];
        let mark = s.next_seq() - 1;
        let status = agora_platform::ExitStatus {
            code: Some(137),
            signal: None,
        };
        s.supervisor_events(
            &[
                SupervisorEvent::Exited {
                    agent: AgentId::new("chat"),
                    status,
                },
                SupervisorEvent::Exited {
                    agent: AgentId::new("term"),
                    status,
                },
            ],
            &views,
        );
        let got: Vec<String> = said(&s, mark)
            .iter()
            .filter_map(|b| match b {
                Body::AgentExited(x) => Some(format!("exited {} {}", x.agent_id, x.exit_code)),
                Body::TerminalClosed(t) => Some(format!("closed {}", t.terminal_id)),
                _ => None,
            })
            .collect();
        assert_eq!(got, ["exited chat 137", "exited term 137", "closed t-term"]);
    }

    /// #902 (RM-13): the node's notices in a chat agent's conversation: a restart with its attempt, the give-up with the restarts and
    /// the exit code only when it is known; nothing for a terminal agent or for another event.
    #[test]
    fn a_chat_agent_hears_of_its_restarts_and_of_the_give_up() {
        let board = crate::chat::ChatBoard::new();
        let mut chat = view("chat", false, false);
        chat.level = agora_core::IntegrationLevel::L3Adapter;
        chat.restarts = 5;
        let mut acp = view("acp", false, false);
        acp.level = agora_core::IntegrationLevel::L2Acp;
        let views = [chat, acp, view("term", false, true)];
        let id = |a: &str| AgentId::new(a);
        for e in [
            SupervisorEvent::Restarting {
                agent: id("chat"),
                attempt: 2,
                after: std::time::Duration::from_secs(1),
            },
            SupervisorEvent::GaveUp {
                agent: id("chat"),
                exit_code: Some(3),
            },
            SupervisorEvent::GaveUp {
                agent: id("acp"),
                exit_code: None,
            },
            SupervisorEvent::Restarting {
                agent: id("term"),
                attempt: 1,
                after: std::time::Duration::from_secs(1),
            },
            SupervisorEvent::GaveUp {
                agent: id("term"),
                exit_code: Some(1),
            },
            SupervisorEvent::Stopped { agent: id("chat") },
        ] {
            restart_notice(&board, &e, &views, 7);
        }
        let notices = |a: &str| -> Vec<(String, Vec<(String, String)>)> {
            board
                .snapshot(&id(a))
                .into_iter()
                .map(|e| {
                    let mut p: Vec<_> = e.params.into_iter().collect();
                    p.sort();
                    (e.code, p)
                })
                .collect()
        };
        let kv = |k: &str, v: &str| (k.to_string(), v.to_string());
        assert_eq!(
            notices("chat"),
            [
                ("adapter.restarting".to_string(), vec![kv("attempt", "2")]),
                (
                    "adapter.gave_up".to_string(),
                    vec![kv("exit_code", "3"), kv("restarts", "5")]
                ),
            ]
        );
        assert_eq!(
            notices("acp"),
            [("adapter.gave_up".to_string(), vec![kv("restarts", "0")])]
        );
        assert!(notices("term").is_empty());
    }

    /// ADR-0055 section 2: the last 256 resolved approvals are said again, oldest first, so one answered inside the lost window
    /// does not stay listed as waiting. Saying them again does not change what is kept.
    #[test]
    fn the_last_resolved_approvals_are_said_again_and_the_ring_is_bounded() {
        let s = sink("resolved");
        for i in 0..300 {
            s.approval_resolved(&format!("ap-{i}"), i % 2 == 0, "dev");
        }
        let restated = |how| {
            let mark = s.next_seq() - 1;
            s.announce_present(&[], &[], how);
            said(&s, mark)
                .into_iter()
                .filter_map(|b| match b {
                    Body::ApprovalResolved(r) => Some(r),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let first = restated(Announce::Present);
        assert_eq!(first.len(), RESOLVED_KEPT);
        let ids: Vec<String> = first.iter().map(|r| r.approval_id.clone()).collect();
        assert_eq!(
            ids,
            (44..300).map(|i| format!("ap-{i}")).collect::<Vec<_>>()
        );
        assert!(first.iter().all(|r| r.signer_device_id == "dev"
            && r.approved == r.approval_id[3..].parse::<u32>().unwrap().is_multiple_of(2)));
        assert_eq!(restated(Announce::Present), first, "the ring did not move");
        assert!(restated(Announce::Restart).is_empty());
    }

    #[test]
    fn the_nodes_own_binary_is_named_only_when_its_folder_is_not_on_path() {
        // ADR-0069.
        let dir = std::env::temp_dir().join(format!("agora-binpath-{}", std::process::id()));
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let exe = bin.join("agora");
        std::fs::write(&exe, b"").unwrap();
        let other = dir.join("other");
        std::fs::create_dir_all(&other).unwrap();
        let path = |dirs: &[&Path]| std::env::join_paths(dirs).unwrap();
        let off = |p: Option<&std::ffi::OsStr>, cwd: Option<&Path>| super::off_path(&exe, p, cwd);
        assert_eq!(off(Some(&path(&[&other])), None), Some(exe.clone()));
        assert_eq!(off(None, None), Some(exe.clone()));
        assert_eq!(off(Some(&path(&[&other, &bin])), None), None);
        // The same folder spelled another way (`bin/.`) is still on PATH.
        assert_eq!(off(Some(&path(&[&bin.join(".")])), None), None);
        // An empty entry is the current folder (`PATH=:/usr/bin`, the node started from its own folder; review of #598).
        let empty_first = std::env::join_paths([Path::new(""), other.as_path()]).unwrap();
        assert_eq!(off(Some(&empty_first), Some(&bin)), None);
        assert_eq!(off(Some(&empty_first), Some(&other)), Some(exe.clone()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_nodes_private_paths_are_its_whole_home_or_what_is_known_to_be_its() {
        let cfg = |data: &str, worktrees: &str| NodeConfig {
            resume_all: false,
            node_id: "n".into(),
            socket: PathBuf::from("/h/node.sock"),
            data_dir: PathBuf::from(data),
            worktree_root: PathBuf::from(worktrees),
            manifests_dir: PathBuf::from("/h/manifests"),
            projects: vec![],
            limits: NodeLimits::default(),
        };
        // Made absolute as the node makes them (on Windows `/h` is `D:\h`).
        let a = |p: &str| std::path::absolute(p).unwrap();
        // The usual layout: the whole home (its worktrees are opened again per agent by the profile).
        let p = node_private_paths(&cfg("/h/data", "/h/worktrees"));
        assert_eq!(p, [a("/h")]);
        // Worktrees elsewhere, data beside the control socket (second review of #477, 3): still the whole home.
        let p = node_private_paths(&cfg("/h/data", "/w"));
        assert_eq!(p, [a("/h")]);
        // The user's home is never taken for the node's (third review of #477): `--data ~/x --socket ~/y`.
        if let Some(u) = crate::usage_files::home_dir() {
            let mut c = cfg("/x", "/w");
            c.data_dir = u.join("x");
            c.socket = u.join("y");
            let p = node_private_paths(&c);
            assert!(!p.contains(&u), "{p:?}");
            assert!(p.contains(&u.join("x")), "{p:?}");
        }
        // Nor one that holds a project the node serves.
        let mut c = cfg("/h/data", "/h/worktrees");
        c.projects = vec!["/h/repo".into()];
        assert!(!node_private_paths(&c).contains(&a("/h")));
        // A relative home is made absolute (second review of #477, 4), not dropped.
        let rel = node_private_paths(&cfg("h/data", "h/worktrees"));
        assert_eq!(
            rel[0],
            std::env::current_dir().unwrap().join("h"),
            "{rel:?}"
        );
        // Data and worktrees elsewhere: only what is known to be the node's.
        let odd = node_private_paths(&cfg("/srv/agora-data", "/w"));
        for want in ["/srv/agora-data", "/h/node.sock", "/h/m", "/h/manifests"] {
            assert!(odd.contains(&a(want)), "{want}: {odd:?}");
        }
        assert!(!odd.iter().any(|x| x.starts_with(a("/w"))), "{odd:?}");
    }

    #[test]
    fn a_policy_file_that_is_not_understood_forbids_skipping_permissions() {
        use crate::launch_options::SkipPermissions as S;
        let dir = std::env::temp_dir().join(format!("policy-skip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("policy");
        let read = |text: &str| {
            std::fs::write(&file, text).unwrap();
            let p = read_policy(&file);
            (p.skip_permissions, p.problems)
        };
        // No file: the maintainer's default.
        assert_eq!(read_policy(&dir.join("absent")).skip_permissions, S::Allow);
        // #1133: the registry line; anything but `on` or `off` switches the registry's kinds off and is a problem.
        let reg = |text: &str| {
            let f = dir.join("registry-policy");
            std::fs::write(&f, text).unwrap();
            read_policy(&f)
        };
        assert!(!read_policy(&dir.join("absent")).registry.off);
        // ADR-0112: `mesh.fleet` is on unless the line says exactly `true`; a typo turns it off, like `mesh.remote`.
        assert!(read_policy(&dir.join("absent")).mesh_fleet);
        assert!(reg("mesh.fleet = true\n").mesh_fleet);
        assert!(!reg("mesh.fleet = false\n").mesh_fleet);
        let fleet_typo = reg("mesh.fleet = flase\n");
        assert!(!fleet_typo.mesh_fleet && fleet_typo.problems.is_empty());
        assert!(reg("agents.registry = off\n").registry.off);
        assert!(!reg("agents.registry = on\n").registry.off);
        let typo = reg("agents.registry = of\n");
        assert!(typo.registry.off && !typo.problems.is_empty());
        let deny = reg("agents.registry.deny = registry-gemini, registry-goose\n").registry;
        assert!(!deny.off);
        assert_eq!(deny.deny.len(), 2);
        // #1133 review F4: quotes are taken off, a second line adds to the first, and a name that is no registry kind is a problem.
        let quoted = reg("agents.registry.deny = \"registry-gemini\"\nagents.registry.deny = 'registry-goose'\nagents.registry = \"off\"\n");
        assert!(quoted.problems.is_empty(), "{:?}", quoted.problems);
        assert!(quoted.registry.off);
        assert_eq!(
            quoted
                .registry
                .deny
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["registry-gemini", "registry-goose"]
        );
        let unknown = reg("agents.registry.deny = registry-gemni\n");
        assert_eq!(unknown.problems.len(), 1, "{:?}", unknown.problems);
        assert!(unknown.registry.deny.contains("registry-gemni"));
        assert_eq!(read("mesh.remote = true\n"), (S::Allow, vec![]));
        assert_eq!(read("skip_permissions = \"forbid\"\n").0, S::Forbid);
        assert_eq!(
            read("skip_permissions = \"confined_only\"\n").0,
            S::ConfinedOnly
        );
        assert_eq!(read("skip_permissions = \"allow\"\n").0, S::Allow);
        // A typo'd key, a line without `=`: forbid, and said.
        assert_eq!(
            read("skip_permission = forbid\n"),
            (S::Forbid, vec!["skip_permission".to_string()])
        );
        assert_eq!(read("skip_permissions allow\n").0, S::Forbid);
        assert_eq!(
            read("skip_permissions = allow\nsandbx = worktree\n").0,
            S::Forbid
        );
        // An existing file that cannot be read (here: a folder): forbid.
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir_all(&file).unwrap();
        let p = read_policy(&file);
        assert_eq!(
            (p.skip_permissions, p.problems),
            (S::Forbid, vec!["unreadable".to_string()])
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ADR-0089 decision 7: the transcripts folder is among the paths the node protects for every confined agent (a confined agent
    /// that tries to read one is in `tests/sandboxed_agent.rs`).
    #[test]
    fn the_transcripts_folder_is_among_the_paths_confinement_protects() {
        // Built from the (absolute) temporary folder, so the same on every OS: a data folder in the node's own home, and one elsewhere.
        let tmp = std::env::temp_dir().join(format!("agora-test-private-{}", std::process::id()));
        let home = tmp.join("agora-home");
        for data in [home.join("data"), tmp.join("elsewhere").join("data")] {
            let cfg = NodeConfig {
                resume_all: false,
                node_id: "n".into(),
                socket: home.join("run").join("n.sock"),
                data_dir: data.clone(),
                worktree_root: home.join("worktrees"),
                manifests_dir: home.join("manifests"),
                projects: vec![],
                limits: crate::supervisor::NodeLimits::default(),
            };
            // `node_private_paths` makes every path absolute, so compare the same way.
            let transcripts =
                std::path::absolute(data.join(crate::transcript::TRANSCRIPTS_DIR)).unwrap();
            assert!(
                node_private_paths(&cfg)
                    .iter()
                    .any(|p| transcripts.starts_with(p)),
                "{}",
                data.display()
            );
        }
    }

    /// ADR-0089 decision 2: the history bounds come from the policy, never past their caps; a value that is not a number keeps the
    /// default and is a line the node did not understand.
    #[test]
    fn read_policy_bounds_the_chat_history() {
        let dir =
            std::env::temp_dir().join(format!("agora-test-history-policy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("policy");
        assert_eq!(
            read_policy(&file).history,
            crate::transcript::Limits::default()
        );
        std::fs::write(
            &file,
            "chat.history_max_bytes = 1048576
chat.history_days = 7
",
        )
        .unwrap();
        let p = read_policy(&file);
        assert_eq!((p.history.max_bytes, p.history.days), (1 << 20, 7));
        assert!(p.problems.is_empty());
        std::fs::write(
            &file,
            "chat.history_max_bytes = 99999999999
chat.history_days = 365
",
        )
        .unwrap();
        let p = read_policy(&file);
        assert_eq!(
            (p.history.max_bytes, p.history.days),
            (
                crate::transcript::DEFAULT_MAX_BYTES,
                crate::transcript::DEFAULT_DAYS
            )
        );
        std::fs::write(
            &file,
            "chat.history_days = a week
",
        )
        .unwrap();
        let p = read_policy(&file);
        assert_eq!(p.history.days, crate::transcript::DEFAULT_DAYS);
        assert_eq!(p.problems.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_policy_handles_overlapping_device_projects_key_without_panicking() {
        let dir =
            std::env::temp_dir().join(format!("agora-test-device-projects-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("policy");
        std::fs::write(
            &file,
            "device.projects = p1\ndevice..projects = p2\ndevice.dev1.projects = p1, p2\n",
        )
        .unwrap();
        let p = read_policy(&file);
        assert_eq!(p.problems, vec!["device.projects".to_string()]);
        assert_eq!(
            p.skip_permissions,
            crate::launch_options::SkipPermissions::Forbid
        );
        assert!(!p.device_projects.contains_key(""));
        assert!(p
            .device_projects
            .get("dev1")
            .is_some_and(|s| s.contains("p1") && s.contains("p2") && s.len() == 2));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod process_record_tests {
    use super::record_new_processes;
    use agora_core::AgentId;
    use agora_protocol::v1::{self, event::Body};
    use std::collections::HashMap;

    fn identity(pid: u32) -> std::io::Result<agora_platform::ProcessIdentity> {
        Ok(agora_platform::ProcessIdentity {
            pid,
            group: pid,
            started: 1_700_000_000_000_000,
            boot: String::new(),
            name: "fake".into(),
        })
    }

    /// The supervisor starts a process before the node records `AgentStarted` (which makes the ledger entry). A tick in between must
    /// not remember the pid as written: the next tick has to write it once the entry exists.
    #[test]
    fn a_tick_before_the_agent_is_in_the_ledger_is_tried_again() {
        let dir = std::env::temp_dir().join(format!("agora-procrec-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ledger = crate::ledger::Ledger::open(&dir, 1).unwrap();
        let agent = AgentId::new("agent-1");
        let pids = vec![(agent.clone(), 4242)];
        let mut recorded = HashMap::new();

        record_new_processes(&ledger, &mut recorded, &pids, &HashMap::new(), identity, 2);
        assert!(ledger.get("agent-1").is_none());
        assert!(
            recorded.is_empty(),
            "a pid the ledger did not take was remembered"
        );

        ledger.observe(
            &Body::AgentStarted(v1::AgentStarted {
                agent_id: "agent-1".into(),
                project_id: "p".into(),
                node_id: "n".into(),
                worktree_id: "w".into(),
                kind: "fake".into(),
                level: 2,
                ..Default::default()
            }),
            3,
        );
        record_new_processes(&ledger, &mut recorded, &pids, &HashMap::new(), identity, 4);
        assert_eq!(ledger.get("agent-1").unwrap().process().unwrap().pid, 4242);
        assert_eq!(recorded.get(&agent), Some(&4242));

        // Written once: a later tick with the same pid reads nothing again.
        record_new_processes(
            &ledger,
            &mut recorded,
            &pids,
            &HashMap::new(),
            |_| panic!("read again"),
            5,
        );
        // An agent that is gone is forgotten, so a new process of it is written.
        record_new_processes(&ledger, &mut recorded, &[], &HashMap::new(), identity, 6);
        assert!(recorded.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #925: the time before a process was spawned is written with the process, in one ledger write, so a node that ends right after it
    /// leaves the bound for the node that takes the agent up from the holder, never a later time. A process whose start the node does
    /// not know is recorded with none, not with the time it was seen; the next node's take-up does not change it.
    #[test]
    fn a_processs_start_bound_is_written_with_the_process_and_outlives_the_node() {
        use super::{held_since, ms_after};
        use std::time::{Duration, UNIX_EPOCH};
        let dir = std::env::temp_dir().join(format!("agora-procsince-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let started = UNIX_EPOCH + Duration::from_nanos(1_700_000_000_123_400_001);
        assert_eq!(ms_after(started), 1_700_000_000_124);
        assert!(held_since(ms_after(started)).unwrap() >= started);
        assert_eq!(held_since(0), None);
        let (known, unknown) = (AgentId::new("agent-1"), AgentId::new("agent-2"));
        let pids = vec![(known.clone(), 4242), (unknown.clone(), 4343)];
        let starts: HashMap<AgentId, u64> = [(known.clone(), ms_after(started))].into();
        {
            let ledger = crate::ledger::Ledger::open(&dir, 1).unwrap();
            for id in ["agent-1", "agent-2"] {
                ledger.observe(
                    &Body::AgentStarted(v1::AgentStarted {
                        agent_id: id.into(),
                        kind: "fake".into(),
                        level: 2,
                        ..Default::default()
                    }),
                    3,
                );
            }
            let mut recorded = HashMap::new();
            record_new_processes(
                &ledger,
                &mut recorded,
                &pids,
                &starts,
                identity,
                1_700_000_000_500,
            );
            // The node ends here, right after the one write.
        }
        let ledger = crate::ledger::Ledger::open(&dir, 1).unwrap();
        let since = |id: &str| ledger.get(id).unwrap().process_since_ms;
        assert_eq!(since("agent-1"), 1_700_000_000_124);
        assert_eq!(
            since("agent-2"),
            0,
            "a time the process was only seen at was kept as its start"
        );
        // The next node takes the same processes up: what was recorded stays.
        let mut recorded = HashMap::new();
        record_new_processes(
            &ledger,
            &mut recorded,
            &pids,
            &HashMap::new(),
            identity,
            1_700_000_009_000,
        );
        assert_eq!((since("agent-1"), since("agent-2")), (1_700_000_000_124, 0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A process that cannot be read yet (it is just starting) is tried again too.
    #[test]
    fn a_process_that_cannot_be_read_yet_is_tried_again() {
        let dir = std::env::temp_dir().join(format!("agora-procrec2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ledger = crate::ledger::Ledger::open(&dir, 1).unwrap();
        let pids = vec![(AgentId::new("agent-1"), 7)];
        let mut recorded = HashMap::new();
        record_new_processes(
            &ledger,
            &mut recorded,
            &pids,
            &HashMap::new(),
            |_| Err(std::io::Error::from(std::io::ErrorKind::NotFound)),
            2,
        );
        assert!(recorded.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F870: an announcement re-states agents from a copy of the supervisor's list taken before the tick recorded their end or their
    /// restart. That stale copy must not change the ledger: a finished agent is not brought back (offered, and by policy resumed with a
    /// "carry on" message, after the node restarts), and a restarted one is not ended (its survivor recognised after a crash).
    #[test]
    fn a_stale_announcement_does_not_change_the_ledger() {
        use super::{Announce, EventSink};
        use crate::supervisor::{AgentView, SupervisorEvent};
        use agora_core::AgentState;
        let view = |running: bool, state: AgentState| AgentView {
            launch_settings: Vec::new(),
            directory: false,
            terminal: true,
            level: agora_core::IntegrationLevel::L1Manifest,
            id: AgentId::new("agent-1"),
            kind: "fake".into(),
            state,
            reported: false,
            worktree: std::path::PathBuf::from("/wt/agent-1"),
            branch: "main".into(),
            project: "p".into(),
            running,
            restarts: 0,
            run_id: None,
            parent: None,
            launch_options: Vec::new(),
            skips_permissions: false,
            runs_inside: None,
        };
        let exited = |code| SupervisorEvent::Exited {
            agent: AgentId::new("agent-1"),
            status: agora_platform::ExitStatus {
                code: Some(code),
                signal: None,
            },
        };
        let started = SupervisorEvent::Started {
            agent: AgentId::new("agent-1"),
        };
        for restarted in [false, true] {
            let dir = std::env::temp_dir().join(format!(
                "agora-procrec-stale-{restarted}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let ledger = std::sync::Arc::new(crate::ledger::Ledger::open(&dir, 1).unwrap());
            let sink = EventSink::open(&dir.join("events"), "n1").unwrap();
            sink.set_ledger(std::sync::Arc::clone(&ledger));
            let up = [view(true, AgentState::Working)];
            sink.supervisor_events(std::slice::from_ref(&started), &up);
            // The tick writes each new process the way it does in the node (#696).
            let mut recorded = HashMap::new();
            let pid = |p: u32| [(AgentId::new("agent-1"), p)];
            record_new_processes(
                &ledger,
                &mut recorded,
                &pid(4242),
                &HashMap::new(),
                identity,
                2,
            );
            let copy = if restarted {
                // Crashed, restart pending: copied now; the tick then restarts it with a new process.
                let down = [view(false, AgentState::Unknown)];
                sink.supervisor_events(&[exited(1)], &down);
                record_new_processes(
                    &ledger,
                    &mut recorded,
                    &pid(5151),
                    &HashMap::new(),
                    identity,
                    3,
                );
                sink.supervisor_events(std::slice::from_ref(&started), &up);
                down.to_vec()
            } else {
                // Copied while it runs; the tick then records that it finished.
                let done = SupervisorEvent::StateChanged {
                    agent: AgentId::new("agent-1"),
                    state: AgentState::Done,
                    reported: false,
                };
                sink.supervisor_events(&[exited(0), done], &[view(false, AgentState::Done)]);
                up.to_vec()
            };
            sink.announce_present(&copy, &[], Announce::Present);
            ledger.flush().unwrap();
            drop((sink, ledger));
            let e = crate::ledger::Ledger::open(&dir, 10)
                .unwrap()
                .get("agent-1")
                .unwrap();
            if restarted {
                assert_eq!(e.process().map(|p| p.pid), Some(5151));
                assert!(
                    e.interrupted(),
                    "a restarted agent was ended by a stale copy"
                );
            } else {
                assert!(
                    !e.interrupted(),
                    "a finished agent was brought back by a stale copy"
                );
                assert_eq!(e.last_state, "done");
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

/// #1147 (audit of #1157): the agent's own words an event carries reach the hub with the node's secret values replaced.
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod event_redaction_tests {
    use super::EventSink;
    use agora_protocol::v1::{self, event::Body};
    use std::sync::Arc;

    #[test]
    fn the_agents_words_in_an_event_hold_no_piece_of_a_secret() {
        let value = "SK-EVENT-0123456789-ABCDEFGHIJ";
        let dir = std::env::temp_dir().join(format!("agora-ev-redact-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        crate::credentials::SecretStore::in_data(&dir)
            .set("T", &crate::credentials::Secret::new(value).unwrap())
            .unwrap();
        let store = Arc::new(crate::transcript::Store::new(
            &dir,
            crate::transcript::Limits::default(),
            Some(crate::credentials::SecretStore::in_data(&dir)),
        ));
        let sink = EventSink::open(&dir.join("events"), "n1").unwrap();
        sink.set_secrets(store);
        let opt = |s: &str| v1::SessionOption {
            id: "plan".into(),
            name: format!("name {s}"),
            description: format!("{s} more"),
            skips_permissions: false,
        };
        // Whole, and cut at a text's end (the first 12 bytes of the value only).
        for s in [value, &value[..12]] {
            sink.record_bodies(vec![
                Body::MessagePosted(v1::MessagePosted {
                    message_id: "m1".into(),
                    from_agent_id: "a".into(),
                    to: "b".into(),
                    body: format!("use {s}"),
                }),
                Body::TaskChanged(v1::TaskChanged {
                    task_id: "t1".into(),
                    title: format!("{s} title"),
                    ..Default::default()
                }),
                Body::HandoffCreated(v1::HandoffCreated {
                    summary: format!("x {s}"),
                    refs: vec![format!("src/{s}")],
                    ..Default::default()
                }),
                Body::AgentSessionOptions(v1::AgentSessionOptions {
                    agent_id: "a".into(),
                    modes: vec![opt(s)],
                    models: vec![opt(s)],
                    current_mode: "plan".into(),
                    current_model: "plan".into(),
                }),
                Body::AgentSessionConfig(v1::AgentSessionConfig {
                    agent_id: "a".into(),
                    options: vec![v1::SessionConfigOption {
                        id: "o".into(),
                        name: format!("name {s}"),
                        description: format!("{s} d"),
                        choices: vec![opt(s)],
                        ..Default::default()
                    }],
                }),
                Body::AgentCommands(v1::AgentCommands {
                    agent_id: "a".into(),
                    commands: vec![v1::AgentCommand {
                        name: "c".into(),
                        description: format!("does {s}"),
                        input_hint: format!("{s} hint"),
                    }],
                }),
            ]);
        }
        let events = sink.events_after(0);
        assert!(events.len() >= 12);
        // Only the bodies carry the agent's words: the envelope's time and sequence are digits that can hold a piece of the
        // value by chance (`at_ms` ending in 2345 failed this test once).
        let bodies: Vec<_> = events.iter().map(|e| &e.body).collect();
        let text = format!("{bodies:?}");
        for i in 0..=value.len() - 4 {
            assert!(
                !text.contains(&value[i..i + 4]),
                "{:?} in {text}",
                &value[i..i + 4]
            );
        }
        // Ids are answered back: kept as they are.
        assert!(text.contains("\"plan\""));
        drop(sink);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
