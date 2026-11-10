//! The hub's network conversations over real sockets: a node streaming events (acks, gaps, resume), a client with a
//! token (snapshot, live, backlog), enrollment, and the full TLS path where the certificate decides who a peer is.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use agora_hub::ca::{new_device_request, Ca};
use agora_hub::enroll::{Enrollment, Lockout, MAX_FAILURES};
use agora_hub::hub::Hub;
use agora_hub::server::{self, ClientAuth, ServerState};
use agora_hub::store::{EventStore, SqliteStore};
use agora_hub::tls;
use agora_protocol::frame::{kinds, STREAM_COMMANDS, STREAM_CONTROL, STREAM_EVENTS};
use agora_protocol::handshake::Support;
use agora_protocol::v1::{self, command_ok, command_request, command_response, event::Body};
use agora_protocol::ws::{self, Conn};
use agora_protocol::Frame;
use prost::Message;
use tokio::net::{TcpListener, TcpStream};

struct Tokens;

impl ClientAuth for Tokens {
    fn check(&self, token: &str) -> bool {
        token == "good-token"
    }
}

fn support() -> Support {
    Support {
        generations: vec![1],
        capabilities: vec![],
        required: vec![],
    }
}

fn state(ca: Option<Ca>) -> Arc<ServerState> {
    Arc::new(ServerState {
        hub: Arc::new(Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap()),
        support: support(),
        ca,
        lockout: Arc::new(Lockout::new()),
        auth: Box::new(Tokens),
        assets: None,
        sessions: None,
    })
}

fn f(stream: u32, kind: u8, payload: Vec<u8>) -> Frame {
    Frame {
        stream_id: stream,
        kind,
        seq: None,
        payload,
    }
}

fn event(node: &str, seq: u64, body: Body) -> Vec<u8> {
    v1::Event {
        producer_id: node.into(),
        seq,
        at_ms: seq,
        body: Some(body),
    }
    .encode_to_vec()
}

fn online(node: &str) -> Body {
    Body::NodeOnline(v1::NodeOnline {
        node_id: node.into(),
        hostname: "h".into(),
        os: "linux".into(),
        build: "b".into(),
    })
}

fn agent(id: &str) -> Body {
    Body::AgentStarted(v1::AgentStarted {
        agent_id: id.into(),
        project_id: "p".into(),
        node_id: "n".into(),
        worktree_id: "w".into(),
        kind: "shell".into(),
        level: 2,
    })
}

async fn recv(c: &mut Conn<TcpStream>) -> Frame {
    tokio::time::timeout(Duration::from_secs(10), c.recv())
        .await
        .expect("timeout")
        .unwrap()
        .expect("closed")
}

/// A hub on a loopback port without TLS (the token and enrollment paths).
async fn start(state: Arc<ServerState>) -> std::net::SocketAddr {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(server::run(l, state, None));
    addr
}

async fn dial(addr: std::net::SocketAddr, token: Option<&str>) -> Conn<TcpStream> {
    let s = TcpStream::connect(addr).await.unwrap();
    let url = format!("ws://{addr}/");
    match token {
        Some(t) => ws::connect_with_token(s, &url, t).await.unwrap(),
        None => ws::connect(s, &url).await.unwrap(),
    }
}

/// Drives `serve_node` for `node` over a socket pair, as the TLS layer would after reading the certificate.
async fn node_conn(
    state: &Arc<ServerState>,
    node: &'static str,
) -> (Conn<TcpStream>, tokio::task::JoinHandle<()>) {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let st = Arc::clone(state);
    let task = tokio::spawn(async move {
        let (s, _) = l.accept().await.unwrap();
        let (conn, _) = ws::accept_with_token(s).await.unwrap();
        let _ = server::serve_node(conn, node, &st.hub, &st.support).await;
    });
    (dial(addr, None).await, task)
}

async fn handshake(c: &mut Conn<TcpStream>) -> v1::ResumeOk {
    let hello = v1::Hello {
        role: v1::Role::Node as i32,
        protocol_generations: vec![1],
        capabilities: vec![],
        build: "t".into(),
        locale: None,
    };
    c.send(&f(STREAM_CONTROL, kinds::HELLO, hello.encode_to_vec()))
        .await
        .unwrap();
    let w = recv(c).await;
    assert_eq!((w.stream_id, w.kind), (STREAM_CONTROL, kinds::WELCOME));
    c.send(&f(
        STREAM_CONTROL,
        kinds::RESUME,
        v1::Resume::default().encode_to_vec(),
    ))
    .await
    .unwrap();
    let r = recv(c).await;
    assert_eq!((r.stream_id, r.kind), (STREAM_CONTROL, kinds::RESUME_OK));
    v1::ResumeOk::decode(r.payload.as_slice()).unwrap()
}

async fn next_ack(c: &mut Conn<TcpStream>) -> u64 {
    loop {
        let fr = recv(c).await;
        if (fr.stream_id, fr.kind) == (STREAM_EVENTS, kinds::EVENT_ACK) {
            return v1::EventAck::decode(fr.payload.as_slice())
                .unwrap()
                .up_to_seq;
        }
    }
}

async fn closed(c: &mut Conn<TcpStream>) -> bool {
    matches!(
        tokio::time::timeout(Duration::from_secs(5), c.recv()).await,
        Ok(Ok(None) | Err(_))
    )
}

#[tokio::test]
async fn a_node_streams_events_gets_acks_and_goes_offline_when_it_leaves() {
    let st = state(None);
    let (mut c, task) = node_conn(&st, "laptop").await;
    assert_eq!(handshake(&mut c).await.from_seq, 1);
    for (i, b) in [online("laptop"), agent("agent-1")].into_iter().enumerate() {
        c.send(&f(
            STREAM_EVENTS,
            kinds::EVENT,
            event("laptop", i as u64 + 1, b),
        ))
        .await
        .unwrap();
    }
    assert_eq!(next_ack(&mut c).await, 2);
    let (view, pos) = st.hub.view();
    assert_eq!(pos, 2);
    assert!(view.nodes["laptop"].online);
    assert!(view.agents.contains_key("laptop/agent-1"));
    c.close().await;
    task.await.unwrap();
    assert!(
        !st.hub.view().0.nodes["laptop"].online,
        "recorded offline by the hub"
    );
}

