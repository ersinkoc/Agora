//! The single Agora binary: `agora hub`, `agora node`, `agora tui`, and CLI subcommands.
//!
//! This build implements the node-local commands: `agora node run`, `agora agent start|stop|list`, and
//! `agora terminal attach`. Each CLI command is one protocol command sent over the node's control socket
//! (`agora_node::control`). All sentences come from `locales/` through `agora_i18n::Catalog`; the process
//! prints only ids, paths and codes of its own.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use agora_i18n::{key, Arg, Catalog};
use agora_node::control::{Client, KIND_TERM_DATA, KIND_TERM_EXIT, KIND_TERM_SNAPSHOT};
use agora_node::daemon::{Node, NodeConfig};
use agora_node::supervisor::NodeLimits;
use agora_protocol::v1::{
    self, command_ok::Result as Ok_, command_request::Command as Cmd, param_value::Value,
};

/// Ctrl-] detaches from an attached terminal (like telnet), since everything else is the agent's input.
const DETACH: u8 = 0x1d;
const TERMINAL_STREAM: u32 = 3;

struct Say(Catalog);

impl Say {
    fn text(&self, id: &str, args: &[(&str, Arg)]) -> String {
        self.0.render(id, args).unwrap_or_else(|| id.to_string())
    }

    fn out(&self, id: &str, args: &[(&str, Arg)]) {
        println!("{}", self.text(id, args));
    }

    fn err(&self, id: &str, args: &[(&str, Arg)]) {
        eprintln!("{}", self.text(id, args));
    }

    fn remote_error(&self, e: &v1::ErrorInfo) {
        let args: Vec<(&str, Arg)> = e
            .params
            .iter()
            .filter_map(|(k, v)| match &v.value {
                Some(Value::Str(s)) => Some((k.as_str(), Arg::Str(s.clone()))),
                Some(Value::Int(i)) => Some((k.as_str(), Arg::Num(*i))),
                None => None,
            })
            .collect();
        let message = self.text(&e.code.replace('.', "-"), &args);
        self.err(
            key!("cli-error"),
            &[
                ("code", e.code.as_str().into()),
                ("message", message.into()),
            ],
        );
    }
}

fn home() -> PathBuf {
    if let Some(h) = std::env::var_os("AGORA_HOME") {
        return PathBuf::from(h);
    }
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map_or_else(
            || PathBuf::from(".agora"),
            |h| PathBuf::from(h).join(".agora"),
        )
}

/// Options shared by the subcommands, plus leftover positional arguments.
#[derive(Default)]
struct Opts {
    socket: Option<PathBuf>,
    manifests: Option<PathBuf>,
    data: Option<PathBuf>,
    worktrees: Option<PathBuf>,
    projects: Vec<PathBuf>,
    project: Option<String>,
    max_agents: Option<usize>,
    positional: Vec<String>,
}

fn parse(args: &[String]) -> Option<Opts> {
    let mut o = Opts::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--socket" => o.socket = Some(it.next()?.into()),
            "--manifests" => o.manifests = Some(it.next()?.into()),
            "--data" => o.data = Some(it.next()?.into()),
            "--worktrees" => o.worktrees = Some(it.next()?.into()),
            "--project" => {
                let v = it.next()?;
                o.projects.push(v.into());
                o.project = Some(v.clone());
            }
            "--max-agents" => o.max_agents = Some(it.next()?.parse().ok()?),
            s if s.starts_with("--") => return None,
            _ => o.positional.push(a.clone()),
        }
    }
    Some(o)
}

fn socket_path(o: &Opts) -> PathBuf {
    o.socket.clone().unwrap_or_else(|| home().join("node.sock"))
}

