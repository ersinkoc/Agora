//! What a node accepts from its hub on a dynamic stream (protocol.md, terminal relay): only a command that opens a stream, and
//! that one under the same grant rules as on stream 1 (ADR-0041). Driven by a fake hub over plain WebSocket against a real node.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use agora_node::control::Client;
use agora_node::daemon::{Node, NodeConfig};
use agora_node::uplink_client::{spawn, HubLink, UplinkHandle};
use agora_protocol::frame::{kinds, STREAM_COMMANDS, STREAM_CONTROL};
use agora_protocol::v1::{
    self, command_ok::Result as Ok_, command_request::Command as Cmd, command_response,
    CommandRequest, CommandResponse,
};
use agora_protocol::Frame;
use prost::Message;

fn git(dir: &Path, args: &[&str]) {
    assert!(std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .status()
        .unwrap()
        .success());
}

struct Env {
    dir: PathBuf,
    repo: PathBuf,
    /// Written by the `stub` kind's program each time it starts.
    marker: PathBuf,
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn env(name: &str, policy: &str) -> Env {
    let dir = std::env::temp_dir().join(format!("agora-dyn-{}-{name}", std::process::id()));
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
        manifests.join("plain.toml"),
        "kind = \"plain\"\ndisplay_name = \"Plain\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"sh\"]\n",
    )
    .unwrap();
    let marker = dir.join("STARTED");
    std::fs::write(
        manifests.join("stub.toml"),
        format!(
            "kind = \"stub\"\ndisplay_name = \"Stub\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"sh\", \"-c\", \"echo x >> \\\"$0\\\"; exec sleep 30\", {:?}]\n",
            marker.to_string_lossy()
        ),
    )
    .unwrap();
    let data = dir.join("data");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("policy"), format!("signed_commands = {policy}\n")).unwrap();
    Env { dir, repo, marker }
}

fn node(e: &Env) -> Node {
    Node::start(&NodeConfig {
        resume_all: false,
        node_id: "laptop".into(),
        socket: e.dir.join("c.sock"),
        data_dir: e.dir.join("data"),
        worktree_root: e.dir.join("wt"),
        manifests_dir: e.dir.join("manifests"),
        projects: vec![e.repo.clone()],
        limits: Default::default(),
    })
    .expect("start node")
}

fn frame(stream_id: u32, kind: u8, payload: Vec<u8>) -> Frame {
    Frame {
        stream_id,
        kind,
        seq: None,
        payload,
    }
}

fn req(id: &str, cmd: Cmd) -> CommandRequest {
    CommandRequest {
        request_id: id.into(),
        command: Some(cmd),
        ..Default::default()
    }
}

/// `ok`, or the error code with its `field` when it names one (`protocol.invalid_message/stream_id`).
fn outcome(r: &CommandResponse) -> String {
    match &r.result {
        Some(command_response::Result::Ok(_)) => "ok".into(),
        Some(command_response::Result::Err(e)) => {
            match e.params.get("field").and_then(|p| p.value.as_ref()) {
                Some(v1::param_value::Value::Str(f)) => format!("{}/{f}", e.code),
                _ => e.code.clone(),
            }
        }
        None => "none".into(),
    }
}

/// What the fake hub saw: each request's outcome by id, and the frame kinds that came on each dynamic stream.
struct Seen {
    answers: HashMap<String, String>,
    streams: HashMap<u32, Vec<u8>>,
    /// How long the node's uplink took to stop once the hub was done.
    stopped_in: Duration,
}

/// One thing the fake hub does: send a request on a stream and wait for its answer, or close a stream (kind 255).
// Test-only, a few per test: the size of a request is no concern here.
#[allow(clippy::large_enum_variant)]
enum Step {
    Ask(u32, CommandRequest),
    Close(u32),
}

/// Plays a hub: takes the node's connection, sends each `(stream, request)` once the previous one was answered, then keeps reading
/// until `done` holds for what came and half a second more (or a deadline passes).
fn hub(
    n: &Node,
    socket: &Path,
    script: Vec<(u32, CommandRequest)>,
    done: impl Fn(&Seen) -> bool,
) -> Seen {
    let steps = script.into_iter().map(|(s, r)| Step::Ask(s, r)).collect();
    hub_steps(n, socket, steps, done)
}

