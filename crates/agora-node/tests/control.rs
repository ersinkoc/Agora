//! Control socket tests: a real node (supervisor + socket) driven through the protocol client, with a plain
//! `sh` and the fake agent as the "agents". PTY and local sockets: Unix and Windows (ADR-0045).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use agora_node::control::{
    Client, KIND_CHAT_ENTRY, KIND_CHAT_SNAPSHOT, KIND_TERM_DATA, KIND_TERM_EXIT, KIND_TERM_RESIZE,
    KIND_TERM_SNAPSHOT,
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
        .call(Cmd::WorktreeRemove(v1::WorktreeRemove::default()))
        .unwrap()
        .unwrap_err();
    assert_eq!(info.code, "protocol.capability_missing");
    assert_eq!(
        info.params["capability"].value,
        Some(v1::param_value::Value::Str("worktree.remove".into()))
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

/// What a hub needs to build its view: the node, its checkout, then for each agent its workspace, the agent and its
/// terminal, in that order, and the matching closings.
#[test]
fn the_node_reports_what_the_hub_needs_to_show_it() {
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
    ok(c.call(Cmd::AgentStop(v1::AgentStop {
        agent_id: s.agent_id.clone(),
    })));
    assert!(
        t0.elapsed() < Duration::from_secs(5),
        "the node waited behind the stuck write: {:?}",
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
    if !p.exists() {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        assert!(Command::new(cargo)
            .args(["build", "-q", "-p", "agora-fake-agent"])
            .status()
            .unwrap()
            .success());
    }
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
    drop(n);
}

#[test]
fn uncommitted_changes_in_an_agents_workspace_are_reported() {
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

    // The watcher checks every few seconds; the latest report for this workspace must show both new files.
    let end = Instant::now() + Duration::from_secs(20);
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
    })));
}

#[test]
fn git_status_and_diff_cover_new_and_changed_files_and_only_agent_workspaces() {
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
    })));
}

#[test]
fn files_list_and_read_stay_inside_an_agents_workspace() {
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
    })));
}

#[test]
fn git_log_lists_the_workspaces_commits_newest_first() {
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
    })));
}

fn fake_agent_path() -> String {
    let mut p = std::env::current_exe().expect("exe");
    p.pop();
    p.pop();
    p.push(format!("agora-fake-agent{}", std::env::consts::EXE_SUFFIX));
    if !p.exists() {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        assert!(
            Command::new(cargo)
                .args(["build", "-q", "-p", "agora-fake-agent"])
                .status()
                .expect("run cargo")
                .success(),
            "building agora-fake-agent failed"
        );
    }
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
    })));
}
