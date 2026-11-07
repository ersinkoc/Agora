//! A node dials a real hub over mTLS and its events appear in the hub's view: the Phase 3 vertical slice with real
//! sockets, certificates and the durable buffer, in one process. Nodes are autonomous: this also covers a hub that is
//! not there yet when the node starts.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use agora_core::AgentId;
use agora_hub::ca::{new_device_request, Ca};
use agora_hub::enroll::{Enrollment, Lockout};
use agora_hub::hub::Hub;
use agora_hub::server::{self, ClientAuth, ServerState};
use agora_hub::store::SqliteStore;
use agora_hub::tls;
use agora_node::daemon::EventSink;
use agora_node::uplink_client::{spawn, HubLink};
use agora_protocol::handshake::Support;

struct NoClients;

impl ClientAuth for NoClients {
    fn check(&self, _: &str) -> bool {
        false
    }
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("agora-dial-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

struct Hubside {
    rt: tokio::runtime::Runtime,
    state: Arc<ServerState>,
    ca_pem: String,
    cert_pem: String,
    key_pem: String,
    acceptor: tokio_rustls::TlsAcceptor,
}

fn hubside(node_id: &str) -> Hubside {
    let ca = Ca::generate("hub ca").unwrap();
    let (server_cert, server_key) = ca.issue_server(&["localhost".to_string()]).unwrap();
    let acceptor = tls::acceptor_with(&server_cert, &server_key, ca.cert_pem(), true).unwrap();
    let store = SqliteStore::open_in_memory().unwrap();
    // Enroll the node directly (the enrollment conversation itself is tested in the hub crate).
    let mut enrollment = Enrollment::new(&store);
    let code = enrollment.create_code(server::now_ms()).unwrap();
    let (key_pem, csr) = new_device_request().unwrap();
    let issued = enrollment
        .redeem(&ca, &code, node_id, &csr, server::now_ms())
        .unwrap();
    let ca_pem = ca.cert_pem().to_string();
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
    });
    Hubside {
        rt: tokio::runtime::Runtime::new().unwrap(),
        state,
        ca_pem,
        cert_pem: issued.cert_pem,
        key_pem,
        acceptor,
    }
}

impl Hubside {
    fn listen(&self, addr: &str) -> SocketAddr {
        let l = self
            .rt
            .block_on(tokio::net::TcpListener::bind(addr))
            .unwrap();
        let bound = l.local_addr().unwrap();
        self.rt.spawn(server::run(
            l,
            Arc::clone(&self.state),
            Some(self.acceptor.clone()),
        ));
        bound
    }

    fn link(&self, addr: SocketAddr) -> HubLink {
        HubLink {
            url: format!("wss://localhost:{}", addr.port()),
            ca_pem: self.ca_pem.clone(),
            cert_pem: self.cert_pem.clone(),
            key_pem: self.key_pem.clone(),
        }
    }
}

fn wait(what: &str, mut ok: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(20);
    while !ok() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn sink_with_agent(dir: &Path, node: &str) -> Arc<EventSink> {
    let sink = Arc::new(EventSink::open(dir, node).unwrap());
    sink.node_online(node);
    sink.checkout_discovered(node, "proj", Path::new("/work/proj"));
    sink.agent_launched(
        &AgentId::new("agent-1"),
        "proj",
        node,
        Path::new("/work/proj"),
        Path::new("/work/wt"),
        "agent/x",
        "shell",
        true,
    );
    sink
}

#[test]
fn a_node_dials_the_hub_and_its_agents_appear_in_the_view_then_new_events_follow() {
    let hub = hubside("laptop");
    let addr = hub.listen("127.0.0.1:0");
    let sink = sink_with_agent(&tmp("live"), "laptop");
    let _link = spawn(Arc::clone(&sink), hub.link(addr)).unwrap();

    wait("the node to appear", || {
        hub.state.hub.view().0.agents.contains_key("laptop/agent-1")
    });
    let (view, _) = hub.state.hub.view();
    assert!(view.nodes["laptop"].online);
    assert_eq!(view.worktrees.len(), 1);
    // The hub acknowledged: the node's buffer knows what is safe.
    wait("the ack to reach the node", || {
        sink.acked() == sink.next_seq() - 1
    });

    // Something new on the node shows up live.
    sink.node_resources("laptop", 3, 3);
    wait("the resource event", || {
        hub.state.hub.view().0.nodes["laptop"].agents_running == 3
    });
}

#[test]
fn a_node_keeps_working_without_the_hub_and_catches_up_when_it_appears() {
    let hub = hubside("server");
    // Reserve a port, then release it so the node's first attempts are refused.
    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = probe.local_addr().unwrap();
    drop(probe);
    let sink = sink_with_agent(&tmp("late"), "server");
    let _link = spawn(Arc::clone(&sink), hub.link(addr)).unwrap();
    // Events keep being recorded locally while nobody listens.
    sink.node_resources("server", 1, 1);
    std::thread::sleep(Duration::from_millis(800));
    assert!(hub.state.hub.view().0.nodes.is_empty());
    assert_eq!(sink.acked(), 0);

    hub.listen(&addr.to_string());
    wait("the backlog to arrive", || {
        hub.state.hub.view().0.agents.contains_key("server/agent-1")
    });
    wait("everything to be acknowledged", || {
        sink.acked() == sink.next_seq() - 1
    });
}

#[test]
fn a_certificate_the_hub_does_not_know_never_gets_in() {
    let hub = hubside("honest");
    let addr = hub.listen("127.0.0.1:0");
    // A node holding a certificate from the same CA that the hub never enrolled (not in its device registry).
    let (key_pem, csr) = new_device_request().unwrap();
    let ca = hub.state.ca.as_ref().unwrap();
    let rogue = ca.issue_node("rogue", &csr).unwrap();
    let mut link = hub.link(addr);
    link.cert_pem = rogue.cert_pem;
    link.key_pem = key_pem;
    let sink = sink_with_agent(&tmp("rogue"), "rogue");
    let _l = spawn(Arc::clone(&sink), link).unwrap();
    std::thread::sleep(Duration::from_secs(2));
    assert!(hub.state.hub.view().0.nodes.is_empty());
    assert_eq!(sink.acked(), 0);
}