fn main() -> ExitCode {
    let say = Say(Catalog::english());
    let args: Vec<String> = std::env::args().skip(1).collect();
    let words: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .take(2)
        .take_while(|w| !w.starts_with("--"))
        .collect();
    let rest = args.get(words.len()..).unwrap_or(&[]);
    let Some(opts) = parse(rest) else {
        say.err(key!("cli-bad-arguments"), &[]);
        say.err(key!("cli-usage"), &[]);
        return ExitCode::from(2);
    };
    match words.as_slice() {
        ["node", "run"] => node_run(&say, &opts),
        ["agent", "start"] => agent_start(&say, &opts),
        ["agent", "stop"] => agent_stop(&say, &opts),
        ["agent", "list"] => {
            // Waiting for ADR-0025 (an `agent.list` command); see docs/adr/0025-agent-list-command.md.
            say.err(key!("cli-agent-list-unavailable"), &[]);
            ExitCode::from(2)
        }
        ["terminal", "attach"] => terminal_attach(&say, &opts),
        ["doctor"] => doctor(&say, &opts),
        ["node", "stdio"] => node_stdio(&say, &opts),
        _ => {
            say.err(key!("cli-usage"), &[]);
            ExitCode::from(2)
        }
    }
}

/// `agora node stdio`: carries the node's framed protocol over this process's stdin/stdout, so a remote hub can
/// reach a node over `ssh host agora node stdio` with no open port (ADR-0011). It bridges to the node already
/// running on this machine; the node, its agents and its policy are unchanged, and it never needs the hub.
fn node_stdio(say: &Say, o: &Opts) -> ExitCode {
    let path = socket_path(o);
    let Ok(sock) = agora_platform::local_connect(&path) else {
        say.err(
            key!("cli-node-not-running"),
            &[("socket", path.display().to_string().into())],
        );
        return ExitCode::from(1);
    };
    let Ok(mut from_node) = sock.try_clone() else {
        return ExitCode::from(1);
    };
    // node → stdout. Ends when the node closes the connection or the reader below shuts the socket.
    let out = std::thread::spawn(move || {
        let mut stdout = std::io::stdout().lock();
        let mut buf = [0u8; 16 * 1024];
        while let Ok(n) = from_node.read(&mut buf) {
            if n == 0
                || stdout
                    .write_all(&buf[..n])
                    .and_then(|()| stdout.flush())
                    .is_err()
            {
                break;
            }
        }
    });
    // stdin → node. EOF means the remote side hung up: close the socket so the thread above wakes.
    let mut to_node = sock;
    let mut stdin = std::io::stdin().lock();
    let mut buf = [0u8; 16 * 1024];
    while let Ok(n) = stdin.read(&mut buf) {
        if n == 0 || to_node.write_all(&buf[..n]).is_err() {
            break;
        }
    }
    let _ = to_node.shutdown();
    let _ = out.join();
    ExitCode::SUCCESS
}

/// `agora doctor`: checks that explain most "it does not work" reports.
fn doctor(say: &Say, o: &Opts) -> ExitCode {
    use agora_node::doctor::{healthy, run, Inputs, Status};
    let h = home();
    let checks = run(&Inputs {
        data_dir: o.data.clone().unwrap_or_else(|| h.join("data")),
        socket: socket_path(o),
        manifests_dir: o.manifests.clone().unwrap_or_else(|| h.join("manifests")),
    });
    for c in &checks {
        let sentence = match (c.id, c.status) {
            ("version", _) => key!("doctor-version"),
            ("pty", Status::Ok) => key!("doctor-pty"),
            ("pty", _) => key!("doctor-pty-unsupported"),
            ("local-sockets", Status::Ok) => key!("doctor-local-sockets"),
            ("local-sockets", _) => key!("doctor-local-sockets-unsupported"),
            ("git", Status::Ok) => key!("doctor-git"),
            ("git", _) => key!("doctor-git-missing"),
            ("data-dir", Status::Ok) => key!("doctor-data-dir"),
            ("data-dir", _) => key!("doctor-data-dir-unwritable"),
            ("node", Status::Ok) => key!("doctor-node"),
            ("node", _) => key!("doctor-node-not-running"),
            ("manifests", Status::Ok) => key!("doctor-manifests"),
            ("manifests", _) => key!("doctor-manifests-missing"),
            ("agent-installed", _) => key!("doctor-agent-installed"),
            ("agent-missing", _) => key!("doctor-agent-missing"),
            _ => key!("doctor-manifest-invalid"),
        };
        let detail: Arg = match (c.id, c.detail.parse::<i64>()) {
            ("manifests", Ok(n)) => Arg::Num(n),
            _ => c.detail.as_str().into(),
        };
        let text = say.text(sentence, &[("detail", detail)]);
        let status = say.text(
            match c.status {
                Status::Ok => key!("doctor-status-ok"),
                Status::Warn => key!("doctor-status-warn"),
                Status::Fail => key!("doctor-status-fail"),
            },
            &[],
        );
        say.out(
            key!("doctor-line"),
            &[("status", status.into()), ("text", text.into())],
        );
    }
    if healthy(&checks) {
        say.out(key!("doctor-summary-ok"), &[]);
        ExitCode::SUCCESS
    } else {
        say.err(key!("doctor-summary-problems"), &[]);
        ExitCode::from(1)
    }
}