#[tokio::test]
async fn a_gap_is_answered_with_where_to_resume_and_a_reconnect_starts_after_what_is_stored() {
    let st = state(None);
    let (mut c, task) = node_conn(&st, "n").await;
    handshake(&mut c).await;
    c.send(&f(STREAM_EVENTS, kinds::EVENT, event("n", 1, online("n"))))
        .await
        .unwrap();
    c.send(&f(STREAM_EVENTS, kinds::EVENT, event("n", 3, agent("a"))))
        .await
        .unwrap(); // 2 is missing
    let mut from = None;
    while from.is_none() {
        let fr = recv(&mut c).await;
        if (fr.stream_id, fr.kind) == (STREAM_CONTROL, kinds::RESUME_OK) {
            from = Some(
                v1::ResumeOk::decode(fr.payload.as_slice())
                    .unwrap()
                    .from_seq,
            );
        }
    }
    assert_eq!(from, Some(2));
    c.close().await;
    task.await.unwrap();
    let (mut c2, _t) = node_conn(&st, "n").await;
    assert_eq!(handshake(&mut c2).await.from_seq, 2);
}

#[tokio::test]
async fn a_node_cannot_use_another_nodes_producer_id() {
    let st = state(None);
    let (mut c, task) = node_conn(&st, "honest").await;
    handshake(&mut c).await;
    c.send(&f(
        STREAM_EVENTS,
        kinds::EVENT,
        event("victim", 1, online("victim")),
    ))
    .await
    .unwrap();
    // The session refuses and the connection ends; nothing was stored for the victim.
    task.await.unwrap();
    assert_eq!(st.hub.store().acked("victim").unwrap(), 0);
    assert!(!st.hub.view().0.nodes.contains_key("victim"));
}

#[tokio::test]
async fn a_client_needs_a_token_then_gets_a_snapshot_and_live_events_and_can_resume() {
    let st = state(None);
    let addr = start(Arc::clone(&st)).await;
    let (mut node, _t) = node_conn(&st, "n1").await;
    handshake(&mut node).await;
    node.send(&f(
        STREAM_EVENTS,
        kinds::EVENT,
        event("n1", 1, online("n1")),
    ))
    .await
    .unwrap();
    next_ack(&mut node).await;

    for tok in [None, Some("wrong")] {
        let mut c = dial(addr, tok).await;
        c.send(&f(
            STREAM_EVENTS,
            kinds::SUBSCRIBE,
            v1::Subscribe::default().encode_to_vec(),
        ))
        .await
        .unwrap();
        assert!(closed(&mut c).await, "{tok:?}");
    }

    let mut c = dial(addr, Some("good-token")).await;
    c.send(&f(
        STREAM_EVENTS,
        kinds::SUBSCRIBE,
        v1::Subscribe::default().encode_to_vec(),
    ))
    .await
    .unwrap();
    let snap = recv(&mut c).await;
    assert_eq!(
        (snap.stream_id, snap.kind),
        (STREAM_EVENTS, kinds::SNAPSHOT)
    );
    let snap = v1::Snapshot::decode(snap.payload.as_slice()).unwrap();
    assert_eq!((snap.pos, snap.nodes.len()), (1, 1));
    assert_eq!(snap.nodes[0].node_id, "n1");

    node.send(&f(
        STREAM_EVENTS,
        kinds::EVENT,
        event("n1", 2, agent("agent-1")),
    ))
    .await
    .unwrap();
    let live = recv(&mut c).await;
    assert_eq!(live.kind, kinds::POSITIONED_EVENT);
    assert_eq!(
        v1::PositionedEvent::decode(live.payload.as_slice())
            .unwrap()
            .pos,
        2
    );

    let mut c2 = dial(addr, Some("good-token")).await;
    c2.send(&f(
        STREAM_EVENTS,
        kinds::SUBSCRIBE,
        v1::Subscribe { after_pos: Some(1) }.encode_to_vec(),
    ))
    .await
    .unwrap();
    let fr = recv(&mut c2).await;
    assert_eq!(fr.kind, kinds::POSITIONED_EVENT);
    assert_eq!(
        v1::PositionedEvent::decode(fr.payload.as_slice())
            .unwrap()
            .pos,
        2
    );
}

fn enroll_request(code: &str, node: &str, csr: &str) -> Frame {
    let req = v1::CommandRequest {
        request_id: "r1".into(),
        idempotency_key: String::new(),
        target_node: String::new(),
        command: Some(command_request::Command::Enroll(v1::EnrollRequest {
            code: code.into(),
            node_id: node.into(),
            csr_pem: csr.into(),
        })),
    };
    f(STREAM_COMMANDS, 0, req.encode_to_vec())
}

async fn enroll(
    addr: std::net::SocketAddr,
    code: &str,
    node: &str,
    csr: &str,
) -> v1::CommandResponse {
    let mut c = dial(addr, None).await;
    c.send(&enroll_request(code, node, csr)).await.unwrap();
    let fr = recv(&mut c).await;
    assert_eq!((fr.stream_id, fr.kind), (STREAM_COMMANDS, 1));
    v1::CommandResponse::decode(fr.payload.as_slice()).unwrap()
}

fn err_code(r: &v1::CommandResponse) -> &str {
    match &r.result {
        Some(command_response::Result::Err(e)) => &e.code,
        _ => "",
    }
}

