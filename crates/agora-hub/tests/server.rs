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
        binary: String::new(),
    })
}

fn agent(node: &str, id: &str) -> Body {
    Body::AgentStarted(v1::AgentStarted {
        run_id: Vec::new(),
        agent_id: id.into(),
        project_id: "p".into(),
        node_id: node.into(),
        worktree_id: "w".into(),
        kind: "shell".into(),
        level: 2,
        parent_agent_id: String::new(),
        same_process: false,
        launch_options: Vec::new(),
        skips_permissions: false,
    })
}

/// Subscribes as a window and reads the hub's first frame, which must be the `Welcome` naming the person's role (ADR-0106). [`recv`]
/// refuses any later one.
async fn subscribe<S>(c: &mut Conn<S>, payload: Vec<u8>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    c.send(&f(STREAM_EVENTS, kinds::SUBSCRIBE, payload))
        .await
        .unwrap();
    let first = tokio::time::timeout(Duration::from_secs(10), c.recv())
        .await
        .expect("timeout")
        .unwrap()
        .expect("closed");
    assert!(client_welcome(&first), "the Welcome comes first: {first:?}");
}

/// A `Welcome` that names the person's role: a window's, which comes once and first ([`subscribe`]). A node's names none.
fn client_welcome(f: &Frame) -> bool {
    (f.stream_id, f.kind) == (STREAM_CONTROL, kinds::WELCOME)
        && v1::Welcome::decode(f.payload.as_slice())
            .is_ok_and(|w| w.person_role() != v1::PersonRole::Unspecified)
}

