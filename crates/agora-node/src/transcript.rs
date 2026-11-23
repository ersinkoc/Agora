//! ADR-0089 decisions 1 to 3 and 7, and ADR-0104 C1: the chat transcript of every structured agent, on the node's own disk, so the
//! conversation is still there after the node restarts (#931). The window reads it as it always did: the node fills the agent's ring
//! ([`crate::chat::ChatBoard`]) from the tail of the file when it takes the agent up from the holder or resumes it, and the ring
//! snapshot a subscriber gets is that ring. No schema: paging (`chat.history`) is ADR-0089's later step.
//!
//! **Where.** `<data>/transcripts/<agent id>.jsonl`, the folder `0700` and each file `0600` (an owner-only ACL on Windows), made through
//! `agora-platform` (`ensure_private_dir`, `open_append_private`, `write_private`). Never sent to the hub or the holder. The folder is
//! inside the node's data folder, which every confined agent's profile protects (`node_private_paths`).
//!
//! **Format.** JSON lines. The first line says the format and its version (and, after the oldest entries were dropped, that it was
//! truncated). Every other line is one version of one entry, or the ids of entries taken back out (a prompt that was not sent, queued
//! prompts that were dropped):
//! ```text
//! {"format":"chat-transcript","version":1}
//! {"n":12,"entry":{"entry_id":12,"at_ms":...,"role":2,"kind":2,"text":"..."},"tool":"read","args":"{...}"}
//! {"n":14,"removed":[13]}
//! ```
//! `entry` holds every field of the protocol's `ChatEntry` under its proto name (enums as their numbers), defaults left out. A tool
//! call's line also keeps its tool name and arguments (`tool`, `args`; the entry's text joins them and may be cut), and the plan's
//! line `"plan":true`, so a call that finishes after a restart updates its one entry in place. `n` is the highest entry id the
//! node had given the agent when the line was written: ids continue from the highest `n` (ADR-0089 decision 4), also past entries
//! that were given an id and never written. A rotated file, and the new file of a fresh start, begin with a line that holds only that
//! (`{"n":H,"removed":[]}`), so the ids go on even when no entry is left in it. A file purged or removed by retention takes its
//! numbering with it: a later resume numbers from 1 again, and every client then holds that node's snapshot. An entry that changes
//! in place (ADR-0090) is written again with the same id; the last line of an id is the entry (ADR-0090 open question 3), and a
//! reader skips a line it cannot parse (a torn last line).
//!
//! **Writes.** Off every agent's path: the chat path only marks which entries changed (an id in a set); the writer thread
//! ([`spawn_writer`]) takes the changed entries under the board's lock once a second, or as soon as 64 KiB of them wait, and encodes,
//! redacts and appends them after letting it go (an entry that changed many times is written once, as it is then). A batch after a
//! write that was cut short starts on a line of its own. A write that fails (a full disk, a folder that cannot be made) drops that batch:
//! the entries stay in the ring, the agent is not touched, and its conversation says once `chat.history_not_saved`. A node that is
//! killed loses at most the last batch (up to a second or 64 KiB).
//!
//! **Bounds** (ADR-0089 decision 2). Per agent 32 MiB by default, lowered (or raised up to 256 MiB) by the node policy
//! `chat.history_max_bytes` (0 writes nothing); over it, the newest half is written to a new file that replaces the old one (whole
//! lines, a header that says `truncated`). The file of an agent that does not run is removed once it was not written for
//! `chat.history_days` (30 by default, never more); a fresh start ("Start again") that went through moves the earlier file aside as
//! `<id>.earlier.jsonl` (replacing an older one), which ages the same way. Over 2 GiB in all, the files of agents that do not run
//! go, oldest first. Checked when the node starts and every hour. `agora history purge` removes the folder.
//!
//! **Secrets** (ADR-0089 decision 7). The values of the secrets this node holds (`<data>/secrets`, and the environment variables its
//! policy passes to agents, ADR-0084) are replaced by
//! `<redacted>` in every text of a line before it is written. A secret the node does not hold cannot be found in what an agent or a
//! tool printed; the file is as sensitive as the terminal, and its mode is what protects it.
//!
//! **Not built here.** A forget at the hub (ADR-0053 `agent.forget`) is answered by the hub and never reaches the node, so it does not
//! remove the file; retention does.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use agora_core::AgentId;
use agora_protocol::v1::{ChatEntry, PlanStep, ToolLocation};
use serde_json::{json, Map, Value};

use crate::credentials::SecretStore;

/// The folder of transcripts in the node's data folder.
pub const TRANSCRIPTS_DIR: &str = "transcripts";
/// The format the first line names, and its version.
pub const FORMAT: &str = "chat-transcript";
pub const VERSION: u64 = 1;
/// ADR-0089 decision 2: per agent, by default and at most; per node.
pub const DEFAULT_MAX_BYTES: u64 = 32 << 20;
pub const HARD_MAX_BYTES: u64 = 256 << 20;
pub const NODE_MAX_BYTES: u64 = 2 << 30;
/// ADR-0089 decision 2: days a file is kept after it was last written, by default and at most (the policy only lowers it).
pub const DEFAULT_DAYS: u64 = 30;
/// ADR-0089 decision 1: a batch at most once per this long, or as soon as this many bytes of entries wait.
pub const BATCH_INTERVAL: Duration = Duration::from_secs(1);
pub const BATCH_BYTES: usize = 64 * 1024;
/// The most of a file's end that seeding reads (the ring keeps at most 256 KiB of text; a line is larger than its text).
pub const TAIL_READ: u64 = 4 << 20;
/// How often retention is checked while the node runs.
pub const SWEEP_EVERY: Duration = Duration::from_secs(60 * 60);
/// The notice an agent's conversation gets, once, when its history could not be written.
pub const NOT_SAVED: &str = "chat.history_not_saved";
/// What a secret's value is replaced by on disk.
pub const REDACTED: &str = "<redacted>";
/// Shorter secret values are not looked for: a token or a key is longer, and a shorter value would match ordinary words and ids
/// (review of #1103, N3: a value `toolu` turned every Anthropic tool call id into `<redacted>_01...`).
const MIN_SECRET: usize = 8;
/// Review of #1103 (N1): the shortest piece of a secret looked for at the edge of a text, where a value split across two entries
/// (the bridge sends what an agent streams in pieces) or across the cut of a long tool output leaves a part on each side. Shorter
/// pieces are left: they are too common at a text's edge to tell from ordinary words.
const MIN_PIECE: usize = 4;
const DAY_MS: u64 = 24 * 60 * 60 * 1000;

