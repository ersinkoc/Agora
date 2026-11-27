//! Control socket tests: a real node (supervisor + socket) driven through the protocol client, with a plain
//! `sh` and the fake agent as the "agents". PTY and local sockets: Unix and Windows (ADR-0045).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use agora_node::control::{
    Client, KIND_CHAT_ENTRY, KIND_CHAT_SNAPSHOT, KIND_TERM_DATA, KIND_TERM_EXIT, KIND_TERM_HISTORY,
    KIND_TERM_RESIZE, KIND_TERM_SNAPSHOT,
};
use agora_node::daemon::{Node, NodeConfig};
use agora_node::supervisor::NodeLimits;
use agora_protocol::v1::{self, command_ok::Result as Ok_, command_request::Command as Cmd};
use agora_protocol::Frame;

/// Whether `sh` (the test agent) can be found on `PATH`, as in `crates/agora/tests/cli.rs`. Every Unix has it; on Windows it comes
/// with Git's `usr\bin`, which CI runners put on `PATH` and a default Git for Windows install does not (#972).
fn sh_on_path() -> bool {
    let found = std::env::var_os("PATH").is_some_and(|p| {
        std::env::split_paths(&p).any(|d| d.join("sh").is_file() || d.join("sh.exe").is_file())
    });
    if !found {
        eprintln!("skipped: no `sh` on PATH (on Windows, add Git's usr\\bin to PATH)");
    }
    found
}

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
        resume_all: false,
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
    if !sh_on_path() {
        return;
    }
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
        stop_helpers: false,
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
            agent_id: "agent-404".into(),
            stop_helpers: false,
        }))),
        "agent.not_found"
    );
    // Commands this node does not implement yet say so.
    let info = c
        .call(Cmd::WorktreeRemove(v1::WorktreeRemove::default()))
        .unwrap()
        .unwrap_err();
    assert_eq!(info.code, "protocol.capability_missing");
    assert_eq!(
        info.params["capability"].value,
        Some(v1::param_value::Value::Str("worktree.remove".into()))
    );
    // ADR-0109: a node that does not take durable handoffs says so to an old or local caller.
    let info = c
        .call(Cmd::DeliverTaskHandoff(v1::DeliverTaskHandoff::default()))
        .unwrap()
        .unwrap_err();
    assert_eq!(info.code, "protocol.capability_missing");
    assert_eq!(
        info.params["capability"].value,
        Some(v1::param_value::Value::Str("mesh.task_handoff.v1".into()))
    );
}

#[test]
fn resource_limits_are_reported_over_the_wire() {
    if !sh_on_path() {
        return;
    }
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
    if !sh_on_path() {
        return;
    }
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
    // What arrived instead, so a timeout on a slow machine says which wait failed and what the terminal sent.
    let mut seen: Vec<(u8, String)> = Vec::new();
    while Instant::now() < end {
        if let Some(f) = c.read_frame(Some(Duration::from_millis(200))).unwrap() {
            if f.stream_id == stream && want(&f) {
                return f;
            }
            if seen.len() < 40 {
                seen.push((f.kind, String::from_utf8_lossy(&f.payload).into_owned()));
            }
        }
    }
    panic!("timed out waiting for a frame on stream {stream}; frames seen meanwhile: {seen:?}");
}

#[test]
fn terminal_subscription_streams_snapshot_data_resize_and_exit() {
    if !sh_on_path() {
        return;
    }
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
            history_lines: 0,
        }),
    ));
    let snap = wait_frame(&mut c, 3, |f| f.kind == KIND_TERM_SNAPSHOT);
    // ADR-0100: the program's screen first (the shell is on the normal one), then the drawing state (origin, insert, pen, ASCII) and the clear.
    assert!(snap
        .payload
        .starts_with(b"\x1b[?1049l\x1b[?6l\x1b[4l\x1b[0m\x1b(B\x0f\x1b[2J"));
    // Right after it, the size it was painted at (the node's default terminal, 80x24).
    let size = wait_frame(&mut c, 3, |f| f.kind == KIND_TERM_RESIZE);
    assert_eq!(size.payload, [80u8, 0, 24, 0]);

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

    // A grid is allocated for whatever size is asked: absurd sizes are refused instead of taking the node down.
    assert_eq!(
        err_code(c.call(Cmd::TerminalResize(v1::TerminalResize {
            terminal_id: s.terminal_id.clone(),
            cols: 65535,
            rows: 65535,
        }))),
        "protocol.invalid_message"
    );
    ok(c.call(Cmd::TerminalResize(v1::TerminalResize {
        terminal_id: s.terminal_id.clone(),
        cols: 100,
        rows: 30,
    })));
    let r = wait_frame(&mut c, 3, |f| f.kind == KIND_TERM_RESIZE);
    assert_eq!(r.payload, [100, 0, 30, 0]);

    // A leading empty line: on Windows the msys `sh` sometimes loses the first byte typed after a ConPTY resize (about 1 run in 10,
    // `xit 5`; never without the resize, and waiting a second does not help). Losing the newline instead is harmless.
    ok(c.call(Cmd::TerminalInput(v1::TerminalInput {
        terminal_id: s.terminal_id.clone(),
        data: b"\nexit 5\n".to_vec(),
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
                history_lines: 0,
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
                history_lines: 0,
            })
        )),
        "protocol.capability_missing"
    );
    // The commands stream itself is not a terminal stream.
    assert_eq!(
        err_code(c.call(Cmd::TerminalSubscribe(v1::TerminalSubscribe {
            terminal_id: s.terminal_id,
            format: String::new(),
            history_lines: 0,
        }))),
        "protocol.invalid_message"
    );
}

/// A `terminal.subscribe` for `terminal_id` that asks for `history_lines` lines of history (ADR-0100).
fn subscribe_cmd(terminal_id: &str, history_lines: u32) -> Cmd {
    Cmd::TerminalSubscribe(v1::TerminalSubscribe {
        terminal_id: terminal_id.into(),
        format: "terminal.raw".into(),
        history_lines,
    })
}

/// Scrollback lines of `t`, oldest first.
fn history_lines(t: &dyn agora_vt::Terminal) -> Vec<String> {
    (1..=t.history_len())
        .rev()
        .map(|b| t.history_line(b))
        .collect()
}

/// ADR-0100 decision 2: a subscription that asks for history gets kind 4 frames, then the kind 1 repaint, then the kind 2 size,
/// then live output; fed to an empty emulator they give the node's scrollback and screen. One that asks for none (absent or 0) gets
/// no kind 4, and a resync after falling behind never carries history.
#[test]
fn history_comes_before_the_snapshot_only_when_asked_and_never_with_a_resync() {
    if !sh_on_path() {
        return;
    }
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let e = env("history");
    let n = node(&e, NodeLimits::default());
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let s = started(ok(c.call(start_cmd("shell"))));
    let id = agora_core::AgentId::new(s.agent_id.clone());

    // Without history: the snapshot comes first.
    ok(c.call_on(3, subscribe_cmd(&s.terminal_id, 0)));
    let first = wait_frame(&mut c, 3, |_| true);
    assert_eq!(first.kind, KIND_TERM_SNAPSHOT);
    ok(c.call(Cmd::TerminalInput(v1::TerminalInput {
        terminal_id: s.terminal_id.clone(),
        data: b"i=1; while [ $i -le 60 ]; do echo hist-$i; i=$((i+1)); done\n".to_vec(),
    })));
    let mut seen = Vec::new();
    wait_frame(&mut c, 3, |f| {
        assert_ne!(f.kind, KIND_TERM_HISTORY, "no history was asked for");
        if f.kind == KIND_TERM_DATA {
            seen.extend_from_slice(&f.payload);
        }
        // The whole last line, not a prefix of it: "hist-60" followed by its line ending, or (a newer ConPTY, which writes the line
        // ending before each line) by the escape sequence that comes next (#973).
        let text = String::from_utf8_lossy(&seen);
        text.match_indices("hist-60").any(|(at, m)| {
            matches!(
                text[at + m.len()..].chars().next(),
                Some('\r' | '\n' | '\x1b')
            )
        })
    });

    // With history: kind 4 frames, kind 1, kind 2, then live output.
    ok(c.call_on(5, subscribe_cmd(&s.terminal_id, 1000)));
    let mut kinds = Vec::new();
    let mut window: Option<agora_vt::AlacrittyTerminal> = None;
    let mut fed = Vec::new();
    wait_frame(&mut c, 5, |f| {
        kinds.push(f.kind);
        if f.kind == KIND_TERM_HISTORY || f.kind == KIND_TERM_SNAPSHOT {
            fed.push(f.payload.clone());
        }
        if f.kind == KIND_TERM_RESIZE {
            let size = agora_vt::Size {
                cols: u16::from_le_bytes([f.payload[0], f.payload[1]]),
                rows: u16::from_le_bytes([f.payload[2], f.payload[3]]),
            };
            let mut w = agora_vt::AlacrittyTerminal::new(size);
            for b in &fed {
                agora_vt::Terminal::feed(&mut w, b);
            }
            window = Some(w);
        }
        f.kind == KIND_TERM_RESIZE
    });
    let snapshot_at = kinds.iter().position(|&k| k == KIND_TERM_SNAPSHOT).unwrap();
    assert!(snapshot_at > 0, "history first: {kinds:?}");
    assert!(
        kinds[..snapshot_at].iter().all(|&k| k == KIND_TERM_HISTORY),
        "{kinds:?}"
    );
    assert_eq!(kinds[snapshot_at + 1..], [KIND_TERM_RESIZE], "{kinds:?}");
    let mut window = window.unwrap();
    // The window is the terminal as of the subscribe, and the node is read later. Output still arriving in between can scroll one
    // more line into the node's history: the shell's prompt after `hist-60`, which a newer ConPTY ends the line for only when it
    // writes it (under load on Windows the window ended at `hist-36`, the node at `hist-37`). So the live output after the size is
    // fed in too, until the two agree or 10 s pass: history and snapshot plus live output give the node's scrollback and screen.
    // A line the history frames lost never comes back this way, so the comparison below still catches it (#1150).
    let node_state = || {
        let sup = n.supervisor.lock().unwrap();
        let t = sup.terminal(&id).unwrap();
        t.inspect(|t| (history_lines(t), t.bottom_lines(usize::from(t.size().rows))))
    };
    let end = Instant::now() + Duration::from_secs(10);
    let (node_history, node_screen) = loop {
        let (history, screen) = node_state();
        let agree = history_lines(&window) == history
            && agora_vt::Terminal::bottom_lines(&window, screen.len()) == screen;
        if agree || Instant::now() >= end {
            break (history, screen);
        }
        if let Some(f) = c.read_frame(Some(Duration::from_millis(200))).unwrap() {
            if f.stream_id == 5 {
                assert_eq!(f.kind, KIND_TERM_DATA, "live output after the size");
                agora_vt::Terminal::feed(&mut window, &f.payload);
            }
        }
    };
    assert!(
        node_history.iter().any(|l| l == "hist-1"),
        "{node_history:?}"
    );
    assert_eq!(history_lines(&window), node_history);
    assert_eq!(
        agora_vt::Terminal::bottom_lines(&window, node_screen.len()),
        node_screen
    );
    ok(c.call(Cmd::TerminalInput(v1::TerminalInput {
        terminal_id: s.terminal_id.clone(),
        data: b"echo live-$((6*7))\n".to_vec(),
    })));
    let mut live = Vec::new();
    wait_frame(&mut c, 5, |f| {
        assert_eq!(f.kind, KIND_TERM_DATA, "live output after the size");
        live.extend_from_slice(&f.payload);
        String::from_utf8_lossy(&live).contains("live-42")
    });

    // A resync after falling behind: the client stops reading while the terminal floods, so the node's queue for it overflows; the
    // next snapshot on the stream that asked for history comes without any.
    ok(c.call_on(7, subscribe_cmd(&s.terminal_id, 2000)));
    ok(c.call(Cmd::TerminalInput(v1::TerminalInput {
        terminal_id: s.terminal_id.clone(),
        data: b"yes 'resync flood resync flood' | head -n 1500000; echo flood-$((6*7))-done\n"
            .to_vec(),
    })));
    // How long the queue takes to overflow depends on how fast the shell writes, and load slows it: after a fixed 3 s pause it
    // sometimes had not, and no resync came (#1150). So the pauses grow (3, 6, 12, then 24 s) with reading in between, 120 s at
    // most. A subscriber that fell behind stays behind until its resync, so reading between pauses never hides one.
    let mut snapshots = 0;
    let mut kinds = Vec::new();
    let end = Instant::now() + Duration::from_secs(120);
    let mut pause = Duration::from_secs(3);
    while snapshots < 2 && Instant::now() < end {
        std::thread::sleep(pause);
        pause = (pause * 2).min(Duration::from_secs(24));
        let read_until = Instant::now() + Duration::from_secs(5);
        while snapshots < 2 && Instant::now() < read_until {
            if let Some(f) = c.read_frame(Some(Duration::from_millis(200))).unwrap() {
                if f.stream_id != 7 {
                    continue;
                }
                if f.kind != KIND_TERM_DATA && kinds.len() < 200 {
                    kinds.push(f.kind);
                }
                if f.kind == KIND_TERM_SNAPSHOT {
                    snapshots += 1;
                } else if f.kind == KIND_TERM_HISTORY {
                    assert_eq!(
                        snapshots, 0,
                        "history only before the first snapshot: {kinds:?}"
                    );
                }
            }
        }
    }
    assert_eq!(snapshots, 2, "the flood made the stream resync: {kinds:?}");
    assert!(kinds[0] == KIND_TERM_HISTORY, "{kinds:?}");
    let _ = c.call(Cmd::TerminalInput(v1::TerminalInput {
        terminal_id: s.terminal_id.clone(),
        data: vec![0x03],
    }));
    let _ = std::fs::remove_dir_all(&e.dir);
}

