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
use agora_node::daemon::{Announce, EventSink};
use agora_node::uplink_client::{spawn, HubLink};
use agora_protocol::handshake::Support;

/// Accepts one client token (the tests' only signed-in user).
struct NoClients;

impl ClientAuth for NoClients {
    fn check(&self, t: &str) -> bool {
        t == "client-token"
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
            capabilities: vec![
                "terminal.raw".into(),
                "workspace.directory".into(),
                "directory.list".into(),
            ],
            required: vec![],
        },
        ca: Some(ca),
        lockout: Arc::new(Lockout::new()),
        auth: Box::new(NoClients),
        assets: None,
        sessions: None,
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
    /// A certificate for one more node (the hub's own CA signs it, as enrollment would).
    fn enroll(&self, node_id: &str) -> (String, String) {
        let (key_pem, csr) = new_device_request().unwrap();
        let mut e = Enrollment::new(self.state.hub.store());
        let code = e.create_code(server::now_ms()).unwrap();
        let ca = self.state.ca.as_ref().unwrap();
        let issued = e
            .redeem(ca, &code, node_id, &csr, server::now_ms())
            .unwrap();
        (issued.cert_pem, key_pem)
    }

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
            mesh_out: None,
            url: format!("wss://localhost:{}", addr.port()),
            ca_pem: self.ca_pem.clone(),
            cert_pem: self.cert_pem.clone(),
            key_pem: self.key_pem.clone(),
            commands: None,
            grants: None,
            mesh: None,
        }
    }

    /// A signed-in client's WebSocket over TLS, trusting only the hub's CA.
    fn client(
        &self,
        addr: SocketAddr,
    ) -> agora_protocol::ws::Conn<tokio_rustls::client::TlsStream<tokio::net::TcpStream>> {
        use rustls::pki_types::pem::PemObject;
        use rustls::pki_types::{CertificateDer, ServerName};
        self.rt.block_on(async {
            let mut roots = rustls::RootCertStore::empty();
            for c in CertificateDer::pem_slice_iter(self.ca_pem.as_bytes()) {
                roots.add(c.unwrap()).unwrap();
            }
            let cfg = rustls::ClientConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
            let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
            let tls = tokio_rustls::TlsConnector::from(Arc::new(cfg))
                .connect(ServerName::try_from("localhost").unwrap(), tcp)
                .await
                .unwrap();
            agora_protocol::ws::connect_with_subprotocol_token(
                tls,
                &format!("wss://localhost:{}/", addr.port()),
                "client-token",
            )
            .await
            .unwrap()
        })
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
    sink.node_resources("laptop", 3, 3, 0);
    wait("the resource event", || {
        hub.state.hub.view().0.nodes["laptop"].agents_running == 3
    });
}

#[test]
fn the_terminal_console_gets_the_snapshot_of_a_node_and_draws_it() {
    use agora_tui::client::{Hub as ConsoleHub, Session, Update};
    let hub = hubside("laptop");
    let addr = hub.listen("127.0.0.1:0");
    let sink = sink_with_agent(&tmp("console"), "laptop");
    let _link = spawn(Arc::clone(&sink), hub.link(addr)).unwrap();
    wait("the node to appear", || {
        hub.state.hub.view().0.agents.contains_key("laptop/agent-1")
    });

    let console = ConsoleHub {
        addr: addr.to_string(),
        ca_pem: hub.ca_pem.clone(),
    };
    // A token the hub does not know is refused; the tests' signed-in one gets the snapshot first.
    assert!(
        Session::connect(&console, "wrong").is_err() || {
            let mut s = Session::connect(&console, "wrong").unwrap();
            matches!(s.next(Duration::from_secs(2)), Update::Closed)
        }
    );
    let mut session = Session::connect(&console, "client-token").unwrap();
    let end = Instant::now() + Duration::from_secs(20);
    let snapshot = loop {
        match session.next(Duration::from_millis(200)) {
            Update::Snapshot(s) => break *s,
            Update::Closed => panic!("the hub closed the console's connection"),
            _ => assert!(Instant::now() < end, "no snapshot"),
        }
    };
    assert!(snapshot
        .agents
        .iter()
        .any(|a| a.agent_id == "laptop/agent-1"));
    // The hub answers the cost question itself, with no machine in between (nothing is priced here, so zero).
    assert_eq!(session.usage_total(), Ok(0));
    let catalog = agora_i18n::Catalog::english();
    let text = |id: &str, args: &[(&str, agora_i18n::Arg)]| {
        catalog.render(id, args).unwrap_or_else(|| id.to_string())
    };
    let screen = agora_tui::render::render(&snapshot, false, false, "", "", 0, &text).join("\n");
    assert!(
        screen.contains("agent-1") && screen.contains("1 of 1 machines online"),
        "{screen}"
    );
}

