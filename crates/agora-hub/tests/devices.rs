//! ADR-0061 at the hub, over real sockets: a client registers its device and lists its own; the hub carries endorsements and
//! revocations to the nodes that verify them (now, and to a node that connects later); a node's trusted devices reach the snapshot;
//! enrollment offers the registered devices; and a node asks for them on a connection of its own, with its certificate.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use agora_hub::ca::{new_device_request, Ca};
use agora_hub::enroll::{Enrollment, Lockout};
use agora_hub::hub::Hub;
use agora_hub::server::{self, ClientAuth, ServerState};
use agora_hub::store::SqliteStore;
use agora_hub::tls;
use agora_protocol::frame::{kinds, STREAM_COMMANDS, STREAM_CONTROL, STREAM_EVENTS};
use agora_protocol::handshake::Support;
use agora_protocol::v1::{self, command_ok, command_request as cr, command_response, event::Body};
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

fn state(ca: Option<Ca>) -> Arc<ServerState> {
    Arc::new(ServerState {
        hub: Arc::new(Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap()),
        support: Support {
            generations: vec![1],
            capabilities: vec!["device.trust".into(), "device.list".into()],
            required: vec![],
        },
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

fn command(id: &str, c: cr::Command) -> Frame {
    f(
        STREAM_COMMANDS,
        0,
        v1::CommandRequest {
            request_id: id.into(),
            idempotency_key: String::new(),
            target_node: String::new(),
            command: Some(c),
        }
        .encode_to_vec(),
    )
}

async fn recv<S>(c: &mut Conn<S>) -> Frame
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    tokio::time::timeout(Duration::from_secs(10), c.recv())
        .await
        .expect("timeout")
        .unwrap()
        .expect("closed")
}

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

/// `serve_node` for `node` over a socket pair, handshaken with `capabilities`.
async fn node(
    state: &Arc<ServerState>,
    node: &'static str,
    capabilities: &[&str],
) -> Conn<TcpStream> {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let st = Arc::clone(state);
    tokio::spawn(async move {
        let (s, _) = l.accept().await.unwrap();
        let (conn, _) = ws::accept_with_token(s).await.unwrap();
        let _ = server::serve_node(conn, node, &st.hub, &st.support).await;
    });
    let mut c = dial(addr, None).await;
    let hello = v1::Hello {
        role: v1::Role::Node as i32,
        protocol_generations: vec![1],
        capabilities: capabilities.iter().map(|c| (*c).to_string()).collect(),
        build: "t".into(),
        locale: None,
    };
    c.send(&f(STREAM_CONTROL, kinds::HELLO, hello.encode_to_vec()))
        .await
        .unwrap();
    assert_eq!(recv(&mut c).await.kind, kinds::WELCOME);
    c.send(&f(
        STREAM_CONTROL,
        kinds::RESUME,
        v1::Resume::default().encode_to_vec(),
    ))
    .await
    .unwrap();
    assert_eq!(recv(&mut c).await.kind, kinds::RESUME_OK);
    c
}

async fn client(addr: std::net::SocketAddr) -> Conn<TcpStream> {
    let mut c = dial(addr, Some("good-token")).await;
    c.send(&f(
        STREAM_EVENTS,
        kinds::SUBSCRIBE,
        v1::Subscribe::default().encode_to_vec(),
    ))
    .await
    .unwrap();
    assert_eq!(recv(&mut c).await.kind, kinds::SNAPSHOT);
    c
}

async fn answer(c: &mut Conn<TcpStream>) -> v1::CommandResponse {
    loop {
        let fr = recv(c).await;
        if (fr.stream_id, fr.kind) == (STREAM_COMMANDS, 1) {
            return v1::CommandResponse::decode(fr.payload.as_slice()).unwrap();
        }
    }
}

fn ok(r: &v1::CommandResponse) -> Option<&command_ok::Result> {
    match &r.result {
        Some(command_response::Result::Ok(v1::CommandOk { result })) => result.as_ref(),
        _ => None,
    }
}

fn err(r: &v1::CommandResponse) -> &str {
    match &r.result {
        Some(command_response::Result::Err(e)) => &e.code,
        _ => "",
    }
}

/// Test device keys, one per number, kept for the whole run.
fn pair(n: u8) -> &'static ring::signature::EcdsaKeyPair {
    use ring::signature::{EcdsaKeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
    static KEYS: std::sync::OnceLock<Vec<EcdsaKeyPair>> = std::sync::OnceLock::new();
    let keys = KEYS.get_or_init(|| {
        let rng = ring::rand::SystemRandom::new();
        (0..12)
            .map(|_| {
                let p =
                    EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
                EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, p.as_ref(), &rng)
                    .unwrap()
            })
            .collect()
    });
    &keys[usize::from(n)]
}

fn key(n: u8) -> Vec<u8> {
    use ring::signature::KeyPair;
    pair(n).public_key().as_ref().to_vec()
}

fn sign(n: u8, m: &[u8]) -> Vec<u8> {
    pair(n)
        .sign(&ring::rand::SystemRandom::new(), m)
        .unwrap()
        .as_ref()
        .to_vec()
}

fn registration(id: &str, n: u8) -> v1::DeviceRegister {
    use agora_hub::devices::register_message;
    let (name, at) = (format!("Browser {n}"), server::now_ms());
    v1::DeviceRegister {
        device_id: id.into(),
        name: name.clone(),
        public_key: key(n),
        signed_at_ms: at,
        signature: sign(n, &register_message(id, &name, &key(n), at)),
    }
}

fn register(id: &str, n: u8) -> cr::Command {
    cr::Command::DeviceRegister(registration(id, n))
}

/// `device` (key `n`) endorsed by `by`, whose key is 1.
fn endorse(device: &str, n: u8, by: &str, nodes: &[&str]) -> cr::Command {
    let now = server::now_ms();
    let mut e = v1::DeviceEndorsement {
        device_id: device.into(),
        public_key: key(n),
        name: "New".into(),
        nodes: nodes.iter().map(|n| (*n).to_string()).collect(),
        issued_at_ms: now,
        expires_at_ms: now + 86_400_000,
        endorser_device_id: by.into(),
        signature: Vec::new(),
    };
    e.signature = sign(1, &agora_hub::devices::endorse_message(&e));
    cr::Command::DeviceEndorse(v1::DeviceEndorse {
        endorsement: Some(e),
    })
}

/// The next command the hub routes to a node.
async fn routed(c: &mut Conn<TcpStream>) -> v1::CommandRequest {
    loop {
        let fr = recv(c).await;
        if (fr.stream_id, fr.kind) == (STREAM_COMMANDS, 0) {
            return v1::CommandRequest::decode(fr.payload.as_slice()).unwrap();
        }
    }
}

async fn nothing_routed(c: &mut Conn<TcpStream>) -> bool {
    loop {
        match tokio::time::timeout(Duration::from_millis(300), c.recv()).await {
            Err(_) => return true,
            Ok(Ok(Some(fr))) if (fr.stream_id, fr.kind) == (STREAM_COMMANDS, 0) => return false,
            Ok(Ok(Some(_))) => {}
            Ok(_) => return true,
        }
    }
}

#[tokio::test]
async fn a_client_registers_lists_and_asks_and_the_hub_carries_endorsements_only_to_nodes_that_verify(
) {
    let st = state(None);
    let addr = start(Arc::clone(&st)).await;
    let mut verifying = node(&st, "laptop", &["device.trust"]).await;
    let mut old = node(&st, "old", &[]).await;
    let mut c = client(addr).await;

    c.send(&command("r1", register("web-old", 1)))
        .await
        .unwrap();
    assert!(matches!(
        ok(&answer(&mut c).await),
        Some(command_ok::Result::Empty(_))
    ));
    c.send(&command("r2", register("web-new", 2)))
        .await
        .unwrap();
    answer(&mut c).await;
    // Another key under a registered id: refused.
    c.send(&command("r3", register("web-new", 9)))
        .await
        .unwrap();
    assert_eq!(err(&answer(&mut c).await), "device.key_mismatch");
    c.send(&command(
        "r4",
        cr::Command::DeviceAskApproval(v1::DeviceAskApproval {
            device_id: "web-new".into(),
            signed_at_ms: server::now_ms(),
            signature: sign(
                2,
                &agora_hub::devices::ask_message("web-new", server::now_ms()),
            ),
        }),
    ))
    .await
    .unwrap();
    answer(&mut c).await;
    c.send(&command("r5", cr::Command::DeviceList(v1::DeviceList {})))
        .await
        .unwrap();
    let Some(command_ok::Result::DeviceList(list)) = ok(&answer(&mut c).await).cloned() else {
        panic!("no list")
    };
    let ids: Vec<_> = list.devices.iter().map(|d| d.device_id.as_str()).collect();
    assert_eq!(ids, ["web-old", "web-new"]);
    assert!(list.devices[1].approval_asked_ms > 0);

    // A device command never names a node.
    let mut aimed = v1::CommandRequest {
        request_id: "r6".into(),
        idempotency_key: String::new(),
        target_node: "laptop".into(),
        command: Some(cr::Command::DeviceList(v1::DeviceList {})),
    };
    c.send(&f(STREAM_COMMANDS, 0, aimed.encode_to_vec()))
        .await
        .unwrap();
    assert_eq!(err(&answer(&mut c).await), "protocol.invalid_message");
    aimed.command = Some(endorse("web-new", 2, "web-old", &[]));
    aimed.request_id = "r7".into();
    c.send(&f(STREAM_COMMANDS, 0, aimed.encode_to_vec()))
        .await
        .unwrap();
    assert_eq!(err(&answer(&mut c).await), "protocol.invalid_message");
    assert!(nothing_routed(&mut verifying).await);

    // Carried, unchanged, to the node that verifies; never to one that cannot.
    c.send(&command("r8", endorse("web-new", 2, "web-old", &[])))
        .await
        .unwrap();
    assert!(ok(&answer(&mut c).await).is_some());
    let got = routed(&mut verifying).await;
    let Some(cr::Command::DeviceEndorse(d)) = got.command else {
        panic!("{got:?}")
    };
    let e = d.endorsement.unwrap();
    assert_eq!(e.device_id, "web-new");
    assert!(agora_hub::devices::verifies(
        &key(1),
        &agora_hub::devices::endorse_message(&e),
        &e.signature
    ));
    assert!(nothing_routed(&mut old).await);

    // One that names other machines does not go to this one.
    c.send(&command("r9", endorse("web-new", 2, "web-old", &["vps"])))
        .await
        .unwrap();
    answer(&mut c).await;
    assert!(nothing_routed(&mut verifying).await);

    // A revocation goes everywhere that verifies.
    c.send(&command(
        "r10",
        cr::Command::DeviceRevoke(v1::DeviceRevoke {
            revocation: Some({
                let mut r = v1::DeviceRevocation {
                    device_id: "web-new".into(),
                    issued_at_ms: server::now_ms(),
                    revoker_device_id: "web-old".into(),
                    signature: Vec::new(),
                };
                r.signature = sign(1, &agora_hub::devices::revoke_message(&r));
                r
            }),
        }),
    ))
    .await
    .unwrap();
    answer(&mut c).await;
    assert!(matches!(
        routed(&mut verifying).await.command,
        Some(cr::Command::DeviceRevoke(_))
    ));

    // A node that connects later is given what it missed: the endorsement for every machine, and the revocation.
    let mut later = node(&st, "later", &["device.trust"]).await;
    let first = routed(&mut later).await;
    let second = routed(&mut later).await;
    assert!(matches!(first.command, Some(cr::Command::DeviceEndorse(_))));
    assert!(matches!(second.command, Some(cr::Command::DeviceRevoke(_))));
    assert!(
        nothing_routed(&mut later).await,
        "the one for `vps` is not its"
    );

    // What a node says it trusts reaches the snapshot.
    for (seq, body) in [
        Body::NodeOnline(v1::NodeOnline {
            node_id: "laptop".into(),
            hostname: "h".into(),
            os: "linux".into(),
            build: "b".into(),
        }),
        Body::NodeTrustedDevices(v1::NodeTrustedDevices {
            device_ids: vec!["web-old".into()],
        }),
    ]
    .into_iter()
    .enumerate()
    {
        verifying
            .send(&f(
                STREAM_EVENTS,
                kinds::EVENT,
                v1::Event {
                    producer_id: "laptop".into(),
                    seq: seq as u64 + 1,
                    at_ms: 1,
                    body: Some(body),
                }
                .encode_to_vec(),
            ))
            .await
            .unwrap();
    }
    for _ in 0..100 {
        if st.hub.view().0.trusted_devices.contains_key("laptop") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let (view, pos) = st.hub.view();
    let snapshot = server::snapshot_of(&view, pos);
    let laptop = snapshot
        .nodes
        .iter()
        .find(|n| n.node_id == "laptop")
        .unwrap();
    assert_eq!(laptop.trusted_device_ids, ["web-old"]);
}

// ---- over TLS, with a node's certificate ----

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
        Some((cert, key)) => b
            .with_client_auth_cert(
                CertificateDer::pem_slice_iter(cert.as_bytes())
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap(),
                PrivateKeyDer::from_pem_slice(key.as_bytes()).unwrap(),
            )
            .unwrap(),
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

#[tokio::test]
async fn enrollment_offers_the_registered_devices_and_a_node_asks_for_them_with_its_certificate() {
    let ca = Ca::generate("hub ca").unwrap();
    let (server_cert, server_key) = ca.issue_server(&["localhost".to_string()]).unwrap();
    let acceptor = tls::acceptor_with(&server_cert, &server_key, ca.cert_pem(), true).unwrap();
    let trust = Ca::from_pem(ca.cert_pem(), &ca.key_pem()).unwrap();
    let st = state(Some(ca));
    agora_hub::devices::Devices::new(st.hub.store())
        .register("owner", &registration("web-1", 1), server::now_ms())
        .unwrap();
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(server::run(l, Arc::clone(&st), Some(acceptor)));

    let code = Enrollment::new(st.hub.store())
        .create_code(server::now_ms())
        .unwrap();
    let (key_pem, csr) = new_device_request().unwrap();
    let mut c = dial_tls(addr, client_tls(&trust, None)).await;
    c.send(&command(
        "e",
        cr::Command::Enroll(v1::EnrollRequest {
            code,
            node_id: "box".into(),
            csr_pem: csr,
        }),
    ))
    .await
    .unwrap();
    let r = v1::CommandResponse::decode(recv(&mut c).await.payload.as_slice()).unwrap();
    let Some(command_ok::Result::Enroll(e)) = ok(&r).cloned() else {
        panic!("{r:?}")
    };
    assert_eq!(e.devices.len(), 1);
    assert_eq!(
        (
            e.devices[0].device_id.as_str(),
            e.devices[0].public_key.clone()
        ),
        ("web-1", key(1))
    );

    // Without a certificate, asking is refused like anything but enrollment.
    let mut anon = dial_tls(addr, client_tls(&trust, None)).await;
    anon.send(&command("q", cr::Command::DeviceList(v1::DeviceList {})))
        .await
        .unwrap();
    let r = v1::CommandResponse::decode(recv(&mut anon).await.payload.as_slice()).unwrap();
    assert_eq!(err(&r), "auth.unauthenticated");

    // With the node's certificate: one answer on a connection of its own.
    let creds = Some((e.cert_pem.clone(), key_pem.clone()));
    let mut q = dial_tls(addr, client_tls(&trust, creds.clone())).await;
    q.send(&command("q", cr::Command::DeviceList(v1::DeviceList {})))
        .await
        .unwrap();
    let r = v1::CommandResponse::decode(recv(&mut q).await.payload.as_slice()).unwrap();
    let Some(command_ok::Result::DeviceList(list)) = ok(&r) else {
        panic!("{r:?}")
    };
    assert_eq!(list.devices.len(), 1);
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(5), q.recv()).await,
        Ok(Ok(None) | Err(_))
    ));
    // Nothing else is answered that way.
    let mut q = dial_tls(addr, client_tls(&trust, creds)).await;
    q.send(&command(
        "q",
        cr::Command::AgentList(v1::AgentList::default()),
    ))
    .await
    .unwrap();
    let r = v1::CommandResponse::decode(recv(&mut q).await.payload.as_slice()).unwrap();
    assert_eq!(err(&r), "protocol.invalid_message");
    assert!(
        !st.hub.view().0.nodes.contains_key("box"),
        "a question is not a node coming online"
    );
}