fn node_run(say: &Say, o: &Opts) -> ExitCode {
    let h = home();
    let cfg = NodeConfig {
        node_id: "local".into(),
        socket: socket_path(o),
        data_dir: o.data.clone().unwrap_or_else(|| h.join("data")),
        worktree_root: o.worktrees.clone().unwrap_or_else(|| h.join("worktrees")),
        manifests_dir: o.manifests.clone().unwrap_or_else(|| h.join("manifests")),
        projects: o.projects.clone(),
        limits: NodeLimits {
            max_agents: o.max_agents,
            ..NodeLimits::default()
        },
    };
    if let Some(parent) = cfg.socket.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let node = match Node::start(&cfg) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };
    for (path, reason) in &node.manifest_problems {
        eprintln!("{}: {reason}", path.display());
    }
    say.out(
        key!("cli-node-listening"),
        &[("socket", cfg.socket.display().to_string().into())],
    );
    // Run until the process is stopped; agents die with it and the socket file is replaced on next start.
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

fn connect(say: &Say, o: &Opts) -> Result<Client, ExitCode> {
    let path = socket_path(o);
    Client::connect(&path).map_err(|_| {
        say.err(
            key!("cli-node-not-running"),
            &[("socket", path.display().to_string().into())],
        );
        ExitCode::from(1)
    })
}

/// Send one command; print the error (and return failure) if the node refuses it.
fn call(say: &Say, c: &mut Client, cmd: Cmd) -> Result<v1::CommandOk, ExitCode> {
    match c.call(cmd) {
        Ok(Ok(ok)) => Ok(ok),
        Ok(Err(e)) => {
            say.remote_error(&e);
            Err(ExitCode::from(1))
        }
        Err(e) => {
            eprintln!("{e}");
            Err(ExitCode::from(1))
        }
    }
}

fn agent_start(say: &Say, o: &Opts) -> ExitCode {
    let Some(kind) = o.positional.first() else {
        say.err(key!("cli-usage"), &[]);
        return ExitCode::from(2);
    };
    let mut c = match connect(say, o) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let cmd = Cmd::AgentStart(v1::AgentStart {
        kind: kind.clone(),
        project_id: o.project.clone().unwrap_or_default(),
        ..Default::default()
    });
    // A bare `--project NAME` is a name, not a path: the node resolves it against its registered projects.
    let cmd = match (&o.project, cmd) {
        (Some(p), Cmd::AgentStart(mut a)) => {
            a.project_id = PathBuf::from(p)
                .file_name()
                .map_or_else(|| p.clone(), |n| n.to_string_lossy().into_owned());
            Cmd::AgentStart(a)
        }
        (_, cmd) => cmd,
    };
    match call(say, &mut c, cmd) {
        Ok(v1::CommandOk {
            result: Some(Ok_::AgentStart(r)),
        }) => {
            say.out(
                key!("cli-agent-started"),
                &[
                    ("agent_id", r.agent_id.into()),
                    ("worktree", r.worktree_id.into()),
                ],
            );
            ExitCode::SUCCESS
        }
        Ok(_) => ExitCode::from(1),
        Err(code) => code,
    }
}