/// What a hub needs to build its view: the node, its checkout, then for each agent its workspace, the agent and its
/// terminal, in that order, and the matching closings.
#[test]
fn the_node_reports_what_the_hub_needs_to_show_it() {
    if !sh_on_path() {
        return;
    }
    use v1::event::Body;
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let e = env("events");
    let n = node(&e, NodeLimits::default());
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let s = started(ok(c.call(start_cmd("shell"))));
    ok(c.call(Cmd::AgentStop(v1::AgentStop {
        agent_id: s.agent_id.clone(),
        stop_helpers: false,
    })));

    // Exit is noticed by the supervision tick; wait for the terminal to be reported closed.
    let end = Instant::now() + Duration::from_secs(15);
    let events = loop {
        let evs = n.events.events_after(0);
        if evs
            .iter()
            .any(|e| matches!(e.body, Some(Body::TerminalClosed(_))))
            || Instant::now() > end
        {
            break evs;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let names: Vec<&str> = events
        .iter()
        .filter_map(|e| e.body.as_ref())
        .map(|b| match b {
            Body::NodeOnline(_) => "node_online",
            Body::CheckoutDiscovered(_) => "checkout",
            Body::WorktreeCreated(_) => "worktree",
            Body::AgentStarted(_) => "agent_started",
            Body::TerminalOpened(_) => "terminal_opened",
            Body::NodeResources(_) => "resources",
            Body::AgentStateChanged(_) => "state",
            Body::AgentExited(_) => "exited",
            Body::TerminalClosed(_) => "terminal_closed",
            _ => "other",
        })
        .collect();
    let pos = |n: &str| names.iter().position(|x| *x == n).unwrap_or(usize::MAX);
    assert_eq!(pos("node_online"), 0, "{names:?}");
    assert!(pos("checkout") < pos("worktree"), "{names:?}");
    assert!(pos("worktree") < pos("agent_started"), "{names:?}");
    assert!(pos("agent_started") < pos("terminal_opened"), "{names:?}");
    assert!(pos("exited") < pos("terminal_closed"), "{names:?}");

    // Closed once, not once for the stop and again when the supervisor notices the process is gone.
    std::thread::sleep(Duration::from_millis(500));
    let closed = n
        .events
        .events_after(0)
        .iter()
        .filter(|e| matches!(e.body, Some(Body::TerminalClosed(_))))
        .count();
    assert_eq!(closed, 1);

    // Sequence numbers are contiguous from 1, as the hub's exactly-once rule requires.
    let seqs: Vec<u64> = events.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, (1..=seqs.len() as u64).collect::<Vec<_>>());

    // The workspace event carries the branch the supervisor made, and the terminal id matches the control socket's.
    let wt = events
        .iter()
        .find_map(|e| match &e.body {
            Some(Body::WorktreeCreated(w)) => Some(w.clone()),
            _ => None,
        })
        .unwrap();
    assert!(wt.branch.starts_with("agent/"), "{wt:?}");
    let term = events
        .iter()
        .find_map(|e| match &e.body {
            Some(Body::TerminalOpened(t)) => Some(t.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(term.terminal_id, s.terminal_id);

    // Going down cleanly says so.
    drop(c);
    let events_handle = std::sync::Arc::clone(&n.events);
    drop(n);
    let last = events_handle.events_after(0).pop().unwrap();
    assert!(matches!(last.body, Some(Body::NodeOffline(_))));
}

/// A raw MCP client on an agent's mesh socket: what an agent's MCP client would do, minus the agent.
struct Mcp {
    w: agora_platform::LocalStream,
    r: std::io::BufReader<agora_platform::LocalStream>,
    next: u64,
}

impl Mcp {
    fn connect(path: &Path) -> Self {
        use std::io::Write;
        let w = agora_platform::local_connect(path).expect("connect mesh socket");
        let r = w.try_clone().unwrap();
        r.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let mut c = Self {
            w,
            r: std::io::BufReader::new(r),
            next: 1,
        };
        let init = c.rpc(
            "initialize",
            serde_json::json!({"protocolVersion": "2025-06-18"}),
        );
        assert_eq!(init["result"]["serverInfo"]["name"], "agora-mesh");
        c.w.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
            .unwrap();
        c
    }

    fn send(&mut self, method: &str, params: serde_json::Value) -> u64 {
        use std::io::Write;
        let id = self.next;
        self.next += 1;
        let line =
            serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(self.w, "{line}").unwrap();
        id
    }

    fn read(&mut self) -> serde_json::Value {
        use std::io::BufRead;
        let mut s = String::new();
        self.r.read_line(&mut s).expect("reply");
        serde_json::from_str(&s).unwrap_or_else(|_| panic!("not json: {s:?}"))
    }

    fn rpc(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        let id = self.send(method, params);
        let v = self.read();
        assert_eq!(v["id"], id);
        v
    }

    /// A tool call: the decoded JSON text of the result, and whether it was an error.
    fn tool(&mut self, name: &str, args: serde_json::Value) -> (serde_json::Value, bool) {
        let v = self.rpc(
            "tools/call",
            serde_json::json!({"name": name, "arguments": args}),
        );
        decode_tool(&v)
    }
}

fn decode_tool(v: &serde_json::Value) -> (serde_json::Value, bool) {
    let text = v["result"]["content"][0]["text"]
        .as_str()
        .expect("tool result");
    (
        serde_json::from_str(text).unwrap(),
        v["result"]["isError"] == true,
    )
}

#[test]
fn agents_on_one_node_coordinate_through_their_own_mesh_endpoints() {
    if !sh_on_path() {
        return;
    }
    use agora_core::AgentId;
    use serde_json::json;
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let e = env("mesh");
    let n = node(&e, NodeLimits::default());
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let a1 = started(ok(c.call(start_cmd("shell")))).agent_id;
    let a2 = started(ok(c.call(start_cmd("shell")))).agent_id;
    let sock = |id: &str| n.mesh.socket_path(&AgentId::new(id));
    let (mut m1, mut m2) = (Mcp::connect(&sock(&a1)), Mcp::connect(&sock(&a2)));

    // Each endpoint is its agent: the broker stamps who sent what.
    let (list, _) = m1.tool("agents_list", json!({}));
    assert_eq!(list.as_array().unwrap().len(), 2);
    let (r, err) = m1.tool("send_message", json!({"to": a2, "body": "hello from one"}));
    assert!(!err, "{r}");
    let (inbox, _) = m2.tool("inbox", json!({}));
    assert_eq!(inbox[0]["body"], "hello from one");
    assert_eq!(inbox[0]["provenance"]["agent"], json!(a1));
    assert_eq!(inbox[0]["provenance"]["node"], "node-test");

    // An ask waits for the reply without blocking the asker's other calls.
    let ask = m1.send(
        "tools/call",
        json!({"name": "ask", "arguments": {"to": a2, "question": "which port?", "timeout_s": 20}}),
    );
    let ping = m1.rpc("ping", json!({}));
    assert!(
        ping["result"].is_object(),
        "ping answered while the ask waits"
    );
    let (inbox, _) = m2.tool("inbox", json!({"since": inbox[0]["message_id"]}));
    let q = inbox
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "ask")
        .expect("the question");
    let (_, err) = m2.tool(
        "send_message",
        json!({"to": a1, "body": "8080", "reply_to": q["message_id"]}),
    );
    assert!(!err);
    let reply = m1.read();
    assert_eq!(reply["id"], ask);
    let (r, err) = decode_tool(&reply);
    assert!(!err);
    assert_eq!(
        (r["reply"].as_str(), r["from"].as_str()),
        (Some("8080"), Some(a2.as_str()))
    );

    // Leases conflict between agents; when an agent stops, it gives its leases back and its endpoint closes.
    let (l, err) = m1.tool(
        "lease_acquire",
        json!({"paths": ["src/main.rs"], "ttl_s": 600, "reason": "edit"}),
    );
    assert!(!err, "{l}");
    let (r, err) = m2.tool("lease_acquire", json!({"paths": ["src"], "ttl_s": 60}));
    assert!(err);
    assert_eq!(r["error"]["code"], "lease.held");
    ok(c.call(Cmd::AgentStop(v1::AgentStop {
        agent_id: a1.clone(),
        stop_helpers: false,
    })));
    assert!(!n.mesh.is_registered(&AgentId::new(&a1)));
    assert!(!sock(&a1).exists(), "its endpoint is gone");
    let (_, err) = m2.tool("lease_acquire", json!({"paths": ["src"], "ttl_s": 60}));
    assert!(!err, "the stopped agent's lease was released");

    // Mesh activity reaches the node's event log (the supervision tick moves it there, so wait for it).
    let end = Instant::now() + Duration::from_secs(15);
    loop {
        let events = n.events.events_after(0);
        let posted = events
            .iter()
            .any(|e| matches!(e.body, Some(v1::event::Body::MessagePosted(_))));
        let granted = events
            .iter()
            .any(|e| matches!(e.body, Some(v1::event::Body::LeaseGranted(_))));
        if posted && granted {
            break;
        }
        assert!(
            Instant::now() < end,
            "mesh events never reached the log (posted {posted}, granted {granted})"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn add_manifest(e: &Env, file: &str, toml: &str) {
    std::fs::write(e.dir.join("manifests").join(file), toml).unwrap();
}

fn worktree_of(r: &v1::AgentStartResult) -> PathBuf {
    PathBuf::from(&r.worktree_id)
}

fn git_status(dir: &Path) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["status", "--porcelain"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn a_manifest_with_mcp_gets_its_config_file_and_the_file_stays_out_of_git() {
    if !sh_on_path() {
        return;
    }
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let e = env("inject-file");
    add_manifest(
        &e,
        "mcp.toml",
        "kind = \"mcp-agent\"\ndisplay_name = \"M\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"sh\"]\n[mcp]\nmethod = \"config_file\"\npath = \".agora/mcp.json\"\n",
    );
    let n = node(&e, NodeLimits::default());
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let s = started(ok(c.call(start_cmd("mcp-agent"))));
    let cfg = std::fs::read_to_string(worktree_of(&s).join(".agora/mcp.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&cfg).unwrap();
    let args = v["mcpServers"]["agora-mesh"]["args"].as_array().unwrap();
    assert_eq!(&args[..3], &["mesh", "stdio", "--socket"]);
    let socket = PathBuf::from(args[3].as_str().unwrap());
    assert_eq!(
        socket,
        n.mesh.socket_path(&agora_core::AgentId::new(&s.agent_id))
    );
    assert!(
        socket.exists(),
        "the endpoint is already listening when the agent starts"
    );
    assert_eq!(
        git_status(&worktree_of(&s)),
        "",
        "the injected file is not the agent's work"
    );
    // An agent whose manifest has no [mcp] is still in the mesh, with nothing injected.
    let plain = started(ok(c.call(start_cmd("shell"))));
    assert!(!worktree_of(&plain).join(".agora").exists());
    assert!(n
        .mesh
        .is_registered(&agora_core::AgentId::new(&plain.agent_id)));
}

#[test]
fn a_symlink_in_the_repository_cannot_redirect_the_injected_file() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let e = env("inject-symlink");
    let victim = e.dir.join("victim.txt");
    std::fs::write(&victim, "precious").unwrap();
    // The repository itself contains `.mcp.json -> victim.txt` and a `cfg -> <dir>` directory link.
    if agora_platform::symlink(&victim, &e.repo.join(".mcp.json")).is_err() {
        return; // no symlinks here (Windows): nothing to prove
    }
    std::fs::create_dir_all(e.dir.join("elsewhere")).unwrap();
    agora_platform::symlink(&e.dir.join("elsewhere"), &e.repo.join("cfg")).unwrap();
    git(&e.repo, &["add", "."]);
    git(&e.repo, &["commit", "-q", "-m", "links"]);
    add_manifest(
        &e,
        "a.toml",
        "kind = \"file-agent\"\ndisplay_name = \"A\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"sh\"]\n[mcp]\nmethod = \"config_file\"\npath = \".mcp.json\"\n",
    );
    add_manifest(
        &e,
        "b.toml",
        "kind = \"dir-agent\"\ndisplay_name = \"B\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"sh\"]\n[mcp]\nmethod = \"config_file\"\npath = \"cfg/mcp.json\"\n",
    );
    let n = node(&e, NodeLimits::default());
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    // A symlinked file is replaced by a real one; the target is untouched.
    let s = started(ok(c.call(start_cmd("file-agent"))));
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "precious");
    let md = std::fs::symlink_metadata(worktree_of(&s).join(".mcp.json")).unwrap();
    assert!(md.is_file() && !md.file_type().is_symlink());
    // A symlinked directory on the path is refused: nothing is written through it, and the agent does not start.
    assert!(r_is_err(c.call(start_cmd("dir-agent"))));
    assert!(std::fs::read_dir(e.dir.join("elsewhere"))
        .unwrap()
        .next()
        .is_none());
}

fn r_is_err(r: std::io::Result<Result<v1::CommandOk, v1::ErrorInfo>>) -> bool {
    matches!(r, Ok(Err(_)))
}

#[test]
fn a_manifest_cannot_point_its_config_outside_the_worktree() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let e = env("inject-escape");
    add_manifest(
        &e,
        "evil.toml",
        "kind = \"evil\"\ndisplay_name = \"E\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"sh\"]\n[mcp]\nmethod = \"config_file\"\npath = \"../../escaped.json\"\n",
    );
    let n = node(&e, NodeLimits::default());
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    assert!(r_is_err(c.call(start_cmd("evil"))));
    assert!(!e.dir.join("escaped.json").exists());
    assert!(!e.dir.join("worktrees").join("escaped.json").exists());
}

#[test]
fn input_to_a_program_that_never_reads_does_not_wedge_the_node() {
    if !sh_on_path() {
        return;
    }
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let e = env("stalled-input");
    add_manifest(
        &e,
        "sleeper.toml",
        "kind = \"sleeper\"\ndisplay_name = \"S\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"sh\", \"-c\", \"stty raw -echo; sleep 60\"]\n",
    );
    let n = node(&e, NodeLimits::default());
    let socket = n.socket().unwrap().to_path_buf();
    let mut c = Client::connect(&socket).unwrap();
    let s = started(ok(c.call(start_cmd("sleeper"))));
    // The shell has put the terminal in raw mode by now (in cooked mode the excess is thrown away and nothing blocks).
    std::thread::sleep(Duration::from_millis(1000));
    // More than the terminal can queue for a program that is not reading (the old code held the supervisor lock for the whole
    // write and this test hung): this call stays stuck.
    let terminal = s.terminal_id.clone();
    let stuck = std::thread::spawn(move || {
        let mut c = Client::connect(&socket).unwrap();
        let _ = c.call(Cmd::TerminalInput(v1::TerminalInput {
            terminal_id: terminal,
            data: vec![b'x'; 512 * 1024],
        }));
    });
    std::thread::sleep(Duration::from_millis(1500));
    // Meanwhile the node answers, and the agent can be stopped.
    let t0 = Instant::now();
    ok(c.call(Cmd::AgentList(v1::AgentList::default())));
    let listed = t0.elapsed();
    ok(c.call(Cmd::AgentStop(v1::AgentStop {
        agent_id: s.agent_id.clone(),
        stop_helpers: false,
    })));
    // A node wedged behind the write answers only once the program's `sleep 60` ends, about a minute from here. Both calls take
    // well under a second on a quiet machine, but a busy Windows runner once took 5.5 s for them; 20 s still tells the two apart.
    assert!(
        t0.elapsed() < Duration::from_secs(20),
        "the node waited behind the stuck write: list {listed:?}, list and stop {:?}",
        t0.elapsed()
    );
    drop(stuck); // the writer ends when the process is gone
}

#[test]
fn a_start_that_fails_gives_back_its_worktree_and_its_mesh_endpoint() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let e = env("start-fails");
    add_manifest(
        &e,
        "ghost.toml",
        "kind = \"ghost\"\ndisplay_name = \"G\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"/no/such/program-agora\"]\n",
    );
    add_manifest(
        &e,
        "evil.toml",
        "kind = \"evil\"\ndisplay_name = \"E\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"sh\"]\n[mcp]\nmethod = \"config_file\"\npath = \"../../escaped.json\"\n",
    );
    let n = node(&e, NodeLimits::default());
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    for kind in ["ghost", "evil", "ghost"] {
        assert!(r_is_err(c.call(start_cmd(kind))), "{kind}");
    }
    let left = std::fs::read_dir(e.dir.join("worktrees")).map_or(0, |d| d.count());
    assert_eq!(left, 0, "failed starts left worktrees behind");
    for id in ["agent-1", "agent-2", "agent-3"] {
        assert!(
            !n.mesh.is_registered(&agora_core::AgentId::new(id)),
            "{id} still has a mesh endpoint"
        );
    }
}

#[test]
fn flags_and_variables_are_added_at_every_launch_including_restarts() {
    if !sh_on_path() {
        return;
    }
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let e = env("inject-flags");
    // The "agent" records its arguments and environment, then waits.
    add_manifest(
        &e,
        "rec.toml",
        "kind = \"rec\"\ndisplay_name = \"R\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"sh\", \"-c\", \"echo \\\"$@\\\" >> launches.txt; exit 1\", \"x\"]\n[mcp]\nmethod = \"cli_flag\"\npath = \".agora/mcp.json\"\nflag = [\"--mcp-config\", \"{config_file}\"]\n",
    );
    add_manifest(
        &e,
        "env.toml",
        "kind = \"envrec\"\ndisplay_name = \"V\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"sh\", \"-c\", \"echo $AGORA_MCP_SOCKET > socks.txt; sleep 60\"]\n[mcp]\nmethod = \"env\"\n[mcp.env]\nAGORA_MCP_SOCKET = \"{socket}\"\n",
    );
    let n = node(&e, NodeLimits::default());
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let wait_file = |p: PathBuf| {
        let end = Instant::now() + Duration::from_secs(15);
        loop {
            if let Ok(s) = std::fs::read_to_string(&p) {
                if !s.trim().is_empty() {
                    return s;
                }
            }
            assert!(Instant::now() < end, "{} never appeared", p.display());
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    let a = started(ok(c.call(start_cmd("rec"))));
    // The agent exits with an error right away, so the supervisor restarts it; every launch gets the flag.
    let expected = format!(
        "--mcp-config {}",
        worktree_of(&a).join(".agora/mcp.json").display()
    );
    let end = Instant::now() + Duration::from_secs(20);
    let launches = loop {
        let text =
            std::fs::read_to_string(worktree_of(&a).join("launches.txt")).unwrap_or_default();
        if text.lines().count() >= 2 {
            break text;
        }
        assert!(Instant::now() < end, "no restart seen: {text:?}");
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(launches.lines().all(|l| l == expected), "{launches:?}");
    let b = started(ok(c.call(start_cmd("envrec"))));
    let sock = wait_file(worktree_of(&b).join("socks.txt"));
    assert_eq!(
        sock.trim(),
        n.mesh
            .socket_path(&agora_core::AgentId::new(&b.agent_id))
            .to_string_lossy()
    );
}

/// A verifier for tests: a device "signs" by sending the message bytes themselves. The real scheme is a separate
/// decision; this exercises the node's whole approval path.
struct EchoVerifier;

impl agora_node::approvals::Verifier for EchoVerifier {
    fn verify(&self, device: &str, message: &[u8], signature: &[u8]) -> bool {
        device == "phone" && message == signature
    }
}

fn fake_adapter_path() -> String {
    let mut p = std::env::current_exe().unwrap();
    p.pop();
    p.pop();
    p.push(format!("agora-fake-agent{}", std::env::consts::EXE_SUFFIX));
    // Built once per test binary, even when present: a binary left from an older checkout may lack newer flags.
    static FRESH: std::sync::Once = std::sync::Once::new();
    FRESH.call_once(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        assert!(Command::new(cargo)
            .args(["build", "-q", "-p", "agora-fake-agent"])
            .status()
            .unwrap()
            .success());
    });
    p.to_string_lossy().into_owned()
}

#[test]
fn an_adapter_approval_waits_for_a_signed_decision_and_is_relayed() {
    use agora_node::approvals::signed_message;
    use v1::event::Body;
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let e = env("approvals");
    add_manifest(
        &e,
        "ad.toml",
        &format!("kind = \"adapted\"\ndisplay_name = \"A\"\nversion = \"1\"\nmin_engine = 1\n[adapter]\ncommand = [{:?}, \"--adapter\"]\n", fake_adapter_path()),
    );
    let n = node(&e, NodeLimits::default());
    n.approvals.set_verifier(Box::new(EchoVerifier));
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let s = started(ok(c.call(start_cmd("adapted"))));
    assert_eq!(s.terminal_id, "", "adapter agents have no terminal");

    let wait_request = |n: &Node| -> String {
        let end = Instant::now() + Duration::from_secs(15);
        loop {
            let found = n
                .events
                .events_after(0)
                .into_iter()
                .rev()
                .find_map(|e| match e.body {
                    Some(Body::ApprovalRequested(a)) => Some(a),
                    _ => None,
                });
            if let Some(a) = found {
                return a.approval_id;
            }
            assert!(Instant::now() < end, "no approval request announced");
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    let resolve = |c: &mut Client, id: &str, approved: bool, device: &str, node: &str| {
        c.call(Cmd::ApprovalResolve(v1::ApprovalResolve {
            approval_id: id.into(),
            approved,
            signer_device_id: device.into(),
            signature: signed_message(node, id, approved),
            helper_grant: 0,
            helpers_may_start: false,
            ..Default::default()
        }))
    };
    let state_of = |n: &Node, id: &str| {
        n.supervisor
            .lock()
            .unwrap()
            .list()
            .into_iter()
            .find(|v| v.id.as_str() == id)
            .unwrap()
            .state
    };

    // The adapter asks: the node announces a request under its own random id and waits.
    ok(c.call(Cmd::AgentPrompt(v1::AgentPrompt {
        agent_id: s.agent_id.clone(),
        text: "ask".into(),
    })));
    let id = wait_request(&n);
    assert!(
        id.starts_with("ap-") && id != "ap-1",
        "the node's id, not the adapter's: {id}"
    );
    assert_eq!(n.approvals.pending_count(), 1);

    // A decision from an unknown device or for another node is refused; the request keeps waiting.
    assert_eq!(
        err_code(resolve(&mut c, &id, true, "stranger", "node-test")),
        "approval.signature_invalid"
    );
    assert_eq!(
        err_code(resolve(&mut c, &id, true, "phone", "other-node")),
        "approval.signature_invalid"
    );
    assert_eq!(n.approvals.pending_count(), 1);
    assert_eq!(state_of(&n, &s.agent_id), agora_core::AgentState::Blocked);

    // A signed decision is verified by the node and relayed to the adapter, which carries on.
    ok(resolve(&mut c, &id, true, "phone", "node-test"));
    let end = Instant::now() + Duration::from_secs(15);
    while state_of(&n, &s.agent_id) != agora_core::AgentState::Idle {
        assert!(Instant::now() < end, "agent never resumed");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(n.approvals.pending_count(), 0);
    // Spent: replaying the same decision is refused.
    assert!(r_is_err(resolve(&mut c, &id, true, "phone", "node-test")));
    let resolved = n
        .events
        .events_after(0)
        .into_iter()
        .find_map(|e| match e.body {
            Some(Body::ApprovalResolved(r)) => Some(r),
            _ => None,
        });
    let r = resolved.expect("resolution recorded");
    assert_eq!(
        (
            r.approval_id.as_str(),
            r.approved,
            r.signer_device_id.as_str()
        ),
        (id.as_str(), true, "phone")
    );
    // Usage and session reports from the adapter reached the event log too.
    ok(c.call(Cmd::AgentPrompt(v1::AgentPrompt {
        agent_id: s.agent_id.clone(),
        text: "hello".into(),
    })));
    let end = Instant::now() + Duration::from_secs(15);
    loop {
        let evs = n.events.events_after(0);
        let usage = evs
            .iter()
            .any(|e| matches!(e.body, Some(Body::AgentUsageReported(_))));
        let session = evs
            .iter()
            .any(|e| matches!(e.body, Some(Body::AgentSessionReported(_))));
        if usage && session {
            break;
        }
        assert!(
            Instant::now() < end,
            "adapter reports never reached the log"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// ADR-0021: an adapter's usage report names the price catalog's model when the kind's `[usage.model_map]` lists the model it
/// reported; the hub then prices it by that id.
#[test]
fn an_adapters_usage_names_the_catalog_model_from_the_model_map() {
    use v1::event::Body;
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    for (tag, map, want) in [
        (
            "model-map",
            "[usage.model_map]\n\"fake/model\" = \"example/model-2026\"\n",
            "example/model-2026",
        ),
        // Negative control: without a map the agent's own id is reported.
        ("model-map-none", "", "fake/model"),
    ] {
        let e = env(tag);
        add_manifest(
            &e,
            "ad.toml",
            &format!("kind = \"adapted\"\ndisplay_name = \"A\"\nversion = \"1\"\nmin_engine = 1\n[adapter]\ncommand = [{:?}, \"--adapter\"]\n{map}", fake_adapter_path()),
        );
        let n = node(&e, NodeLimits::default());
        let mut c = Client::connect(n.socket().unwrap()).unwrap();
        let s = started(ok(c.call(start_cmd("adapted"))));
        ok(c.call(Cmd::AgentPrompt(v1::AgentPrompt {
            agent_id: s.agent_id.clone(),
            text: "hello".into(),
        })));
        let end = Instant::now() + Duration::from_secs(15);
        let model = loop {
            let found = n
                .events
                .events_after(0)
                .into_iter()
                .find_map(|e| match e.body {
                    Some(Body::AgentUsageReported(u)) => Some(u.model),
                    _ => None,
                });
            if let Some(m) = found {
                break m;
            }
            assert!(Instant::now() < end, "no usage reported");
            std::thread::sleep(Duration::from_millis(50));
        };
        assert_eq!(model, want);
    }
}

#[test]
fn with_no_trusted_device_every_decision_is_refused() {
    use agora_node::approvals::signed_message;
    use v1::event::Body;
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let e = env("approvals-deny");
    add_manifest(
        &e,
        "ad.toml",
        &format!("kind = \"adapted\"\ndisplay_name = \"A\"\nversion = \"1\"\nmin_engine = 1\n[adapter]\ncommand = [{:?}, \"--adapter\"]\n", fake_adapter_path()),
    );
    let n = node(&e, NodeLimits::default());
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let s = started(ok(c.call(start_cmd("adapted"))));
    ok(c.call(Cmd::AgentPrompt(v1::AgentPrompt {
        agent_id: s.agent_id,
        text: "ask".into(),
    })));
    let end = Instant::now() + Duration::from_secs(15);
    let id = loop {
        if let Some(a) = n
            .events
            .events_after(0)
            .into_iter()
            .find_map(|e| match e.body {
                Some(Body::ApprovalRequested(a)) => Some(a),
                _ => None,
            })
        {
            break a.approval_id;
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(50));
    };
    let r = c.call(Cmd::ApprovalResolve(v1::ApprovalResolve {
        approval_id: id.clone(),
        approved: true,
        signer_device_id: "phone".into(),
        signature: signed_message("node-test", &id, true),
        helper_grant: 0,
        helpers_may_start: false,
        ..Default::default()
    }));
    assert_eq!(err_code(r), "approval.signature_invalid");
}

#[test]
fn a_manifest_added_while_the_node_runs_is_listed_and_startable_without_a_restart() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let e = env("reload");
    let n = node(&e, NodeLimits::default());
    let mut c = Client::connect(&e.dir.join("c.sock")).unwrap();
    let kinds = |c: &mut Client| -> Vec<String> {
        let r = ok(c.call(Cmd::AgentKinds(v1::AgentKinds {})));
        let Some(Ok_::AgentKinds(list)) = r.result else {
            panic!("not an AgentKinds result");
        };
        list.kinds.into_iter().map(|k| k.kind).collect()
    };
    assert!(!kinds(&mut c).contains(&"late".to_string()));
    std::fs::write(
        e.dir.join("manifests/late.toml"),
        "kind = \"late\"\ndisplay_name = \"Late\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"sh\"]\n",
    )
    .unwrap();
    assert!(kinds(&mut c).contains(&"late".to_string()));
    let started = c.call(start_cmd("late"));
    assert!(started.is_ok(), "{started:?}");
    drop(n);
    let _ = std::fs::remove_dir_all(&e.dir);
}

#[test]
fn agent_kinds_lists_what_this_node_can_start() {
    if !sh_on_path() {
        return;
    }
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let e = env("kinds");
    // A manifest whose program does not exist is listed as not installed.
    std::fs::write(
        e.dir.join("manifests/ghost.toml"),
        "kind = \"ghost\"\ndisplay_name = \"Ghost\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"definitely-not-installed-xyz\"]\n",
    )
    .unwrap();
    // ADR-0069: a manifest that names its program by path still reports only the file name.
    std::fs::write(
        e.dir.join("manifests/pathed.toml"),
        "kind = \"pathed\"\ndisplay_name = \"Pathed\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"/nowhere/at/all/wstack\"]\n",
    )
    .unwrap();
    let n = node(&e, NodeLimits::default());
    let mut c = Client::connect(&e.dir.join("c.sock")).unwrap();
    let r = ok(c.call(Cmd::AgentKinds(v1::AgentKinds {})));
    let Some(Ok_::AgentKinds(list)) = r.result else {
        panic!("not an AgentKinds result");
    };
    let by: std::collections::BTreeMap<_, _> = list
        .kinds
        .iter()
        .map(|k| {
            (
                k.kind.as_str(),
                (k.display_name.as_str(), k.installed, k.level),
            )
        })
        .collect();
    assert_eq!(by["shell"].0, "Shell");
    assert!(by["shell"].1, "sh is on PATH");
    assert!(!by["ghost"].1);
    assert_eq!(by["shell"].2, v1::IntegrationLevel::L1Manifest as i32);
    assert!(!by.contains_key("broken"), "invalid manifests are omitted");
    // ADR-0069: the program each kind looks for, a bare file name, so a missing one can say what to install.
    let program: std::collections::BTreeMap<_, _> = list
        .kinds
        .iter()
        .map(|k| (k.kind.as_str(), k.program.as_str()))
        .collect();
    assert_eq!(program["shell"], "sh");
    assert_eq!(program["ghost"], "definitely-not-installed-xyz");
    assert_eq!(program["pathed"], "wstack");
    assert!(!by["pathed"].1);
    drop(n);
}

#[test]
fn a_node_with_an_empty_manifests_folder_offers_and_starts_every_built_in_kind_and_a_folder_file_overrides_one(
) {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let e = env("builtin");
    let empty = e.dir.join("empty-manifests");
    std::fs::create_dir_all(&empty).unwrap();
    let n = Node::start(&NodeConfig {
        resume_all: false,
        node_id: "node-test".into(),
        socket: e.dir.join("c.sock"),
        data_dir: e.dir.join("data"),
        worktree_root: e.dir.join("worktrees"),
        manifests_dir: empty.clone(),
        projects: vec![e.repo.clone()],
        limits: NodeLimits::default(),
    })
    .expect("start node");
    assert!(n.manifest_problems.is_empty());
    let mut c = Client::connect(&e.dir.join("c.sock")).unwrap();
    let kinds = |c: &mut Client| -> std::collections::BTreeMap<String, String> {
        let r = ok(c.call(Cmd::AgentKinds(v1::AgentKinds {})));
        let Some(Ok_::AgentKinds(list)) = r.result else {
            panic!("not an AgentKinds result");
        };
        list.kinds
            .into_iter()
            .map(|k| (k.kind, k.display_name))
            .collect()
    };
    // ADR-0092: the ACP registry's converted agents are offered too, each under `registry-<id>`.
    let shipped: Vec<String> = agora_node::manifests::BUILT_IN
        .iter()
        .chain(agora_node::manifests::REGISTRY)
        .map(|(_, text)| {
            agora_adapters::Manifest::parse(text)
                .expect("shipped manifest")
                .kind
        })
        .collect();
    let listed = kinds(&mut c);
    assert_eq!(
        listed.keys().cloned().collect::<Vec<_>>(),
        {
            let mut s = shipped.clone();
            s.push(agora_node::manifests::SHELL_KIND.to_string());
            s.sort();
            s
        },
        "every shipped kind, the registry's and the built-in shell, nothing else"
    );
    assert_eq!(listed["shell"], "Shell");
    assert!(listed.keys().filter(|k| k.starts_with("registry-")).count() >= 40);
    // With no file at all, the user's shell starts.
    let started = c.call(start_cmd("shell"));
    assert!(started.is_ok(), "{started:?}");
    // What the node looks for is the program's name, for the "not installed" line.
    let programs: std::collections::BTreeMap<_, _> = n
        .supervisor
        .lock()
        .unwrap()
        .kinds()
        .into_iter()
        .map(|k| (k.kind, k.program))
        .collect();
    assert_eq!(programs["wrongstack"], "wstack");
    // A folder file of the same kind wins, a broken one leaves the built-in kind in place; both without a restart.
    std::fs::write(
        empty.join("codex.toml"),
        "kind = \"codex\"\ndisplay_name = \"Codex here\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"sh\"]\n",
    )
    .unwrap();
    std::fs::write(empty.join("wrongstack.toml"), "kind = \"wrongstack\"\n[").unwrap();
    let listed = kinds(&mut c);
    assert_eq!(listed["codex"], "Codex here");
    assert_eq!(listed["wrongstack"], "WrongStack");
    let started = c.call(start_cmd("codex"));
    assert!(started.is_ok(), "{started:?}");
    drop(n);
    let _ = std::fs::remove_dir_all(&e.dir);
}

#[test]
fn uncommitted_changes_in_an_agents_workspace_are_reported() {
    if !sh_on_path() {
        return;
    }
    use v1::event::Body;
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let e = env("gitwatch");
    let n = node(&e, NodeLimits::default());
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let s = started(ok(c.call(start_cmd("shell"))));
    std::fs::write(Path::new(&s.worktree_id).join("new.txt"), "x").unwrap();
    std::fs::write(Path::new(&s.worktree_id).join("other.txt"), "y").unwrap();

    // The watcher checks every few seconds; the latest report for this workspace must show both new files. One check is six git
    // processes (each command with its filter question): 2-6 s on an idle Windows machine and up to about 50 s while this whole
    // file runs in parallel there (measured), so the wait is generous.
    let end = Instant::now() + Duration::from_secs(120);
    loop {
        let latest = n
            .events
            .events_after(0)
            .into_iter()
            .filter_map(|ev| match ev.body {
                Some(Body::GitStatusChanged(g)) if g.worktree_id == s.worktree_id => Some(g),
                _ => None,
            })
            .next_back();
        if latest.as_ref().is_some_and(|g| g.files_changed == 2) {
            break;
        }
        assert!(
            Instant::now() < end,
            "no report of the two changed files: {latest:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    ok(c.call(Cmd::AgentStop(v1::AgentStop {
        agent_id: s.agent_id,
        stop_helpers: false,
    })));
}

#[test]
fn git_status_and_diff_cover_new_and_changed_files_and_only_agent_workspaces() {
    if !sh_on_path() {
        return;
    }
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let e = env("gitdiff");
    let n = node(&e, NodeLimits::default());
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let s = started(ok(c.call(start_cmd("shell"))));
    let wt = Path::new(&s.worktree_id);
    std::fs::write(wt.join("a.txt"), "changed\n").unwrap();
    std::fs::write(wt.join("fresh.txt"), "brand new\n").unwrap();

    let Some(Ok_::GitStatus(st)) = ok(c.call(Cmd::GitStatus(v1::GitStatus {
        worktree_id: s.worktree_id.clone(),
    })))
    .result
    else {
        panic!("no status");
    };
    assert_eq!(st.files_changed, 2);
    let Some(Ok_::GitDiff(d)) = ok(c.call(Cmd::GitDiff(v1::GitDiff {
        worktree_id: s.worktree_id.clone(),
        path: String::new(),
    })))
    .result
    else {
        panic!("no diff");
    };
    assert!(
        d.diff.contains("+changed") && d.diff.contains("+brand new"),
        "{}",
        d.diff
    );
    assert!(d.diff.contains("fresh.txt"), "untracked file is named");

    // Not an agent's workspace, and a path that climbs out: refused.
    let other = e.repo.to_string_lossy().into_owned();
    assert_eq!(
        err_code(c.call(Cmd::GitDiff(v1::GitDiff {
            worktree_id: other,
            path: String::new()
        }))),
        "protocol.invalid_message"
    );
    for bad in ["../x", "C:\\w\\secret", "\\\\server\\share", ".git/config"] {
        assert_eq!(
            err_code(c.call(Cmd::GitDiff(v1::GitDiff {
                worktree_id: s.worktree_id.clone(),
                path: bad.into()
            }))),
            "protocol.invalid_message",
            "{bad}"
        );
    }
    ok(c.call(Cmd::AgentStop(v1::AgentStop {
        agent_id: s.agent_id,
        stop_helpers: false,
    })));
}

#[test]
fn files_list_and_read_stay_inside_an_agents_workspace() {
    if !sh_on_path() {
        return;
    }
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let e = env("filesview");
    let n = node(&e, NodeLimits::default());
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let s = started(ok(c.call(start_cmd("shell"))));
    let wt = Path::new(&s.worktree_id);
    std::fs::create_dir_all(wt.join("src")).unwrap();
    std::fs::write(wt.join("src/lib.rs"), "pub fn x() {}\n").unwrap();

    let Some(Ok_::FilesList(l)) = ok(c.call(Cmd::FilesList(v1::FilesList {
        worktree_id: s.worktree_id.clone(),
        path: String::new(),
    })))
    .result
    else {
        panic!("no list");
    };
    let names: Vec<(&str, bool)> = l
        .entries
        .iter()
        .map(|e| (e.name.as_str(), e.is_dir))
        .collect();
    assert!(
        names.contains(&("src", true)) && names.contains(&("a.txt", false)),
        "{names:?}"
    );
    assert!(
        !names.iter().any(|(n, _)| *n == ".git"),
        "git's own files are hidden: {names:?}"
    );

    let Some(Ok_::FilesRead(r)) = ok(c.call(Cmd::FilesRead(v1::FilesRead {
        worktree_id: s.worktree_id.clone(),
        path: "src/lib.rs".into(),
    })))
    .result
    else {
        panic!("no read");
    };
    assert_eq!(r.data, b"pub fn x() {}\n");
    assert_eq!(
        err_code(c.call(Cmd::FilesRead(v1::FilesRead {
            worktree_id: s.worktree_id.clone(),
            path: "../../etc/passwd".into()
        }))),
        "files.outside_root"
    );
    // Git's own directory is not project content, whether listed or read directly.
    for p in [".git", ".git/config", "sub/.git"] {
        assert_eq!(
            err_code(c.call(Cmd::FilesRead(v1::FilesRead {
                worktree_id: s.worktree_id.clone(),
                path: p.into()
            }))),
            "protocol.invalid_message",
            "{p}"
        );
        assert_eq!(
            err_code(c.call(Cmd::FilesList(v1::FilesList {
                worktree_id: s.worktree_id.clone(),
                path: p.into()
            }))),
            "protocol.invalid_message",
            "{p}"
        );
    }
    assert_eq!(
        err_code(c.call(Cmd::FilesList(v1::FilesList {
            worktree_id: e.repo.to_string_lossy().into_owned(),
            path: String::new()
        }))),
        "protocol.invalid_message"
    );
    ok(c.call(Cmd::AgentStop(v1::AgentStop {
        agent_id: s.agent_id,
        stop_helpers: false,
    })));
}

#[test]
fn git_log_lists_the_workspaces_commits_newest_first() {
    if !sh_on_path() {
        return;
    }
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let e = env("gitlog");
    let n = node(&e, NodeLimits::default());
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let s = started(ok(c.call(start_cmd("shell"))));
    let wt = Path::new(&s.worktree_id);
    std::fs::write(wt.join("b.txt"), "b").unwrap();
    git(wt, &["add", "."]);
    git(wt, &["commit", "-q", "-m", "add b: the second commit"]);

    let mut log = |limit: u32| {
        let Some(Ok_::GitLog(l)) = ok(c.call(Cmd::GitLog(v1::GitLog {
            worktree_id: s.worktree_id.clone(),
            limit,
        })))
        .result
        else {
            panic!("no log");
        };
        l.commits
    };
    let all = log(0);
    assert_eq!(all.len(), 2, "{all:?}");
    assert_eq!(all[0].subject, "add b: the second commit");
    assert_eq!(all[1].subject, "init");
    assert_eq!(all[0].author, "t");
    assert!(
        all[0].at_ms > 1_600_000_000_000 && all[0].id.len() >= 40,
        "{:?}",
        all[0]
    );
    assert_eq!(log(1).len(), 1);
    assert_eq!(
        err_code(c.call(Cmd::GitLog(v1::GitLog {
            worktree_id: e.repo.to_string_lossy().into_owned(),
            limit: 0
        }))),
        "protocol.invalid_message"
    );
    ok(c.call(Cmd::AgentStop(v1::AgentStop {
        agent_id: s.agent_id,
        stop_helpers: false,
    })));
}

fn fake_agent_path() -> String {
    let mut p = std::env::current_exe().expect("exe");
    p.pop();
    p.pop();
    p.push(format!("agora-fake-agent{}", std::env::consts::EXE_SUFFIX));
    // Built once per test binary, even when present: a binary left from an older checkout may lack newer flags.
    static FRESH: std::sync::Once = std::sync::Once::new();
    FRESH.call_once(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        assert!(
            Command::new(cargo)
                .args(["build", "-q", "-p", "agora-fake-agent"])
                .status()
                .expect("run cargo")
                .success(),
            "building agora-fake-agent failed"
        );
    });
    p.to_string_lossy().into_owned()
}

#[test]
fn a_chat_agents_conversation_streams_to_whoever_subscribes() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let e = env("chat");
    std::fs::write(
        e.dir.join("manifests/fake.toml"),
        format!(
            "kind = \"fake-adapter\"\ndisplay_name = \"F\"\nversion = \"1\"\nmin_engine = 1\n[adapter]\ncommand = [{:?}, \"--adapter\"]\n",
            fake_agent_path()
        ),
    )
    .unwrap();
    let n = node(&e, NodeLimits::default());
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let s = started(ok(c.call(start_cmd("fake-adapter"))));
    let prompt = |c: &mut Client, text: &str| {
        ok(c.call(Cmd::AgentPrompt(v1::AgentPrompt {
            agent_id: s.agent_id.clone(),
            text: text.into(),
        })));
    };
    // A request sent before anyone subscribes is in the snapshot.
    std::thread::sleep(Duration::from_millis(800)); // the adapter finishes its handshake
    prompt(&mut c, "first question");

    ok(c.call_on(
        3,
        Cmd::ChatSubscribe(v1::ChatSubscribe {
            agent_id: s.agent_id.clone(),
        }),
    ));
    let snap = wait_frame(&mut c, 3, |f| f.kind == KIND_CHAT_SNAPSHOT);
    let snap = <v1::ChatSnapshot as prost::Message>::decode(snap.payload.as_slice()).unwrap();
    assert!(
        snap.entries
            .iter()
            .any(|e| e.text == "first question" && e.role == v1::ChatRole::User as i32),
        "{snap:?}"
    );

    // A later one arrives live, as the user's request and as the agent's answer (the fake agent echoes it).
    prompt(&mut c, "second question");
    let mut got = Vec::new();
    wait_frame(&mut c, 3, |f| {
        if f.kind == KIND_CHAT_ENTRY {
            got.push(<v1::ChatEntry as prost::Message>::decode(f.payload.as_slice()).unwrap());
        }
        got.iter()
            .any(|e| e.role == v1::ChatRole::Agent as i32 && e.text == "second question")
    });
    assert!(
        got.iter()
            .any(|e| e.role == v1::ChatRole::User as i32 && e.text == "second question"),
        "{got:?}"
    );
    let ids: Vec<u64> = got.iter().map(|e| e.entry_id).collect();
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "ids increase: {ids:?}");

    assert_eq!(
        err_code(c.call_on(
            4,
            Cmd::ChatSubscribe(v1::ChatSubscribe {
                agent_id: "agent-nope".into()
            })
        )),
        "agent.not_found"
    );
    ok(c.call(Cmd::AgentStop(v1::AgentStop {
        agent_id: s.agent_id,
        stop_helpers: false,
    })));
}

#[test]
fn the_node_lists_a_folders_subfolders_for_the_folder_picker() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let e = env("dirlist");
    // Outside the node's own folder (the one its worktrees are in), which is never listed.
    let pick = std::env::temp_dir().join(format!("agora-pick-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&pick);
    std::fs::create_dir_all(pick.join("zeta")).unwrap();
    std::fs::create_dir_all(pick.join("alpha/.git")).unwrap();
    std::fs::write(pick.join("file.txt"), "x").unwrap();
    let n = node(&e, NodeLimits::default());
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let r = ok(c.call(Cmd::DirectoryList(v1::DirectoryList {
        path: pick.to_string_lossy().into_owned(),
        authorization: None,
    })));
    let Some(Ok_::DirectoryEntries(list)) = r.result else {
        panic!("not a folder listing: {r:?}");
    };
    let names: Vec<(&str, bool)> = list
        .entries
        .iter()
        .map(|x| (x.name.as_str(), x.is_git))
        .collect();
    assert_eq!(names, [("alpha", true), ("zeta", false)]);
    assert!(!list.truncated);
    assert!(
        list.path
            .ends_with(&*pick.file_name().unwrap().to_string_lossy()),
        "{}",
        list.path
    );
    // The node's own folder holds its keys: never listed, and refused when asked for.
    assert_eq!(
        err_code(c.call(Cmd::DirectoryList(v1::DirectoryList {
            path: e.dir.join("data").to_string_lossy().into_owned(),
            authorization: None,
        }))),
        "protocol.invalid_message"
    );
    let _ = std::fs::remove_dir_all(&pick);
}

fn diff_of(c: &mut Client, wt: &str) -> v1::GitDiffResult {
    let Some(Ok_::GitDiff(d)) = ok(c.call(Cmd::GitDiff(v1::GitDiff {
        worktree_id: wt.into(),
        path: String::new(),
    })))
    .result
    else {
        panic!("no diff");
    };
    d
}

fn list_of(c: &mut Client, wt: &str) -> Vec<(String, bool)> {
    let Some(Ok_::FilesList(l)) = ok(c.call(Cmd::FilesList(v1::FilesList {
        worktree_id: wt.into(),
        path: String::new(),
    })))
    .result
    else {
        panic!("no list");
    };
    l.entries.into_iter().map(|e| (e.name, e.secret)).collect()
}

fn read_cmd(wt: &str, path: &str) -> Cmd {
    Cmd::FilesRead(v1::FilesRead {
        worktree_id: wt.into(),
        path: path.into(),
    })
}

/// A started agent whose workspace holds a changed tracked `.env.local`, an untracked `.env` and `id_rsa`, and ordinary files.
fn secret_workspace(name: &str, policy: Option<&str>) -> (Env, Node, String, String) {
    let e = env(name);
    if let Some(p) = policy {
        std::fs::create_dir_all(e.dir.join("data")).unwrap();
        std::fs::write(e.dir.join("data/policy"), p).unwrap();
    }
    let n = node(&e, NodeLimits::default());
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let s = started(ok(c.call(start_cmd("shell"))));
    let wt = PathBuf::from(&s.worktree_id);
    std::fs::write(wt.join(".env.local"), "OLD=1\n").unwrap();
    git(&wt, &["add", "-f", ".env.local"]);
    git(&wt, &["commit", "-q", "-m", "add env"]);
    std::fs::write(wt.join(".env.local"), "OLD=1\nSECRETVALUE=2\n").unwrap();
    std::fs::write(wt.join(".env"), "TOKEN=hunter2\nB=2\n").unwrap();
    std::fs::write(wt.join("id_rsa"), "PRIVATEKEYBYTES\n").unwrap();
    std::fs::write(wt.join("plain.txt"), "plain line\n").unwrap();
    (e, n, s.worktree_id, s.agent_id)
}

#[test]
fn secret_files_are_marked_refused_and_left_out_of_the_diff() {
    if !sh_on_path() {
        return;
    }
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let (_e, n, wt, agent) = secret_workspace("secret-off", None);
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let list = list_of(&mut c, &wt);
    for (name, secret) in [
        (".env", true),
        (".env.local", true),
        ("id_rsa", true),
        ("plain.txt", false),
        ("a.txt", false),
    ] {
        assert!(
            list.contains(&(name.to_string(), secret)),
            "{name}: {list:?}"
        );
    }
    assert_eq!(err_code(c.call(read_cmd(&wt, ".env"))), "files.secret");
    assert_eq!(err_code(c.call(read_cmd(&wt, "id_rsa"))), "files.secret");
    // Negative control: an ordinary file still reads.
    ok(c.call(read_cmd(&wt, "plain.txt")));
    // A link to a secret file is refused by where it leads.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(Path::new(&wt).join(".env"), Path::new(&wt).join("link.txt"))
            .unwrap();
        assert_eq!(err_code(c.call(read_cmd(&wt, "link.txt"))), "files.secret");
    }

    let d = diff_of(&mut c, &wt);
    for leaked in ["hunter2", "SECRETVALUE", "PRIVATEKEYBYTES", "OLD=1"] {
        assert!(!d.diff.contains(leaked), "{leaked} in {}", d.diff);
    }
    assert!(d.diff.contains("plain line"), "{}", d.diff);
    let by = |p: &str| {
        d.secret_files
            .iter()
            .find(|s| s.path == p)
            .map(|s| (s.added, s.removed))
    };
    assert_eq!(by(".env.local"), Some((1, 0)), "{d:?}");
    assert_eq!(by(".env"), Some((2, 0)), "{d:?}");
    assert_eq!(by("id_rsa"), Some((1, 0)), "{d:?}");
    assert!(by("plain.txt").is_none());
    for p in [".env.local", ".env", "id_rsa"] {
        assert!(d.diff.contains(&format!("diff --git a/{p} b/{p}")), "{p}");
    }
    ok(c.call(Cmd::AgentStop(v1::AgentStop {
        agent_id: agent,
        stop_helpers: false,
    })));
}

#[test]
fn the_owners_policy_switch_shows_secret_files() {
    if !sh_on_path() {
        return;
    }
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let (_e, n, wt, agent) = secret_workspace("secret-on", Some("files.show_secret = true\n"));
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    assert!(list_of(&mut c, &wt).iter().all(|(_, secret)| !secret));
    let Some(Ok_::FilesRead(r)) = ok(c.call(read_cmd(&wt, ".env"))).result else {
        panic!("no read");
    };
    assert_eq!(r.data, b"TOKEN=hunter2\nB=2\n");
    let d = diff_of(&mut c, &wt);
    assert!(d.secret_files.is_empty());
    assert!(d.diff.contains("hunter2") && d.diff.contains("SECRETVALUE"));
    ok(c.call(Cmd::AgentStop(v1::AgentStop {
        agent_id: agent,
        stop_helpers: false,
    })));
}

/// F251: a subscriber that fell behind (it stopped reading while the agent printed far more than the node queues for it) still
/// hears that the terminal ended, even though the exit was among what was dropped.
#[test]
fn a_subscriber_that_fell_behind_still_gets_the_exit_frame() {
    if !sh_on_path() {
        return;
    }
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    const OUTPUT: usize = 32 * 1024 * 1024;
    let e = env("lagexit");
    let n = node(&e, NodeLimits::default());
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let s = started(ok(c.call(start_cmd("shell"))));
    ok(c.call_on(
        3,
        Cmd::TerminalSubscribe(v1::TerminalSubscribe {
            terminal_id: s.terminal_id.clone(),
            format: "terminal.raw".into(),
            history_lines: 0,
        }),
    ));
    ok(c.call(Cmd::TerminalInput(v1::TerminalInput {
        terminal_id: s.terminal_id.clone(),
        data: format!("head -c {OUTPUT} /dev/zero | tr '\\0' x; exit 0\n").into_bytes(),
    })));
    // Not reading the stream until the agent has ended.
    let id = agora_core::AgentId::new(&s.agent_id);
    let end = Instant::now() + Duration::from_secs(180);
    while n
        .supervisor
        .lock()
        .unwrap()
        .list()
        .iter()
        .any(|v| v.id == id && v.running)
    {
        assert!(Instant::now() < end, "the agent never ended");
        std::thread::sleep(Duration::from_millis(50));
    }
    let mut data = 0;
    let exit = wait_frame(&mut c, 3, |f| {
        if f.kind == KIND_TERM_DATA {
            data += f.payload.len();
        }
        f.kind == KIND_TERM_EXIT
    });
    assert!(data < OUTPUT, "the subscriber never fell behind");
    assert_eq!(exit.payload.len(), 4);
}

/// ADR-0090: a permission request also shows in the conversation, as an entry that points at the node's own approval id, and its
/// outcome follows only the node's approval state: the user's signed decision (approved, denied) or the agent leaving (cancelled).
#[test]
fn a_permission_shows_in_the_conversation_and_follows_only_the_nodes_approval_state() {
    use agora_node::approvals::signed_message;
    use v1::event::Body;
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let e = env("permission-entry");
    add_manifest(
        &e,
        "ad.toml",
        &format!("kind = \"adapted\"\ndisplay_name = \"A\"\nversion = \"1\"\nmin_engine = 1\n[adapter]\ncommand = [{:?}, \"--adapter\"]\n", fake_adapter_path()),
    );
    let n = node(&e, NodeLimits::default());
    n.approvals.set_verifier(Box::new(EchoVerifier));
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let s = started(ok(c.call(start_cmd("adapted"))));
    let agent = agora_core::AgentId::new(s.agent_id.clone());
    let ask = |c: &mut Client| {
        ok(c.call(Cmd::AgentPrompt(v1::AgentPrompt {
            agent_id: s.agent_id.clone(),
            text: "ask".into(),
        })));
    };
    let requested = |n: &Node, skip: usize| -> String {
        let end = Instant::now() + Duration::from_secs(15);
        loop {
            let found: Vec<String> = n
                .events
                .events_after(0)
                .into_iter()
                .filter_map(|e| match e.body {
                    Some(Body::ApprovalRequested(a)) => Some(a.approval_id),
                    _ => None,
                })
                .collect();
            if let Some(id) = found.get(skip) {
                return id.clone();
            }
            assert!(Instant::now() < end, "no approval request announced");
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    let entry_of = |id: &str| {
        n.chat
            .snapshot(&agent)
            .into_iter()
            .find(|e| e.kind == v1::ChatKind::Permission as i32 && e.approval_id == id)
    };
    let decide = |c: &mut Client, id: &str, approved: bool| {
        ok(c.call(Cmd::ApprovalResolve(v1::ApprovalResolve {
            approval_id: id.into(),
            approved,
            signer_device_id: "phone".into(),
            signature: signed_message("node-test", id, approved),
            helper_grant: 0,
            helpers_may_start: false,
            ..Default::default()
        })));
    };
    let outcome_is = |id: &str, want: v1::PermissionOutcome| {
        let end = Instant::now() + Duration::from_secs(15);
        loop {
            if entry_of(id).is_some_and(|e| e.permission_outcome == want as i32) {
                return;
            }
            assert!(Instant::now() < end, "{id}: {:?}", entry_of(id));
            std::thread::sleep(Duration::from_millis(50));
        }
    };

    // Pending: the entry has the node's id (`ap-...`), not the adapter's own, and the action as its label.
    ask(&mut c);
    let first = requested(&n, 0);
    assert!(first.starts_with("ap-"), "{first}");
    outcome_is(&first, v1::PermissionOutcome::Pending);
    assert!(n
        .chat
        .snapshot(&agent)
        .iter()
        .all(|e| e.approval_id.is_empty() || e.approval_id.starts_with("ap-")));
    // A decision that fails verification changes nothing in the conversation.
    let bad = c.call(Cmd::ApprovalResolve(v1::ApprovalResolve {
        approval_id: first.clone(),
        approved: true,
        signer_device_id: "stranger".into(),
        signature: signed_message("node-test", &first, true),
        helper_grant: 0,
        helpers_may_start: false,
        ..Default::default()
    }));
    assert_eq!(err_code(bad), "approval.signature_invalid");
    outcome_is(&first, v1::PermissionOutcome::Pending);
    // The verified decision: approved.
    decide(&mut c, &first, true);
    outcome_is(&first, v1::PermissionOutcome::Approved);

    // Another request, denied.
    let end = Instant::now() + Duration::from_secs(15);
    while n
        .supervisor
        .lock()
        .unwrap()
        .list()
        .into_iter()
        .find(|v| v.id == agent)
        .unwrap()
        .state
        != agora_core::AgentState::Idle
    {
        assert!(Instant::now() < end, "agent never resumed");
        std::thread::sleep(Duration::from_millis(50));
    }
    ask(&mut c);
    let second = requested(&n, 1);
    outcome_is(&second, v1::PermissionOutcome::Pending);
    decide(&mut c, &second, false);
    outcome_is(&second, v1::PermissionOutcome::Denied);
    // The first stays approved: nothing later rewrites a decision.
    outcome_is(&first, v1::PermissionOutcome::Approved);

    // A third is still waiting when the agent is stopped: cancelled, and the first two are untouched.
    let end = Instant::now() + Duration::from_secs(15);
    while n
        .supervisor
        .lock()
        .unwrap()
        .list()
        .into_iter()
        .find(|v| v.id == agent)
        .unwrap()
        .state
        != agora_core::AgentState::Idle
    {
        assert!(Instant::now() < end, "agent never resumed");
        std::thread::sleep(Duration::from_millis(50));
    }
    ask(&mut c);
    let third = requested(&n, 2);
    outcome_is(&third, v1::PermissionOutcome::Pending);
    ok(c.call(Cmd::AgentStop(v1::AgentStop {
        agent_id: s.agent_id.clone(),
        stop_helpers: false,
    })));
    outcome_is(&third, v1::PermissionOutcome::Cancelled);
    outcome_is(&first, v1::PermissionOutcome::Approved);
    outcome_is(&second, v1::PermissionOutcome::Denied);
}

/// ADR-0110: a prompt the adapter refused because its agent exited reaches the client as `adapter.agent_exited` with the adapter's
/// stage (`turn`), not as `internal`, and the prompt is taken back from the conversation as for any prompt that did not go out.
#[test]
fn a_prompt_refused_because_the_agent_exited_reaches_the_client_with_its_reason() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let e = env("prompt-exited");
    add_manifest(
        &e,
        "ad.toml",
        &format!("kind = \"adapted\"\ndisplay_name = \"A\"\nversion = \"1\"\nmin_engine = 1\n[adapter]\ncommand = [{:?}, \"--adapter\"]\n", fake_adapter_path()),
    );
    let n = node(&e, NodeLimits::default());
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let s = started(ok(c.call(start_cmd("adapted"))));
    let info = c
        .call(Cmd::AgentPrompt(v1::AgentPrompt {
            agent_id: s.agent_id.clone(),
            text: "prompt-exited".into(),
        }))
        .expect("io")
        .expect_err("the adapter refused the prompt");
    assert_eq!(info.code, "adapter.agent_exited");
    let str_param = |k: &str| match &info.params[k].value {
        Some(v1::param_value::Value::Str(s)) => s.clone(),
        other => panic!("{k}: {other:?}"),
    };
    assert_eq!(str_param("stage"), "turn");
    assert_eq!(str_param("exit_code"), "3");
    assert_eq!(str_param("stderr_tail").len(), 600);
    assert_eq!(info.params.len(), 3, "{:?}", info.params);
    let agent = agora_core::AgentId::new(s.agent_id.clone());
    assert!(
        !n.chat
            .snapshot(&agent)
            .iter()
            .any(|e| e.role == v1::ChatRole::User as i32),
        "the prompt that did not go out is taken back"
    );
}

/// Security review of PR #1190, S1: a test scheme in which every signature differs, as the real signers' do (a random nonce): a
/// device "signs" by appending a nonce byte to the message.
struct NonceVerifier;

impl agora_node::approvals::Verifier for NonceVerifier {
    fn verify(&self, device: &str, message: &[u8], signature: &[u8]) -> bool {
        device == "phone" && signature.len() == message.len() + 1 && signature.starts_with(message)
    }
}

fn signed_with(message: Vec<u8>, nonce: u8) -> Vec<u8> {
    let mut s = message;
    s.push(nonce);
    s
}

/// ADR-0110 decision 6: a node with an adapter agent (`agora-fake-agent --adapter` and `args`) that was given `prompt` and asked for
/// approval `ap-1` (one of the fake adapter's `ask` prompts): the node, a client, the agent and the request as announced.
fn asked_over_control(
    name: &str,
    prompt: &str,
    args: &[&str],
) -> (
    Env,
    Node,
    Client,
    agora_core::AgentId,
    v1::ApprovalRequested,
) {
    use v1::event::Body;
    let e = env(name);
    let mut command = format!("{:?}, \"--adapter\"", fake_adapter_path());
    for a in args {
        command.push_str(&format!(", {a:?}"));
    }
    add_manifest(
        &e,
        "ad.toml",
        &format!("kind = \"adapted\"\ndisplay_name = \"A\"\nversion = \"1\"\nmin_engine = 1\n[adapter]\ncommand = [{command}]\n"),
    );
    let n = node(&e, NodeLimits::default());
    n.approvals.set_verifier(Box::new(NonceVerifier));
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let s = started(ok(c.call(start_cmd("adapted"))));
    let agent = agora_core::AgentId::new(s.agent_id.clone());
    ok(c.call(Cmd::AgentPrompt(v1::AgentPrompt {
        agent_id: s.agent_id.clone(),
        text: prompt.into(),
    })));
    let end = Instant::now() + Duration::from_secs(15);
    let asked = loop {
        if let Some(a) = n
            .events
            .events_after(0)
            .into_iter()
            .find_map(|e| match e.body {
                Some(Body::ApprovalRequested(a)) => Some(a),
                _ => None,
            })
        {
            break a;
        }
        assert!(Instant::now() < end, "no approval request announced");
        std::thread::sleep(Duration::from_millis(50));
    };
    (e, n, c, agent, asked)
}

/// A single answer to `id`, signed with `nonce` (a new signature for each nonce).
fn answer(
    c: &mut Client,
    id: &str,
    approved: bool,
    nonce: u8,
) -> Result<v1::CommandOk, v1::ErrorInfo> {
    use agora_node::approvals::signed_message;
    c.call(Cmd::ApprovalResolve(v1::ApprovalResolve {
        approval_id: id.to_string(),
        approved,
        signer_device_id: "phone".into(),
        signature: signed_with(signed_message("node-test", id, approved), nonce),
        ..Default::default()
    }))
    .expect("io")
}

/// ADR-0101: an "always" answer to the request `r` announced, signed with `nonce`.
fn answer_always(
    c: &mut Client,
    r: &v1::ApprovalRequested,
    nonce: u8,
) -> Result<v1::CommandOk, v1::ErrorInfo> {
    use agora_node::approvals::{always_signed_message, AlwaysOption, Offer, Pending};
    let pending = Pending {
        id: r.approval_id.clone(),
        agent_id: r.agent_id.clone(),
        action: r.action.clone(),
        detail: r.detail.clone(),
        requested_ms: 0,
        expires_ms: r.expires_at_ms,
        binding: None,
        remote: None,
        offer: Offer {
            once: r.scopes.contains(&(v1::ApprovalScope::Once as i32)),
            always: (!r.always_option_id.is_empty()).then(|| AlwaysOption {
                label: r.always_label.clone(),
                option_id: r.always_option_id.clone(),
            }),
        },
    };
    c.call(Cmd::ApprovalResolve(v1::ApprovalResolve {
        approval_id: r.approval_id.clone(),
        approved: true,
        signer_device_id: "phone".into(),
        signature: signed_with(
            always_signed_message("node-test", &r.approval_id, &pending),
            nonce,
        ),
        scope: v1::ApprovalScope::Always as i32,
        ..Default::default()
    }))
    .expect("io")
}

fn outcome_of(n: &Node, agent: &agora_core::AgentId, id: &str) -> Option<i32> {
    n.chat
        .snapshot(agent)
        .into_iter()
        .find(|e| e.kind == v1::ChatKind::Permission as i32 && e.approval_id == id)
        .map(|e| e.permission_outcome)
}

fn resolved_events(n: &Node) -> Vec<v1::ApprovalResolved> {
    n.events
        .events_after(0)
        .into_iter()
        .filter_map(|e| match e.body {
            Some(v1::event::Body::ApprovalResolved(r)) => Some(r),
            _ => None,
        })
        .collect()
}

/// `info` is `adapter.no_answer` for `call` after `waited_ms` (the node's 10 s wait; 0 for a request never given).
fn no_answer_after(info: &v1::ErrorInfo, call: &str, waited_ms: i64) {
    use v1::param_value::Value;
    assert_eq!(info.code, "adapter.no_answer", "{info:?}");
    assert_eq!(info.params["call"].value, Some(Value::Str(call.into())));
    assert_eq!(
        info.params["kind"].value,
        Some(Value::Str("adapted".into()))
    );
    assert_eq!(info.params["waited_ms"].value, Some(Value::Int(waited_ms)));
}

fn is_no_answer(info: &v1::ErrorInfo, call: &str) {
    no_answer_after(info, call, 10_000);
}

/// Waits until the card of `id` is no longer pending; its outcome.
fn settled(n: &Node, agent: &agora_core::AgentId, id: &str) -> Option<i32> {
    let end = Instant::now() + Duration::from_secs(20);
    while outcome_of(n, agent, id) == Some(v1::PermissionOutcome::Pending as i32) {
        assert!(Instant::now() < end, "the card was never settled");
        std::thread::sleep(Duration::from_millis(50));
    }
    // Anything that would record it a second time has had its chance.
    std::thread::sleep(Duration::from_millis(500));
    outcome_of(n, agent, id)
}

/// ADR-0110 decision 6: a decision whose answer the adapter does not take within the node's wait is answered `adapter.no_answer`, and
/// its card stays pending (no `ApprovalResolved`, the entry still pending), so the person can answer again. S1: the same signed answer
/// sent again (a hub replaying it) is refused and changes nothing; a new answer is accepted and settles it once, signed.
#[test]
fn a_decision_the_adapter_does_not_take_in_time_keeps_its_card_and_can_be_answered_again() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let (_e, n, mut c, agent, asked) = asked_over_control("decision-no-answer", "ask-hang", &[]);
    let id = asked.approval_id;
    let info = answer(&mut c, &id, true, 1).expect_err("the adapter did not take the answer");
    is_no_answer(&info, "approval");
    assert_eq!(
        outcome_of(&n, &agent, &id),
        Some(v1::PermissionOutcome::Pending as i32)
    );
    assert!(resolved_events(&n).is_empty(), "{:?}", resolved_events(&n));
    let replay = answer(&mut c, &id, true, 1).expect_err("the same signed answer again");
    assert_eq!(replay.code, "approval.signature_invalid", "{replay:?}");
    assert_eq!(
        outcome_of(&n, &agent, &id),
        Some(v1::PermissionOutcome::Pending as i32)
    );
    // A new answer (its own signature, its own request id): accepted, and the adapter takes this one.
    answer(&mut c, &id, true, 2).expect("the second answer is accepted");
    assert_eq!(
        outcome_of(&n, &agent, &id),
        Some(v1::PermissionOutcome::Approved as i32)
    );
    let resolved = resolved_events(&n);
    assert_eq!(resolved.len(), 1, "{resolved:?}");
    assert_eq!(
        (
            resolved[0].approval_id.as_str(),
            resolved[0].approved,
            resolved[0].signer_device_id.as_str()
        ),
        (id.as_str(), true, "phone")
    );
}

/// ADR-0110 decision 6: the adapter's answer to a decision that comes after the node's wait ran out still settles the card, once,
/// with the decision that was given; the request is then over, so another answer is refused.
#[test]
fn a_late_answer_from_the_adapter_settles_the_kept_card_once() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let (_e, n, mut c, agent, asked) = asked_over_control("decision-late", "ask-late", &[]);
    let id = asked.approval_id;
    let info = answer(&mut c, &id, false, 1).expect_err("the adapter answered after the wait");
    is_no_answer(&info, "approval");
    assert_eq!(
        outcome_of(&n, &agent, &id),
        Some(v1::PermissionOutcome::Pending as i32)
    );
    assert_eq!(
        settled(&n, &agent, &id),
        Some(v1::PermissionOutcome::Denied as i32)
    );
    let resolved = resolved_events(&n);
    assert_eq!(resolved.len(), 1, "{resolved:?}");
    assert_eq!(
        (resolved[0].approved, resolved[0].signer_device_id.as_str()),
        (false, "phone")
    );
    let again = answer(&mut c, &id, true, 2).expect_err("the request is over");
    assert_eq!(again.code, "protocol.invalid_message", "{again:?}");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(resolved_events(&n).len(), 1);
}

/// Security review of PR #1190, S2 (a): the person answers again while the first answer's call is still followed; the adapter then
/// takes the first answer late and refuses the second (a request it no longer holds). What is recorded is what reached the agent:
/// the first answer, once, never "cancelled".
#[test]
fn a_late_answer_that_reached_the_agent_wins_over_a_second_answer_it_refused() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let (_e, n, mut c, agent, asked) = asked_over_control("decision-late-second", "ask-late", &[]);
    let id = asked.approval_id;
    is_no_answer(&answer(&mut c, &id, true, 1).expect_err("late"), "approval");
    let second = answer(&mut c, &id, false, 2)
        .expect_err("the adapter refuses an answer to a request it no longer holds");
    assert_eq!(second.code, "internal", "{second:?}");
    assert_eq!(
        settled(&n, &agent, &id),
        Some(v1::PermissionOutcome::Approved as i32)
    );
    let resolved = resolved_events(&n);
    assert_eq!(resolved.len(), 1, "{resolved:?}");
    assert_eq!(
        (resolved[0].approved, resolved[0].signer_device_id.as_str()),
        (true, "phone")
    );
}

/// S2 (c): an "always" answer the adapter takes late is recorded as one the agent holds (ADR-0101: `ApprovalResolved` with ALWAYS
/// and `AgentAlwaysAllowed`), also when a second answer it refused came in between.
#[test]
fn an_always_answer_taken_late_is_recorded_as_one_the_agent_holds() {
    use v1::event::Body;
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let (_e, n, mut c, agent, asked) =
        asked_over_control("decision-late-always", "ask-late", &["--scopes"]);
    let id = asked.approval_id.clone();
    assert!(!asked.always_option_id.is_empty(), "{asked:?}");
    is_no_answer(
        &answer_always(&mut c, &asked, 1).expect_err("late"),
        "approval",
    );
    answer(&mut c, &id, false, 2).expect_err("refused: the agent already holds the first answer");
    assert_eq!(
        settled(&n, &agent, &id),
        Some(v1::PermissionOutcome::Approved as i32)
    );
    let resolved = resolved_events(&n);
    assert_eq!(resolved.len(), 1, "{resolved:?}");
    assert_eq!(
        resolved[0].scope,
        v1::ApprovalScope::Always as i32,
        "{resolved:?}"
    );
    assert!(n
        .events
        .events_after(0)
        .into_iter()
        .any(|e| matches!(e.body, Some(Body::AgentAlwaysAllowed(_)))));
}

/// N5: the agent stops while its card is kept (after `adapter.no_answer`): the stop cancels the card, once; the followed call ending
/// with the adapter records nothing more.
#[test]
fn a_kept_card_whose_agent_stops_is_cancelled_once() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let (_e, n, mut c, agent, asked) = asked_over_control("decision-kept-stop", "ask-hang", &[]);
    let id = asked.approval_id;
    is_no_answer(
        &answer(&mut c, &id, true, 1).expect_err("no answer"),
        "approval",
    );
    ok(c.call(Cmd::AgentStop(v1::AgentStop {
        agent_id: agent.as_str().to_string(),
        stop_helpers: false,
    })));
    assert_eq!(
        settled(&n, &agent, &id),
        Some(v1::PermissionOutcome::Cancelled as i32)
    );
    std::thread::sleep(Duration::from_secs(1));
    let resolved = resolved_events(&n);
    assert_eq!(resolved.len(), 1, "{resolved:?}");
    assert_eq!(
        (resolved[0].approved, resolved[0].signer_device_id.as_str()),
        (false, "")
    );
}

/// Fills the input of `agent`'s adapter (one that stopped reading): a large prompt the writer blocks on and enough small ones to fill
/// its queue, each on a thread of its own. Returns once a call found the queue full (`given` false).
fn fill_adapter_input(n: &Node, agent: &agora_core::AgentId) {
    let (tx, rx) = std::sync::mpsc::channel();
    for i in 0..80 {
        let call = n
            .supervisor
            .lock()
            .unwrap()
            .prepare_prompt(
                agent,
                &if i == 0 {
                    "x".repeat(1 << 20)
                } else {
                    "y".into()
                },
                false,
            )
            .unwrap();
        let tx = tx.clone();
        std::thread::spawn(move || {
            let full = matches!(
                call.run(),
                Err(agora_node::supervisor::SupervisorError::NoAnswer { given: false, .. })
            );
            let _ = tx.send(full);
        });
        std::thread::sleep(Duration::from_millis(5));
    }
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        let left = end.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(true) => return,
            Ok(false) => {}
            Err(_) => panic!("the adapter's input never filled"),
        }
    }
}

/// Security review of PR #1190, B1: an answer that finds its adapter's input full was never given to it: answered at once
/// (`waited_ms` 0), and the card stays pending, never left taken without a record.
#[test]
fn an_answer_never_given_to_a_full_adapter_input_keeps_its_card() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let (_e, n, mut c, agent, asked) = asked_over_control("decision-full", "ask-deaf", &[]);
    let id = asked.approval_id;
    fill_adapter_input(&n, &agent);
    let begun = Instant::now();
    let info = answer(&mut c, &id, true, 1).expect_err("never given");
    assert!(
        begun.elapsed() < Duration::from_secs(5),
        "{:?}",
        begun.elapsed()
    );
    no_answer_after(&info, "approval", 0);
    assert_eq!(
        outcome_of(&n, &agent, &id),
        Some(v1::PermissionOutcome::Pending as i32)
    );
    assert!(resolved_events(&n).is_empty());
    // It is still there to answer: the same check passes (the node did not spend it), only the adapter is still not reading.
    let again = answer(&mut c, &id, true, 2).expect_err("still full");
    no_answer_after(&again, "approval", 0);
}

/// B1: a prompt that finds its adapter's input full was never given: it is taken back (no "not confirmed" notice), at once.
#[test]
fn a_prompt_never_given_to_a_full_adapter_input_is_taken_back() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let (_e, n, mut c, agent, _asked) = asked_over_control("prompt-full", "ask-deaf", &[]);
    fill_adapter_input(&n, &agent);
    let info = c
        .call(Cmd::AgentPrompt(v1::AgentPrompt {
            agent_id: agent.as_str().to_string(),
            text: "never given".into(),
        }))
        .expect("io")
        .expect_err("never given");
    no_answer_after(&info, "prompt", 0);
    let entries = n.chat.snapshot(&agent);
    assert!(
        !entries.iter().any(|e| e.text == "never given"),
        "taken back"
    );
    assert!(
        !entries.iter().any(|e| e.code == "adapter.no_answer"),
        "no mark"
    );
}

