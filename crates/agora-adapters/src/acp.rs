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
//! - A prompt turn: `session/prompt` → state `working`, and the agent's answer (its stop reason) → state `idle`.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// How long agent text chunks are joined into one chat item.
const JOIN: Duration = Duration::from_millis(150);
/// The most text one joined item holds before it is sent anyway.
const JOIN_MAX: usize = 4096;
const MAX_DETAIL: usize = 300;

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
        let sid = self.session.clone().unwrap_or_default();
        // The adapter protocol carries `AgentState` as its proto JSON name; the node reads anything else as unknown.
        self.note(
            "state/update",
            json!({"sessionId": sid, "state": format!("AGENT_STATE_{s}")}),
        )
    }

    fn chat(&mut self, item: Value) -> io::Result<()> {
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

    fn ask_agent(&mut self, method: &str, params: Value, waiting: Waiting) -> io::Result<()> {
        self.next += 1;
        self.waiting.insert(self.next, waiting);
        write_line(
            &mut self.agent,
            &json!({"jsonrpc": "2.0", "id": self.next, "method": method, "params": params}),
        )
    }

    fn tell_agent(&mut self, method: &str, params: Value) -> io::Result<()> {
        write_line(
            &mut self.agent,
            &json!({"jsonrpc": "2.0", "method": method, "params": params}),
        )
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
                )?;
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
                        )?;
                    }
                    None => self.ask_agent(
                        "session/new",
                        json!({"cwd": cwd, "mcpServers": []}),
                        Waiting::NewSession { node_id },
                    )?,
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
                self.state("WORKING")?;
                self.ask_agent(
                    "session/prompt",
                    json!({"sessionId": sid, "prompt": [{"type": "text", "text": text}]}),
                    Waiting::Prompt,
                )?;
            }
            "session/cancel" => {
                done(self)?;
                if let Some(sid) = self.session.clone() {
                    self.tell_agent("session/cancel", json!({"sessionId": sid}))?;
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
                    write_line(
                        &mut self.agent,
                        &json!({"jsonrpc": "2.0", "id": p.request, "result": {"outcome": outcome}}),
                    )?;
                    if self.perms.is_empty() {
                        self.state("WORKING")?;
                    }
                }
            }
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
            _ => {} // agent_thought_chunk, user_message_chunk, available_commands_update, ...
        }
        Ok(())
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
                self.state("BLOCKED")?;
                self.note("approval/request", json!({"sessionId": sid, "approvalId": approval, "action": action, "detail": detail}))?;
            }
            // Files and terminals are the agent's own business in its worktree; the bridge offers neither.
            (Some(_), Some(id)) => {
                write_line(
                    &mut self.agent,
                    &json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "method not found"}}),
                )?;
            }
            (Some(_), None) => {}
            (None, Some(id)) => {
                let Some(n) = id.as_u64() else { return Ok(()) };
                match self.waiting.remove(&n) {
                    Some(Waiting::Initialize { node_id }) => {
                        if msg.get("error").is_some() {
                            self.send_node(&json!({"jsonrpc": "2.0", "id": node_id, "error": {"code": -32000, "message": "the agent refused to initialize"}}))?;
                        } else {
                            self.can_load = msg
                                .pointer("/result/agentCapabilities/loadSession")
                                .and_then(Value::as_bool)
                                .unwrap_or(false);
                            if !node_id.is_null() {
                                self.reply(
                                    &node_id,
                                    json!({"adapterName": "acp-bridge", "version": "1", "agentKind": "acp",
                                           "capabilities": ["chat", "approvals", "plan", "diff", "cancel"]}),
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
                                self.reply(&node_id, json!({"sessionId": sid}))?;
                                self.note("session/reported", json!({"sessionId": sid, "nativeSessionId": sid}))?;
                                self.state("IDLE")?;
                            }
                            _ => self.send_node(&json!({"jsonrpc": "2.0", "id": node_id, "error": {"code": -32000, "message": "the agent refused the session"}}))?,
                        }
                    }
                    Some(Waiting::Prompt) => {
                        self.flush_text()?;
                        self.perms.clear();
                        self.state("IDLE")?;
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
        .stderr(Stdio::inherit())
        .spawn()?;
    let (stdin, stdout) = match (child.stdin.take(), child.stdout.take()) {
        (Some(i), Some(o)) => (i, o),
        _ => {
            let _ = child.kill();
            return Err(io::Error::other("agent pipes missing"));
        }
    };
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
                Ok(Ev::AgentGone) => return Err(io::Error::other("the agent exited")),
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
    fn long_details_are_cut_on_a_character_boundary() {
        let s = short(&json!({"x": "é".repeat(400)}));
        assert!(s.ends_with('…') && s.len() <= MAX_DETAIL + 4);
    }
}
