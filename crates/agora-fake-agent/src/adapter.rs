//! A fake L3 adapter (docs/specs/adapter-protocol.md): speaks newline-delimited JSON-RPC 2.0 on stdio and behaves
//! according to the prompt text, so the node's adapter host can be tested end to end without any real agent.
//!
//! Prompts (the `text` of `session/prompt`):
//! - `ask`: goes `blocked`, sends an `approval/request`, waits for `approval/resolve`, then works and finishes.
//! - `crash`: exits abruptly with status 3 (a crashing adapter).
//! - `env`: appends a chat item saying whether the variable `AGORA_TEST_SECRET` is visible to the adapter.
//! - `mcp-servers`: appends a chat item with the `mcpServers` of `session/start`, as JSON (`[]` when there were none).
//! - `noise`: prints a line that is not JSON and a notification with an unknown method, then finishes normally.
//! - anything else: works, appends the text back as a chat item, reports usage, goes idle.
//!
//! JSON uses the proto3 JSON mapping (lowerCamelCase, enum names), as `docs/specs/adapter-protocol.md` says.

use std::io::{self, BufRead, Write};

use serde_json::{json, Value};

fn send<W: Write>(out: &mut W, v: &Value) -> io::Result<()> {
    writeln!(out, "{v}")?;
    out.flush()
}

fn note<W: Write>(out: &mut W, method: &str, params: Value) -> io::Result<()> {
    send(
        out,
        &json!({"jsonrpc": "2.0", "method": method, "params": params}),
    )
}

fn state<W: Write>(out: &mut W, sid: &str, s: &str) -> io::Result<()> {
    note(
        out,
        "state/update",
        json!({"sessionId": sid, "state": format!("AGENT_STATE_{s}")}),
    )
}

/// Runs until `shutdown` or end of input.
pub fn run<R: BufRead, W: Write>(input: R, out: &mut W) -> io::Result<()> {
    let mut session = String::new();
    let mut selected_cwd = Value::Null;
    let mut servers = Value::Array(Vec::new());
    let mut lines = input.lines();
    while let Some(line) = lines.next() {
        let line = line?;
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let (id, method, params) = (
            msg.get("id").cloned(),
            msg.get("method").and_then(Value::as_str).unwrap_or(""),
            msg.get("params").cloned().unwrap_or(Value::Null),
        );
        let reply = |out: &mut W, result: Value| -> io::Result<()> {
            match &id {
                Some(id) => send(out, &json!({"jsonrpc": "2.0", "id": id, "result": result})),
                None => Ok(()),
            }
        };
        match method {
            "initialize" => reply(
                out,
                json!({"adapterName": "fake-adapter", "version": "1", "agentKind": "fake",
                       "capabilities": ["chat", "approvals", "usage", "cancel", "resume"]}),
            )?,
            "session/start" => {
                selected_cwd = params
                    .get("worktree")
                    .or_else(|| params.get("cwd"))
                    .cloned()
                    .unwrap_or(Value::Null);
                servers = params
                    .get("mcpServers")
                    .cloned()
                    .unwrap_or_else(|| Value::Array(Vec::new()));
                let resumed = params.get("resumeSessionId").and_then(Value::as_str);
                session = resumed.map_or_else(|| "s-1".to_string(), str::to_string);
                reply(out, json!({"sessionId": session}))?;
                // A resumed session is the agent's own session again: it reports the same native id it was asked to resume.
                let native = resumed.map_or_else(|| format!("native-{session}"), str::to_string);
                note(
                    out,
                    "session/reported",
                    json!({"sessionId": session, "nativeSessionId": native}),
                )?;
                state(out, &session, "IDLE")?;
            }
            "session/prompt" => {
                reply(out, json!({}))?;
                let text = params.get("text").and_then(Value::as_str).unwrap_or("");
                match text {
                    "cwd" => note(
                        out,
                        "chat/append",
                        json!({"sessionId": session, "text": {"text": json!({"cwd": std::env::current_dir().ok(), "session_cwd": selected_cwd}).to_string()}}),
                    )?,
                    "crash" => std::process::exit(3),
                    "mcp-servers" => note(
                        out,
                        "chat/append",
                        json!({"sessionId": session, "text": {"text": servers.to_string()}}),
                    )?,
                    "env" => {
                        let seen = std::env::var_os("AGORA_TEST_SECRET").is_some();
                        note(
                            out,
                            "chat/append",
                            json!({"sessionId": session, "text": {"text": if seen { "secret:visible" } else { "secret:hidden" }}}),
                        )?;
                    }
                    "noise" => {
                        writeln!(out, "this is not json")?;
                        note(out, "future/method", json!({"x": 1}))?;
                        out.flush()?;
                        state(out, &session, "WORKING")?;
                        state(out, &session, "IDLE")?;
                    }
                    "ask" => {
                        state(out, &session, "BLOCKED")?;
                        note(
                            out,
                            "approval/request",
                            json!({"sessionId": session, "approvalId": "ap-1", "action": "run_command", "detail": "ls"}),
                        )?;
                        // Wait for the decision; everything else is answered normally meanwhile.
                        for next in lines.by_ref() {
                            let next = next?;
                            let Ok(m) = serde_json::from_str::<Value>(&next) else {
                                continue;
                            };
                            if m.get("method").and_then(Value::as_str) == Some("approval/resolve") {
                                if let Some(id) = m.get("id") {
                                    send(out, &json!({"jsonrpc": "2.0", "id": id, "result": {}}))?;
                                }
                                let approved = m["params"]["approved"].as_bool().unwrap_or(false);
                                state(out, &session, "WORKING")?;
                                note(
                                    out,
                                    "chat/append",
                                    json!({"sessionId": session, "text": {"text": if approved { "approved" } else { "denied" }}}),
                                )?;
                                state(out, &session, "IDLE")?;
                                break;
                            }
                        }
                    }
                    other => {
                        state(out, &session, "WORKING")?;
                        note(
                            out,
                            "chat/append",
                            json!({"sessionId": session, "text": {"text": other}}),
                        )?;
                        note(
                            out,
                            "usage/report",
                            json!({"sessionId": session, "model": "fake/model",
                                   "inputTokens": "10", "outputTokens": other.len().to_string()}),
                        )?;
                        state(out, &session, "IDLE")?;
                    }
                }
            }
            "session/cancel" => {
                reply(out, json!({}))?;
                state(out, &session, "IDLE")?;
            }
            "session/stop" | "mesh/deliver" | "approval/resolve" => reply(out, json!({}))?,
            "shutdown" => {
                reply(out, json!({}))?;
                return Ok(());
            }
            // Unknown requests get an error (a request has an id); unknown notifications are ignored.
            _ => {
                if let Some(id) = &id {
                    send(
                        out,
                        &json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "error"}}),
                    )?;
                }
            }
        }
    }
    Ok(())
}
