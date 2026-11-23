//! L2: a bridge from the Agent Client Protocol (ACP) to the adapter protocol (docs/specs/adapter-protocol.md).
//!
//! An agent that speaks ACP over stdio needs no adapter of its own: `agora acp-bridge -- <agent argv>` runs the agent, plays the
//! ACP *client* towards it, and plays an *adapter* towards the node. The node therefore treats an ACP agent exactly like an L3
//! one (chat items, approvals, state, cancel), and nothing in the node, hub or web client knows about ACP. The bridge never
//! offers the agent filesystem or terminal access (`fs/*` and `terminal/*` requests are refused): the agent works in its own
//! worktree with its own tools, and every permission it asks for becomes an approval the user decides.
//!
//! Mapping (ACP → adapter protocol):
//! - `session/update` `agent_message_chunk` → `chat/append` text (chunks are joined for up to 150 ms); `tool_call` → a tool call
//!   item; `plan` → a plan item; a `diff` in a tool call's content → a diff item; `agent_thought_chunk` is dropped.
//! - `session/request_permission` → `approval/request` and state `blocked`, with the answers the agent offers (ADR-0101: `scopes`, the
//!   always option's name and id); `approval/resolve` picks exactly one option: ONCE → `allow_once`, never anything else; ALWAYS →
//!   the one `allow_always`; a denial → `reject_once`, else `cancelled`. A request with no way to allow it raises no card.
//! - A prompt turn: `session/prompt` → state `working`, and the agent's answer (its stop reason) → state `idle`. A turn that failed,
//!   was stopped by the agent (`refusal`, `max_tokens`, ...) or ended with nothing said carries a stable reason code in the state's
//!   `message` and in a `log` line (docs/specs/adapter-protocol.md "Turn and exit reasons"); never text.
//! - `usage_update` → `usage/report` with the cost's growth since the last report and the context's level and size.
//! - The agent exiting: requests the node still waits for are answered with an error whose `data` is `adapter.agent_exited` with the
//!   stage, the exit code and a short tail of the agent's stderr with anything that looks like a secret removed.
//! - ADR-0058: a failed, stopped or empty turn and the agent exiting are also a notice in the conversation (`chat/append` notice);
//!   the `modes` and `models` of `session/new` (and `current_mode_update`) become `session/options`, and the node's
//!   `session/set_option` becomes ACP `session/set_mode` or `session/set_model`.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::ops::ControlFlow;
use std::process::{Child, ChildStderr, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{
    channel, sync_channel, Receiver, RecvTimeoutError, SyncSender, TrySendError,
};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::host::MAX_LINE;
use crate::perm::{self, Category};

/// Both readers share this queue. Store line bytes so queued JSON has not expanded into values.
const INPUT_QUEUE: usize = 8;
/// Commands waiting behind the one being written to the agent. A busy agent that is not reading stdin must survive a burst of
/// permission answers, cancels and mode switches, so the count is generous; the byte budget below is what bounds memory.
const AGENT_WRITE_QUEUE: usize = 64;
/// The most bytes waiting in the writer queue (ADR-0022). Commands are small, but a prompt line can be up to `MAX_LINE`, so the
/// queue also stops at this many bytes; one line over the budget is still accepted when the queue is empty. Worst case memory of
/// the writer: this budget (or one `MAX_LINE` line if larger) queued, plus the line being written, which is at most `MAX_LINE`.
const AGENT_WRITE_BYTES: usize = 8 * 1024 * 1024;
/// How long agent text chunks are joined into one chat item.
const JOIN: Duration = Duration::from_millis(150);
/// The most text one joined item holds: a chunk that would take it past this is sent in an item of its own (Audit F54).
const JOIN_MAX: usize = 4096;
const MAX_DETAIL: usize = 300;
/// The bytes of the agent's stderr kept for an exit report, and the most of them a report carries (after secrets are removed).
const STDERR_KEEP: usize = 4096;
const STDERR_TAIL: usize = 400;
const STDERR_LINES: usize = 5;
/// What stands in for a run of text that looks like a secret.
const REDACTED: &str = "[redacted]";
/// An unbroken run of token characters this long that is not a path is taken for a secret.
const LONG_RUN: usize = 32;
/// Words after which (and `=` or `:`) the next word is taken for a secret.
const SECRET_WORDS: &[&str] = &[
    "key",
    "token",
    "secret",
    "password",
    "passwd",
    "credential",
    "authorization",
    "cookie",
    "session",
];
/// How long the agent gets to finish exiting after its output ended, before it is killed.
const EXIT_WAIT: Duration = Duration::from_secs(2);
/// How long a failed or empty turn waits for the stderr lines the agent wrote just before it answered.
const STDERR_SETTLE: Duration = Duration::from_millis(100);
/// How long the bridge waits for a late request after its agent dies before opening a session.
const LATE_REQUEST_WAIT: Duration = Duration::from_secs(2);
/// ADR-0101 (Codex review of PR #1007): how long a person's answer may wait for the writer's report before the agent is ended (as for
/// a full writer queue) and the node is told its delivery was not confirmed. Expiry does not prove no byte reached the agent (the
/// success report may be queued behind another event), so the agent's termination is requested first and the node records no grant
/// for that run (what the agent saved in its own settings is the agent's, as the client's note says).
/// Shorter than the node's wait for the answer to `approval/resolve`.
const DELIVER_WAIT: Duration = Duration::from_secs(5);
/// ADR-0058: the most modes or models passed on, and the longest id, name and description kept. An option whose id is longer is
/// dropped (a cut id would name another option).
const OPTIONS_MAX: usize = 200;
const OPTION_ID_MAX: usize = 200;
const OPTION_NAME_MAX: usize = 200;
const OPTION_DESCRIPTION_MAX: usize = 500;
enum Ev {
    Node(Input),
    Agent(Input),
    /// ADR-0101: the writer's outcome for a line queued with a report token ([`AgentInput::flush_reported`]): whether it was written
    /// to the agent's stdin and flushed.
    Written(u64, bool),
}

enum Input {
    Line(Box<[u8]>),
    Gone,
    TooLong,
}

enum Waiting {
    /// The node's `initialize` is answered only when the ACP agent has answered its own: a strict agent refuses
    /// `session/new` before that.
    Initialize { node_id: Value },
    NewSession {
        node_id: Value,
        resume_id: Option<String>,
    },
    Prompt {
        cancelling: bool,
        /// Cancelled, and a newer prompt was sent since: its reply ends nothing.
        superseded: bool,
    },
    /// ADR-0090: the node's `session/set_config`, until the agent answered `session/set_config_option`.
    SetConfig {
        node_id: Value,
        session_id: String,
        id: String,
        value: String,
    },
    /// ADR-0058: the node's `session/set_option`, until the agent answered `session/set_mode` or `session/set_model`.
    SetOption {
        node_id: Value,
        session_id: String,
        mode: bool,
        id: String,
    },
}

struct Permission {
    /// The id of the agent's `session/request_permission` request.
    request: Value,
    /// ADR-0101: the options captured when the agent asked, used until the request is answered. `once`: the first `allow_once`;
    /// `always`: the one `allow_always` (none when the agent offered two or more: ambiguous, Codex review of ADR-0101); `reject`:
    /// the first `reject_once`. Nothing else is ever selected (`reject_always` is a remembered refusal, an unknown kind may do
    /// anything).
    once: Option<String>,
    always: Option<String>,
    reject: Option<String>,
}

/// ADR-0101: how far a decision from the node reaches. `None`: a value this build does not know (refused, never guessed).
fn decision_scope(params: &Value) -> Option<bool> {
    match params.get("scope") {
        None | Some(Value::Null) => Some(false),
        Some(Value::String(s)) => match s.as_str() {
            "APPROVAL_SCOPE_UNSPECIFIED" | "APPROVAL_SCOPE_ONCE" => Some(false),
            "APPROVAL_SCOPE_ALWAYS" => Some(true),
            _ => None,
        },
        Some(v) => match v.as_u64() {
            Some(0 | 1) => Some(false),
            Some(2) => Some(true),
            _ => None,
        },
    }
}

fn lines<R: Read + Send + 'static>(r: R, wrap: fn(Input) -> Ev, tx: SyncSender<Ev>) {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(r);
        let end = loop {
            let mut line = Vec::new();
            // One extra byte distinguishes a full line followed by LF from a line that exceeds the limit.
            match (&mut reader)
                .take((MAX_LINE + 1) as u64)
                .read_until(b'\n', &mut line)
            {
                Ok(0) | Err(_) => break Input::Gone,
                Ok(_) => {}
            }
            if line.last() == Some(&b'\n') {
                line.pop();
            }
            if line.len() > MAX_LINE {
                break Input::TooLong;
            }
            if tx.send(wrap(Input::Line(line.into_boxed_slice()))).is_err() {
                return;
            }
        };
        drop(reader);
        let _ = tx.send(wrap(end));
    });
}

fn write_line<W: Write>(w: &mut W, v: &Value) -> io::Result<()> {
    writeln!(w, "{v}")?;
    w.flush()
}

/// The agent's stdin as the bridge writes it: a line is written, then flushed (queued, for the writer thread).
trait AgentInput: Write {
    /// ADR-0101 (Codex review of PR #1007): queues what was written since the last flush, as `flush` does, and has its outcome reported
    /// as [`Ev::Written`]`(token, ok)` once it was written to the agent and flushed (or could not be). `Ok(false)`: this input writes
    /// before it returns (a buffer in tests), so success means the line was given to the agent and nothing is reported.
    fn flush_reported(&mut self, _token: u64) -> io::Result<bool> {
        self.flush().map(|()| false)
    }
}

impl AgentInput for io::Sink {}

/// A queued line and its report token (0: not reported).
type Queued = (Box<[u8]>, u64);

struct AgentWriter {
    line: Vec<u8>,
    pending: SyncSender<Queued>,
    /// Bytes queued and not yet taken by the writer thread.
    queued: Arc<AtomicUsize>,
    /// Set by the writer thread once a write failed: nothing more is queued, and every line still queued is reported failed.
    broken: Arc<AtomicBool>,
}

impl AgentWriter {
    /// Writes to `stdin` on a thread of its own; a line queued with a report token has its outcome sent on `report`.
    fn start<S: Write + Send + 'static>(mut stdin: S, report: SyncSender<Ev>) -> Self {
        let (pending, lines) = sync_channel::<Queued>(AGENT_WRITE_QUEUE);
        let queued = Arc::new(AtomicUsize::new(0));
        let taken = Arc::clone(&queued);
        let broken = Arc::new(AtomicBool::new(false));
        let failed = Arc::clone(&broken);
        std::thread::spawn(move || {
            // Runs until the bridge lets the writer go. After a failed write nothing more is written; the lines still queued are only
            // reported (so no answer waits for an outcome that never comes).
            while let Ok((line, token)) = lines.recv() {
                taken.fetch_sub(line.len(), Ordering::SeqCst);
                let ok = !failed.load(Ordering::SeqCst)
                    && stdin.write_all(&line).and_then(|()| stdin.flush()).is_ok();
                if !ok {
                    failed.store(true, Ordering::SeqCst);
                }
                if token != 0 {
                    // The bridge may be ending (its queue is gone): nobody waits for the outcome then.
                    let _ = report.send(Ev::Written(token, ok));
                }
            }
        });
        Self {
            line: Vec::new(),
            pending,
            queued,
            broken,
        }
    }

    fn queue(&mut self, token: u64) -> io::Result<()> {
        let line = std::mem::take(&mut self.line).into_boxed_slice();
        if self.broken.load(Ordering::SeqCst) {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let len = line.len();
        // Only this thread adds, so the check cannot race another add; the writer thread only subtracts.
        let held = self.queued.load(Ordering::SeqCst);
        if held > 0 && held.saturating_add(len) > AGENT_WRITE_BYTES {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        self.queued.fetch_add(len, Ordering::SeqCst);
        self.pending.try_send((line, token)).map_err(|error| {
            self.queued.fetch_sub(len, Ordering::SeqCst);
            match error {
                TrySendError::Full(_) => io::ErrorKind::WouldBlock,
                TrySendError::Disconnected(_) => io::ErrorKind::BrokenPipe,
            }
            .into()
        })
    }
}

impl Write for AgentWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.line.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.queue(0)
    }
}

impl AgentInput for AgentWriter {
    fn flush_reported(&mut self, token: u64) -> io::Result<bool> {
        self.queue(token).map(|()| true)
    }
}

/// ADR-0101: what became of a line sent with [`Bridge::send_agent_reported`].
#[derive(Debug, PartialEq, Eq)]
enum Delivery {
    /// It could not be queued (or written): the agent was not given it.
    Failed,
    /// It was written to the agent before the send returned.
    Done,
    /// It is queued; [`Ev::Written`] with this token says whether it was written.
    Pending(u64),
}

/// ADR-0101: a node request answered only once the line it sent the agent was written ([`Bridge::written`]), or refused when that
/// takes longer than `by`.
struct Delivering {
    node_id: Value,
    by: Instant,
}

/// A unified diff that replaces `old` by `new` as a whole (ACP gives both texts, not a patch).
pub fn replace_diff(path: &str, old: Option<&str>, new: &str) -> String {
    let count = |s: &str| s.lines().count();
    let old_text = old.unwrap_or("");
    let mut out = format!(
        "--- {}\n+++ b/{path}\n@@ -{},{} +{},{} @@\n",
        if old.is_some() {
            format!("a/{path}")
        } else {
            "/dev/null".to_string()
        },
        usize::from(count(old_text) > 0),
        count(old_text),
        usize::from(count(new) > 0),
        count(new),
    );
    for (prefix, text) in [('-', old_text), ('+', new)] {
        for line in text.split_inclusive('\n') {
            out.push(prefix);
            out.push_str(line);
            if !line.ends_with('\n') {
                out.push_str("\n\\ No newline at end of file\n");
            }
        }
    }
    out
}

fn short(v: &Value) -> String {
    let s = v.to_string();
    if s.len() <= MAX_DETAIL {
        return s;
    }
    let mut end = MAX_DETAIL;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// The longest model id kept from the agent.
const MODEL_MAX: usize = 200;

fn clip(s: &str, max: usize) -> String {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// An ACP cost `{amount, currency}` as (currency, micro-units). `None` for anything that is not a finite, non-negative amount in a
/// three-letter currency.
fn cost_micros(cost: &Value) -> Option<(String, u64)> {
    let amount = cost.get("amount")?.as_f64()?;
    let currency = cost.get("currency")?.as_str()?;
    if !amount.is_finite()
        || amount < 0.0
        || currency.len() != 3
        || !currency.bytes().all(|b| b.is_ascii_alphabetic())
    {
        return None;
    }
    let micros = (amount * 1_000_000.0).round();
    // In range by the check: `as` saturates anyway, so a huge amount cannot wrap.
    let micros = if micros >= u64::MAX as f64 {
        u64::MAX
    } else {
        micros as u64
    };
    Some((currency.to_ascii_uppercase(), micros))
}

/// How a prompt turn ended when that needs saying: a stable code and its parameters (docs/specs/adapter-protocol.md).
/// `None` for a turn that ended normally or was cancelled.
fn turn_outcome(answer: &Value, said: bool) -> Option<(&'static str, Value)> {
    if let Some(e) = answer.get("error") {
        let code = e.get("code").and_then(Value::as_i64).unwrap_or(0);
        if code == -32800 {
            return None; // ACP Request Cancelled.
        }
        return Some(("adapter.turn_failed", json!({"rpc_code": code.to_string()})));
    }
    match answer
        .pointer("/result/stopReason")
        .and_then(Value::as_str)
        .unwrap_or("")
    {
        "cancelled" => None,
        "end_turn" | "" if !said => Some(("adapter.turn_empty", json!({}))),
        "end_turn" | "" => None,
        other => {
            // The agent's word, kept only if it looks like one of ACP's stop reasons.
            let reason = if other.len() <= 40
                && other.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
            {
                other
            } else {
                "other"
            };
            Some(("adapter.turn_stopped", json!({"stop_reason": reason})))
        }
    }
}

/// ADR-0058: an ACP mode list (`availableModes` of `{id, name, description}`) or model list (`availableModels` of `{modelId, name,
/// description}`) as adapter-protocol options, bounded.
fn acp_options(list: Option<&Value>, id_key: &str, mode: bool) -> Vec<Value> {
    let text =
        |o: &Value, k: &str, max: usize| clip(o.get(k).and_then(Value::as_str).unwrap_or(""), max);
    list.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|o| {
                    let id = o.get(id_key)?.as_str()?;
                    if id.is_empty() || id.len() > OPTION_ID_MAX {
                        return None;
                    }
                    let name = text(o, "name", OPTION_NAME_MAX);
                    let mut v = json!({"id": id, "name": name,
                                       "description": text(o, "description", OPTION_DESCRIPTION_MAX)});
                    // ADR-0090: modes and models alike; a mode that is not clearly one that asks is flagged too (`perm`).
                    let (category, option) = if mode {
                        (Category::Mode, ("mode", "mode"))
                    } else {
                        (Category::Model, ("model", "model"))
                    };
                    let description = o.get("description").and_then(Value::as_str).unwrap_or("");
                    if perm::value_skips(category, option, (id, &name, description)) {
                        v["skipsPermissions"] = json!(true);
                    }
                    Some(v)
                })
                .take(OPTIONS_MAX)
                .collect()
        })
        .unwrap_or_default()
}

/// ADR-0090: the most config options passed on, and the most choices of one; the node bounds them again.
const CONFIG_OPTIONS_MAX: usize = 64;
const CONFIG_CHOICES_MAX: usize = 64;
/// ADR-0090: the most slash commands passed on.
const COMMANDS_MAX: usize = 100;
/// ADR-0090: the most paths of one tool call passed on.
const LOCATIONS_MAX: usize = 64;
/// ADR-0090: a tool's output is cut here (head and tail kept) before secrets are removed; the node cuts it to its own cap after.
const OUTPUT_MAX: usize = 64 * 1024;
/// Audit F54: a tool call's arguments and a diff are cut here (head and tail kept), and a rich plan to this many entries of this much
/// text (each is sent twice, as a step and as an entry), so an ordinary item stays far below `MAX_LINE` on its way to the node,
/// whose host drops a longer line whole; [`fit_line`] enforces the bound on the line itself. The node keeps less of each (16 KiB, 64
/// entries of 512 bytes).
const ARGUMENTS_MAX: usize = 64 * 1024;
const DIFF_MAX: usize = 64 * 1024;
const PLAN_ENTRIES_MAX: usize = 256;
const PLAN_TEXT_MAX: usize = 4096;

/// Audit F54 (review of #920): a string at most this long is never cut by [`fit_line`].
const FIT_FLOOR: usize = 256;
/// Audit F54 (Codex review of #920): the longest session id the bridge accepts, from the agent (`session/new`, `session/load`) or
/// from the node (`resumeSessionId`). Longer ones are refused, not cut: an item cut down to strings of `FIT_FLOOR` bytes and lists of
/// one element is then a few KiB, so [`fit_line`] always finds a line that fits.
const SESSION_ID_MAX: usize = 1024;

/// A `chat/append` line with `params`, serialized as the node will read it. The per-field caps above count decoded bytes, while
/// JSON escaping can make each one up to six bytes and a rich plan carries its text twice, so the bound is enforced here on the
/// complete line, envelope and escaping included: while it is longer than `MAX_LINE`, every string of the item longer than
/// `FIT_FLOOR` (head and tail kept) and every list of more than one element is cut to half, the session id excepted. A line that
/// fits, which is every ordinary one, is sent as it is. Cutting ends once nothing is left to cut; with the session id at most
/// `SESSION_ID_MAX` the line then fits. Should it not, `Err` with the length reached: the caller drops the item and logs why, so an
/// overlong line is never sent.
fn fit_line(params: Value) -> Result<String, usize> {
    let mut line = json!({"jsonrpc": "2.0", "method": "chat/append", "params": params});
    loop {
        let text = line.to_string();
        if text.len() <= MAX_LINE {
            return Ok(text);
        }
        let Some(params) = line.get_mut("params").and_then(Value::as_object_mut) else {
            return Err(text.len());
        };
        let mut cut = false;
        for (k, v) in params.iter_mut() {
            if k != "sessionId" {
                cut |= halve(v);
            }
        }
        if !cut {
            return Err(text.len());
        }
    }
}