/// ADR-0110 decision 6: a prompt the adapter does not answer within the node's wait stays in the conversation (the adapter may hold
/// it), marked as not confirmed by a notice `adapter.no_answer` that names its entry.
#[test]
fn a_prompt_the_adapter_does_not_answer_in_time_stays_marked_not_confirmed() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let e = env("prompt-no-answer");
    add_manifest(
        &e,
        "ad.toml",
        &format!("kind = \"adapted\"\ndisplay_name = \"A\"\nversion = \"1\"\nmin_engine = 1\n[adapter]\ncommand = [{:?}, \"--adapter\"]\n", fake_adapter_path()),
    );
    let n = node(&e, NodeLimits::default());
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let s = started(ok(c.call(start_cmd("adapted"))));
    let info = c
        .call(Cmd::AgentPrompt(v1::AgentPrompt {
            agent_id: s.agent_id.clone(),
            text: "hang".into(),
        }))
        .expect("io")
        .expect_err("the adapter did not answer");
    is_no_answer(&info, "prompt");
    let agent = agora_core::AgentId::new(s.agent_id.clone());
    let entries = n.chat.snapshot(&agent);
    let user = entries
        .iter()
        .find(|e| e.role == v1::ChatRole::User as i32 && e.text == "hang")
        .expect("the prompt stays in the conversation");
    assert!(!user.queued);
    let notice = entries
        .iter()
        .find(|e| e.kind == v1::ChatKind::Notice as i32 && e.code == "adapter.no_answer")
        .expect("the prompt is marked as not confirmed");
    assert_eq!(notice.params["entries"], user.entry_id.to_string());
    assert_eq!(notice.params["call"], "prompt");
    assert_eq!(notice.params["kind"], "adapted");
    assert_eq!(notice.params["waited_ms"], "10000");
}

