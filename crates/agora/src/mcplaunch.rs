//! ADR-0122 Amendment A: `agora mcp-launch --socket <agent's mesh socket> <item> <launch id>`, the node's launcher for a shared MCP
//! server that names a secret. The agent's tool starts it from the MCP file the node wrote (which holds no secret); it asks the node,
//! over the agent's own mesh socket, for the item's program with its secrets filled, then runs that program with the agent's tool's
//! standard input and output, and ends with its exit code. Nothing it learns is written anywhere.
//!
//! ADR-0133: the launch id lasts the agent's run, with one server at a time. The launcher keeps its connection to the node open while
//! the server runs, says the server's process id and, at the end, its exit code; it closes the server when the node says so (the item
//! was removed or changed, the agent ended). The server is bound to the launcher (`agora_platform::spawn_bound`), so ending the launcher
//! ends it where the platform can. Asked while the previous server is still being let go, it waits a few seconds and asks again.
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::{key, Opts, Say};

/// The longest answer the node may give (a program, its arguments and environment).
const MAX_ANSWER: u64 = 1 << 20;

/// What the node answered: the program to run, or the stable reason it refused.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    Run {
        command: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
    },
    Refused(String),
}

/// Reads the node's answer line.
pub(crate) fn read_answer(line: &str) -> Answer {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return Answer::Refused("unreadable".into());
    };
    if let Some(r) = v.get("result") {
        let command = r
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let args = r
            .get("args")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let env = r
            .get("env")
            .and_then(Value::as_object)
            .map(|o| {
                o.iter()
                    .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        if command.is_empty() {
            return Answer::Refused("unreadable".into());
        }
        return Answer::Run { command, args, env };
    }
    let reason = v
        .pointer("/error/data/reason")
        .and_then(Value::as_str)
        .unwrap_or("refused");
    Answer::Refused(
        reason
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
            .take(40)
            .collect(),
    )
}

/// How long the launcher keeps asking while the node still holds the previous server (`instance_live`, `too_soon`).
const WAIT_FOR_ROOM: Duration = Duration::from_secs(8);

/// Asks the node over `socket` for `item`'s program; the connection stays open for the server's life.
fn ask(
    socket: &Path,
    item: &str,
    launch_id: &str,
) -> std::io::Result<(Answer, agora_platform::LocalStream)> {
    let mut conn = agora_platform::local_connect(socket)?;
    let request = json!({"jsonrpc": "2.0", "id": 1, "method": "agora/shared_launch", "params": {"item": item, "launch_id": launch_id}});
    conn.write_all(format!("{request}\n").as_bytes())?;
    let mut line = String::new();
    BufReader::new(conn.try_clone()?.take(MAX_ANSWER)).read_line(&mut line)?;
    Ok((read_answer(&line), conn))
}

/// Asks, and asks again for a few seconds while the node is still letting the previous server go.
fn ask_with_room(
    socket: &Path,
    item: &str,
    launch_id: &str,
) -> std::io::Result<(Answer, agora_platform::LocalStream)> {
    let until = Instant::now() + WAIT_FOR_ROOM;
    loop {
        let (answer, conn) = ask(socket, item, launch_id)?;
        match &answer {
            Answer::Refused(r)
                if (r == "instance_live" || r == "too_soon") && Instant::now() < until =>
            {
                drop(conn);
                std::thread::sleep(Duration::from_millis(500));
            }
            _ => return Ok((answer, conn)),
        }
    }
}

/// One notice to the node on the launcher's connection (best effort: the server runs whether or not the node hears it).
fn notice(conn: &mut agora_platform::LocalStream, method: &str, params: Value) {
    let line = json!({"jsonrpc": "2.0", "method": method, "params": params});
    let _ = conn.write_all(format!("{line}\n").as_bytes());
}

/// Whether a line from the node tells this launcher to close its server.
pub(crate) fn is_close(line: &str) -> bool {
    serde_json::from_str::<Value>(line)
        .ok()
        .and_then(|v| v.get("method").and_then(Value::as_str).map(str::to_string))
        .is_some_and(|m| m == "agora/shared_launch_close")
}

pub(crate) fn mcp_launch(say: &Say, o: &Opts) -> ExitCode {
    let (Some(socket), [item, launch_id]) = (&o.socket, o.positional.as_slice()) else {
        say.err(key!("cli-bad-arguments"), &[]);
        return ExitCode::from(2);
    };
    let (answer, mut conn) = match ask_with_room(socket, item, launch_id) {
        Ok(a) => a,
        Err(e) => {
            say.err(
                key!("cli-mcp-launch-refused"),
                &[
                    ("item", item.as_str().into()),
                    ("reason", e.kind().to_string().into()),
                ],
            );
            return ExitCode::from(1);
        }
    };
    let (command, args, env) = match answer {
        Answer::Run { command, args, env } => (command, args, env),
        // A launch id lives as long as the agent's run: one this node does not know (the agent's run ended, or the node restarted since
        // and could not take it up) needs the agent restarted.
        Answer::Refused(reason) if reason == "unknown_launch" => {
            say.err(
                key!("cli-mcp-launch-spent"),
                &[("item", item.as_str().into())],
            );
            return ExitCode::from(1);
        }
        Answer::Refused(reason) => {
            say.err(
                key!("cli-mcp-launch-refused"),
                &[("item", item.as_str().into()), ("reason", reason.into())],
            );
            return ExitCode::from(1);
        }
    };
    // The server talks to the agent's tool on this process's standard input and output; the secrets live only in its environment.
    let mut cmd = Command::new(&command);
    cmd.args(&args)
        .envs(env)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let started = agora_platform::spawn_bound(&mut cmd).map(|b| b.child);
    match started {
        Ok(mut child) => {
            notice(
                &mut conn,
                "agora/shared_launch_started",
                json!({"item": item, "launch_id": launch_id, "pid": child.id()}),
            );
            // The node's word to close the server, read on a thread of its own; the server's end is waited for here.
            let close = Arc::new(AtomicBool::new(false));
            if let Ok(read) = conn.try_clone() {
                let close = Arc::clone(&close);
                let _ = std::thread::Builder::new()
                    .name("mcp-launch-close".into())
                    .spawn(move || {
                        for line in BufReader::new(read.take(MAX_ANSWER)).lines() {
                            match line {
                                Ok(l) if is_close(&l) => {
                                    close.store(true, Ordering::Release);
                                    return;
                                }
                                Ok(_) => {}
                                Err(_) => return,
                            }
                        }
                    });
            }
            let mut killed = false;
            let code = loop {
                match child.try_wait() {
                    Ok(Some(s)) => break s.code().unwrap_or(1),
                    Ok(None) => {}
                    Err(_) => break 1,
                }
                if close.load(Ordering::Acquire) && !killed {
                    killed = true;
                    let _ = child.kill();
                }
                std::thread::sleep(Duration::from_millis(100));
            };
            notice(
                &mut conn,
                "agora/shared_launch_ended",
                json!({"item": item, "launch_id": launch_id, "code": code}),
            );
            ExitCode::from(u8::try_from(code.clamp(0, 255)).unwrap_or(1))
        }
        Err(e) => {
            say.err(
                key!("cli-mcp-launch-refused"),
                &[
                    ("item", item.as_str().into()),
                    ("reason", e.kind().to_string().into()),
                ],
            );
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn the_nodes_answer_is_a_program_to_run_or_a_reason() {
        let run = read_answer(
            r#"{"jsonrpc":"2.0","id":1,"result":{"command":"npx","args":["-y","docs"],"env":{"TOKEN":"s3"}}}"#,
        );
        assert_eq!(
            run,
            Answer::Run {
                command: "npx".into(),
                args: vec!["-y".into(), "docs".into()],
                env: vec![("TOKEN".into(), "s3".into())]
            }
        );
        let refused = read_answer(
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32001,"data":{"code":"share.refused","reason":"unknown_launch"}}}"#,
        );
        assert_eq!(refused, Answer::Refused("unknown_launch".into()));
        assert_eq!(
            read_answer("not json"),
            Answer::Refused("unreadable".into())
        );
        assert_eq!(
            read_answer(r#"{"result":{"command":""}}"#),
            Answer::Refused("unreadable".into())
        );
        assert!(is_close(
            r#"{"jsonrpc":"2.0","method":"agora/shared_launch_close","params":{"item":"db"}}"#
        ));
        assert!(!is_close(
            r#"{"jsonrpc":"2.0","method":"agora/shared_launch_started"}"#
        ));
        assert!(!is_close("not json"));
        // A reason is a code: anything else in it is dropped.
        assert_eq!(
            read_answer(r#"{"error":{"data":{"reason":"a b\u001b[2J"}}}"#),
            Answer::Refused("ab2J".into())
        );
    }
}
