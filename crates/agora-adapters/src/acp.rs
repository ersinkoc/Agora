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
//! - `session/request_permission` → `approval/request` and state `blocked`; `approval/resolve` picks the agent's own allow or reject
//!   option (`allow_once` before `allow_always`, `reject_once` before `reject_always`, otherwise the first option or `cancelled`).
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
use std::process::{Child, ChildStderr, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// How long agent text chunks are joined into one chat item.
const JOIN: Duration = Duration::from_millis(150);
/// The most text one joined item holds before it is sent anyway.
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
/// How long a bridge whose agent died before the node asked anything waits for the node's first request, to answer it with the
/// reason.
const LATE_REQUEST_WAIT: Duration = Duration::from_secs(2);
/// ADR-0058: the most modes or models passed on, and the longest id, name and description kept. An option whose id is longer is
/// dropped (a cut id would name another option).
const OPTIONS_MAX: usize = 200;
const OPTION_ID_MAX: usize = 200;
const OPTION_NAME_MAX: usize = 200;
const OPTION_DESCRIPTION_MAX: usize = 500;
/// ADR-0058: words in a mode's id or name (lowercase, letters and digits only) that mark a mode in which the agent acts without
/// asking. ACP has no field for it, so the bridge errs on the side of asking the user to confirm.
const SKIPS_PERMISSIONS: &[&str] = &[
    "bypass",
    "yolo",
    "danger",
    "skippermission",
    "nopermission",
    "noapproval",
    "fullaccess",
    "autoapprove",
    "autoaccept",
    "acceptedits",
    "acceptall",
    "unsafe",
];

enum Ev {
    Node(Value),
    Agent(Value),
    NodeGone,
    AgentGone,
}

enum Waiting {
    /// The node's `initialize` is answered only when the ACP agent has answered its own: a strict agent refuses
    /// `session/new` before that.
    Initialize {
        node_id: Value,
    },
    NewSession {
        node_id: Value,
    },
    Prompt,
    /// ADR-0058: the node's `session/set_option`, until the agent answered `session/set_mode` or `session/set_model`.
    SetOption {
        node_id: Value,
        mode: bool,
        id: String,
    },
}

struct Permission {
    /// The id of the agent's `session/request_permission` request.
    request: Value,
    options: Vec<(String, String)>, // (option id, kind)
}

fn lines<R: Read + Send + 'static>(
    r: R,
    wrap: fn(Value) -> Ev,
    gone: Ev,
    tx: std::sync::mpsc::Sender<Ev>,
) {
    std::thread::spawn(move || {
        // Lines as bytes: one that is not UTF-8 must not read as the end of the stream.
        for line in BufReader::new(r).split(b'\n') {
            let Ok(line) = line else { break };
            // A line that is not JSON (or not UTF-8) is the agent's own noise; it is ignored like any unknown message.
            if let Ok(v) = serde_json::from_slice::<Value>(&line) {
                if tx.send(wrap(v)).is_err() {
                    return;
                }
            }
        }
        let _ = tx.send(gone);
    });
}