/// ADR-0110 decision 6: the same for a prompt that waited behind a turn: when it is sent and its adapter does not answer within the
/// node's wait, it stays in the conversation (no longer shown waiting, never "not sent"), marked as not confirmed.
#[test]
fn a_queued_prompt_the_adapter_does_not_answer_in_time_stays_marked_not_confirmed() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let e = env("queued-no-answer");
    add_manifest(
        &e,
        "ad.toml",
        &format!("kind = \"adapted\"\ndisplay_name = \"A\"\nversion = \"1\"\nmin_engine = 1\n[adapter]\ncommand = [{:?}, \"--adapter\", \"--turns\"]\n", fake_adapter_path()),
    );
    let n = node(&e, NodeLimits::default());
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let s = started(ok(c.call(start_cmd("adapted"))));
    let agent = agora_core::AgentId::new(s.agent_id.clone());
    let prompt = |c: &mut Client, text: &str| {
        ok(c.call(Cmd::AgentPrompt(v1::AgentPrompt {
            agent_id: s.agent_id.clone(),
            text: text.into(),
        })))
    };
    // A turn that stays open until `go.txt` appears, and `hang` queued behind it.
    prompt(&mut c, "hold:working");
    let end = Instant::now() + Duration::from_secs(15);
    let queued = loop {
        let r = prompt(&mut c, "hang");
        match r.result {
            Some(v1::command_ok::Result::AgentPrompt(p)) if p.queued => break true,
            _ => {}
        }
        assert!(Instant::now() < end, "the second prompt was never queued");
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(queued);
    let folder = n.supervisor.lock().unwrap().list()[0].worktree.clone();
    std::fs::write(folder.join("go.txt"), "go").unwrap();
    let end = Instant::now() + Duration::from_secs(25);
    let entries = loop {
        let entries = n.chat.snapshot(&agent);
        if entries.iter().any(|e| {
            e.kind == v1::ChatKind::Notice as i32
                && (e.code == "adapter.no_answer" || e.code == "adapter.queue_dropped")
        }) {
            break entries;
        }
        assert!(Instant::now() < end, "the queued prompt's send never ended");
        std::thread::sleep(Duration::from_millis(100));
    };
    let user = entries
        .iter()
        .find(|e| e.role == v1::ChatRole::User as i32 && e.text == "hang")
        .expect("the prompt stays in the conversation");
    assert!(!user.queued, "it is no longer shown waiting");
    let notice = entries
        .iter()
        .find(|e| e.kind == v1::ChatKind::Notice as i32 && e.code == "adapter.no_answer")
        .expect("the prompt is marked as not confirmed");
    assert_eq!(notice.params["entries"], user.entry_id.to_string());
    assert!(!entries.iter().any(|e| e.code == "adapter.queue_dropped"));
}

