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

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use agora_adapters::host::{ChatItem, PlanItem, ToolItem};
use agora_core::AgentId;
use agora_protocol::v1::{
    ChatEntry, ChatKind, ChatRole, PermissionOutcome, PlanPriority, PlanStep, ToolKind,
    ToolLocation, ToolStatus,
};

use crate::transcript::{Batch, Line, Seed, Side, Store, Wake};

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
/// The most entries a follower may have waiting. A follower that falls further behind (a client that stopped reading) is dropped, and
/// its stream ends: the client subscribes again and gets the ring. One slow client therefore never grows the node's memory.
pub const FOLLOWER_QUEUE: usize = 1024;
/// Work on a text is done on at most this many bytes of it (head and tail) before it is cleaned and cut to its cap, so an adapter
/// cannot make the node spend more than a bounded pass on one item.
const PRE_BOUND: usize = 64 * 1024;
/// Notice codes only the node makes. An adapter's item with one of them is dropped: it could otherwise forge the node's own words.
pub const NODE_NOTICES: &[&str] = &[
    "adapter.queue_dropped",
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

fn cut(text: String) -> String {
    cut_to(text, MAX_ENTRY_BYTES)
}

fn cut_to(mut text: String, max: usize) -> String {
    if text.len() > max {
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

/// `text` as is when it fits `max` bytes; otherwise its first and its last bytes (cut on character boundaries, `max` in all) and
/// `true`. Tool output keeps its head (what ran) and its tail (how it ended).
fn head_and_tail(text: String, max: usize) -> (String, bool) {
    if text.len() <= max {
        return (text, false);
    }
    let mut head = max / 2;
    while !text.is_char_boundary(head) {
        head -= 1;
    }
    let mut tail = text.len() - (max - head);
    while !text.is_char_boundary(tail) {
        tail += 1;
    }
    (format!("{}{}", &text[..head], &text[tail..]), true)
}

/// The text of a tool call entry: the tool and its arguments as JSON (what an old client shows).
fn tool_text(tool: &str, arguments_json: &str) -> String {
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
            let text = if agora_core::secrets::is_secret_path(path) {
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
    if code.len() > MAX_CODE || !code.contains('.') || !code.split('.').all(word) {
        return None;
    }
    let params = params
        .iter()
        .filter(|(k, _)| k.len() <= MAX_PARAM_KEY && word(k))
        .take(MAX_PARAMS)
        .map(|(k, v)| (k.clone(), cut_to(v.clone(), MAX_PARAM_BYTES)))
        .collect();
    Some((code.to_string(), params))
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

/// How a text is shown, which decides what is kept of it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    /// Prose over several lines (thinking, tool output): line breaks and tabs stay; the direction marks of Arabic and Hebrew text
    /// (LRM, RLM, ALM) stay, and so do isolates (U+2066 to U+2069) when each one is closed.
    Prose,
    /// Prose on one line (a plan step): as `Prose`, with line breaks as spaces.
    Step,
    /// Code on several lines (a tool's arguments): line breaks stay, every direction control goes.
    Code,
    /// A name, a path or a label on one line: every direction control goes, since there they only serve to make one thing read as
    /// another (`gpj.exe` written as `exe.jpg`).
    Label,
}

impl Style {
    fn multiline(self) -> bool {
        matches!(self, Self::Prose | Self::Code)
    }
    fn keeps_marks(self) -> bool {
        matches!(self, Self::Prose | Self::Step)
    }
}

/// The longest payload an unterminated OSC, DCS, ... sequence may swallow: to the next line break or this many characters.
const ESCAPE_PAYLOAD_MAX: usize = 4096;

/// Removes what would act on a terminal or a reader instead of being shown: escape sequences (CSI, OSC, DCS, ...) with their payload
/// (a sequence that is never closed swallows at most to the next line break or `ESCAPE_PAYLOAD_MAX` characters, never the rest of the
/// text), every control character (C0, DEL, C1) and, always, the characters that override or embed a direction (U+202A to U+202E:
/// the ones that spoof). Other direction marks are kept or removed by the [`Style`]. A bare carriage return is a line break in
/// multi-line text and a space otherwise; `\r\n` is one line break.
fn strip_controls(s: &str, style: Style) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' | '\u{9b}' | '\u{9d}' | '\u{90}' | '\u{98}' | '\u{9e}' | '\u{9f}' => {
                let kind = match c {
                    '\u{1b}' => chars.next(),
                    '\u{9b}' => Some('['),
                    '\u{9d}' => Some(']'),
                    _ => Some('P'),
                };
                match kind {
                    Some('[') => {
                        for c in chars.by_ref() {
                            if ('@'..='~').contains(&c) {
                                break;
                            }
                        }
                    }
                    Some(']' | 'P' | '_' | '^' | 'X') => {
                        let mut taken = 0;
                        while let Some(&c) = chars.peek() {
                            // A line break ends a sequence that was never closed (it is left for the text).
                            if c == '\n' || taken >= ESCAPE_PAYLOAD_MAX {
                                break;
                            }
                            chars.next();
                            taken += 1;
                            if c == '\u{7}' || c == '\u{9c}' {
                                break;
                            }
                            if c == '\u{1b}' {
                                if chars.peek() == Some(&'\\') {
                                    chars.next();
                                }
                                break;
                            }
                        }
                    }
                    _ => {}
                }
            }
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    continue;
                }
                out.push(if style.multiline() { '\n' } else { ' ' });
            }
            '\n' | '\t' if style.multiline() => out.push(c),
            '\n' | '\t' => out.push(' '),
            c if c.is_control() => {}
            // Override and embedding controls: the spoofing risk, never kept.
            '\u{202a}'..='\u{202e}' => {}
            // LRM, RLM, ALM and the isolates: Arabic and Hebrew need them in prose.
            '\u{200e}' | '\u{200f}' | '\u{61c}' | '\u{2066}'..='\u{2069}'
                if !style.keeps_marks() => {}
            c => out.push(c),
        }
    }
    if style.keeps_marks() && out.contains(['\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}']) {
        out = balance_isolates(&out);
    }
    out
}