fn hub_steps(n: &Node, socket: &Path, script: Vec<Step>, done: impl Fn(&Seen) -> bool) -> Seen {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let l = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let port = l.local_addr().unwrap().port();
    let uplink: UplinkHandle = spawn(
        Arc::clone(&n.events),
        HubLink {
            mesh_out: None,
            url: format!("ws://127.0.0.1:{port}"),
            ca_pem: String::new(),
            cert_pem: String::new(),
            key_pem: String::new(),
            commands: Some(socket.to_path_buf()),
            grants: Some(Arc::clone(&n.grants)),
            mesh: None,
            devices: None,
            state_dir: None,
            certs: None,
        },
    )
    .unwrap();
    let mut seen = rt.block_on(async move {
        let (s, _) = l.accept().await.unwrap();
        let mut c = agora_protocol::ws::accept(s).await.unwrap();
        let hello = c.recv().await.unwrap().unwrap();
        assert_eq!(hello.kind, kinds::HELLO);
        let welcome = v1::Welcome {
            chosen_generation: 1,
            ..Default::default()
        };
        c.send(&frame(
            STREAM_CONTROL,
            kinds::WELCOME,
            welcome.encode_to_vec(),
        ))
        .await
        .unwrap();
        loop {
            let f = c.recv().await.unwrap().unwrap();
            if f.kind == kinds::RESUME {
                break;
            }
        }
        let ok = v1::ResumeOk { from_seq: 1 };
        c.send(&frame(STREAM_CONTROL, kinds::RESUME_OK, ok.encode_to_vec()))
            .await
            .unwrap();
        let mut seen = Seen {
            answers: HashMap::new(),
            streams: HashMap::new(),
            stopped_in: Duration::ZERO,
        };
        let mut script = script.into_iter();
        let mut waiting = 0;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        // Once everything was answered and `done` holds, frames that should not come get a while longer to show up.
        let mut settled: Option<tokio::time::Instant> = None;
        loop {
            if waiting == 0 {
                match script.next() {
                    Some(Step::Ask(stream, r)) => {
                        c.send(&frame(stream, 0, r.encode_to_vec())).await.unwrap();
                        waiting = 1;
                    }
                    Some(Step::Close(stream)) => {
                        c.send(&frame(stream, 255, Vec::new())).await.unwrap();
                        // Kind 255 has no answer; a moment for the node to let the stream go before the next step.
                        tokio::time::sleep(Duration::from_millis(300)).await;
                        continue;
                    }
                    None if done(&seen) => {
                        settled.get_or_insert_with(|| {
                            tokio::time::Instant::now() + Duration::from_millis(500)
                        });
                    }
                    None => {}
                }
            }
            let now = tokio::time::Instant::now();
            if now >= deadline || settled.is_some_and(|at| now >= at) {
                break;
            }
            match tokio::time::timeout(Duration::from_millis(200), c.recv()).await {
                Ok(Ok(Some(f))) if (f.stream_id, f.kind) == (STREAM_COMMANDS, 1) => {
                    let r = CommandResponse::decode(f.payload.as_slice()).unwrap();
                    seen.answers.insert(r.request_id.clone(), outcome(&r));
                    waiting = 0;
                }
                Ok(Ok(Some(f))) if (f.stream_id, f.kind) == (STREAM_CONTROL, kinds::PING) => {
                    let _ = c.send(&frame(STREAM_CONTROL, kinds::PONG, f.payload)).await;
                }
                Ok(Ok(Some(f))) if f.stream_id >= 3 => {
                    seen.streams.entry(f.stream_id).or_default().push(f.kind);
                }
                Ok(Ok(Some(_))) | Err(_) => {}
                Ok(Ok(None)) | Ok(Err(_)) => break,
            }
        }
        seen
    });
    let at = std::time::Instant::now();
    drop(uplink);
    seen.stopped_in = at.elapsed();
    seen
}

fn local_agents(socket: &Path) -> Vec<String> {
    let mut c = Client::connect(socket).unwrap();
    match c
        .call(Cmd::AgentList(v1::AgentList::default()))
        .unwrap()
        .unwrap()
        .result
    {
        Some(Ok_::AgentList(l)) => l.agents.into_iter().map(|a| a.agent_id).collect(),
        other => panic!("not an agent list: {other:?}"),
    }
}

