//! ADR-0122 Amendment A: `agora mcp-launch --socket <agent's mesh socket> <item> <launch id>`, the node's launcher for a shared MCP
//! server that names a secret. The agent's tool starts it from the MCP file the node wrote (which holds no secret); it asks the node,
//! over the agent's own mesh socket, for the item's program with its secrets filled (the launch id is single use), then runs that
//! program with the agent's tool's standard input and output, and ends with its exit code. Nothing it learns is written anywhere.
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

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

/// Asks the node over `socket` for `item`'s program.
fn ask(socket: &Path, item: &str, launch_id: &str) -> std::io::Result<Answer> {
    let mut conn = agora_platform::local_connect(socket)?;
    let request = json!({"jsonrpc": "2.0", "id": 1, "method": "agora/shared_launch", "params": {"item": item, "launch_id": launch_id}});
    conn.write_all(format!("{request}\n").as_bytes())?;
    let mut line = String::new();
    BufReader::new(conn.take(MAX_ANSWER)).read_line(&mut line)?;
    Ok(read_answer(&line))
}

pub(crate) fn mcp_launch(say: &Say, o: &Opts) -> ExitCode {
    let (Some(socket), [item, launch_id]) = (&o.socket, o.positional.as_slice()) else {
        say.err(key!("cli-bad-arguments"), &[]);
        return ExitCode::from(2);
    };
    let answer = match ask(socket, item, launch_id) {
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
        // A launch id is single use: a server the agent's tool starts again (after a crash, a reconnect) needs the agent restarted.
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
    let status = Command::new(&command)
        .args(&args)
        .envs(env)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status();
    match status {
        Ok(s) => ExitCode::from(u8::try_from(s.code().unwrap_or(1).clamp(0, 255)).unwrap_or(1)),
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
        // A reason is a code: anything else in it is dropped.
        assert_eq!(
            read_answer(r#"{"error":{"data":{"reason":"a b\u001b[2J"}}}"#),
            Answer::Refused("ab2J".into())
        );
    }
}
