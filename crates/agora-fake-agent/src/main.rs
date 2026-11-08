//! `agora-fake-agent --script x.toml`: plays a scripted screen sequence to the terminal.

use std::io;
use std::process::ExitCode;

use agora_fake_agent::{play, Script};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut path = None;
    if std::env::args().nth(1).as_deref() == Some("--adapter") {
        // L3 adapter mode: JSON-RPC over stdio (docs/specs/adapter-protocol.md).
        return match agora_fake_agent::adapter::run(
            std::io::stdin().lock(),
            &mut io::stdout().lock(),
        ) {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => ExitCode::from(1),
        };
    }
    if std::env::args().nth(1).as_deref() == Some("--acp") {
        // L2 mode: the Agent Client Protocol over stdio.
        return match agora_fake_agent::acp::run(std::io::stdin().lock(), &mut io::stdout().lock()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => ExitCode::from(1),
        };
    }
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--script" => path = args.next(),
            _ => {
                eprintln!("usage: agora-fake-agent --script <file.toml>");
                return ExitCode::from(2);
            }
        }
    }
    let Some(path) = path else {
        eprintln!("usage: agora-fake-agent --script <file.toml>");
        return ExitCode::from(2);
    };
    let script = match std::fs::read_to_string(&path)
        .map_err(|e| e.to_string())
        .and_then(|s| Script::parse(&s).map_err(|e| e.to_string()))
    {
        Ok(s) => s,
        Err(e) => {
            eprintln!("agora-fake-agent: {path}: {e}");
            return ExitCode::from(1);
        }
    };
    match play(&script, &mut io::stdout().lock(), std::thread::sleep) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("agora-fake-agent: {e}");
            ExitCode::from(1)
        }
    }
}
