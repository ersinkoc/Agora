//! A fake ACP agent (L2): speaks the Agent Client Protocol over stdio so the bridge can be tested end to end without a real agent.
//!
//! Prompts (the text of `session/prompt`):
//! - `ask`: sends `session/request_permission`, waits for the client's answer, says `allowed` or `denied`, and finishes the turn.
//! - `plan`: sends a plan, a tool call carrying a diff, and a message in two chunks, then finishes.
//! - `crash`: exits at once.
//! - `crash-loud`: writes a fatal error (with a fake token in it) to stderr and exits with code 4, mid-turn.
//! - `silent`: finishes the turn (`end_turn`) without saying anything, like an agent whose provider failed.
//! - `silent-loud`: the same, after writing the reason (an auth error, a home path and a key) to stderr.
//! - `refuse`: finishes the turn with the stop reason `refusal`.
//! - `fail`: answers the prompt with a JSON-RPC error.
//! - `usage`: reports `usage_update` twice (a running cost of 0.0123 USD, then 0.02 USD), says `counted` and finishes.
//! - `mcp-servers`: says the `mcpServers` its session was started with, as JSON.
//! - `mcp TOOL JSON`: starts the first of those servers (`command args`, MCP over its stdio, as a real agent does), calls `TOOL` with
//!   the arguments `JSON` and says the tool's result text (`error: ` before it when the result is an error), or `waiting` when no
//!   answer came within 3 s; then that server is kept running (its call stays open) until the agent exits.
//! - anything else: echoes the text as a message and finishes.
//!
//! - `auto`: switches itself to the `plan` mode (`current_mode_update`), says `switched` and finishes.
//!
//! `session/new` answers with a model list and a mode list (`build`, `plan`) like real ACP agents; with
//! `AGORA_FAKE_ACP_BYPASS_MODE=1` the list also has `bypassPermissions`. `session/set_mode` and `session/set_model` take an id from
//! those lists and refuse anything else (and `fake/model-2` with `AGORA_FAKE_ACP_REFUSE_MODEL=1`); with
//! `AGORA_FAKE_ACP_REFUSE_SESSION=1` `session/new` itself is refused; with `AGORA_FAKE_ACP_NO_STDIO_MCP=1` `initialize` says
//! `mcpCapabilities.stdio: false`. [`fail_at_start`] is an agent
//! that dies before `initialize` (`agora-fake-agent --acp --fail-at-start`).

/// What an agent killed by a broken plugin writes before it exits: an error, a path in the home folder and a token that must not
/// reach a report. Returns the exit code to use (5).
pub fn fail_at_start<W: Write>(err: &mut W) -> i32 {
    let home = std::env::var("HOME").unwrap_or_default();
    let _ = writeln!(err, "loading plugin {home}/.config/fake/plugin.js");
    let _ = writeln!(
        err,
        "\u{1b}[31merror\u{1b}[0m: plugin failed: api_key=sk-fake0123456789abcdefghij"
    );
    let _ = writeln!(err, "TypeError: undefined is not a function");
    let _ = err.flush();
    5
}

use std::io::{self, BufRead, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// A started MCP server: the process and the lines it writes.
struct McpChild {
    child: Child,
    lines: Receiver<String>,
}

impl McpChild {
    fn start(server: &Value) -> io::Result<Self> {
        let command = server
            .get("command")
            .and_then(Value::as_str)
            .ok_or_else(|| io::Error::other("no command"))?;
        let args: Vec<&str> = server
            .get("args")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let mut child = Command::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("no stdout"))?;
        let (tx, lines) = channel();
        std::thread::spawn(move || {
            for line in io::BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Self { child, lines })
    }

    fn send(&mut self, id: u64, method: &str, params: Value) -> io::Result<()> {
        let stdin = self
            .child
            .stdin
            .as_mut()
            .ok_or_else(|| io::Error::other("no stdin"))?;
        writeln!(
            stdin,
            "{}",
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
        )?;
        stdin.flush()
    }

    /// The answer to request `id`, if it comes within `wait`.
    fn answer(&self, id: u64, wait: Duration) -> Option<Value> {
        let end = Instant::now() + wait;
        while let Some(left) = end.checked_duration_since(Instant::now()) {
            let line = self.lines.recv_timeout(left).ok()?;
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                if v.get("id") == Some(&json!(id)) {
                    return Some(v);
                }
            }
        }
        None
    }
}

