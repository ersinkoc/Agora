//! PROOF about certificate lifetime (docs/adr/0085-machine-and-hub-certificates-renew-themselves.md). Production certificates last 90 days; these
//! tests make the lifetime a few seconds (a test-only setting of the CA) and wait past the end, to show what the product does:
//! a machine whose certificate has ended is refused on its next connection, and a hub whose own certificate has ended is
//! refused by every machine that dials it. Nothing renews either. If a renewal is ever built, these tests are the ones that
//! must change, on purpose.
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
        let code = enrollment.create_code(server::now_ms()).unwrap();
        let (key_pem, csr) = new_device_request().unwrap();
        let issuer = if *short {
            Ca::from_pem(ca.cert_pem(), &ca.key_pem())
                .unwrap()
                .with_lifetime_for_tests(node_life)
        } else {
            Ca::from_pem(ca.cert_pem(), &ca.key_pem()).unwrap()
        };
        let issued = enrollment
            .redeem(&issuer, &code, id, &csr, server::now_ms())
            .unwrap();
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

    fn dial(
        &self,
        id: &str,
        addr: SocketAddr,
    ) -> (Arc<EventSink>, agora_node::uplink_client::UplinkHandle) {
        let m = self.machines.iter().find(|m| m.id == id).unwrap();
        let link = HubLink {
            mesh_out: None,
            url: format!("wss://localhost:{}", addr.port()),
            ca_pem: self.ca_pem.clone(),
            cert_pem: m.cert_pem.clone(),
            key_pem: m.key_pem.clone(),
            commands: None,
            grants: None,
            mesh: None,
            devices: None,
            state_dir: None,
        };
        let sink = sink(&tmp(&format!("{}-{id}", self.tag)), id);
        let handle = spawn(Arc::clone(&sink), link).unwrap();
        (sink, handle)
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

/// A machine certificate that has ended is refused on the next connection, and nothing gives the machine a new one.
#[test]
fn proof_a_machine_whose_certificate_ended_cannot_connect_and_nothing_renews_it() {
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

/// A hub whose own certificate has ended is refused by the machines that dial it: this is what a hub kept running for 90 days
/// serves to every new connection, because it issues the certificate once, when it starts.
#[test]
fn proof_a_hub_whose_own_certificate_ended_is_refused_by_new_connections() {
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

    // A restarted hub (a new server certificate from the same CA) is accepted again: restarting is the only cure today.
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
