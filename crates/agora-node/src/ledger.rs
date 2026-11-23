//! The node's session ledger (ADR-0060): what it must remember about its agents across its own restart, so an agent that was
//! running when the node stopped can be offered again (Resume, or Start again in the same folder).
//!
//! One file, `<data>/sessions/ledger.json`, readable by the owner only (file `0600` in a `0700` folder on Unix, an owner-only ACL on
//! Windows, both through `agora-platform`), replaced whole on every write (a temporary file in the same folder, then a rename). It holds
//! per agent: id, kind, project, folder (its workspace or its own worktree), parent agent, the agent's own session id when one was
//! reported, start time, last state, and whether it ended. Never terminal contents, prompts, or secrets. Beside it, `kept-<id>.pb` holds
//! what an adapter agent last reported of its session (modes and models, config options, commands, as the node recorded them), for a
//! node that takes the agent up from the holder (ADR-0093, #932); removed when the agent ends, restarts, or is not taken up.
//!
//! The ledger learns from the events the node records (see [`Ledger::observe`]), so every way an agent starts, reports its session,
//! or ends is covered without a call at each place. It writes when an agent starts, when its session id is reported, and when it
//! ends; a state change is kept in memory and written with the next write (and when the node stops cleanly, [`Ledger::flush`]).
//! An agent the node stops with itself does not end: that is exactly what makes it interrupted at the next start.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use agora_protocol::v1::{self, event::Body};
use serde_json::{json, Value};

/// The ledger's folder in the node's data folder.
pub const LEDGER_DIR: &str = "sessions";
/// The ledger file in [`LEDGER_DIR`].
pub const LEDGER_FILE: &str = "ledger.json";
/// How long an interrupted agent is offered, and an ended one kept (ADR-0060: 14 days).
pub const KEEP_MS: u64 = 14 * 24 * 60 * 60 * 1000;
/// Most entries kept; the oldest ended ones go first.
pub const MAX_ENTRIES: usize = 512;
/// A larger file is not read (it was not written by this ledger).
const MAX_FILE_BYTES: u64 = 4 << 20;
/// Longest text kept in one field; a longer one is not something the node wrote and is dropped.
const MAX_FIELD: usize = 4096;
/// Folders remembered between `WorkspaceRegistered` and the `AgentStarted` that uses them.
const MAX_WORKSPACES: usize = 1024;
const VERSION: u64 = 1;
/// Session ids held for agents whose start is not recorded yet.
const MAX_EARLY: usize = 64;

/// ADR-0093: what is kept of an adapter's session id and capabilities, on write and on read alike.
const ADAPTER_SESSION_MAX: usize = 256;
const ADAPTER_CAPS_MAX: usize = 64;
const ADAPTER_CAP_MAX: usize = 64;

/// One agent in the ledger.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Entry {
    pub agent_id: String,
    pub kind: String,
    pub project_id: String,
    /// The folder it works in: its workspace path (ADR-0052) or its own worktree.
    pub folder: String,
    /// Started in a selected folder (ADR-0052) rather than in a worktree the node made for it (`AgentStart`).
    pub directory: bool,
    /// For a directory agent: the folder is a separate worktree the node made, and its Git root and branch.
    pub managed_worktree: bool,
    pub git_root: String,
    pub branch: String,
    /// ADR-0057: the agent on this node that started it; empty when the user did.
    pub parent_agent_id: String,
    /// The agent's own session id (`claude --resume`, `codex resume`, an ACP session), empty until one is reported.
    pub session_id: String,
    /// ADR-0059's launch options, when that lands; kept for the record and never applied by a resume (a resumed agent asks again).
    pub launch_options: Vec<String>,
    /// ADR-0059: one of `launch_options` made the agent act without asking (the node's word at the launch, from `AgentStarted`). An
    /// agent the holder kept is still that process, so the next node shows it the same way (#907).
    pub skips_permissions: bool,
    pub started_at_ms: u64,
    pub updated_at_ms: u64,
    /// `working`, `blocked`, `idle`, `done` or `unknown`.
    pub last_state: String,
    /// It ended while the node ran (exited, stopped, given up). One that did not was interrupted by the node stopping.
    pub ended: bool,
    /// When the node found it interrupted; 0 while it runs or after it ended.
    pub interrupted_at_ms: u64,
    /// The agent's top process (its terminal's program, or its adapter) while it runs: pid, process group and start time
    /// (`agora_platform::ProcessIdentity`), so that one that outlived a crashed node is recognised, and a reused pid is not. 0: none.
    pub pid: u32,
    pub process_group: u32,
    pub process_started: u64,
    /// The boot it started in (Linux) and its program's name: a process is the recorded one only when both match too.
    pub process_boot: String,
    pub process_name: String,
    /// A time before that process started (ms since the epoch), taken before it was spawned, written with the process
    /// ([`Ledger::set_process`]): what a node that takes the agent up from the holder knows of when it started, for session discovery
    /// (#925). 0: not known (never a time the node only saw it at).
    pub process_since_ms: u64,
    /// ADR-0062 item 4: when the node last brought it back by itself (0: never), so an agent interrupted again soon after (a crash
    /// loop) is not brought back again by itself.
    pub auto_resumed_at_ms: u64,
    /// ADR-0060 Amendment A: when the node last gave this agent its one "carry on" message after bringing it back by itself (0: never).
    /// One for each interruption: it is sent only while this is older than `interrupted_at_ms`.
    pub continue_sent_ms: u64,
    /// ADR-0079 decision 9: the terminal holder whose process the agent is while it runs (the holder's pid and start time); 0: the node's
    /// own. A node that starts and finds that holder alive attaches to the agent instead of reporting it interrupted.
    pub holder_pid: u32,
    pub holder_started: u64,
    /// ADR-0093: for an agent an adapter hosts that runs in the holder: what a node that takes it up later must know and cannot ask
    /// (the adapter's own session id, and the capabilities it declared in `initialize`). Empty otherwise.
    pub adapter_session: String,
    pub adapter_capabilities: Vec<String>,
}

#[derive(Debug, Clone)]
struct Workspace {
    path: String,
    git_root: String,
    branch: String,
    managed: bool,
}

#[derive(Default)]
struct State {
    next_id: u64,
    entries: BTreeMap<String, Entry>,
    workspaces: BTreeMap<String, Workspace>,
    bound: BTreeMap<String, String>,
    /// Session ids reported before their agent's `AgentStarted` was recorded (an adapter reports its session while it starts, and the
    /// node's tick can record that before the start is recorded): applied when the start comes.
    early_sessions: BTreeMap<String, String>,
    /// The same for the last state: a state change can be recorded (by the supervision tick) before the control thread records the
    /// agent's `AgentStarted`, and an idle agent reports no further change, so a dropped one would leave `last_state` empty for good.
    early_states: BTreeMap<String, String>,
    /// ADR-0093 (#932): what each adapter agent last reported of its session, as the node recorded it, in `kept-<id>.pb` beside the
    /// ledger (not in it: a full set of config options is larger than the ledger may be). Filled as the node records, or from the file
    /// when an agent is taken up from the holder.
    kept: BTreeMap<String, KeptSession>,
    /// Agents whose kept session file is not written as `kept` says yet, each with the number of its latest change (written off the
    /// recording thread by one writer, and by `flush_all`). A mark goes only after the file was written or removed, and only when no
    /// newer change came meanwhile: while it is there, `kept` is the truth and the file is not read.
    kept_pending: BTreeMap<String, u64>,
    /// Numbers the changes in `kept_pending`.
    kept_seq: u64,
    /// The writer thread runs (one at a time: changes that come while it runs are written by it).
    kept_writer: bool,
    /// Something changed that is not on disk yet.
    dirty: bool,
}

impl State {
    fn mark_kept_pending(&mut self, agent: String) {
        self.kept_seq += 1;
        self.kept_pending.insert(agent, self.kept_seq);
    }
}

/// ADR-0093 (#932): the last `AgentSessionOptions`, `AgentSessionConfig` and `AgentCommands` the node recorded for an adapter agent. The
/// adapter of an agent the holder kept is not initialised again and does not report them again, so a node that takes it up shows them
/// from here: its modes and settings, and whether one in use makes it act without asking.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct KeptSession {
    pub options: Option<v1::AgentSessionOptions>,
    pub config: Option<v1::AgentSessionConfig>,
    pub commands: Option<v1::AgentCommands>,
    /// ADR-0101: the "always" answers the agent's process holds (`AgentAlwaysAllowed`, at most [`MAX_KEPT_ALWAYS`], newest last), so a
    /// node that takes a kept agent up says them again. Never the agent's own rule: the action, approval id, signer and time.
    pub always: Vec<v1::AgentAlwaysAllowed>,
}

/// ADR-0101: how many "always" answers are kept per agent (the hub shows as many).
pub const MAX_KEPT_ALWAYS: usize = 16;

impl KeptSession {
    /// The events, in the order the node records them.
    pub fn bodies(&self) -> Vec<Body> {
        let mut out = Vec::new();
        if let Some(o) = &self.options {
            out.push(Body::AgentSessionOptions(o.clone()));
        }
        if let Some(c) = &self.config {
            out.push(Body::AgentSessionConfig(c.clone()));
        }
        if let Some(c) = &self.commands {
            out.push(Body::AgentCommands(c.clone()));
        }
        out.extend(self.always.iter().cloned().map(Body::AgentAlwaysAllowed));
        out
    }

    fn encode(&self) -> Vec<u8> {
        use prost::Message;
        let mut out = Vec::new();
        for body in self.bodies() {
            let ev = v1::Event {
                body: Some(body),
                ..Default::default()
            };
            out.extend(ev.encode_length_delimited_to_vec());
        }
        out
    }

