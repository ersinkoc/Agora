//! Control socket tests: a real node (supervisor + socket) driven through the protocol client, with a plain
//! `sh` and the fake agent as the "agents". PTY and local sockets are Unix-only until Phase 4.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use agora_node::control::{
    Client, KIND_TERM_DATA, KIND_TERM_EXIT, KIND_TERM_RESIZE, KIND_TERM_SNAPSHOT,
};
use agora_node::daemon::{Node, NodeConfig};
use agora_node::supervisor::NodeLimits;
use agora_protocol::v1::{self, command_ok::Result as Ok_, command_request::Command as Cmd};
use agora_protocol::Frame;

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

struct Env {
    dir: PathBuf,
    repo: PathBuf,
}

fn env(name: &str) -> Env {
    let dir = std::env::temp_dir().join(format!("agora-ctl-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let repo = dir.join("myproject");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let manifests = dir.join("manifests");
    std::fs::create_dir_all(&manifests).unwrap();
    std::fs::write(
        manifests.join("shell.toml"),
        r#"
kind = "shell"
display_name = "Shell"
version = "1"
min_engine = 1
[process]
launch = ["sh"]
"#,
    )
    .unwrap();
    std::fs::write(manifests.join("broken.toml"), "kind = ").unwrap();
    Env { dir, repo }
}

fn node(e: &Env, limits: NodeLimits) -> Node {
    Node::start(&NodeConfig {
        node_id: "node-test".into(),
        // Short path: Unix socket paths are limited to about 100 bytes.
        socket: e.dir.join("c.sock"),
        data_dir: e.dir.join("data"),
        worktree_root: e.dir.join("worktrees"),
        manifests_dir: e.dir.join("manifests"),
        projects: vec![e.repo.clone()],
        limits,
    })
    .expect("start node")
}

fn start_cmd(kind: &str) -> Cmd {
    Cmd::AgentStart(v1::AgentStart {
        kind: kind.into(),
        ..Default::default()
    })
}

fn ok(r: std::io::Result<Result<v1::CommandOk, v1::ErrorInfo>>) -> v1::CommandOk {
    r.expect("io").expect("command failed")
}

fn err_code(r: std::io::Result<Result<v1::CommandOk, v1::ErrorInfo>>) -> String {
    r.expect("io").expect_err("expected an error").code
}

fn started(r: v1::CommandOk) -> v1::AgentStartResult {
    match r.result {
        Some(Ok_::AgentStart(s)) => s,
        other => panic!("not an AgentStart result: {other:?}"),
    }
}

#[test]
fn project_list_and_agent_start_stop_over_the_socket() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let e = env("basic");
    let n = node(&e, NodeLimits::default());
    assert_eq!(
        n.manifest_problems.len(),
        1,
        "the broken manifest is reported, not fatal"
    );
    let mut c = Client::connect(n.socket().unwrap()).unwrap();

    let Some(Ok_::ProjectList(list)) = ok(c.call(Cmd::ProjectList(v1::ProjectList {}))).result
    else {
        panic!("no project list");
    };
    assert_eq!(list.projects.len(), 1);
    assert_eq!(list.projects[0].name, "myproject");

    // Empty project id works when there is exactly one project.
    let s = started(ok(c.call(start_cmd("shell"))));
    assert!(s.agent_id.starts_with("agent-"));
    assert_eq!(s.terminal_id, format!("t-{}", s.agent_id));
    assert!(
        Path::new(&s.worktree_id).join("a.txt").exists(),
        "own worktree"
    );
    assert_eq!(n.supervisor.lock().unwrap().list().len(), 1);

    ok(c.call(Cmd::AgentStop(v1::AgentStop {
        agent_id: s.agent_id.clone(),
    })));
    assert!(!n.supervisor.lock().unwrap().list()[0].running);

    // The start was recorded in the durable event buffer.
    assert!(n.events.next_seq() > 1);
}

#[test]
fn errors_travel_as_codes_with_params() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let e = env("errors");
    let n = node(&e, NodeLimits::default());
    let mut c = Client::connect(n.socket().unwrap()).unwrap();

    let r = c.call(Cmd::AgentStart(v1::AgentStart {
        kind: "shell".into(),
        project_id: "nope".into(),
        ..Default::default()
    }));
    let info = r.unwrap().unwrap_err();
    assert_eq!(info.code, "project.not_found");
    assert!(info.params.contains_key("project_id"));

    assert_eq!(
        err_code(c.call(start_cmd("no-such-kind"))),
        "agent.kind_unknown"
    );
    assert_eq!(
        err_code(c.call(Cmd::AgentStop(v1::AgentStop {
            agent_id: "agent-404".into()
        }))),
        "agent.not_found"
    );
    // Commands this node does not implement yet say so.
    let info = c
        .call(Cmd::FilesList(v1::FilesList::default()))
        .unwrap()
        .unwrap_err();
    assert_eq!(info.code, "protocol.capability_missing");
    assert_eq!(
        info.params["capability"].value,
        Some(v1::param_value::Value::Str("files.list".into()))
    );
}

