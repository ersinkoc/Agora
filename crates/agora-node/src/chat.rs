//! What agents without a terminal say (ADR-0038): a bounded transcript per agent that clients can read and follow.
//!
//! The adapter host reports chat items to the node; the node keeps the latest of them, plus the requests users sent, in a
//! ring per agent (like a terminal's screen). A subscriber gets the ring first and every entry after it. Nothing here touches the
//! hub's event store.
//!
//! ADR-0089, ADR-0104 C1: on a board that saves ([`ChatBoard::saving`]), each change marks its entry for the transcript writer
//! ([`crate::transcript`]), which takes the marked entries once a second off the chat path and appends them to the agent's file. A
//! node that takes an agent up from the holder, or resumes it, fills its ring from that file ([`ChatBoard::seed_from_disk`]) and
//! numbers on from the file's highest id, so the conversation and its ids survive a node restart.
//!
//! ADR-0090: a tool call and a plan are ONE entry the node changes in place (a later version carries the same `entry_id`, which a
//! client that knows the capability `chat.rich` replaces); the node measures a call's duration, enforces every size cap before an
//! entry exists, removes secrets from what a tool printed, and sends at most one update of an entry per 150 ms (the last one is
//! never lost: [`ChatBoard::flush_due`] sends what was held back). Everything in an entry is untrusted data from a model or a tool.
//! Entries of kind `CHAT_KIND_PERMISSION` are made by the node from its own approvals queue, never from an adapter's item.
//!
//! Cost of one entry: O(its size) once to cut it (a cap is applied with one pass over at most the cap's bytes after a length test),
//! one clone per follower per version, one binary search of the ring (at most 500) per update; no per-byte work on terminals.
//!
//! **Live redaction** (#1147, ADR-0089 Amendment A). Followers (`ChatSubscribe`, then the hub's relay and the window) never get a
//! value of a secret the node holds: everything they are sent is redacted as the transcript writer redacts its lines, by the same
//! functions (`transcript::redact_lines`, `redact_entry`) with the same values (`transcript::Secrets`, which the writer refreshes and
//! the chat path only reads, so no folder is read on an agent's path).
//! - **When.** An operation marks what it changed (`Transcript::add`, `modify`, `flush_due`, seeding); the board's lock sends it when
//!   it is let go (`Locked`), so the redaction sees what the operation recorded beside an entry (its cuts, a call's tool and
//!   arguments) and no operation can forget to send. With no value held, every entry goes as it is, as before.
//! - **Other entries.** A tool call (each version), plan, notice, permission entry, diff or prompt goes as the writer redacts its line:
//!   whole values, and fragments at its cuts, the bridge's marks, a notice parameter's ends and the end of a version not final.
//!   Its cuts include those an adapter that declared `chat.cuts` reported (ADR-0111, [`ChatBoard::push_item_cut`]), recorded as the
//!   node's own; in the agent's words they are edges of the stream too.
//! - **The agent's words** (text and thinking) are one stream, as in the file. A piece goes once the stream after it cannot end inside
//!   a value that started in it: while the stream ends in a part of a value of 4 bytes or more (what mechanism (b) would replace were
//!   the end an edge; `Transcript::releasable`), the pieces that hold that part wait in `Live::unsent`. Words that cannot end in such a
//!   part go at once. What goes is redacted over one run, as a writer's batch: the stream sent so far (the live carry, raw, at most
//!   the longest value less one byte, with its edges), the pieces sent now, and the pieces still held after them. A value whole in it
//!   is replaced in the pieces sent now; the rest of it, when it goes, is joined with the carry and replaced too.
//! - **Sent without what follows.** Any other new entry of the agent sends every word held first (so ids still increase on the
//!   stream: a client may drop a new id below one it has), as do the end of a turn, Start again, the ring's bound, and the node's tick
//!   once the oldest held piece waited [`LIVE_HOLD_MS`]. The end of what goes is then an edge: a part of a value there (4 bytes or
//!   more) is replaced, and the carry joins the rest when it comes. A value longer than 16 KiB (only one from the node's environment
//!   can be) has no automaton: the last bytes of the stream within its length wait too, and a forced send can show part of it.
//! - **Snapshot.** A new follower gets the words as they were sent (kept beside the ring where they differ, never redacted again
//!   with another stream), every other entry redacted as it is now, and none of the words still held: its stream brings them.
//! - **Latency** (ADR-0022): with no value held, nothing waits and nothing more is done. With one held, words wait only while the
//!   stream ends in a part of a value, at most [`LIVE_HOLD_MS`] plus one tick; the work per piece is one pass of each value's
//!   automaton over the piece's bytes (where each stands at the stream's end is kept, `Live::at_end`), and a release's run (carry,
//!   pieces, held pieces) as a batch's.
//! - **Not covered**: what the file does not cover; where a part of a value stands at an edge by chance, the live view and the file
//!   may differ (each masks it at its own edges); a value's first 3 bytes or fewer, sent before the next words show it goes on (the
//!   rest is replaced; the file, which holds back more, masks them); text sent before a value was added to the store stays as sent on
//!   streams already open (a later snapshot redacts the words again, as one stream, with the values held then: `shown_entries`), and
//!   a value added is replaced from the next read of the values (the writer's, or the node's tick in `flush_due`, at most a second);
//!   a value over 16 KiB can show in part when
//!   its words are sent without what follows.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use agora_adapters::host::{ChatItem, CutField, PlanItem, ReportedCut, ToolItem};
use agora_core::AgentId;
use agora_protocol::v1::{
    ChatEntry, ChatKind, ChatRole, PermissionOutcome, PlanPriority, PlanStep, ToolKind,
    ToolLocation, ToolStatus,
};

use crate::transcript::{
    redact_entry, redact_lines, redact_text, redact_words_again, AtEnd, Batch, Cuts, Line, Run,
    RunPart, Secrets, Seed, Side, Store, Wake, MIN_PART,
};
use agora_adapters::text::{balanced, cap_end, cap_middle, strip_core_at, Style};

/// Most entries and most bytes of text a transcript keeps; older entries are dropped first.
pub const MAX_ENTRIES: usize = 500;
pub const MAX_BYTES: usize = 256 * 1024;
/// Longest text of one entry; more is cut.
pub const MAX_ENTRY_BYTES: usize = 16 * 1024;
/// ADR-0090: a tool call's `output` (head and tail kept, `output_truncated` set) and one thought entry's text.
pub const MAX_OUTPUT_BYTES: usize = 16 * 1024;
pub const MAX_THOUGHT_BYTES: usize = 8 * 1024;
/// ADR-0090: a plan's steps and the longest text of one; the files of one tool call; the longest path and call id.
pub const MAX_PLAN_STEPS: usize = 64;
pub const MAX_PLAN_STEP_BYTES: usize = 512;
pub const MAX_PATHS: usize = 64;
pub const MAX_PATH_BYTES: usize = 1024;
pub const MAX_CALL_ID: usize = 256;
/// ADR-0090: the longest label of a permission entry (the action the agent asked to take).
pub const MAX_PERMISSION_BYTES: usize = 512;
/// ADR-0090: the most often one entry's changes are sent: one version per this many milliseconds.
pub const COALESCE_MS: u64 = 150;
/// #1147: the longest the agent's words wait before they are sent to followers while the stream ends in a part of a secret value
/// (checked on the node's tick, every 100 ms): then they go with that part replaced, as at an edge.
pub const LIVE_HOLD_MS: u64 = 300;
/// The most entries a follower may have waiting. A follower that falls further behind (a client that stopped reading) is dropped, and
/// its stream ends: the client subscribes again and gets the ring. One slow client therefore never grows the node's memory.
pub const FOLLOWER_QUEUE: usize = 1024;
/// Work on a text is done on at most this many bytes of it (head and tail) before it is cleaned and cut to its cap, so an adapter
/// cannot make the node spend more than a bounded pass on one item.
const PRE_BOUND: usize = 64 * 1024;
/// Notice codes only the node makes. An adapter's item with one of them is dropped: it could otherwise forge the node's own words.
pub const NODE_NOTICES: &[&str] = &[
    "adapter.queue_dropped",
    // ADR-0110: a prompt whose adapter did not confirm it within the node's wait.
    "adapter.no_answer",
    "adapter.output_dropped",
    // ADR-0101: the node refused an always-only request under its policy.
    "approval.always_only_forbidden",
    // ADR-0089: the agent's history could not be written to the node's disk.
    crate::transcript::NOT_SAVED,
];
/// ADR-0058: a notice's code is at most this long and has this many parameters at most, each key at most `MAX_PARAM_KEY` bytes and
/// each value at most `MAX_PARAM_BYTES` (more is cut). An adapter is untrusted; a notice that does not fit the shape is dropped.
pub const MAX_CODE: usize = 64;
pub const MAX_PARAMS: usize = 8;
pub const MAX_PARAM_KEY: usize = 32;
pub const MAX_PARAM_BYTES: usize = 1024;

/// What the node knows of a tool call it has an entry for: its name and arguments (the entry's `text` is made of both, and an update
/// may carry only one).
struct Call {
    entry: u64,
    tool: String,
    args: String,
}

#[derive(Default)]
struct Transcript {
    entries: VecDeque<ChatEntry>,
    bytes: usize,
    next_id: u64,
    followers: Vec<(u64, SyncSender<ChatEntry>)>,
    /// Tool calls by the agent's id of the call; each points at an entry that is still in the ring.
    calls: HashMap<String, Call>,
    /// The entry of the plan, while it is in the ring: a new plan replaces it in place.
    plan: Option<u64>,
    /// When each entry that changes in place was last sent, and which ones hold a change not yet sent.
    sent_ms: HashMap<u64, u64>,
    dirty: BTreeSet<u64>,
    /// ADR-0089, ADR-0104 C1: what is not on disk yet, on a board that saves (`None` otherwise).
    save: Option<Save>,
    /// Review of #1103 (N1): where the node cut something out of an entry's texts, for the transcript writer (on a board that saves).
    cuts: HashMap<u64, Cuts>,
    /// Review of #1103, round 3: the agent's words (its text and thinking, whatever other entries lie between) as a stream. `carry`:
    /// the last bytes of it written to the file, as said (in memory only, at most the longest secret value less one byte), so a value
    /// that begins in written words and ends in new ones is found whole. `carry_open`: the stream was written by an earlier node
    /// (taken up or resumed), whose last words this node does not have: the start of the next words written is a place a value may
    /// have been cut. `working`: the agent is in a turn (its last reported state); `closed_upto`: words up to this id were said
    /// before it last left one.
    carry: String,
    carry_open: bool,
    /// Edges inside `carry` (where a session's or an earlier node's words begin or end), while the carry still reaches back to them.
    carry_edges: Vec<usize>,
    working: bool,
    closed_upto: u64,
    /// #1147: what followers are sent, and when.
    live: Live,
}

/// #1147: what of one transcript followers are sent while the node holds a secret value (see the module doc, **Live redaction**).
#[derive(Default)]
struct Live {
    /// The agent's words not sent yet, by id, with when each came (its `at_ms`).
    unsent: BTreeMap<u64, u64>,
    /// What followers were sent of each of the agent's words in the ring, where that differs from the ring's entry.
    shown: HashMap<u64, ChatEntry>,
    /// The words sent, as one stream (like the writer's `carry`, `carry_edges` and `carry_open`): the last of them (raw, in memory,
    /// at most the longest value less one byte), the edges in it, and whether the next words start at one.
    carry: String,
    carry_edges: Vec<usize>,
    open: bool,
    /// Where each value's automaton stands at the end of the stream (the carry and the words held), moved on by each new piece; made
    /// again from the stream's end when the values change or the carry is replaced (seeding).
    at_end: AtEnd,
    /// The set of values every word shown so far was redacted with (`None`: none shown yet; `Some(0)`: several, or words shown with
    /// none held or seeded from the file). A snapshot taken with another set redacts the words again (`shown_entries`).
    words_gen: Option<u64>,
    /// What the operation under way changed, sent when it ends (each entry once, in the order it was first changed).
    outbox: Vec<(u64, Out)>,
    /// Send every word held when the operation ends, their end an edge (a turn ended, Start again, the hold's deadline).
    force: bool,
}

/// #1147: how an entry in the outbox changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Out {
    /// A new entry.
    New,
    /// A new version of an entry that changes in place.
    Version,
    /// An entry seeded from the file (written redacted already).
    Seeded,
}

/// The most bytes of changed entries that left the ring before a batch took them, kept for that batch (a flood between two batches);
/// older ones beyond it are not written.
const EVICTED_MAX: usize = 1024 * 1024;

/// What of one transcript the writer has not taken yet. The chat path only marks ids here; the writer clones the entries when it takes
/// a batch, so an entry that changed many times is written once, as it is then.
struct Save {
    wake: Arc<Wake>,
    /// Entries in the ring that changed since the last batch.
    dirty: BTreeSet<u64>,
    /// Changed entries that left the ring before the batch took them, with what was kept beside them.
    evicted: VecDeque<(ChatEntry, Side)>,
    evicted_bytes: usize,
    /// Entries taken back out of the transcript (a prompt not sent, queued prompts dropped).
    removed: Vec<u64>,
    /// About how many bytes changed since the last batch; at [`crate::transcript::BATCH_BYTES`] the writer is woken.
    bytes: usize,
    woken: bool,
    /// `chat.history_not_saved` was said in this conversation.
    said: bool,
    /// A fresh start: the earlier session's file is moved aside (keeping this highest id) before the next batch is written.
    fresh: Option<u64>,
}

impl Save {
    fn new(wake: Arc<Wake>) -> Self {
        Self {
            wake,
            dirty: BTreeSet::new(),
            evicted: VecDeque::new(),
            evicted_bytes: 0,
            removed: Vec::new(),
            bytes: 0,
            woken: false,
            said: false,
            fresh: None,
        }
    }

    fn grew(&mut self, bytes: usize) {
        self.bytes += bytes;
        if self.bytes >= crate::transcript::BATCH_BYTES && !self.woken {
            self.woken = true;
            self.wake.notify();
        }
    }
}

/// An agent's transcript, made saving when the board saves.
fn slot<'a>(
    g: &'a mut HashMap<AgentId, Transcript>,
    agent: &AgentId,
    wake: Option<&Arc<Wake>>,
) -> &'a mut Transcript {
    g.entry(agent.clone()).or_insert_with(|| Transcript {
        save: wake.map(|w| Save::new(Arc::clone(w))),
        ..Default::default()
    })
}

/// The transcripts of every chat agent on this node.
#[derive(Default)]
pub struct ChatBoard {
    agents: Arc<Mutex<HashMap<AgentId, Transcript>>>,
    next_follower_id: AtomicU64,
    /// ADR-0089, ADR-0104 C1: where the transcripts are saved, and what wakes their writer; `None` on a board that keeps memory only.
    store: Option<Arc<Store>>,
    wake: Option<Arc<Wake>>,
    /// ADR-0108: the agents whose files are to be removed, each with where to say it was done (`true`) or failed, in the order forgotten.
    forgets: Mutex<Vec<(AgentId, SyncSender<bool>)>>,
    /// ADR-0108: the transcript writer runs ([`crate::transcript::spawn_writer`]); when it does not, a forget removes the files itself.
    writer: AtomicBool,
}

/// #1147: the board's lock. When it is let go, what the operation changed is sent to followers (`Transcript::deliver`), still under
/// the lock and in order, once the operation has also recorded what goes beside its entries (their cuts, a call's tool and
/// arguments), which the redaction needs. No operation can forget to send.
struct Locked<'a> {
    g: std::sync::MutexGuard<'a, HashMap<AgentId, Transcript>>,
    store: Option<&'a Store>,
}

impl std::ops::Deref for Locked<'_> {
    type Target = HashMap<AgentId, Transcript>;
    fn deref(&self) -> &Self::Target {
        &self.g
    }
}

impl std::ops::DerefMut for Locked<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.g
    }
}

impl Drop for Locked<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        let mut sec: Option<Arc<Secrets>> = None;
        for t in self.g.values_mut() {
            if t.live.outbox.is_empty() && !t.live.force {
                continue;
            }
            if sec.is_none() {
                sec = self.store.map(Store::current);
            }
            t.deliver(sec.as_deref());
        }
    }
}

/// A live chat subscription. Dropping it unregisters its follower even if the transcript stays quiet.
pub struct ChatSubscription {
    rx: Receiver<ChatEntry>,
    agents: Weak<Mutex<HashMap<AgentId, Transcript>>>,
    agent: AgentId,
    follower_id: u64,
}

impl ChatSubscription {
    pub fn recv_timeout(&self, timeout: Duration) -> Result<ChatEntry, RecvTimeoutError> {
        self.rx.recv_timeout(timeout)
    }

    #[cfg(test)]
    fn try_recv(&self) -> Result<ChatEntry, std::sync::mpsc::TryRecvError> {
        self.rx.try_recv()
    }

    #[cfg(test)]
    fn try_iter(&self) -> std::sync::mpsc::TryIter<'_, ChatEntry> {
        self.rx.try_iter()
    }
}

impl Drop for ChatSubscription {
    fn drop(&mut self) {
        let Some(agents) = self.agents.upgrade() else {
            return;
        };
        let mut agents = agents.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(transcript) = agents.get_mut(&self.agent) {
            transcript
                .followers
                .retain(|(id, _)| *id != self.follower_id);
        }
    }
}

/// [`cap_end`] at the longest text of one entry, the place dropped.
fn cut(text: String) -> String {
    cap_end(text, MAX_ENTRY_BYTES).0
}

/// [`cap_end`] at the longest text of one entry.
fn cut_at(text: String) -> (String, Option<usize>) {
    cap_end(text, MAX_ENTRY_BYTES)
}

/// The text of a tool call entry: the tool and its arguments as JSON (what an old client shows). The transcript writer makes it
/// again from the redacted tool and arguments, so it is redacted exactly as they are.
pub(crate) fn tool_text(tool: &str, arguments_json: &str) -> String {
    cut(format!(
        "{{\"tool\":{},\"arguments\":{}}}",
        json_string(tool),
        if arguments_json.is_empty() {
            "null"
        } else {
            arguments_json
        }
    ))
}

/// The text of a plan entry: its steps' lines joined (what an old client shows). The transcript writer makes it again from the
/// redacted steps.
pub(crate) fn plan_text(steps: &[PlanStep]) -> String {
    cut(steps
        .iter()
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join("\n"))
}

/// What an adapter's chat item looks like as an entry (kind, text, path).
pub fn describe(item: &ChatItem) -> (ChatKind, String, String) {
    match item {
        ChatItem::Text(t) => (ChatKind::Text, t.clone(), String::new()),
        ChatItem::ToolCall {
            tool,
            arguments_json,
        } => (
            ChatKind::ToolCall,
            format!(
                "{{\"tool\":{},\"arguments\":{}}}",
                json_string(tool),
                if arguments_json.is_empty() {
                    "null"
                } else {
                    arguments_json
                }
            ),
            String::new(),
        ),
        ChatItem::Plan(lines) => (ChatKind::Plan, lines.join("\n"), String::new()),
        ChatItem::Diff { path, unified_diff } => {
            // ADR-0080: what a secret file's change says is not shown, only that it changed and where.
            let text = if is_secret_file(path) {
                String::new()
            } else {
                unified_diff.clone()
            };
            (ChatKind::Diff, text, path.clone())
        }
        // The code and parameters travel in their own fields (`notice`).
        ChatItem::Notice { .. } => (ChatKind::Notice, String::new(), String::new()),
        ChatItem::Thought(t) => (ChatKind::Thought, t.clone(), String::new()),
        // These two have entries of their own (`upsert_tool`, `upsert_plan`); this is only what an old reader would show.
        ChatItem::Tool(t) => (
            ChatKind::ToolCall,
            tool_text(t.tool.as_deref().unwrap_or("tool"), ""),
            String::new(),
        ),
        ChatItem::PlanSteps(steps) => (
            ChatKind::Plan,
            steps
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            String::new(),
        ),
    }
}