/// ADR-0090: a verified decision that cannot be delivered (the adapter answers with an error) does not leave the entry in the
/// conversation pending for good: it is shown as cancelled, and the user gets the error.
#[test]
fn a_decision_the_adapter_cannot_take_does_not_leave_the_permission_entry_pending() {
    use agora_node::approvals::signed_message;
    use v1::event::Body;
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let e = env("permission-undeliverable");
    add_manifest(
        &e,
        "ad.toml",
        &format!("kind = \"adapted\"\ndisplay_name = \"A\"\nversion = \"1\"\nmin_engine = 1\n[adapter]\ncommand = [{:?}, \"--adapter\"]\n", fake_adapter_path()),
    );
    let n = node(&e, NodeLimits::default());
    n.approvals.set_verifier(Box::new(EchoVerifier));
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let s = started(ok(c.call(start_cmd("adapted"))));
    let agent = agora_core::AgentId::new(s.agent_id.clone());
    ok(c.call(Cmd::AgentPrompt(v1::AgentPrompt {
        agent_id: s.agent_id.clone(),
        text: "ask-error".into(),
    })));
    let end = Instant::now() + Duration::from_secs(15);
    let id = loop {
        if let Some(a) = n
            .events
            .events_after(0)
            .into_iter()
            .find_map(|e| match e.body {
                Some(Body::ApprovalRequested(a)) => Some(a),
                _ => None,
            })
        {
            break a.approval_id;
        }
        assert!(Instant::now() < end, "no approval request announced");
        std::thread::sleep(Duration::from_millis(50));
    };
    let outcome = || {
        n.chat
            .snapshot(&agent)
            .into_iter()
            .find(|e| e.kind == v1::ChatKind::Permission as i32 && e.approval_id == id)
            .map(|e| e.permission_outcome)
    };
    assert_eq!(outcome(), Some(v1::PermissionOutcome::Pending as i32));
    let r = c.call(Cmd::ApprovalResolve(v1::ApprovalResolve {
        approval_id: id.clone(),
        approved: true,
        signer_device_id: "phone".into(),
        signature: signed_message("node-test", &id, true),
        helper_grant: 0,
        helpers_may_start: false,
        ..Default::default()
    }));
    // #1150: the user is told the decision did not reach the agent, as a failed approval answer (never as a failed start, reference
    // `unassigned`).
    let info = r
        .expect("io")
        .expect_err("the decision did not reach the agent");
    assert_eq!(info.code, "internal");
    assert_eq!(
        info.params["incident_id"].value,
        Some(v1::param_value::Value::Str("approval.resolve".into()))
    );
    assert_eq!(outcome(), Some(v1::PermissionOutcome::Cancelled as i32));
    // Codex review of #794: the request is over, so the hub is told as for a cancelled turn (unsigned, not approved), or it would
    // keep showing an approval the node no longer holds.
    let resolved: Vec<_> = n
        .events
        .events_after(0)
        .into_iter()
        .filter_map(|e| match e.body {
            Some(Body::ApprovalResolved(r)) => Some(r),
            _ => None,
        })
        .collect();
    assert_eq!(resolved.len(), 1, "{resolved:?}");
    assert_eq!(
        (
            resolved[0].approval_id.as_str(),
            resolved[0].approved,
            resolved[0].signer_device_id.as_str()
        ),
        (id.as_str(), false, "")
    );
}