#[tokio::test]
async fn an_endorsement_reaches_only_machines_its_person_works_on() {
    use agora_hub::users::{Role, Users};
    let st = state(None);
    let users = Users::new(st.hub.store());
    let eda = users.add("Eda", Role::Member, 1).unwrap();
    users.grant(&eda.id, "vps", "web").unwrap();
    let devices = agora_hub::devices::Devices::new(st.hub.store());
    devices
        .register(&eda.id, &registration("eda-old", 3), server::now_ms())
        .unwrap();
    devices
        .register(&eda.id, &registration("eda-new", 4), server::now_ms())
        .unwrap();
    let now = server::now_ms();
    let mut e = v1::DeviceEndorsement {
        device_id: "eda-new".into(),
        public_key: key(4),
        name: "New".into(),
        nodes: vec![],
        issued_at_ms: now,
        expires_at_ms: now + 86_400_000,
        endorser_device_id: "eda-old".into(),
        signature: Vec::new(),
    };
    e.signature = sign(3, &agora_hub::devices::endorse_message(&e));
    devices.record_endorsement(&eda.id, &e, now).unwrap();
    // A machine she has no project on is not given it; one she works on is.
    let mut laptop = node(&st, "laptop", &["device.trust"]).await;
    assert!(nothing_routed(&mut laptop).await);
    let mut vps = node(&st, "vps", &["device.trust"]).await;
    assert!(matches!(
        routed(&mut vps).await.command,
        Some(cr::Command::DeviceEndorse(_))
    ));
}