impl Drop for McpChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// `mcp TOOL JSON` (see the module docs): what the agent says about the call.
fn mcp_call(servers: &Value, text: &str, open: &mut Vec<McpChild>) -> String {
    let mut parts = text.splitn(3, ' ').skip(1);
    let (Some(tool), args) = (parts.next(), parts.next().unwrap_or("{}")) else {
        return "error: no tool".into();
    };
    let Ok(args) = serde_json::from_str::<Value>(args) else {
        return "error: arguments are not JSON".into();
    };
    let Some(server) = servers.get(0) else {
        return "error: no mcp server".into();
    };
    let mut child = match McpChild::start(server) {
        Ok(c) => c,
        Err(e) => return format!("error: {e}"),
    };
    let wait = Duration::from_secs(3);
    if child.send(1, "initialize", json!({})).is_err() || child.answer(1, wait).is_none() {
        return "error: no initialize answer".into();
    }
    if child
        .send(2, "tools/call", json!({"name": tool, "arguments": args}))
        .is_err()
    {
        return "error: call not sent".into();
    }
    match child.answer(2, wait) {
        Some(r) => {
            let text = r
                .pointer("/result/content/0/text")
                .and_then(Value::as_str)
                .map_or_else(|| r.to_string(), str::to_string);
            if r.pointer("/result/isError").and_then(Value::as_bool) == Some(true) {
                format!("error: {text}")
            } else {
                text
            }
        }
        None => {
            open.push(child);
            "waiting".into()
        }
    }
}

fn send<W: Write>(out: &mut W, v: &Value) -> io::Result<()> {
    writeln!(out, "{v}")?;
    out.flush()
}

fn update<W: Write>(out: &mut W, session: &str, u: Value) -> io::Result<()> {
    send(
        out,
        &json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": session, "update": u}}),
    )
}

