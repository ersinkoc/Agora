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

/// The lifetime of a certificate that a test's first connection must still find valid: the dial, the TLS handshake and the hub's
/// welcome come before it ends (and, for a machine that renews, the renewal, due at once as the certificate is dated five minutes
/// back). They take about 0.1 s on Linux, but on a busy Windows runner each refused address of `localhost` costs up to half a second
/// (`dial::connect_bounded`) and a debug-build handshake runs beside the file's other tests, so the 3 s of [`SHORT`] (2 to 3 s in
/// fact, as `not_after` is whole seconds) was too tight (#1111). A test that waits past the end pays these seconds once.
const ROOM_TO_DIAL: Duration = Duration::from_secs(8);

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
            capabilities: vec!["terminal.raw".into(), "enroll.renew".into()],
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
            certs: None,
            spawner: None,
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

    /// `(action, node)` of every audit row.
    fn audit_rows(&self) -> Vec<(String, String)> {
        agora_hub::audit::Audit::new(self.state.hub.store())
            .list(0, None, 1000)
            .unwrap()
            .into_iter()
            .map(|r| (r.action, r.node))
            .collect()
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
        ROOM_TO_DIAL,
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
        Some(ROOM_TO_DIAL),
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
    let rig = rig(
        "renew",
        None,
        ROOM_TO_DIAL,
        &[("m", true), ("control", false)],
    );
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
    let rig = rig(
        "late-renew",
        None,
        SHORT,
        &[("m", true), ("control", false)],
    );
    let addr = rig.listen_with(rig.acceptor.clone());
    rig.wait_past_expiry();
    // The hub refuses the TLS handshake of an ended certificate, so there is no one to answer.
    let err = rig
        .renew(addr, "m")
        .expect_err("an ended certificate renewed");
    // Refused by TLS before any command, never by a hub answer (`enroll.renew_refused`). How the refusal surfaces depends on the
    // platform: a certificate alert (Linux, macOS), or the connection reset, aborted or closed while the handshake finishes (Windows:
    // os error 10053 / 10054).
    let text = err.to_lowercase();
    let tls_refusal = [
        "cert",
        "alert",
        "expired",
        "reset",
        "abort",
        "broken pipe",
        "closed",
        "10053",
        "10054",
    ]
    .iter()
    .any(|w| text.contains(w));
    assert!(
        !err.starts_with("enroll.") && tls_refusal,
        "refused for another reason than the ended certificate: {err}"
    );
    // It is the certificate and not the setup: a machine with a valid certificate renews at the same hub, in the same test.
    rig.renew(addr, "control")
        .expect("a valid certificate renews at the same hub");
    // And the hub saw no renewal command from the ended one: the only audit row of any renewal is the control machine's.
    let rows = rig.audit_rows();
    assert!(
        rows.iter()
            .all(|(action, node)| !action.starts_with("enroll.renew") || node == "control"),
        "the hub recorded a renewal for the ended certificate: {rows:?}"
    );
    assert_eq!(
        rows.iter().filter(|(a, _)| a == "enroll.renew").count(),
        1,
        "{rows:?}"
    );
}

/// ADR-0085: the hub replaces its own server certificate before it ends, with no restart, and a connection made after the old
/// one's end succeeds.
#[test]
fn the_hubs_certificate_is_replaced_before_it_ends_and_a_new_connection_after_the_old_end_succeeds()
{
    use agora_hub::leaf::Leaf;
    // The rig's own server certificate is not served here; the leaf's is.
    let rig = rig(
        "reload",
        Some(SHORT),
        SHORT,
        &[("early", false), ("late", false)],
    );
    // A leaf whose certificates last `ROOM_TO_DIAL`, signed by the hub's own CA (the machines pin it). Both dials below must finish
    // inside one of them: the first before the first certificate is 1 s from its end, the late one before the replacement ends.
    let ca = Ca::from_pem(&rig.ca_pem, &rig.state.ca.as_ref().unwrap().key_pem())
        .unwrap()
        .with_lifetime_for_tests(ROOM_TO_DIAL);
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

    // Near the end of its life the daily check (here with a 3 second threshold) replaces it.
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
    std::thread::sleep(Duration::from_secs((first_end - 2 - now()).max(0) as u64));
    assert!(leaf.renew_if_ending(now(), 3).unwrap());
    assert_ne!(leaf.hash(), first_hash);
    assert!(leaf.not_after_secs() > first_end);

    // After the first certificate's end, a connection that begins now succeeds, and the one opened before is untouched.
    std::thread::sleep(Duration::from_secs((first_end + 1 - now()).max(0) as u64));
    let (_s2, _late) = rig.dial("late", addr);
    rig.wait_online("late");
    assert!(rig.online("early"));
}

