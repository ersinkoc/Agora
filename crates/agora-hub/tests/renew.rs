//! ADR-0085 at the hub, over real sockets: a machine renews its certificate on a connection of its own, made with the certificate it has.
//! The refusal reasons themselves are tested against the record in `enroll.rs`; here the point is who may ask and what comes back.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use agora_hub::ca::{new_device_request, Ca};
use agora_hub::enroll::{Enrollment, Lockout};
use agora_hub::hub::Hub;
use agora_hub::server::{self, ClientAuth, ServerState};
use agora_hub::store::SqliteStore;
use agora_hub::tls;
use agora_protocol::frame::kinds::{SNAPSHOT, SUBSCRIBE};
use agora_protocol::frame::{STREAM_COMMANDS, STREAM_EVENTS};
use agora_protocol::handshake::Support;
use agora_protocol::v1::{self, command_ok, command_request as cr, command_response};
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

type TlsConn = Conn<tokio_rustls::client::TlsStream<TcpStream>>;

fn command(id: &str, c: cr::Command) -> Frame {
    Frame {
        stream_id: STREAM_COMMANDS,
        kind: 0,
        seq: None,
        payload: v1::CommandRequest {
            request_id: id.into(),
            idempotency_key: String::new(),
            target_node: String::new(),
            command: Some(c),
        }
        .encode_to_vec(),
    }
}

fn renew(csr: &str) -> Frame {
    command(
        "r",
        cr::Command::Renew(v1::RenewRequest {
            csr_pem: csr.to_string(),
        }),
    )
}

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

/// What a connection answers to `first`: the response, or `None` when the hub closed it without one.
async fn ask(c: &mut TlsConn, first: Frame) -> Option<v1::CommandResponse> {
    // A refused connection may be closed before or while the frame is written.
    let _ = c.send(&first).await;
    match tokio::time::timeout(Duration::from_secs(10), c.recv()).await {
        Ok(Ok(Some(f))) if (f.stream_id, f.kind) == (STREAM_COMMANDS, 1) => {
            Some(v1::CommandResponse::decode(f.payload.as_slice()).unwrap())
        }
        _ => None,
    }
}

fn code(r: &v1::CommandResponse) -> String {
    match &r.result {
        Some(command_response::Result::Err(e)) => format!(
            "{} {}",
            e.code,
            e.params
                .values()
                .filter_map(|p| match &p.value {
                    Some(v1::param_value::Value::Str(s)) => Some(s.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(",")
        )
        .trim()
        .to_string(),
        _ => String::new(),
    }
}

struct Rig {
    addr: std::net::SocketAddr,
    trust: Ca,
    state: Arc<ServerState>,
    cert: String,
    key: String,
}

const HOUR: u64 = 60 * 60 * 1000;

/// A TLS hub with machine `box` enrolled two hours ago (so the hour between certificates has passed).
async fn rig() -> Rig {
    let ca = Ca::generate("hub ca").unwrap();
    let (server_cert, server_key) = ca.issue_server(&["localhost".to_string()]).unwrap();
    let acceptor = tls::acceptor_with(&server_cert, &server_key, ca.cert_pem(), true).unwrap();
    let trust = Ca::from_pem(ca.cert_pem(), &ca.key_pem()).unwrap();
    let state = Arc::new(ServerState {
        hub: Arc::new(Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap()),
        support: Support {
            generations: vec![1],
            capabilities: vec![],
            required: vec![],
        },
        ca: Some(ca),
        lockout: Arc::new(Lockout::new()),
        auth: Box::new(Tokens),
        assets: None,
        sessions: None,
    });
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(server::run(l, Arc::clone(&state), Some(acceptor)));
    let at = server::now_ms() - 2 * HOUR;
    let mut e = Enrollment::new(state.hub.store());
    let c = e.create_code(at).unwrap();
    let (key, csr) = new_device_request().unwrap();
    let issued = e
        .redeem(state.ca.as_ref().unwrap(), &c, "box", &csr, at)
        .unwrap();
    Rig {
        addr,
        trust,
        state,
        cert: issued.cert_pem,
        key,
    }
}

#[tokio::test]
async fn a_machine_renews_on_a_connection_made_with_its_certificate() {
    let r = rig().await;
    let (new_key, csr) = new_device_request().unwrap();
    let mut c = dial_tls(
        r.addr,
        client_tls(&r.trust, Some((r.cert.clone(), r.key.clone()))),
    )
    .await;
    let resp = ask(&mut c, renew(&csr)).await.expect("an answer");
    let Some(command_response::Result::Ok(v1::CommandOk {
        result: Some(command_ok::Result::Renew(done)),
    })) = resp.result
    else {
        panic!("{resp:?}")
    };
    assert!(done.not_after_ms > server::now_ms() + 80 * 24 * HOUR);
    // The answer ends the connection: one question, one answer.
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(5), c.recv()).await,
        Ok(Ok(None) | Err(_))
    ));
    // The new certificate (with the new key) is the machine's: the hub lists it, and the old one is still valid (grace).
    let id = tls::identity(Some(&[rustls::pki_types::CertificateDer::from(pem_der(
        &done.cert_pem,
    ))]))
    .unwrap();
    assert_eq!(id.node_id, "box");
    let e = Enrollment::new(r.state.hub.store());
    assert!(e.is_active(&id.serial_hex).unwrap());
    let old = tls::identity(Some(&[rustls::pki_types::CertificateDer::from(pem_der(
        &r.cert,
    ))]))
    .unwrap();
    assert!(e.is_active(&old.serial_hex).unwrap());
    // It can be used to connect; asking again at once is refused by the hour limit.
    let mut again = dial_tls(
        r.addr,
        client_tls(&r.trust, Some((done.cert_pem.clone(), new_key))),
    )
    .await;
    let (_, csr2) = new_device_request().unwrap();
    let refused = ask(&mut again, renew(&csr2)).await.expect("an answer");
    assert_eq!(code(&refused), "enroll.renew_refused too_frequent");
    // The old certificate is still valid (grace) but is superseded: it may connect, and may not renew. Asking over and over is
    // one audit row.
    for _ in 0..3 {
        let mut stale = dial_tls(
            r.addr,
            client_tls(&r.trust, Some((r.cert.clone(), r.key.clone()))),
        )
        .await;
        let refused = ask(&mut stale, renew(&csr2)).await.expect("an answer");
        assert_eq!(code(&refused), "enroll.renew_refused superseded");
    }
    // The audit trail has one row for the renewal (with the serial presented) and one per kind of refusal.
    let rows = agora_hub::audit::Audit::new(r.state.hub.store())
        .list(0, None, 100)
        .unwrap();
    let renewals: Vec<_> = rows.iter().filter(|a| a.action == "enroll.renew").collect();
    assert_eq!(renewals.len(), 1);
    assert_eq!(renewals[0].node, "box");
    assert!(renewals[0].outcome.contains(&id.serial_hex));
    assert!(renewals[0]
        .outcome
        .contains(&format!("presented={}", old.serial_hex)));
    let refusals: Vec<_> = rows
        .iter()
        .filter(|a| a.action == "enroll.renew_refused")
        .map(|a| a.outcome.clone())
        .collect();
    assert_eq!(refusals.len(), 2, "{refusals:?}");
    assert!(refusals.contains(&format!("reason=superseded presented={}", old.serial_hex)));
    assert!(refusals.contains(&format!("reason=too_frequent presented={}", id.serial_hex)));
}