/// The bounds the node policy sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Per agent; 0 writes nothing.
    pub max_bytes: u64,
    /// Days after the last write a file of an agent that does not run is kept.
    pub days: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: DEFAULT_MAX_BYTES,
            days: DEFAULT_DAYS,
        }
    }
}

impl Limits {
    /// `chat.history_max_bytes`: a number of bytes, at most [`HARD_MAX_BYTES`]. `None` for a value that is not a number.
    pub fn parse_max_bytes(v: &str) -> Option<u64> {
        v.trim().parse::<u64>().ok().map(|b| b.min(HARD_MAX_BYTES))
    }

    /// `chat.history_days`: a number of days, at most [`DEFAULT_DAYS`] (the policy lowers it). `None` for a value that is not a number.
    pub fn parse_days(v: &str) -> Option<u64> {
        v.trim().parse::<u64>().ok().map(|d| d.min(DEFAULT_DAYS))
    }
}

/// What the node keeps beside an entry so it can change it in place after a restart.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Side {
    /// A tool call's tool name and arguments, as the node holds them apart from the entry's text.
    pub tool: Option<(String, String)>,
    /// This entry is the plan a new plan replaces in place.
    pub plan: bool,
}

/// One line after the header.
#[derive(Debug, Clone, PartialEq)]
pub enum Line {
    Entry(Box<ChatEntry>, Side),
    Removed(Vec<u64>),
}

/// What one agent's transcript adds to its file in one write.
#[derive(Debug, Clone, PartialEq)]
pub struct Batch {
    pub agent: AgentId,
    /// The highest entry id given to the agent so far.
    pub high: u64,
    pub lines: Vec<Line>,
    /// ADR-0089 decision 5: the agent was started again fresh: its earlier file is moved aside, keeping this highest id, before the
    /// lines are written. The writer does both, so nothing else writes the file meanwhile (review of #1103, N8).
    pub fresh: Option<u64>,
}

/// What the file gives back: the newest version of each entry still in it, oldest first, and the highest id ever given.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Seed {
    pub entries: Vec<(ChatEntry, Side)>,
    pub high: u64,
}

/// The transcripts folder of one node.
pub struct Store {
    dir: PathBuf,
    limits: Limits,
    secrets: Option<SecretStore>,
    /// ADR-0084: values the node passes from its own environment (a `credentials.<kind>` name it does not hold in its store).
    env_values: Vec<String>,
    /// The secret values last read, longest first, and when their folder last changed.
    known: Mutex<(Option<SystemTime>, Vec<String>)>,
}