#[tokio::test]
async fn enrollment_issues_a_certificate_once_and_locks_out_guessing() {
    let ca = Ca::generate("hub ca").unwrap();
    let ca_pem = ca.cert_pem().to_string();
    let st = state(Some(ca));
    let code = Enrollment::new(st.hub.store())
        .create_code(server::now_ms())
        .unwrap();
    let addr = start(Arc::clone(&st)).await;
    let (_, csr) = new_device_request().unwrap();

    assert_eq!(
        err_code(&enroll(addr, "AAAA-AAAA", "laptop", &csr).await),
        "auth.unauthenticated"
    );
    let r = enroll(addr, &code, "laptop", &csr).await;
    let Some(command_response::Result::Ok(v1::CommandOk {
        result: Some(command_ok::Result::Enroll(e)),
    })) = r.result
    else {
        panic!("{r:?}")
    };
    assert_eq!(e.ca_pem, ca_pem);
    assert!(e.cert_pem.contains("BEGIN CERTIFICATE"));
    assert_eq!(
        err_code(&enroll(addr, &code, "laptop2", &csr).await),
        "auth.unauthenticated",
        "spent"
    );

    let fresh = Enrollment::new(st.hub.store())
        .create_code(server::now_ms())
        .unwrap();
    for _ in 0..MAX_FAILURES {
        enroll(addr, "BBBB-BBBB", "x", &csr).await;
    }
    assert_eq!(
        err_code(&enroll(addr, &fresh, "x", &csr).await),
        "auth.forbidden"
    );
}

#[tokio::test]
async fn an_unauthenticated_peer_can_do_nothing_but_enroll() {
    let st = state(Some(Ca::generate("ca").unwrap()));
    let addr = start(Arc::clone(&st)).await;
    let mut c = dial(addr, None).await;
    let req = v1::CommandRequest {
        request_id: "r".into(),
        idempotency_key: String::new(),
        target_node: String::new(),
        command: Some(command_request::Command::ProjectList(v1::ProjectList {})),
    };
    c.send(&f(STREAM_COMMANDS, 0, req.encode_to_vec()))
        .await
        .unwrap();
    let fr = recv(&mut c).await;
    assert_eq!(
        err_code(&v1::CommandResponse::decode(fr.payload.as_slice()).unwrap()),
        "auth.unauthenticated"
    );
    let mut c = dial(addr, None).await;
    c.send(&f(STREAM_EVENTS, kinds::EVENT, event("n", 1, online("n"))))
        .await
        .unwrap();
    assert!(closed(&mut c).await);
    assert!(st.hub.view().0.nodes.is_empty());
}

// ---- the full TLS path ----

type TlsConn = Conn<tokio_rustls::client::TlsStream<TcpStream>>;

fn client_tls(trust: &Ca, cert_key: Option<(String, String)>) -> tokio_rustls::TlsConnector {
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut roots = rustls::RootCertStore::empty();
    for c in CertificateDer::pem_slice_iter(trust.cert_pem().as_bytes()) {
        roots.add(c.unwrap()).unwrap();
    }
    let b = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots);
    let cfg = match cert_key {
        None => b.with_no_client_auth(),
        Some((cert, key)) => {
            let chain = CertificateDer::pem_slice_iter(cert.as_bytes())
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            b.with_client_auth_cert(
                chain,
                PrivateKeyDer::from_pem_slice(key.as_bytes()).unwrap(),
            )
            .unwrap()
        }
    };
    tokio_rustls::TlsConnector::from(Arc::new(cfg))
}

async fn dial_tls(addr: std::net::SocketAddr, connector: tokio_rustls::TlsConnector) -> TlsConn {
    let tcp = TcpStream::connect(addr).await.unwrap();
    let name = rustls::pki_types::ServerName::try_from("localhost").unwrap();
    let tls = connector.connect(name, tcp).await.unwrap();
    ws::connect(tls, &format!("wss://localhost:{}/", addr.port()))
        .await
        .unwrap()
}

async fn recv_tls(c: &mut TlsConn) -> Frame {
    tokio::time::timeout(Duration::from_secs(10), c.recv())
        .await
        .expect("timeout")
        .unwrap()
        .expect("closed")
}

#[tokio::test]
async fn over_tls_the_certificate_decides_who_a_peer_is_and_revocation_is_immediate() {
    let ca = Ca::generate("hub ca").unwrap();
    let (server_cert, server_key) = ca.issue_server(&["localhost".to_string()]).unwrap();
    let acceptor = tls::acceptor_with(&server_cert, &server_key, ca.cert_pem(), true).unwrap();
    let trust = Ca::from_pem(ca.cert_pem(), &ca.key_pem()).unwrap();
    let st = state(Some(ca));
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(server::run(l, Arc::clone(&st), Some(acceptor)));

    // 1. Enroll over TLS with no client certificate.
    let code = Enrollment::new(st.hub.store())
        .create_code(server::now_ms())
        .unwrap();
    let (key_pem, csr) = new_device_request().unwrap();
    let mut c = dial_tls(addr, client_tls(&trust, None)).await;
    c.send(&enroll_request(&code, "tls-node", &csr))
        .await
        .unwrap();
    let r = v1::CommandResponse::decode(recv_tls(&mut c).await.payload.as_slice()).unwrap();
    let Some(command_response::Result::Ok(v1::CommandOk {
        result: Some(command_ok::Result::Enroll(e)),
    })) = r.result
    else {
        panic!("{r:?}")
    };

    // 2. Reconnect with the certificate: now it is a node, named by the certificate.
    let mut node = dial_tls(
        addr,
        client_tls(&trust, Some((e.cert_pem.clone(), key_pem.clone()))),
    )
    .await;
    let hello = v1::Hello {
        role: v1::Role::Node as i32,
        protocol_generations: vec![1],
        capabilities: vec![],
        build: "t".into(),
        locale: None,
    };
    node.send(&f(STREAM_CONTROL, kinds::HELLO, hello.encode_to_vec()))
        .await
        .unwrap();
    let w = recv_tls(&mut node).await;
    assert_eq!(
        v1::Welcome::decode(w.payload.as_slice()).unwrap().peer_id,
        "tls-node"
    );
    node.send(&f(
        STREAM_CONTROL,
        kinds::RESUME,
        v1::Resume::default().encode_to_vec(),
    ))
    .await
    .unwrap();
    recv_tls(&mut node).await;
    node.send(&f(
        STREAM_EVENTS,
        kinds::EVENT,
        event("tls-node", 1, online("tls-node")),
    ))
    .await
    .unwrap();
    assert_eq!(recv_tls(&mut node).await.kind, kinds::EVENT_ACK);
    assert!(st.hub.view().0.nodes["tls-node"].online);
    drop(node);

    // 3. Revoke: the very next connection with that certificate is refused.
    Enrollment::new(st.hub.store())
        .revoke("tls-node", server::now_ms())
        .unwrap();
    let mut again = dial_tls(addr, client_tls(&trust, Some((e.cert_pem, key_pem)))).await;
    again
        .send(&f(STREAM_CONTROL, kinds::HELLO, hello.encode_to_vec()))
        .await
        .unwrap();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(5), again.recv()).await,
        Ok(Ok(None) | Err(_))
    ));
}

