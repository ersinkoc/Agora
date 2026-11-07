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
    let child = Command::new(AGORA)
        .args(["node", "run", "--project"])
        .arg(&s.repo)
        .env("AGORA_HOME", &s.home)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start node");
    let sock = s.home.join("node.sock");
    let end = Instant::now() + Duration::from_secs(15);
    while !sock.exists() {
        assert!(Instant::now() < end, "node did not create its socket");
        std::thread::sleep(Duration::from_millis(50));
    }
    NodeProc(child)
}

/// Run `agora <args>` and return (exit code, stdout, stderr).
fn cli(s: &Setup, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(AGORA)
        .args(args)
        .env("AGORA_HOME", &s.home)
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

#[test]
fn start_attach_stop_from_the_command_line() {
    if !supported() {
        return;
    }
    let s = setup("e2e");
    let _node = start_node(&s);

    // Start an agent in its own worktree.
    let (code, out, err) = cli(&s, &["agent", "start", "shell", "--project", "demo"]);
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.starts_with("Started agent agent-1 in "), "{out}");
    let worktree = out.trim().rsplit(" in ").next().unwrap().to_string();
    assert!(
        Path::new(&worktree).join("README.md").exists(),
        "own worktree: {worktree}"
    );
    assert!(worktree.starts_with(s.home.join("worktrees").to_str().unwrap()));

    // Attach in a real terminal: type a command, see its output, detach with Ctrl-].
    let mut cfg = PtyConfig::new(vec![
        AGORA.into(),
        "terminal".into(),
        "attach".into(),
        "agent-1".into(),
    ]);
    cfg.env
        .push(("AGORA_HOME".into(), s.home.display().to_string()));
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
    if !supported() {
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

    let (code, _, err) = cli(&s, &["agent", "start", "nope"]);
    assert_eq!(code, 1);
    assert!(err.contains("agent.kind_unknown"), "{err}");

    let (code, _, err) = cli(&s, &["agent", "start", "shell", "--project", "ghost"]);
    assert_eq!(code, 1);
    assert!(err.contains("project.not_found"), "{err}");

    // Listing waits for ADR-0025.
    let (code, _, err) = cli(&s, &["agent", "list"]);
    assert_eq!(code, 2);
    assert!(err.contains("not available"), "{err}");
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
    // The manifest launches `sh`, which exists wherever this test can run on Unix; elsewhere it may not.
    if agora_platform::PTY_SUPPORTED {
        assert!(out.contains("shell is installed."), "{out}");
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