/// The file name of an agent: its id when that is a plain name, never a path.
fn file_stem(agent: &str) -> Option<&str> {
    (!agent.is_empty()
        && agent.len() <= 128
        && !agent.starts_with('.')
        && agent
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.'))
    .then_some(agent)
}

fn header(truncated: bool) -> Vec<u8> {
    let mut v = json!({"format": FORMAT, "version": VERSION});
    if truncated {
        v["truncated"] = Value::Bool(true);
    }
    let mut out = v.to_string().into_bytes();
    out.push(b'\n');
    out
}

/// A line that only says the highest id given (`{"n":high,"removed":[]}`): what a rotated or a new file starts with.
fn mark(high: u64) -> Vec<u8> {
    encode_line(high, &Line::Removed(Vec::new()))
}

fn put_str(m: &mut Map<String, Value>, k: &str, v: &str) {
    if !v.is_empty() {
        m.insert(k.into(), Value::String(v.into()));
    }
}

fn put_u64(m: &mut Map<String, Value>, k: &str, v: u64) {
    if v != 0 {
        m.insert(k.into(), Value::from(v));
    }
}

fn put_i32(m: &mut Map<String, Value>, k: &str, v: i32) {
    if v != 0 {
        m.insert(k.into(), Value::from(v));
    }
}

fn put_bool(m: &mut Map<String, Value>, k: &str, v: bool) {
    if v {
        m.insert(k.into(), Value::Bool(true));
    }
}

/// Every field of a `ChatEntry` under its proto name, defaults left out.
pub fn entry_json(e: &ChatEntry) -> Value {
    let mut m = Map::new();
    put_u64(&mut m, "entry_id", e.entry_id);
    put_u64(&mut m, "at_ms", e.at_ms);
    put_i32(&mut m, "role", e.role);
    put_i32(&mut m, "kind", e.kind);
    put_str(&mut m, "text", &e.text);
    put_str(&mut m, "path", &e.path);
    put_str(&mut m, "code", &e.code);
    if !e.params.is_empty() {
        let params: BTreeMap<&String, &String> = e.params.iter().collect();
        m.insert("params".into(), json!(params));
    }
    put_str(&mut m, "call_id", &e.call_id);
    put_i32(&mut m, "tool_kind", e.tool_kind);
    put_i32(&mut m, "status", e.status);
    if !e.paths.is_empty() {
        m.insert(
            "paths".into(),
            Value::Array(
                e.paths
                    .iter()
                    .map(|p| json!({"path": p.path, "line": p.line}))
                    .collect(),
            ),
        );
    }
    put_str(&mut m, "output", &e.output);
    put_bool(&mut m, "output_truncated", e.output_truncated);
    put_u64(&mut m, "started_ms", e.started_ms);
    put_u64(&mut m, "ended_ms", e.ended_ms);
    if !e.plan_steps.is_empty() {
        m.insert(
            "plan_steps".into(),
            Value::Array(
                e.plan_steps
                    .iter()
                    .map(|s| json!({"text": s.text, "status": s.status, "priority": s.priority}))
                    .collect(),
            ),
        );
    }
    put_bool(&mut m, "queued", e.queued);
    put_str(&mut m, "approval_id", &e.approval_id);
    put_i32(&mut m, "permission_outcome", e.permission_outcome);
    put_bool(&mut m, "permission_always", e.permission_always);
    Value::Object(m)
}

fn get_str(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

fn get_u64(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(Value::as_u64).unwrap_or(0)
}

fn get_i32(v: &Value, k: &str) -> i32 {
    v.get(k)
        .and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok())
        .unwrap_or(0)
}

fn get_bool(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}

/// The entry a line's `entry` holds; `None` without an id.
pub fn entry_from_json(v: &Value) -> Option<ChatEntry> {
    let entry_id = get_u64(v, "entry_id");
    if entry_id == 0 {
        return None;
    }
    Some(ChatEntry {
        entry_id,
        at_ms: get_u64(v, "at_ms"),
        role: get_i32(v, "role"),
        kind: get_i32(v, "kind"),
        text: get_str(v, "text"),
        path: get_str(v, "path"),
        code: get_str(v, "code"),
        params: v
            .get("params")
            .and_then(Value::as_object)
            .map(|o| {
                o.iter()
                    .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                    .collect()
            })
            .unwrap_or_default(),
        call_id: get_str(v, "call_id"),
        tool_kind: get_i32(v, "tool_kind"),
        status: get_i32(v, "status"),
        paths: v
            .get("paths")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|p| ToolLocation {
                        path: get_str(p, "path"),
                        line: u32::try_from(get_u64(p, "line")).unwrap_or(0),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        output: get_str(v, "output"),
        output_truncated: get_bool(v, "output_truncated"),
        started_ms: get_u64(v, "started_ms"),
        ended_ms: get_u64(v, "ended_ms"),
        plan_steps: v
            .get("plan_steps")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|s| PlanStep {
                        text: get_str(s, "text"),
                        status: get_i32(s, "status"),
                        priority: get_i32(s, "priority"),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        queued: get_bool(v, "queued"),
        approval_id: get_str(v, "approval_id"),
        permission_outcome: get_i32(v, "permission_outcome"),
        permission_always: get_bool(v, "permission_always"),
    })
}

/// One line, encoded (with its line break).
pub fn encode_line(high: u64, line: &Line) -> Vec<u8> {
    let v = match line {
        Line::Entry(e, side) => {
            let mut m = Map::new();
            m.insert("n".into(), Value::from(high));
            m.insert("entry".into(), entry_json(e));
            if let Some((tool, args)) = &side.tool {
                m.insert("tool".into(), Value::String(tool.clone()));
                m.insert("args".into(), Value::String(args.clone()));
            }
            if side.plan {
                m.insert("plan".into(), Value::Bool(true));
            }
            Value::Object(m)
        }
        Line::Removed(ids) => json!({"n": high, "removed": ids}),
    };
    let mut out = v.to_string().into_bytes();
    out.push(b'\n');
    out
}

/// One line read back with its `n`; `None` for the header and for a line that cannot be read (a torn last line).
pub fn decode_line(text: &str) -> Option<(u64, Line)> {
    let v: Value = serde_json::from_str(text).ok()?;
    let n = get_u64(&v, "n");
    if let Some(e) = v.get("entry") {
        let e = entry_from_json(e)?;
        let tool = match (v.get("tool"), v.get("args")) {
            (Some(Value::String(t)), Some(Value::String(a))) => Some((t.clone(), a.clone())),
            _ => None,
        };
        let side = Side {
            tool,
            plan: get_bool(&v, "plan"),
        };
        return Some((n, Line::Entry(Box::new(e), side)));
    }
    let removed = v.get("removed")?.as_array()?;
    Some((
        n,
        Line::Removed(removed.iter().filter_map(Value::as_u64).collect()),
    ))
}

/// The newest version of each entry in `text` (whole lines; a first partial line is the caller's to skip) and the highest id given.
pub fn fold(text: &str) -> Seed {
    let mut by_id: BTreeMap<u64, (ChatEntry, Side)> = BTreeMap::new();
    let mut high = 0;
    for line in text.lines() {
        let Some((n, line)) = decode_line(line) else {
            continue;
        };
        high = high.max(n);
        match line {
            Line::Entry(e, side) => {
                high = high.max(e.entry_id);
                by_id.insert(e.entry_id, (*e, side));
            }
            Line::Removed(ids) => {
                for id in ids {
                    high = high.max(id);
                    by_id.remove(&id);
                }
            }
        }
    }
    Seed {
        entries: by_id.into_values().collect(),
        high,
    }
}

fn redact(s: &mut String, known: &[String]) {
    for k in known {
        if s.contains(k.as_str()) {
            *s = s.replace(k.as_str(), REDACTED);
        }
    }
}

/// The longest piece (at least [`MIN_PIECE`] bytes, shorter than the whole value) of a known value that `s` ends with as its start,
/// as a byte count; 0 for none.
fn ends_with_start(s: &str, known: &[String]) -> usize {
    let b = s.as_bytes();
    let mut best = 0;
    for k in known {
        let k = k.as_bytes();
        let longest = (k.len() - 1).min(b.len());
        for n in (MIN_PIECE.max(best + 1)..=longest).rev() {
            if b[b.len() - n..] == k[..n] && s.is_char_boundary(b.len() - n) {
                best = n;
                break;
            }
        }
    }
    best
}

/// The longest piece of a known value that `s` starts with as its end, as a byte count; 0 for none.
fn starts_with_end(s: &str, known: &[String]) -> usize {
    let b = s.as_bytes();
    let mut best = 0;
    for k in known {
        let k = k.as_bytes();
        let longest = (k.len() - 1).min(b.len());
        for n in (MIN_PIECE.max(best + 1)..=longest).rev() {
            if b[..n] == k[k.len() - n..] && s.is_char_boundary(n) {
                best = n;
                break;
            }
        }
    }
    best
}

/// Review of #1103 (N1): [`redact`], and a piece of a known value at either edge of `s` (what is left of a value split across two
/// entries) replaced too.
fn redact_split(s: &mut String, known: &[String]) {
    redact(s, known);
    let end = ends_with_start(s, known);
    if end > 0 {
        s.truncate(s.len() - end);
        s.push_str(REDACTED);
    }
    let start = starts_with_end(s, known);
    if start > 0 {
        s.replace_range(..start, REDACTED);
    }
}

/// A tool output the node cut (head and tail kept, `chat::MAX_OUTPUT_BYTES` in all): the head ends within 3 bytes before half of it
/// (on a character boundary), so a value cut there is looked for on both sides of each such place.
fn redact_cut_output(s: &mut String, known: &[String]) {
    let half = crate::chat::MAX_OUTPUT_BYTES / 2;
    for cut in (half.saturating_sub(3)..=half.min(s.len())).rev() {
        if !s.is_char_boundary(cut) {
            continue;
        }
        let (head, tail) = s.split_at(cut);
        let (end, start) = (ends_with_start(head, known), starts_with_end(tail, known));
        if end > 0 || start > 0 {
            let out = format!(
                "{}{}{}",
                &head[..head.len() - end],
                if end > 0 || start > 0 { REDACTED } else { "" },
                &tail[start..]
            );
            *s = out;
            return;
        }
    }
}

/// `line` with every known secret value replaced in each text it holds.
fn redacted(line: &Line, known: &[String]) -> Line {
    let mut line = line.clone();
    if known.is_empty() {
        return line;
    }
    if let Line::Entry(e, side) = &mut line {
        // Not `call_id` or `approval_id`: ids the node matches later (a call that finishes after a restart), never a value it holds.
        // The text and the output are what an agent streams in pieces: a value split between two entries is found at their edges.
        redact_split(&mut e.text, known);
        redact_split(&mut e.output, known);
        if e.output_truncated {
            redact_cut_output(&mut e.output, known);
        }
        redact(&mut e.path, known);
        for v in e.params.values_mut() {
            redact(v, known);
        }
        for p in &mut e.paths {
            redact(&mut p.path, known);
        }
        for s in &mut e.plan_steps {
            redact(&mut s.text, known);
        }
        if let Some((tool, args)) = &mut side.tool {
            redact(tool, known);
            redact(args, known);
        }
    }
    line
}

impl Store {
    /// The store of a node whose data folder is `data_dir`; `secrets` are the values to keep out of the files.
    pub fn new(data_dir: &Path, limits: Limits, secrets: Option<SecretStore>) -> Self {
        Self {
            dir: data_dir.join(TRANSCRIPTS_DIR),
            limits,
            secrets,
            env_values: Vec::new(),
            known: Mutex::new((None, Vec::new())),
        }
    }

    /// ADR-0084: the values of these environment variables of the node (the names its policy lists for agents, which it passes from
    /// its environment when its store does not hold them) are replaced too.
    pub fn with_env_secrets<'a>(mut self, names: impl IntoIterator<Item = &'a str>) -> Self {
        self.env_values = names
            .into_iter()
            .filter_map(|n| std::env::var(n).ok())
            .filter(|v| v.len() >= MIN_SECRET)
            .collect();
        self.known = Mutex::new((None, Vec::new()));
        self
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// The file of an agent; `None` for an id that is not a plain name.
    pub fn path(&self, agent: &str) -> Option<PathBuf> {
        file_stem(agent).map(|s| self.dir.join(format!("{s}.jsonl")))
    }

    fn earlier_path(&self, agent: &str) -> Option<PathBuf> {
        file_stem(agent).map(|s| self.dir.join(format!("{s}.earlier.jsonl")))
    }

    /// The secret values to replace, read again only when their folder changed.
    fn known_secrets(&self) -> Vec<String> {
        let changed = self
            .secrets
            .as_ref()
            .and_then(|s| std::fs::metadata(s.dir()).and_then(|m| m.modified()).ok());
        let mut g = self.known.lock().unwrap_or_else(|e| e.into_inner());
        if g.0 != changed || changed.is_none() {
            let mut values: Vec<String> = self
                .secrets
                .iter()
                .flat_map(|store| {
                    store
                        .list()
                        .iter()
                        .filter_map(|n| store.get(n))
                        .map(|s| s.expose().to_string())
                        .collect::<Vec<_>>()
                })
                .filter(|v| v.len() >= MIN_SECRET)
                .chain(self.env_values.iter().cloned())
                .collect();
            // Review of #1103 (N2): a tool's arguments and a call's text are JSON, where a value with `"` or `\` is escaped.
            let escaped: Vec<String> = values
                .iter()
                .filter_map(|v| {
                    let e = serde_json::to_string(v).ok()?;
                    let e = e.get(1..e.len() - 1)?.to_string();
                    (e != *v).then_some(e)
                })
                .collect();
            values.extend(escaped);
            values.sort_by_key(|v| std::cmp::Reverse(v.len()));
            values.dedup();
            *g = (changed, values);
        }
        g.1.clone()
    }

    /// Appends a batch to its agent's file (made private, with its header, when new), then keeps the file within its bound.
    pub fn write(&self, batch: &Batch) -> io::Result<()> {
        if let Some(high) = batch.fresh {
            self.move_aside(batch.agent.as_str(), high);
        }
        if self.limits.max_bytes == 0 || batch.lines.is_empty() {
            return Ok(());
        }
        let path = self
            .path(batch.agent.as_str())
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
        agora_platform::ensure_private_dir(&self.dir)?;
        let known = self.known_secrets();
        let mut out = Vec::new();
        for line in &batch.lines {
            out.extend(encode_line(batch.high, &redacted(line, &known)));
        }
        let mut f = agora_platform::open_append_private(&path)?;
        let len = f.metadata()?.len();
        if len == 0 {
            f.write_all(&header(false))?;
        } else if !ends_a_line(&path, len) {
            // A write cut short (a full disk) left half a line: ended here, so the next line is whole and only the torn one is lost.
            f.write_all(b"\n")?;
        }
        f.write_all(&out)?;
        f.flush()?;
        if len + out.len() as u64 > self.limits.max_bytes {
            drop(f);
            self.rotate(&path, batch.high)?;
        }
        Ok(())
    }

    /// ADR-0089 decision 2: the newest half of the bound, whole lines, in a new file that replaces the old one and says it was cut.
    /// The line after the header holds `high`, the highest id given, so the ids go on past it even when no whole entry fits (review
    /// of #1103: one entry larger than the half kept left a file with no `n` at all).
    fn rotate(&self, path: &Path, high: u64) -> io::Result<()> {
        let mut f = agora_platform::open_read_no_follow(path)?;
        let len = f.metadata()?.len();
        let keep = self.limits.max_bytes / 2;
        let start = len.saturating_sub(keep);
        f.seek(SeekFrom::Start(start))?;
        let mut tail = Vec::new();
        f.take(keep).read_to_end(&mut tail)?;
        let from = if start == 0 {
            0
        } else {
            tail.iter()
                .position(|b| *b == b'\n')
                .map_or(tail.len(), |i| i + 1)
        };
        let mut out = header(true);
        out.extend(mark(high));
        out.extend_from_slice(&tail[from..]);
        agora_platform::write_private(path, &out)
    }

    /// The newest entries the end of an agent's file holds (at most [`TAIL_READ`] of it). A missing or unreadable file gives nothing.
    pub fn read(&self, agent: &str) -> Seed {
        self.path(agent)
            .and_then(|p| read_tail(&p).ok())
            .unwrap_or_default()
    }

    /// ADR-0089 decision 5: a fresh start of the agent ("Start again") that went through: its file is moved aside as the earlier
    /// session's (`<id>.earlier.jsonl`, replacing an older one), so the new session begins a file of its own. Only after the start
    /// succeeded: a refused one leaves the conversation where a later Resume finds it. The new file starts with `high`, the earlier
    /// session's highest id, so a node that takes the new session up before it wrote anything still numbers on after it.
    pub fn move_aside(&self, agent: &str, high: u64) {
        let (Some(path), Some(earlier)) = (self.path(agent), self.earlier_path(agent)) else {
            return;
        };
        if let Err(e) = std::fs::rename(&path, &earlier) {
            if e.kind() != io::ErrorKind::NotFound {
                // Operational log, not UI text. The new session is written after the earlier one in the same file then.
                eprintln!("transcripts: {agent}: the earlier session was not moved aside: {e}");
                return;
            }
        }
        if high > 0 && self.limits.max_bytes > 0 {
            let mut out = header(false);
            out.extend(mark(high));
            if let Err(e) = agora_platform::write_private(&path, &out) {
                // Operational log, not UI text. The ids still go on in this node's memory; the first batch writes them.
                eprintln!("transcripts: {agent}: the new session's file was not started: {e}");
            }
        }
    }

    /// ADR-0089 decision 2: removes the files of agents that do not run (`running` holds the ids that do) not written for
    /// [`Limits::days`], then, over [`NODE_MAX_BYTES`] in all, the oldest files of agents that do not run. Returns how many went.
    pub fn sweep(&self, running: &HashSet<String>, now: SystemTime) -> usize {
        let Ok(dir) = std::fs::read_dir(&self.dir) else {
            return 0;
        };
        let keep_ms = self.limits.days.saturating_mul(DAY_MS);
        let mut files: Vec<(SystemTime, u64, PathBuf, bool)> = Vec::new();
        let mut removed = 0;
        for item in dir.flatten() {
            let name = item.file_name().to_string_lossy().into_owned();
            let Some(stem) = name.strip_suffix(".jsonl") else {
                continue;
            };
            let (agent, earlier) = match stem.strip_suffix(".earlier") {
                Some(a) => (a, true),
                None => (stem, false),
            };
            let Ok(meta) = item.metadata() else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            let modified = meta.modified().unwrap_or(now);
            let live = !earlier && running.contains(agent);
            let age = now
                .duration_since(modified)
                .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
            if !live && age >= keep_ms {
                if std::fs::remove_file(item.path()).is_ok() {
                    removed += 1;
                }
                continue;
            }
            files.push((modified, meta.len(), item.path(), live));
        }
        let mut total: u64 = files.iter().map(|f| f.1).sum();
        files.sort_by_key(|f| f.0);
        for (_, len, path, live) in files {
            if total <= NODE_MAX_BYTES {
                break;
            }
            if !live && std::fs::remove_file(&path).is_ok() {
                total = total.saturating_sub(len);
                removed += 1;
            }
        }
        removed
    }
}

/// The file's last byte is a line break (or cannot be read: then a line break is written, which a reader skips as an empty line).
fn ends_a_line(path: &Path, len: u64) -> bool {
    let mut last = [0u8; 1];
    agora_platform::open_read_no_follow(path)
        .and_then(|mut f| {
            f.seek(SeekFrom::Start(len.saturating_sub(1)))?;
            f.read_exact(&mut last)
        })
        .is_ok_and(|()| last[0] == b'\n')
}

fn read_tail(path: &Path) -> io::Result<Seed> {
    let mut f = agora_platform::open_read_no_follow(path)?;
    let meta = f.metadata()?;
    if !meta.is_file() {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    let start = meta.len().saturating_sub(TAIL_READ);
    f.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    f.take(TAIL_READ).read_to_end(&mut bytes)?;
    let from = if start == 0 {
        0
    } else {
        bytes
            .iter()
            .position(|b| *b == b'\n')
            .map_or(bytes.len(), |i| i + 1)
    };
    Ok(fold(&String::from_utf8_lossy(&bytes[from..])))
}

/// ADR-0089 decision 3: `agora history purge`: every transcript of the node whose data folder is `data_dir`. Fine when there is none.
pub fn purge_all(data_dir: &Path) -> io::Result<()> {
    match std::fs::remove_dir_all(data_dir.join(TRANSCRIPTS_DIR)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Wakes the writer early (64 KiB waiting, or the node stopping).
#[derive(Default)]
pub struct Wake {
    due: Mutex<bool>,
    cv: std::sync::Condvar,
}

impl Wake {
    pub fn notify(&self) {
        *self.due.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.cv.notify_one();
    }

    /// Waits until woken or `timeout` passed.
    pub fn wait(&self, timeout: Duration) {
        let mut due = self.due.lock().unwrap_or_else(|e| e.into_inner());
        if !*due {
            due = self
                .cv
                .wait_timeout(due, timeout)
                .map(|(g, _)| g)
                .unwrap_or_else(|e| e.into_inner().0);
        }
        *due = false;
    }
}

/// Runs the writer of `board` until `stop`: a batch once per [`BATCH_INTERVAL`] or when woken, and retention at its start and every
/// [`SWEEP_EVERY`] (`running` names the agents that run). A failed write is said once in that agent's conversation. Nothing is
/// written when it stops: ADR-0104 O2 (a later step) flushes on a clean stop.
pub fn spawn_writer(
    board: Arc<crate::chat::ChatBoard>,
    running: Box<dyn Fn() -> HashSet<String> + Send>,
    stop: Arc<AtomicBool>,
) -> io::Result<Option<std::thread::JoinHandle<()>>> {
    let (Some(store), Some(wake)) = (board.store(), board.wake()) else {
        return Ok(None);
    };
    std::thread::Builder::new()
        .name("agora-transcripts".into())
        .spawn(move || {
            let mut swept: Option<std::time::Instant> = None;
            let mut failed: HashMap<AgentId, ()> = HashMap::new();
            while !stop.load(Ordering::Acquire) {
                if swept.is_none_or(|t| t.elapsed() >= SWEEP_EVERY) {
                    store.sweep(&running(), SystemTime::now());
                    swept = Some(std::time::Instant::now());
                }
                wake.wait(BATCH_INTERVAL);
                if stop.load(Ordering::Acquire) {
                    break;
                }
                for batch in board.take_unsaved() {
                    match store.write(&batch) {
                        Ok(()) => {
                            failed.remove(&batch.agent);
                        }
                        Err(e) => {
                            if failed.insert(batch.agent.clone(), ()).is_none() {
                                // Operational log, not UI text; the person is told by the notice.
                                eprintln!(
                                    "transcripts: {}: history not written: {e}",
                                    batch.agent.as_str()
                                );
                            }
                            board.not_saved(&batch.agent, crate::daemon::now_ms());
                        }
                    }
                }
            }
        })
        .map(Some)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use agora_protocol::v1::{ChatKind, ChatRole};
    use prost::Message;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("agora-tr-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn text(id: u64, t: &str) -> Line {
        Line::Entry(
            Box::new(ChatEntry {
                entry_id: id,
                at_ms: id * 10,
                role: ChatRole::Agent as i32,
                kind: ChatKind::Text as i32,
                text: t.into(),
                ..Default::default()
            }),
            Side::default(),
        )
    }

    fn batch(agent: &str, high: u64, lines: Vec<Line>) -> Batch {
        Batch {
            agent: AgentId::new(agent),
            high,
            lines,
            fresh: None,
        }
    }

    /// Every field set, so a field the line leaves out shows as a difference in the encoded protobuf.
    fn full() -> ChatEntry {
        ChatEntry {
            entry_id: 7,
            at_ms: 1_700_000_000_000,
            role: 2,
            kind: 3,
            text: "t \"q\" \n é".into(),
            path: "src/a.rs".into(),
            code: "adapter.x".into(),
            params: [("a".to_string(), "1".to_string()), ("b".into(), "2".into())].into(),
            call_id: "c1".into(),
            tool_kind: 4,
            status: 2,
            paths: vec![ToolLocation {
                path: "p".into(),
                line: 9,
            }],
            output: "out".into(),
            output_truncated: true,
            started_ms: 5,
            ended_ms: 6,
            plan_steps: vec![PlanStep {
                text: "s".into(),
                status: 1,
                priority: 2,
            }],
            queued: true,
            approval_id: "ap".into(),
            permission_outcome: 3,
            permission_always: true,
        }
    }

    #[test]
    fn every_field_of_an_entry_survives_its_line() {
        let side = Side {
            tool: Some(("read".into(), "{\"p\":1}".into())),
            plan: true,
        };
        let line = encode_line(9, &Line::Entry(Box::new(full()), side.clone()));
        let text = String::from_utf8(line).unwrap();
        assert!(text.ends_with('\n') && text.matches('\n').count() == 1);
        let (n, back) = decode_line(text.trim_end()).unwrap();
        assert_eq!(n, 9);
        let Line::Entry(e, s) = back else {
            panic!("an entry line")
        };
        // Compared whole (the params map has no order on the wire, so not byte for byte).
        assert_eq!(*e, full());
        assert_eq!(e.encoded_len(), full().encoded_len());
        assert_eq!(s, side);
        // The proto's field count: a field added to `ChatEntry` must be added to the line (and to `full`) too.
        let fields = entry_json(&full()).as_object().unwrap().len();
        assert_eq!(fields, 21);
    }

    #[test]
    fn the_last_version_of_an_id_wins_removed_ids_go_and_ids_continue_past_both() {
        let dir = temp("fold");
        let store = Store::new(&dir, Limits::default(), None);
        store
            .write(&batch(
                "agent-1",
                3,
                vec![text(1, "a"), text(2, "b"), text(3, "c")],
            ))
            .unwrap();
        store
            .write(&batch(
                "agent-1",
                6,
                vec![text(2, "b2"), Line::Removed(vec![3, 5])],
            ))
            .unwrap();
        let seed = store.read("agent-1");
        let got: Vec<(u64, &str)> = seed
            .entries
            .iter()
            .map(|(e, _)| (e.entry_id, e.text.as_str()))
            .collect();
        assert_eq!(got, [(1, "a"), (2, "b2")]);
        // 6 was given (and never written): the next id is 7, not 3 or 4.
        assert_eq!(seed.high, 6);
        assert!(store.read("agent-2").entries.is_empty());
        assert_eq!(store.read("agent-2").high, 0);
    }

    #[test]
    fn a_torn_last_line_and_a_foreign_line_are_skipped() {
        let dir = temp("torn");
        let store = Store::new(&dir, Limits::default(), None);
        store
            .write(&batch("a", 2, vec![text(1, "x"), text(2, "y")]))
            .unwrap();
        let path = store.path("a").unwrap();
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        f.write_all(b"not json\n{\"n\":3,\"entry\":{\"entry_id\":3,\"te")
            .unwrap();
        let seed = store.read("a");
        assert_eq!(seed.entries.len(), 2);
        assert_eq!(seed.high, 2);
    }

    /// A write cut short left half a line: the next batch starts on a line of its own, so only the torn line is lost.
    #[test]
    fn a_batch_after_a_torn_line_is_read_whole() {
        let dir = temp("torn-write");
        let store = Store::new(&dir, Limits::default(), None);
        store.write(&batch("a", 1, vec![text(1, "x")])).unwrap();
        let path = store.path("a").unwrap();
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        f.write_all(b"{\"n\":2,\"entry\":{\"entry_id\":2,\"te")
            .unwrap();
        store.write(&batch("a", 3, vec![text(3, "z")])).unwrap();
        let seed = store.read("a");
        let ids: Vec<u64> = seed.entries.iter().map(|(e, _)| e.entry_id).collect();
        assert_eq!(ids, [1, 3]);
        assert_eq!(seed.high, 3);
    }

    #[test]
    fn values_the_node_passes_from_its_environment_are_replaced_too() {
        let dir = temp("env-secret");
        // A variable every test process has, so nothing here sets the environment.
        let value = std::env::var("PATH").unwrap();
        let store = Store::new(&dir, Limits::default(), Some(SecretStore::in_data(&dir)))
            .with_env_secrets(["PATH", "AGORA_TRANSCRIPT_TEST_UNSET"]);
        store
            .write(&batch(
                "a",
                1,
                vec![text(1, &format!("it is {value} here"))],
            ))
            .unwrap();
        let body = std::fs::read_to_string(store.path("a").unwrap()).unwrap();
        assert!(body.contains(&format!("it is {REDACTED} here")), "{body}");
    }

    #[cfg(unix)]
    #[test]
    fn the_folder_and_the_file_are_the_owners_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp("mode");
        let store = Store::new(&dir, Limits::default(), None);
        store.write(&batch("a", 1, vec![text(1, "x")])).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(store.dir()), 0o700);
        assert_eq!(mode(&store.path("a").unwrap()), 0o600);
        let first = std::fs::read_to_string(store.path("a").unwrap()).unwrap();
        assert!(first.starts_with("{\"format\":\"chat-transcript\",\"version\":1}\n"));
    }

    #[test]
    fn an_id_that_is_not_a_plain_name_has_no_file() {
        let store = Store::new(
            &std::env::temp_dir().join("agora-tr-names"),
            Limits::default(),
            None,
        );
        for bad in ["", "../x", "a/b", ".hidden", "a\\b"] {
            assert!(store.path(bad).is_none(), "{bad}");
        }
        assert!(store.path("agent-7").is_some());
    }

    #[test]
    fn over_its_bound_a_file_keeps_its_newest_whole_lines_and_says_it_was_cut() {
        let dir = temp("rotate");
        let limits = Limits {
            max_bytes: 8 * 1024,
            days: 30,
        };
        let store = Store::new(&dir, limits, None);
        for i in 1..=200u64 {
            store
                .write(&batch("a", i, vec![text(i, &"z".repeat(100))]))
                .unwrap();
        }
        let path = store.path("a").unwrap();
        let len = std::fs::metadata(&path).unwrap().len();
        assert!(len <= limits.max_bytes, "{len}");
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(
            body.starts_with("{\"format\":\"chat-transcript\",\"truncated\":true,\"version\":1}\n")
                || body.starts_with(
                    "{\"format\":\"chat-transcript\",\"version\":1,\"truncated\":true}\n"
                ),
            "{}",
            &body[..80]
        );
        let seed = store.read("a");
        assert_eq!(seed.high, 200);
        assert_eq!(seed.entries.last().unwrap().0.entry_id, 200);
        assert!(
            seed.entries.first().unwrap().0.entry_id > 1,
            "the oldest went"
        );
        // Every line after the header is whole.
        assert!(body.lines().skip(1).all(|l| decode_line(l).is_some()));
        // 0 writes nothing at all.
        let none = Store::new(
            &dir,
            Limits {
                max_bytes: 0,
                days: 30,
            },
            None,
        );
        none.write(&batch("b", 1, vec![text(1, "x")])).unwrap();
        assert!(!none.path("b").unwrap().exists());
    }

    /// Review of #1103: a rotation that keeps no whole entry (one entry larger than the half it keeps) still says the highest id given,
    /// so a node that takes the agent up after it never gives an id again.
    #[test]
    fn a_rotation_that_keeps_no_entry_keeps_the_highest_id() {
        let dir = temp("rotate-empty");
        let limits = Limits {
            max_bytes: 1024,
            days: 30,
        };
        let store = Store::new(&dir, limits, None);
        store
            .write(&batch("a", 41, vec![text(41, &"s".repeat(32))]))
            .unwrap();
        assert_eq!(store.read("a").high, 41);
        store
            .write(&batch("a", 42, vec![text(42, &"l".repeat(1500))]))
            .unwrap();
        let seed = Store::new(&dir, limits, None).read("a");
        assert!(seed.entries.is_empty());
        assert_eq!(seed.high, 42);
        // And the next rotation after it, from a file that holds only the mark.
        store
            .write(&batch("a", 43, vec![text(43, &"m".repeat(1500))]))
            .unwrap();
        assert_eq!(Store::new(&dir, limits, None).read("a").high, 43);
    }

    /// "Start again" moves the file aside; the new session's file starts with the earlier session's highest id, so a node that takes
    /// up or resumes the new session before it wrote anything still numbers on after it.
    #[test]
    fn a_file_moved_aside_leaves_the_highest_id_behind() {
        let dir = temp("aside-mark");
        let store = Store::new(&dir, Limits::default(), None);
        store.write(&batch("a", 9, vec![text(9, "x")])).unwrap();
        store.move_aside("a", 9);
        let seed = store.read("a");
        assert!(seed.entries.is_empty());
        assert_eq!(seed.high, 9);
        assert_eq!(store.read("a.earlier").entries.len(), 1);
        // A new entry is appended after the mark, and the file stays private.
        store.write(&batch("a", 10, vec![text(10, "y")])).unwrap();
        assert_eq!(store.read("a").entries.len(), 1);
    }

    #[test]
    fn the_nodes_own_secret_values_never_reach_the_file() {
        let dir = temp("secret");
        let secrets = SecretStore::in_data(&dir);
        secrets
            .set(
                "TOKEN",
                &crate::credentials::Secret::new("sk-very-secret-1234").unwrap(),
            )
            .unwrap();
        let store = Store::new(&dir, Limits::default(), Some(SecretStore::in_data(&dir)));
        let mut e = full();
        e.output = "the key is sk-very-secret-1234.".into();
        e.params.insert("v".into(), "sk-very-secret-1234".into());
        let side = Side {
            tool: Some(("sh".into(), "echo sk-very-secret-1234".into())),
            plan: false,
        };
        store
            .write(&batch("a", 7, vec![Line::Entry(Box::new(e), side)]))
            .unwrap();
        let body = std::fs::read_to_string(store.path("a").unwrap()).unwrap();
        assert!(!body.contains("sk-very-secret-1234"));
        assert_eq!(body.matches(REDACTED).count(), 3, "{body}");
    }

    fn store_with_secret(tag: &str, value: &str) -> (PathBuf, Store) {
        let dir = temp(tag);
        SecretStore::in_data(&dir)
            .set("TOKEN", &crate::credentials::Secret::new(value).unwrap())
            .unwrap();
        let store = Store::new(&dir, Limits::default(), Some(SecretStore::in_data(&dir)));
        (dir, store)
    }

    /// Review of #1103 (N1): what an agent streams arrives in pieces, so a value can be split between two entries (here written in
    /// two batches); neither piece reaches the file.
    #[test]
    fn a_secret_split_between_two_entries_is_replaced_on_both_sides() {
        let (_dir, store) = store_with_secret("secret-split", "sk-very-secret-1234");
        store.write(&batch("a", 1, vec![text(1, "my key is sk-very-se")])).unwrap();
        store.write(&batch("a", 2, vec![text(2, "cret-1234, keep it")])).unwrap();
        let body = std::fs::read_to_string(store.path("a").unwrap()).unwrap();
        assert!(!body.contains("sk-very-se") && !body.contains("cret-1234"), "{body}");
        let seed = store.read("a");
        assert_eq!(seed.entries[0].0.text, format!("my key is {REDACTED}"));
        assert_eq!(seed.entries[1].0.text, format!("{REDACTED}, keep it"));
    }

    /// Review of #1103 (N1): a long tool output is cut in its middle (head and tail kept); a value on the cut is replaced on both sides.
    #[test]
    fn a_secret_on_the_cut_of_a_long_output_is_replaced() {
        let (_dir, store) = store_with_secret("secret-cut", "sk-very-secret-1234");
        let half = crate::chat::MAX_OUTPUT_BYTES / 2;
        let mut e = full();
        // The head ends with the value's start, the middle (`cre`) was cut out, the tail starts with its end.
        e.output = format!("{}sk-very-se{}", "a".repeat(half - 10), "t-1234 the end");
        e.output_truncated = true;
        store
            .write(&batch("a", 7, vec![Line::Entry(Box::new(e), Side::default())]))
            .unwrap();
        let body = std::fs::read_to_string(store.path("a").unwrap()).unwrap();
        assert!(!body.contains("sk-very-se") && !body.contains("t-1234"), "{body}");
        assert!(body.contains(&format!("a{REDACTED} the end")));
    }

    /// Review of #1103 (N2, N3): a value with `"` or `\` is found where it is JSON-escaped (a tool's arguments); ids are never
    /// changed, and a value shorter than 8 bytes is not looked for.
    #[test]
    fn escaped_values_are_found_and_ids_and_short_values_are_left() {
        let (dir, store) = store_with_secret("secret-escaped", "pa\"ss\\word-99");
        let mut e = full();
        e.call_id = "toolu_01pa\"ss\\word-99".into();
        let side = Side {
            tool: Some(("sh".into(), r#"{"pw":"pa\"ss\\word-99"}"#.into())),
            plan: false,
        };
        store
            .write(&batch("a", 7, vec![Line::Entry(Box::new(e.clone()), side)]))
            .unwrap();
        let seed = store.read("a");
        let (got, side) = &seed.entries[0];
        assert_eq!(side.tool.as_ref().unwrap().1, format!(r#"{{"pw":"{REDACTED}"}}"#));
        assert_eq!(got.call_id, e.call_id, "an id is never changed");
        SecretStore::in_data(&dir)
            .set("SHORT", &crate::credentials::Secret::new("toolu").unwrap())
            .unwrap();
        let store = Store::new(&dir, Limits::default(), Some(SecretStore::in_data(&dir)));
        store.write(&batch("b", 1, vec![text(1, "toolu_01abc")])).unwrap();
        assert_eq!(store.read("b").entries[0].0.text, "toolu_01abc");
    }

    #[test]
    fn retention_removes_what_does_not_run_and_was_not_written_for_its_days() {
        let dir = temp("sweep");
        let store = Store::new(
            &dir,
            Limits {
                max_bytes: 1 << 20,
                days: 2,
            },
            None,
        );
        for a in ["old", "running", "fresh"] {
            store.write(&batch(a, 1, vec![text(1, "x")])).unwrap();
        }
        assert_eq!(store.read("fresh").high, 1);
        store.move_aside("fresh", 1);
        assert!(store.earlier_path("fresh").unwrap().exists());
        // The new session's file holds only the earlier session's highest id.
        assert!(store.read("fresh").entries.is_empty());
        let running: HashSet<String> = ["running".to_string()].into();
        // Nothing is old yet.
        assert_eq!(store.sweep(&running, SystemTime::now()), 0);
        // Three days on: everything but the running agent's file goes, both of the fresh agent's too.
        let later = SystemTime::now() + Duration::from_secs(3 * 24 * 3600);
        assert_eq!(store.sweep(&running, later), 3);
        assert!(store.path("running").unwrap().exists());
        assert!(!store.path("old").unwrap().exists());
        assert!(!store.earlier_path("fresh").unwrap().exists());
        assert!(!store.path("fresh").unwrap().exists());
        purge_all(&dir).unwrap();
        assert!(!store.dir().exists());
        purge_all(&dir).unwrap();
    }

    #[test]
    fn policy_values_are_bounded() {
        assert_eq!(Limits::parse_max_bytes("1000"), Some(1000));
        assert_eq!(
            Limits::parse_max_bytes("999999999999"),
            Some(HARD_MAX_BYTES)
        );
        assert_eq!(Limits::parse_max_bytes("lots"), None);
        assert_eq!(Limits::parse_days("7"), Some(7));
        assert_eq!(Limits::parse_days("90"), Some(DEFAULT_DAYS));
        assert_eq!(Limits::parse_days("-1"), None);
    }
}
