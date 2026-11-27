//! Pairing, hub side, over the hub's real TLS (ADR-0065, docs/specs/pairing.md): a fake machine and a fake owner window talk to a real
//! `agora-hub` listener. The machine here is the test: it computes the number itself with `ring`, on its own side of the connection,
//! so "the same number on both sides" is checked, not assumed. The node's own side (the verifier type, the stored certificate) is
//! another pull request.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use agora_hub::ca::{csr_der, new_device_request, Ca};
use agora_hub::enroll::{Enrollment, Lockout};
use agora_hub::hub::Hub;
use agora_hub::server::{self, ClientAuth, ServerState};
use agora_hub::store::SqliteStore;
use agora_hub::tls;
use agora_hub::users::{Access, Role};
use agora_protocol::frame::{kinds, STREAM_COMMANDS, STREAM_EVENTS};
use agora_protocol::handshake::Support;
use agora_protocol::v1::{self, command_ok, command_request, command_response};
use agora_protocol::{ErrorCode, Frame};
use prost::Message;
use ring::digest;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

type Ws = agora_protocol::ws::Conn<TlsStream<TcpStream>>;

/// Three signed-in people: the owner, a member and a viewer.
struct People;

impl ClientAuth for People {
    fn check(&self, t: &str) -> bool {
        matches!(t, "owner-token" | "member-token" | "viewer-token")
    }

    fn access(&self, t: &str) -> Option<Access> {
        let role = match t {
            "owner-token" => Role::Owner,
            "member-token" => Role::Member,
            "viewer-token" => Role::Viewer,
            _ => return None,
        };
        Some(Access {
            role,
            user: t.to_string(),
            projects: (role != Role::Owner).then(std::collections::BTreeSet::new),
            session: String::new(),
        })
    }
}

struct Rig {
    addr: std::net::SocketAddr,
    state: Arc<ServerState>,
    ca_pem: String,
    leaf_hash: [u8; 32],
}

