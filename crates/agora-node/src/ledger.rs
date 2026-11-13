//! The node's session ledger (ADR-0060): what it must remember about its agents across its own restart, so an agent that was
//! running when the node stopped can be offered again (Resume, or Start again in the same folder).
//!
//! One file, `<data>/sessions/ledger.json`, readable by the owner only (file `0600` in a `0700` folder on Unix, an owner-only ACL on
//! Windows, both through `agora-platform`), replaced whole on every write (a temporary file in the same folder, then a rename). It holds
//! per agent: id, kind, project, folder (its workspace or its own worktree), parent agent, the agent's own session id when one was
//! reported, start time, last state, and whether it ended. Never terminal contents, prompts, or secrets.
//!
//! The ledger learns from the events the node records (see [`Ledger::observe`]), so every way an agent starts, reports its session,
//! or ends is covered without a call at each place. It writes when an agent starts, when its session id is reported, and when it
//! ends; a state change is kept in memory and written with the next write (and when the node stops cleanly, [`Ledger::flush`]).
//! An agent the node stops with itself does not end: that is exactly what makes it interrupted at the next start.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

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
    pub started_at_ms: u64,
    pub updated_at_ms: u64,
    /// `working`, `blocked`, `idle`, `done` or `unknown`.
    pub last_state: String,
    /// It ended while the node ran (exited, stopped, given up). One that did not was interrupted by the node stopping.
    pub ended: bool,
    /// When the node found it interrupted; 0 while it runs or after it ended.
    pub interrupted_at_ms: u64,
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
    /// Something changed that is not on disk yet.
    dirty: bool,
}

/// See the module documentation. Every method takes the ledger's own lock alone and never another one while holding it.
pub struct Ledger {
    /// `None`: kept in memory only (tests, a node without a data folder).
    path: Option<PathBuf>,
    state: Mutex<State>,
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
            "started_at_ms": self.started_at_ms,
            "updated_at_ms": self.updated_at_ms,
            "last_state": self.last_state,
            "ended": self.ended,
            "interrupted_at_ms": self.interrupted_at_ms,
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
            session_id: field(v, "session_id"),
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
            started_at_ms: number(v, "started_at_ms"),
            updated_at_ms: number(v, "updated_at_ms"),
            last_state: field(v, "last_state"),
            ended: flag(v, "ended"),
            interrupted_at_ms: number(v, "interrupted_at_ms"),
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
            state: Mutex::new(State {
                next_id: 1,
                ..State::default()
            }),
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
            state: Mutex::new(state),
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
        let write = {
            let mut st = self.st();
            let write = apply(&mut st, body, now_ms);
            if write {
                prune(&mut st, now_ms);
            }
            write
        };
        if write {
            let _ = self.flush();
        }
    }

    /// Writes what changed since the last write, if anything (the node calls it when it stops cleanly).
    pub fn flush(&self) -> io::Result<()> {
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
            Some(e) if e.session_id != r.session_id && r.session_id.len() <= MAX_FIELD => {
                e.session_id = bounded(&r.session_id);
                e.updated_at_ms = now;
                st.dirty = true;
                true
            }
            _ => false,
        },
        Body::AgentStateChanged(s) => {
            if let Some(e) = st.entries.get_mut(&s.agent_id) {
                let word = state_word(s.state);
                if e.last_state != word {
                    e.last_state = word.to_string();
                    st.dirty = true;
                }
            }
            false
        }
        Body::AgentExited(x) => match st.entries.get_mut(&x.agent_id) {
            Some(e) if !e.ended => {
                e.ended = true;
                e.interrupted_at_ms = 0;
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
    let before = st.entries.len();
    st.entries.retain(|_, e| {
        let running = !e.ended && e.interrupted_at_ms == 0;
        running || now.saturating_sub(e.last_touched()) <= KEEP_MS
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
        }
    }
    let known: BTreeSet<&String> = st.entries.keys().collect();
    st.bound.retain(|a, _| known.contains(a));
    if st.entries.len() != before {
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

    #[test]
    fn an_unreadable_ledger_is_set_aside_not_deleted() {
        let dir = temp("unreadable");
        std::fs::create_dir_all(dir.join(LEDGER_DIR)).unwrap();
        std::fs::write(dir.join(LEDGER_DIR).join(LEDGER_FILE), b"{not json").unwrap();
        let l = Ledger::open(&dir, 77).unwrap();
        assert!(l.interrupted().is_empty());
        assert!(dir
            .join(LEDGER_DIR)
            .join(format!("{LEDGER_FILE}.unreadable-77"))
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
            }),
            3,
        );
        l.flush().unwrap();
        let text = std::fs::read_to_string(dir.join(LEDGER_DIR).join(LEDGER_FILE)).unwrap();
        assert!(!text.contains("secret prompt") && !text.contains("TOKEN"));
    }
}