#[tokio::test]
async fn the_same_port_serves_the_web_app_and_takes_a_browser_style_subscription() {
    use agora_hub::web::{Asset, Assets};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    struct App;
    impl Assets for App {
        fn get(&self, p: &str) -> Option<Asset> {
            (p == "/").then(|| Asset {
                content_type: "text/html; charset=utf-8",
                body: b"<html>app</html>".to_vec(),
            })
        }
    }
    let st = Arc::new(ServerState {
        hub: Arc::new(Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap()),
        support: support(),
        ca: None,
        lockout: Arc::new(Lockout::new()),
        auth: Box::new(Tokens),
        assets: Some(Arc::new(App)),
        sessions: None,
    });
    let addr = start(st).await;

    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    assert!(out.starts_with("HTTP/1.1 200"), "{out}");
    assert!(out.ends_with("<html>app</html>"));

    // A browser cannot set headers: the token rides in the subprotocol list.
    let tcp = TcpStream::connect(addr).await.unwrap();
    let mut c = ws::connect_with_subprotocol_token(tcp, &format!("ws://{addr}/"), "good-token")
        .await
        .unwrap();
    c.send(&f(
        STREAM_EVENTS,
        kinds::SUBSCRIBE,
        v1::Subscribe::default().encode_to_vec(),
    ))
    .await
    .unwrap();
    let first = recv(&mut c).await;
    assert_eq!(first.kind, kinds::SNAPSHOT);

    let tcp = TcpStream::connect(addr).await.unwrap();
    let mut bad = ws::connect_with_subprotocol_token(tcp, &format!("ws://{addr}/"), "nope")
        .await
        .unwrap();
    bad.send(&f(
        STREAM_EVENTS,
        kinds::SUBSCRIBE,
        v1::Subscribe::default().encode_to_vec(),
    ))
    .await
    .unwrap();
    assert!(closed(&mut bad).await);
}

fn routed(id: &str, target: &str) -> Frame {
    let req = v1::CommandRequest {
        request_id: id.into(),
        idempotency_key: String::new(),
        target_node: target.into(),
        command: Some(command_request::Command::AgentList(v1::AgentList::default())),
    };
    f(STREAM_COMMANDS, 0, req.encode_to_vec())
}

async fn response(c: &mut Conn<TcpStream>) -> v1::CommandResponse {
    let fr = recv(c).await;
    assert_eq!((fr.stream_id, fr.kind), (STREAM_COMMANDS, 1));
    v1::CommandResponse::decode(fr.payload.as_slice()).unwrap()
}

#[tokio::test]
async fn a_clients_command_is_routed_to_its_node_and_the_answer_comes_back() {
    let st = state(None);
    let addr = start(Arc::clone(&st)).await;
    let (mut node, _t) = node_conn(&st, "n1").await;
    handshake(&mut node).await;
    node.send(&f(
        STREAM_EVENTS,
        kinds::EVENT,
        event("n1", 1, online("n1")),
    ))
    .await
    .unwrap();
    next_ack(&mut node).await;

    let mut client = dial(addr, Some("good-token")).await;
    client
        .send(&f(
            STREAM_EVENTS,
            kinds::SUBSCRIBE,
            v1::Subscribe::default().encode_to_vec(),
        ))
        .await
        .unwrap();
    assert_eq!(recv(&mut client).await.kind, kinds::SNAPSHOT);

    // Unknown node, no target, and enrollment are refused by the hub itself.
    for (id, target, code) in [("a", "nope", "node.unknown"), ("b", "", "node.unknown")] {
        client.send(&routed(id, target)).await.unwrap();
        let r = response(&mut client).await;
        assert_eq!(r.request_id, id);
        assert_eq!(err_code(&r), code);
    }

    client.send(&routed("mine", "n1")).await.unwrap();
    let got = recv(&mut node).await;
    assert_eq!((got.stream_id, got.kind), (STREAM_COMMANDS, 0));
    let forwarded = v1::CommandRequest::decode(got.payload.as_slice()).unwrap();
    assert_ne!(forwarded.request_id, "mine", "the hub picks the id");
    assert!(matches!(
        forwarded.command,
        Some(command_request::Command::AgentList(_))
    ));
    let answer = v1::CommandResponse {
        request_id: forwarded.request_id,
        result: Some(command_response::Result::Ok(v1::CommandOk {
            result: Some(command_ok::Result::AgentList(v1::AgentListResult::default())),
        })),
    };
    node.send(&f(STREAM_COMMANDS, 1, answer.encode_to_vec()))
        .await
        .unwrap();
    let r = response(&mut client).await;
    assert_eq!(r.request_id, "mine", "mapped back to the client's id");
    assert!(matches!(r.result, Some(command_response::Result::Ok(_))));
}

