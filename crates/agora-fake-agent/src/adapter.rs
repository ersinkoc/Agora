//! A fake L3 adapter (docs/specs/adapter-protocol.md): speaks newline-delimited JSON-RPC 2.0 on stdio and behaves
//! according to the prompt text, so the node's adapter host can be tested end to end without any real agent.
//!
//! Prompts (the `text` of `session/prompt`):
//! - `ask`: goes `blocked`, sends an `approval/request`, waits for `approval/resolve`, then works and finishes.
//! - `first:TEXT`: says TEXT, then answers the prompt 300 ms later.
//! - `hang`: does not answer the prompt for 30 s (an adapter that is stuck); writes `hang.txt` in its folder once it has read the
//!   prompt, so a test knows the prompt reached it.
//! - `ask-error`: like `ask`, but answers the decision with a JSON-RPC error.
//! - `ask-hang`: like `ask`, but never answers the decision (an adapter that is stuck while the node waits for it to take an answer,
//!   ADR-0110).
//! - `ask-exited`: like `ask`, but answers the decision with an error whose `data` is `adapter.agent_exited` (stage `turn`, exit code
//!   `3`, a 2000-byte stderr tail and a parameter the code does not have), as an adapter whose agent ended does (ADR-0110).
//! - `ask-late`: like `ask`, but answers the decision 12 s late, after the node's wait for it ran out (ADR-0110 decision 6).
//! - `prompt-error`: answers the prompt with a JSON-RPC error that carries no stable reason (ADR-0110).
//! - `prompt-exited`: answers the prompt with the error `data` of `ask-exited` (ADR-0110).
//! - `ask-reserved`: like `ask`, but the request's action is `agent.start` (a name only the node may use); the answer is written to `applied.txt`.
//! - `ask:ACTION|DETAIL`: like `ask`, with that action and detail (#1147: words of the agent's own on an approval card).
//! - `later-ask`: answers the prompt, waits 1.5 s, then behaves like `ask` (a request that comes while the node may be away, ADR-0093),
//!   and writes `later.txt` in its folder once the request is out.
//! - `later-flood`: answers the prompt, waits 1.5 s, then says 3000 chat items of 1000 bytes each and `flood-end` (output that fills
//!   what a holder keeps while no node reads, ADR-0093), then writes `later.txt` in its folder.
//! - `later-tool`: answers the prompt, starts a tool call (`callId` `t1`, tool `read`, arguments `{"path":"a.rs"}`, `in_progress`), then
//!   waits until a file `go.txt` appears in its folder (a test makes it once the node it wants gone is gone), then finishes the call
//!   with only its status and output (`completed`, `done`), says `tool-done`, goes idle and writes `later.txt` (ADR-0104 C1).
//! - `hold:TEXT`: goes working, says TEXT, and stays working until a file `go.txt` appears in its folder; then goes idle and writes
//!   `later.txt` (review of #1103, round 3). `hold:TEXT|REST` says REST too once `go.txt` appears, before it goes idle (words that
//!   go on across a node's stop, ADR-0104 O2).
//! - `crash`: exits abruptly with status 3 (a crashing adapter).
//! - `mode:ID` (with `--modes`): switches itself to mode ID, as an agent may on its own (ACP `current_mode_update`), and reports its
//!   options again with ID as the current mode.
//! - `config:auto_approve=VALUE` (with `--modes`): sets its option `auto_approve` to VALUE on its own and reports its config options again.
//! - `env`: appends a chat item saying whether the variable `AGORA_TEST_SECRET` is visible to the adapter.
//! - `mcp-servers`: appends a chat item with the `mcpServers` of `session/start`, as JSON (`[]` when there were none).
//! - `noise`: prints a line that is not JSON and a notification with an unknown method, then finishes normally.
//! - anything else: works, appends the text back as a chat item, reports usage, goes idle.
//!
//! With `--turns` it declares `turns` (it reports its turns), so the node queues a prompt sent while one runs.
//! With `--modes` the adapter also declares `set_mode` and `config`, reports four modes (`default`, `plan`, `turbo` whose words do not say
//! whether it asks, and `bypassPermissions`) and a boolean option `auto_approve`, and says what it was told to apply by appending
//! `applied mode:ID` or `applied config:ID=VALUE` as a line to `applied.txt` in its folder (ADR-0090 Amendment A tests).
//! With `--mode-config` as well, it is an agent whose mode is a config option of category `mode` (what `agora acp-bridge` reports for
//! an ACP agent that gives its mode as a config option): it also reports an option `approvals` with the four modes as its choices,
//! and a mode switch is the switch of that option, recorded as `applied config:approvals=ID` (audit F1322).
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

/// The four modes `--modes` reports.
fn all_modes() -> Value {
    let mode = |id: &str, name: &str, skips: bool| json!({"id": id, "name": name, "description": "", "skipsPermissions": skips});
    json!([
        mode("default", "Ask first", false),
        mode("plan", "Plan", false),
        mode("turbo", "Turbo", false),
        mode("bypassPermissions", "Bypass Permissions", true),
    ])
}