    /// What `encode` wrote for `agent`; anything else in the file is left out.
    fn decode(mut bytes: &[u8], agent: &str) -> Self {
        use prost::Message;
        let mut kept = Self::default();
        while !bytes.is_empty() {
            let Ok(ev) = v1::Event::decode_length_delimited(&mut bytes) else {
                break;
            };
            match ev.body {
                Some(Body::AgentSessionOptions(o)) if o.agent_id == agent => kept.options = Some(o),
                Some(Body::AgentSessionConfig(c)) if c.agent_id == agent => kept.config = Some(c),
                Some(Body::AgentCommands(c)) if c.agent_id == agent => kept.commands = Some(c),
                Some(Body::AgentAlwaysAllowed(a))
                    if a.agent_id == agent && kept.always.len() < MAX_KEPT_ALWAYS =>
                {
                    kept.always.push(a);
                }
                _ => {}
            }
        }
        kept
    }
}

/// The largest report kept, encoded: the hub's own bound for a config or command list (`agora-hub` `MAX_CONFIG_BYTES`), so what is
/// put back is never more than a window could have been shown, and a write stays small. A larger report is not kept, and the one before
/// it is dropped, as it was replaced (as on the hub).
const MAX_KEPT_REPORT: usize = 64 * 1024;
/// A kept session file larger than this (three reports and the always answers, ADR-0101) is not read.
const MAX_KEPT_BYTES: u64 = 3 * MAX_KEPT_REPORT as u64 + 64 + MAX_KEPT_ALWAYS as u64 * 1024;

/// The file a kept session of `agent` is written to, in the ledger's folder; `None` for an id that is not a plain node-made id.
fn kept_file(dir: &Path, agent: &str) -> Option<PathBuf> {
    (!agent.is_empty()
        && agent.len() <= 64
        && agent
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
    .then(|| dir.join(format!("kept-{agent}.pb")))
}

fn remove_kept_file(path: &Path) -> bool {
    match std::fs::remove_file(path) {
        Ok(()) => true,
        Err(e) => e.kind() == io::ErrorKind::NotFound,
    }
}

/// Attempts each pending change under the write lock and returns the last change number in the snapshot. A failed write or removal
/// retains its mark for the next batch; a reader cannot recover an old report from a file whose removal failed.
fn write_kept_pending(state: &Mutex<State>, write: &Mutex<()>, dir: Option<&Path>) -> u64 {
    let _writing = write.lock().unwrap_or_else(|e| e.into_inner());
    let lock = || state.lock().unwrap_or_else(|e| e.into_inner());
    let Some(dir) = dir else {
        let mut st = lock();
        st.kept_pending.clear();
        return st.kept_seq;
    };
    let (through, mut batch) = {
        let st = lock();
        let batch: Vec<_> = st
            .kept_pending
            .iter()
            .map(|(a, n)| (a.clone(), *n, st.kept.get(a).map(KeptSession::encode)))
            .collect();
        (st.kept_seq, batch)
    };
    batch.retain(|(agent, _, bytes)| {
        let Some(file) = kept_file(dir, agent) else {
            return true;
        };
        match bytes {
            Some(b) => agora_platform::write_private(&file, b).is_ok(),
            None => remove_kept_file(&file),
        }
    });
    let mut st = lock();
    for (agent, n, _) in batch {
        if st.kept_pending.get(&agent) == Some(&n) {
            st.kept_pending.remove(&agent);
        }
    }
    through
}

/// The writer retries when a newer change arrives while it is working. With no newer change it stops, leaving failures for the next
/// change or a clean flush, so a persistent I/O error does not keep the writer running.
fn kept_writer(state: &Mutex<State>, write: &Mutex<()>, dir: Option<&Path>) {
    loop {
        let through = write_kept_pending(state, write, dir);
        let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
        if st.kept_seq == through {
            st.kept_writer = false;
            return;
        }
    }
}

/// What `body` changes in the kept sessions: the agent whose file must be written again (or removed), if any. A report may be recorded
/// before its agent's `AgentStarted` (the supervision tick records it while the control thread is still recording the start), so it is
/// kept without an entry, up to [`MAX_ENTRIES`] agents.
fn keep(st: &mut State, body: &Body, dir: Option<&Path>) -> Option<String> {
    use prost::Message;
    let room = |st: &State, id: &str| st.kept.contains_key(id) || st.kept.len() < MAX_ENTRIES;
    // What is kept of a report: itself, or nothing when it is too large.
    fn fit<M: Message + Clone>(m: &M) -> Option<M> {
        (m.encoded_len() <= MAX_KEPT_REPORT).then(|| m.clone())
    }
    match body {
        Body::AgentSessionOptions(o) if room(st, &o.agent_id) => {
            let k = st.kept.entry(o.agent_id.clone()).or_default();
            let next = fit(o);
            (k.options != next).then(|| {
                k.options = next;
                o.agent_id.clone()
            })
        }
        Body::AgentSessionConfig(c) if room(st, &c.agent_id) => {
            let k = st.kept.entry(c.agent_id.clone()).or_default();
            let next = fit(c);
            (k.config != next).then(|| {
                k.config = next;
                c.agent_id.clone()
            })
        }
        // ADR-0101: an always answer the agent's process now holds (an announcement's repeat of one is not learned here). Bounded,
        // newest kept, never twice.
        Body::AgentAlwaysAllowed(a) if room(st, &a.agent_id) && a.encoded_len() <= 1024 => {
            let k = st.kept.entry(a.agent_id.clone()).or_default();
            if k.always.iter().any(|x| x.approval_id == a.approval_id) {
                return None;
            }
            if k.always.len() == MAX_KEPT_ALWAYS {
                k.always.remove(0);
            }
            k.always.push(a.clone());
            Some(a.agent_id.clone())
        }
        Body::AgentCommands(c) if room(st, &c.agent_id) => {
            let k = st.kept.entry(c.agent_id.clone()).or_default();
            let next = fit(c);
            (k.commands != next).then(|| {
                k.commands = next;
                c.agent_id.clone()
            })
        }
        // A new process of an agent that runs (a restart after a crash, recorded by the tick before the new process can report)
        // reports its own. A first start keeps what was reported before it; a resume follows an end or an interruption, which already
        // removed the old one, and keeps what the new process may have reported before its start was recorded.
        Body::AgentStarted(v1::AgentStarted { agent_id, .. })
            if st
                .entries
                .get(agent_id)
                .is_some_and(|e| !e.ended && !e.interrupted()) =>
        {
            drop_kept(st, agent_id, dir)
        }
        // An agent that ended or was interrupted has none to keep. The file goes too when only the file has it (a ledger opened by a
        // later node has not read it).
        Body::AgentExited(v1::AgentExited { agent_id, .. })
        | Body::AgentInterrupted(v1::AgentInterrupted { agent_id, .. }) => {
            drop_kept(st, agent_id, dir)
        }
        _ => None,
    }
}

/// Drops what is kept of `agent`; the agent when its file must go too: it was kept, a write of it is pending, or its file is on disk
/// (looked at under the state lock, so no reader can bring the file back in between). An agent with nothing kept anywhere (a terminal
/// agent) needs no writer.
fn drop_kept(st: &mut State, agent: &str, dir: Option<&Path>) -> Option<String> {
    let had = st.kept.remove(agent).is_some() || st.kept_pending.contains_key(agent);
    let on_disk = || {
        dir.and_then(|d| kept_file(d, agent))
            .is_some_and(|f| f.exists())
    };
    (had || on_disk()).then(|| agent.to_string())
}

/// See the module documentation. Every method takes the ledger's own lock alone and never another one while holding it.
pub struct Ledger {
    /// `None`: kept in memory only (tests, a node without a data folder).
    path: Option<PathBuf>,
    state: Arc<Mutex<State>>,
    /// Held through a whole write (snapshot, temporary file, rename): two writers never share the temporary file, and the newer
    /// snapshot is always the one left on disk. Taken before `state`, never after it.
    write: Arc<Mutex<()>>,
}

fn field(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(Value::as_str)
        .filter(|s| s.len() <= MAX_FIELD && !s.contains('\0'))
        .unwrap_or_default()
        .to_string()
}

fn number(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(Value::as_u64).unwrap_or(0)
}

fn flag(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}

fn bounded(s: &str) -> String {
    if s.len() <= MAX_FIELD && !s.contains('\0') {
        s.to_string()
    } else {
        String::new()
    }
}

impl Entry {
    fn to_json(&self) -> Value {
        json!({
            "agent_id": self.agent_id,
            "kind": self.kind,
            "project_id": self.project_id,
            "folder": self.folder,
            "directory": self.directory,
            "managed_worktree": self.managed_worktree,
            "git_root": self.git_root,
            "branch": self.branch,
            "parent_agent_id": self.parent_agent_id,
            "session_id": self.session_id,
            "launch_options": self.launch_options,
            "skips_permissions": self.skips_permissions,
            "started_at_ms": self.started_at_ms,
            "updated_at_ms": self.updated_at_ms,
            "last_state": self.last_state,
            "ended": self.ended,
            "interrupted_at_ms": self.interrupted_at_ms,
            "pid": self.pid,
            "process_group": self.process_group,
            "process_started": self.process_started,
            "process_boot": self.process_boot,
            "process_name": self.process_name,
            "process_since_ms": self.process_since_ms,
            "auto_resumed_at_ms": self.auto_resumed_at_ms,
            "continue_sent_ms": self.continue_sent_ms,
            "holder_pid": self.holder_pid,
            "holder_started": self.holder_started,
            "adapter_session": self.adapter_session,
            "adapter_capabilities": self.adapter_capabilities,
        })
    }

    fn from_json(v: &Value) -> Option<Self> {
        let e = Self {
            agent_id: field(v, "agent_id"),
            kind: field(v, "kind"),
            project_id: field(v, "project_id"),
            folder: field(v, "folder"),
            directory: flag(v, "directory"),
            managed_worktree: flag(v, "managed_worktree"),
            git_root: field(v, "git_root"),
            branch: field(v, "branch"),
            parent_agent_id: field(v, "parent_agent_id"),
            // A file someone else wrote must not put an option on the agent's command line: such an id is dropped and the agent
            // is offered Start again only.
            session_id: Some(field(v, "session_id"))
                .filter(|s| crate::session_files::argv_safe(s))
                .unwrap_or_default(),
            launch_options: v
                .get("launch_options")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .filter(|s| s.len() <= 200)
                        .take(32)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            skips_permissions: flag(v, "skips_permissions"),
            started_at_ms: number(v, "started_at_ms"),
            updated_at_ms: number(v, "updated_at_ms"),
            last_state: field(v, "last_state"),
            ended: flag(v, "ended"),
            interrupted_at_ms: number(v, "interrupted_at_ms"),
            pid: u32::try_from(number(v, "pid")).unwrap_or(0),
            process_group: u32::try_from(number(v, "process_group")).unwrap_or(0),
            process_started: number(v, "process_started"),
            process_boot: field(v, "process_boot"),
            process_name: field(v, "process_name"),
            process_since_ms: number(v, "process_since_ms"),
            auto_resumed_at_ms: number(v, "auto_resumed_at_ms"),
            continue_sent_ms: number(v, "continue_sent_ms"),
            holder_pid: u32::try_from(number(v, "holder_pid")).unwrap_or(0),
            holder_started: number(v, "holder_started"),
            adapter_session: Some(field(v, "adapter_session"))
                .filter(|s| s.len() <= ADAPTER_SESSION_MAX)
                .unwrap_or_default(),
            adapter_capabilities: v
                .get("adapter_capabilities")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .filter(|s| s.len() <= ADAPTER_CAP_MAX)
                        .take(ADAPTER_CAPS_MAX)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
        };
        (!e.agent_id.is_empty() && !e.kind.is_empty()).then_some(e)
    }

    /// When it stops being offered or kept.
    fn last_touched(&self) -> u64 {
        self.updated_at_ms
            .max(self.started_at_ms)
            .max(self.interrupted_at_ms)
    }

    pub fn interrupted(&self) -> bool {
        !self.ended && self.interrupted_at_ms != 0
    }

    /// The process it ran as, when one was recorded.
    pub fn process(&self) -> Option<agora_platform::ProcessIdentity> {
        (self.pid > 1 && self.process_started != 0).then(|| agora_platform::ProcessIdentity {
            pid: self.pid,
            group: self.process_group,
            started: self.process_started,
            boot: self.process_boot.clone(),
            name: self.process_name.clone(),
        })
    }

    /// ADR-0060: its process outlived the node that started it (a node killed with `kill -9`: terminal agents lead their own
    /// process group and do not die with it) and still runs: the same pid with the same start time.
    pub fn still_running(&self) -> bool {
        self.interrupted()
            && self
                .process()
                .is_some_and(|p| agora_platform::process_still_running(&p))
    }
}

/// The number in a node's agent id (`agent-7` is 7).
fn id_number(id: &str) -> Option<u64> {
    id.strip_prefix("agent-")?.parse().ok()
}

fn state_word(s: i32) -> &'static str {
    match v1::AgentState::try_from(s) {
        Ok(v1::AgentState::Working) => "working",
        Ok(v1::AgentState::Blocked) => "blocked",
        Ok(v1::AgentState::Idle) => "idle",
        Ok(v1::AgentState::Done) => "done",
        _ => "unknown",
    }
}