/// Cuts every string in `v` longer than `FIT_FLOOR` and every list of more than one element to half; whether anything was cut.
fn halve(v: &mut Value) -> bool {
    match v {
        Value::String(s) if s.len() > FIT_FLOOR => {
            *s = head_tail(s, s.len() / 2);
            true
        }
        Value::Array(a) => {
            let mut cut = a.len() > 1;
            a.truncate(a.len().div_ceil(2));
            for e in a.iter_mut() {
                cut |= halve(e);
            }
            cut
        }
        Value::Object(o) => o.values_mut().fold(false, |cut, e| halve(e) | cut),
        _ => false,
    }
}

/// `s` as is when it fits `max` bytes, otherwise its first and last `max / 2` bytes (cut on character boundaries).
fn head_tail(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let half = max / 2;
    let mut head = half;
    while !s.is_char_boundary(head) {
        head -= 1;
    }
    let mut tail = s.len() - half;
    while !s.is_char_boundary(tail) {
        tail += 1;
    }
    format!("{}{}", &s[..head], &s[tail..])
}

/// ADR-0090: ACP `configOptions` as adapter-protocol options (`session/config`): `{id, name, description, category, type, current,
/// choices[], skipsPermissions}`, bounded. Choices of a grouped select are flattened. `skipsPermissions` follows `perm` (the
/// agent's category and words, with an unknown value in a permission option counted as skipping); for a boolean it is set on the
/// option, for a select on each choice.
fn acp_config(list: Option<&Value>) -> Vec<Value> {
    let text =
        |o: &Value, k: &str, max: usize| clip(o.get(k).and_then(Value::as_str).unwrap_or(""), max);
    let choice = |category: Category, option: (&str, &str), o: &Value| -> Option<Value> {
        let id = o.get("value")?.as_str()?;
        if id.is_empty() || id.len() > OPTION_ID_MAX {
            return None;
        }
        let name = text(o, "name", OPTION_NAME_MAX);
        let description = text(o, "description", OPTION_DESCRIPTION_MAX);
        let mut v = json!({"id": id, "name": name, "description": description});
        if perm::value_skips(category, option, (id, &name, &description)) {
            v["skipsPermissions"] = json!(true);
        }
        Some(v)
    };
    list.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|o| {
                    let id = o.get("id")?.as_str()?;
                    if id.is_empty() || id.len() > OPTION_ID_MAX {
                        return None;
                    }
                    let name = text(o, "name", OPTION_NAME_MAX);
                    let description = text(o, "description", OPTION_DESCRIPTION_MAX);
                    let word = o
                        .get("category")
                        .and_then(Value::as_str)
                        .map(|c| clip(c, 40));
                    let category = Category::from_acp(word.as_deref());
                    let mut v = json!({"id": id, "name": name, "description": description,
                                       "category": word.clone().unwrap_or_default()});
                    if o.get("type").and_then(Value::as_str) == Some("boolean") {
                        v["type"] = json!("boolean");
                        v["current"] = json!(o
                            .get("currentValue")
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                            .to_string());
                        v["choices"] = json!([]);
                        if perm::boolean_skips(category, (id, &name), &description) {
                            v["skipsPermissions"] = json!(true);
                        }
                    } else {
                        v["type"] = json!("select");
                        v["current"] = json!(clip(
                            o.get("currentValue").and_then(Value::as_str).unwrap_or(""),
                            OPTION_ID_MAX
                        ));
                        let flat: Vec<&Value> = o
                            .get("options")
                            .and_then(Value::as_array)
                            .map(|l| {
                                l.iter()
                                    .flat_map(|x| {
                                        match x.get("options").and_then(Value::as_array) {
                                            Some(group) => group.iter().collect::<Vec<_>>(),
                                            None => vec![x],
                                        }
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        v["choices"] = json!(flat
                            .into_iter()
                            .filter_map(|c| choice(category, (id, &name), c))
                            .take(CONFIG_CHOICES_MAX)
                            .collect::<Vec<_>>());
                    }
                    Some(v)
                })
                .take(CONFIG_OPTIONS_MAX)
                .collect()
        })
        .unwrap_or_default()
}

/// ADR-0090: the text a tool call produced, from an ACP `content` array (`content` blocks of type text) or, failing that, its
/// `rawOutput`: bounded, with anything that looks like a secret replaced (the same removal as for a stderr tail). `None` when the
/// update carries neither.
fn tool_output(u: &Value) -> Option<String> {
    let from_content = u.get("content").and_then(Value::as_array).map(|a| {
        a.iter()
            .filter(|c| c.get("type").and_then(Value::as_str) == Some("content"))
            .filter_map(|c| c.pointer("/content/text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n")
    });
    let raw = u
        .get("rawOutput")
        .filter(|v| !v.is_null())
        .map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        });
    // Text blocks win; a diff-only `content` carries no output (the diff has its own item), so `rawOutput` is used, or nothing.
    let text = match (from_content, raw) {
        (Some(c), _) if !c.is_empty() => c,
        (_, Some(r)) => r,
        _ => return None,
    };
    Some(bounded_output(&text))
}

/// What the bridge sends of a tool's output: its escape sequences and control characters removed first (the node removes them
/// anyway), then cut to `OUTPUT_MAX` (head and tail kept), then what looks like a secret replaced. Removed before the cut, so the
/// node, which keeps less of it (16 KiB, head and tail), never keeps the place this cut joins: a value of the node's secrets cut
/// there would otherwise leave a piece on each side that the node cannot place (review of #1103, N1).
pub fn bounded_output(text: &str) -> String {
    redact(&head_tail(&without_escapes(text), OUTPUT_MAX))
}

/// `text` without escape sequences (CSI, OSC, DCS, ... with their payload), control characters other than line breaks and tabs,
/// and the characters that override a direction (U+202A to U+202E): what the node removes from a tool's output.
fn without_escapes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
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
                        // As the node: a sequence never closed ends at a line break or after 4096 characters.
                        let mut taken = 0;
                        while let Some(&c) = chars.peek() {
                            if c == '\n' || taken >= 4096 {
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
            '\n' | '\t' | '\r' => out.push(c),
            c if c.is_control() => {}
            '\u{202a}'..='\u{202e}' => {}
            c => out.push(c),
        }
    }
    out
}

/// A notice's parameters: a reason's parameters with every value as a string.
fn notice_params(params: &Value) -> Value {
    let mut out = serde_json::Map::new();
    if let Some(m) = params.as_object() {
        for (k, v) in m {
            let v = match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            out.insert(k.clone(), Value::String(v));
        }
    }
    Value::Object(out)
}

/// Replaces anything in `s` that looks like a secret: a long run of letters and digits (a key, a token, an id), and the word after
/// `key=`, `token:`, `Bearer ` and the like.
fn redact(s: &str) -> String {
    redact_words(&redact_runs(&redact_userinfo(s)))
}