pub fn run<R: BufRead, W: Write>(input: R, out: &mut W) -> io::Result<()> {
    let mut lines = input.lines();
    // The `mcpServers` of the session, and the servers whose call is still open.
    let mut servers = Value::Array(Vec::new());
    let mut open: Vec<McpChild> = Vec::new();
    while let Some(line) = lines.next() {
        let Ok(msg) = serde_json::from_str::<Value>(&line?) else {
            continue;
        };
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let reply = |out: &mut W, result: Value| -> io::Result<()> {
            match &id {
                Some(id) => send(out, &json!({"jsonrpc": "2.0", "id": id, "result": result})),
                None => Ok(()),
            }
        };
        match method {
            "initialize" => {
                // A slow agent: `AGORA_FAKE_ACP_INIT_DELAY_MS` makes initialization take that long.
                if let Some(ms) = std::env::var("AGORA_FAKE_ACP_INIT_DELAY_MS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                {
                    std::thread::sleep(std::time::Duration::from_millis(ms));
                }
                let mut caps = json!({"loadSession": false});
                if std::env::var_os("AGORA_FAKE_ACP_NO_STDIO_MCP").is_some() {
                    caps["mcpCapabilities"] = json!({"stdio": false});
                }
                reply(
                    out,
                    json!({"protocolVersion": 1, "agentCapabilities": caps}),
                )?;
            }
            "session/new" if std::env::var_os("AGORA_FAKE_ACP_REFUSE_SESSION").is_some() => {
                if let Some(id) = &id {
                    send(
                        out,
                        &json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32603, "message": "ServiceUnavailableError: service not running at /Users/someone/.local/share/opencode"}}),
                    )?;
                }
            }
            "session/new" => {
                servers = params.get("mcpServers").cloned().unwrap_or(Value::Null);
                let mut modes = vec![
                    json!({"id": "build", "name": "Build"}),
                    json!({"id": "plan", "name": "Plan", "description": "No edits"}),
                ];
                if std::env::var_os("AGORA_FAKE_ACP_BYPASS_MODE").is_some() {
                    modes.push(json!({"id": "bypassPermissions", "name": "Bypass Permissions"}));
                }
                reply(
                    out,
                    json!({"sessionId": "acp-s1",
                           "models": {"currentModelId": "fake/model-1", "availableModels": [
                               {"modelId": "fake/model-1", "name": "Fake 1"}, {"modelId": "fake/model-2", "name": "Fake 2"}]},
                           "modes": {"currentModeId": "build", "availableModes": modes}}),
                )?;
            }
            "session/set_mode" | "session/set_model" => {
                let (key, known): (&str, &[&str]) = if method == "session/set_mode" {
                    ("modeId", &["build", "plan", "bypassPermissions"])
                } else {
                    ("modelId", &["fake/model-1", "fake/model-2"])
                };
                let want = params.get(key).and_then(Value::as_str).unwrap_or("");
                let refused = want == "fake/model-2"
                    && std::env::var_os("AGORA_FAKE_ACP_REFUSE_MODEL").is_some();
                if known.contains(&want) && !refused {
                    reply(out, json!({}))?;
                } else if let Some(id) = &id {
                    send(
                        out,
                        &json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32602, "message": "unknown id"}}),
                    )?;
                }
            }
            "session/prompt" => {
                let session = params
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .unwrap_or("acp-s1")
                    .to_string();
                let text = params
                    .pointer("/prompt/0/text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let mut stop = "end_turn";
                match text.as_str() {
                    "crash" => std::process::exit(3),
                    "crash-loud" => {
                        eprintln!(
                            "fatal: free usage limit reached (token: fake0123456789abcdefghijklmn)"
                        );
                        std::process::exit(4)
                    }
                    "silent" => {}
                    "silent-loud" => {
                        // Like opencode with `--print-logs`: the reason is only on stderr.
                        let home = std::env::var("HOME").unwrap_or_default();
                        eprintln!("ERROR service=llm provider rejected: invalid x-api-key (auth) at {home}/.local/share/fake/auth.json");
                        eprintln!(
                            "ERROR at /Users/someone/.config/fake key=sk-fake0123456789abcdefghij"
                        );
                    }
                    "auto" => {
                        update(
                            out,
                            &session,
                            json!({"sessionUpdate": "current_mode_update", "currentModeId": "plan"}),
                        )?;
                        update(
                            out,
                            &session,
                            json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "switched"}}),
                        )?;
                    }
                    "refuse" => stop = "refusal",
                    "mcp-servers" => update(
                        out,
                        &session,
                        json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": servers.to_string()}}),
                    )?,
                    t if t.starts_with("mcp ") => {
                        let said = mcp_call(&servers, t, &mut open);
                        update(
                            out,
                            &session,
                            json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": said}}),
                        )?;
                    }
                    "fail" => {
                        if let Some(id) = &id {
                            send(
                                out,
                                &json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32603, "message": "provider error"}}),
                            )?;
                        }
                        continue;
                    }
                    "usage" => {
                        for amount in [0.0123, 0.02] {
                            update(
                                out,
                                &session,
                                json!({"sessionUpdate": "usage_update", "used": 1200, "size": 1_000_000,
                                       "cost": {"amount": amount, "currency": "USD"}}),
                            )?;
                        }
                        update(
                            out,
                            &session,
                            json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "counted"}}),
                        )?;
                    }
                    "ask" => {
                        send(
                            out,
                            &json!({"jsonrpc": "2.0", "id": 900, "method": "session/request_permission", "params": {
                            "sessionId": session,
                            "toolCall": {"toolCallId": "t1", "title": "run_command", "kind": "execute", "rawInput": {"command": "ls"}},
                            "options": [
                                {"optionId": "no", "name": "Reject", "kind": "reject_once"},
                                {"optionId": "yes", "name": "Allow", "kind": "allow_once"}
                            ]}}),
                        )?;
                        let mut verdict = "denied";
                        for l in lines.by_ref() {
                            let Ok(m) = serde_json::from_str::<Value>(&l?) else {
                                continue;
                            };
                            if m.get("id") == Some(&json!(900)) {
                                if m.pointer("/result/outcome/optionId")
                                    .and_then(Value::as_str)
                                    == Some("yes")
                                {
                                    verdict = "allowed";
                                }
                                break;
                            }
                        }
                        update(
                            out,
                            &session,
                            json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": verdict}}),
                        )?;
                    }
                    "plan" => {
                        update(
                            out,
                            &session,
                            json!({"sessionUpdate": "plan", "entries": [
                            {"content": "read the code", "priority": "high", "status": "pending"},
                            {"content": "fix the bug", "priority": "high", "status": "pending"}]}),
                        )?;
                        update(
                            out,
                            &session,
                            json!({"sessionUpdate": "tool_call", "toolCallId": "t2", "title": "edit a.rs", "kind": "edit", "status": "completed",
                            "rawInput": {"path": "a.rs"},
                            "content": [{"type": "diff", "path": "a.rs", "oldText": "old\n", "newText": "new\n"}]}),
                        )?;
                        update(
                            out,
                            &session,
                            json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "hello "}}),
                        )?;
                        update(
                            out,
                            &session,
                            json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "world"}}),
                        )?;
                    }
                    other => update(
                        out,
                        &session,
                        json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": other}}),
                    )?,
                }
                reply(out, json!({"stopReason": stop}))?;
            }
            _ => {
                // An unknown request is answered with an error; a notification (`session/cancel`) is ignored.
                if let Some(id) = &id {
                    send(
                        out,
                        &json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "method not found"}}),
                    )?;
                }
            }
        }
    }
    Ok(())
}