async fn rig() -> Rig {
    let ca = Ca::generate("hub ca").unwrap();
    let (cert, key) = ca.issue_server(&["localhost".to_string()]).unwrap();
    let acceptor = tls::acceptor_with(&cert, &key, ca.cert_pem(), true).unwrap();
    let hub = Arc::new(Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap());
    let leaf_hash = tls::leaf_hash(&cert).unwrap();
    hub.pairing().enable(leaf_hash);
    let ca_pem = ca.cert_pem().to_string();
    let state = Arc::new(ServerState {
        hub,
        support: Support {
            generations: vec![1],
            capabilities: vec!["pairing.v1".into()],
            required: vec![],
        },
        ca: Some(ca),
        lockout: Arc::new(Lockout::new()),
        auth: Box::new(People),
        assets: None,
        sessions: None,
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(server::run(listener, Arc::clone(&state), Some(acceptor)));
    Rig {
        addr,
        state,
        ca_pem,
        leaf_hash,
    }
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// What a pairing machine's verifier does (spec section 5): it cannot chain the leaf to anything yet, so it relaxes that one check,
/// still verifies the handshake signature, and records the leaf's hash.
#[derive(Debug)]
struct AnyLeaf {
    provider: Arc<rustls::crypto::CryptoProvider>,
    seen: Mutex<Option<[u8; 32]>>,
}

impl ServerCertVerifier for AnyLeaf {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if end_entity.is_empty() {
            return Err(rustls::Error::NoCertificatesPresented);
        }
        let h = digest::digest(&digest::SHA256, end_entity.as_ref());
        *self.seen.lock().unwrap() = h.as_ref().try_into().ok();
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        // Only a test of the hub's refusal of TLS 1.2 uses this; the node's own verifier never accepts it.
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// A machine that is pairing: its connection, and what it saw of the hub's TLS.
struct Machine {
    ws: Ws,
    exporter: [u8; 32],
    leaf_hash: [u8; 32],
    leaf_der: Vec<u8>,
    nonce_n: [u8; 16],
    key_pem: String,
    csr_pem: String,
    csr_hash: [u8; 32],
}

fn sha(parts: &[&[u8]]) -> [u8; 32] {
    let mut c = digest::Context::new(&digest::SHA256);
    for p in parts {
        c.update(p);
    }
    c.finish().as_ref().try_into().unwrap()
}

impl Machine {
    /// Dials the hub as a machine that does not know its CA yet (TLS 1.3 only, as pairing requires).
    async fn dial(rig: &Rig) -> Self {
        Self::dial_with(rig, &[&rustls::version::TLS13]).await
    }

    async fn dial_with(rig: &Rig, versions: &[&'static rustls::SupportedProtocolVersion]) -> Self {
        let verifier = Arc::new(AnyLeaf {
            provider: provider(),
            seen: Mutex::new(None),
        });
        let cfg = rustls::ClientConfig::builder_with_provider(provider())
            .with_protocol_versions(versions)
            .unwrap()
            .dangerous()
            .with_custom_certificate_verifier(verifier.clone())
            .with_no_client_auth();
        let tcp = TcpStream::connect(rig.addr).await.unwrap();
        let tls = tokio_rustls::TlsConnector::from(Arc::new(cfg))
            .connect(ServerName::try_from("localhost").unwrap(), tcp)
            .await
            .unwrap();
        let mut exporter = [0u8; 32];
        if let Ok(e) =
            tls.get_ref()
                .1
                .export_keying_material([0u8; 32], b"EXPORTER-agora-pair-v1", Some(b""))
        {
            exporter = e;
        }
        let leaf_der = tls.get_ref().1.peer_certificates().unwrap()[0]
            .as_ref()
            .to_vec();
        let leaf_hash = verifier.seen.lock().unwrap().unwrap();
        let ws = agora_protocol::ws::connect(tls, &format!("wss://localhost:{}/", rig.addr.port()))
            .await
            .unwrap();
        let (key_pem, csr_pem) = new_device_request().unwrap();
        let csr_hash = sha(&[&csr_der(&csr_pem).unwrap()]);
        let mut nonce_n = [0u8; 16];
        getrandom::getrandom(&mut nonce_n).unwrap();
        Self {
            ws,
            exporter,
            leaf_hash,
            leaf_der,
            nonce_n,
            key_pem,
            csr_pem,
            csr_hash,
        }
    }

    fn commitment(&self) -> Vec<u8> {
        sha(&[b"agora/pair/v1/commit", &self.nonce_n, &self.csr_hash]).to_vec()
    }

    /// The number as this side computes it, with `ring`'s HKDF written out here (not the hub's function).
    fn number(&self, nonce_h: &[u8]) -> String {
        struct Len8;
        impl ring::hkdf::KeyType for Len8 {
            fn len(&self) -> usize {
                8
            }
        }
        let mut ikm = Vec::new();
        for p in [
            &self.exporter[..],
            &self.leaf_hash,
            &self.csr_hash,
            &self.nonce_n,
            nonce_h,
        ] {
            ikm.extend_from_slice(p);
        }
        let prk = ring::hkdf::Salt::new(ring::hkdf::HKDF_SHA256, b"agora/pair/v1").extract(&ikm);
        let mut out = [0u8; 8];
        prk.expand(&[b"sas"], Len8).unwrap().fill(&mut out).unwrap();
        format!("{:06}", u64::from_be_bytes(out) % 1_000_000)
    }

    async fn send(&mut self, id: &str, c: command_request::Command) {
        let req = v1::CommandRequest {
            request_id: id.into(),
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
    }

    async fn recv(&mut self) -> Option<Result<command_ok::Result, ErrorCode>> {
        let f = tokio::time::timeout(Duration::from_secs(20), self.ws.recv())
            .await
            .expect("the hub answered in time")
            .ok()??;
        Some(answer(&f))
    }

    /// Step 1 and 2: the commitment, then the challenge (`pair_id`, `nonce_h`).
    async fn request(&mut self, name: &str) -> Result<(Vec<u8>, Vec<u8>), ErrorCode> {
        let c = self.commitment();
        self.send(
            "req",
            command_request::Command::PairRequest(v1::PairRequest {
                commitment: c,
                node_name: name.into(),
                os: "linux".into(),
                version: "0.0".into(),
            }),
        )
        .await;
        match self.recv().await {
            Some(Ok(command_ok::Result::PairChallenge(c))) => Ok((c.pair_id, c.nonce_h)),
            Some(Err(code)) => Err(code),
            other => panic!("unexpected {other:?}"),
        }
    }

    async fn reveal(&mut self, pair_id: &[u8]) {
        let (nonce_n, csr) = (self.nonce_n.to_vec(), self.csr_pem.clone());
        self.send(
            "reveal",
            command_request::Command::PairReveal(v1::PairReveal {
                pair_id: pair_id.to_vec(),
                nonce_n,
                csr_pem: csr,
            }),
        )
        .await;
    }
}

fn answer(f: &Frame) -> Result<command_ok::Result, ErrorCode> {
    assert_eq!((f.stream_id, f.kind), (STREAM_COMMANDS, 1), "{f:?}");
    let r = v1::CommandResponse::decode(f.payload.as_slice()).unwrap();
    match r.result.unwrap() {
        command_response::Result::Ok(ok) => Ok(ok.result.unwrap()),
        command_response::Result::Err(e) => Err(ErrorCode::parse(&e.code)),
    }
}

/// A signed-in window, over TLS, subscribed. It reads the cards the hub pushes.
struct Window {
    ws: Ws,
    cards: Vec<v1::PairRequested>,
    decided: Vec<v1::PairDecided>,
}

impl Window {
    async fn open(rig: &Rig, token: &str) -> Self {
        let mut roots = rustls::RootCertStore::empty();
        for c in CertificateDer::pem_slice_iter(rig.ca_pem.as_bytes()) {
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
            token,
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
            decided: Vec::new(),
        };
        // The snapshot comes first.
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
        if f.stream_id == STREAM_EVENTS && f.kind == kinds::PAIR_DECIDED {
            self.decided
                .push(v1::PairDecided::decode(f.payload.as_slice()).unwrap());
        }
        f
    }

    async fn command(
        &mut self,
        c: command_request::Command,
    ) -> Result<command_ok::Result, ErrorCode> {
        let req = v1::CommandRequest {
            request_id: "w1".into(),
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

    async fn card(&mut self) -> v1::PairRequested {
        while self.cards.is_empty() {
            self.next_frame().await;
        }
        self.cards.remove(0)
    }

    async fn open_switch(&mut self) {
        let Ok(command_ok::Result::PairStatus(s)) = self
            .command(command_request::Command::PairOpen(v1::PairOpen {}))
            .await
        else {
            panic!("open")
        };
        assert!(s.open && s.closes_at_ms > 0);
    }

    async fn approve(&mut self, id: &[u8], replace: bool) -> Result<command_ok::Result, ErrorCode> {
        self.command(command_request::Command::PairApprove(v1::PairApprove {
            pair_id: id.to_vec(),
            replace,
        }))
        .await
    }
}

fn result_of(r: Option<Result<command_ok::Result, ErrorCode>>) -> v1::PairResult {
    match r {
        Some(Ok(command_ok::Result::PairResult(p))) => p,
        other => panic!("expected a PairResult, got {other:?}"),
    }
}

/// Step 6 on the machine, as the node will do it: the leaf it saw must chain to the CA it was given and be for the name it dialled.
fn machine_accepts(ca_pem: &str, leaf_der: &[u8], dialled: &str) -> bool {
    use rustls::client::WebPkiServerVerifier;
    let mut roots = rustls::RootCertStore::empty();
    for c in CertificateDer::pem_slice_iter(ca_pem.as_bytes()) {
        roots.add(c.unwrap()).unwrap();
    }
    let v = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider())
        .build()
        .unwrap();
    v.verify_server_cert(
        &CertificateDer::from(leaf_der.to_vec()),
        &[],
        &ServerName::try_from(dialled.to_string()).unwrap(),
        &[],
        UnixTime::now(),
    )
    .is_ok()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_machine_is_paired_end_to_end_and_can_then_connect_with_its_certificate() {
    let rig = rig().await;
    let mut owner = Window::open(&rig, "owner-token").await;
    owner.open_switch().await;

    let mut m = Machine::dial(&rig).await;
    assert_eq!(
        m.leaf_hash, rig.leaf_hash,
        "the machine saw the hub's own leaf"
    );
    let (pair_id, nonce_h) = m.request("laptop").await.unwrap();
    m.reveal(&pair_id).await;

    // The owner's card carries the number, and it is the one the machine computes on its own side.
    let card = owner.card().await;
    assert_eq!(card.pair_id, pair_id);
    assert_eq!((card.name.as_str(), card.os.as_str()), ("laptop", "linux"));
    assert_eq!(card.address, "127.0.0.1");
    assert_eq!(card.number, m.number(&nonce_h));
    assert_eq!(card.number.len(), 6);

    owner.approve(&pair_id, false).await.unwrap();
    let result = result_of(m.recv().await);
    assert!(result.approved);
    assert_eq!(result.ca_pem, rig.ca_pem);

    // Step 6 as the machine will do it: the leaf chains to the CA it was just given, and the name it dialled is the leaf's.
    assert!(machine_accepts(&result.ca_pem, &m.leaf_der, "localhost"));
    // Not an address that is in no name of the leaf: fails closed.
    assert!(!machine_accepts(&result.ca_pem, &m.leaf_der, "127.0.0.1"));
    assert!(!machine_accepts(
        &result.ca_pem,
        &m.leaf_der,
        "other.example"
    ));

    // The certificate is recorded like an enrolled one, and the new node can connect with it.
    assert!(Enrollment::new(rig.state.hub.store())
        .issued_ms_by_node()
        .unwrap()
        .contains_key("laptop"));
    let dir = std::env::temp_dir().join(format!("agora-pair-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let sink = Arc::new(agora_node::daemon::EventSink::open(&dir, "laptop").unwrap());
    sink.node_online("laptop");
    let _link = agora_node::uplink_client::spawn(
        Arc::clone(&sink),
        agora_node::uplink_client::HubLink {
            shared: None,
            mesh_out: None,
            url: format!("wss://localhost:{}", rig.addr.port()),
            ca_pem: result.ca_pem.clone(),
            cert_pem: result.cert_pem.clone(),
            key_pem: m.key_pem.clone(),
            commands: None,
            grants: None,
            mesh: None,
            devices: None,
            state_dir: None,
            certs: None,
            spawner: None,
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

    // The owner's window was told, and the audit trail has the story without the number, the nonces or the CSR.
    while owner.decided.is_empty() {
        owner.next_frame().await;
    }
    assert_eq!(owner.decided[0].outcome, v1::PairOutcome::Approved as i32);
    let rows = agora_hub::audit::Audit::new(rig.state.hub.store())
        .list(0, None, 100)
        .unwrap();
    let actions: Vec<&str> = rows.iter().map(|r| r.action.as_str()).collect();
    for want in ["pair.open", "pair.request", "pair.approve", "pair.issued"] {
        assert!(actions.contains(&want), "{want} in {actions:?}");
    }
    let text = format!("{rows:?}");
    assert!(
        !text.contains(&card.number),
        "the number is in the audit trail"
    );
    assert!(!text.contains("nonce") && !text.contains("BEGIN CERTIFICATE REQUEST"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_denied_request_gets_no_certificate_and_records_no_machine() {
    let rig = rig().await;
    let mut owner = Window::open(&rig, "owner-token").await;
    owner.open_switch().await;
    let mut m = Machine::dial(&rig).await;
    let (id, _) = m.request("denied-one").await.unwrap();
    m.reveal(&id).await;
    let card = owner.card().await;
    owner
        .command(command_request::Command::PairDeny(v1::PairDeny {
            pair_id: card.pair_id.clone(),
        }))
        .await
        .unwrap();
    let r = result_of(m.recv().await);
    assert!(!r.approved && r.cert_pem.is_empty() && r.ca_pem.is_empty());
    assert!(!Enrollment::new(rig.state.hub.store())
        .node_in_use("denied-one")
        .unwrap());
    // The decision is final: approving it afterwards finds nothing.
    assert_eq!(
        owner.approve(&id, false).await.unwrap_err(),
        ErrorCode::PairUnknown
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn closing_the_switch_ends_a_waiting_machine_and_nothing_is_issued() {
    let rig = rig().await;
    let mut owner = Window::open(&rig, "owner-token").await;
    owner.open_switch().await;
    let mut m = Machine::dial(&rig).await;
    let (id, _) = m.request("closing").await.unwrap();
    m.reveal(&id).await;
    owner.card().await;
    owner
        .command(command_request::Command::PairClose(v1::PairClose {}))
        .await
        .unwrap();
    assert_eq!(m.recv().await.unwrap().unwrap_err(), ErrorCode::PairClosed);
    assert!(!Enrollment::new(rig.state.hub.store())
        .node_in_use("closing")
        .unwrap());
    // And a new request is refused while it is closed.
    let mut again = Machine::dial(&rig).await;
    assert_eq!(
        again.request("closing").await.unwrap_err(),
        ErrorCode::PairClosed
    );
}

/// F810: the switch can close without any owner command (its timeout, the fifth failure of its window), so nothing wakes the machine
/// that is waiting. It must still learn within a few seconds, not at the end of its own five minutes. The close here goes through the
/// state directly, as a timeout does: no command arrives, nothing calls `wake`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_waiting_machine_learns_the_switch_closed_by_itself_without_any_owner_command() {
    let rig = rig().await;
    let mut owner = Window::open(&rig, "owner-token").await;
    owner.open_switch().await;
    let mut m = Machine::dial(&rig).await;
    let (id, _) = m.request("closing-alone").await.unwrap();
    m.reveal(&id).await;
    owner.card().await;
    rig.state.hub.pairing().with(|p| p.close(1_000));
    let told = tokio::time::timeout(Duration::from_secs(4), m.recv())
        .await
        .expect("the waiting machine was never told the switch closed");
    assert_eq!(told.unwrap().unwrap_err(), ErrorCode::PairClosed);
    assert!(!Enrollment::new(rig.state.hub.store())
        .node_in_use("closing-alone")
        .unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_closed_hub_refuses_before_it_opens() {
    let rig = rig().await;
    let mut m = Machine::dial(&rig).await;
    assert_eq!(m.request("early").await.unwrap_err(), ErrorCode::PairClosed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_the_owner_may_open_close_approve_or_deny_and_only_the_owner_sees_cards() {
    let rig = rig().await;
    for token in ["member-token", "viewer-token"] {
        let mut w = Window::open(&rig, token).await;
        for c in [
            command_request::Command::PairOpen(v1::PairOpen {}),
            command_request::Command::PairClose(v1::PairClose {}),
            command_request::Command::PairStatus(v1::PairStatus {}),
            command_request::Command::PairApprove(v1::PairApprove {
                pair_id: vec![0; 16],
                replace: false,
            }),
            command_request::Command::PairDeny(v1::PairDeny {
                pair_id: vec![0; 16],
            }),
        ] {
            assert_eq!(
                w.command(c).await.unwrap_err(),
                ErrorCode::AuthForbidden,
                "{token}"
            );
        }
    }
    // The switch is still closed: nothing the member sent opened it.
    let mut m = Machine::dial(&rig).await;
    assert_eq!(m.request("x").await.unwrap_err(), ErrorCode::PairClosed);

    // A pending request is shown to the owner and not to a member's window.
    let mut owner = Window::open(&rig, "owner-token").await;
    owner.open_switch().await;
    let mut member = Window::open(&rig, "member-token").await;
    let mut m = Machine::dial(&rig).await;
    let (id, _) = m.request("seen").await.unwrap();
    m.reveal(&id).await;
    owner.card().await;
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    member
        .command(command_request::Command::PairStatus(v1::PairStatus {}))
        .await
        .unwrap_err();
    assert!(member.cards.is_empty(), "a member saw a card");
    // A window that connects later still gets the card (it is state, not only a push).
    let mut late = Window::open(&rig, "owner-token").await;
    assert_eq!(late.card().await.pair_id, id);
    // A machine's message sent from a window is not accepted.
    let r = owner
        .command(command_request::Command::PairRequest(
            v1::PairRequest::default(),
        ))
        .await;
    assert_eq!(r.unwrap_err(), ErrorCode::ProtocolInvalidMessage);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reveal_with_another_nonce_is_a_mismatch_and_a_second_connection_cannot_claim_the_pair_id(
) {
    let rig = rig().await;
    let mut owner = Window::open(&rig, "owner-token").await;
    owner.open_switch().await;

    // A second connection names the first one's pair id: unknown, and no authority over it.
    let mut m = Machine::dial(&rig).await;
    let (id, _) = m.request("victim").await.unwrap();
    let mut thief = Machine::dial(&rig).await;
    // The thief must be in the "requested" state to send a reveal at all; it sends the reveal as its first message.
    thief.reveal(&id).await;
    assert_eq!(
        thief.recv().await.unwrap().unwrap_err(),
        ErrorCode::PairUnknown
    );
    // The victim is unharmed and completes.
    m.reveal(&id).await;
    let card = owner.card().await;
    assert_eq!(card.pair_id, id);

    // A tampered reveal (a nonce that was not committed to) ends another session.
    let mut bad = Machine::dial(&rig).await;
    let (bid, _) = bad.request("tamper").await.unwrap();
    bad.nonce_n = [0xAB; 16];
    bad.reveal(&bid).await;
    assert_eq!(
        bad.recv().await.unwrap().unwrap_err(),
        ErrorCode::PairMismatch
    );
    // Nothing is left to approve for it.
    assert_eq!(
        owner.approve(&bid, false).await.unwrap_err(),
        ErrorCode::PairUnknown
    );
    // The first one is still pending and approvable.
    owner.approve(&id, false).await.unwrap();
    assert!(result_of(m.recv().await).approved);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_reveal_or_a_message_while_waiting_ends_the_session() {
    let rig = rig().await;
    let mut owner = Window::open(&rig, "owner-token").await;
    owner.open_switch().await;
    let mut m = Machine::dial(&rig).await;
    let (id, _) = m.request("twice").await.unwrap();
    m.reveal(&id).await;
    owner.card().await;
    m.reveal(&id).await;
    assert_eq!(m.recv().await.unwrap().unwrap_err(), ErrorCode::PairState);
    assert_eq!(
        owner.approve(&id, false).await.unwrap_err(),
        ErrorCode::PairUnknown
    );
    assert!(!Enrollment::new(rig.state.hub.store())
        .node_in_use("twice")
        .unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_name_that_belongs_to_another_machine_needs_replace() {
    let rig = rig().await;
    // "laptop" is already enrolled.
    let (_, csr) = new_device_request().unwrap();
    let ca = rig.state.ca.as_ref().unwrap();
    let code = Enrollment::new(rig.state.hub.store())
        .create_code(server::now_ms())
        .unwrap();
    let mut e = Enrollment::new(rig.state.hub.store());
    let old = e
        .redeem(ca, &code, "laptop", &csr, server::now_ms())
        .unwrap();

    let mut owner = Window::open(&rig, "owner-token").await;
    owner.open_switch().await;
    let mut m = Machine::dial(&rig).await;
    let (id, _) = m.request("laptop").await.unwrap();
    m.reveal(&id).await;
    owner.card().await;
    // Refused, and the request stays pending.
    assert_eq!(
        owner.approve(&id, false).await.unwrap_err(),
        ErrorCode::PairState
    );
    assert!(Enrollment::new(rig.state.hub.store())
        .is_active(&old.serial_hex)
        .unwrap());
    owner.approve(&id, true).await.unwrap();
    assert!(result_of(m.recv().await).approved);
    // The old certificate was revoked.
    assert!(!Enrollment::new(rig.state.hub.store())
        .is_active(&old.serial_hex)
        .unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_fourth_request_in_a_minute_from_one_address_is_refused_with_a_wait() {
    let rig = rig().await;
    let mut owner = Window::open(&rig, "owner-token").await;
    owner.open_switch().await;
    for i in 0..3 {
        let mut m = Machine::dial(&rig).await;
        m.request(&format!("m{i}")).await.unwrap();
        // The connection ends here (dropped), freeing its pending slot.
    }
    let mut m = Machine::dial(&rig).await;
    assert_eq!(m.request("m3").await.unwrap_err(), ErrorCode::PairLimit);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tls_12_connection_cannot_pair() {
    let rig = rig().await;
    let mut owner = Window::open(&rig, "owner-token").await;
    owner.open_switch().await;
    let mut m = Machine::dial_with(&rig, &[&rustls::version::TLS12]).await;
    // No exporter binding exists on TLS 1.2, so the hub does not answer a pairing request on it at all.
    let c = m.commitment();
    m.send(
        "req",
        command_request::Command::PairRequest(v1::PairRequest {
            commitment: c,
            node_name: "old-tls".into(),
            os: "linux".into(),
            version: "0".into(),
        }),
    )
    .await;
    assert!(m.recv().await.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_leaf_from_another_ca_or_for_another_name_fails_the_machines_check() {
    let rig = rig().await;
    let mut owner = Window::open(&rig, "owner-token").await;
    owner.open_switch().await;
    let mut m = Machine::dial(&rig).await;
    let (id, _) = m.request("check").await.unwrap();
    m.reveal(&id).await;
    owner.card().await;
    owner.approve(&id, false).await.unwrap();
    let r = result_of(m.recv().await);
    // The right CA and the right leaf and name: accepted.
    assert!(machine_accepts(&r.ca_pem, &m.leaf_der, "localhost"));
    // A CA that did not issue the leaf (what a party in the middle would hand over): refused.
    let other = Ca::generate("another hub").unwrap();
    assert!(!machine_accepts(other.cert_pem(), &m.leaf_der, "localhost"));
    // A leaf the CA issued for another name: refused for the name the machine dialled.
    let ca = rig.state.ca.as_ref().unwrap();
    let (cert, _) = ca.issue_server(&["elsewhere.example".to_string()]).unwrap();
    let der = CertificateDer::from_pem_slice(cert.as_bytes()).unwrap();
    assert!(!machine_accepts(&r.ca_pem, der.as_ref(), "localhost"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_party_in_the_middle_gets_a_different_number_than_the_owner_sees() {
    let rig = rig().await;
    let mut owner = Window::open(&rig, "owner-token").await;
    owner.open_switch().await;
    // The relay is a machine of its own toward the hub: it forwards the commitment and the reveal unchanged. The real machine
    // sits behind a different TLS session with a different leaf, so what it computes differs from the card.
    let mut relay = Machine::dial(&rig).await;
    let (id, nonce_h) = relay.request("victim").await.unwrap();
    relay.reveal(&id).await;
    let card = owner.card().await;
    let mut behind = Machine::dial(&rig).await;
    behind.nonce_n = relay.nonce_n;
    behind.csr_hash = relay.csr_hash;
    // Same nonces and CSR, but another exporter (its own TLS session) and the attacker's leaf instead of the hub's.
    let mut seen_by_victim = behind.number(&nonce_h);
    assert_ne!(
        seen_by_victim, card.number,
        "numbers must differ across TLS sessions"
    );
    behind.leaf_hash = [0x55; 32];
    seen_by_victim = behind.number(&nonce_h);
    assert_ne!(seen_by_victim, card.number);
    owner
        .command(command_request::Command::PairDeny(v1::PairDeny {
            pair_id: id,
        }))
        .await
        .unwrap();
}