#[test]
fn the_terminal_console_answers_an_approval_with_its_own_key_through_the_hub() {
    use agora_node::control::Client;
    use agora_protocol::v1::{self, command_request::Command as Cmd};
    use agora_tui::client::{Hub as ConsoleHub, Session, Update};
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let hub = hubside("node-test");
    let addr = hub.listen("127.0.0.1:0");
    let dir = tmp("console-approve");
    let repo = dir.join("myproject");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let manifests = dir.join("manifests");
    std::fs::create_dir_all(&manifests).unwrap();
    let mut fake = std::path::PathBuf::from(env!("CARGO_BIN_EXE_agora"));
    fake.pop();
    fake.push(format!("agora-fake-agent{}", std::env::consts::EXE_SUFFIX));
    // Built once per test binary, even when present: a binary left from an older checkout may lack newer flags.
    static FRESH: std::sync::Once = std::sync::Once::new();
    FRESH.call_once(|| {
        assert!(std::process::Command::new(
            std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())
        )
        .args(["build", "-q", "-p", "agora-fake-agent"])
        .status()
        .unwrap()
        .success());
    });
    std::fs::write(
        manifests.join("ad.toml"),
        format!("kind = \"adapted\"\ndisplay_name = \"A\"\nversion = \"1\"\nmin_engine = 1\n[adapter]\ncommand = [{:?}, \"--adapter\"]\n", fake.to_string_lossy()),
    )
    .unwrap();

    // This console's key; the machine's owner trusts it the way `agora node trust` does.
    let (device, _) = agora_tui::device::Device::generate().unwrap();
    let data = dir.join("data");
    std::fs::create_dir_all(&data).unwrap();
    agora_node::approvals::trust_device(
        &data.join(agora_node::approvals::TRUSTED_DEVICES_FILE),
        &device.id,
        &device.public_key_hex(),
    )
    .unwrap();
    let cfg = agora_node::daemon::NodeConfig {
        node_id: "node-test".into(),
        socket: dir.join("c.sock"),
        data_dir: data,
        worktree_root: dir.join("wt"),
        manifests_dir: manifests,
        projects: vec![repo],
        limits: Default::default(),
    };
    let node = agora_node::daemon::Node::start(&cfg).unwrap();
    let mut link = hub.link(addr);
    link.commands = Some(cfg.socket.clone());
    let _uplink = spawn(Arc::clone(&node.events), link).unwrap();
    wait("the node to be online in the hub", || {
        hub.state
            .hub
            .view()
            .0
            .nodes
            .get("node-test")
            .is_some_and(|n| n.online)
    });

    // The adapter agent asks for something; the machine now waits for a signed answer.
    let mut local = Client::connect(&cfg.socket).unwrap();
    let started = local
        .call(Cmd::AgentStart(v1::AgentStart {
            kind: "adapted".into(),
            ..Default::default()
        }))
        .unwrap()
        .unwrap();
    let Some(v1::command_ok::Result::AgentStart(s)) = started.result else {
        panic!("not started");
    };
    local
        .call(Cmd::AgentPrompt(v1::AgentPrompt {
            agent_id: s.agent_id,
            text: "ask".into(),
        }))
        .unwrap()
        .unwrap();

    let console = ConsoleHub {
        addr: addr.to_string(),
        ca_pem: hub.ca_pem.clone(),
    };
    let end = Instant::now() + Duration::from_secs(20);
    let (mut session, snapshot) = loop {
        let mut session = Session::connect(&console, "client-token").unwrap();
        if let Update::Snapshot(s) = session.next(Duration::from_secs(2)) {
            if !s.approvals.is_empty() {
                break (session, *s);
            }
        }
        assert!(
            Instant::now() < end,
            "the approval never reached the console"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    assert_eq!(node.approvals.pending_count(), 1);

    // A device the machine does not trust is told so; the request keeps waiting.
    let (stranger, _) = agora_tui::device::Device::generate().unwrap();
    assert_eq!(
        session.resolve_first_approval(&snapshot, &stranger, true),
        Err("approval.signature_invalid".to_string())
    );
    assert_eq!(node.approvals.pending_count(), 1);
    // This console's own key is accepted.
    assert_eq!(
        session.resolve_first_approval(&snapshot, &device, true),
        Ok(true)
    );
    wait("the machine to take the answer", || {
        node.approvals.pending_count() == 0
    });
    drop(node);
}

#[test]
fn the_terminal_console_opens_an_agents_terminal_through_the_hub_types_into_it_and_leaves() {
    use agora_node::control::Client;
    use agora_protocol::v1::{self, command_request::Command as Cmd};
    use agora_tui::client::{Hub as ConsoleHub, Session, TermEvent, Update};
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let hub = hubside("laptop");
    let addr = hub.listen("127.0.0.1:0");
    let dir = tmp("console-term");
    let repo = dir.join("myproject");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let manifests = dir.join("manifests");
    std::fs::create_dir_all(&manifests).unwrap();
    std::fs::write(
        manifests.join("shell.toml"),
        "kind = \"shell\"\ndisplay_name = \"Shell\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"sh\"]\n",
    )
    .unwrap();
    let cfg = agora_node::daemon::NodeConfig {
        node_id: "laptop".into(),
        socket: dir.join("c.sock"),
        data_dir: dir.join("data"),
        worktree_root: dir.join("wt"),
        manifests_dir: manifests,
        projects: vec![repo],
        limits: Default::default(),
    };
    let node = agora_node::daemon::Node::start(&cfg).unwrap();
    let mut link = hub.link(addr);
    link.commands = Some(cfg.socket.clone());
    let _uplink = spawn(Arc::clone(&node.events), link).unwrap();
    let mut local = Client::connect(&cfg.socket).unwrap();
    let started = local
        .call(Cmd::AgentStart(v1::AgentStart {
            kind: "shell".into(),
            ..Default::default()
        }))
        .unwrap()
        .unwrap();
    let Some(v1::command_ok::Result::AgentStart(s)) = started.result else {
        panic!("not started");
    };

    let console = ConsoleHub {
        addr: addr.to_string(),
        ca_pem: hub.ca_pem.clone(),
    };
    let end = Instant::now() + Duration::from_secs(20);
    let (mut session, snapshot) = loop {
        let mut session = Session::connect(&console, "client-token").unwrap();
        if let Update::Snapshot(sn) = session.next(Duration::from_secs(2)) {
            if sn
                .terminals
                .iter()
                .any(|t| t.agent_id == format!("laptop/{}", s.agent_id))
            {
                break (session, *sn);
            }
        }
        assert!(
            Instant::now() < end,
            "the terminal never reached the console"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    // The machine's own id for it is `t-<agent id>`, which is how the console names it.
    assert!(snapshot
        .terminals
        .iter()
        .any(|t| t.terminal_id.ends_with(&format!("t-{}", s.agent_id))));
    let terminal = format!("t-{}", s.agent_id);
    let stream = session.terminal_open("laptop", &terminal).unwrap();
    session.terminal_input("laptop", &terminal, b"echo seen-$((20+22))\n");
    let mut out = Vec::new();
    let end = Instant::now() + Duration::from_secs(20);
    while !String::from_utf8_lossy(&out).contains("seen-42") {
        assert!(
            Instant::now() < end,
            "no output: {}",
            String::from_utf8_lossy(&out)
        );
        match session.terminal_next(stream, Duration::from_millis(200)) {
            TermEvent::Data(b) | TermEvent::Screen(b) => out.extend_from_slice(&b),
            TermEvent::Lost | TermEvent::Ended => panic!("the terminal stream ended"),
            _ => {}
        }
    }
    session.terminal_close(stream);
    drop(node);
}

#[test]
fn the_terminal_console_follows_a_chat_agents_conversation_sends_a_request_and_stops_it() {
    use agora_node::control::Client;
    use agora_protocol::v1::{self, command_request::Command as Cmd};
    use agora_tui::client::{ChatEvent, Hub as ConsoleHub, Session, Update};
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let hub = hubside("laptop");
    let addr = hub.listen("127.0.0.1:0");
    let dir = tmp("console-chat");
    let repo = dir.join("myproject");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let manifests = dir.join("manifests");
    std::fs::create_dir_all(&manifests).unwrap();
    let mut fake = std::path::PathBuf::from(env!("CARGO_BIN_EXE_agora"));
    fake.pop();
    fake.push(format!("agora-fake-agent{}", std::env::consts::EXE_SUFFIX));
    // Built once per test binary, even when present: a binary left from an older checkout may lack newer flags.
    static FRESH: std::sync::Once = std::sync::Once::new();
    FRESH.call_once(|| {
        assert!(std::process::Command::new(
            std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())
        )
        .args(["build", "-q", "-p", "agora-fake-agent"])
        .status()
        .unwrap()
        .success());
    });
    std::fs::write(
        manifests.join("ad.toml"),
        format!("kind = \"adapted\"\ndisplay_name = \"A\"\nversion = \"1\"\nmin_engine = 1\n[adapter]\ncommand = [{:?}, \"--adapter\"]\n", fake.to_string_lossy()),
    )
    .unwrap();
    let cfg = agora_node::daemon::NodeConfig {
        node_id: "laptop".into(),
        socket: dir.join("c.sock"),
        data_dir: dir.join("data"),
        worktree_root: dir.join("wt"),
        manifests_dir: manifests,
        projects: vec![repo],
        limits: Default::default(),
    };
    let node = agora_node::daemon::Node::start(&cfg).unwrap();
    let mut link = hub.link(addr);
    link.commands = Some(cfg.socket.clone());
    let _uplink = spawn(Arc::clone(&node.events), link).unwrap();
    let mut local = Client::connect(&cfg.socket).unwrap();
    let started = local
        .call(Cmd::AgentStart(v1::AgentStart {
            kind: "adapted".into(),
            ..Default::default()
        }))
        .unwrap()
        .unwrap();
    let Some(v1::command_ok::Result::AgentStart(s)) = started.result else {
        panic!("not started");
    };

    let console = ConsoleHub {
        addr: addr.to_string(),
        ca_pem: hub.ca_pem.clone(),
    };
    let end = Instant::now() + Duration::from_secs(20);
    let mut session = loop {
        let mut session = Session::connect(&console, "client-token").unwrap();
        if let Update::Snapshot(sn) = session.next(Duration::from_secs(2)) {
            if sn
                .agents
                .iter()
                .any(|a| a.agent_id == format!("laptop/{}", s.agent_id))
            {
                break session;
            }
        }
        assert!(Instant::now() < end, "the agent never reached the console");
        std::thread::sleep(Duration::from_millis(100));
    };
    let stream = session.chat_open("laptop", &s.agent_id).unwrap();
    session
        .prompt("laptop", &s.agent_id, "hello from the console")
        .unwrap();
    let end = Instant::now() + Duration::from_secs(20);
    let mut said = String::new();
    while !said.contains("hello from the console") {
        assert!(
            Instant::now() < end,
            "the request never showed in the conversation: {said}"
        );
        match session.chat_next(stream, Duration::from_millis(200)) {
            ChatEvent::Lines(entries, _) => entries.iter().for_each(|e| said.push_str(&e.text)),
            ChatEvent::Lost | ChatEvent::Ended => panic!("the chat stream ended"),
            ChatEvent::Quiet => {}
        }
    }
    session.terminal_close(stream);
    session.stop_agent("laptop", &s.agent_id).unwrap();
    // Starting another one from the console: the machine lists what it can start, the project comes from the snapshot.
    let kinds = session.agent_kinds("laptop").unwrap();
    assert!(kinds.iter().any(|(k, _)| k == "adapted"), "{kinds:?}");
    let project = loop {
        let mut again = Session::connect(&console, "client-token").unwrap();
        if let Update::Snapshot(sn) = again.next(Duration::from_secs(2)) {
            if let Some(c) = sn.checkouts.first() {
                break c.project_id.clone();
            }
        }
        assert!(Instant::now() < end + Duration::from_secs(20), "no project");
    };
    let second = session.start_agent("laptop", &project, "adapted").unwrap();
    assert!(!second.is_empty() && second != s.agent_id, "{second}");
    drop(node);
}

#[test]
fn the_terminal_console_gives_a_machine_a_signed_grant_when_its_policy_asks_for_one() {
    use agora_node::control::Client;
    use agora_protocol::v1::{self, command_request::Command as Cmd};
    use agora_tui::client::{Hub as ConsoleHub, Session, Update};
    use std::time::Duration as D;
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let hub = hubside("laptop");
    let addr = hub.listen("127.0.0.1:0");
    let dir = tmp("console-grant");
    let repo = dir.join("myproject");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let manifests = dir.join("manifests");
    std::fs::create_dir_all(&manifests).unwrap();
    std::fs::write(
        manifests.join("shell.toml"),
        "kind = \"shell\"\ndisplay_name = \"Shell\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"sh\"]\n",
    )
    .unwrap();
    let (device, _) = agora_tui::device::Device::generate().unwrap();
    let data = dir.join("data");
    std::fs::create_dir_all(&data).unwrap();
    agora_node::approvals::trust_device(
        &data.join(agora_node::approvals::TRUSTED_DEVICES_FILE),
        &device.id,
        &device.public_key_hex(),
    )
    .unwrap();
    // The owner's policy: stopping an agent through the hub needs a grant.
    std::fs::write(data.join("policy"), "signed_commands = agent.stop\n").unwrap();
    let cfg = agora_node::daemon::NodeConfig {
        node_id: "laptop".into(),
        socket: dir.join("c.sock"),
        data_dir: data,
        worktree_root: dir.join("wt"),
        manifests_dir: manifests,
        projects: vec![repo],
        limits: Default::default(),
    };
    let node = agora_node::daemon::Node::start(&cfg).unwrap();
    let mut link = hub.link(addr);
    link.commands = Some(cfg.socket.clone());
    link.grants = Some(Arc::clone(&node.grants));
    let _uplink = spawn(Arc::clone(&node.events), link).unwrap();
    let mut local = Client::connect(&cfg.socket).unwrap();
    let started = local
        .call(Cmd::AgentStart(v1::AgentStart {
            kind: "shell".into(),
            ..Default::default()
        }))
        .unwrap()
        .unwrap();
    let Some(v1::command_ok::Result::AgentStart(s)) = started.result else {
        panic!("not started");
    };

    let console = ConsoleHub {
        addr: addr.to_string(),
        ca_pem: hub.ca_pem.clone(),
    };
    let end = Instant::now() + D::from_secs(20);
    let mut session = loop {
        let mut session = Session::connect(&console, "client-token").unwrap();
        if let Update::Snapshot(sn) = session.next(D::from_secs(2)) {
            if sn
                .agents
                .iter()
                .any(|a| a.agent_id == format!("laptop/{}", s.agent_id))
            {
                break session;
            }
        }
        assert!(Instant::now() < end, "the agent never reached the console");
        std::thread::sleep(D::from_millis(100));
    };
    // Refused until the owner allows it; a device the machine does not trust cannot allow it.
    assert_eq!(
        session.stop_agent("laptop", &s.agent_id),
        Err("approval.grant_required".to_string())
    );
    let scope = format!("agent:{}", s.agent_id);
    let commands = ["agent.stop", "agent.prompt", "terminal.input"];
    let (stranger, _) = agora_tui::device::Device::generate().unwrap();
    assert_eq!(
        session.grant("laptop", &stranger, &scope, &commands, D::from_secs(3600)),
        Err("approval.signature_invalid".to_string())
    );
    assert_eq!(
        session.stop_agent("laptop", &s.agent_id),
        Err("approval.grant_required".to_string())
    );
    // This console's own key can; then the same stop goes through.
    assert_eq!(
        session.grant("laptop", &device, &scope, &commands, D::from_secs(3600)),
        Ok(())
    );
    assert_eq!(session.stop_agent("laptop", &s.agent_id), Ok(()));
    drop(node);
}

#[test]
fn the_terminal_console_learns_that_typing_was_refused_for_want_of_a_grant() {
    use agora_node::control::Client;
    use agora_protocol::v1::{self, command_request::Command as Cmd};
    use agora_tui::client::{Hub as ConsoleHub, Session, Update};
    use std::time::Duration as D;
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let hub = hubside("laptop");
    let addr = hub.listen("127.0.0.1:0");
    let dir = tmp("console-typing-grant");
    let repo = dir.join("myproject");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let manifests = dir.join("manifests");
    std::fs::create_dir_all(&manifests).unwrap();
    std::fs::write(
        manifests.join("shell.toml"),
        "kind = \"shell\"\ndisplay_name = \"Shell\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"sh\"]\n",
    )
    .unwrap();
    let (device, _) = agora_tui::device::Device::generate().unwrap();
    let data = dir.join("data");
    std::fs::create_dir_all(&data).unwrap();
    agora_node::approvals::trust_device(
        &data.join(agora_node::approvals::TRUSTED_DEVICES_FILE),
        &device.id,
        &device.public_key_hex(),
    )
    .unwrap();
    // The owner's policy: typing into a terminal through the hub needs a grant.
    std::fs::write(data.join("policy"), "signed_commands = terminal.input\n").unwrap();
    let cfg = agora_node::daemon::NodeConfig {
        node_id: "laptop".into(),
        socket: dir.join("c.sock"),
        data_dir: data,
        worktree_root: dir.join("wt"),
        manifests_dir: manifests,
        projects: vec![repo],
        limits: Default::default(),
    };
    let node = agora_node::daemon::Node::start(&cfg).unwrap();
    let mut link = hub.link(addr);
    link.commands = Some(cfg.socket.clone());
    link.grants = Some(Arc::clone(&node.grants));
    let _uplink = spawn(Arc::clone(&node.events), link).unwrap();
    let mut local = Client::connect(&cfg.socket).unwrap();
    let started = local
        .call(Cmd::AgentStart(v1::AgentStart {
            kind: "shell".into(),
            ..Default::default()
        }))
        .unwrap()
        .unwrap();
    let Some(v1::command_ok::Result::AgentStart(s)) = started.result else {
        panic!("not started");
    };

    let console = ConsoleHub {
        addr: addr.to_string(),
        ca_pem: hub.ca_pem.clone(),
    };
    let end = Instant::now() + D::from_secs(20);
    let mut session = loop {
        let mut session = Session::connect(&console, "client-token").unwrap();
        if let Update::Snapshot(sn) = session.next(D::from_secs(2)) {
            if sn
                .agents
                .iter()
                .any(|a| a.agent_id == format!("laptop/{}", s.agent_id))
            {
                break session;
            }
        }
        assert!(Instant::now() < end, "the agent never reached the console");
        std::thread::sleep(D::from_millis(100));
    };
    let terminal = format!("t-{}", s.agent_id);
    let stream = session.terminal_open("laptop", &terminal).unwrap();
    // Typing is sent without waiting for an answer; the refusal is noticed all the same.
    session.terminal_input("laptop", &terminal, b"echo first-try\n");
    let end = Instant::now() + D::from_secs(10);
    while !session.take_grant_wanted() {
        let _ = session.terminal_next(stream, D::from_millis(100));
        assert!(Instant::now() < end, "the refusal was never noticed");
    }
    // After a grant the same typing reaches the shell.
    let scope = format!("agent:{}", s.agent_id);
    let commands = ["agent.stop", "agent.prompt", "terminal.input"];
    assert_eq!(
        session.grant("laptop", &device, &scope, &commands, D::from_secs(3600)),
        Ok(())
    );
    session.terminal_input("laptop", &terminal, b"echo second-try\n");
    let mut seen = String::new();
    let end = Instant::now() + D::from_secs(10);
    while !seen.contains("second-try") {
        if let agora_tui::client::TermEvent::Data(b) | agora_tui::client::TermEvent::Screen(b) =
            session.terminal_next(stream, D::from_millis(100))
        {
            seen.push_str(&String::from_utf8_lossy(&b));
        }
        assert!(
            Instant::now() < end,
            "typing after the grant never arrived: {seen}"
        );
    }
    assert!(!session.take_grant_wanted());
    drop(node);
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
    sink.node_resources("server", 1, 1, 0);
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

/// Found by running a real install over ssh: a machine that comes back under a known name with an empty data directory starts
/// numbering at 1, but the hub already acknowledged those numbers for the earlier life. Its first events were dropped as duplicates.
#[test]
fn a_node_that_returns_under_a_known_name_with_an_empty_data_dir_is_still_heard() {
    let hub = hubside("vps");
    let addr = hub.listen("127.0.0.1:0");
    // First life: five or more events reach the hub and are acknowledged.
    let first = sink_with_agent(&tmp("returns-1"), "vps");
    first.node_resources("vps", 1, 1, 0);
    first.node_resources("vps", 2, 2, 0);
    let link = spawn(Arc::clone(&first), hub.link(addr)).unwrap();
    wait("the first life to be acknowledged", || {
        first.acked() >= 6 && first.acked() == first.next_seq() - 1
    });
    let acked_by_hub = first.acked();
    drop(link);

    // Second life: same name and certificate, a fresh data directory; it records two events and knows how to announce itself.
    let second = Arc::new(EventSink::open(&tmp("returns-2"), "vps").unwrap());
    second.node_online("vps");
    second.checkout_discovered("vps", "reborn", Path::new("/work/reborn"));
    assert!(
        second.next_seq() <= acked_by_hub,
        "the test needs the hub to be ahead"
    );
    second.set_announcer(Box::new(|ev, how| {
        if how == Announce::Restart {
            ev.node_online("vps");
        }
        ev.checkout_discovered("vps", "reborn", Path::new("/work/reborn"));
    }));
    let _link = spawn(Arc::clone(&second), hub.link(addr)).unwrap();
    wait("the returning node's checkout to show up", || {
        hub.state
            .hub
            .view()
            .0
            .checkouts
            .values()
            .any(|(node, project, _)| node == "vps" && project == "reborn")
    });
    wait("everything to be acknowledged", || {
        second.acked() == second.next_seq() - 1
    });
}

/// ADR-0054 over the real transport: a node whose size cap dropped events the hub never saw used to end every connection in
/// `Stale` and retry forever. Now it says `SnapshotRequired`, the hub takes its oldest held event as the new base, records the
/// lost numbers (store and audit trail) and shows what the node announced as its present.
#[test]
fn a_node_that_lost_unacknowledged_events_resyncs_over_the_link() {
    let hub = hubside("burst");
    let addr = hub.listen("127.0.0.1:0");
    let sink = Arc::new(EventSink::open_with_cap(&tmp("burst"), "burst", 1200).unwrap());
    // Its start (`NodeOnline`, the first event) is among what the cap dropped, so the announcement is led by `NodeOnline` (ADR-0055).
    sink.set_announcer(Box::new(|ev, how| {
        if how == Announce::Restart {
            ev.node_online("burst");
        }
        ev.agent_launched(
            &AgentId::new("agent-now"),
            "proj",
            "burst",
            Path::new("/work/proj"),
            Path::new("/work/now"),
            "agent/now",
            "shell",
            true,
        );
    }));
    // Offline long enough that the cap dropped events the hub never saw.
    sink.node_online("burst");
    for i in 0..60 {
        sink.node_resources("burst", i, 0, 0);
    }
    let oldest = sink.oldest_retained();
    assert!(oldest > 1);
    let _link = spawn(Arc::clone(&sink), hub.link(addr)).unwrap();
    wait("the announced agent to show up", || {
        hub.state
            .hub
            .view()
            .0
            .agents
            .contains_key("burst/agent-now")
    });
    wait("everything to be acknowledged", || {
        sink.acked() == sink.next_seq() - 1
    });
    let store = hub.state.hub.store();
    let resyncs = store.resyncs().unwrap();
    assert_eq!(resyncs.len(), 1);
    let (producer, from, to, _) = &resyncs[0];
    assert_eq!((producer.as_str(), *from), ("burst", 1));
    // At least what was dropped offline; more if the announcement itself pushed older events out under the cap.
    assert!(*to >= oldest - 1, "{to} vs {oldest}");
    let audit = agora_hub::audit::Audit::new(store)
        .list(0, None, 50)
        .unwrap();
    assert!(audit.iter().any(|e| e.action == "node.resynced"
        && e.who == "hub"
        && e.node == "burst"
        && e.outcome == format!("1..={to}")));
    // From then on the link is ordinary.
    sink.node_resources("burst", 7, 7, 0);
    wait("a newer event", || {
        hub.state.hub.view().0.nodes["burst"].agents_running == 7
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

#[test]
fn a_signed_in_client_runs_a_command_on_a_node_through_the_hub() {
    // Routed commands run on the node's local socket, which this platform does not have yet.
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    use agora_protocol::frame::{kinds, STREAM_COMMANDS, STREAM_EVENTS};
    use agora_protocol::v1::{self, command_request, command_response};
    use agora_protocol::Frame;
    use prost::Message;

    let hub = hubside("laptop");
    let addr = hub.listen("127.0.0.1:0");
    // A real node: its control socket is what routed commands run on.
    let dir = tmp("routed");
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = agora_node::daemon::NodeConfig {
        node_id: "laptop".into(),
        socket: dir.join("c.sock"),
        data_dir: dir.join("data"),
        worktree_root: dir.join("wt"),
        manifests_dir: dir.join("manifests"),
        projects: vec![],
        limits: Default::default(),
    };
    let node = agora_node::daemon::Node::start(&cfg).unwrap();
    let mut link = hub.link(addr);
    link.commands = Some(cfg.socket.clone());
    let _uplink = spawn(Arc::clone(&node.events), link).unwrap();
    wait("the node to be online in the hub", || {
        hub.state
            .hub
            .view()
            .0
            .nodes
            .get("laptop")
            .is_some_and(|n| n.online)
    });

    let mut client = hub.client(addr);
    let f = |stream, kind, payload| Frame {
        stream_id: stream,
        kind,
        seq: None,
        payload,
    };
    let answer = hub.rt.block_on(async {
        client
            .send(&f(
                STREAM_EVENTS,
                kinds::SUBSCRIBE,
                v1::Subscribe::default().encode_to_vec(),
            ))
            .await
            .unwrap();
        let req = v1::CommandRequest {
            request_id: "c1".into(),
            idempotency_key: String::new(),
            target_node: "laptop".into(),
            command: Some(command_request::Command::AgentList(v1::AgentList::default())),
        };
        client
            .send(&f(STREAM_COMMANDS, 0, req.encode_to_vec()))
            .await
            .unwrap();
        loop {
            let fr = tokio::time::timeout(Duration::from_secs(20), client.recv())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            if (fr.stream_id, fr.kind) == (STREAM_COMMANDS, 1) {
                break v1::CommandResponse::decode(fr.payload.as_slice()).unwrap();
            }
        }
    });
    assert_eq!(answer.request_id, "c1");
    assert!(
        matches!(answer.result, Some(command_response::Result::Ok(_))),
        "{answer:?}"
    );
    drop(node);
}

async fn next_frame<S>(c: &mut agora_protocol::ws::Conn<S>) -> agora_protocol::Frame
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    tokio::time::timeout(Duration::from_secs(20), c.recv())
        .await
        .expect("timed out")
        .unwrap()
        .expect("closed")
}

fn git(dir: &Path, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

#[test]
fn a_clients_terminal_is_relayed_through_the_hub_with_input_and_close() {
    // Routed commands run on the node's local socket, which this platform does not have yet.
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    use agora_protocol::frame::{kinds, STREAM_CLOSE, STREAM_COMMANDS, STREAM_EVENTS};
    use agora_protocol::v1::{self, command_ok, command_request, command_response};
    use agora_protocol::Frame;
    use prost::Message;

    let hub = hubside("laptop");
    let addr = hub.listen("127.0.0.1:0");
    let dir = tmp("relay");
    let repo = dir.join("myproject");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    std::fs::create_dir_all(dir.join("manifests")).unwrap();
    std::fs::write(
        dir.join("manifests/shell.toml"),
        "kind = \"shell\"\ndisplay_name = \"Shell\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"sh\"]\n",
    )
    .unwrap();
    let cfg = agora_node::daemon::NodeConfig {
        node_id: "laptop".into(),
        socket: dir.join("c.sock"),
        data_dir: dir.join("data"),
        worktree_root: dir.join("wt"),
        manifests_dir: dir.join("manifests"),
        projects: vec![repo.clone()],
        limits: Default::default(),
    };
    let node = agora_node::daemon::Node::start(&cfg).unwrap();
    let mut link = hub.link(addr);
    link.commands = Some(cfg.socket.clone());
    let _uplink = spawn(Arc::clone(&node.events), link).unwrap();
    wait("the node to be online in the hub", || {
        hub.state
            .hub
            .view()
            .0
            .nodes
            .get("laptop")
            .is_some_and(|n| n.online)
    });

    let mut client = hub.client(addr);
    let f = |stream, kind, payload| Frame {
        stream_id: stream,
        kind,
        seq: None,
        payload,
    };
    let routed = |id: &str, command| {
        f(
            STREAM_COMMANDS,
            0,
            v1::CommandRequest {
                request_id: id.into(),
                idempotency_key: String::new(),
                target_node: "laptop".into(),
                command: Some(command),
            }
            .encode_to_vec(),
        )
    };
    hub.rt.block_on(async {
        client
            .send(&f(
                STREAM_EVENTS,
                kinds::SUBSCRIBE,
                v1::Subscribe::default().encode_to_vec(),
            ))
            .await
            .unwrap();
        // Start a shell agent on the node.
        client
            .send(&routed(
                "start",
                command_request::Command::AgentStart(v1::AgentStart {
                    project_id: "myproject".into(),
                    kind: "shell".into(),
                    ..Default::default()
                }),
            ))
            .await
            .unwrap();
        let terminal = loop {
            let fr = next_frame(&mut client).await;
            if (fr.stream_id, fr.kind) == (STREAM_COMMANDS, 1) {
                let r = v1::CommandResponse::decode(fr.payload.as_slice()).unwrap();
                match r.result {
                    Some(command_response::Result::Ok(v1::CommandOk {
                        result: Some(command_ok::Result::AgentStart(s)),
                    })) => break s.terminal_id,
                    other => panic!("start failed: {other:?}"),
                }
            }
        };
        // Subscribe on a stream of the client's choosing.
        client
            .send(&Frame {
                stream_id: 5,
                kind: 0,
                seq: None,
                payload: v1::CommandRequest {
                    request_id: "sub".into(),
                    idempotency_key: String::new(),
                    target_node: "laptop".into(),
                    command: Some(command_request::Command::TerminalSubscribe(
                        v1::TerminalSubscribe {
                            terminal_id: terminal.clone(),
                            format: String::new(),
                        },
                    )),
                }
                .encode_to_vec(),
            })
            .await
            .unwrap();
        let mut saw_snapshot = false;
        let mut got_answer = false;
        while !(saw_snapshot && got_answer) {
            let fr = next_frame(&mut client).await;
            if fr.stream_id == 5 && fr.kind == 1 {
                saw_snapshot = true;
            } else if (fr.stream_id, fr.kind) == (STREAM_COMMANDS, 1) {
                let r = v1::CommandResponse::decode(fr.payload.as_slice()).unwrap();
                if r.request_id == "sub" {
                    assert!(
                        matches!(r.result, Some(command_response::Result::Ok(_))),
                        "{r:?}"
                    );
                    got_answer = true;
                }
            }
        }
        // Type into it (an ordinary routed command) and watch the output arrive on the stream.
        client
            .send(&routed(
                "in",
                command_request::Command::TerminalInput(v1::TerminalInput {
                    terminal_id: terminal.clone(),
                    data: b"echo relayed-$((20+22))\n".to_vec(),
                }),
            ))
            .await
            .unwrap();
        let mut out = Vec::new();
        while !String::from_utf8_lossy(&out).contains("relayed-42") {
            let fr = next_frame(&mut client).await;
            if fr.stream_id == 5 && fr.kind == 0 {
                out.extend_from_slice(&fr.payload);
            }
        }
        // Keystrokes arrive as one command each; the node must run them in the order they were sent, or typing scrambles.
        out.clear();
        for (i, b) in b"echo ordered-0123456789abcdefghijklmnopqrstuv\n"
            .iter()
            .enumerate()
        {
            client
                .send(&routed(
                    &format!("k{i}"),
                    command_request::Command::TerminalInput(v1::TerminalInput {
                        terminal_id: terminal.clone(),
                        data: vec![*b],
                    }),
                ))
                .await
                .unwrap();
        }
        while !String::from_utf8_lossy(&out).contains("ordered-0123456789abcdefghijklmnopqrstuv") {
            let fr = next_frame(&mut client).await;
            if fr.stream_id == 5 && fr.kind == 0 {
                out.extend_from_slice(&fr.payload);
            }
        }
        // Closing the stream tells the node to stop writing on it.
        client.send(&f(5, STREAM_CLOSE, Vec::new())).await.unwrap();
    });
    drop(node);
}

/// One JSON-RPC call on an agent's mesh socket; returns the `result` object.
fn mcp(
    conn: &mut agora_platform::LocalStream,
    id: u64,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    use std::io::{BufRead, BufReader, Write};
    let line = serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
    writeln!(conn, "{line}").unwrap();
    let mut reader = BufReader::new(conn.try_clone().unwrap());
    let mut out = String::new();
    reader.read_line(&mut out).unwrap();
    serde_json::from_str::<serde_json::Value>(&out).unwrap()["result"].clone()
}

#[test]
fn an_agent_messages_an_agent_on_another_node_through_the_hub_and_a_failure_comes_back_as_a_notice()
{
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    use agora_core::AgentId;
    use agora_mesh::broker::AgentInfo;
    use agora_node::uplink_client::MeshOut;
    let hub = hubside("alpha");
    let addr = hub.listen("127.0.0.1:0");
    let start = |name: &str, cert: String, key: String| {
        let dir = tmp(name);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = agora_node::daemon::NodeConfig {
            node_id: name.into(),
            socket: dir.join("c.sock"),
            data_dir: dir.join("data"),
            worktree_root: dir.join("wt"),
            manifests_dir: dir.join("manifests"),
            projects: vec![],
            limits: Default::default(),
        };
        let node = agora_node::daemon::Node::start(&cfg).unwrap();
        let out = MeshOut::new();
        node.mesh.set_remote(out.clone());
        let mut link = hub.link(addr);
        (link.cert_pem, link.key_pem) = (cert, key);
        link.commands = Some(cfg.socket.clone());
        link.mesh_out = Some(out);
        let up = spawn(Arc::clone(&node.events), link).unwrap();
        let agent = AgentId::new(if name == "alpha" { "a1" } else { "b1" });
        let socket = node
            .mesh
            .register(AgentInfo {
                id: agent.clone(),
                kind: "shell".into(),
                node: name.into(),
                project: "p".into(),
                label: String::new(),
                worktree: String::new(),
                state: "idle".into(),
            })
            .unwrap();
        (node, up, agent, socket)
    };
    let (cert, key) = (hub.enroll("alpha"), hub.enroll("beta"));
    let (a, _ua, a1, a_sock) = start("alpha", cert.0, cert.1);
    let (b, _ub, b1, _b_sock) = start("beta", key.0, key.1);
    for n in ["alpha", "beta"] {
        wait("a node to be online", || {
            hub.state
                .hub
                .view()
                .0
                .nodes
                .get(n)
                .is_some_and(|x| x.online)
        });
    }

    let mut conn = agora_platform::local_connect(&a_sock).unwrap();
    mcp(
        &mut conn,
        1,
        "initialize",
        serde_json::json!({"protocolVersion": "2025-03-26"}),
    );
    let call = |conn: &mut agora_platform::LocalStream, id, to: &str, body: &str| {
        mcp(
            conn,
            id,
            "tools/call",
            serde_json::json!({"name": "send_message", "arguments": {"to": to, "body": body, "refs": ["src/auth.rs", "commit:abc123"]}}),
        )
    };
    let sent = call(&mut conn, 2, "beta/b1", "hello from alpha");
    assert_ne!(sent["isError"], true, "{sent}");

    // It arrives in b1's inbox, from alpha/a1: the hub vouched for "alpha".
    wait("the message to arrive on the other node", || {
        !b.mesh
            .mesh()
            .lock()
            .unwrap()
            .inbox(&b1, None, 10)
            .unwrap()
            .is_empty()
    });
    let got = b.mesh.mesh().lock().unwrap().inbox(&b1, None, 10).unwrap();
    assert_eq!(
        (
            got[0].body.as_str(),
            got[0].provenance.from.as_str(),
            got[0].provenance.node.as_str()
        ),
        ("hello from alpha", "alpha/a1", "alpha")
    );
    // Its refs mean something on alpha only: they arrive with alpha's name in front.
    assert_eq!(got[0].refs, ["alpha:src/auth.rs", "alpha:commit:abc123"]);

    // A node the hub does not know, and an agent the target does not have: the sender is told.
    call(&mut conn, 3, "nowhere/x", "lost");
    call(&mut conn, 4, "beta/ghost", "lost");
    wait("both failures to come back", || {
        a.mesh
            .mesh()
            .lock()
            .unwrap()
            .inbox(&a1, None, 10)
            .unwrap()
            .len()
            == 2
    });
    let notices = a.mesh.mesh().lock().unwrap().inbox(&a1, None, 10).unwrap();
    assert!(notices.iter().all(|m| m.kind == "delivery_failed"));
    let codes: Vec<&str> = notices.iter().map(|m| m.body.as_str()).collect();
    assert!(
        codes.contains(&"node.unknown") && codes.contains(&"mesh.recipient_unknown"),
        "{codes:?}"
    );
}

/// ADR-0039: two nodes share a project, so tasks, leases and handoffs are decided on the hub's board.
#[test]
fn agents_on_two_nodes_share_a_board_claims_leases_and_handoffs_and_it_fails_closed_without_the_hub(
) {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    use agora_mesh::broker::AgentInfo;
    use agora_node::uplink_client::MeshOut;
    use serde_json::{json, Value};
    let hub = hubside("alpha");
    let addr = hub.listen("127.0.0.1:0");
    let start = |name: &str, agent: &str, project: &str, cert: String, key: String| {
        let dir = tmp(&format!("board-{name}"));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = agora_node::daemon::NodeConfig {
            node_id: name.into(),
            socket: dir.join("c.sock"),
            data_dir: dir.join("data"),
            worktree_root: dir.join("wt"),
            manifests_dir: dir.join("manifests"),
            projects: vec![],
            limits: Default::default(),
        };
        let node = agora_node::daemon::Node::start(&cfg).unwrap();
        let out = MeshOut::new();
        node.mesh.set_remote(out.clone());
        let mut link = hub.link(addr);
        (link.cert_pem, link.key_pem) = (cert, key);
        link.commands = Some(cfg.socket.clone());
        link.mesh_out = Some(out);
        let up = spawn(Arc::clone(&node.events), link).unwrap();
        node.events
            .checkout_discovered(name, project, Path::new("/work/p"));
        let id = AgentId::new(agent);
        node.events.agent_launched(
            &id,
            project,
            name,
            Path::new("/work/p"),
            Path::new("/work/wt"),
            "agent/x",
            "shell",
            true,
        );
        let socket = node
            .mesh
            .register(AgentInfo {
                id: id.clone(),
                kind: "shell".into(),
                node: name.into(),
                project: project.into(),
                label: String::new(),
                worktree: String::new(),
                state: "idle".into(),
            })
            .unwrap();
        let mut conn = agora_platform::local_connect(&socket).unwrap();
        mcp(
            &mut conn,
            1,
            "initialize",
            json!({"protocolVersion": "2025-03-26"}),
        );
        (node, up, id, conn)
    };
    let (ca, cb) = (hub.enroll("alpha"), hub.enroll("beta"));
    let (a, ua, _a1, mut ac) = start("alpha", "a1", "p", ca.0.clone(), ca.1.clone());
    let (b, _ub, b1, mut bc) = start("beta", "b1", "p", cb.0, cb.1);
    // A project only alpha has: it never reaches the board.
    let (_solo, _us, _s1, mut sc) = {
        let (c2, k2) = hub.enroll("solo");
        start("solo", "s1", "only-here", c2, k2)
    };
    for (node, agent) in [
        ("alpha", "alpha/a1"),
        ("beta", "beta/b1"),
        ("solo", "solo/s1"),
    ] {
        wait(
            &format!("{node} and {agent} to reach the hub: {:?}", {
                let (v, _) = hub.state.hub.view();
                (
                    v.nodes.keys().cloned().collect::<Vec<_>>(),
                    v.agents.keys().cloned().collect::<Vec<_>>(),
                )
            }),
            || {
                let (v, _) = hub.state.hub.view();
                v.nodes.get(node).is_some_and(|n| n.online) && v.agents.contains_key(agent)
            },
        );
    }
    let call = |c: &mut agora_platform::LocalStream, name: &str, args: Value| -> (Value, bool) {
        let r = mcp(c, 9, "tools/call", json!({"name": name, "arguments": args}));
        let text = r["content"][0]["text"]
            .as_str()
            .unwrap_or("null")
            .to_string();
        (
            serde_json::from_str(&text).unwrap_or(Value::Null),
            r["isError"] == true,
        )
    };

    // A task made on alpha is on beta's board; only one of two claims wins.
    let (t, err) = call(
        &mut ac,
        "task_create",
        json!({"title": "port the parser", "depends_on": []}),
    );
    assert!(!err, "{t}");
    let tid = t["task_id"].as_str().unwrap().to_string();
    let (list, _) = call(&mut bc, "task_list", json!({}));
    assert_eq!(list[0]["title"], "port the parser", "{list}");
    let (_, first) = call(&mut bc, "task_claim", json!({"task_id": tid}));
    let (lost, second) = call(&mut ac, "task_claim", json!({"task_id": tid}));
    assert!(!first && second, "{lost}");
    assert_eq!(lost["error"]["code"], "task.not_open");

    // Leases are exclusive across nodes, and name who holds the path.
    let (l, err) = call(
        &mut ac,
        "lease_acquire",
        json!({"paths": ["src/x.rs"], "ttl_s": 60}),
    );
    assert!(!err, "{l}");
    let (held, err) = call(
        &mut bc,
        "lease_acquire",
        json!({"paths": ["src/x.rs", "src/y.rs"], "ttl_s": 60}),
    );
    assert!(err);
    assert_eq!(held["error"]["code"], "lease.held");
    assert_eq!(held["error"]["params"]["holder_agent_id"], "alpha/a1");
    // All or nothing: the failed batch did not leave src/y.rs locked.
    let (y, err) = call(
        &mut ac,
        "lease_acquire",
        json!({"paths": ["src/y.rs"], "ttl_s": 60}),
    );
    assert!(!err, "{y}");
    let (_, err) = call(&mut ac, "lease_release", json!({"lease_id": l["lease_id"]}));
    assert!(!err);
    let (_, err) = call(
        &mut bc,
        "lease_acquire",
        json!({"paths": ["src/x.rs"], "ttl_s": 60}),
    );
    assert!(!err);

    // A handoff moves the task and tells the receiver, in one step; to a node that is gone it changes nothing.
    let (_, err) = call(
        &mut bc,
        "handoff",
        json!({"to": "alpha/a1", "task_id": tid, "summary": "parser is yours", "refs": ["src/parser.rs"]}),
    );
    assert!(!err);
    let (list, _) = call(&mut ac, "task_list", json!({"status": "claimed"}));
    assert_eq!(list[0]["owner"], "alpha/a1", "{list}");
    wait("the handoff message on the receiving node", || {
        !a.mesh
            .mesh()
            .lock()
            .unwrap()
            .inbox(&AgentId::new("a1"), None, 10)
            .unwrap()
            .is_empty()
    });
    let got = a
        .mesh
        .mesh()
        .lock()
        .unwrap()
        .inbox(&AgentId::new("a1"), None, 10)
        .unwrap();
    assert_eq!(got[0].body, "parser is yours");
    assert_eq!(got[0].provenance.from.as_str(), "beta/b1");

    // A `#channel` message reaches the project's agents on the other node, from the sender, with the channel's name.
    let (_, err) = call(
        &mut ac,
        "send_message",
        json!({"to": "#dev", "body": "who has the parser?", "refs": ["src/parser.rs"]}),
    );
    assert!(!err);
    wait("the channel message on the other node", || {
        !b.mesh
            .mesh()
            .lock()
            .unwrap()
            .inbox(&b1, None, 10)
            .unwrap()
            .is_empty()
    });
    let got = b.mesh.mesh().lock().unwrap().inbox(&b1, None, 10).unwrap();
    assert_eq!(got.last().unwrap().body, "[#dev] who has the parser?");
    assert_eq!(got.last().unwrap().provenance.from.as_str(), "alpha/a1");

    // A project on one node is untouched by the board.
    let (t, err) = call(
        &mut sc,
        "task_create",
        json!({"title": "local", "depends_on": []}),
    );
    assert!(!err, "{t}");
    let (list, _) = call(&mut sc, "task_list", json!({}));
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["owner"], Value::Null);

    // `mesh.remote` turned off on a node that knows the project is shared: the board is not reachable and local state is not used.
    a.mesh.set_remote_allowed(false);
    let (e, err) = call(
        &mut ac,
        "lease_acquire",
        json!({"paths": ["src/z.rs"], "ttl_s": 60}),
    );
    assert!(err && e["error"]["code"] == "board.unavailable", "{e}");

    // Without the hub a shared project's board fails closed and says so; the agent's own tools keep working.
    drop(ua);
    let (e, err) = call(&mut ac, "task_list", json!({}));
    assert!(err, "{e}");
    assert_eq!(e["error"]["code"], "board.unavailable");
    let (_, err) = call(&mut ac, "inbox", json!({}));
    assert!(!err);
    drop((a, b, b1));
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[test]
fn a_command_the_policy_lists_needs_a_grant_signed_by_a_trusted_device_and_the_local_socket_does_not(
) {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    use agora_protocol::frame::{kinds, STREAM_COMMANDS, STREAM_EVENTS};
    use agora_protocol::v1::{self, command_ok, command_request as cr, command_response};
    use agora_protocol::{ErrorCode, Frame};
    use prost::Message;
    use ring::rand::SystemRandom;
    use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};

    let hub = hubside("laptop");
    let addr = hub.listen("127.0.0.1:0");
    let dir = tmp("grants");
    std::fs::create_dir_all(dir.join("data")).unwrap();
    let rng = SystemRandom::new();
    let key = |rng: &SystemRandom| {
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, rng).unwrap();
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), rng).unwrap()
    };
    let (owner, stranger) = (key(&rng), key(&rng));
    agora_node::approvals::trust_device(
        &dir.join("data").join("trusted-devices"),
        "phone",
        &hex(owner.public_key().as_ref()),
    )
    .unwrap();
    std::fs::write(
        dir.join("data").join("policy"),
        "signed_commands = agent.prompt, terminal.input\n",
    )
    .unwrap();
    let cfg = agora_node::daemon::NodeConfig {
        node_id: "laptop".into(),
        socket: dir.join("c.sock"),
        data_dir: dir.join("data"),
        worktree_root: dir.join("wt"),
        manifests_dir: dir.join("manifests"),
        projects: vec![],
        limits: Default::default(),
    };
    let node = agora_node::daemon::Node::start(&cfg).unwrap();
    let mut link = hub.link(addr);
    link.commands = Some(cfg.socket.clone());
    link.grants = Some(Arc::clone(&node.grants));
    let _uplink = spawn(Arc::clone(&node.events), link).unwrap();
    wait("the node to be online in the hub", || {
        hub.state
            .hub
            .view()
            .0
            .nodes
            .get("laptop")
            .is_some_and(|n| n.online)
    });

    let mut client = hub.client(addr);
    let f = |stream, kind, payload| Frame {
        stream_id: stream,
        kind,
        seq: None,
        payload,
    };
    let mut n = 0u32;
    let mut call = |cmd: cr::Command| {
        n += 1;
        let id = format!("r{n}");
        hub.rt.block_on(async {
            client
                .send(&f(
                    STREAM_EVENTS,
                    kinds::SUBSCRIBE,
                    v1::Subscribe::default().encode_to_vec(),
                ))
                .await
                .unwrap();
            let req = v1::CommandRequest {
                request_id: id.clone(),
                idempotency_key: String::new(),
                target_node: "laptop".into(),
                command: Some(cmd),
            };
            client
                .send(&f(STREAM_COMMANDS, 0, req.encode_to_vec()))
                .await
                .unwrap();
            loop {
                let fr = next_frame(&mut client).await;
                if (fr.stream_id, fr.kind) == (STREAM_COMMANDS, 1) {
                    let r = v1::CommandResponse::decode(fr.payload.as_slice()).unwrap();
                    if r.request_id == id {
                        break r;
                    }
                }
            }
        })
    };
    let prompt = || {
        cr::Command::AgentPrompt(v1::AgentPrompt {
            agent_id: "no-such-agent".into(),
            text: "hi".into(),
        })
    };
    let code = |r: &v1::CommandResponse| match &r.result {
        Some(command_response::Result::Err(e)) => e.code.clone(),
        _ => String::new(),
    };
    let grant_for = |signer: &EcdsaKeyPair, id: &str, node: &str, expires: u64| {
        let cmds = vec!["agent.prompt".to_string()];
        let msg = agora_node::grants::signed_message(node, id, "node", &cmds, expires);
        v1::GrantCreate {
            grant_id: id.into(),
            scope: "node".into(),
            commands: cmds,
            expires_at_ms: expires,
            signer_device_id: "phone".into(),
            signature: signer
                .sign(&SystemRandom::new(), &msg)
                .unwrap()
                .as_ref()
                .to_vec(),
        }
    };
    let soon = agora_node::grants::now_ms() + 3_600_000;

    // Listed and no grant: refused, and nothing ran. Reading stays free.
    assert_eq!(
        code(&call(prompt())),
        ErrorCode::ApprovalGrantRequired.as_str()
    );
    assert!(matches!(
        call(cr::Command::AgentList(v1::AgentList::default())).result,
        Some(command_response::Result::Ok(_))
    ));
    // A grant the hub (or anyone without the device key) made up, and one signed for another node, do not count.
    let forged = call(cr::Command::GrantCreate(grant_for(
        &stranger, "g-forged", "laptop", soon,
    )));
    assert_eq!(code(&forged), ErrorCode::ApprovalSignatureInvalid.as_str());
    let wrong_node = call(cr::Command::GrantCreate(grant_for(
        &owner, "g-other", "desktop", soon,
    )));
    assert_eq!(
        code(&wrong_node),
        ErrorCode::ApprovalSignatureInvalid.as_str()
    );
    assert_eq!(
        code(&call(prompt())),
        ErrorCode::ApprovalGrantRequired.as_str()
    );
    // The owner's signed grant lets it through (the agent does not exist, so the node says so instead of refusing).
    let ok = call(cr::Command::GrantCreate(grant_for(
        &owner, "g-1", "laptop", soon,
    )));
    assert!(
        matches!(ok.result, Some(command_response::Result::Ok(_))),
        "{ok:?}"
    );
    assert_ne!(
        code(&call(prompt())),
        ErrorCode::ApprovalGrantRequired.as_str()
    );
    // The same grant id again is a replay.
    let replay = call(cr::Command::GrantCreate(grant_for(
        &owner, "g-1", "laptop", soon,
    )));
    assert_eq!(code(&replay), ErrorCode::ApprovalSignatureInvalid.as_str());
    // It shows in the list and ends when revoked.
    let listed = call(cr::Command::GrantList(v1::GrantList::default()));
    let Some(command_response::Result::Ok(v1::CommandOk {
        result: Some(command_ok::Result::GrantList(l)),
    })) = listed.result
    else {
        panic!("{listed:?}");
    };
    assert_eq!(l.grants.len(), 1);
    assert_eq!(l.grants[0].grant_id, "g-1");
    call(cr::Command::GrantRevoke(v1::GrantRevoke {
        grant_id: "g-1".into(),
    }));
    assert_eq!(
        code(&call(prompt())),
        ErrorCode::ApprovalGrantRequired.as_str()
    );

    // The person at the machine uses the control socket and needs no grant.
    let mut local = agora_node::control::Client::connect(&cfg.socket).unwrap();
    let r = local.call(prompt()).unwrap();
    let e = r.unwrap_err();
    assert_ne!(e.code, ErrorCode::ApprovalGrantRequired.as_str(), "{e:?}");
    drop(node);
}

#[test]
fn the_terminal_console_removes_a_closed_agent_from_every_list_but_not_a_running_one() {
    use agora_tui::client::{Hub as ConsoleHub, Session};
    let hub = hubside("laptop");
    let addr = hub.listen("127.0.0.1:0");
    let sink = sink_with_agent(&tmp("forget"), "laptop");
    let _link = spawn(Arc::clone(&sink), hub.link(addr)).unwrap();
    wait("the node to appear", || {
        hub.state.hub.view().0.agents.contains_key("laptop/agent-1")
    });
    let console = ConsoleHub {
        addr: addr.to_string(),
        ca_pem: hub.ca_pem.clone(),
    };
    let mut session = Session::connect(&console, "client-token").unwrap();
    // Running: the hub refuses, and the agent stays.
    let error = session.forget_agent("laptop/agent-1").unwrap_err();
    assert_eq!(error.code, "agent.running");
    assert_eq!(
        error.params["agent_id"].value,
        Some(agora_protocol::v1::param_value::Value::Str(
            "laptop/agent-1".into()
        ))
    );
    assert!(hub.state.hub.view().0.agents.contains_key("laptop/agent-1"));
    // Once it has exited, `d` removes it for everyone.
    sink.record_bodies(vec![agora_protocol::v1::event::Body::AgentExited(
        agora_protocol::v1::AgentExited {
            agent_id: "agent-1".into(),
            exit_code: 0,
        },
    )]);
    wait("the agent to stop running", || {
        hub.state
            .hub
            .view()
            .0
            .agents
            .get("laptop/agent-1")
            .is_some_and(|a| !a.running)
    });
    assert_eq!(session.forget_agent("laptop/agent-1"), Ok(()));
    wait("the hub to forget the agent", || {
        hub.state
            .hub
            .view()
            .0
            .agents
            .get("laptop/agent-1")
            .is_some_and(|a| a.forgotten)
    });
    // A window that opens now does not list it.
    let mut fresh = Session::connect(&console, "client-token").unwrap();
    let end = Instant::now() + Duration::from_secs(20);
    let snapshot = loop {
        match fresh.next(Duration::from_millis(200)) {
            agora_tui::client::Update::Snapshot(s) => break *s,
            agora_tui::client::Update::Closed => panic!("the hub closed the connection"),
            _ => assert!(Instant::now() < end, "no snapshot"),
        }
    };
    assert!(!snapshot
        .agents
        .iter()
        .any(|a| a.agent_id == "laptop/agent-1"));
}

#[test]
fn the_terminal_console_picks_a_folder_on_a_machine_and_starts_an_agent_there() {
    use agora_tui::client::{Hub as ConsoleHub, Session};
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let hub = hubside("laptop");
    let addr = hub.listen("127.0.0.1:0");
    let dir = tmp("console-folder");
    let manifests = dir.join("manifests");
    std::fs::create_dir_all(&manifests).unwrap();
    std::fs::write(
        manifests.join("shell.toml"),
        "kind = \"shell\"\ndisplay_name = \"Shell\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"sh\"]\n",
    )
    .unwrap();
    // The folder to pick: outside the node's own folder, with a project inside it.
    let pick = tmp("console-folder-pick");
    std::fs::create_dir_all(pick.join("app")).unwrap();
    std::fs::create_dir_all(pick.join("notes")).unwrap();
    git(&pick.join("app"), &["init", "-q", "-b", "main"]);
    let (device, _) = agora_tui::device::Device::generate().unwrap();
    let (stranger, _) = agora_tui::device::Device::generate().unwrap();
    let data = dir.join("data");
    std::fs::create_dir_all(&data).unwrap();
    agora_node::approvals::trust_device(
        &data.join(agora_node::approvals::TRUSTED_DEVICES_FILE),
        &device.id,
        &device.public_key_hex(),
    )
    .unwrap();
    let cfg = agora_node::daemon::NodeConfig {
        node_id: "laptop".into(),
        socket: dir.join("c.sock"),
        data_dir: data,
        worktree_root: dir.join("wt"),
        manifests_dir: manifests,
        projects: vec![],
        limits: Default::default(),
    };
    let node = agora_node::daemon::Node::start(&cfg).unwrap();
    let mut link = hub.link(addr);
    link.commands = Some(cfg.socket.clone());
    link.grants = Some(Arc::clone(&node.grants));
    let _uplink = spawn(Arc::clone(&node.events), link).unwrap();
    wait("the node to be online in the hub", || {
        hub.state
            .hub
            .view()
            .0
            .nodes
            .get("laptop")
            .is_some_and(|n| n.online)
    });
    let console = ConsoleHub {
        addr: addr.to_string(),
        ca_pem: hub.ca_pem.clone(),
    };
    let mut session = Session::connect(&console, "client-token").unwrap();
    let now = || {
        u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap()
    };
    let folder = pick.to_string_lossy().into_owned();
    let sign = |d: &agora_tui::device::Device, op: &str, path: &str, kind: &str, project: &str| {
        d.authorize("laptop", op, path, kind, project, false, false, now())
            .unwrap()
    };
    // A device the machine does not trust cannot look.
    assert_eq!(
        session.list_folder("laptop", &folder, sign(&stranger, "list", &folder, "", "")),
        Err("approval.signature_invalid".to_string())
    );
    // The trusted one sees the subfolders, the project marked; one level down works the same way.
    let listed = session
        .list_folder("laptop", &folder, sign(&device, "list", &folder, "", ""))
        .unwrap();
    let names: Vec<(&str, bool)> = listed
        .entries
        .iter()
        .map(|e| (e.name.as_str(), e.is_git))
        .collect();
    assert_eq!(names, [("app", true), ("notes", false)]);
    let app = agora_tui::picker::child(&listed.path, "app");
    let inside = session
        .list_folder("laptop", &app, sign(&device, "list", &app, "", ""))
        .unwrap();
    assert!(inside.entries.is_empty(), "{inside:?}");
    // A signature for one folder does not list another.
    assert_eq!(
        session.list_folder("laptop", &folder, sign(&device, "list", &app, "", "")),
        Err("approval.signature_invalid".to_string())
    );
    // Chosen: inspected, then an agent starts there.
    let info = session
        .inspect_folder("laptop", &app, sign(&device, "inspect", &app, "", ""))
        .unwrap();
    let agent = session
        .launch_in(
            "laptop",
            agora_protocol::v1::AgentLaunch {
                path: info.path.clone(),
                project_id: info.project_id.clone(),
                kind: "shell".into(),
                separate_worktree: false,
                authorization: Some(sign(
                    &device,
                    "launch",
                    &info.path,
                    "shell",
                    &info.project_id,
                )),
                accept_identity_change: false,
            },
        )
        .unwrap();
    wait("the agent to appear in the hub", || {
        hub.state.hub.view().0.agents.contains_key(&agent)
    });
    let _ = std::fs::remove_dir_all(&pick);
}

#[test]
fn the_terminal_console_keeps_a_forget_refusals_permission_details() {
    use agora_hub::users::{Access, Role};
    use agora_tui::client::{Hub as ConsoleHub, Session};
    struct Viewer;
    impl ClientAuth for Viewer {
        fn check(&self, token: &str) -> bool {
            token == "client-token"
        }
        fn access(&self, token: &str) -> Option<Access> {
            self.check(token).then(|| Access {
                role: Role::Viewer,
                user: "viewer".into(),
                projects: None,
                session: String::new(),
            })
        }
    }
    let mut hub = hubside("laptop");
    Arc::get_mut(&mut hub.state).unwrap().auth = Box::new(Viewer);
    let addr = hub.listen("127.0.0.1:0");
    let sink = sink_with_agent(&tmp("forget-viewer"), "laptop");
    sink.record_bodies(vec![agora_protocol::v1::event::Body::AgentExited(
        agora_protocol::v1::AgentExited {
            agent_id: "agent-1".into(),
            exit_code: 0,
        },
    )]);
    let _link = spawn(Arc::clone(&sink), hub.link(addr)).unwrap();
    wait("the closed agent", || {
        hub.state
            .hub
            .view()
            .0
            .agents
            .get("laptop/agent-1")
            .is_some_and(|a| !a.running)
    });
    let console = ConsoleHub {
        addr: addr.to_string(),
        ca_pem: hub.ca_pem.clone(),
    };
    let mut session = Session::connect(&console, "client-token").unwrap();
    let error = session.forget_agent("laptop/agent-1").unwrap_err();
    assert_eq!(error.code, "auth.forbidden");
    assert_eq!(
        error.params["action"].value,
        Some(agora_protocol::v1::param_value::Value::Str(
            "AgentForget".into()
        ))
    );
    assert!(!hub.state.hub.view().0.agents["laptop/agent-1"].forgotten);
}

/// Found on macOS: a machine whose node ran once before it was enrolled has events recorded as `local` in its buffer. After
/// `agora node enroll` the node used to replay them, the hub ended the connection because they name another node than the
/// certificate, and the node saw only "connection reset", forever. Now the node sets the unlinked log aside at start (renamed, never
/// deleted), links at once under its enrolled name, and its new events reach the hub.
#[test]
fn a_node_that_ran_before_it_was_enrolled_links_and_its_new_events_arrive() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let hub = hubside("mac-tm3");
    let addr = hub.listen("127.0.0.1:0");
    let dir = tmp("before-enroll");
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = |id: &str| agora_node::daemon::NodeConfig {
        node_id: id.into(),
        socket: dir.join("c.sock"),
        data_dir: dir.join("data"),
        worktree_root: dir.join("wt"),
        manifests_dir: dir.join("manifests"),
        projects: vec![],
        limits: Default::default(),
    };
    // Not enrolled yet: the node runs as `local` and records events nobody acknowledges.
    {
        let node = agora_node::daemon::Node::start(&cfg("local")).unwrap();
        node.events
            .checkout_discovered("local", "proj", Path::new("/work/proj"));
        node.events.node_resources("local", 1, 1, 0);
        assert!(node.events.next_seq() > 2);
    }
    // Enrolled, started again under the certificate's name.
    let node = agora_node::daemon::Node::start(&cfg("mac-tm3")).unwrap();
    assert_eq!(
        node.events_set_aside.as_ref().map(|s| s.old_id.as_str()),
        Some("local"),
        "the node says it set the unlinked events aside"
    );
    let _link = spawn(Arc::clone(&node.events), hub.link(addr)).unwrap();
    wait("the enrolled node to be online in the hub", || {
        hub.state
            .hub
            .view()
            .0
            .nodes
            .get("mac-tm3")
            .is_some_and(|n| n.online)
    });
    node.events.node_resources("mac-tm3", 4, 4, 0);
    wait("a new event to arrive", || {
        hub.state.hub.view().0.nodes["mac-tm3"].agents_running == 4
    });
    wait("everything to be acknowledged", || {
        node.events.acked() == node.events.next_seq() - 1
    });
    assert!(!hub.state.hub.view().0.nodes.contains_key("local"));
    // The old events are kept beside the new log, never deleted.
    let kept: Vec<String> = std::fs::read_dir(dir.join("data").join("events"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("events.local.") && n.ends_with(".log"))
        .collect();
    assert_eq!(kept.len(), 1, "{kept:?}");
}

/// When the hub ends a node's connection because an event names another node, it says why with a stable code before it closes,
/// and the node's link reports that code instead of a bare reset.
#[test]
fn the_hub_says_why_it_refuses_events_that_name_another_node() {
    let hub = hubside("laptop");
    let addr = hub.listen("127.0.0.1:0");
    // A buffer written under another name, sent over the certificate of `laptop`.
    let sink = sink_with_agent(&tmp("mismatch"), "someone-else");
    let link = hub.link(addr);
    let stop = std::sync::atomic::AtomicBool::new(false);
    let r = hub
        .rt
        .block_on(agora_node::uplink_client::run_once(&link, &sink, &stop));
    match r {
        Err(agora_node::uplink_client::LinkError::Refused(code)) => {
            assert_eq!(code, "protocol.producer_mismatch");
        }
        other => panic!("expected a refusal with a code, got {other:?}"),
    }
    assert_eq!(sink.acked(), 0);
    assert!(!hub.state.hub.view().0.nodes.contains_key("someone-else"));
}

/// Found on macOS in review: a node enrolled as one name, linked and fully acknowledged, then enrolled again under another name,
/// still replayed the old name's events (a resync from 1 reached them) and was refused forever. Any log of another producer is set
/// aside, not only `local`'s.
#[test]
fn a_node_enrolled_again_under_another_name_links_and_keeps_the_old_log() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let hub = hubside("mac-tm4");
    let addr = hub.listen("127.0.0.1:0");
    let dir = tmp("re-enroll");
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = |id: &str| agora_node::daemon::NodeConfig {
        node_id: id.into(),
        socket: dir.join("c.sock"),
        data_dir: dir.join("data"),
        worktree_root: dir.join("wt"),
        manifests_dir: dir.join("manifests"),
        projects: vec![],
        limits: Default::default(),
    };
    // First enrollment: linked, everything acknowledged.
    {
        let node = agora_node::daemon::Node::start(&cfg("mac-tm4")).unwrap();
        assert!(node.events_set_aside.is_none());
        node.events
            .checkout_discovered("mac-tm4", "proj", Path::new("/work/proj"));
        let events = Arc::clone(&node.events);
        let _link = spawn(Arc::clone(&events), hub.link(addr)).unwrap();
        // The node stops while linked, so even its last event (`NodeOffline`) is acknowledged: nothing of the old name waits.
        drop(node);
        wait("the first name to be acknowledged", || {
            events.acked() == events.next_seq() - 1 && events.acked() >= 3
        });
    }
    // Enrolled again under a new name.
    let (cert, key) = hub.enroll("mac-tm4b");
    let node = agora_node::daemon::Node::start(&cfg("mac-tm4b")).unwrap();
    assert_eq!(
        node.events_set_aside.as_ref().map(|s| s.old_id.as_str()),
        Some("mac-tm4")
    );
    let mut link = hub.link(addr);
    (link.cert_pem, link.key_pem) = (cert, key);
    let _link = spawn(Arc::clone(&node.events), link).unwrap();
    wait("the new name to be online", || {
        hub.state
            .hub
            .view()
            .0
            .nodes
            .get("mac-tm4b")
            .is_some_and(|n| n.online)
    });
    node.events.node_resources("mac-tm4b", 5, 5, 0);
    wait("a new event under the new name", || {
        hub.state.hub.view().0.nodes["mac-tm4b"].agents_running == 5
    });
    wait("everything to be acknowledged", || {
        node.events.acked() == node.events.next_seq() - 1
    });
    let names: Vec<String> = std::fs::read_dir(dir.join("data").join("events"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    let logs = names
        .iter()
        .filter(|n| n.starts_with("events.mac-tm4.") && n.ends_with(".log"))
        .count();
    let acks = names
        .iter()
        .filter(|n| n.starts_with("events.mac-tm4.") && n.ends_with(".ack"))
        .count();
    assert_eq!((logs, acks), (1, 1), "{names:?}");
    drop(_link);
    drop(node);
    // Back to not enrolled: the enrolled name's log is set aside again, never mixed with `local` events.
    let node = agora_node::daemon::Node::start(&cfg("local")).unwrap();
    assert_eq!(
        node.events_set_aside.as_ref().map(|s| s.old_id.as_str()),
        Some("mac-tm4b")
    );
}