/// Keeps an isolate (FSI, LRI, RLI) only with the PDI that closes it, and a PDI only after its opener: an unclosed one would reorder
/// everything that follows it in the page.
fn balance_isolates(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut keep = vec![true; chars.len()];
    let mut open: Vec<usize> = Vec::new();
    for (i, c) in chars.iter().enumerate() {
        match c {
            '\u{2066}'..='\u{2068}' => open.push(i),
            '\u{2069}' if open.pop().is_none() => keep[i] = false,
            _ => {}
        }
    }
    for i in open {
        keep[i] = false;
    }
    chars
        .into_iter()
        .zip(keep)
        .filter_map(|(c, k)| k.then_some(c))
        .collect()
}

/// A text from an adapter made safe and bounded: at most `PRE_BOUND` bytes of it are looked at (head and tail), control characters and
/// escape sequences are removed, and it is cut to `max` bytes. `true` when something was cut. Done before any lock is taken.
fn tidy(text: &str, max: usize, style: Style) -> (String, bool) {
    let (bounded, cut_first) = head_and_tail(text.to_string(), PRE_BOUND);
    let (out, cut_last) = head_and_tail(strip_controls(&bounded, style), max);
    (out, cut_first || cut_last)
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

    /// Keeps the ring within its bounds, and forgets what pointed at an entry that left it.
    fn evict(&mut self) {
        while self.entries.len() > MAX_ENTRIES || self.bytes > MAX_BYTES {
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
            while s.evicted_bytes > EVICTED_MAX {
                let Some((e, _)) = s.evicted.pop_front() else {
                    break;
                };
                s.evicted_bytes = s.evicted_bytes.saturating_sub(Self::size(&e));
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
        self.evict();
        if !self.followers.is_empty() {
            let all: Vec<ChatEntry> = self.entries.iter().cloned().collect();
            for e in &all {
                self.send(e);
            }
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
    }

    fn send(&mut self, entry: &ChatEntry) {
        // A follower that fell `FOLLOWER_QUEUE` entries behind is dropped here.
        self.followers
            .retain(|(_, sender)| sender.try_send(entry.clone()).is_ok());
    }

    /// Numbers an entry, keeps it, and hands it to every follower. Returns its id.
    fn add(&mut self, mut entry: ChatEntry, now_ms: u64) -> u64 {
        // Seeding keeps the id far below the end of the range (`MAX_SEEDED_ID`), so this never saturates in practice; if it did, ids
        // would repeat rather than the node panic or wrap under the board's lock.
        self.next_id = self.next_id.saturating_add(1);
        entry.entry_id = self.next_id;
        self.bytes += Self::size(&entry);
        self.entries.push_back(entry.clone());
        self.sent_ms.insert(entry.entry_id, now_ms);
        self.changed(entry.entry_id, Self::size(&entry));
        self.evict();
        self.send(&entry);
        entry.entry_id
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
        let entry = self.entries[i].clone();
        self.bytes = (self.bytes + Self::size(&entry)).saturating_sub(before);
        self.changed(id, Self::size(&entry));
        let due = self
            .sent_ms
            .get(&id)
            .is_none_or(|last| now_ms < *last || now_ms - *last >= COALESCE_MS);
        if coalesce && !due {
            self.dirty.insert(id);
        } else {
            self.dirty.remove(&id);
            self.sent_ms.insert(id, now_ms);
            self.send(&entry);
        }
        self.evict();
        true
    }
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
    e.approval_id = cut_to(std::mem::take(&mut e.approval_id), MAX_CALL_ID);
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
            return false;
        }
        self.seed(agent, seed, now_ms)
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
        let Some(seed) = seed else {
            return;
        };
        let high = seed.high;
        let mut g = self.lock();
        let t = slot(&mut g, agent, self.wake.as_ref());
        t.seed(seed, now_ms);
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
    pub fn replayed(&self, agent: &AgentId, items: &[ChatItem], now_ms: u64) -> bool {
        let holds = self.lock().get(agent).is_some_and(|t| t.next_id > 0);
        if holds {
            return false;
        }
        for item in items {
            self.push_item(agent, item, now_ms);
        }
        true
    }

    /// ADR-0089 decision 1: what changed since the last batch, per agent: the entries as they are now (each once), the changed ones
    /// that left the ring meanwhile, and the ids taken out. Only marks are kept on the chat path: the clones are made here.
    pub fn take_unsaved(&self) -> Vec<Batch> {
        let mut g = self.lock();
        let mut out = Vec::new();
        for (agent, t) in g.iter_mut() {
            let Some(s) = &mut t.save else {
                continue;
            };
            if s.dirty.is_empty()
                && s.evicted.is_empty()
                && s.removed.is_empty()
                && s.fresh.is_none()
            {
                continue;
            }
            let fresh = s.fresh.take();
            let dirty = std::mem::take(&mut s.dirty);
            let evicted = std::mem::take(&mut s.evicted);
            let removed = std::mem::take(&mut s.removed);
            (s.bytes, s.evicted_bytes, s.woken) = (0, 0, false);
            let mut lines: Vec<Line> = evicted
                .into_iter()
                .map(|(e, side)| Line::Entry(Box::new(e), side))
                .collect();
            for id in dirty {
                if let Some(i) = t.index(id) {
                    let e = &t.entries[i];
                    lines.push(Line::Entry(Box::new(e.clone()), t.side(e)));
                }
            }
            if !removed.is_empty() {
                lines.push(Line::Removed(removed));
            }
            out.push(Batch {
                agent: agent.clone(),
                high: t.next_id,
                lines,
                fresh,
            });
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

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<AgentId, Transcript>> {
        self.agents.lock().unwrap_or_else(|e| e.into_inner())
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
        let mut e = entry(role, kind, cut(text), at_ms);
        e.path = path;
        self.insert(agent, e);
    }

    /// The user's own prompt as an entry. Returns its id, which [`Self::retract`] names if the prompt did not go out.
    pub fn push_user(&self, agent: &AgentId, text: String, at_ms: u64) -> u64 {
        self.insert(
            agent,
            entry(ChatRole::User, ChatKind::Text, cut(text), at_ms),
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
        let mut e = entry(ChatRole::User, ChatKind::Text, cut(text), at_ms);
        e.queued = true;
        self.insert(agent, e)
    }

    /// Numbers an entry, keeps it within the ring's bounds and hands it to every follower. Returns its id.
    fn insert(&self, agent: &AgentId, e: ChatEntry) -> u64 {
        let at = e.at_ms;
        let mut g = self.lock();
        slot(&mut g, agent, self.wake.as_ref()).add(e, at)
    }

    /// What an adapter said. A notice whose code has the wrong shape is dropped (ADR-0058).
    pub fn push_item(&self, agent: &AgentId, item: &ChatItem, at_ms: u64) {
        match item {
            ChatItem::Notice { code, params } => {
                // The node's own notices are not the adapter's to make.
                if NODE_NOTICES.contains(&code.as_str()) {
                    return;
                }
                let Some((code, params)) = notice(code, params) else {
                    return;
                };
                let mut e = entry(ChatRole::Agent, ChatKind::Notice, String::new(), at_ms);
                e.code = code;
                e.params = params;
                self.insert(agent, e);
            }
            ChatItem::Thought(text) => {
                // The agent's thinking: an entry of its own, cut at 8 KiB, and nothing else (it is never routed anywhere).
                let (text, _) = tidy(text, MAX_THOUGHT_BYTES, Style::Prose);
                self.insert(
                    agent,
                    entry(ChatRole::Agent, ChatKind::Thought, text, at_ms),
                );
            }
            ChatItem::Tool(tool) => self.upsert_tool(agent, tool, at_ms),
            ChatItem::PlanSteps(steps) => self.upsert_plan(agent, steps, at_ms),
            other => {
                let (kind, text, path) = describe(other);
                self.push(agent, ChatRole::Agent, kind, text, path, at_ms);
            }
        }
    }

    /// ADR-0090: a tool call or an update to one. The first sight of a call id makes the entry; a later one changes it in place: a
    /// field the item carries replaces the field, one it leaves out keeps it. The duration is the node's own: `started_ms` is when
    /// the call was first seen and `ended_ms` when it first reached `completed` or `failed`. Paths are cut to 64; output keeps
    /// its head and tail within 16 KiB; and what a tool printed about a secret file (ADR-0080) is not kept at all.
    fn upsert_tool(&self, agent: &AgentId, item: &ToolItem, now_ms: u64) {
        // An id that does not fit would, cut, name another call: such an item is dropped.
        if item.call_id.is_empty() || item.call_id.len() > MAX_CALL_ID {
            return;
        }
        // Everything that costs work in proportion to the text (cleaning, cutting, formatting, cloning) is done here, before the lock.
        let tool_in = item
            .tool
            .as_ref()
            .map(|x| tidy(x, MAX_PATH_BYTES, Style::Label).0);
        let args_in = item
            .arguments_json
            .as_ref()
            .map(|x| tidy(x, MAX_ENTRY_BYTES, Style::Code).0);
        let kind = tool_kind(&item.kind);
        let status = tool_status(&item.status);
        let paths: Option<Vec<ToolLocation>> = item.locations.as_ref().map(|l| {
            l.iter()
                .take(MAX_PATHS)
                .map(|(path, line)| ToolLocation {
                    path: tidy(path, MAX_PATH_BYTES, Style::Label).0,
                    line: *line,
                })
                .collect()
        });
        let secret_path = paths.as_ref().is_some_and(|p| {
            p.iter()
                .any(|p| agora_core::secrets::is_secret_path(&p.path))
        });
        let output = item
            .output
            .as_ref()
            .map(|o| tidy(o, MAX_OUTPUT_BYTES, Style::Prose));
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
            if secret_path
                || e.paths
                    .iter()
                    .any(|p| agora_core::secrets::is_secret_path(&p.path))
            {
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
                }
            }
            None => {
                let mut e = entry(ChatRole::Agent, ChatKind::ToolCall, String::new(), now_ms);
                e.call_id = call_id.clone();
                e.started_ms = now_ms;
                apply(&mut e, true, text_new);
                let id = t.add(e, now_ms);
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
    fn upsert_plan(&self, agent: &AgentId, steps: &[PlanItem], now_ms: u64) {
        let steps: Vec<PlanStep> = steps
            .iter()
            .take(MAX_PLAN_STEPS)
            .map(|s| PlanStep {
                text: tidy(&s.text, MAX_PLAN_STEP_BYTES, Style::Step).0,
                status: plan_status(&s.status) as i32,
                priority: plan_priority(&s.priority) as i32,
            })
            .collect();
        let text = cut(steps
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join("\n"));
        let mut g = self.lock();
        let t = slot(&mut g, agent, self.wake.as_ref());
        if let Some(id) = t.plan.filter(|id| t.index(*id).is_some()) {
            let (text, steps) = (text.clone(), steps.clone());
            if t.modify(id, now_ms, true, move |e| {
                e.text = text;
                e.plan_steps = steps;
            }) {
                return;
            }
        }
        let mut e = entry(ChatRole::Agent, ChatKind::Plan, text, now_ms);
        e.plan_steps = steps;
        let id = t.add(e, now_ms);
        t.plan = Some(id);
    }

    /// ADR-0090: sends the changes of entries that were held back to keep to one version per [`COALESCE_MS`], once that time is
    /// over. Called by the node's tick; an entry whose last change is final was never held back.
    pub fn flush_due(&self, now_ms: u64) {
        let mut g = self.lock();
        for t in g.values_mut() {
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
                if let Some(i) = t.index(id) {
                    let e = t.entries[i].clone();
                    t.sent_ms.insert(id, now_ms);
                    t.send(&e);
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
        n.params = HashMap::from([(
            "action".to_string(),
            tidy(action, MAX_PERMISSION_BYTES, Style::Label).0,
        )]);
        t.add(n, now_ms);
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
        let mut e = entry(
            ChatRole::Agent,
            ChatKind::Permission,
            tidy(action, MAX_PERMISSION_BYTES, Style::Label).0,
            now_ms,
        );
        e.approval_id = approval_id.to_string();
        e.permission_outcome = PermissionOutcome::Pending as i32;
        self.insert(agent, e);
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

    /// The transcript so far.
    pub fn snapshot(&self, agent: &AgentId) -> Vec<ChatEntry> {
        self.lock()
            .get(agent)
            .map(|t| t.entries.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// The transcript so far and a channel with every entry from now on, taken together so none is missed or repeated. A version of
    /// an entry that changed in place arrives as a new `ChatEntry` with the same `entry_id`.
    pub fn follow(&self, agent: &AgentId) -> (Vec<ChatEntry>, ChatSubscription) {
        let follower_id = self.next_follower_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = sync_channel(FOLLOWER_QUEUE);
        let mut g = self.lock();
        let t = slot(&mut g, agent, self.wake.as_ref());
        let snapshot = t.entries.iter().cloned().collect();
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
        let replay = [ChatItem::Text("one".into()), ChatItem::Text("two".into())];
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