fn start_shell(socket: &Path) -> v1::AgentStartResult {
    let mut c = Client::connect(socket).unwrap();
    match c
        .call(Cmd::AgentStart(v1::AgentStart {
            kind: "plain".into(),
            ..Default::default()
        }))
        .unwrap()
        .unwrap()
        .result
    {
        Some(Ok_::AgentStart(s)) => s,
        other => panic!("not a start: {other:?}"),
    }
}

fn stop(socket: &Path, agent_id: &str) {
    let mut c = Client::connect(socket).unwrap();
    let _ = c.call(Cmd::AgentStop(v1::AgentStop {
        agent_id: agent_id.into(),
        stop_helpers: false,
    }));
}

/// Protocol.md: a dynamic stream carries a stream-opening command and nothing else. The node holds to that itself: a command for
/// stream 1 sent on a dynamic stream is `protocol.invalid_message` and never runs, while subscriptions there work as before.
#[test]
fn a_node_takes_only_stream_opening_commands_on_a_dynamic_stream() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let e = env("allow", "agent.start, agent.launch");
    let n = node(&e);
    let socket = e.dir.join("c.sock");
    let shell = start_shell(&socket);
    // A launch that the node would carry out if it ran: the folder's real project id, as the window sends it.
    let project_id = {
        let mut c = Client::connect(&socket).unwrap();
        match c
            .call(Cmd::DirectoryInspect(v1::DirectoryInspect {
                path: e.repo.to_string_lossy().into_owned(),
                ..Default::default()
            }))
            .unwrap()
            .unwrap()
            .result
        {
            Some(Ok_::DirectoryInfo(d)) => d.project_id,
            other => panic!("not a directory: {other:?}"),
        }
    };

    let start = || {
        Cmd::AgentStart(v1::AgentStart {
            kind: "stub".into(),
            ..Default::default()
        })
    };
    let script = vec![
        // On stream 1 the policy asks for a grant.
        (STREAM_COMMANDS, req("cmd", start())),
        // A command that opens a stream needs one of its own.
        (
            STREAM_COMMANDS,
            req(
                "cmd-term",
                Cmd::TerminalSubscribe(v1::TerminalSubscribe {
                    terminal_id: shell.terminal_id.clone(),
                    ..Default::default()
                }),
            ),
        ),
        (
            STREAM_COMMANDS,
            req(
                "cmd-preview",
                Cmd::PreviewConnect(v1::PreviewConnect {
                    forward_id: "f".into(),
                }),
            ),
        ),
        (3, req("dyn-start", start())),
        (
            5,
            req(
                "dyn-launch",
                Cmd::AgentLaunch(v1::AgentLaunch {
                    kind: "stub".into(),
                    path: e.repo.to_string_lossy().into_owned(),
                    project_id: project_id.clone(),
                    ..Default::default()
                }),
            ),
        ),
        (
            7,
            req(
                "dyn-stop",
                Cmd::AgentStop(v1::AgentStop {
                    agent_id: shell.agent_id.clone(),
                    stop_helpers: false,
                }),
            ),
        ),
        (
            9,
            req(
                "dyn-auth",
                Cmd::ControlAuth(v1::ControlAuth {
                    token: "guess".into(),
                }),
            ),
        ),
        (
            11,
            CommandRequest {
                request_id: "dyn-empty".into(),
                ..Default::default()
            },
        ),
        (
            13,
            req(
                "term",
                Cmd::TerminalSubscribe(v1::TerminalSubscribe {
                    terminal_id: shell.terminal_id.clone(),
                    format: "terminal.raw".into(),
                    history_lines: 0,
                }),
            ),
        ),
        (
            15,
            req(
                "chat",
                Cmd::ChatSubscribe(v1::ChatSubscribe {
                    agent_id: shell.agent_id.clone(),
                }),
            ),
        ),
        (
            17,
            req(
                "preview",
                Cmd::PreviewConnect(v1::PreviewConnect {
                    forward_id: "f".into(),
                }),
            ),
        ),
    ];
    let seen = hub(&n, &socket, script, |s| {
        s.streams.get(&13).is_some_and(|k| k.contains(&1))
            && s.streams.get(&15).is_some_and(|k| k.contains(&10))
    });
    let a = |id: &str| seen.answers.get(id).cloned().unwrap_or_default();
    assert_eq!(a("cmd"), "approval.grant_required", "{:?}", seen.answers);
    for id in [
        "cmd-term",
        "cmd-preview",
        "dyn-start",
        "dyn-launch",
        "dyn-stop",
        "dyn-auth",
        "dyn-empty",
    ] {
        assert_eq!(
            a(id),
            "protocol.invalid_message/stream_id",
            "{id}: {:?}",
            seen.answers
        );
    }
    for stream in [3, 5, 7, 9, 11] {
        assert!(!seen.streams.contains_key(&stream), "{:?}", seen.streams);
    }
    // The subscriptions work: a terminal snapshot (kind 1) and a chat snapshot (kind 10) on their streams.
    assert_eq!(a("term"), "ok");
    assert_eq!(a("chat"), "ok");
    assert!(
        seen.streams.get(&13).is_some_and(|k| k.contains(&1)),
        "{:?}",
        seen.streams
    );
    assert!(
        seen.streams.get(&15).is_some_and(|k| k.contains(&10)),
        "{:?}",
        seen.streams
    );
    // A preview stream is allowed there; the node has no preview yet, so it says so.
    assert_eq!(a("preview"), "protocol.capability_missing");

    std::thread::sleep(Duration::from_millis(500));
    assert!(!e.marker.exists(), "the stub ran");
    assert_eq!(local_agents(&socket), vec![shell.agent_id.clone()]);
    stop(&socket, &shell.agent_id);
}