fn pem_der(pem: &str) -> Vec<u8> {
    x509_parser::pem::parse_x509_pem(pem.as_bytes())
        .unwrap()
        .1
        .contents
}

#[tokio::test]
async fn a_peer_without_a_certificate_cannot_renew() {
    let r = rig().await;
    let (_, csr) = new_device_request().unwrap();
    let mut anon = dial_tls(r.addr, client_tls(&r.trust, None)).await;
    let resp = ask(&mut anon, renew(&csr)).await.expect("an answer");
    assert_eq!(code(&resp), "auth.unauthenticated");
    let rows = agora_hub::audit::Audit::new(r.state.hub.store())
        .list(0, None, 10)
        .unwrap();
    assert!(rows.iter().all(|a| a.action != "enroll.renew"));
}

#[tokio::test]
async fn a_revoked_certificate_cannot_connect_to_renew() {
    let r = rig().await;
    Enrollment::new(r.state.hub.store())
        .revoke("box", server::now_ms())
        .unwrap();
    let (_, csr) = new_device_request().unwrap();
    let mut c = dial_tls(
        r.addr,
        client_tls(&r.trust, Some((r.cert.clone(), r.key.clone()))),
    )
    .await;
    assert!(ask(&mut c, renew(&csr)).await.is_none());
    // Nothing was issued: the machine has no certificate that is not revoked, and no audit row says otherwise.
    let e = Enrollment::new(r.state.hub.store());
    assert!(e.issued_ms_by_node().unwrap().is_empty());
    let rows = agora_hub::audit::Audit::new(r.state.hub.store())
        .list(0, None, 10)
        .unwrap();
    assert!(rows.iter().all(|a| a.action != "enroll.renew"));
}

#[tokio::test]
async fn a_csr_that_does_not_parse_is_an_invalid_message_and_issues_nothing() {
    let r = rig().await;
    let mut c = dial_tls(
        r.addr,
        client_tls(&r.trust, Some((r.cert.clone(), r.key.clone()))),
    )
    .await;
    let resp = ask(&mut c, renew("not a csr")).await.expect("an answer");
    assert_eq!(code(&resp), "protocol.invalid_message csr_pem");
}

/// A signed-in client (a window) cannot renew a machine's certificate for it: the command is not one a client forwards.
#[tokio::test]
async fn a_client_cannot_send_a_renewal() {
    let state = Arc::new(ServerState {
        hub: Arc::new(Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap()),
        support: Support {
            generations: vec![1],
            capabilities: vec![],
            required: vec![],
        },
        ca: Some(Ca::generate("ca").unwrap()),
        lockout: Arc::new(Lockout::new()),
        auth: Box::new(Tokens),
        assets: None,
        sessions: None,
    });
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(server::run(l, state, None));
    let s = TcpStream::connect(addr).await.unwrap();
    let mut c = ws::connect_with_token(s, &format!("ws://{addr}/"), "good-token")
        .await
        .unwrap();
    c.send(&Frame {
        stream_id: STREAM_EVENTS,
        kind: SUBSCRIBE,
        seq: None,
        payload: v1::Subscribe::default().encode_to_vec(),
    })
    .await
    .unwrap();
    loop {
        let f = tokio::time::timeout(Duration::from_secs(10), c.recv())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if f.kind == SNAPSHOT {
            break;
        }
    }
    let mut req = renew("x");
    let mut decoded = v1::CommandRequest::decode(req.payload.as_slice()).unwrap();
    decoded.target_node = "box".into();
    req.payload = decoded.encode_to_vec();
    c.send(&req).await.unwrap();
    let answer = loop {
        let f = tokio::time::timeout(Duration::from_secs(10), c.recv())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if (f.stream_id, f.kind) == (STREAM_COMMANDS, 1) {
            break v1::CommandResponse::decode(f.payload.as_slice()).unwrap();
        }
    };
    assert_eq!(code(&answer), "protocol.invalid_message command");
}