/// ADR-0090: a client that reads too slowly is dropped from the conversation, and told: the stream ends with a close frame (which the
/// uplink relays to the hub so the window subscribes again), and a new subscription gets the ring as a fresh snapshot with no entry
/// twice.
#[test]
fn a_subscriber_that_falls_behind_is_told_its_stream_ended_and_can_subscribe_again() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let e = env("chat-lag");
    std::fs::write(
        e.dir.join("manifests/fake.toml"),
        format!(
            "kind = \"fake-adapter\"\ndisplay_name = \"F\"\nversion = \"1\"\nmin_engine = 1\n[adapter]\ncommand = [{:?}, \"--adapter\"]\n",
            fake_agent_path()
        ),
    )
    .unwrap();
    let n = node(&e, NodeLimits::default());
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let s = started(ok(c.call(start_cmd("fake-adapter"))));
    let agent = agora_core::AgentId::new(s.agent_id.clone());
    ok(c.call_on(
        3,
        Cmd::ChatSubscribe(v1::ChatSubscribe {
            agent_id: s.agent_id.clone(),
        }),
    ));
    // The client does not read while the transcript grows by far more than the follower's queue and the socket's buffers hold.
    for i in 0..3000 {
        n.chat.push(
            &agent,
            v1::ChatRole::Agent,
            v1::ChatKind::Text,
            format!("{i}{}", "x".repeat(8000)),
            String::new(),
            0,
        );
    }
    // Now it reads: entries (in order, no gaps in what it got, since the queue was only ever cut at the end), then the close frame.
    let mut ids = Vec::new();
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        assert!(
            Instant::now() < end,
            "the stream never ended: {} entries",
            ids.len()
        );
        let Some(f) = c.read_frame(Some(Duration::from_millis(500))).unwrap() else {
            continue;
        };
        if f.stream_id != 3 {
            continue;
        }
        match f.kind {
            agora_node::control::KIND_CHAT_ENTRY => ids.push(
                <v1::ChatEntry as prost::Message>::decode(f.payload.as_slice())
                    .unwrap()
                    .entry_id,
            ),
            agora_node::control::KIND_CHAT_SNAPSHOT => {}
            agora_protocol::frame::STREAM_CLOSE => break,
            other => panic!("unexpected frame kind {other}"),
        }
    }
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "in order, none twice");
    assert!(
        ids.len() < 3000,
        "the follower really was dropped part way: {}",
        ids.len()
    );
    // Subscribing again gives the ring, newest entries included, each entry once.
    ok(c.call_on(
        4,
        Cmd::ChatSubscribe(v1::ChatSubscribe {
            agent_id: s.agent_id.clone(),
        }),
    ));
    let snap = wait_frame(&mut c, 4, |f| {
        f.kind == agora_node::control::KIND_CHAT_SNAPSHOT
    });
    let snap = <v1::ChatSnapshot as prost::Message>::decode(snap.payload.as_slice()).unwrap();
    let fresh: Vec<u64> = snap.entries.iter().map(|e| e.entry_id).collect();
    assert!(fresh.windows(2).all(|w| w[0] < w[1]), "no entry id twice");
    assert_eq!(
        fresh.last().copied(),
        Some(snap.entries.iter().map(|e| e.entry_id).max().unwrap())
    );
    assert!(
        snap.entries.iter().any(|e| e.text.starts_with("2999")),
        "the newest entry is there"
    );
}