/// ADR-0041: a subscription the policy lists needs a live grant on a dynamic stream, as it does on stream 1.
#[test]
fn a_listed_subscription_needs_a_grant_on_a_dynamic_stream_too() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let e = env("gate", "terminal.subscribe, chat.subscribe");
    let n = node(&e);
    let socket = e.dir.join("c.sock");
    let shell = start_shell(&socket);
    let term = || {
        Cmd::TerminalSubscribe(v1::TerminalSubscribe {
            terminal_id: shell.terminal_id.clone(),
            format: "terminal.raw".into(),
            history_lines: 0,
        })
    };
    let script = vec![
        (3, req("term", term())),
        (
            5,
            req(
                "chat",
                Cmd::ChatSubscribe(v1::ChatSubscribe {
                    agent_id: shell.agent_id.clone(),
                }),
            ),
        ),
    ];
    let seen = hub(&n, &socket, script, |_| true);
    assert_eq!(
        seen.answers.get("term").map(String::as_str),
        Some("approval.grant_required"),
        "{:?}",
        seen.answers
    );
    assert_eq!(
        seen.answers.get("chat").map(String::as_str),
        Some("approval.grant_required"),
        "{:?}",
        seen.answers
    );
    assert!(seen.streams.is_empty(), "{:?}", seen.streams);
    stop(&socket, &shell.agent_id);
}

/// ADR-0109: until this node keeps a durable handoff inbox, the hub's delivery is refused as a missing capability where the hub's
/// commands enter, before a `signed_commands` line that covers it (`mesh.*`) could ask for a grant on it.
#[test]
fn a_handoff_delivery_is_refused_before_the_signed_command_policy() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let e = env("handoff", "mesh.*");
    let n = node(&e);
    let socket = e.dir.join("c.sock");
    let script = vec![(
        STREAM_COMMANDS,
        req(
            "handoff",
            Cmd::DeliverTaskHandoff(v1::DeliverTaskHandoff::default()),
        ),
    )];
    let seen = hub(&n, &socket, script, |_| true);
    assert_eq!(
        seen.answers.get("handoff").map(String::as_str),
        Some("protocol.capability_missing"),
        "{:?}",
        seen.answers
    );
}

