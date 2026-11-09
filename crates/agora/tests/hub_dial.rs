//! A node dials a real hub over mTLS and its events appear in the hub's view: the Phase 3 vertical slice with real
//! sockets, certificates and the durable buffer, in one process. Nodes are autonomous: this also covers a hub that is
//! not there yet when the node starts.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use agora_core::AgentId;
use agora_hub::ca::{new_device_request, Ca};
use agora_hub::enroll::{Enrollment, Lockout};
use agora_hub::hub::Hub;
use agora_hub::server::{self, ClientAuth, ServerState};
use agora_hub::store::SqliteStore;
use agora_hub::tls;
use agora_node::daemon::EventSink;
use agora_node::uplink_client::{spawn, HubLink};
use agora_protocol::handshake::Support;

/// Accepts one client token (the tests' only signed-in user).
struct NoClients;

impl ClientAuth for NoClients {
    fn check(&self, t: &str) -> bool {
        t == "client-token"
    }
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("agora-dial-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

struct Hubside {
    rt: tokio::runtime::Runtime,
    state: Arc<ServerState>,
    ca_pem: String,
    cert_pem: String,
    key_pem: String,
    acceptor: tokio_rustls::TlsAcceptor,
}

fn hubside(node_id: &str) -> Hubside {
    let ca = Ca::generate("hub ca").unwrap();
    let (server_cert, server_key) = ca.issue_server(&["localhost".to_string()]).unwrap();
    let acceptor = tls::acceptor_with(&server_cert, &server_key, ca.cert_pem(), true).unwrap();
    let store = SqliteStore::open_in_memory().unwrap();
    // Enroll the node directly (the enrollment conversation itself is tested in the hub crate).
    let mut enrollment = Enrollment::new(&store);
    let code = enrollment.create_code(server::now_ms()).unwrap();
    let (key_pem, csr) = new_device_request().unwrap();
    let issued = enrollment
        .redeem(&ca, &code, node_id, &csr, server::now_ms())
        .unwrap();
    let ca_pem = ca.cert_pem().to_string();
    let state = Arc::new(ServerState {
        hub: Arc::new(Hub::open(store).unwrap()),
        support: Support {
            generations: vec![1],
            capabilities: vec!["terminal.raw".into()],
            required: vec![],
        },
        ca: Some(ca),
        lockout: Arc::new(Lockout::new()),
        auth: Box::new(NoClients),
        assets: None,
        sessions: None,
    });
    Hubside {
        rt: tokio::runtime::Runtime::new().unwrap(),
        state,
        ca_pem,
        cert_pem: issued.cert_pem,
        key_pem,
        acceptor,
    }
}

impl Hubside {
    /// A certificate for one more node (the hub's own CA signs it, as enrollment would).
    fn enroll(&self, node_id: &str) -> (String, String) {
        let (key_pem, csr) = new_device_request().unwrap();
        let mut e = Enrollment::new(self.state.hub.store());
        let code = e.create_code(server::now_ms()).unwrap();
        let ca = self.state.ca.as_ref().unwrap();
        let issued = e
            .redeem(ca, &code, node_id, &csr, server::now_ms())
            .unwrap();
        (issued.cert_pem, key_pem)
    }

    fn listen(&self, addr: &str) -> SocketAddr {
        let l = self
            .rt
            .block_on(tokio::net::TcpListener::bind(addr))
            .unwrap();
        let bound = l.local_addr().unwrap();
        self.rt.spawn(server::run(
            l,
            Arc::clone(&self.state),
            Some(self.acceptor.clone()),
        ));
        bound
    }

    fn link(&self, addr: SocketAddr) -> HubLink {
        HubLink {
            mesh_out: None,
            url: format!("wss://localhost:{}", addr.port()),
            ca_pem: self.ca_pem.clone(),
            cert_pem: self.cert_pem.clone(),
            key_pem: self.key_pem.clone(),
            commands: None,
        }
    }

    /// A signed-in client's WebSocket over TLS, trusting only the hub's CA.
    fn client(
        &self,
        addr: SocketAddr,
    ) -> agora_protocol::ws::Conn<tokio_rustls::client::TlsStream<tokio::net::TcpStream>> {
        use rustls::pki_types::pem::PemObject;
        use rustls::pki_types::{CertificateDer, ServerName};
        self.rt.block_on(async {
            let mut roots = rustls::RootCertStore::empty();
            for c in CertificateDer::pem_slice_iter(self.ca_pem.as_bytes()) {
                roots.add(c.unwrap()).unwrap();
            }
            let cfg = rustls::ClientConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
            let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
            let tls = tokio_rustls::TlsConnector::from(Arc::new(cfg))
                .connect(ServerName::try_from("localhost").unwrap(), tcp)
                .await
                .unwrap();
            agora_protocol::ws::connect_with_subprotocol_token(
                tls,
                &format!("wss://localhost:{}/", addr.port()),
                "client-token",
            )
            .await
            .unwrap()
        })
    }
}

fn wait(what: &str, mut ok: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(20);
    while !ok() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn sink_with_agent(dir: &Path, node: &str) -> Arc<EventSink> {
    let sink = Arc::new(EventSink::open(dir, node).unwrap());
    sink.node_online(node);
    sink.checkout_discovered(node, "proj", Path::new("/work/proj"));
    sink.agent_launched(
        &AgentId::new("agent-1"),
        "proj",
        node,
        Path::new("/work/proj"),
        Path::new("/work/wt"),
        "agent/x",
        "shell",
        true,
    );
    sink
}

#[test]
fn a_node_dials_the_hub_and_its_agents_appear_in_the_view_then_new_events_follow() {
    let hub = hubside("laptop");
    let addr = hub.listen("127.0.0.1:0");
    let sink = sink_with_agent(&tmp("live"), "laptop");
    let _link = spawn(Arc::clone(&sink), hub.link(addr)).unwrap();

    wait("the node to appear", || {
        hub.state.hub.view().0.agents.contains_key("laptop/agent-1")
    });
    let (view, _) = hub.state.hub.view();
    assert!(view.nodes["laptop"].online);
    assert_eq!(view.worktrees.len(), 1);
    // The hub acknowledged: the node's buffer knows what is safe.
    wait("the ack to reach the node", || {
        sink.acked() == sink.next_seq() - 1
    });

    // Something new on the node shows up live.
    sink.node_resources("laptop", 3, 3);
    wait("the resource event", || {
        hub.state.hub.view().0.nodes["laptop"].agents_running == 3
    });
}

#[test]
fn a_node_keeps_working_without_the_hub_and_catches_up_when_it_appears() {
    let hub = hubside("server");
    // Reserve a port, then release it so the node's first attempts are refused.
    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = probe.local_addr().unwrap();
    drop(probe);
    let sink = sink_with_agent(&tmp("late"), "server");
    let _link = spawn(Arc::clone(&sink), hub.link(addr)).unwrap();
    // Events keep being recorded locally while nobody listens.
    sink.node_resources("server", 1, 1);
    std::thread::sleep(Duration::from_millis(800));
    assert!(hub.state.hub.view().0.nodes.is_empty());
    assert_eq!(sink.acked(), 0);

    hub.listen(&addr.to_string());
    wait("the backlog to arrive", || {
        hub.state.hub.view().0.agents.contains_key("server/agent-1")
    });
    wait("everything to be acknowledged", || {
        sink.acked() == sink.next_seq() - 1
    });
}

#[test]
fn a_certificate_the_hub_does_not_know_never_gets_in() {
    let hub = hubside("honest");
    let addr = hub.listen("127.0.0.1:0");
    // A node holding a certificate from the same CA that the hub never enrolled (not in its device registry).
    let (key_pem, csr) = new_device_request().unwrap();
    let ca = hub.state.ca.as_ref().unwrap();
    let rogue = ca.issue_node("rogue", &csr).unwrap();
    let mut link = hub.link(addr);
    link.cert_pem = rogue.cert_pem;
    link.key_pem = key_pem;
    let sink = sink_with_agent(&tmp("rogue"), "rogue");
    let _l = spawn(Arc::clone(&sink), link).unwrap();
    std::thread::sleep(Duration::from_secs(2));
    assert!(hub.state.hub.view().0.nodes.is_empty());
    assert_eq!(sink.acked(), 0);
}

#[test]
fn a_signed_in_client_runs_a_command_on_a_node_through_the_hub() {
    // Routed commands run on the node's local socket, which this platform does not have yet.
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    use agora_protocol::frame::{kinds, STREAM_COMMANDS, STREAM_EVENTS};
    use agora_protocol::v1::{self, command_request, command_response};
    use agora_protocol::Frame;
    use prost::Message;

    let hub = hubside("laptop");
    let addr = hub.listen("127.0.0.1:0");
    // A real node: its control socket is what routed commands run on.
    let dir = tmp("routed");
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = agora_node::daemon::NodeConfig {
        node_id: "laptop".into(),
        socket: dir.join("c.sock"),
        data_dir: dir.join("data"),
        worktree_root: dir.join("wt"),
        manifests_dir: dir.join("manifests"),
        projects: vec![],
        limits: Default::default(),
    };
    let node = agora_node::daemon::Node::start(&cfg).unwrap();
    let mut link = hub.link(addr);
    link.commands = Some(cfg.socket.clone());
    let _uplink = spawn(Arc::clone(&node.events), link).unwrap();
    wait("the node to be online in the hub", || {
        hub.state
            .hub
            .view()
            .0
            .nodes
            .get("laptop")
            .is_some_and(|n| n.online)
    });

    let mut client = hub.client(addr);
    let f = |stream, kind, payload| Frame {
        stream_id: stream,
        kind,
        seq: None,
        payload,
    };
    let answer = hub.rt.block_on(async {
        client
            .send(&f(
                STREAM_EVENTS,
                kinds::SUBSCRIBE,
                v1::Subscribe::default().encode_to_vec(),
            ))
            .await
            .unwrap();
        let req = v1::CommandRequest {
            request_id: "c1".into(),
            idempotency_key: String::new(),
            target_node: "laptop".into(),
            command: Some(command_request::Command::AgentList(v1::AgentList::default())),
        };
        client
            .send(&f(STREAM_COMMANDS, 0, req.encode_to_vec()))
            .await
            .unwrap();
        loop {
            let fr = tokio::time::timeout(Duration::from_secs(20), client.recv())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            if (fr.stream_id, fr.kind) == (STREAM_COMMANDS, 1) {
                break v1::CommandResponse::decode(fr.payload.as_slice()).unwrap();
            }
        }
    });
    assert_eq!(answer.request_id, "c1");
    assert!(
        matches!(answer.result, Some(command_response::Result::Ok(_))),
        "{answer:?}"
    );
    drop(node);
}

async fn next_frame<S>(c: &mut agora_protocol::ws::Conn<S>) -> agora_protocol::Frame
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    tokio::time::timeout(Duration::from_secs(20), c.recv())
        .await
        .expect("timed out")
        .unwrap()
        .expect("closed")
}

fn git(dir: &Path, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

#[test]
fn a_clients_terminal_is_relayed_through_the_hub_with_input_and_close() {
    // Routed commands run on the node's local socket, which this platform does not have yet.
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    use agora_protocol::frame::{kinds, STREAM_CLOSE, STREAM_COMMANDS, STREAM_EVENTS};
    use agora_protocol::v1::{self, command_ok, command_request, command_response};
    use agora_protocol::Frame;
    use prost::Message;

    let hub = hubside("laptop");
    let addr = hub.listen("127.0.0.1:0");
    let dir = tmp("relay");
    let repo = dir.join("myproject");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    std::fs::create_dir_all(dir.join("manifests")).unwrap();
    std::fs::write(
        dir.join("manifests/shell.toml"),
        "kind = \"shell\"\ndisplay_name = \"Shell\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"sh\"]\n",
    )
    .unwrap();
    let cfg = agora_node::daemon::NodeConfig {
        node_id: "laptop".into(),
        socket: dir.join("c.sock"),
        data_dir: dir.join("data"),
        worktree_root: dir.join("wt"),
        manifests_dir: dir.join("manifests"),
        projects: vec![repo.clone()],
        limits: Default::default(),
    };
    let node = agora_node::daemon::Node::start(&cfg).unwrap();
    let mut link = hub.link(addr);
    link.commands = Some(cfg.socket.clone());
    let _uplink = spawn(Arc::clone(&node.events), link).unwrap();
    wait("the node to be online in the hub", || {
        hub.state
            .hub
            .view()
            .0
            .nodes
            .get("laptop")
            .is_some_and(|n| n.online)
    });

    let mut client = hub.client(addr);
    let f = |stream, kind, payload| Frame {
        stream_id: stream,
        kind,
        seq: None,
        payload,
    };
    let routed = |id: &str, command| {
        f(
            STREAM_COMMANDS,
            0,
            v1::CommandRequest {
                request_id: id.into(),
                idempotency_key: String::new(),
                target_node: "laptop".into(),
                command: Some(command),
            }
            .encode_to_vec(),
        )
    };
    hub.rt.block_on(async {
        client
            .send(&f(
                STREAM_EVENTS,
                kinds::SUBSCRIBE,
                v1::Subscribe::default().encode_to_vec(),
            ))
            .await
            .unwrap();
        // Start a shell agent on the node.
        client
            .send(&routed(
                "start",
                command_request::Command::AgentStart(v1::AgentStart {
                    project_id: "myproject".into(),
                    kind: "shell".into(),
                    ..Default::default()
                }),
            ))
            .await
            .unwrap();
        let terminal = loop {
            let fr = next_frame(&mut client).await;
            if (fr.stream_id, fr.kind) == (STREAM_COMMANDS, 1) {
                let r = v1::CommandResponse::decode(fr.payload.as_slice()).unwrap();
                match r.result {
                    Some(command_response::Result::Ok(v1::CommandOk {
                        result: Some(command_ok::Result::AgentStart(s)),
                    })) => break s.terminal_id,
                    other => panic!("start failed: {other:?}"),
                }
            }
        };
        // Subscribe on a stream of the client's choosing.
        client
            .send(&Frame {
                stream_id: 5,
                kind: 0,
                seq: None,
                payload: v1::CommandRequest {
                    request_id: "sub".into(),
                    idempotency_key: String::new(),
                    target_node: "laptop".into(),
                    command: Some(command_request::Command::TerminalSubscribe(
                        v1::TerminalSubscribe {
                            terminal_id: terminal.clone(),
                            format: String::new(),
                        },
                    )),
                }
                .encode_to_vec(),
            })
            .await
            .unwrap();
        let mut saw_snapshot = false;
        let mut got_answer = false;
        while !(saw_snapshot && got_answer) {
            let fr = next_frame(&mut client).await;
            if fr.stream_id == 5 && fr.kind == 1 {
                saw_snapshot = true;
            } else if (fr.stream_id, fr.kind) == (STREAM_COMMANDS, 1) {
                let r = v1::CommandResponse::decode(fr.payload.as_slice()).unwrap();
                if r.request_id == "sub" {
                    assert!(
                        matches!(r.result, Some(command_response::Result::Ok(_))),
                        "{r:?}"
                    );
                    got_answer = true;
                }
            }
        }
        // Type into it (an ordinary routed command) and watch the output arrive on the stream.
        client
            .send(&routed(
                "in",
                command_request::Command::TerminalInput(v1::TerminalInput {
                    terminal_id: terminal.clone(),
                    data: b"echo relayed-$((20+22))\n".to_vec(),
                }),
            ))
            .await
            .unwrap();
        let mut out = Vec::new();
        while !String::from_utf8_lossy(&out).contains("relayed-42") {
            let fr = next_frame(&mut client).await;
            if fr.stream_id == 5 && fr.kind == 0 {
                out.extend_from_slice(&fr.payload);
            }
        }
        // Keystrokes arrive as one command each; the node must run them in the order they were sent, or typing scrambles.
        out.clear();
        for (i, b) in b"echo ordered-0123456789abcdefghijklmnopqrstuv\n"
            .iter()
            .enumerate()
        {
            client
                .send(&routed(
                    &format!("k{i}"),
                    command_request::Command::TerminalInput(v1::TerminalInput {
                        terminal_id: terminal.clone(),
                        data: vec![*b],
                    }),
                ))
                .await
                .unwrap();
        }
        while !String::from_utf8_lossy(&out).contains("ordered-0123456789abcdefghijklmnopqrstuv") {
            let fr = next_frame(&mut client).await;
            if fr.stream_id == 5 && fr.kind == 0 {
                out.extend_from_slice(&fr.payload);
            }
        }
        // Closing the stream tells the node to stop writing on it.
        client.send(&f(5, STREAM_CLOSE, Vec::new())).await.unwrap();
    });
    drop(node);
}

/// One JSON-RPC call on an agent's mesh socket; returns the `result` object.
fn mcp(
    conn: &mut agora_platform::LocalStream,
    id: u64,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    use std::io::{BufRead, BufReader, Write};
    let line = serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
    writeln!(conn, "{line}").unwrap();
    let mut reader = BufReader::new(conn.try_clone().unwrap());
    let mut out = String::new();
    reader.read_line(&mut out).unwrap();
    serde_json::from_str::<serde_json::Value>(&out).unwrap()["result"].clone()
}

#[test]
fn an_agent_messages_an_agent_on_another_node_through_the_hub_and_a_failure_comes_back_as_a_notice()
{
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    use agora_core::AgentId;
    use agora_mesh::broker::AgentInfo;
    use agora_node::uplink_client::MeshOut;
    let hub = hubside("alpha");
    let addr = hub.listen("127.0.0.1:0");
    let start = |name: &str, cert: String, key: String| {
        let dir = tmp(name);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = agora_node::daemon::NodeConfig {
            node_id: name.into(),
            socket: dir.join("c.sock"),
            data_dir: dir.join("data"),
            worktree_root: dir.join("wt"),
            manifests_dir: dir.join("manifests"),
            projects: vec![],
            limits: Default::default(),
        };
        let node = agora_node::daemon::Node::start(&cfg).unwrap();
        let out = MeshOut::new();
        node.mesh.set_remote(out.clone());
        let mut link = hub.link(addr);
        (link.cert_pem, link.key_pem) = (cert, key);
        link.commands = Some(cfg.socket.clone());
        link.mesh_out = Some(out);
        let up = spawn(Arc::clone(&node.events), link).unwrap();
        let agent = AgentId::new(if name == "alpha" { "a1" } else { "b1" });
        let socket = node
            .mesh
            .register(AgentInfo {
                id: agent.clone(),
                kind: "shell".into(),
                node: name.into(),
                project: "p".into(),
                label: String::new(),
                worktree: String::new(),
                state: "idle".into(),
            })
            .unwrap();
        (node, up, agent, socket)
    };
    let (cert, key) = (hub.enroll("alpha"), hub.enroll("beta"));
    let (a, _ua, a1, a_sock) = start("alpha", cert.0, cert.1);
    let (b, _ub, b1, _b_sock) = start("beta", key.0, key.1);
    for n in ["alpha", "beta"] {
        wait("a node to be online", || {
            hub.state
                .hub
                .view()
                .0
                .nodes
                .get(n)
                .is_some_and(|x| x.online)
        });
    }

    let mut conn = agora_platform::local_connect(&a_sock).unwrap();
    mcp(
        &mut conn,
        1,
        "initialize",
        serde_json::json!({"protocolVersion": "2025-03-26"}),
    );
    let call = |conn: &mut agora_platform::LocalStream, id, to: &str, body: &str| {
        mcp(
            conn,
            id,
            "tools/call",
            serde_json::json!({"name": "send_message", "arguments": {"to": to, "body": body, "refs": ["src/auth.rs", "commit:abc123"]}}),
        )
    };
    let sent = call(&mut conn, 2, "beta/b1", "hello from alpha");
    assert_ne!(sent["isError"], true, "{sent}");

    // It arrives in b1's inbox, from alpha/a1: the hub vouched for "alpha".
    wait("the message to arrive on the other node", || {
        !b.mesh
            .mesh()
            .lock()
            .unwrap()
            .inbox(&b1, None, 10)
            .unwrap()
            .is_empty()
    });
    let got = b.mesh.mesh().lock().unwrap().inbox(&b1, None, 10).unwrap();
    assert_eq!(
        (
            got[0].body.as_str(),
            got[0].provenance.from.as_str(),
            got[0].provenance.node.as_str()
        ),
        ("hello from alpha", "alpha/a1", "alpha")
    );
    // Its refs mean something on alpha only: they arrive with alpha's name in front.
    assert_eq!(got[0].refs, ["alpha:src/auth.rs", "alpha:commit:abc123"]);

    // A node the hub does not know, and an agent the target does not have: the sender is told.
    call(&mut conn, 3, "nowhere/x", "lost");
    call(&mut conn, 4, "beta/ghost", "lost");
    wait("both failures to come back", || {
        a.mesh
            .mesh()
            .lock()
            .unwrap()
            .inbox(&a1, None, 10)
            .unwrap()
            .len()
            == 2
    });
    let notices = a.mesh.mesh().lock().unwrap().inbox(&a1, None, 10).unwrap();
    assert!(notices.iter().all(|m| m.kind == "delivery_failed"));
    let codes: Vec<&str> = notices.iter().map(|m| m.body.as_str()).collect();
    assert!(
        codes.contains(&"node.unknown") && codes.contains(&"mesh.recipient_unknown"),
        "{codes:?}"
    );
}

/// ADR-0039: two nodes share a project, so tasks, leases and handoffs are decided on the hub's board.
#[test]
fn agents_on_two_nodes_share_a_board_claims_leases_and_handoffs_and_it_fails_closed_without_the_hub(
) {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    use agora_mesh::broker::AgentInfo;
    use agora_node::uplink_client::MeshOut;
    use serde_json::{json, Value};
    let hub = hubside("alpha");
    let addr = hub.listen("127.0.0.1:0");
    let start = |name: &str, agent: &str, project: &str, cert: String, key: String| {
        let dir = tmp(&format!("board-{name}"));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = agora_node::daemon::NodeConfig {
            node_id: name.into(),
            socket: dir.join("c.sock"),
            data_dir: dir.join("data"),
            worktree_root: dir.join("wt"),
            manifests_dir: dir.join("manifests"),
            projects: vec![],
            limits: Default::default(),
        };
        let node = agora_node::daemon::Node::start(&cfg).unwrap();
        let out = MeshOut::new();
        node.mesh.set_remote(out.clone());
        let mut link = hub.link(addr);
        (link.cert_pem, link.key_pem) = (cert, key);
        link.commands = Some(cfg.socket.clone());
        link.mesh_out = Some(out);
        let up = spawn(Arc::clone(&node.events), link).unwrap();
        node.events
            .checkout_discovered(name, project, Path::new("/work/p"));
        let id = AgentId::new(agent);
        node.events.agent_launched(
            &id,
            project,
            name,
            Path::new("/work/p"),
            Path::new("/work/wt"),
            "agent/x",
            "shell",
            true,
        );
        let socket = node
            .mesh
            .register(AgentInfo {
                id: id.clone(),
                kind: "shell".into(),
                node: name.into(),
                project: project.into(),
                label: String::new(),
                worktree: String::new(),
                state: "idle".into(),
            })
            .unwrap();
        let mut conn = agora_platform::local_connect(&socket).unwrap();
        mcp(
            &mut conn,
            1,
            "initialize",
            json!({"protocolVersion": "2025-03-26"}),
        );
        (node, up, id, conn)
    };
    let (ca, cb) = (hub.enroll("alpha"), hub.enroll("beta"));
    let (a, ua, _a1, mut ac) = start("alpha", "a1", "p", ca.0.clone(), ca.1.clone());
    let (b, _ub, b1, mut bc) = start("beta", "b1", "p", cb.0, cb.1);
    // A project only alpha has: it never reaches the board.
    let (_solo, _us, _s1, mut sc) = {
        let (c2, k2) = hub.enroll("solo");
        start("solo", "s1", "only-here", c2, k2)
    };
    for (node, agent) in [
        ("alpha", "alpha/a1"),
        ("beta", "beta/b1"),
        ("solo", "solo/s1"),
    ] {
        wait(
            &format!("{node} and {agent} to reach the hub: {:?}", {
                let (v, _) = hub.state.hub.view();
                (
                    v.nodes.keys().cloned().collect::<Vec<_>>(),
                    v.agents.keys().cloned().collect::<Vec<_>>(),
                )
            }),
            || {
                let (v, _) = hub.state.hub.view();
                v.nodes.get(node).is_some_and(|n| n.online) && v.agents.contains_key(agent)
            },
        );
    }
    let call = |c: &mut agora_platform::LocalStream, name: &str, args: Value| -> (Value, bool) {
        let r = mcp(c, 9, "tools/call", json!({"name": name, "arguments": args}));
        let text = r["content"][0]["text"]
            .as_str()
            .unwrap_or("null")
            .to_string();
        (
            serde_json::from_str(&text).unwrap_or(Value::Null),
            r["isError"] == true,
        )
    };

    // A task made on alpha is on beta's board; only one of two claims wins.
    let (t, err) = call(
        &mut ac,
        "task_create",
        json!({"title": "port the parser", "depends_on": []}),
    );
    assert!(!err, "{t}");
    let tid = t["task_id"].as_str().unwrap().to_string();
    let (list, _) = call(&mut bc, "task_list", json!({}));
    assert_eq!(list[0]["title"], "port the parser", "{list}");
    let (_, first) = call(&mut bc, "task_claim", json!({"task_id": tid}));
    let (lost, second) = call(&mut ac, "task_claim", json!({"task_id": tid}));
    assert!(!first && second, "{lost}");
    assert_eq!(lost["error"]["code"], "task.not_open");

    // Leases are exclusive across nodes, and name who holds the path.
    let (l, err) = call(
        &mut ac,
        "lease_acquire",
        json!({"paths": ["src/x.rs"], "ttl_s": 60}),
    );
    assert!(!err, "{l}");
    let (held, err) = call(
        &mut bc,
        "lease_acquire",
        json!({"paths": ["src/x.rs", "src/y.rs"], "ttl_s": 60}),
    );
    assert!(err);
    assert_eq!(held["error"]["code"], "lease.held");
    assert_eq!(held["error"]["params"]["holder_agent_id"], "alpha/a1");
    // All or nothing: the failed batch did not leave src/y.rs locked.
    let (y, err) = call(
        &mut ac,
        "lease_acquire",
        json!({"paths": ["src/y.rs"], "ttl_s": 60}),
    );
    assert!(!err, "{y}");
    let (_, err) = call(&mut ac, "lease_release", json!({"lease_id": l["lease_id"]}));
    assert!(!err);
    let (_, err) = call(
        &mut bc,
        "lease_acquire",
        json!({"paths": ["src/x.rs"], "ttl_s": 60}),
    );
    assert!(!err);

    // A handoff moves the task and tells the receiver, in one step; to a node that is gone it changes nothing.
    let (_, err) = call(
        &mut bc,
        "handoff",
        json!({"to": "alpha/a1", "task_id": tid, "summary": "parser is yours", "refs": ["src/parser.rs"]}),
    );
    assert!(!err);
    let (list, _) = call(&mut ac, "task_list", json!({"status": "claimed"}));
    assert_eq!(list[0]["owner"], "alpha/a1", "{list}");
    wait("the handoff message on the receiving node", || {
        !a.mesh
            .mesh()
            .lock()
            .unwrap()
            .inbox(&AgentId::new("a1"), None, 10)
            .unwrap()
            .is_empty()
    });
    let got = a
        .mesh
        .mesh()
        .lock()
        .unwrap()
        .inbox(&AgentId::new("a1"), None, 10)
        .unwrap();
    assert_eq!(got[0].body, "parser is yours");
    assert_eq!(got[0].provenance.from.as_str(), "beta/b1");

    // A `#channel` message reaches the project's agents on the other node, from the sender, with the channel's name.
    let (_, err) = call(
        &mut ac,
        "send_message",
        json!({"to": "#dev", "body": "who has the parser?", "refs": ["src/parser.rs"]}),
    );
    assert!(!err);
    wait("the channel message on the other node", || {
        !b.mesh
            .mesh()
            .lock()
            .unwrap()
            .inbox(&b1, None, 10)
            .unwrap()
            .is_empty()
    });
    let got = b.mesh.mesh().lock().unwrap().inbox(&b1, None, 10).unwrap();
    assert_eq!(got.last().unwrap().body, "[#dev] who has the parser?");
    assert_eq!(got.last().unwrap().provenance.from.as_str(), "alpha/a1");

    // A project on one node is untouched by the board.
    let (t, err) = call(
        &mut sc,
        "task_create",
        json!({"title": "local", "depends_on": []}),
    );
    assert!(!err, "{t}");
    let (list, _) = call(&mut sc, "task_list", json!({}));
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["owner"], Value::Null);

    // Without the hub a shared project's board fails closed and says so; the agent's own tools keep working.
    drop(ua);
    let (e, err) = call(&mut ac, "task_list", json!({}));
    assert!(err, "{e}");
    assert_eq!(e["error"]["code"], "board.unavailable");
    let (_, err) = call(&mut ac, "inbox", json!({}));
    assert!(!err);
    drop((a, b, b1));
}