/// `scheme://user:password@host` → `scheme://[redacted]@host`.
fn redact_userinfo(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("://") {
        let (head, tail) = rest.split_at(i + 3);
        out.push_str(head);
        let end = tail
            .find(|c: char| c == '/' || c == '?' || c == '#' || c.is_whitespace())
            .unwrap_or(tail.len());
        match tail[..end].rfind('@') {
            Some(at) => {
                out.push_str(REDACTED);
                out.push_str(&tail[at..end]);
            }
            None => out.push_str(&tail[..end]),
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// A token character: what base64, base64url, hex and most API keys are made of (and paths, which are kept).
fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+' | '/' | '=' | '.')
}

/// Replaces every unbroken run of `LONG_RUN` or more token characters that is not a path, whatever letters it is made of: a
/// secret need not contain a digit. A path starts with `/`, `~` or `.`, or has `/` between parts all shorter than `LONG_RUN`
/// and no `+` or `=` (which base64 has and paths rarely do).
fn redact_runs(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(c) = rest.chars().next() {
        if !is_token_char(c) {
            out.push(c);
            rest = &rest[c.len_utf8()..];
            continue;
        }
        let end = rest.find(|c: char| !is_token_char(c)).unwrap_or(rest.len());
        let run = &rest[..end];
        let path = run.starts_with(['/', '~', '.'])
            || (run.contains('/')
                && !run.contains(['+', '='])
                && run.split('/').all(|p| p.len() < LONG_RUN));
        out.push_str(if run.len() >= LONG_RUN && !path {
            REDACTED
        } else {
            run
        });
        rest = &rest[end..];
    }
    out
}

/// Long words with letters and digits, and the word after `key=`, `token:`, `Bearer ` and the like.
fn redact_words(s: &str) -> String {
    let is_word = |c: char| c.is_alphanumeric() || matches!(c, '_' | '-' | '+');
    let mut out = String::with_capacity(s.len());
    let (mut keyword, mut armed) = (false, false);
    let mut rest = s;
    while let Some(c) = rest.chars().next() {
        if is_word(c) {
            let end = rest.find(|c: char| !is_word(c)).unwrap_or(rest.len());
            let word = &rest[..end];
            let long = word.chars().count() >= 20
                && word.chars().any(|c| c.is_ascii_digit())
                && word.chars().any(char::is_alphabetic);
            out.push_str(if armed || long { REDACTED } else { word });
            let lower = word.to_ascii_lowercase();
            keyword = SECRET_WORDS.iter().any(|w| lower.contains(w));
            armed = lower == "bearer" || lower == "basic";
            rest = &rest[end..];
        } else {
            out.push(c);
            match c {
                '=' | ':' => armed = armed || keyword,
                ' ' | '"' | '\'' => {}
                _ => (keyword, armed) = (false, false),
            }
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

/// The last lines of what the agent wrote to stderr, as a short report: control characters and escape sequences removed, the home
/// folder shown as `~`, secrets replaced, at most `STDERR_LINES` lines and `STDERR_TAIL` bytes (cut at the front).
pub fn stderr_tail(raw: &[u8], home: Option<&str>) -> String {
    let text = String::from_utf8_lossy(raw);
    let mut plain = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            // ESC [ ... final byte (CSI); ESC ] (OSC), ESC P, ESC _ and ESC ^ with their whole payload, up to BEL or ESC \;
            // otherwise ESC and one more character. The 8-bit forms (CSI U+009B, OSC U+009D) are read the same way.
            '\u{1b}' | '\u{9b}' | '\u{9d}' => {
                let kind = if c == '\u{1b}' {
                    chars.next()
                } else if c == '\u{9b}' {
                    Some('[')
                } else {
                    Some(']')
                };
                match kind {
                    Some('[') => {
                        for c in chars.by_ref() {
                            if ('@'..='~').contains(&c) {
                                break;
                            }
                        }
                    }
                    Some(']' | 'P' | '_' | '^') => {
                        while let Some(c) = chars.next() {
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
            '\n' => plain.push('\n'),
            '\t' => plain.push(' '),
            c if c.is_control() => {}
            c => plain.push(c),
        }
    }
    let home = home.filter(|h| h.len() > 1);
    let lines: Vec<String> = plain
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| {
            let l = match home {
                Some(h) => replace_home(l, h),
                None => l.to_string(),
            };
            // The bridge runs without `HOME` under a node (its environment is cleared), so any user's home folder is replaced too.
            redact(&replace_any_home(&l))
        })
        .collect();
    // An agent that logs at INFO as well (opencode with `--print-logs`) would bury its error under routine lines: when some lines name
    // a level of ERROR, FATAL or WARN or a failing HTTP status, those are the ones kept; otherwise the last lines are.
    let serious: Vec<&String> = lines.iter().filter(|l| names_a_problem(l)).collect();
    let kept: Vec<&String> = if serious.is_empty() {
        lines.iter().collect()
    } else {
        serious
    };
    let tail = kept[kept.len().saturating_sub(STDERR_LINES)..]
        .iter()
        .map(|l| l.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    if tail.len() <= STDERR_TAIL {
        return tail;
    }
    let mut start = tail.len() - STDERR_TAIL;
    while !tail.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", &tail[start..])
}

/// Characters that end a user name in a path when no `/` or `\` follows it.
const NAME_END: &[char] = &[
    '"', '\'', '<', '>', '|', ':', ';', ',', '(', ')', '[', ']', '`',
];

/// The length of the user name at the start of `rest` (after `/Users/`, `/home/`, `C:\Users\` ...), or 0 when there is none. The
/// name runs to the next `/` or `\`; one with spaces (macOS and Windows allow them) only when that separator follows within 40 bytes and
/// 3 words; without a separator after it, the name ends at a space or punctuation.
fn user_name_len(rest: &str, spaces: bool) -> usize {
    let stop = |c: char| c == '/' || c == '\\';
    if let Some(sep) = rest.find(stop) {
        let seg = &rest[..sep];
        let ok = !seg.is_empty()
            && !seg.contains(NAME_END)
            && (!seg.contains(' ')
                || (spaces
                    && seg.len() <= 40
                    && seg.split(' ').count() <= 3
                    && !seg.starts_with(' ')));
        if ok {
            return sep;
        }
    }
    rest.find(|c: char| c.is_whitespace() || stop(c) || NAME_END.contains(&c))
        .unwrap_or(rest.len())
}

/// Replaces any user's home folder with `~`, whatever the bridge's own `HOME` is: `/Users/<name>`, `/home/<name>`, `/root`,
/// `C:\Users\<name>`, `C:/Users/<name>` and `/c/Users/<name>` (drive and `Users` in any case).
pub fn replace_any_home(line: &str) -> String {
    let lower = line.to_ascii_lowercase();
    let b = lower.as_bytes();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    let boundary = |at: usize| {
        at == 0 || !(b[at - 1].is_ascii_alphanumeric() || matches!(b[at - 1], b'_' | b'.' | b'-'))
    };
    while i < line.len() {
        if !line.is_char_boundary(i) {
            i += 1;
            continue;
        }
        let at = &lower[i..];
        // (prefix length, whether the name may hold spaces); a drive letter form first, so `C:/Users/` is not read as `/Users/`.
        let drive = b[i].is_ascii_alphabetic()
            && (at[1..].starts_with(":\\users\\") || at[1..].starts_with(":/users/"));
        let git_bash = at.starts_with('/')
            && b.get(i + 1).is_some_and(u8::is_ascii_alphabetic)
            && at[2..].starts_with("/users/");
        let prefix = if boundary(i) && (drive || git_bash) {
            Some((9, true))
        } else if at.starts_with("/users/") && boundary(i) {
            Some((7, true))
        } else if at.starts_with("/home/") && boundary(i) {
            Some((6, false))
        } else if at.starts_with("/root")
            && boundary(i)
            && !b
                .get(i + 5)
                .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'-'))
        {
            out.push('~');
            i += 5;
            continue;
        } else {
            None
        };
        if let Some((len, spaces)) = prefix {
            let name = user_name_len(&line[i + len..], spaces);
            if name > 0 {
                out.push('~');
                i += len + name;
                continue;
            }
        }
        let c = line[i..].chars().next().unwrap_or(' ');
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// Whether a log line names a level that means something went wrong (`ERROR`, `FATAL`, `WARN`, `WARNING`) as a whole word in capitals.
fn names_a_problem(line: &str) -> bool {
    line.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|w| matches!(w, "ERROR" | "FATAL" | "WARN" | "WARNING"))
        || has_failing_http_status(line)
}

/// `http.status=503` or `status=404` in a log line: opencode 2.x logs a failure as an INFO line carrying only such a status.
fn has_failing_http_status(line: &str) -> bool {
    line.match_indices("status=").any(|(at, key)| {
        let digits: String = line[at + key.len()..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        digits.len() == 3 && matches!(digits.as_bytes()[0], b'4' | b'5')
    })
}

/// Replaces the home folder with `~` in every spelling a tool may print: as given, with `/` instead of `\\`, as the Git Bash form
/// (`/c/Users/name`) of a drive path, and ignoring ASCII case (Windows paths do not care about it).
fn replace_home(line: &str, home: &str) -> String {
    let mut spellings = vec![home.to_string(), home.replace('\\', "/")];
    let b = home.as_bytes();
    if b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        spellings.push(format!(
            "/{}{}",
            home[..1].to_ascii_lowercase(),
            home[2..].replace('\\', "/")
        ));
    }
    spellings.dedup();
    let mut out = line.to_string();
    for sp in spellings {
        out = replace_ignoring_ascii_case(&out, &sp, "~");
    }
    out
}

fn replace_ignoring_ascii_case(text: &str, needle: &str, with: &str) -> String {
    if needle.is_empty() {
        return text.to_string();
    }
    let (t, n) = (text.as_bytes(), needle.as_bytes());
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < t.len() {
        if i + n.len() <= t.len()
            && t[i..i + n.len()].eq_ignore_ascii_case(n)
            && text.is_char_boundary(i)
            && text.is_char_boundary(i + n.len())
        {
            out.push_str(with);
            i += n.len();
        } else {
            let step = text[i..].chars().next().map_or(1, char::len_utf8);
            out.push_str(&text[i..i + step]);
            i += step;
        }
    }
    out
}

/// Copies the agent's stderr to the bridge's own (diagnostics for whoever runs the node, as before) and keeps its last bytes. The
/// receiver hears when the stream ended.
/// The last bytes of the agent's stderr and how many it wrote in all (so a turn can take what was written during it).
#[derive(Default)]
struct Kept {
    buf: Vec<u8>,
    total: u64,
}

impl Kept {
    /// What was written since `mark` (a `total` taken earlier), as far as it is still kept.
    fn since(&self, mark: u64) -> &[u8] {
        let new = usize::try_from(self.total.saturating_sub(mark)).unwrap_or(usize::MAX);
        &self.buf[self.buf.len().saturating_sub(new)..]
    }
}

fn keep_stderr(r: ChildStderr, kept: Arc<Mutex<Kept>>) -> Receiver<()> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let mut r = r;
        let mut buf = [0u8; 1024];
        loop {
            let n = match r.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let _ = io::stderr().write_all(&buf[..n]);
            let mut k = kept.lock().unwrap_or_else(|e| e.into_inner());
            k.buf.extend_from_slice(&buf[..n]);
            k.total = k.total.saturating_add(n as u64);
            let over = k.buf.len().saturating_sub(STDERR_KEEP);
            k.buf.drain(..over);
        }
        let _ = tx.send(());
    });
    rx
}

/// Waits a little for an agent whose output ended to exit, then kills it. Its exit code when it exited on its own with one.
fn wait_briefly(child: &mut Child) -> Option<i32> {
    let end = Instant::now() + EXIT_WAIT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.code(),
            Ok(None) if Instant::now() < end => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// The most MCP servers passed on to the agent; the node passes one (the agent's mesh server).
const MCP_SERVERS_MAX: usize = 16;

/// `session/start` `mcpServers` (adapter protocol: `{name, command, args}`) in the ACP shape of a stdio MCP server:
/// `{name, command, args, env: []}`. An entry without a name or a command, or with an argument that is not a string, is dropped.
fn acp_mcp_servers(from_node: Option<&Value>) -> Vec<Value> {
    let Some(list) = from_node.and_then(Value::as_array) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|m| {
            let name = m
                .get("name")
                .and_then(Value::as_str)
                .filter(|n| !n.is_empty())?;
            let command = m
                .get("command")
                .and_then(Value::as_str)
                .filter(|c| !c.is_empty())?;
            let args = match m.get("args") {
                None | Some(Value::Null) => Vec::new(),
                Some(a) => a
                    .as_array()?
                    .iter()
                    .map(|x| x.as_str().map(str::to_string))
                    .collect::<Option<Vec<_>>>()?,
            };
            Some(json!({"name": name, "command": command, "args": args, "env": []}))
        })
        .take(MCP_SERVERS_MAX)
        .collect()
}

#[derive(Default)]
struct SessionState {
    /// The session's cumulative cost (currency, micro-units).
    cost: Option<(String, u64)>,
    context: (u64, u64),
    model: String,
    mode: String,
}

/// ADR-0090: what the bridge keeps only for a node that said it takes rich chat (`nodeCapabilities` has `chat.rich`). Without it the
/// bridge behaves as before: thinking, tool status, config options and commands are dropped.
#[derive(Default)]
struct Rich {
    on: bool,
    /// The buffered text is the agent's thinking.
    thought: bool,
    /// The agent's config options as last reported (adapter-protocol form), to check what the node asks to set.
    config: Vec<Value>,
    /// The id of the config option the mode (or model) list was built from, when the agent gave no `modes` (or `models`) of its own.
    modes_from: Option<String>,
    models_from: Option<String>,
}

struct Bridge<W: Write> {
    out: W,
    /// The agent's stdin (a buffer in tests, so what the agent was sent can be read on every platform).
    agent: Box<dyn AgentInput>,
    agent_writer_full: bool,
    /// ADR-0101: the last report token given out ([`Self::send_agent_reported`]); 0 is never one.
    next_report: u64,
    /// ADR-0101: the node requests (`approval/resolve`) waiting for the writer's outcome of their line, by report token.
    delivering: HashMap<u64, Delivering>,
    next: u64,
    waiting: HashMap<u64, Waiting>,
    session: Option<String>,
    perms: HashMap<String, Permission>,
    text: String,
    text_session: String,
    text_since: Instant,
    initialized: bool,
    can_load: bool,
    /// The agent takes stdio MCP servers in `session/new` and `session/load`. ACP requires every agent to, so this is true unless
    /// the agent's `initialize` says `mcpCapabilities.stdio: false`.
    mcp_stdio: bool,
    /// Something reached the node during the current turn (text, a tool call, a plan, a diff, an approval).
    said: bool,
    session_state: HashMap<String, SessionState>,
    /// ADR-0058: the modes and models offered by the active session.
    modes: Vec<Value>,
    models: Vec<Value>,
    /// The agent's stderr, how much of it there was when the current turn began, and the bridge's own home folder (if it has one).
    stderr: Arc<Mutex<Kept>>,
    turn_mark: u64,
    home: Option<String>,
    rich: Rich,
}

impl<W: Write> Bridge<W> {
    /// What the agent wrote to stderr during the current turn, as a report (scrubbed and size-limited like an exit's tail). Its
    /// stderr is read by another thread, so a moment is given for the lines written just before the answer.
    fn turn_stderr(&self) -> String {
        std::thread::sleep(STDERR_SETTLE);
        let k = self.stderr.lock().unwrap_or_else(|e| e.into_inner());
        stderr_tail(k.since(self.turn_mark), self.home.as_deref())
    }

    fn send_node(&mut self, v: &Value) -> io::Result<()> {
        write_line(&mut self.out, v)
    }

    fn note(&mut self, method: &str, params: Value) -> io::Result<()> {
        self.send_node(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    fn reply(&mut self, id: &Value, result: Value) -> io::Result<()> {
        self.send_node(&json!({"jsonrpc": "2.0", "id": id, "result": result}))
    }

    fn state(&mut self, s: &str) -> io::Result<()> {
        self.state_because(s, "")
    }

    /// A state with a stable reason code in its diagnostic `message` (never text).
    fn state_because(&mut self, s: &str, reason: &str) -> io::Result<()> {
        let sid = self.session.clone().unwrap_or_default();
        // The adapter protocol carries `AgentState` as its proto JSON name; the node reads anything else as unknown.
        let mut p = json!({"sessionId": sid, "state": format!("AGENT_STATE_{s}")});
        if !reason.is_empty() {
            p["message"] = json!(reason);
        }
        self.note("state/update", p)
    }

    /// A `log` line carrying a stable reason: `{"code": ..., "params": {...}}` as its message.
    fn log_reason(&mut self, level: &str, code: &str, params: &Value) -> io::Result<()> {
        let message = json!({"code": code, "params": params}).to_string();
        self.note(
            "log",
            json!({"level": format!("LOG_LEVEL_{level}"), "message": message}),
        )
    }

    /// ADR-0058: a reason as a notice in the conversation. It does not count as the agent saying something.
    fn notice(&mut self, code: &str, params: &Value) -> io::Result<()> {
        let sid = self.session.clone().unwrap_or_default();
        self.note(
            "chat/append",
            json!({"sessionId": sid, "notice": {"code": code, "params": notice_params(params)}}),
        )
    }

    /// ADR-0058: what the agent offers and uses, sent whenever it changes.
    fn options(&mut self) -> io::Result<()> {
        let Some(sid) = self.session.as_deref() else {
            return Ok(());
        };
        let (mode, model) = self
            .session_state
            .get(sid)
            .map(|state| (state.mode.as_str(), state.model.as_str()))
            .unwrap_or_default();
        self.note(
            "session/options",
            json!({"sessionId": sid, "modes": &self.modes, "models": &self.models, "currentMode": mode, "currentModel": model}),
        )
    }

    fn chat(&mut self, item: Value, replay_id: Option<&str>) -> io::Result<()> {
        self.said = true;
        let sid = replay_id.or(self.session.as_deref()).unwrap_or_default();
        let mut p = item;
        p["sessionId"] = json!(sid);
        self.append(p)
    }

    /// A `chat/append` of an item made from the agent's content, kept within the node's line limit (audit F54, [`fit_line`]).
    fn append(&mut self, params: Value) -> io::Result<()> {
        match fit_line(params) {
            Ok(line) => {
                writeln!(self.out, "{line}")?;
                self.out.flush()
            }
            // Not reached with ids within `SESSION_ID_MAX`; if it were, the item is dropped with a stable reason, not sent too long.
            Err(bytes) => self.log_reason(
                "WARN",
                "adapter.line_too_long",
                &json!({"method": "chat/append", "bytes": bytes.to_string()}),
            ),
        }
    }

    fn flush_text(&mut self) -> io::Result<()> {
        if self.text.is_empty() {
            return Ok(());
        }
        let t = std::mem::take(&mut self.text);
        let sid = std::mem::take(&mut self.text_session);
        if std::mem::take(&mut self.rich.thought) {
            // Thinking is not the agent saying something: a turn of nothing else is still an empty turn.
            let mut p = json!({"thought": {"text": t}});
            p["sessionId"] = json!(sid);
            return self.append(p);
        }
        self.chat(json!({"text": {"text": t}}), Some(&sid))
    }

    fn flush_on_node_eof(&mut self) -> io::Result<()> {
        match self.flush_text() {
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
            result => result,
        }
    }

    /// Queue a line to the agent. If its writer has fallen behind, the dispatcher ends the agent via the normal exit path.
    /// A broken pipe is reported when the agent's output ends, with its own exit status and stderr.
    fn send_agent(&mut self, v: &Value) -> bool {
        if self.agent_writer_full {
            return false;
        }
        match write_line(&mut self.agent, v) {
            Ok(()) => true,
            Err(e) => {
                self.agent_writer_full = e.kind() == io::ErrorKind::WouldBlock;
                false
            }
        }
    }

    /// ADR-0101 (Codex review of PR #1007): [`Self::send_agent`] for a line whose delivery the node is told about: the caller answers
    /// the node only once the line was written to the agent (now for [`Delivery::Done`], on [`Ev::Written`] for
    /// [`Delivery::Pending`]).
    fn send_agent_reported(&mut self, v: &Value) -> Delivery {
        if self.agent_writer_full {
            return Delivery::Failed;
        }
        self.next_report += 1;
        let token = self.next_report;
        match writeln!(self.agent, "{v}").and_then(|()| self.agent.flush_reported(token)) {
            Ok(true) => Delivery::Pending(token),
            Ok(false) => Delivery::Done,
            Err(e) => {
                self.agent_writer_full = e.kind() == io::ErrorKind::WouldBlock;
                Delivery::Failed
            }
        }
    }

    /// The node's request `id` sent the agent something that did not reach it.
    fn undelivered(&mut self, id: &Value) -> io::Result<()> {
        self.send_node(&json!({"jsonrpc": "2.0", "id": id,
            "error": {"code": -32603, "message": "adapter.approval_undelivered"}}))
    }

    /// ADR-0101: the writer's outcome for the line of report `token`: the node request waiting for it is answered (success only when
    /// the line was written to the agent and flushed).
    fn written(&mut self, token: u64, ok: bool) -> io::Result<()> {
        let Some(d) = self.delivering.remove(&token) else {
            return Ok(());
        };
        if ok {
            self.reply(&d.node_id, json!({}))
        } else {
            self.undelivered(&d.node_id)
        }
    }

    /// ADR-0101: takes every node request whose line waited longer than [`DELIVER_WAIT`] for the writer's report, oldest first. The
    /// caller ends the agent before [`Self::unconfirmed`] tells the node, so an answer whose delivery was not confirmed cannot reach a
    /// running agent after the node heard it failed.
    fn take_overdue(&mut self, now: Instant) -> Vec<Value> {
        let mut late: Vec<u64> = self
            .delivering
            .iter()
            .filter(|(_, d)| now >= d.by)
            .map(|(t, _)| *t)
            .collect();
        late.sort_unstable();
        late.iter()
            .filter_map(|t| self.delivering.remove(t).map(|d| d.node_id))
            .collect()
    }

    /// ADR-0101: the node requests [`Self::take_overdue`] took get `adapter.approval_undelivered` (delivery not confirmed; the agent was
    /// ended first).
    fn unconfirmed(&mut self, ids: &[Value]) -> io::Result<()> {
        for id in ids {
            self.undelivered(id)?;
        }
        Ok(())
    }

    fn ask_agent(&mut self, method: &str, params: Value, waiting: Waiting) {
        self.next += 1;
        self.waiting.insert(self.next, waiting);
        let line = json!({"jsonrpc": "2.0", "id": self.next, "method": method, "params": params});
        self.send_agent(&line);
    }

    fn tell_agent(&mut self, method: &str, params: Value) {
        self.send_agent(&json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    /// Whether a cancelled `session/prompt` still waits for the agent's answer. Until it answered, every permission request is
    /// refused: ACP names the session in one, not the prompt, so it may be the stopped turn's.
    fn cancel_open(&self) -> bool {
        self.waiting.values().any(|w| {
            matches!(
                w,
                Waiting::Prompt {
                    cancelling: true,
                    ..
                }
            )
        })
    }

    /// Whether a `session/prompt` that was not cancelled still waits for the agent's answer (a turn is running).
    fn prompt_open(&self) -> bool {
        self.waiting.values().any(|w| {
            matches!(
                w,
                Waiting::Prompt {
                    cancelling: false,
                    ..
                }
            )
        })
    }

    fn cancel_permission(&mut self, request: Value) {
        self.send_agent(&json!({"jsonrpc": "2.0", "id": request,
            "result": {"outcome": {"outcome": "cancelled"}}}));
    }

    /// Forgets every pending permission, answering each `cancelled` first: the agent still waits for those answers.
    fn cancel_permissions(&mut self) {
        for permission in std::mem::take(&mut self.perms).into_values() {
            self.cancel_permission(permission.request);
        }
    }

    /// The agent's output ended and it exited (`exit_code` when known). Every request the node still waits for is answered with
    /// `adapter.agent_exited` and the stage it was in; a `log` line says the same, so an exit in the middle of a turn or while
    /// idle is reported too.
    fn agent_gone(&mut self, exit_code: Option<i32>, stderr_tail: &str) -> io::Result<()> {
        self.flush_text()?;
        // ADR-0101: an answer whose write was not confirmed before the agent ended is not said to have reached it.
        let mut delivering: Vec<(u64, Delivering)> = self.delivering.drain().collect();
        delivering.sort_by_key(|(t, _)| *t);
        for (_, d) in delivering {
            self.undelivered(&d.node_id)?;
        }
        let mut waiting: Vec<(u64, Waiting)> = self.waiting.drain().collect();
        waiting.sort_by_key(|(n, _)| *n);
        let stage_of = |w: &Waiting| match w {
            Waiting::Initialize { .. } => "initialize",
            Waiting::NewSession { .. } => "session",
            Waiting::Prompt { .. } => "turn",
            Waiting::SetOption { .. } | Waiting::SetConfig { .. } => "option",
        };
        let stage = ["initialize", "session", "turn", "option"]
            .into_iter()
            .find(|s| waiting.iter().any(|(_, w)| stage_of(w) == *s))
            .unwrap_or(if self.session.is_some() {
                "idle"
            } else if self.initialized {
                "session"
            } else {
                "start"
            });
        let params = |stage: &str| {
            let mut p = json!({"stage": stage, "stderr_tail": stderr_tail});
            if let Some(c) = exit_code {
                p["exit_code"] = json!(c.to_string());
            }
            p
        };
        for (_, w) in &waiting {
            if let Waiting::Initialize { node_id }
            | Waiting::NewSession { node_id, .. }
            | Waiting::SetOption { node_id, .. }
            | Waiting::SetConfig { node_id, .. } = w
            {
                if !node_id.is_null() {
                    let data =
                        json!({"code": "adapter.agent_exited", "params": params(stage_of(w))});
                    self.send_node(&json!({"jsonrpc": "2.0", "id": node_id,
                        "error": {"code": -32000, "message": "the agent exited", "data": data}}))?;
                }
            }
        }
        self.notice("adapter.agent_exited", &params(stage))?;
        self.log_reason("ERROR", "adapter.agent_exited", &params(stage))
    }

    /// A request the node sent after the agent was gone, answered with `adapter.agent_exited` for the stage it asks for. True once
    /// a request was answered.
    fn answer_late(&mut self, msg: &Value, exit_code: Option<i32>, tail: &str) -> io::Result<bool> {
        let Some(id) = msg.get("id").filter(|id| !id.is_null()).cloned() else {
            return Ok(false);
        };
        let Some(method) = msg.get("method").and_then(Value::as_str) else {
            return Ok(false);
        };
        let stage = match method {
            "initialize" => "initialize",
            "session/start" => "session",
            _ => "start",
        };
        let mut p = json!({"stage": stage, "stderr_tail": tail});
        if let Some(c) = exit_code {
            p["exit_code"] = json!(c.to_string());
        }
        let data = json!({"code": "adapter.agent_exited", "params": p});
        self.send_node(&json!({"jsonrpc": "2.0", "id": id,
            "error": {"code": -32000, "message": "the agent exited", "data": data}}))?;
        Ok(true)
    }

    /// The agent answered its own `initialize` or `session/new` with an error. The node's request is answered with
    /// `adapter.agent_refused`, and a notice says the same with the agent's error code and its message (scrubbed and cut like a
    /// stderr tail), so a restart that fails this way is visible in the conversation: opencode 2.x, for one, answers `session/new` with
    /// -32603 `ServiceUnavailableError` when its background service (`opencode service`) is not running.
    fn refused(&mut self, node_id: &Value, stage: &str, answer: &Value) -> io::Result<()> {
        let rpc_code = answer
            .pointer("/error/code")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let message = answer
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("");
        let detail = stderr_tail(message.as_bytes(), self.home.as_deref());
        self.refused_because(node_id, stage, rpc_code, &detail)
    }

    /// [`Self::refused`] with the notice's `rpc_code` and `detail` given: `0` and a stable reason when the agent's answer was not an
    /// error but cannot be used (a session id longer than `SESSION_ID_MAX`, `session_id_too_long`).
    fn refused_because(
        &mut self,
        node_id: &Value,
        stage: &str,
        rpc_code: i64,
        detail: &str,
    ) -> io::Result<()> {
        self.notice(
            "adapter.agent_refused",
            &json!({"stage": stage, "rpc_code": rpc_code.to_string(), "detail": detail}),
        )?;
        if node_id.is_null() {
            return Ok(());
        }
        let data = json!({"code": "adapter.agent_refused", "params": {"stage": stage}});
        self.send_node(&json!({"jsonrpc": "2.0", "id": node_id,
            "error": {"code": -32000, "message": "the agent refused to start", "data": data}}))
    }

    /// ADR-0058: the node's `session/set_option`. Only an id the agent offered is passed on; anything else is refused here.
    fn set_option(&mut self, id: Option<Value>, params: &Value) -> io::Result<()> {
        let node_id = id.unwrap_or(Value::Null);
        let mode = match params.get("kind") {
            Some(Value::String(k)) if k == "SESSION_OPTION_KIND_MODE" => Some(true),
            Some(Value::String(k)) if k == "SESSION_OPTION_KIND_MODEL" => Some(false),
            Some(Value::Number(n)) if n.as_u64() == Some(1) => Some(true),
            Some(Value::Number(n)) if n.as_u64() == Some(2) => Some(false),
            _ => None,
        };
        let want = params
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let offered = |list: &[Value]| list.iter().any(|o| o["id"].as_str() == Some(want.as_str()));
        let ok = match mode {
            Some(true) => offered(&self.modes),
            Some(false) => offered(&self.models),
            None => false,
        };
        let (Some(mode), Some(sid), true) = (mode, self.session.clone(), ok) else {
            if !node_id.is_null() {
                let data = json!({"code": "adapter.option_refused", "params": {"id": want}});
                self.send_node(&json!({"jsonrpc": "2.0", "id": node_id,
                    "error": {"code": -32602, "message": "not offered", "data": data}}))?;
            }
            return Ok(());
        };
        let from_config = if mode {
            self.rich.modes_from.clone()
        } else {
            self.rich.models_from.clone()
        };
        let (method, params) = match from_config {
            // The agent gave no mode (or model) list of its own: the list is its config option, set the way ACP sets one.
            Some(config_id) => (
                "session/set_config_option",
                json!({"sessionId": sid, "configId": config_id, "value": want}),
            ),
            None if mode => (
                "session/set_mode",
                json!({"sessionId": sid, "modeId": want}),
            ),
            None => (
                "session/set_model",
                json!({"sessionId": sid, "modelId": want}),
            ),
        };
        self.ask_agent(
            method,
            params,
            Waiting::SetOption {
                node_id,
                session_id: sid,
                mode,
                id: want.clone(),
            },
        );
        Ok(())
    }

    /// ADR-0090: the node's `session/set_config`. Only an option and a value the agent reported are passed on; anything else is
    /// refused here with `adapter.option_refused`.
    fn set_config(&mut self, id: Option<Value>, params: &Value) -> io::Result<()> {
        let node_id = id.unwrap_or(Value::Null);
        let text = |k: &str| {
            params
                .get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        let (config_id, value) = (text("configId"), text("value"));
        let acp_value = self
            .rich
            .config
            .iter()
            .find(|o| o["id"].as_str() == Some(config_id.as_str()))
            .and_then(|o| {
                if o["type"] == "boolean" {
                    match value.as_str() {
                        "true" => Some(json!({"type": "boolean", "value": true})),
                        "false" => Some(json!({"type": "boolean", "value": false})),
                        _ => None,
                    }
                } else if o["choices"]
                    .as_array()
                    .is_some_and(|c| c.iter().any(|c| c["id"].as_str() == Some(value.as_str())))
                {
                    Some(json!({"value": value}))
                } else {
                    None
                }
            });
        let (Some(acp_value), Some(sid)) =
            (acp_value.filter(|_| self.rich.on), self.session.clone())
        else {
            if !node_id.is_null() {
                let data = json!({"code": "adapter.option_refused", "params": {"id": config_id}});
                self.send_node(&json!({"jsonrpc": "2.0", "id": node_id,
                    "error": {"code": -32602, "message": "not offered", "data": data}}))?;
            }
            return Ok(());
        };
        let mut p = json!({"sessionId": sid, "configId": config_id});
        if let (Some(m), Some(v)) = (p.as_object_mut(), acp_value.as_object()) {
            m.extend(v.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        self.ask_agent(
            "session/set_config_option",
            p,
            Waiting::SetConfig {
                node_id,
                session_id: sid,
                id: config_id,
                value,
            },
        );
        Ok(())
    }

    /// ADR-0090: the mode and model lists of the older API, built from the agent's config options when the agent gave no list of its
    /// own (the ACP spec has agents send both during the transition; an agent that sends only config options still gets a mode and
    /// a model switch). Only for a node that takes rich chat, so a node without it sees what it always saw.
    fn derive_from_config(&mut self) {
        if !self.rich.on {
            return;
        }
        let Some(sid) = self.session.clone() else {
            return;
        };
        let find = |config: &[Value], category: &str| {
            config
                .iter()
                .find(|o| o["category"] == category && o["type"] == "select")
                .cloned()
        };
        let state = self.session_state.entry(sid).or_default();
        if self.modes.is_empty() || self.rich.modes_from.is_some() {
            match find(&self.rich.config, "mode") {
                Some(o) => {
                    self.modes = o["choices"].as_array().cloned().unwrap_or_default();
                    self.rich.modes_from = o["id"].as_str().map(str::to_string);
                    state.mode = o["current"].as_str().unwrap_or("").to_string();
                }
                None => {
                    if self.rich.modes_from.take().is_some() {
                        self.modes.clear();
                    }
                }
            }
        }
        if self.models.is_empty() || self.rich.models_from.is_some() {
            match find(&self.rich.config, "model") {
                Some(o) => {
                    self.models = o["choices"].as_array().cloned().unwrap_or_default();
                    self.rich.models_from = o["id"].as_str().map(str::to_string);
                    state.model = clip(o["current"].as_str().unwrap_or(""), MODEL_MAX);
                }
                None => {
                    if self.rich.models_from.take().is_some() {
                        self.models.clear();
                    }
                }
            }
        }
    }

    /// ADR-0090: the agent's config options changed (a reply to `session/new`, `session/set_config_option` or a
    /// `config_option_update`): remembered, and reported to the node together with the lists built from them.
    fn config_changed(&mut self, list: Option<&Value>) -> io::Result<()> {
        if !self.rich.on {
            return Ok(());
        }
        self.config_replaced(acp_config(list))
    }

    /// [`Self::config_changed`] for options already in the adapter-protocol form.
    fn config_replaced(&mut self, config: Vec<Value>) -> io::Result<()> {
        self.rich.config = config;
        self.derive_from_config();
        let Some(sid) = self.session.clone() else {
            return Ok(());
        };
        self.note(
            "session/config",
            json!({"sessionId": sid, "options": &self.rich.config}),
        )?;
        if self.rich.modes_from.is_some() || self.rich.models_from.is_some() {
            self.options()?;
        }
        Ok(())
    }

    /// What the bridge declares to the node. The ADR-0090 ones only to a node that said `chat.rich`: `thought`, `tool_status`,
    /// `plan_status`, `config`, `commands`, and `turns` (the bridge reports `working` at the start of every prompt turn and `idle` at
    /// its end, which is what lets the node queue a prompt behind a turn).
    fn capabilities(&self) -> Vec<&'static str> {
        let mut c = vec![
            "chat",
            "approvals",
            // ADR-0101: `approval/request` carries `scopes`, and a plain Allow is never answered with a remembered option.
            "approval_scopes",
            "plan",
            "diff",
            "cancel",
            "usage",
            "set_mode",
            "set_model",
        ];
        if self.rich.on {
            c.extend([
                "thought",
                "tool_status",
                "plan_status",
                "config",
                "commands",
                "turns",
            ]);
        }
        c
    }

    /// A request or notification from the node.
    fn handle_node(&mut self, msg: &Value) -> io::Result<ControlFlow<Option<Value>>> {
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let done = |me: &mut Self| match &id {
            Some(id) => me.reply(id, json!({})),
            None => Ok(()),
        };
        match method {
            "initialize" => {
                self.rich.on = params
                    .get("nodeCapabilities")
                    .and_then(Value::as_array)
                    .is_some_and(|a| a.iter().any(|c| c.as_str() == Some("chat.rich")));
                self.ask_agent(
                    "initialize",
                    json!({"protocolVersion": 1, "clientCapabilities": {"fs": {"readTextFile": false, "writeTextFile": false}, "terminal": false}}),
                    Waiting::Initialize {
                        node_id: id.clone().unwrap_or(Value::Null),
                    },
                );
            }
            "session/start" => {
                let cwd = params
                    .get("worktree")
                    .and_then(Value::as_str)
                    .unwrap_or(".")
                    .to_string();
                let resume = params
                    .get("resumeSessionId")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let node_id = id.clone().unwrap_or(Value::Null);
                // Audit F54 (Codex review of #920): every line about a session carries its id, so one over the limit is refused.
                if resume.as_ref().is_some_and(|r| r.len() > SESSION_ID_MAX) {
                    if !node_id.is_null() {
                        let data = json!({"code": "protocol.invalid_message", "params": {"field": "resumeSessionId"}});
                        self.send_node(&json!({"jsonrpc": "2.0", "id": node_id,
                            "error": {"code": -32602, "message": "invalid params", "data": data}}))?;
                    }
                    return Ok(ControlFlow::Continue(()));
                }
                // The node's MCP servers (the agent's own mesh server), as ACP stdio servers.
                let servers = if self.mcp_stdio {
                    acp_mcp_servers(params.get("mcpServers"))
                } else {
                    Vec::new()
                };
                match resume.filter(|_| self.can_load) {
                    Some(r) => {
                        self.ask_agent(
                            "session/load",
                            json!({"sessionId": r, "cwd": cwd, "mcpServers": servers}),
                            Waiting::NewSession {
                                node_id,
                                resume_id: Some(r),
                            },
                        );
                    }
                    None => self.ask_agent(
                        "session/new",
                        json!({"cwd": cwd, "mcpServers": servers}),
                        Waiting::NewSession {
                            node_id,
                            resume_id: None,
                        },
                    ),
                }
            }
            "session/prompt" => {
                let text = params
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let Some(sid) = self.session.clone() else {
                    if let Some(id) = &id {
                        self.send_node(&json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": "no session"}}))?;
                    }
                    return Ok(ControlFlow::Continue(()));
                };
                done(self)?;
                self.said = false;
                self.turn_mark = self.stderr.lock().unwrap_or_else(|e| e.into_inner()).total;
                // A permission still pending keeps the agent waiting for the user, whatever else it is sent.
                self.state(if self.perms.is_empty() {
                    "WORKING"
                } else {
                    "BLOCKED"
                })?;
                // A cancelled prompt still unanswered (the node's cancel grace passed) no longer counts for the turn: this one's
                // reply ends it, and its own late reply ends nothing.
                for request in self.waiting.values_mut() {
                    if let Waiting::Prompt {
                        cancelling: true,
                        superseded,
                    } = request
                    {
                        *superseded = true;
                    }
                }
                self.ask_agent(
                    "session/prompt",
                    json!({"sessionId": sid, "prompt": [{"type": "text", "text": text}]}),
                    Waiting::Prompt {
                        cancelling: false,
                        superseded: false,
                    },
                );
            }
            "session/cancel" => {
                done(self)?;
                if let Some(sid) = self.session.clone() {
                    for request in self.waiting.values_mut() {
                        if let Waiting::Prompt { cancelling, .. } = request {
                            *cancelling = true;
                        }
                    }
                    self.tell_agent("session/cancel", json!({"sessionId": sid}));
                    self.cancel_permissions();
                }
            }
            "approval/resolve" => {
                let approval = params
                    .get("approvalId")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let approved = params
                    .get("approved")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                // ADR-0101: the scope is read first; a value this build does not know is refused and nothing is sent.
                let Some(always) = decision_scope(&params) else {
                    if let Some(id) = &id {
                        self.send_node(&json!({"jsonrpc": "2.0", "id": id,
                            "error": {"code": -32602, "message": "protocol.invalid_message",
                                      "data": {"code": "protocol.invalid_message", "params": {"field": "scope"}}}}))?;
                    }
                    return Ok(ControlFlow::Continue(()));
                };
                let Some(p) = self.perms.get(approval) else {
                    // A permission the bridge already answered `cancelled` (a prompt's reply, a cancel, a change of session), or
                    // never knew: the decision cannot reach the agent, so it is refused and the node cancels its entry.
                    if let Some(id) = &id {
                        self.send_node(&json!({"jsonrpc": "2.0", "id": id,
                            "error": {"code": -32602, "message": "unknown approval"}}))?;
                    }
                    return Ok(ControlFlow::Continue(()));
                };
                // ADR-0101: exactly the option the person's answer names, captured when the agent asked. An answer the agent did not
                // offer is refused before anything is sent, and the request stays held (answered again, or `cancelled` at turn end).
                let chosen = match (approved, always) {
                    (true, false) => Ok(p.once.clone()),
                    (true, true) => Ok(p.always.clone()),
                    (false, false) => Err(p.reject.clone()),
                    (false, true) => {
                        // A remembered refusal is reserved: never sent.
                        if let Some(id) = &id {
                            self.send_node(&json!({"jsonrpc": "2.0", "id": id,
                                "error": {"code": -32602, "message": "protocol.invalid_message",
                                          "data": {"code": "protocol.invalid_message", "params": {"field": "scope"}}}}))?;
                        }
                        return Ok(ControlFlow::Continue(()));
                    }
                };
                let outcome = match chosen {
                    Ok(Some(option)) | Err(Some(option)) => {
                        json!({"outcome": "selected", "optionId": option})
                    }
                    Err(None) => json!({"outcome": "cancelled"}),
                    Ok(None) => {
                        if let Some(id) = &id {
                            self.send_node(&json!({"jsonrpc": "2.0", "id": id,
                                "error": {"code": -32602, "message": "approval.scope_unavailable",
                                          "data": {"code": "approval.scope_unavailable"}}}))?;
                        }
                        return Ok(ControlFlow::Continue(()));
                    }
                };
                let Some(p) = self.perms.remove(approval) else {
                    return Ok(ControlFlow::Continue(()));
                };
                // Codex review of PR #1007: the node is answered only once the answer was written to the agent's stdin (the node
                // records an always answer as one the agent holds on this success).
                match self.send_agent_reported(
                    &json!({"jsonrpc": "2.0", "id": p.request, "result": {"outcome": outcome}}),
                ) {
                    Delivery::Failed => {
                        if let Some(id) = &id {
                            self.undelivered(id)?;
                        }
                        return Ok(ControlFlow::Continue(()));
                    }
                    Delivery::Done => done(self)?,
                    Delivery::Pending(token) => {
                        if let Some(id) = &id {
                            self.delivering.insert(
                                token,
                                Delivering {
                                    node_id: id.clone(),
                                    by: Instant::now() + DELIVER_WAIT,
                                },
                            );
                        }
                    }
                }
                // Permissions are kept only while a prompt runs (one outside a turn is refused, any prompt's reply cancels them).
                if self.perms.is_empty() {
                    self.state("WORKING")?;
                }
            }
            "session/set_option" => self.set_option(id, &params)?,
            "session/set_config" => self.set_config(id, &params)?,
            "session/stop" | "shutdown" => {
                self.flush_text()?;
                return Ok(ControlFlow::Break(id));
            }
            // `mesh/deliver` and anything else new: acknowledged, not acted on.
            _ => done(self)?,
        }
        Ok(ControlFlow::Continue(()))
    }

    fn update(&mut self, u: &Value, replay_id: Option<&str>) -> io::Result<()> {
        match u.get("sessionUpdate").and_then(Value::as_str).unwrap_or("") {
            "agent_message_chunk" => {
                if let Some(t) = u.pointer("/content/text").and_then(Value::as_str) {
                    let sid = replay_id.or(self.session.as_deref()).unwrap_or_default();
                    if !self.text.is_empty()
                        && (self.rich.thought
                            || self.text_session != sid
                            || self.text.len() + t.len() > JOIN_MAX)
                    {
                        self.flush_text()?;
                    }
                    if self.text.is_empty() {
                        self.text_session = replay_id
                            .or(self.session.as_deref())
                            .unwrap_or_default()
                            .to_string();
                        self.text_since = Instant::now();
                    }
                    self.text.push_str(t);
                    if self.text.len() >= JOIN_MAX {
                        self.flush_text()?;
                    }
                }
            }
            "agent_thought_chunk" if self.rich.on => {
                if let Some(t) = u.pointer("/content/text").and_then(Value::as_str) {
                    let sid = replay_id
                        .or(self.session.as_deref())
                        .unwrap_or_default()
                        .to_string();
                    // A thought and a message are different entries: whichever the buffer holds goes first.
                    if !self.text.is_empty()
                        && (!self.rich.thought
                            || self.text_session != sid
                            || self.text.len() + t.len() > JOIN_MAX)
                    {
                        self.flush_text()?;
                    }
                    if self.text.is_empty() {
                        self.text_session = sid.clone();
                        self.text_since = Instant::now();
                        self.rich.thought = true;
                    }
                    self.text.push_str(t);
                    if self.text.len() >= JOIN_MAX {
                        self.flush_text()?;
                    }
                }
            }
            "tool_call" | "tool_call_update" if self.rich.on && self.tool_item(u).is_some() => {
                self.flush_text()?;
                if let Some(item) = self.tool_item(u) {
                    self.chat(json!({"toolCall": item}), replay_id)?;
                }
                self.diffs_in(u, replay_id)?;
            }
            "available_commands_update" if self.rich.on && replay_id.is_none() => {
                let commands: Vec<Value> = u
                    .get("availableCommands")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|c| {
                                let name = c.get("name")?.as_str().filter(|n| !n.is_empty())?;
                                Some(json!({
                                    "name": clip(name, OPTION_ID_MAX),
                                    "description": clip(c.get("description").and_then(Value::as_str).unwrap_or(""), OPTION_DESCRIPTION_MAX),
                                    "inputHint": clip(c.pointer("/input/hint").and_then(Value::as_str).unwrap_or(""), OPTION_NAME_MAX),
                                }))
                            })
                            .take(COMMANDS_MAX)
                            .collect()
                    })
                    .unwrap_or_default();
                if let Some(sid) = self.session.clone() {
                    self.note(
                        "session/commands",
                        json!({"sessionId": sid, "commands": commands}),
                    )?;
                }
            }
            "config_option_update" if self.rich.on && replay_id.is_none() => {
                self.config_changed(u.get("configOptions"))?;
            }
            "tool_call" => {
                self.flush_text()?;
                let title = u
                    .get("title")
                    .and_then(Value::as_str)
                    .or_else(|| u.get("kind").and_then(Value::as_str))
                    .unwrap_or("tool");
                let args = u
                    .get("rawInput")
                    .map(|v| head_tail(&v.to_string(), ARGUMENTS_MAX))
                    .unwrap_or_default();
                self.chat(
                    json!({"toolCall": {"tool": title, "argumentsJson": args}}),
                    replay_id,
                )?;
                self.diffs_in(u, replay_id)?;
            }
            "tool_call_update" => self.diffs_in(u, replay_id)?,
            "plan" => {
                self.flush_text()?;
                // A rich node reads only the entries; a plain one gets the steps as they came (they cannot outgrow the line).
                let (most, text_max) = if self.rich.on {
                    (PLAN_ENTRIES_MAX, PLAN_TEXT_MAX)
                } else {
                    (usize::MAX, usize::MAX)
                };
                let steps: Vec<String> = u
                    .get("entries")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|e| {
                                e.get("content")
                                    .and_then(Value::as_str)
                                    .map(|c| head_tail(c, text_max))
                            })
                            .take(most)
                            .collect()
                    })
                    .unwrap_or_default();
                let mut plan = json!({"steps": steps});
                if self.rich.on {
                    // ADR-0090: the steps' status and priority.
                    plan["entries"] = json!(u
                        .get("entries")
                        .and_then(Value::as_array)
                        .map(|a| a
                            .iter()
                            .filter_map(|e| {
                                Some(json!({"text": head_tail(e.get("content")?.as_str()?, PLAN_TEXT_MAX),
                                            "status": clip(e.get("status").and_then(Value::as_str).unwrap_or(""), 40),
                                            "priority": clip(e.get("priority").and_then(Value::as_str).unwrap_or(""), 40)}))
                            })
                            .take(PLAN_ENTRIES_MAX)
                            .collect::<Vec<_>>())
                        .unwrap_or_default());
                }
                self.chat(json!({"plan": plan}), replay_id)?;
            }
            "usage_update" => self.usage(u, replay_id)?,
            "current_mode_update" => {
                let sid = replay_id.or(self.session.as_deref());
                let mode = u
                    .get("currentModeId")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty() && id.len() <= OPTION_ID_MAX);
                if let (Some(sid), Some(mode)) = (sid, mode) {
                    let state = self.session_state.entry(sid.to_owned()).or_default();
                    if mode != state.mode {
                        state.mode = mode.to_string();
                        // A loading session's options are published with its successful reply.
                        if replay_id.is_none() {
                            self.options()?;
                        }
                    }
                }
            }
            _ => {} // agent_thought_chunk, user_message_chunk, available_commands_update, ...
        }
        Ok(())
    }

    /// ADR-0090: an ACP `tool_call` or `tool_call_update` as an adapter-protocol tool item that names its call. `None` when the
    /// update has no usable call id (the older item is sent then). A field the update leaves out is left out here too, so the node
    /// keeps what it holds, as ACP means an update.
    fn tool_item(&self, u: &Value) -> Option<Value> {
        let call = u
            .get("toolCallId")
            .and_then(Value::as_str)
            .filter(|c| !c.is_empty() && c.len() <= 256)?;
        let create = u.get("sessionUpdate").and_then(Value::as_str) == Some("tool_call");
        let mut t = json!({"callId": call});
        let title = u.get("title").and_then(Value::as_str);
        let kind = u
            .get("kind")
            .and_then(Value::as_str)
            .filter(|k| k.len() <= 40);
        if let Some(title) = title.or(if create { kind.or(Some("tool")) } else { None }) {
            t["tool"] = json!(title);
        }
        if let Some(raw) = u.get("rawInput").filter(|v| !v.is_null()) {
            t["argumentsJson"] = json!(head_tail(&raw.to_string(), ARGUMENTS_MAX));
        } else if create {
            t["argumentsJson"] = json!("");
        }
        if let Some(k) = kind {
            t["kind"] = json!(k);
        }
        if let Some(s) = u
            .get("status")
            .and_then(Value::as_str)
            .filter(|s| s.len() <= 40)
        {
            t["status"] = json!(s);
        }
        if let Some(a) = u.get("locations").and_then(Value::as_array) {
            t["locations"] = json!(a
                .iter()
                .filter_map(|l| {
                    Some(json!({"path": clip(l.get("path")?.as_str()?, 1024),
                                "line": l.get("line").and_then(Value::as_u64).unwrap_or(0)}))
                })
                .take(LOCATIONS_MAX)
                .collect::<Vec<_>>());
        }
        if let Some(out) = tool_output(u) {
            t["output"] = json!(out);
        }
        Some(t)
    }

    /// ACP `usage_update`: the cost is the session's running total, so the report carries its growth since the last one (like
    /// tokens, which are reported as they grow); `used` and `size` are the context's level and size (ADR-0058), passed on as they
    /// are. Nothing is sent when neither changed.
    fn usage(&mut self, u: &Value, replay_id: Option<&str>) -> io::Result<()> {
        let sid = replay_id.or(self.session.as_deref()).unwrap_or_default();
        let previous = self.session_state.entry(sid.to_owned()).or_default();
        let cost = u.get("cost").and_then(cost_micros);
        let grew = cost
            .as_ref()
            .map(|(currency, micros)| match &previous.cost {
                Some((c, last)) if c == currency => micros.saturating_sub(*last),
                _ => *micros,
            });
        let context = (
            u.get("used").and_then(Value::as_u64).unwrap_or(0),
            u.get("size").and_then(Value::as_u64).unwrap_or(0),
        );
        let context_moved = context != (0, 0) && context != previous.context;
        if context != (0, 0) {
            previous.context = context;
        }
        let currency = cost.as_ref().map(|(c, _)| c.clone());
        if let Some(c) = cost.filter(|(currency, micros)| {
            previous
                .cost
                .as_ref()
                .is_none_or(|(previous_currency, last)| {
                    currency != previous_currency || micros >= last
                })
        }) {
            previous.cost = Some(c);
        }
        let grew = grew.filter(|g| *g > 0);
        if grew.is_none() && !context_moved {
            return Ok(());
        }
        let mut p = json!({"sessionId": sid, "model": previous.model, "inputTokens": 0, "outputTokens": 0,
                           "contextTokens": previous.context.0, "contextWindow": previous.context.1});
        if let (Some(g), Some(c)) = (grew, currency) {
            p["costMicros"] = json!(g);
            p["currency"] = json!(c);
        }
        self.note("usage/report", p)
    }

    fn diffs_in(&mut self, u: &Value, replay_id: Option<&str>) -> io::Result<()> {
        let Some(content) = u.get("content").and_then(Value::as_array) else {
            return Ok(());
        };
        for c in content
            .iter()
            .filter(|c| c.get("type").and_then(Value::as_str) == Some("diff"))
        {
            let path = c.get("path").and_then(Value::as_str).unwrap_or("");
            let new = c.get("newText").and_then(Value::as_str).unwrap_or("");
            let old = c.get("oldText").and_then(Value::as_str);
            self.flush_text()?;
            self.chat(
                json!({"diff": {"path": path, "unifiedDiff": head_tail(&replace_diff(path, old, new), DIFF_MAX)}}),
                replay_id,
            )?;
        }
        Ok(())
    }

    /// A message from the agent: a request or notification, or the answer to one of ours.
    fn handle_agent(&mut self, msg: &Value) -> io::Result<()> {
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(Value::as_str);
        match (method, id) {
            (Some("session/update"), _) => {
                if let Some(u) = msg.pointer("/params/update") {
                    // ACP replays history before acknowledging the load. Route those updates
                    // without changing the session used for commands.
                    let replay_id = msg
                        .pointer("/params/sessionId")
                        .and_then(Value::as_str)
                        .filter(|&id| self.session.as_deref() != Some(id));
                    if replay_id.is_some_and(|id| {
                        !self.waiting.values().any(|request| {
                            matches!(
                                request,
                                Waiting::NewSession { resume_id: Some(requested), .. }
                                    if requested == id
                            )
                        })
                    }) {
                        return Ok(());
                    }
                    self.update(u, replay_id)?;
                }
            }
            (Some("session/request_permission"), Some(request)) => {
                // Fail closed: while a cancelled prompt is unanswered, a request may be the stopped turn's (a new turn's is refused
                // too, until then). With no prompt running it belongs to no turn (Audit F1241, Codex review of #794): a `blocked`
                // could be read by the node as the start of its next prompt, and the decision would end that turn early.
                if self.cancel_open() || !self.prompt_open() {
                    self.cancel_permission(request);
                    return Ok(());
                }
                self.flush_text()?;
                let p = msg.get("params").cloned().unwrap_or(Value::Null);
                let call = p.get("toolCall").cloned().unwrap_or(Value::Null);
                let action = call
                    .get("title")
                    .and_then(Value::as_str)
                    .or_else(|| call.get("kind").and_then(Value::as_str))
                    .unwrap_or("permission")
                    .to_string();
                let detail = call.get("rawInput").map(short).unwrap_or_default();
                // (option id, name, kind) of every option with an id.
                let options: Vec<(String, String, String)> = p
                    .get("options")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|o| {
                                let text = |k: &str| {
                                    o.get(k).and_then(Value::as_str).unwrap_or("").to_string()
                                };
                                Some((
                                    o.get("optionId")?.as_str()?.to_string(),
                                    text("name"),
                                    text("kind"),
                                ))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let first = |kind: &str| {
                    options
                        .iter()
                        .find(|(_, _, k)| k == kind)
                        .map(|(id, _, _)| id.clone())
                };
                let mut always_options = options.iter().filter(|(_, _, k)| k == "allow_always");
                // ADR-0101 (Codex review): exactly one always option, or none is offered; two would leave the signed answer ambiguous.
                let always = match (always_options.next(), always_options.next()) {
                    (Some(one), None) => Some(one.clone()),
                    _ => None,
                };
                let once = first("allow_once");
                let reject = first("reject_once");
                if once.is_none() && always.is_none() {
                    // ADR-0101: nothing to allow, so nobody is asked: refused at once, and the person is told why.
                    match &reject {
                        Some(option) => self.send_agent(&json!({"jsonrpc": "2.0", "id": request,
                            "result": {"outcome": {"outcome": "selected", "optionId": option}}})),
                        None => self.send_agent(&json!({"jsonrpc": "2.0", "id": request,
                            "result": {"outcome": {"outcome": "cancelled"}}})),
                    };
                    let reason = json!({"action": action});
                    self.log_reason("WARN", "adapter.permission_unanswerable", &reason)?;
                    self.notice("adapter.permission_unanswerable", &reason)?;
                    return Ok(());
                }
                let mut scopes = Vec::new();
                if once.is_some() {
                    scopes.push("APPROVAL_SCOPE_ONCE");
                }
                let (always_label, always_option_id) = match &always {
                    Some((id, name, _)) => {
                        scopes.push("APPROVAL_SCOPE_ALWAYS");
                        (name.clone(), id.clone())
                    }
                    None => (String::new(), String::new()),
                };
                self.next += 1;
                let approval = format!("acp-{}", self.next);
                self.perms.insert(
                    approval.clone(),
                    Permission {
                        request,
                        once,
                        always: always.map(|(id, _, _)| id),
                        reject,
                    },
                );
                let sid = self.session.clone().unwrap_or_default();
                self.said = true;
                self.state("BLOCKED")?;
                self.note(
                    "approval/request",
                    json!({"sessionId": sid, "approvalId": approval, "action": action, "detail": detail,
                           "scopes": scopes, "alwaysLabel": always_label, "alwaysOptionId": always_option_id}),
                )?;
            }
            // Files and terminals are the agent's own business in its worktree; the bridge offers neither.
            (Some(_), Some(id)) => {
                self.send_agent(
                    &json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "method not found"}}),
                );
            }
            (Some(_), None) => {}
            (None, Some(id)) => {
                let has_result = msg.get("result").is_some();
                let has_error = msg.get("error").is_some();
                if has_result == has_error {
                    return Ok(());
                }
                let Some(n) = id.as_u64() else { return Ok(()) };
                match self.waiting.remove(&n) {
                    Some(Waiting::Initialize { node_id }) => {
                        if msg.get("error").is_some() {
                            self.refused(&node_id, "initialize", msg)?;
                        } else {
                            self.initialized = true;
                            self.can_load = msg
                                .pointer("/result/agentCapabilities/loadSession")
                                .and_then(Value::as_bool)
                                .unwrap_or(false);
                            self.mcp_stdio = msg
                                .pointer("/result/agentCapabilities/mcpCapabilities/stdio")
                                .and_then(Value::as_bool)
                                .unwrap_or(true);
                            if !node_id.is_null() {
                                self.reply(
                                    &node_id,
                                    json!({"adapterName": "acp-bridge", "version": "1", "agentKind": "acp",
                                           "capabilities": self.capabilities()}),
                                )?;
                            }
                        }
                    }
                    Some(Waiting::NewSession { node_id, resume_id }) => {
                        // What a loaded session replayed reaches the node before the answer, text still held for its window too: the
                        // node takes everything before the answer as the replay (review of #1103, B1).
                        self.flush_text()?;
                        let sid = msg
                            .pointer("/result/sessionId")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                            .or(resume_id);
                        match sid {
                            // Audit F54 (Codex review of #920): an id over the limit is refused, never cut (that would be
                            // another session), so no line the bridge sends about the session outgrows the node's limit.
                            Some(sid)
                                if msg.get("error").is_none() && sid.len() > SESSION_ID_MAX =>
                            {
                                self.refused_because(
                                    &node_id,
                                    "session",
                                    0,
                                    "session_id_too_long",
                                )?;
                            }
                            Some(sid) if msg.get("error").is_none() => {
                                if self.session.as_deref() != Some(sid.as_str()) {
                                    // A reply from the previous session cannot finish the new turn.
                                    self.waiting.retain(|_, request| {
                                        !matches!(request, Waiting::Prompt { .. })
                                    });
                                    self.cancel_permissions();
                                }
                                let state = self.session_state.entry(sid.clone()).or_default();
                                self.session = Some(sid.clone());
                                if let Some(m) = msg
                                    .pointer("/result/models/currentModelId")
                                    .and_then(Value::as_str)
                                {
                                    state.model = clip(m, MODEL_MAX);
                                }
                                self.models = acp_options(
                                    msg.pointer("/result/models/availableModels"),
                                    "modelId",
                                    false,
                                );
                                self.modes = acp_options(
                                    msg.pointer("/result/modes/availableModes"),
                                    "id",
                                    true,
                                );
                                if let Some(m) = msg
                                    .pointer("/result/modes/currentModeId")
                                    .and_then(Value::as_str)
                                    .filter(|m| m.len() <= OPTION_ID_MAX)
                                {
                                    state.mode = m.to_string();
                                }
                                self.reply(&node_id, json!({"sessionId": sid}))?;
                                self.note(
                                    "session/reported",
                                    json!({"sessionId": sid, "nativeSessionId": sid}),
                                )?;
                                if self.rich.on {
                                    self.rich.modes_from = None;
                                    self.rich.models_from = None;
                                    let list = msg.pointer("/result/configOptions");
                                    self.rich.config = acp_config(list);
                                    self.derive_from_config();
                                    self.note(
                                        "session/config",
                                        json!({"sessionId": sid, "options": &self.rich.config}),
                                    )?;
                                }
                                self.options()?;
                                self.state("IDLE")?;
                            }
                            _ => {
                                self.refused(&node_id, "session", msg)?;
                            }
                        }
                    }
                    // A cancelled prompt a newer one replaced: the newer turn is not its business.
                    Some(Waiting::Prompt {
                        superseded: true, ..
                    }) => {}
                    Some(Waiting::Prompt { .. }) => {
                        self.flush_text()?;
                        // Audit F1240: prompts overlap when the node sends one into a running turn (a spawned helper's request,
                        // a node without `turns`). Only the last reply ends the turn: until then no `idle` is reported; this reply's
                        // own failure or stop is still reported (whether it was empty cannot be told). Cancelled prompts do not hold
                        // the turn open. Any reply ends every pending permission (fail closed): ACP names the session in one, not the
                        // prompt, so it may be this finished prompt's; the agent asks again if it still needs it.
                        self.cancel_permissions();
                        let still_open = self.prompt_open();
                        match turn_outcome(msg, self.said || still_open) {
                            Some((code, mut params)) => {
                                // A failed or empty turn says nothing about why; what the agent wrote to stderr meanwhile often does
                                // (an expired login, a provider's refusal).
                                if code != "adapter.turn_stopped" {
                                    let tail = self.turn_stderr();
                                    if !tail.is_empty() {
                                        params["stderr_tail"] = json!(tail);
                                    }
                                }
                                self.notice(code, &params)?;
                                self.log_reason("WARN", code, &params)?;
                                if !still_open {
                                    self.state_because("IDLE", code)?;
                                }
                            }
                            None if !still_open => self.state("IDLE")?,
                            None => {}
                        }
                        if still_open {
                            self.state("WORKING")?;
                        }
                    }
                    Some(Waiting::SetConfig {
                        node_id,
                        session_id,
                        id,
                        value,
                    }) => {
                        if msg.get("error").is_some() {
                            let data =
                                json!({"code": "adapter.option_refused", "params": {"id": id}});
                            self.send_node(&json!({"jsonrpc": "2.0", "id": node_id,
                                "error": {"code": -32000, "message": "the agent refused", "data": data}}))?;
                        } else {
                            self.reply(&node_id, json!({}))?;
                            if self.session.as_deref() == Some(session_id.as_str()) {
                                match msg.pointer("/result/configOptions") {
                                    Some(list) => self.config_changed(Some(list))?,
                                    None => {
                                        // The agent did not repeat its options: the one that was set holds the new value.
                                        let mut kept = std::mem::take(&mut self.rich.config);
                                        if let Some(o) = kept
                                            .iter_mut()
                                            .find(|o| o["id"].as_str() == Some(id.as_str()))
                                        {
                                            o["current"] = json!(value);
                                        }
                                        self.config_replaced(kept)?;
                                    }
                                }
                            }
                        }
                    }
                    Some(Waiting::SetOption {
                        node_id,
                        session_id,
                        mode,
                        id,
                    }) => {
                        if msg.get("error").is_some() {
                            let data =
                                json!({"code": "adapter.option_refused", "params": {"id": id}});
                            self.send_node(&json!({"jsonrpc": "2.0", "id": node_id,
                                "error": {"code": -32000, "message": "the agent refused", "data": data}}))?;
                        } else {
                            let is_current = self.session.as_deref() == Some(session_id.as_str());
                            let state = self.session_state.entry(session_id).or_default();
                            if mode {
                                state.mode = id;
                            } else {
                                state.model = clip(&id, MODEL_MAX);
                            }
                            self.reply(&node_id, json!({}))?;
                            if is_current {
                                self.options()?;
                            }
                        }
                    }
                    None => {}
                }
            }
            (None, None) => {}
        }
        Ok(())
    }
}

/// Runs the bridge until the node asks it to stop, closes its input, or the agent exits. `agent` is the ACP agent's command line.
pub fn run<R, W>(agent: &[String], input: R, output: W) -> io::Result<()>
where
    R: Read + Send + 'static,
    W: Write,
{
    let (program, args) = agent
        .split_first()
        .ok_or_else(|| io::Error::other("no agent command"))?;
    // A Windows `.cmd` shim (`npx`, which registry agents use) is started by its full path; elsewhere the name is unchanged.
    let mut child = Command::new(agora_platform::program_path(program, None))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let (stdin, stdout, stderr) =
        match (child.stdin.take(), child.stdout.take(), child.stderr.take()) {
            (Some(i), Some(o), Some(e)) => (i, o, e),
            _ => {
                let _ = child.kill();
                return Err(io::Error::other("agent pipes missing"));
            }
        };
    let kept = Arc::new(Mutex::new(Kept::default()));
    let stderr_done = keep_stderr(stderr, Arc::clone(&kept));
    let (tx, rx) = sync_channel(INPUT_QUEUE);
    lines(input, Ev::Node, tx.clone());
    lines(stdout, Ev::Agent, tx.clone());
    let mut b = Bridge {
        out: output,
        agent: Box::new(AgentWriter::start(stdin, tx)),
        agent_writer_full: false,
        next_report: 0,
        delivering: HashMap::new(),
        next: 0,
        waiting: HashMap::new(),
        session: None,
        perms: HashMap::new(),
        text: String::new(),
        text_session: String::new(),
        text_since: Instant::now(),
        initialized: false,
        can_load: false,
        mcp_stdio: true,
        said: false,
        session_state: HashMap::new(),
        modes: Vec::new(),
        models: Vec::new(),
        stderr: Arc::clone(&kept),
        turn_mark: 0,
        rich: Rich::default(),
        home: std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .ok(),
    };
    let result = (|| -> io::Result<Option<Value>> {
        let failure = loop {
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(Ev::Node(Input::Line(line))) => {
                    if let Ok(m) = serde_json::from_slice(&line) {
                        if let ControlFlow::Break(id) = b.handle_node(&m)? {
                            return Ok(id);
                        }
                    }
                }
                Ok(Ev::Agent(Input::Line(line))) => {
                    // The agent can print noise on stdout. Malformed JSON and non-UTF-8 lines are ignored.
                    if let Ok(m) = serde_json::from_slice(&line) {
                        b.handle_agent(&m)?;
                    }
                }
                Ok(Ev::Node(Input::Gone)) => return Ok(None),
                Ok(Ev::Node(Input::TooLong)) => return Err(io::ErrorKind::InvalidData.into()),
                Ok(Ev::Agent(Input::Gone)) => break None,
                Ok(Ev::Agent(Input::TooLong)) => break Some(io::ErrorKind::InvalidData),
                Ok(Ev::Written(token, ok)) => b.written(token, ok)?,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Ok(None),
            }
            if b.agent_writer_full {
                break Some(io::ErrorKind::WouldBlock);
            }
            let late = b.take_overdue(Instant::now());
            if !late.is_empty() {
                // End the agent before the node hears the answer failed: expiry does not prove nothing reached it.
                let _ = child.kill();
                b.unconfirmed(&late)?;
                break Some(io::ErrorKind::TimedOut);
            }
            if !b.text.is_empty() && b.text_since.elapsed() >= JOIN {
                b.flush_text()?;
            }
        };
        if failure.is_some() {
            let _ = child.kill();
        }
        let code = wait_briefly(&mut child);
        // The rest of its stderr, unless something it started still holds the stream open.
        let _ = stderr_done.recv_timeout(Duration::from_millis(500));
        let raw = kept.lock().unwrap_or_else(|e| e.into_inner()).buf.clone();
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .ok();
        let tail = stderr_tail(&raw, home.as_deref());
        b.agent_gone(code, &tail)?;
        // The node's initialize or session/start may still be on its way. Give it the exit reason too.
        if b.session.is_none() {
            let end = Instant::now() + LATE_REQUEST_WAIT;
            while let Some(left) = end.checked_duration_since(Instant::now()) {
                match rx.recv_timeout(left) {
                    Ok(Ev::Node(Input::Line(line))) => {
                        if let Ok(m) = serde_json::from_slice(&line) {
                            if b.answer_late(&m, code, &tail)? {
                                break;
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        }
        Err(failure.map_or_else(|| io::Error::other("the agent exited"), io::Error::from))
    })();
    drop(rx);
    let _ = child.kill();
    let reaped = child.wait();
    let Some(id) = result? else {
        return b.flush_on_node_eof();
    };
    // The host may kill the bridge as soon as it sees the reply.
    reaped?;
    b.reply(&id, json!({}))
}

#[cfg(test)]
mod tests {
    fn bridge_with_buffered_chat() -> std::io::Result<super::Bridge<Vec<u8>>> {
        use super::*;

        let mut bridge = Bridge {
            out: Vec::new(),
            agent: Box::new(io::sink()),
            agent_writer_full: false,
            next_report: 0,
            delivering: HashMap::new(),
            next: 0,
            waiting: HashMap::new(),
            session: Some("s".to_string()),
            perms: HashMap::new(),
            text: String::new(),
            text_session: String::new(),
            text_since: Instant::now(),
            initialized: true,
            can_load: false,
            mcp_stdio: true,
            said: false,
            session_state: HashMap::new(),
            modes: Vec::new(),
            models: Vec::new(),
            stderr: Arc::new(Mutex::new(Kept::default())),
            turn_mark: 0,
            rich: Rich::default(),
            home: None,
        };
        // Dispatch chunks directly to leave the join buffer populated.
        for text in ["last ", "words"] {
            bridge.handle_agent(&json!({"jsonrpc":"2.0","method":"session/update","params":{
                "sessionId":"s","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":text}}
            }}))?;
        }
        assert!(bridge.out.is_empty());
        Ok(bridge)
    }

    #[test]
    fn explicit_stop_delivers_buffered_chat() -> Result<(), Box<dyn std::error::Error>> {
        use super::*;

        for method in ["session/stop", "shutdown"] {
            let mut bridge = bridge_with_buffered_chat()?;
            assert_eq!(
                bridge.handle_node(&json!({"jsonrpc":"2.0","id":7,"method":method}))?,
                ControlFlow::Break(Some(json!(7)))
            );
            assert!(bridge.text.is_empty());
            assert_eq!(
                serde_json::from_slice::<Value>(&bridge.out)?,
                json!({"jsonrpc":"2.0","method":"chat/append","params":{"sessionId":"s","text":{"text":"last words"}}})
            );
        }
        Ok(())
    }

    #[test]
    fn node_eof_delivers_buffered_chat() -> Result<(), Box<dyn std::error::Error>> {
        let mut bridge = bridge_with_buffered_chat()?;
        bridge.flush_on_node_eof()?;
        assert!(bridge.text.is_empty());
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bridge.out)?,
            serde_json::json!({"jsonrpc":"2.0","method":"chat/append",
                "params":{"sessionId":"s","text":{"text":"last words"}}})
        );
        Ok(())
    }

    #[test]
    fn the_error_line_is_kept_among_routine_ones() {
        let raw = b"INFO service=default providers=2\nINFO service=config path=/config/providers\nERROR service=llm provider rejected: auth\nINFO service=session idle\nINFO service=bus done\nINFO service=x y\nINFO service=z w\n";
        let tail = super::stderr_tail(raw, None);
        assert!(tail.contains("ERROR service=llm"), "{tail}");
        assert!(!tail.contains("INFO"), "{tail}");
        // Without a line naming a problem the last lines are kept as before.
        let plain = super::stderr_tail(b"one\ntwo\nthree\n", None);
        assert_eq!(plain, "one\ntwo\nthree");
        // A word that only contains the level is not one.
        assert!(!super::names_a_problem("TERRORIST warned"));
        assert!(super::names_a_problem("2026 WARN: slow"));
        // opencode 2.x: a failure is only an INFO line with an HTTP status.
        assert!(super::names_a_problem(
            "INFO message=\"Sent HTTP response\" http.url=/api/model/default http.status=503"
        ));
        assert!(!super::names_a_problem("INFO http.status=200 ok"));
        assert!(!super::names_a_problem("INFO status=5 x"));
    }

    #[test]
    fn the_home_folder_is_hidden_in_every_spelling() {
        let home = "C:\\Users\\someone";
        for line in [
            "C:\\Users\\someone\\.config\\x",
            "C:/Users/someone/.config/x",
            "c:\\users\\someone\\x",
            "/c/Users/someone/x",
            "\\\\?\\C:\\Users\\someone\\x",
        ] {
            let out = super::stderr_tail(line.as_bytes(), Some(home));
            assert!(
                !out.to_ascii_lowercase().contains("someone"),
                "{line} -> {out}"
            );
            assert!(out.contains('~'), "{line} -> {out}");
        }
        assert_eq!(
            super::stderr_tail(b"/home/x/.cache", Some("/home/x")),
            "~/.cache"
        );
    }

    use super::*;

    #[test]
    fn the_nodes_mcp_servers_become_acp_stdio_servers() {
        let from_node = json!([
            {"name": "agora-mesh", "command": "/usr/bin/agora", "args": ["mesh", "stdio", "--socket", "/s/agent-1.sock"]},
            {"name": "no-args", "command": "x"},
            {"name": "", "command": "x"},
            {"name": "bad", "command": "x", "args": [1]},
            {"command": "x"},
        ]);
        assert_eq!(
            acp_mcp_servers(Some(&from_node)),
            vec![
                json!({"name": "agora-mesh", "command": "/usr/bin/agora",
                       "args": ["mesh", "stdio", "--socket", "/s/agent-1.sock"], "env": []}),
                json!({"name": "no-args", "command": "x", "args": [], "env": []}),
            ]
        );
        assert!(acp_mcp_servers(None).is_empty());
        assert!(acp_mcp_servers(Some(&json!("x"))).is_empty());
    }

    #[test]
    fn a_replacement_diff_shows_every_old_line_removed_and_new_line_added() {
        let d = replace_diff("a.rs", Some("x\ny\n"), "z\n");
        assert_eq!(d, "--- a/a.rs\n+++ b/a.rs\n@@ -1,2 +1,1 @@\n-x\n-y\n+z\n");
        let n = replace_diff("n.txt", None, "hi\n");
        assert!(
            n.starts_with("--- /dev/null\n+++ b/n.txt\n@@ -0,0 +1,1 @@\n+hi"),
            "{n}"
        );
    }

    #[test]
    fn a_replacement_diff_preserves_line_endings_and_missing_final_newlines() {
        assert_eq!(
            replace_diff("a", Some("old"), "new\n"),
            "--- a/a\n+++ b/a\n@@ -1,1 +1,1 @@\n-old\n\\ No newline at end of file\n+new\n"
        );
        assert!(replace_diff("a", Some("old\n"), "new")
            .ends_with("-old\n+new\n\\ No newline at end of file\n"));
        assert!(replace_diff("a", Some("old\r\n"), "new\r\n").ends_with("-old\r\n+new\r\n"));
        assert_eq!(
            replace_diff("a", Some(""), ""),
            "--- a/a\n+++ b/a\n@@ -0,0 +0,0 @@\n"
        );
        assert!(replace_diff("a", None, "new").ends_with("+new\n\\ No newline at end of file\n"));
    }

    /// A line that is not UTF-8 used to end the reader as if the agent had exited (audit F044).
    #[test]
    fn a_line_that_is_not_utf8_is_noise_not_the_end_of_the_agent() {
        let (tx, rx) = sync_channel(INPUT_QUEUE);
        let data = b"\xff\xfe noise\n{\"text\":\"\xff\"}\n{\"id\":1}\r\n{\"id\":2}".to_vec();
        lines(std::io::Cursor::new(data), Ev::Agent, tx);
        let mut ids = Vec::new();
        loop {
            match rx.recv_timeout(Duration::from_secs(10)) {
                Ok(Ev::Agent(Input::Line(line))) => {
                    if let Ok(v) = serde_json::from_slice::<Value>(&line) {
                        ids.push(v["id"].clone());
                    }
                }
                Ok(Ev::Agent(Input::Gone)) => break,
                Ok(_) => {}
                Err(e) => panic!("reader stalled: {e}"),
            }
        }
        assert_eq!(ids, vec![json!(1), json!(2)]);
    }

    #[test]
    fn the_bridge_accepts_a_full_line_and_refuses_the_next_line_before_its_newline() {
        let (tx, rx) = sync_channel(INPUT_QUEUE);
        let mut data = b"{}".to_vec();
        data.resize(MAX_LINE, b' ');
        data.push(b'\n');
        data.resize(data.len() + MAX_LINE + 1, b' ');
        lines(std::io::Cursor::new(data), Ev::Node, tx);
        let wait = Duration::from_secs(10);
        let Ok(Ev::Node(Input::Line(line))) = rx.recv_timeout(wait) else {
            panic!("the full line should be accepted");
        };
        assert_eq!(line.len(), MAX_LINE);
        assert_eq!(serde_json::from_slice::<Value>(&line).ok(), Some(json!({})));
        assert!(matches!(
            rx.recv_timeout(wait),
            Ok(Ev::Node(Input::TooLong))
        ));
        assert!(matches!(
            rx.recv_timeout(wait),
            Err(RecvTimeoutError::Disconnected)
        ));
    }

    /// ADR-0022: the writer queue is bounded by bytes as well as by count.
    #[test]
    fn the_writer_queue_is_bounded_by_count_and_by_bytes() {
        use std::io::Write;
        let writer = |count: usize| {
            let (pending, rx) = sync_channel(count);
            (
                super::AgentWriter {
                    line: Vec::new(),
                    pending,
                    queued: Default::default(),
                    broken: Default::default(),
                },
                rx,
            )
        };
        let put = |w: &mut super::AgentWriter, n: usize| {
            w.write_all(&vec![b'x'; n]).and_then(|()| w.flush())
        };
        let (mut w, _rx) = writer(super::AGENT_WRITE_QUEUE);
        for _ in 0..super::AGENT_WRITE_QUEUE {
            assert!(put(&mut w, 10).is_ok());
        }
        assert_eq!(
            put(&mut w, 10).err().map(|e| e.kind()),
            Some(io::ErrorKind::WouldBlock)
        );
        let (mut w, _rx) = writer(super::AGENT_WRITE_QUEUE);
        assert!(
            put(&mut w, super::AGENT_WRITE_BYTES + 1).is_ok(),
            "a lone big line"
        );
        assert_eq!(
            put(&mut w, 1).err().map(|e| e.kind()),
            Some(io::ErrorKind::WouldBlock)
        );
    }

    #[test]
    fn a_burst_larger_than_the_queue_is_delivered_in_order() {
        let (tx, rx) = sync_channel(INPUT_QUEUE);
        let count = INPUT_QUEUE * 2;
        let mut data = Vec::new();
        for id in 0..count {
            assert!(writeln!(data, "{}", json!({"id": id})).is_ok());
        }
        lines(std::io::Cursor::new(data), Ev::Agent, tx);
        let wait = Duration::from_secs(10);
        for id in 0..count {
            let Ok(Ev::Agent(Input::Line(line))) = rx.recv_timeout(wait) else {
                panic!("missing frame {id}");
            };
            assert_eq!(
                serde_json::from_slice::<Value>(&line).ok(),
                Some(json!({"id": id}))
            );
        }
        assert!(matches!(rx.recv_timeout(wait), Ok(Ev::Agent(Input::Gone))));
    }

    #[test]
    fn secrets_in_stderr_are_replaced_and_ordinary_words_kept() {
        assert_eq!(
            redact("auth failed: api_key=abc token: xyz Bearer qq"),
            "auth failed: api_key=[redacted] token: [redacted] Bearer [redacted]"
        );
        assert_eq!(
            redact("Invalid API key provided, see https://x.io/docs"),
            "Invalid API key provided, see https://x.io/docs"
        );
        assert_eq!(
            redact("id sk-proj-0123456789abcdefABCDEF end"),
            "id [redacted] end"
        );
        assert_eq!(
            redact("exit 3 at /usr/lib/node_modules/opencode"),
            "exit 3 at /usr/lib/node_modules/opencode"
        );
    }

    #[test]
    fn url_passwords_long_tokens_and_osc_payloads_are_removed() {
        assert_eq!(
            redact("fetch https://bob:hunter2@git.example.com/x.git failed"),
            "fetch https://[redacted]@git.example.com/x.git failed"
        );
        assert_eq!(
            redact("see http://example.com/a@b"),
            "see http://example.com/a@b"
        );
        // Letters only, and base64 with + / =: no digit needed.
        let letters = "abcdefghijklmnopqrstuvwxyzABCDEFGH";
        assert_eq!(
            redact(&format!("token {letters} end")),
            "token [redacted] end"
        );
        assert_eq!(
            redact("blob aGVsbG8+d29ybGQ/aGVsbG8gd29ybGQgaGVsbG8= ok"),
            "blob [redacted] ok"
        );
        // Paths stay, also long ones.
        let path = "/Users/me/projects/some-really-long-folder-name/src/main.ts";
        assert_eq!(redact(path), path);
        assert_eq!(
            redact("at node_modules/@opencode-ai/plugin/dist/index.js:12"),
            "at node_modules/@opencode-ai/plugin/dist/index.js:12"
        );
        let raw =
            b"a\x1b]8;;https://u:p@x/secretlink\x07link\x1b]8;;\x07 b\x1b]0;title sk\x1b\\ c\n";
        assert_eq!(stderr_tail(raw, None), "alink b c");
        assert_eq!(stderr_tail("x\u{9d}0;hidden\u{7}y".as_bytes(), None), "xy");
    }

    #[test]
    fn any_users_home_folder_is_replaced_without_knowing_home() {
        for (line, want) in [
            (
                "at /Users/jane/.config/opencode/x.js:3",
                "at ~/.config/opencode/x.js:3",
            ),
            ("at /Users/Jane Doe/proj/a.ts", "at ~/proj/a.ts"),
            ("at /users/j.doe/x", "at ~/x"),
            (
                "open /home/ali.veli/.cache/y failed",
                "open ~/.cache/y failed",
            ),
            ("open /home/bob failed", "open ~ failed"),
            ("in /root/.npm and /root", "in ~/.npm and ~"),
            ("C:\\Users\\John Smith\\AppData\\x", "~\\AppData\\x"),
            ("c:/users/john.smith/AppData", "~/AppData"),
            ("/c/Users/john/AppData", "~/AppData"),
            ("/D/users/x.y/z", "~/z"),
            ("\"/Users/jane\": no such file", "\"~\": no such file"),
            (
                "/rootless and /var/home2/x and x/home/y",
                "/rootless and /var/home2/x and x/home/y",
            ),
            (
                "/usr/lib/node_modules/opencode",
                "/usr/lib/node_modules/opencode",
            ),
            ("/home/ is empty", "/home/ is empty"),
        ] {
            assert_eq!(replace_any_home(line), want, "{line}");
        }
        // Through the whole report, with no home given.
        let t = stderr_tail(
            b"Error: /Users/someone.else/.local/share/opencode/auth.json missing\n",
            None,
        );
        assert_eq!(t, "Error: ~/.local/share/opencode/auth.json missing");
        assert!(!stderr_tail(b"x /home/user name/y\n", None).contains("user"));
    }

    #[test]
    fn a_stderr_tail_is_short_plain_and_without_the_home_folder() {
        let raw = b"one\ntwo\n\x1b[31mthree\x1b[0m\n\nfour /home/me/.config/x\nfive\nsix\x07\n";
        assert_eq!(
            stderr_tail(raw, Some("/home/me")),
            "two\nthree\nfour ~/.config/x\nfive\nsix"
        );
        let long = "é".repeat(1000);
        let t = stderr_tail(long.as_bytes(), None);
        assert!(
            t.starts_with('…') && t.len() <= STDERR_TAIL + '…'.len_utf8(),
            "{t}"
        );
        assert_eq!(stderr_tail(b"", None), "");
    }

    #[test]
    fn costs_become_micro_units_and_bad_ones_are_ignored() {
        assert_eq!(
            cost_micros(&json!({"amount": 0.0123, "currency": "usd"})),
            Some(("USD".into(), 12300))
        );
        assert_eq!(cost_micros(&json!({"amount": -1, "currency": "USD"})), None);
        assert_eq!(
            cost_micros(&json!({"amount": 1, "currency": "dollars"})),
            None
        );
        assert_eq!(
            cost_micros(&json!({"amount": "1", "currency": "USD"})),
            None
        );
        assert_eq!(
            cost_micros(&json!({"amount": 1e300, "currency": "USD"})),
            Some(("USD".into(), u64::MAX))
        );
    }

    #[test]
    fn turn_outcomes_name_failures_stops_and_silence_only() {
        let end = json!({"result": {"stopReason": "end_turn"}});
        assert_eq!(turn_outcome(&end, true), None);
        assert_eq!(
            turn_outcome(&end, false).map(|o| o.0),
            Some("adapter.turn_empty")
        );
        assert_eq!(
            turn_outcome(&json!({"result": {"stopReason": "cancelled"}}), false),
            None
        );
        assert_eq!(
            turn_outcome(&json!({"error": {"code": -32800}}), false),
            None
        );
        assert_eq!(
            turn_outcome(&json!({"result": {"stopReason": "max_tokens"}}), true),
            Some(("adapter.turn_stopped", json!({"stop_reason": "max_tokens"})))
        );
        assert_eq!(
            turn_outcome(&json!({"result": {"stopReason": "<b>Nope</b>"}}), true),
            Some(("adapter.turn_stopped", json!({"stop_reason": "other"})))
        );
        assert_eq!(
            turn_outcome(&json!({"error": {"code": -32603, "message": "x"}}), true),
            Some(("adapter.turn_failed", json!({"rpc_code": "-32603"})))
        );
    }

    #[test]
    fn modes_that_act_without_asking_are_marked_and_others_are_not() {
        let skips = |id: &str, name: &str, d: &str| {
            perm::value_skips(Category::Mode, ("mode", "mode"), (id, name, d))
        };
        assert!(skips("bypassPermissions", "Bypass Permissions", ""));
        assert!(skips("full", "Full Access", ""));
        assert!(skips("acceptEdits", "", ""));
        assert!(!skips("build", "Build", ""));
        assert!(!skips("plan", "Plan", ""));
        assert!(!skips("default", "Always Ask", ""));
        // Codex's `auto`; `auto` only as a whole word.
        assert!(skips("auto", "Auto", ""));
        assert!(!perm::says_it_skips("autosave", "Autosave", "", true));
        // What a description says.
        assert!(skips("x", "X", "Runs tools without asking"));
        assert!(skips("x", "X", "Approve every edit automatically"));
        // A mode the rule cannot place is not taken for one that asks (ADR-0090): the user is asked to confirm it.
        assert!(skips("turbo", "Turbo", "Ask before every edit"));
        assert!(!skips("ask", "Ask", "Ask before every edit"));
    }

    /// The mode list of Antigravity (`agy`) 1.5.6 as it offers it: `accept-edits` and `auto` act without asking first.
    #[test]
    fn antigravitys_modes_mark_accept_edits_and_auto_only() {
        let list = json!([
            {"id": "default", "name": "Default", "description": "Ask before edits and commands"},
            {"id": "accept-edits", "name": "Accept edits", "description": "Approve file edits in the workspace without asking"},
            {"id": "auto", "name": "Auto", "description": "Approve edits and safe commands; ask before irreversible actions"},
            {"id": "plan", "name": "Plan", "description": "Read and plan only; no edits"},
        ]);
        let marked: Vec<(String, bool)> = acp_options(Some(&list), "id", true)
            .iter()
            .map(|o| {
                (
                    o["id"].as_str().unwrap_or("").to_string(),
                    o.get("skipsPermissions").is_some(),
                )
            })
            .collect();
        assert_eq!(
            marked,
            [
                ("default".to_string(), false),
                ("accept-edits".to_string(), true),
                ("auto".to_string(), true),
                ("plan".to_string(), false)
            ]
        );
    }

    #[test]
    fn option_lists_are_bounded_and_ids_are_never_cut() {
        let long = "m".repeat(OPTION_ID_MAX + 1);
        let list = json!([{"modelId": "a", "name": "A"}, {"modelId": long}, {"modelId": ""}, {"name": "no id"},
                          {"modelId": "b", "description": "é".repeat(400)}]);
        let o = acp_options(Some(&list), "modelId", false);
        assert_eq!(o.len(), 2);
        assert_eq!(o[1]["id"], "b");
        assert!(o[1]["description"].as_str().map_or(0, str::len) <= OPTION_DESCRIPTION_MAX);
        assert!(o.iter().all(|v| v.get("skipsPermissions").is_none()));
        let many: Vec<Value> = (0..300).map(|i| json!({"id": format!("m{i}")})).collect();
        assert_eq!(
            acp_options(Some(&json!(many)), "id", true).len(),
            OPTIONS_MAX
        );
        assert_eq!(
            notice_params(&json!({"a": "x", "b": 3})),
            json!({"a": "x", "b": "3"})
        );
    }

    #[test]
    fn a_long_output_keeps_its_head_and_tail_on_character_boundaries() {
        assert_eq!(head_tail("short", 10), "short");
        let long = format!("{}{}{}", "a".repeat(100), "é".repeat(100), "z".repeat(100));
        let cut = head_tail(&long, 50);
        assert!(
            cut.len() <= 50 && cut.starts_with("aaa") && cut.ends_with("zzz"),
            "{cut}"
        );
        // Two-byte characters on both cut points.
        let wide = "é".repeat(101);
        assert!(head_tail(&wide, 51).chars().all(|c| c == 'é'));
    }

    #[test]
    fn a_tool_output_comes_from_text_blocks_or_the_raw_output_and_is_redacted() {
        let u = json!({"content": [{"type": "diff", "path": "a", "newText": "x"}]});
        assert_eq!(tool_output(&u), None, "a diff alone is not output");
        let u = json!({"content": [{"type": "content", "content": {"type": "text", "text": "a"}},
                                   {"type": "content", "content": {"type": "text", "text": "b"}}]});
        assert_eq!(tool_output(&u).as_deref(), Some("a\nb"));
        let u = json!({"rawOutput": {"exit": 0}});
        assert_eq!(tool_output(&u).as_deref(), Some("{\"exit\":0}"));
        let u = json!({"rawOutput": "token=abcdefghijklmnopqrstuvwxyz0123456789ABCDEF"});
        assert!(tool_output(&u)
            .is_some_and(|o| o.contains(REDACTED) && !o.contains("abcdefghijklmnopqrstuv")));
    }

    #[test]
    fn long_details_are_cut_on_a_character_boundary() {
        let s = short(&json!({"x": "é".repeat(400)}));
        assert!(s.ends_with('…') && s.len() <= MAX_DETAIL + 4);
    }

    /// Audit F54: an agent line that fits `MAX_LINE` never becomes a longer line to the node (whose host drops it whole): arguments
    /// escaped twice, a replacement diff, a rich plan sent twice and a chunk joined to buffered text all stay within it.
    #[test]
    fn an_agent_line_within_the_limit_stays_within_it_on_its_way_to_the_node(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let update = |u: Value| json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s", "update": u}});
        let cases = [
            vec![update(
                json!({"sessionUpdate": "tool_call", "toolCallId": "c1", "title": "Write",
                       "rawInput": {"content": "\"".repeat(1_900_000)}}),
            )],
            vec![update(
                json!({"sessionUpdate": "tool_call", "toolCallId": "c1", "title": "Write",
                       "content": [{"type": "diff", "path": "f", "oldText": "a\n".repeat(650_000), "newText": "b\n".repeat(650_000)}]}),
            )],
            vec![update(json!({"sessionUpdate": "plan",
                "entries": (0..270_000).map(|_| json!({"content": ""})).collect::<Vec<_>>()}))],
            // A title is not capped: what the arguments grow by (escaped twice) takes a line that fits past the limit.
            vec![update(
                json!({"sessionUpdate": "tool_call", "toolCallId": "c1", "rawInput": {"a": "\"".repeat(60_000)},
                       "title": "\u{1}".repeat((MAX_LINE - 130_000) / 6)}),
            )],
            vec![
                update(
                    json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "x".repeat(4000)}}),
                ),
                update(
                    json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "y".repeat(MAX_LINE - 300)}}),
                ),
            ],
        ];
        for rich in [false, true] {
            for lines in &cases {
                let mut b = bridge_in_session()?;
                b.rich.on = rich;
                for line in lines {
                    assert!(line.to_string().len() <= MAX_LINE);
                    b.handle_agent(line)?;
                }
                b.flush_text()?;
                let sent: Vec<&[u8]> = b
                    .out
                    .split(|c| *c == b'\n')
                    .filter(|l| !l.is_empty())
                    .collect();
                assert!(!sent.is_empty());
                assert!(
                    sent.iter().all(|l| l.len() <= MAX_LINE),
                    "rich={rich}: {:?}",
                    sent.iter().map(|l| l.len()).collect::<Vec<_>>()
                );
            }
        }
        Ok(())
    }

    /// Audit F54 (review of #920): the per-field caps count decoded bytes and a rich plan carries its text twice, so a plan whose
    /// text is escaped (each control character six bytes, a quote or backslash two) stays under every cap and still doubled past
    /// `MAX_LINE`. The whole serialized line is bounded: the plan arrives cut, not dropped by the node's host.
    #[test]
    fn a_rich_plan_of_escaped_text_stays_within_the_line_limit(
    ) -> Result<(), Box<dyn std::error::Error>> {
        // 3600 decoded bytes (under `PLAN_TEXT_MAX`) that take 12000 escaped; 256 entries (`PLAN_ENTRIES_MAX`): a 3 MB line that
        // was sent to the node as a 6 MB one.
        let text = "\"\\\u{1}".repeat(1200);
        let entries: Vec<Value> = (0..PLAN_ENTRIES_MAX)
            .map(|_| json!({"content": text, "status": "pending", "priority": "high"}))
            .collect();
        let line = json!({"jsonrpc": "2.0", "method": "session/update",
            "params": {"sessionId": "s", "update": {"sessionUpdate": "plan", "entries": entries}}});
        assert!(line.to_string().len() <= MAX_LINE);
        for rich in [false, true] {
            let mut b = bridge_in_session()?;
            b.rich.on = rich;
            b.handle_agent(&line)?;
            let sent: Vec<&[u8]> = b
                .out
                .split(|c| *c == b'\n')
                .filter(|l| !l.is_empty())
                .collect();
            assert_eq!(sent.len(), 1, "rich={rich}");
            assert!(
                sent[0].len() <= MAX_LINE,
                "rich={rich}: {} bytes",
                sent[0].len()
            );
            let v: Value = serde_json::from_slice(sent[0])?;
            let plan = &v["params"]["plan"];
            let steps = plan["steps"].as_array().ok_or("no steps")?;
            assert!(!steps.is_empty(), "rich={rich}");
            // What is kept is the plan's own text, from its start.
            assert!(steps[0]
                .as_str()
                .ok_or("step")?
                .starts_with("\"\\\u{1}\"\\"));
            if rich {
                let kept = plan["entries"].as_array().ok_or("no entries")?;
                assert!(!kept.is_empty());
                assert_eq!(kept[0]["status"], "pending");
            } else {
                // A plain plan carries each text once, which cannot outgrow the agent's line: nothing is cut.
                assert_eq!(steps.len(), PLAN_ENTRIES_MAX);
                assert!(steps.iter().all(|s| s == &json!(text)));
            }
        }
        Ok(())
    }

    /// Audit F54 (Codex review of #920 at b80c630a): `fit_line` never cuts the session id, which came from the agent unchecked. A
    /// session id of almost 4 MiB was accepted (its reply fit), and a plan update for it with one small entry, which fit too, went
    /// to the node as a longer line. The session is refused now, and so is a resume of an id over the limit.
    #[test]
    fn a_session_id_over_the_limit_is_refused_so_no_line_about_it_outgrows_the_node_limit(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let lines = |out: &[u8]| -> Vec<Value> {
            out.split(|c| *c == b'\n')
                .filter_map(|l| serde_json::from_slice(l).ok())
                .collect()
        };
        for rich in [false, true] {
            let mut b = bridge_in_session()?;
            b.session = None;
            b.rich.on = rich;
            let _ = b.handle_node(
                &json!({"jsonrpc": "2.0", "id": 7, "method": "session/start", "params": {"worktree": "/w"}}),
            )?;
            let sid = "s".repeat(MAX_LINE - 400);
            b.handle_agent(&json!({"jsonrpc": "2.0", "id": b.next, "result": {"sessionId": sid}}))?;
            let plan = json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": sid,
                "update": {"sessionUpdate": "plan", "entries": [{"content": "step", "status": "pending", "priority": "high"}]}}});
            assert!(plan.to_string().len() <= MAX_LINE);
            b.handle_agent(&plan)?;
            assert!(
                b.out.split(|c| *c == b'\n').all(|l| l.len() <= MAX_LINE),
                "rich={rich}"
            );
            assert_eq!(b.session, None, "rich={rich}");
            let sent = lines(&b.out);
            let answer = sent.iter().find(|v| v["id"] == 7).ok_or("no answer")?;
            assert_eq!(answer["error"]["data"]["code"], "adapter.agent_refused");
            assert_eq!(answer["error"]["data"]["params"]["stage"], "session");
            assert!(sent
                .iter()
                .any(|v| v["params"]["notice"]["params"]["detail"] == "session_id_too_long"));
            assert!(!sent.iter().any(|v| v["params"].get("plan").is_some()));

            // One at the limit is accepted.
            let mut b = bridge_in_session()?;
            b.session = None;
            let _ = b.handle_node(
                &json!({"jsonrpc": "2.0", "id": 8, "method": "session/start", "params": {"worktree": "/w"}}),
            )?;
            let sid = "s".repeat(SESSION_ID_MAX);
            b.handle_agent(&json!({"jsonrpc": "2.0", "id": b.next, "result": {"sessionId": sid}}))?;
            assert_eq!(b.session.as_deref(), Some(sid.as_str()));

            // The node's resume of an id over the limit is refused before the agent is asked.
            let mut b = bridge_in_session()?;
            b.can_load = true;
            let asked = b.next;
            let _ = b.handle_node(
                &json!({"jsonrpc": "2.0", "id": 9, "method": "session/start",
                "params": {"worktree": "/w", "resumeSessionId": "r".repeat(SESSION_ID_MAX + 1)}}),
            )?;
            assert_eq!(b.next, asked);
            let sent = lines(&b.out);
            let answer = sent.iter().find(|v| v["id"] == 9).ok_or("no answer")?;
            assert_eq!(answer["error"]["data"]["code"], "protocol.invalid_message");
            assert_eq!(
                answer["error"]["data"]["params"]["field"],
                "resumeSessionId"
            );
        }
        Ok(())
    }

    /// `fit_line` never returns a line over the limit: when nothing is left to cut, it says so (the item is dropped and logged).
    #[test]
    fn fit_line_gives_up_rather_than_return_a_line_over_the_limit() {
        let small = json!({"sessionId": "s", "plan": {"steps": ["x".repeat(5000)]}});
        assert!(fit_line(small).is_ok());
        let stuck = json!({"sessionId": "s".repeat(MAX_LINE), "plan": {"steps": ["step"]}});
        assert!(matches!(fit_line(stuck), Err(n) if n > MAX_LINE));
    }

    /// A bridge with an open session and no turn, whose agent input goes nowhere (what it is sent is read from its state).
    fn bridge_in_session() -> std::io::Result<super::Bridge<Vec<u8>>> {
        let mut b = bridge_with_buffered_chat()?;
        b.text.clear();
        Ok(b)
    }

    fn states(out: &[u8]) -> Vec<String> {
        out.split(|b| *b == b'\n')
            .filter_map(|l| serde_json::from_slice::<serde_json::Value>(l).ok())
            .filter(|v| v["method"] == "state/update")
            .filter_map(|v| v["params"]["state"].as_str().map(str::to_string))
            .collect()
    }

    /// What the agent is sent, kept in memory.
    #[derive(Clone, Default)]
    struct Sent(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);

    impl std::io::Write for Sent {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl super::AgentInput for Sent {}

    impl Sent {
        /// The lines sent since the last call, as JSON.
        fn take(&self) -> Vec<serde_json::Value> {
            let bytes = std::mem::take(&mut *self.0.borrow_mut());
            bytes
                .split(|b| *b == b'\n')
                .filter_map(|l| serde_json::from_slice(l).ok())
                .collect()
        }
    }

    /// [`bridge_in_session`] whose agent input is kept, so a test reads what the agent was sent.
    fn bridge_with_agent() -> std::io::Result<(super::Bridge<Vec<u8>>, Sent)> {
        let mut b = bridge_in_session()?;
        let sent = Sent::default();
        b.agent = Box::new(sent.clone());
        Ok((b, sent))
    }

    /// The outcome the agent was sent for its permission request `id`, if any.
    fn answer_to(sent: &[serde_json::Value], id: &str) -> Option<String> {
        sent.iter()
            .find(|v| v["id"] == id && v.get("method").is_none())
            .and_then(|v| {
                v["result"]["outcome"]["outcome"]
                    .as_str()
                    .map(str::to_string)
            })
    }

    /// The texts of the prompts the agent was sent.
    fn prompts(sent: &[serde_json::Value]) -> Vec<String> {
        sent.iter()
            .filter(|v| v["method"] == "session/prompt")
            .filter_map(|v| {
                v["params"]["prompt"][0]["text"]
                    .as_str()
                    .map(str::to_string)
            })
            .collect()
    }

    fn prompt(b: &mut super::Bridge<Vec<u8>>, id: u64, text: &str) -> std::io::Result<()> {
        b.handle_node(&serde_json::json!({"jsonrpc":"2.0","id":id,"method":"session/prompt","params":{"text":text}}))
            .map(drop)
    }

    fn cancel(b: &mut super::Bridge<Vec<u8>>) -> std::io::Result<()> {
        b.handle_node(
            &serde_json::json!({"jsonrpc":"2.0","id":99,"method":"session/cancel","params":{}}),
        )
        .map(drop)
    }

    fn ask(b: &mut super::Bridge<Vec<u8>>, id: &str) -> std::io::Result<()> {
        b.handle_agent(&serde_json::json!({"jsonrpc":"2.0","id":id,"method":"session/request_permission",
            "params":{"sessionId":"s","toolCall":{"title":"edit"},"options":[{"optionId":"allow","kind":"allow_once"}]}}))
    }

    /// Audit F1240: prompts overlap when the node sends one into a running turn (a spawned helper's request, a node without
    /// `turns`). The reply to one of them must not end the other: no `idle` while it runs. Codex review of #794: a pending
    /// permission cannot be told to be the running prompt's (ACP names the session, not the prompt), so any prompt's reply answers
    /// it `cancelled` (fail closed; the agent asks again if it still needs it) and the state returns to `working`.
    #[test]
    fn the_reply_to_an_overlapping_prompt_does_not_end_the_running_turn(
    ) -> Result<(), Box<dyn std::error::Error>> {
        use serde_json::json;
        let (mut b, sent) = bridge_with_agent()?;
        prompt(&mut b, 10, "t")?;
        prompt(&mut b, 11, "t")?;
        ask(&mut b, "perm-1")?;
        b.out.clear();
        let _ = sent.take();
        // The agent refuses the second prompt while a permission waits for the user's decision.
        b.handle_agent(&json!({"jsonrpc":"2.0","id":2,"error":{"code":-32603,"message":"busy"}}))?;
        assert_eq!(
            states(&b.out),
            ["AGENT_STATE_WORKING"],
            "the first turn still runs"
        );
        assert!(b.perms.is_empty());
        assert_eq!(
            answer_to(&sent.take(), "perm-1").as_deref(),
            Some("cancelled")
        );
        b.out.clear();
        let _ = b.handle_node(&json!({"jsonrpc":"2.0","id":12,"method":"approval/resolve",
            "params":{"approvalId":"acp-3","approved":true}}))?;
        assert!(states(&b.out).is_empty(), "a late decision changes nothing");
        assert!(sent.take().is_empty(), "and sends the agent nothing");
        b.handle_agent(&json!({"jsonrpc":"2.0","id":1,"result":{"stopReason":"end_turn"}}))?;
        assert_eq!(
            states(&b.out),
            ["AGENT_STATE_IDLE"],
            "the last reply ends the turn"
        );
        Ok(())
    }

    /// Codex review of #794: while a cancelled prompt is unanswered, every permission request is answered `cancelled` and never
    /// shown (ACP names the session in it, not the prompt, so it may be the stopped turn's). A new prompt still goes to the agent
    /// at once, its reply ends the turn with `idle` though the cancelled prompt never answered, and that prompt's late reply changes
    /// nothing.
    #[test]
    fn a_cancelled_turns_permissions_stay_refused_and_a_new_prompt_is_not_held(
    ) -> Result<(), Box<dyn std::error::Error>> {
        use serde_json::json;
        let (mut b, sent) = bridge_with_agent()?;
        prompt(&mut b, 10, "t")?;
        cancel(&mut b)?;
        assert_eq!(prompts(&sent.take()), ["t"]);
        prompt(&mut b, 12, "u")?;
        assert_eq!(
            prompts(&sent.take()),
            ["u"],
            "the new prompt is sent at once"
        );
        b.out.clear();
        ask(&mut b, "old-perm")?;
        assert!(states(&b.out).is_empty(), "nobody is asked");
        assert!(b.perms.is_empty());
        assert_eq!(
            answer_to(&sent.take(), "old-perm").as_deref(),
            Some("cancelled")
        );
        b.handle_agent(&json!({"jsonrpc":"2.0","id":2,"result":{"stopReason":"end_turn"}}))?;
        assert_eq!(
            states(&b.out),
            ["AGENT_STATE_IDLE"],
            "the new prompt's reply ends the turn"
        );
        b.out.clear();
        b.handle_agent(&json!({"jsonrpc":"2.0","id":1,"result":{"stopReason":"cancelled"}}))?;
        assert!(b.out.is_empty(), "the late reply changes nothing");
        Ok(())
    }

    /// Once the cancelled prompt answered, the new turn's permission requests are asked as usual.
    #[test]
    fn a_permission_after_the_cancelled_prompt_answered_is_asked(
    ) -> Result<(), Box<dyn std::error::Error>> {
        use serde_json::json;
        let (mut b, sent) = bridge_with_agent()?;
        prompt(&mut b, 10, "t")?;
        cancel(&mut b)?;
        prompt(&mut b, 12, "u")?;
        b.handle_agent(&json!({"jsonrpc":"2.0","id":1,"result":{"stopReason":"cancelled"}}))?;
        b.out.clear();
        let _ = sent.take();
        ask(&mut b, "new-perm")?;
        assert_eq!(states(&b.out), ["AGENT_STATE_BLOCKED"]);
        let approval = b.perms.keys().next().cloned().ok_or("no approval")?;
        b.out.clear();
        let _ = b.handle_node(&json!({"jsonrpc":"2.0","id":13,"method":"approval/resolve",
            "params":{"approvalId":approval,"approved":true}}))?;
        assert_eq!(states(&b.out), ["AGENT_STATE_WORKING"]);
        assert_eq!(
            answer_to(&sent.take(), "new-perm").as_deref(),
            Some("selected")
        );
        b.out.clear();
        b.handle_agent(&json!({"jsonrpc":"2.0","id":2,"result":{"stopReason":"end_turn"}}))?;
        assert_eq!(states(&b.out), ["AGENT_STATE_IDLE"]);
        Ok(())
    }

    /// Codex review of #794: a permission the agent still waits for is answered `cancelled` before the bridge forgets it (here the
    /// turn's reply came while the request was pending), so the agent never waits for an answer that cannot come.
    #[test]
    fn a_permission_dropped_when_the_turn_ends_is_answered_cancelled(
    ) -> Result<(), Box<dyn std::error::Error>> {
        use serde_json::json;
        let (mut b, sent) = bridge_with_agent()?;
        prompt(&mut b, 10, "t")?;
        ask(&mut b, "perm")?;
        b.out.clear();
        b.handle_agent(&json!({"jsonrpc":"2.0","id":1,"result":{"stopReason":"end_turn"}}))?;
        assert_eq!(states(&b.out), ["AGENT_STATE_IDLE"]);
        assert!(b.perms.is_empty());
        assert_eq!(
            answer_to(&sent.take(), "perm").as_deref(),
            Some("cancelled")
        );
        Ok(())
    }

    /// Codex review of #794: a decision for a permission the bridge no longer holds (it answered it `cancelled` itself, or never
    /// knew it) is refused with an error and sends the agent nothing, so the node marks its entry cancelled instead of recording a
    /// decision that never reached the agent. A decision it holds still succeeds.
    #[test]
    fn a_decision_the_bridge_cannot_deliver_is_refused() -> Result<(), Box<dyn std::error::Error>> {
        use serde_json::json;
        let answer = |out: &[u8], id: u64| -> Option<serde_json::Value> {
            out.split(|b| *b == b'\n')
                .filter_map(|l| serde_json::from_slice::<serde_json::Value>(l).ok())
                .find(|v| v["id"] == id && v.get("method").is_none())
        };
        let resolve = |b: &mut super::Bridge<Vec<u8>>, id: u64, approval: &str| {
            b.handle_node(&json!({"jsonrpc":"2.0","id":id,"method":"approval/resolve",
                "params":{"approvalId":approval,"approved":true}}))
                .map(drop)
        };
        let (mut b, sent) = bridge_with_agent()?;
        prompt(&mut b, 10, "t")?;
        prompt(&mut b, 11, "u")?;
        ask(&mut b, "perm-1")?;
        let approval = b.perms.keys().next().cloned().ok_or("no approval")?;
        // The agent refuses the second prompt as busy: the bridge cancels the permission.
        b.handle_agent(&json!({"jsonrpc":"2.0","id":2,"error":{"code":-32603,"message":"busy"}}))?;
        let _ = sent.take();
        for (id, approval) in [(20, approval.as_str()), (21, "acp-99")] {
            b.out.clear();
            resolve(&mut b, id, approval)?;
            let reply = answer(&b.out, id).ok_or("no reply")?;
            assert_eq!(reply["error"]["code"], -32602, "{approval}: {reply}");
            assert!(reply.get("result").is_none(), "{approval}: {reply}");
            assert!(states(&b.out).is_empty(), "{approval}");
            assert!(
                sent.take().is_empty(),
                "{approval}: the agent is sent nothing"
            );
        }
        for full in [true, false] {
            let (mut b, _) = bridge_with_agent()?;
            prompt(&mut b, 10, "t")?;
            ask(&mut b, "perm-unavailable")?;
            let approval = b.perms.keys().next().cloned().ok_or("no approval")?;
            b.out.clear();
            let (pending, rx) = std::sync::mpsc::sync_channel(1);
            let queued = if full {
                pending.try_send((Box::from(&b"pending\n"[..]), 0))?;
                Some(rx)
            } else {
                drop(rx);
                None
            };
            b.agent = Box::new(super::AgentWriter {
                line: Vec::new(),
                pending,
                queued: Default::default(),
                broken: Default::default(),
            });
            resolve(&mut b, 23, &approval)?;
            let reply = answer(&b.out, 23).ok_or("no reply")?;
            assert_eq!(reply["error"]["code"], -32603, "{reply}");
            assert!(reply.get("result").is_none(), "{reply}");
            assert!(states(&b.out).is_empty());
            assert_eq!(b.agent_writer_full, full);
            if let Some(rx) = queued {
                assert_eq!(&*rx.try_recv()?.0, b"pending\n");
                assert!(rx.try_recv().is_err());
            }
        }
        // A decision the bridge holds is delivered and succeeds.
        let (mut b, sent) = bridge_with_agent()?;
        prompt(&mut b, 10, "t")?;
        ask(&mut b, "perm-2")?;
        let approval = b.perms.keys().next().cloned().ok_or("no approval")?;
        b.out.clear();
        resolve(&mut b, 22, &approval)?;
        let reply = answer(&b.out, 22).ok_or("no reply")?;
        assert_eq!(reply["result"], json!({}), "{reply}");
        assert_eq!(
            answer_to(&sent.take(), "perm-2").as_deref(),
            Some("selected")
        );
        Ok(())
    }

    /// ADR-0101, Codex review of PR #1007: `approval/resolve` succeeds only once the writer wrote the answer to the agent's stdin and
    /// flushed it, never when it was only queued. A write that fails (or an agent that ends first) is an error, so the node records no
    /// always answer; one that waits longer than [`super::DELIVER_WAIT`] is an error too, and the bridge ends the agent.
    #[test]
    fn an_answer_is_acknowledged_only_once_the_writer_gave_it_to_the_agent(
    ) -> Result<(), Box<dyn std::error::Error>> {
        use serde_json::json;
        use std::sync::mpsc::sync_channel;
        struct Broken;
        impl std::io::Write for Broken {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        #[derive(Clone, Default)]
        struct Shared(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
        impl std::io::Write for Shared {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let always = |b: &mut super::Bridge<Vec<u8>>, id: u64| {
            let approval = b.perms.keys().next().cloned().unwrap_or_default();
            b.handle_node(&json!({"jsonrpc":"2.0","id":id,"method":"approval/resolve",
                "params":{"approvalId":approval,"approved":true,"scope":"APPROVAL_SCOPE_ALWAYS"}}))
                .map(drop)
        };
        let outcome = |rx: &std::sync::mpsc::Receiver<super::Ev>| match rx
            .recv_timeout(std::time::Duration::from_secs(10))
        {
            Ok(super::Ev::Written(token, ok)) => Some((token, ok)),
            _ => None,
        };
        let ready = |agent: Box<dyn super::AgentInput>| -> std::io::Result<super::Bridge<Vec<u8>>> {
            let mut b = bridge_in_session()?;
            prompt(&mut b, 10, "t")?;
            ask_with(&mut b, "perm-1", all_four())?;
            b.agent = agent;
            b.out.clear();
            Ok(b)
        };

        // Written and flushed: the node hears success only after the writer said so.
        let (tx, rx) = sync_channel(8);
        let stdin = Shared::default();
        let mut b = ready(Box::new(super::AgentWriter::start(stdin.clone(), tx)))?;
        always(&mut b, 30)?;
        assert!(
            reply_to(&b.out, 30).is_none(),
            "acknowledged when only queued"
        );
        let (token, ok) = outcome(&rx).ok_or("no outcome")?;
        assert!(ok);
        b.written(token, ok)?;
        assert_eq!(reply_to(&b.out, 30).ok_or("no reply")?["result"], json!({}));
        let written = String::from_utf8(stdin.0.lock().unwrap_or_else(|e| e.into_inner()).clone())?;
        assert!(written.contains("\"optionId\":\"aa\""), "{written}");

        // The write fails: an error, never a success, and nothing more is queued.
        let (tx, rx) = sync_channel(8);
        let mut b = ready(Box::new(super::AgentWriter::start(Broken, tx)))?;
        always(&mut b, 31)?;
        assert!(reply_to(&b.out, 31).is_none());
        let (token, ok) = outcome(&rx).ok_or("no outcome")?;
        assert!(!ok);
        b.written(token, ok)?;
        let reply = reply_to(&b.out, 31).ok_or("no reply")?;
        assert_eq!(
            reply["error"]["message"], "adapter.approval_undelivered",
            "{reply}"
        );
        assert!(reply.get("result").is_none(), "{reply}");
        // A late or repeated outcome changes nothing.
        b.out.clear();
        b.written(token, true)?;
        assert!(b.out.is_empty());

        // The agent ends before the write was confirmed: an error.
        let (pending, _held) = sync_channel(4);
        let stuck = super::AgentWriter {
            line: Vec::new(),
            pending,
            queued: Default::default(),
            broken: Default::default(),
        };
        let mut b = ready(Box::new(stuck))?;
        always(&mut b, 32)?;
        assert!(reply_to(&b.out, 32).is_none());
        b.agent_gone(Some(1), "")?;
        let reply = reply_to(&b.out, 32).ok_or("no reply")?;
        assert_eq!(
            reply["error"]["message"], "adapter.approval_undelivered",
            "{reply}"
        );

        // The writer never gets to it: refused after the wait, and the caller ends the agent.
        let (pending, _held) = sync_channel(4);
        let stuck = super::AgentWriter {
            line: Vec::new(),
            pending,
            queued: Default::default(),
            broken: Default::default(),
        };
        let mut b = ready(Box::new(stuck))?;
        always(&mut b, 33)?;
        let now = std::time::Instant::now();
        assert!(b.take_overdue(now).is_empty());
        assert!(reply_to(&b.out, 33).is_none());
        let late = b.take_overdue(now + super::DELIVER_WAIT);
        assert_eq!(late.len(), 1);
        // Taking it tells the node nothing yet: the bridge ends the agent first, then reports.
        assert!(reply_to(&b.out, 33).is_none());
        b.unconfirmed(&late)?;
        let reply = reply_to(&b.out, 33).ok_or("no reply")?;
        assert_eq!(
            reply["error"]["message"], "adapter.approval_undelivered",
            "{reply}"
        );
        Ok(())
    }

    /// Audit F1241, reworked in the Codex review of #794: a permission request with no prompt running (a straggler of a turn that
    /// ended, or of a cancelled one) has no turn to belong to. It is answered `cancelled` at once and the node hears nothing: a
    /// `blocked` here could be read as the start of the node's next prompt, and its decision would end that turn early.
    #[test]
    fn a_permission_asked_outside_a_turn_is_refused() -> Result<(), Box<dyn std::error::Error>> {
        use serde_json::json;
        let (mut b, sent) = bridge_with_agent()?;
        let refused = |b: &mut super::Bridge<Vec<u8>>, id: &str| -> std::io::Result<()> {
            b.out.clear();
            let _ = sent.take();
            ask(b, id)?;
            assert!(b.out.is_empty(), "{id}: the node hears nothing");
            assert!(b.perms.is_empty(), "{id}: nothing is kept");
            assert_eq!(
                answer_to(&sent.take(), id).as_deref(),
                Some("cancelled"),
                "{id}: the agent is answered at once"
            );
            Ok(())
        };
        refused(&mut b, "before-any-prompt")?;
        prompt(&mut b, 10, "t")?;
        cancel(&mut b)?;
        b.handle_agent(&json!({"jsonrpc":"2.0","id":1,"result":{"stopReason":"cancelled"}}))?;
        refused(&mut b, "after-a-cancelled-turn")?;
        prompt(&mut b, 12, "u")?;
        b.handle_agent(&json!({"jsonrpc":"2.0","id":2,"result":{"stopReason":"end_turn"}}))?;
        refused(&mut b, "after-a-finished-turn")?;
        Ok(())
    }

    /// Codex review of #794: a prompt that comes while a permission is pending does not report `working`: the agent still waits for
    /// the user. Its refusal as busy is a prompt's reply, so it answers the permission `cancelled` (whose it was cannot be told) and
    /// the state returns to `working` for the turn still open; the last reply reports `idle`.
    #[test]
    fn a_pending_permission_keeps_the_agent_blocked_when_another_prompt_comes(
    ) -> Result<(), Box<dyn std::error::Error>> {
        use serde_json::json;
        let (mut b, sent) = bridge_with_agent()?;
        prompt(&mut b, 10, "t")?;
        ask(&mut b, "perm-1")?;
        b.out.clear();
        prompt(&mut b, 11, "u")?;
        assert_eq!(states(&b.out), ["AGENT_STATE_BLOCKED"]);
        assert_eq!(prompts(&sent.take()), ["t", "u"]);
        b.out.clear();
        b.handle_agent(&json!({"jsonrpc":"2.0","id":3,"error":{"code":-32603,"message":"busy"}}))?;
        assert_eq!(states(&b.out), ["AGENT_STATE_WORKING"]);
        assert_eq!(
            answer_to(&sent.take(), "perm-1").as_deref(),
            Some("cancelled")
        );
        b.out.clear();
        b.handle_agent(&json!({"jsonrpc":"2.0","id":1,"result":{"stopReason":"end_turn"}}))?;
        assert_eq!(states(&b.out), ["AGENT_STATE_IDLE"]);
        Ok(())
    }

    // ---- ADR-0101: the offer travels with the request, and the decision's scope picks exactly one option ----

    fn ask_with(
        b: &mut super::Bridge<Vec<u8>>,
        id: &str,
        options: serde_json::Value,
    ) -> std::io::Result<()> {
        b.handle_agent(&serde_json::json!({"jsonrpc":"2.0","id":id,"method":"session/request_permission",
            "params":{"sessionId":"s","toolCall":{"title":"run_command","rawInput":{"command":"git status"}},"options":options}}))
    }

    /// The node's notes of one method.
    fn notes_of(out: &[u8], method: &str) -> Vec<serde_json::Value> {
        out.split(|b| *b == b'\n')
            .filter_map(|l| serde_json::from_slice::<serde_json::Value>(l).ok())
            .filter(|v| v["method"] == method)
            .map(|v| v["params"].clone())
            .collect()
    }

    /// The option the agent was sent for its permission request `id`, if one was selected.
    fn option_sent(sent: &[serde_json::Value], id: &str) -> Option<String> {
        sent.iter()
            .find(|v| v["id"] == id && v.get("method").is_none())
            .and_then(|v| {
                v["result"]["outcome"]["optionId"]
                    .as_str()
                    .map(str::to_string)
            })
    }

    fn reply_to(out: &[u8], id: u64) -> Option<serde_json::Value> {
        out.split(|b| *b == b'\n')
            .filter_map(|l| serde_json::from_slice::<serde_json::Value>(l).ok())
            .find(|v| v["id"] == id && v.get("method").is_none())
    }

    fn all_four() -> serde_json::Value {
        serde_json::json!([
            {"optionId":"ra","name":"Never","kind":"reject_always"},
            {"optionId":"aa","name":"Always allow git","kind":"allow_always"},
            {"optionId":"ro","name":"No","kind":"reject_once"},
            {"optionId":"ao","name":"Yes","kind":"allow_once"}
        ])
    }

    #[test]
    fn the_bridge_declares_strict_scopes() -> Result<(), Box<dyn std::error::Error>> {
        let b = bridge_in_session()?;
        assert!(b.capabilities().contains(&"approval_scopes"));
        Ok(())
    }

    /// The request names which answers the agent offers, the always option's own name and its exact id.
    #[test]
    fn the_offer_travels_with_the_request() -> Result<(), Box<dyn std::error::Error>> {
        use serde_json::json;
        for (options, scopes, label, option) in [
            (
                all_four(),
                json!(["APPROVAL_SCOPE_ONCE", "APPROVAL_SCOPE_ALWAYS"]),
                "Always allow git",
                "aa",
            ),
            (
                json!([{"optionId":"ao","name":"Yes","kind":"allow_once"}]),
                json!(["APPROVAL_SCOPE_ONCE"]),
                "",
                "",
            ),
            (
                json!([{"optionId":"aa","name":"Always","kind":"allow_always"},{"optionId":"ro","name":"No","kind":"reject_once"}]),
                json!(["APPROVAL_SCOPE_ALWAYS"]),
                "Always",
                "aa",
            ),
            // Two always options are ambiguous (Codex review of ADR-0101): ALWAYS is withheld, the single answer stays.
            (
                json!([{"optionId":"a1","name":"A","kind":"allow_always"},{"optionId":"a2","name":"A","kind":"allow_always"},
                    {"optionId":"ao","name":"Yes","kind":"allow_once"}]),
                json!(["APPROVAL_SCOPE_ONCE"]),
                "",
                "",
            ),
        ] {
            let (mut b, _) = bridge_with_agent()?;
            prompt(&mut b, 10, "t")?;
            b.out.clear();
            ask_with(&mut b, "p", options.clone())?;
            let req = notes_of(&b.out, "approval/request");
            assert_eq!(req.len(), 1, "{options}");
            assert_eq!(req[0]["scopes"], scopes, "{options}");
            assert_eq!(req[0]["alwaysLabel"], json!(label), "{options}");
            assert_eq!(req[0]["alwaysOptionId"], json!(option), "{options}");
        }
        Ok(())
    }

    /// A plain Allow (ONCE, or a node that sends no scope) is never answered with a remembered option, and ALWAYS selects only the
    /// one always option the request offered. An option of an unknown kind is never selected.
    #[test]
    fn once_never_selects_allow_always_and_always_selects_only_it(
    ) -> Result<(), Box<dyn std::error::Error>> {
        use serde_json::json;
        let cases: Vec<(serde_json::Value, serde_json::Value, bool, Option<&str>)> = vec![
            (all_four(), json!(null), true, Some("ao")),
            (
                all_four(),
                json!("APPROVAL_SCOPE_UNSPECIFIED"),
                true,
                Some("ao"),
            ),
            (all_four(), json!("APPROVAL_SCOPE_ONCE"), true, Some("ao")),
            (all_four(), json!("APPROVAL_SCOPE_ALWAYS"), true, Some("aa")),
            // Denying never widens: `reject_once`, else `cancelled`, never `reject_always`.
            (all_four(), json!("APPROVAL_SCOPE_ONCE"), false, Some("ro")),
            (
                json!([{"optionId":"ra","kind":"reject_always"},{"optionId":"ao","kind":"allow_once"}]),
                json!(null),
                false,
                None,
            ),
        ];
        for (options, scope, approved, want) in cases {
            let (mut b, sent) = bridge_with_agent()?;
            prompt(&mut b, 10, "t")?;
            ask_with(&mut b, "p", options.clone())?;
            let approval = b.perms.keys().next().cloned().ok_or("no approval")?;
            let _ = sent.take();
            b.out.clear();
            let mut params = json!({"approvalId": approval, "approved": approved});
            if !scope.is_null() {
                params["scope"] = scope.clone();
            }
            let _ = b.handle_node(
                &json!({"jsonrpc":"2.0","id":30,"method":"approval/resolve","params":params}),
            )?;
            let reply = reply_to(&b.out, 30).ok_or("no reply")?;
            assert_eq!(reply["result"], json!({}), "{options} {scope}: {reply}");
            let sent = sent.take();
            assert_eq!(
                option_sent(&sent, "p").as_deref(),
                want,
                "{options} {scope} {approved}"
            );
            if want.is_none() {
                assert_eq!(answer_to(&sent, "p").as_deref(), Some("cancelled"));
            }
        }
        Ok(())
    }

    /// The decision names an answer the request did not offer (ONCE on an always-only request, ALWAYS on a once-only one), or a scope
    /// this build does not know: refused with `approval.scope_unavailable` / an invalid scope, the agent is sent nothing, and the
    /// request is still held (it can be answered again, or is answered `cancelled` when the turn ends).
    #[test]
    fn an_answer_the_agent_did_not_offer_is_refused_and_keeps_the_request(
    ) -> Result<(), Box<dyn std::error::Error>> {
        use serde_json::json;
        let always_only = json!([{"optionId":"aa","name":"Always","kind":"allow_always"},{"optionId":"ro","kind":"reject_once"}]);
        let once_only =
            json!([{"optionId":"ao","kind":"allow_once"},{"optionId":"x","kind":"something_new"}]);
        for (options, scope, approved, code) in [
            (
                always_only.clone(),
                json!("APPROVAL_SCOPE_ONCE"),
                true,
                "approval.scope_unavailable",
            ),
            (
                always_only.clone(),
                json!(null),
                true,
                "approval.scope_unavailable",
            ),
            (
                once_only.clone(),
                json!("APPROVAL_SCOPE_ALWAYS"),
                true,
                "approval.scope_unavailable",
            ),
            (
                once_only.clone(),
                json!("APPROVAL_SCOPE_BOGUS"),
                true,
                "protocol.invalid_message",
            ),
            (
                once_only.clone(),
                json!(7),
                true,
                "protocol.invalid_message",
            ),
            // A remembered refusal is reserved (ADR-0101): never sent.
            (
                all_four(),
                json!("APPROVAL_SCOPE_ALWAYS"),
                false,
                "protocol.invalid_message",
            ),
        ] {
            let (mut b, sent) = bridge_with_agent()?;
            prompt(&mut b, 10, "t")?;
            ask_with(&mut b, "p", options.clone())?;
            let approval = b.perms.keys().next().cloned().ok_or("no approval")?;
            let _ = sent.take();
            b.out.clear();
            let mut params = json!({"approvalId": approval, "approved": approved});
            if !scope.is_null() {
                params["scope"] = scope.clone();
            }
            let _ = b.handle_node(
                &json!({"jsonrpc":"2.0","id":31,"method":"approval/resolve","params":params}),
            )?;
            let reply = reply_to(&b.out, 31).ok_or("no reply")?;
            assert_eq!(reply["error"]["code"], -32602, "{options} {scope}: {reply}");
            assert_eq!(
                reply["error"]["data"]["code"],
                json!(code),
                "{options} {scope}: {reply}"
            );
            assert!(
                sent.take().is_empty(),
                "{options} {scope}: the agent is sent nothing"
            );
            assert!(
                b.perms.contains_key(&approval),
                "{options} {scope}: the request is still held"
            );
            assert!(states(&b.out).is_empty());
        }
        // A deny still works on an always-only request (a signed ONCE deny stays valid there).
        let (mut b, sent) = bridge_with_agent()?;
        prompt(&mut b, 10, "t")?;
        ask_with(&mut b, "p", always_only)?;
        let approval = b.perms.keys().next().cloned().ok_or("no approval")?;
        let _ = sent.take();
        let _ = b.handle_node(&json!({"jsonrpc":"2.0","id":32,"method":"approval/resolve",
            "params":{"approvalId": approval, "approved": false, "scope": "APPROVAL_SCOPE_ONCE"}}))?;
        assert_eq!(option_sent(&sent.take(), "p").as_deref(), Some("ro"));
        Ok(())
    }

    /// A request that offers no way to allow it (no `allow_once`, and no single `allow_always`) raises no card: the agent is answered
    /// `reject_once` (else `cancelled`), and the node gets a log line and a notice with the action.
    #[test]
    fn a_request_that_cannot_be_allowed_raises_no_card() -> Result<(), Box<dyn std::error::Error>> {
        use serde_json::json;
        for (options, want) in [
            (
                json!([{"optionId":"ro","kind":"reject_once"},{"optionId":"ra","kind":"reject_always"}]),
                Some("ro"),
            ),
            (
                json!([{"optionId":"ra","kind":"reject_always"},{"optionId":"x","kind":"something_new"}]),
                None,
            ),
            (
                json!([{"optionId":"a1","kind":"allow_always"},{"optionId":"a2","kind":"allow_always"}]),
                None,
            ),
            (json!([]), None),
        ] {
            let (mut b, sent) = bridge_with_agent()?;
            prompt(&mut b, 10, "t")?;
            let _ = sent.take();
            b.out.clear();
            ask_with(&mut b, "p", options.clone())?;
            assert!(notes_of(&b.out, "approval/request").is_empty(), "{options}");
            assert!(states(&b.out).is_empty(), "{options}: not blocked");
            assert!(b.perms.is_empty(), "{options}");
            let sent = sent.take();
            assert_eq!(option_sent(&sent, "p").as_deref(), want, "{options}");
            if want.is_none() {
                assert_eq!(
                    answer_to(&sent, "p").as_deref(),
                    Some("cancelled"),
                    "{options}"
                );
            }
            let notices = notes_of(&b.out, "chat/append");
            assert_eq!(notices.len(), 1, "{options}");
            assert_eq!(
                notices[0]["notice"]["code"],
                "adapter.permission_unanswerable"
            );
            assert_eq!(notices[0]["notice"]["params"]["action"], "run_command");
            let logs = notes_of(&b.out, "log");
            assert!(
                logs.iter().any(|l| l["message"]
                    .as_str()
                    .is_some_and(|m| m.contains("adapter.permission_unanswerable"))),
                "{options}: {logs:?}"
            );
        }
        Ok(())
    }
}