// ---- ADR-0085 step 2: the node renews its own certificate ----

use agora_node::cert_renew::{pair_matches, validity};
use agora_node::link_renewal::{self, Outcome, Real, Renewer, Timing, PENDING, PREVIOUS};
use agora_node::uplink_client::{renew, CertSource, LinkError, RenewError};

/// Waits that keep a test quick; the product's are an hour (look), a minute (hub not known yet) and an hour (too frequent).
fn fast() -> Timing {
    Timing {
        check_every: Duration::from_millis(200),
        unknown_every: Duration::from_millis(100),
        too_frequent: Duration::from_secs(3600),
        refused_every: Duration::from_secs(86_400),
        retry_first: Duration::from_millis(100),
        retry_max: Duration::from_millis(400),
    }
}

type Ask = Box<dyn Fn(&HubLink, &str) -> Result<String, RenewError> + Send + Sync>;

fn real_ask() -> Ask {
    Box::new(renew)
}

/// A link folder for machine `id` as enrollment leaves it (a short path: socket paths are limited).
fn link_folder(rig: &Rig, id: &str, addr: SocketAddr, tag: &str) -> PathBuf {
    let m = rig.machines.iter().find(|m| m.id == id).unwrap();
    let d = std::env::temp_dir().join(format!("al{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    for (name, text) in [
        ("node-id", id.to_string()),
        ("url", format!("wss://localhost:{}", addr.port())),
        ("ca.pem", rig.ca_pem.clone()),
        ("cert.pem", m.cert_pem.clone()),
        ("key.pem", m.key_pem.clone()),
    ] {
        std::fs::write(d.join(name), text).unwrap();
    }
    d
}

fn renewer(dir: &Path, ask: Ask, clock_offset_secs: i64) -> Arc<Renewer> {
    let r = Renewer::with_parts(
        dir.to_path_buf(),
        fast(),
        || new_device_request().ok(),
        || Ok(Box::new(()) as Box<dyn std::any::Any + Send>),
        ask,
        move || {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64
                + clock_offset_secs
        },
        Arc::new(Real),
    );
    r.set_hub_offers(true);
    r
}

fn file(d: &Path, n: &str) -> String {
    std::fs::read_to_string(d.join(n)).unwrap_or_default()
}

fn one_pair(d: &Path) -> bool {
    pair_matches(&file(d, "cert.pem"), &file(d, "key.pem"))
}

fn count(rig: &Rig, action: &str, node: &str) -> usize {
    rig.audit_rows()
        .iter()
        .filter(|(a, n)| a == action && n == node)
        .count()
}

fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(30);
    while !ok() {
        assert!(Instant::now() < end, "{what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// ADR-0085: a machine whose certificate lasts 3 seconds renews it by itself against a real hub, stays connected past the original
/// end, and its next connection (made after the old certificate ended) uses the new certificate; the old pair is given up only after
/// that connection was accepted.
#[test]
fn a_node_renews_its_own_short_certificate_and_stays_connected_past_the_original_end() {
    let rig = rig(
        "auto",
        None,
        ROOM_TO_DIAL,
        &[("m", true), ("control", false)],
    );
    let addr = rig.listen_with(rig.acceptor.clone());
    let dir = link_folder(&rig, "m", addr, "auto");
    let original = file(&dir, "cert.pem");
    let r = renewer(&dir, real_ask(), 0);
    r.set_hub_offers(false);
    let link = |cert: &str, key: &str| HubLink {
        mesh_out: None,
        url: format!("wss://localhost:{}", addr.port()),
        ca_pem: rig.ca_pem.clone(),
        cert_pem: cert.to_string(),
        key_pem: key.to_string(),
        commands: None,
        grants: None,
        mesh: None,
        devices: None,
        state_dir: None,
        certs: Some(Arc::clone(&r) as Arc<dyn CertSource>),
        spawner: None,
    };
    let m = rig.machines.iter().find(|m| m.id == "m").unwrap();
    let timer = link_renewal::spawn(Arc::clone(&r)).unwrap();
    let sink1 = sink(&tmp("auto-sink1"), "m");
    let first = spawn(Arc::clone(&sink1), link(&m.cert_pem, &m.key_pem)).unwrap();
    rig.wait_online("m");
    // The node learns from the hub's welcome that it handles renewals, then renews with no one asking.
    // The last step of storing an answer is dropping the request: after it the folder is the new pair whole.
    wait_for("the certificate was never renewed", || {
        file(&dir, "cert.pem") != original && !dir.join(PENDING).exists()
    });
    assert!(one_pair(&dir), "the folder is not one pair");
    let new_cert = file(&dir, "cert.pem");
    assert!(
        cert_not_after_secs(&new_cert).unwrap() > rig.ends_at + 24 * 3600,
        "the new certificate lasts a normal lifetime"
    );
    assert!(
        !dir.join(PENDING).exists(),
        "the request stays after the answer was stored"
    );
    assert_eq!(count(&rig, "enroll.renew", "m"), 1);

    rig.wait_past_expiry();
    assert!(rig.online("m"), "an open connection is cut at the end");

    // The first connection ends; a new one made with the original (ended) certificate in the link would be refused, so what connects
    // is the pair the renewal stored, which the uplink reads before each dial.
    drop(first);
    wait_for("m never went offline", || !rig.online("m"));
    let sink2 = sink(&tmp("auto-sink2"), "m");
    let _second = spawn(Arc::clone(&sink2), link(&m.cert_pem, &m.key_pem)).unwrap();
    rig.wait_online("m");
    // Control: the original certificate itself is refused after its end (the hub accepts only the new one).
    assert!(cert_not_after_secs(&m.cert_pem).unwrap() < now_secs());
    // The pair before the renewal is given up once a connection with the new certificate was accepted.
    wait_for("the old pair was kept after the new one connected", || {
        !dir.join(PREVIOUS).exists()
    });
    assert_eq!(file(&dir, "cert.pem"), new_cert);
    // Still exactly one renewal for the machine, however many looks the timer made.
    assert_eq!(count(&rig, "enroll.renew", "m"), 1);
    drop(timer);
    let _ = std::fs::remove_dir_all(&dir);
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// ADR-0085: the answer is lost (the process stops after the hub issued the certificate, before it was stored). The next start sends the
/// SAME persisted request, the hub answers with the same certificate, and the machine ends with exactly one new certificate.
#[test]
fn a_restart_after_a_lost_answer_sends_the_same_request_and_gets_the_same_certificate() {
    let rig = rig("lost", None, ROOM_TO_DIAL, &[("m", true)]);
    let addr = rig.listen_with(rig.acceptor.clone());
    let dir = link_folder(&rig, "m", addr, "lost");
    let original = (file(&dir, "cert.pem"), file(&dir, "key.pem"));
    let answers = Arc::new(std::sync::Mutex::new(Vec::<(String, String)>::new()));

    // First life: the hub issues the certificate, the answer never reaches the files.
    let a = Arc::clone(&answers);
    let lost: Ask = Box::new(move |link, csr| {
        let cert = renew(link, csr)?;
        a.lock().unwrap().push((csr.to_string(), cert));
        Err(RenewError::Link(LinkError::Closed))
    });
    let first = renewer(&dir, lost, 0);
    assert_eq!(first.tick().0, Outcome::Failed);
    assert!(dir.join(PENDING).exists(), "the request was not kept");
    assert_eq!((file(&dir, "cert.pem"), file(&dir, "key.pem")), original);
    drop(first);

    // Second life (a restart): the same request, the same certificate.
    let a = Arc::clone(&answers);
    let again: Ask = Box::new(move |link, csr| {
        let cert = renew(link, csr)?;
        a.lock().unwrap().push((csr.to_string(), cert.clone()));
        Ok(cert)
    });
    let second = renewer(&dir, again, 0);
    assert_eq!(second.tick().0, Outcome::Renewed);
    let answers = answers.lock().unwrap();
    assert_eq!(answers.len(), 2);
    assert_eq!(answers[0].0, answers[1].0, "a restart made another request");
    assert_eq!(
        answers[0].1, answers[1].1,
        "the hub issued another certificate"
    );
    assert_eq!(file(&dir, "cert.pem").trim(), answers[1].1.trim());
    assert!(one_pair(&dir));
    assert!(!dir.join(PENDING).exists());
    // The hub issued one certificate and sent it again; it did not issue a second.
    assert_eq!(count(&rig, "enroll.renew", "m"), 1);
    assert_eq!(count(&rig, "enroll.renew_resent", "m"), 1);

    // And the machine connects with it after the original ended.
    rig.wait_past_expiry();
    let (_s, _h) = rig.dial_with("m", &file(&dir, "cert.pem"), &file(&dir, "key.pem"), addr);
    rig.wait_online("m");
    let _ = std::fs::remove_dir_all(&dir);
}

/// ADR-0085: what the hub refuses, over the wire, and what the node does about it.
#[test]
fn refusals_from_a_real_hub_are_remembered_never_looped_and_the_node_keeps_its_certificate() {
    // Certificates last 10 minutes here (15 with the five before their start) so the test is not racing a 3 second end; the node's own
    // clock is 400 seconds ahead, which puts them inside the last third.
    let rig = rig(
        "refuse",
        None,
        Duration::from_secs(600),
        &[
            ("sup", true),
            ("freq", true),
            ("gone", true),
            ("control", true),
        ],
    );
    let addr = rig.listen_with(rig.acceptor.clone());

    // Superseded: a copy of the machine's certificate renewed already; this one is no longer the newest.
    let dir = link_folder(&rig, "sup", addr, "sup");
    rig.renew(addr, "sup").expect("the other holder renews");
    let asked = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let a = Arc::clone(&asked);
    let counted: Ask = Box::new(move |l, c| {
        a.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        renew(l, c)
    });
    let r = renewer(&dir, counted, 400);
    assert_eq!(r.tick().0, Outcome::Refused("superseded".into()));
    for _ in 0..3 {
        r.tick();
    }
    assert_eq!(
        asked.load(std::sync::atomic::Ordering::Relaxed),
        1,
        "asked in a loop"
    );
    let ends = validity(&file(&dir, "cert.pem")).unwrap().1;
    assert_eq!(link_renewal::state(&dir, ends).unwrap().code, "superseded");
    assert!(r.log_lines().iter().any(|l| l.contains("SECURITY")));
    assert!(!dir.join(PENDING).exists());
    assert!(one_pair(&dir));

    // Too frequent: the machine renewed a moment ago; its new certificate is due again only on the node's own clock.
    let dir = link_folder(&rig, "freq", addr, "freq");
    let first = renewer(&dir, real_ask(), 400);
    assert_eq!(first.tick().0, Outcome::Renewed);
    let r = renewer(&dir, real_ask(), 65 * 86_400);
    let (outcome, wait) = r.tick();
    assert_eq!(outcome, Outcome::Refused("too_frequent".into()));
    assert_eq!(
        wait,
        Duration::from_secs(3600),
        "waits the hour the hub asked for"
    );
    let ends = validity(&file(&dir, "cert.pem")).unwrap().1;
    let s = link_renewal::state(&dir, ends).unwrap();
    assert_eq!(s.code, "too_frequent");
    assert!(s.retry_at > now_secs() + 3000);
    assert!(one_pair(&dir));
    // The request is kept for the hour that follows, not made anew.
    assert!(dir.join(PENDING).exists());

    // A revoked machine's connection is refused before any command: the node sees a failed connection, waits longer each time and
    // keeps what it has. Control: the same hub, in the same test, renews another machine.
    let dir = link_folder(&rig, "gone", addr, "gone");
    agora_hub::enroll::Enrollment::new(rig.state.hub.store())
        .revoke("gone", agora_hub::server::now_ms())
        .unwrap();
    let r = renewer(&dir, real_ask(), 400);
    let waits: Vec<u128> = (0..4)
        .map(|_| {
            let (o, w) = r.tick();
            assert_eq!(o, Outcome::Failed);
            w.as_millis()
        })
        .collect();
    assert_eq!(waits, [100, 200, 400, 400]);
    assert!(one_pair(&dir));
    assert_eq!(
        file(&dir, "cert.pem"),
        rig.machines
            .iter()
            .find(|m| m.id == "gone")
            .unwrap()
            .cert_pem
    );
    let control = link_folder(&rig, "control", addr, "ctl");
    let c = renewer(&control, real_ask(), 400);
    assert_eq!(c.tick().0, Outcome::Renewed);
    for d in ["sup", "freq", "gone", "ctl"] {
        let _ = std::fs::remove_dir_all(
            std::env::temp_dir().join(format!("al{}-{d}", std::process::id())),
        );
    }
}

/// ADR-0085: an answer that is not for this machine, not for this key, or not from the pinned CA is rejected and the pair in use is
/// kept whole (the hub being what it says is checked by the machine, not assumed).
#[test]
fn an_answer_for_another_node_or_key_or_ca_is_rejected_and_the_pair_in_use_is_kept() {
    let rig = rig(
        "wrong",
        None,
        Duration::from_secs(600),
        &[("m", true), ("control", false)],
    );
    let addr = rig.listen_with(rig.acceptor.clone());
    let ca = rig.state.ca.as_ref().unwrap();
    let foreign = Ca::generate("another hub").unwrap();
    for (name, how) in [("node", 0), ("key", 1), ("ca", 2)] {
        let dir = link_folder(&rig, "m", addr, &format!("wrong-{name}"));
        let before = (file(&dir, "cert.pem"), file(&dir, "key.pem"));
        let ask: Ask = {
            let (ca, foreign) = (
                Ca::from_pem(ca.cert_pem(), &ca.key_pem()).unwrap(),
                Ca::from_pem(foreign.cert_pem(), &foreign.key_pem()).unwrap(),
            );
            Box::new(move |_, csr| {
                Ok(match how {
                    0 => ca.issue_node("control", csr).unwrap().cert_pem,
                    1 => {
                        ca.issue_node("m", &new_device_request().unwrap().1)
                            .unwrap()
                            .cert_pem
                    }
                    _ => foreign.issue_node("m", csr).unwrap().cert_pem,
                })
            })
        };
        let r = renewer(&dir, ask, 400);
        assert_eq!(r.tick().0, Outcome::Failed, "{name}");
        assert_eq!(
            (file(&dir, "cert.pem"), file(&dir, "key.pem")),
            before,
            "{name}"
        );
        assert!(one_pair(&dir), "{name}");
        assert!(!dir.join(PREVIOUS).exists(), "{name}");
        assert!(
            dir.join(PENDING).exists(),
            "{name}: the request is kept for the same answer again"
        );
        for line in r.log_lines() {
            assert!(!line.contains("PRIVATE"), "{line}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
    assert_eq!(count(&rig, "enroll.renew", "m"), 0);
}