#[tokio::test]
async fn a_node_that_leaves_mid_command_gives_the_client_node_offline() {
    let st = state(None);
    let addr = start(Arc::clone(&st)).await;
    let (mut node, task) = node_conn(&st, "n2").await;
    handshake(&mut node).await;
    node.send(&f(
        STREAM_EVENTS,
        kinds::EVENT,
        event("n2", 1, online("n2")),
    ))
    .await
    .unwrap();
    next_ack(&mut node).await;
    let mut client = dial(addr, Some("good-token")).await;
    client
        .send(&f(
            STREAM_EVENTS,
            kinds::SUBSCRIBE,
            v1::Subscribe::default().encode_to_vec(),
        ))
        .await
        .unwrap();
    assert_eq!(recv(&mut client).await.kind, kinds::SNAPSHOT);
    client.send(&routed("x", "n2")).await.unwrap();
    let _ = recv(&mut node).await; // the node got it and never answers
    drop(node);
    let _ = task.await;
    loop {
        let fr = recv(&mut client).await;
        if (fr.stream_id, fr.kind) == (STREAM_COMMANDS, 1) {
            let r = v1::CommandResponse::decode(fr.payload.as_slice()).unwrap();
            assert_eq!(r.request_id, "x");
            assert_eq!(err_code(&r), "node.offline");
            break;
        }
    }
    // And with the node gone, a new command is refused at once.
    client.send(&routed("y", "n2")).await.unwrap();
    loop {
        let fr = recv(&mut client).await;
        if (fr.stream_id, fr.kind) == (STREAM_COMMANDS, 1) {
            let r = v1::CommandResponse::decode(fr.payload.as_slice()).unwrap();
            assert_eq!(err_code(&r), "node.offline");
            break;
        }
    }
}

async fn post_session(addr: std::net::SocketAddr, code: &str) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let body = format!("{{\"code\":\"{code}\"}}");
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(
        format!(
            "POST /api/session HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .as_bytes(),
    )
    .await
    .unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    let status = out[9..12].parse().unwrap();
    let body = out.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    (status, body)
}

#[tokio::test]
async fn signing_in_with_a_code_gives_a_session_token_the_hub_accepts() {
    use agora_hub::signin::{HubSessions, SignIn};
    let hub = Arc::new(Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap());
    let sessions = Arc::new(HubSessions::new(Arc::clone(&hub)));
    let st = Arc::new(ServerState {
        hub: Arc::clone(&hub),
        support: support(),
        ca: None,
        lockout: Arc::new(Lockout::new()),
        auth: Box::new(HubSessionsAuth(Arc::clone(&sessions))),
        assets: None,
        sessions: Some(sessions),
    });
    let addr = start(st).await;
    let code = SignIn::new(hub.store(), Arc::new(Lockout::new()))
        .create_code(server::now_ms())
        .unwrap();

    let (status, body) = post_session(addr, &code).await;
    assert_eq!(status, 200, "{body}");
    let token = body
        .split("\"token\":\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap()
        .to_string();
    // The code is spent.
    assert_eq!(post_session(addr, &code).await.0, 401);

    // The token opens a client connection; a made-up one does not.
    let tcp = TcpStream::connect(addr).await.unwrap();
    let mut c = ws::connect_with_subprotocol_token(tcp, &format!("ws://{addr}/"), &token)
        .await
        .unwrap();
    c.send(&f(
        STREAM_EVENTS,
        kinds::SUBSCRIBE,
        v1::Subscribe::default().encode_to_vec(),
    ))
    .await
    .unwrap();
    assert_eq!(recv(&mut c).await.kind, kinds::SNAPSHOT);
    let mut bad = dial(addr, Some("0123456789abcdef0123456789abcdef")).await;
    bad.send(&f(
        STREAM_EVENTS,
        kinds::SUBSCRIBE,
        v1::Subscribe::default().encode_to_vec(),
    ))
    .await
    .unwrap();
    assert!(closed(&mut bad).await);

    // Guessing is rate limited.
    for _ in 0..6 {
        let _ = post_session(addr, "AAAA-AAAA").await;
    }
    assert_eq!(post_session(addr, "AAAA-AAAA").await.0, 429);
}

struct HubSessionsAuth(Arc<agora_hub::signin::HubSessions>);

impl ClientAuth for HubSessionsAuth {
    fn check(&self, token: &str) -> bool {
        self.0.check(token)
    }
}

async fn post_json(
    addr: std::net::SocketAddr,
    path: &str,
    bearer: Option<&str>,
    body: &serde_json::Value,
) -> (u16, serde_json::Value) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let body = body.to_string();
    let auth = bearer.map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"));
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(
        format!(
            "POST {path} HTTP/1.1\r\nHost: hub.test:7443\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .as_bytes(),
    )
    .await
    .unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    let status = out[9..12].parse().unwrap();
    let json = serde_json::from_str(out.split("\r\n\r\n").nth(1).unwrap_or("")).unwrap_or_default();
    (status, json)
}

#[tokio::test]
async fn a_passkey_added_by_a_signed_in_owner_signs_in_later_without_a_code() {
    use agora_hub::passkey::{b64url_encode as b64, SPKI_P256_PREFIX};
    use agora_hub::signin::{HubSessions, SignIn};
    use ring::rand::SystemRandom;
    use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_ASN1_SIGNING};
    use sha2::{Digest, Sha256};
    let hub = Arc::new(Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap());
    let sessions = Arc::new(HubSessions::new(Arc::clone(&hub)));
    let st = Arc::new(ServerState {
        hub: Arc::clone(&hub),
        support: support(),
        ca: None,
        lockout: Arc::new(Lockout::new()),
        auth: Box::new(HubSessionsAuth(Arc::clone(&sessions))),
        assets: None,
        sessions: Some(sessions.clone()),
    });
    let addr = start(st).await;
    let rng = SystemRandom::new();
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
    let pair =
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng).unwrap();
    let origin = "https://hub.test:7443";

    // Adding a passkey needs a session.
    let none = serde_json::json!({});
    assert_eq!(
        post_json(addr, "/api/passkey/register-begin", None, &none)
            .await
            .0,
        401
    );
    let code = SignIn::new(hub.store(), Arc::new(Lockout::new()))
        .create_code(server::now_ms())
        .unwrap();
    let (_, body) = post_json(
        addr,
        "/api/session",
        None,
        &serde_json::json!({ "code": code }),
    )
    .await;
    let session = body["token"].as_str().unwrap().to_string();

    let (status, begin) =
        post_json(addr, "/api/passkey/register-begin", Some(&session), &none).await;
    assert_eq!(status, 200);
    assert_eq!(begin["rpId"], "hub.test");
    let cd = serde_json::json!({"type": "webauthn.create", "challenge": begin["challenge"], "origin": origin}).to_string();
    let mut spki = SPKI_P256_PREFIX.to_vec();
    spki.extend_from_slice(pair.public_key().as_ref());
    let reg = serde_json::json!({"id": "cred_1", "publicKey": b64(&spki), "clientDataJSON": b64(cd.as_bytes())});
    assert_eq!(
        post_json(addr, "/api/passkey/register-finish", Some(&session), &reg)
            .await
            .0,
        200
    );

    // Sign in with the passkey alone.
    let (status, begin) = post_json(addr, "/api/passkey/login-begin", None, &none).await;
    assert_eq!(status, 200);
    assert_eq!(begin["ids"], serde_json::json!(["cred_1"]));
    let cd = serde_json::json!({"type": "webauthn.get", "challenge": begin["challenge"], "origin": origin}).to_string();
    let mut auth = Sha256::digest(b"hub.test").to_vec();
    auth.extend_from_slice(&[0x05, 0, 0, 0, 1]);
    let mut signed = auth.clone();
    signed.extend_from_slice(&Sha256::digest(cd.as_bytes()));
    let sig = pair.sign(&rng, &signed).unwrap();
    let assertion = serde_json::json!({"id": "cred_1", "authenticatorData": b64(&auth),
        "clientDataJSON": b64(cd.as_bytes()), "signature": b64(sig.as_ref())});
    let (status, body) = post_json(addr, "/api/passkey/login-finish", None, &assertion).await;
    assert_eq!(status, 200, "{body}");
    let token = body["token"].as_str().unwrap();
    assert!(sessions.check(token));

    // The same assertion cannot be replayed: its challenge is spent.
    assert_eq!(
        post_json(addr, "/api/passkey/login-finish", None, &assertion)
            .await
            .0,
        401
    );
}