/// The control socket holds to the same rule for a local client: a command that is not a subscription on a dynamic stream is
/// refused there, and a chat subscription on stream 1 too (it needs a stream of its own).
#[test]
fn the_control_socket_refuses_a_misplaced_command() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let e = env("local", "agent.start");
    let _n = node(&e);
    let socket = e.dir.join("c.sock");
    let shell = start_shell(&socket);
    let mut c = Client::connect(&socket).unwrap();
    let code = |r: std::io::Result<Result<v1::CommandOk, v1::ErrorInfo>>| match r.unwrap() {
        Ok(_) => "ok".to_string(),
        Err(e) => e.code,
    };
    assert_eq!(
        code(c.call_on(
            3,
            Cmd::AgentStart(v1::AgentStart {
                kind: "stub".into(),
                ..Default::default()
            })
        )),
        "protocol.invalid_message"
    );
    assert_eq!(
        code(c.call_on(
            4,
            Cmd::AgentStop(v1::AgentStop {
                agent_id: shell.agent_id.clone(),
                stop_helpers: false,
            })
        )),
        "protocol.invalid_message"
    );
    assert_eq!(
        code(c.call(Cmd::ChatSubscribe(v1::ChatSubscribe {
            agent_id: shell.agent_id.clone(),
        }))),
        "protocol.invalid_message"
    );
    assert_eq!(
        code(c.call_on(
            5,
            Cmd::ChatSubscribe(v1::ChatSubscribe {
                agent_id: shell.agent_id.clone(),
            })
        )),
        "ok"
    );
    assert_eq!(
        code(c.call(Cmd::PreviewConnect(v1::PreviewConnect {
            forward_id: "f".into(),
        }))),
        "protocol.invalid_message"
    );
    // A request with no command on a dynamic stream is refused for the stream, before anything looks at the command.
    let empty = CommandRequest {
        request_id: "empty".into(),
        ..Default::default()
    };
    let r = c.request_on(6, &empty, Duration::from_secs(10)).unwrap();
    assert_eq!(outcome(&r), "protocol.invalid_message/stream_id");
    // `ControlAuth` only proves the connection, so a local client may send it on any stream (an empty token when the node asks for
    // none).
    let token =
        agora_node::control_token::read_token(&agora_node::control_token::token_path(&socket))
            .unwrap_or_default();
    assert_eq!(
        code(c.call_on(7, Cmd::ControlAuth(v1::ControlAuth { token }))),
        "ok"
    );
    std::thread::sleep(Duration::from_millis(500));
    assert!(!e.marker.exists(), "the stub ran");
    assert_eq!(local_agents(&socket), vec![shell.agent_id.clone()]);
    stop(&socket, &shell.agent_id);
}

/// A device the node trusts, and grants it signs for this node (ADR-0041).
struct Owner(ring::signature::EcdsaKeyPair);

impl Owner {
    fn new(e: &Env) -> Self {
        use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng)
            .unwrap();
        let hex: String = key
            .public_key()
            .as_ref()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        std::fs::write(
            e.dir
                .join("data")
                .join(agora_node::approvals::TRUSTED_DEVICES_FILE),
            format!("owner {hex}\n"),
        )
        .unwrap();
        Owner(key)
    }

    fn grant(&self, id: &str, scope: &str, commands: &[&str]) -> CommandRequest {
        let commands: Vec<String> = commands.iter().map(|c| (*c).to_string()).collect();
        let expires = agora_node::grants::now_ms() + 3_600_000;
        let message = agora_node::grants::signed_message("laptop", id, scope, &commands, expires);
        let signature = self
            .0
            .sign(&ring::rand::SystemRandom::new(), &message)
            .unwrap()
            .as_ref()
            .to_vec();
        req(
            id,
            Cmd::GrantCreate(v1::GrantCreate {
                grant_id: id.into(),
                scope: scope.into(),
                commands,
                expires_at_ms: expires,
                signer_device_id: "owner".into(),
                signature,
            }),
        )
    }
}

fn watch(id: &str, terminal_id: &str) -> CommandRequest {
    req(
        id,
        Cmd::TerminalSubscribe(v1::TerminalSubscribe {
            terminal_id: terminal_id.into(),
            format: "terminal.raw".into(),
            history_lines: 0,
        }),
    )
}

fn follow(id: &str, agent_id: &str) -> CommandRequest {
    req(
        id,
        Cmd::ChatSubscribe(v1::ChatSubscribe {
            agent_id: agent_id.into(),
        }),
    )
}

