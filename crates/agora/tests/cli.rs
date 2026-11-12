//! The Phase 2 exit criterion as a script: a node runs agents in worktrees and reports state, controlled
//! locally through the CLI. Real `agora` processes, real sockets, a real PTY for `terminal attach`; the
//! "agent" is a plain `sh` (never a real coding agent).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use agora_platform::{spawn, PtyConfig};

const AGORA: &str = env!("CARGO_BIN_EXE_agora");

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .status()
        .expect("git")
        .success();
    assert!(ok, "git {args:?}");
}

struct Setup {
    home: PathBuf,
    repo: PathBuf,
}

fn setup(name: &str) -> Setup {
    let home = std::env::temp_dir().join(format!("agora-cli-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let repo = home.join("demo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), "hi").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    std::fs::create_dir_all(home.join("manifests")).unwrap();
    std::fs::write(
        home.join("manifests/shell.toml"),
        "kind = \"shell\"\ndisplay_name = \"Shell\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"sh\"]\n",
    )
    .unwrap();
    Setup { home, repo }
}

struct NodeProc(Child);

impl Drop for NodeProc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_node(s: &Setup) -> NodeProc {
    start_node_at(s, &s.home)
}
fn start_node_at(s: &Setup, cwd: &Path) -> NodeProc {
    // Owned before the wait: a node that is slow to start is still killed when the test gives up on it (left running, it held the
    // binary open and broke the next build).
    let node = NodeProc(
        Command::new(AGORA)
            .current_dir(cwd)
            .args(["node", "run", "--project"])
            .arg(&s.repo)
            .env("AGORA_HOME", &s.home)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start node"),
    );
    let sock = s.home.join("node.sock");
    // A node on a busy Windows machine took up to 24 s to open its socket (measured), mostly in git.
    let end = Instant::now() + Duration::from_secs(60);
    while !sock.exists() {
        assert!(Instant::now() < end, "node did not create its socket");
        std::thread::sleep(Duration::from_millis(50));
    }
    node
}

/// Run `agora <args>` and return (exit code, stdout, stderr).
fn cli(s: &Setup, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(AGORA)
        .args(args)
        .env("AGORA_HOME", &s.home)
        // Text assertions below are English whatever the machine running the tests is set to.
        .env("AGORA_LANG", "en")
        .output()
        .expect("run agora");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn supported() -> bool {
    agora_platform::LOCAL_SOCKETS_SUPPORTED && agora_platform::PTY_SUPPORTED
}

/// Whether `sh` (the test agent) can be found on `PATH`. Every Unix has it; on Windows it comes with Git's `usr\bin`, which CI runners
/// put on `PATH` and a default Git for Windows install does not.
fn sh_on_path() -> bool {
    std::env::var_os("PATH").is_some_and(|p| {
        std::env::split_paths(&p).any(|d| d.join("sh").is_file() || d.join("sh.exe").is_file())
    })
}

/// The tests that start the `sh` agent run only where it can start; elsewhere they say why they did nothing.
fn shell_agents_supported() -> bool {
    if !supported() {
        return false;
    }
    if !sh_on_path() {
        eprintln!("skipped: no `sh` on PATH (on Windows, add Git's usr\\bin to PATH)");
        return false;
    }
    true
}

#[test]
fn start_attach_stop_from_the_command_line() {
    if !shell_agents_supported() {
        return;
    }
    let s = setup("e2e");
    let _node = start_node(&s);

    // Start an agent in its own worktree.
    let (code, out, err) = cli(
        &s,
        &["agent", "start", "shell", "--project", "demo", "--worktree"],
    );
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.starts_with("Started agent agent-1 in "), "{out}");
    let worktree = out.trim().rsplit(" in ").next().unwrap().to_string();
    assert!(
        Path::new(&worktree).join("README.md").exists(),
        "own worktree: {worktree}"
    );
    assert!(std::fs::canonicalize(&worktree)
        .unwrap()
        .starts_with(std::fs::canonicalize(s.home.join("worktrees")).unwrap()));

    // Attach in a real terminal: type a command, see its output, detach with Ctrl-].
    let mut cfg = PtyConfig::new(vec![
        AGORA.into(),
        "terminal".into(),
        "attach".into(),
        "agent-1".into(),
    ]);
    cfg.env
        .push(("AGORA_HOME".into(), s.home.display().to_string()));
    cfg.env.push(("AGORA_LANG".into(), "en".into()));
    let mut attach = spawn(&cfg).expect("spawn attach");
    attach.write_input(b"echo attached-$((6*7))\r").unwrap();
    let mut seen = String::new();
    let end = Instant::now() + Duration::from_secs(15);
    while !seen.contains("attached-42") {
        assert!(
            Instant::now() < end,
            "never saw the command output; got: {seen:?}"
        );
        if let Ok(chunk) = attach.output().recv_timeout(Duration::from_millis(100)) {
            seen.push_str(&String::from_utf8_lossy(&chunk));
        }
    }
    // Resizing the window resizes the agent's terminal: the shell inside reports the new size.
    attach.resize(101, 33).unwrap();
    attach.write_input(b"stty size\r").unwrap();
    let end = Instant::now() + Duration::from_secs(15);
    while !seen.contains("33 101") {
        assert!(
            Instant::now() < end,
            "the agent never saw the new size; got: {seen:?}"
        );
        if let Ok(chunk) = attach.output().recv_timeout(Duration::from_millis(100)) {
            seen.push_str(&String::from_utf8_lossy(&chunk));
        }
    }
    attach.write_input(&[0x1d]).unwrap();
    let status = attach.wait().unwrap();
    assert!(status.success(), "attach exited with {status:?}");
    while let Ok(chunk) = attach.output().recv_timeout(Duration::from_millis(500)) {
        seen.push_str(&String::from_utf8_lossy(&chunk));
    }
    assert!(seen.contains("Detached from agent-1."), "{seen:?}");

    // The agent is still running after detaching; stop it.
    let (code, out, err) = cli(&s, &["agent", "stop", "agent-1"]);
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(out.trim(), "Stopped agent agent-1");
}

