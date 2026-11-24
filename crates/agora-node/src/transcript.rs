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
//! write that was cut short starts on a line of its own. A write that fails (a full disk, a folder that cannot be made) drops that
//! batch and is not tried again: the entries stay in the ring, the agent is not touched, and its conversation says once
//! `chat.history_not_saved`. The writer is the only one that writes, rotates or moves a file aside, so none of these races another.
//! While the node holds a secret value and the agent is in a turn, its newest words (less than the longest value's length of them)
//! wait until more words follow or the turn ends (see **Secrets**). A node that is killed loses at most the last batch (up to a
//! second or 64 KiB) and, with a secret value held, those held words.
//!
//! **Seeding** (ADR-0104 C1). On take-up and on resume the ring is filled from the file's end, every entry held to the live path's
//! shapes and bounds (`chat::seeded_entry`) and ids kept below 2^52. A permission still pending there is shown cancelled, and a prompt
//! still queued is dropped with `adapter.queue_dropped` (its queue ended with the old node; one the old node sent in its last unwritten
//! second is said dropped too, as the file cannot tell). A resumed agent that loads its session replays its conversation
//! (ACP `session/load`): that replay is shown only when the ring is empty (review of #1103, B1).
//!
//! **Bounds** (ADR-0089 decision 2). Per agent 32 MiB by default, only lowered by the node policy `chat.history_max_bytes` (0 writes
//! nothing); over it, the newest half is written to a new file that replaces the old one (whole lines, a header that says
//! `truncated`). A file is removed `chat.history_days` (30 by default, only lowered; 0: once its agent ended) after its agent ended,
//! as the session ledger says (the file's time is set to it, so it holds after the ledger forgets the agent), or after its last write
//! for an agent the ledger no longer knows; never while its agent runs or is
//! offered for Resume. A fresh start ("Start again") that went through moves the earlier file aside as `<id>.earlier.jsonl`
//! (replacing an older one), which follows its agent. Over 2 GiB in all, the files of agents that are not kept go, oldest first.
//! Leftovers of a rewrite go. Checked when the node starts and every hour. `agora history purge` removes the folder.
//!
//! **Secrets** (ADR-0089 decision 7; review of #1103, rounds 1 to 4). The values of the secrets this node holds (`<data>/secrets`,
//! and the environment variables its policy passes to agents, ADR-0084), of 8 bytes or more, are kept out of the file by two
//! mechanisms only:
//! - **(a) whole values**, raw and as `serde_json` escapes them, every occurrence (overlapping ones too): in every text of a line, and
//!   in the agent's words (its text and thinking) as one stream across the pieces they came in, joined with the words held back
//!   after them and with the last words written before them (the carry, in memory);
//! - **(b) fragments** at every place a value may have been cut (an edge): every part of a value (4 bytes or more) that touches or
//!   crosses an edge. That covers a value's start or end cut off there, a part cut on both sides, pieces of under 4 bytes beside
//!   several edges that add up, and a part whose bytes sit on both sides of an edge by chance (a value's first letters before words
//!   that were lost, its last letters after them). Found with a suffix automaton of each value, in time linear in the text however
//!   many edges it has.
//!
//! Every place agent text is cut, split, spliced, partly redacted or written before it is final, and what covers it:
//!
//! | Where | What happens to the text | Covered by |
//! | --- | --- | --- |
//! | bridge: message and thought chunks | joined in 150 ms / 4 KiB windows, so a value spans entries | (a) over the stream; its newest words held while the turn runs |
//! | bridge: tool output, cut to 64 KiB | head and tail kept | cleaned as the node cleans it and redacted before the cut, so its length does not change after it; the node keeps 8 + 8 KiB of it and never this place |
//! | bridge: arguments (64 KiB), plan step (4 KiB), location path (4 KiB) | head and tail kept | cleaned as the node cleans that field first; the node keeps less of each (8 + 8 KiB, 256 + 256 bytes, 512 + 512 bytes), never this place |
//! | bridge: diff, cut to 64 KiB | head and tail kept | the node keeps its first 16 KiB, uncleaned: never this place |
//! | bridge: its own shape check (`[redacted]`) | part of a value replaced | (b): both sides of each mark are edges, also of a mark a later cut (the stderr tail's, the node's) went through. Only tool output and stderr are shape-checked, never the agent's words |
//! | bridge: stderr tail (400 bytes, cut at its front and marked `…`) | into a notice's parameters | (b): both ends of every notice parameter are edges, and the place just after a leading `…` (just before a trailing one) |
//! | bridge: the stderr tail's home path (`/home/<name>` written `~`) | spliced | **not covered** across the splice |
//! | bridge: an item whose line would pass the node's 4 MiB limit (`fit_line`) | every string over 256 bytes in it halved, head and tail kept, until it fits | **not covered** (a large plan can reach it) |
//! | node: cleaning, then the 64 KiB cut and each field's cap (output 16 KiB, thought 8 KiB, arguments 16 KiB, tool name and path 1 KiB, plan step, permission label and refused action 512 bytes) | head and tail kept | the place is recorded (`agora_adapters::text::cap_middle` through `chat::tidy_cuts`, after the isolates' balance): (b) |
//! | node: a text, a diff or a prompt cut to 16 KiB; a notice parameter to 1 KiB (also one of digits) | end dropped | recorded (`cap_end`): (b) in the field, and for the agent's words in their stream too, so the next piece (which may go on with the value) starts at an edge; the place stays with the carry for later batches |
//! | node: a tool call's text, a plan's text | made of the call's arguments, the plan's steps | made again by the writer from the redacted parts |
//! | node: a tool call still running | its output and arguments are written as they are, then replaced | (b): the end of a version not final is an edge |
//! | node: a prompt, a queued prompt or a notice between the agent's words | entries between pieces of one value | not a break: the stream is the agent's words only |
//! | node: the ring drops words not written yet (a flood) | kept aside (up to 1 MiB) and written at the next batch | the carry joins them; past 1 MiB the oldest are dropped unwritten: the carry is kept and its end is an edge, so what follows the gap starts at one |
//! | writer: any release of held words with less than a value's length after them (the turn ends, words said outside a turn, a stop's flush `take_unsaved`) | written without what may follow | the end is an edge (b), even at a turn's end (an adapter may say it is idle before its last words); the carry (raw, in memory) joins what follows in this node |
//! | take-up and resume (seeding) | this node does not have the earlier node's raw words | the earlier node's last words as written become the carry (up to 16 KiB): every boundary between them, their end and their `<redacted>` marks are edges. With no file at all (never written, purged, killed before the first batch), the next words start with an edge |
//! | Start again | the earlier session's words flushed to its file, the new session starts | as a stop's flush; the new session's first word is an edge |
//! | rotation | the newest whole lines kept | nothing split |
//!
//! Every cut in the node's chat code, the transcript writer, the bridge and the cut helpers goes through those helpers or is listed
//! with why it needs no record (`chat::every_cut`, which looks for truncations, drains and splits, range slices with or without brackets, and cut helpers named or passed), and
//! a seeded random test drives the whole path (`chat::property`, 600 cases by default, `AGORA_PROPERTY_CASES` and
//! `AGORA_PROPERTY_BASE` for more). Not covered: a value shorter than 8 bytes; a fragment shorter than 4 bytes; a part of 4 bytes or
//! more of a value that stands at an edge only by chance is replaced too (a stop's flush, words said outside a turn, a turn's end, a
//! take-up's boundaries); a value whose characters cleaning changes (`\r\n` written `\n`; line breaks and tabs as spaces in one-line
//! fields) is not found; cuts another adapter makes on its own (#1123), the bridge's `fit_line` halving and the stderr tail's home
//! path splice; JSON forms of a value other than `serde_json`'s escaping (`\/`, `\uXXXX`) in arguments; and a secret the node does not
//! hold. The bridge cleans and redacts a tool's whole output before
//! cutting it (work in proportion to all of it, off the node). Ids (`call_id`, `approval_id`) are never changed: a `call_id` is text
//! the agent chose and is written as it is. What the window is sent live is not redacted (#1147). The matcher of fragments costs
//! about 27 bytes of memory per byte of the values held (13 MiB for 64 values of 8,000 bytes, built in about 70 ms when the values
//! change), kept while the node runs; a value over 16 KiB (only one from the node's environment can be) gets none and is only
//! replaced whole. The values are read again when the secrets folder or a file in it changes (name, size or time), and at every
//! write while one of those times is under 2 s old, so a change within one tick of a coarse clock is not missed. The file is as sensitive as the
//! terminal, and its mode is what protects it.
//!
//! **Not built here.** A forget at the hub (ADR-0053 `agent.forget`) is answered by the hub and never reaches the node, so it does not
//! remove the file; retention does.