#[tokio::test]
async fn a_client_asks_the_hub_for_usage_and_gets_tokens_with_a_price_only_where_one_is_known() {
    let st = state(None);
    let addr = start(Arc::clone(&st)).await;
    let (mut node, _t) = node_conn(&st, "n1").await;
    handshake(&mut node).await;
    let usage = |agent: &str, model: &str| {
        Body::AgentUsageReported(v1::AgentUsageReported {
            agent_id: agent.into(),
            model: model.into(),
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
        })
    };
    let bodies = vec![
        online("n1"),
        agent("a1"),
        agent("a2"),
        usage("a1", "anthropic/claude-haiku-4-5"),
        usage("a2", "no-such-model"),
    ];
    for (i, b) in bodies.into_iter().enumerate() {
        let seq = i as u64 + 1;
        node.send(&f(STREAM_EVENTS, kinds::EVENT, event("n1", seq, b)))
            .await
            .unwrap();
        next_ack(&mut node).await;
    }
    let mut client = dial(addr, Some("good-token")).await;
    client
        .send(&f(
            STREAM_EVENTS,
            kinds::SUBSCRIBE,
            v1::Subscribe::default().encode_to_vec(),
        ))
        .await
        .unwrap();
    assert_eq!(recv(&mut client).await.kind, kinds::SNAPSHOT);

    let ask = |target: &str| {
        let req = v1::CommandRequest {
            request_id: "u".into(),
            idempotency_key: String::new(),
            target_node: target.into(),
            command: Some(command_request::Command::UsageReport(v1::UsageReport {})),
        };
        f(STREAM_COMMANDS, 0, req.encode_to_vec())
    };
    client.send(&ask("")).await.unwrap();
    let r = response(&mut client).await;
    let Some(command_response::Result::Ok(v1::CommandOk {
        result: Some(command_ok::Result::UsageReport(report)),
    })) = r.result
    else {
        panic!("not a usage report: {r:?}");
    };
    assert_eq!(report.agents.len(), 2);
    let priced = report
        .agents
        .iter()
        .find(|a| a.agent_id == "n1/a1")
        .unwrap();
    let unpriced = report
        .agents
        .iter()
        .find(|a| a.agent_id == "n1/a2")
        .unwrap();
    assert!(priced.micro_usd.is_some_and(|c| c > 0), "{priced:?}");
    assert_eq!(unpriced.micro_usd, None, "no price, no guess");
    assert_eq!(report.total_micro_usd, priced.micro_usd.unwrap());

    // A usage report that names a node is not a routed command.
    client.send(&ask("n1")).await.unwrap();
    assert_eq!(
        err_code(&response(&mut client).await),
        "protocol.invalid_message"
    );
}

fn mesh_post(id: &str, to: &str, origin: &str) -> Frame {
    let req = v1::CommandRequest {
        request_id: id.into(),
        idempotency_key: String::new(),
        target_node: String::new(),
        command: Some(command_request::Command::MeshPost(v1::MeshPost {
            to: to.into(),
            body: "hi".into(),
            origin: origin.into(),
            from_agent: "a1".into(),
            refs: vec!["src/lib.rs".into(), "commit:abc".into()],
        })),
    };
    f(STREAM_COMMANDS, 0, req.encode_to_vec())
}

