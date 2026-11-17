//! Pairing, the machine's side, against a real hub over real TLS (ADR-0065, docs/specs/pairing.md section 5): the node's own client
//! (`agora_node::pairing_client::pair`) talks to an in-process hub listener while a fake owner window compares the number and answers.
//! The refusal paths each leave nothing behind: this client stores nothing, and the tests check what it returned (or did not).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use agora_hub::ca::{csr_der, new_device_request, Ca};
use agora_hub::enroll::{Enrollment, Lockout};
use agora_hub::hub::Hub;
use agora_hub::server::{self, ClientAuth, ServerState};
use agora_hub::store::SqliteStore;
use agora_hub::tls;
use agora_hub::users::{Access, Role};
use agora_node::pairing_client::{pair, PairError, Timeouts, Verified, Wanted};
use agora_protocol::frame::{kinds, STREAM_COMMANDS, STREAM_EVENTS};
use agora_protocol::handshake::Support;
use agora_protocol::v1::{self, command_ok, command_request, command_response};
use agora_protocol::{ErrorCode, Frame};
use prost::Message;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::client::TlsStream;

type Ws = agora_protocol::ws::Conn<TlsStream<TcpStream>>;

struct Owner;

impl ClientAuth for Owner {
    fn check(&self, t: &str) -> bool {
        t == "owner-token"
    }