/// The next frame. A window's `Welcome` (ADR-0106) is read once, first, by [`subscribe`]; another one here is a bug.
async fn recv(c: &mut Conn<TcpStream>) -> Frame {
    let f = tokio::time::timeout(Duration::from_secs(10), c.recv())
        .await
        .expect("timeout")
        .unwrap()
        .expect("closed");
    assert!(!client_welcome(&f), "a second Welcome: {f:?}");
    f
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
    for (i, b) in [online("laptop"), agent("laptop", "agent-1")]
        .into_iter()
        .enumerate()
    {
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
    assert_eq!(pos, 3, "the hub's NodeLink and the node's two events");
    assert!(view.nodes["laptop"].online);
    assert!(view.agents.contains_key("laptop/agent-1"));
    c.close().await;
    task.await.unwrap();
    assert!(!st.hub.view().0.nodes["laptop"].online);
    // ADR-0070: the hub's own presence facts, from this connection: open, then closed.
    assert_eq!(
        links(&st, "laptop"),
        vec![
            (true, v1::NodeLinkReason::Opened, 1),
            (false, v1::NodeLinkReason::Closed, 1)
        ]
    );
}

/// The `NodeLink` facts the hub recorded for `node`, in log order: `(up, reason, generation)`.
fn links(st: &Arc<ServerState>, node: &str) -> Vec<(bool, v1::NodeLinkReason, u64)> {
    let sub = st.hub.subscribe(Some(0)).unwrap();
    sub.backlog
        .iter()
        .filter_map(|d| match &d.event.body {
            Some(Body::NodeLink(l)) if l.node_id == node => {
                assert_eq!(d.event.producer_id, "hub", "only the hub writes presence");
                Some((
                    l.up,
                    v1::NodeLinkReason::try_from(l.reason).unwrap(),
                    l.generation,
                ))
            }
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_replaced_connection_ending_late_does_not_record_the_node_offline() {
    let st = state(None);
    let (mut old, old_task) = node_conn(&st, "laptop").await;
    let from = handshake(&mut old).await.from_seq;
    old.send(&f(
        STREAM_EVENTS,
        kinds::EVENT,
        event("laptop", from, online("laptop")),
    ))
    .await
    .unwrap();
    assert_eq!(next_ack(&mut old).await, from);
    // The machine restarted and is back while the hub still holds its old connection (half-open after a reboot).
    let (mut new, new_task) = node_conn(&st, "laptop").await;
    let from = handshake(&mut new).await.from_seq;
    new.send(&f(
        STREAM_EVENTS,
        kinds::EVENT,
        event("laptop", from, online("laptop")),
    ))
    .await
    .unwrap();
    assert_eq!(next_ack(&mut new).await, from);
    old.close().await;
    old_task.await.unwrap();
    assert!(st.hub.router().is_connected("laptop"));
    assert!(
        st.hub.view().0.nodes["laptop"].online,
        "the live connection keeps it online"
    );
    // The old connection's late end is recorded as `replaced` and changes nothing; the newer connection's `up` stands.
    assert_eq!(
        links(&st, "laptop"),
        vec![
            (true, v1::NodeLinkReason::Opened, 1),
            (true, v1::NodeLinkReason::Opened, 2),
            (false, v1::NodeLinkReason::Replaced, 1),
        ]
    );
    // A window that rebuilds its view from the log agrees.
    let rebuilt = agora_hub::projection::View::rebuild(st.hub.store())
        .unwrap()
        .0;
    assert!(rebuilt.nodes["laptop"].online);
    new.close().await;
    new_task.await.unwrap();
    assert!(!st.hub.view().0.nodes["laptop"].online);
}

/// ADR-0070: a node whose connection ended and that dials again is online again by itself, with no reset of what it runs (a link
/// that goes down and up is not a process start, so no `NodeOnline` is needed or sent).
#[tokio::test]
async fn a_node_that_dials_again_is_online_again_by_itself_and_its_agents_are_untouched() {
    let st = state(None);
    let (mut old, old_task) = node_conn(&st, "laptop").await;
    let from = handshake(&mut old).await.from_seq;
    for (i, b) in [online("laptop"), agent("laptop", "agent-1")]
        .into_iter()
        .enumerate()
    {
        old.send(&f(
            STREAM_EVENTS,
            kinds::EVENT,
            event("laptop", from + i as u64, b),
        ))
        .await
        .unwrap();
    }
    assert_eq!(next_ack(&mut old).await, from + 1);
    old.close().await;
    old_task.await.unwrap();
    let view = st.hub.view().0;
    assert!(!view.nodes["laptop"].online);
    assert!(
        view.agents["laptop/agent-1"].running,
        "offline is not finished"
    );
    let (mut new, new_task) = node_conn(&st, "laptop").await;
    assert_eq!(handshake(&mut new).await.from_seq, from + 2);
    let view = st.hub.view().0;
    assert!(
        view.nodes["laptop"].online,
        "online again on the new connection"
    );
    assert!(view.agents["laptop/agent-1"].running, "nothing was reset");
    assert_eq!(view.nodes["laptop"].link_generation, 2);
    new.close().await;
    new_task.await.unwrap();
}

#[tokio::test]
async fn a_gap_is_answered_with_where_to_resume_and_a_reconnect_starts_after_what_is_stored() {
    let st = state(None);
    let (mut c, task) = node_conn(&st, "n").await;
    handshake(&mut c).await;
    c.send(&f(STREAM_EVENTS, kinds::EVENT, event("n", 1, online("n"))))
        .await
        .unwrap();
    c.send(&f(
        STREAM_EVENTS,
        kinds::EVENT,
        event("n", 3, agent("n", "a")),
    ))
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
        // Refused before any Welcome: no subscription is answered.
        let _ = c
            .send(&f(
                STREAM_EVENTS,
                kinds::SUBSCRIBE,
                v1::Subscribe::default().encode_to_vec(),
            ))
            .await;
        assert!(closed(&mut c).await, "{tok:?}");
    }

    let mut c = dial(addr, Some("good-token")).await;
    subscribe(&mut c, v1::Subscribe::default().encode_to_vec()).await;
    let snap = recv(&mut c).await;
    assert_eq!(
        (snap.stream_id, snap.kind),
        (STREAM_EVENTS, kinds::SNAPSHOT)
    );
    let snap = v1::Snapshot::decode(snap.payload.as_slice()).unwrap();
    // The hub's `NodeLink { up }` (ADR-0070) is position 1, the node's `NodeOnline` 2.
    assert_eq!((snap.pos, snap.nodes.len()), (2, 1));
    assert_eq!(snap.nodes[0].node_id, "n1");

    node.send(&f(
        STREAM_EVENTS,
        kinds::EVENT,
        event("n1", 2, agent("n1", "agent-1")),
    ))
    .await
    .unwrap();
    let live = recv(&mut c).await;
    assert_eq!(live.kind, kinds::POSITIONED_EVENT);
    assert_eq!(
        v1::PositionedEvent::decode(live.payload.as_slice())
            .unwrap()
            .pos,
        3
    );

    let mut c2 = dial(addr, Some("good-token")).await;
    subscribe(
        &mut c2,
        v1::Subscribe { after_pos: Some(2) }.encode_to_vec(),
    )
    .await;
    let fr = recv(&mut c2).await;
    assert_eq!(fr.kind, kinds::POSITIONED_EVENT);
    assert_eq!(
        v1::PositionedEvent::decode(fr.payload.as_slice())
            .unwrap()
            .pos,
        3
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

/// The next frame. A window's `Welcome` (ADR-0106) is read once, first, by [`subscribe`]; another one here is a bug.
async fn recv_tls(c: &mut TlsConn) -> Frame {
    let f = tokio::time::timeout(Duration::from_secs(10), c.recv())
        .await
        .expect("timeout")
        .unwrap()
        .expect("closed");
    assert!(!client_welcome(&f), "a second Welcome: {f:?}");
    f
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
    subscribe(&mut c, v1::Subscribe::default().encode_to_vec()).await;
    let first = recv(&mut c).await;
    assert_eq!(first.kind, kinds::SNAPSHOT);

    let tcp = TcpStream::connect(addr).await.unwrap();
    let mut bad = ws::connect_with_subprotocol_token(tcp, &format!("ws://{addr}/"), "nope")
        .await
        .unwrap();
    // Refused before any Welcome: no subscription is answered.
    let _ = bad
        .send(&f(
            STREAM_EVENTS,
            kinds::SUBSCRIBE,
            v1::Subscribe::default().encode_to_vec(),
        ))
        .await;
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
    subscribe(&mut client, v1::Subscribe::default().encode_to_vec()).await;
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
    // `GET /api/metrics` (R8): written to the node and not answered yet.
    assert_eq!(st.hub.router().node_load(), (0, 1));
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
    // Answered: nothing is waiting any more (the count follows the answer on the node's own loop).
    let end = std::time::Instant::now() + Duration::from_secs(10);
    while st.hub.router().node_load() != (0, 0) {
        assert!(
            std::time::Instant::now() < end,
            "{:?}",
            st.hub.router().node_load()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
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
    subscribe(&mut client, v1::Subscribe::default().encode_to_vec()).await;
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
    subscribe(&mut c, v1::Subscribe::default().encode_to_vec()).await;
    assert_eq!(recv(&mut c).await.kind, kinds::SNAPSHOT);
    let mut bad = dial(addr, Some("0123456789abcdef0123456789abcdef")).await;
    // Refused before any Welcome: no subscription is answered.
    let _ = bad
        .send(&f(
            STREAM_EVENTS,
            kinds::SUBSCRIBE,
            v1::Subscribe::default().encode_to_vec(),
        ))
        .await;
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
            ..Default::default()
        })
    };
    let bodies = vec![
        online("n1"),
        agent("n1", "a1"),
        agent("n1", "a2"),
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
    subscribe(&mut client, v1::Subscribe::default().encode_to_vec()).await;
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

/// #915: a mesh post to a machine whose link is dead but not yet found so (the hub still lists it online), and to one the hub
/// already records offline, is answered `node.offline` to the sending node: never left without an answer.
#[tokio::test]
async fn a_mesh_post_to_a_node_whose_link_died_or_ended_is_answered_node_offline() {
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
    let silence = Duration::from_millis(400);
    let (mut n2, t2) = node_conn_within(&st, "n2", silence).await;
    let from = handshake(&mut n2).await.from_seq;
    n2.send(&f(
        STREAM_EVENTS,
        kinds::EVENT,
        event("n2", from, online("n2")),
    ))
    .await
    .unwrap();
    next_ack(&mut n2).await;

    // n2's machine is gone, its connection still open: the hub lists it online and forwards the post to it.
    assert!(st.hub.view().0.nodes["n2"].online);
    n1.send(&mesh_post("p1", "n2/b1", "")).await.unwrap();
    let got = recv(&mut n2).await;
    assert_eq!((got.stream_id, got.kind), (STREAM_COMMANDS, 0));
    // n2 never answers; when the hub finds the link silent, the sender is told.
    let r = response(&mut n1).await;
    assert_eq!(
        (r.request_id.as_str(), err_code(&r)),
        ("p1", "node.offline")
    );
    assert!(t2.await.unwrap(), "n2's link ended for silence");
    assert!(!st.hub.view().0.nodes["n2"].online);

    // Recorded offline (its agents ended with it): answered at once.
    n1.send(&mesh_post("p2", "n2/b1", "")).await.unwrap();
    let r = response(&mut n1).await;
    assert_eq!(
        (r.request_id.as_str(), err_code(&r)),
        ("p2", "node.offline")
    );
}

/// Audit F2060: what the hub adds to a command on its way (its own request id, a post's `origin`) can take one that arrived at the
/// frame limit over it. That one is refused to whoever sent it; the node it was for keeps its connection (it used to be ended).
#[tokio::test]
async fn a_command_the_hub_enlarges_past_the_frame_limit_is_refused_and_its_target_keeps_its_link()
{
    use agora_protocol::frame::DEFAULT_MAX_PAYLOAD;
    let post = |id: &str, to: &str, origin: &str, body: String| v1::CommandRequest {
        request_id: id.into(),
        idempotency_key: String::new(),
        target_node: String::new(),
        command: Some(command_request::Command::MeshPost(v1::MeshPost {
            to: to.into(),
            body,
            origin: origin.into(),
            from_agent: String::new(),
            refs: Vec::new(),
        })),
    };
    let st = state(None);
    let mut nodes = Vec::new();
    for name in ["n1", "n2"] {
        let (mut c, t) = node_conn(&st, name).await;
        handshake(&mut c).await;
        c.send(&f(
            STREAM_EVENTS,
            kinds::EVENT,
            event(name, 1, online(name)),
        ))
        .await
        .unwrap();
        next_ack(&mut c).await;
        nodes.push((c, t));
    }
    let (mut n2, t2) = nodes.pop().unwrap();
    let (mut n1, _t1) = nodes.pop().unwrap();

    // The largest post n1 can send; forwarded with the hub's id and n1's name as its origin, it no longer fits one frame.
    let mut k = DEFAULT_MAX_PAYLOAD - post("", "n2/b", "", String::new()).encoded_len();
    while post("", "n2/b", "", "x".repeat(k)).encoded_len() > DEFAULT_MAX_PAYLOAD {
        k -= 1;
    }
    assert!(post("h0", "b", "n1", "x".repeat(k)).encoded_len() > DEFAULT_MAX_PAYLOAD);
    let at_limit = post("", "n2/b", "", "x".repeat(k)).encode_to_vec();
    n1.send(&f(STREAM_COMMANDS, 0, at_limit)).await.unwrap();
    let r = response(&mut n1).await;
    assert_eq!(err_code(&r), "protocol.invalid_message");

    // n2 is still connected and gets the next post.
    n1.send(&mesh_post("p2", "n2/b1", "")).await.unwrap();
    let got = recv(&mut n2).await;
    assert_eq!((got.stream_id, got.kind), (STREAM_COMMANDS, 0));
    let forwarded = v1::CommandRequest::decode(got.payload.as_slice()).unwrap();
    assert!(matches!(
        forwarded.command,
        Some(command_request::Command::MeshPost(p)) if p.body == "hi"
    ));
    assert!(!t2.is_finished(), "n2's link is still served");
    assert!(st.hub.view().0.nodes["n2"].online);
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
    subscribe(&mut c, v1::Subscribe::default().encode_to_vec()).await;
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
            session: String::new(),
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
    subscribe(&mut client, v1::Subscribe::default().encode_to_vec()).await;
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
    subscribe(&mut client, v1::Subscribe::default().encode_to_vec()).await;
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

/// ADR-0106: subscribes as a window and reads what the hub sends first, which must be the `Welcome` naming the person's role; the
/// snapshot follows it.
async fn welcomed(c: &mut Conn<TcpStream>) -> v1::PersonRole {
    c.send(&f(
        STREAM_EVENTS,
        kinds::SUBSCRIBE,
        v1::Subscribe::default().encode_to_vec(),
    ))
    .await
    .unwrap();
    let first = tokio::time::timeout(Duration::from_secs(10), c.recv())
        .await
        .expect("timeout")
        .unwrap()
        .expect("closed");
    assert_eq!(
        (first.stream_id, first.kind),
        (STREAM_CONTROL, kinds::WELCOME),
        "the Welcome comes first"
    );
    let w = v1::Welcome::decode(first.payload.as_slice()).unwrap();
    // Nothing was negotiated: a window sends no Hello.
    assert_eq!(
        (
            w.chosen_generation,
            w.capabilities.len(),
            w.peer_id.as_str()
        ),
        (1, 0, "")
    );
    assert!(w.server_time_ms > 0);
    assert_eq!(recv(c).await.kind, kinds::SNAPSHOT);
    w.person_role()
}

#[tokio::test]
async fn a_window_is_told_its_persons_role_first_and_a_role_change_reconnects_with_the_new_one() {
    use agora_hub::users::Role;
    let role = Arc::new(std::sync::Mutex::new(Role::Owner));
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
    // One connection per role, each told its own.
    for (r, want) in [
        (Role::Owner, v1::PersonRole::Owner),
        (Role::Member, v1::PersonRole::Member),
        (Role::Viewer, v1::PersonRole::Viewer),
    ] {
        *role.lock().unwrap() = r;
        let mut c = dial(addr, Some("good-token")).await;
        assert_eq!(welcomed(&mut c).await, want, "{r:?}");
    }
    // A member made a viewer while connected: the hub ends the connection (checked once a second, with no command sent), and the
    // next one says viewer. Then made the owner: the same, and owner.
    *role.lock().unwrap() = Role::Member;
    let mut c = dial(addr, Some("good-token")).await;
    assert_eq!(welcomed(&mut c).await, v1::PersonRole::Member);
    *role.lock().unwrap() = Role::Viewer;
    assert!(closed(&mut c).await);
    let mut c = dial(addr, Some("good-token")).await;
    assert_eq!(welcomed(&mut c).await, v1::PersonRole::Viewer);
    *role.lock().unwrap() = Role::Owner;
    assert!(closed(&mut c).await);
    let mut c = dial(addr, Some("good-token")).await;
    assert_eq!(welcomed(&mut c).await, v1::PersonRole::Owner);
}

#[tokio::test]
async fn a_hub_not_in_team_mode_tells_its_window_owner_and_a_node_is_told_no_role() {
    // `Tokens` answers `access` with the trait's default: the one person of a hub not in team mode.
    let st = state(None);
    let addr = start(Arc::clone(&st)).await;
    let mut c = dial(addr, Some("good-token")).await;
    assert_eq!(welcomed(&mut c).await, v1::PersonRole::Owner);

    let (mut node, _t) = node_conn(&st, "n1").await;
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
    let w = recv(&mut node).await;
    assert_eq!((w.stream_id, w.kind), (STREAM_CONTROL, kinds::WELCOME));
    let w = v1::Welcome::decode(w.payload.as_slice()).unwrap();
    assert_eq!(
        (w.person_role(), w.peer_id.as_str()),
        (v1::PersonRole::Unspecified, "n1")
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
            session: String::new(),
        })
    }
}

fn started(id: &str, project: &str) -> Body {
    Body::AgentStarted(v1::AgentStarted {
        run_id: Vec::new(),
        agent_id: id.into(),
        project_id: project.into(),
        node_id: "n1".into(),
        worktree_id: String::new(),
        kind: "shell".into(),
        level: 2,
        parent_agent_id: String::new(),
        same_process: false,
        launch_options: Vec::new(),
        skips_permissions: false,
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
            stop_helpers: false,
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
    // ADR-0069: the machine's executable is off its PATH, in a folder named after its user.
    let online_at = |path: &str| {
        Body::NodeOnline(v1::NodeOnline {
            node_id: "n1".into(),
            hostname: "h".into(),
            os: "linux".into(),
            build: "b".into(),
            binary: path.into(),
        })
    };
    for (seq, body) in [
        (1, online_at("/home/eda/Downloads/agora")),
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
    subscribe(
        &mut client,
        v1::Subscribe { after_pos: Some(0) }.encode_to_vec(),
    )
    .await;
    let fr = recv(&mut client).await;
    assert_eq!(fr.kind, kinds::SNAPSHOT);
    let snap = v1::Snapshot::decode(fr.payload.as_slice()).unwrap();
    let agents: Vec<&str> = snap.agents.iter().map(|a| a.agent_id.as_str()).collect();
    assert_eq!(agents, ["n1/a1"], "{snap:?}");
    // Only the owner sees where the machine's user keeps it: a member gets the file name.
    assert_eq!(snap.nodes[0].binary, "agora");

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

    // A later start of the machine, live: shortened the same way.
    node.send(&f(
        STREAM_EVENTS,
        kinds::EVENT,
        event("n1", 6, online_at(r"C:\Users\eda\Downloads\agora.exe")),
    ))
    .await
    .unwrap();
    next_ack(&mut node).await;
    let binary = loop {
        let fr = recv(&mut client).await;
        if fr.kind != kinds::POSITIONED_EVENT {
            continue;
        }
        let ev = v1::PositionedEvent::decode(fr.payload.as_slice()).unwrap();
        if let Some(Body::NodeOnline(n)) = ev.event.and_then(|e| e.body) {
            break n.binary;
        }
    };
    assert_eq!(binary, "agora.exe");
}

/// A request on the command stream, as a window sends it.
fn command_frame(id: &str, target: &str, command: command_request::Command) -> Frame {
    let req = v1::CommandRequest {
        request_id: id.into(),
        idempotency_key: String::new(),
        target_node: target.into(),
        command: Some(command),
    };
    f(STREAM_COMMANDS, 0, req.encode_to_vec())
}

/// Sends `command` from the window, answers it on the node with `answer`, and returns what the window then gets.
async fn answered(
    client: &mut Conn<TcpStream>,
    node: &mut Conn<TcpStream>,
    id: &str,
    command: command_request::Command,
    answer: command_ok::Result,
) -> v1::CommandResponse {
    client
        .send(&command_frame(id, "n1", command))
        .await
        .unwrap();
    let got = recv(node).await;
    let forwarded = v1::CommandRequest::decode(got.payload.as_slice()).unwrap();
    node.send(&f(
        STREAM_COMMANDS,
        1,
        v1::CommandResponse {
            request_id: forwarded.request_id,
            result: Some(command_response::Result::Ok(v1::CommandOk {
                result: Some(answer),
            })),
        }
        .encode_to_vec(),
    ))
    .await
    .unwrap();
    response(client).await
}

/// ADR-0049 Amendment A (#1049), through a real hub: a member granted `n1/mine` (`/srv/mine`) browses, chooses and starts in it; another
/// project's folder, a folder above it and an escape by `..` stop at the hub; a folder the machine really opened elsewhere (a link
/// inside the project) is refused in place of the machine's answer; the way up from the project's folder is left out.
#[tokio::test]
async fn a_member_browses_and_starts_only_inside_a_project_granted_on_that_machine() {
    let caps = || {
        vec![
            "directory.list".to_string(),
            "workspace.directory".to_string(),
        ]
    };
    let st = Arc::new(ServerState {
        hub: Arc::new(Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap()),
        support: Support {
            generations: vec![1],
            capabilities: caps(),
            required: vec![],
        },
        ca: None,
        lockout: Arc::new(Lockout::new()),
        auth: Box::new(Scoped),
        assets: None,
        sessions: None,
    });
    let addr = start(Arc::clone(&st)).await;
    let (mut node, _t) = node_conn(&st, "n1").await;
    let hello = v1::Hello {
        role: v1::Role::Node as i32,
        protocol_generations: vec![1],
        capabilities: caps(),
        build: "t".into(),
        locale: None,
    };
    node.send(&f(STREAM_CONTROL, kinds::HELLO, hello.encode_to_vec()))
        .await
        .unwrap();
    assert_eq!(recv(&mut node).await.kind, kinds::WELCOME);
    node.send(&f(
        STREAM_CONTROL,
        kinds::RESUME,
        v1::Resume::default().encode_to_vec(),
    ))
    .await
    .unwrap();
    assert_eq!(recv(&mut node).await.kind, kinds::RESUME_OK);
    let checkout = |id: &str, project: &str, path: &str| {
        Body::CheckoutDiscovered(v1::CheckoutDiscovered {
            checkout_id: id.into(),
            node_id: "n1".into(),
            project_id: project.into(),
            path: path.into(),
        })
    };
    for (seq, body) in [
        (1, online("n1")),
        (2, checkout("c1", "mine", "/srv/mine")),
        (3, checkout("c2", "theirs", "/srv/theirs")),
    ] {
        node.send(&f(STREAM_EVENTS, kinds::EVENT, event("n1", seq, body)))
            .await
            .unwrap();
        next_ack(&mut node).await;
    }
    let mut client = dial(addr, Some("good-token")).await;
    subscribe(&mut client, v1::Subscribe::default().encode_to_vec()).await;
    assert_eq!(recv(&mut client).await.kind, kinds::SNAPSHOT);
    let list = |path: &str| {
        command_request::Command::DirectoryList(v1::DirectoryList {
            path: path.into(),
            authorization: None,
        })
    };
    let launch = |path: &str| {
        command_request::Command::AgentLaunch(v1::AgentLaunch {
            path: path.into(),
            project_id: "mine".into(),
            kind: "shell".into(),
            ..Default::default()
        })
    };
    // Refused at the hub, named by the command (#1044's role wording): the node hears nothing.
    for (id, command) in [
        ("x1", list("/srv/theirs")),
        ("x2", list("/srv")),
        ("x3", list("/srv/mine/../theirs")),
        ("x4", list("")),
        ("x5", launch("/srv/theirs")),
    ] {
        client
            .send(&command_frame(id, "n1", command))
            .await
            .unwrap();
        let r = response(&mut client).await;
        assert_eq!(
            (r.request_id.as_str(), err_code(&r)),
            (id, "auth.forbidden")
        );
        let Some(command_response::Result::Err(info)) = &r.result else {
            panic!("expected an error");
        };
        let action = if id == "x5" {
            "AgentLaunch"
        } else {
            "DirectoryList"
        };
        assert_eq!(
            info.params.get("action").and_then(|v| v.value.clone()),
            Some(v1::param_value::Value::Str(action.into())),
            "{info:?}"
        );
    }
    // Inside the project: forwarded, and the machine's answer is checked on its way back.
    let entries = |path: &str, parent: &str| {
        command_ok::Result::DirectoryEntries(v1::DirectoryEntries {
            path: path.into(),
            parent: parent.into(),
            entries: vec![v1::DirectoryEntry {
                name: "api".into(),
                is_git: false,
            }],
            truncated: false,
        })
    };
    let r = answered(
        &mut client,
        &mut node,
        "in",
        list("/srv/mine"),
        entries("/srv/mine", "/srv"),
    )
    .await;
    let Some(command_response::Result::Ok(v1::CommandOk {
        result: Some(command_ok::Result::DirectoryEntries(e)),
    })) = r.result
    else {
        panic!("expected the listing: {r:?}");
    };
    assert_eq!((e.path.as_str(), e.parent.as_str()), ("/srv/mine", ""));
    assert_eq!(e.entries.len(), 1);
    // `/srv/mine/out` is a link the machine followed to `/etc`: the member gets a refusal, not `/etc`'s folders.
    let r = answered(
        &mut client,
        &mut node,
        "link",
        list("/srv/mine/out"),
        entries("/etc", "/"),
    )
    .await;
    assert_eq!(
        (r.request_id.as_str(), err_code(&r)),
        ("link", "auth.forbidden")
    );
    let r = answered(
        &mut client,
        &mut node,
        "go",
        launch("/srv/mine/api"),
        command_ok::Result::AgentLaunch(v1::AgentLaunchResult {
            agent_id: "a1".into(),
            ..Default::default()
        }),
    )
    .await;
    assert!(
        matches!(r.result, Some(command_response::Result::Ok(_))),
        "{r:?}"
    );

    // The audit trail says how each ended, the refused answer too.
    let rows = agora_hub::audit::Audit::new(st.hub.store())
        .list(0, Some("eda"), 100)
        .unwrap();
    let seen: Vec<(&str, &str)> = rows
        .iter()
        .map(|e| (e.action.as_str(), e.outcome.as_str()))
        .collect();
    assert_eq!(
        seen,
        [
            ("DirectoryList", "auth.forbidden"),
            ("DirectoryList", "auth.forbidden"),
            ("DirectoryList", "auth.forbidden"),
            ("DirectoryList", "auth.forbidden"),
            ("AgentLaunch", "auth.forbidden"),
            ("DirectoryList", "ok"),
            ("DirectoryList", "auth.forbidden"),
            ("AgentLaunch", "ok"),
        ]
    );
}

async fn get_audit(addr: std::net::SocketAddr, token: Option<&str>, query: &str) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let auth = token.map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"));
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(format!("GET /api/audit{query} HTTP/1.1\r\nHost: x\r\n{auth}\r\n").as_bytes())
        .await
        .unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    let status = out[9..12].parse().unwrap();
    (
        status,
        out.split("\r\n\r\n").nth(1).unwrap_or("").to_string(),
    )
}

#[tokio::test]
async fn only_the_owner_can_read_the_audit_trail_over_http() {
    use agora_hub::signin::{HubSessions, SignIn};
    use agora_hub::users::{Role, Users};
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
    let signin = SignIn::new(hub.store(), Arc::new(Lockout::new()));
    let now = server::now_ms();
    let owner = signin
        .redeem(&signin.create_code(now).unwrap(), now)
        .unwrap();
    let eda = Users::new(hub.store())
        .add("Eda", Role::Viewer, now)
        .unwrap();
    let viewer = signin
        .redeem(&signin.create_code_for(Some(&eda.id), now).unwrap(), now)
        .unwrap();

    // No token and a made-up one are 401; a viewer is 403; the owner gets the rows (the sign-ins and the new person are in them).
    assert_eq!(get_audit(addr, None, "").await.0, 401);
    assert_eq!(get_audit(addr, Some("nope"), "").await.0, 401);
    assert_eq!(get_audit(addr, Some(&viewer), "").await.0, 403);
    let (status, body) = get_audit(addr, Some(&owner), "").await;
    assert_eq!(status, 200, "{body}");
    let rows: Vec<serde_json::Value> = serde_json::from_str(&body).unwrap();
    let actions: Vec<&str> = rows.iter().map(|r| r["action"].as_str().unwrap()).collect();
    assert_eq!(actions, ["sign_in", "user.add", "sign_in"]);
    // Filtering by person and a limit; a malformed query is refused, not guessed at.
    let (_, body) = get_audit(addr, Some(&owner), "?who=eda").await;
    let rows: Vec<serde_json::Value> = serde_json::from_str(&body).unwrap();
    assert_eq!(rows.len(), 1);
    let (_, body) = get_audit(addr, Some(&owner), "?limit=1").await;
    assert_eq!(
        serde_json::from_str::<Vec<serde_json::Value>>(&body)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(get_audit(addr, Some(&owner), "?bogus=1").await.0, 400);
    assert_eq!(get_audit(addr, Some(&owner), "?who=a%20b").await.0, 400);
}

async fn forget_agent(client: &mut Conn<TcpStream>, id: &str) -> v1::CommandResponse {
    let req = v1::CommandRequest {
        request_id: "f".into(),
        idempotency_key: String::new(),
        target_node: String::new(),
        command: Some(command_request::Command::AgentForget(v1::AgentForget {
            agent_id: id.into(),
        })),
    };
    client
        .send(&f(STREAM_COMMANDS, 0, req.encode_to_vec()))
        .await
        .unwrap();
    response(client).await
}

#[tokio::test]
async fn a_closed_agent_is_forgotten_for_every_client_but_its_usage_still_counts() {
    let st = state(None);
    let addr = start(Arc::clone(&st)).await;
    let (mut node, _t) = node_conn(&st, "n1").await;
    handshake(&mut node).await;
    let bodies = vec![
        online("n1"),
        agent("n1", "a1"),
        Body::AgentUsageReported(v1::AgentUsageReported {
            agent_id: "a1".into(),
            model: "anthropic/claude-haiku-4-5".into(),
            input_tokens: 1_000,
            output_tokens: 2_000,
            ..Default::default()
        }),
    ];
    for (i, b) in bodies.into_iter().enumerate() {
        node.send(&f(
            STREAM_EVENTS,
            kinds::EVENT,
            event("n1", i as u64 + 1, b),
        ))
        .await
        .unwrap();
        next_ack(&mut node).await;
    }
    let mut client = dial(addr, Some("good-token")).await;
    subscribe(&mut client, v1::Subscribe::default().encode_to_vec()).await;
    assert_eq!(recv(&mut client).await.kind, kinds::SNAPSHOT);
    // It still runs: refused.
    let r = forget_agent(&mut client, "n1/a1").await;
    assert_eq!(err_code(&r), "agent.running");
    // The node says it exited; now the hub forgets it, and says so on the event stream.
    node.send(&f(
        STREAM_EVENTS,
        kinds::EVENT,
        event(
            "n1",
            4,
            Body::AgentExited(v1::AgentExited {
                agent_id: "a1".into(),
                exit_code: 0,
            }),
        ),
    ))
    .await
    .unwrap();
    next_ack(&mut node).await;
    assert_eq!(recv(&mut client).await.kind, kinds::POSITIONED_EVENT); // the exit
    let r = forget_agent(&mut client, "n1/a1").await;
    assert!(
        matches!(r.result, Some(command_response::Result::Ok(_))),
        "{r:?}"
    );
    let heard = recv(&mut client).await;
    let ev = v1::PositionedEvent::decode(heard.payload.as_slice())
        .unwrap()
        .event
        .unwrap();
    assert!(matches!(ev.body, Some(Body::AgentForgotten(ref e)) if e.agent_id == "n1/a1"));

    let missing = forget_agent(&mut client, "n1/missing").await;
    assert_eq!(err_code(&missing), "agent.not_found");

    let rows = agora_hub::audit::Audit::new(st.hub.store())
        .list(0, None, 10)
        .unwrap();
    let rows: Vec<_> = rows.iter().filter(|r| r.action == "agent.forget").collect();
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows.iter().map(|r| r.outcome.as_str()).collect::<Vec<_>>(),
        ["agent.running", "ok", "agent.not_found"]
    );
    println!("CONTROL OK: all three forget outcomes were recorded");
    let targets: Vec<_> = rows
        .iter()
        .map(|r| (r.node.as_str(), r.project_id.as_str(), r.agent_id.as_str()))
        .collect();
    let expected = [("n1", "p", "a1"), ("n1", "p", "a1"), ("n1", "", "missing")];
    if targets != expected {
        println!("EXPECTED: {expected:?}\nACTUAL: {targets:?}\nPROBLEM CONFIRMED");
    } else {
        println!("PROBLEM NOT REPRODUCED");
    }
    assert_eq!(targets, expected);

    // A client that arrives now no longer sees the agent, but the hub's usage still counts it.
    let mut later = dial(addr, Some("good-token")).await;
    subscribe(&mut later, v1::Subscribe::default().encode_to_vec()).await;
    let snap = recv(&mut later).await;
    assert_eq!(snap.kind, kinds::SNAPSHOT);
    let snapshot = v1::Snapshot::decode(snap.payload.as_slice()).unwrap();
    assert!(snapshot.agents.is_empty(), "{:?}", snapshot.agents);
    let req = v1::CommandRequest {
        request_id: "u".into(),
        idempotency_key: String::new(),
        target_node: String::new(),
        command: Some(command_request::Command::UsageReport(v1::UsageReport {})),
    };
    later
        .send(&f(STREAM_COMMANDS, 0, req.encode_to_vec()))
        .await
        .unwrap();
    let r = response(&mut later).await;
    let Some(command_response::Result::Ok(v1::CommandOk {
        result: Some(command_ok::Result::UsageReport(report)),
    })) = r.result
    else {
        panic!("not a usage report: {r:?}");
    };
    assert_eq!(
        report.agents.len(),
        1,
        "a forgotten agent's tokens still count"
    );
}

/// ADR-0108: a node's handshake offering `caps`.
async fn handshake_offering(c: &mut Conn<TcpStream>, caps: &[&str]) {
    let hello = v1::Hello {
        role: v1::Role::Node as i32,
        protocol_generations: vec![1],
        capabilities: caps.iter().map(|c| (*c).to_string()).collect(),
        build: "t".into(),
        locale: None,
    };
    c.send(&f(STREAM_CONTROL, kinds::HELLO, hello.encode_to_vec()))
        .await
        .unwrap();
    assert_eq!(recv(c).await.kind, kinds::WELCOME);
    c.send(&f(
        STREAM_CONTROL,
        kinds::RESUME,
        v1::Resume::default().encode_to_vec(),
    ))
    .await
    .unwrap();
    assert_eq!(recv(c).await.kind, kinds::RESUME_OK);
}

/// The next command the hub sends the node, or `None` when none comes within `wait`.
async fn command_to_node(c: &mut Conn<TcpStream>, wait: Duration) -> Option<v1::CommandRequest> {
    let end = tokio::time::Instant::now() + wait;
    loop {
        let left = end.saturating_duration_since(tokio::time::Instant::now());
        let fr = match tokio::time::timeout(left, c.recv()).await {
            Ok(Ok(Some(fr))) => fr,
            _ => return None,
        };
        if (fr.stream_id, fr.kind) == (STREAM_COMMANDS, 0) {
            return Some(v1::CommandRequest::decode(fr.payload.as_slice()).unwrap());
        }
    }
}

/// A client's forget and the hub's answer, past the events the client hears meanwhile.
async fn forget_answer(client: &mut Conn<TcpStream>, id: &str) -> v1::CommandResponse {
    let req = v1::CommandRequest {
        request_id: format!("forget-{id}"),
        idempotency_key: String::new(),
        target_node: String::new(),
        command: Some(command_request::Command::AgentForget(v1::AgentForget {
            agent_id: id.into(),
        })),
    };
    client
        .send(&f(STREAM_COMMANDS, 0, req.encode_to_vec()))
        .await
        .unwrap();
    loop {
        let fr = recv(client).await;
        if (fr.stream_id, fr.kind) == (STREAM_COMMANDS, 1) {
            return v1::CommandResponse::decode(fr.payload.as_slice()).unwrap();
        }
    }
}

async fn until(what: &str, mut ok: impl FnMut() -> bool) {
    let end = tokio::time::Instant::now() + Duration::from_secs(10);
    while !ok() {
        assert!(
            tokio::time::Instant::now() < end,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// ADR-0108: a forget of a closed agent whose machine is offline is owed to it; when it connects it is sent the forget (the node's own
/// id), again on the next connection if this one ends before it answers, and once it answers nothing is owed. A machine that does not
/// offer `agent.forget.local` is never sent one.
#[tokio::test]
async fn a_forget_reaches_a_capable_node_when_it_connects_and_is_owed_until_it_answers() {
    const CAP: &str = "agent.forget.local";
    let st = Arc::new(ServerState {
        hub: Arc::new(Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap()),
        support: Support {
            generations: vec![1],
            capabilities: vec![CAP.into()],
            required: vec![],
        },
        ca: None,
        lockout: Arc::new(Lockout::new()),
        auth: Box::new(Tokens),
        assets: None,
        sessions: None,
    });
    let addr = start(Arc::clone(&st)).await;
    // Each machine ran an agent that has ended, then went away.
    for (node, offers) in [("n1", true), ("n2", false)] {
        let (mut conn, task) = node_conn(&st, node).await;
        handshake_offering(&mut conn, if offers { &[CAP] } else { &[] }).await;
        let exited = Body::AgentExited(v1::AgentExited {
            agent_id: "agent-3".into(),
            exit_code: 0,
        });
        for (i, b) in [online(node), agent(node, "agent-3"), exited]
            .into_iter()
            .enumerate()
        {
            conn.send(&f(
                STREAM_EVENTS,
                kinds::EVENT,
                event(node, i as u64 + 1, b),
            ))
            .await
            .unwrap();
            next_ack(&mut conn).await;
        }
        drop(conn);
        let _ = task.await;
        until("the machine to leave", || {
            !st.hub.router().is_connected(node)
        })
        .await;
    }
    let mut client = dial(addr, Some("good-token")).await;
    subscribe(&mut client, v1::Subscribe::default().encode_to_vec()).await;
    assert_eq!(recv(&mut client).await.kind, kinds::SNAPSHOT);
    for id in ["n1/agent-3", "n2/agent-3"] {
        let r = forget_answer(&mut client, id).await;
        assert!(
            matches!(r.result, Some(command_response::Result::Ok(_))),
            "{r:?}"
        );
    }
    assert_eq!(st.hub.forgets().pending("n1"), vec!["agent-3"]);
    assert_eq!(st.hub.forgets().pending("n2"), vec!["agent-3"]);

    // n1 connects and is sent the forget; its connection ends before it answers: still owed.
    let (mut n1, task) = node_conn(&st, "n1").await;
    handshake_offering(&mut n1, &[CAP]).await;
    let sent = command_to_node(&mut n1, Duration::from_secs(10))
        .await
        .unwrap();
    assert!(matches!(
        sent.command,
        Some(command_request::Command::AgentForget(ref f)) if f.agent_id == "agent-3"
    ));
    drop(n1);
    let _ = task.await;
    until("n1 to leave", || !st.hub.router().is_connected("n1")).await;
    assert_eq!(st.hub.forgets().pending("n1"), vec!["agent-3"]);

    // Again on the next connection; this time it answers (here: refused, the agent runs there), and nothing is owed any more.
    let (mut n1, _task) = node_conn(&st, "n1").await;
    handshake_offering(&mut n1, &[CAP]).await;
    let sent = command_to_node(&mut n1, Duration::from_secs(10))
        .await
        .unwrap();
    assert!(matches!(
        sent.command,
        Some(command_request::Command::AgentForget(_))
    ));
    let answer = v1::CommandResponse {
        request_id: sent.request_id,
        result: Some(command_response::Result::Err(
            agora_protocol::Error::new(agora_protocol::ErrorCode::AgentRunning)
                .with("agent_id", "agent-3")
                .to_info(),
        )),
    };
    n1.send(&f(STREAM_COMMANDS, 1, answer.encode_to_vec()))
        .await
        .unwrap();
    until("the answer to settle the forget", || {
        st.hub.forgets().pending("n1").is_empty()
    })
    .await;

    // A forget while it is connected goes at once, and an `ok` settles it. Another machine that offers the capability and is
    // connected is sent nothing: the forget goes to the agent's own machine only.
    let (mut n3, _t3) = node_conn(&st, "n3").await;
    handshake_offering(&mut n3, &[CAP]).await;
    let r = forget_answer(&mut client, "n1/agent-3").await;
    assert!(
        matches!(r.result, Some(command_response::Result::Ok(_))),
        "{r:?}"
    );
    let sent = command_to_node(&mut n1, Duration::from_secs(10))
        .await
        .unwrap();
    assert!(matches!(
        sent.command,
        Some(command_request::Command::AgentForget(_))
    ));
    let ok = v1::CommandResponse {
        request_id: sent.request_id,
        result: Some(command_response::Result::Ok(v1::CommandOk {
            result: Some(command_ok::Result::Empty(v1::Empty {})),
        })),
    };
    n1.send(&f(STREAM_COMMANDS, 1, ok.encode_to_vec()))
        .await
        .unwrap();
    until("the ok to settle the forget", || {
        st.hub.forgets().pending("n1").is_empty()
    })
    .await;
    assert!(command_to_node(&mut n3, Duration::from_millis(500))
        .await
        .is_none());
    assert!(st.hub.forgets().pending("n3").is_empty());
    // Each answer is in the audit trail under the hub's own id: its outcome the bare code, the agent and its project in their columns.
    let rows: Vec<(String, String, String, String)> = agora_hub::audit::Audit::new(st.hub.store())
        .list(0, Some(agora_hub::ca::RESERVED_HUB_ID), 100)
        .unwrap()
        .into_iter()
        .filter(|e| e.action == "agent.forget.local")
        .map(|e| (e.node, e.outcome, e.agent_id, e.project_id))
        .collect();
    let row = |outcome: &str| {
        (
            "n1".to_string(),
            outcome.to_string(),
            "agent-3".to_string(),
            "p".to_string(),
        )
    };
    assert_eq!(rows, vec![row("agent.running"), row("ok")]);

    // n2 does not offer the capability: it is never sent the forget, which stays owed.
    let (mut n2, _t2) = node_conn(&st, "n2").await;
    handshake_offering(&mut n2, &[]).await;
    assert!(command_to_node(&mut n2, Duration::from_millis(500))
        .await
        .is_none());
    assert_eq!(st.hub.forgets().pending("n2"), vec!["agent-3"]);
}

const FORGET_CAP: &str = "agent.forget.local";

/// A hub that offers `agent.forget.local`, on `store`.
fn forget_state(store: SqliteStore) -> Arc<ServerState> {
    Arc::new(ServerState {
        hub: Arc::new(Hub::open(store).unwrap()),
        support: Support {
            generations: vec![1],
            capabilities: vec![FORGET_CAP.into()],
            required: vec![],
        },
        ca: None,
        lockout: Arc::new(Lockout::new()),
        auth: Box::new(Tokens),
        assets: None,
        sessions: None,
    })
}

/// `n1` ran `agents`, which ended, and went away; then the person forgot them: each is owed to `n1`.
async fn forgets_owed_while_away(st: &Arc<ServerState>, agents: &[&str]) {
    let (mut conn, task) = node_conn(st, "n1").await;
    handshake_offering(&mut conn, &[FORGET_CAP]).await;
    let mut bodies = vec![online("n1")];
    for a in agents {
        bodies.push(agent("n1", a));
        bodies.push(Body::AgentExited(v1::AgentExited {
            agent_id: (*a).into(),
            exit_code: 0,
        }));
    }
    for (i, b) in bodies.into_iter().enumerate() {
        conn.send(&f(
            STREAM_EVENTS,
            kinds::EVENT,
            event("n1", i as u64 + 1, b),
        ))
        .await
        .unwrap();
        next_ack(&mut conn).await;
    }
    drop(conn);
    let _ = task.await;
    until("n1 to leave", || !st.hub.router().is_connected("n1")).await;
    for a in agents {
        st.hub.forget_agent(&format!("n1/{a}"), 100).unwrap();
    }
    assert_eq!(st.hub.forgets().pending("n1"), agents.to_vec());
}

/// `n1` connects offering `agent.forget.local` and is sent the forget of `agent`.
async fn forget_sent(
    st: &Arc<ServerState>,
    agent: &str,
) -> (
    Conn<TcpStream>,
    tokio::task::JoinHandle<()>,
    v1::CommandRequest,
) {
    let (mut conn, task) = node_conn(st, "n1").await;
    handshake_offering(&mut conn, &[FORGET_CAP]).await;
    let sent = command_to_node(&mut conn, Duration::from_secs(10))
        .await
        .expect("the forget is sent after the Welcome");
    assert!(
        matches!(sent.command, Some(command_request::Command::AgentForget(ref f)) if f.agent_id == agent),
        "{sent:?}"
    );
    (conn, task, sent)
}

/// The node answers `sent` with `code` (`None`: `ok`), and the hub has taken the answer in (its audit row is written).
async fn answer_forget(
    st: &Arc<ServerState>,
    conn: &mut Conn<TcpStream>,
    sent: v1::CommandRequest,
    code: Option<agora_protocol::ErrorCode>,
) {
    let rows = || {
        agora_hub::audit::Audit::new(st.hub.store())
            .list(0, Some(agora_hub::ca::RESERVED_HUB_ID), 1000)
            .unwrap()
            .into_iter()
            .filter(|e| e.action == "agent.forget.local")
            .count()
    };
    let before = rows();
    let result = match code {
        None => command_response::Result::Ok(v1::CommandOk {
            result: Some(command_ok::Result::Empty(v1::Empty {})),
        }),
        Some(c) => command_response::Result::Err(
            agora_protocol::Error::new(c)
                .with("node_id", "n1")
                .to_info(),
        ),
    };
    let answer = v1::CommandResponse {
        request_id: sent.request_id,
        result: Some(result),
    };
    conn.send(&f(STREAM_COMMANDS, 1, answer.encode_to_vec()))
        .await
        .unwrap();
    until("the hub to take the answer in", || rows() > before).await;
}

async fn leave(st: &Arc<ServerState>, conn: Conn<TcpStream>, task: tokio::task::JoinHandle<()>) {
    drop(conn);
    let _ = task.await;
    until("n1 to leave", || !st.hub.router().is_connected("n1")).await;
}

/// ADR-0108 amendment A: an answer that says the node could not decide now (`node.offline` from a stopping node, `node.timeout`,
/// `internal`) keeps the forget, which is sent again after the node's next `Welcome`; an `ok` then settles it. A stopping node's
/// `node.offline` also ends this connection's round: the forgets behind it wait for the next one.
#[tokio::test]
async fn a_forget_answered_with_a_passing_failure_is_kept_and_sent_again_after_the_next_welcome() {
    use agora_protocol::ErrorCode;
    for code in [
        ErrorCode::NodeOffline,
        ErrorCode::NodeTimeout,
        ErrorCode::Internal,
    ] {
        let st = forget_state(SqliteStore::open_in_memory().unwrap());
        forgets_owed_while_away(&st, &["agent-3", "agent-4"]).await;
        let (mut conn, task, sent) = forget_sent(&st, "agent-3").await;
        answer_forget(&st, &mut conn, sent, Some(code.clone())).await;
        if code == ErrorCode::NodeOffline {
            assert!(
                command_to_node(&mut conn, Duration::from_millis(500))
                    .await
                    .is_none(),
                "a stopping node is sent no more forgets"
            );
        } else {
            // Another passing failure does not end the round: the next forget goes on this connection (and is settled).
            let sent = command_to_node(&mut conn, Duration::from_secs(10))
                .await
                .unwrap();
            answer_forget(&st, &mut conn, sent, None).await;
        }
        let kept = if code == ErrorCode::NodeOffline {
            vec!["agent-3", "agent-4"]
        } else {
            vec!["agent-3"]
        };
        assert_eq!(st.hub.forgets().pending("n1"), kept, "{code:?}");
        leave(&st, conn, task).await;
        assert_eq!(st.hub.forgets().pending("n1"), kept, "{code:?}");

        // The next connection is sent it again, and an `ok` settles it.
        let (mut conn, _task, sent) = forget_sent(&st, "agent-3").await;
        answer_forget(&st, &mut conn, sent, None).await;
        if code == ErrorCode::NodeOffline {
            let sent = command_to_node(&mut conn, Duration::from_secs(10))
                .await
                .unwrap();
            answer_forget(&st, &mut conn, sent, None).await;
        }
        assert!(st.hub.forgets().pending("n1").is_empty(), "{code:?}");
    }
}

/// ADR-0108 amendment A: an answer that says the node decided drops the forget at once, as before.
#[tokio::test]
async fn a_forget_answered_with_a_decision_is_dropped() {
    use agora_protocol::ErrorCode;
    for code in [
        None,
        Some(ErrorCode::AgentRunning),
        Some(ErrorCode::ProtocolInvalidMessage),
        Some(ErrorCode::ApprovalGrantRequired),
        Some(ErrorCode::NodePolicyDenied),
    ] {
        let st = forget_state(SqliteStore::open_in_memory().unwrap());
        forgets_owed_while_away(&st, &["agent-3"]).await;
        let (mut conn, task, sent) = forget_sent(&st, "agent-3").await;
        answer_forget(&st, &mut conn, sent, code.clone()).await;
        assert!(st.hub.forgets().pending("n1").is_empty(), "{code:?}");
        leave(&st, conn, task).await;
        let (mut conn, _task) = node_conn(&st, "n1").await;
        handshake_offering(&mut conn, &[FORGET_CAP]).await;
        assert!(
            command_to_node(&mut conn, Duration::from_millis(500))
                .await
                .is_none(),
            "{code:?}"
        );
    }
}

/// ADR-0108 amendment A: a node that keeps answering `internal` is not sent the same forget for ever: after the fifth such answer
/// (`forgets::MAX_TRANSIENT`) it is dropped (retention removes the files, as for a forget past the 1000 bound).
#[tokio::test]
async fn a_forget_that_keeps_failing_is_dropped_after_five_passing_failures() {
    let st = forget_state(SqliteStore::open_in_memory().unwrap());
    forgets_owed_while_away(&st, &["agent-3"]).await;
    for i in 1..=agora_hub::forgets::MAX_TRANSIENT {
        let (mut conn, task, sent) = forget_sent(&st, "agent-3").await;
        answer_forget(
            &st,
            &mut conn,
            sent,
            Some(agora_protocol::ErrorCode::Internal),
        )
        .await;
        leave(&st, conn, task).await;
        let left: Vec<&str> = if i < agora_hub::forgets::MAX_TRANSIENT {
            vec!["agent-3"]
        } else {
            vec![]
        };
        assert_eq!(st.hub.forgets().pending("n1"), left, "after {i}");
    }
    let (mut conn, _task) = node_conn(&st, "n1").await;
    handshake_offering(&mut conn, &[FORGET_CAP]).await;
    assert!(command_to_node(&mut conn, Duration::from_millis(500))
        .await
        .is_none());
    // The person forgets it again: it is owed afresh.
    st.hub.forget_agent("n1/agent-3", 200).unwrap();
    assert_eq!(st.hub.forgets().pending("n1"), vec!["agent-3"]);
}

/// ADR-0108 amendment A: a connection that ends before the node answers keeps the forget and does not count (the node may never have
/// received it): after three such ends it is still stored as a plain id, and it still takes `MAX_TRANSIENT` passing failures to drop.
#[tokio::test]
async fn a_connection_lost_before_the_answer_keeps_the_forget_uncounted() {
    let st = forget_state(SqliteStore::open_in_memory().unwrap());
    forgets_owed_while_away(&st, &["agent-3"]).await;
    for _ in 0..3 {
        let (conn, task, _sent) = forget_sent(&st, "agent-3").await;
        leave(&st, conn, task).await;
        assert_eq!(st.hub.forgets().pending("n1"), vec!["agent-3"]);
    }
    let stored = st.hub.store().setting("forget.n1").unwrap().unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&stored).unwrap(),
        serde_json::json!(["agent-3"]),
        "no passing failure counted"
    );
    for i in 1..=agora_hub::forgets::MAX_TRANSIENT {
        let (mut conn, task, sent) = forget_sent(&st, "agent-3").await;
        answer_forget(
            &st,
            &mut conn,
            sent,
            Some(agora_protocol::ErrorCode::Internal),
        )
        .await;
        leave(&st, conn, task).await;
        assert_eq!(
            st.hub.forgets().pending("n1").is_empty(),
            i == agora_hub::forgets::MAX_TRANSIENT,
            "after {i}"
        );
    }
}

/// ADR-0108 amendment A: a kept forget, and how many passing failures it has had, survive a hub restart.
#[tokio::test]
async fn a_kept_forget_and_its_failures_survive_a_hub_restart() {
    let path =
        std::env::temp_dir().join(format!("agora-hub-forget-kept-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    {
        let st = forget_state(SqliteStore::open(&path).unwrap());
        forgets_owed_while_away(&st, &["agent-3"]).await;
        for _ in 0..4 {
            let (mut conn, task, sent) = forget_sent(&st, "agent-3").await;
            answer_forget(
                &st,
                &mut conn,
                sent,
                Some(agora_protocol::ErrorCode::NodeTimeout),
            )
            .await;
            leave(&st, conn, task).await;
        }
        assert_eq!(st.hub.forgets().pending("n1"), vec!["agent-3"]);
    }
    // A new hub on the same store: still owed, and sent after the Welcome; one more passing failure reaches the bound.
    let st = forget_state(SqliteStore::open(&path).unwrap());
    assert_eq!(st.hub.forgets().pending("n1"), vec!["agent-3"]);
    let (mut conn, task, sent) = forget_sent(&st, "agent-3").await;
    answer_forget(
        &st,
        &mut conn,
        sent,
        Some(agora_protocol::ErrorCode::Internal),
    )
    .await;
    leave(&st, conn, task).await;
    assert!(st.hub.forgets().pending("n1").is_empty());
    drop(st);
    let _ = std::fs::remove_file(&path);
}

/// The audit outcome of the newest `AgentList` row, once it is no longer `sent` (or `sent` after `wait`).
async fn agent_list_outcome(st: &ServerState, wait: Duration) -> String {
    let end = tokio::time::Instant::now() + wait;
    loop {
        let rows = agora_hub::audit::Audit::new(st.hub.store())
            .list(0, None, 100)
            .unwrap();
        let outcome = rows
            .iter()
            .rev()
            .find(|r| r.action == "AgentList")
            .map(|r| r.outcome.clone())
            .unwrap_or_default();
        if outcome != "sent" || tokio::time::Instant::now() >= end {
            return outcome;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn a_command_whose_client_left_still_gets_its_outcome_in_the_audit_trail() {
    // L17: a client that leaves (a page reload, a closed tab) while its command is on its way left the audit row `sent` for good,
    // although the node carried the command out. The row now says how it really ended.
    let st = state(None);
    let addr = start(Arc::clone(&st)).await;
    for (node_id, answers) in [("n1", true), ("n2", false)] {
        let (mut node, task) = node_conn(&st, node_id).await;
        handshake(&mut node).await;
        node.send(&f(
            STREAM_EVENTS,
            kinds::EVENT,
            event(node_id, 1, online(node_id)),
        ))
        .await
        .unwrap();
        next_ack(&mut node).await;
        let mut client = dial(addr, Some("good-token")).await;
        subscribe(&mut client, v1::Subscribe::default().encode_to_vec()).await;
        assert_eq!(recv(&mut client).await.kind, kinds::SNAPSHOT);
        client.send(&routed("gone", node_id)).await.unwrap();
        let got = recv(&mut node).await;
        let forwarded = v1::CommandRequest::decode(got.payload.as_slice()).unwrap();
        drop(client); // the client leaves before the answer
        tokio::time::sleep(Duration::from_millis(200)).await;
        if answers {
            // The node answers after the client left: the command happened.
            let answer = v1::CommandResponse {
                request_id: forwarded.request_id,
                result: Some(command_response::Result::Ok(v1::CommandOk {
                    result: Some(command_ok::Result::AgentList(v1::AgentListResult::default())),
                })),
            };
            node.send(&f(STREAM_COMMANDS, 1, answer.encode_to_vec()))
                .await
                .unwrap();
            assert_eq!(agent_list_outcome(&st, Duration::from_secs(10)).await, "ok");
        } else {
            // The node leaves too, without answering.
            drop(node);
            let _ = task.await;
            assert_eq!(
                agent_list_outcome(&st, Duration::from_secs(10)).await,
                "node.offline"
            );
        }
    }
}

#[tokio::test]
async fn a_command_whose_session_ended_still_gets_its_outcome_in_the_audit_trail() {
    // Sign-out, expiry or a revoked session ends the client's connection from the hub's side (macos's review of L17): the command
    // the node is still carrying out gets its outcome in the audit trail all the same.
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
    let addr = start(Arc::clone(&st)).await;
    let (mut node, _task) = node_conn(&st, "n3").await;
    handshake(&mut node).await;
    node.send(&f(
        STREAM_EVENTS,
        kinds::EVENT,
        event("n3", 1, online("n3")),
    ))
    .await
    .unwrap();
    next_ack(&mut node).await;
    let mut client = dial(addr, Some("any")).await;
    subscribe(&mut client, v1::Subscribe::default().encode_to_vec()).await;
    assert_eq!(recv(&mut client).await.kind, kinds::SNAPSHOT);
    client.send(&routed("revoked", "n3")).await.unwrap();
    let got = recv(&mut node).await;
    let forwarded = v1::CommandRequest::decode(got.payload.as_slice()).unwrap();
    valid.store(false, std::sync::atomic::Ordering::SeqCst);
    assert!(closed(&mut client).await);
    let answer = v1::CommandResponse {
        request_id: forwarded.request_id,
        result: Some(command_response::Result::Ok(v1::CommandOk {
            result: Some(command_ok::Result::AgentList(v1::AgentListResult::default())),
        })),
    };
    node.send(&f(STREAM_COMMANDS, 1, answer.encode_to_vec()))
        .await
        .unwrap();
    assert_eq!(agent_list_outcome(&st, Duration::from_secs(10)).await, "ok");
}

#[tokio::test]
async fn audit_records_the_routed_project_agent_and_device_outcome() {
    let st = state(None);
    let addr = start(Arc::clone(&st)).await;
    let (mut node, _task) = node_conn(&st, "n1").await;
    handshake(&mut node).await;
    for (i, body) in [
        online("n1"),
        Body::AgentStarted(v1::AgentStarted {
            agent_id: "a1".into(),
            node_id: "n1".into(),
            project_id: "p1".into(),
            ..Default::default()
        }),
        Body::ApprovalRequested(v1::ApprovalRequested {
            approval_id: "ap1".into(),
            agent_id: "a1".into(),
            action: "review".into(),
            ..Default::default()
        }),
    ]
    .into_iter()
    .enumerate()
    {
        node.send(&f(
            STREAM_EVENTS,
            kinds::EVENT,
            event("n1", (i + 1) as u64, body),
        ))
        .await
        .unwrap();
        next_ack(&mut node).await;
    }

    let mut client = dial(addr, Some("good-token")).await;
    subscribe(&mut client, v1::Subscribe::default().encode_to_vec()).await;
    assert_eq!(recv(&mut client).await.kind, kinds::SNAPSHOT);
    for (i, accepted) in [false, true, true].into_iter().enumerate() {
        let req = v1::CommandRequest {
            request_id: format!("req-{i}"),
            target_node: "n1".into(),
            idempotency_key: if i == 2 {
                "cached-key".into()
            } else {
                String::new()
            },
            command: Some(command_request::Command::ApprovalResolve(
                v1::ApprovalResolve {
                    approval_id: "ap1".into(),
                    signer_device_id: format!("device-{i}"),
                    ..Default::default()
                },
            )),
        };
        client
            .send(&f(STREAM_COMMANDS, 0, req.encode_to_vec()))
            .await
            .unwrap();
        let msg = recv(&mut node).await;
        let routed = v1::CommandRequest::decode(msg.payload.as_slice()).unwrap();
        let result = if accepted {
            command_response::Result::Ok(v1::CommandOk {
                result: Some(command_ok::Result::Empty(v1::Empty {})),
            })
        } else {
            command_response::Result::Err(v1::ErrorInfo {
                code: "approval.signature_invalid".into(),
                ..Default::default()
            })
        };
        let response = v1::CommandResponse {
            request_id: routed.request_id,
            result: Some(result),
        };
        node.send(&f(STREAM_COMMANDS, 1, response.encode_to_vec()))
            .await
            .unwrap();
        assert_eq!(self::response(&mut client).await.request_id, req.request_id);
    }
    let rows = agora_hub::audit::Audit::new(st.hub.store())
        .list(0, None, 10)
        .unwrap();
    let rows: Vec<_> = rows
        .iter()
        .filter(|r| r.action == "ApprovalResolve")
        .map(agora_hub::audit::json_value)
        .collect();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["outcome"], "approval.signature_invalid");
    assert_eq!(rows[1]["outcome"], "ok");
    assert_eq!(rows[2]["outcome"], "ok");
    println!("CONTROL OK: the routed outcomes appear in the audit");
    if rows[1]["project_id"].is_null() {
        println!("PROBLEM CONFIRMED: project, agent and signing device missing from {rows:?}");
    }
    for row in &rows {
        assert_eq!(row["project_id"], "p1");
        assert_eq!(row["agent_id"], "a1");
    }
    assert_eq!(rows[0]["requested_device_id"], "device-0");
    assert_eq!(rows[0]["signer_device_id"], "");
    assert_eq!(rows[1]["requested_device_id"], "device-1");
    assert_eq!(rows[1]["signer_device_id"], "device-1");
    assert_eq!(rows[2]["requested_device_id"], "device-2");
    assert_eq!(rows[2]["signer_device_id"], "");
}

/// ADR-0058: a mode or model switch is never sent to a node that cannot do it, and every switch, refused or sent, is in the audit
/// trail.
#[tokio::test]
async fn a_model_switch_for_a_node_without_it_is_refused_and_audited() {
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
    subscribe(&mut client, v1::Subscribe::default().encode_to_vec()).await;
    assert_eq!(recv(&mut client).await.kind, kinds::SNAPSHOT);
    for (id, command) in [
        (
            "m1",
            command_request::Command::AgentSetModel(v1::AgentSetModel {
                agent_id: "agent-1".into(),
                model: "fake/model-2".into(),
            }),
        ),
        (
            "m2",
            command_request::Command::AgentSetMode(v1::AgentSetMode {
                agent_id: "agent-1".into(),
                mode: "plan".into(),
            }),
        ),
    ] {
        let req = v1::CommandRequest {
            request_id: id.into(),
            idempotency_key: String::new(),
            target_node: "n1".into(),
            command: Some(command),
        };
        client
            .send(&f(STREAM_COMMANDS, 0, req.encode_to_vec()))
            .await
            .unwrap();
        let r = response(&mut client).await;
        assert_eq!(
            (r.request_id.as_str(), err_code(&r)),
            (id, "protocol.capability_missing")
        );
    }
    let rows = agora_hub::audit::Audit::new(st.hub.store())
        .list(0, None, 100)
        .unwrap();
    let seen: Vec<(&str, &str, &str)> = rows
        .iter()
        .map(|e| (e.action.as_str(), e.node.as_str(), e.outcome.as_str()))
        .collect();
    assert_eq!(
        seen,
        [
            ("AgentSetModel", "n1", "protocol.capability_missing"),
            ("AgentSetMode", "n1", "protocol.capability_missing"),
        ]
    );
}

/// Like [`node_conn`], with a short silence deadline (the hub's own is `server::NODE_SILENCE`).
async fn node_conn_within(
    state: &Arc<ServerState>,
    node: &'static str,
    silence: Duration,
) -> (Conn<TcpStream>, tokio::task::JoinHandle<bool>) {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let st = Arc::clone(state);
    let task = tokio::spawn(async move {
        let (s, _) = l.accept().await.unwrap();
        let (conn, _) = ws::accept_with_token(s).await.unwrap();
        matches!(
            server::serve_node_within(conn, node, &st.hub, &st.support, silence).await,
            Err(server::ServeError::Silent)
        )
    });
    (dial(addr, None).await, task)
}

#[tokio::test]
async fn a_node_that_goes_silent_on_an_open_connection_is_closed_and_unreachable_for_commands() {
    let st = state(None);
    let silence = Duration::from_millis(400);
    let (mut c, task) = node_conn_within(&st, "laptop", silence).await;
    let from = handshake(&mut c).await.from_seq;
    c.send(&f(
        STREAM_EVENTS,
        kinds::EVENT,
        event("laptop", from, online("laptop")),
    ))
    .await
    .unwrap();
    assert_eq!(next_ack(&mut c).await, from);
    // The hub still answers a ping.
    c.send(&f(STREAM_CONTROL, kinds::PING, b"x".to_vec()))
        .await
        .unwrap();
    let pong = recv(&mut c).await;
    assert_eq!(
        (pong.stream_id, pong.kind, pong.payload.as_slice()),
        (STREAM_CONTROL, kinds::PONG, b"x".as_slice())
    );
    assert!(st.hub.view().0.nodes["laptop"].online);
    // Then nothing: the node froze, but the connection stays open.
    let started = std::time::Instant::now();
    let silent = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("the hub must end a silent connection")
        .unwrap();
    assert!(silent, "ended for silence");
    assert!(started.elapsed() >= silence / 2);
    // Commands to it fail at once with `node.offline`, and the connection is closed so it dials again as soon as it runs.
    assert!(!st.hub.router().is_connected("laptop"));
    assert!(closed(&mut c).await, "the hub closed the silent connection");
    // ADR-0070: recorded offline as silent, with when it was last heard (the ping, before the silence began).
    let view = st.hub.view().0;
    assert!(!view.nodes["laptop"].online);
    assert_eq!(
        links(&st, "laptop").last().copied(),
        Some((false, v1::NodeLinkReason::Silent, 1))
    );
    let heard = view.nodes["laptop"].last_event_ms;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    assert!(
        heard <= now.saturating_sub(silence.as_millis() as u64 / 2) && heard + 10_000 > now,
        "last seen {heard} is about when the ping arrived (now {now})"
    );
}

#[tokio::test]
async fn an_idle_node_that_pings_is_never_dropped() {
    // A quarter of the deadline between pings. The deadline is wide enough that a loaded CI runner pausing this test cannot reach
    // it; the ratio is what the test is about.
    idle_node_pinging_every(Duration::from_millis(1600), 4, 6).await;
}

/// ADR-0070 compatibility: a node from before it pings every 15 s against the hub's 25 s deadline (3/5 of it). It is never dropped.
#[tokio::test]
async fn an_old_node_that_pings_every_15_s_is_never_dropped() {
    // 15/25 of the deadline between pings. A 500 ms deadline left 200 ms of slack, which loaded macOS runners used up (the hub
    // closed the link for silence); 2 s leaves 800 ms at the same ratio.
    let silence = Duration::from_millis(2000);
    idle_node_pinging_every(silence, 0, 4).await;
}

/// An idle node that only pings, every `silence / quarters` (or every 3/5 of it when `quarters` is 0), `pings` times.
async fn idle_node_pinging_every(silence: Duration, quarters: u32, pings: u32) {
    let every = if quarters == 0 {
        silence * 3 / 5
    } else {
        silence / quarters
    };
    let st = state(None);
    let (mut c, task) = node_conn_within(&st, "laptop", silence).await;
    let from = handshake(&mut c).await.from_seq;
    c.send(&f(
        STREAM_EVENTS,
        kinds::EVENT,
        event("laptop", from, online("laptop")),
    ))
    .await
    .unwrap();
    assert_eq!(next_ack(&mut c).await, from);
    // Several deadlines with nothing on the link but pings.
    for _ in 0..pings {
        tokio::time::sleep(every).await;
        c.send(&f(STREAM_CONTROL, kinds::PING, Vec::new()))
            .await
            .unwrap();
        let pong = recv(&mut c).await;
        assert_eq!((pong.stream_id, pong.kind), (STREAM_CONTROL, kinds::PONG));
    }
    assert!(!task.is_finished());
    assert!(st.hub.view().0.nodes["laptop"].online);
    c.close().await;
    assert!(!task.await.unwrap(), "ended by the node, not for silence");
}

#[tokio::test]
async fn a_window_that_pings_the_hub_gets_a_pong() {
    let st = state(None);
    let addr = start(Arc::clone(&st)).await;
    let mut c = dial(addr, Some("good-token")).await;
    subscribe(&mut c, v1::Subscribe::default().encode_to_vec()).await;
    let snap = recv(&mut c).await;
    assert_eq!(
        (snap.stream_id, snap.kind),
        (STREAM_EVENTS, kinds::SNAPSHOT)
    );
    c.send(&f(STREAM_CONTROL, kinds::PING, b"w1".to_vec()))
        .await
        .unwrap();
    let pong = recv(&mut c).await;
    assert_eq!(
        (pong.stream_id, pong.kind, pong.payload.as_slice()),
        (STREAM_CONTROL, kinds::PONG, b"w1".as_slice())
    );
}

/// A node frozen past the deadline (a laptop asleep, a stopped process) and then resumed: the hub has closed its connection and
/// recorded it offline (`NodeLink { up: false, silent }`), so the node sees the end of the stream and dials again, and the new
/// connection records it online by itself (ADR-0070), with nothing it runs reset.
#[tokio::test]
async fn a_node_frozen_past_the_deadline_is_disconnected_then_dials_again_and_is_online() {
    let st = state(None);
    let silence = Duration::from_millis(300);
    let (mut c, task) = node_conn_within(&st, "laptop", silence).await;
    let from = handshake(&mut c).await.from_seq;
    for (i, b) in [online("laptop"), agent("laptop", "agent-1")]
        .into_iter()
        .enumerate()
    {
        c.send(&f(
            STREAM_EVENTS,
            kinds::EVENT,
            event("laptop", from + i as u64, b),
        ))
        .await
        .unwrap();
    }
    assert_eq!(next_ack(&mut c).await, from + 1);
    // Frozen: nothing read, nothing sent, for longer than the deadline.
    tokio::time::sleep(silence * 3).await;
    assert!(task.await.unwrap(), "ended for silence");
    assert!(!st.hub.router().is_connected("laptop"));
    // Resumed: the first thing the node reads is the end of the connection, so it dials again.
    assert!(closed(&mut c).await, "the hub closed the silent connection");
    let (mut again, task) = node_conn_within(&st, "laptop", silence).await;
    handshake(&mut again).await;
    again
        .send(&f(STREAM_CONTROL, kinds::PING, Vec::new()))
        .await
        .unwrap();
    let pong = recv(&mut again).await;
    assert_eq!((pong.stream_id, pong.kind), (STREAM_CONTROL, kinds::PONG));
    assert!(st.hub.router().is_connected("laptop"));
    assert!(
        st.hub.view().0.nodes["laptop"].online,
        "online again on the new connection"
    );
    assert!(
        st.hub.view().0.agents["laptop/agent-1"].running,
        "its agents are untouched"
    );
    assert_eq!(
        links(&st, "laptop"),
        vec![
            (true, v1::NodeLinkReason::Opened, 1),
            (false, v1::NodeLinkReason::Silent, 1),
            (true, v1::NodeLinkReason::Opened, 2),
        ]
    );
    again.close().await;
    let _ = task.await;
}

fn terminal_cmd(stream: u32, id: &str, command: command_request::Command) -> Frame {
    let req = v1::CommandRequest {
        request_id: id.into(),
        idempotency_key: String::new(),
        target_node: "n9".into(),
        command: Some(command),
    };
    f(stream, 0, req.encode_to_vec())
}

fn subscribe_to(stream: u32, id: &str) -> Frame {
    terminal_cmd(
        stream,
        id,
        command_request::Command::TerminalSubscribe(v1::TerminalSubscribe {
            terminal_id: "t-a".into(),
            format: String::new(),
            history_lines: 0,
        }),
    )
}

fn resize_to(id: &str, cols: u32, rows: u32) -> Frame {
    terminal_cmd(
        STREAM_COMMANDS,
        id,
        command_request::Command::TerminalResize(v1::TerminalResize {
            terminal_id: "t-a".into(),
            cols,
            rows,
        }),
    )
}

/// The next command the node is sent (closes of relayed streams are skipped), answered with an empty ok.
async fn node_command(node: &mut Conn<TcpStream>) -> v1::CommandRequest {
    loop {
        let fr = recv(node).await;
        if fr.kind != 0 || !(fr.stream_id == STREAM_COMMANDS || fr.stream_id >= 3) {
            continue;
        }
        let req = v1::CommandRequest::decode(fr.payload.as_slice()).unwrap();
        let ok = v1::CommandResponse {
            request_id: req.request_id.clone(),
            result: Some(command_response::Result::Ok(v1::CommandOk {
                result: Some(command_ok::Result::Empty(v1::Empty {})),
            })),
        };
        node.send(&f(STREAM_COMMANDS, 1, ok.encode_to_vec()))
            .await
            .unwrap();
        return req;
    }
}

fn size_of(req: &v1::CommandRequest) -> (u32, u32) {
    match &req.command {
        Some(command_request::Command::TerminalResize(t)) => (t.cols, t.rows),
        other => panic!("expected a resize, got {other:?}"),
    }
}

async fn signed_in(addr: std::net::SocketAddr) -> Conn<TcpStream> {
    let mut c = dial(addr, Some("good-token")).await;
    subscribe(&mut c, v1::Subscribe::default().encode_to_vec()).await;
    assert_eq!(recv(&mut c).await.kind, kinds::SNAPSHOT);
    c
}

/// The answer to `id` on stream 1 (terminal frames in between are skipped).
async fn answer_to(c: &mut Conn<TcpStream>, id: &str) -> v1::CommandResponse {
    loop {
        let fr = recv(c).await;
        if (fr.stream_id, fr.kind) == (STREAM_COMMANDS, 1) {
            let r = v1::CommandResponse::decode(fr.payload.as_slice()).unwrap();
            if r.request_id == id {
                return r;
            }
        }
    }
}

#[tokio::test]
async fn windows_sharing_a_terminal_give_it_the_smallest_size_and_it_grows_back_when_the_small_one_leaves(
) {
    let st = state(None);
    let addr = start(Arc::clone(&st)).await;
    let (mut node, _t) = node_conn(&st, "n9").await;
    handshake(&mut node).await;
    node.send(&f(
        STREAM_EVENTS,
        kinds::EVENT,
        event("n9", 1, online("n9")),
    ))
    .await
    .unwrap();
    next_ack(&mut node).await;

    // The big window watches first: its size goes through.
    let mut big = signed_in(addr).await;
    big.send(&subscribe_to(3, "s1")).await.unwrap();
    assert!(matches!(
        node_command(&mut node).await.command,
        Some(command_request::Command::TerminalSubscribe(_))
    ));
    assert!(matches!(
        answer_to(&mut big, "s1").await.result,
        Some(command_response::Result::Ok(_))
    ));
    big.send(&resize_to("r1", 200, 50)).await.unwrap();
    assert_eq!(size_of(&node_command(&mut node).await), (200, 50));
    answer_to(&mut big, "r1").await;

    // A smaller window opens the same terminal: the node is sent the smallest of each.
    let mut small = signed_in(addr).await;
    small.send(&subscribe_to(3, "s2")).await.unwrap();
    node_command(&mut node).await;
    answer_to(&mut small, "s2").await;
    small.send(&resize_to("r2", 100, 60)).await.unwrap();
    assert_eq!(size_of(&node_command(&mut node).await), (100, 50));
    answer_to(&mut small, "r2").await;

    // The big window saying its size again changes nothing: the hub answers, the node hears nothing.
    big.send(&resize_to("r3", 200, 50)).await.unwrap();
    assert!(matches!(
        answer_to(&mut big, "r3").await.result,
        Some(command_response::Result::Ok(_))
    ));

    // The small window closes: the big one gets its own size back, once.
    drop(small);
    assert_eq!(size_of(&node_command(&mut node).await), (200, 50));
    // It reopens and shrinks the terminal again.
    let mut small = signed_in(addr).await;
    small.send(&subscribe_to(3, "s3")).await.unwrap();
    node_command(&mut node).await;
    answer_to(&mut small, "s3").await;
    small.send(&resize_to("r4", 100, 60)).await.unwrap();
    assert_eq!(size_of(&node_command(&mut node).await), (100, 50));
    answer_to(&mut small, "r4").await;
    // Closing only its terminal view does the same as closing the window.
    small
        .send(&f(3, agora_protocol::frame::STREAM_CLOSE, Vec::new()))
        .await
        .unwrap();
    assert_eq!(size_of(&node_command(&mut node).await), (200, 50));
}

async fn forget_node(client: &mut Conn<TcpStream>, id: &str, target: &str) -> v1::CommandResponse {
    let req = v1::CommandRequest {
        request_id: "fn".into(),
        idempotency_key: String::new(),
        target_node: target.into(),
        command: Some(command_request::Command::NodeForget(v1::NodeForget {
            node_id: id.into(),
            refuse_return: false,
        })),
    };
    client
        .send(&f(STREAM_COMMANDS, 0, req.encode_to_vec()))
        .await
        .unwrap();
    response(client).await
}

/// ADR-0073: the owner forgets an offline machine; every client hears it. A connected machine is refused, and so is a request that
/// names a target machine (the hub answers it).
#[tokio::test]
async fn an_offline_machine_is_forgotten_for_every_client() {
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
    subscribe(&mut client, v1::Subscribe::default().encode_to_vec()).await;
    assert_eq!(recv(&mut client).await.kind, kinds::SNAPSHOT);
    assert_eq!(
        err_code(&forget_node(&mut client, "n1", "").await),
        "node.online"
    );
    assert_eq!(
        err_code(&forget_node(&mut client, "n1", "n1").await),
        "protocol.invalid_message"
    );
    // The machine goes away; once the hub has it offline, forgetting it works and is heard as an event.
    drop(node);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let r = forget_node(&mut client, "n1", "").await;
        if matches!(r.result, Some(command_response::Result::Ok(_))) {
            break;
        }
        assert_eq!(err_code(&r), "node.online");
        assert!(
            tokio::time::Instant::now() < deadline,
            "the machine never went offline"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let (view, _) = st.hub.view();
    assert!(view.nodes["n1"].forgotten);
}

/// The whole raw HTTP response to one request.
async fn raw(addr: std::net::SocketAddr, method: &str, path: &str, token: Option<&str>) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let auth = token.map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"));
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(format!("{method} {path} HTTP/1.1\r\nHost: x\r\n{auth}\r\n").as_bytes())
        .await
        .unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    out
}

async fn get_metrics(addr: std::net::SocketAddr, token: Option<&str>) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let auth = token.map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"));
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(format!("GET /api/metrics HTTP/1.1\r\nHost: x\r\n{auth}\r\n").as_bytes())
        .await
        .unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    let status = out[9..12].parse().unwrap();
    (
        status,
        out.split("\r\n\r\n").nth(1).unwrap_or("").to_string(),
    )
}

/// R8: `GET /api/metrics` is not there until the hub is started with `--metrics`; then only the owner may read it, and it counts what
/// the hub stored.
#[tokio::test]
async fn only_the_owner_reads_the_hubs_metrics_and_only_when_they_are_on() {
    use agora_hub::signin::{HubSessions, SignIn};
    use agora_hub::users::{Role, Users};
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
    let addr = start(Arc::clone(&st)).await;
    let signin = SignIn::new(hub.store(), Arc::new(Lockout::new()));
    let now = server::now_ms();
    let owner = signin
        .redeem(&signin.create_code(now).unwrap(), now)
        .unwrap();
    let eda = Users::new(hub.store())
        .add("Eda", Role::Viewer, now)
        .unwrap();
    let viewer = signin
        .redeem(&signin.create_code_for(Some(&eda.id), now).unwrap(), now)
        .unwrap();

    // Off: answered exactly as an unknown path, for GET and HEAD, with or without the owner's token (#562 review).
    for method in ["GET", "HEAD"] {
        for token in [None, Some(owner.as_str())] {
            assert_eq!(
                raw(addr, method, "/api/metrics", token).await,
                raw(addr, method, "/api/nothing", token).await,
                "{method} with token {}",
                token.is_some()
            );
        }
    }
    hub.enable_metrics();
    assert_eq!(get_metrics(addr, None).await.0, 401);
    assert_eq!(get_metrics(addr, Some("nope")).await.0, 401);
    assert_eq!(get_metrics(addr, Some(&viewer)).await.0, 403);

    // A machine sends three events; the owner reads them counted.
    let (mut node, _t) = node_conn(&st, "n1").await;
    handshake(&mut node).await;
    for (i, b) in [online("n1"), agent("n1", "a1"), agent("n1", "a2")]
        .into_iter()
        .enumerate()
    {
        node.send(&f(
            STREAM_EVENTS,
            kinds::EVENT,
            event("n1", i as u64 + 1, b),
        ))
        .await
        .unwrap();
        next_ack(&mut node).await;
    }
    // An approval asked and answered 12 s later on the machine's clock: one wait of 12 s.
    let at = |seq: u64, at_ms: u64, body: Body| {
        v1::Event {
            producer_id: "n1".into(),
            seq,
            at_ms,
            body: Some(body),
        }
        .encode_to_vec()
    };
    let asked = Body::ApprovalRequested(v1::ApprovalRequested {
        approval_id: "ap1".into(),
        agent_id: "a1".into(),
        action: "run_command".into(),
        ..Default::default()
    });
    let answered = Body::ApprovalResolved(v1::ApprovalResolved {
        approval_id: "ap1".into(),
        approved: true,
        ..Default::default()
    });
    for e in [at(4, 100_000, asked), at(5, 112_000, answered)] {
        node.send(&f(STREAM_EVENTS, kinds::EVENT, e)).await.unwrap();
        next_ack(&mut node).await;
    }
    let (status, text) = get_metrics(addr, Some(&owner)).await;
    assert_eq!(status, 200, "{text}");
    for line in [
        "agora_hub_events_ingested_total 5\n",
        "agora_hub_ingest_seconds_count 5\n",
        "agora_hub_lock_wait_seconds_count 5\n",
        "agora_hub_nodes_online 1\n",
        "agora_hub_resyncs_total 0\n",
        "agora_hub_node_commands_queued 0\n",
        "agora_hub_node_commands_unanswered 0\n",
        "agora_hub_approvals_waiting 0\n",
        "agora_hub_approval_wait_seconds_bucket{le=\"5\"} 0\n",
        "agora_hub_approval_wait_seconds_bucket{le=\"15\"} 1\n",
        "agora_hub_approval_wait_seconds_sum 12\n",
        "agora_hub_approval_wait_seconds_count 1\n",
        "agora_hub_relays_dropped_total 0\n",
        "agora_hub_terminal_bytes_total 0\n",
    ] {
        assert!(text.contains(line), "missing {line:?} in:\n{text}");
    }
}

/// An unstarted-metrics hub (or one with them on) that checks tokens against sessions, for the metrics token tests (ADR-0076).
async fn metrics_hub() -> (Arc<Hub>, Arc<ServerState>, std::net::SocketAddr) {
    use agora_hub::signin::HubSessions;
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
    let addr = start(Arc::clone(&st)).await;
    (hub, st, addr)
}

/// ADR-0076: a metrics token reads `/api/metrics` (GET and HEAD) and is refused once revoked or expired; a made-up one, a session of
/// a viewer and no token are refused as before; the owner's session still works.
#[tokio::test]
async fn a_metrics_token_reads_the_numbers_until_it_is_revoked_or_expires() {
    use agora_hub::metrics_token::MetricsTokens;
    use agora_hub::signin::SignIn;
    use agora_hub::users::{Role, Users};
    let (hub, _st, addr) = metrics_hub().await;
    hub.enable_metrics();
    let now = server::now_ms();
    let tokens = MetricsTokens::new(hub.store());
    let live = tokens.create("prom", now, None).unwrap();
    let revoked = tokens.create("old", now, None).unwrap();
    let expired = tokens.create("short", now, Some(now + 1)).unwrap();
    tokens.revoke("old", now).unwrap();
    let signin = SignIn::new(hub.store(), Arc::new(Lockout::new()));
    let owner = signin
        .redeem(&signin.create_code(now).unwrap(), now)
        .unwrap();
    let eda = Users::new(hub.store())
        .add("Eda", Role::Viewer, now)
        .unwrap();
    let viewer = signin
        .redeem(&signin.create_code_for(Some(&eda.id), now).unwrap(), now)
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;

    let (status, text) = get_metrics(addr, Some(&live)).await;
    assert_eq!(status, 200, "{text}");
    assert!(text.contains("agora_hub_events_ingested_total"));
    assert!(raw(addr, "HEAD", "/api/metrics", Some(&live))
        .await
        .starts_with("HTTP/1.1 200"));
    assert_eq!(get_metrics(addr, Some(&owner)).await.0, 200);
    assert_eq!(get_metrics(addr, Some(&viewer)).await.0, 403);
    // Refused as an unknown token: revoked, expired, made up (same shape), no token.
    for t in [Some(revoked.as_str()), Some(expired.as_str())] {
        assert_eq!(get_metrics(addr, t).await.0, 401);
    }
    assert_eq!(get_metrics(addr, Some("agm_0000")).await.0, 401);
    assert_eq!(get_metrics(addr, None).await.0, 401);
    // Revoking is immediate: the very next scrape fails.
    assert_eq!(get_metrics(addr, Some(&live)).await.0, 200);
    MetricsTokens::new(hub.store())
        .revoke("prom", server::now_ms())
        .unwrap();
    assert_eq!(get_metrics(addr, Some(&live)).await.0, 401);
}

/// ADR-0076 item 4: with `--metrics` off, a metrics token is answered exactly as an unknown path, as any other bearer.
#[tokio::test]
async fn with_metrics_off_a_metrics_token_is_answered_as_an_unknown_path() {
    let (hub, _st, addr) = metrics_hub().await;
    let token = agora_hub::metrics_token::MetricsTokens::new(hub.store())
        .create("prom", server::now_ms(), None)
        .unwrap();
    for method in ["GET", "HEAD"] {
        assert_eq!(
            raw(addr, method, "/api/metrics", Some(&token)).await,
            raw(addr, method, "/api/nothing", Some(&token)).await,
        );
    }
}

/// ADR-0076 item 1, negative controls: the token is an unknown token everywhere but `/api/metrics`. Each route answers it as it
/// answers a made-up one, it starts no session, and the WebSocket closes on it.
#[tokio::test]
async fn a_metrics_token_reaches_nothing_but_the_metrics() {
    let (hub, _st, addr) = metrics_hub().await;
    hub.enable_metrics();
    let token = agora_hub::metrics_token::MetricsTokens::new(hub.store())
        .create("prom", server::now_ms(), None)
        .unwrap();
    let status = |r: &str| r[9..12].to_string();
    for (method, path) in [
        ("GET", "/api/audit"),
        ("GET", "/api/audit?since=0"),
        ("DELETE", "/api/session"),
    ] {
        let with = raw(addr, method, path, Some(&token)).await;
        let made_up = raw(addr, method, path, Some("agm_deadbeef")).await;
        assert_eq!(with, made_up, "{method} {path}");
        assert_ne!(status(&with), "200", "{method} {path}: {with}");
    }
    // The routes that act as a signed-in person refuse it like a made-up token.
    for path in ["/api/password", "/api/passkey/add"] {
        let body = serde_json::json!({});
        let (with, _) = post_json(addr, path, Some(&token), &body).await;
        let (made_up, _) = post_json(addr, path, Some("agm_deadbeef"), &body).await;
        assert_eq!(with, made_up, "{path}");
        assert!(matches!(with, 401 | 403 | 404), "{path}: {with}");
    }
    // It is not a session: it cannot sign anything out, and the audit trail stays closed to it.
    assert_eq!(
        status(&raw(addr, "GET", "/api/audit", Some(&token)).await),
        "401"
    );
    // The WebSocket treats it as an unknown token.
    let tcp = TcpStream::connect(addr).await.unwrap();
    let mut c = ws::connect_with_subprotocol_token(tcp, &format!("ws://{addr}/"), &token)
        .await
        .unwrap();
    // Refused before any Welcome: no subscription is answered.
    let _ = c
        .send(&f(
            STREAM_EVENTS,
            kinds::SUBSCRIBE,
            v1::Subscribe::default().encode_to_vec(),
        ))
        .await;
    assert!(closed(&mut c).await);
    // And a scrape did not start or renew any session.
    assert_eq!(get_metrics(addr, Some(&token)).await.0, 200);
    let signin = agora_hub::signin::SignIn::new(hub.store(), Arc::new(Lockout::new()));
    assert!(!signin.check(&token, server::now_ms()));
    assert!(signin.access(&token, server::now_ms()).is_none());
}

/// ADR-0076 item 4: a burst of wrong bearers on `/api/metrics` locks the address out (429), the lockout of the sign-in.
#[tokio::test]
async fn wrong_bearers_on_the_metrics_path_count_toward_the_lockout() {
    let (hub, _st, addr) = metrics_hub().await;
    hub.enable_metrics();
    let mut last = 0;
    for _ in 0..agora_hub::password::ADDRESS_FREE_FAILURES + 1 {
        last = get_metrics(addr, Some("agm_wrong")).await.0;
    }
    assert_eq!(last, 429);
    // Locked out, even a good token waits.
    let token = agora_hub::metrics_token::MetricsTokens::new(hub.store())
        .create("prom", server::now_ms(), None)
        .unwrap();
    assert_eq!(get_metrics(addr, Some(&token)).await.0, 429);
}

/// Integration review F1: a valid metrics token interleaved with wrong tries must not wipe the address's failure count; the wrong
/// ones lock the address out exactly as they do without the token.
#[tokio::test]
async fn a_valid_metrics_token_does_not_clear_the_failures_of_wrong_tries() {
    let (hub, _st, addr) = metrics_hub().await;
    hub.enable_metrics();
    let token = agora_hub::metrics_token::MetricsTokens::new(hub.store())
        .create("prom", server::now_ms(), None)
        .unwrap();
    let mut locked_after = None;
    for round in 1..=agora_hub::password::ADDRESS_FREE_FAILURES + 2 {
        get_metrics(addr, Some("agm_wrong")).await;
        // The wrong try that spends the last free one starts the lockout; the good token then waits too.
        if get_metrics(addr, Some(&token)).await.0 == 429 {
            locked_after = Some(round);
            break;
        }
    }
    assert_eq!(
        locked_after,
        Some(agora_hub::password::ADDRESS_FREE_FAILURES),
        "wrong tries interleaved with a valid token lock out as they do alone"
    );
}

/// #558 review: a certificate revoked between the node's Hello and its Resume ends the connection too (the handshake is watched as
/// the conversation is), and the node is never recorded online.
#[tokio::test]
async fn a_certificate_revoked_during_the_handshake_ends_the_connection() {
    // The recheck of a live connection's certificate, shortened for this test (the default is 10 s).
    std::env::set_var("AGORA_RECHECK_MS", "50");
    let st = state(None);
    let valid = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let (st2, v2) = (Arc::clone(&st), Arc::clone(&valid));
    let task = tokio::spawn(async move {
        let (s, _) = l.accept().await.unwrap();
        let (conn, _) = ws::accept_with_token(s).await.unwrap();
        let check = move || v2.load(std::sync::atomic::Ordering::SeqCst);
        server::serve_node_while(
            conn,
            "n1",
            &st2.hub,
            &st2.support,
            Duration::from_secs(30),
            &check,
        )
        .await
    });
    let mut c = dial(addr, None).await;
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
    let w = recv(&mut c).await;
    assert_eq!((w.stream_id, w.kind), (STREAM_CONTROL, kinds::WELCOME));
    // It holds the connection without resuming; its certificate is revoked meanwhile.
    valid.store(false, std::sync::atomic::Ordering::SeqCst);
    assert!(closed(&mut c).await, "the revoked node kept its connection");
    // The hub waits up to 5 s for the close to be answered; this peer just goes away.
    drop(c);
    let ended = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(&ended, Err(server::ServeError::Protocol(e)) if e.code.as_str() == "auth.device_revoked"),
        "{ended:?}"
    );
    assert!(!st.hub.view().0.nodes.get("n1").is_some_and(|n| n.online));
}

// ADR-0069, #598 cross-review (linux Codex, docs/reviews/0598-privacy-review.md on its review branch): over the real hub, a machine's
// executable kept straight in its user's home never reaches anyone but the owner with that user name in it, by any route.
struct PrivacyProbeAuth {
    role: agora_hub::users::Role,
    scoped: bool,
}

impl ClientAuth for PrivacyProbeAuth {
    fn check(&self, token: &str) -> bool {
        token == "good-token"
    }
    fn access(&self, token: &str) -> Option<agora_hub::users::Access> {
        self.check(token).then(|| agora_hub::users::Access {
            role: self.role,
            user: "reader".into(),
            projects: self.scoped.then(|| ["n1/mine".to_string()].into()),
            session: String::new(),
        })
    }
}

async fn probe_online_binary(client: &mut Conn<TcpStream>) -> String {
    loop {
        let fr = recv(client).await;
        assert_eq!(fr.kind, kinds::POSITIONED_EVENT);
        let ev = v1::PositionedEvent::decode(fr.payload.as_slice()).unwrap();
        if let Some(Body::NodeOnline(n)) = ev.event.and_then(|e| e.body) {
            return n.binary;
        }
    }
}

#[tokio::test]
async fn a_machines_user_name_in_its_binary_path_reaches_only_the_owner() {
    use agora_hub::users::Role;
    let mut leaks = Vec::new();
    let mut observations = 0;
    for path in [
        "/home/alice/agora",
        "/Users/alice/agora",
        r"C:\Users\alice\agora.exe",
    ] {
        for (role, scoped) in [
            (Role::Owner, false),
            (Role::Member, false),
            (Role::Member, true),
            (Role::Viewer, false),
            (Role::Viewer, true),
        ] {
            let st = Arc::new(ServerState {
                hub: Arc::new(Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap()),
                support: support(),
                ca: None,
                lockout: Arc::new(Lockout::new()),
                auth: Box::new(PrivacyProbeAuth { role, scoped }),
                assets: None,
                sessions: None,
            });
            let addr = start(Arc::clone(&st)).await;
            let (mut node, task) = node_conn(&st, "n1").await;
            handshake(&mut node).await;
            let online = || {
                Body::NodeOnline(v1::NodeOnline {
                    node_id: "n1".into(),
                    binary: path.into(),
                    ..Default::default()
                })
            };
            node.send(&f(STREAM_EVENTS, kinds::EVENT, event("n1", 1, online())))
                .await
                .unwrap();
            next_ack(&mut node).await;
            let mut client = dial(addr, Some("good-token")).await;
            subscribe(&mut client, v1::Subscribe::default().encode_to_vec()).await;
            let fr = recv(&mut client).await;
            assert_eq!(fr.kind, kinds::SNAPSHOT);
            let snap = v1::Snapshot::decode(fr.payload.as_slice()).unwrap();
            let mut seen = vec![("snapshot", snap.nodes[0].binary.clone())];
            node.send(&f(STREAM_EVENTS, kinds::EVENT, event("n1", 2, online())))
                .await
                .unwrap();
            next_ack(&mut node).await;
            seen.push(("live", probe_online_binary(&mut client).await));
            if !scoped {
                let mut replay = dial(addr, Some("good-token")).await;
                subscribe(
                    &mut replay,
                    v1::Subscribe { after_pos: Some(0) }.encode_to_vec(),
                )
                .await;
                seen.push(("backlog", probe_online_binary(&mut replay).await));
            }
            for (route, shown) in seen {
                observations += 1;
                if role == Role::Owner {
                    assert_eq!(shown, path, "owner full-path control");
                } else if shown.contains("alice") {
                    leaks.push(format!("{role:?} scoped={scoped} {route}: {shown}"));
                } else {
                    let file = if path.ends_with(".exe") {
                        "agora.exe"
                    } else {
                        "agora"
                    };
                    assert_eq!(shown, file, "{role:?} scoped={scoped} {route}");
                }
            }
            task.abort();
        }
    }
    assert_eq!(observations, 39, "all routes and owner controls exercised");
    assert!(
        leaks.is_empty(),
        "{} of 30 nonowner deliveries expose the fixture username: {leaks:?}",
        leaks.len()
    );
}

// #629 cross-review (linux Codex, P2): a history stored before the session refused cross-node records. The rebuilt view reads them
// safely; a reconnecting client must not be handed their stored bodies, and one that applied them before reconnecting is healed.
const SAFE_CHECKPOINT: u64 = 3;
const LAST_FORGED: u64 = 6;

fn seed_legacy(store: &SqliteStore, forged: bool) {
    let seed = |producer: &str, seq: u64, body: Body| {
        store
            .append(producer, seq, seq, &event(producer, seq, body))
            .unwrap();
    };
    seed("n1", 1, online("n1"));
    seed("n1", 2, agent("n1", "a"));
    seed("n2", 1, online("n2"));
    if forged {
        // Bytes the pre-fix session accepted and stored (the new one refuses them, so they go in through storage).
        seed(
            "n2",
            2,
            Body::AgentForgotten(v1::AgentForgotten {
                agent_id: "n1/a".into(),
            }),
        );
        seed(
            "n2",
            3,
            Body::NodeResources(v1::NodeResources {
                node_id: "n1".into(),
                agents_running: 99,
                ..Default::default()
            }),
        );
        seed(
            "n2",
            4,
            Body::CheckoutDiscovered(v1::CheckoutDiscovered {
                checkout_id: "forged".into(),
                node_id: "n1".into(),
                project_id: "p".into(),
                path: "/fixture".into(),
            }),
        );
    } else {
        seed("n2", 2, working("a"));
    }
}

fn legacy_state(forged: bool, auth: Box<dyn ClientAuth + Send + Sync>) -> Arc<ServerState> {
    let store = SqliteStore::open_in_memory().unwrap();
    seed_legacy(&store, forged);
    Arc::new(ServerState {
        // Opening records the machines offline for old clients: hub-written `NodeOffline` events after the stored ones.
        hub: Arc::new(Hub::open_at(store, 100).unwrap()),
        support: support(),
        ca: None,
        lockout: Arc::new(Lockout::new()),
        auth,
        assets: None,
        sessions: None,
    })
}

async fn subscribe_after(
    addr: std::net::SocketAddr,
    after: Option<u64>,
) -> (Conn<TcpStream>, Frame) {
    let mut c = dial(addr, Some("good-token")).await;
    subscribe(&mut c, v1::Subscribe { after_pos: after }.encode_to_vec()).await;
    let first = recv(&mut c).await;
    (c, first)
}

#[tokio::test]
async fn a_reconnecting_client_is_never_replayed_a_record_the_session_would_now_refuse() {
    let st = legacy_state(true, Box::new(Tokens));
    let (view, end) = st.hub.view();
    assert_eq!(end, 10, "three stored, three forged, four hub-written");
    // The safe projection the replay must agree with.
    assert!(!view.agents["n1/a"].forgotten);
    assert_eq!(view.nodes["n1"].agents_running, 0);
    assert_eq!(view.checkouts["n2/forged"].0, "n2");
    let addr = start(Arc::clone(&st)).await;

    let (_fresh, first) = subscribe_after(addr, None).await;
    assert_eq!(first.kind, kinds::SNAPSHOT);
    let fresh = v1::Snapshot::decode(first.payload.as_slice()).unwrap();
    assert!(fresh
        .agents
        .iter()
        .any(|a| a.agent_id == "n1/a" && a.running));

    // Before the bad records, and on the last of them (a client that applied them live and reconnects with nothing missing): each
    // gets the clean snapshot, which equals the fresh one.
    for after in [SAFE_CHECKPOINT, LAST_FORGED] {
        let (_c, first) = subscribe_after(addr, Some(after)).await;
        assert_eq!(first.kind, kinds::SNAPSHOT, "resume after {after}");
        let resumed = v1::Snapshot::decode(first.payload.as_slice()).unwrap();
        assert_eq!(resumed, fresh, "resume after {after}");
    }
}

#[tokio::test]
async fn events_from_a_hub_that_opened_over_such_history_replay_once_the_client_has_its_clean_snapshot(
) {
    let st = legacy_state(true, Box::new(Tokens));
    let addr = start(Arc::clone(&st)).await;
    // The snapshot is at the end of the log; a live event after it reaches the client as an ordinary event, and a client that
    // reconnects after it resumes by replay: the barrier is behind it.
    let (mut node, _t) = node_conn(&st, "n1").await;
    handshake(&mut node).await;
    node.send(&f(
        STREAM_EVENTS,
        kinds::EVENT,
        event("n1", 3, working("a")),
    ))
    .await
    .unwrap();
    next_ack(&mut node).await;
    let (_c, first) = subscribe_after(addr, Some(LAST_FORGED)).await;
    assert_eq!(
        first.kind,
        kinds::SNAPSHOT,
        "a client at the last refused record may hold a forgery, so it is healed"
    );
    // Position 7 is the hub's own first record after opening: nothing a client could have had from before.
    let (_c, first) = subscribe_after(addr, Some(LAST_FORGED + 1)).await;
    assert!(
        first.kind == kinds::POSITIONED_EVENT,
        "a client past the barrier replays, got kind {}",
        first.kind
    );
}

#[tokio::test]
async fn a_history_with_no_refused_records_replays_unchanged_including_the_hubs_own_events() {
    let st = legacy_state(false, Box::new(Tokens));
    let (_, end) = st.hub.view();
    assert_eq!(end, 8, "four stored, four hub-written");
    let addr = start(Arc::clone(&st)).await;
    // A checkpoint inside the stored events and one on the hub's own `NodeOffline`: both replay, never a snapshot.
    for after in [SAFE_CHECKPOINT, 5] {
        let (mut c, first) = subscribe_after(addr, Some(after)).await;
        let mut seen = Vec::new();
        let mut fr = first;
        loop {
            assert_eq!(fr.kind, kinds::POSITIONED_EVENT, "resume after {after}");
            let p = v1::PositionedEvent::decode(fr.payload.as_slice()).unwrap();
            seen.push((p.pos, p.event.unwrap()));
            if p.pos == end {
                break;
            }
            fr = recv(&mut c).await;
        }
        assert_eq!(seen[0].0, after + 1);
        let offline: Vec<_> = seen
            .iter()
            .filter(|(_, e)| e.producer_id == "hub" && matches!(e.body, Some(Body::NodeOffline(_))))
            .map(|(pos, _)| *pos)
            .collect();
        assert!(
            !offline.is_empty(),
            "the hub's own NodeOffline events are delivered: {seen:?}"
        );
        // Same-node events arrive with the body they were stored with.
        if after == SAFE_CHECKPOINT {
            assert!(matches!(
                &seen[0].1.body,
                Some(Body::AgentStateChanged(e)) if e.agent_id == "a"
            ));
        }
    }
}

#[tokio::test]
async fn a_scoped_client_reconnecting_over_such_history_gets_a_clean_cut_snapshot() {
    let st = legacy_state(true, Box::new(Scoped));
    let addr = start(Arc::clone(&st)).await;
    for after in [Some(SAFE_CHECKPOINT), Some(LAST_FORGED), None] {
        let (_c, first) = subscribe_after(addr, after).await;
        assert_eq!(first.kind, kinds::SNAPSHOT, "resume after {after:?}");
        let snap = v1::Snapshot::decode(first.payload.as_slice()).unwrap();
        // The scoped client sees the machine's agent in its project only; nothing is forgotten or reassigned.
        assert!(
            snap.checkouts
                .iter()
                .all(|c| c.checkout_id != "n2/forged" || c.node_id == "n2"),
            "{snap:?}"
        );
        assert!(snap.nodes.iter().all(|n| n.agents_running == 0), "{snap:?}");
    }
}

// ---- ADR-0067 rung 4: a trusted reverse proxy, over real sockets and the real server loop ----

/// One raw request to a real hub over loopback; the whole answer (the hub closes the connection after it).
async fn raw_request(addr: std::net::SocketAddr, request: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(request.as_bytes()).await.unwrap();
    let mut out = String::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), s.read_to_string(&mut out)).await;
    out
}

fn get_info(headers: &[&str]) -> String {
    format!(
        "GET /api/session HTTP/1.1\r\n{}\r\n\r\n",
        headers.join("\r\n")
    )
}

fn upgrade_request(headers: &[&str]) -> String {
    format!(
        "GET / HTTP/1.1\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n{}\r\n\r\n",
        headers.join("\r\n")
    )
}

/// The file that turns the proxy on is followed by a running hub (on and off, no restart); while it is on, a request from the
/// proxy (loopback here) with a foreign Host is refused, one with the real address header is never the owner, and a WebSocket
/// upgrade is checked like any request. Fake proxy headers only: no tunnel, no network service.
#[tokio::test]
async fn a_running_hub_follows_the_proxy_file_and_checks_what_comes_through_it() {
    use agora_hub::proxy::{ProxyHandle, ProxyTrust, FILE};
    use agora_hub::signin::HubSessions;
    let dir = std::env::temp_dir().join(format!("agora-proxy-server-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(FILE);

    let hub = Arc::new(Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap());
    agora_hub::password::ScreenPassword::with_iterations(hub.store(), 1_000)
        .set("long enough")
        .unwrap();
    let sessions = Arc::new(HubSessions::new(Arc::clone(&hub)).with_proxy(Arc::new(
        ProxyHandle::watch(file.clone(), |p| std::fs::File::open(p)),
    )));
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
    let status = |out: &str| out.get(9..12).unwrap_or("").to_string();

    // Off by default: nothing is checked, and the owner's browser is the owner.
    let out = raw_request(addr, &get_info(&["Host: evil.example.org"])).await;
    assert_eq!(status(&out), "200", "{out}");
    assert!(out.contains(r#""passwordRequired":false"#), "{out}");

    // The owner turns it on.
    let trust =
        ProxyTrust::new("CF-Connecting-IP", &["agora.example.com".to_string()], &[]).unwrap();
    std::fs::write(&file, trust.render()).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let out = raw_request(addr, &get_info(&["Host: evil.example.org"])).await;
        if status(&out) == "421" {
            assert!(out.contains("proxy.host_not_allowed"), "{out}");
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the hub never took the file: {out}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    // Through the proxy: the real address comes from the header, and the visitor is not the owner whatever the header says.
    for ip in ["203.0.113.9", "127.0.0.1", "::1"] {
        let out = raw_request(
            addr,
            &get_info(&[
                "Host: agora.example.com",
                &format!("CF-Connecting-IP: {ip}"),
            ]),
        )
        .await;
        assert_eq!(status(&out), "200", "{out}");
        assert!(out.contains(r#""passwordRequired":true"#), "{ip}: {out}");
    }
    // The owner's browser by a loopback name still is the owner.
    let out = raw_request(addr, &get_info(&[&format!("Host: {addr}")])).await;
    assert!(out.contains(r#""passwordRequired":false"#), "{out}");
    // A cross-site page.
    let out = raw_request(
        addr,
        &get_info(&[
            "Host: agora.example.com",
            "Origin: https://evil.example.org",
            "CF-Connecting-IP: 203.0.113.9",
        ]),
    )
    .await;
    assert_eq!(status(&out), "403", "{out}");
    // A WebSocket upgrade: a foreign Host or Origin never reaches the upgrade.
    let out = raw_request(addr, &upgrade_request(&["Host: evil.example.org"])).await;
    assert_eq!(status(&out), "421", "{out}");
    let out = raw_request(
        addr,
        &upgrade_request(&[
            "Host: agora.example.com",
            "Origin: https://evil.example.org",
            "CF-Connecting-IP: 203.0.113.9",
        ]),
    )
    .await;
    assert_eq!(status(&out), "403", "{out}");
    // A dial from this machine (a node or a test) still connects: its Host is the address it dialed.
    let _ok = dial(addr, None).await;

    // The owner turns it off: the hub follows without a restart.
    std::fs::remove_file(&file).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let out = raw_request(addr, &get_info(&["Host: evil.example.org"])).await;
        if status(&out) == "200" {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the hub kept trusting the proxy: {out}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn a_window_that_connects_later_is_sent_the_last_config_and_commands_of_a_running_agent() {
    let st = state(None);
    let addr = start(Arc::clone(&st)).await;
    let (mut node, _t) = node_conn(&st, "n1").await;
    handshake(&mut node).await;
    let bodies = vec![
        online("n1"),
        agent("n1", "a1"),
        Body::AgentSessionConfig(v1::AgentSessionConfig {
            agent_id: "a1".into(),
            options: vec![v1::SessionConfigOption {
                id: "effort".into(),
                ..Default::default()
            }],
        }),
        Body::AgentCommands(v1::AgentCommands {
            agent_id: "a1".into(),
            commands: vec![v1::AgentCommand {
                name: "review".into(),
                ..Default::default()
            }],
        }),
    ];
    for (i, b) in bodies.into_iter().enumerate() {
        let seq = i as u64 + 1;
        node.send(&f(STREAM_EVENTS, kinds::EVENT, event("n1", seq, b)))
            .await
            .unwrap();
        next_ack(&mut node).await;
    }
    let mut client = dial(addr, Some("good-token")).await;
    subscribe(&mut client, v1::Subscribe::default().encode_to_vec()).await;
    let snap = recv(&mut client).await;
    assert_eq!(snap.kind, kinds::SNAPSHOT);
    let pos = v1::Snapshot::decode(snap.payload.as_slice()).unwrap().pos;
    let mut seen = Vec::new();
    for _ in 0..2 {
        let fr = recv(&mut client).await;
        assert_eq!(fr.kind, kinds::POSITIONED_EVENT);
        let pe = v1::PositionedEvent::decode(fr.payload.as_slice()).unwrap();
        assert_eq!(pe.pos, pos, "at the snapshot's position");
        let e = pe.event.unwrap();
        assert_eq!(e.producer_id, "n1");
        seen.push(e.body.unwrap());
    }
    assert!(
        matches!(&seen[0], Body::AgentSessionConfig(c) if c.agent_id == "a1" && c.options[0].id == "effort")
    );
    assert!(matches!(&seen[1], Body::AgentCommands(c) if c.commands[0].name == "review"));
}

/// The next command the hub sends this node for which `pick` says something, skipping acks and other commands.
async fn next_command<T>(
    c: &mut Conn<TcpStream>,
    pick: impl Fn(command_request::Command) -> Option<T>,
) -> T {
    loop {
        let fr = recv(c).await;
        if (fr.stream_id, fr.kind) != (STREAM_COMMANDS, 0) {
            continue;
        }
        let req = v1::CommandRequest::decode(fr.payload.as_slice()).unwrap();
        if let Some(t) = req.command.and_then(&pick) {
            return t;
        }
    }
}

/// ADR-0115 over real sockets: an older node whose clone's remote is `…/ersinkoc/Agora` and a newer one that folds it to
/// `…/ersinkoc/agora` are one project. The window gets the older node's events with the folded id (as its snapshot has them), the
/// board asks both to join, and the join and a window's start reach the older node in its own spelling.
#[tokio::test]
async fn an_older_and_a_newer_node_with_two_cases_of_one_github_repository_are_one_project() {
    let root = "db6f7d0123456789abcdef0123456789abcdef01";
    let old_id = format!("github.com/ersinkoc/Agora#{root}");
    let folded = format!("github.com/ersinkoc/agora#{root}");
    let st = Arc::new(ServerState {
        hub: Arc::new(Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap()),
        support: Support {
            generations: vec![1],
            capabilities: vec!["board.join".into()],
            required: vec![],
        },
        ca: None,
        lockout: Arc::new(Lockout::new()),
        auth: Box::new(Tokens),
        assets: None,
        sessions: None,
    });
    let addr = start(Arc::clone(&st)).await;
    let mut client = dial(addr, Some("good-token")).await;
    subscribe(&mut client, v1::Subscribe::default().encode_to_vec()).await;
    assert_eq!(recv(&mut client).await.kind, kinds::SNAPSHOT);

    let mut nodes = Vec::new();
    for (node, project) in [("old", &old_id), ("new", &folded)] {
        let (mut conn, task) = node_conn(&st, node).await;
        handshake_offering(&mut conn, &["board.join"]).await;
        for (seq, body) in [
            (1, online(node)),
            (
                2,
                Body::CheckoutDiscovered(v1::CheckoutDiscovered {
                    checkout_id: "c".into(),
                    node_id: node.into(),
                    project_id: project.clone(),
                    path: "/src/agora".into(),
                }),
            ),
            (
                3,
                Body::AgentStarted(v1::AgentStarted {
                    project_id: project.clone(),
                    ..match agent(node, "a") {
                        Body::AgentStarted(a) => a,
                        _ => unreachable!(),
                    }
                }),
            ),
        ] {
            conn.send(&f(STREAM_EVENTS, kinds::EVENT, event(node, seq, body)))
                .await
                .unwrap();
        }
        nodes.push((conn, task));
    }

    // The window: every project id it is sent is the folded one.
    let mut seen = 0;
    while seen < 4 {
        let fr = recv(&mut client).await;
        if fr.kind != kinds::POSITIONED_EVENT {
            continue;
        }
        let ev = v1::PositionedEvent::decode(fr.payload.as_slice())
            .unwrap()
            .event
            .unwrap();
        let id = match ev.body {
            Some(Body::CheckoutDiscovered(c)) => c.project_id,
            Some(Body::AgentStarted(a)) => a.project_id,
            _ => continue,
        };
        assert_eq!(id, folded, "from {}", ev.producer_id);
        seen += 1;
    }
    // A window that connects now: the snapshot has one project on both machines.
    let mut later = dial(addr, Some("good-token")).await;
    subscribe(&mut later, v1::Subscribe::default().encode_to_vec()).await;
    let snap = v1::Snapshot::decode(recv(&mut later).await.payload.as_slice()).unwrap();
    assert_eq!(snap.checkouts.len(), 2);
    assert!(snap.checkouts.iter().all(|c| c.project_id == folded));
    assert!(snap.agents.iter().all(|a| a.project_id == folded));

    // The project is shared: both are asked to join its board, each in its own spelling.
    let (mut old, _old_task) = nodes.remove(0);
    let (mut new, _new_task) = nodes.remove(0);
    let join = |c: command_request::Command| match c {
        command_request::Command::BoardJoin(j) => Some(j.project_id),
        _ => None,
    };
    assert_eq!(next_command(&mut old, join).await, old_id);
    assert_eq!(next_command(&mut new, join).await, folded);

    // A window starts an agent in the project it was shown: the older node is asked in its own spelling.
    for (target, conn, want) in [("old", &mut old, &old_id), ("new", &mut new, &folded)] {
        let req = v1::CommandRequest {
            request_id: format!("start-{target}"),
            idempotency_key: String::new(),
            target_node: target.into(),
            command: Some(command_request::Command::AgentStart(v1::AgentStart {
                project_id: folded.clone(),
                kind: "shell".into(),
                ..Default::default()
            })),
        };
        client
            .send(&f(STREAM_COMMANDS, 0, req.encode_to_vec()))
            .await
            .unwrap();
        let got = next_command(conn, |c| match c {
            command_request::Command::AgentStart(s) => Some(s.project_id),
            _ => None,
        })
        .await;
        assert_eq!(&got, want, "{target}");
    }
}

/// A person of a team hub limited to one project, with the role the test sets.
struct InOneProject(Arc<std::sync::Mutex<agora_hub::users::Role>>);

impl ClientAuth for InOneProject {
    fn check(&self, token: &str) -> bool {
        token == "good-token"
    }
    fn access(&self, token: &str) -> Option<agora_hub::users::Access> {
        self.check(token).then(|| agora_hub::users::Access {
            role: *self.0.lock().unwrap(),
            user: "eda".into(),
            projects: Some(["n1/mine".to_string()].into()),
            session: String::new(),
        })
    }
}

fn catalog_request(id: &str, target: &str, command: command_request::Command) -> Frame {
    let req = v1::CommandRequest {
        request_id: id.into(),
        idempotency_key: String::new(),
        target_node: target.into(),
        command: Some(command),
    };
    f(STREAM_COMMANDS, 0, req.encode_to_vec())
}

/// ADR-0113 decision 6: a member or viewer limited to one project reads the theme catalog once the owner turned it on, and only then;
/// while it is off the list says so (no refusal, so no row in the owner's Activity) and a theme is refused. The hub answers both itself.
#[tokio::test]
async fn members_and_viewers_read_the_theme_catalog_only_once_the_owner_turned_it_on() {
    use agora_hub::theme_catalog::{save, set_on, Accepted, Entry};
    use agora_hub::users::Role;
    use command_request::Command as C;
    let role = Arc::new(std::sync::Mutex::new(Role::Viewer));
    let st = Arc::new(ServerState {
        hub: Arc::new(Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap()),
        support: support(),
        ca: None,
        lockout: Arc::new(Lockout::new()),
        auth: Box::new(InOneProject(Arc::clone(&role))),
        assets: None,
        sessions: None,
    });
    let data = r#"{"agora-theme": 1, "name": "Harbour", "scheme": "dark", "tokens": {}}"#;
    save(
        st.hub.store(),
        &[Accepted {
            entry: Entry {
                id: "harbour".into(),
                name: "Harbour".into(),
                version: 2,
                author: "Ada".into(),
                licence: "MIT".into(),
                sha256: "a".repeat(64),
                file: "themes/harbour.json".into(),
                added: String::new(),
                popularity: 0,
            },
            data: data.into(),
        }],
        None,
        1_700_000_000_000,
    )
    .unwrap();
    let addr = start(Arc::clone(&st)).await;
    let list = || C::ThemeCatalogList(v1::ThemeCatalogList::default());
    let get = |id: &str| C::ThemeCatalogGet(v1::ThemeCatalogGet { id: id.into() });
    for who in [Role::Viewer, Role::Member] {
        *role.lock().unwrap() = who;
        set_on(st.hub.store(), false, 1).unwrap();
        let mut client = dial(addr, Some("good-token")).await;
        subscribe(&mut client, v1::Subscribe::default().encode_to_vec()).await;
        while recv(&mut client).await.kind != kinds::SNAPSHOT {}

        client
            .send(&catalog_request("l1", "", list()))
            .await
            .unwrap();
        let r = response(&mut client).await;
        let Some(command_response::Result::Ok(v1::CommandOk {
            result: Some(command_ok::Result::ThemeCatalogList(off)),
        })) = r.result
        else {
            panic!("expected the list: {r:?}");
        };
        assert_eq!(off, v1::ThemeCatalogListResult::default(), "{who:?}");
        client
            .send(&catalog_request("g1", "", get("harbour")))
            .await
            .unwrap();
        assert_eq!(err_code(&response(&mut client).await), "theme.catalog_off");

        set_on(st.hub.store(), true, 2).unwrap();
        client
            .send(&catalog_request("l2", "", list()))
            .await
            .unwrap();
        let r = response(&mut client).await;
        let Some(command_response::Result::Ok(v1::CommandOk {
            result: Some(command_ok::Result::ThemeCatalogList(on)),
        })) = r.result
        else {
            panic!("expected the list: {r:?}");
        };
        assert!(on.on);
        assert_eq!(on.updated_at_ms, 1_700_000_000_000);
        assert_eq!(on.entries.len(), 1);
        assert_eq!(
            (
                on.entries[0].id.as_str(),
                on.entries[0].version,
                on.entries[0].size as usize
            ),
            ("harbour", 2, data.len())
        );
        client
            .send(&catalog_request("g2", "", get("harbour")))
            .await
            .unwrap();
        let r = response(&mut client).await;
        let Some(command_response::Result::Ok(v1::CommandOk {
            result: Some(command_ok::Result::ThemeCatalogTheme(theme)),
        })) = r.result
        else {
            panic!("expected the theme: {r:?}");
        };
        assert_eq!(theme.data, data);
        client
            .send(&catalog_request("g3", "", get("nope")))
            .await
            .unwrap();
        assert_eq!(err_code(&response(&mut client).await), "theme.not_found");
        // The hub answers it itself: one that names a machine is refused.
        client
            .send(&catalog_request("l3", "n1", list()))
            .await
            .unwrap();
        assert_eq!(
            err_code(&response(&mut client).await),
            "protocol.invalid_message"
        );
    }
    // Reading the catalog, on or off, wrote no refusal to the owner's Activity.
    let rows = agora_hub::audit::Audit::new(st.hub.store())
        .list(0, Some("eda"), 100)
        .unwrap();
    assert!(
        rows.iter().all(|e| e.outcome != "auth.forbidden"),
        "{rows:?}"
    );
}