#[test]
fn errors_are_rendered_from_codes_and_exit_nonzero() {
    if !shell_agents_supported() {
        return;
    }
    let s = setup("errors");
    let _node = start_node(&s);

    let (code, _, err) = cli(&s, &["agent", "stop", "agent-9"]);
    assert_eq!(code, 1);
    assert!(
        err.contains("agent.not_found") && err.contains("agent-9 was not found"),
        "{err}"
    );

    let (code, _, err) = cli(&s, &["agent", "prompt", "agent-9", "hello", "there"]);
    assert_eq!(code, 1);
    assert!(err.contains("agent.not_found"), "{err}");
    let (code, _, err) = cli(&s, &["agent", "prompt", "agent-9"]);
    assert_eq!(code, 2, "a prompt needs text: {err}");

    let (code, _, err) = cli(&s, &["agent", "start", "nope"]);
    assert_eq!(code, 1);
    assert!(err.contains("agent.kind_unknown"), "{err}");

    let (code, _, err) = cli(&s, &["agent", "start", "shell", "--project", "ghost"]);
    assert_eq!(code, 1);
    assert!(err.contains("project.not_found"), "{err}");

    // Listing (ADR-0025): empty first, then one tab-separated line per agent.
    let (code, out, _) = cli(&s, &["agent", "list"]);
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "This node has no agents.");
    let (code, _, err) = cli(
        &s,
        &["agent", "start", "shell", "--project", "demo", "--worktree"],
    );
    assert_eq!(code, 0, "{err}");
    let (code, out, _) = cli(&s, &["agent", "list"]);
    assert_eq!(code, 0);
    let cols: Vec<&str> = out.trim().split('\t').collect();
    assert_eq!(
        (cols[0], cols[1], cols[3]),
        ("agent-1", "shell", "running"),
        "{out}"
    );
    cli(&s, &["agent", "stop", "agent-1"]);
    let (_, out, _) = cli(&s, &["agent", "list"]);
    assert!(out.trim().ends_with("stopped"), "{out}");
}