fn agent_stop(say: &Say, o: &Opts) -> ExitCode {
    let Some(id) = o.positional.first() else {
        say.err(key!("cli-usage"), &[]);
        return ExitCode::from(2);
    };
    let mut c = match connect(say, o) {
        Ok(c) => c,
        Err(code) => return code,
    };
    match call(
        say,
        &mut c,
        Cmd::AgentStop(v1::AgentStop {
            agent_id: id.clone(),
        }),
    ) {
        Ok(_) => {
            say.out(
                key!("cli-agent-stopped"),
                &[("agent_id", id.as_str().into())],
            );
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

fn terminal_attach(say: &Say, o: &Opts) -> ExitCode {
    let Some(agent) = o.positional.first() else {
        say.err(key!("cli-usage"), &[]);
        return ExitCode::from(2);
    };
    let terminal_id = format!("t-{agent}");
    let mut c = match connect(say, o) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let Ok(raw) = agora_platform::RawMode::enable() else {
        say.err(key!("cli-attach-needs-terminal"), &[]);
        return ExitCode::from(1);
    };
    // Match the node's terminal to ours, then follow it.
    if let Some((cols, rows)) = agora_platform::terminal_size() {
        let _ = c.call(Cmd::TerminalResize(v1::TerminalResize {
            terminal_id: terminal_id.clone(),
            cols: u32::from(cols),
            rows: u32::from(rows),
        }));
    }
    let subscribed = c.call_on(
        TERMINAL_STREAM,
        Cmd::TerminalSubscribe(v1::TerminalSubscribe {
            terminal_id: terminal_id.clone(),
            format: "terminal.raw".into(),
        }),
    );
    match subscribed {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => {
            drop(raw);
            say.remote_error(&e);
            return ExitCode::from(1);
        }
        Err(e) => {
            drop(raw);
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    }

    let detach = Arc::new(AtomicBool::new(false));
    if let Ok(writer) = c.writer() {
        let (detach, terminal_id) = (Arc::clone(&detach), terminal_id.clone());
        // Detached thread: it ends with the process (a blocked stdin read cannot be interrupted).
        let _ = std::thread::Builder::new()
            .name("agora-attach-stdin".into())
            .spawn(move || {
                let mut buf = [0u8; 4096];
                let mut stdin = std::io::stdin().lock();
                while let Ok(n) = stdin.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    let cut = buf[..n].iter().position(|b| *b == DETACH);
                    let data = buf[..cut.unwrap_or(n)].to_vec();
                    if !data.is_empty()
                        && writer
                            .send(Cmd::TerminalInput(v1::TerminalInput {
                                terminal_id: terminal_id.clone(),
                                data,
                            }))
                            .is_err()
                    {
                        break;
                    }
                    if cut.is_some() {
                        detach.store(true, Ordering::Release);
                        break;
                    }
                }
                detach.store(true, Ordering::Release);
            });
    }

    let mut stdout = std::io::stdout();
    loop {
        match c.read_frame(Some(Duration::from_millis(100))) {
            Ok(Some(f)) if f.stream_id == TERMINAL_STREAM => match f.kind {
                KIND_TERM_SNAPSHOT | KIND_TERM_DATA => {
                    let _ = stdout.write_all(&f.payload).and_then(|()| stdout.flush());
                }
                KIND_TERM_EXIT => break,
                _ => {}
            },
            Ok(Some(_)) => {}
            Ok(None) => {}
            // The node went away: nothing left to show.
            Err(_) => break,
        }
        if detach.load(Ordering::Acquire) {
            break;
        }
    }
    drop(raw);
    println!();
    say.out(
        key!("cli-terminal-detached"),
        &[("agent_id", agent.as_str().into())],
    );
    ExitCode::SUCCESS
}