#[tokio::test]
async fn a_nodes_mesh_post_is_routed_with_the_origin_the_hub_saw_and_nothing_else_is_accepted_from_a_node(
) {
    let st = state(None);
    let (mut n1, _t1) = node_conn(&st, "n1").await;
    handshake(&mut n1).await;
    n1.send(&f(
        STREAM_EVENTS,
        kinds::EVENT,
        event("n1", 1, online("n1")),
    ))
    .await
    .unwrap();
    next_ack(&mut n1).await;
    let (mut n2, _t2) = node_conn(&st, "n2").await;
    handshake(&mut n2).await;
    n2.send(&f(
        STREAM_EVENTS,
        kinds::EVENT,
        event("n2", 1, online("n2")),
    ))
    .await
    .unwrap();
    next_ack(&mut n2).await;

    // n1 claims to be n9; the hub writes n1. The target sees its own agent id, not the `n2/` prefix.
    n1.send(&mesh_post("p1", "n2/b1", "n9")).await.unwrap();
    let got = recv(&mut n2).await;
    assert_eq!((got.stream_id, got.kind), (STREAM_COMMANDS, 0));
    let forwarded = v1::CommandRequest::decode(got.payload.as_slice()).unwrap();
    let Some(command_request::Command::MeshPost(p)) = forwarded.command else {
        panic!("not a mesh post");
    };
    assert_eq!(
        (p.to.as_str(), p.origin.as_str(), p.from_agent.as_str()),
        ("b1", "n1", "a1")
    );
    assert_eq!(
        p.refs,
        ["src/lib.rs", "commit:abc"],
        "refs travel as written; the receiver qualifies them"
    );
    let answer = v1::CommandResponse {
        request_id: forwarded.request_id,
        result: Some(command_response::Result::Ok(v1::CommandOk::default())),
    };
    n2.send(&f(STREAM_COMMANDS, 1, answer.encode_to_vec()))
        .await
        .unwrap();
    let r = response(&mut n1).await;
    assert_eq!(r.request_id, "p1", "the answer carries the id n1 chose");
    assert!(matches!(r.result, Some(command_response::Result::Ok(_))));

    // Refusals: an unknown node, itself, a bare id, and any other command.
    for (id, to, code) in [
        ("p2", "nowhere/x", "node.unknown"),
        ("p3", "n1/a2", "protocol.invalid_message"),
        ("p4", "bare", "protocol.invalid_message"),
    ] {
        n1.send(&mesh_post(id, to, "")).await.unwrap();
        let r = response(&mut n1).await;
        assert_eq!((r.request_id.as_str(), err_code(&r)), (id, code));
    }
    n1.send(&routed("p5", "n2")).await.unwrap();
    let r = response(&mut n1).await;
    assert_eq!(
        (r.request_id.as_str(), err_code(&r)),
        ("p5", "protocol.invalid_message")
    );
}

struct Switch(Arc<std::sync::atomic::AtomicBool>);

