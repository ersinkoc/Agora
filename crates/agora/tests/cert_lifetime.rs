//! Certificate lifetime (docs/adr/0085-machine-and-hub-certificates-renew-themselves.md). Production certificates last 90 days; these
//! tests make the lifetime a few seconds (a test-only setting of the CA) and wait past the end, to show what the product does:
//! a machine whose certificate has ended and that did not renew is refused on its next connection, a hub whose own certificate has
//! ended and was not replaced is refused by every machine that dials it, and with renewal (a machine asks the hub for a new
//! certificate; the hub replaces its own before it ends, without a restart) both keep working past the first certificate's end.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use agora_core::AgentId;
use agora_hub::ca::{cert_not_after_secs, new_device_request, Ca};
use agora_hub::enroll::{Enrollment, Lockout};
use agora_hub::hub::Hub;
use agora_hub::server::{self, ClientAuth, ServerState};
use agora_hub::store::SqliteStore;
use agora_hub::tls;
use agora_node::daemon::EventSink;
use agora_node::uplink_client::{spawn, HubLink};
use agora_protocol::handshake::Support;

const SHORT: Duration = Duration::from_secs(3);
static DIALS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

struct NoClients;

impl ClientAuth for NoClients {
    fn check(&self, _: &str) -> bool {
        false
    }
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("agora-life-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

struct Machine {
    id: &'static str,
    cert_pem: String,
    key_pem: String,
}

struct Rig {
    rt: tokio::runtime::Runtime,
    state: Arc<ServerState>,
    ca_pem: String,
    acceptor: tokio_rustls::TlsAcceptor,
    machines: Vec<Machine>,
    /// Seconds since 1970 when the shortest certificate here ends.
    ends_at: i64,
    tag: &'static str,
}

/// A hub with these machines enrolled. `server_life` and `node_life` shorten the hub's own certificate and the machines' ones
/// (None keeps the 90 days of production). Machines are `(id, short)`: `short` gives it the `node_life`.
fn rig(
    tag: &'static str,
    server_life: Option<Duration>,
    node_life: Duration,
    machines: &[(&'static str, bool)],
) -> Rig {
    let mut ca = Ca::generate("hub ca").unwrap();
    let store = SqliteStore::open_in_memory().unwrap();
    let mut ends = Vec::new();
    let server_ca;
    let (server_cert, server_key) = match server_life {
        Some(l) => {
            server_ca = Ca::from_pem(ca.cert_pem(), &ca.key_pem())
                .unwrap()
                .with_lifetime_for_tests(l);
            server_ca.issue_server(&["localhost".to_string()]).unwrap()
        }
        None => ca.issue_server(&["localhost".to_string()]).unwrap(),
    };
    if server_life.is_some() {
        ends.push(cert_not_after_secs(&server_cert).unwrap());
    }
    let acceptor = tls::acceptor_with(&server_cert, &server_key, ca.cert_pem(), true).unwrap();
    let mut out = Vec::new();
    for (id, short) in machines {
        let mut enrollment = Enrollment::new(&store);
        // Enrolled two hours ago, so that the hour a machine must wait between certificates (ADR-0085) has passed.
        let at = server::now_ms() - 2 * 3600 * 1000;
        let code = enrollment.create_code(at).unwrap();
        let (key_pem, csr) = new_device_request().unwrap();
        let issuer = if *short {
            Ca::from_pem(ca.cert_pem(), &ca.key_pem())
                .unwrap()
                .with_lifetime_for_tests(node_life)
        } else {
            Ca::from_pem(ca.cert_pem(), &ca.key_pem()).unwrap()
        };
        let issued = enrollment.redeem(&issuer, &code, id, &csr, at).unwrap();
        if *short {
            ends.push(cert_not_after_secs(&issued.cert_pem).unwrap());
        }
        out.push(Machine {
            id,
            cert_pem: issued.cert_pem,
            key_pem,
        });
    }
    let ca_pem = ca.cert_pem().to_string();
    ca = Ca::from_pem(&ca_pem, &ca.key_pem()).unwrap();
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
    Rig {
        rt: tokio::runtime::Runtime::new().unwrap(),
        state,
        ca_pem,
        acceptor,
        machines: out,
        ends_at: ends.into_iter().min().unwrap(),
        tag,
    }
}

impl Rig {
    fn listen_with(&self, acceptor: tokio_rustls::TlsAcceptor) -> SocketAddr {
        let l = self
            .rt
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .unwrap();
        let bound = l.local_addr().unwrap();
        self.rt
            .spawn(server::run(l, Arc::clone(&self.state), Some(acceptor)));
        bound
    }

    fn listen_slot(&self, slot: tls::TlsSlot) -> SocketAddr {
        let l = self
            .rt
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .unwrap();
        let bound = l.local_addr().unwrap();
        self.rt
            .spawn(server::run_slot(l, Arc::clone(&self.state), slot));
        bound
    }

    fn dial(
        &self,
        id: &str,
        addr: SocketAddr,
    ) -> (Arc<EventSink>, agora_node::uplink_client::UplinkHandle) {
        let m = self.machines.iter().find(|m| m.id == id).unwrap();
        self.dial_with(id, &m.cert_pem, &m.key_pem, addr)
    }

    /// `dial` with other credentials for the same machine (the ones a renewal returned).
    fn dial_with(
        &self,
        id: &str,
        cert_pem: &str,
        key_pem: &str,
        addr: SocketAddr,
    ) -> (Arc<EventSink>, agora_node::uplink_client::UplinkHandle) {
        let link = HubLink {
            mesh_out: None,
            url: format!("wss://localhost:{}", addr.port()),
            ca_pem: self.ca_pem.clone(),
            cert_pem: cert_pem.to_string(),
            key_pem: key_pem.to_string(),
            commands: None,
            grants: None,
            mesh: None,
            devices: None,
            state_dir: None,
        };
        let n = DIALS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let sink = sink(&tmp(&format!("{}-{id}-{n}", self.tag)), id);
        let handle = spawn(Arc::clone(&sink), link).unwrap();
        (sink, handle)
    }

    /// What a machine does when it renews: a connection made with its certificate, one `RenewRequest` with a fresh key, one answer.
    /// Returns the new `(cert_pem, key_pem)`, or the error code the hub answered with.
    fn renew(&self, addr: SocketAddr, id: &str) -> Result<(String, String), String> {
        use agora_protocol::frame::STREAM_COMMANDS;
        use agora_protocol::v1::{
            command_ok, command_request, command_response, CommandRequest, CommandResponse,
            RenewRequest,
        };
        use agora_protocol::Frame;
        use prost::Message;
        use rustls::pki_types::pem::PemObject;
        use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
        let m = self.machines.iter().find(|m| m.id == id).unwrap();
        let (new_key, csr) = new_device_request().unwrap();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut roots = rustls::RootCertStore::empty();
        for c in CertificateDer::pem_slice_iter(self.ca_pem.as_bytes()) {
            roots.add(c.unwrap()).unwrap();
        }
        let cfg = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_client_auth_cert(
                CertificateDer::pem_slice_iter(m.cert_pem.as_bytes())
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap(),
                PrivateKeyDer::from_pem_slice(m.key_pem.as_bytes()).unwrap(),
            )
            .unwrap();
        let connector = tokio_rustls::TlsConnector::from(Arc::new(cfg));
        self.rt.block_on(async {
            let tcp = tokio::net::TcpStream::connect(addr)
                .await
                .map_err(|e| e.to_string())?;
            let tls = connector
                .connect(ServerName::try_from("localhost").unwrap(), tcp)
                .await
                .map_err(|e| e.to_string())?;
            let mut c =
                agora_protocol::ws::connect(tls, &format!("wss://localhost:{}/", addr.port()))
                    .await
                    .map_err(|e| e.to_string())?;
            let req = CommandRequest {
                request_id: "renew".into(),
                idempotency_key: String::new(),
                target_node: String::new(),
                command: Some(command_request::Command::Renew(RenewRequest {
                    csr_pem: csr,
                })),
            };
            c.send(&Frame {
                stream_id: STREAM_COMMANDS,
                kind: 0,
                seq: None,
                payload: req.encode_to_vec(),
            })
            .await
            .map_err(|e| e.to_string())?;
            let f = tokio::time::timeout(Duration::from_secs(10), c.recv())
                .await
                .map_err(|_| "no answer".to_string())?
                .map_err(|e| e.to_string())?
                .ok_or("closed")?;
            match CommandResponse::decode(f.payload.as_slice())
                .unwrap()
                .result
            {
                Some(command_response::Result::Ok(ok)) => match ok.result {
                    Some(command_ok::Result::Renew(r)) => Ok((r.cert_pem, new_key)),
                    other => Err(format!("{other:?}")),
                },
                Some(command_response::Result::Err(e)) => Err(e.code),
                None => Err("empty".into()),
            }
        })
    }

    fn online(&self, id: &str) -> bool {
        self.state
            .hub
            .view()
            .0
            .nodes
            .get(id)
            .is_some_and(|n| n.online)
    }

    fn wait_online(&self, id: &str) {
        let end = Instant::now() + Duration::from_secs(20);
        while !self.online(id) {
            assert!(Instant::now() < end, "{id} never connected");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Waits until a few seconds past the end of the shortest certificate here.
    fn wait_past_expiry(&self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let wait = (self.ends_at + 2 - now).max(0) as u64;
        std::thread::sleep(Duration::from_secs(wait));
    }
}

fn sink(dir: &Path, node: &str) -> Arc<EventSink> {
    let sink = Arc::new(EventSink::open(dir, node).unwrap());
    sink.node_online(node);
    sink.agent_launched(
        &AgentId::new("a1"),
        "proj",
        node,
        Path::new("/w/proj"),
        Path::new("/w/wt"),
        "agent/x",
        "shell",
        true,
    );
    sink
}

/// A machine certificate that has ended is refused on the next connection (a machine that did not renew in time must join again).
#[test]
fn a_machine_that_missed_its_renewal_is_refused_after_its_certificate_ends() {
    let rig = rig(
        "machine",
        None,
        SHORT,
        &[("early", true), ("late", true), ("control", false)],
    );
    let addr = rig.listen_with(rig.acceptor.clone());

    // Before the end, the short certificate works.
    let (_early_sink, _early) = rig.dial("early", addr);
    rig.wait_online("early");

    rig.wait_past_expiry();

    // After the end, the same enrolled machine dialing again is refused, however long it keeps trying (the uplink retries)...
    let (late_sink, _late) = rig.dial("late", addr);
    // ...while a machine enrolled with a normal certificate connects through the very same hub, so the harness is sound.
    let (_c_sink, _control) = rig.dial("control", addr);
    rig.wait_online("control");
    std::thread::sleep(Duration::from_secs(4));
    assert!(
        !rig.online("late"),
        "an expired machine certificate was accepted"
    );
    assert_eq!(
        late_sink.acked(),
        0,
        "the hub took events from an expired certificate"
    );
}

/// A hub whose own certificate has ended is refused by the machines that dial it. The product replaces it before it ends
/// (`the_hubs_certificate_is_replaced_before_it_ends...` below); this is what the hub would serve if that did not happen.
#[test]
fn a_hub_whose_own_certificate_ended_and_was_not_replaced_is_refused_by_new_connections() {
    let rig = rig(
        "server",
        Some(SHORT),
        SHORT,
        &[("early", false), ("late", false)],
    );
    let addr = rig.listen_with(rig.acceptor.clone());
    let (_s, _early) = rig.dial("early", addr);
    rig.wait_online("early");

    rig.wait_past_expiry();

    let (late_sink, _late) = rig.dial("late", addr);
    std::thread::sleep(Duration::from_secs(4));
    assert!(
        !rig.online("late"),
        "a machine trusted a hub certificate that had ended"
    );
    assert_eq!(late_sink.acked(), 0);

    // A hub with a new server certificate from the same CA is accepted again.
    let fresh = {
        let (c, k) = rig
            .state
            .ca
            .as_ref()
            .unwrap()
            .issue_server(&["localhost".to_string()])
            .unwrap();
        tls::acceptor_with(&c, &k, &rig.ca_pem, true).unwrap()
    };
    let addr2 = rig.listen_with(fresh);
    let (_s2, _late2) = rig.dial("late", addr2);
    rig.wait_online("late");
}

/// ADR-0085: a machine whose certificate would have ended while it was connected renews it, and connects with the new one after the end
/// of the old.
#[test]
fn a_machine_connected_longer_than_its_lifetime_still_connects_after_renewal() {
    let rig = rig("renew", None, SHORT, &[("m", true), ("control", false)]);
    let addr = rig.listen_with(rig.acceptor.clone());
    let (_sink, first) = rig.dial("m", addr);
    rig.wait_online("m");

    // The machine renews while its certificate is valid (the hub's CA gives the new one the production lifetime).
    let (cert, key) = rig.renew(addr, "m").expect("renewal answered");
    assert!(
        cert_not_after_secs(&cert).unwrap() > rig.ends_at + 24 * 3600,
        "the new certificate lasts a normal lifetime"
    );

    rig.wait_past_expiry();
    // It has now been connected for longer than the first certificate lasts, and is still there.
    assert!(rig.online("m"), "an open connection is not cut at the end");

    // The first certificate cannot start a new connection any more; the renewed one can.
    drop(first);
    let end = Instant::now() + Duration::from_secs(20);
    while rig.online("m") {
        assert!(Instant::now() < end, "m never went offline");
        std::thread::sleep(Duration::from_millis(50));
    }
    let (_s2, _again) = rig.dial_with("m", &cert, &key, addr);
    rig.wait_online("m");
}

/// ADR-0085: the renewal itself needs a certificate that is valid and active; a machine that is late cannot ask.
#[test]
fn a_machine_whose_certificate_ended_cannot_renew_it() {
    let rig = rig("late-renew", None, SHORT, &[("m", true)]);
    let addr = rig.listen_with(rig.acceptor.clone());
    rig.wait_past_expiry();
    // The hub refuses the TLS handshake of an ended certificate, so there is no one to answer.
    assert!(
        rig.renew(addr, "m").is_err(),
        "an ended certificate renewed"
    );
}

/// ADR-0085: the hub replaces its own server certificate before it ends, with no restart, and a connection made after the old
/// one's end succeeds.
#[test]
fn the_hubs_certificate_is_replaced_before_it_ends_and_a_new_connection_after_the_old_end_succeeds()
{
    use agora_hub::leaf::Leaf;
    let rig = rig(
        "reload",
        Some(SHORT),
        SHORT,
        &[("early", false), ("late", false)],
    );
    // A leaf whose certificates last 6 seconds, signed by the hub's own CA (the machines pin it).
    let ca = Ca::from_pem(&rig.ca_pem, &rig.state.ca.as_ref().unwrap().key_pem())
        .unwrap()
        .with_lifetime_for_tests(Duration::from_secs(6));
    let (leaf, _) = Leaf::start(
        ca,
        vec![],
        "127.0.0.1".parse().unwrap(),
        Box::new(|| None),
        Box::new(|| Ok(vec![])),
        None,
    )
    .unwrap();
    let first_end = leaf.not_after_secs();
    let first_hash = leaf.hash();
    let addr = rig.listen_slot(leaf.slot());
    let (_s, _early) = rig.dial("early", addr);
    rig.wait_online("early");

    // Half way through its life the daily check (here with a 4 second threshold) replaces it.
    let now = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    };
    assert!(
        !leaf.renew_if_ending(now(), 1).unwrap(),
        "a certificate with time left is left alone"
    );
    std::thread::sleep(Duration::from_secs((first_end - 3 - now()).max(0) as u64));
    assert!(leaf.renew_if_ending(now(), 4).unwrap());
    assert_ne!(leaf.hash(), first_hash);
    assert!(leaf.not_after_secs() > first_end);

    // After the first certificate's end, a connection that begins now succeeds, and the one opened before is untouched.
    std::thread::sleep(Duration::from_secs((first_end + 1 - now()).max(0) as u64));
    let (_s2, _late) = rig.dial("late", addr);
    rig.wait_online("late");
    assert!(rig.online("early"));
}