#[test]
fn resource_limits_are_reported_over_the_wire() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let e = env("limit");
    let n = node(
        &e,
        NodeLimits {
            max_agents: Some(1),
            ..NodeLimits::default()
        },
    );
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    ok(c.call(start_cmd("shell")));
    let info = c.call(start_cmd("shell")).unwrap().unwrap_err();
    assert_eq!(info.code, "node.resource_limit");
    assert_eq!(
        info.params["resource"].value,
        Some(v1::param_value::Value::Str("agents".into()))
    );
    assert_eq!(
        info.params["limit"].value,
        Some(v1::param_value::Value::Int(1))
    );
}

#[test]
fn a_retried_request_does_not_repeat_its_effect() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let e = env("idem");
    let n = node(&e, NodeLimits::default());
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let first = started(ok(c.call_with_key("key-1", start_cmd("shell"))));
    let retry = started(ok(c.call_with_key("key-1", start_cmd("shell"))));
    assert_eq!(first, retry, "the stored response is returned");
    assert_eq!(
        n.supervisor.lock().unwrap().list().len(),
        1,
        "only one agent was started"
    );
    // A different key is a different request.
    let other = started(ok(c.call_with_key("key-2", start_cmd("shell"))));
    assert_ne!(first.agent_id, other.agent_id);
    assert_eq!(n.supervisor.lock().unwrap().list().len(), 2);
}

fn wait_frame(c: &mut Client, stream: u32, mut want: impl FnMut(&Frame) -> bool) -> Frame {
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        if let Some(f) = c.read_frame(Some(Duration::from_millis(200))).unwrap() {
            if f.stream_id == stream && want(&f) {
                return f;
            }
        }
    }
    panic!("timed out waiting for a frame on stream {stream}");
}

#[test]
fn terminal_subscription_streams_snapshot_data_resize_and_exit() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let e = env("term");
    let n = node(&e, NodeLimits::default());
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let s = started(ok(c.call(start_cmd("shell"))));

    // Subscribing on a dynamic stream: response first, then a snapshot, then live data.
    ok(c.call_on(
        3,
        Cmd::TerminalSubscribe(v1::TerminalSubscribe {
            terminal_id: s.terminal_id.clone(),
            format: "terminal.raw".into(),
        }),
    ));
    let snap = wait_frame(&mut c, 3, |f| f.kind == KIND_TERM_SNAPSHOT);
    assert!(snap.payload.starts_with(b"\x1b[0m\x1b[2J"));

    ok(c.call(Cmd::TerminalInput(v1::TerminalInput {
        terminal_id: s.terminal_id.clone(),
        data: b"echo wire-$((6*7))\n".to_vec(),
    })));
    let mut seen = Vec::new();
    wait_frame(&mut c, 3, |f| {
        if f.kind == KIND_TERM_DATA {
            seen.extend_from_slice(&f.payload);
        }
        String::from_utf8_lossy(&seen).contains("wire-42")
    });

    ok(c.call(Cmd::TerminalResize(v1::TerminalResize {
        terminal_id: s.terminal_id.clone(),
        cols: 100,
        rows: 30,
    })));
    let r = wait_frame(&mut c, 3, |f| f.kind == KIND_TERM_RESIZE);
    assert_eq!(r.payload, [100, 0, 30, 0]);

    ok(c.call(Cmd::TerminalInput(v1::TerminalInput {
        terminal_id: s.terminal_id.clone(),
        data: b"exit 5\n".to_vec(),
    })));
    let x = wait_frame(&mut c, 3, |f| f.kind == KIND_TERM_EXIT);
    assert_eq!(i32::from_le_bytes(x.payload.clone().try_into().unwrap()), 5);

    // Bad subscriptions are rejected cleanly.
    assert_eq!(
        err_code(c.call_on(
            4,
            Cmd::TerminalSubscribe(v1::TerminalSubscribe {
                terminal_id: "t-nope".into(),
                format: String::new(),
            })
        )),
        "agent.not_found"
    );
    assert_eq!(
        err_code(c.call_on(
            5,
            Cmd::TerminalSubscribe(v1::TerminalSubscribe {
                terminal_id: s.terminal_id.clone(),
                format: "terminal.cells".into(),
            })
        )),
        "protocol.capability_missing"
    );
    // The commands stream itself is not a terminal stream.
    assert_eq!(
        err_code(c.call(Cmd::TerminalSubscribe(v1::TerminalSubscribe {
            terminal_id: s.terminal_id,
            format: String::new(),
        }))),
        "protocol.invalid_message"
    );
}