#[test]
fn without_a_node_the_cli_says_so_and_bad_arguments_print_usage() {
    if !supported() {
        return;
    }
    let s = setup("nonode");
    let (code, _, err) = cli(&s, &["agent", "stop", "agent-1"]);
    assert_eq!(code, 1);
    assert!(err.contains("No node is listening"), "{err}");

    let (code, _, err) = cli(&s, &["agent", "start"]);
    assert_eq!(code, 2);
    assert!(err.contains("Usage:"), "{err}");
    let (code, _, err) = cli(&s, &["frobnicate"]);
    assert_eq!(code, 2);
    assert!(err.contains("Usage:"), "{err}");
    let (code, _, err) = cli(&s, &["agent", "stop", "--bogus"]);
    assert_eq!(code, 2);
    assert!(err.contains("not understood"), "{err}");

    // Attaching needs a terminal; with piped stdio it refuses instead of corrupting anything.
    let _node = start_node(&s);
    let (code, _, err) = cli(&s, &["terminal", "attach", "agent-1"]);
    assert_eq!(code, 1);
    assert!(err.contains("interactive terminal"), "{err}");
}

#[test]
fn doctor_reports_agents_and_exits_zero_on_a_healthy_machine() {
    let s = setup("doctor");
    let (code, out, err) = cli(&s, &["doctor"]);
    // Without terminal support (Windows for now) doctor correctly reports a problem and exits 1.
    let expected = if agora_platform::PTY_SUPPORTED { 0 } else { 1 };
    assert_eq!(code, expected, "{out}{err}");
    assert!(out.contains("Git is available"), "{out}");
    assert!(out.contains("1 agent description loaded."), "{out}");
    // The manifest launches `sh`: doctor says whether it is installed, and a missing one is a note, not a failure.
    if agora_platform::PTY_SUPPORTED {
        if sh_on_path() {
            assert!(out.contains("shell is installed."), "{out}");
        } else {
            assert!(out.contains("[note] shell is not installed"), "{out}");
        }
    }
    // No node is running in this setup: a note, not a failure.
    assert!(out.contains("[note] No node is running"), "{out}");
}

#[test]
fn node_stdio_carries_the_framed_protocol_over_pipes() {
    use agora_protocol::frame::DEFAULT_MAX_PAYLOAD;
    use agora_protocol::v1::{command_request::Command as Cmd, command_response::Result as Res};
    use agora_protocol::{Decoder, Frame};
    use prost::Message;

    if !supported() {
        return;
    }
    let s = setup("stdio");
    let _node = start_node(&s);
    let mut child = Command::new(AGORA)
        .args(["node", "stdio"])
        .env("AGORA_HOME", &s.home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let req = agora_protocol::v1::CommandRequest {
        request_id: "r1".into(),
        idempotency_key: String::new(),
        target_node: String::new(),
        command: Some(Cmd::AgentStop(agora_protocol::v1::AgentStop {
            agent_id: "agent-404".into(),
        })),
    };
    let bytes = Frame {
        stream_id: 1,
        kind: 0,
        seq: None,
        payload: req.encode_to_vec(),
    }
    .encode(DEFAULT_MAX_PAYLOAD)
    .unwrap();
    child.stdin.as_mut().unwrap().write_all(&bytes).unwrap();

    let mut out = child.stdout.take().unwrap();
    let mut dec = Decoder::new(DEFAULT_MAX_PAYLOAD);
    let mut buf = [0u8; 4096];
    let frame = loop {
        let n = out.read(&mut buf).unwrap();
        assert!(n > 0, "bridge closed before answering");
        dec.push(&buf[..n]);
        if let Some(f) = dec.next_frame().unwrap() {
            break f;
        }
    };
    assert_eq!((frame.stream_id, frame.kind), (1, 1));
    let resp = agora_protocol::v1::CommandResponse::decode(frame.payload.as_slice()).unwrap();
    assert_eq!(resp.request_id, "r1");
    match resp.result {
        Some(Res::Err(e)) => assert_eq!(e.code, "agent.not_found"),
        other => panic!("expected agent.not_found, got {other:?}"),
    }
    // The remote side hanging up ends the bridge.
    drop(child.stdin.take());
    let status = child.wait().unwrap();
    assert!(status.success());
}

#[test]
fn node_stdio_without_a_node_says_so() {
    if !supported() {
        return;
    }
    let s = setup("stdio-none");
    let (code, _, err) = cli(&s, &["node", "stdio"]);
    assert_eq!(code, 1);
    assert!(err.contains("No node is listening"), "{err}");
}

#[test]
fn mesh_stdio_is_an_mcp_server_for_one_agent() {
    if !shell_agents_supported() {
        return;
    }
    let s = setup("mesh-stdio");
    let _node = start_node(&s);
    let (code, out, err) = cli(
        &s,
        &["agent", "start", "shell", "--project", "demo", "--worktree"],
    );
    assert_eq!(code, 0, "{out}{err}");
    // This is the command an agent's MCP configuration would run.
    let socket = s.home.join("m").join("agent-1.sock");
    let mut child = Command::new(AGORA)
        .args(["mesh", "stdio", "--socket"])
        .arg(&socket)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut out = std::io::BufReader::new(child.stdout.take().unwrap());
    let mut ask = |req: &str| -> String {
        use std::io::BufRead;
        writeln!(stdin, "{req}").unwrap();
        let mut line = String::new();
        out.read_line(&mut line).unwrap();
        line
    };
    let init = ask(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
    );
    assert!(init.contains("agora-mesh"), "{init}");
    let tools = ask(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
    assert!(
        tools.contains("send_message") && tools.contains("handoff"),
        "{tools}"
    );
    let me = ask(
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"agents_list","arguments":{}}}"#,
    );
    assert!(me.contains("agent-1"), "{me}");
    drop(stdin);
    assert!(child.wait().unwrap().success());
}