/// The user's prompt is in the conversation before whatever the agent answers: the fake agent echoes at once, and every echo comes
/// after the prompt it answers.
#[test]
fn the_users_prompt_comes_before_the_agents_answer() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let e = env("chat-order");
    std::fs::write(
        e.dir.join("manifests/fake.toml"),
        format!(
            "kind = \"fake-adapter\"\ndisplay_name = \"F\"\nversion = \"1\"\nmin_engine = 1\n[adapter]\ncommand = [{:?}, \"--adapter\"]\n",
            fake_agent_path()
        ),
    )
    .unwrap();
    let n = node(&e, NodeLimits::default());
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let s = started(ok(c.call(start_cmd("fake-adapter"))));
    let agent = agora_core::AgentId::new(s.agent_id.clone());
    std::thread::sleep(Duration::from_millis(800)); // the adapter finishes its handshake
                                                    // The agent speaks BEFORE the node has its answer to the prompt: the user's entry must already be in the conversation.
    ok(c.call(Cmd::AgentPrompt(v1::AgentPrompt {
        agent_id: s.agent_id.clone(),
        text: "first:hello".into(),
    })));
    let end = Instant::now() + Duration::from_secs(15);
    let all = loop {
        let all = n.chat.snapshot(&agent);
        if all
            .iter()
            .any(|e| e.role == v1::ChatRole::Agent as i32 && e.text == "hello")
        {
            break all;
        }
        assert!(Instant::now() < end, "the agent never spoke");
        std::thread::sleep(Duration::from_millis(10));
    };
    let id_of = |role: v1::ChatRole, text: &str| {
        all.iter()
            .find(|e| e.role == role as i32 && e.text == text)
            .map(|e| e.entry_id)
            .unwrap()
    };
    assert!(id_of(v1::ChatRole::User, "first:hello") < id_of(v1::ChatRole::Agent, "hello"));
}