use std::collections::{BTreeMap, HashMap};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::ops::Range;
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
/// ADR-0089 decision 2: per agent by default (the policy only lowers it: "a node policy `chat.history_max_bytes` and
/// `chat.history_days` lowers (never raises past a hard cap of 256 MiB per agent, 2 GiB per node) the defaults"; review of #1103, N7),
/// and per node.
pub const DEFAULT_MAX_BYTES: u64 = 32 << 20;
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
    /// `chat.history_max_bytes`: a number of bytes, at most [`DEFAULT_MAX_BYTES`] (the policy lowers it). `None` for a value that is
    /// not a number.
    pub fn parse_max_bytes(v: &str) -> Option<u64> {
        v.trim()
            .parse::<u64>()
            .ok()
            .map(|b| b.min(DEFAULT_MAX_BYTES))
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
    /// Where the node cut something out of the entry's texts (not written; for the writer's redaction).
    pub cuts: Box<Cuts>,
}

/// Review of #1103 (N1): places in an entry's texts where the node cut something out (its head and tail kept, or its end dropped):
/// a value cut there leaves a piece on one side or both, which the writer looks for. Byte offsets in the text as kept.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cuts {
    pub text: Vec<usize>,
    pub output: Vec<usize>,
    pub args: Vec<usize>,
    /// A tool call's tool name, each of its paths, each plan step, and a notice's parameters by key.
    pub tool: Vec<usize>,
    pub paths: Vec<Vec<usize>>,
    pub steps: Vec<Vec<usize>>,
    pub params: Vec<(String, Vec<usize>)>,
    /// The entry is a version that is not final yet (a tool call still running): its output and arguments may end inside a value
    /// the next version completes, so each of their ends counts as a place cut too.
    pub provisional: bool,
}

impl Cuts {
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
            && self.output.is_empty()
            && self.args.is_empty()
            && self.tool.is_empty()
            && self.paths.iter().all(Vec::is_empty)
            && self.steps.iter().all(Vec::is_empty)
            && self.params.iter().all(|(_, c)| c.is_empty())
            && !self.provisional
    }
}

/// Review of #1103 (N1): consecutive pieces of the agent's streamed text and thinking, joined, so a value split between pieces is found
/// whole. Each part is one entry's text in `text`; `line` is its line in the batch when it is written there, `None` for a piece that
/// is only context (written already, or held back).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    pub parts: Vec<RunPart>,
    /// Places in `text` where a value may have been cut: the start of words whose earlier part an earlier node wrote, and the end of
    /// words written before what follows them (a stop's flush, words said outside a turn).
    pub edges: Vec<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunPart {
    pub line: Option<usize>,
    pub start: usize,
    pub end: usize,
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
    /// The runs of streamed words a line of this batch belongs to (empty when the node holds no secret value).
    pub runs: Vec<Run>,
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
    /// The secret values last read, longest first, and what their folder looked like then.
    known: Mutex<(Option<Fingerprint>, Vec<String>)>,
    /// What finding pieces of those values needs, made again only when they change.
    pieces: Mutex<(Vec<String>, Arc<Vec<Needle>>)>,
}

/// What the secrets folder looks like: its own time, and each file's name, size and time.
#[derive(Clone, PartialEq, Eq)]
struct Fingerprint {
    folder: SystemTime,
    files: Vec<(std::ffi::OsString, u64, Option<SystemTime>)>,
}

