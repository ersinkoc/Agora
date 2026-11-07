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
        // Closing the stream tells the node to stop writing on it.
        client.send(&f(5, STREAM_CLOSE, Vec::new())).await.unwrap();
    });
    drop(node);
}