fn write_line<W: Write>(w: &mut W, v: &Value) -> io::Result<()> {
    writeln!(w, "{v}")?;
    w.flush()
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
    for l in old_text.lines() {
        out.push('-');
        out.push_str(l);
        out.push('\n');
    }
    for l in new.lines() {
        out.push('+');
        out.push_str(l);
        out.push('\n');
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

/// ADR-0058: whether a mode's id or name says the agent acts without asking in it.
fn skips_permissions(id: &str, name: &str) -> bool {
    let squash = |s: &str| -> String {
        s.chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|c| c.to_ascii_lowercase())
            .collect()
    };
    let (id, name) = (squash(id), squash(name));
    SKIPS_PERMISSIONS
        .iter()
        .any(|w| id.contains(w) || name.contains(w))
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
                    if mode && skips_permissions(id, &name) {
                        v["skipsPermissions"] = json!(true);
                    }
                    Some(v)
                })
                .take(OPTIONS_MAX)
                .collect()
        })
        .unwrap_or_default()
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
            redact(&l)
        })
        .collect();
    let tail = lines[lines.len().saturating_sub(STDERR_LINES)..].join("\n");
    if tail.len() <= STDERR_TAIL {
        return tail;
    }
    let mut start = tail.len() - STDERR_TAIL;
    while !tail.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", &tail[start..])
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
fn keep_stderr(r: ChildStderr, kept: Arc<Mutex<Vec<u8>>>) -> Receiver<()> {
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
            k.extend_from_slice(&buf[..n]);
            let over = k.len().saturating_sub(STDERR_KEEP);
            k.drain(..over);
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

struct Bridge<W: Write> {
    out: W,
    agent: std::process::ChildStdin,
    next: u64,
    waiting: HashMap<u64, Waiting>,
    session: Option<String>,
    perms: HashMap<String, Permission>,
    text: String,
    text_since: Instant,
    can_load: bool,
    /// Something reached the node during the current turn (text, a tool call, a plan, a diff, an approval).
    said: bool,
    /// The model the agent said it uses (`session/new` `models.currentModelId`), for usage reports.
    model: String,
    /// The agent's cumulative session cost as last reported: (currency, micro-units).
    cost: Option<(String, u64)>,
    /// The context's level and size as last reported (ADR-0058).
    context: (u64, u64),
    /// ADR-0058: the modes and models the agent offers (adapter-protocol options) and the ones in use.
    modes: Vec<Value>,
    models: Vec<Value>,
    current_mode: String,
}

impl<W: Write> Bridge<W> {
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
        if self.modes.is_empty() && self.models.is_empty() {
            return Ok(());
        }
        let sid = self.session.clone().unwrap_or_default();
        let (modes, models) = (self.modes.clone(), self.models.clone());
        let (mode, model) = (self.current_mode.clone(), self.model.clone());
        self.note(
            "session/options",
            json!({"sessionId": sid, "modes": modes, "models": models, "currentMode": mode, "currentModel": model}),
        )
    }

    fn chat(&mut self, item: Value) -> io::Result<()> {
        self.said = true;
        let sid = self.session.clone().unwrap_or_default();
        let mut p = item;
        p["sessionId"] = json!(sid);
        self.note("chat/append", p)
    }

    fn flush_text(&mut self) -> io::Result<()> {
        if self.text.is_empty() {
            return Ok(());
        }
        let t = std::mem::take(&mut self.text);
        self.chat(json!({"text": {"text": t}}))
    }

    /// A line to the agent. A failed write means the agent is gone or going: the end of its output follows and is reported then
    /// (with what was still waiting), so the failure itself is not an error of the bridge.
    fn send_agent(&mut self, v: &Value) {
        let _ = write_line(&mut self.agent, v);
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

    /// The agent's output ended and it exited (`exit_code` when known). Every request the node still waits for is answered with
    /// `adapter.agent_exited` and the stage it was in; a `log` line says the same, so an exit in the middle of a turn or while
    /// idle is reported too.
    fn agent_gone(&mut self, exit_code: Option<i32>, stderr_tail: &str) -> io::Result<()> {
        self.flush_text()?;
        let mut waiting: Vec<(u64, Waiting)> = self.waiting.drain().collect();
        waiting.sort_by_key(|(n, _)| *n);
        let stage_of = |w: &Waiting| match w {
            Waiting::Initialize { .. } => "initialize",
            Waiting::NewSession { .. } => "session",
            Waiting::Prompt => "turn",
            Waiting::SetOption { .. } => "option",
        };
        let stage = ["initialize", "session", "turn"]
            .into_iter()
            .find(|s| waiting.iter().any(|(_, w)| stage_of(w) == *s))
            .unwrap_or(if self.session.is_some() {
                "idle"
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
            | Waiting::NewSession { node_id }
            | Waiting::SetOption { node_id, .. } = w
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
        let stage = match msg.get("method").and_then(Value::as_str) {
            Some("initialize") => "initialize",
            Some("session/start") => "session",
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
        let (method, key) = if mode {
            ("session/set_mode", "modeId")
        } else {
            ("session/set_model", "modelId")
        };
        self.ask_agent(
            method,
            json!({"sessionId": sid, key: want}),
            Waiting::SetOption {
                node_id,
                mode,
                id: want.clone(),
            },
        );
        Ok(())
    }

    /// A request or notification from the node.
    fn handle_node(&mut self, msg: &Value) -> io::Result<bool> {
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let done = |me: &mut Self| match &id {
            Some(id) => me.reply(id, json!({})),
            None => Ok(()),
        };
        match method {
            "initialize" => {
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
                match resume.filter(|_| self.can_load) {
                    Some(r) => {
                        self.session = Some(r.clone());
                        self.ask_agent(
                            "session/load",
                            json!({"sessionId": r, "cwd": cwd, "mcpServers": []}),
                            Waiting::NewSession { node_id },
                        );
                    }
                    None => self.ask_agent(
                        "session/new",
                        json!({"cwd": cwd, "mcpServers": []}),
                        Waiting::NewSession { node_id },
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
                    return Ok(false);
                };
                done(self)?;
                self.said = false;
                self.state("WORKING")?;
                self.ask_agent(
                    "session/prompt",
                    json!({"sessionId": sid, "prompt": [{"type": "text", "text": text}]}),
                    Waiting::Prompt,
                );
            }
            "session/cancel" => {
                done(self)?;
                if let Some(sid) = self.session.clone() {
                    self.tell_agent("session/cancel", json!({"sessionId": sid}));
                }
            }
            "approval/resolve" => {
                done(self)?;
                let approval = params
                    .get("approvalId")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let approved = params
                    .get("approved")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if let Some(p) = self.perms.remove(approval) {
                    let want: &[&str] = if approved {
                        &["allow_once", "allow_always"]
                    } else {
                        &["reject_once", "reject_always"]
                    };
                    let chosen = want
                        .iter()
                        .find_map(|k| p.options.iter().find(|(_, kind)| kind == k))
                        .or_else(|| if approved { p.options.first() } else { None });
                    let outcome = match chosen {
                        Some((option, _)) => json!({"outcome": "selected", "optionId": option}),
                        None => json!({"outcome": "cancelled"}),
                    };
                    self.send_agent(
                        &json!({"jsonrpc": "2.0", "id": p.request, "result": {"outcome": outcome}}),
                    );
                    if self.perms.is_empty() {
                        self.state("WORKING")?;
                    }
                }
            }
            "session/set_option" => self.set_option(id, &params)?,
            "session/stop" | "shutdown" => {
                done(self)?;
                return Ok(true);
            }
            // `mesh/deliver` and anything else new: acknowledged, not acted on.
            _ => done(self)?,
        }
        Ok(false)
    }

    fn update(&mut self, u: &Value) -> io::Result<()> {
        match u.get("sessionUpdate").and_then(Value::as_str).unwrap_or("") {
            "agent_message_chunk" => {
                if let Some(t) = u.pointer("/content/text").and_then(Value::as_str) {
                    if self.text.is_empty() {
                        self.text_since = Instant::now();
                    }
                    self.text.push_str(t);
                    if self.text.len() >= JOIN_MAX {
                        self.flush_text()?;
                    }
                }
            }
            "tool_call" => {
                self.flush_text()?;
                let title = u
                    .get("title")
                    .and_then(Value::as_str)
                    .or_else(|| u.get("kind").and_then(Value::as_str))
                    .unwrap_or("tool");
                let args = u.get("rawInput").map(Value::to_string).unwrap_or_default();
                self.chat(json!({"toolCall": {"tool": title, "argumentsJson": args}}))?;
                self.diffs_in(u)?;
            }
            "tool_call_update" => self.diffs_in(u)?,
            "plan" => {
                self.flush_text()?;
                let steps: Vec<String> = u
                    .get("entries")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|e| {
                                e.get("content").and_then(Value::as_str).map(str::to_string)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                self.chat(json!({"plan": {"steps": steps}}))?;
            }
            "usage_update" => self.usage(u)?,
            "current_mode_update" => {
                let id = u.get("currentModeId").and_then(Value::as_str).unwrap_or("");
                if !id.is_empty() && id != self.current_mode && id.len() <= OPTION_ID_MAX {
                    self.current_mode = id.to_string();
                    self.options()?;
                }
            }
            _ => {} // agent_thought_chunk, user_message_chunk, available_commands_update, ...
        }
        Ok(())
    }

    /// ACP `usage_update`: the cost is the session's running total, so the report carries its growth since the last one (like
    /// tokens, which are reported as they grow); `used` and `size` are the context's level and size (ADR-0058), passed on as they
    /// are. Nothing is sent when neither changed.
    fn usage(&mut self, u: &Value) -> io::Result<()> {
        let cost = u.get("cost").and_then(cost_micros);
        let grew = cost.as_ref().map(|(currency, micros)| match &self.cost {
            Some((c, last)) if c == currency => micros.saturating_sub(*last),
            _ => *micros,
        });
        let context = (
            u.get("used").and_then(Value::as_u64).unwrap_or(0),
            u.get("size").and_then(Value::as_u64).unwrap_or(0),
        );
        let context_moved = context != (0, 0) && context != self.context;
        if context != (0, 0) {
            self.context = context;
        }
        let currency = cost.as_ref().map(|(c, _)| c.clone());
        if let Some(c) = cost {
            self.cost = Some(c);
        }
        let grew = grew.filter(|g| *g > 0);
        if grew.is_none() && !context_moved {
            return Ok(());
        }
        let sid = self.session.clone().unwrap_or_default();
        let model = self.model.clone();
        let mut p = json!({"sessionId": sid, "model": model, "inputTokens": 0, "outputTokens": 0,
                           "contextTokens": self.context.0, "contextWindow": self.context.1});
        if let (Some(g), Some(c)) = (grew, currency) {
            p["costMicros"] = json!(g);
            p["currency"] = json!(c);
        }
        self.note("usage/report", p)
    }

    fn diffs_in(&mut self, u: &Value) -> io::Result<()> {
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
                json!({"diff": {"path": path, "unifiedDiff": replace_diff(path, old, new)}}),
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
                    self.update(u)?;
                }
            }
            (Some("session/request_permission"), Some(request)) => {
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
                let options = p
                    .get("options")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|o| {
                                Some((
                                    o.get("optionId")?.as_str()?.to_string(),
                                    o.get("kind")
                                        .and_then(Value::as_str)
                                        .unwrap_or("")
                                        .to_string(),
                                ))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                self.next += 1;
                let approval = format!("acp-{}", self.next);
                self.perms
                    .insert(approval.clone(), Permission { request, options });
                let sid = self.session.clone().unwrap_or_default();
                self.said = true;
                self.state("BLOCKED")?;
                self.note("approval/request", json!({"sessionId": sid, "approvalId": approval, "action": action, "detail": detail}))?;
            }
            // Files and terminals are the agent's own business in its worktree; the bridge offers neither.
            (Some(_), Some(id)) => {
                self.send_agent(
                    &json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "method not found"}}),
                );
            }
            (Some(_), None) => {}
            (None, Some(id)) => {
                let Some(n) = id.as_u64() else { return Ok(()) };
                match self.waiting.remove(&n) {
                    Some(Waiting::Initialize { node_id }) => {
                        if msg.get("error").is_some() {
                            let data = json!({"code": "adapter.agent_refused", "params": {"stage": "initialize"}});
                            self.send_node(&json!({"jsonrpc": "2.0", "id": node_id, "error": {"code": -32000, "message": "the agent refused to initialize", "data": data}}))?;
                        } else {
                            self.can_load = msg
                                .pointer("/result/agentCapabilities/loadSession")
                                .and_then(Value::as_bool)
                                .unwrap_or(false);
                            if !node_id.is_null() {
                                self.reply(
                                    &node_id,
                                    json!({"adapterName": "acp-bridge", "version": "1", "agentKind": "acp",
                                           "capabilities": ["chat", "approvals", "plan", "diff", "cancel", "usage", "set_mode", "set_model"]}),
                                )?;
                            }
                        }
                    }
                    Some(Waiting::NewSession { node_id }) => {
                        let sid = msg
                            .pointer("/result/sessionId")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                            .or_else(|| self.session.clone());
                        match sid {
                            Some(sid) if msg.get("error").is_none() => {
                                self.session = Some(sid.clone());
                                if let Some(m) = msg
                                    .pointer("/result/models/currentModelId")
                                    .and_then(Value::as_str)
                                {
                                    self.model = clip(m, MODEL_MAX);
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
                                    self.current_mode = m.to_string();
                                }
                                self.reply(&node_id, json!({"sessionId": sid}))?;
                                self.note(
                                    "session/reported",
                                    json!({"sessionId": sid, "nativeSessionId": sid}),
                                )?;
                                self.options()?;
                                self.state("IDLE")?;
                            }
                            _ => {
                                let data = json!({"code": "adapter.agent_refused", "params": {"stage": "session"}});
                                self.send_node(&json!({"jsonrpc": "2.0", "id": node_id, "error": {"code": -32000, "message": "the agent refused the session", "data": data}}))?;
                            }
                        }
                    }
                    Some(Waiting::Prompt) => {
                        self.flush_text()?;
                        self.perms.clear();
                        match turn_outcome(msg, self.said) {
                            Some((code, params)) => {
                                self.notice(code, &params)?;
                                self.log_reason("WARN", code, &params)?;
                                self.state_because("IDLE", code)?;
                            }
                            None => self.state("IDLE")?,
                        }
                    }
                    Some(Waiting::SetOption { node_id, mode, id }) => {
                        if msg.get("error").is_some() {
                            let data =
                                json!({"code": "adapter.option_refused", "params": {"id": id}});
                            self.send_node(&json!({"jsonrpc": "2.0", "id": node_id,
                                "error": {"code": -32000, "message": "the agent refused", "data": data}}))?;
                        } else {
                            if mode {
                                self.current_mode = id;
                            } else {
                                self.model = clip(&id, MODEL_MAX);
                            }
                            self.reply(&node_id, json!({}))?;
                            self.options()?;
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
    let mut child = Command::new(program)
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
    let kept = Arc::new(Mutex::new(Vec::new()));
    let stderr_done = keep_stderr(stderr, Arc::clone(&kept));
    let (tx, rx) = channel();
    lines(input, Ev::Node, Ev::NodeGone, tx.clone());
    lines(stdout, Ev::Agent, Ev::AgentGone, tx);
    let mut b = Bridge {
        out: output,
        agent: stdin,
        next: 0,
        waiting: HashMap::new(),
        session: None,
        perms: HashMap::new(),
        text: String::new(),
        text_since: Instant::now(),
        can_load: false,
        said: false,
        model: String::new(),
        cost: None,
        context: (0, 0),
        modes: Vec::new(),
        models: Vec::new(),
        current_mode: String::new(),
    };
    let result = (|| -> io::Result<()> {
        loop {
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(Ev::Node(m)) => {
                    if b.handle_node(&m)? {
                        return Ok(());
                    }
                }
                Ok(Ev::Agent(m)) => b.handle_agent(&m)?,
                Ok(Ev::NodeGone) => return Ok(()),
                Ok(Ev::AgentGone) => {
                    let code = wait_briefly(&mut child);
                    // The rest of its stderr, unless something it started still holds the stream open.
                    let _ = stderr_done.recv_timeout(Duration::from_millis(500));
                    let raw = kept.lock().unwrap_or_else(|e| e.into_inner()).clone();
                    let home = std::env::var("HOME")
                        .or_else(|_| std::env::var("USERPROFILE"))
                        .ok();
                    let tail = stderr_tail(&raw, home.as_deref());
                    b.agent_gone(code, &tail)?;
                    // An agent that died before the node asked anything: the node's `initialize` may still be on its way. It is
                    // answered with the reason too, instead of with the bridge's exit (which the node could only call `internal`).
                    if b.session.is_none() {
                        let end = Instant::now() + LATE_REQUEST_WAIT;
                        while let Some(left) = end.checked_duration_since(Instant::now()) {
                            match rx.recv_timeout(left) {
                                Ok(Ev::Node(m)) => {
                                    if b.answer_late(&m, code, &tail)? {
                                        break;
                                    }
                                }
                                Ok(_) => {}
                                Err(_) => break,
                            }
                        }
                    }
                    return Err(io::Error::other("the agent exited"));
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            }
            if !b.text.is_empty() && b.text_since.elapsed() >= JOIN {
                b.flush_text()?;
            }
        }
    })();
    let _ = child.kill();
    let _ = child.wait();
    result
}

#[cfg(test)]
mod tests {
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
    fn a_replacement_diff_shows_every_old_line_removed_and_new_line_added() {
        let d = replace_diff("a.rs", Some("x\ny\n"), "z\n");
        assert_eq!(d, "--- a/a.rs\n+++ b/a.rs\n@@ -1,2 +1,1 @@\n-x\n-y\n+z\n");
        let n = replace_diff("n.txt", None, "hi\n");
        assert!(
            n.starts_with("--- /dev/null\n+++ b/n.txt\n@@ -0,0 +1,1 @@\n+hi"),
            "{n}"
        );
    }

    /// A line that is not UTF-8 used to end the reader as if the agent had exited (audit F044).
    #[test]
    fn a_line_that_is_not_utf8_is_noise_not_the_end_of_the_agent() {
        let (tx, rx) = channel();
        let data = b"\xff\xfe noise\n{\"text\":\"\xff\"}\n{\"id\":1}\r\n{\"id\":2}".to_vec();
        lines(std::io::Cursor::new(data), Ev::Agent, Ev::AgentGone, tx);
        let mut ids = Vec::new();
        loop {
            match rx.recv_timeout(Duration::from_secs(10)) {
                Ok(Ev::Agent(v)) => ids.push(v["id"].clone()),
                Ok(Ev::AgentGone) => break,
                Ok(_) => {}
                Err(e) => panic!("reader stalled: {e}"),
            }
        }
        assert_eq!(ids, vec![json!(1), json!(2)]);
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
        assert!(skips_permissions("bypassPermissions", "Bypass Permissions"));
        assert!(skips_permissions("full", "Full Access"));
        assert!(skips_permissions("acceptEdits", ""));
        assert!(!skips_permissions("build", "Build"));
        assert!(!skips_permissions("plan", "Plan"));
        assert!(!skips_permissions("default", "Always Ask"));
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
    fn long_details_are_cut_on_a_character_boundary() {
        let s = short(&json!({"x": "é".repeat(400)}));
        assert!(s.ends_with('…') && s.len() <= MAX_DETAIL + 4);
    }
}