/// The config options `--modes` reports: `auto_approve` set to `auto`, and with `--mode-config` the mode as the option `approvals`.
fn config_options(mode_config: bool, auto: &str) -> Value {
    let mut options = vec![json!({
        "id": "auto_approve", "name": "Auto approve", "description": "",
        "category": "other", "type": "boolean", "current": auto,
        "choices": [], "skipsPermissions": false})];
    if mode_config {
        options.push(json!({
            "id": "approvals", "name": "Approvals", "description": "",
            "category": "mode", "type": "select", "current": "default",
            "choices": all_modes(), "skipsPermissions": false}));
    }
    Value::Array(options)
}

/// ADR-0110: the error `data` of an adapter whose agent ended during a turn, with a stderr tail longer than the node passes on and a
/// parameter `adapter.agent_exited` does not have.
fn exited_reason() -> Value {
    json!({"code": "adapter.agent_exited",
           "params": {"stage": "turn", "exit_code": "3", "stderr_tail": "x".repeat(2000), "extra": "no"}})
}

/// Runs until `shutdown` or end of input.
pub fn run<R: BufRead, W: Write>(input: R, out: &mut W) -> io::Result<()> {
    let modes = std::env::args().any(|a| a == "--modes");
    let mode_config = modes && std::env::args().any(|a| a == "--mode-config");
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
            "initialize" => {
                let mut caps = vec!["chat", "approvals", "usage", "cancel", "resume"];
                // `--turns`: it reports its turns, so the node queues a prompt behind the turn that runs (ADR-0090).
                if std::env::args().any(|a| a == "--turns") {
                    caps.push("turns");
                }
                if modes {
                    caps.extend(["set_mode", "config"]);
                }
                reply(
                    out,
                    json!({"adapterName": "fake-adapter", "version": "1", "agentKind": "fake",
                           "capabilities": caps}),
                )?
            }
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
                if modes {
                    note(
                        out,
                        "session/options",
                        json!({"sessionId": session, "modes": all_modes(),
                               "models": [], "currentMode": "default", "currentModel": ""}),
                    )?;
                    note(
                        out,
                        "session/config",
                        json!({"sessionId": session, "options": config_options(mode_config, "false")}),
                    )?;
                }
            }
            "session/set_option" | "session/set_config" if modes => {
                reply(out, json!({}))?;
                let text = if method == "session/set_option" && mode_config {
                    // The mode is the `approvals` option: switching one is switching the other.
                    format!(
                        "applied config:approvals={}",
                        params["id"].as_str().unwrap_or("")
                    )
                } else if method == "session/set_option" {
                    format!("applied mode:{}", params["id"].as_str().unwrap_or(""))
                } else {
                    format!(
                        "applied config:{}={}",
                        params["configId"].as_str().unwrap_or(""),
                        params["value"].as_str().unwrap_or("")
                    )
                };
                // One line per switch in the agent's own folder, so a test reads what the agent was told.
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open("applied.txt")?;
                writeln!(file, "{text}")?;
            }
            "session/prompt" => {
                let text = params.get("text").and_then(Value::as_str).unwrap_or("");
                // `hang`: never answers the prompt (for 30 s), like an adapter that is stuck.
                // `first:TEXT`: says TEXT before it answers the prompt (and answers 300 ms later), so a client sees the agent's words
                // before the reply to its own request.
                if let Some(said) = text.strip_prefix("first:") {
                    note(
                        out,
                        "chat/append",
                        json!({"sessionId": session, "text": {"text": said}}),
                    )?;
                    std::thread::sleep(std::time::Duration::from_millis(300));
                }
                if text == "hang" {
                    // A marker in the agent's folder (best effort: a folder it cannot write leaves the prompt as it was).
                    let _ = std::fs::write("hang.txt", "read");
                    std::thread::sleep(std::time::Duration::from_secs(30));
                }
                // ADR-0110: the prompt is not taken, with or without one of the stable reasons the node passes on.
                if let (Some(id), "prompt-error" | "prompt-exited") = (&id, text) {
                    let error = if text == "prompt-exited" {
                        json!({"code": -32000, "message": "the agent exited", "data": exited_reason()})
                    } else {
                        json!({"code": -32000, "message": "no"})
                    };
                    send(out, &json!({"jsonrpc": "2.0", "id": id, "error": error}))?;
                    continue;
                }
                reply(out, json!({}))?;
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
                    "later-flood" => {
                        std::thread::sleep(std::time::Duration::from_millis(1500));
                        state(out, &session, "WORKING")?;
                        let filler = "x".repeat(1000);
                        for i in 0..3000 {
                            note(
                                out,
                                "chat/append",
                                json!({"sessionId": session, "text": {"text": format!("f{i}:{filler}")}}),
                            )?;
                        }
                        note(
                            out,
                            "chat/append",
                            json!({"sessionId": session, "text": {"text": "flood-end"}}),
                        )?;
                        state(out, &session, "IDLE")?;
                        // A marker in the agent's folder, so a test waits for it instead of guessing how long 1.5 s took.
                        std::fs::write("later.txt", "done")?;
                    }
                    held if held.starts_with("hold:") => {
                        // Review of #1103, round 3: words said in a turn that stays open until the test makes `go.txt`.
                        state(out, &session, "WORKING")?;
                        let (first, rest) = match held["hold:".len()..].split_once('|') {
                            Some((first, rest)) => (first, Some(rest)),
                            None => (&held["hold:".len()..], None),
                        };
                        note(
                            out,
                            "chat/append",
                            json!({"sessionId": session, "text": {"text": first}}),
                        )?;
                        let end = std::time::Instant::now() + std::time::Duration::from_secs(60);
                        while !std::path::Path::new("go.txt").exists()
                            && std::time::Instant::now() < end
                        {
                            std::thread::sleep(std::time::Duration::from_millis(25));
                        }
                        if let Some(rest) = rest {
                            note(
                                out,
                                "chat/append",
                                json!({"sessionId": session, "text": {"text": rest}}),
                            )?;
                        }
                        state(out, &session, "IDLE")?;
                        std::fs::write("later.txt", "done")?;
                    }
                    "later-tool" => {
                        state(out, &session, "WORKING")?;
                        note(
                            out,
                            "chat/append",
                            json!({"sessionId": session, "toolCall": {"callId": "t1", "tool": "read",
                                   "argumentsJson": "{\"path\":\"a.rs\"}", "kind": "read", "status": "in_progress"}}),
                        )?;
                        let end = std::time::Instant::now() + std::time::Duration::from_secs(60);
                        while !std::path::Path::new("go.txt").exists()
                            && std::time::Instant::now() < end
                        {
                            std::thread::sleep(std::time::Duration::from_millis(25));
                        }
                        note(
                            out,
                            "chat/append",
                            json!({"sessionId": session, "toolCall": {"callId": "t1", "status": "completed", "output": "done"}}),
                        )?;
                        note(
                            out,
                            "chat/append",
                            json!({"sessionId": session, "text": {"text": "tool-done"}}),
                        )?;
                        state(out, &session, "IDLE")?;
                        std::fs::write("later.txt", "done")?;
                    }
                    asked
                        if matches!(
                            asked,
                            "ask"
                                | "ask-error"
                                | "ask-hang"
                                | "ask-exited"
                                | "ask-late"
                                | "ask-reserved"
                                | "later-ask"
                        ) || asked.starts_with("ask:") =>
                    {
                        if text == "later-ask" {
                            std::thread::sleep(std::time::Duration::from_millis(1500));
                        }
                        state(out, &session, "BLOCKED")?;
                        let (action, detail) = match asked.strip_prefix("ask:") {
                            Some(rest) => rest.split_once('|').unwrap_or((rest, "")),
                            None if text == "ask-reserved" => ("agent.start", "ls"),
                            None => ("run_command", "ls"),
                        };
                        note(
                            out,
                            "approval/request",
                            json!({"sessionId": session, "approvalId": "ap-1",
                                   "action": action, "detail": detail}),
                        )?;
                        if text == "later-ask" {
                            std::fs::write("later.txt", "asked")?;
                        }
                        // Wait for the decision; everything else is answered normally meanwhile.
                        for next in lines.by_ref() {
                            let next = next?;
                            let Ok(m) = serde_json::from_str::<Value>(&next) else {
                                continue;
                            };
                            if m.get("method").and_then(Value::as_str) == Some("approval/resolve") {
                                if let Some(id) = m.get("id") {
                                    if text == "ask-error" {
                                        // The adapter cannot take the decision (it answers with an error).
                                        send(
                                            out,
                                            &json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": "no"}}),
                                        )?;
                                        break;
                                    }
                                    if text == "ask-exited" {
                                        send(
                                            out,
                                            &json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000,
                                                    "message": "the agent exited", "data": exited_reason()}}),
                                        )?;
                                        break;
                                    }
                                    if text == "ask-hang" {
                                        // The decision is never answered; everything after it still is.
                                        break;
                                    }
                                    if text == "ask-late" {
                                        std::thread::sleep(std::time::Duration::from_secs(12));
                                    }
                                    send(out, &json!({"jsonrpc": "2.0", "id": id, "result": {}}))?;
                                }
                                let approved = m["params"]["approved"].as_bool().unwrap_or(false);
                                if text == "ask-reserved" {
                                    // What the node answered the request, for a test to read.
                                    let mut file = std::fs::OpenOptions::new()
                                        .create(true)
                                        .append(true)
                                        .open("applied.txt")?;
                                    writeln!(
                                        file,
                                        "answered:{}",
                                        if approved { "approved" } else { "denied" }
                                    )?;
                                }
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
                    switch if modes && switch.starts_with("config:auto_approve=") => note(
                        out,
                        "session/config",
                        json!({"sessionId": session,
                               "options": config_options(mode_config, &switch["config:auto_approve=".len()..])}),
                    )?,
                    switch if modes && switch.starts_with("mode:") => note(
                        out,
                        "session/options",
                        json!({"sessionId": session, "modes": all_modes(), "models": [],
                               "currentMode": &switch["mode:".len()..], "currentModel": ""}),
                    )?,
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