/// #1133 ("yes, add a policy line"): `agents.registry = off` in the node's own policy switches the ACP registry's kinds off on this
/// machine: none is offered, a start of one is refused with `node.policy_denied` (rule `registry_off`), and the hand-made kinds are not
/// affected. `agents.registry.deny` switches off the kinds it names only (rule `registry_denied`).
#[test]
fn the_registry_policy_line_switches_registry_kinds_off_and_leaves_hand_made_ones() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let rule_of = |r: std::io::Result<Result<v1::CommandOk, v1::ErrorInfo>>| {
        let e = r.expect("io").expect_err("expected a refusal");
        let rule = match e.params.get("rule").and_then(|p| p.value.clone()) {
            Some(v1::param_value::Value::Str(s)) => s,
            other => format!("{other:?}"),
        };
        (e.code, rule)
    };
    let kinds = |c: &mut Client| -> Vec<String> {
        match ok(c.call(Cmd::AgentKinds(v1::AgentKinds {}))).result {
            Some(Ok_::AgentKinds(k)) => k.kinds.into_iter().map(|k| k.kind).collect(),
            other => panic!("{other:?}"),
        }
    };
    for (name, line, gone, kept) in [
        (
            "registry-off",
            "agents.registry = off\n",
            vec!["registry-gemini", "registry-goose"],
            vec![],
        ),
        (
            "registry-deny",
            "agents.registry.deny = registry-gemini-mine, registry-gemini\n",
            vec!["registry-gemini"],
            vec!["registry-goose"],
        ),
    ] {
        let e = env(name);
        std::fs::create_dir_all(e.dir.join("data")).unwrap();
        std::fs::write(e.dir.join("data").join("policy"), line).unwrap();
        // Review F5: a hand-made description whose name looks like a registry kind's is the owner's own, never switched off.
        std::fs::write(
            e.dir.join("manifests").join("mine.toml"),
            "kind = \"registry-gemini-mine\"\ndisplay_name = \"Mine\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"sh\"]\n",
        )
        .unwrap();
        let n = node(&e, NodeLimits::default());
        let mut c = Client::connect(&e.dir.join("c.sock")).unwrap();
        let listed = kinds(&mut c);
        for k in ["codex", "claude-code", "shell"] {
            assert!(listed.iter().any(|x| x == k), "{name}: {k} stays");
        }
        for k in &gone {
            assert!(!listed.iter().any(|x| x == k), "{name}: {k} is not offered");
            let want = if name == "registry-off" {
                "registry_off"
            } else {
                "registry_denied"
            };
            assert_eq!(
                rule_of(c.call(start_cmd(k))),
                ("node.policy_denied".to_string(), want.to_string()),
                "{name}: {k}"
            );
        }
        for k in &kept {
            assert!(listed.iter().any(|x| x == k), "{name}: {k} stays offered");
        }
        // Hand-made kinds still start, also one named like a registry kind.
        assert!(listed.iter().any(|x| x == "registry-gemini-mine"), "{name}");
        for k in ["shell", "registry-gemini-mine"] {
            let s = started(ok(c.call(start_cmd(k))));
            assert!(!s.agent_id.is_empty(), "{name}: {k}");
        }
        drop(n);
        let _ = std::fs::remove_dir_all(&e.dir);
    }
}

/// #1147 (audit of #1157): a value the node holds, in an agent's tool title and arguments, never reaches the hub's event for the
/// approval card (nor the queue the window's card and the always signature read, nor the conversation's permission entry).
#[test]
fn a_secret_in_an_approvals_action_and_detail_never_reaches_the_hub_event() {
    use v1::event::Body;
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let value = "SK-APPROVAL-0123456789-ABCDEFGHIJ";
    let e = env("approval-secret");
    agora_node::credentials::SecretStore::in_data(&e.dir.join("data"))
        .set(
            "TOKEN",
            &agora_node::credentials::Secret::new(value).unwrap(),
        )
        .unwrap();
    add_manifest(
        &e,
        "ad.toml",
        &format!("kind = \"adapted\"\ndisplay_name = \"A\"\nversion = \"1\"\nmin_engine = 1\n[adapter]\ncommand = [{:?}, \"--adapter\"]\n", fake_adapter_path()),
    );
    let n = node(&e, NodeLimits::default());
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let s = started(ok(c.call(start_cmd("adapted"))));
    // The title holds the whole value; the arguments hold it whole and, cut where the bridge cut them, its first 20 bytes.
    let detail = format!("{{\"cmd\":\"curl -H {value}\",\"tail\":\"{}…", &value[..20]);
    ok(c.call(Cmd::AgentPrompt(v1::AgentPrompt {
        agent_id: s.agent_id.clone(),
        text: format!("ask:login with {value}|{detail}"),
    })));
    let end = Instant::now() + Duration::from_secs(15);
    let a = loop {
        let found = n
            .events
            .events_after(0)
            .into_iter()
            .find_map(|e| match e.body {
                Some(Body::ApprovalRequested(a)) => Some(a),
                _ => None,
            });
        if let Some(a) = found {
            break a;
        }
        assert!(Instant::now() < end, "no approval request announced");
        std::thread::sleep(Duration::from_millis(50));
    };
    let pieces: Vec<&str> = (0..=value.len() - 4).map(|i| &value[i..i + 4]).collect();
    let leaks = |t: &str| pieces.iter().filter(|p| t.contains(**p)).count();
    assert_eq!(leaks(&a.action), 0, "{a:?}");
    assert_eq!(leaks(&a.detail), 0, "{a:?}");
    assert!(a.action.starts_with("login with "), "{a:?}");
    assert!(a.detail.starts_with("{\"cmd\":\"curl -H "), "{a:?}");
    let queued = n.approvals.waiting();
    assert!(queued
        .iter()
        .all(|p| leaks(&p.action) == 0 && leaks(&p.detail) == 0));
    let entries = n
        .chat
        .snapshot(&agora_core::AgentId::new(s.agent_id.clone()));
    // The agent's entries (the permission entry); the person's own prompt is theirs, typed through the window.
    assert!(
        entries
            .iter()
            .filter(|e| e.role == v1::ChatRole::Agent as i32)
            .all(|e| leaks(&e.text) == 0),
        "{entries:#?}"
    );
}