/// A word of lowercase letters, digits and `_`.
fn word(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// ADR-0058: a notice's code and parameters as an entry keeps them, or `None` when the code is not a dotted lowercase code
/// (`adapter.turn_failed`). Clients turn the code into a message id, so nothing else is let through; parameters with other keys are
/// dropped and long values cut.
pub fn notice(
    code: &str,
    params: &[(String, String)],
) -> Option<(String, std::collections::HashMap<String, String>)> {
    notice_cuts(code, params).map(|(code, params, _)| (code, params))
}

/// The parameters a notice keeps, by key.
type Params = std::collections::HashMap<String, String>;

/// Where each of a notice's parameters was cut, by key.
type ParamCuts = Vec<(String, Vec<usize>)>;

/// [`notice`], and where each parameter was cut (review of #1103, round 4).
fn notice_cuts(code: &str, params: &[(String, String)]) -> Option<(String, Params, ParamCuts)> {
    if code.len() > MAX_CODE || !code.contains('.') || !code.split('.').all(word) {
        return None;
    }
    let mut cuts = Vec::new();
    let params = params
        .iter()
        .filter(|(k, _)| k.len() <= MAX_PARAM_KEY && word(k))
        .take(MAX_PARAMS)
        .map(|(k, v)| {
            let (v, end) = cap_end(v.clone(), MAX_PARAM_BYTES);
            if let Some(end) = end {
                cuts.push((k.clone(), vec![end]));
            }
            (k.clone(), v)
        })
        .collect();
    Some((code.to_string(), params, cuts))
}

/// ADR-0111: the places an adapter reported in one of an item's strings.
fn reported(cuts: &[ReportedCut], field: &CutField) -> Vec<usize> {
    cuts.iter()
        .filter(|c| c.field == *field)
        .map(|c| c.at)
        .collect()
}

/// ADR-0111: where the places an adapter reported in a call's tool name or arguments, or in a plan's steps, land in the text
/// [`describe`] makes of the item (`{"tool":<the name as JSON>,"arguments":<the arguments>}`; the steps joined by line breaks). One
/// pass over the name or the steps, however many places there are; a place past the longest entry is left out (the text is cut
/// before it, and its end is a place then).
fn composed_text_cuts(item: &ChatItem, cuts: &[ReportedCut]) -> Vec<usize> {
    const HEAD: usize = "{\"tool\":".len();
    const ARGS: usize = ",\"arguments\":".len();
    let mut out = Vec::new();
    match item {
        ChatItem::ToolCall {
            tool,
            arguments_json,
        } => {
            let mut name: Vec<usize> = cuts
                .iter()
                .filter(|c| c.field == CutField::Tool && c.at <= MAX_ENTRY_BYTES)
                .map(|c| c.at)
                .collect();
            name.sort_unstable();
            let args: Vec<usize> = cuts
                .iter()
                .filter(|c| c.field == CutField::Arguments && c.at <= MAX_ENTRY_BYTES)
                .map(|c| c.at)
                .collect();
            if name.is_empty() && (args.is_empty() || arguments_json.is_empty()) {
                return out;
            }
            // The name's characters before each place, escaped as JSON, after its opening quote; the whole name only when a place
            // in the arguments needs its length.
            let mut escaped = 0;
            let mut k = 0;
            for (i, ch) in tool.char_indices() {
                while k < name.len() && name[k] <= i {
                    out.push(HEAD + 1 + escaped);
                    k += 1;
                }
                if k == name.len() && (args.is_empty() || arguments_json.is_empty()) {
                    break;
                }
                escaped += json_len(ch);
            }
            out.extend(name[k..].iter().map(|_| HEAD + 1 + escaped));
            if !arguments_json.is_empty() {
                out.extend(args.iter().map(|a| HEAD + escaped + 2 + ARGS + a));
            }
        }
        ChatItem::Plan(lines) => {
            let mut steps: Vec<(usize, usize)> = cuts
                .iter()
                .filter_map(|c| match c.field {
                    CutField::Step(i) if c.at <= MAX_ENTRY_BYTES => Some((i, c.at)),
                    _ => None,
                })
                .collect();
            steps.sort_unstable();
            let mut k = 0;
            let mut start = 0;
            for (i, line) in lines.iter().enumerate() {
                if k == steps.len() || start > MAX_ENTRY_BYTES {
                    break;
                }
                while k < steps.len() && steps[k].0 == i {
                    out.push(start + steps[k].1);
                    k += 1;
                }
                start += line.len() + 1;
            }
        }
        _ => {}
    }
    out
}

/// The length of a character as [`json_string`] writes it.
fn json_len(c: char) -> usize {
    match c {
        '"' | '\\' | '\n' | '\r' | '\t' => 2,
        c if (c as u32) < 0x20 => 6,
        c => c.len_utf8(),
    }
}

fn json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A text from an adapter made safe and bounded: at most `PRE_BOUND` bytes of it are looked at (head and tail), control characters and
/// escape sequences are removed, and it is cut to `max` bytes. `true` when something was cut. Done before any lock is taken.
/// ADR-0080: whether `path` names a secret file, also with each of its parts trimmed: cleaning turns a line break or a tab into a
/// space (`cfg/.env\n` becomes `cfg/.env `), and the name is still the secret one (review of #1221).
fn is_secret_file(path: &str) -> bool {
    agora_core::secrets::is_secret_path(path)
        || agora_core::secrets::is_secret_path(
            &path
                .split(['/', '\\'])
                .map(str::trim)
                .collect::<Vec<_>>()
                .join("/"),
        )
}

fn tidy(text: &str, max: usize, style: Style) -> (String, bool) {
    let (out, cut, _) = tidy_cuts(text, max, style);
    (out, cut)
}

/// [`tidy`], and the places in the result where something was cut out (review of #1103, N1): the transcript writer looks for a
/// piece of a secret on each side of them. The `PRE_BOUND` cut is made first and each side cleaned apart, so where it lands is
/// known however much cleaning took away; then the `max` cut. The isolates are balanced last, in each part between two places
/// apart, so the places recorded are exact.
fn tidy_cuts(text: &str, max: usize, style: Style) -> (String, bool, Vec<usize>) {
    tidy_cuts_at(text, max, style, &[])
}

/// [`tidy_cuts`], with the places an adapter reported it cut `text` (ADR-0111: byte offsets in `text` as sent, character boundaries,
/// its end included). The text is cleaned as without them; each lands where cleaning puts the text around it
/// (`agora_adapters::text::strip_core_at`) and is recorded like the node's own places (also at the very end of the result, where the
/// node's are not). One in the middle a cut dropped is gone with it (that cut is a place itself). The `bool` says only whether the
/// node cut something.
fn tidy_cuts_at(
    text: &str,
    max: usize,
    style: Style,
    reported: &[usize],
) -> (String, bool, Vec<usize>) {
    let (bounded, first) = cap_middle(text.to_string(), PRE_BOUND);
    // The reported places in `bounded`, in order. One in the middle the first cut dropped is gone (that cut is a place itself).
    let mut places: Vec<usize> = reported
        .iter()
        .filter_map(|&r| match first {
            Some((h, tail)) if r > h && r < tail => None,
            Some((h, tail)) if r >= tail => Some(r - tail + h),
            _ => Some(r),
        })
        .filter(|&r| bounded.is_char_boundary(r))
        .collect();
    places.sort_unstable();
    places.dedup();
    // Where each place is in the cleaned text, and whether an adapter reported it. The text is cleaned apart only at the node's own
    // first cut; a reported place never splits the cleaning (inside an escape sequence or a `\r\n` it lands where cleaning the whole
    // puts it), so it never changes what is kept.
    let mut seams: Vec<(usize, bool)> = Vec::with_capacity(places.len() + 2);
    let cleaned = match first {
        Some((h, _)) => {
            let split = places.partition_point(|&p| p <= h);
            let (head, at) = strip_core_at(&bounded[..h], style, &places[..split]);
            seams.extend(at.into_iter().map(|a| (a, true)));
            let seam = head.len();
            seams.push((seam, false));
            let tail_places: Vec<usize> = places[split..].iter().map(|p| p - h).collect();
            let (tail, at) = strip_core_at(&bounded[h..], style, &tail_places);
            seams.extend(at.into_iter().map(|a| (a + seam, true)));
            head + &tail
        }
        None => {
            let (cleaned, at) = strip_core_at(&bounded, style, &places);
            seams.extend(at.into_iter().map(|a| (a, true)));
            cleaned
        }
    };
    let (out, last) = cap_middle(cleaned, max);
    if let Some((h, dropped)) = last {
        // A place in the dropped middle is gone; one in the kept tail moves with it.
        seams = seams
            .into_iter()
            .filter_map(|(s, r)| match s {
                s if s < h => Some((s, r)),
                s if s >= dropped => Some((s - dropped + h, r)),
                _ => None,
            })
            .chain(std::iter::once((h, false)))
            .collect();
    }
    seams.sort_unstable();
    seams.dedup_by(|b, a| {
        a.0 == b.0 && {
            a.1 |= b.1;
            true
        }
    });
    // Isolates are balanced last, in each part between two places apart (review of #1103, B6): what the balance removes cannot move a
    // place, and the whole is balanced as each part is.
    let node_cut = first.is_some() || last.is_some();
    if seams.is_empty() {
        return (balanced(out, style), node_cut, Vec::new());
    }
    let mut joined = String::with_capacity(out.len());
    let mut kept = Vec::with_capacity(seams.len());
    let mut at = 0;
    for (seam, reported) in seams.into_iter().chain(std::iter::once((out.len(), false))) {
        joined.push_str(&balanced(out[at..seam].to_string(), style));
        // The node's own place at the very end is no place (the text goes on nowhere); a reported one there is (an end dropped).
        if seam < out.len() || reported {
            kept.push(joined.len());
        }
        at = seam;
    }
    (joined, node_cut, kept)
}

/// ACP `ToolKind` as the protocol's; a word this build does not know is `UNSPECIFIED`.
fn tool_kind(word: &str) -> ToolKind {
    match word {
        "read" => ToolKind::Read,
        "edit" => ToolKind::Edit,
        "delete" => ToolKind::Delete,
        "move" => ToolKind::Move,
        "search" => ToolKind::Search,
        "execute" => ToolKind::Execute,
        "think" => ToolKind::Think,
        "fetch" => ToolKind::Fetch,
        "switch_mode" => ToolKind::SwitchMode,
        "other" => ToolKind::Other,
        _ => ToolKind::Unspecified,
    }
}

/// ACP `ToolCallStatus` (and a plan entry's status) as the protocol's.
fn tool_status(word: &str) -> ToolStatus {
    match word {
        "pending" => ToolStatus::Pending,
        "in_progress" => ToolStatus::InProgress,
        "completed" => ToolStatus::Completed,
        "failed" => ToolStatus::Failed,
        _ => ToolStatus::Unspecified,
    }
}

fn plan_status(word: &str) -> ToolStatus {
    // A plan step is never failed.
    match tool_status(word) {
        ToolStatus::Failed => ToolStatus::Unspecified,
        s => s,
    }
}

fn plan_priority(word: &str) -> PlanPriority {
    match word {
        "high" => PlanPriority::High,
        "medium" => PlanPriority::Medium,
        "low" => PlanPriority::Low,
        _ => PlanPriority::Unspecified,
    }
}

fn finished(status: i32) -> bool {
    status == ToolStatus::Completed as i32 || status == ToolStatus::Failed as i32
}

impl Transcript {
    fn index(&self, id: u64) -> Option<usize> {
        self.entries.binary_search_by_key(&id, |e| e.entry_id).ok()
    }

    fn size(e: &ChatEntry) -> usize {
        e.text.len()
            + e.path.len()
            + e.code.len()
            + e.params
                .iter()
                .map(|(k, v)| k.len() + v.len())
                .sum::<usize>()
            + e.call_id.len()
            + e.output.len()
            + e.approval_id.len()
            + e.paths.iter().map(|p| p.path.len()).sum::<usize>()
            + e.plan_steps.iter().map(|s| s.text.len()).sum::<usize>()
    }

    fn over(&self) -> bool {
        self.entries.len() > MAX_ENTRIES || self.bytes > MAX_BYTES
    }

    /// Keeps the ring within its bounds, and forgets what pointed at an entry that left it. The agent's words not sent to followers
    /// yet stay until they are (#1147): the operation's end sends them and bounds the ring again (`deliver`).
    fn evict(&mut self) {
        while self.over() {
            if self
                .entries
                .front()
                .is_some_and(|e| self.live.unsent.contains_key(&e.entry_id))
            {
                break;
            }
            let Some(old) = self.entries.pop_front() else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(Self::size(&old));
            self.keep_unsaved(&old);
            self.forget(&old);
        }
    }

    /// What is kept beside an entry for the file: its call's tool and arguments, and whether it is the plan.
    fn side(&self, e: &ChatEntry) -> Side {
        Side {
            tool: (!e.call_id.is_empty())
                .then(|| self.calls.get(&e.call_id))
                .flatten()
                .filter(|c| c.entry == e.entry_id)
                .map(|c| (c.tool.clone(), c.args.clone())),
            plan: self.plan == Some(e.entry_id),
            cuts: {
                let mut c = self.cuts.get(&e.entry_id).cloned().unwrap_or_default();
                // A tool call still running: its output and arguments may stop inside a value the next version completes.
                c.provisional = e.kind == ChatKind::ToolCall as i32 && !finished(e.status);
                Box::new(c)
            },
        }
    }

    /// Review of #1103 (rounds 2 and 3): the words the writer holds back: while the agent is in a turn, each of its words said since it
    /// last left one with fewer than `span - 1` bytes of words after it, where `span` is the longest secret value. A value that starts
    /// in a word is then whole before any part of it is written. Nothing is held when the agent is not in a turn: words it says then
    /// are written at the next batch, and the end of what is written counts as a place cut (see `take`). A kill loses what is held.
    fn held(&self, span: usize) -> BTreeSet<u64> {
        let mut held = BTreeSet::new();
        if !self.working {
            return held;
        }
        let mut after = 0usize;
        for e in self
            .entries
            .iter()
            .rev()
            .filter(|e| in_run(e))
            .take_while(|e| e.entry_id > self.closed_upto)
        {
            if after + 1 >= span {
                break;
            }
            held.insert(e.entry_id);
            after += e.text.len();
        }
        held
    }

    /// Notes where the node cut an entry's texts (only on a board that saves; replaces what an update of a field sent again).
    fn note_cuts(&mut self, id: u64, f: impl FnOnce(&mut Cuts)) {
        if self.save.is_some() {
            let c = self.cuts.entry(id).or_default();
            f(c);
            if c.is_empty() {
                self.cuts.remove(&id);
            }
        }
    }

    /// An entry leaves the ring: one with a change not written yet goes to the next batch as it is (bounded, oldest dropped first).
    fn keep_unsaved(&mut self, old: &ChatEntry) {
        if !self
            .save
            .as_ref()
            .is_some_and(|s| s.dirty.contains(&old.entry_id))
        {
            return;
        }
        let side = self.side(old);
        let size = Self::size(old);
        if let Some(s) = &mut self.save {
            s.dirty.remove(&old.entry_id);
            s.evicted_bytes += size;
            s.evicted.push_back((old.clone(), side));
            let mut gap = false;
            while s.evicted_bytes > EVICTED_MAX {
                let Some((e, _)) = s.evicted.pop_front() else {
                    break;
                };
                s.evicted_bytes = s.evicted_bytes.saturating_sub(Self::size(&e));
                gap |= in_run(&e);
            }
            if gap {
                // Words dropped here are never written: in the file, what was written before (the carry) lies right beside what is
                // written next, and the place between is an edge (review of #1103, round 4).
                self.carry_edges.push(self.carry.len());
                if self.carry.is_empty() {
                    self.carry_open = true;
                }
            }
        }
    }

    /// An entry changed (or is new): the next batch writes it.
    fn changed(&mut self, id: u64, bytes: usize) {
        // Counted once per batch: an entry that changes many times (a tool call's output) is written once, as it is then, so its
        // changes neither wake the writer early nor fill the file with copies.
        if let Some(s) = &mut self.save {
            if s.dirty.insert(id) {
                s.grew(bytes);
            }
        }
    }

    /// Takes the entry at `i` out of the transcript (a new subscriber never sees it, and the file says it went).
    fn take_out(&mut self, i: usize) {
        if let Some(old) = self.entries.remove(i) {
            self.bytes = self.bytes.saturating_sub(Self::size(&old));
            self.forget(&old);
            if let Some(s) = &mut self.save {
                s.dirty.remove(&old.entry_id);
                s.removed.push(old.entry_id);
                s.grew(16);
            }
        }
    }

    /// [`ChatBoard::drop_queued`] on this transcript.
    fn drop_queued(&mut self, entry_ids: &[u64], now_ms: u64) {
        for id in entry_ids {
            if let Some(i) = self.index(*id) {
                self.take_out(i);
            }
        }
        let mut n = entry(ChatRole::Agent, ChatKind::Notice, String::new(), now_ms);
        n.code = "adapter.queue_dropped".into();
        let ids: Vec<String> = entry_ids.iter().map(u64::to_string).collect();
        n.params = HashMap::from([
            ("count".to_string(), entry_ids.len().to_string()),
            ("entries".to_string(), ids.join(",")),
        ]);
        self.add(n, now_ms);
    }

    /// ADR-0104 C1: fills an empty transcript (one that never gave an id) from what its file gave back, and numbers on from the
    /// highest id the file shows. Rebuilds the calls and the plan that change in place; a permission entry still pending is shown
    /// cancelled (this node holds no approval of the node before it: a request still open is asked again under a new id), and a
    /// prompt still queued is dropped as the queue that held it ended with that node. Followers (a window that already follows a
    /// resumed agent) get the entries. False when the transcript already gave ids.
    fn seed(&mut self, seed: Seed, now_ms: u64) -> bool {
        if self.next_id > 0 || !self.entries.is_empty() {
            return false;
        }
        // Review of #1103 (N4): what the file says is held to the live path's shapes and bounds before it is kept.
        let entries: Vec<(ChatEntry, Side)> = seed
            .entries
            .into_iter()
            .filter_map(|(e, side)| seeded_entry(e, side))
            .collect();
        let high = seed
            .high
            .max(entries.last().map_or(0, |(e, _)| e.entry_id))
            .min(MAX_SEEDED_ID);
        for (e, side) in entries {
            if let (Some((tool, args)), false) = (side.tool, e.call_id.is_empty()) {
                self.calls.insert(
                    e.call_id.clone(),
                    Call {
                        entry: e.entry_id,
                        tool,
                        args,
                    },
                );
            }
            if side.plan {
                self.plan = Some(e.entry_id);
            }
            self.bytes += Self::size(&e);
            self.entries.push_back(e);
        }
        self.next_id = high;
        // The earlier node's words are all written; whether its last ones ended inside a value is not known here. Its last words
        // as written (redacted, with up to 3 bytes of a value beside a mark or an edge) become the carry, and their end an edge:
        // what this node writes next is looked at together with them (review of #1103, round 3).
        self.closed_upto = high;
        let mut written = String::new();
        let mut bounds = vec![0];
        for e in self.entries.iter().filter(|e| in_run(e)) {
            written.push_str(&e.text);
            bounds.push(written.len());
        }
        self.carry = tail_of(&written, agora_adapters::text::MAX_SECRET_SPAN);
        // Every boundary between the earlier words counts as an edge: the file does not say which of them were a stop's flush or
        // an earlier take-up, beside which pieces of under 4 bytes of a value may have been written (review of #1103, round 4).
        // Their end is where this node takes over.
        let from = written.len() - self.carry.len();
        self.carry_edges = bounds
            .into_iter()
            .filter_map(|b| b.checked_sub(from))
            .collect();
        self.carry_open = false;
        // #1147: what followers were sent before is the file too, so the live stream goes on from the same words and edges.
        self.live.carry = self.carry.clone();
        self.live.carry_edges = self.carry_edges.clone();
        self.live.open = false;
        self.live.at_end = AtEnd::default();
        self.evict();
        // Every seeded entry goes to followers (a window that already follows a resumed agent) as written, redacted again.
        let ids: Vec<u64> = self.entries.iter().map(|e| e.entry_id).collect();
        for id in ids {
            self.out(id, Out::Seeded);
        }
        let pending: Vec<u64> = self
            .entries
            .iter()
            .filter(|e| {
                e.kind == ChatKind::Permission as i32
                    && e.permission_outcome == PermissionOutcome::Pending as i32
            })
            .map(|e| e.entry_id)
            .collect();
        for id in pending {
            self.modify(id, now_ms, false, |e| {
                e.permission_outcome = PermissionOutcome::Cancelled as i32;
            });
        }
        let queued: Vec<u64> = self
            .entries
            .iter()
            .filter(|e| e.queued)
            .map(|e| e.entry_id)
            .collect();
        if !queued.is_empty() {
            self.drop_queued(&queued, now_ms);
        }
        true
    }

    fn forget(&mut self, e: &ChatEntry) {
        if !e.call_id.is_empty()
            && self
                .calls
                .get(&e.call_id)
                .is_some_and(|c| c.entry == e.entry_id)
        {
            self.calls.remove(&e.call_id);
        }
        if self.plan == Some(e.entry_id) {
            self.plan = None;
        }
        self.sent_ms.remove(&e.entry_id);
        self.dirty.remove(&e.entry_id);
        self.cuts.remove(&e.entry_id);
        self.live.shown.remove(&e.entry_id);
        self.live.unsent.remove(&e.entry_id);
    }

    fn send(&mut self, entry: &ChatEntry) {
        // A follower that fell `FOLLOWER_QUEUE` entries behind is dropped here.
        self.followers
            .retain(|(_, sender)| sender.try_send(entry.clone()).is_ok());
    }

    /// Numbers an entry, keeps it, and hands it to every follower when the operation ends (#1147: `deliver`). Returns its id.
    fn add(&mut self, mut entry: ChatEntry, now_ms: u64) -> u64 {
        // Seeding keeps the id far below the end of the range (`MAX_SEEDED_ID`), so this never saturates in practice; if it did, ids
        // would repeat rather than the node panic or wrap under the board's lock.
        self.next_id = self.next_id.saturating_add(1);
        entry.entry_id = self.next_id;
        let (id, size) = (entry.entry_id, Self::size(&entry));
        self.bytes += size;
        self.entries.push_back(entry);
        self.sent_ms.insert(id, now_ms);
        self.changed(id, size);
        self.evict();
        self.out(id, Out::New);
        id
    }

    /// #1147: an entry to send to followers when the operation ends (once, at the place it was first marked, as it is then).
    fn out(&mut self, id: u64, how: Out) {
        if !self.live.outbox.iter().any(|(i, _)| *i == id) {
            self.live.outbox.push((id, how));
        }
    }

    /// #1147: sends what the operation changed, in order. With no secret value held, every entry goes as it is, as before. With one:
    /// an entry that is not the agent's words goes as the writer redacts its line (each version on its own); the agent's words wait
    /// in `unsent` and go when [`Self::release`] lets them, every word held going first when any other new entry comes (so ids still
    /// increase on the stream, which a client may rely on).
    fn deliver(&mut self, sec: Option<&Secrets>) {
        let outbox = std::mem::take(&mut self.live.outbox);
        let force = std::mem::take(&mut self.live.force);
        let Some(sec) = sec.filter(|s| !s.is_empty()) else {
            // Whatever went now went with no value held: a later snapshot with one redacts the words again.
            self.live.words_gen = Some(0);
            // No value held (or none any more): words held while one was go first, as they are.
            let held: Vec<u64> = std::mem::take(&mut self.live.unsent).into_keys().collect();
            for id in held.into_iter().chain(outbox.into_iter().map(|(id, _)| id)) {
                if self.followers.is_empty() {
                    break;
                }
                if let Some(i) = self.index(id) {
                    let e = self.entries[i].clone();
                    self.send(&e);
                }
            }
            self.evict();
            return;
        };
        if !self.live.at_end.of(sec) {
            let mut text = self.live.carry.clone();
            for id in self.live.unsent.keys() {
                if let Some(i) = self.index(*id) {
                    text.push_str(&self.entries[i].text);
                }
            }
            self.live.at_end = sec.at_end(&text);
        }
        for (id, how) in outbox {
            let Some(i) = self.index(id) else {
                continue;
            };
            let words = in_run(&self.entries[i]);
            match how {
                Out::New if words => {
                    self.live.unsent.insert(id, self.entries[i].at_ms);
                    sec.feed(&mut self.live.at_end, &self.entries[i].text);
                }
                Out::Seeded if words => {
                    self.live.words_gen = Some(0);
                    let e = &self.entries[i];
                    let view = redact_entry(e, self.side(e), sec);
                    self.show(view);
                }
                _ => {
                    if how == Out::New && !self.live.unsent.is_empty() {
                        self.release(sec, true);
                    }
                    if !self.followers.is_empty() {
                        if let Some(i) = self.index(id) {
                            let e = &self.entries[i];
                            let view = redact_entry(e, self.side(e), sec);
                            self.send(&view);
                        }
                    }
                }
            }
        }
        self.release(sec, force);
        if self.over() {
            // Words held at the ring's front kept it over its bounds: they go now, and it is bounded again.
            self.release(sec, true);
            self.evict();
        }
    }

    /// #1147: how many of the words not sent (`ids`, in order) may go now: those with at least as many bytes after them as the part
    /// of a value the stream ends in, when that part is 4 bytes or more (what mechanism (b) would replace were the end an edge), and
    /// as a value with no automaton is long less one byte.
    fn releasable(&self, sec: &Secrets, ids: &[u64]) -> usize {
        let part = self.live.at_end.part();
        let mut wait = if part >= MIN_PART { part } else { 0 };
        if sec.unpieced() > 0 {
            wait = wait.max(sec.unpieced() - 1);
        }
        // The bytes after each piece, from the newest back: the pieces with fewer than `wait` after them stay.
        let mut after = 0usize;
        let mut stay = 0usize;
        for id in ids.iter().rev() {
            if after >= wait {
                break;
            }
            stay += 1;
            after += self.index(*id).map_or(0, |i| self.entries[i].text.len());
        }
        ids.len() - stay
    }

    /// #1147: sends the agent's words that may go ([`Self::releasable`]; all of them when `force`, their end then an edge, as the
    /// writer's end of words written without what may follow). They are redacted as the writer redacts a batch: one run of the
    /// stream sent so far (the carry), the words sent now, and the words still held after them, with every edge the writer would use.
    fn release(&mut self, sec: &Secrets, force: bool) {
        if self.live.unsent.is_empty() {
            return;
        }
        let ids: Vec<u64> = self.live.unsent.keys().copied().collect();
        let n = if force {
            ids.len()
        } else {
            self.releasable(sec, &ids)
        };
        if n == 0 {
            return;
        }
        let reach = sec.span().saturating_sub(1);
        let mut run = Run::default();
        run.text.push_str(&self.live.carry);
        if !run.text.is_empty() {
            run.parts.push(RunPart {
                line: None,
                start: 0,
                end: run.text.len(),
            });
        } else if self.live.open {
            self.live.carry_edges = vec![0];
        }
        run.edges.extend(self.live.carry_edges.iter().copied());
        let mut lines = Vec::new();
        let mut cut_edges = Vec::new();
        for id in ids.iter().take(n) {
            let Some(i) = self.index(*id) else {
                continue;
            };
            let e = &self.entries[i];
            let side = self.side(e);
            let at = run.text.len();
            run.text.push_str(&e.text);
            cut_edges.extend(side.cuts.text.iter().map(|c| at + c));
            run.parts.push(RunPart {
                line: Some(lines.len()),
                start: at,
                end: run.text.len(),
            });
            lines.push(Line::Entry(Box::new(e.clone()), side));
        }
        let written = run.text.len();
        let mut ahead = 0usize;
        for id in ids.iter().skip(n) {
            if ahead >= reach {
                break;
            }
            let Some(i) = self.index(*id) else {
                continue;
            };
            let e = &self.entries[i];
            let at = run.text.len();
            run.text.push_str(&e.text);
            ahead += e.text.len();
            if let Some(c) = self.cuts.get(id) {
                run.edges.extend(c.text.iter().map(|c| at + c));
            }
            run.parts.push(RunPart {
                line: None,
                start: at,
                end: run.text.len(),
            });
        }
        if force {
            run.edges.push(written);
        }
        run.edges.extend(cut_edges.iter().copied());
        let views = redact_lines(&lines, std::slice::from_ref(&run), sec);
        self.live.carry = tail_of(&run.text[..written], reach);
        let from = written - self.live.carry.len();
        self.live.carry_edges = self
            .live
            .carry_edges
            .iter()
            .chain(&cut_edges)
            .filter_map(|e| e.checked_sub(from))
            .collect();
        self.live.open = false;
        for id in ids.iter().take(n) {
            self.live.unsent.remove(id);
        }
        self.live.words_gen = match self.live.words_gen {
            None => Some(sec.generation()),
            Some(g) if g == sec.generation() => Some(g),
            _ => Some(0),
        };
        for view in views {
            if let Line::Entry(v, _) = view {
                self.show(*v);
            }
        }
    }

    /// #1147: the agent's words as followers are sent them: kept beside the ring where they differ from it (a snapshot shows them so,
    /// never redacted again with another stream), then sent.
    fn show(&mut self, view: ChatEntry) {
        let id = view.entry_id;
        if self.index(id).is_some_and(|i| self.entries[i] == view) {
            self.live.shown.remove(&id);
        } else {
            self.live.shown.insert(id, view.clone());
        }
        self.send(&view);
    }

    /// #1147: what a new follower's snapshot holds: every entry as followers were sent it, the words still held left out (they come
    /// on the stream when they go). The agent's words as they were sent; any other entry redacted as it is now (its newest version).
    fn shown_entries(&self, sec: Option<&Secrets>) -> Vec<ChatEntry> {
        let sec = sec.filter(|s| !s.is_empty());
        let mut out: Vec<ChatEntry> = self
            .entries
            .iter()
            .filter(|e| !self.live.unsent.contains_key(&e.entry_id))
            .map(|e| match (self.live.shown.get(&e.entry_id), sec) {
                (Some(v), _) => v.clone(),
                (None, Some(sec)) if !in_run(e) => redact_entry(e, self.side(e), sec),
                (None, _) => e.clone(),
            })
            .collect();
        // Audit of #1157: words shown before a value was added to the store (or seeded from the file) are redacted again as one stream
        // with the values held now, so no later snapshot shows it; the start of the ring's first word is an edge (words before it may
        // have left the ring, a value's start among them). Masks are only added to what was sent.
        if let Some(sec) = sec {
            if self.live.words_gen.is_some_and(|g| g != sec.generation()) {
                let at: Vec<usize> = (0..out.len()).filter(|&i| in_run(&out[i])).collect();
                let mut words: Vec<ChatEntry> = at.iter().map(|&i| out[i].clone()).collect();
                redact_words_again(&mut words, sec);
                for (i, w) in at.into_iter().zip(words) {
                    out[i] = w;
                }
            }
        }
        out
    }

    /// Changes an entry in place. With `coalesce`, a version is sent only when the last one went out at least [`COALESCE_MS`] ago;
    /// otherwise the entry is marked as holding a change and [`ChatBoard::flush_due`] sends it. False when the entry is gone.
    fn modify(
        &mut self,
        id: u64,
        now_ms: u64,
        coalesce: bool,
        f: impl FnOnce(&mut ChatEntry),
    ) -> bool {
        let Some(i) = self.index(id) else {
            return false;
        };
        let before = Self::size(&self.entries[i]);
        f(&mut self.entries[i]);
        let after = Self::size(&self.entries[i]);
        self.bytes = (self.bytes + after).saturating_sub(before);
        self.changed(id, after);
        let due = self
            .sent_ms
            .get(&id)
            .is_none_or(|last| now_ms < *last || now_ms - *last >= COALESCE_MS);
        if coalesce && !due {
            self.dirty.insert(id);
        } else {
            self.dirty.remove(&id);
            self.sent_ms.insert(id, now_ms);
            self.out(id, Out::Version);
        }
        self.evict();
        true
    }
}

/// An entry of the agent's words: its text or its thinking, which an agent sends in pieces.
fn in_run(e: &ChatEntry) -> bool {
    e.role == ChatRole::Agent as i32
        && (e.kind == ChatKind::Text as i32 || e.kind == ChatKind::Thought as i32)
}

/// The last `keep` bytes of `s`, from a character boundary.
fn tail_of(s: &str, keep: usize) -> String {
    let mut from = s.len().saturating_sub(keep);
    while !s.is_char_boundary(from) {
        from += 1;
    }
    s[from..].to_string()
}

/// The highest entry id seeding takes from a file (2^52): ids stay exact in the window (a JavaScript number) and can never reach the
/// end of the `u64` range however many entries come after. An entry with a higher id is not seeded.
pub const MAX_SEEDED_ID: u64 = 1 << 52;

/// Review of #1103 (N4): an entry from a transcript file, held to what the live path makes: a role and kind the protocol knows (a
/// person's entry is text), the size caps, the cleaning of what a tool printed and of labels, enum values in range, and a notice's
/// code and parameters in their shape. `None` for one that cannot be such an entry. The file is the node's own record (only the
/// node's user can write it, and no confined agent can reach it), so its person's prompts and the node's own notices are taken as
/// such, as the ledger and the policy file are.
fn seeded_entry(mut e: ChatEntry, mut side: Side) -> Option<(ChatEntry, Side)> {
    if e.entry_id == 0 || e.entry_id > MAX_SEEDED_ID {
        return None;
    }
    let role = ChatRole::try_from(e.role).ok()?;
    let kind = ChatKind::try_from(e.kind).ok()?;
    match (role, kind) {
        (ChatRole::Unspecified, _) | (_, ChatKind::Unspecified) => return None,
        (ChatRole::User, k) if k != ChatKind::Text => return None,
        _ => {}
    }
    if e.call_id.len() > MAX_CALL_ID {
        return None;
    }
    e.text = match kind {
        ChatKind::Thought => tidy(&e.text, MAX_THOUGHT_BYTES, Style::Prose).0,
        ChatKind::Permission => tidy(&e.text, MAX_PERMISSION_BYTES, Style::Label).0,
        _ => cut(std::mem::take(&mut e.text)),
    };
    e.path = tidy(&e.path, MAX_PATH_BYTES, Style::Label).0;
    if kind == ChatKind::Notice {
        let params: Vec<(String, String)> = e.params.drain().collect();
        let (code, params) = notice(&e.code, &params)?;
        (e.code, e.params) = (code, params);
    } else {
        e.code.clear();
        e.params.clear();
    }
    let (output, cut_now) = tidy(&e.output, MAX_OUTPUT_BYTES, Style::Prose);
    e.output = output;
    e.output_truncated |= cut_now;
    e.paths.truncate(MAX_PATHS);
    for p in &mut e.paths {
        p.path = tidy(&p.path, MAX_PATH_BYTES, Style::Label).0;
    }
    e.plan_steps.truncate(MAX_PLAN_STEPS);
    for st in &mut e.plan_steps {
        st.text = tidy(&st.text, MAX_PLAN_STEP_BYTES, Style::Step).0;
        st.status = plan_status_valid(st.status);
        st.priority = PlanPriority::try_from(st.priority).map_or(0, |p| p as i32);
    }
    e.tool_kind = ToolKind::try_from(e.tool_kind).map_or(0, |k| k as i32);
    e.status = ToolStatus::try_from(e.status).map_or(0, |k| k as i32);
    e.permission_outcome =
        PermissionOutcome::try_from(e.permission_outcome).map_or(0, |k| k as i32);
    e.approval_id = cap_end(std::mem::take(&mut e.approval_id), MAX_CALL_ID).0;
    side.tool = side
        .tool
        .filter(|_| kind == ChatKind::ToolCall)
        .map(|(tool, args)| {
            (
                tidy(&tool, MAX_PATH_BYTES, Style::Label).0,
                tidy(&args, MAX_ENTRY_BYTES, Style::Code).0,
            )
        });
    side.plan &= kind == ChatKind::Plan;
    Some((e, side))
}

fn plan_status_valid(status: i32) -> i32 {
    match ToolStatus::try_from(status) {
        Ok(ToolStatus::Failed) | Err(_) => 0,
        Ok(s) => s as i32,
    }
}

fn entry(role: ChatRole, kind: ChatKind, text: String, at_ms: u64) -> ChatEntry {
    ChatEntry {
        entry_id: 0,
        at_ms,
        role: role as i32,
        kind: kind as i32,
        text,
        ..Default::default()
    }
}

impl ChatBoard {
    pub fn new() -> Self {
        Self::default()
    }

    /// ADR-0089, ADR-0104 C1: a board whose transcripts are saved to `store` by its writer ([`crate::transcript::spawn_writer`]).
    pub fn saving(store: Arc<Store>) -> Self {
        // #1147: the values the live path replaces, read once now so no operation on the chat path reads the folder.
        store.current();
        Self {
            store: Some(store),
            wake: Some(Arc::new(Wake::default())),
            ..Self::default()
        }
    }

    pub fn store(&self) -> Option<Arc<Store>> {
        self.store.clone()
    }

    pub fn wake(&self) -> Option<Arc<Wake>> {
        self.wake.clone()
    }

    /// #1147: the secret values the live path replaces, as the writer last read them (`None` on a board that keeps memory only).
    fn secrets(&self) -> Option<Arc<Secrets>> {
        self.store.as_deref().map(Store::current)
    }

    /// #1147 (audit of #1157): `text` the agent chose, with the node's secret values replaced as in a chat entry, for what the node
    /// records as a hub event (an approval's action and detail, a mesh message, a task's title, an option's words).
    pub fn redact_text(&self, text: &str) -> String {
        match self.secrets() {
            Some(sec) => redact_text(text, &sec),
            None => text.to_string(),
        }
    }

    /// ADR-0108: whether the transcript writer runs; set by the writer as it starts and ends.
    pub fn set_writer_running(&self, running: bool) {
        self.writer.store(running, Ordering::Release);
    }

    /// ADR-0108: the person forgot the agent: its ring and what of it was not written yet go now (so a take-up or a resume seeds
    /// nothing from it, and no later batch carries it), and its files are removed by the writer after the batches it already took.
    /// The answer comes on the returned channel: `true` once the files are gone (also when there were none), `false` when they could
    /// not be removed. A board that saves nothing answers at once.
    pub fn forget(&self, agent: &AgentId) -> Receiver<bool> {
        let (done, answer) = sync_channel(1);
        {
            let mut g = self.lock();
            g.remove(agent);
            if self.store.is_none() {
                let _ = done.send(true);
                return answer;
            }
            self.forgets
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push((agent.clone(), done));
        }
        match (&self.wake, self.writer.load(Ordering::Acquire)) {
            (Some(wake), true) => wake.notify(),
            // No writer: nothing else touches the files, and the ring is already gone, so no batch can carry the agent.
            _ => self.run_forgets(),
        }
        answer
    }

    /// ADR-0108: removes the files of every agent forgotten so far, oldest first. Run by the writer after the batches it took, by
    /// [`crate::transcript::flush`], and by [`Self::forget`] when no writer runs.
    pub fn run_forgets(&self) {
        let jobs = std::mem::take(&mut *self.forgets.lock().unwrap_or_else(|e| e.into_inner()));
        for (agent, done) in jobs {
            let removed = self
                .store
                .as_ref()
                .is_none_or(|s| match s.delete(agent.as_str()) {
                    Ok(()) => true,
                    Err(e) => {
                        // Operational log, not UI text: the node answers the forget with a code.
                        eprintln!("transcripts: {}: not removed: {e}", agent.as_str());
                        false
                    }
                });
            let _ = done.send(removed);
        }
    }

    /// ADR-0104 C1: fills the agent's transcript from `seed` when it has given no id yet (see `Transcript::seed`).
    pub fn seed(&self, agent: &AgentId, seed: Seed, now_ms: u64) -> bool {
        let mut g = self.lock();
        slot(&mut g, agent, self.wake.as_ref()).seed(seed, now_ms)
    }

    /// ADR-0104 C1: [`Self::seed`] from the agent's file (read before the board's lock is taken), when the node takes the agent up
    /// from the holder or resumes it. A missing or unreadable file seeds nothing.
    pub fn seed_from_disk(&self, agent: &AgentId, now_ms: u64) -> bool {
        let Some(store) = &self.store else {
            return false;
        };
        let seed = store.read(agent.as_str());
        if seed.high == 0 {
            // No file (never written, purged, or the earlier node was killed before its first batch): the words that come next may
            // still continue a value the earlier node held (review of #1103, round 4).
            self.mark_taken_up(agent);
            return false;
        }
        self.seed(agent, seed, now_ms)
    }

    /// A take-up or a resume found nothing to seed: the start of the next words is an edge.
    fn mark_taken_up(&self, agent: &AgentId) {
        let mut g = self.lock();
        let t = slot(&mut g, agent, self.wake.as_ref());
        if t.next_id == 0 {
            t.carry_open = true;
            t.live.open = true;
        }
    }

    /// ADR-0060, ADR-0089 decision 5: what a resume will seed the agent's transcript with, read from its file before the resume (so
    /// no file is read under the supervisor's lock): the newest entries, or for a fresh start ("Start again") only the highest id, so
    /// the new session numbers on after the earlier one. `None` when there is nothing.
    pub fn read_seed(&self, agent: &AgentId, fresh: bool) -> Option<Seed> {
        let mut seed = self.store.as_ref()?.read(agent.as_str());
        if fresh {
            seed.entries.clear();
        }
        (seed.high > 0).then_some(seed)
    }

    /// ADR-0060, ADR-0089 decision 5: a resume went through (called before its adapter's first words are read): the transcript is
    /// seeded with what [`Self::read_seed`] gave. For a fresh start the earlier session's file is to be moved aside: only marked here
    /// (no file is touched under the supervisor's lock), and moved by the writer before it writes anything of the new session, so a
    /// batch never lands in the file being moved (review of #1103, N5 and N8). A resume that is refused calls nothing.
    pub fn resumed(&self, agent: &AgentId, seed: Option<Seed>, fresh: bool, now_ms: u64) {
        let sec = self.secrets();
        let mut g = self.lock();
        let t = slot(&mut g, agent, self.wake.as_ref());
        let high = seed.as_ref().map_or(0, |s| s.high);
        match seed {
            Some(seed) => {
                t.seed(seed, now_ms);
            }
            // Nothing to seed: the next words may still continue the earlier process's (review of #1103, round 4).
            None if t.next_id == 0 => {
                t.carry_open = true;
                t.live.open = true;
            }
            None => {}
        }
        if fresh {
            // #1147: the earlier session's words go to followers now, their end an edge. A follower sees both sessions in one
            // conversation, so the live stream goes on across them (the writer's starts again: its sessions are separate files).
            t.live.force = true;
            t.deliver(sec.as_deref());
        }
        // Marked whether or not a file exists yet: what the earlier session said and was not written yet goes to its own file
        // (review of #1103, probe 5).
        let high = high.max(t.next_id);
        if fresh {
            if let Some(s) = &mut t.save {
                s.fresh = Some(high);
                s.wake.notify();
            }
        }
    }

    /// Review of #1103 (B1): the chat a resumed session replayed before it answered its start (ACP `session/load`). An agent whose
    /// transcript already gave ids (seeded from its file, or kept in this node's memory across a restart after a crash) holds that
    /// conversation, so the replay is dropped: shown again it would be every turn twice, and written again a copy on every resume.
    /// An agent with an empty transcript (no file: purged, `chat.history_max_bytes = 0`, or a first resume after an upgrade) shows
    /// the replay as before. Returns whether it was kept.
    pub fn replayed(
        &self,
        agent: &AgentId,
        items: &[(ChatItem, Vec<ReportedCut>)],
        now_ms: u64,
    ) -> bool {
        let holds = self.lock().get(agent).is_some_and(|t| t.next_id > 0);
        if holds {
            return false;
        }
        for (item, cuts) in items {
            self.push_item_cut(agent, item, cuts, now_ms);
        }
        true
    }

    /// ADR-0089 decision 1: everything that changed since the last batch, per agent: the entries as they are now (each once), the
    /// changed ones that left the ring meanwhile, and the ids taken out. Nothing is held back: what a node stopping writes, and what
    /// tests write. Only marks are kept on the chat path: the clones are made here.
    pub fn take_unsaved(&self) -> Vec<Batch> {
        self.take(true)
    }

    /// [`Self::take_unsaved`] as the writer takes it: while an agent is in a turn and the node holds a secret value, its newest
    /// words are held back until a value starting in them is whole (review of #1103; see `Transcript::held`).
    pub fn take_due(&self, _now_ms: u64) -> Vec<Batch> {
        self.take(false)
    }

    /// Review of #1103, round 3: the agent's state, from what it reports. Leaving a turn closes the words said in it: what is held is
    /// written at the next batch, and their end is not a place cut. Words said outside a turn are written at the next batch with their
    /// end counted as a place cut (the agent may go on with them).
    pub fn set_working(&self, agent: &AgentId, working: bool) {
        let mut g = self.lock();
        let t = slot(&mut g, agent, self.wake.as_ref());
        if t.working && !working {
            t.closed_upto = t.next_id;
            if let Some(s) = &t.save {
                s.wake.notify();
            }
            // #1147: the turn's last words go to followers now (their end an edge).
            t.live.force = !t.live.unsent.is_empty();
        }
        t.working = working;
    }

    /// [`Self::set_working`] for agents that have a transcript only (the supervision tick says every agent's state each round, as a
    /// backstop for a change no event carried).
    pub fn set_working_if_known(&self, agent: &AgentId, working: bool) {
        if self.lock().contains_key(agent) {
            self.set_working(agent, working);
        }
    }

    fn take(&self, force: bool) -> Vec<Batch> {
        // The longest value looked for, read before the lock (the store caches it): 0 when the node holds none, and then nothing is
        // held back and no context gathered.
        let span = self.store.as_ref().map_or(0, |s| s.longest_secret());
        let reach = span.saturating_sub(1);
        let mut g = self.lock();
        let mut out = Vec::new();
        for (agent, t) in g.iter_mut() {
            let Some(s) = &t.save else {
                continue;
            };
            if s.dirty.is_empty()
                && s.evicted.is_empty()
                && s.removed.is_empty()
                && s.fresh.is_none()
            {
                continue;
            }
            let held = if !force && span > 0 {
                t.held(span)
            } else {
                BTreeSet::new()
            };
            let Some(s) = &mut t.save else {
                continue;
            };
            let fresh = s.fresh.take();
            let dirty = std::mem::take(&mut s.dirty);
            let evicted = std::mem::take(&mut s.evicted);
            let removed = std::mem::take(&mut s.removed);
            (s.bytes, s.evicted_bytes, s.woken) = (0, 0, false);
            // An entry of the session before a fresh start is never held: it goes to that session's file now.
            let (back, dirty): (BTreeSet<u64>, BTreeSet<u64>) = dirty
                .into_iter()
                .partition(|id| held.contains(id) && fresh.is_none_or(|h| *id > h));
            s.dirty.extend(back);
            // A fresh start splits what is taken by id: up to the earlier session's highest id goes to its file before it is moved
            // aside, the rest to the new one (review of #1103, probe 5). Each part: which ids it takes, the fresh start's mark it
            // carries, and its highest id.
            let parts: Vec<(std::ops::RangeInclusive<u64>, Option<u64>, u64)> = match fresh {
                None => vec![(0..=u64::MAX, None, t.next_id)],
                Some(h) => vec![
                    (0..=h, None, h.min(t.next_id)),
                    (h.saturating_add(1)..=u64::MAX, Some(h), t.next_id),
                ],
            };
            for (ids, mark, high) in parts {
                let mine = |id: u64| ids.contains(&id);
                let mut lines: Vec<Line> = Vec::new();
                // The agent's words written now, in order (the ones that left the ring first), with their line.
                let mut words: Vec<(u64, usize)> = Vec::new();
                for (e, side) in evicted.iter().filter(|(e, _)| mine(e.entry_id)) {
                    if span > 0 && in_run(e) {
                        words.push((e.entry_id, lines.len()));
                    }
                    lines.push(Line::Entry(Box::new(e.clone()), side.clone()));
                }
                for id in dirty.iter().copied().filter(|id| mine(*id)) {
                    if let Some(i) = t.index(id) {
                        let e = &t.entries[i];
                        if span > 0 && in_run(e) {
                            words.push((id, lines.len()));
                        }
                        lines.push(Line::Entry(Box::new(e.clone()), t.side(e)));
                    }
                }
                let gone: Vec<u64> = removed.iter().copied().filter(|id| mine(*id)).collect();
                if !gone.is_empty() {
                    lines.push(Line::Removed(gone));
                }
                if lines.is_empty() && mark.is_none() {
                    continue;
                }
                let mut runs = Vec::new();
                if let Some(&(last_id, _)) = words.last() {
                    if mark.is_some() {
                        // A new session's words do not continue the earlier one's; the start of the first ones still counts as a
                        // place cut, as after a take-up.
                        t.carry.clear();
                        t.carry_open = true;
                        t.carry_edges.clear();
                    }
                    // One stream: what was written of it (the carry), the words written now, then the words held after them.
                    let mut run = Run::default();
                    run.text.push_str(&t.carry);
                    if !run.text.is_empty() {
                        run.parts.push(RunPart {
                            line: None,
                            start: 0,
                            end: run.text.len(),
                        });
                    } else if t.carry_open {
                        t.carry_edges = vec![0];
                    }
                    run.edges.extend(t.carry_edges.iter().copied());
                    // Where the node cut a piece of words (a text at 16 KiB, a thought in its middle) is a place cut in the stream
                    // too: the next piece may continue the value whose end was dropped (review of #1103, round 5). Kept with the
                    // carry, so a later batch still knows it.
                    let mut cut_edges = Vec::new();
                    for &(_, line) in &words {
                        let at = run.text.len();
                        if let Some(Line::Entry(e, side)) = lines.get(line) {
                            run.text.push_str(&e.text);
                            cut_edges.extend(side.cuts.text.iter().map(|c| at + c));
                        }
                        run.parts.push(RunPart {
                            line: Some(line),
                            start: at,
                            end: run.text.len(),
                        });
                    }
                    let written = run.text.len();
                    let mut ahead = 0usize;
                    for e in t
                        .entries
                        .iter()
                        .filter(|e| in_run(e) && e.entry_id > last_id && mine(e.entry_id))
                    {
                        if ahead >= reach {
                            break;
                        }
                        let at = run.text.len();
                        run.text.push_str(&e.text);
                        ahead += e.text.len();
                        if let Some(c) = t.cuts.get(&e.entry_id) {
                            run.edges.extend(c.text.iter().map(|c| at + c));
                        }
                        run.parts.push(RunPart {
                            line: None,
                            start: at,
                            end: run.text.len(),
                        });
                    }
                    // Written without a value's length of words after it: what comes next may continue a value, so this end counts
                    // as a place cut, whatever ended the hold (a turn's end, a stop's flush, words said outside a turn, words that
                    // left the ring, the earlier session at a fresh start). Even a turn's end: an adapter may say it is idle before
                    // its last words.
                    if ahead < reach {
                        run.edges.push(written);
                    }
                    run.edges.extend(cut_edges.iter().copied());
                    t.carry = tail_of(&run.text[..written], reach);
                    // The earlier node's edge stays while the carry reaches back to it (review of #1103, round 3), and so do the
                    // places the node cut the words written now.
                    let from = written - t.carry.len();
                    t.carry_edges = t
                        .carry_edges
                        .iter()
                        .chain(&cut_edges)
                        .filter_map(|e| e.checked_sub(from))
                        .collect();
                    t.carry_open = false;
                    runs.push(run);
                }
                out.push(Batch {
                    agent: agent.clone(),
                    high,
                    lines,
                    fresh: mark,
                    runs,
                });
            }
        }
        out
    }

    /// ADR-0089 decision 1: the agent's history could not be written. Said once in its conversation (`chat.history_not_saved`, a
    /// node notice); the agent is not touched and its entries stay in the ring.
    pub fn not_saved(&self, agent: &AgentId, now_ms: u64) {
        let mut g = self.lock();
        let Some(t) = g.get_mut(agent) else {
            return;
        };
        match &mut t.save {
            Some(s) if !s.said => s.said = true,
            _ => return,
        }
        let mut n = entry(ChatRole::Agent, ChatKind::Notice, String::new(), now_ms);
        n.code = crate::transcript::NOT_SAVED.into();
        t.add(n, now_ms);
    }

    fn lock(&self) -> Locked<'_> {
        Locked {
            g: self.agents.lock().unwrap_or_else(|e| e.into_inner()),
            store: self.store.as_deref(),
        }
    }

    /// Adds one entry to an agent's transcript and hands it to everyone following it.
    pub fn push(
        &self,
        agent: &AgentId,
        role: ChatRole,
        kind: ChatKind,
        text: String,
        path: String,
        at_ms: u64,
    ) {
        let (text, end) = cut_at(text);
        let mut e = entry(role, kind, text, at_ms);
        e.path = path;
        self.insert_cut(agent, e, end.into_iter().collect());
    }

    /// The user's own prompt as an entry. Returns its id, which [`Self::retract`] names if the prompt did not go out.
    pub fn push_user(&self, agent: &AgentId, text: String, at_ms: u64) -> u64 {
        let (text, end) = cut_at(text);
        self.insert_cut(
            agent,
            entry(ChatRole::User, ChatKind::Text, text, at_ms),
            end.into_iter().collect(),
        )
    }

    /// Takes an entry back out of the transcript (a prompt that could not be sent): a new subscriber never sees it. A follower that
    /// already got it keeps it; the user is told by the error of the command.
    pub fn retract(&self, agent: &AgentId, entry_id: u64) {
        if let Some(t) = self.lock().get_mut(agent) {
            if let Some(i) = t.index(entry_id) {
                t.take_out(i);
            }
        }
    }

    /// ADR-0090: a prompt of the user's that waits behind the agent's turn (`queued`). Returns the id of its entry, which
    /// [`Self::unqueue`] or [`Self::drop_queued`] names later.
    pub fn push_queued(&self, agent: &AgentId, text: String, at_ms: u64) -> u64 {
        let (text, end) = cut_at(text);
        let mut e = entry(ChatRole::User, ChatKind::Text, text, at_ms);
        e.queued = true;
        self.insert_cut(agent, e, end.into_iter().collect())
    }

    /// Numbers an entry, keeps it within the ring's bounds and hands it to every follower, noting where its text was cut (`cuts`).
    /// Returns its id.
    fn insert_cut(&self, agent: &AgentId, e: ChatEntry, cuts: Vec<usize>) -> u64 {
        let at = e.at_ms;
        let mut g = self.lock();
        let t = slot(&mut g, agent, self.wake.as_ref());
        let id = t.add(e, at);
        if !cuts.is_empty() {
            t.note_cuts(id, |c| c.text = cuts);
        }
        id
    }

    /// What an adapter said. A notice whose code has the wrong shape is dropped (ADR-0058).
    pub fn push_item(&self, agent: &AgentId, item: &ChatItem, at_ms: u64) {
        self.push_item_cut(agent, item, &[], at_ms);
    }

    /// [`Self::push_item`], with the places the adapter reported it cut the item's text (ADR-0111: from an adapter that declared
    /// `chat.cuts`, each checked by the host against the item as sent). Each one is recorded where the node records its own cuts, as
    /// a place a value may have been cut, for the live stream and the file alike.
    pub fn push_item_cut(
        &self,
        agent: &AgentId,
        item: &ChatItem,
        cuts: &[ReportedCut],
        at_ms: u64,
    ) {
        match item {
            ChatItem::Notice { code, params } => {
                // The node's own notices are not the adapter's to make.
                if NODE_NOTICES.contains(&code.as_str()) {
                    return;
                }
                let Some((code, params, mut cuts_at)) = notice_cuts(code, params) else {
                    return;
                };
                // A parameter is only cut at its end by the node, so a place the adapter reported is where it was, if still kept.
                for c in cuts {
                    let CutField::Param(k) = &c.field else {
                        continue;
                    };
                    if !params.get(k).is_some_and(|v| c.at <= v.len()) {
                        continue;
                    }
                    match cuts_at.iter_mut().find(|(key, _)| key == k) {
                        Some((_, at)) => at.push(c.at),
                        None => cuts_at.push((k.clone(), vec![c.at])),
                    }
                }
                let cuts = cuts_at;
                let mut e = entry(ChatRole::Agent, ChatKind::Notice, String::new(), at_ms);
                e.code = code;
                e.params = params;
                let mut g = self.lock();
                let t = slot(&mut g, agent, self.wake.as_ref());
                let id = t.add(e, at_ms);
                if !cuts.is_empty() {
                    t.note_cuts(id, |c| c.params = cuts);
                }
            }
            ChatItem::Thought(text) => {
                // The agent's thinking: an entry of its own, cut at 8 KiB, and nothing else (it is never routed anywhere).
                let reported = reported(cuts, &CutField::Text);
                let (text, _, cuts) =
                    tidy_cuts_at(text, MAX_THOUGHT_BYTES, Style::Prose, &reported);
                self.insert_cut(
                    agent,
                    entry(ChatRole::Agent, ChatKind::Thought, text, at_ms),
                    cuts,
                );
            }
            ChatItem::Tool(tool) => self.upsert_tool(agent, tool, cuts, at_ms),
            ChatItem::PlanSteps(steps) => self.upsert_plan(agent, steps, cuts, at_ms),
            other => {
                let (kind, mut text, path) = describe(other);
                // Audit F1660: a diff's path is untrusted like a tool call's: cleaned and cut before the entry exists (each place the
                // adapter reported lands where cleaning puts it), and a secret file is known by the cleaned path (ADR-0080).
                let (path, path_at) = match other {
                    ChatItem::Diff { .. } => {
                        let (path, _, at) = tidy_cuts_at(
                            &path,
                            MAX_PATH_BYTES,
                            Style::Label,
                            &reported(cuts, &CutField::DiffPath),
                        );
                        if is_secret_file(&path) {
                            text.clear();
                        }
                        (path, at)
                    }
                    _ => (path, Vec::new()),
                };
                // The places in the entry's text (as `describe` made it) and its path; the node only cuts the text's end.
                let at: Vec<usize> = match other {
                    ChatItem::Text(_) => reported(cuts, &CutField::Text),
                    ChatItem::Diff { .. } if !text.is_empty() => reported(cuts, &CutField::Text),
                    ChatItem::ToolCall { .. } | ChatItem::Plan(_) => {
                        composed_text_cuts(other, cuts)
                    }
                    _ => Vec::new(),
                };
                let (text, end) = cut_at(text);
                let text_at: Vec<usize> = at
                    .into_iter()
                    .filter(|&a| a <= text.len())
                    .chain(end)
                    .collect();
                let mut e = entry(ChatRole::Agent, kind, text, at_ms);
                e.path = path;
                let mut g = self.lock();
                let t = slot(&mut g, agent, self.wake.as_ref());
                let id = t.add(e, at_ms);
                if !text_at.is_empty() || !path_at.is_empty() {
                    t.note_cuts(id, |c| {
                        c.text = text_at;
                        c.path = path_at;
                    });
                }
            }
        }
    }

    /// ADR-0090: a tool call or an update to one. The first sight of a call id makes the entry; a later one changes it in place: a
    /// field the item carries replaces the field, one it leaves out keeps it. The duration is the node's own: `started_ms` is when
    /// the call was first seen and `ended_ms` when it first reached `completed` or `failed`. Paths are cut to 64; output keeps
    /// its head and tail within 16 KiB; and what a tool printed about a secret file (ADR-0080) is not kept at all.
    fn upsert_tool(&self, agent: &AgentId, item: &ToolItem, cuts: &[ReportedCut], now_ms: u64) {
        // An id that does not fit would, cut, name another call: such an item is dropped.
        if item.call_id.is_empty() || item.call_id.len() > MAX_CALL_ID {
            return;
        }
        // Everything that costs work in proportion to the text (cleaning, cutting, formatting, cloning) is done here, before the lock.
        let (tool_in, tool_cuts) = match item.tool.as_ref() {
            Some(x) => {
                let (t, _, c) = tidy_cuts_at(
                    x,
                    MAX_PATH_BYTES,
                    Style::Label,
                    &reported(cuts, &CutField::Tool),
                );
                (Some(t), Some(c))
            }
            None => (None, None),
        };
        let (args_in, args_cuts) = match item.arguments_json.as_ref() {
            Some(x) => {
                let (a, _, c) = tidy_cuts_at(
                    x,
                    MAX_ENTRY_BYTES,
                    Style::Code,
                    &reported(cuts, &CutField::Arguments),
                );
                (Some(a), Some(c))
            }
            None => (None, None),
        };
        let kind = tool_kind(&item.kind);
        let status = tool_status(&item.status);
        let mut path_cuts = Vec::new();
        let paths: Option<Vec<ToolLocation>> = item.locations.as_ref().map(|l| {
            l.iter()
                .take(MAX_PATHS)
                .enumerate()
                .map(|(i, (path, line))| {
                    let (path, _, c) = tidy_cuts_at(
                        path,
                        MAX_PATH_BYTES,
                        Style::Label,
                        &reported(cuts, &CutField::Path(i)),
                    );
                    path_cuts.push(c);
                    ToolLocation { path, line: *line }
                })
                .collect()
        });
        let path_cuts = paths.as_ref().map(|_| path_cuts);
        let secret_path = paths
            .as_ref()
            .is_some_and(|p| p.iter().any(|p| is_secret_file(&p.path)));
        let (output, output_cuts) = match item.output.as_ref() {
            Some(o) => {
                let (o, truncated, c) = tidy_cuts_at(
                    o,
                    MAX_OUTPUT_BYTES,
                    Style::Prose,
                    &reported(cuts, &CutField::Output),
                );
                (Some((o, truncated)), Some(c))
            }
            None => (None, None),
        };
        // The text of the entry when this item is the whole story (a new call, or one that names both), and when it is only part.
        let text_new = tool_text(
            tool_in.as_deref().unwrap_or("tool"),
            args_in.as_deref().unwrap_or(""),
        );
        let text_whole = tool_in
            .as_ref()
            .zip(args_in.as_ref())
            .map(|(tool, args)| tool_text(tool, args));
        let call_id = item.call_id.clone();
        let terminal = status == ToolStatus::Completed || status == ToolStatus::Failed;

        let mut g = self.lock();
        let t = slot(&mut g, agent, self.wake.as_ref());
        let known = t
            .calls
            .get(&item.call_id)
            .filter(|c| t.index(c.entry).is_some())
            .map(|c| (c.entry, c.tool.clone(), c.args.clone()));
        let apply = |e: &mut ChatEntry, creating: bool, text: String| {
            e.text = text;
            if kind != ToolKind::Unspecified || creating {
                e.tool_kind = kind as i32;
            }
            if status != ToolStatus::Unspecified {
                e.status = status as i32;
            } else if creating {
                e.status = ToolStatus::Pending as i32;
            }
            if let Some(p) = paths {
                e.paths = p;
            }
            if let Some((o, truncated)) = output {
                e.output = o;
                e.output_truncated = truncated;
            }
            // What a tool printed about a secret file is not shown, whatever order the pieces arrived in.
            if secret_path || e.paths.iter().any(|p| is_secret_file(&p.path)) {
                e.output.clear();
                e.output_truncated = false;
            }
            if finished(e.status) {
                if e.ended_ms == 0 {
                    e.ended_ms = now_ms.max(e.started_ms);
                }
            } else {
                e.ended_ms = 0;
            }
        };
        match known {
            Some((id, old_tool, old_args)) => {
                let tool = tool_in.unwrap_or(old_tool);
                let args = args_in.unwrap_or(old_args);
                let text = text_whole.unwrap_or_else(|| tool_text(&tool, &args));
                // A call that reaches `completed` or `failed` is sent at once, once. The same final status again (a burst of repeats)
                // is held to one per 150 ms like any other change.
                let was_final = t.index(id).is_some_and(|i| finished(t.entries[i].status));
                let ok = t.modify(id, now_ms, !terminal || was_final, |e| {
                    // The time it began is kept from the first sight; `now_ms` only ends it.
                    apply(e, false, text);
                });
                if ok {
                    if let Some(c) = t.calls.get_mut(&call_id) {
                        c.tool = tool;
                        c.args = args;
                    }
                    t.note_cuts(id, |c| {
                        if let Some(a) = args_cuts {
                            c.args = a;
                        }
                        if let Some(o) = output_cuts {
                            c.output = o;
                        }
                        if let Some(x) = tool_cuts {
                            c.tool = x;
                        }
                        if let Some(p) = path_cuts {
                            c.paths = p;
                        }
                    });
                }
            }
            None => {
                let mut e = entry(ChatRole::Agent, ChatKind::ToolCall, String::new(), now_ms);
                e.call_id = call_id.clone();
                e.started_ms = now_ms;
                apply(&mut e, true, text_new);
                let id = t.add(e, now_ms);
                t.note_cuts(id, |c| {
                    c.args = args_cuts.unwrap_or_default();
                    c.output = output_cuts.unwrap_or_default();
                    c.tool = tool_cuts.unwrap_or_default();
                    c.paths = path_cuts.unwrap_or_default();
                });
                t.calls.insert(
                    call_id,
                    Call {
                        entry: id,
                        tool: tool_in.unwrap_or_else(|| "tool".into()),
                        args: args_in.unwrap_or_default(),
                    },
                );
            }
        }
    }

    /// ADR-0090: the plan with each step's status. A plan replaces the earlier one in the same entry while that is still in the ring;
    /// at most 64 steps of 512 bytes; `text` stays the step lines joined for old clients.
    fn upsert_plan(&self, agent: &AgentId, steps: &[PlanItem], cuts: &[ReportedCut], now_ms: u64) {
        let mut step_cuts = Vec::new();
        let steps: Vec<PlanStep> = steps
            .iter()
            .take(MAX_PLAN_STEPS)
            .enumerate()
            .map(|(i, s)| {
                let (text, _, cuts) = tidy_cuts_at(
                    &s.text,
                    MAX_PLAN_STEP_BYTES,
                    Style::Step,
                    &reported(cuts, &CutField::Step(i)),
                );
                step_cuts.push(cuts);
                PlanStep {
                    text,
                    status: plan_status(&s.status) as i32,
                    priority: plan_priority(&s.priority) as i32,
                }
            })
            .collect();
        let text = plan_text(&steps);
        let mut g = self.lock();
        let t = slot(&mut g, agent, self.wake.as_ref());
        if let Some(id) = t.plan.filter(|id| t.index(*id).is_some()) {
            let (text, steps) = (text.clone(), steps.clone());
            if t.modify(id, now_ms, true, move |e| {
                e.text = text;
                e.plan_steps = steps;
            }) {
                t.note_cuts(id, |c| c.steps = step_cuts);
                return;
            }
        }
        let mut e = entry(ChatRole::Agent, ChatKind::Plan, text, now_ms);
        e.plan_steps = steps;
        let id = t.add(e, now_ms);
        t.note_cuts(id, |c| c.steps = step_cuts);
        t.plan = Some(id);
    }

    /// ADR-0090: sends the changes of entries that were held back to keep to one version per [`COALESCE_MS`], once that time is
    /// over. Called by the node's tick; an entry whose last change is final was never held back.
    pub fn flush_due(&self, now_ms: u64) {
        // Audit of #1157: the values are read again at most once a second here too (the node's tick, before the board's lock), so the
        // live path follows a change even without the writer thread.
        if let Some(store) = &self.store {
            store.refresh_if_older(Duration::from_secs(1));
        }
        let mut g = self.lock();
        for t in g.values_mut() {
            // #1147: words that waited [`LIVE_HOLD_MS`] go now, the part of a value they end in replaced.
            if t.live
                .unsent
                .values()
                .min()
                .is_some_and(|since| now_ms < *since || now_ms - since >= LIVE_HOLD_MS)
            {
                t.live.force = true;
            }
            if t.dirty.is_empty() {
                continue;
            }
            let due: Vec<u64> = t
                .dirty
                .iter()
                .copied()
                .filter(|id| {
                    t.sent_ms
                        .get(id)
                        .is_none_or(|last| now_ms < *last || now_ms - *last >= COALESCE_MS)
                })
                .collect();
            for id in due {
                t.dirty.remove(&id);
                if t.index(id).is_some() {
                    t.sent_ms.insert(id, now_ms);
                    t.out(id, Out::Version);
                }
            }
        }
    }

    /// ADR-0090, #908: the queued prompt of this entry became the turn and is being sent at `sent_ms` (wall clock). An entry's
    /// `at_ms` is when its event happened, and a queued prompt's event is its send, so the entry takes that time: a client places
    /// the prompt by it (ties and out-of-order times broken by entry id), before the answer it gets, so the replies of two turns
    /// do not read as one. Nothing is sent to followers here; the new time goes out with [`Self::unqueue`] (or the entry leaves
    /// with [`Self::drop_queued`]).
    pub fn sending(&self, agent: &AgentId, entry_id: u64, sent_ms: u64) {
        if let Some(t) = self.lock().get_mut(agent) {
            if let Some(i) = t.index(entry_id) {
                if t.entries[i].queued {
                    t.entries[i].at_ms = sent_ms;
                    t.changed(entry_id, 0);
                }
            }
        }
    }

    /// ADR-0090: the prompt of this entry was sent (it no longer waits).
    pub fn unqueue(&self, agent: &AgentId, entry_id: u64, now_ms: u64) {
        if let Some(t) = self.lock().get_mut(agent) {
            t.modify(entry_id, now_ms, false, |e| e.queued = false);
        }
    }

    /// ADR-0090: queued prompts that will not be sent. Their entries leave the transcript (a new subscriber never sees them) and a
    /// notice with the stable code `adapter.queue_dropped`, the count and (#908) `entries`, the ids of exactly those entries joined
    /// by commas, says so to everyone following; a client that holds one of them clears its waiting mark, and only for those (one
    /// failed send among several queued prompts drops that one only).
    pub fn drop_queued(&self, agent: &AgentId, entry_ids: &[u64], now_ms: u64) {
        if entry_ids.is_empty() {
            return;
        }
        let mut g = self.lock();
        slot(&mut g, agent, self.wake.as_ref()).drop_queued(entry_ids, now_ms);
    }

    /// ADR-0110 decision 6: the prompt of entry `entry_id` went to an adapter of `kind` that did not answer within `waited_ms`, so it
    /// may hold it. The entry stays (a queued one is shown sent), and a notice with the stable code `adapter.no_answer`, `kind`, `call`
    /// `prompt`, `waited_ms` and `entries` (the entry's id, as for `adapter.queue_dropped`) marks it as not confirmed.
    pub fn unconfirmed(
        &self,
        agent: &AgentId,
        entry_id: u64,
        kind: &str,
        waited_ms: u64,
        now_ms: u64,
    ) {
        let mut g = self.lock();
        let t = slot(&mut g, agent, self.wake.as_ref());
        t.modify(entry_id, now_ms, false, |e| e.queued = false);
        let mut n = entry(ChatRole::Agent, ChatKind::Notice, String::new(), now_ms);
        n.code = "adapter.no_answer".into();
        n.params = HashMap::from([
            ("kind".to_string(), kind.to_string()),
            ("call".to_string(), "prompt".to_string()),
            ("waited_ms".to_string(), waited_ms.to_string()),
            ("entries".to_string(), entry_id.to_string()),
        ]);
        t.add(n, now_ms);
    }

    /// ADR-0093: the holder kept only the newest of what the agent wrote while no node read it. A notice with the stable code
    /// `adapter.output_dropped` and the count of bytes says so to everyone following.
    /// ADR-0101: an adapter's request was refused with no card: `approval.always_only_forbidden` (it offered only "always" under a
    /// policy that refuses it) or `adapter.permission_unanswerable` (nothing the person could allow). `action` is the request's, as
    /// the agent named it, bounded like a permission entry's.
    pub fn refused_request(&self, agent: &AgentId, code: &'static str, action: &str, now_ms: u64) {
        let mut g = self.lock();
        let t = slot(&mut g, agent, self.wake.as_ref());
        let mut n = entry(ChatRole::Agent, ChatKind::Notice, String::new(), now_ms);
        n.code = code.into();
        let (label, _, cuts) = tidy_cuts(action, MAX_PERMISSION_BYTES, Style::Label);
        n.params = HashMap::from([("action".to_string(), label)]);
        let id = t.add(n, now_ms);
        t.note_cuts(id, |c| c.params = vec![("action".to_string(), cuts)]);
    }

    pub fn output_dropped(&self, agent: &AgentId, bytes: u64, now_ms: u64) {
        let mut g = self.lock();
        let t = slot(&mut g, agent, self.wake.as_ref());
        let mut n = entry(ChatRole::Agent, ChatKind::Notice, String::new(), now_ms);
        n.code = "adapter.output_dropped".into();
        n.params = HashMap::from([("bytes".to_string(), bytes.to_string())]);
        t.add(n, now_ms);
    }

    /// #902 (RM-13): what the node did about a chat agent whose adapter ended on its own, said in its conversation: it starts it again
    /// (`adapter.restarting`, with the attempt), or it gave up on it (`adapter.gave_up`, with the exit code it reported). The node's own
    /// words: a fixed code and numbers, never text from the agent.
    pub fn restart_notice(
        &self,
        agent: &AgentId,
        code: &'static str,
        params: &[(&str, String)],
        now_ms: u64,
    ) {
        let mut g = self.lock();
        let t = slot(&mut g, agent, self.wake.as_ref());
        let mut n = entry(ChatRole::Agent, ChatKind::Notice, String::new(), now_ms);
        n.code = code.into();
        n.params = params
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
        t.add(n, now_ms);
    }

    /// ADR-0090: a permission entry for an approval the node itself queued. `approval_id` is the id the node minted for it (never the
    /// adapter's own id and never text from the agent); `action` is the label the approvals list shows, cut. The entry holds no
    /// decision: it starts `pending` and only [`Self::set_permission`] changes it, from the node's approval state.
    pub fn push_permission(&self, agent: &AgentId, approval_id: &str, action: &str, now_ms: u64) {
        let (label, _, cuts) = tidy_cuts(action, MAX_PERMISSION_BYTES, Style::Label);
        let mut e = entry(ChatRole::Agent, ChatKind::Permission, label, now_ms);
        e.approval_id = approval_id.to_string();
        e.permission_outcome = PermissionOutcome::Pending as i32;
        self.insert_cut(agent, e, cuts);
    }

    /// ADR-0090: how the approval of a permission entry ended, from the node's own approval state: the user's verified decision, or
    /// `Cancelled` when the turn was cancelled, the agent went away or the request ran out of time. Only a pending entry changes: a
    /// decision, once shown, is not rewritten by what comes after it.
    pub fn set_permission(
        &self,
        agent: &AgentId,
        approval_id: &str,
        outcome: PermissionOutcome,
        now_ms: u64,
    ) {
        self.set_permission_as(agent, approval_id, outcome, false, now_ms);
    }

    /// ADR-0101: [`Self::set_permission`], marking an approved "always" decision (`ChatEntry.permission_always`).
    pub fn set_permission_as(
        &self,
        agent: &AgentId,
        approval_id: &str,
        outcome: PermissionOutcome,
        always: bool,
        now_ms: u64,
    ) {
        if approval_id.is_empty() {
            return;
        }
        let mut g = self.lock();
        let Some(t) = g.get_mut(agent) else {
            return;
        };
        let found = t.entries.iter().find(|e| {
            e.kind == ChatKind::Permission as i32
                && e.approval_id == approval_id
                && e.permission_outcome == PermissionOutcome::Pending as i32
        });
        if let Some(id) = found.map(|e| e.entry_id) {
            t.modify(id, now_ms, false, |e| {
                e.permission_outcome = outcome as i32;
                e.permission_always = always && outcome == PermissionOutcome::Approved;
            });
        }
    }

    /// The transcript so far, as a follower is shown it (#1147: redacted, without the words still held).
    pub fn snapshot(&self, agent: &AgentId) -> Vec<ChatEntry> {
        let sec = self.secrets();
        self.lock()
            .get(agent)
            .map(|t| t.shown_entries(sec.as_deref()))
            .unwrap_or_default()
    }

    /// The transcript so far and a channel with every entry from now on, taken together so none is missed or repeated. A version of
    /// an entry that changed in place arrives as a new `ChatEntry` with the same `entry_id`.
    pub fn follow(&self, agent: &AgentId) -> (Vec<ChatEntry>, ChatSubscription) {
        let follower_id = self.next_follower_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = sync_channel(FOLLOWER_QUEUE);
        let sec = self.secrets();
        let mut g = self.lock();
        let t = slot(&mut g, agent, self.wake.as_ref());
        let snapshot = t.shown_entries(sec.as_deref());
        t.followers.push((follower_id, tx));
        (
            snapshot,
            ChatSubscription {
                rx,
                agents: Arc::downgrade(&self.agents),
                agent: agent.clone(),
                follower_id,
            },
        )
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use agora_adapters::text::{strip_controls, ESCAPE_PAYLOAD_MAX};

    #[test]
    fn closing_subscriptions_removes_their_followers_without_another_entry() {
        let b = ChatBoard::new();
        let a = AgentId::new("agent-1");
        b.push_item(&a, &ChatItem::Text("first".into()), 1);
        let (_, staying) = b.follow(&a);

        for _ in 0..3 {
            let (snapshot, closing) = b.follow(&a);
            assert_eq!(snapshot[0].text, "first");
            assert_eq!(b.lock()[&a].followers.len(), 2);
            drop(closing);
            assert_eq!(b.lock()[&a].followers.len(), 1);
        }

        b.push_item(&a, &ChatItem::Text("second".into()), 2);
        assert_eq!(staying.try_recv().unwrap().text, "second");
        drop(staying);
        assert!(b.lock()[&a].followers.is_empty());
        assert_eq!(b.snapshot(&a).len(), 2);
    }

    #[test]
    fn entries_get_increasing_ids_and_followers_see_exactly_what_comes_after() {
        let b = ChatBoard::new();
        let a = AgentId::new("agent-1");
        b.push_item(&a, &ChatItem::Text("one".into()), 1);
        let (so_far, rx) = b.follow(&a);
        assert_eq!(
            so_far.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(),
            ["one"]
        );
        b.push(
            &a,
            ChatRole::User,
            ChatKind::Text,
            "two".into(),
            String::new(),
            2,
        );
        let e = rx.try_recv().unwrap();
        assert_eq!(
            (e.entry_id, e.text.as_str(), e.role),
            (2, "two", ChatRole::User as i32)
        );
        assert!(rx.try_recv().is_err());
        assert!(b.snapshot(&AgentId::new("other")).is_empty());
    }

    #[test]
    fn the_ring_is_bounded_by_count_and_bytes_and_an_entry_by_size() {
        let b = ChatBoard::new();
        let a = AgentId::new("a");
        for i in 0..(MAX_ENTRIES + 20) {
            b.push(
                &a,
                ChatRole::Agent,
                ChatKind::Text,
                format!("m{i}"),
                String::new(),
                0,
            );
        }
        let s = b.snapshot(&a);
        assert_eq!(s.len(), MAX_ENTRIES);
        assert_eq!(s[0].text, "m20", "oldest dropped first");
        let big = "é".repeat(MAX_ENTRY_BYTES); // two bytes each: the cut must land on a character boundary
        b.push(&a, ChatRole::Agent, ChatKind::Text, big, String::new(), 0);
        let last = b.snapshot(&a).pop().unwrap();
        assert!(last.text.len() <= MAX_ENTRY_BYTES && last.text.chars().all(|c| c == 'é'));
        let c = AgentId::new("c");
        for _ in 0..40 {
            b.push(
                &c,
                ChatRole::Agent,
                ChatKind::Text,
                "x".repeat(MAX_ENTRY_BYTES),
                String::new(),
                0,
            );
        }
        assert!(b.snapshot(&c).iter().map(|e| e.text.len()).sum::<usize>() <= MAX_BYTES);
    }

    #[test]
    fn items_become_entries_with_their_kind() {
        let (k, t, p) = describe(&ChatItem::Diff {
            path: "a.rs".into(),
            unified_diff: "@@".into(),
        });
        assert_eq!((k, t.as_str(), p.as_str()), (ChatKind::Diff, "@@", "a.rs"));
        let (k, t, _) = describe(&ChatItem::ToolCall {
            tool: "run \"x\"".into(),
            arguments_json: "{\"a\":1}".into(),
        });
        assert_eq!(k, ChatKind::ToolCall);
        assert_eq!(t, "{\"tool\":\"run \\\"x\\\"\",\"arguments\":{\"a\":1}}");
        let (k, t, _) = describe(&ChatItem::Plan(vec!["a".into(), "b".into()]));
        assert_eq!((k, t.as_str()), (ChatKind::Plan, "a\nb"));
    }

    /// #902: the node's own notices about an adapter that ended on its own are notice entries with their code and numbers only.
    #[test]
    fn a_restart_notice_is_a_notice_with_its_code_and_numbers() {
        let b = ChatBoard::new();
        let a = AgentId::new("a");
        b.restart_notice(&a, "adapter.restarting", &[("attempt", "2".into())], 1);
        b.restart_notice(&a, "adapter.gave_up", &[("exit_code", "3".into())], 2);
        let got: Vec<_> = b
            .snapshot(&a)
            .into_iter()
            .map(|e| {
                (
                    e.kind,
                    e.code,
                    e.params.into_iter().collect::<Vec<_>>(),
                    e.text,
                )
            })
            .collect();
        assert_eq!(
            got,
            [
                (
                    ChatKind::Notice as i32,
                    "adapter.restarting".to_string(),
                    vec![("attempt".to_string(), "2".to_string())],
                    String::new()
                ),
                (
                    ChatKind::Notice as i32,
                    "adapter.gave_up".to_string(),
                    vec![("exit_code".to_string(), "3".to_string())],
                    String::new()
                ),
            ]
        );
    }

    #[test]
    fn a_notice_keeps_its_code_and_bounded_params_and_a_bad_code_is_dropped() {
        let b = ChatBoard::new();
        let a = AgentId::new("a");
        let mut params = vec![
            ("stderr_tail".to_string(), "é".repeat(MAX_PARAM_BYTES)),
            ("Bad Key".to_string(), "x".to_string()),
        ];
        params.extend((0..20).map(|i| (format!("k{i}"), "v".to_string())));
        b.push_item(
            &a,
            &ChatItem::Notice {
                code: "adapter.agent_exited".into(),
                params,
            },
            1,
        );
        for bad in [
            "",
            "adapter",
            "Adapter.x",
            "a..b",
            "a.b c",
            &"a.b".repeat(30),
            "notice-x.y",
        ] {
            b.push_item(
                &a,
                &ChatItem::Notice {
                    code: bad.into(),
                    params: vec![],
                },
                2,
            );
        }
        let s = b.snapshot(&a);
        assert_eq!(s.len(), 1, "{s:?}");
        let n = &s[0];
        assert_eq!(
            (n.kind, n.code.as_str(), n.text.as_str()),
            (ChatKind::Notice as i32, "adapter.agent_exited", "")
        );
        assert_eq!(n.params.len(), MAX_PARAMS);
        assert!(!n.params.contains_key("Bad Key"));
        assert!(n.params["stderr_tail"].len() <= MAX_PARAM_BYTES);
    }

    // ---- ADR-0090 ----

    fn tool(call: &str) -> ToolItem {
        ToolItem {
            call_id: call.into(),
            ..ToolItem::default()
        }
    }

    /// The bytes the transcript counts equal what its entries add up to: updates in place never drift the ring's accounting.
    fn accounted(b: &ChatBoard, a: &AgentId) {
        let g = b.lock();
        let t = g.get(a).unwrap();
        assert_eq!(
            t.bytes,
            t.entries.iter().map(Transcript::size).sum::<usize>()
        );
    }

    #[test]
    fn a_tool_call_is_one_entry_that_changes_in_place_with_its_duration_measured_by_the_node() {
        let b = ChatBoard::new();
        let a = AgentId::new("a");
        let (_, rx) = b.follow(&a);
        b.push_item(
            &a,
            &ChatItem::Tool(ToolItem {
                tool: Some("read a.rs".into()),
                arguments_json: Some("{\"p\":1}".into()),
                kind: "read".into(),
                status: "pending".into(),
                locations: Some(vec![("a.rs".into(), 3)]),
                ..tool("t1")
            }),
            1000,
        );
        // Later versions carry less: a field the update leaves out is kept.
        b.push_item(
            &a,
            &ChatItem::Tool(ToolItem {
                status: "in_progress".into(),
                ..tool("t1")
            }),
            1500,
        );
        b.push_item(
            &a,
            &ChatItem::Tool(ToolItem {
                status: "completed".into(),
                output: Some("file text".into()),
                ..tool("t1")
            }),
            2600,
        );
        let s = b.snapshot(&a);
        assert_eq!(s.len(), 1, "{s:?}");
        let e = &s[0];
        assert_eq!(e.kind, ChatKind::ToolCall as i32);
        assert_eq!(e.call_id, "t1");
        assert_eq!(e.tool_kind, ToolKind::Read as i32);
        assert_eq!(e.status, ToolStatus::Completed as i32);
        assert_eq!(e.text, "{\"tool\":\"read a.rs\",\"arguments\":{\"p\":1}}");
        assert_eq!((e.paths[0].path.as_str(), e.paths[0].line), ("a.rs", 3));
        assert_eq!(e.output, "file text");
        // The node's clock: first seen at 1000, finished at 2600.
        assert_eq!((e.at_ms, e.started_ms, e.ended_ms), (1000, 1000, 2600));
        // Followers saw every version under the same entry id (the middle one was inside 150 ms of the first: held, see below).
        let seen: Vec<ChatEntry> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(seen.iter().all(|x| x.entry_id == e.entry_id));
        assert_eq!(seen.first().unwrap().status, ToolStatus::Pending as i32);
        assert_eq!(seen.last().unwrap().status, ToolStatus::Completed as i32);
        accounted(&b, &a);
    }

    #[test]
    fn an_update_for_a_call_that_left_the_ring_or_was_never_seen_makes_a_new_entry() {
        let b = ChatBoard::new();
        let a = AgentId::new("a");
        b.push_item(
            &a,
            &ChatItem::Tool(ToolItem {
                tool: Some("x".into()),
                ..tool("t1")
            }),
            0,
        );
        for i in 0..MAX_ENTRIES {
            b.push(
                &a,
                ChatRole::Agent,
                ChatKind::Text,
                format!("{i}"),
                String::new(),
                0,
            );
        }
        assert!(
            b.snapshot(&a).iter().all(|e| e.call_id != "t1"),
            "t1 was evicted"
        );
        b.push_item(
            &a,
            &ChatItem::Tool(ToolItem {
                status: "completed".into(),
                ..tool("t1")
            }),
            5,
        );
        let s = b.snapshot(&a);
        assert_eq!(s.iter().filter(|e| e.call_id == "t1").count(), 1);
        assert_eq!(s.len(), MAX_ENTRIES);
        // The ring's index forgot the evicted call; it did not grow with it.
        assert!(b.lock().get(&a).unwrap().calls.len() <= 1);
        accounted(&b, &a);
    }

    #[test]
    fn updates_in_place_do_not_grow_the_ring_and_keep_its_byte_accounting() {
        let b = ChatBoard::new();
        let a = AgentId::new("a");
        for i in 0..2000u64 {
            b.push_item(
                &a,
                &ChatItem::Tool(ToolItem {
                    output: Some("y".repeat(((i % 7) as usize) * 4096)),
                    status: "in_progress".into(),
                    ..tool("t1")
                }),
                i * 1000,
            );
        }
        assert_eq!(b.snapshot(&a).len(), 1);
        accounted(&b, &a);
        // Many calls with big output stay under the byte bound.
        for i in 0..100 {
            b.push_item(
                &a,
                &ChatItem::Tool(ToolItem {
                    output: Some("z".repeat(MAX_OUTPUT_BYTES * 2)),
                    ..tool(&format!("c{i}"))
                }),
                0,
            );
        }
        let total: usize = b.snapshot(&a).iter().map(Transcript::size).sum();
        assert!(total <= MAX_BYTES && b.snapshot(&a).len() <= MAX_ENTRIES);
        accounted(&b, &a);
    }

    #[test]
    fn every_size_cap_of_the_protocol_spec_is_enforced_before_the_entry_exists() {
        let b = ChatBoard::new();
        let a = AgentId::new("a");
        // Output: 16 KiB, head and tail kept, the flag says it was cut, on character boundaries.
        let long = format!("HEAD{}TAIL", "é".repeat(MAX_OUTPUT_BYTES));
        b.push_item(
            &a,
            &ChatItem::Tool(ToolItem {
                output: Some(long),
                ..tool("o")
            }),
            0,
        );
        // Thought: 8 KiB.
        b.push_item(&a, &ChatItem::Thought("t".repeat(MAX_THOUGHT_BYTES * 3)), 0);
        // Paths: 64, each cut.
        let many: Vec<(String, u32)> = (0..200)
            .map(|i| (format!("{}{i}", "p".repeat(2000)), 1))
            .collect();
        b.push_item(
            &a,
            &ChatItem::Tool(ToolItem {
                locations: Some(many),
                ..tool("p")
            }),
            0,
        );
        // A call id too long to keep is dropped, not cut into another call's id.
        b.push_item(&a, &ChatItem::Tool(tool(&"i".repeat(MAX_CALL_ID + 1))), 0);
        // Plan: 64 steps of 512 bytes.
        let steps: Vec<PlanItem> = (0..200)
            .map(|i| PlanItem {
                text: format!("{i}{}", "s".repeat(2000)),
                ..PlanItem::default()
            })
            .collect();
        b.push_item(&a, &ChatItem::PlanSteps(steps), 0);
        let s = b.snapshot(&a);
        assert_eq!(
            s.len(),
            4,
            "the call id of {} bytes was dropped",
            MAX_CALL_ID + 1
        );
        let o = &s[0];
        assert!(o.output.len() <= MAX_OUTPUT_BYTES && o.output_truncated);
        assert!(
            o.output.starts_with("HEAD") && o.output.ends_with("TAIL"),
            "head and tail kept"
        );
        assert!(s[1].text.len() <= MAX_THOUGHT_BYTES && s[1].kind == ChatKind::Thought as i32);
        assert_eq!(s[2].paths.len(), MAX_PATHS);
        assert!(s[2].paths.iter().all(|p| p.path.len() <= MAX_PATH_BYTES));
        assert_eq!(s[3].plan_steps.len(), MAX_PLAN_STEPS);
        assert!(s[3]
            .plan_steps
            .iter()
            .all(|p| p.text.len() <= MAX_PLAN_STEP_BYTES));
        // An output that fits is passed whole and not flagged.
        b.push_item(
            &a,
            &ChatItem::Tool(ToolItem {
                output: Some("ok".into()),
                ..tool("o")
            }),
            0,
        );
        let o = b
            .snapshot(&a)
            .into_iter()
            .find(|e| e.call_id == "o")
            .unwrap();
        assert!(!o.output_truncated && o.output == "ok");
    }

    /// Audit F1660: a diff's path is untrusted like a tool call's: cleaned and cut to `MAX_PATH_BYTES` before the entry exists (one
    /// long path does not empty the ring), and a secret file is known by the cleaned path, so its change keeps no text.
    #[test]
    fn a_diffs_path_is_cleaned_and_cut_before_the_entry_exists() {
        let b = ChatBoard::new();
        let a = AgentId::new("a");
        b.push_item(&a, &ChatItem::Text("kept".into()), 0);
        let diff = |path: String, text: &str| ChatItem::Diff {
            path,
            unified_diff: text.into(),
        };
        b.push_item(&a, &diff("p".repeat(MAX_BYTES + 1), "+x"), 0);
        b.push_item(
            &a,
            &diff("src/\u{1b}]0;t\u{7}b.rs\u{1b}[2J".into(), "+y"),
            0,
        );
        b.push_item(&a, &diff("cfg/.env\u{7}".into(), "+KEY=v"), 0);
        let s = b.snapshot(&a);
        assert_eq!(s.len(), 4, "one long path does not empty the ring");
        assert_eq!(s[0].text, "kept");
        assert!(s[1].path.len() <= MAX_PATH_BYTES && s[1].text == "+x");
        assert_eq!(s[2].path, "src/b.rs");
        assert_eq!((s[3].path.as_str(), s[3].text.as_str()), ("cfg/.env", ""));
    }

    #[test]
    fn at_most_one_version_of_an_entry_is_sent_per_150_ms_and_the_last_is_never_lost() {
        let b = ChatBoard::new();
        let a = AgentId::new("a");
        b.push_item(
            &a,
            &ChatItem::Tool(ToolItem {
                status: "pending".into(),
                ..tool("t")
            }),
            1000,
        );
        let (_, rx) = b.follow(&a);
        // A burst of 50 updates in 100 ms: none is sent now (the first version went out at 1000).
        for i in 1..=50u64 {
            b.push_item(
                &a,
                &ChatItem::Tool(ToolItem {
                    output: Some(format!("line {i}")),
                    status: "in_progress".into(),
                    ..tool("t")
                }),
                1000 + i * 2,
            );
        }
        assert!(rx.try_recv().is_err(), "held back inside the window");
        // The ring holds the newest version already (a new subscriber sees it).
        assert_eq!(b.snapshot(&a)[0].output, "line 50");
        // Not yet due at 1149; due at 1150: one version with the newest content.
        b.flush_due(1149);
        assert!(rx.try_recv().is_err());
        b.flush_due(1150);
        let sent = rx.try_recv().unwrap();
        assert_eq!(sent.output, "line 50");
        assert!(rx.try_recv().is_err(), "one version for the burst");
        b.flush_due(5000);
        assert!(rx.try_recv().is_err(), "nothing is sent twice");
        // A final status is never held back, even inside the window.
        b.push_item(
            &a,
            &ChatItem::Tool(ToolItem {
                output: Some("a".into()),
                status: "in_progress".into(),
                ..tool("t")
            }),
            1151,
        );
        b.push_item(
            &a,
            &ChatItem::Tool(ToolItem {
                status: "completed".into(),
                ..tool("t")
            }),
            1152,
        );
        let last = std::iter::from_fn(|| rx.try_recv().ok()).last().unwrap();
        assert_eq!(last.status, ToolStatus::Completed as i32);
        assert_eq!(
            last.output, "a",
            "the held change rides along with the final version"
        );
        // A clock that moved back does not stall an entry.
        b.push_item(
            &a,
            &ChatItem::Tool(ToolItem {
                output: Some("b".into()),
                status: "in_progress".into(),
                ..tool("t")
            }),
            10,
        );
        b.flush_due(10);
        assert_eq!(rx.try_recv().unwrap().output, "b");
    }

    #[test]
    fn a_plan_is_replaced_in_the_same_entry_with_its_step_status() {
        let b = ChatBoard::new();
        let a = AgentId::new("a");
        let step = |t: &str, s: &str, p: &str| PlanItem {
            text: t.into(),
            status: s.into(),
            priority: p.into(),
        };
        b.push_item(
            &a,
            &ChatItem::PlanSteps(vec![
                step("one", "in_progress", "high"),
                step("two", "pending", "low"),
            ]),
            0,
        );
        b.push_item(&a, &ChatItem::Text("between".into()), 1);
        b.push_item(
            &a,
            &ChatItem::PlanSteps(vec![
                step("one", "completed", "high"),
                step("two", "in_progress", "weird"),
            ]),
            2000,
        );
        let s = b.snapshot(&a);
        assert_eq!(s.len(), 2, "one plan entry and the text");
        let p = &s[0];
        assert_eq!(p.text, "one\ntwo", "old clients read the lines");
        assert_eq!(p.plan_steps[0].status, ToolStatus::Completed as i32);
        assert_eq!(p.plan_steps[1].status, ToolStatus::InProgress as i32);
        assert_eq!(p.plan_steps[0].priority, PlanPriority::High as i32);
        assert_eq!(
            p.plan_steps[1].priority,
            PlanPriority::Unspecified as i32,
            "an unknown word is unspecified"
        );
        // A plan step is never failed.
        assert_eq!(plan_status("failed"), ToolStatus::Unspecified);
        accounted(&b, &a);
    }

    /// Review of #1221: a path whose end is a line break or a tab is cleaned to a trailing space, and is still known as a secret file's:
    /// a diff of it keeps no text, and a tool call naming it keeps no output (in one piece, or with the output after the location).
    #[test]
    fn a_secret_path_with_trailing_whitespace_is_still_secret() {
        for (i, path) in ["cfg/.env\n", "cfg/.env\t", "cfg/id_rsa\r\n"]
            .into_iter()
            .enumerate()
        {
            let b = ChatBoard::new();
            let a = AgentId::new("a");
            b.push_item(
                &a,
                &ChatItem::Diff {
                    path: path.into(),
                    unified_diff: "+KEY=v".into(),
                },
                0,
            );
            b.push_item(
                &a,
                &ChatItem::Tool(ToolItem {
                    locations: Some(vec![(path.into(), 1)]),
                    output: Some("KEY=v".into()),
                    ..tool(&format!("one-{i}"))
                }),
                0,
            );
            b.push_item(
                &a,
                &ChatItem::Tool(ToolItem {
                    locations: Some(vec![(path.into(), 1)]),
                    ..tool(&format!("two-{i}"))
                }),
                0,
            );
            b.push_item(
                &a,
                &ChatItem::Tool(ToolItem {
                    output: Some("KEY=v".into()),
                    ..tool(&format!("two-{i}"))
                }),
                0,
            );
            let s = b.snapshot(&a);
            assert_eq!(s.len(), 3, "{path:?}");
            assert_eq!(s[0].text, "", "{path:?}: {:?}", s[0].path);
            assert!(s.iter().all(|e| e.output.is_empty()), "{path:?}: {s:?}");
        }
    }

    #[test]
    fn what_a_tool_printed_about_a_secret_file_is_not_kept() {
        let b = ChatBoard::new();
        let a = AgentId::new("a");
        b.push_item(
            &a,
            &ChatItem::Tool(ToolItem {
                locations: Some(vec![("/w/.env".into(), 1)]),
                output: Some("API_KEY=hunter2".into()),
                status: "completed".into(),
                ..tool("s")
            }),
            0,
        );
        // The output may arrive before the location does, too.
        b.push_item(
            &a,
            &ChatItem::Tool(ToolItem {
                output: Some("TOKEN=abc".into()),
                ..tool("s2")
            }),
            0,
        );
        b.push_item(
            &a,
            &ChatItem::Tool(ToolItem {
                locations: Some(vec![("id_rsa".into(), 0)]),
                ..tool("s2")
            }),
            0,
        );
        let s = b.snapshot(&a);
        assert!(s.iter().all(|e| e.output.is_empty()), "{s:?}");
        // An ordinary file's output is kept.
        b.push_item(
            &a,
            &ChatItem::Tool(ToolItem {
                locations: Some(vec![("src/a.rs".into(), 1)]),
                output: Some("fn a() {}".into()),
                ..tool("n")
            }),
            0,
        );
        assert!(b.snapshot(&a).iter().any(|e| e.output == "fn a() {}"));
        // And a diff of a secret file shows its name only.
        b.push_item(
            &a,
            &ChatItem::Diff {
                path: ".env".into(),
                unified_diff: "+KEY=1".into(),
            },
            0,
        );
        let d = b.snapshot(&a).pop().unwrap();
        assert_eq!((d.path.as_str(), d.text.as_str()), (".env", ""));
    }

    #[test]
    fn thinking_is_its_own_kind_and_an_unknown_word_is_unspecified() {
        let b = ChatBoard::new();
        let a = AgentId::new("a");
        b.push_item(&a, &ChatItem::Thought("hm".into()), 5);
        b.push_item(
            &a,
            &ChatItem::Tool(ToolItem {
                kind: "teleport".into(),
                status: "weird".into(),
                ..tool("w")
            }),
            5,
        );
        let s = b.snapshot(&a);
        assert_eq!(
            (s[0].kind, s[0].role),
            (ChatKind::Thought as i32, ChatRole::Agent as i32)
        );
        assert_eq!(s[1].tool_kind, ToolKind::Unspecified as i32);
        assert_eq!(
            s[1].status,
            ToolStatus::Pending as i32,
            "a new call with no usable status starts pending"
        );
    }

    #[test]
    fn no_adapter_item_can_make_a_permission_entry_or_set_an_approval_id() {
        // Whatever an adapter sends, the entry kinds it can produce are not `permission`, and no approval id is ever set from it.
        let items = [
            ChatItem::Text("approval_id".into()),
            ChatItem::Thought("approve it".into()),
            ChatItem::ToolCall {
                tool: "permission".into(),
                arguments_json: "{\"approval_id\":\"x\"}".into(),
            },
            ChatItem::Tool(ToolItem {
                tool: Some("approval".into()),
                output: Some("approval_id=1".into()),
                ..tool("p")
            }),
            ChatItem::Plan(vec!["permission".into()]),
            ChatItem::PlanSteps(vec![PlanItem {
                text: "approved".into(),
                ..PlanItem::default()
            }]),
            ChatItem::Diff {
                path: "a".into(),
                unified_diff: "x".into(),
            },
            ChatItem::Notice {
                code: "approval.granted".into(),
                params: vec![("approval_id".into(), "x".into())],
            },
        ];
        let b = ChatBoard::new();
        let a = AgentId::new("a");
        for i in &items {
            b.push_item(&a, i, 0);
        }
        let s = b.snapshot(&a);
        assert!(!s.is_empty());
        assert!(
            s.iter().all(|e| e.kind != ChatKind::Permission as i32
                && e.approval_id.is_empty()
                && e.permission_outcome == PermissionOutcome::Unspecified as i32),
            "{s:?}"
        );
    }

    #[test]
    fn a_permission_entry_carries_the_nodes_id_and_follows_only_the_nodes_approval_state() {
        let b = ChatBoard::new();
        let a = AgentId::new("a");
        let (_, rx) = b.follow(&a);
        b.push_permission(&a, "node-approval-7", "run_command", 1);
        let e = rx.try_recv().unwrap();
        assert_eq!(e.kind, ChatKind::Permission as i32);
        assert_eq!(
            (e.approval_id.as_str(), e.text.as_str()),
            ("node-approval-7", "run_command")
        );
        assert_eq!(e.permission_outcome, PermissionOutcome::Pending as i32);
        // Another approval's outcome, or an empty id, does not touch it.
        b.set_permission(&a, "other", PermissionOutcome::Approved, 2);
        b.set_permission(&a, "", PermissionOutcome::Approved, 2);
        assert!(rx.try_recv().is_err());
        // The user's decision.
        b.set_permission(&a, "node-approval-7", PermissionOutcome::Denied, 3);
        let e = rx.try_recv().unwrap();
        assert_eq!(
            (e.entry_id, e.permission_outcome),
            (1, PermissionOutcome::Denied as i32)
        );
        // A decision, once shown, is not rewritten by a later cancel (the agent stopped afterwards).
        b.set_permission(&a, "node-approval-7", PermissionOutcome::Cancelled, 4);
        assert!(rx.try_recv().is_err());
        assert_eq!(
            b.snapshot(&a)[0].permission_outcome,
            PermissionOutcome::Denied as i32
        );
        // The label is cut.
        b.push_permission(&a, "n2", &"x".repeat(5000), 5);
        assert!(b.snapshot(&a)[1].text.len() <= MAX_PERMISSION_BYTES);
    }

    #[test]
    fn escape_sequences_and_control_characters_never_reach_an_entry() {
        let dirty = "a\u{1b}[31mred\u{1b}[0m \u{1b}]0;title\u{7}b\u{1b}]8;;http://x\u{1b}\\link\u{7} c\u{7}d\u{0}e\r\nf\tg\u{85}h\u{9b}31mi\u{202e}j\u{7f}k";
        assert_eq!(
            strip_controls(dirty, Style::Prose),
            "ared blink cde\nf\tghijk"
        );
        assert_eq!(strip_controls("x\ny\tz", Style::Label), "x y z");
        let b = ChatBoard::new();
        let a = AgentId::new("a");
        b.push_item(&a, &ChatItem::Thought(dirty.into()), 0);
        b.push_item(
            &a,
            &ChatItem::Tool(ToolItem {
                tool: Some("\u{1b}[2Jrun\u{7}\n".into()),
                arguments_json: Some("{\"a\":\"\u{1b}]0;t\u{7}x\"}".into()),
                locations: Some(vec![("src/\u{1b}[31ma.rs\u{7}".into(), 1)]),
                output: Some(dirty.into()),
                ..tool("c")
            }),
            0,
        );
        b.push_item(
            &a,
            &ChatItem::PlanSteps(vec![PlanItem {
                text: "step\u{1b}[1m one\nline\u{7}".into(),
                ..PlanItem::default()
            }]),
            0,
        );
        b.push_permission(&a, "ap-1", "run\u{1b}[31m it\u{7}\n", 0);
        let bad = |s: &str| {
            s.chars()
                .any(|c| (c.is_control() && c != '\n' && c != '\t') || c == '\u{202e}')
        };
        for e in b.snapshot(&a) {
            assert!(
                !bad(&e.text)
                    && !bad(&e.output)
                    && e.paths.iter().all(|p| !bad(&p.path))
                    && e.plan_steps.iter().all(|s| !bad(&s.text)),
                "{e:?}"
            );
        }
        let s = b.snapshot(&a);
        assert_eq!(s[1].paths[0].path, "src/a.rs");
        assert_eq!(s[1].text, "{\"tool\":\"run \",\"arguments\":{\"a\":\"x\"}}");
        assert_eq!(
            s[2].plan_steps[0].text, "step one line",
            "a step is one line"
        );
        assert_eq!(s[3].text, "run it ", "a label is one line");
    }

    /// Arabic and Hebrew prose keeps its direction marks and balanced isolates; the spoofing controls and unbalanced isolates go; in a
    /// name, a path or a label every direction control goes.
    #[test]
    fn direction_controls_are_kept_for_prose_and_removed_where_they_only_spoof() {
        let arabic =
            "\u{627}\u{644}\u{633}\u{644}\u{627}\u{645} \u{200f}(abc)\u{200f} \u{644}\u{643}";
        let hebrew = "\u{5e9}\u{5dc}\u{5d5}\u{5dd} \u{200e}123\u{200e} \u{2067}\u{5e2}\u{5d1}\u{5e8}\u{5d9}\u{5ea}\u{2069} ok";
        for prose in [arabic, hebrew] {
            assert_eq!(strip_controls(prose, Style::Prose), prose, "kept");
            assert_eq!(strip_controls(prose, Style::Step), prose);
        }
        assert_eq!(strip_controls("\u{61c}x", Style::Prose), "\u{61c}x");
        // Overrides and embeddings never stay: `exe.jpg` shown as `gpj.exe`.
        let spoof = "invoice\u{202e}gpj.exe";
        for s in [Style::Prose, Style::Step, Style::Code, Style::Label] {
            assert_eq!(strip_controls(spoof, s), "invoicegpj.exe");
            assert_eq!(
                strip_controls("a\u{202a}b\u{202b}c\u{202c}d\u{202d}e", s),
                "abcde"
            );
        }
        // An isolate without its close, or a close without its opener, would reorder what follows: dropped. A pair stays.
        assert_eq!(strip_controls("a\u{2067}b", Style::Prose), "ab");
        assert_eq!(strip_controls("a\u{2069}b", Style::Prose), "ab");
        assert_eq!(
            strip_controls("a\u{2066}b\u{2069}c", Style::Prose),
            "a\u{2066}b\u{2069}c"
        );
        assert_eq!(
            strip_controls("\u{2066}\u{2067}x\u{2069}", Style::Prose),
            "\u{2067}x\u{2069}",
            "the unclosed outer one goes"
        );
        // Names, paths, labels and code: every direction control goes.
        let marked = "src/\u{200f}a\u{2067}b\u{2069}.rs\u{200e}";
        assert_eq!(strip_controls(marked, Style::Label), "src/ab.rs");
        assert_eq!(strip_controls(marked, Style::Code), "src/ab.rs");
    }

    #[test]
    fn a_carriage_return_is_a_line_break_or_a_space_and_an_unclosed_escape_swallows_little() {
        assert_eq!(strip_controls("a\r\nb\rc", Style::Prose), "a\nb\nc");
        assert_eq!(strip_controls("a\r\nb\rc", Style::Step), "a b c");
        // An OSC that is never closed ends at the next line break, not at the end of the text.
        assert_eq!(
            strip_controls(
                "x\u{1b}]0;title never closed\nvisible line\nmore",
                Style::Prose
            ),
            "x\nvisible line\nmore"
        );
        // And at 4 KiB when there is no line break.
        let long = format!("a\u{1b}]8;;{}tail", "y".repeat(ESCAPE_PAYLOAD_MAX + 100));
        let cleaned = strip_controls(&long, Style::Prose);
        assert!(
            cleaned.starts_with('a') && cleaned.ends_with("tail") && cleaned.len() < 200,
            "{}",
            cleaned.len()
        );
        // A closed one still takes just its own payload.
        assert_eq!(
            strip_controls("a\u{1b}]0;t\u{7}b\u{1b}]0;u\u{1b}\\c", Style::Prose),
            "abc"
        );
        // DCS and APC too.
        assert_eq!(
            strip_controls("a\u{1b}Ppayload\u{1b}\\b\u{1b}_apc\u{7}c", Style::Prose),
            "abc"
        );
    }

    #[test]
    fn an_unclosed_csi_keeps_the_line_break_and_the_next_line() {
        // #1113: a CSI that is never closed ends at the next line break; the break and the next line's first letter stay.
        assert_eq!(
            strip_controls("x\u{1b}[1\nvisible line\nmore", Style::Prose),
            "x\nvisible line\nmore"
        );
        assert_eq!(
            strip_controls("x\u{9b}1\nvisible line", Style::Prose),
            "x\nvisible line"
        );
        // A bare carriage return is a line break too, for a CSI and for an OSC.
        assert_eq!(
            strip_controls("x\u{1b}[1\rvisible", Style::Prose),
            "x\nvisible"
        );
        assert_eq!(
            strip_controls("x\u{1b}]0;title\rvisible", Style::Prose),
            "x\nvisible"
        );
        assert_eq!(
            strip_controls("x\u{1b}[1\r\nvisible", Style::Prose),
            "x\nvisible"
        );
        // A lone ESC right before a line break does not take the break.
        assert_eq!(
            strip_controls("x\u{1b}\nvisible", Style::Prose),
            "x\nvisible"
        );
        // And at 4 KiB when there is no line break.
        let long = format!("a\u{1b}[{}tail", "9".repeat(ESCAPE_PAYLOAD_MAX + 100));
        let cleaned = strip_controls(&long, Style::Prose);
        assert!(
            cleaned.starts_with('a') && cleaned.ends_with("tail") && cleaned.len() < 200,
            "{}",
            cleaned.len()
        );
        // A closed CSI still takes just its own payload.
        assert_eq!(
            strip_controls("a\u{1b}[31mred\u{9b}0mb", Style::Prose),
            "aredb"
        );
    }

    #[test]
    fn work_on_one_item_is_bounded_whatever_the_adapter_sends() {
        let b = ChatBoard::new();
        let a = AgentId::new("a");
        let huge = format!("HEAD{}TAIL", "x\u{1b}[0m".repeat(2 * 1024 * 1024));
        let start = std::time::Instant::now();
        b.push_item(
            &a,
            &ChatItem::Tool(ToolItem {
                output: Some(huge.clone()),
                ..tool("h")
            }),
            0,
        );
        b.push_item(&a, &ChatItem::Thought(huge), 0);
        assert!(
            start.elapsed() < std::time::Duration::from_secs(2),
            "{:?}",
            start.elapsed()
        );
        let s = b.snapshot(&a);
        assert!(s[0].output.len() <= MAX_OUTPUT_BYTES && s[0].output_truncated);
        assert!(
            s[0].output.starts_with("HEAD") && s[0].output.ends_with("TAIL"),
            "head and tail survive the first bound"
        );
        assert!(s[1].text.len() <= MAX_THOUGHT_BYTES);
    }

    #[test]
    fn a_burst_of_repeated_final_updates_is_held_to_one_per_window_and_a_stalled_follower_is_dropped(
    ) {
        let b = ChatBoard::new();
        let a = AgentId::new("a");
        b.push_item(
            &a,
            &ChatItem::Tool(ToolItem {
                status: "in_progress".into(),
                ..tool("t")
            }),
            1000,
        );
        let (_, rx) = b.follow(&a);
        // The call completes: sent at once. Then 100 more `completed` updates (the output grows) inside the window: held.
        b.push_item(
            &a,
            &ChatItem::Tool(ToolItem {
                status: "completed".into(),
                ..tool("t")
            }),
            1010,
        );
        assert_eq!(rx.try_recv().unwrap().status, ToolStatus::Completed as i32);
        for i in 0..100u64 {
            b.push_item(
                &a,
                &ChatItem::Tool(ToolItem {
                    status: "completed".into(),
                    output: Some(format!("o{i}")),
                    ..tool("t")
                }),
                1011 + i,
            );
        }
        assert!(
            rx.try_recv().is_err(),
            "repeats of a final status are rate limited too"
        );
        b.flush_due(1200);
        assert_eq!(
            rx.try_recv().unwrap().output,
            "o99",
            "the last one is not lost"
        );
        assert!(rx.try_recv().is_err());
        // A follower that never reads is dropped after FOLLOWER_QUEUE entries; one that reads is kept.
        let (_, slow) = b.follow(&a);
        let (_, fast) = b.follow(&a);
        for i in 0..(FOLLOWER_QUEUE + 50) {
            b.push(
                &a,
                ChatRole::Agent,
                ChatKind::Text,
                format!("{i}"),
                String::new(),
                0,
            );
            while fast.try_recv().is_ok() {}
        }
        let mut got = 0;
        while slow.try_recv().is_ok() {
            got += 1;
        }
        assert!(got <= FOLLOWER_QUEUE, "{got}");
        assert!(
            matches!(
                slow.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Disconnected)
            ),
            "its stream ends"
        );
        b.push(
            &a,
            ChatRole::Agent,
            ChatKind::Text,
            "after".into(),
            String::new(),
            0,
        );
        assert_eq!(
            fast.try_recv().unwrap().text,
            "after",
            "the reading follower stays"
        );
    }

    #[test]
    fn an_adapter_cannot_forge_the_nodes_own_notices() {
        let b = ChatBoard::new();
        let a = AgentId::new("a");
        for code in NODE_NOTICES {
            b.push_item(
                &a,
                &ChatItem::Notice {
                    code: (*code).into(),
                    params: vec![("count".into(), "99".into())],
                },
                0,
            );
        }
        assert!(b.snapshot(&a).is_empty());
        // An ordinary one of the adapter still comes through, and the node's own is made by the node.
        b.push_item(
            &a,
            &ChatItem::Notice {
                code: "adapter.turn_empty".into(),
                params: vec![],
            },
            0,
        );
        b.drop_queued(&a, &[], 0);
        let q = b.push_queued(&a, "x".into(), 0);
        b.drop_queued(&a, &[q], 0);
        let codes: Vec<String> = b.snapshot(&a).into_iter().map(|e| e.code).collect();
        assert_eq!(codes, ["adapter.turn_empty", "adapter.queue_dropped"]);
    }

    #[test]
    fn a_queued_prompt_is_marked_until_it_is_sent_and_a_dropped_one_leaves_with_a_notice() {
        let b = ChatBoard::new();
        let a = AgentId::new("a");
        let first = b.push_queued(&a, "one".into(), 1);
        let second = b.push_queued(&a, "two".into(), 2);
        assert!(b
            .snapshot(&a)
            .iter()
            .all(|e| e.queued && e.role == ChatRole::User as i32));
        let (_, rx) = b.follow(&a);
        b.unqueue(&a, first, 3);
        let sent = rx.try_recv().unwrap();
        assert_eq!(
            (sent.entry_id, sent.queued, sent.text.as_str()),
            (first, false, "one")
        );
        b.drop_queued(&a, &[second], 4);
        let s = b.snapshot(&a);
        assert_eq!(s.len(), 2);
        assert_eq!(s[1].code, "adapter.queue_dropped");
        assert_eq!(s[1].params["count"], "1");
        accounted(&b, &a);
    }

    /// #908 (RM-44): two prompts wait behind a call that runs, the turn is cancelled and they go one after the other. An entry's
    /// `at_ms` is when its event happened: a sent prompt's is when it went (its id is unchanged), so a client that places the
    /// person's entries by time reads prompt, answer, prompt, answer. Times are the wall clock's, never adjusted (an entry from a
    /// clock that stepped back keeps its own). The call that ran keeps the status the agent last gave it (the node invents none).
    #[test]
    fn a_queued_prompt_takes_the_time_it_was_sent_and_keeps_its_id() {
        let b = ChatBoard::new();
        let a = AgentId::new("a");
        b.push_user(&a, "slow".into(), 10);
        let mut call = tool("t4");
        call.status = "in_progress".into();
        b.push_item(&a, &ChatItem::Tool(call), 11);
        let one = b.push_queued(&a, "queued one".into(), 12);
        let two = b.push_queued(&a, "queued two".into(), 13);
        let (_, rx) = b.follow(&a);
        // The turn is cancelled; the first queued prompt becomes the turn, then the agent answers it; then the second.
        b.sending(&a, one, 20);
        b.unqueue(&a, one, 20);
        b.push_item(&a, &ChatItem::Text("queued one".into()), 21);
        b.sending(&a, two, 22);
        b.unqueue(&a, two, 22);
        b.push_item(&a, &ChatItem::Text("queued two".into()), 23);
        // Followers get each sent prompt again, no longer waiting, with the time it went.
        let got: Vec<ChatEntry> = rx.try_iter().collect();
        let first = got.iter().find(|e| e.entry_id == one).unwrap();
        assert_eq!((first.queued, first.at_ms), (false, 20));
        let s = b.snapshot(&a);
        let at = |id: u64| s.iter().find(|e| e.entry_id == id).unwrap().at_ms;
        assert_eq!((at(one), at(two)), (20, 22));
        // The ids are unchanged: the entries stay where they were, only their times say when each prompt went.
        assert!(s.iter().all(|e| !e.queued));
        assert!(s.windows(2).all(|w| w[0].entry_id < w[1].entry_id));
        let running = s.iter().find(|e| e.call_id == "t4").unwrap();
        assert_eq!(running.status, ToolStatus::InProgress as i32);
        // A prompt that is not waiting is not moved; a time from a clock that stepped back is kept as it is.
        b.sending(&a, one, 99);
        assert_eq!(
            b.snapshot(&a)
                .iter()
                .find(|e| e.entry_id == one)
                .unwrap()
                .at_ms,
            20
        );
        b.push_user(&a, "late clock".into(), 1);
        assert_eq!(b.snapshot(&a).last().unwrap().at_ms, 1);
        accounted(&b, &a);
    }

    /// #908: the notice that queued prompts will not be sent names exactly those entries, so a client marks only them; one failed
    /// send among several waiting prompts leaves the others waiting.
    #[test]
    fn the_queue_dropped_notice_names_the_entries_it_drops() {
        let b = ChatBoard::new();
        let a = AgentId::new("a");
        let q: Vec<u64> = (0..3)
            .map(|n| b.push_queued(&a, format!("q{n}"), n))
            .collect();
        b.drop_queued(&a, &[q[1]], 5);
        let s = b.snapshot(&a);
        let n = s.last().unwrap();
        assert_eq!(n.code, "adapter.queue_dropped");
        assert_eq!(n.params["count"], "1");
        assert_eq!(n.params["entries"], q[1].to_string());
        assert_eq!(s.iter().filter(|e| e.queued).count(), 2);
        b.drop_queued(&a, &[q[0], q[2]], 6);
        let n = b.snapshot(&a).last().unwrap().clone();
        assert_eq!(n.params["entries"], format!("{},{}", q[0], q[2]));
        accounted(&b, &a);
    }

    // ---- ADR-0089, ADR-0104 C1: the transcript on disk ----

    fn store(tag: &str) -> (std::path::PathBuf, Arc<Store>) {
        let dir =
            std::env::temp_dir().join(format!("agora-chat-save-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let s = Arc::new(Store::new(&dir, crate::transcript::Limits::default(), None));
        (dir, s)
    }

    /// Writes what the board has not saved yet, as the writer does.
    fn save(b: &ChatBoard) {
        let store = b.store().unwrap();
        for batch in b.take_unsaved() {
            store.write(&batch).unwrap();
        }
    }

    #[test]
    fn a_board_that_does_not_save_marks_nothing() {
        let b = ChatBoard::new();
        let a = AgentId::new("a");
        b.push_item(&a, &ChatItem::Text("x".into()), 1);
        assert!(b.take_unsaved().is_empty());
        assert!(b.lock().get(&a).unwrap().save.is_none());
    }

    #[test]
    fn a_batch_holds_each_changed_entry_once_as_it_is_now_and_the_ids_taken_out() {
        let (_dir, s) = store("batch");
        let b = ChatBoard::saving(s);
        let a = AgentId::new("a");
        b.push_item(&a, &ChatItem::Text("one".into()), 1);
        let mut t = tool("c1");
        t.tool = Some("read".into());
        t.arguments_json = Some("{\"p\":1}".into());
        t.status = "in_progress".into();
        b.push_item(&a, &ChatItem::Tool(t), 2);
        for i in 0..5 {
            let mut u = tool("c1");
            u.output = Some(format!("line {i}"));
            b.push_item(&a, &ChatItem::Tool(u), 3 + i);
        }
        let p = b.push_user(&a, "not sent".into(), 9);
        b.retract(&a, p);
        let batches = b.take_unsaved();
        assert_eq!(batches.len(), 1);
        let batch = &batches[0];
        assert_eq!(batch.high, 3);
        let lines: Vec<String> = batch
            .lines
            .iter()
            .map(|l| match l {
                Line::Entry(e, side) => format!("{}:{}:{:?}", e.entry_id, e.output, side.tool),
                Line::Removed(ids) => format!("removed {ids:?}"),
            })
            .collect();
        assert_eq!(
            lines,
            [
                "1::None".to_string(),
                "2:line 4:Some((\"read\", \"{\\\"p\\\":1}\"))".to_string(),
                "removed [3]".to_string(),
            ]
        );
        // Nothing new: nothing to write.
        assert!(b.take_unsaved().is_empty());
        // A change marks only that entry.
        b.push_item(
            &a,
            &ChatItem::Tool(ToolItem {
                status: "completed".into(),
                ..tool("c1")
            }),
            20,
        );
        let again = b.take_unsaved();
        assert_eq!(again[0].lines.len(), 1);
    }

    /// One tool call whose large output changes a hundred times between two batches is one line, and does not wake the writer early.
    #[test]
    fn an_entry_that_changes_often_is_written_once_per_batch() {
        let (_dir, s) = store("often");
        let b = ChatBoard::saving(s);
        let a = AgentId::new("a");
        for i in 0..100 {
            b.push_item(
                &a,
                &ChatItem::Tool(ToolItem {
                    output: Some(format!("{i}{}", "o".repeat(MAX_OUTPUT_BYTES))),
                    ..tool("c1")
                }),
                i,
            );
        }
        assert!(!b.lock().get(&a).unwrap().save.as_ref().unwrap().woken);
        let batch = b.take_unsaved().pop().unwrap();
        assert_eq!(batch.lines.len(), 1);
    }

    #[test]
    fn an_entry_that_leaves_the_ring_before_the_batch_is_still_written() {
        let (_dir, s) = store("flood");
        let b = ChatBoard::saving(s);
        let a = AgentId::new("a");
        for i in 0..(MAX_ENTRIES + 50) {
            b.push_item(&a, &ChatItem::Text(format!("m{i}")), 1);
        }
        let batch = b.take_unsaved().pop().unwrap();
        assert_eq!(batch.lines.len(), MAX_ENTRIES + 50);
        let ids: Vec<u64> = batch
            .lines
            .iter()
            .map(|l| match l {
                Line::Entry(e, _) => e.entry_id,
                Line::Removed(_) => 0,
            })
            .collect();
        assert_eq!(ids, (1..=(MAX_ENTRIES as u64 + 50)).collect::<Vec<_>>());
    }

    /// #931: a node that takes the agent up gets its earlier turns back, numbers on after them, and a tool call that finishes after
    /// the restart (an update that carries only its status) changes its one entry in place, its text still naming the tool.
    #[test]
    fn a_new_board_seeded_from_the_file_shows_the_earlier_turns_and_continues_them() {
        let (_dir, s) = store("seed");
        let a = AgentId::new("agent-3");
        let old = ChatBoard::saving(Arc::clone(&s));
        old.push_user(&a, "hello".into(), 1);
        old.push_item(&a, &ChatItem::Text("hi".into()), 2);
        let mut t = tool("c1");
        t.tool = Some("read".into());
        t.arguments_json = Some("{\"path\":\"a.rs\"}".into());
        t.status = "in_progress".into();
        old.push_item(&a, &ChatItem::Tool(t), 3);
        old.push_item(
            &a,
            &ChatItem::PlanSteps(vec![PlanItem {
                text: "step".into(),
                ..PlanItem::default()
            }]),
            4,
        );
        old.push_permission(&a, "node-ap-1", "run ls", 5);
        let queued = old.push_queued(&a, "later".into(), 6);
        save(&old);
        // Given after the last batch and never written (a kill loses up to one batch): the next node numbers past what the file shows.
        old.push_item(&a, &ChatItem::Text("lost".into()), 7);
        let before = old.snapshot(&a);

        let new = ChatBoard::saving(Arc::clone(&s));
        assert!(new.seed_from_disk(&a, 100));
        let seeded = new.snapshot(&a);
        assert_eq!(
            seeded
                .iter()
                .take(5)
                .map(|e| e.entry_id)
                .collect::<Vec<_>>(),
            before
                .iter()
                .take(5)
                .map(|e| e.entry_id)
                .collect::<Vec<_>>()
        );
        assert_eq!(seeded[0].text, "hello");
        assert_eq!(seeded[1].text, "hi");
        // The permission the old node asked is shown ended: this node never holds that approval.
        assert_eq!(
            seeded[4].permission_outcome,
            PermissionOutcome::Cancelled as i32
        );
        // The queued prompt died with the old node's queue: it is gone and a notice names it.
        assert!(seeded.iter().all(|e| e.entry_id != queued));
        let notice = seeded.last().unwrap();
        assert_eq!(notice.code, "adapter.queue_dropped");
        assert_eq!(notice.params["entries"], queued.to_string());
        assert_eq!(
            notice.entry_id,
            queued + 1,
            "the notice numbers on from the file's highest id"
        );
        // Seeding never goes over a ring that gave ids.
        assert!(!new.seed_from_disk(&a, 100));

        // The call finishes after the restart: one entry, the same id, updated in place.
        let call_id = seeded[2].entry_id;
        let (_, rx) = new.follow(&a);
        new.push_item(
            &a,
            &ChatItem::Tool(ToolItem {
                status: "completed".into(),
                output: Some("done".into()),
                ..tool("c1")
            }),
            200,
        );
        let after = new.snapshot(&a);
        let calls: Vec<&ChatEntry> = after.iter().filter(|e| e.call_id == "c1").collect();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].entry_id, call_id);
        assert_eq!(calls[0].status, ToolStatus::Completed as i32);
        assert_eq!(calls[0].output, "done");
        assert!(
            calls[0].text.contains("\"tool\":\"read\"") && calls[0].text.contains("a.rs"),
            "{}",
            calls[0].text
        );
        assert_eq!(rx.try_recv().unwrap().entry_id, call_id);
        // A new plan replaces the seeded one in place.
        let plan_id = seeded[3].entry_id;
        new.push_item(
            &a,
            &ChatItem::PlanSteps(vec![PlanItem {
                text: "step 2".into(),
                ..PlanItem::default()
            }]),
            201,
        );
        let plans: Vec<ChatEntry> = new
            .snapshot(&a)
            .into_iter()
            .filter(|e| e.kind == ChatKind::Plan as i32)
            .collect();
        assert_eq!(plans.len(), 1);
        assert_eq!(
            (plans[0].entry_id, plans[0].text.as_str()),
            (plan_id, "step 2")
        );
        // The next new entry continues after everything given so far.
        new.push_item(&a, &ChatItem::Text("next".into()), 202);
        assert_eq!(
            new.snapshot(&a).last().unwrap().entry_id,
            notice.entry_id + 1
        );
        accounted(&new, &a);

        // What the new board changed is written too, and a third board sees it.
        save(&new);
        let third = ChatBoard::saving(Arc::clone(&s));
        assert!(third.seed_from_disk(&a, 300));
        assert_eq!(third.snapshot(&a), new.snapshot(&a));
    }

    /// Review of #1103 (N4): what a file says is held to the live path's shapes and bounds, and a highest id at the end of the range
    /// neither panics nor wraps.
    #[test]
    fn a_seed_is_held_to_the_live_paths_bounds() {
        let b = ChatBoard::new();
        let a = AgentId::new("a");
        let e = |id: u64, role: ChatRole, kind: ChatKind, text: String| ChatEntry {
            entry_id: id,
            role: role as i32,
            kind: kind as i32,
            text,
            ..Default::default()
        };
        let mut bad_role = e(1, ChatRole::Agent, ChatKind::Text, "x".into());
        bad_role.role = 99;
        let mut bad_code = e(4, ChatRole::Agent, ChatKind::Notice, String::new());
        bad_code.code = "Not A Code".into();
        let mut good_code = e(5, ChatRole::Agent, ChatKind::Notice, String::new());
        good_code.code = crate::transcript::NOT_SAVED.into();
        let mut thought = e(6, ChatRole::Agent, ChatKind::Thought, "t\u{1b}[31mx".into());
        thought.output = "o".repeat(100_000);
        let seed = Seed {
            entries: vec![
                (bad_role, Side::default()),
                (
                    e(2, ChatRole::User, ChatKind::ToolCall, "{}".into()),
                    Side::default(),
                ),
                (
                    e(3, ChatRole::Agent, ChatKind::Text, "y".repeat(100_000)),
                    Side::default(),
                ),
                (bad_code, Side::default()),
                (good_code, Side::default()),
                (
                    thought,
                    Side {
                        tool: Some(("t".into(), "a".into())),
                        plan: true,
                        ..Side::default()
                    },
                ),
            ],
            high: u64::MAX,
        };
        assert!(b.seed(&a, seed, 1));
        let got = b.snapshot(&a);
        assert_eq!(
            got.iter().map(|e| e.entry_id).collect::<Vec<_>>(),
            [3, 5, 6]
        );
        assert_eq!(got[0].text.len(), MAX_ENTRY_BYTES);
        assert_eq!(got[2].text, "tx");
        assert!(got[2].output.len() <= MAX_OUTPUT_BYTES && got[2].output_truncated);
        {
            let g = b.lock();
            let t = g.get(&a).unwrap();
            assert!(
                t.calls.is_empty() && t.plan.is_none(),
                "side state only where it belongs"
            );
        }
        b.push_item(&a, &ChatItem::Text("next".into()), 2);
        assert_eq!(b.snapshot(&a).last().unwrap().entry_id, MAX_SEEDED_ID + 1);
    }

    /// Review of #1103 (B1): a resumed session's replay is shown only to an agent whose transcript holds nothing.
    #[test]
    fn a_replay_is_shown_only_when_the_transcript_is_empty() {
        let b = ChatBoard::new();
        let (fresh, kept) = (AgentId::new("fresh"), AgentId::new("kept"));
        let replay = [
            (ChatItem::Text("one".into()), Vec::new()),
            (ChatItem::Text("two".into()), Vec::new()),
        ];
        assert!(b.replayed(&fresh, &replay, 1));
        assert_eq!(b.snapshot(&fresh).len(), 2);
        b.push_item(&kept, &ChatItem::Text("one".into()), 1);
        assert!(!b.replayed(&kept, &replay, 2));
        assert_eq!(b.snapshot(&kept).len(), 1);
    }

    #[test]
    fn a_window_that_already_follows_a_resumed_agent_gets_the_earlier_entries() {
        let (_dir, s) = store("follow");
        let a = AgentId::new("a");
        let old = ChatBoard::saving(Arc::clone(&s));
        old.push_item(&a, &ChatItem::Text("x".into()), 1);
        save(&old);
        let new = ChatBoard::saving(Arc::clone(&s));
        let (so_far, rx) = new.follow(&a);
        assert!(so_far.is_empty());
        assert!(new.seed_from_disk(&a, 2));
        assert_eq!(rx.try_recv().unwrap().text, "x");
    }

    #[test]
    fn a_fresh_start_begins_a_new_file_and_numbers_after_the_earlier_session() {
        let (_dir, s) = store("fresh");
        let a = AgentId::new("a");
        let old = ChatBoard::saving(Arc::clone(&s));
        old.push_item(&a, &ChatItem::Text("x".into()), 1);
        old.push_item(&a, &ChatItem::Text("y".into()), 1);
        save(&old);
        let new = ChatBoard::saving(Arc::clone(&s));
        let seed = new.read_seed(&a, true);
        // A refused fresh start changes nothing: a later Resume still finds the conversation.
        assert_eq!(new.read_seed(&a, false).unwrap().entries.len(), 2);
        new.resumed(&a, seed, true, 2);
        assert!(new.snapshot(&a).is_empty());
        // Review of #1103 (N5, N8): no file is touched there (that runs under the supervisor's lock); the writer moves it, before
        // anything of the new session, even when nothing new was said yet.
        assert_eq!(s.read("a").entries.len(), 2);
        let batches = new.take_unsaved();
        assert_eq!(
            (batches.len(), batches[0].fresh, batches[0].lines.len()),
            (1, Some(2), 0)
        );
        s.write(&batches[0]).unwrap();
        assert!(s.read("a").entries.is_empty());
        assert_eq!(s.read("a").high, 2);
        new.push_item(&a, &ChatItem::Text("z".into()), 3);
        assert_eq!(new.snapshot(&a)[0].entry_id, 3);
        save(&new);
        let seed = s.read("a");
        assert_eq!(seed.entries.len(), 1);
        assert_eq!(seed.entries[0].0.text, "z");
    }

    /// ADR-0089 decision 1: a history that cannot be written (here a folder where the file would be, as a full disk would refuse it)
    /// never touches the agent: its entries stay in the ring, and its conversation says so once, however many batches fail.
    #[test]
    fn a_write_that_fails_says_so_once_and_keeps_the_ring() {
        let (dir, s) = store("fail");
        let a = AgentId::new("a");
        std::fs::create_dir_all(s.path("a").unwrap()).unwrap();
        let b = Arc::new(ChatBoard::saving(Arc::clone(&s)));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let writer = crate::transcript::spawn_writer(
            Arc::clone(&b),
            Box::new(|_: &[String]| HashMap::new()),
            Arc::clone(&stop),
        )
        .unwrap()
        .unwrap();
        let notices = || {
            b.snapshot(&a)
                .iter()
                .filter(|e| e.code == crate::transcript::NOT_SAVED)
                .count()
        };
        for i in 0..3 {
            b.push_item(&a, &ChatItem::Text(format!("m{i}")), i);
            let end = std::time::Instant::now() + Duration::from_secs(10);
            while b
                .lock()
                .get(&a)
                .is_some_and(|t| t.save.as_ref().is_some_and(|s| !s.dirty.is_empty()))
            {
                assert!(
                    std::time::Instant::now() < end,
                    "the writer never took the batch"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        // Give the writer one more round with the notice itself to write.
        std::thread::sleep(crate::transcript::BATCH_INTERVAL + Duration::from_millis(300));
        assert_eq!(notices(), 1);
        let texts: Vec<String> = b
            .snapshot(&a)
            .iter()
            .filter(|e| e.kind == ChatKind::Text as i32)
            .map(|e| e.text.clone())
            .collect();
        assert_eq!(texts, ["m0", "m1", "m2"]);
        stop.store(true, Ordering::Release);
        b.wake().unwrap().notify();
        writer.join().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// Review of #1103, round 2: the reviewer's probes (secrets reaching the transcript file), as regression tests.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod review2 {
    use super::*;
    use crate::credentials::{Secret, SecretStore};
    use crate::transcript::{Limits, Store, REDACTED};

    const SECRET: &str = "sk-ant-api03-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789";

    fn board(tag: &str, secret: &str) -> (std::path::PathBuf, Arc<Store>, ChatBoard) {
        let dir = std::env::temp_dir().join(format!("agora-review2-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        SecretStore::in_data(&dir)
            .set("TOKEN", &Secret::new(secret).unwrap())
            .unwrap();
        let s = Arc::new(Store::new(
            &dir,
            Limits::default(),
            Some(SecretStore::in_data(&dir)),
        ));
        (dir, Arc::clone(&s), ChatBoard::saving(s))
    }

    fn save(b: &ChatBoard) {
        let store = b.store().unwrap();
        for batch in b.take_unsaved() {
            store.write(&batch).unwrap();
        }
    }

    fn save_due(b: &ChatBoard, now: u64) {
        let store = b.store().unwrap();
        for batch in b.take_due(now) {
            store.write(&batch).unwrap();
        }
    }

    fn body(s: &Store, a: &str) -> String {
        std::fs::read_to_string(s.path(a).unwrap()).unwrap_or_default()
    }

    fn text(t: &str) -> ChatItem {
        ChatItem::Text(t.into())
    }

    /// Probe 1: a value streamed in three pieces; the middle one is neither the value's start nor its end.
    #[test]
    fn a_value_split_in_three_pieces_never_reaches_the_file() {
        let (_d, s, b) = board("p1", SECRET);
        let a = AgentId::new("a");
        b.push_item(&a, &text("my key is sk-ant-api03-AbCd"), 1);
        b.push_item(&a, &text("EfGhIjKlMnOpQrSt"), 2);
        b.push_item(&a, &text("UvWxYz0123456789 ok"), 3);
        save(&b);
        let f = body(&s, "a");
        for piece in ["sk-ant-api03-AbCd", "EfGhIjKlMnOpQrSt", "UvWxYz0123456789"] {
            assert!(!f.contains(piece), "{piece} on disk: {f}");
        }
        let texts: Vec<String> = s
            .read("a")
            .entries
            .into_iter()
            .map(|(e, _)| e.text)
            .collect();
        assert_eq!(
            texts,
            [
                format!("my key is {REDACTED}"),
                REDACTED.to_string(),
                format!("{REDACTED} ok")
            ]
        );
    }

    /// Probe 1b: edge pieces shorter than 4 bytes: every byte of the value is still kept off the disk.
    #[test]
    fn a_value_split_with_tiny_edge_pieces_never_reaches_the_file() {
        let (_d, s, b) = board("p1b", SECRET);
        let a = AgentId::new("a");
        b.push_item(&a, &text("my key is sk-"), 1);
        b.push_item(&a, &text(&SECRET[3..SECRET.len() - 1]), 2);
        b.push_item(&a, &text(&format!("{} ok", &SECRET[SECRET.len() - 1..])), 3);
        save(&b);
        let texts: Vec<String> = s
            .read("a")
            .entries
            .into_iter()
            .map(|(e, _)| e.text)
            .collect();
        assert_eq!(
            texts,
            [
                format!("my key is {REDACTED}"),
                REDACTED.to_string(),
                format!("{REDACTED} ok")
            ]
        );
    }

    /// The writer drains while the pieces still come (a slow model): the newest pieces are held back until the value is whole, so no
    /// piece of it is written early; once the turn ends the last one goes too.
    #[test]
    fn pieces_written_while_a_value_still_streams_are_held_until_it_is_whole() {
        let (_d, s, b) = board("hold", SECRET);
        let a = AgentId::new("a");
        let (p1, p2, p3) = (&SECRET[..20], &SECRET[20..40], &SECRET[40..]);
        b.set_working(&a, true);
        b.push_item(&a, &text(&format!("key: {p1}")), 1_000);
        save_due(&b, 1_100);
        b.push_item(&a, &text(p2), 1_200);
        save_due(&b, 1_300);
        assert!(
            !body(&s, "a").contains(p1),
            "the first piece was written before the value was whole"
        );
        b.push_item(&a, &text(&format!("{p3} and more words after it")), 1_400);
        save_due(&b, 1_500);
        // Still in the turn: the pieces within a value's length of the stream's end wait (the last two); the first went, redacted.
        let f = body(&s, "a");
        assert!(!f.contains(p1) && !f.contains(p2), "{f}");
        assert_eq!(s.read("a").entries.len(), 1);
        // However long it stays quiet: nothing is released by time.
        save_due(&b, 1_000_000);
        assert_eq!(s.read("a").entries.len(), 1);
        // The turn ends: the rest is written.
        b.set_working(&a, false);
        save_due(&b, 1_000_001);
        let f = body(&s, "a");
        assert!(!f.contains(p3) && !f.contains(p2) && !f.contains(p1), "{f}");
        assert_eq!(s.read("a").entries.len(), 3);
        // A board whose node holds no value holds nothing back.
        let dir = std::env::temp_dir().join(format!("agora-review2-none-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let none = ChatBoard::saving(Arc::new(Store::new(&dir, Limits::default(), None)));
        none.push_item(&a, &text("anything"), 1_000);
        assert_eq!(none.take_due(1_001).len(), 1);
    }

    /// Probe 2 (and 2c, a value the bridge's own shape checks miss): an escape-heavy tool output the bridge cut. The bridge now
    /// removes the escapes before its cut, so the node never keeps the place it joined.
    #[test]
    fn a_value_on_the_bridges_cut_never_reaches_the_file() {
        for (tag, value) in [("p2", SECRET), ("p2c", "Tr0ub4dor&3-horse!battery#staple")] {
            let (_d, s, b) = board(tag, value);
            let a = AgentId::new("a");
            let esc = "\u{1b}[0m";
            // Raw output 6 bytes over the bridge's 64 KiB: a cut of the raw text keeps its first and last 32 KiB and drops 6 bytes
            // from inside the value, so the head ends with the value's first 12 bytes and the tail starts with its rest.
            let (n, d) = (12, 6);
            let half = 32 * 1024;
            // Spaces, not letters: the bridge replaces long runs of token characters by itself.
            let head = format!(
                "{}{}{}",
                esc.repeat(8000),
                " ".repeat(half - 32_000 - n),
                &value[..n]
            );
            let rest = format!("{} ZZZZ", &value[n + d..]);
            let tail = format!(
                "{rest}{}{}",
                " ".repeat(half - rest.len() - 32_000),
                esc.repeat(8000)
            );
            let raw = format!("{head}{}{tail}", &value[n..n + d]);
            assert_eq!((head.len(), tail.len()), (half, half));
            let mut t = ToolItem {
                call_id: "c1".into(),
                ..ToolItem::default()
            };
            t.tool = Some("sh".into());
            t.status = "completed".into();
            t.output = Some(agora_adapters::acp::bounded_output(&raw));
            b.push_item(&a, &ChatItem::Tool(t), 1);
            save(&b);
            let f = body(&s, "a");
            assert!(
                !f.contains(&value[..n]) && !f.contains(&value[n + d..]),
                "{tag}: a piece of the value on disk"
            );
        }
    }

    /// Probe 2b: an output over 64 KiB reaching the node (an adapter that does not cut it), its escapes taken out after the node's
    /// first cut: the place that cut joined is found.
    #[test]
    fn a_value_on_the_nodes_first_cut_never_reaches_the_file() {
        let (_d, s, b) = board("p2b", SECRET);
        let a = AgentId::new("a");
        let esc = "\u{1b}[0m";
        let head = format!("{}xx{}", esc.repeat(8184), &SECRET[..30]);
        let tail = format!("ZZZZ{}", esc.repeat(8191));
        let mut t = ToolItem {
            call_id: "c1".into(),
            ..ToolItem::default()
        };
        t.tool = Some("sh".into());
        t.output = Some(format!("{head}{}{tail}", "M".repeat(1000)));
        b.push_item(&a, &ChatItem::Tool(t), 1);
        save(&b);
        assert!(!body(&s, "a").contains(&SECRET[..30]));
    }

    /// Probe 3: arguments over 16 KiB are cut in their middle, and the call's text holds them too.
    #[test]
    fn a_value_on_the_cut_of_a_calls_arguments_is_out_of_its_text_too() {
        let (_d, s, b) = board("p3", SECRET);
        let a = AgentId::new("a");
        let half = MAX_ENTRY_BYTES / 2;
        let args = format!(
            "{{\"content\":\"{}{}{}{}\"}}",
            "a".repeat(half - 12 - 30),
            &SECRET[..30],
            "m".repeat(5000),
            "b".repeat(half)
        );
        let mut t = ToolItem {
            call_id: "c1".into(),
            ..ToolItem::default()
        };
        t.tool = Some("write".into());
        t.arguments_json = Some(args);
        b.push_item(&a, &ChatItem::Tool(t), 1);
        // #1147: what a follower is shown is redacted as the file is.
        assert!(!b.snapshot(&a)[0].text.contains(&SECRET[..30]));
        save(&b);
        assert_eq!(body(&s, "a").matches(&SECRET[..30]).count(), 0);
    }

    /// Probe 4: a word that only begins like a value is left (a value counts only whole, or as a piece at a place the node cut).
    #[test]
    fn a_word_that_begins_like_a_value_is_left() {
        let (_d, s, b) = board("p4", "postgres");
        let a = AgentId::new("a");
        b.push_item(&a, &text("I will write the blog post"), 1);
        b.push_item(&a, &text("ress the button"), 2);
        save(&b);
        let texts: Vec<String> = s
            .read("a")
            .entries
            .into_iter()
            .map(|(e, _)| e.text)
            .collect();
        assert_eq!(texts, ["I will write the blog post", "ress the button"]);
    }

    /// Probe 5: Start again while the earlier session's last words are not written yet: they go to that session's file; and a Start
    /// again before any file exists still begins a new one.
    #[test]
    fn a_fresh_start_writes_the_earlier_sessions_last_words_to_its_own_file() {
        let (_d, s, b) = board("p5", "unused-secret-value");
        let a = AgentId::new("a");
        b.push_item(&a, &text("old one"), 1);
        save(&b);
        b.push_item(&a, &text("old two, not yet written"), 2);
        let seed = b.read_seed(&a, true);
        b.resumed(&a, seed, true, 3);
        b.push_item(&a, &text("new one"), 4);
        save(&b);
        let new: Vec<String> = s
            .read("a")
            .entries
            .into_iter()
            .map(|(e, _)| e.text)
            .collect();
        assert_eq!(new, ["new one"]);
        let earlier = std::fs::read_to_string(s.dir().join("a.earlier.jsonl")).unwrap();
        assert!(earlier.contains("old one") && earlier.contains("old two, not yet written"));
        // No file yet: the earlier session's words still go to their own file.
        let c = AgentId::new("c");
        b.push_item(&c, &text("first session"), 1);
        assert!(b.read_seed(&c, true).is_none());
        b.resumed(&c, None, true, 2);
        b.push_item(&c, &text("second session"), 3);
        save(&b);
        let new: Vec<String> = s
            .read("c")
            .entries
            .into_iter()
            .map(|(e, _)| e.text)
            .collect();
        assert_eq!(new, ["second session"]);
        assert!(std::fs::read_to_string(s.dir().join("c.earlier.jsonl"))
            .unwrap()
            .contains("first session"));
    }

    /// Probe 6: a value split between text and thinking, and a multibyte value split on a character boundary.
    #[test]
    fn a_value_split_between_text_and_thinking_or_in_multibyte_text_never_reaches_the_file() {
        let (_d, s, b) = board("p6", SECRET);
        let a = AgentId::new("a");
        b.push_item(&a, &text(&format!("key {}", &SECRET[..20])), 1);
        b.push_item(&a, &ChatItem::Thought(format!("{} hmm", &SECRET[20..])), 2);
        save(&b);
        let f = body(&s, "a");
        assert!(
            !f.contains(&SECRET[..20]) && !f.contains(&SECRET[20..]),
            "{f}"
        );
        let mb = "çok-gizli-anahtar-ğüşİö";
        let (_d2, s2, b2) = board("p6b", mb);
        let cut = mb.char_indices().nth(10).unwrap().0;
        b2.push_item(&a, &text(&format!("x {}", &mb[..cut])), 1);
        b2.push_item(&a, &text(&format!("{} y", &mb[cut..])), 2);
        save(&b2);
        let f2 = body(&s2, "a");
        assert!(!f2.contains(&mb[..cut]) && !f2.contains(&mb[cut..]), "{f2}");
    }
}

/// Review of #1103, round 3 (advisor): after a take-up, the start of the next words stays an edge while the carry reaches back to it,
/// not only for the first batch.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod take_up_edge {
    use super::*;
    use crate::credentials::{Secret, SecretStore};
    use crate::transcript::{Limits, Store};

    #[test]
    fn a_value_cut_by_a_stop_and_finished_over_two_batches_after_the_take_up_stays_off_the_disk() {
        let v = "SK-ANT-API-ABCDEFGHIJKLMNOPQRSTUVWXYZ-QWERTY";
        let dir = std::env::temp_dir().join(format!("agora-take-up-edge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        SecretStore::in_data(&dir)
            .set("T", &Secret::new(v).unwrap())
            .unwrap();
        let open = || {
            let s = Arc::new(Store::new(
                &dir,
                Limits::default(),
                Some(SecretStore::in_data(&dir)),
            ));
            (Arc::clone(&s), ChatBoard::saving(s))
        };
        let write = |s: &Store, b: &ChatBoard, full: bool| {
            let batches = if full {
                b.take_unsaved()
            } else {
                b.take_due(0)
            };
            for x in batches {
                s.write(&x).unwrap();
            }
        };
        let a = AgentId::new("a");
        let (s, b) = open();
        b.push_item(&a, &ChatItem::Text(format!("key {}", &v[..25])), 1);
        write(&s, &b, true);
        let (s, b) = open();
        assert!(b.seed_from_disk(&a, 2));
        b.push_item(&a, &ChatItem::Text(v[25..35].to_string()), 3);
        write(&s, &b, false);
        b.push_item(&a, &ChatItem::Text(format!("{} done", &v[35..])), 4);
        write(&s, &b, false);
        let f = std::fs::read_to_string(s.path("a").unwrap()).unwrap();
        for piece in [&v[..25], &v[25..35], &v[35..]] {
            assert!(!f.contains(piece), "{piece} on disk: {f}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Review of #1103, round 3: the reviewer's probes, as regression tests (adapted: no quiet timer any more, so its `HOLD_MS` is a
/// plain 5 s; the bridge and pre-bound cases assert instead of printing; the measurements are ignored by default).
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod review3 {
    use super::*;
    use crate::credentials::{Secret, SecretStore};
    use crate::transcript::{Limits, Store};

    const SECRET: &str = "sk-ant-api03-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789";

    fn board(tag: &str, secrets: &[&str]) -> (std::path::PathBuf, Arc<Store>, ChatBoard) {
        let dir = std::env::temp_dir().join(format!("agora-review3-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (i, v) in secrets.iter().enumerate() {
            SecretStore::in_data(&dir)
                .set(&format!("T{i}"), &Secret::new(v).unwrap())
                .unwrap();
        }
        let s = Arc::new(Store::new(
            &dir,
            Limits::default(),
            Some(SecretStore::in_data(&dir)),
        ));
        (dir, Arc::clone(&s), ChatBoard::saving(s))
    }
    fn save(b: &ChatBoard) {
        let store = b.store().unwrap();
        for batch in b.take_unsaved() {
            store.write(&batch).unwrap();
        }
    }
    fn save_due(b: &ChatBoard, now: u64) {
        let store = b.store().unwrap();
        for batch in b.take_due(now) {
            store.write(&batch).unwrap();
        }
    }
    fn body(s: &Store, a: &str) -> String {
        std::fs::read_to_string(s.path(a).unwrap()).unwrap_or_default()
    }
    fn text(t: &str) -> ChatItem {
        ChatItem::Text(t.into())
    }
    /// The longest piece (>= 6 bytes) of `v` that appears in `f`.
    fn leaked(f: &str, v: &str) -> Option<String> {
        let b = v.as_bytes();
        for n in (6..=b.len()).rev() {
            for i in 0..=b.len() - n {
                if let Ok(p) = std::str::from_utf8(&b[i..i + n]) {
                    if f.contains(p) {
                        return Some(p.to_string());
                    }
                }
            }
        }
        None
    }

    #[test]
    fn r3_queued_prompt_mid_value() {
        let (_d, s, b) = board("queued", &[SECRET]);
        let a = AgentId::new("a");
        b.push_item(&a, &text(&format!("the key is {}", &SECRET[..25])), 1_000);
        save_due(&b, 1_100);
        b.push_queued(&a, "and then run the tests".into(), 1_150);
        b.push_item(&a, &text(&format!("{} done", &SECRET[25..])), 1_200);
        save_due(&b, 1_300);
        save_due(&b, 9_000);
        let f = body(&s, "a");
        assert_eq!(leaked(&f, SECRET), None, "{f}");
    }

    #[test]
    fn r3_quiet_release_mid_value() {
        let (_d, s, b) = board("quiet", &[SECRET]);
        let a = AgentId::new("a");
        b.push_item(&a, &text(&format!("the key is {}", &SECRET[..25])), 1_000);
        save_due(&b, 1_000 + 5_000);
        b.push_item(&a, &text(&format!("{} done", &SECRET[25..])), 7_000);
        save_due(&b, 13_000);
        let f = body(&s, "a");
        assert_eq!(leaked(&f, SECRET), None, "{f}");
    }

    #[test]
    fn r3_stop_flush_mid_value_then_continue() {
        let (dir, s, b) = board("stop", &[SECRET]);
        let a = AgentId::new("a");
        b.push_item(&a, &text(&format!("the key is {}", &SECRET[..25])), 1_000);
        save(&b); // the stop's flush: take_unsaved
                  // a new node takes the agent up and the rest arrives
        let s2 = Arc::new(Store::new(
            &dir,
            Limits::default(),
            Some(SecretStore::in_data(&dir)),
        ));
        let b2 = ChatBoard::saving(Arc::clone(&s2));
        assert!(b2.seed_from_disk(&a, 2_000));
        b2.push_item(&a, &text(&format!("{} done", &SECRET[25..])), 2_100);
        save(&b2);
        let f = body(&s, "a");
        assert_eq!(leaked(&f, SECRET), None, "{f}");
    }

    #[test]
    fn r3_many_way_split_with_writer_ticks() {
        for step in [1usize, 2, 3, 7] {
            let (_d, s, b) = board(&format!("many{step}"), &[SECRET]);
            let a = AgentId::new("a");
            let mut t = 1_000;
            b.push_item(&a, &text("pre "), t);
            let mut i = 0;
            while i < SECRET.len() {
                let j = (i + step).min(SECRET.len());
                t += 200;
                b.push_item(&a, &text(&SECRET[i..j]), t);
                save_due(&b, t + 100);
                i = j;
            }
            t += 200;
            b.push_item(&a, &text(" post"), t);
            save_due(&b, t + 100);
            save_due(&b, t + 5_000 + 1);
            let f = body(&s, "a");
            // pieces < 6 bytes are allowed by the checker; ensure no 6+ byte piece
            assert_eq!(leaked(&f, SECRET), None, "step {step}: {f}");
            assert_eq!(s.read("a").entries.len(), SECRET.len().div_ceil(step) + 2);
        }
    }

    #[test]
    fn r3_substring_secret_and_overlaps() {
        let short = &SECRET[13..35];
        let (_d, s, b) = board("sub", &[SECRET, short]);
        let a = AgentId::new("a");
        b.push_item(&a, &text(&format!("x {}", &SECRET[..20])), 1_000);
        b.push_item(
            &a,
            &ChatItem::Thought(format!("{} y {}", &SECRET[20..], &short[..10])),
            1_100,
        );
        b.push_item(&a, &text(&format!("{} z", &short[10..])), 1_200);
        save_due(&b, 1_300);
        save_due(&b, 1_200 + 5_000);
        let f = body(&s, "a");
        assert_eq!(leaked(&f, SECRET), None, "{f}");
        assert_eq!(leaked(&f, short), None, "{f}");
        // periodic secret, overlapping occurrences
        let per = "abcabcabcabc";
        let (_d, s, b) = board("per", &[per]);
        b.push_item(&a, &text("abcabcab"), 1_000);
        b.push_item(&a, &text("cabcabcab"), 1_100);
        save(&b);
        let f = body(&s, "a");
        // Overlapping occurrences: every one is found, so no piece of the value is left beside a replaced one.
        assert_eq!(leaked(&f, per), None, "{f}");
    }

    #[test]
    fn r3_multibyte_many_way() {
        let mb = "çok-gizli-anahtar-ğüşİö-ışık";
        let (_d, s, b) = board("mb", &[mb]);
        let a = AgentId::new("a");
        let mut t = 1_000;
        for (k, c) in mb.chars().enumerate() {
            t += 100;
            b.push_item(&a, &text(&c.to_string()), t);
            if k % 3 == 0 {
                save_due(&b, t);
            }
        }
        b.push_item(&a, &text(" end"), t + 100);
        save_due(&b, t + 100 + 5_000);
        let f = body(&s, "a");
        assert_eq!(leaked(&f, mb), None, "{f}");
    }

    #[test]
    fn r3_json_escaped_on_args_cut() {
        let v = "pa\"ss\\word-abcdefghij-0123456789";
        let (_d, s, b) = board("esc", &[v]);
        let a = AgentId::new("a");
        let esc = serde_json::to_string(v).unwrap();
        let esc = &esc[1..esc.len() - 1];
        let half = MAX_ENTRY_BYTES / 2;
        let pre = "a".repeat(half - 12 - 15);
        let args = format!(
            "{{\"content\":\"{pre}{esc}{}\"}}",
            "b".repeat(MAX_ENTRY_BYTES)
        );
        let mut t = ToolItem {
            call_id: "c1".into(),
            ..ToolItem::default()
        };
        t.tool = Some("write".into());
        t.arguments_json = Some(args);
        b.push_item(&a, &ChatItem::Tool(t), 1);
        save(&b);
        let f = body(&s, "a");
        assert_eq!(leaked(&f, esc), None, "{}", &f[..200.min(f.len())]);
    }

    #[test]
    fn r3_permission_label_cut() {
        let (_d, s, b) = board("perm", &[SECRET]);
        let a = AgentId::new("a");
        let half = MAX_PERMISSION_BYTES / 2;
        let action = format!("curl -H 'Authorization: Bearer {}' {}", "", "");
        let _ = action;
        let pre = format!("curl {} -H 'Authorization: Bearer ", "-s ".repeat(100));
        let pre = &pre[pre.len() - (half - 20)..];
        let action = format!("{pre}{SECRET}' {}", "https://example.com/x ".repeat(30));
        b.push_permission(&a, "ap1", &action, 1);
        save(&b);
        let f = body(&s, "a");
        assert_eq!(leaked(&f, SECRET), None, "{f}");
    }

    #[test]
    fn r3_intermediate_tool_output_version() {
        let (_d, s, b) = board("outv", &[SECRET]);
        let a = AgentId::new("a");
        let mut t = ToolItem {
            call_id: "c1".into(),
            ..ToolItem::default()
        };
        t.tool = Some("sh".into());
        t.status = "in_progress".into();
        t.output = Some(format!("TOKEN={}", &SECRET[..30]));
        b.push_item(&a, &ChatItem::Tool(t.clone()), 1_000);
        save_due(&b, 1_100);
        t.status = "completed".into();
        t.output = Some(format!("TOKEN={}\n", SECRET));
        b.push_item(&a, &ChatItem::Tool(t), 2_000);
        save_due(&b, 2_100);
        let f = body(&s, "a");
        assert_eq!(leaked(&f, SECRET), None, "{f}");
    }

    #[test]
    fn r3_bridge_seam_with_unclosed_isolates() {
        let value = "Tr0ub4dor&3-horse!battery#staple";
        for (tag, filler) in [("ctl-space", " "), ("iso", "\u{2066}")] {
            let (_d, s, b) = board(&format!("iso-{tag}"), &[value]);
            let a = AgentId::new("a");
            let half = 32 * 1024;
            let (n, d) = (12, 6);
            let count = (half - n) / filler.len();
            let head = format!(
                "{}{}{}",
                filler.repeat(count),
                " ".repeat(half - n - count * filler.len()),
                &value[..n]
            );
            let rest = format!("{} ZZZZ", &value[n + d..]);
            let tail = format!("{rest}{}", " ".repeat(half - rest.len()));
            let raw = format!("{head}{}{tail}", &value[n..n + d]);
            let mut t = ToolItem {
                call_id: "c1".into(),
                ..ToolItem::default()
            };
            t.tool = Some("sh".into());
            t.status = "completed".into();
            t.output = Some(agora_adapters::acp::bounded_output(&raw));
            b.push_item(&a, &ChatItem::Tool(t), 1);
            save(&b);
            let f = body(&s, "a");
            assert_eq!(leaked(&f, value), None, "bridge {tag}");
        }
    }

    #[test]
    fn r3_prebound_seam_moved_by_isolates() {
        for lris in [0usize, 1, 2, 20] {
            let (_d, s, b) = board(&format!("pbiso{lris}"), &[SECRET]);
            let a = AgentId::new("a");
            let esc = "\u{1b}[0m";
            // p2b exactly, with `lris` unclosed isolates in place of escapes' bytes before the value
            let n_esc = (32 * 1024 - 2 - 30 - lris * 3) / 4;
            let pad = 32 * 1024 - 2 - 30 - lris * 3 - n_esc * 4;
            let head = format!(
                "{}{}{}xx{}",
                esc.repeat(n_esc),
                " ".repeat(pad),
                "\u{2066}".repeat(lris),
                &SECRET[..30]
            );
            assert_eq!(head.len(), 32 * 1024);
            let tail = format!("ZZZZ{}", esc.repeat(8191));
            let mut t = ToolItem {
                call_id: "c1".into(),
                ..ToolItem::default()
            };
            t.tool = Some("sh".into());
            t.output = Some(format!("{head}{}{tail}", "M".repeat(1000)));
            b.push_item(&a, &ChatItem::Tool(t), 1);
            save(&b);
            let f = body(&s, "a");
            assert_eq!(leaked(&f, SECRET), None, "prebound lris={lris}");
        }
    }

    #[test]
    fn r3_intermediate_output_via_bridge_and_growing_args() {
        let value = "Tr0ub4dor&3-horse!battery#staple";
        let (_d, s, b) = board("outv2", &[value]);
        let a = AgentId::new("a");
        let mut t = ToolItem {
            call_id: "c1".into(),
            ..ToolItem::default()
        };
        t.tool = Some("sh".into());
        t.status = "in_progress".into();
        t.output = Some(agora_adapters::acp::bounded_output(&format!(
            "PASSWORD={}",
            &value[..20]
        )));
        t.arguments_json = Some(format!("{{\"command\":\"login --password {}", &value[..18]));
        b.push_item(&a, &ChatItem::Tool(t.clone()), 1_000);
        save_due(&b, 1_100);
        t.status = "completed".into();
        t.output = Some(agora_adapters::acp::bounded_output(&format!(
            "PASSWORD={value}\n"
        )));
        t.arguments_json = Some(format!("{{\"command\":\"login --password {value}\"}}"));
        b.push_item(&a, &ChatItem::Tool(t), 2_000);
        save_due(&b, 2_100);
        let f = body(&s, "a");
        eprintln!("outv2 file:\n{f}");
        assert_eq!(leaked(&f, value), None);
    }

    #[test]
    #[ignore = "a measurement (review of #1103, round 3): cargo test -- --ignored r3_perf"]
    fn r3_perf_long_stream() {
        let (_d, s, b) = board("perf", &[SECRET, "another-secret-value-xyz"]);
        let a = AgentId::new("a");
        let start = std::time::Instant::now();
        let mut t = 1_000u64;
        let mut written = 0usize;
        for i in 0..20_000u64 {
            t += 10;
            b.push_item(&a, &text(&format!("word{i} ")), t);
            if i % 100 == 0 {
                save_due(&b, t);
                written += 1;
            }
        }
        save_due(&b, t + 5_000);
        let el = start.elapsed();
        eprintln!(
            "perf: 20k pieces, {written} batches: {el:?}, file {} bytes",
            body(&s, "a").len()
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod review3perf {
    use super::*;
    use crate::credentials::{Secret, SecretStore};
    use crate::transcript::{Limits, Store};

    fn run(tag: &str, secrets: Vec<String>, piece: usize, n: u64, every: u64) {
        let dir = std::env::temp_dir().join(format!("agora-review3p-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (i, v) in secrets.iter().enumerate() {
            SecretStore::in_data(&dir)
                .set(&format!("T{i}"), &Secret::new(v).unwrap())
                .unwrap();
        }
        let s = Arc::new(Store::new(
            &dir,
            Limits::default(),
            Some(SecretStore::in_data(&dir)),
        ));
        let b = ChatBoard::saving(Arc::clone(&s));
        let a = AgentId::new("a");
        let chunk = "w".repeat(piece - 1) + " ";
        let mut t = 1_000u64;
        let mut take = std::time::Duration::ZERO;
        let mut write = std::time::Duration::ZERO;
        let start = std::time::Instant::now();
        for i in 0..n {
            t += 10;
            b.push_item(&a, &ChatItem::Text(chunk.clone()), t);
            if i % every == 0 {
                let t0 = std::time::Instant::now();
                let batches = b.take_due(t);
                take += t0.elapsed();
                let t1 = std::time::Instant::now();
                for x in batches {
                    s.write(&x).unwrap();
                }
                write += t1.elapsed();
            }
        }
        eprintln!(
            "{tag}: total {:?} take {:?} write {:?}",
            start.elapsed(),
            take,
            write
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "a measurement (review of #1103, round 3): cargo test -- --ignored r3p"]
    fn r3p() {
        run("none", vec![], 40, 20_000, 10);
        run(
            "small",
            vec!["sk-ant-api03-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789".into()],
            40,
            20_000,
            10,
        );
        run(
            "pem4k",
            vec!["Q".to_string() + &"abcdefghij".repeat(400)],
            40,
            20_000,
            10,
        );
        run(
            "pem8k",
            vec!["Q".to_string() + &"abcdefghij".repeat(800)],
            40,
            20_000,
            10,
        );
        run("aaaa8k", vec!["a".repeat(8_000)], 40, 5_000, 10);
        run(
            "tiny-pieces",
            vec!["sk-ant-api03-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789".into()],
            2,
            50_000,
            1,
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod review3b {
    use super::*;
    use crate::credentials::{Secret, SecretStore};
    use crate::transcript::{Limits, Store};

    /// Round 3, B2: the bridge's own shape check replaces only part of a value; the node finds the rest beside its mark.
    #[test]
    fn r3_bridge_redact_splits_value() {
        for (v, line) in [
            (
                "Tr0ub4dor&3-horse!battery#staple",
                "PASSWORD=Tr0ub4dor&3-horse!battery#staple\n",
            ),
            (
                "Tr0ub4dor&3-horse!battery#staple",
                "export DB_PASSWORD='Tr0ub4dor&3-horse!battery#staple'\n",
            ),
            (
                "ghp_AbCdEfGhIj0123456789xyzXYZ_abcdefgh",
                "token: ghp_AbCdEfGhIj0123456789xyzXYZ_abcdefgh\n",
            ),
            (
                "correct horse battery staple 42!",
                "password = correct horse battery staple 42!\n",
            ),
        ] {
            let out = agora_adapters::acp::bounded_output(line);
            let dir = std::env::temp_dir().join(format!(
                "agora-review3b-{}-{}",
                v.len(),
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            SecretStore::in_data(&dir)
                .set("T", &Secret::new(v).unwrap())
                .unwrap();
            let s = Arc::new(Store::new(
                &dir,
                Limits::default(),
                Some(SecretStore::in_data(&dir)),
            ));
            let b = ChatBoard::saving(Arc::clone(&s));
            let a = AgentId::new("a");
            let mut t = ToolItem {
                call_id: "c1".into(),
                ..ToolItem::default()
            };
            t.tool = Some("sh".into());
            t.status = "completed".into();
            t.output = Some(out.clone());
            b.push_item(&a, &ChatItem::Tool(t), 1);
            for batch in b.take_unsaved() {
                s.write(&batch).unwrap();
            }
            let f = std::fs::read_to_string(s.path("a").unwrap()).unwrap();
            for n in (4..v.len()).rev() {
                for i in 0..=v.len() - n {
                    if let Some(p) = v.get(i..i + n) {
                        assert!(!f.contains(p), "{line:?} -> {out:?}: {p:?} on disk");
                    }
                }
            }
        }
    }
}

/// Review of #1103, round 3: the acceptance gate for ADR-0089 decision 7. Seeded random cases through the real path (adapter items
/// into the board, the bridge's `bounded_output` for some tool outputs, the writer's timed and full takes into the file, restarts
/// that seed a new board from it), with secrets drawn from an alphabet the rest of the text never uses (upper case, punctuation,
/// quotes, backslashes, Turkish capitals; no digits or lower case), so any piece of a secret in the file came from the secret. The
/// check: no piece of 4 bytes or more of any secret, raw or as JSON escapes it, in any field of any line, nor in the agent's words
/// joined in order; and every case's own marker word reached the file (nothing is simply held back for ever).
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod property {
    use super::*;
    use crate::credentials::{Secret, SecretStore};
    use crate::transcript::{decode_line, Limits, Line as FileLine, Store};
    use std::collections::BTreeMap;

    /// The cases run (each a few dozen to a few hundred steps); `AGORA_PROPERTY_CASES` sets another number.
    const CASES: u64 = 600;

    /// How many cases reached each scenario (reported by the test).
    const SCENARIOS: [&str; N] = [
        "words in pieces",
        "a stop's flush mid-words",
        "a take-up mid-words",
        "a turn begins or ends mid-words",
        "Start again mid-words",
        "a tool call in versions",
        "an output over 64 KiB through the bridge",
        "a secret in a tool name or path",
        "a permission label",
        "a plan step",
        "a user prompt or a notice parameter with a secret",
        "a writer batch mid-words",
        "a stderr tail cut at its front (through the bridge)",
        "a notice parameter over 1 KiB",
        "a flood that drops words from the evicted buffer",
        "a kill, then a take-up (perhaps with no file)",
        "a text over 16 KiB or a thought over 8 KiB",
        "a legacy tool call, plan or diff",
        "a secret of digits",
        "a text cut at 16 KiB inside a value the next piece continues",
    ];
    const N: usize = 20;
    static HITS: [std::sync::atomic::AtomicUsize; N] =
        [const { std::sync::atomic::AtomicUsize::new(0) }; N];

    fn hit(seen: &mut [bool; N], i: usize) {
        if !seen[i] {
            seen[i] = true;
            HITS[i].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n.max(1) as u64) as usize
        }
        fn chance(&mut self, percent: usize) -> bool {
            self.below(100) < percent
        }
        fn pick(&mut self, of: &[char]) -> char {
            of[self.below(of.len())]
        }
    }

    const TOKEN: &[char] = &[
        'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'J', 'K', 'L', 'M', 'N', 'P', 'Q', 'R', 'S', 'T',
        'U', 'V', 'W', 'X', 'Y', 'Z', '_', '-',
    ];
    const PUNCT: &[char] = &[
        'A', 'B', 'C', 'X', 'Y', 'Z', '!', '#', '&', '%', '"', '\\', '-', '*', '@',
    ];
    const WIDE: &[char] = &['Ç', 'Ğ', 'İ', 'Ö', 'Ş', 'Ü', 'K', 'M', 'R'];
    const FILLER: &[char] = &[
        'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'k', 'l', 'm', 'n', 'o', 'p', 'r', 's', 't',
        'u', 'v', 'w', ' ', ' ', '.',
    ];

    const DIGITS: &[char] = &['0', '1', '2', '3', '4', '5', '6', '7', '8', '9'];

    fn secret(r: &mut Rng) -> String {
        let (alphabet, len) = match r.below(4) {
            0 => (TOKEN, 24 + r.below(30)),
            1 => (PUNCT, 10 + r.below(16)),
            2 => (WIDE, 8 + r.below(10)),
            _ => (DIGITS, 10 + r.below(10)),
        };
        (0..len).map(|_| r.pick(alphabet)).collect()
    }

    fn filler(r: &mut Rng, len: usize) -> String {
        (0..len).map(|_| r.pick(FILLER)).collect()
    }

    /// Splits `s` into pieces of 1 to `most` characters.
    fn pieces(r: &mut Rng, s: &str, most: usize) -> Vec<String> {
        let chars: Vec<char> = s.chars().collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            let n = 1 + r.below(most);
            out.push(chars[i..(i + n).min(chars.len())].iter().collect());
            i += n;
        }
        out
    }

    /// A text with secrets in it: filler, a secret, filler, ...
    fn message(r: &mut Rng, secrets: &[String]) -> String {
        let n = 1 + r.below(30);
        let mut m = filler(r, n);
        for _ in 0..1 + r.below(2) {
            m.push_str(&secrets[r.below(secrets.len())]);
            let n = r.below(25);
            m.push_str(&filler(r, n));
        }
        m
    }

    /// The shortest pieces of `v` of `min` bytes or more from each character (on character boundaries): any longer piece holds one,
    /// so finding none of these finds none at all.
    fn pieces_of(v: &str, min: usize) -> Vec<&str> {
        let bounds: Vec<usize> = v.char_indices().map(|(i, _)| i).chain([v.len()]).collect();
        let mut out = Vec::new();
        for (x, &a) in bounds.iter().enumerate() {
            if let Some(&z) = bounds[x + 1..].iter().find(|&&z| z - a >= min) {
                out.push(&v[a..z]);
            }
        }
        out
    }

    struct Case {
        dir: std::path::PathBuf,
        store: Arc<Store>,
        board: ChatBoard,
        /// #1147: every follower of every board of the case, and what it was sent.
        followers: Vec<Follower>,
    }

    /// #1147: one follower: its snapshot, then every version of every entry sent to it, and whether an id ever went back (a new id
    /// below one it already had).
    struct Follower {
        sub: ChatSubscription,
        got: Vec<ChatEntry>,
        high: u64,
        seen: BTreeSet<u64>,
        backwards: Option<String>,
    }

    impl Follower {
        fn drain(&mut self) {
            for e in self.sub.try_iter() {
                if e.entry_id <= self.high
                    && !self.seen.contains(&e.entry_id)
                    && self.backwards.is_none()
                {
                    self.backwards = Some(format!("id {} after {}", e.entry_id, self.high));
                }
                self.high = self.high.max(e.entry_id);
                self.seen.insert(e.entry_id);
                self.got.push(e);
            }
        }

        /// Every field of every version it got, and its agent's words joined by id.
        fn shown(&self) -> (Vec<String>, String) {
            let mut fields = Vec::new();
            let mut words: BTreeMap<u64, String> = BTreeMap::new();
            for e in &self.got {
                if in_run(e) {
                    words.insert(e.entry_id, e.text.clone());
                }
                fields.extend([e.text.clone(), e.output.clone(), e.path.clone()]);
                fields.extend(
                    e.params
                        .iter()
                        .filter(|(k, _)| *k != "bytes" && *k != "entries")
                        .map(|(_, v)| v.clone()),
                );
                fields.extend(e.paths.iter().map(|p| p.path.clone()));
                fields.extend(e.plan_steps.iter().map(|s| s.text.clone()));
            }
            (fields, words.into_values().collect())
        }
    }

    impl Case {
        /// A new follower of the board as it is now (its snapshot first).
        fn follow(&mut self, a: &AgentId) {
            let (got, sub) = self.board.follow(a);
            let seen: BTreeSet<u64> = got.iter().map(|e| e.entry_id).collect();
            let high = seen.last().copied().unwrap_or(0);
            self.followers.push(Follower {
                sub,
                got,
                high,
                seen,
                backwards: None,
            });
        }

        fn drain(&mut self) {
            for f in &mut self.followers {
                f.drain();
            }
        }

        fn write(&self, full: bool) {
            let batches = if full {
                self.board.take_unsaved()
            } else {
                self.board.take_due(0)
            };
            for b in batches {
                self.store.write(&b).unwrap();
            }
        }
    }

    /// What the files hold: every string of every line, and the agent's words joined in order (per file).
    fn on_disk(dir: &std::path::Path) -> (Vec<String>, Vec<String>, String) {
        let mut fields = Vec::new();
        let mut streams = Vec::new();
        let mut raw = String::new();
        for item in std::fs::read_dir(dir.join("transcripts"))
            .into_iter()
            .flatten()
            .flatten()
        {
            let text = std::fs::read_to_string(item.path()).unwrap_or_default();
            raw.push_str(&text);
            let mut words: BTreeMap<u64, String> = BTreeMap::new();
            for line in text.lines() {
                let Some((_, FileLine::Entry(e, side))) = decode_line(line) else {
                    continue;
                };
                if in_run(&e) {
                    words.insert(e.entry_id, e.text.clone());
                }
                fields.extend([e.text, e.output, e.path, e.approval_id, e.call_id]);
                // The node's own numbers (`bytes`, `entries`) are not the agent's text.
                fields.extend(
                    e.params
                        .into_iter()
                        .filter(|(k, _)| k != "bytes" && k != "entries")
                        .map(|(_, v)| v),
                );
                fields.extend(e.paths.into_iter().map(|p| p.path));
                fields.extend(e.plan_steps.into_iter().map(|s| s.text));
                if let Some((tool, args)) = side.tool {
                    fields.extend([tool, args]);
                }
            }
            streams.push(words.into_values().collect::<String>());
        }
        (fields, streams, raw)
    }

    fn run_case(seed: u64) -> Result<(), String> {
        let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let dir =
            std::env::temp_dir().join(format!("agora-property-{}-{seed}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let secrets: Vec<String> = (0..1 + r.below(3)).map(|_| secret(&mut r)).collect();
        for (i, v) in secrets.iter().enumerate() {
            SecretStore::in_data(&dir)
                .set(&format!("S{i}"), &Secret::new(v).unwrap())
                .unwrap();
        }
        let open = |dir: &std::path::Path| {
            let store = Arc::new(Store::new(
                dir,
                Limits::default(),
                Some(SecretStore::in_data(dir)),
            ));
            let board = ChatBoard::saving(Arc::clone(&store));
            (store, board)
        };
        let (store, board) = open(&dir);
        let mut c = Case {
            dir: dir.clone(),
            store,
            board,
            followers: Vec::new(),
        };
        let a = AgentId::new("agent-1");
        // #1147: what the live path does is drawn from a second generator, so the file's cases are the ones they always were.
        let mut live = Rng(seed.wrapping_mul(0xD1B5_4A32_D192_ED03) | 1);
        c.follow(&a);
        // A word only this case writes, in lower case, said last in a turn: it must reach the file once the turn ends (a Start again
        // replaces an older earlier session's file, so it is said at the end).
        let marker = format!("marker{}", filler(&mut r, 6).replace([' ', '.'], "q"));
        let mut calls = 0;
        let mut seen = [false; N];
        if secrets
            .iter()
            .any(|v| v.chars().all(|c| c.is_ascii_digit()))
        {
            hit(&mut seen, 18);
        }
        for _ in 0..10 + r.below(40) {
            match r.below(17) {
                // The agent's words, in pieces, with other entries, the writer, stops, take-ups, turns and Start again between them.
                0..=4 => {
                    hit(&mut seen, 0);
                    let m = message(&mut r, &secrets);
                    let most = [1, 3, 8, 20][r.below(4)];
                    for p in pieces(&mut r, &m, most) {
                        let item = if r.chance(20) {
                            ChatItem::Thought(p)
                        } else {
                            ChatItem::Text(p)
                        };
                        c.board.push_item(&a, &item, 1);
                        match r.below(24) {
                            0 | 1 => {
                                c.board.push_queued(&a, filler(&mut r, 12), 1);
                            }
                            2 => c.board.output_dropped(&a, 7, 1),
                            3..=6 => {
                                hit(&mut seen, 11);
                                c.write(false);
                            }
                            7 => {
                                c.board.push_user(&a, filler(&mut r, 9), 1);
                            }
                            8 => {
                                hit(&mut seen, 1);
                                c.write(true);
                            }
                            9 => {
                                hit(&mut seen, 2);
                                c.write(true);
                                let (store, board) = open(&dir);
                                board.seed_from_disk(&a, 1);
                                c.drain();
                                c.store = store;
                                c.board = board;
                                c.follow(&a);
                            }
                            10 | 11 => {
                                hit(&mut seen, 3);
                                c.board.set_working(&a, r.chance(50));
                            }
                            12 => {
                                hit(&mut seen, 4);
                                let seed = c.board.read_seed(&a, true);
                                c.board.resumed(&a, seed, true, 1);
                            }
                            13 => {
                                // Killed: nothing more is written; the next node takes the agent up from whatever file there is.
                                hit(&mut seen, 15);
                                let (store, board) = open(&dir);
                                board.seed_from_disk(&a, 1);
                                c.drain();
                                c.store = store;
                                c.board = board;
                                c.follow(&a);
                            }
                            14 if r.chance(1) => {
                                // A flood of other entries: words not written yet leave the ring and the evicted buffer.
                                hit(&mut seen, 14);
                                let big = filler(&mut r, MAX_OUTPUT_BYTES);
                                for i in 0..80 {
                                    let item = ToolItem {
                                        call_id: format!("flood{i}"),
                                        tool: Some("sh".into()),
                                        status: "completed".into(),
                                        output: Some(big.clone()),
                                        ..ToolItem::default()
                                    };
                                    c.board.push_item(&a, &ChatItem::Tool(item), 1);
                                }
                            }
                            _ => {}
                        }
                        if live.chance(10) {
                            c.board.flush_due(1 + LIVE_HOLD_MS);
                        }
                        if live.chance(3) {
                            c.follow(&a);
                        }
                        c.drain();
                    }
                }
                // A stderr tail the bridge cut at its front, or a notice parameter over 1 KiB, holding a secret.
                13 => {
                    let m = message(&mut r, &secrets);
                    let value = if r.chance(50) {
                        hit(&mut seen, 12);
                        let n = r.below(500);
                        let pre = filler(&mut r, n);
                        let n = 300 + r.below(200);
                        let post = filler(&mut r, n);
                        agora_adapters::acp::stderr_tail(format!("{pre}{m}{post}").as_bytes(), None)
                    } else {
                        hit(&mut seen, 13);
                        let n = MAX_PARAM_BYTES.saturating_sub(r.below(40));
                        format!("{}{m}", filler(&mut r, n))
                    };
                    c.board.push_item(
                        &a,
                        &ChatItem::Notice {
                            code: "adapter.turn_failed".into(),
                            params: vec![("stderr_tail".into(), value)],
                        },
                        1,
                    );
                }
                // A text over 16 KiB or a thought over 8 KiB, with a secret where the node cuts it.
                14 => {
                    hit(&mut seen, 16);
                    let m = message(&mut r, &secrets);
                    let (item, n) = if r.chance(50) {
                        (0, MAX_ENTRY_BYTES.saturating_sub(r.below(40)))
                    } else {
                        (1, MAX_THOUGHT_BYTES / 2 - r.below(40))
                    };
                    let pre = filler(&mut r, n);
                    if item == 0 && r.chance(50) {
                        // One chunk over 16 KiB that ends inside a value, and the next that goes on with it (review of #1103,
                        // round 5): the node cuts the first, and the value's end is in the second.
                        hit(&mut seen, 19);
                        let bounds: Vec<usize> = m.char_indices().map(|(i, _)| i).collect();
                        let k = bounds[r.below(bounds.len())];
                        c.board
                            .push_item(&a, &ChatItem::Text(format!("{pre}{}", &m[..k])), 1);
                        if r.chance(50) {
                            c.write(false);
                        }
                        let n = r.below(40);
                        let post = filler(&mut r, n);
                        c.board
                            .push_item(&a, &ChatItem::Text(format!("{}{post}", &m[k..])), 1);
                    } else {
                        let post = filler(&mut r, MAX_ENTRY_BYTES);
                        let text = format!("{pre}{m}{post}");
                        let item = if item == 0 {
                            ChatItem::Text(text)
                        } else {
                            ChatItem::Thought(text)
                        };
                        c.board.push_item(&a, &item, 1);
                    }
                }
                // The legacy items: a tool call without an id, a plan of lines, a diff.
                15 => {
                    hit(&mut seen, 17);
                    let m = message(&mut r, &secrets);
                    let item = match r.below(3) {
                        0 => ChatItem::ToolCall {
                            tool: "sh".into(),
                            arguments_json: serde_json::to_string(&m).unwrap(),
                        },
                        1 => ChatItem::Plan(vec![m.clone(), filler(&mut r, 20)]),
                        _ => {
                            let n = MAX_ENTRY_BYTES.saturating_sub(r.below(40));
                            ChatItem::Diff {
                                path: "src/a.rs".into(),
                                unified_diff: format!("{}{m}", filler(&mut r, n)),
                            }
                        }
                    };
                    c.board.push_item(&a, &item, 1);
                }
                // A user prompt or an adapter's notice holding a secret.
                12 | 16 => {
                    hit(&mut seen, 10);
                    if r.chance(50) {
                        let m = message(&mut r, &secrets);
                        c.board.push_user(&a, m, 1);
                    } else {
                        let m = message(&mut r, &secrets);
                        c.board.push_item(
                            &a,
                            &ChatItem::Notice {
                                code: "adapter.turn_failed".into(),
                                params: vec![("stderr_tail".into(), m)],
                            },
                            1,
                        );
                    }
                }
                // A tool call in versions: the output and arguments growing, then final; some through the bridge, some long.
                5 | 6 => {
                    hit(&mut seen, 5);
                    calls += 1;
                    let id = format!("c{calls}");
                    let mut out = message(&mut r, &secrets);
                    let mut big = false;
                    if r.chance(15) {
                        let n = 9_000 + r.below(9_000);
                        let pad = filler(&mut r, n);
                        out = format!("{pad}{out}{pad}");
                    } else if r.chance(5) {
                        // Over the bridge's 64 KiB, with a secret where its cut falls.
                        big = true;
                        let pad = filler(&mut r, 32 * 1024 - 7);
                        let more = filler(&mut r, 2_000);
                        out = format!("{pad}{out}{pad}{more}");
                    }
                    let named = r.chance(15);
                    if named {
                        hit(&mut seen, 7);
                    }
                    let tool = if named {
                        message(&mut r, &secrets)
                    } else {
                        "sh".to_string()
                    };
                    let path = if named {
                        Some(vec![(message(&mut r, &secrets), 1)])
                    } else {
                        None
                    };
                    let args = format!(
                        "{{\"cmd\":{}}}",
                        serde_json::to_string(&message(&mut r, &secrets)).unwrap()
                    );
                    let bridge = big || r.chance(40);
                    if big {
                        hit(&mut seen, 6);
                    }
                    let versions = 1 + r.below(4);
                    for v in 1..=versions {
                        let last = v == versions;
                        let cut = |s: &str, r: &mut Rng| {
                            if last {
                                return s.to_string();
                            }
                            let mut n = r.below(s.len() + 1);
                            while !s.is_char_boundary(n) {
                                n -= 1;
                            }
                            s[..n].to_string()
                        };
                        let o = cut(&out, &mut r);
                        let o = if bridge {
                            agora_adapters::acp::bounded_output(&o)
                        } else {
                            o
                        };
                        let item = ToolItem {
                            call_id: id.clone(),
                            tool: Some(tool.clone()),
                            locations: path.clone(),
                            arguments_json: Some(cut(&args, &mut r)),
                            status: if last { "completed" } else { "in_progress" }.into(),
                            output: Some(o),
                            ..ToolItem::default()
                        };
                        c.board.push_item(&a, &ChatItem::Tool(item), 1);
                        if r.chance(50) {
                            c.write(false);
                        }
                    }
                }
                // A permission whose label holds a secret, sometimes where the node cuts it.
                7 => {
                    hit(&mut seen, 8);
                    let mut label = message(&mut r, &secrets);
                    if r.chance(50) {
                        label = format!("{}{label}{}", filler(&mut r, 240), filler(&mut r, 400));
                    }
                    c.board
                        .push_permission(&a, &format!("ap{}", r.below(1000)), &label, 1);
                }
                // A plan whose step holds a secret.
                8 => {
                    hit(&mut seen, 9);
                    let step = PlanItem {
                        text: message(&mut r, &secrets),
                        ..PlanItem::default()
                    };
                    c.board.push_item(&a, &ChatItem::PlanSteps(vec![step]), 1);
                }
                // The agent's turn begins or ends.
                9 => {
                    hit(&mut seen, 3);
                    c.board.set_working(&a, r.chance(60));
                }
                // The writer, or a stop's full flush.
                10 => c.write(r.chance(20)),
                // A restart: the node stops (its flush), and the next one takes the agent up from the file.
                _ => {
                    c.write(true);
                    let (store, board) = open(&dir);
                    board.seed_from_disk(&a, 1);
                    c.drain();
                    c.store = store;
                    c.board = board;
                    c.follow(&a);
                }
            }
            // #1147: a window that subscribes now (a snapshot mid-stream), the node's tick (the hold's deadline, or not yet), and
            // every follower reading what it was sent.
            if live.chance(15) {
                c.follow(&a);
            }
            if live.chance(30) {
                c.board
                    .flush_due(if live.chance(50) { 1 } else { 1 + LIVE_HOLD_MS });
            }
            c.drain();
        }
        c.board.set_working(&a, true);
        c.board
            .push_item(&a, &ChatItem::Text(format!(" {marker}")), 1);
        c.board.set_working(&a, false);
        // #1147: a turn that ended leaves nothing held: its last words reached the follower that followed all along.
        c.drain();
        let newest = c.followers.last().map(Follower::shown).unwrap_or_default();
        if !newest.1.contains(&marker) {
            return Err(format!(
                "seed {seed}: the words of a turn that ended did not reach a follower"
            ));
        }
        let mut late = c.board.snapshot(&a);
        c.follow(&a);
        c.drain();
        if let Some(f) = c.followers.iter().find(|f| f.backwards.is_some()) {
            return Err(format!(
                "seed {seed}: a follower's ids went back: {}",
                f.backwards.as_deref().unwrap_or_default()
            ));
        }
        c.write(false);
        let (_, _, before_stop) = on_disk(&c.dir);
        c.write(true);
        let (fields, streams, raw) = on_disk(&c.dir);
        if !before_stop.contains(&marker) {
            return Err(format!(
                "seed {seed}: the words of a turn that ended were not written before a stop"
            ));
        }
        let _ = std::fs::remove_dir_all(&c.dir);
        if !raw.contains(&marker) {
            return Err(format!(
                "seed {seed}: the case's own words never reached the file"
            ));
        }
        // #1147: what every follower was sent (every version, its words joined), and the snapshot `snapshot` gives.
        let sent: Vec<(Vec<String>, String)> = c.followers.iter().map(Follower::shown).collect();
        let snapshot: Vec<String> = late
            .drain(..)
            .flat_map(|e| {
                let mut f = vec![e.text, e.output, e.path];
                f.extend(
                    e.params
                        .into_iter()
                        .filter(|(k, _)| k != "bytes" && k != "entries")
                        .map(|(_, v)| v),
                );
                f.extend(e.paths.into_iter().map(|p| p.path));
                f.extend(e.plan_steps.into_iter().map(|s| s.text));
                f
            })
            .collect();
        for v in &secrets {
            let escaped = serde_json::to_string(v).unwrap();
            let escaped = escaped[1..escaped.len() - 1].to_string();
            for form in [v.as_str(), escaped.as_str()] {
                for piece in pieces_of(form, 4) {
                    for (i, (fields, words)) in sent.iter().enumerate() {
                        if let Some(f) = fields.iter().find(|f| f.contains(piece)) {
                            return Err(format!(
                                "seed {seed}: {piece:?} of {v:?} sent to follower {i}: {f:?}"
                            ));
                        }
                        if words.contains(piece) {
                            return Err(format!(
                                "seed {seed}: {piece:?} of {v:?} in follower {i}'s joined words: {words:?}"
                            ));
                        }
                    }
                    if let Some(f) = snapshot.iter().find(|f| f.contains(piece)) {
                        return Err(format!(
                            "seed {seed}: {piece:?} of {v:?} in a snapshot: {f:?}"
                        ));
                    }
                    if let Some(f) = fields.iter().find(|f| f.contains(piece)) {
                        return Err(format!("seed {seed}: {piece:?} of {v:?} in a field: {f:?}"));
                    }
                    if let Some(w) = streams.iter().find(|w| w.contains(piece)) {
                        return Err(format!(
                            "seed {seed}: {piece:?} of {v:?} in the joined words: {w:?}"
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    #[test]
    fn no_piece_of_a_secret_reaches_the_file_or_a_follower() {
        let cases = std::env::var("AGORA_PROPERTY_CASES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(CASES);
        // `AGORA_PROPERTY_BASE` starts the seeds elsewhere (another set of cases).
        let base: u64 = std::env::var("AGORA_PROPERTY_BASE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let failed: Vec<String> = (base + 1..=base + cases)
            .filter_map(|seed| run_case(seed).err())
            .collect();
        for (name, n) in SCENARIOS.iter().zip(&HITS) {
            eprintln!(
                "property: {name}: {} cases",
                n.load(std::sync::atomic::Ordering::Relaxed)
            );
        }
        assert!(
            failed.is_empty(),
            "{} of {cases} cases failed; first: {}",
            failed.len(),
            failed.first().map_or("", String::as_str)
        );
    }
}

/// Review of #1103, rounds 3 and 4: every cut of an agent's text says where it cut (`agora_adapters::text::cap_end`, `cap_middle`,
/// and `tidy_cuts`, which record the place for the transcript writer), or is listed here with why its place needs no record. A line
/// cuts when it truncates, drains or splits (`.truncate(`, `.drain(`, `.split_off(`, `.split_at(`, `.replace_range(`), takes a range
/// with or without brackets (`[a..b]`, `.get(a..b)`), or names a cutting helper (called or passed by
/// name). A new one in the node's chat code, its transcript writer, the bridge or the cut helpers fails this test until it is
/// recorded or listed.
#[cfg(test)]
mod every_cut {
    const HELPERS: &[&str] = &[
        "cut",
        "cap_end",
        "cap_middle",
        "tidy",
        "head_tail",
        "cut_kept",
        "clip",
        "short",
        "tail_of",
    ];

    fn code(src: &str) -> &str {
        src.split("\n#[cfg(test)]").next().unwrap_or(src)
    }

    fn word(c: Option<char>) -> bool {
        c.is_some_and(|c| c.is_alphanumeric() || c == '_')
    }

    /// Whether a line (trimmed, not a comment) cuts text.
    pub(super) fn cutting(line: &str) -> bool {
        const DROPPING: &[&str] = &[
            ".truncate(",
            ".split_off(",
            ".split_at(",
            ".drain(",
            ".replace_range(",
        ];
        if DROPPING.iter().any(|d| line.contains(d)) {
            return true;
        }
        // `.get(a..b)`: a range taken without brackets.
        for (i, m) in line.match_indices(".get(") {
            let mut depth = 0usize;
            for (j, c) in line[i + m.len()..].char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' if depth == 0 => {
                        if line[i + m.len()..i + m.len() + j].contains("..") {
                            return true;
                        }
                        break;
                    }
                    ')' => depth -= 1,
                    _ => {}
                }
            }
        }
        let mut open = None;
        for (i, c) in line.char_indices() {
            match c {
                '[' => open = Some(i),
                // A `]` closes the `[` before it either way.
                ']' if open.take().is_some_and(|o| line[o..i].contains("..")) => return true,
                _ => {}
            }
        }
        HELPERS.iter().any(|h| {
            line.match_indices(h).any(|(i, m)| {
                !word(line[..i].chars().next_back()) && !word(line[i + m.len()..].chars().next())
            })
        })
    }

    fn check(name: &str, src: &str, allowed: &[(&str, &str)]) {
        let unlisted: Vec<&str> = code(src)
            .lines()
            .map(str::trim)
            .filter(|l| !l.starts_with("//") && cutting(l))
            .filter(|l| !allowed.iter().any(|(a, _)| a == l))
            .collect();
        assert!(
            unlisted.is_empty(),
            "{name}: cuts that do not say where they cut: {unlisted:#?}"
        );
    }

    #[test]
    fn the_predicate_finds_a_truncation_a_slice_and_a_helper_passed_by_name() {
        assert!(cutting("s.truncate(3);"));
        assert!(cutting("let x = s[..3].to_string();"));
        assert!(cutting("let x = &tail[start..];"));
        assert!(cutting(".map(short)"));
        assert!(cutting("let x = s.get(..3);"));
        assert!(cutting("let x = b.get(e - k..e) == m.get(f(1)..);"));
        assert!(cutting("let rest = s.split_off(3);"));
        assert!(cutting("let (a, _) = s.split_at(3);"));
        assert!(cutting("s.drain(..3);"));
        assert!(!cutting("let x = map.get(&key);"));
        assert!(!cutting("let x = insert_cut(a);"));
        assert!(!cutting("let y = v[3];"));
    }

    #[test]
    fn every_cut_of_agent_text_says_where_it_cut_or_is_listed() {
        check(
            "chat.rs",
            include_str!("chat.rs"),
            &[
                ("use agora_adapters::text::{balanced, cap_end, cap_middle, strip_core_at, Style};", "an import"),
                ("let params: Vec<(String, String)> = e.params.drain().collect();", "a map's whole items moved out: nothing of a text is dropped"),
                ("fn cut(text: String) -> String {", "only texts the writer makes again from redacted parts (a call's, a plan's) and seeding"),
                ("cap_end(text, MAX_ENTRY_BYTES).0", "only texts the writer makes again from redacted parts (a call's, a plan's) and seeding"),
                ("cap_end(text, MAX_ENTRY_BYTES)", "the place is recorded"),
                ("cut(format!(", "only texts the writer makes again from redacted parts (a call's, a plan's) and seeding"),
                ("cut(steps", "only texts the writer makes again from redacted parts (a call's, a plan's) and seeding"),
                ("let (v, end) = cap_end(v.clone(), MAX_PARAM_BYTES);", "the place is recorded"),
                ("fn tidy(text: &str, max: usize, style: Style) -> (String, bool) {", "the seeding helper"),
                ("let (out, cut, _) = tidy_cuts(text, max, style);", "the place is recorded"),
                ("(out, cut)", "the seeding helper"),
                ("let (bounded, first) = cap_middle(text.to_string(), PRE_BOUND);", "the place is recorded"),
                ("let (head, at) = strip_core_at(&bounded[..h], style, &places[..split]);", "the place is recorded"),
                ("let tail_places: Vec<usize> = places[split..].iter().map(|p| p - h).collect();", "ADR-0111: a list of places, not a text"),
                ("out.extend(name[k..].iter().map(|_| HEAD + 1 + escaped));", "ADR-0111: a list of places, not a text"),
                ("let (tail, at) = strip_core_at(&bounded[h..], style, &tail_places);", "the place is recorded"),
                ("let (out, last) = cap_middle(cleaned, max);", "the place is recorded"),
                ("joined.push_str(&balanced(out[at..seam].to_string(), style));", "the place is recorded"),
                ("self.carry = tail_of(&written, agora_adapters::text::MAX_SECRET_SPAN);", "the carry: kept in memory, never written"),
                ("fn tail_of(s: &str, keep: usize) -> String {", "the carry: kept in memory, never written"),
                ("s[from..].to_string()", "the carry: kept in memory, never written"),
                ("ChatKind::Thought => tidy(&e.text, MAX_THOUGHT_BYTES, Style::Prose).0,", "seeding: the file's text, redacted already (or a list: whole items go)"),
                ("ChatKind::Permission => tidy(&e.text, MAX_PERMISSION_BYTES, Style::Label).0,", "seeding: the file's text, redacted already (or a list: whole items go)"),
                ("_ => cut(std::mem::take(&mut e.text)),", "seeding: the file's text, redacted already (or a list: whole items go)"),
                ("e.path = tidy(&e.path, MAX_PATH_BYTES, Style::Label).0;", "seeding: the file's text, redacted already (or a list: whole items go)"),
                ("let (output, cut_now) = tidy(&e.output, MAX_OUTPUT_BYTES, Style::Prose);", "seeding: the file's text, redacted already (or a list: whole items go)"),
                ("e.paths.truncate(MAX_PATHS);", "seeding: the file's text, redacted already (or a list: whole items go)"),
                ("p.path = tidy(&p.path, MAX_PATH_BYTES, Style::Label).0;", "seeding: the file's text, redacted already (or a list: whole items go)"),
                ("e.plan_steps.truncate(MAX_PLAN_STEPS);", "seeding: the file's text, redacted already (or a list: whole items go)"),
                ("st.text = tidy(&st.text, MAX_PLAN_STEP_BYTES, Style::Step).0;", "seeding: the file's text, redacted already (or a list: whole items go)"),
                ("e.approval_id = cap_end(std::mem::take(&mut e.approval_id), MAX_CALL_ID).0;", "seeding: the file's text, redacted already (or a list: whole items go)"),
                ("tidy(&tool, MAX_PATH_BYTES, Style::Label).0,", "seeding: the file's text, redacted already (or a list: whole items go)"),
                ("tidy(&args, MAX_ENTRY_BYTES, Style::Code).0,", "seeding: the file's text, redacted already (or a list: whole items go)"),
                ("None => vec![(0..=u64::MAX, None, t.next_id)],", "a range of entry ids"),
                ("t.carry = tail_of(&run.text[..written], reach);", "the carry: kept in memory, never written"),
                ("self.live.carry = tail_of(&run.text[..written], reach);", "#1147: the live carry: kept in memory, never sent"),
            ],
        );
        check(
            "acp.rs",
            include_str!("../../agora-adapters/src/acp.rs"),
            &[
                ("fn short(v: &Value) -> String {", "an approval's detail, not chat text"),
                ("let (head, tail) = rest.split_at(i + 3);", "the bridge's shape check: copies its text and drops nothing (its marks are edges)"),
                ("k.buf.drain(..over);", "stderr bytes collected, the oldest dropped: what stays begins the tail, and a notice parameter's start is an edge"),
                ("let mut delivering: Vec<(u64, Delivering)> = self.delivering.drain().collect();", "a map's whole items moved out: nothing of a text is dropped"),
                ("let mut waiting: Vec<(u64, Waiting)> = self.waiting.drain().collect();", "a map's whole items moved out: nothing of a text is dropped"),
                ("format!(\"{}…\", &s[..end])", "an approval's detail, not chat text"),
                ("fn clip(s: &str, max: usize) -> String {", "ids, names and words, not chat text"),
                ("s[..end].to_string()", "ids, names and words, not chat text"),
                ("|o: &Value, k: &str, max: usize| clip(o.get(k).and_then(Value::as_str).unwrap_or(\"\"), max);", "ids, names and words, not chat text"),
                (".map(|c| clip(c, 40));", "ids, names and words, not chat text"),
                ("v[\"current\"] = json!(clip(", "ids, names and words, not chat text"),
                ("cut_kept(redact(&clean), OUTPUT_MAX)", "ADR-0111: the place is reported (cleaned, and redacted, first: the node does not keep it either)"),
                ("cut_kept(", "ADR-0111: the place is reported (cleaned, and redacted, first: the node does not keep it either)"),
                ("fn cut_kept(text: String, max: usize) -> (String, Option<usize>) {", "ADR-0111: the place is reported (cleaned, and redacted, first: the node does not keep it either)"),
                ("let (kept, place) = crate::text::cap_middle(text, max);", "ADR-0111: the place is reported (cleaned, and redacted, first: the node does not keep it either)"),
                ("let (kept, place) = crate::text::cap_middle(std::mem::take(s), max);", "`fit_line`'s halving: the place is reported (ADR-0111)"),
                ("a.truncate(keep);", "`fit_line`'s halving: whole elements of a list dropped, with the places in them (ADR-0111)"),
                ("at.truncate(base);", "ADR-0111: a JSON pointer being built, not a text"),
                ("match tail[..end].rfind('@') {", "the bridge's shape check: copies its text and drops nothing (its marks are edges)"),
                ("out.push_str(&tail[at..end]);", "the bridge's shape check: copies its text and drops nothing (its marks are edges)"),
                ("None => out.push_str(&tail[..end]),", "the bridge's shape check: copies its text and drops nothing (its marks are edges)"),
                ("rest = &tail[end..];", "the bridge's shape check: copies its text and drops nothing (its marks are edges)"),
                ("rest = &rest[c.len_utf8()..];", "the bridge's shape check: copies its text and drops nothing (its marks are edges)"),
                ("let run = &rest[..end];", "the bridge's shape check: copies its text and drops nothing (its marks are edges)"),
                ("rest = &rest[end..];", "the bridge's shape check: copies its text and drops nothing (its marks are edges)"),
                ("let word = &rest[..end];", "the bridge's shape check: copies its text and drops nothing (its marks are edges)"),
                ("(format!(\"…{}\", &tail[start..]), moved)", "a stderr tail's front cut: reported (ADR-0111), and the node counts the place after `…` as an edge"),
                ("let seg = &rest[..sep];", "the bridge's shape check: copies its text and drops nothing (its marks are edges)"),
                ("let at = &lower[i..];", "the stderr tail's home path, spliced to `~`: a residual (T61)"),
                ("&& (at[1..].starts_with(\":\\\\users\\\\\") || at[1..].starts_with(\":/users/\"));", "the stderr tail's home path, spliced to `~`: a residual (T61)"),
                ("&& at[2..].starts_with(\"/users/\");", "the stderr tail's home path, spliced to `~`: a residual (T61)"),
                ("let name = user_name_len(&line[i + len..], spaces);", "the stderr tail's home path, spliced to `~`: a residual (T61)"),
                ("let c = line[i..].chars().next().unwrap_or(' ');", "the stderr tail's home path, spliced to `~`: a residual (T61)"),
                ("let digits: String = line[at + key.len()..]", "reading a log line's status"),
                ("home[..1].to_ascii_lowercase(),", "the stderr tail's home path, spliced to `~`: a residual (T61)"),
                ("home[2..].replace('\\\\', \"/\")", "the stderr tail's home path, spliced to `~`: a residual (T61)"),
                ("&& t[i..i + n.len()].eq_ignore_ascii_case(n)", "the stderr tail's home path, spliced to `~`: a residual (T61)"),
                ("let step = text[i..].chars().next().map_or(1, char::len_utf8);", "the stderr tail's home path, spliced to `~`: a residual (T61)"),
                ("out.push_str(&text[i..i + step]);", "the stderr tail's home path, spliced to `~`: a residual (T61)"),
                ("&self.buf[self.buf.len().saturating_sub(new)..]", "stderr bytes being collected, not chat text"),
                ("let _ = io::stderr().write_all(&buf[..n]);", "stderr bytes being collected, not chat text"),
                ("k.buf.extend_from_slice(&buf[..n]);", "stderr bytes being collected, not chat text"),
                ("state.model = clip(o[\"current\"].as_str().unwrap_or(\"\"), MODEL_MAX);", "ids, names and words, not chat text"),
                ("\"name\": clip(name, OPTION_ID_MAX),", "ids, names and words, not chat text"),
                ("\"description\": clip(c.get(\"description\").and_then(Value::as_str).unwrap_or(\"\"), OPTION_DESCRIPTION_MAX),", "ids, names and words, not chat text"),
                ("\"inputHint\": clip(c.pointer(\"/input/hint\").and_then(Value::as_str).unwrap_or(\"\"), OPTION_NAME_MAX),", "ids, names and words, not chat text"),
                ("\"status\": clip(e.get(\"status\").and_then(Value::as_str).unwrap_or(\"\"), 40),", "ids, names and words, not chat text"),
                ("\"priority\": clip(e.get(\"priority\").and_then(Value::as_str).unwrap_or(\"\"), 40)}));", "ids, names and words, not chat text"),
                ("let (path, place) = cut_kept(", "ADR-0111: the place is reported (cleaned, and redacted, first: the node does not keep it either)"),
                ("let (diff, place) = cut_kept(replace_diff(path, old, new), DIFF_MAX);", "ADR-0111: the place is reported (the node keeps a diff's first 16 KiB only, so not this place either)"),
                ("let detail = call.get(\"rawInput\").map(short).unwrap_or_default();", "an approval's detail, not chat text"),
                ("state.model = clip(m, MODEL_MAX);", "ids, names and words, not chat text"),
                ("state.model = clip(&id, MODEL_MAX);", "ids, names and words, not chat text"),
            ],
        );
        check(
            "transcript.rs",
            include_str!("transcript.rs"),
            &[
                (
                    ".push(n.sam.run((0, 0), b.get(from..).unwrap_or_default()));",
                    "#1147: only read, to find the part of a value the stream ends in; nothing is kept or sent",
                ),
                (
                    "let by = self.by.get(a..z)?;",
                    "the matcher's own table of moves, not a text",
                ),
                (
                    ".find(|&k| e >= k && b.get(e - k..e) == m.get(..k))",
                    "bytes beside an edge compared with a bridge mark's: nothing is cut",
                ),
                (
                    ".find(|&k| b.get(e..e + k) == m.get(m.len() - k..))",
                    "bytes beside an edge compared with a bridge mark's: nothing is cut",
                ),
                (
                    "let e = e.get(1..e.len() - 1)?.to_string();",
                    "a secret value's JSON form without its quotes: a needle, not written",
                ),
                (
                    "while let Some(i) = s[from..].find(head) {",
                    "searching a text for a value's first bytes: nothing of a text is dropped",
                ),
                (
                    "let head = k.get(..lead).unwrap_or(k);",
                    "#1147: a value's first bytes, only searched for: nothing kept or sent",
                ),
                (
                    "if s.as_bytes().get(at..at + k.len()) == Some(k.as_bytes()) {",
                    "#1147: only compared with the value: nothing kept or sent",
                ),
                (
                    "from = at + s[at..].chars().next().map_or(1, char::len_utf8);",
                    "copying whole texts or reading the file's tail: nothing of a text is dropped",
                ),
                (
                    "out.push_str(&s[at..start]);",
                    "copying whole texts or reading the file's tail: nothing of a text is dropped",
                ),
                (
                    "out.push_str(&s[at..]);",
                    "copying whole texts or reading the file's tail: nothing of a text is dropped",
                ),
                (
                    "out.extend_from_slice(&tail[from..]);",
                    "copying whole texts or reading the file's tail: nothing of a text is dropped",
                ),
                (
                    "Ok(fold(&String::from_utf8_lossy(&bytes[from..])))",
                    "copying whole texts or reading the file's tail: nothing of a text is dropped",
                ),
            ],
        );
        check(
            "text.rs",
            include_str!("../../agora-adapters/src/text.rs"),
            &[
                ("pub fn cap_end(mut text: String, max: usize) -> (String, Option<usize>) {", "the cut helpers themselves"),
                ("text.truncate(end);", "the cut helpers themselves"),
                ("pub fn cap_middle(text: String, max: usize) -> (String, Option<(usize, usize)>) {", "the cut helpers themselves"),
                ("format!(\"{}{}\", &text[..head], &text[tail..]),", "the cut helpers themselves"),
            ],
        );
    }
}

/// Review of #1103, round 4: the audit's probes, as regression tests.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod review4 {
    use super::*;
    use crate::credentials::{Secret, SecretStore};
    use crate::transcript::{Limits, Store};

    fn board(tag: &str, secret: &str) -> (std::path::PathBuf, Arc<Store>, ChatBoard) {
        let dir = std::env::temp_dir().join(format!("agora-review4-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        SecretStore::in_data(&dir)
            .set("T", &Secret::new(secret).unwrap())
            .unwrap();
        let s = Arc::new(Store::new(
            &dir,
            Limits::default(),
            Some(SecretStore::in_data(&dir)),
        ));
        (dir, Arc::clone(&s), ChatBoard::saving(s))
    }

    fn write(s: &Store, b: &ChatBoard, full: bool) {
        let batches = if full {
            b.take_unsaved()
        } else {
            b.take_due(0)
        };
        for x in batches {
            s.write(&x).unwrap();
        }
    }

    /// No piece of `v` of 4 bytes or more in `f`.
    fn clean_of(f: &str, v: &str) -> Result<(), String> {
        let bounds: Vec<usize> = v.char_indices().map(|(i, _)| i).chain([v.len()]).collect();
        for (x, &a) in bounds.iter().enumerate() {
            if let Some(&z) = bounds[x + 1..].iter().find(|&&z| z - a >= 4) {
                if f.contains(&v[a..z]) {
                    return Err(format!("{:?} on disk", &v[a..z]));
                }
            }
        }
        Ok(())
    }

    /// B1: the bridge's stderr tail starts with `…` where it cut; that place is an edge.
    #[test]
    fn a_value_cut_at_the_front_of_a_stderr_tail_is_masked() {
        let v = "hunter2!hunter2!xy";
        let (dir, s, b) = board("stderr", v);
        let raw = format!("Error: {}{v}{}", "ab ".repeat(100), "cd ".repeat(130));
        let tail = agora_adapters::acp::stderr_tail(raw.as_bytes(), None);
        let a = AgentId::new("a");
        b.push_item(
            &a,
            &ChatItem::Notice {
                code: "adapter.turn_failed".into(),
                params: vec![("stderr_tail".into(), tail)],
            },
            1,
        );
        write(&s, &b, true);
        let f = std::fs::read_to_string(s.path("a").unwrap()).unwrap();
        let _ = std::fs::remove_dir_all(dir);
        clean_of(&f, v).unwrap();
    }

    /// B2: a take-up with no file still makes the start of the next words an edge.
    #[test]
    fn a_take_up_with_no_file_makes_the_next_words_start_an_edge() {
        let v = "hunter2!hunter2!xy";
        let (dir, s, b) = board("nofile", v);
        let a = AgentId::new("a");
        b.set_working(&a, true);
        b.push_item(&a, &ChatItem::Text("hello hunter2!hun".into()), 1);
        drop(b); // killed: nothing written
        let b = ChatBoard::saving(Arc::clone(&s));
        assert!(!b.seed_from_disk(&a, 2));
        b.push_item(&a, &ChatItem::Text("ter2!xy and the rest of it".into()), 3);
        write(&s, &b, false);
        let f = std::fs::read_to_string(s.path("a").unwrap()).unwrap();
        let _ = std::fs::remove_dir_all(dir);
        clean_of(&f, v).unwrap();
    }

    /// A notice parameter cut at 1 KiB is an edge there, also for one made of digits.
    #[test]
    fn a_digits_parameter_cut_at_its_cap_is_masked() {
        let v = "9876543210123456";
        let (dir, s, b) = board("digits", v);
        let a = AgentId::new("a");
        let value = format!("{}{v}", "0".repeat(MAX_PARAM_BYTES - 8));
        b.push_item(
            &a,
            &ChatItem::Notice {
                code: "adapter.turn_failed".into(),
                params: vec![("rpc_code".into(), value)],
            },
            1,
        );
        write(&s, &b, true);
        let f = std::fs::read_to_string(s.path("a").unwrap()).unwrap();
        let _ = std::fs::remove_dir_all(dir);
        clean_of(&f, v).unwrap();
    }

    /// Words dropped from the evicted buffer are never written: the next words start with an edge.
    #[test]
    fn words_after_a_gap_in_the_evicted_buffer_start_with_an_edge() {
        let v = "hunter2!hunter2!xy";
        let (dir, s, b) = board("evicted", v);
        let a = AgentId::new("a");
        b.set_working(&a, true);
        b.push_item(&a, &ChatItem::Text("x hunter2!h".into()), 1);
        // A flood of other entries (they do not end the agent's words): far more than the ring and the evicted buffer hold, before
        // any batch, so the words before are dropped unwritten.
        let big = "z".repeat(MAX_OUTPUT_BYTES);
        for i in 0..(EVICTED_MAX / MAX_OUTPUT_BYTES + MAX_BYTES / MAX_OUTPUT_BYTES + 8) {
            let item = ToolItem {
                call_id: format!("f{i}"),
                tool: Some("sh".into()),
                status: "completed".into(),
                output: Some(big.clone()),
                ..ToolItem::default()
            };
            b.push_item(&a, &ChatItem::Tool(item), 2);
        }
        b.push_item(&a, &ChatItem::Text("unter2!xy rest".into()), 3);
        b.set_working(&a, false);
        write(&s, &b, false);
        write(&s, &b, true);
        let f = std::fs::read_to_string(s.path("a").unwrap()).unwrap();
        let _ = std::fs::remove_dir_all(dir);
        clean_of(&f, v).unwrap();
    }

    /// Property seed 1003545: a cut (the bridge's, at a stderr tail's front, or the node's) through a bridge mark that replaced part of
    /// a value leaves a piece of the mark between the edge and the rest of the value; the mark's far side is an edge too.
    #[test]
    fn a_value_beside_a_bridge_mark_that_a_cut_went_through_is_masked() {
        let v = "hunter2!hunter2!xy";
        let (dir, s, b) = board("cutmark", v);
        let a = AgentId::new("a");
        let front = format!("…ted]{} and more", &v[2..]);
        let back = format!("and more {}[reda…", &v[..16]);
        b.push_item(
            &a,
            &ChatItem::Notice {
                code: "adapter.turn_failed".into(),
                params: vec![("stderr_tail".into(), front), ("detail".into(), back)],
            },
            1,
        );
        write(&s, &b, true);
        let f = std::fs::read_to_string(s.path("a").unwrap()).unwrap();
        let _ = std::fs::remove_dir_all(dir);
        assert!(f.contains("and more"), "{f}");
        clean_of(&f, v).unwrap();
    }

    /// Round 5: a text cut at 16 KiB inside a value, whose dropped end the next piece continues. The cut is a place in the stream,
    /// in or out of a turn, with or without a batch between the two pieces.
    #[test]
    fn the_piece_after_a_text_cut_at_16_kib_starts_at_an_edge() {
        let v = "hunter2!hunter2!xy";
        for (n, (working, between)) in [(false, false), (false, true), (true, false), (true, true)]
            .into_iter()
            .enumerate()
        {
            let (dir, s, b) = board(&format!("cut16-{n}"), v);
            let a = AgentId::new("a");
            b.set_working(&a, working);
            b.push_item(
                &a,
                &ChatItem::Text(format!("{}hunter2!h", "a ".repeat(8191))),
                1,
            );
            if between {
                write(&s, &b, false);
            }
            b.push_item(&a, &ChatItem::Text("unter2!xy and the rest".into()), 2);
            b.set_working(&a, false);
            write(&s, &b, false);
            write(&s, &b, true);
            let f = std::fs::read_to_string(s.path("a").unwrap()).unwrap();
            let _ = std::fs::remove_dir_all(dir);
            assert!(f.contains("and the rest"), "variant {n}");
            clean_of(&f, v).unwrap_or_else(|e| panic!("variant {n}: {e}"));
        }
    }

    /// The agent's words in the file, joined in order.
    fn joined_words(f: &str) -> String {
        let mut words = std::collections::BTreeMap::new();
        for line in f.lines() {
            if let Some((_, crate::transcript::Line::Entry(e, _))) =
                crate::transcript::decode_line(line)
            {
                if in_run(&e) {
                    words.insert(e.entry_id, e.text);
                }
            }
        }
        words.into_values().collect()
    }

    /// Property seed 2461: a kill loses the words between a value's first letters (written) and its last ones (after the take-up),
    /// and those letters, each under 4 bytes, make a part of the value where they meet.
    #[test]
    fn letters_that_meet_across_lost_words_are_masked() {
        let v = "Ab#Cd%AbCd";
        let (dir, s, b) = board("cross", v);
        let a = AgentId::new("a");
        b.push_item(&a, &ChatItem::Text("hello Ab".into()), 1);
        b.set_working(&a, false);
        write(&s, &b, false);
        b.set_working(&a, true);
        b.push_item(&a, &ChatItem::Text("#Cd%Ab".into()), 2);
        drop(b); // killed: those words are lost
        let b = ChatBoard::saving(Arc::clone(&s));
        assert!(b.seed_from_disk(&a, 3));
        b.push_item(&a, &ChatItem::Text("Cd there".into()), 4);
        b.set_working(&a, false);
        write(&s, &b, true);
        let f = std::fs::read_to_string(s.path("a").unwrap()).unwrap();
        let _ = std::fs::remove_dir_all(dir);
        let words = joined_words(&f);
        assert!(words.contains("there"), "{words:?}");
        clean_of(&words, v).unwrap();
    }

    /// Performance: text dense with edges (the bridge's marks), with an 8 KiB value held, is written in near-linear time.
    #[test]
    fn text_dense_with_edges_is_written_quickly() {
        let v = format!("Q{}", "abcdefghij".repeat(800));
        let (dir, s, b) = board("dense", &v);
        let a = AgentId::new("a");
        for dense in ["[redacted]".repeat(1600), "[redacted] ".repeat(1480)] {
            let item = ToolItem {
                call_id: "c1".into(),
                tool: Some("sh".into()),
                status: "completed".into(),
                output: Some(dense),
                ..ToolItem::default()
            };
            b.push_item(&a, &ChatItem::Tool(item), 1);
            let start = std::time::Instant::now();
            write(&s, &b, true);
            let took = start.elapsed();
            assert!(took < std::time::Duration::from_secs(3), "{took:?}");
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// #1147: what followers are sent (the live stream and a new follower's snapshot) holds no piece of the node's own secret values,
/// and the agent's words are not delayed when nothing can hide one.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod live {
    use super::*;
    use crate::credentials::{Secret, SecretStore};
    use crate::transcript::Limits;
    use std::collections::BTreeMap;

    const SECRET: &str = "sk-LIVE-0123456789ABCDEFGHIJKLMNOPQRSTUV";
    /// The hold's deadline (`LIVE_HOLD_MS`, 300 ms), written out so these tests also build against a node without it.
    const DEADLINE: u64 = 300;

    fn board(
        tag: &str,
        secrets: &[&str],
        limits: Limits,
    ) -> (std::path::PathBuf, Arc<Store>, ChatBoard) {
        let dir = std::env::temp_dir().join(format!("agora-live-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (i, v) in secrets.iter().enumerate() {
            SecretStore::in_data(&dir)
                .set(&format!("S{i}"), &Secret::new(v).unwrap())
                .unwrap();
        }
        let store = Arc::new(Store::new(&dir, limits, Some(SecretStore::in_data(&dir))));
        let board = ChatBoard::saving(Arc::clone(&store));
        (dir, store, board)
    }

    /// One follower: its snapshot, then every entry sent to it.
    struct Follower {
        sub: ChatSubscription,
        got: Vec<ChatEntry>,
    }

    impl Follower {
        fn of(b: &ChatBoard, a: &AgentId) -> Self {
            let (snapshot, sub) = b.follow(a);
            Self { sub, got: snapshot }
        }

        fn drain(&mut self) -> &[ChatEntry] {
            self.got.extend(self.sub.try_iter());
            &self.got
        }

        /// The agent's words it holds, by id, joined.
        fn words(&mut self) -> String {
            let mut w: BTreeMap<u64, String> = BTreeMap::new();
            for e in self.drain() {
                if in_run(e) {
                    w.insert(e.entry_id, e.text.clone());
                }
            }
            w.into_values().collect()
        }
    }

    fn fields(e: &ChatEntry) -> Vec<&str> {
        let mut f = vec![e.text.as_str(), e.output.as_str(), e.path.as_str()];
        f.extend(e.params.values().map(String::as_str));
        f.extend(e.paths.iter().map(|p| p.path.as_str()));
        f.extend(e.plan_steps.iter().map(|s| s.text.as_str()));
        f
    }

    /// The shortest pieces of `v` of 4 bytes or more from each character: finding none of them finds no longer one.
    fn pieces_of(v: &str) -> Vec<&str> {
        let bounds: Vec<usize> = v.char_indices().map(|(i, _)| i).chain([v.len()]).collect();
        bounds
            .iter()
            .enumerate()
            .filter_map(|(x, &a)| {
                bounds[x + 1..]
                    .iter()
                    .find(|&&z| z - a >= 4)
                    .map(|&z| &v[a..z])
            })
            .collect()
    }

    /// A piece of `v` in any field of any version the follower got, or in its words joined.
    fn leak(f: &mut Follower, v: &str) -> Option<String> {
        let words = f.words();
        for p in pieces_of(v) {
            if let Some(e) = f
                .got
                .iter()
                .find(|e| fields(e).iter().any(|x| x.contains(p)))
            {
                return Some(format!("{p:?} in entry {}: {:?}", e.entry_id, fields(e)));
            }
            if words.contains(p) {
                return Some(format!("{p:?} in the joined words: {words:?}"));
            }
        }
        None
    }

    fn text(t: &str) -> ChatItem {
        ChatItem::Text(t.into())
    }

    /// Splits `s` at the given byte offsets.
    fn split<'a>(s: &'a str, at: &[usize]) -> Vec<&'a str> {
        let mut out = Vec::new();
        let mut from = 0;
        for &k in at.iter().chain([&s.len()]) {
            out.push(&s[from..k]);
            from = k;
        }
        out
    }

    /// A value streamed in pieces of 1 to 7 bytes inside a turn, with a follower from the start and one that joins in its middle.
    #[test]
    fn a_value_streamed_in_pieces_never_reaches_a_follower() {
        for cuts in [
            vec![1, 2, 3],
            vec![3, 4, 5, 20, 21],
            vec![7, 14, 21, 28, 35],
            (1..SECRET.len()).collect::<Vec<_>>(),
        ] {
            let (_d, _s, b) = board("pieces", &[SECRET], Limits::default());
            let a = AgentId::new("a");
            b.set_working(&a, true);
            let mut first = Follower::of(&b, &a);
            b.push_item(&a, &text("the key is "), 10);
            let mut late = None;
            let parts = split(SECRET, &cuts);
            for (i, p) in parts.iter().enumerate() {
                b.push_item(&a, &text(p), 10 + i as u64);
                if i == parts.len() / 2 {
                    late = Some(Follower::of(&b, &a));
                }
            }
            b.push_item(&a, &text(" and that is all."), 100);
            b.set_working(&a, false);
            let mut late = late.unwrap();
            for f in [&mut first, &mut late] {
                assert_eq!(leak(f, SECRET), None, "split at {cuts:?}");
                assert!(f.words().ends_with(" and that is all."), "{:?}", f.words());
            }
            assert!(first.words().contains(REDACTED_MARK));
        }
    }

    const REDACTED_MARK: &str = "<redacted>";

    /// The words go before a tool call, at the turn's end and at the hold's deadline while a value is half said: what was sent of it
    /// is replaced at that end (an edge), and the rest when it comes; and ids never go back on the stream.
    #[test]
    fn a_value_cut_by_a_tool_call_a_turns_end_or_the_deadline_never_reaches_a_follower() {
        for (how, k) in [
            ("tool", 9),
            ("turn", 12),
            ("deadline", 20),
            ("tool", 4),
            ("deadline", 5),
        ] {
            let (_d, _s, b) = board("forced", &[SECRET], Limits::default());
            let a = AgentId::new("a");
            b.set_working(&a, true);
            let mut f = Follower::of(&b, &a);
            b.push_item(&a, &text(&format!("say {}", &SECRET[..k])), 1_000);
            match how {
                "tool" => b.push_item(
                    &a,
                    &ChatItem::Tool(ToolItem {
                        call_id: "c1".into(),
                        tool: Some("sh".into()),
                        status: "in_progress".into(),
                        ..ToolItem::default()
                    }),
                    1_001,
                ),
                "turn" => {
                    b.set_working(&a, false);
                    b.set_working(&a, true);
                }
                _ => {
                    b.flush_due(1_000 + DEADLINE - 1);
                    assert_eq!(
                        f.words(),
                        "",
                        "{how} {k}: the end is part of the value: it waits"
                    );
                    b.flush_due(1_000 + DEADLINE);
                }
            }
            // Whatever made them go, the words are out now.
            assert!(f.words().starts_with("say "), "{how} {k}: {:?}", f.words());
            b.push_item(&a, &text(&format!("{} done", &SECRET[k..])), 2_000);
            b.set_working(&a, false);
            assert_eq!(leak(&mut f, SECRET), None, "{how} at {k}");
            assert!(f.words().ends_with(" done"));
            let mut high = 0;
            let mut seen = BTreeSet::new();
            for e in f.drain() {
                assert!(
                    e.entry_id > high || seen.contains(&e.entry_id),
                    "{how}: id {} after {high}",
                    e.entry_id
                );
                high = high.max(e.entry_id);
                seen.insert(e.entry_id);
            }
        }
    }

    /// A text over 16 KiB that the node cuts inside a value, the next piece going on with it; then a stop, a take-up by a new node
    /// in the middle of another value, and a follower of the new node.
    #[test]
    fn a_value_on_a_16_kib_cut_or_across_a_take_up_never_reaches_a_follower() {
        let (dir, store, b) = board("cut", &[SECRET], Limits::default());
        let a = AgentId::new("a");
        b.set_working(&a, true);
        let mut f = Follower::of(&b, &a);
        let pre = "w".repeat(MAX_ENTRY_BYTES - 10);
        b.push_item(&a, &text(&format!("{pre}{}", &SECRET[..25])), 1);
        b.push_item(&a, &text(&format!("{} next ", &SECRET[25..])), 2);
        b.push_item(&a, &text(&format!("then {}", &SECRET[..14])), 3);
        assert_eq!(leak(&mut f, SECRET), None);
        // The node stops (its flush writes all), and the next one takes the agent up and hears the rest.
        for batch in b.take_unsaved() {
            store.write(&batch).unwrap();
        }
        drop(b);
        let b = ChatBoard::saving(Arc::new(Store::new(
            &dir,
            Limits::default(),
            Some(SecretStore::in_data(&dir)),
        )));
        b.seed_from_disk(&a, 10);
        b.set_working(&a, true);
        let mut g = Follower::of(&b, &a);
        b.push_item(&a, &text(&format!("{} end", &SECRET[14..])), 11);
        b.set_working(&a, false);
        assert_eq!(leak(&mut g, SECRET), None);
        assert!(g.words().ends_with(" end"));
        let mut late = Follower::of(&b, &a);
        assert_eq!(leak(&mut late, SECRET), None);
    }

    /// Everything else a follower is sent is redacted too: a tool call's output, arguments and text in every version, a notice's
    /// parameters, a plan step, a permission label and the person's own prompt; also with no history kept on disk.
    #[test]
    fn every_entry_a_follower_is_sent_is_redacted_also_with_no_history_kept() {
        for limits in [
            Limits::default(),
            Limits {
                max_bytes: 0,
                days: 0,
            },
        ] {
            let (_d, _s, b) = board("fields", &[SECRET], limits);
            let a = AgentId::new("a");
            let mut f = Follower::of(&b, &a);
            for (i, out) in [&SECRET[..10], &SECRET[..30], SECRET].iter().enumerate() {
                b.push_item(
                    &a,
                    &ChatItem::Tool(ToolItem {
                        call_id: "c1".into(),
                        tool: Some("sh".into()),
                        // Arguments that grow as the call is streamed: a version not final ends where it was cut.
                        arguments_json: Some(if i == 2 {
                            format!("{{\"cmd\":\"echo {out}\"}}")
                        } else {
                            format!("{{\"cmd\":\"echo {out}")
                        }),
                        status: if i == 2 { "completed" } else { "in_progress" }.into(),
                        output: Some(format!("printed {out}")),
                        ..ToolItem::default()
                    }),
                    1_000 * i as u64,
                );
            }
            b.push_item(
                &a,
                &ChatItem::Notice {
                    code: "adapter.turn_failed".into(),
                    params: vec![("stderr_tail".into(), format!("…{}", &SECRET[6..]))],
                },
                5_000,
            );
            b.push_item(
                &a,
                &ChatItem::PlanSteps(vec![PlanItem {
                    text: format!("use {SECRET}"),
                    ..PlanItem::default()
                }]),
                5_001,
            );
            b.push_permission(&a, "ap-1", &format!("run with {SECRET}"), 5_002);
            b.push_user(&a, format!("my key is {SECRET}"), 5_003);
            b.flush_due(1_000_000);
            assert_eq!(leak(&mut f, SECRET), None, "{limits:?}");
            let mut late = Follower::of(&b, &a);
            assert_eq!(leak(&mut late, SECRET), None, "{limits:?}");
            assert!(b
                .snapshot(&a)
                .iter()
                .all(|e| fields(e).iter().all(|x| !x.contains(&SECRET[..12]))));
        }
    }

    /// ADR-0022: with no secret value held, every word reaches a follower at once, as it is; with one held, words whose end cannot
    /// be part of it are not delayed either, and only an end that is part of it (4 bytes or more) waits.
    #[test]
    fn words_reach_a_follower_at_once_unless_their_end_is_part_of_a_secret() {
        let (_d, _s, none) = board("none", &[], Limits::default());
        let memory = ChatBoard::new();
        let (_d2, _s2, held) = board("held", &[SECRET], Limits::default());
        let a = AgentId::new("a");
        for b in [&none, &memory, &held] {
            b.set_working(&a, true);
            let (_, sub) = b.follow(&a);
            for w in [
                "Let me look ",
                "at the file",
                ". It says sk",
                "-L and more.",
            ] {
                b.push_item(&a, &text(w), 1);
                let e = sub.try_recv().expect("sent at once");
                assert_eq!(e.text, w);
            }
        }
        held.set_working(&a, true);
        let (_, sub) = held.follow(&a);
        held.push_item(&a, &text("the key: sk-LIVE"), 1);
        assert!(
            sub.try_recv().is_err(),
            "an end that is part of the value waits"
        );
        held.push_item(&a, &text(" was wrong"), 2);
        assert_eq!(sub.try_recv().unwrap().text, "the key: sk-LIVE");
        assert_eq!(sub.try_recv().unwrap().text, " was wrong");
    }

    /// Letters only a value holds (upper case), never the words around it (lower case).
    fn value(seed: u64, len: usize) -> String {
        let mut x = seed | 1;
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                char::from(b'A' + (x % 26) as u8)
            })
            .collect()
    }

    /// A value of 2 KiB streamed a byte at a time fills the ring (500 entries) before it is whole: the words held at its front go,
    /// their end an edge, so the ring stays bounded, and no piece of the value reaches a follower.
    #[test]
    fn words_held_at_the_rings_bound_go_with_the_value_replaced() {
        let v = value(7, 2048);
        let (_d, _s, b) = board("ring", &[&v], Limits::default());
        let a = AgentId::new("a");
        b.set_working(&a, true);
        let mut f = Follower::of(&b, &a);
        b.push_item(&a, &text("start "), 1);
        for c in v.chars() {
            b.push_item(&a, &text(&c.to_string()), 1);
            f.drain();
        }
        assert!(f.got.len() > 400, "the words at the ring's front went");
        assert!(b.snapshot(&a).len() <= MAX_ENTRIES);
        b.push_item(&a, &text(" end"), 2);
        b.set_working(&a, false);
        assert_eq!(leak(&mut f, &v), None);
        assert!(f.words().starts_with("start "));
        assert!(f.words().ends_with(" end"));
        let late = b.snapshot(&a);
        assert!(late.len() <= MAX_ENTRIES);
        assert!(late
            .iter()
            .all(|e| !e.text.chars().any(|c| c.is_ascii_uppercase())));
    }

    /// A value over 16 KiB (one the node passes from its environment) has no automaton: the words within its length of the stream's
    /// end wait, so it is whole before any of it goes, and it is replaced whole.
    #[test]
    fn a_value_over_16_kib_waits_until_it_is_whole() {
        let name = format!("AGORA_TEST_LIVE_LONG_{}", std::process::id());
        let v = value(11, 20_000);
        std::env::set_var(&name, &v);
        let dir = std::env::temp_dir().join(format!("agora-live-long-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store =
            Arc::new(Store::new(&dir, Limits::default(), None).with_env_secrets([name.as_str()]));
        std::env::remove_var(&name);
        let b = ChatBoard::saving(store);
        let a = AgentId::new("a");
        b.set_working(&a, true);
        let mut f = Follower::of(&b, &a);
        b.push_item(&a, &text("the key "), 1);
        for chunk in v.as_bytes().chunks(4096) {
            b.push_item(&a, &text(std::str::from_utf8(chunk).unwrap()), 1);
        }
        b.push_item(&a, &text(" ok"), 1);
        assert_eq!(
            f.words(),
            "the key ",
            "within the value's length of the end: waits"
        );
        b.set_working(&a, false);
        assert!(f.words().ends_with(" ok"));
        assert!(f
            .got
            .iter()
            .all(|e| !e.text.chars().any(|c| c.is_ascii_uppercase())));
    }

    /// Audit of #1157: words shown before a value was added to the store (with another value held, or with none) are redacted again
    /// in every later snapshot, also when the value lies across two of them; only masks are added.
    #[test]
    fn a_value_added_after_words_were_shown_is_out_of_every_later_snapshot() {
        let added = "ADDED-VALUE-0123456789-XYZ";
        for held in [vec![SECRET], vec![]] {
            let (dir, store, b) = board("added", &held, Limits::default());
            let a = AgentId::new("a");
            b.push_item(&a, &text(&format!("first {added} then ")), 1);
            b.push_item(&a, &text(&format!("split {}", &added[..10])), 2);
            b.push_item(&a, &text(&format!("{} end", &added[10..])), 3);
            assert!(
                b.snapshot(&a).iter().any(|e| e.text.contains(added)),
                "shown before it was a secret"
            );
            SecretStore::in_data(&dir)
                .set("ADDED", &Secret::new(added).unwrap())
                .unwrap();
            // The node's tick (or the writer) reads the values again.
            store.refresh_if_older(std::time::Duration::ZERO);
            let mut late = Follower::of(&b, &a);
            assert_eq!(leak(&mut late, added), None, "{held:?}");
            assert!(late.words().starts_with("first <redacted> then "));
            assert!(b.snapshot(&a).iter().all(|e| !e.text.contains(&added[..8])));
        }
    }

    /// Audit of #1157, round 2: a value whose start left the ring before the value was added to the store: the ring's first word
    /// holds only its tail, and a later snapshot still masks it (the start of that word is an edge).
    #[test]
    fn a_value_whose_start_left_the_ring_is_masked_once_it_is_added() {
        let added = "ADDED-KEY-0123456789-9876";
        let (dir, store, b) = board("evicted", &[SECRET], Limits::default());
        let a = AgentId::new("a");
        b.push_item(&a, &text(&format!("the key is {}", &added[..12])), 1);
        for _ in 0..MAX_ENTRIES {
            b.output_dropped(&a, 7, 1);
        }
        b.push_item(&a, &text(&format!("{} ok", &added[12..])), 2);
        assert!(
            b.snapshot(&a)
                .iter()
                .all(|e| !e.text.starts_with("the key")),
            "the start left the ring"
        );
        SecretStore::in_data(&dir)
            .set("ADDED", &Secret::new(added).unwrap())
            .unwrap();
        store.refresh_if_older(std::time::Duration::ZERO);
        let mut late = Follower::of(&b, &a);
        assert_eq!(leak(&mut late, added), None);
        assert_eq!(late.words(), "<redacted> ok");
    }

    /// A follower's snapshot never holds the words still held, and gets them on its stream once they go.
    #[test]
    fn a_snapshot_leaves_out_the_words_held_and_the_stream_brings_them() {
        let (_d, _s, b) = board("snap", &[SECRET], Limits::default());
        let a = AgentId::new("a");
        b.set_working(&a, true);
        b.push_item(&a, &text("before "), 1);
        b.push_item(&a, &text(&SECRET[..20]), 2);
        let (snap, sub) = b.follow(&a);
        assert_eq!(
            snap.iter().map(|e| e.text.as_str()).collect::<String>(),
            "before "
        );
        b.set_working(&a, false);
        let e = sub.try_recv().unwrap();
        assert_eq!(e.text, REDACTED_MARK);
        assert!(b
            .snapshot(&a)
            .iter()
            .all(|e| !e.text.contains(&SECRET[..8])));
    }

    /// ADR-0111: a value an adapter cuts in the middle of one of an item's strings (letters only, so no number the file holds
    /// matches a piece of it by chance).
    const CUT_VALUE: &str = "sk-CUTS-qwertyuiopASDFGHJKLzxcvbnmPOIUYT";

    /// `whole` with 6 bytes taken out of the middle of [`CUT_VALUE`] in it, and where they were.
    fn cut_value(whole: &str) -> (String, usize) {
        let at = whole.find(CUT_VALUE).unwrap() + CUT_VALUE.len() / 2 - 3;
        (format!("{}{}", &whole[..at], &whole[at + 6..]), at)
    }

    /// Each string of each item an adapter may report a cut in, as the adapter sends it once cut, with that cut.
    fn cut_items() -> Vec<(&'static str, ChatItem, ReportedCut)> {
        let whole = format!("the key is {CUT_VALUE} and more words");
        let (kept, at) = cut_value(&whole);
        let rc = |field| ReportedCut { field, at };
        let tool = |t: ToolItem| {
            ChatItem::Tool(ToolItem {
                call_id: "c1".into(),
                status: "completed".into(),
                ..t
            })
        };
        let k = || kept.clone();
        vec![
            ("text", ChatItem::Text(k()), rc(CutField::Text)),
            ("thought", ChatItem::Thought(k()), rc(CutField::Text)),
            (
                "output",
                tool(ToolItem {
                    tool: Some("sh".into()),
                    output: Some(k()),
                    ..ToolItem::default()
                }),
                rc(CutField::Output),
            ),
            (
                "arguments",
                tool(ToolItem {
                    tool: Some("sh".into()),
                    arguments_json: Some(k()),
                    ..ToolItem::default()
                }),
                rc(CutField::Arguments),
            ),
            (
                "tool",
                tool(ToolItem {
                    tool: Some(k()),
                    ..ToolItem::default()
                }),
                rc(CutField::Tool),
            ),
            (
                "path",
                tool(ToolItem {
                    tool: Some("sh".into()),
                    locations: Some(vec![("a.rs".into(), 1), (k(), 2)]),
                    ..ToolItem::default()
                }),
                rc(CutField::Path(1)),
            ),
            (
                "step",
                ChatItem::PlanSteps(vec![
                    PlanItem {
                        text: "first".into(),
                        ..PlanItem::default()
                    },
                    PlanItem {
                        text: k(),
                        ..PlanItem::default()
                    },
                ]),
                rc(CutField::Step(1)),
            ),
            (
                "planline",
                ChatItem::Plan(vec!["first".into(), k()]),
                rc(CutField::Step(1)),
            ),
            (
                "callargs",
                ChatItem::ToolCall {
                    tool: "sh".into(),
                    arguments_json: k(),
                },
                rc(CutField::Arguments),
            ),
            (
                "callname",
                ChatItem::ToolCall {
                    tool: k(),
                    arguments_json: "{}".into(),
                },
                rc(CutField::Tool),
            ),
            (
                "diff",
                ChatItem::Diff {
                    path: "a.rs".into(),
                    unified_diff: k(),
                },
                rc(CutField::Text),
            ),
            (
                "diffpath",
                ChatItem::Diff {
                    path: k(),
                    unified_diff: "+x".into(),
                },
                rc(CutField::DiffPath),
            ),
            (
                "notice",
                ChatItem::Notice {
                    code: "adapter.turn_failed".into(),
                    params: vec![("detail".into(), k())],
                },
                rc(CutField::Param("detail".into())),
            ),
        ]
    }

    /// A piece of `v` (4 bytes or more) in any field of the entries the file holds, or in a call's tool name and arguments.
    fn saved_leak(store: &Store, agent: &str, v: &str) -> Option<String> {
        for (e, side) in store.read(agent).entries {
            let mut f = fields(&e);
            if let Some((tool, args)) = &side.tool {
                f.extend([tool.as_str(), args.as_str()]);
            }
            for p in pieces_of(v) {
                if let Some(x) = f.iter().find(|x| x.contains(p)) {
                    return Some(format!("{p:?} in entry {}: {x:?}", e.entry_id));
                }
            }
        }
        None
    }

    /// ADR-0111: an adapter cuts a held value in the middle of each kind of string it sends and reports the cut: neither a follower
    /// nor the file holds a piece of it (4 bytes or more). The same cut not reported leaves pieces in both: the residual of an
    /// adapter without `chat.cuts` (and of one that declared it but did not report), which this test documents.
    #[test]
    fn a_value_an_adapter_cut_and_reported_never_reaches_a_follower_or_the_file() {
        for (name, item, cut) in cut_items() {
            for report in [true, false] {
                let (_d, store, b) = board(
                    &format!("adr0111-{name}-{report}"),
                    &[CUT_VALUE],
                    Limits::default(),
                );
                let a = AgentId::new("a");
                let mut f = Follower::of(&b, &a);
                let cuts = if report {
                    std::slice::from_ref(&cut)
                } else {
                    &[]
                };
                b.push_item_cut(&a, &item, cuts, 1);
                for batch in b.take_unsaved() {
                    store.write(&batch).unwrap();
                }
                let (live, file) = (leak(&mut f, CUT_VALUE), saved_leak(&store, "a", CUT_VALUE));
                if report {
                    assert_eq!((&live, &file), (&None, &None), "{name}");
                    // A window that follows only now gets a snapshot redacted again from the places kept beside the ring.
                    let mut late = Follower::of(&b, &a);
                    assert_eq!(leak(&mut late, CUT_VALUE), None, "{name}: snapshot");
                } else {
                    assert!(
                        live.is_some() && file.is_some(),
                        "{name}: an unreported cut leaves a piece ({live:?}, {file:?})"
                    );
                }
            }
        }
    }

    /// ADR-0111: a cut reported on an update of a running call (how an adapter mostly sends output): the update's places replace
    /// the earlier version's, in the live stream, a late snapshot and the file.
    #[test]
    fn a_cut_reported_on_an_update_of_a_call_is_kept_with_that_version() {
        let (_d, store, b) = board("adr0111-update", &[CUT_VALUE], Limits::default());
        let a = AgentId::new("a");
        let mut f = Follower::of(&b, &a);
        let call = |status: &str, output: Option<String>| {
            ChatItem::Tool(ToolItem {
                call_id: "c1".into(),
                tool: Some("sh".into()),
                status: status.into(),
                output,
                ..ToolItem::default()
            })
        };
        b.push_item(&a, &call("in_progress", Some("starting".into())), 1);
        for batch in b.take_unsaved() {
            store.write(&batch).unwrap();
        }
        let (kept, at) = cut_value(&format!("the key is {CUT_VALUE} and more words"));
        let cut = ReportedCut {
            field: CutField::Output,
            at,
        };
        b.push_item_cut(&a, &call("completed", Some(kept)), &[cut], 2);
        for batch in b.take_unsaved() {
            store.write(&batch).unwrap();
        }
        assert_eq!(leak(&mut f, CUT_VALUE), None);
        assert_eq!(saved_leak(&store, "a", CUT_VALUE), None);
        let mut late = Follower::of(&b, &a);
        assert_eq!(leak(&mut late, CUT_VALUE), None);
        assert!(late
            .drain()
            .iter()
            .any(|e| e.output.ends_with("more words")));
    }

    /// ADR-0111: a reported cut in the agent's words is a place in their stream too: the next piece, which goes on with the value,
    /// starts at no edge of its own, but the cut before it is one.
    #[test]
    fn a_reported_cut_in_the_agents_words_is_an_edge_of_their_stream() {
        let (_d, store, b) = board("adr0111-stream", &[CUT_VALUE], Limits::default());
        let a = AgentId::new("a");
        b.set_working(&a, true);
        let mut f = Follower::of(&b, &a);
        // The adapter dropped 6 bytes inside the value at the end of one piece; the next piece goes on with the rest.
        let head = format!("say {}", &CUT_VALUE[..12]);
        let cut = ReportedCut {
            field: CutField::Text,
            at: head.len(),
        };
        b.push_item_cut(&a, &ChatItem::Text(head.clone()), &[cut], 1);
        b.push_item(&a, &ChatItem::Text(format!("{} done", &CUT_VALUE[18..])), 2);
        b.set_working(&a, false);
        for batch in b.take_unsaved() {
            store.write(&batch).unwrap();
        }
        assert_eq!(leak(&mut f, CUT_VALUE), None);
        assert_eq!(saved_leak(&store, "a", CUT_VALUE), None);
        assert!(f.words().ends_with(" done"), "{:?}", f.words());
    }

    /// ADR-0111: where a reported place lands through the node's own cleaning and cuts: it moves with what cleaning removes before
    /// it, one in a dropped middle is gone, one at the very end is kept (the node's own is not), and a place reported alone does
    /// not make the text count as cut by the node.
    #[test]
    fn reported_places_move_through_cleaning_and_the_nodes_cuts() {
        let esc = "\u{1b}[31m";
        let text = format!("ab{esc}cd{esc}ef");
        let at = 2 + esc.len() + 2;
        let (out, node_cut, places) = tidy_cuts_at(&text, 100, Style::Prose, &[at, 0, text.len()]);
        assert_eq!((out.as_str(), node_cut), ("abcdef", false));
        assert_eq!(places, [0, 4, 6]);
        // Without a report the result is the node's alone, as before.
        assert_eq!(
            tidy_cuts_at(&text, 100, Style::Prose, &[]),
            tidy_cuts(&text, 100, Style::Prose)
        );
        assert_eq!(tidy_cuts(&text, 100, Style::Prose).2, Vec::<usize>::new());
        // The node keeps 4 + 4 of 20 bytes: a place in the dropped middle is gone, one in the kept tail moves with it.
        let long = "abcdefghijklmnopqrst";
        let (out, node_cut, places) = tidy_cuts_at(long, 8, Style::Prose, &[2, 10, 18]);
        assert_eq!((out.as_str(), node_cut), ("abcdqrst", true));
        assert_eq!(places, [2, 4, 6]);
        // A place inside a character (never from the host, which checks) is left out rather than cut through.
        let (out, _, places) = tidy_cuts_at("aé", 100, Style::Prose, &[2]);
        assert_eq!((out.as_str(), places), ("aé", Vec::new()));
    }

    /// ADR-0111 (review of #1187): a place reported inside an escape sequence, or between `\r` and `\n`, never splits the cleaning:
    /// a value the escape sat in is whole once cleaned, and is replaced whole, exactly as without the place. (Cleaning apart at the
    /// place left `31m` in the text, so the value's second half stood away from every edge.)
    #[test]
    fn a_place_inside_an_escape_sequence_masks_no_less_than_none() {
        let half = CUT_VALUE.len() / 2;
        for (tag, mid) in [
            ("esc", "\u{1b}[31m"),
            ("osc", "\u{1b}]0;t\u{7}"),
            ("crlf", ""),
        ] {
            let (out, at) = if mid.is_empty() {
                // The place between `\r` and `\n`, the value after them.
                let out = format!("the key is\r\n{CUT_VALUE} and more words");
                let at = out.find('\n').unwrap();
                (out, at)
            } else {
                let out = format!(
                    "the key is {}{mid}{} and more words",
                    &CUT_VALUE[..half],
                    &CUT_VALUE[half..]
                );
                // Just after the sequence's first two characters.
                let at = out.find('\u{1b}').unwrap() + 2;
                (out, at)
            };
            let mut outputs = Vec::new();
            for report in [false, true] {
                let (_d, store, b) = board(
                    &format!("adr0111-inside-{tag}-{report}"),
                    &[CUT_VALUE],
                    Limits::default(),
                );
                let a = AgentId::new("a");
                let mut f = Follower::of(&b, &a);
                let item = ChatItem::Tool(ToolItem {
                    call_id: "c1".into(),
                    tool: Some("sh".into()),
                    status: "completed".into(),
                    output: Some(out.clone()),
                    ..ToolItem::default()
                });
                let cut = ReportedCut {
                    field: CutField::Output,
                    at,
                };
                let cuts = if report {
                    std::slice::from_ref(&cut)
                } else {
                    &[]
                };
                b.push_item_cut(&a, &item, cuts, 1);
                for batch in b.take_unsaved() {
                    store.write(&batch).unwrap();
                }
                assert_eq!(leak(&mut f, CUT_VALUE), None, "{tag} {report}");
                assert_eq!(saved_leak(&store, "a", CUT_VALUE), None, "{tag} {report}");
                outputs.push(store.read("a").entries[0].0.output.clone());
            }
            assert_eq!(
                outputs[0], outputs[1],
                "{tag}: the place changed what is kept"
            );
        }
        // Where such places land: where the sequence was, and before the line break `\r\n` makes.
        let (out, at) = strip_core_at("ab\u{1b}[31mcd", Style::Prose, &[4, 6]);
        assert_eq!((out.as_str(), at), ("abcd", vec![2, 2]));
        let (out, at) = strip_core_at("a\r\nb", Style::Prose, &[2, 4]);
        assert_eq!((out.as_str(), at), ("a\nb", vec![1, 3]));
    }

    /// ADR-0111: places that never come from the host (past the end, inside a character, an index past the list) go straight to the
    /// board: nothing panics and every item is kept as without them.
    #[test]
    fn hostile_places_on_the_board_change_nothing_and_never_panic() {
        for (name, item, _) in cut_items() {
            let hostile: Vec<ReportedCut> = [
                CutField::Text,
                CutField::Tool,
                CutField::Arguments,
                CutField::Output,
                CutField::Path(usize::MAX),
                CutField::Step(usize::MAX),
                CutField::Step(1),
                CutField::DiffPath,
                CutField::Param("detail".into()),
            ]
            .into_iter()
            .flat_map(|field| {
                [usize::MAX, MAX_ENTRY_BYTES + 1, 1_000_000].map(|at| ReportedCut {
                    field: field.clone(),
                    at,
                })
            })
            .collect();
            let a = AgentId::new("a");
            let (_d, _s, plain) = board(
                &format!("adr0111-plain-{name}"),
                &[CUT_VALUE],
                Limits::default(),
            );
            plain.push_item(&a, &item, 1);
            let (_d, store, b) = board(
                &format!("adr0111-hostile-{name}"),
                &[CUT_VALUE],
                Limits::default(),
            );
            b.push_item_cut(&a, &item, &hostile, 1);
            for batch in b.take_unsaved() {
                store.write(&batch).unwrap();
            }
            let shown = |b: &ChatBoard| -> Vec<Vec<String>> {
                b.snapshot(&a)
                    .iter()
                    .map(|e| fields(e).into_iter().map(str::to_string).collect())
                    .collect()
            };
            assert_eq!(shown(&b), shown(&plain), "{name}");
        }
    }

    /// ADR-0111: a resumed session's replay keeps its reported places.
    #[test]
    fn a_replayed_item_keeps_its_reported_places() {
        let (_d, store, b) = board("adr0111-replay", &[CUT_VALUE], Limits::default());
        let a = AgentId::new("a");
        let (_, item, cut) = cut_items()
            .into_iter()
            .find(|(n, _, _)| *n == "output")
            .unwrap();
        assert!(b.replayed(&a, &[(item, vec![cut])], 1));
        for batch in b.take_unsaved() {
            store.write(&batch).unwrap();
        }
        let mut late = Follower::of(&b, &a);
        assert_eq!(leak(&mut late, CUT_VALUE), None);
        assert_eq!(saved_leak(&store, "a", CUT_VALUE), None);
    }

    /// ADR-0111 (review of #1187): places in a plan of very many steps cost one pass, however many there are.
    #[test]
    #[ignore = "a measurement (ADR-0111): cargo test -p agora-node --lib -- --ignored adr0111_many_steps"]
    fn adr0111_many_steps() {
        let lines = vec![String::new(); 1_400_000];
        let cuts: Vec<ReportedCut> = (0..64)
            .map(|k| ReportedCut {
                field: CutField::Step(lines.len() - 1 - k),
                at: 0,
            })
            .collect();
        let b = ChatBoard::new();
        let a = AgentId::new("a");
        let item = ChatItem::Plan(lines);
        let start = std::time::Instant::now();
        b.push_item_cut(&a, &item, &cuts, 1);
        eprintln!("plan of 1.4M steps, 64 places: {:?}", start.elapsed());
        let start = std::time::Instant::now();
        b.push_item(&a, &item, 2);
        eprintln!("the same without places: {:?}", start.elapsed());
    }

    /// ADR-0111 (ADR-0022): the cost of reported places on the chat path: a 128 KiB tool output cleaned and capped with none and with
    /// 64, and a board taking the item and writing it with the value held.
    #[test]
    #[ignore = "a measurement (ADR-0111): cargo test -p agora-node --lib -- --ignored adr0111_perf"]
    fn adr0111_perf() {
        let raw: String = (0..1600)
            .map(|i| format!("line {i} \u{1b}[32mok\u{1b}[0m words\n"))
            .collect::<String>()
            .chars()
            .take(64 * 1024)
            .collect();
        // Twice that: over the node's first bound (`PRE_BOUND`), so its own seam is split and cleaned apart too.
        let raw = raw.repeat(2);
        let none: Vec<usize> = Vec::new();
        let many: Vec<usize> = (1..=64)
            .map(|i| i * raw.len() / 65)
            .filter(|&i| raw.is_char_boundary(i))
            .collect();
        for (label, cuts) in [("0 places", &none), ("64 places", &many)] {
            let n = 2_000u32;
            let start = std::time::Instant::now();
            for _ in 0..n {
                std::hint::black_box(tidy_cuts_at(
                    std::hint::black_box(&raw),
                    MAX_OUTPUT_BYTES,
                    Style::Prose,
                    cuts,
                ));
            }
            eprintln!(
                "tidy 128 KiB output, {label}: {:?} each",
                start.elapsed() / n
            );
        }
        for (label, cuts) in [("0 places", &none), ("64 places", &many)] {
            let (_d, store, b) = board(
                &format!("adr0111-perf-{}", cuts.len()),
                &[CUT_VALUE],
                Limits::default(),
            );
            let a = AgentId::new("a");
            let reported: Vec<ReportedCut> = cuts
                .iter()
                .map(|&at| ReportedCut {
                    field: CutField::Output,
                    at,
                })
                .collect();
            let n = 500u32;
            let start = std::time::Instant::now();
            for i in 0..n {
                let item = ChatItem::Tool(ToolItem {
                    call_id: format!("c{i}"),
                    tool: Some("sh".into()),
                    status: "completed".into(),
                    output: Some(raw.clone()),
                    ..ToolItem::default()
                });
                b.push_item_cut(&a, &item, &reported, u64::from(i));
                for batch in b.take_unsaved() {
                    store.write(&batch).unwrap();
                }
            }
            eprintln!(
                "push + redact + write, {label}: {:?} each",
                start.elapsed() / n
            );
        }
    }
}