impl ClientAuth for Switch {
    fn check(&self, _: &str) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[tokio::test]
async fn a_live_client_connection_ends_when_its_session_stops_being_valid() {
    std::env::set_var("AGORA_RECHECK_MS", "100");
    let valid = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let st = Arc::new(ServerState {
        hub: Arc::new(Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap()),
        support: support(),
        ca: None,
        lockout: Arc::new(Lockout::new()),
        auth: Box::new(Switch(Arc::clone(&valid))),
        assets: None,
        sessions: None,
    });
    let addr = start(st).await;
    let mut c = dial(addr, Some("any")).await;
    c.send(&f(
        STREAM_EVENTS,
        kinds::SUBSCRIBE,
        v1::Subscribe::default().encode_to_vec(),
    ))
    .await
    .unwrap();
    assert_eq!(recv(&mut c).await.kind, kinds::SNAPSHOT);
    // Expiry or "sign out everywhere": the connection that was already open goes too.
    valid.store(false, std::sync::atomic::Ordering::SeqCst);
    assert!(closed(&mut c).await);
}

/// A token whose person's role can be changed while the connection is open (ADR-0049).
struct Demotable(Arc<std::sync::Mutex<agora_hub::users::Role>>);

impl ClientAuth for Demotable {
    fn check(&self, token: &str) -> bool {
        token == "good-token"
    }
    fn access(&self, token: &str) -> Option<agora_hub::users::Access> {
        self.check(token).then(|| agora_hub::users::Access {
            role: *self.0.lock().unwrap(),
            user: "eda".into(),
            projects: None,
        })
    }
}

fn stop_request(id: &str, target: &str) -> Frame {
    let req = v1::CommandRequest {
        request_id: id.into(),
        idempotency_key: String::new(),
        target_node: target.into(),
        command: Some(command_request::Command::AgentStop(v1::AgentStop::default())),
    };
    f(STREAM_COMMANDS, 0, req.encode_to_vec())
}

#[tokio::test]
async fn a_viewer_can_read_but_every_command_that_changes_something_stops_at_the_hub() {
    use agora_hub::users::Role;
    let role = Arc::new(std::sync::Mutex::new(Role::Viewer));
    let st = Arc::new(ServerState {
        hub: Arc::new(Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap()),
        support: support(),
        ca: None,
        lockout: Arc::new(Lockout::new()),
        auth: Box::new(Demotable(Arc::clone(&role))),
        assets: None,
        sessions: None,
    });
    let addr = start(Arc::clone(&st)).await;
    let (mut node, _t) = node_conn(&st, "n1").await;
    handshake(&mut node).await;
    node.send(&f(
        STREAM_EVENTS,
        kinds::EVENT,
        event("n1", 1, online("n1")),
    ))
    .await
    .unwrap();
    next_ack(&mut node).await;
    let mut client = dial(addr, Some("good-token")).await;
    client
        .send(&f(
            STREAM_EVENTS,
            kinds::SUBSCRIBE,
            v1::Subscribe::default().encode_to_vec(),
        ))
        .await
        .unwrap();
    assert_eq!(recv(&mut client).await.kind, kinds::SNAPSHOT);

    // A viewer's stop is refused by the hub, naming what was refused; the node never hears of it.
    client.send(&stop_request("s1", "n1")).await.unwrap();
    let r = response(&mut client).await;
    assert_eq!(
        (r.request_id.as_str(), err_code(&r)),
        ("s1", "auth.forbidden")
    );
    let Some(command_response::Result::Err(info)) = &r.result else {
        panic!("expected an error");
    };
    assert!(format!("{info:?}").contains("AgentStop"), "{info:?}");

    // Reading goes through.
    client.send(&routed("r1", "n1")).await.unwrap();
    let got = recv(&mut node).await;
    let forwarded = v1::CommandRequest::decode(got.payload.as_slice()).unwrap();
    assert!(matches!(
        forwarded.command,
        Some(command_request::Command::AgentList(_))
    ));

    // A role that changes ends the open connection (the client reconnects into what it may now do). As a member, a stop goes through.
    *role.lock().unwrap() = Role::Member;
    client.send(&stop_request("s2", "n1")).await.unwrap();
    assert!(closed(&mut client).await);
    let mut client = dial(addr, Some("good-token")).await;
    client
        .send(&f(
            STREAM_EVENTS,
            kinds::SUBSCRIBE,
            v1::Subscribe::default().encode_to_vec(),
        ))
        .await
        .unwrap();
    assert_eq!(recv(&mut client).await.kind, kinds::SNAPSHOT);
    client.send(&stop_request("s3", "n1")).await.unwrap();
    let got = recv(&mut node).await;
    let forwarded = v1::CommandRequest::decode(got.payload.as_slice()).unwrap();
    assert!(matches!(
        forwarded.command,
        Some(command_request::Command::AgentStop(_))
    ));
    *role.lock().unwrap() = Role::Viewer;
    client.send(&stop_request("s4", "n1")).await.unwrap();
    assert!(closed(&mut client).await);

    // Everything above is in the audit trail with who asked, what, where and how it ended; a refusal is recorded too.
    let rows = agora_hub::audit::Audit::new(st.hub.store())
        .list(0, Some("eda"), 100)
        .unwrap();
    let seen: Vec<(&str, &str, &str)> = rows
        .iter()
        .map(|e| (e.action.as_str(), e.node.as_str(), e.outcome.as_str()))
        .collect();
    assert_eq!(
        seen,
        [
            ("AgentStop", "n1", "auth.forbidden"),
            ("AgentList", "n1", "sent"),
            ("AgentStop", "n1", "sent"),
        ]
    );
}

/// A member who was granted one project (ADR-0049).
struct Scoped;

impl ClientAuth for Scoped {
    fn check(&self, token: &str) -> bool {
        token == "good-token"
    }
    fn access(&self, token: &str) -> Option<agora_hub::users::Access> {
        self.check(token).then(|| agora_hub::users::Access {
            role: agora_hub::users::Role::Member,
            user: "eda".into(),
            projects: Some(["n1/mine".to_string()].into()),
        })
    }
}

fn started(id: &str, project: &str) -> Body {
    Body::AgentStarted(v1::AgentStarted {
        agent_id: id.into(),
        project_id: project.into(),
        node_id: "n1".into(),
        worktree_id: String::new(),
        kind: "shell".into(),
        level: 2,
    })
}

fn working(id: &str) -> Body {
    Body::AgentStateChanged(v1::AgentStateChanged {
        agent_id: id.into(),
        state: v1::AgentState::Working as i32,
        reported: false,
    })
}

fn stop_of(id: &str, agent: &str) -> Frame {
    let req = v1::CommandRequest {
        request_id: id.into(),
        idempotency_key: String::new(),
        target_node: "n1".into(),
        command: Some(command_request::Command::AgentStop(v1::AgentStop {
            agent_id: agent.into(),
        })),
    };
    f(STREAM_COMMANDS, 0, req.encode_to_vec())
}

#[tokio::test]
async fn a_member_never_receives_another_projects_snapshot_events_or_commands() {
    let st = Arc::new(ServerState {
        hub: Arc::new(Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap()),
        support: support(),
        ca: None,
        lockout: Arc::new(Lockout::new()),
        auth: Box::new(Scoped),
        assets: None,
        sessions: None,
    });
    let addr = start(Arc::clone(&st)).await;
    let (mut node, _t) = node_conn(&st, "n1").await;
    handshake(&mut node).await;
    for (seq, body) in [
        (1, online("n1")),
        (2, started("a1", "mine")),
        (3, started("a2", "theirs")),
    ] {
        node.send(&f(STREAM_EVENTS, kinds::EVENT, event("n1", seq, body)))
            .await
            .unwrap();
        next_ack(&mut node).await;
    }

    let mut client = dial(addr, Some("good-token")).await;
    // Asking to replay the log from the start must not replay anyone else's events: the answer is a cut snapshot.
    client
        .send(&f(
            STREAM_EVENTS,
            kinds::SUBSCRIBE,
            v1::Subscribe { after_pos: Some(0) }.encode_to_vec(),
        ))
        .await
        .unwrap();
    let fr = recv(&mut client).await;
    assert_eq!(fr.kind, kinds::SNAPSHOT);
    let snap = v1::Snapshot::decode(fr.payload.as_slice()).unwrap();
    let agents: Vec<&str> = snap.agents.iter().map(|a| a.agent_id.as_str()).collect();
    assert_eq!(agents, ["n1/a1"], "{snap:?}");

    // Live: the other project's change is not sent, this one's is (and it is the next frame the client gets).
    for (seq, body) in [(4, working("a2")), (5, working("a1"))] {
        node.send(&f(STREAM_EVENTS, kinds::EVENT, event("n1", seq, body)))
            .await
            .unwrap();
        next_ack(&mut node).await;
    }
    let fr = recv(&mut client).await;
    assert_eq!(fr.kind, kinds::POSITIONED_EVENT);
    let ev = v1::PositionedEvent::decode(fr.payload.as_slice()).unwrap();
    let Some(Body::AgentStateChanged(e)) = ev.event.and_then(|e| e.body) else {
        panic!("expected a state change");
    };
    assert_eq!(e.agent_id, "a1");

    // Commands: the other project's agent and anything machine-wide are refused at the hub; their own goes through.
    for (id, frame) in [
        ("x1", stop_of("x1", "a2")),
        ("x2", routed("x2", "n1")), // AgentList across the machine
    ] {
        client.send(&frame).await.unwrap();
        let r = response(&mut client).await;
        assert_eq!(
            (r.request_id.as_str(), err_code(&r)),
            (id, "auth.forbidden")
        );
    }
    client.send(&stop_of("ok", "a1")).await.unwrap();
    let got = recv(&mut node).await;
    let forwarded = v1::CommandRequest::decode(got.payload.as_slice()).unwrap();
    assert!(
        matches!(&forwarded.command, Some(command_request::Command::AgentStop(s)) if s.agent_id == "a1")
    );
}
