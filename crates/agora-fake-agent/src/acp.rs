//! A fake ACP agent (L2): speaks the Agent Client Protocol over stdio so the bridge can be tested end to end without a real agent.
//!
//! Prompts (the text of `session/prompt`):
//! - `ask`: sends `session/request_permission`, waits for the client's answer, says `allowed` or `denied`, and finishes the turn.
//! - `plan`: sends a plan, a tool call carrying a diff, and a message in two chunks, then finishes.
//! - `crash`: exits at once.
//! - anything else: echoes the text as a message and finishes.

use std::io::{self, BufRead, Write};

use serde_json::{json, Value};

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
                reply(
                    out,
                    json!({"protocolVersion": 1, "agentCapabilities": {"loadSession": false}}),
                )?;
            }
            "session/new" => reply(out, json!({"sessionId": "acp-s1"}))?,
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
                match text.as_str() {
                    "crash" => std::process::exit(3),
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
                reply(out, json!({"stopReason": "end_turn"}))?;
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