/// ADR-0041: watching names its agent, so a grant for that agent lets a window watch it (and no other), and a machine-wide grant
/// still covers every agent.
#[test]
fn a_grant_for_one_agent_lets_a_window_watch_that_agent_only() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let e = env("scope", "terminal.*, chat.subscribe");
    let owner = Owner::new(&e);
    let n = node(&e);
    let socket = e.dir.join("c.sock");
    let (a, b) = (start_shell(&socket), start_shell(&socket));
    let script = vec![
        (
            STREAM_COMMANDS,
            owner.grant(
                "g-agent",
                &format!("agent:{}", a.agent_id),
                &["terminal.*", "chat.subscribe"],
            ),
        ),
        (3, watch("term-a", &a.terminal_id)),
        (5, watch("term-b", &b.terminal_id)),
        (7, follow("chat-a", &a.agent_id)),
        (9, follow("chat-b", &b.agent_id)),
        (
            STREAM_COMMANDS,
            owner.grant("g-node", "node", &["terminal.subscribe"]),
        ),
        (11, watch("term-b-node", &b.terminal_id)),
    ];
    let seen = hub(&n, &socket, script, |s| {
        [(3, 1), (7, 10), (11, 1)]
            .iter()
            .all(|(st, k)| s.streams.get(st).is_some_and(|ks| ks.contains(k)))
    });
    let got = |id: &str| seen.answers.get(id).cloned().unwrap_or_default();
    for id in ["g-agent", "term-a", "chat-a", "g-node", "term-b-node"] {
        assert_eq!(got(id), "ok", "{id}: {:?}", seen.answers);
    }
    for id in ["term-b", "chat-b"] {
        assert_eq!(
            got(id),
            "approval.grant_required",
            "{id}: {:?}",
            seen.answers
        );
    }
    assert!(
        seen.streams.get(&3).is_some_and(|k| k.contains(&1)),
        "{:?}",
        seen.streams
    );
    assert!(
        seen.streams.get(&7).is_some_and(|k| k.contains(&10)),
        "{:?}",
        seen.streams
    );
    assert!(
        seen.streams.get(&11).is_some_and(|k| k.contains(&1)),
        "{:?}",
        seen.streams
    );
    assert!(
        !seen.streams.contains_key(&5) && !seen.streams.contains_key(&9),
        "{:?}",
        seen.streams
    );
    stop(&socket, &a.agent_id);
    stop(&socket, &b.agent_id);
}

/// A stream the node has open takes no second subscription (it would take the first one's place and leave it running); once closed,
/// or refused, the stream may be used again.
#[test]
fn a_second_subscription_on_an_open_stream_is_refused() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let e = env("again", "chat.subscribe");
    let n = node(&e);
    let socket = e.dir.join("c.sock");
    let a = start_shell(&socket);
    let steps = vec![
        Step::Ask(3, watch("first", &a.terminal_id)),
        Step::Ask(3, watch("again", &a.terminal_id)),
        Step::Ask(3, follow("again-chat", &a.agent_id)),
        Step::Close(3),
        Step::Ask(3, watch("reopened", &a.terminal_id)),
        // Refused for want of a grant: the stream is not kept.
        Step::Ask(5, follow("refused", &a.agent_id)),
        Step::Ask(5, watch("after-refusal", &a.terminal_id)),
    ];
    let seen = hub_steps(&n, &socket, steps, |s| {
        s.streams
            .get(&3)
            .is_some_and(|k| k.iter().filter(|&&x| x == 1).count() >= 2)
            && s.streams.get(&5).is_some_and(|k| k.contains(&1))
    });
    let got = |id: &str| seen.answers.get(id).cloned().unwrap_or_default();
    assert_eq!(got("first"), "ok", "{:?}", seen.answers);
    assert_eq!(
        got("again"),
        "protocol.invalid_message/stream_id",
        "{:?}",
        seen.answers
    );
    assert_eq!(
        got("again-chat"),
        "protocol.invalid_message/stream_id",
        "{:?}",
        seen.answers
    );
    assert_eq!(got("reopened"), "ok", "{:?}", seen.answers);
    assert_eq!(
        got("refused"),
        "approval.grant_required",
        "{:?}",
        seen.answers
    );
    assert_eq!(got("after-refusal"), "ok", "{:?}", seen.answers);
    // Every relay stopped with the connection: none was left behind to hold it up.
    assert!(
        seen.stopped_in < Duration::from_secs(5),
        "{:?}",
        seen.stopped_in
    );
    stop(&socket, &a.agent_id);
}