impl Fingerprint {
    fn times(&self) -> impl Iterator<Item = SystemTime> + '_ {
        std::iter::once(self.folder).chain(self.files.iter().filter_map(|f| f.2))
    }
}

/// `None` when the folder cannot be read (nothing is cached then).
fn fingerprint(dir: &Path) -> Option<Fingerprint> {
    let folder = std::fs::metadata(dir).and_then(|m| m.modified()).ok()?;
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| {
            let m = e.metadata().ok();
            (
                e.file_name(),
                m.as_ref().map_or(0, |m| m.len()),
                m.and_then(|m| m.modified().ok()),
            )
        })
        .collect();
    files.sort();
    Some(Fingerprint { folder, files })
}

/// The file name of an agent: its id when that is a plain name (letters, digits, `-` and `_`), never a path. Without dots, so no id
/// names another agent's earlier session (`x.earlier`) or a leftover of a rewrite (review of #1103, N9).
fn file_stem(agent: &str) -> Option<&str> {
    (!agent.is_empty()
        && agent.len() <= 128
        && agent
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
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
            ..Side::default()
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

/// What `agora acp-bridge` puts where its own shape check replaced something (it may be part of a node secret value: the rest is
/// then beside it).
pub const BRIDGE_MARK: &str = "[redacted]";

/// Mechanism (a): every occurrence of each known value whole in `s`, overlapping ones too.
fn whole(s: &str, known: &[String], out: &mut Vec<Range<usize>>) {
    // A needle longer than the text is skipped before the search is set up (its setup costs the needle's length).
    for k in known.iter().filter(|k| !k.is_empty() && k.len() <= s.len()) {
        let mut from = 0;
        while let Some(i) = s[from..].find(k.as_str()) {
            let at = from + i;
            out.push(at..at + k.len());
            // The next search starts one character on, so an occurrence overlapping this one is found too.
            from = at + s[at..].chars().next().map_or(1, char::len_utf8);
            if from > s.len() - k.len() {
                break;
            }
        }
    }
}

/// A value to look for, with what finding its pieces needs: a suffix automaton of it (made when the store's values change, not per
/// write).
pub(crate) struct Needle {
    sam: Sam,
}

/// A value longer than [`MAX_SECRET_SPAN`] (a value the node keeps is at most 8 KiB, ADR-0084; one passed from its environment may be
/// longer) gets no automaton: it is still replaced whole, and the node keeps no more of the words beside a piece than that span.
fn needles(known: &[String]) -> Vec<Needle> {
    known
        .iter()
        .filter(|v| v.len() <= agora_adapters::text::MAX_SECRET_SPAN)
        .map(|v| Needle {
            sam: Sam::of(v.as_bytes()),
        })
        .collect()
}

/// A suffix automaton of one value: it recognises every part of it, so the longest part of the value ending at each place of a
/// text is found in one pass over the text (review of #1103, round 4: the earlier pairwise comparison of places was quadratic).
/// Kept in flat tables (at most 2 states and 3 moves per byte of the value: about 40 bytes per byte, review of #1103, round 5).
struct Sam {
    len: Vec<u32>,
    link: Vec<u32>,
    /// The moves of state `s` are `first[s]..first[s + 1]` of `by` (sorted) and `to`.
    first: Vec<u32>,
    by: Vec<u8>,
    to: Vec<u32>,
}

/// A suffix automaton while it is built.
struct Building {
    len: Vec<usize>,
    link: Vec<usize>,
    next: Vec<Vec<(u8, usize)>>,
}

impl Building {
    fn get(&self, p: usize, c: u8) -> Option<usize> {
        let n = &self.next[p];
        n.binary_search_by_key(&c, |x| x.0).ok().map(|i| n[i].1)
    }

    fn set(&mut self, p: usize, c: u8, to: usize) {
        let n = &mut self.next[p];
        match n.binary_search_by_key(&c, |x| x.0) {
            Ok(i) => n[i].1 = to,
            Err(i) => n.insert(i, (c, to)),
        }
    }
}

impl Sam {
    fn get(&self, p: usize, c: u8) -> Option<usize> {
        let (a, z) = (self.first[p] as usize, self.first[p + 1] as usize);
        let by = self.by.get(a..z)?;
        by.binary_search(&c).ok().map(|i| self.to[a + i] as usize)
    }

    fn of(k: &[u8]) -> Self {
        const NONE: usize = usize::MAX;
        let mut sam = Building {
            len: vec![0],
            link: vec![NONE],
            next: vec![Vec::new()],
        };
        let mut last = 0;
        for &c in k {
            let cur = sam.len.len();
            sam.len.push(sam.len[last] + 1);
            sam.link.push(0);
            sam.next.push(Vec::new());
            let mut p = last;
            while p != NONE && sam.get(p, c).is_none() {
                sam.set(p, c, cur);
                p = sam.link[p];
            }
            if p != NONE {
                let q = sam.get(p, c).unwrap_or(0);
                if sam.len[p] + 1 == sam.len[q] {
                    sam.link[cur] = q;
                } else {
                    let clone = sam.len.len();
                    sam.len.push(sam.len[p] + 1);
                    sam.link.push(sam.link[q]);
                    sam.next.push(sam.next[q].clone());
                    while p != NONE && sam.get(p, c) == Some(q) {
                        sam.set(p, c, clone);
                        p = sam.link[p];
                    }
                    sam.link[q] = clone;
                    sam.link[cur] = clone;
                }
            }
            last = cur;
        }
        // A value is at most a few KiB (ADR-0084), so every number fits in 32 bits; the root's link (none) becomes 0, never followed.
        let narrow = |x: usize| u32::try_from(x).unwrap_or(0);
        let mut first = Vec::with_capacity(sam.next.len() + 1);
        let (mut by, mut to) = (Vec::new(), Vec::new());
        for n in &sam.next {
            first.push(narrow(by.len()));
            for &(c, t) in n {
                by.push(c);
                to.push(narrow(t));
            }
        }
        first.push(narrow(by.len()));
        Sam {
            len: sam.len.iter().map(|&x| narrow(x)).collect(),
            link: sam
                .link
                .iter()
                .map(|&x| if x == NONE { 0 } else { narrow(x) })
                .collect(),
            first,
            by,
            to,
        }
    }

    /// For each place `i` in `text` (0 to its length), raises `out[i]` to the length of the longest part of the value ending there.
    fn ending(&self, text: &[u8], out: &mut [usize]) {
        let (mut v, mut l) = (0usize, 0usize);
        for (i, &c) in text.iter().enumerate() {
            while v != 0 && self.get(v, c).is_none() {
                v = self.link[v] as usize;
                l = self.len[v] as usize;
            }
            match self.get(v, c) {
                Some(n) => {
                    v = n;
                    l += 1;
                }
                None => (v, l) = (0, 0),
            }
            out[i + 1] = out[i + 1].max(l);
        }
    }
}

/// Mechanism (b), fragments: every part (4 bytes or more) of a known value that touches or crosses a place in `edges`. That covers
/// a value's start cut off at a place, its end, a part cut on both sides, pieces of under 4 bytes beside several places that add
/// up, and a part whose bytes happen to sit on both sides of a place (review of #1103, round 4: words lost between two writes can
/// leave a value's last letters right after its first ones). Linear in the text for each value, however many places there are: the
/// longest part ending at each byte is found in one pass, and a part touches a place when the last place at or before its end is at
/// or after its start (the earlier pairwise comparison of places was quadratic).
fn fragments(s: &str, edges: &[usize], known: &[Needle], out: &mut Vec<Range<usize>>) {
    if edges.is_empty() || known.is_empty() {
        return;
    }
    let n = s.len();
    let mut ending = vec![0usize; n + 1];
    for k in known {
        k.sam.ending(s.as_bytes(), &mut ending);
    }
    // `last[i]`: the last place at or before `i`, plus one (0: none).
    let mut last = vec![0usize; n + 1];
    for &e in edges.iter().filter(|&&e| e <= n && s.is_char_boundary(e)) {
        last[e] = e + 1;
    }
    for i in 1..=n {
        last[i] = last[i].max(last[i - 1]);
    }
    let floor = |mut i: usize| {
        while !s.is_char_boundary(i) {
            i -= 1;
        }
        i
    };
    let ceil = |mut i: usize| {
        while !s.is_char_boundary(i) {
            i += 1;
        }
        i
    };
    // The starts of the longest parts never move back as their ends move on, so touching parts are joined as they come.
    let mut cur: Option<Range<usize>> = None;
    for end in MIN_PIECE..=n {
        let len = ending[end];
        if len < MIN_PIECE || last[end] == 0 || last[end] - 1 < end - len {
            continue;
        }
        let part = floor(end - len)..ceil(end);
        cur = match cur {
            Some(c) if part.start <= c.end => Some(c.start.min(part.start)..c.end.max(part.end)),
            Some(c) => {
                out.push(c);
                Some(part)
            }
            None => Some(part),
        };
    }
    out.extend(cur);
}

/// The places on either side of every mark the bridge left.
fn bridge_marks(s: &str) -> Vec<usize> {
    s.match_indices(BRIDGE_MARK)
        .flat_map(|(i, m)| [i, i + m.len()])
        .collect()
}

/// The far side of each part of a bridge mark that a cut at one of `edges` left (review of #1103, round 4, property seed 1003545:
/// the bridge's stderr tail cut through a `[redacted]` that had replaced the start of a value, so `…]` stood before the rest of it):
/// the mark's start before an edge that ends with its first bytes, and its end after an edge that starts with its last ones.
fn cut_marks(s: &str, edges: &[usize]) -> Vec<usize> {
    let (b, m) = (s.as_bytes(), BRIDGE_MARK.as_bytes());
    let mut out = Vec::new();
    for &e in edges.iter().filter(|&&e| e <= b.len()) {
        if let Some(k) = (1..m.len())
            .rev()
            .find(|&k| e >= k && b.get(e - k..e) == m.get(..k))
        {
            out.push(e - k);
        }
        if let Some(k) = (1..m.len())
            .rev()
            .find(|&k| b.get(e..e + k) == m.get(m.len() - k..))
        {
            out.push(e + k);
        }
    }
    out
}

/// A field redacted: its whole values, and fragments at `edges` (with the bridge's marks, whole or cut).
fn redact_field(
    s: &mut String,
    known: &[String],
    pieces: &[Needle],
    mut edges: Vec<usize>,
    mut ranges: Vec<Range<usize>>,
) {
    whole(s, known, &mut ranges);
    edges.extend(bridge_marks(s));
    let torn = cut_marks(s, &edges);
    edges.extend(torn);
    fragments(s, &edges, pieces, &mut ranges);
    mask(s, ranges);
}

/// `s` with each of `ranges` (merged where they touch) replaced by [`REDACTED`]. The ranges are on character boundaries.
fn mask(s: &mut String, mut ranges: Vec<Range<usize>>) {
    if ranges.is_empty() {
        return;
    }
    ranges.sort_by_key(|r| (r.start, r.end));
    let mut out = String::with_capacity(s.len());
    let mut at = 0;
    let mut i = 0;
    while i < ranges.len() {
        let start = ranges[i].start.max(at);
        let mut end = ranges[i].end;
        i += 1;
        while i < ranges.len() && ranges[i].start <= end {
            end = end.max(ranges[i].end);
            i += 1;
        }
        if end <= start {
            continue;
        }
        out.push_str(&s[at..start]);
        out.push_str(REDACTED);
        at = end;
    }
    out.push_str(&s[at..]);
    *s = out;
}

/// What to replace in each line's text, from the batch's runs of the agent's words: every whole value in a run, and fragments at its
/// edges, in the part of it each line holds.
fn run_masks(batch: &Batch, known: &[String], pieces: &[Needle]) -> Vec<Vec<Range<usize>>> {
    let mut masks = vec![Vec::new(); batch.lines.len()];
    for run in &batch.runs {
        let mut found = Vec::new();
        whole(&run.text, known, &mut found);
        // The node's own marks in words written already (a carry from the file) are places a value was cut too.
        let mut edges = run.edges.clone();
        edges.extend(
            run.text
                .match_indices(REDACTED)
                .flat_map(|(i, m)| [i, i + m.len()]),
        );
        fragments(&run.text, &edges, pieces, &mut found);
        for r in &found {
            for p in &run.parts {
                let (Some(line), a, b) = (p.line, r.start.max(p.start), r.end.min(p.end)) else {
                    continue;
                };
                if a < b {
                    if let Some(m) = masks.get_mut(line) {
                        m.push(a - p.start..b - p.start);
                    }
                }
            }
        }
    }
    masks
}

/// `line` with every known secret value replaced (ADR-0089 decision 7; review of #1103): (a) a value whole anywhere in a text, also
/// where JSON escapes it, and in the agent's words across the pieces they came in (`text_mask`, from the runs); (b) a fragment at each
/// place a value may have been cut: every cut the node made (`Side::cuts`), the bridge's marks, the ends of a version not final yet,
/// and both ends of a notice's parameters (the bridge cuts a stderr tail at its front). Not `call_id` or `approval_id`: ids, never a
/// value. A tool call's text and a plan's text are made again from their redacted parts, as the node made them.
fn redacted(
    line: &Line,
    known: &[String],
    pieces: &[Needle],
    text_mask: Vec<Range<usize>>,
) -> Line {
    let mut line = line.clone();
    if known.is_empty() {
        return line;
    }
    if let Line::Entry(e, side) = &mut line {
        let c = std::mem::take(&mut side.cuts);
        redact_field(&mut e.text, known, pieces, c.text.clone(), text_mask);
        let open = |s: &String, mut cuts: Vec<usize>| {
            if c.provisional {
                cuts.push(s.len());
            }
            cuts
        };
        let edges = open(&e.output, c.output.clone());
        redact_field(&mut e.output, known, pieces, edges, Vec::new());
        redact_field(&mut e.path, known, pieces, Vec::new(), Vec::new());
        for (k, v) in e.params.iter_mut() {
            // Both ends of a parameter are edges (a stderr tail is cut at its front), but not of the node's own numbers and ids
            // (`bytes`, `entries`), which would only be masked by chance.
            let numbers = v.bytes().all(|b| b.is_ascii_digit() || b == b',');
            let mut edges = if numbers {
                Vec::new()
            } else {
                vec![0, v.len()]
            };
            // The bridge marks the front cut of a stderr tail (and the end of a shortened detail) with `…`: just beside it is
            // where it cut (review of #1103, round 4).
            if v.starts_with('…') {
                edges.push('…'.len_utf8());
            }
            if v.ends_with('…') {
                edges.push(v.len() - '…'.len_utf8());
            }
            if let Some((_, cuts)) = c.params.iter().find(|(key, _)| key == k) {
                edges.extend(cuts);
            }
            redact_field(v, known, pieces, edges, Vec::new());
        }
        for (i, p) in e.paths.iter_mut().enumerate() {
            let edges = c.paths.get(i).cloned().unwrap_or_default();
            redact_field(&mut p.path, known, pieces, edges, Vec::new());
        }
        for (i, st) in e.plan_steps.iter_mut().enumerate() {
            let edges = c.steps.get(i).cloned().unwrap_or_default();
            redact_field(&mut st.text, known, pieces, edges, Vec::new());
        }
        if e.kind == agora_protocol::v1::ChatKind::Plan as i32 && !e.plan_steps.is_empty() {
            e.text = crate::chat::plan_text(&e.plan_steps);
        }
        if let Some((tool, args)) = &mut side.tool {
            redact_field(tool, known, pieces, c.tool.clone(), Vec::new());
            let edges = open(args, c.args.clone());
            redact_field(args, known, pieces, edges, Vec::new());
            if e.kind == agora_protocol::v1::ChatKind::ToolCall as i32 {
                e.text = crate::chat::tool_text(tool, args);
            }
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
            pieces: Mutex::new((Vec::new(), Arc::new(Vec::new()))),
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

    /// The secret values to replace, read again only when their folder or a file in it changed (its name, size or time), and always
    /// while any of those times is under 2 s old: a change within one tick of a coarse clock keeps the time it had (review of #1103,
    /// round 5).
    fn known_secrets(&self) -> Vec<String> {
        let changed = self.secrets.as_ref().and_then(|s| fingerprint(s.dir()));
        let racy = changed.as_ref().is_none_or(|f| {
            f.times().any(|t| {
                SystemTime::now()
                    .duration_since(t)
                    .map_or(true, |age| age < Duration::from_secs(2))
            })
        });
        let mut g = self.known.lock().unwrap_or_else(|e| e.into_inner());
        if g.0 != changed || racy {
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

    /// The longest value looked for (0: none), so the board gathers enough of a run around a piece and holds back enough of it.
    pub fn longest_secret(&self) -> usize {
        self.known_secrets()
            .iter()
            .map(String::len)
            .max()
            .unwrap_or(0)
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
        let pieces = {
            let mut g = self.pieces.lock().unwrap_or_else(|e| e.into_inner());
            if g.0 != known {
                *g = (known.clone(), Arc::new(needles(&known)));
            }
            Arc::clone(&g.1)
        };
        let masks = run_masks(batch, &known, &pieces);
        let mut out = Vec::new();
        for (line, m) in batch.lines.iter().zip(masks) {
            out.extend(encode_line(batch.high, &redacted(line, &known, &pieces, m)));
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
            // Nothing may have been written yet (a take-up with no file, then Start again): the folder is made first.
            if let Err(e) = agora_platform::ensure_private_dir(&self.dir)
                .and_then(|()| agora_platform::write_private(&path, &out))
            {
                // Operational log, not UI text. The ids still go on in this node's memory; the first batch writes them.
                eprintln!("transcripts: {agent}: the new session's file was not started: {e}");
            }
        }
    }

    /// ADR-0089 decision 2: removes the files of agents that closed more than [`Limits::days`] ago, then, over [`NODE_MAX_BYTES`] in
    /// all, the oldest files of agents that are not kept. `status` tells, for the agents whose files are there, which the node keeps
    /// whatever their age (running, or interrupted and offered for Resume) and when the others ended; an agent it does not know ages
    /// from its file's last write (review of #1103, N6). An earlier session's file (`.earlier`) follows its agent. Leftovers of a
    /// rewrite another process left (`.tmp-<pid>`) go. Returns how many files went.
    pub fn sweep(
        &self,
        status: &dyn Fn(&[String]) -> HashMap<String, Kept>,
        now: SystemTime,
    ) -> usize {
        let Ok(dir) = std::fs::read_dir(&self.dir) else {
            return 0;
        };
        let mut removed = 0;
        let mut found: Vec<(String, std::fs::Metadata, PathBuf)> = Vec::new();
        let ours = format!(".tmp-{}", std::process::id());
        for item in dir.flatten() {
            let name = item.file_name().to_string_lossy().into_owned();
            let Ok(meta) = item.metadata() else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            if name.contains(".jsonl.tmp-") {
                if !name.ends_with(&ours) && std::fs::remove_file(item.path()).is_ok() {
                    removed += 1;
                }
                continue;
            }
            let Some(stem) = name.strip_suffix(".jsonl") else {
                continue;
            };
            let agent = stem.strip_suffix(".earlier").unwrap_or(stem).to_string();
            found.push((agent, meta, item.path()));
        }
        let names: Vec<String> = found.iter().map(|f| f.0.clone()).collect();
        let known = status(&names);
        let keep_ms = self.limits.days.saturating_mul(DAY_MS);
        let ms = |t: SystemTime| {
            t.duration_since(SystemTime::UNIX_EPOCH)
                .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        };
        let now_ms = ms(now);
        let mut files: Vec<(SystemTime, u64, PathBuf, bool)> = Vec::new();
        for (agent, meta, path) in found {
            let modified = meta.modified().unwrap_or(now);
            let kept = known.get(&agent).copied().unwrap_or(Kept::Unknown);
            let since = match kept {
                Kept::Keep => {
                    files.push((modified, meta.len(), path, true));
                    continue;
                }
                Kept::EndedAt(t) => {
                    // The ledger forgets an ended agent after 14 days; the file then ages from its last write. Stamping that with
                    // the end time keeps "after the agent ended" true then (an agent idle for long before it ended).
                    if ms(modified) < t {
                        // An existing file of the node's private folder, opened only to set its time (nothing is written).
                        let _ = std::fs::OpenOptions::new()
                            .write(true)
                            .open(&path)
                            .and_then(|f| {
                                f.set_modified(SystemTime::UNIX_EPOCH + Duration::from_millis(t))
                            });
                    }
                    t.max(ms(modified))
                }
                Kept::Unknown => ms(modified),
            };
            if now_ms.saturating_sub(since) >= keep_ms {
                if std::fs::remove_file(&path).is_ok() {
                    removed += 1;
                }
                continue;
            }
            files.push((modified, meta.len(), path, false));
        }
        let mut total: u64 = files.iter().map(|f| f.1).sum();
        files.sort_by_key(|f| f.0);
        for (_, len, path, keep) in files {
            if total <= NODE_MAX_BYTES {
                break;
            }
            if !keep && std::fs::remove_file(&path).is_ok() {
                total = total.saturating_sub(len);
                removed += 1;
            }
        }
        removed
    }
}

/// What the writer asks the node for retention: for the agents whose files are there, which are kept and when the others ended.
pub type AgentStatus = Box<dyn Fn(&[String]) -> HashMap<String, Kept> + Send>;

/// ADR-0089 decision 2: what the node knows of an agent whose transcript is on disk, for retention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kept {
    /// It runs, or it is interrupted and offered for Resume: its file is kept whatever its age.
    Keep,
    /// It ended at this time (ms since the epoch): its file is kept [`Limits::days`] after that.
    EndedAt(u64),
    /// The node no longer knows it: its file ages from its last write.
    Unknown,
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
/// [`SWEEP_EVERY`] (`status` says which agents' files are kept and when the others ended, see [`Store::sweep`]). A failed write is
/// said once in that agent's conversation; its batch is not tried again (its entries stay in the ring, and every later change is
/// written as it comes). Nothing is written when it stops: a clean stop writes the last batch itself with [`flush`], through the same
/// [`Store::write`], once it has recorded everything and this writer has ended (ADR-0104 O2).
pub fn spawn_writer(
    board: Arc<crate::chat::ChatBoard>,
    status: AgentStatus,
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
                    store.sweep(&*status, SystemTime::now());
                    swept = Some(std::time::Instant::now());
                }
                wake.wait(BATCH_INTERVAL);
                if stop.load(Ordering::Acquire) {
                    break;
                }
                let due = board.take_due(crate::daemon::now_ms());
                write_batches(&board, &store, due, &mut failed);
            }
        })
        .map(Some)
}

/// ADR-0104 O2: writes what `board` has not saved yet, now, on the caller's thread: the last batch of a node that stops cleanly, after
/// its writer ([`spawn_writer`]) has ended. Everything is taken (`take_unsaved`), the pieces the writer would still hold back too, since
/// the process is ending; it goes through the writer's own [`Store::write`] (redaction over runs, the watermark line). Nothing when the
/// board keeps no history.
pub fn flush(board: &crate::chat::ChatBoard) {
    if let Some(store) = board.store() {
        let all = board.take_unsaved();
        write_batches(board, &store, all, &mut HashMap::new());
    }
}

/// Writes `batches`; a failed write is logged once per agent in `failed` and said in its conversation.
fn write_batches(
    board: &crate::chat::ChatBoard,
    store: &Store,
    batches: Vec<Batch>,
    failed: &mut HashMap<AgentId, ()>,
) {
    for batch in batches {
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
            runs: Vec::new(),
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
            ..Side::default()
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
        for bad in [
            "",
            "../x",
            "a/b",
            ".hidden",
            "a\\b",
            "x.earlier",
            "a.jsonl.tmp-1",
        ] {
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
        assert_eq!(
            read_tail(&store.earlier_path("a").unwrap())
                .unwrap()
                .entries
                .len(),
            1
        );
        // A new entry is appended after the mark, and the file stays private.
        store.write(&batch("a", 10, vec![text(10, "y")])).unwrap();
        assert_eq!(store.read("a").entries.len(), 1);
    }

    /// Found by the property test (round 4): Start again before anything was written left no file with the highest id, because the
    /// transcripts folder did not exist yet.
    #[test]
    fn start_again_before_any_write_still_leaves_the_highest_id() {
        let dir = temp("aside-nofolder");
        let store = Store::new(&dir, Limits::default(), None);
        store.move_aside("a", 7);
        assert_eq!(store.read("a").high, 7);
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
            ..Side::default()
        };
        store
            .write(&batch("a", 7, vec![Line::Entry(Box::new(e), side)]))
            .unwrap();
        let body = std::fs::read_to_string(store.path("a").unwrap()).unwrap();
        assert!(!body.contains("sk-very-secret-1234"));
        assert_eq!(body.matches(REDACTED).count(), 3, "{body}");
    }

    /// Round 5: a value changed within one tick of a coarse clock (the folder's and the file's times and the size as before) is
    /// still read again.
    #[test]
    fn a_secret_changed_within_one_clock_tick_is_still_replaced() {
        let dir = temp("secret-tick");
        let secrets = SecretStore::in_data(&dir);
        let set = |v: &str| {
            secrets
                .set("TOKEN", &crate::credentials::Secret::new(v).unwrap())
                .unwrap()
        };
        set("first-value-0001");
        let store = Store::new(&dir, Limits::default(), Some(SecretStore::in_data(&dir)));
        let line = |t: &str| {
            let mut e = full();
            e.output = t.into();
            Line::Entry(Box::new(e), Side::default())
        };
        store
            .write(&batch("a", 1, vec![line("first-value-0001")]))
            .unwrap();
        let file = secrets.dir().join("TOKEN");
        let times = |p: &Path| std::fs::metadata(p).unwrap().modified().unwrap();
        let (folder_t, file_t) = (times(secrets.dir()), times(&file));
        set("other-value-0002");
        // A folder can't be opened this way on Windows; there its time stays newer, which only makes the store read again sooner.
        if let Ok(f) = std::fs::File::open(secrets.dir()) {
            let _ = f.set_modified(folder_t);
        }
        std::fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(file_t)
            .unwrap();
        store
            .write(&batch("a", 2, vec![line("other-value-0002")]))
            .unwrap();
        let body = std::fs::read_to_string(store.path("a").unwrap()).unwrap();
        assert!(!body.contains("other-value-0002"), "{body}");
    }

    /// Round 5, a measurement (`--ignored --nocapture`): the matcher's tables and build time for 64 values of 8,000 bytes.
    #[test]
    #[ignore]
    fn measure_the_matcher_for_64_values_of_8000_bytes() {
        let mut x = 0x9E37_79B9_7F4A_7C15u64;
        let values: Vec<String> = (0..64)
            .map(|_| {
                (0..8000)
                    .map(|_| {
                        x ^= x << 13;
                        x ^= x >> 7;
                        x ^= x << 17;
                        char::from(b'!' + (x % 90) as u8)
                    })
                    .collect()
            })
            .collect();
        let t = std::time::Instant::now();
        let n = needles(&values);
        let bytes: usize = n
            .iter()
            .map(|k| k.sam.len.len() * 12 + 4 + k.sam.by.len() * 5)
            .sum();
        eprintln!(
            "matcher: {} KiB for {} bytes of values, built in {:?}",
            bytes / 1024,
            64 * 8000,
            t.elapsed()
        );
    }

    /// Round 5: the matcher's tables stay within 2 states and 3 moves per byte of a value, and it finds what a brute force finds.
    #[test]
    fn the_matcher_is_small_and_agrees_with_a_brute_force() {
        let mut x = 0x2545_F491_4F6C_DD1Du64;
        let mut next = |m: u64| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x % m) as u8
        };
        let big: Vec<u8> = (0..8000).map(|_| b'a' + next(26)).collect();
        let sam = Sam::of(&big);
        assert!(sam.len.len() <= 2 * big.len() && sam.by.len() <= 3 * big.len());
        for _ in 0..300 {
            let v: Vec<u8> = (0..1 + next(12)).map(|_| b'a' + next(3)).collect();
            let t: Vec<u8> = (0..1 + next(40)).map(|_| b'a' + next(3)).collect();
            let mut got = vec![0; t.len() + 1];
            Sam::of(&v).ending(&t, &mut got);
            for (end, &g) in got.iter().enumerate() {
                let want = (0..=end)
                    .filter(|&a| a == end || v.windows(end - a).any(|w| w == &t[a..end]))
                    .map(|a| end - a)
                    .max()
                    .unwrap_or(0);
                assert_eq!(g, want, "{v:?} in {t:?} at {end}");
            }
        }
    }

    fn store_with_secret(tag: &str, value: &str) -> (PathBuf, Store) {
        let dir = temp(tag);
        SecretStore::in_data(&dir)
            .set("TOKEN", &crate::credentials::Secret::new(value).unwrap())
            .unwrap();
        let store = Store::new(&dir, Limits::default(), Some(SecretStore::in_data(&dir)));
        (dir, store)
    }

    /// Review of #1103 (N1): a long tool output is cut in its middle (head and tail kept); a value on the cut (where the node noted
    /// it) is replaced on both sides.
    #[test]
    fn a_secret_on_the_cut_of_a_long_output_is_replaced() {
        let (_dir, store) = store_with_secret("secret-cut", "sk-very-secret-1234");
        let half = crate::chat::MAX_OUTPUT_BYTES / 2;
        let mut e = full();
        // The head ends with the value's start, the middle (`cre`) was cut out, the tail starts with its end.
        e.output = format!("{}sk-very-se{}", "a".repeat(half - 10), "t-1234 the end");
        e.output_truncated = true;
        store
            .write(&batch(
                "a",
                7,
                vec![Line::Entry(
                    Box::new(e),
                    Side {
                        cuts: Box::new(Cuts {
                            output: vec![half],
                            ..Cuts::default()
                        }),
                        ..Side::default()
                    },
                )],
            ))
            .unwrap();
        let body = std::fs::read_to_string(store.path("a").unwrap()).unwrap();
        assert!(
            !body.contains("sk-very-se") && !body.contains("t-1234"),
            "{body}"
        );
        assert!(body.contains(&format!("a{REDACTED} the end")));
        // A whole value in the head too: the piece on the cut is still found (the cut is looked at first).
        let mut e = full();
        e.output = format!(
            "sk-very-secret-1234{}sk-very-se{}",
            "a".repeat(half - 29),
            "t-1234 the end"
        );
        e.output_truncated = true;
        store
            .write(&batch(
                "b",
                7,
                vec![Line::Entry(
                    Box::new(e),
                    Side {
                        cuts: Box::new(Cuts {
                            output: vec![half],
                            ..Cuts::default()
                        }),
                        ..Side::default()
                    },
                )],
            ))
            .unwrap();
        let body = std::fs::read_to_string(store.path("b").unwrap()).unwrap();
        assert!(
            !body.contains("sk-very-se") && !body.contains("t-1234 the"),
            "{body}"
        );
        // And a thought as long as its cap.
        let half = crate::chat::MAX_THOUGHT_BYTES / 2;
        let mut e = full();
        e.kind = agora_protocol::v1::ChatKind::Thought as i32;
        e.text = format!(
            "{}sk-very-se{}",
            "b".repeat(half - 10),
            "t-1234 ".repeat(half / 7)
        );
        store
            .write(&batch(
                "c",
                7,
                vec![Line::Entry(
                    Box::new(e),
                    Side {
                        cuts: Box::new(Cuts {
                            text: vec![half],
                            ..Cuts::default()
                        }),
                        ..Side::default()
                    },
                )],
            ))
            .unwrap();
        let body = std::fs::read_to_string(store.path("c").unwrap()).unwrap();
        assert!(!body.contains("sk-very-se"), "the head's piece");
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
            ..Side::default()
        };
        store
            .write(&batch("a", 7, vec![Line::Entry(Box::new(e.clone()), side)]))
            .unwrap();
        let seed = store.read("a");
        let (got, side) = &seed.entries[0];
        assert_eq!(
            side.tool.as_ref().unwrap().1,
            format!(r#"{{"pw":"{REDACTED}"}}"#)
        );
        assert_eq!(got.call_id, e.call_id, "an id is never changed");
        SecretStore::in_data(&dir)
            .set("SHORT", &crate::credentials::Secret::new("toolu").unwrap())
            .unwrap();
        let store = Store::new(&dir, Limits::default(), Some(SecretStore::in_data(&dir)));
        store
            .write(&batch("b", 1, vec![text(1, "toolu_01abc")]))
            .unwrap();
        assert_eq!(store.read("b").entries[0].0.text, "toolu_01abc");
    }

    #[test]
    fn retention_counts_from_when_an_agent_ended_and_never_touches_one_offered_for_resume() {
        let dir = temp("sweep");
        let limits = |days| Limits {
            max_bytes: 1 << 20,
            days,
        };
        let store = Store::new(&dir, limits(2), None);
        for a in ["old", "running", "offered", "idle-long", "fresh"] {
            store.write(&batch(a, 1, vec![text(1, "x")])).unwrap();
        }
        store.move_aside("fresh", 1);
        assert!(store.earlier_path("fresh").unwrap().exists());
        // The new session's file holds only the earlier session's highest id.
        assert!(store.read("fresh").entries.is_empty());
        // A leftover of a rewrite another process did not finish.
        std::fs::write(store.dir().join("old.jsonl.tmp-1"), "x").unwrap();
        let ms = |t: SystemTime| {
            t.duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64
        };
        let later = SystemTime::now() + Duration::from_secs(3 * 24 * 3600);
        // Review of #1103 (N6): an agent idle for long (its file not written for 3 days) that ended a day ago keeps its file.
        let status = |names: &[String]| -> HashMap<String, Kept> {
            names
                .iter()
                .filter_map(|n| {
                    let k = match n.as_str() {
                        "running" | "offered" => Kept::Keep,
                        "idle-long" => Kept::EndedAt(ms(later) - DAY_MS),
                        _ => return None,
                    };
                    Some((n.clone(), k))
                })
                .collect()
        };
        // Nothing is old yet; the leftover goes.
        assert_eq!(store.sweep(&status, SystemTime::now()), 1);
        // The idle agent's file now carries its end time, so it keeps it once the ledger forgot the agent.
        let stamped = std::fs::metadata(store.path("idle-long").unwrap())
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(ms(stamped), ms(later) - DAY_MS);
        // Three days on: the unknown agents' files go (both of the fresh agent's), the others stay.
        assert_eq!(store.sweep(&status, later), 3);
        for kept in ["running", "offered", "idle-long"] {
            assert!(store.path(kept).unwrap().exists(), "{kept}");
        }
        assert!(!store.path("old").unwrap().exists());
        assert!(!store.earlier_path("fresh").unwrap().exists());
        assert!(!store.path("fresh").unwrap().exists());
        // `chat.history_days = 0`: a file goes as soon as its agent ended, never one offered for Resume or running.
        let none = Store::new(&dir, limits(0), None);
        assert_eq!(none.sweep(&status, SystemTime::now()), 1);
        assert!(!store.path("idle-long").unwrap().exists());
        assert!(store.path("offered").unwrap().exists() && store.path("running").unwrap().exists());
        purge_all(&dir).unwrap();
        assert!(!store.dir().exists());
        purge_all(&dir).unwrap();
    }

    #[test]
    fn policy_values_are_bounded() {
        assert_eq!(Limits::parse_max_bytes("1000"), Some(1000));
        assert_eq!(
            Limits::parse_max_bytes("999999999999"),
            Some(DEFAULT_MAX_BYTES)
        );
        assert_eq!(Limits::parse_max_bytes("lots"), None);
        assert_eq!(Limits::parse_days("7"), Some(7));
        assert_eq!(Limits::parse_days("90"), Some(DEFAULT_DAYS));
        assert_eq!(Limits::parse_days("-1"), None);
    }
}
