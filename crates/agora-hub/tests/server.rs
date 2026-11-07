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