/// The protocol state for a word [`Ledger`] wrote.
pub fn proto_state(word: &str) -> v1::AgentState {
    match word {
        "working" => v1::AgentState::Working,
        "blocked" => v1::AgentState::Blocked,
        "idle" => v1::AgentState::Idle,
        "done" => v1::AgentState::Done,
        _ => v1::AgentState::Unspecified,
    }
}

impl Ledger {
    /// A ledger that never touches the disk.
    pub fn in_memory() -> Self {
        Self {
            path: None,
            state: Arc::new(Mutex::new(State {
                next_id: 1,
                ..State::default()
            })),
            write: Arc::new(Mutex::new(())),
        }
    }

    /// Opens the ledger in `data_dir` (made owner-only), as the node starts. Every entry that had not ended was running when the
    /// node last stopped: it is interrupted from `now_ms` on. Entries older than [`KEEP_MS`] are dropped, and at most
    /// [`MAX_ENTRIES`] kept. A file that cannot be read as a ledger is set aside (renamed, never deleted) and a new one starts.
    pub fn open(data_dir: &Path, now_ms: u64) -> io::Result<Self> {
        let dir = data_dir.join(LEDGER_DIR);
        std::fs::create_dir_all(&dir)?;
        agora_platform::restrict_to_owner(&dir)?;
        let path = dir.join(LEDGER_FILE);
        let mut state = State {
            next_id: 1,
            ..State::default()
        };
        match std::fs::metadata(&path) {
            Ok(meta) => {
                let parsed = (meta.len() <= MAX_FILE_BYTES)
                    .then(|| std::fs::read(&path).ok())
                    .flatten()
                    .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                    .filter(|v| number(v, "version") == VERSION);
                match parsed {
                    Some(v) => {
                        state.next_id = number(&v, "next_id").max(1);
                        for e in v
                            .get("entries")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .filter_map(Entry::from_json)
                        {
                            state.entries.insert(e.agent_id.clone(), e);
                        }
                    }
                    None => {
                        let mut aside = path.as_os_str().to_owned();
                        aside.push(format!(".unreadable-{now_ms}"));
                        std::fs::rename(&path, PathBuf::from(aside))?;
                        // The ids it gave are not known any more: start above any id a node could have given until now (one a
                        // second at most), so no agent id is given twice.
                        state.next_id = now_ms / 1000;
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        for e in state.entries.values_mut() {
            if let Some(n) = id_number(&e.agent_id) {
                state.next_id = state.next_id.max(n.saturating_add(1));
            }
            if !e.ended && e.interrupted_at_ms == 0 {
                e.interrupted_at_ms = now_ms;
            }
        }
        prune(&mut state, now_ms);
        state.dirty = true;
        let ledger = Self {
            path: Some(path),
            state: Arc::new(Mutex::new(state)),
            write: Arc::new(Mutex::new(())),
        };
        ledger.flush()?;
        Ok(ledger)
    }

    fn st(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The number the node's next agent id must start from, so an id is never given twice across restarts (an interrupted
    /// `agent-3` keeps its id, and the hub keys agents by it).
    pub fn next_id(&self) -> u64 {
        self.st().next_id
    }

    /// One agent's entry.
    pub fn get(&self, agent_id: &str) -> Option<Entry> {
        self.st().entries.get(agent_id).cloned()
    }

    /// The agents found interrupted and still offered, oldest id first.
    pub fn interrupted(&self) -> Vec<Entry> {
        let mut v: Vec<Entry> = self
            .st()
            .entries
            .values()
            .filter(|e| e.interrupted())
            .cloned()
            .collect();
        v.sort_by_key(|e| {
            (
                id_number(&e.agent_id).unwrap_or(u64::MAX),
                e.agent_id.clone(),
            )
        });
        v
    }

    /// #1120: the agents that ended and are still kept ([`KEEP_MS`] from `now_ms`), oldest id first. A restarted node lists them in
    /// `agent.list` as its previous process did, whatever folder they ran in.
    pub fn ended(&self, now_ms: u64) -> Vec<Entry> {
        let mut v: Vec<Entry> = self
            .st()
            .entries
            .values()
            .filter(|e| e.ended && now_ms.saturating_sub(e.last_touched()) <= KEEP_MS)
            .cloned()
            .collect();
        v.sort_by_key(|e| {
            (
                id_number(&e.agent_id).unwrap_or(u64::MAX),
                e.agent_id.clone(),
            )
        });
        v
    }

    /// The process an agent runs as now (`None`: it has none any more). Written when it changed. `false` when the ledger does not hold
    /// the agent (yet): nothing was written, and the caller must ask again later (the entry is made by `AgentStarted`, which the
    /// node records just after the supervisor has started the process). `since_ms` is a time before it was spawned, 0 when not known
    /// (`Entry::process_since_ms`, #925), written in the same write.
    pub fn set_process(
        &self,
        agent_id: &str,
        p: Option<agora_platform::ProcessIdentity>,
        since_ms: u64,
        now_ms: u64,
    ) -> bool {
        let mut st = self.st();
        let Some(e) = st.entries.get_mut(agent_id) else {
            return false;
        };
        let (pid, group, started, boot, name) =
            p.map_or((0, 0, 0, String::new(), String::new()), |p| {
                (
                    p.pid,
                    p.group,
                    p.started,
                    bounded(&p.boot),
                    bounded(&p.name),
                )
            });
        let now = (pid, group, started, boot, name);
        let was = (
            e.pid,
            e.process_group,
            e.process_started,
            e.process_boot.clone(),
            e.process_name.clone(),
        );
        if was == now {
            return true;
        }
        (
            e.pid,
            e.process_group,
            e.process_started,
            e.process_boot,
            e.process_name,
        ) = now;
        e.process_since_ms = if e.pid == 0 { 0 } else { since_ms };
        e.updated_at_ms = now_ms;
        st.dirty = true;
        drop(st);
        let _ = self.flush();
        true
    }

    /// ADR-0079 decision 9: the terminal holder the agent runs in now (`None`: the node's own). Written when it changed. `false` when the
    /// ledger does not hold the agent yet (try again later), as [`Self::set_process`].
    pub fn set_holder(&self, agent_id: &str, holder: Option<(u32, u64)>, now_ms: u64) -> bool {
        let mut st = self.st();
        let Some(e) = st.entries.get_mut(agent_id) else {
            return false;
        };
        let (pid, started) = holder.unwrap_or((0, 0));
        if (e.holder_pid, e.holder_started) == (pid, started) {
            return true;
        }
        (e.holder_pid, e.holder_started) = (pid, started);
        e.updated_at_ms = now_ms;
        st.dirty = true;
        drop(st);
        let _ = self.flush();
        true
    }

    /// ADR-0093: what a later node needs to take up an adapter agent that runs in the holder. Written when it changed. `false` when the
    /// ledger does not hold the agent yet (try again later), as [`Self::set_process`].
    pub fn set_adapter_state(
        &self,
        agent_id: &str,
        session: &str,
        capabilities: &[String],
        now_ms: u64,
    ) -> bool {
        // The same bounds the file is read with: a value that would read back empty or cut is not written at all (an empty session
        // makes the next node end the agent instead of taking it up with something it was never told).
        let (session, capabilities) = if session.len() <= ADAPTER_SESSION_MAX
            && capabilities.len() <= ADAPTER_CAPS_MAX
            && capabilities.iter().all(|c| c.len() <= ADAPTER_CAP_MAX)
        {
            (session, capabilities)
        } else {
            ("", &[][..])
        };
        let mut st = self.st();
        let Some(e) = st.entries.get_mut(agent_id) else {
            return false;
        };
        if e.adapter_session == session && e.adapter_capabilities == capabilities {
            return true;
        }
        e.adapter_session = session.to_string();
        e.adapter_capabilities = capabilities.to_vec();
        e.updated_at_ms = now_ms;
        st.dirty = true;
        drop(st);
        let _ = self.flush();
        true
    }

    /// ADR-0079 decision 9: an agent whose holder is alive was found running in it: it is not interrupted. `true` when `agent_id` was
    /// interrupted, held by exactly this holder, and now is not (the caller then attaches it).
    pub fn reclaim(&self, agent_id: &str, holder: (u32, u64), now_ms: u64) -> bool {
        let mut st = self.st();
        let Some(e) = st.entries.get_mut(agent_id) else {
            return false;
        };
        if !e.interrupted() || (e.holder_pid, e.holder_started) != holder || holder.0 == 0 {
            return false;
        }
        e.interrupted_at_ms = 0;
        e.updated_at_ms = now_ms;
        st.dirty = true;
        drop(st);
        let _ = self.flush();
        true
    }

    /// ADR-0062 item 4: the node brought this agent back by itself now.
    pub fn mark_auto_resumed(&self, agent_id: &str, now_ms: u64) {
        let mut st = self.st();
        if let Some(e) = st.entries.get_mut(agent_id) {
            e.auto_resumed_at_ms = now_ms;
            e.updated_at_ms = now_ms;
            st.dirty = true;
        }
        drop(st);
        let _ = self.flush();
    }

    /// ADR-0060 Amendment A: the agent was given its "carry on" message now, so it is not given another for this interruption.
    pub fn mark_continue_sent(&self, agent_id: &str, now_ms: u64) {
        let mut st = self.st();
        if let Some(e) = st.entries.get_mut(agent_id) {
            e.continue_sent_ms = now_ms;
            e.updated_at_ms = now_ms;
            st.dirty = true;
        }
        drop(st);
        let _ = self.flush();
    }

    /// Whether an automatic resume of this agent should carry its "carry on" message: it was working when the node stopped (not
    /// waiting for a person, idle, finished or unknown) and has not had one for this interruption (Amendment A, 1, 2 and 5).
    pub fn wants_continue(&self, agent_id: &str) -> bool {
        self.get(agent_id)
            .is_some_and(|e| e.last_state == "working" && e.continue_sent_ms < e.interrupted_at_ms)
    }

    /// Start again (ADR-0060): the agent's next run is a new session, so the old session id is no longer its own.
    pub fn forget_session(&self, agent_id: &str, now_ms: u64) {
        let mut st = self.st();
        if let Some(e) = st.entries.get_mut(agent_id) {
            if !e.session_id.is_empty() {
                e.session_id.clear();
                e.updated_at_ms = now_ms;
                st.dirty = true;
            }
        }
        drop(st);
        let _ = self.flush();
    }

    /// Learns from one event the node records. Writes the file when an agent started, reported its session, or ended; a failed
    /// write is retried with the next one (a full disk must never stop agents).
    pub fn observe(&self, body: &Body, now_ms: u64) {
        let dir = self.dir();
        let (write, start_writer) = {
            let mut st = self.st();
            // The kept copy changes and its write is marked pending in one step: a reader never finds neither (and reads the file).
            let mut start_writer = false;
            if let Some(agent) = keep(&mut st, body, dir.as_deref()) {
                st.mark_kept_pending(agent);
                start_writer = !std::mem::replace(&mut st.kept_writer, true);
            }
            let write = apply(&mut st, body, now_ms);
            if write {
                prune(&mut st, now_ms);
            }
            (write, start_writer)
        };
        if write {
            let _ = self.flush();
        }
        if start_writer {
            // One writer on a thread of its own: a report is recorded on the supervision thread, which must not wait for a disk, and
            // a flood of reports is written by the one writer as it goes (it writes what the ledger holds when it runs).
            let (state, write) = (Arc::clone(&self.state), Arc::clone(&self.write));
            let thread_dir = dir.clone();
            let spawned = std::thread::Builder::new()
                .name("agora-ledger-kept".into())
                .spawn(move || kept_writer(&state, &write, thread_dir.as_deref()));
            if spawned.is_err() {
                kept_writer(&self.state, &self.write, dir.as_deref());
            }
        }
    }

    fn dir(&self) -> Option<PathBuf> {
        self.path
            .as_ref()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf)
    }

    /// ADR-0093 (#932): what `agent` last reported of its session, as the last node recorded it (read from its file the first time),
    /// for a node that takes the agent up from the holder.
    pub fn kept_session(&self, agent: &str) -> KeptSession {
        {
            let st = self.st();
            if let Some(k) = st.kept.get(agent) {
                return k.clone();
            }
            // A change is not on disk yet (its file is about to be removed): what is kept here is the truth.
            if st.kept_pending.contains_key(agent) {
                return KeptSession::default();
            }
        }
        let file = self
            .path
            .as_ref()
            .and_then(|p| p.parent())
            .and_then(|d| kept_file(d, agent));
        let Some(file) = file else {
            return KeptSession::default();
        };
        // Read under the write lock, so no writer removes or replaces the file meanwhile; what was read is taken in only when no change
        // came for the agent while it was read (a change marks it pending under the state lock, which is taken after the read).
        let _writing = self.write.lock().unwrap_or_else(|e| e.into_inner());
        let read = std::fs::metadata(&file)
            .ok()
            .filter(|m| m.len() <= MAX_KEPT_BYTES)
            .and_then(|_| std::fs::read(&file).ok());
        let kept = read.map_or_else(KeptSession::default, |b| KeptSession::decode(&b, agent));
        let mut st = self.st();
        if let Some(k) = st.kept.get(agent) {
            return k.clone();
        }
        if st.kept_pending.contains_key(agent) {
            return KeptSession::default();
        }
        if kept != KeptSession::default() {
            st.kept.insert(agent.to_string(), kept.clone());
        }
        kept
    }

    /// Removes kept session files from earlier processes at startup, keeping those of agents just taken up from the holder.
    /// Failed removals stay pending while this node runs, so those files cannot be read back.
    pub fn forget_kept_except(&self, keep: &[&str]) {
        let Some(dir) = self.path.as_ref().and_then(|p| p.parent()) else {
            return;
        };
        let _writing = self.write.lock().unwrap_or_else(|e| e.into_inner());
        let Ok(list) = std::fs::read_dir(dir) else {
            return;
        };
        for f in list.flatten() {
            let name = f.file_name();
            // A write that never finished (the node ended between the temporary file and the rename) is swept too.
            if name
                .to_str()
                .is_some_and(|n| n.starts_with("kept-") && n.contains(".pb.tmp-"))
            {
                let _ = std::fs::remove_file(f.path());
                continue;
            }
            let Some(agent) = name
                .to_str()
                .and_then(|n| n.strip_prefix("kept-"))
                .and_then(|n| n.strip_suffix(".pb"))
            else {
                continue;
            };
            if !keep.contains(&agent) {
                let removed = remove_kept_file(&f.path());
                let mut st = self.st();
                st.kept.remove(agent);
                if !removed {
                    st.mark_kept_pending(agent.to_string());
                }
            }
        }
    }

    /// Writes what changed since the last write, if anything (the node calls it when it stops cleanly).
    pub fn flush(&self) -> io::Result<()> {
        let _writing = self.write.lock().unwrap_or_else(|e| e.into_inner());
        let Some(path) = &self.path else {
            self.st().dirty = false;
            return Ok(());
        };
        let bytes = {
            let mut st = self.st();
            if !st.dirty {
                return Ok(());
            }
            st.dirty = false;
            let entries: Vec<Value> = st.entries.values().map(Entry::to_json).collect();
            serde_json::to_vec(&json!({
                "version": VERSION,
                "next_id": st.next_id,
                "entries": entries,
            }))
            .map_err(io::Error::other)?
        };
        let written = agora_platform::write_private(path, &bytes);
        if written.is_err() {
            self.st().dirty = true;
        }
        written
    }

    /// [`Self::flush`], and the kept session files a report changed that are not written yet (their writer threads may not have run):
    /// what the node calls when it stops cleanly, so what a kept agent reported last is on disk for the next node.
    pub fn flush_all(&self) -> io::Result<()> {
        let written = self.flush();
        write_kept_pending(&self.state, &self.write, self.dir().as_deref());
        written
    }
}

/// Folds one event in; true when the change must be written now.
fn apply(st: &mut State, body: &Body, now: u64) -> bool {
    match body {
        Body::WorkspaceRegistered(w) => {
            if st.workspaces.len() >= MAX_WORKSPACES && !st.workspaces.contains_key(&w.workspace_id)
            {
                // Only the ones bound to a known agent matter after their start; the rest can go.
                let keep: BTreeSet<String> = st.bound.values().cloned().collect();
                st.workspaces.retain(|k, _| keep.contains(k));
                if st.workspaces.len() >= MAX_WORKSPACES {
                    return false;
                }
            }
            st.workspaces.insert(
                w.workspace_id.clone(),
                Workspace {
                    path: bounded(&w.path),
                    git_root: bounded(&w.git_root),
                    branch: bounded(&w.branch),
                    managed: w.managed_worktree,
                },
            );
            false
        }
        Body::AgentWorkspaceBound(b) => {
            if st.bound.len() < MAX_WORKSPACES || st.bound.contains_key(&b.agent_id) {
                st.bound.insert(b.agent_id.clone(), b.workspace_id.clone());
            }
            false
        }
        Body::AgentStarted(a) => {
            if a.agent_id.is_empty() || a.agent_id.len() > MAX_FIELD {
                return false;
            }
            if let Some(n) = id_number(&a.agent_id) {
                st.next_id = st.next_id.max(n.saturating_add(1));
            }
            let workspace = if a.worktree_id.is_empty() {
                st.bound
                    .get(&a.agent_id)
                    .and_then(|w| st.workspaces.get(w))
                    .cloned()
            } else {
                None
            };
            let fresh = !st.entries.contains_key(&a.agent_id);
            let e = st.entries.entry(a.agent_id.clone()).or_default();
            let before = e.clone();
            if fresh {
                e.agent_id = a.agent_id.clone();
                e.started_at_ms = now;
            }
            e.kind = bounded(&a.kind);
            e.project_id = bounded(&a.project_id);
            e.parent_agent_id = bounded(&a.parent_agent_id);
            // ADR-0059: kept for the record only; a resume never applies them (a resumed agent asks before acting again).
            e.launch_options = a
                .launch_options
                .iter()
                .filter(|o| o.len() <= 32)
                .take(8)
                .cloned()
                .collect();
            e.skips_permissions = a.skips_permissions;
            match workspace {
                Some(w) => {
                    e.directory = true;
                    e.folder = w.path;
                    e.git_root = w.git_root;
                    e.branch = w.branch;
                    e.managed_worktree = w.managed;
                }
                None if !a.worktree_id.is_empty() => {
                    e.directory = false;
                    e.folder = bounded(&a.worktree_id);
                }
                // A directory agent said again without its folder (a resync of an agent the node knows) keeps what it had.
                None => {}
            }
            if let Some(session) = st.early_sessions.remove(&a.agent_id) {
                e.session_id = session;
            }
            if let Some(state) = st.early_states.remove(&a.agent_id) {
                e.last_state = state;
            }
            // Running again (a restart after a crash, a resume): neither ended nor interrupted.
            e.ended = false;
            e.interrupted_at_ms = 0;
            let changed = *e != before;
            if changed {
                e.updated_at_ms = now;
                st.dirty = true;
            }
            changed
        }
        Body::AgentSessionReported(r) => match st.entries.get_mut(&r.agent_id) {
            None if crate::session_files::argv_safe(&r.session_id)
                && (st.early_sessions.len() < MAX_EARLY
                    || st.early_sessions.contains_key(&r.agent_id)) =>
            {
                st.early_sessions
                    .insert(r.agent_id.clone(), r.session_id.clone());
                false
            }
            Some(e) if e.session_id != r.session_id => {
                // Only an id that may go on a command line is kept for a resume after a restart (an adapter's own id may be any
                // string in its JSON, but such an agent is then offered Start again only).
                e.session_id = if crate::session_files::argv_safe(&r.session_id) {
                    r.session_id.clone()
                } else {
                    String::new()
                };
                e.updated_at_ms = now;
                st.dirty = true;
                true
            }
            _ => false,
        },
        Body::AgentStateChanged(s) => {
            let word = state_word(s.state);
            match st.entries.get_mut(&s.agent_id) {
                Some(e) => {
                    if e.last_state != word {
                        e.last_state = word.to_string();
                        st.dirty = true;
                    }
                }
                // Before the agent's start is recorded: held for it, never creating an entry (an entry is a running agent).
                None if !s.agent_id.is_empty()
                    && s.agent_id.len() <= MAX_FIELD
                    && (st.early_states.len() < MAX_EARLY
                        || st.early_states.contains_key(&s.agent_id)) =>
                {
                    st.early_states.insert(s.agent_id.clone(), word.to_string());
                }
                None => {}
            }
            false
        }
        Body::AgentExited(x) => match st.entries.get_mut(&x.agent_id) {
            Some(e) if !e.ended => {
                e.ended = true;
                e.interrupted_at_ms = 0;
                (e.pid, e.process_group, e.process_started) = (0, 0, 0);
                e.process_since_ms = 0;
                e.process_boot.clear();
                e.process_name.clear();
                e.updated_at_ms = now;
                st.dirty = true;
                true
            }
            _ => false,
        },
        _ => false,
    }
}

/// Drops entries older than [`KEEP_MS`], then the oldest ended ones (and then the oldest of any) above [`MAX_ENTRIES`]. Running
/// agents are never dropped by age.
fn prune(st: &mut State, now: u64) {
    let mut removed = Vec::new();
    st.entries.retain(|id, e| {
        let running = !e.ended && e.interrupted_at_ms == 0;
        let keep = running || now.saturating_sub(e.last_touched()) <= KEEP_MS;
        if !keep {
            removed.push(id.clone());
        }
        keep
    });
    if st.entries.len() > MAX_ENTRIES {
        let mut order: Vec<(bool, u64, String)> = st
            .entries
            .values()
            .map(|e| (!e.ended, e.last_touched(), e.agent_id.clone()))
            .collect();
        order.sort();
        let extra = st.entries.len() - MAX_ENTRIES;
        for (_, _, id) in order.into_iter().take(extra) {
            st.entries.remove(&id);
            removed.push(id);
        }
    }
    // Only the bindings of the entries dropped here go: a binding whose agent's start is not recorded yet (another agent's
    // write can land between `AgentWorkspaceBound` and `AgentStarted`) is still needed for its folder.
    for id in &removed {
        st.bound.remove(id);
    }
    if !removed.is_empty() {
        st.dirty = true;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("agora-ledger-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn started(id: &str, worktree: &str, parent: &str) -> Body {
        Body::AgentStarted(v1::AgentStarted {
            agent_id: id.into(),
            project_id: "p".into(),
            node_id: "n".into(),
            worktree_id: worktree.into(),
            kind: "fake".into(),
            level: 2,
            parent_agent_id: parent.into(),
            ..Default::default()
        })
    }

    fn state_event(id: &str, state: v1::AgentState) -> Body {
        Body::AgentStateChanged(v1::AgentStateChanged {
            agent_id: id.into(),
            state: state as i32,
            reported: false,
        })
    }

    #[test]
    fn a_state_recorded_before_the_agent_started_is_kept_for_it() {
        let l = Ledger::in_memory();
        l.observe(&state_event("agent-1", v1::AgentState::Working), 10);
        l.observe(&state_event("agent-1", v1::AgentState::Idle), 11);
        assert!(l.get("agent-1").is_none(), "a state alone made an entry");
        l.observe(&started("agent-1", "/wt/a", ""), 12);
        assert_eq!(l.get("agent-1").unwrap().last_state, "idle");
        // After the start, changes apply as before.
        l.observe(&state_event("agent-1", v1::AgentState::Blocked), 13);
        assert_eq!(l.get("agent-1").unwrap().last_state, "blocked");
        // The held state is used once: a later start of the same id does not bring it back.
        l.observe(&started("agent-1", "/wt/a", ""), 14);
        assert_eq!(l.get("agent-1").unwrap().last_state, "blocked");
    }

    /// #907: what the launch said of acting without asking is kept with the agent and written, so a node that takes the agent up from
    /// the holder shows it the same way; a later start of the id (a resume, which gets no option back) replaces it.
    #[test]
    fn a_launch_that_skips_permissions_is_kept_across_a_reopen() {
        let dir = temp("skips");
        let l = Ledger::open(&dir, 1).unwrap();
        l.observe(
            &Body::AgentStarted(v1::AgentStarted {
                agent_id: "agent-1".into(),
                kind: "fake".into(),
                worktree_id: "/wt/a".into(),
                launch_options: vec!["skip".into()],
                skips_permissions: true,
                ..Default::default()
            }),
            2,
        );
        l.flush().unwrap();
        let again = Ledger::open(&dir, 3).unwrap();
        let e = again.get("agent-1").unwrap();
        assert!(e.skips_permissions);
        assert_eq!(e.launch_options, vec!["skip".to_string()]);
        again.observe(&started("agent-1", "/wt/a", ""), 4);
        assert!(!again.get("agent-1").unwrap().skips_permissions);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #932: what an adapter agent last reported of its session is written beside the ledger (also when it came before the agent's
    /// start), read back by a later node, replaced by a newer report, and removed when the agent ends or is not taken up.
    #[test]
    /// ADR-0101: a kept agent's always answers are written beside its session, read back by a later node (which says them again after
    /// it takes the agent up), bounded and never twice; a new process, an end or an interruption drops them.
    fn a_kept_agents_always_answers_survive_a_node_restart_and_end_with_its_process() {
        let dir = temp("kept-always");
        let l = Ledger::open(&dir, 1).unwrap();
        let always = |id: &str| {
            Body::AgentAlwaysAllowed(v1::AgentAlwaysAllowed {
                agent_id: "agent-1".into(),
                approval_id: id.into(),
                action: "run_command".into(),
                signer_device_id: "phone".into(),
                at_ms: 2,
            })
        };
        l.observe(&started("agent-1", "/wt/a", ""), 2);
        l.observe(&always("ap-1"), 3);
        l.observe(&always("ap-1"), 4);
        for i in 0..20 {
            l.observe(&always(&format!("x-{i}")), 5);
        }
        l.flush_all().unwrap();
        let again = Ledger::open(&dir, 6).unwrap();
        let kept = again.kept_session("agent-1");
        assert_eq!(kept.always.len(), MAX_KEPT_ALWAYS);
        assert_eq!(kept.always.last().unwrap().approval_id, "x-19");
        assert!(kept
            .bodies()
            .iter()
            .any(|b| matches!(b, Body::AgentAlwaysAllowed(a) if a.approval_id == "x-19")));
        // A new process of the running agent (a restart after a crash, on the node that ran it) holds none of them.
        l.observe(&started("agent-1", "/wt/a", ""), 7);
        assert!(l.kept_session("agent-1").always.is_empty());
        l.observe(&always("ap-2"), 8);
        assert_eq!(l.kept_session("agent-1").always.len(), 1);
        l.observe(
            &Body::AgentInterrupted(v1::AgentInterrupted {
                agent_id: "agent-1".into(),
                ..Default::default()
            }),
            9,
        );
        assert!(l.kept_session("agent-1").always.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_kept_session_is_written_read_back_and_removed() {
        let dir = temp("kept");
        let l = Ledger::open(&dir, 1).unwrap();
        let options = |mode: &str| {
            Body::AgentSessionOptions(v1::AgentSessionOptions {
                agent_id: "agent-1".into(),
                current_mode: mode.into(),
                ..Default::default()
            })
        };
        l.observe(&options("plan"), 2);
        l.observe(&started("agent-1", "/wt/a", ""), 3);
        l.observe(&options("bypassPermissions"), 4);
        l.flush_all().unwrap();
        let file = dir.join(LEDGER_DIR).join("kept-agent-1.pb");
        assert!(file.is_file());
        let again = Ledger::open(&dir, 5).unwrap();
        let kept = again.kept_session("agent-1");
        assert_eq!(kept.options.unwrap().current_mode, "bypassPermissions");
        assert!(kept.config.is_none());
        again.forget_kept_except(&["agent-1"]);
        assert!(file.is_file());
        again.observe(
            &Body::AgentExited(v1::AgentExited {
                agent_id: "agent-1".into(),
                exit_code: 0,
            }),
            6,
        );
        assert_eq!(again.kept_session("agent-1"), KeptSession::default());
        again.flush_all().unwrap();
        assert!(!file.exists(), "an ended agent kept its session");
        again.observe(&options("plan"), 7);
        again.flush_all().unwrap();
        assert!(file.is_file());
        again.flush_all().unwrap();
        again.forget_kept_except(&[]);
        assert!(!file.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Codex review of #928: a kept session only on disk (a later node opened the ledger and has not read it) is an earlier process's
    /// once the agent is interrupted, and must not come back for the agent's next process. A restart of a running agent drops it too;
    /// a resume keeps what its new process reported before the start was recorded.
    #[test]
    fn a_kept_session_only_on_disk_goes_when_the_agent_is_interrupted_or_restarted() {
        let dir = temp("kept-stale");
        let options = |mode: &str| {
            Body::AgentSessionOptions(v1::AgentSessionOptions {
                agent_id: "agent-1".into(),
                current_mode: mode.into(),
                ..Default::default()
            })
        };
        let file = dir.join(LEDGER_DIR).join("kept-agent-1.pb");
        let old = Ledger::open(&dir, 1).unwrap();
        old.observe(&started("agent-1", "/wt/a", ""), 2);
        old.observe(&options("bypassPermissions"), 3);
        old.flush_all().unwrap();
        drop(old);
        let current = Ledger::open(&dir, 4).unwrap();
        current.observe(
            &Body::AgentInterrupted(v1::AgentInterrupted {
                agent_id: "agent-1".into(),
                ..Default::default()
            }),
            5,
        );
        assert_eq!(current.kept_session("agent-1"), KeptSession::default());
        current.flush_all().unwrap();
        assert!(
            !file.exists(),
            "an interrupted agent kept its session on disk"
        );
        // The resumed process reports before its start is recorded: that report is its own and stays.
        current.observe(&options("plan"), 6);
        current.observe(&started("agent-1", "/wt/a", ""), 7);
        assert_eq!(
            current
                .kept_session("agent-1")
                .options
                .map(|o| o.current_mode),
            Some("plan".to_string())
        );
        current.flush_all().unwrap();
        drop(current);
        // A restart of the running agent (after a crash) is a new process: what the old one reported goes, from disk too.
        let later = Ledger::open(&dir, 8).unwrap();
        // As if it were running again (taken up, or this node's own).
        let mut st = later.st();
        if let Some(e) = st.entries.get_mut("agent-1") {
            e.interrupted_at_ms = 0;
        }
        drop(st);
        later.observe(&started("agent-1", "/wt/a", ""), 10);
        assert_eq!(later.kept_session("agent-1"), KeptSession::default());
        later.flush_all().unwrap();
        assert!(
            !file.exists(),
            "a restarted agent kept the old process's session"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Independent review of #928: a report larger than the hub keeps is not kept (and the one before it goes, as it was replaced),
    /// and an unfinished write is swept at the next start.
    #[test]
    fn a_kept_report_is_bounded_and_unfinished_writes_are_swept() {
        let dir = temp("kept-bound");
        let l = Ledger::open(&dir, 1).unwrap();
        l.observe(&started("agent-1", "/wt/a", ""), 2);
        let commands = |n: usize| {
            Body::AgentCommands(v1::AgentCommands {
                agent_id: "agent-1".into(),
                commands: (0..n)
                    .map(|i| v1::AgentCommand {
                        name: format!("c{i}"),
                        description: "d".repeat(1000),
                        input_hint: String::new(),
                    })
                    .collect(),
            })
        };
        l.observe(&commands(2), 3);
        assert!(l.kept_session("agent-1").commands.is_some());
        l.observe(&commands(100), 4);
        assert!(
            l.kept_session("agent-1").commands.is_none(),
            "a report larger than the hub keeps was kept"
        );
        let sessions = dir.join(LEDGER_DIR);
        let stray = sessions.join("kept-agent-1.pb.tmp-99999");
        std::fs::write(&stray, b"half").unwrap();
        l.forget_kept_except(&["agent-1"]);
        assert!(!stray.exists(), "an unfinished write was left behind");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Independent verification of #928: a reader (a resync announcement on the uplink thread) that looks for a kept session while the
    /// agent's end is recorded must never bring the old file back, in memory or on disk. The file is only on disk (a later node opened
    /// the ledger), and the reader loops while `AgentExited` is observed; 200 rounds.
    #[test]
    fn a_reader_never_brings_back_the_session_of_an_agent_that_ended() {
        let dir = temp("kept-race");
        let file = dir.join(LEDGER_DIR).join("kept-agent-1.pb");
        for round in 0..200u64 {
            let _ = std::fs::remove_dir_all(&dir);
            {
                let old = Ledger::open(&dir, 1).unwrap();
                old.observe(&started("agent-1", "/wt/a", ""), 2);
                old.observe(
                    &Body::AgentSessionOptions(v1::AgentSessionOptions {
                        agent_id: "agent-1".into(),
                        current_mode: "bypassPermissions".into(),
                        ..Default::default()
                    }),
                    3,
                );
                old.flush_all().unwrap();
            }
            assert!(file.is_file(), "round {round}: setup");
            let l = std::sync::Arc::new(Ledger::open(&dir, 4).unwrap());
            let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let reader = {
                let (l, stop) = (std::sync::Arc::clone(&l), std::sync::Arc::clone(&stop));
                std::thread::spawn(move || {
                    while !stop.load(std::sync::atomic::Ordering::Acquire) {
                        let _ = l.kept_session("agent-1");
                    }
                })
            };
            l.observe(
                &Body::AgentExited(v1::AgentExited {
                    agent_id: "agent-1".into(),
                    exit_code: 0,
                }),
                5,
            );
            stop.store(true, std::sync::atomic::Ordering::Release);
            reader.join().unwrap();
            l.flush_all().unwrap();
            assert_eq!(
                l.kept_session("agent-1"),
                KeptSession::default(),
                "round {round}: an ended agent's session came back in memory"
            );
            assert!(
                !file.exists(),
                "round {round}: an ended agent's session came back on disk"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Ending a terminal agent (nothing kept anywhere) starts no writer.
    #[test]
    fn an_agent_with_nothing_kept_starts_no_writer() {
        let dir = temp("kept-none");
        let l = Ledger::open(&dir, 1).unwrap();
        l.observe(&started("agent-1", "/wt/a", ""), 2);
        l.observe(
            &Body::AgentExited(v1::AgentExited {
                agent_id: "agent-1".into(),
                exit_code: 0,
            }),
            3,
        );
        let st = l.st();
        assert!(!st.kept_writer && st.kept_pending.is_empty());
        drop(st);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_kept_removal_hides_the_old_report_and_can_be_retried() {
        let dir = temp("kept-remove-failed");
        let sessions = dir.join(LEDGER_DIR);
        let file = sessions.join("kept-agent-1.pb");
        let backup = sessions.join("original.pb");
        let options = |agent_id: &str| {
            Body::AgentSessionOptions(v1::AgentSessionOptions {
                agent_id: agent_id.into(),
                current_mode: "bypassPermissions".into(),
                ..Default::default()
            })
        };
        let start = started("agent-1", "/wt/a", "");
        let old = Ledger::open(&dir, 1).unwrap();
        old.observe(&start, 2);
        old.observe(&options("agent-1"), 3);
        old.flush_all().unwrap();
        drop(old);

        // A directory at the sidecar path makes remove_file fail on every platform. Restore the sidecar itself after the attempt.
        std::fs::rename(&file, &backup).unwrap();
        std::fs::create_dir(&file).unwrap();
        let ledger = Ledger::open(&dir, 4).unwrap();
        // Drive the single writer ourselves so both pending changes are in its batch.
        ledger.st().kept_writer = true;
        ledger.observe(
            &Body::AgentInterrupted(v1::AgentInterrupted {
                agent_id: "agent-1".into(),
                ..Default::default()
            }),
            5,
        );
        ledger.observe(&start, 6);
        ledger.observe(&options("agent-2"), 7);
        kept_writer(&ledger.state, &ledger.write, Some(&sessions));
        assert!(!ledger.st().kept_writer);
        assert!(sessions.join("kept-agent-2.pb").is_file());
        std::fs::remove_dir(&file).unwrap();
        std::fs::rename(&backup, &file).unwrap();

        let report = ledger.kept_session("agent-1");
        let still_on_disk = file.is_file();
        ledger.flush_all().unwrap();
        let removed_on_retry = !file.exists();
        drop(ledger);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(still_on_disk);
        assert_eq!(report, KeptSession::default());
        assert!(removed_on_retry);
    }

    #[test]
    fn a_failed_startup_sweep_hides_the_previous_processs_report() {
        let dir = temp("kept-sweep-failed");
        let sessions = dir.join(LEDGER_DIR);
        let file = sessions.join("kept-agent-1.pb");
        let backup = sessions.join("original.pb");
        let start = started("agent-1", "/wt/a", "");
        let old = Ledger::open(&dir, 1).unwrap();
        old.observe(&start, 2);
        old.observe(
            &Body::AgentSessionOptions(v1::AgentSessionOptions {
                agent_id: "agent-1".into(),
                current_mode: "bypassPermissions".into(),
                ..Default::default()
            }),
            3,
        );
        old.flush_all().unwrap();
        drop(old);

        std::fs::rename(&file, &backup).unwrap();
        std::fs::create_dir(&file).unwrap();
        let ledger = Ledger::open(&dir, 4).unwrap();
        ledger.forget_kept_except(&[]);
        std::fs::remove_dir(&file).unwrap();
        std::fs::rename(&backup, &file).unwrap();
        ledger.observe(&start, 5);

        let report = ledger.kept_session("agent-1");
        ledger.flush_all().unwrap();
        let removed_on_retry = !file.exists();
        drop(ledger);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(report, KeptSession::default());
        assert!(removed_on_retry);
    }

    #[test]
    fn states_held_for_agents_not_yet_started_are_bounded() {
        let l = Ledger::in_memory();
        for n in 0..(MAX_EARLY + 10) {
            l.observe(&state_event(&format!("agent-{n}"), v1::AgentState::Idle), 1);
        }
        assert_eq!(l.st().early_states.len(), MAX_EARLY);
    }

    fn directory_start(l: &Ledger, id: &str, path: &str, now: u64) {
        l.observe(
            &Body::WorkspaceRegistered(v1::WorkspaceRegistered {
                workspace_id: format!("directory:{path}"),
                node_id: "n".into(),
                project_id: "p".into(),
                path: path.into(),
                git_root: path.into(),
                branch: "main".into(),
                managed_worktree: false,
                files_changed: 0,
            }),
            now,
        );
        l.observe(
            &Body::AgentWorkspaceBound(v1::AgentWorkspaceBound {
                agent_id: id.into(),
                workspace_id: format!("directory:{path}"),
            }),
            now,
        );
        l.observe(&started(id, "", ""), now);
    }

    #[test]
    fn an_agent_that_did_not_end_is_interrupted_at_the_next_open_with_its_folder_and_session() {
        let dir = temp("interrupted");
        let l = Ledger::open(&dir, 1_000).unwrap();
        directory_start(&l, "agent-1", "/work/a", 1_000);
        l.observe(
            &Body::AgentSessionReported(v1::AgentSessionReported {
                agent_id: "agent-1".into(),
                session_id: "s-9".into(),
            }),
            1_100,
        );
        l.observe(&started("agent-2", "/wt/b", "agent-1"), 1_200);
        l.observe(
            &Body::AgentExited(v1::AgentExited {
                agent_id: "agent-2".into(),
                exit_code: 0,
            }),
            1_300,
        );
        l.observe(
            &Body::AgentStateChanged(v1::AgentStateChanged {
                agent_id: "agent-1".into(),
                state: v1::AgentState::Working as i32,
                reported: false,
            }),
            1_400,
        );
        // The node stops (no end recorded for agent-1); a clean stop writes the last state.
        l.flush().unwrap();
        drop(l);
        let again = Ledger::open(&dir, 5_000).unwrap();
        let found = again.interrupted();
        assert_eq!(found.len(), 1);
        let e = &found[0];
        assert_eq!(e.agent_id, "agent-1");
        assert_eq!(e.folder, "/work/a");
        assert!(e.directory);
        assert_eq!(e.session_id, "s-9");
        assert_eq!(e.last_state, "working");
        assert_eq!(e.started_at_ms, 1_000);
        assert_eq!(e.interrupted_at_ms, 5_000);
        assert!(again.get("agent-2").is_some_and(|e| e.ended));
        // Ids are never given twice.
        assert_eq!(again.next_id(), 3);
        // Resumed: running again, not interrupted any more, same start time and session.
        again.observe(&started("agent-1", "", ""), 6_000);
        let e = again.get("agent-1").unwrap();
        assert!(!e.interrupted() && !e.ended);
        assert_eq!(
            (e.folder.as_str(), e.session_id.as_str()),
            ("/work/a", "s-9")
        );
        assert_eq!(e.started_at_ms, 1_000);
    }

    #[cfg(unix)]
    #[test]
    fn the_ledger_is_owner_only_and_replaced_whole() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp("private");
        let l = Ledger::open(&dir, 1).unwrap();
        l.observe(&started("agent-1", "/wt/a", ""), 2);
        let folder = dir.join(LEDGER_DIR);
        let file = folder.join(LEDGER_FILE);
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&folder), 0o700);
        assert_eq!(mode(&file), 0o600);
        // Nothing but the ledger in its folder: no temporary file left behind.
        let names: Vec<_> = std::fs::read_dir(&folder)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from(LEDGER_FILE)]);
    }

    #[test]
    fn old_entries_go_after_fourteen_days_and_the_count_is_bounded() {
        let dir = temp("prune");
        let l = Ledger::open(&dir, 1).unwrap();
        l.observe(&started("agent-1", "/wt/a", ""), 1);
        drop(l);
        // Interrupted at 10, offered for 14 days from then.
        let l = Ledger::open(&dir, 10).unwrap();
        assert_eq!(l.interrupted().len(), 1);
        drop(l);
        let l = Ledger::open(&dir, 10 + KEEP_MS + 1).unwrap();
        assert!(l.interrupted().is_empty());
        assert!(l.get("agent-1").is_none());
        // The id is still not given again.
        assert_eq!(l.next_id(), 2);
        // In memory: the same rules, without a thousand writes.
        let l = Ledger::in_memory();
        for i in 0..(MAX_ENTRIES + 20) {
            let id = format!("agent-{}", i + 10);
            let t = 100 + i as u64;
            l.observe(&started(&id, "/wt/x", ""), t);
            l.observe(
                &Body::AgentExited(v1::AgentExited {
                    agent_id: id,
                    exit_code: 0,
                }),
                t,
            );
        }
        assert_eq!(l.st().entries.len(), MAX_ENTRIES);
        assert!(l.get("agent-10").is_none());
    }

    /// #1120: a reopened ledger offers the agents that ended, directory agents as well as worktree ones, for 14 days from their end;
    /// an interrupted one is not among them (it is offered to resume instead).
    #[test]
    fn the_agents_that_ended_are_read_back_after_a_reopen_whatever_their_folder() {
        let dir = temp("ended");
        let l = Ledger::open(&dir, 1).unwrap();
        l.observe(&started("agent-2", "/wt/a", ""), 1);
        directory_start(&l, "agent-1", "/work/repo", 2);
        l.observe(&started("agent-3", "/wt/c", ""), 3);
        for id in ["agent-1", "agent-2"] {
            l.observe(&state_event(id, v1::AgentState::Done), 5);
            l.observe(
                &Body::AgentExited(v1::AgentExited {
                    agent_id: id.into(),
                    exit_code: 0,
                }),
                10,
            );
        }
        drop(l);
        let l = Ledger::open(&dir, 20).unwrap();
        let ended = l.ended(20);
        let rows: Vec<_> = ended
            .iter()
            .map(|e| {
                (
                    e.agent_id.as_str(),
                    e.folder.as_str(),
                    e.directory,
                    e.last_state.as_str(),
                )
            })
            .collect();
        assert_eq!(
            rows,
            vec![
                ("agent-1", "/work/repo", true, "done"),
                ("agent-2", "/wt/a", false, "done"),
            ]
        );
        assert!(ended.iter().all(|e| !e.managed_worktree));
        assert_eq!(l.interrupted().len(), 1, "agent-3 did not end");
        assert_eq!(l.ended(10 + KEEP_MS).len(), 2);
        assert!(l.ended(10 + KEEP_MS + 1).is_empty(), "kept 14 days only");
    }

    #[test]
    fn a_session_id_that_could_be_an_option_never_survives_into_the_ledger() {
        let long = "a".repeat(129);
        let bad = [
            "--dangerously-bypass-approvals-and-sandbox",
            "-x",
            "a b",
            "a;b",
            "ünï",
            "",
            long.as_str(),
        ];
        // Reported at run time (a hook, an adapter): not kept.
        let l = Ledger::in_memory();
        l.observe(&started("agent-1", "/wt/a", ""), 1);
        for id in bad {
            l.observe(
                &Body::AgentSessionReported(v1::AgentSessionReported {
                    agent_id: "agent-1".into(),
                    session_id: id.into(),
                }),
                2,
            );
            assert!(l.get("agent-1").unwrap().session_id.is_empty(), "{id:?}");
        }
        // Written into the file by someone else: dropped when it is read; the agent is offered Start again.
        let dir = temp("planted");
        std::fs::create_dir_all(dir.join(LEDGER_DIR)).unwrap();
        let entries: Vec<Value> = bad
            .iter()
            .enumerate()
            .map(|(i, id)| json!({"agent_id": format!("agent-{}", i + 1), "kind": "codex", "session_id": id, "started_at_ms": 1}))
            .chain([json!({"agent_id": "agent-9", "kind": "codex", "session_id": "0199aaaa-0000-4000-8000-000000000001", "started_at_ms": 1})])
            .collect();
        std::fs::write(
            dir.join(LEDGER_DIR).join(LEDGER_FILE),
            serde_json::to_vec(&json!({"version": 1, "next_id": 10, "entries": entries})).unwrap(),
        )
        .unwrap();
        let l = Ledger::open(&dir, 5).unwrap();
        let found = l.interrupted();
        assert_eq!(found.len(), bad.len() + 1);
        for e in &found {
            let ok = e.agent_id == "agent-9";
            assert_eq!(e.session_id.is_empty(), !ok, "{e:?}");
        }
    }

    #[test]
    fn concurrent_writes_leave_a_whole_ledger_with_every_agent() {
        let dir = temp("race");
        let l = std::sync::Arc::new(Ledger::open(&dir, 1).unwrap());
        let threads: Vec<_> = (0..8)
            .map(|t| {
                let l = std::sync::Arc::clone(&l);
                std::thread::spawn(move || {
                    for i in 0..10 {
                        l.observe(
                            &started(&format!("agent-{}", t * 100 + i + 1), "/wt/a", ""),
                            2,
                        );
                        l.flush().unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let again = Ledger::open(&dir, 3).unwrap();
        assert_eq!(again.interrupted().len(), 80);
        let names: Vec<_> = std::fs::read_dir(dir.join(LEDGER_DIR))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from(LEDGER_FILE)]);
    }

    #[test]
    fn a_session_reported_before_its_start_is_recorded_is_kept() {
        let l = Ledger::in_memory();
        l.observe(
            &Body::AgentSessionReported(v1::AgentSessionReported {
                agent_id: "agent-1".into(),
                session_id: "native-s-1".into(),
            }),
            1,
        );
        l.observe(&started("agent-1", "/wt/a", ""), 2);
        assert_eq!(l.get("agent-1").unwrap().session_id, "native-s-1");
    }

    #[test]
    fn a_write_for_another_agent_between_binding_and_start_keeps_the_folder() {
        // The daemon records `AgentWorkspaceBound` and `AgentStarted` in two calls; another thread's write can land in between
        // and prune. Forced here by call order.
        let dir = temp("bound-race");
        let l = Ledger::open(&dir, 1).unwrap();
        l.observe(&started("agent-1", "/wt/one", ""), 1);
        l.observe(
            &Body::WorkspaceRegistered(v1::WorkspaceRegistered {
                workspace_id: "directory:/work/two".into(),
                node_id: "n".into(),
                project_id: "p".into(),
                path: "/work/two".into(),
                git_root: "/work/two".into(),
                branch: "main".into(),
                managed_worktree: false,
                files_changed: 0,
            }),
            2,
        );
        l.observe(
            &Body::AgentWorkspaceBound(v1::AgentWorkspaceBound {
                agent_id: "agent-2".into(),
                workspace_id: "directory:/work/two".into(),
            }),
            2,
        );
        l.observe(
            &Body::AgentExited(v1::AgentExited {
                agent_id: "agent-1".into(),
                exit_code: 0,
            }),
            3,
        );
        l.observe(&started("agent-2", "", ""), 4);
        let e = l.get("agent-2").unwrap();
        assert_eq!((e.folder.as_str(), e.directory), ("/work/two", true));
        drop(l);
        // And after a restart, where it is offered again in that folder.
        let e = Ledger::open(&dir, 10).unwrap().get("agent-2").unwrap();
        assert!(e.interrupted());
        assert_eq!((e.folder.as_str(), e.directory), ("/work/two", true));
    }

    #[test]
    fn an_unreadable_ledger_is_set_aside_not_deleted() {
        let dir = temp("unreadable");
        std::fs::create_dir_all(dir.join(LEDGER_DIR)).unwrap();
        std::fs::write(dir.join(LEDGER_DIR).join(LEDGER_FILE), b"{not json").unwrap();
        let l = Ledger::open(&dir, 1_800_000_000_000).unwrap();
        assert!(l.interrupted().is_empty());
        // Ids continue from a floor no earlier run can have reached, not from 1.
        assert_eq!(l.next_id(), 1_800_000_000);
        assert!(dir
            .join(LEDGER_DIR)
            .join(format!("{LEDGER_FILE}.unreadable-1800000000000"))
            .exists());
    }

    #[test]
    fn no_prompt_or_terminal_text_reaches_the_ledger() {
        let dir = temp("contents");
        let l = Ledger::open(&dir, 1).unwrap();
        l.observe(&started("agent-1", "/wt/a", ""), 1);
        // Events that carry text other people or agents wrote are not kept.
        l.observe(
            &Body::MessagePosted(v1::MessagePosted {
                message_id: "m".into(),
                from_agent_id: "agent-1".into(),
                to: "x".into(),
                body: "secret prompt".into(),
            }),
            2,
        );
        l.observe(
            &Body::ApprovalRequested(v1::ApprovalRequested {
                approval_id: "a".into(),
                agent_id: "agent-1".into(),
                action: "run_command".into(),
                detail: "echo TOKEN".into(),
                expires_at_ms: 9,
                ..Default::default()
            }),
            3,
        );
        l.flush().unwrap();
        let text = std::fs::read_to_string(dir.join(LEDGER_DIR).join(LEDGER_FILE)).unwrap();
        assert!(!text.contains("secret prompt") && !text.contains("TOKEN"));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod carry_on_tests {
    use super::*;

    fn state(l: &Ledger, id: &str, s: v1::AgentState, now: u64) {
        l.observe(
            &Body::AgentStateChanged(v1::AgentStateChanged {
                agent_id: id.into(),
                state: s as i32,
                reported: false,
            }),
            now,
        );
    }

    fn agent(l: &Ledger, id: &str, now: u64) {
        l.observe(
            &Body::AgentStarted(v1::AgentStarted {
                agent_id: id.into(),
                project_id: "p".into(),
                node_id: "n".into(),
                worktree_id: "w".into(),
                kind: "claude-code".into(),
                level: 1,
                parent_agent_id: String::new(),
                same_process: false,
                launch_options: Vec::new(),
                skips_permissions: false,
            }),
            now,
        );
    }

    /// ADR-0093: what is written is what reads back: a session id or capability list beyond the bounds is not written (never half).
    #[test]
    fn an_adapter_state_beyond_the_bounds_is_not_written_and_one_within_them_is() {
        let l = Ledger::in_memory();
        agent(&l, "a1", 1_000);
        let caps = vec!["chat".to_string(), "approvals".to_string()];
        assert!(l.set_adapter_state("a1", "s-1", &caps, 1_100));
        let e = l.get("a1").unwrap();
        assert_eq!(
            (e.adapter_session.as_str(), e.adapter_capabilities.clone()),
            ("s-1", caps.clone())
        );
        let long = "s".repeat(ADAPTER_SESSION_MAX + 1);
        assert!(l.set_adapter_state("a1", &long, &caps, 1_200));
        let e = l.get("a1").unwrap();
        assert!(e.adapter_session.is_empty() && e.adapter_capabilities.is_empty());
        let many: Vec<String> = (0..=ADAPTER_CAPS_MAX).map(|i| i.to_string()).collect();
        assert!(l.set_adapter_state("a1", "s-1", &many, 1_300));
        assert!(l.get("a1").unwrap().adapter_session.is_empty());
        // And what is written survives the file.
        let e = Entry::from_json(&e.to_json()).unwrap();
        assert_eq!(e.adapter_session, "");
    }

    /// ADR-0060 Amendment A: the "carry on" message is for an agent that was working, once for each interruption.
    #[test]
    fn only_a_working_agent_wants_the_message_and_only_once_for_each_interruption() {
        let dir = std::env::temp_dir().join(format!("agora-carry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let l = Ledger::open(&dir, 1_000).unwrap();
        for (id, s) in [
            ("w", v1::AgentState::Working),
            ("b", v1::AgentState::Blocked),
            ("i", v1::AgentState::Idle),
            ("u", v1::AgentState::Unspecified),
        ] {
            agent(&l, id, 1_100);
            state(&l, id, s, 1_200);
        }
        drop(l);
        // The node stops and starts again: every one of them is found interrupted at 5_000.
        let l = Ledger::open(&dir, 5_000).unwrap();
        assert!(l.wants_continue("w"));
        for id in ["b", "i", "u", "nobody"] {
            assert!(!l.wants_continue(id), "{id}");
        }
        // Said once: not again for this interruption; the field survives a restart of the node.
        l.mark_continue_sent("w", 5_100);
        assert!(!l.wants_continue("w"));
        drop(l);
        let l = Ledger::open(&dir, 5_200).unwrap();
        assert_eq!(l.get("w").unwrap().continue_sent_ms, 5_100);
        assert!(!l.wants_continue("w"));
        // It was brought back (a start is seen again), works again, and is interrupted by a later stop: a new interruption.
        agent(&l, "w", 5_250);
        assert!(!l.get("w").unwrap().interrupted());
        state(&l, "w", v1::AgentState::Working, 5_300);
        drop(l);
        let l = Ledger::open(&dir, 9_000).unwrap();
        assert!(l.wants_continue("w"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