/// What an agent's MCP client does: read the config the node wrote, run the command in it, speak MCP to it.
#[test]
fn the_injected_mcp_config_launches_a_working_mesh_server() {
    if !shell_agents_supported() {
        return;
    }
    let s = setup("inject-e2e");
    std::fs::write(
        s.home.join("manifests/mcp-shell.toml"),
        "kind = \"mcp-shell\"\ndisplay_name = \"S\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"sh\"]\n[mcp]\nmethod = \"config_file\"\npath = \".mcp.json\"\n",
    )
    .unwrap();
    let _node = start_node(&s);
    let (code, out, err) = cli(
        &s,
        &[
            "agent",
            "start",
            "mcp-shell",
            "--project",
            "demo",
            "--worktree",
        ],
    );
    assert_eq!(code, 0, "{out}{err}");
    let worktree = out.trim().rsplit(" in ").next().unwrap().to_string();
    let cfg: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(Path::new(&worktree).join(".mcp.json")).unwrap(),
    )
    .unwrap();
    let server = &cfg["mcpServers"]["agora-mesh"];
    let args: Vec<String> = server["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap().to_string())
        .collect();
    let mut child = Command::new(server["command"].as_str().unwrap())
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut out = std::io::BufReader::new(child.stdout.take().unwrap());
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{}}}}"#
    )
    .unwrap();
    let mut line = String::new();
    std::io::BufRead::read_line(&mut out, &mut line).unwrap();
    assert!(line.contains("agora-mesh"), "{line}");
    drop(stdin);
    assert!(child.wait().unwrap().success());
}

#[test]
fn a_node_owner_can_trust_a_device_and_bad_input_trusts_nothing() {
    let home = std::env::temp_dir().join(format!("agora-trustcli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let key = format!("04{}", "ab".repeat(64));
    let run = |args: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_agora"))
            .args(args)
            .env("AGORA_HOME", &home)
            .env("AGORA_LANG", "en")
            .output()
            .unwrap()
    };
    assert!(run(&["node", "trust", "phone", &key]).status.success());
    let file = home.join("data/trusted-devices");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        format!("phone {key}\n")
    );
    assert!(!run(&["node", "trust", "phone", "zz"]).status.success());
    assert!(!run(&["node", "trust", "phone"]).status.success());
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        format!("phone {key}\n")
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_registered_project_name_wins_over_same_named_folders_in_both_processes() {
    if !agora_platform::PTY_SUPPORTED || !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let s = setup("project-name");
    let caller = s.home.join("caller");
    let caller_demo = caller.join("demo");
    std::fs::create_dir_all(&caller_demo).unwrap();
    let _node = start_node_at(&s, &caller);
    for (option, value, expected) in [
        ("--project", "demo", s.repo.as_path()),
        ("--cwd", "demo", caller_demo.as_path()),
    ] {
        let out = Command::new(AGORA)
            .args(["agent", "start", "shell", option, value])
            .current_dir(&caller)
            .env("AGORA_HOME", &s.home)
            .env("AGORA_LANG", "en")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8(out.stdout).unwrap();
        let cwd = stdout.trim().rsplit(" in ").next().unwrap();
        assert_eq!(
            std::fs::canonicalize(cwd).unwrap(),
            std::fs::canonicalize(expected).unwrap()
        );
    }
}