    fn access(&self, t: &str) -> Option<Access> {
        (t == "owner-token").then(|| Access {
            role: Role::Owner,
            user: "owner".into(),
            projects: None,
            session: String::new(),
        })
    }
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

struct Rig {
    addr: std::net::SocketAddr,
    state: Arc<ServerState>,
    /// The CA the hub hands to a machine that is approved.
    ca_pem: String,
    /// The CA that signed the leaf the hub presents (another one in the "wrong CA" rig).
    leaf_ca_pem: String,
}

/// A hub whose leaf names `names`; with `leaf_from_another_ca` the leaf is signed by a CA that is not the hub's own.
async fn rig_with(names: &[&str], leaf_from_another_ca: bool) -> Rig {
    let ca = Ca::generate("hub ca").unwrap();
    let other = Ca::generate("another ca").unwrap();
    let names: Vec<String> = names.iter().map(|n| (*n).to_string()).collect();
    let signer = if leaf_from_another_ca { &other } else { &ca };
    let (cert, key) = signer.issue_server(&names).unwrap();
    let acceptor = tls::acceptor_with(&cert, &key, ca.cert_pem(), true).unwrap();
    let hub = Arc::new(Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap());
    hub.pairing().enable(tls::leaf_hash(&cert).unwrap());
    let ca_pem = ca.cert_pem().to_string();
    let leaf_ca_pem = signer.cert_pem().to_string();
    let state = Arc::new(ServerState {
        hub,
        support: Support {
            generations: vec![1],
            capabilities: vec!["pairing.v1".into()],
            required: vec![],
        },
        ca: Some(ca),
        lockout: Arc::new(Lockout::new()),
        auth: Box::new(Owner),
        assets: None,
        sessions: None,
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(server::run(listener, Arc::clone(&state), Some(acceptor)));
    Rig {
        addr,
        state,
        ca_pem,
        leaf_ca_pem,
    }
}

async fn rig() -> Rig {
    rig_with(&["localhost"], false).await
}

fn answer(f: &Frame) -> Result<command_ok::Result, ErrorCode> {
    let r = v1::CommandResponse::decode(f.payload.as_slice()).unwrap();
    match r.result.unwrap() {
        command_response::Result::Ok(ok) => Ok(ok.result.unwrap()),
        command_response::Result::Err(e) => Err(ErrorCode::parse(&e.code)),
    }
}

/// The owner's window: signed in over TLS, subscribed, reading the cards the hub pushes.
struct Window {
    ws: Ws,
    cards: Vec<v1::PairRequested>,
}

impl Window {
    async fn open(rig: &Rig) -> Self {
        let mut roots = rustls::RootCertStore::empty();
        for c in CertificateDer::pem_slice_iter(rig.leaf_ca_pem.as_bytes()) {
            roots.add(c.unwrap()).unwrap();
        }
        let cfg = rustls::ClientConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let tcp = TcpStream::connect(rig.addr).await.unwrap();
        let tls = tokio_rustls::TlsConnector::from(Arc::new(cfg))
            .connect(ServerName::try_from("localhost").unwrap(), tcp)
            .await
            .unwrap();
        let mut ws = agora_protocol::ws::connect_with_subprotocol_token(
            tls,
            &format!("wss://localhost:{}/", rig.addr.port()),
            "owner-token",
        )
        .await
        .unwrap();
        ws.send(&Frame {
            stream_id: STREAM_EVENTS,
            kind: kinds::SUBSCRIBE,
            seq: None,
            payload: v1::Subscribe::default().encode_to_vec(),
        })
        .await
        .unwrap();
        let mut w = Self {
            ws,
            cards: Vec::new(),
        };
        loop {
            let f = w.next_frame().await;
            if (f.stream_id, f.kind) == (STREAM_EVENTS, kinds::SNAPSHOT) {
                break;
            }
        }
        w
    }

    async fn next_frame(&mut self) -> Frame {
        let f = tokio::time::timeout(Duration::from_secs(20), self.ws.recv())
            .await
            .expect("a frame in time")
            .unwrap()
            .expect("the hub kept the window open");
        if f.stream_id == STREAM_EVENTS && f.kind == kinds::PAIR_REQUESTED {
            self.cards
                .push(v1::PairRequested::decode(f.payload.as_slice()).unwrap());
        }
        f
    }

    async fn command(
        &mut self,
        c: command_request::Command,
    ) -> Result<command_ok::Result, ErrorCode> {
        let req = v1::CommandRequest {
            request_id: "w".into(),
            idempotency_key: String::new(),
            target_node: String::new(),
            command: Some(c),
        };
        self.ws
            .send(&Frame {
                stream_id: STREAM_COMMANDS,
                kind: 0,
                seq: None,
                payload: req.encode_to_vec(),
            })
            .await
            .unwrap();
        loop {
            let f = self.next_frame().await;
            if (f.stream_id, f.kind) == (STREAM_COMMANDS, 1) {
                return answer(&f);
            }
        }
    }

    async fn open_switch(&mut self) {
        self.command(command_request::Command::PairOpen(v1::PairOpen {}))
            .await
            .unwrap();
    }

    async fn card(&mut self) -> v1::PairRequested {
        while self.cards.is_empty() {
            self.next_frame().await;
        }
        self.cards.remove(0)
    }

    async fn approve(&mut self, id: &[u8]) {
        self.command(command_request::Command::PairApprove(v1::PairApprove {
            pair_id: id.to_vec(),
            replace: false,
        }))
        .await
        .unwrap();
    }

    async fn deny(&mut self, id: &[u8]) {
        self.command(command_request::Command::PairDeny(v1::PairDeny {
            pair_id: id.to_vec(),
        }))
        .await
        .unwrap();
    }
}

/// A machine running `pair` on a thread of its own, as `agora join` does.
struct Machine {
    number: mpsc::Receiver<String>,
    done: std::thread::JoinHandle<(Result<Verified, PairError>, String, Vec<u8>)>,
}

fn machine(address: &str, name: &str, timeouts: Timeouts) -> Machine {
    let (address, name) = (address.to_string(), name.to_string());
    let (tx, number) = mpsc::channel();
    let done = std::thread::spawn(move || {
        let (key_pem, csr_pem) = new_device_request().unwrap();
        let csr = csr_der(&csr_pem).unwrap();
        let wanted = Wanted {
            address: &address,
            node_name: &name,
            os: "linux",
            version: "0.0",
            csr_pem: &csr_pem,
            csr_der: &csr,
        };
        let r = pair(&wanted, &timeouts, &mut |n| {
            tx.send(n.digits().to_string()).unwrap();
        });
        (r, key_pem, csr)
    });
    Machine { number, done }
}

impl Machine {
    async fn shown(&self) -> String {
        let rx = &self.number;
        for _ in 0..400 {
            if let Ok(n) = rx.try_recv() {
                return n;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("the machine never showed a number");
    }

    async fn finish(self) -> (Result<Verified, PairError>, String, Vec<u8>) {
        tokio::task::spawn_blocking(move || self.done.join().unwrap())
            .await
            .unwrap()
    }
}

fn fast() -> Timeouts {
    Timeouts {
        connect: Duration::from_secs(5),
        answer: Duration::from_secs(10),
        decision: Duration::from_secs(20),
    }
}

fn dial(rig: &Rig) -> String {
    format!("localhost:{}", rig.addr.port())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_machine_pairs_with_the_number_the_owner_sees_and_then_connects_with_what_it_was_given() {
    let rig = rig().await;
    let mut owner = Window::open(&rig).await;
    owner.open_switch().await;

    let m = machine(&dial(&rig), "laptop", fast());
    let shown = m.shown().await;
    let card = owner.card().await;
    // The number on the machine is the number on the owner's card: both sides computed it from what each of them saw.
    assert_eq!(card.number, shown);
    assert_eq!((card.name.as_str(), card.os.as_str()), ("laptop", "linux"));
    owner.approve(&card.pair_id).await;

    let (result, key_pem, csr) = m.finish().await;
    let v = result.expect("paired");
    assert_eq!(v.ca_pem, rig.ca_pem);
    assert_eq!(
        v.ca_fingerprint.len(),
        64,
        "a SHA-256 in hex: {}",
        v.ca_fingerprint
    );
    assert_eq!(v.host, "localhost");
    // The certificate is for the key this machine made, and the hub recorded it like an enrolled one.
    assert!(Enrollment::new(rig.state.hub.store())
        .issued_ms_by_node()
        .unwrap()
        .contains_key("laptop"));
    assert!(!csr.is_empty());

    // The paired machine connects to the hub with exactly this, and shows up online.
    let dir = std::env::temp_dir().join(format!("agora-pairnode-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let sink = Arc::new(agora_node::daemon::EventSink::open(&dir, "laptop").unwrap());
    sink.node_online("laptop");
    let _link = agora_node::uplink_client::spawn(
        Arc::clone(&sink),
        agora_node::uplink_client::HubLink {
            mesh_out: None,
            url: format!("wss://localhost:{}", rig.addr.port()),
            ca_pem: v.ca_pem.clone(),
            cert_pem: v.cert_pem.clone(),
            key_pem: key_pem.clone(),
            commands: None,
            grants: None,
            mesh: None,
            devices: None,
            state_dir: None,
        },
    )
    .unwrap();
    let end = std::time::Instant::now() + Duration::from_secs(20);
    while !rig
        .state
        .hub
        .view()
        .0
        .nodes
        .get("laptop")
        .is_some_and(|n| n.online)
    {
        assert!(
            std::time::Instant::now() < end,
            "the paired node never connected"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // What the hub keeps: its audit trail and its events say nothing of the private key, the number or the nonces.
    let rows = agora_hub::audit::Audit::new(rig.state.hub.store())
        .list(0, None, 100)
        .unwrap();
    let kept = format!("{rows:?}{:?}", rig.state.hub.view().0);
    let body: String = key_pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect();
    assert!(body.len() > 100);
    assert!(
        !kept.contains(&body[60..100]),
        "the private key is in what the hub keeps"
    );
    assert!(!kept.contains(&shown));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_denied_machine_gets_nothing() {
    let rig = rig().await;
    let mut owner = Window::open(&rig).await;
    owner.open_switch().await;
    let m = machine(&dial(&rig), "denied", fast());
    m.shown().await;
    let card = owner.card().await;
    owner.deny(&card.pair_id).await;
    let (r, _, _) = m.finish().await;
    assert_eq!(r.unwrap_err(), PairError::Denied);
    assert!(!Enrollment::new(rig.state.hub.store())
        .node_in_use("denied")
        .unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_closed_hub_and_a_closing_switch_end_the_pairing_with_the_hubs_code() {
    let rig = rig().await;
    // Not open yet.
    let m = machine(&dial(&rig), "early", fast());
    let (r, _, _) = m.finish().await;
    match r.unwrap_err() {
        PairError::Hub { code, .. } => assert_eq!(code, "pair.closed"),
        other => panic!("{other:?}"),
    }
    // Closed while waiting for the owner.
    let mut owner = Window::open(&rig).await;
    owner.open_switch().await;
    let m = machine(&dial(&rig), "closing", fast());
    m.shown().await;
    owner.card().await;
    owner
        .command(command_request::Command::PairClose(v1::PairClose {}))
        .await
        .unwrap();
    let (r, _, _) = m.finish().await;
    match r.unwrap_err() {
        PairError::Hub { code, .. } => assert_eq!(code, "pair.closed"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_request_nobody_answers_expires_on_the_hub_and_on_the_machines_own_clock() {
    let rig = rig().await;
    let mut owner = Window::open(&rig).await;
    owner.open_switch().await;

    // The hub's session time is up (its clock is injected: far past the 5 minutes).
    let m = machine(&dial(&rig), "slow", fast());
    m.shown().await;
    owner.card().await;
    rig.state.hub.pairing().with(|p| p.tick(u64::MAX / 4));
    rig.state.hub.pairing().wake();
    let (r, _, _) = m.finish().await;
    match r.unwrap_err() {
        PairError::Hub { code, .. } => {
            assert!(
                code == "pair.expired" || code == "pair.closed",
                "expired or closed with it: {code}"
            )
        }
        other => panic!("{other:?}"),
    }

    // The machine gives up on its own when the owner never answers within its own (short) wait.
    owner.open_switch().await;
    let m = machine(
        &dial(&rig),
        "patient",
        Timeouts {
            decision: Duration::from_millis(700),
            ..fast()
        },
    );
    m.shown().await;
    let (r, _, _) = m.finish().await;
    assert_eq!(
        r.unwrap_err(),
        PairError::Timeout {
            waiting_for: "decision"
        }
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_hub_nobody_listens_at_is_unreachable() {
    let free = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let m = machine(&format!("127.0.0.1:{free}"), "nobody", fast());
    let (r, _, _) = m.finish().await;
    assert!(matches!(r.unwrap_err(), PairError::Unreachable(_)));
    let (r, _, _) = machine("not an address", "x", fast()).finish().await;
    assert_eq!(r.unwrap_err(), PairError::BadAddress);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_address_that_is_in_no_name_of_the_hubs_leaf_fails_closed_after_the_owner_approved() {
    // The leaf names `localhost` only; the machine dials the IP address.
    let rig = rig().await;
    let mut owner = Window::open(&rig).await;
    owner.open_switch().await;
    let m = machine(&format!("127.0.0.1:{}", rig.addr.port()), "byip", fast());
    m.shown().await;
    let card = owner.card().await;
    owner.approve(&card.pair_id).await;
    let (r, _, _) = m.finish().await;
    assert_eq!(
        r.unwrap_err(),
        PairError::NameUnverified {
            name: "127.0.0.1".into()
        }
    );

    // A leaf for another name than the one dialled.
    let rig = rig_with(&["elsewhere.example"], false).await;
    let mut owner = Window::open_via(&rig).await;
    owner.open_switch().await;
    let m = machine(&dial(&rig), "wrongleaf", fast());
    m.shown().await;
    let card = owner.card().await;
    owner.approve(&card.pair_id).await;
    let (r, _, _) = m.finish().await;
    assert!(matches!(r.unwrap_err(), PairError::NameUnverified { .. }));
}

impl Window {
    /// A window for a hub whose leaf does not name `localhost`: it dials the name the leaf has.
    async fn open_via(rig: &Rig) -> Self {
        // The leaf of this rig is for `elsewhere.example`; the window trusts its CA and connects with that name.
        let mut roots = rustls::RootCertStore::empty();
        for c in CertificateDer::pem_slice_iter(rig.leaf_ca_pem.as_bytes()) {
            roots.add(c.unwrap()).unwrap();
        }
        let cfg = rustls::ClientConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let tcp = TcpStream::connect(rig.addr).await.unwrap();
        let tls = tokio_rustls::TlsConnector::from(Arc::new(cfg))
            .connect(ServerName::try_from("elsewhere.example").unwrap(), tcp)
            .await
            .unwrap();
        let mut ws = agora_protocol::ws::connect_with_subprotocol_token(
            tls,
            "wss://elsewhere.example/",
            "owner-token",
        )
        .await
        .unwrap();
        ws.send(&Frame {
            stream_id: STREAM_EVENTS,
            kind: kinds::SUBSCRIBE,
            seq: None,
            payload: v1::Subscribe::default().encode_to_vec(),
        })
        .await
        .unwrap();
        let mut w = Self {
            ws,
            cards: Vec::new(),
        };
        loop {
            let f = w.next_frame().await;
            if (f.stream_id, f.kind) == (STREAM_EVENTS, kinds::SNAPSHOT) {
                break;
            }
        }
        w
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_leaf_from_another_ca_than_the_one_the_hub_delivers_is_refused() {
    // The hub presents a leaf signed by a CA that is not the one it hands over at the approval: what a party in the middle with a
    // certificate of its own would look like at the end.
    let rig = rig_with(&["localhost"], true).await;
    let mut owner = Window::open(&rig).await;
    owner.open_switch().await;
    let m = machine(&dial(&rig), "wrongca", fast());
    m.shown().await;
    let card = owner.card().await;
    owner.approve(&card.pair_id).await;
    let (r, _, _) = m.finish().await;
    assert!(matches!(r.unwrap_err(), PairError::NameUnverified { .. }));
}

/// Accepts one connection with `config` and keeps it for a while (the client's handshake is what is tested).
async fn serve_tls(config: rustls::ServerConfig) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                if let Ok(tls) = acceptor.accept(tcp).await {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    drop(tls);
                }
            });
        }
    });
    addr
}

fn server_parts(ca: &Ca, name: &str) -> (CertificateDer<'static>, PrivateKeyDer<'static>) {
    let (cert, key) = ca.issue_server(&[name.to_string()]).unwrap();
    (
        CertificateDer::from_pem_slice(cert.as_bytes()).unwrap(),
        PrivateKeyDer::from_pem_slice(key.as_bytes()).unwrap(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_hub_that_only_speaks_tls_1_2_is_refused() {
    let ca = Ca::generate("old hub").unwrap();
    let (cert, key) = server_parts(&ca, "localhost");
    let config = rustls::ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS12])
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .unwrap();
    let addr = serve_tls(config).await;
    let (r, _, _) = machine(&format!("localhost:{}", addr.port()), "old", fast())
        .finish()
        .await;
    assert_eq!(r.unwrap_err(), PairError::Tls13Required);
}

#[derive(Debug)]
struct Mismatched(Arc<rustls::sign::CertifiedKey>);

impl rustls::server::ResolvesServerCert for Mismatched {
    fn resolve(
        &self,
        _: rustls::server::ClientHello<'_>,
    ) -> Option<Arc<rustls::sign::CertifiedKey>> {
        Some(Arc::clone(&self.0))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_peer_that_replays_a_leaf_without_holding_its_key_fails_the_handshake() {
    // The certificate of one leaf, signed with the key of another: what replaying a leaf seen on the wire amounts to.
    let ca = Ca::generate("hub").unwrap();
    let (cert_a, _key_a) = server_parts(&ca, "localhost");
    let (_cert_b, key_b) = server_parts(&ca, "localhost");
    let signing = rustls::crypto::ring::sign::any_supported_type(&key_b).unwrap();
    let certified = rustls::sign::CertifiedKey::new(vec![cert_a], signing);
    let config = rustls::ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(Mismatched(Arc::new(certified))));
    let addr = serve_tls(config).await;
    let (r, _, _) = machine(&format!("localhost:{}", addr.port()), "replay", fast())
        .finish()
        .await;
    match r.unwrap_err() {
        // The refusal is the signature check itself, not some other failure of the handshake.
        PairError::TlsFailed(why) => assert!(why.contains("BadSignature"), "{why}"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_party_in_the_middle_makes_the_machine_show_a_number_the_owner_does_not_see() {
    let rig = rig().await;
    let mut owner = Window::open(&rig).await;
    owner.open_switch().await;

    // The relay terminates the machine's TLS with a certificate of its own and talks to the real hub on a second connection,
    // forwarding every frame unchanged.
    let other = Ca::generate("relay ca").unwrap();
    let (cert, key) = server_parts(&other, "localhost");
    let config = rustls::ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay_addr = listener.local_addr().unwrap();
    let (hub_addr, hub_ca) = (rig.addr, rig.ca_pem.clone());
    tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let tls = tokio_rustls::TlsAcceptor::from(Arc::new(config))
            .accept(tcp)
            .await
            .unwrap();
        let mut from_machine = agora_protocol::ws::accept(tls).await.unwrap();
        let mut roots = rustls::RootCertStore::empty();
        for c in CertificateDer::pem_slice_iter(hub_ca.as_bytes()) {
            roots.add(c.unwrap()).unwrap();
        }
        let cfg = rustls::ClientConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let tcp = TcpStream::connect(hub_addr).await.unwrap();
        let tls = tokio_rustls::TlsConnector::from(Arc::new(cfg))
            .connect(ServerName::try_from("localhost").unwrap(), tcp)
            .await
            .unwrap();
        let mut to_hub = agora_protocol::ws::connect(tls, "wss://localhost/")
            .await
            .unwrap();
        loop {
            tokio::select! {
                f = from_machine.recv() => match f {
                    Ok(Some(f)) => { if to_hub.send(&f).await.is_err() { break; } }
                    _ => break,
                },
                f = to_hub.recv() => match f {
                    Ok(Some(f)) => { if from_machine.send(&f).await.is_err() { break; } }
                    _ => break,
                },
            }
        }
    });

    let m = machine(
        &format!("localhost:{}", relay_addr.port()),
        "victim",
        fast(),
    );
    let shown = m.shown().await;
    let card = owner.card().await;
    assert_ne!(
        shown, card.number,
        "the numbers of two different TLS sessions must differ"
    );
    owner.deny(&card.pair_id).await;
    let (r, _, _) = m.finish().await;
    assert_eq!(r.unwrap_err(), PairError::Denied);
}
