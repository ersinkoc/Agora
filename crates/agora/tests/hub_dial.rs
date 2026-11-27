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
                agora_protocol::handshake::TASK_HANDOFF_V1.into(),
                "agent.launch_settings".into(),
                // ADR-0090: Stop turn and an agent's settings from the terminal console.
                "agent.cancel_turn".into(),
                "agent.set_config".into(),
                // ADR-0108
                "agent.forget.local".into(),
                // ADR-0112: only a node whose link carries its mesh (and whose policy allows it) offers it too.
                agora_protocol::handshake::FLEET_STATUS_V1.into(),
                agora_protocol::handshake::MESH_SPAWN_REMOTE.into(),
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
            devices: None,
            state_dir: None,
            certs: None,
            spawner: None,
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
    let screen = agora_tui::render::render(&snapshot, false, false, "", "", 0, 0, &text).join("\n");
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
        resume_all: false,
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
        resume_all: false,
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
        resume_all: false,
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
    // ADR-0090 step 7: Stop turn and a setting reach the machine through the hub (the agent stays).
    session
        .cancel_turn("laptop", &s.agent_id, false)
        .expect("the machine ends the turn");
    // The fake adapter offers no settings (it does not say `config`), so the machine refuses the setting with the capability it lacks.
    let set = session
        .set_config("laptop", &s.agent_id, "effort", "high")
        .unwrap_err();
    assert_eq!(set.code, "protocol.capability_missing", "{set:?}");
    assert_eq!(
        set.params.get("capability").and_then(|p| p.value.as_ref()),
        Some(&v1::param_value::Value::Str("agent.set_config".into())),
        "{set:?}"
    );
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
        resume_all: false,
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
        resume_all: false,
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
    // A second agent, whose typing stays refused.
    let other = local
        .call(Cmd::AgentStart(v1::AgentStart {
            kind: "shell".into(),
            ..Default::default()
        }))
        .unwrap()
        .unwrap();
    let Some(v1::command_ok::Result::AgentStart(other)) = other.result else {
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
    // Each refusal names the terminal whose typing was refused (the console asks only for the agent in front).
    let refused_here =
        |r: Vec<agora_tui::client::Refused>, t: &str| r.iter().any(|r| r.is_for("laptop", t));
    // Typing is sent without waiting for an answer; the refusal is noticed all the same.
    session.terminal_input("laptop", &terminal, b"echo first-try\n");
    let end = Instant::now() + D::from_secs(10);
    while !refused_here(session.take_grant_wanted(), &terminal) {
        let _ = session.terminal_next(stream, D::from_millis(100));
        assert!(Instant::now() < end, "the refusal was never noticed");
    }
    // F1160: the console reads the link with `next` first on every turn; a refusal it reads is noticed too, so the console asks.
    session.terminal_input("laptop", &terminal, b"echo console-try\n");
    let end = Instant::now() + D::from_secs(10);
    while !refused_here(session.take_grant_wanted(), &terminal) {
        let _ = session.next(D::from_millis(100));
        assert!(
            Instant::now() < end,
            "the refusal read by the console's reader was never noticed"
        );
    }
    // Codex review of #796: a refusal that arrives while a stream opens (a tab opened in the turn the key was typed) is kept too.
    session.terminal_input("laptop", &terminal, b"echo while-opening\n");
    let reopened = session.terminal_open("laptop", &terminal).unwrap();
    session.terminal_close(reopened);
    let end = Instant::now() + D::from_secs(10);
    while !refused_here(session.take_grant_wanted(), &terminal) {
        let _ = session.next(D::from_millis(100));
        assert!(
            Instant::now() < end,
            "the refusal read while a stream opened was lost"
        );
    }
    // Codex review of #796: two terminals' refusals read before the console looks are both kept (it picks the one in front).
    // Gate: every typing answered (read), and only then looked at.
    let other_terminal = format!("t-{}", other.agent_id);
    session.terminal_input("laptop", &other_terminal, b"echo other-first\n");
    session.terminal_input("laptop", &terminal, b"echo this-second\n");
    let end = Instant::now() + D::from_secs(10);
    while session.typing_unanswered() > 0 {
        let _ = session.next(D::from_millis(100));
        assert!(Instant::now() < end, "the typing was never answered");
    }
    let both = session.take_grant_wanted();
    assert!(
        refused_here(both.clone(), &terminal) && refused_here(both.clone(), &other_terminal),
        "a refusal was replaced by another terminal's: {both:?}"
    );
    // Codex review of #796: a focus report or pointer motion the console passes on by itself is refused too, but asks nothing.
    session.terminal_report("laptop", &terminal, b"\x1b[I");
    session.terminal_report("laptop", &terminal, b"\x1b[<35;3;4M");
    let end = Instant::now() + D::from_secs(10);
    while session.typing_unanswered() > 0 {
        let _ = session.next(D::from_millis(100));
        assert!(Instant::now() < end, "the reports were never answered");
    }
    assert!(
        session.take_grant_wanted().is_empty(),
        "a refused report asked for a grant"
    );
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
    assert!(session.take_grant_wanted().is_empty());
    // Codex review of #796 (fourth): typing the machine took is told too (the console's notice goes after the next accepted key).
    let end = Instant::now() + D::from_secs(10);
    while session.typing_unanswered() > 0 {
        let _ = session.next(D::from_millis(100));
        assert!(Instant::now() < end, "the typing was never answered");
    }
    assert!(session
        .take_typing_taken()
        .iter()
        .any(|(n, t, _)| n == "laptop" && *t == terminal));
    // Codex review of #796: the other agent's typing is refused, then typing into the allowed one is taken (its echo seen): the
    // refusal still names the other agent's terminal, never the one typed into last.
    session.terminal_input("laptop", &other_terminal, b"echo refused\n");
    session.terminal_input("laptop", &terminal, b"echo third-try\n");
    let mut seen = String::new();
    let end = Instant::now() + D::from_secs(10);
    while !seen.contains("third-try") {
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
    let end = Instant::now() + D::from_secs(10);
    let refused = loop {
        if let Some(r) = session.take_grant_wanted().pop() {
            break r;
        }
        let _ = session.next(D::from_millis(100));
        assert!(
            Instant::now() < end,
            "the other agent's refusal was never noticed"
        );
    };
    assert_eq!(refused.terminal_id, other_terminal);
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
        resume_all: false,
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
        resume_all: false,
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
                            history_lines: 0,
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
            resume_all: false,
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

/// #905 and #915, through real nodes and a real hub. One message between machines is recorded by both nodes (two `MessagePosted`
/// with their own ids: the sender's names the recipient `<node>/<agent>`, the receiver's names the sender so), which the window
/// folds into one row. A message to a machine that went offline with its agent comes back to the sender as `node.offline`.
#[test]
fn a_message_between_machines_is_recorded_by_both_and_one_to_a_stopped_machine_fails_with_node_offline(
) {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    use agora_core::AgentId;
    use agora_hub::store::EventStore;
    use agora_mesh::broker::AgentInfo;
    use agora_node::uplink_client::MeshOut;
    use agora_protocol::v1::event::Body;
    use prost::Message as _;
    let hub = hubside("alpha");
    let addr = hub.listen("127.0.0.1:0");
    let start = |name: &str, cert: String, key: String| {
        let dir = tmp(&format!("offline-{name}"));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = agora_node::daemon::NodeConfig {
            resume_all: false,
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
    let (b, ub, b1, _b_sock) = start("beta", key.0, key.1);
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
    let send = |conn: &mut agora_platform::LocalStream, id, body: &str| {
        mcp(
            conn,
            id,
            "tools/call",
            serde_json::json!({"name": "send_message", "arguments": {"to": "beta/b1", "body": body}}),
        )
    };
    let sent = send(&mut conn, 2, "ping");
    assert_ne!(sent["isError"], true, "{sent}");
    wait("the message to arrive", || {
        !b.mesh
            .mesh()
            .lock()
            .unwrap()
            .inbox(&b1, None, 10)
            .unwrap()
            .is_empty()
    });
    // #905: both nodes record it; what the window gets is these two, which it shows as one row.
    let posted = || -> Vec<(String, String, String)> {
        hub.state
            .hub
            .store()
            .events_after(0, 10_000)
            .unwrap()
            .iter()
            .filter_map(|e| {
                let ev = agora_protocol::v1::Event::decode(e.payload.as_slice()).ok()?;
                match ev.body? {
                    Body::MessagePosted(m) if m.body == "ping" => {
                        Some((ev.producer_id, m.from_agent_id, m.to))
                    }
                    _ => None,
                }
            })
            .collect()
    };
    wait("both copies to reach the hub", || posted().len() == 2);
    let mut copies = posted();
    copies.sort();
    assert_eq!(
        copies,
        [
            ("alpha".into(), "a1".into(), "beta/b1".into()),
            ("beta".into(), "alpha/a1".into(), "b1".into()),
        ]
    );

    // #915: beta stops, and its agent ends with it. The hub records it offline; a message to that agent fails with a reason.
    drop(ub);
    drop(b);
    wait("beta to be offline", || {
        hub.state
            .hub
            .view()
            .0
            .nodes
            .get("beta")
            .is_some_and(|x| !x.online)
    });
    let sent = send(&mut conn, 3, "are you there");
    assert_ne!(sent["isError"], true, "{sent}");
    let id =
        serde_json::from_str::<serde_json::Value>(sent["content"][0]["text"].as_str().unwrap())
            .unwrap()["message_id"]
            .as_str()
            .unwrap()
            .to_string();
    wait("the failure to come back", || {
        !a.mesh
            .mesh()
            .lock()
            .unwrap()
            .inbox(&a1, None, 10)
            .unwrap()
            .is_empty()
    });
    let notices = a.mesh.mesh().lock().unwrap().inbox(&a1, None, 10).unwrap();
    assert_eq!(
        notices
            .iter()
            .map(|m| (m.kind, m.body.as_str(), m.reply_to.as_deref()))
            .collect::<Vec<_>>(),
        [("delivery_failed", "node.offline", Some(id.as_str()))]
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
            resume_all: false,
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

/// ADR-0109 over the real hub uplink and the node's MCP and durable inbox paths.
#[test]
fn two_fake_agents_on_different_nodes_use_the_durable_handoff() {
    use agora_node::supervisor::StartRequest;
    use agora_node::uplink_client::MeshOut;
    use serde_json::{json, Value};
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let mut fake = PathBuf::from(env!("CARGO_BIN_EXE_agora"));
    fake.pop();
    fake.push(format!("agora-fake-agent{}", std::env::consts::EXE_SUFFIX));
    assert!(
        std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
            .args(["build", "-q", "-p", "agora-fake-agent"])
            .status()
            .unwrap()
            .success()
    );
    let hub = hubside("durable-giver");
    let addr = hub.listen("127.0.0.1:0");
    let project = agora_core::ProjectId::portable("06ab78a2-d589-4494-8a79-4d505677ef3b").unwrap();
    let start = |name: &str| {
        let dir = tmp(name);
        let manifests = dir.join("manifests");
        let folder = dir.join("repo");
        std::fs::create_dir_all(&manifests).unwrap();
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            manifests.join("ad.toml"),
            format!(
                r#"kind = "adapted"
display_name = "A"
version = "1"
min_engine = 1
[adapter]
command = [{:?}, "--adapter"]
"#,
                fake.to_string_lossy(),
            ),
        )
        .unwrap();
        let cfg = agora_node::daemon::NodeConfig {
            resume_all: false,
            node_id: name.into(),
            socket: dir.join("c.sock"),
            data_dir: dir.join("data"),
            worktree_root: dir.join("wt"),
            manifests_dir: manifests,
            projects: vec![],
            limits: Default::default(),
        };
        let node = agora_node::daemon::Node::start(&cfg).unwrap();
        let out = MeshOut::new();
        node.mesh.set_remote(out.clone());
        let mut link = hub.link(addr);
        (link.cert_pem, link.key_pem) = hub.enroll(name);
        link.commands = Some(cfg.socket);
        link.mesh = Some(Arc::clone(&node.mesh));
        link.mesh_out = Some(out);
        let uplink = spawn(Arc::clone(&node.events), link).unwrap();
        node.events
            .checkout_discovered(name, &project.to_string(), &folder);
        let id = node
            .supervisor
            .lock()
            .unwrap()
            .start_in_directory(
                &StartRequest {
                    kind: "adapted".into(),
                    checkout: folder,
                    name: "handoff".into(),
                    worktree_root: cfg.worktree_root,
                    base: None,
                },
                project.clone(),
                std::time::Instant::now(),
            )
            .unwrap();
        node.events.announce();
        let mut conn = agora_platform::local_connect(&node.mesh.socket_path(&id)).unwrap();
        mcp(
            &mut conn,
            1,
            "initialize",
            json!({"protocolVersion": "2025-03-26"}),
        );
        (node, uplink, id, conn, cfg.data_dir)
    };
    let (_giver, _giver_link, giver_id, mut from, _) = start("durable-giver");
    let (_receiver, _receiver_link, receiver_id, mut to, receiver_data) = start("durable-receiver");
    let receiver = format!("durable-receiver/{}", receiver_id.as_str());
    wait("both agents on the shared board", || {
        let view = hub.state.hub.view().0;
        view.agents.contains_key(&receiver)
            && view
                .agents
                .contains_key(&format!("durable-giver/{}", giver_id.as_str()))
    });
    let call = |c: &mut agora_platform::LocalStream, tool, args| {
        let result = mcp(c, 2, "tools/call", json!({"name":tool, "arguments":args}));
        assert_ne!(result["isError"], true, "{result}");
        serde_json::from_str::<Value>(result["content"][0]["text"].as_str().unwrap()).unwrap()
    };
    let id = call(&mut from, "task_create", json!({"title":"port the parser"}))["task_id"]
        .as_str()
        .unwrap()
        .to_string();
    call(&mut from, "task_claim", json!({"task_id":id}));
    call(
        &mut from,
        "handoff",
        json!({"task_id":id, "to":receiver, "summary":"port the parser"}),
    );
    wait("the receiver durably stored the handoff", || {
        std::fs::read_dir(receiver_data.join("handoffs"))
            .unwrap()
            .flatten()
            .any(|f| f.path().extension().is_some_and(|ext| ext == "handoff"))
    });
    let inbox = call(&mut to, "inbox", json!({}));
    let entries = inbox.as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["kind"], "handoff");
    assert_eq!(entries[0]["body"], "port the parser");
    assert_eq!(
        hub.state.hub.view().0.tasks[&format!("{project}/{id}")].owner,
        receiver
    );
    wait(
        "a matching durable receipt reaches the web snapshot",
        || {
            let (view, pos) = hub.state.hub.view();
            let snapshot = server::snapshot_of(&view, pos);
            matches!(snapshot.task_handoff_deliveries.as_slice(), [delivery]
            if delivery.state() == agora_protocol::v1::TaskHandoffDeliveryState::Delivered
                && delivery.task_id == id && delivery.receiver_agent_id == receiver)
        },
    );
}

/// ADR-0112: three machines with different projects on a real hub. An agent on a solo hub sees every machine that agreed on the view
/// and the other machines' agents of other projects (`machines_list`, `agents_list {scope: "hub"}`), while the default `agents_list`
/// still lists only its own projects. Once the hub has a member it refuses (`partial: true`, this machine alone) until the owner turns
/// the view on. A machine whose policy says `mesh.fleet = false` neither offers the capability nor lists the tool, answers from itself,
/// and (Amendment A) is left out of everyone's view with its agents.
#[test]
fn an_agent_sees_every_machine_and_agent_of_its_hub_unless_people_or_its_machine_say_no() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    use agora_mesh::broker::AgentInfo;
    use agora_node::uplink_client::MeshOut;
    use serde_json::{json, Value};
    let hub = hubside("alpha");
    let addr = hub.listen("127.0.0.1:0");
    let start =
        |name: &str, agent: &str, project: &str, policy: &str, cert: String, key: String| {
            let dir = tmp(&format!("fleet-{name}"));
            std::fs::create_dir_all(dir.join("data")).unwrap();
            std::fs::write(dir.join("data").join("policy"), policy).unwrap();
            let cfg = agora_node::daemon::NodeConfig {
                resume_all: false,
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
            link.mesh = Some(Arc::clone(&node.mesh));
            let up = spawn(Arc::clone(&node.events), link).unwrap();
            let folder = format!("/work/{project}");
            node.events
                .checkout_discovered(name, project, Path::new(&folder));
            let id = AgentId::new(agent);
            node.events.agent_launched(
                &id,
                project,
                name,
                Path::new(&folder),
                Path::new(&folder),
                "",
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
                    worktree: folder.clone(),
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
            (node, up, conn, dir)
        };
    let (ca, cb, cg) = (hub.enroll("alpha"), hub.enroll("beta"), hub.enroll("gamma"));
    let (a, ua, mut ac, adir) = start("alpha", "a1", "p", "", ca.0, ca.1);
    let (b, ub, mut bc, bdir) = start("beta", "b1", "q", "mesh.fleet = false\n", cb.0, cb.1);
    let (g, ug, _gc, gdir) = start("gamma", "g1", "r", "", cg.0, cg.1);
    for (node, agent) in [
        ("alpha", "alpha/a1"),
        ("beta", "beta/b1"),
        ("gamma", "gamma/g1"),
    ] {
        wait(&format!("{node} and {agent} to reach the hub"), || {
            let (v, _) = hub.state.hub.view();
            v.nodes.get(node).is_some_and(|n| n.online) && v.agents.contains_key(agent)
        });
    }
    let names = |c: &mut agora_platform::LocalStream| -> Vec<String> {
        mcp(c, 8, "tools/list", json!({}))["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect()
    };
    let call = |c: &mut agora_platform::LocalStream, name: &str, args: Value| -> Value {
        let r = mcp(c, 9, "tools/call", json!({"name": name, "arguments": args}));
        assert_eq!(r["isError"], false, "{r}");
        serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap()
    };
    let ids = |v: &Value, key: &str| -> Vec<String> {
        let mut ids: Vec<String> = v
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r[key].as_str().unwrap().to_string())
            .collect();
        ids.sort();
        ids
    };
    // The hub's `Welcome` reached alpha's mesh: the tool is listed there.
    wait("alpha to offer machines_list", || {
        names(&mut ac).iter().any(|n| n == "machines_list")
    });

    // Solo hub: every machine that agreed, and gamma's agent in its own project. beta (`mesh.fleet = false`) is left out with its
    // agent and folder (Amendment A).
    let m = call(&mut ac, "machines_list", json!({}));
    assert_eq!(m["partial"], false, "{m}");
    assert_eq!(ids(&m["machines"], "id"), ["alpha", "gamma"], "{m}");
    assert!(
        m["machines"]
            .as_array()
            .unwrap()
            .iter()
            .all(|x| x["online"] == true),
        "{m}"
    );
    let rows = call(&mut ac, "agents_list", json!({"scope": "hub"}));
    assert_eq!(ids(&rows, "agent_id"), ["a1", "gamma/g1"], "{rows}");
    assert!(!rows.to_string().contains("/work/q"), "{rows}");
    let g1 = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["agent_id"] == "gamma/g1")
        .unwrap();
    assert_eq!(g1["project"], "r");
    assert_eq!(g1["node"], "gamma");
    assert_eq!(g1["kind"], "shell");
    assert!(
        rows.as_array()
            .unwrap()
            .iter()
            .all(|r| r["partial"] == false),
        "{rows}"
    );
    // The default scope is unchanged: alpha's projects only.
    let rows = call(&mut ac, "agents_list", json!({}));
    assert_eq!(ids(&rows, "agent_id"), ["a1"], "{rows}");

    // beta's policy says no: the tool is not listed there, and its view is its own.
    assert!(!names(&mut bc).iter().any(|n| n == "machines_list"));
    let rows = call(&mut bc, "agents_list", json!({"scope": "hub"}));
    assert_eq!(ids(&rows, "agent_id"), ["b1"], "{rows}");
    assert!(
        rows.as_array()
            .unwrap()
            .iter()
            .all(|r| r["partial"] == true),
        "{rows}"
    );
    let m = call(&mut bc, "machines_list", json!({}));
    assert_eq!(
        (ids(&m["machines"], "id"), &m["partial"]),
        (vec!["beta".to_string()], &json!(true))
    );

    // A team hub refuses: alpha answers from itself.
    agora_hub::users::Users::new(hub.state.hub.store())
        .add("Eda", agora_hub::users::Role::Member, server::now_ms())
        .unwrap();
    let m = call(&mut ac, "machines_list", json!({}));
    assert_eq!(
        (ids(&m["machines"], "id"), &m["partial"]),
        (vec!["alpha".to_string()], &json!(true))
    );
    let rows = call(&mut ac, "agents_list", json!({"scope": "hub"}));
    assert_eq!(ids(&rows, "agent_id"), ["a1"], "{rows}");
    assert!(
        rows.as_array()
            .unwrap()
            .iter()
            .all(|r| r["partial"] == true),
        "{rows}"
    );
    // ...until the owner turns it on.
    agora_hub::users::set_fleet_switch(hub.state.hub.store(), true, server::now_ms()).unwrap();
    let m = call(&mut ac, "machines_list", json!({}));
    assert_eq!(m["partial"], false, "{m}");
    assert_eq!(ids(&m["machines"], "id"), ["alpha", "gamma"]);

    drop((ac, bc, _gc, ua, ub, ug, a, b, g));
    for d in [adir, bdir, gdir] {
        let _ = std::fs::remove_dir_all(d);
    }
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
        resume_all: false,
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

/// ADR-0108: forgetting an interrupted agent in the window reaches its machine: the saved conversation is gone from the machine's
/// disk and the agent is no longer offered for Resume there, also after the node starts again.
#[test]
fn forgetting_an_agent_in_the_window_removes_its_conversation_and_resume_offer_on_its_machine() {
    // Routed commands run on the node's local socket.
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    use agora_protocol::v1::{self, event::Body};
    use agora_tui::client::{Hub as ConsoleHub, Session};
    let hub = hubside("laptop");
    let addr = hub.listen("127.0.0.1:0");
    let dir = tmp("forget-node");
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = agora_node::daemon::NodeConfig {
        resume_all: false,
        node_id: "laptop".into(),
        socket: dir.join("c.sock"),
        data_dir: dir.join("data"),
        worktree_root: dir.join("wt"),
        manifests_dir: dir.join("manifests"),
        projects: vec![],
        limits: Default::default(),
    };
    // An earlier process of the node ran agent-1 and stopped with it: interrupted, offered for Resume, its conversation on disk.
    {
        let ledger = agora_node::ledger::Ledger::open(&cfg.data_dir, 1).unwrap();
        ledger.observe(
            &Body::AgentStarted(v1::AgentStarted {
                agent_id: "agent-1".into(),
                project_id: "proj".into(),
                node_id: "laptop".into(),
                worktree_id: dir.join("wt").join("a").to_string_lossy().into_owned(),
                kind: "shell".into(),
                level: 2,
                ..Default::default()
            }),
            1,
        );
    }
    let transcripts = cfg.data_dir.join(agora_node::transcript::TRANSCRIPTS_DIR);
    std::fs::create_dir_all(&transcripts).unwrap();
    let file = transcripts.join("agent-1.jsonl");
    let earlier = transcripts.join("agent-1.earlier.jsonl");
    std::fs::write(&file, b"{\"format\":\"chat-transcript\",\"version\":1}\n").unwrap();
    std::fs::write(
        &earlier,
        b"{\"format\":\"chat-transcript\",\"version\":1}\n",
    )
    .unwrap();

    let node = agora_node::daemon::Node::start(&cfg).unwrap();
    let mut link = hub.link(addr);
    link.commands = Some(cfg.socket.clone());
    let _uplink = spawn(Arc::clone(&node.events), link).unwrap();
    wait("the interrupted agent to be listed", || {
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
    assert_eq!(session.forget_agent("laptop/agent-1"), Ok(()));
    wait("the machine to remove the conversation", || {
        !file.exists() && !earlier.exists()
    });
    wait("the machine's answer to settle the forget", || {
        hub.state.hub.forgets().pending("laptop").is_empty()
    });
    // Resume is no longer offered: the node does not hold it as interrupted any more.
    let mut local = agora_node::control::Client::connect(&cfg.socket).unwrap();
    let resumed = local
        .call(v1::command_request::Command::AgentResume(v1::AgentResume {
            agent_id: "agent-1".into(),
            ..Default::default()
        }))
        .unwrap();
    assert_eq!(
        resumed.err().map(|e| e.code).as_deref(),
        Some("agent.not_interrupted")
    );
    drop(local);
    drop(node);
    // Nor after the node starts again.
    let ledger = agora_node::ledger::Ledger::open(&cfg.data_dir, 2).unwrap();
    assert!(ledger.interrupted().is_empty());
    assert!(ledger.get("agent-1").is_none());
    assert!(!file.exists());
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
        resume_all: false,
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
        session
            .list_folder("laptop", &folder, sign(&stranger, "list", &folder, "", ""))
            .map_err(|e| e.code),
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
        session
            .list_folder("laptop", &folder, sign(&device, "list", &app, "", ""))
            .map_err(|e| e.code),
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
                launch_options: Vec::new(),
                confirmed_launch_options: Vec::new(),
                settings: Vec::new(),
            },
        )
        .unwrap();
    wait("the agent to appear in the hub", || {
        hub.state.hub.view().0.agents.contains_key(&agent)
    });
    let _ = std::fs::remove_dir_all(&pick);
}

/// ADR-0049 Amendment A (#1049), with a real node: a member granted one of the machine's projects lists its folder and a folder in it,
/// chooses one and starts an agent there (the project the machine answers for the folder is the one the grant names). Another project's
/// folder, the home folder and a link inside the project that leads to the other project are refused for the person's role (the link
/// after the machine followed it); the way up stops at the project's folder. The terminal client's picker opens there, at the project's
/// folder from the member's own snapshot (#1086), also when it does not know the role (the plain list, #1099), so nothing the member did
/// before those refusals leaves a refused row in the owner's Activity.
#[test]
fn a_member_browses_and_starts_in_a_granted_projects_folders_on_a_real_machine() {
    use agora_hub::users::{Access, Role};
    use agora_tui::client::{Hub as ConsoleHub, Session, Update};
    use std::collections::BTreeSet;
    use std::sync::Mutex;
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    struct Member(Arc<Mutex<BTreeSet<String>>>, Arc<Mutex<Role>>);
    impl ClientAuth for Member {
        fn check(&self, token: &str) -> bool {
            token == "client-token"
        }
        fn access(&self, token: &str) -> Option<Access> {
            self.check(token).then(|| Access {
                role: *self.1.lock().unwrap(),
                user: "eda".into(),
                projects: Some(self.0.lock().unwrap().clone()),
                session: String::new(),
            })
        }
    }
    let granted = Arc::new(Mutex::new(BTreeSet::new()));
    let person = Arc::new(Mutex::new(Role::Member));
    let mut hub = hubside("laptop");
    Arc::get_mut(&mut hub.state).unwrap().auth =
        Box::new(Member(Arc::clone(&granted), Arc::clone(&person)));
    let addr = hub.listen("127.0.0.1:0");
    let dir = tmp("member-folder");
    let manifests = dir.join("manifests");
    std::fs::create_dir_all(&manifests).unwrap();
    std::fs::write(
        manifests.join("shell.toml"),
        "kind = \"shell\"\ndisplay_name = \"Shell\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"sh\"]\n",
    )
    .unwrap();
    // Two projects of the machine; a link inside the member's project leads into the other one. Spelled as the machine reports
    // folders (on Windows without the verbatim `\\?\` prefix `std::fs::canonicalize` gives).
    let pick = agora_platform::canonicalize_plain(&{
        let p = tmp("member-folder-pick");
        std::fs::create_dir_all(&p).unwrap();
        p
    })
    .unwrap();
    for name in ["app", "other"] {
        let repo = pick.join(name);
        std::fs::create_dir_all(repo.join("src")).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", name]);
    }
    agora_platform::link_dir(&pick.join("other"), &pick.join("app").join("out")).unwrap();
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
        resume_all: false,
        node_id: "laptop".into(),
        socket: dir.join("c.sock"),
        data_dir: data,
        worktree_root: dir.join("wt"),
        manifests_dir: manifests,
        projects: vec![pick.join("app"), pick.join("other")],
        limits: Default::default(),
    };
    let node = agora_node::daemon::Node::start(&cfg).unwrap();
    let mut link = hub.link(addr);
    link.commands = Some(cfg.socket.clone());
    link.grants = Some(Arc::clone(&node.grants));
    let _uplink = spawn(Arc::clone(&node.events), link).unwrap();
    let project_of = |name: &str| {
        let folder = pick.join(name).to_string_lossy().into_owned();
        hub.state
            .hub
            .view()
            .0
            .checkouts
            .values()
            .find(|(_, _, path)| *path == folder)
            .map(|(_, project, _)| project.clone())
    };
    wait("both projects in the hub", || {
        project_of("app").is_some() && project_of("other").is_some()
    });
    let app_project = project_of("app").unwrap();
    granted
        .lock()
        .unwrap()
        .insert(format!("laptop/{app_project}"));
    let console = ConsoleHub {
        addr: addr.to_string(),
        ca_pem: hub.ca_pem.clone(),
    };
    let mut session = Session::connect(&console, "client-token").unwrap();
    // #1086: the terminal client's folder picker opens where the member may look, from what the hub showed them: their project's
    // folder on that machine (the other project is not in their snapshot), never the home folder.
    let end = Instant::now() + Duration::from_secs(20);
    let snapshot = loop {
        match session.next(Duration::from_millis(200)) {
            Update::Snapshot(s) => break *s,
            Update::Closed => panic!("the hub closed the member's connection"),
            _ => assert!(Instant::now() < end, "no snapshot"),
        }
    };
    // ADR-0106: the hub named the role in its first frame, so the console learns it without asking `PairStatus` (which the hub
    // would refuse and write down): the projects picker, and no pairing cards.
    assert_eq!(
        session.person_role(),
        agora_protocol::v1::PersonRole::Member
    );
    assert_eq!(
        agora_tui::role::learn_on(
            &mut session,
            agora_tui::pair::Owner::Unknown,
            agora_tui::picker::Role::Unknown
        ),
        Some((agora_tui::pair::Owner::No, agora_tui::picker::Role::Limited))
    );
    // The plain list's cost line is not asked for either (the hub answers it to the owner only).
    assert!(session.usage_total().is_err());
    let roots = agora_tui::picker::project_roots(&snapshot, "laptop");
    assert_eq!(roots, [pick.join("app").to_string_lossy().into_owned()]);
    let mut nav = agora_tui::picker::Nav::projects(roots.clone());
    let agora_tui::picker::At::Folder(start) = nav.start() else {
        panic!("one project opens in its folder")
    };
    // #1099: the plain list does not know the role; its picker opens at the same folder, not at the home folder.
    let mut unknown = agora_tui::picker::Nav::for_role(agora_tui::picker::Role::Unknown, roots);
    assert_eq!(
        unknown.start(),
        agora_tui::picker::At::Folder(start.clone())
    );
    let now = || {
        u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap()
    };
    let sign = |op: &str, path: &str, kind: &str, project: &str| {
        device
            .authorize("laptop", op, path, kind, project, false, false, now())
            .unwrap()
    };
    let app = pick.join("app").to_string_lossy().into_owned();
    let listed = session
        .list_folder("laptop", &start, sign("list", &start, "", ""))
        .unwrap();
    assert_eq!(listed.path, app);
    assert_eq!(listed.parent, "", "no way up out of the project");
    assert!(
        !nav.answered(&listed),
        "the hub's answer keeps a member's picker inside the project"
    );
    assert_eq!(nav.up(&listed), None, "the picker goes no higher either");
    assert_eq!(
        unknown.up(&listed),
        None,
        "nor does it with the role not known: the hub named no way up"
    );
    let names: Vec<&str> = listed.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["src"], "a link is never listed");
    let src = agora_tui::picker::child(&listed.path, "src");
    let inside = session
        .list_folder("laptop", &src, sign("list", &src, "", ""))
        .unwrap();
    assert_eq!(inside.parent, app);
    // Browsing as the picker does was never refused: no refused row for the member in the owner's Activity.
    let refused_rows = || {
        agora_hub::audit::Audit::new(hub.state.hub.store())
            .list(0, Some("eda"), 100)
            .unwrap()
            .into_iter()
            .filter(|e| e.outcome == "auth.forbidden")
            .map(|e| e.action)
            .collect::<Vec<_>>()
    };
    assert_eq!(refused_rows(), Vec::<String>::new());
    let refused =
        |r: Result<agora_protocol::v1::DirectoryEntries, agora_protocol::v1::ErrorInfo>| {
            let e = r.unwrap_err();
            (e.code, e.params["action"].value.clone())
        };
    let forbidden = (
        "auth.forbidden".to_string(),
        Some(agora_protocol::v1::param_value::Value::Str(
            "DirectoryList".into(),
        )),
    );
    let out = agora_tui::picker::child(&app, "out");
    let other = pick.join("other").to_string_lossy().into_owned();
    // `out` is spelled inside the granted project, so the hub lets it through; it leads into the other project (a junction on
    // Windows), so the refusal below is the machine's, after it followed the link (#1091).
    assert_eq!(
        agora_platform::canonicalize_plain(std::path::Path::new(&out)).unwrap(),
        pick.join("other")
    );
    for path in [out.as_str(), other.as_str(), ""] {
        assert_eq!(
            refused(session.list_folder("laptop", path, sign("list", path, "", ""))),
            forbidden,
            "{path:?}"
        );
    }
    // A refusal does write one: the empty Activity above is not for want of rows.
    assert!(
        refused_rows().iter().any(|a| a == "DirectoryList"),
        "{:?}",
        refused_rows()
    );
    // Chosen: the machine names the folder's project as the grant does, and the agent starts there.
    let info = session
        .inspect_folder("laptop", &src, sign("inspect", &src, "", ""))
        .unwrap();
    assert_eq!(
        (info.path.as_str(), info.project_id.as_str()),
        (src.as_str(), app_project.as_str())
    );
    let launch = |path: &str, project: &str| agora_protocol::v1::AgentLaunch {
        path: path.into(),
        project_id: project.into(),
        kind: "shell".into(),
        separate_worktree: false,
        authorization: Some(sign("launch", path, "shell", project)),
        accept_identity_change: false,
        launch_options: Vec::new(),
        confirmed_launch_options: Vec::new(),
        settings: Vec::new(),
    };
    let other_project = project_of("other").unwrap();
    assert_eq!(
        session.launch_in("laptop", launch(&other, &other_project)),
        Err("auth.forbidden".to_string())
    );
    let agent = session
        .launch_in("laptop", launch(&info.path, &info.project_id))
        .unwrap();
    wait("the agent to appear in the hub", || {
        hub.state.hub.view().0.agents.contains_key(&agent)
    });
    // ADR-0106: made a viewer, the person's connection ends and the next one says so. Their console learns it without asking, hides
    // the keys that change something (`f` among them) and sends no typing, focus report or size for a terminal it watches: nothing it
    // does in the normal course is refused, so the owner's Activity gets no row from it.
    let refused_before = refused_rows();
    *person.lock().unwrap() = Role::Viewer;
    let end = Instant::now() + Duration::from_secs(20);
    while !matches!(session.next(Duration::from_millis(200)), Update::Closed) {
        assert!(Instant::now() < end, "the hub kept the demoted connection");
    }
    let mut session = Session::connect(&console, "client-token").unwrap();
    assert_eq!(
        session.person_role(),
        agora_protocol::v1::PersonRole::Viewer
    );
    assert_eq!(
        agora_tui::role::learn_on(
            &mut session,
            agora_tui::pair::Owner::No,
            agora_tui::picker::Role::Limited
        ),
        Some((agora_tui::pair::Owner::No, agora_tui::picker::Role::Limited))
    );
    assert!(agora_tui::role::changes('f') && agora_tui::role::changes('s'));
    assert!(session.usage_total().is_err());
    let shell = |agent_id: &str| agent_id == agent || agent.ends_with(&format!("/{agent_id}"));
    wait("the agent's terminal in the hub", || {
        hub.state
            .hub
            .view()
            .0
            .terminals
            .values()
            .any(|t| shell(&t.agent_id))
    });
    let terminal = hub
        .state
        .hub
        .view()
        .0
        .terminals
        .iter()
        .find(|(_, t)| shell(&t.agent_id))
        .map(|(id, _)| id.rsplit('/').next().unwrap_or(id).to_string())
        .unwrap();
    session.terminal_input("laptop", &terminal, b"echo hi\r");
    session.terminal_report("laptop", &terminal, b"\x1b[I");
    session.terminal_resize("laptop", &terminal, 100, 30);
    // Anything the hub would answer has had time to arrive.
    let end = Instant::now() + Duration::from_secs(1);
    while Instant::now() < end {
        let _ = session.next(Duration::from_millis(100));
    }
    assert_eq!(refused_rows(), refused_before);
    drop(node);
    let _ = std::fs::remove_dir_all(&pick);
    let _ = std::fs::remove_dir_all(&dir);
}

/// ADR-0066: launch settings (model, effort) through a real hub and node. A launch the user's device signed over its settings starts with
/// exactly them; a hub that adds, changes or drops one (here: a client that sends other settings than it signed) gets
/// `approval.signature_invalid` and nothing starts; unknown or invalid ones are refused before anything starts. An agent's `agent_start`
/// with settings reaches the window as an approval card that lists them, is decided only over a signature that covers them, and the helper
/// starts with exactly what the card showed.
#[test]
fn launch_settings_are_signed_through_the_hub_and_an_agents_start_shows_them_on_the_card() {
    use agora_protocol::v1::{self, command_request::Command as Cmd};
    use agora_tui::client::{Hub as ConsoleHub, Session};
    use std::io::{BufRead, BufReader, Write};
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let hub = hubside("laptop");
    let addr = hub.listen("127.0.0.1:0");
    let dir = tmp("launch-settings");
    let manifests = dir.join("manifests");
    std::fs::create_dir_all(&manifests).unwrap();
    // Reports its own arguments to a file named by its pid, then waits. It has an `[mcp]` section, so it is in the mesh and may ask.
    std::fs::write(
        manifests.join("probe.toml"),
        r#"kind = "probe"
display_name = "Probe"
version = "1"
min_engine = 3
[process]
launch = ["sh", "-c", "printf '%s\\n' \"$@\" > args-$$.txt; sleep 60", "sh"]
[[launch.setting]]
id = "model"
label = "setting-model"
args = ["--model", "{value}"]
[[launch.setting]]
id = "effort"
label = "setting-effort"
args = ["--effort", "{value}"]
values = ["low", "high"]
[mcp]
method = "cli_flag"
path = ".mcp.json"
flag = ["--mcp-config", "{config_file}"]
"#,
    )
    .unwrap();
    let pick = tmp("launch-settings-pick");
    std::fs::create_dir_all(pick.join("app")).unwrap();
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
        resume_all: false,
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
    let app = pick.join("app").to_string_lossy().into_owned();
    let inspect_auth = device
        .authorize("laptop", "inspect", &app, "", "", false, false, now())
        .unwrap();
    let info = session
        .inspect_folder("laptop", &app, inspect_auth)
        .unwrap();
    let pairs = |v: &[(&str, &str)]| -> Vec<(String, String)> {
        v.iter()
            .map(|(i, x)| ((*i).to_string(), (*x).to_string()))
            .collect()
    };
    let launch =
        |who: &agora_tui::device::Device, signed: &[(&str, &str)], sent: &[(&str, &str)]| {
            v1::AgentLaunch {
                path: info.path.clone(),
                project_id: info.project_id.clone(),
                kind: "probe".into(),
                authorization: who.authorize_launch_with_settings(
                    "laptop",
                    &info.path,
                    "probe",
                    &info.project_id,
                    false,
                    false,
                    &[],
                    &[],
                    &pairs(signed),
                    now(),
                ),
                settings: sent
                    .iter()
                    .map(|(i, x)| v1::LaunchSetting {
                        id: (*i).into(),
                        value: (*x).into(),
                    })
                    .collect(),
                ..Default::default()
            }
        };
    let both = [("model", "gpt-6.1-sol"), ("effort", "high")];
    let agents = || hub.state.hub.view().0.agents.len();
    // Whatever a hub (or anyone between the window and the machine) does to the settings after the user signed, nothing starts.
    for (signed, sent) in [
        (
            &both[..],
            &[("model", "gpt-6.1-terra"), ("effort", "high")][..],
        ),
        (
            &both[..],
            &[("effort", "high"), ("model", "gpt-6.1-sol")][..],
        ),
        (&both[..], &[("model", "gpt-6.1-sol")][..]),
        (
            &both[..],
            &[
                ("model", "gpt-6.1-sol"),
                ("effort", "high"),
                ("mode", "plan"),
            ][..],
        ),
        (&[][..], &both[..]),
    ] {
        assert_eq!(
            session.launch_in("laptop", launch(&device, signed, sent)),
            Err("approval.signature_invalid".to_string()),
            "{sent:?}"
        );
    }
    // A device the machine does not trust cannot sign settings in.
    assert_eq!(
        session.launch_in("laptop", launch(&stranger, &both, &both)),
        Err("approval.signature_invalid".to_string())
    );
    // Signed exactly, but the machine's manifest does not declare it, or does not take the value: refused before anything starts.
    let unknown = [("mode", "plan")];
    assert_eq!(
        session.launch_in("laptop", launch(&device, &unknown, &unknown)),
        Err("agent.setting_unknown".to_string())
    );
    let invalid = [("effort", "max")];
    assert_eq!(
        session.launch_in("laptop", launch(&device, &invalid, &invalid)),
        Err("agent.setting_invalid".to_string())
    );
    assert_eq!(agents(), 0, "nothing started");
    // The launch the user signed starts with exactly its settings, applied after the manifest's own command.
    let parent = session
        .launch_in("laptop", launch(&device, &both, &both))
        .unwrap();
    wait("the agent to appear in the hub", || {
        hub.state.hub.view().0.agents.contains_key(&parent)
    });
    let reported = |needle: &str| -> Option<Vec<String>> {
        std::fs::read_dir(&info.path).ok()?.flatten().find_map(|e| {
            let text = std::fs::read_to_string(e.path()).ok()?;
            (e.file_name().to_string_lossy().starts_with("args-") && text.contains(needle))
                .then(|| text.lines().map(str::to_string).collect())
        })
    };
    // The fake agent writes its arguments file line by line: wait for the last one, not the first.
    wait("the agent to report its arguments", || {
        reported("--model").is_some_and(|args| args.iter().any(|a| a == "high"))
    });
    let got = reported("--model").unwrap();
    let at = got.iter().position(|a| a == "--model").unwrap();
    assert_eq!(
        got[at..at + 4],
        ["--model", "gpt-6.1-sol", "--effort", "high"]
    );

    // The agent asks for a helper with a model: the window gets a card that lists it.
    let local_id = parent.strip_prefix("laptop/").unwrap().to_string();
    let socket = node.mesh.socket_path(&AgentId::new(&local_id));
    let mut conn = agora_platform::local_connect(&socket).unwrap();
    conn.set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let mut reader = BufReader::new(conn.try_clone().unwrap());
    let mut say = |value: serde_json::Value| writeln!(conn, "{value}").unwrap();
    say(serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}));
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    say(
        serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "agent_start", "arguments": {"kind": "probe", "settings": {"effort": "low", "model": "gpt-6.1-terra"}}}}),
    );
    wait("the card to reach the hub", || {
        hub.state
            .hub
            .view()
            .0
            .approvals
            .values()
            .any(|a| a.action == "agent.start")
    });
    let (key, card) = hub
        .state
        .hub
        .view()
        .0
        .approvals
        .iter()
        .find(|(_, a)| a.action == "agent.start")
        .map(|(k, a)| (k.clone(), a.detail.clone()))
        .unwrap();
    let local_approval = key.strip_prefix("laptop/").unwrap().to_string();
    let shown: serde_json::Value = serde_json::from_str(&card).unwrap();
    assert_eq!(
        shown["settings"],
        serde_json::json!([
            {"id": "model", "label": "setting-model", "value": "gpt-6.1-terra"},
            {"id": "effort", "label": "setting-effort", "value": "low"},
        ]),
        "in the manifest's order, with its labels"
    );
    let signed = agora_node::approvals::detail_settings(&card);
    let resolve = |session: &mut Session, signature: Vec<u8>| {
        session.command(
            "laptop",
            Cmd::ApprovalResolve(v1::ApprovalResolve {
                approval_id: local_approval.clone(),
                approved: true,
                signer_device_id: device.id.clone(),
                signature,
                helper_grant: 0,
                helpers_may_start: false,
                ..Default::default()
            }),
            Duration::from_secs(15),
        )
    };
    let sign = |m: Vec<u8>| device.sign(&m).unwrap();
    // The v1 message does not name the settings; other settings and a changed card do not match what was shown.
    let mut changed = signed.clone();
    changed[0].1 = "gpt-6.1-sol".into();
    for attempt in [
        agora_tui::device::start_message("laptop", &local_approval, true, 0, false, &card),
        agora_tui::device::start_message_v2(
            "laptop",
            &local_approval,
            true,
            0,
            false,
            &changed,
            &card,
        ),
        agora_tui::device::start_message_v2("laptop", &local_approval, true, 0, false, &[], &card),
    ] {
        assert_eq!(
            resolve(&mut session, sign(attempt)),
            Err("approval.signature_invalid".to_string())
        );
    }
    assert_eq!(node.approvals.pending_count(), 1, "still waiting");
    // The terminal console's own answer (ADR-0066): it reads the card from the hub's snapshot, shows its settings and signs over
    // `agora-approval-start-v2` by itself, with no help from this test. A stranger's key is still refused, and the request keeps waiting.
    let console_snapshot = {
        let end = Instant::now() + Duration::from_secs(20);
        loop {
            let mut fresh = Session::connect(&console, "client-token").unwrap();
            if let agora_tui::client::Update::Snapshot(s) = fresh.next(Duration::from_secs(2)) {
                if s.approvals.iter().any(|a| a.action == "agent.start") {
                    break *s;
                }
            }
            assert!(Instant::now() < end, "the card never reached the console");
            std::thread::sleep(Duration::from_millis(100));
        }
    };
    let shown_rows =
        agora_tui::client::decision_message("laptop", &local_approval, "agent.start", &card, true);
    assert!(shown_rows.starts_with(b"agora-approval-start-v2\0"));
    let (stranger, _) = agora_tui::device::Device::generate().unwrap();
    assert_eq!(
        session.resolve_first_approval(&console_snapshot, &stranger, true),
        Err("approval.signature_invalid".to_string())
    );
    assert_eq!(node.approvals.pending_count(), 1, "still waiting");
    assert_eq!(
        session.resolve_first_approval(&console_snapshot, &device, true),
        Ok(true)
    );
    line.clear();
    reader.read_line(&mut line).unwrap();
    let reply: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert!(reply["result"]["isError"] != true, "{reply}");
    wait("the helper to report its arguments", || {
        reported("gpt-6.1-terra").is_some()
    });
    let got = reported("gpt-6.1-terra").unwrap();
    let at = got.iter().position(|a| a == "--model").unwrap();
    assert_eq!(
        got[at..at + 4],
        ["--model", "gpt-6.1-terra", "--effort", "low"]
    );
    drop(node);
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
        resume_all: false,
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
        resume_all: false,
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

/// The node writes its link state where `agora status` reads it: a closed port gives a refusal code and never "up"; a real hub gives
/// "up" once the hub has taken the connection.
#[test]
fn the_node_reports_its_hub_link_state_for_status() {
    use agora_node::link_state::{LinkState, REFUSED, UNREACHABLE};
    // A port nothing listens on.
    let closed = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap()
    };
    let hub = hubside("laptop");
    let dir = tmp("link-state");
    std::fs::create_dir_all(&dir).unwrap();
    let sink = sink_with_agent(&tmp("link-state-events"), "laptop");
    let mut link = hub.link(closed);
    link.state_dir = Some(dir.clone());
    let _refused = spawn(Arc::clone(&sink), link).unwrap();
    wait("a refusal to be recorded", || {
        matches!(LinkState::read(&dir), Some(LinkState::Down { code, .. })
            // Windows gives up on a closed loopback port as a timeout, Unix refuses it.
            if code == REFUSED || (cfg!(windows) && code == UNREACHABLE))
    });
    assert_ne!(LinkState::read(&dir), Some(LinkState::Up));

    let addr = hub.listen("127.0.0.1:0");
    let up_dir = tmp("link-state-up");
    std::fs::create_dir_all(&up_dir).unwrap();
    let mut link = hub.link(addr);
    link.state_dir = Some(up_dir.clone());
    let _up = spawn(sink, link).unwrap();
    wait("the link to be recorded as up", || {
        LinkState::read(&up_dir) == Some(LinkState::Up)
    });
}

/// #1016: a message between two agents on one machine reaches the hub's store and a subscribed window within 2 s, on its own: no
/// later event of the node is needed to carry it (the supervision tick drains the broker's events every 100 ms, the uplink sends
/// what the log holds every 50 ms). Coverage for the report; it passed before any change, as did the same path with real
/// `agora-fake-agent --acp` agents, `agora node run` and `agora hub run` in a browser.
#[test]
fn a_message_between_agents_on_one_machine_reaches_the_hub_and_a_window_on_its_own() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    use agora_hub::store::EventStore;
    use agora_mesh::broker::AgentInfo;
    use agora_protocol::frame::{kinds, STREAM_EVENTS};
    use agora_protocol::v1::event::Body;
    use prost::Message as _;
    let hub = hubside("alpha");
    let addr = hub.listen("127.0.0.1:0");
    let dir = tmp("local-message");
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = agora_node::daemon::NodeConfig {
        resume_all: false,
        node_id: "alpha".into(),
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
    let _up = spawn(Arc::clone(&node.events), link).unwrap();
    let register = |id: &str| {
        node.mesh
            .register(AgentInfo {
                id: AgentId::new(id),
                kind: "shell".into(),
                node: "alpha".into(),
                project: "p".into(),
                label: String::new(),
                worktree: String::new(),
                state: "idle".into(),
            })
            .unwrap()
    };
    let a_sock = register("a1");
    let _b_sock = register("a2");
    wait("the node to be online", || {
        hub.state
            .hub
            .view()
            .0
            .nodes
            .get("alpha")
            .is_some_and(|x| x.online)
    });
    let posted = || -> Vec<(String, String, String)> {
        hub.state
            .hub
            .store()
            .events_after(0, 10_000)
            .unwrap()
            .iter()
            .filter_map(|e| {
                let ev = agora_protocol::v1::Event::decode(e.payload.as_slice()).ok()?;
                match ev.body? {
                    Body::MessagePosted(m) if m.body == "hello next door" => {
                        Some((ev.producer_id, m.from_agent_id, m.to))
                    }
                    _ => None,
                }
            })
            .collect()
    };
    // A window subscribed before the message: it must get it live.
    let mut window = hub.client(addr);
    hub.rt.block_on(async {
        window
            .send(&agora_protocol::Frame {
                stream_id: STREAM_EVENTS,
                kind: kinds::SUBSCRIBE,
                seq: None,
                payload: agora_protocol::v1::Subscribe::default().encode_to_vec(),
            })
            .await
            .unwrap();
        // Subscribed once its snapshot is here: a message the hub took before it would be in no snapshot and never come live.
        while next_frame(&mut window).await.kind != kinds::SNAPSHOT {}
    });
    let mut conn = agora_platform::local_connect(&a_sock).unwrap();
    mcp(
        &mut conn,
        1,
        "initialize",
        serde_json::json!({"protocolVersion": "2025-03-26"}),
    );
    let sent = mcp(
        &mut conn,
        2,
        "tools/call",
        serde_json::json!({"name": "send_message", "arguments": {"to": "a2", "body": "hello next door"}}),
    );
    assert_ne!(sent["isError"], true, "{sent}");
    // Nothing else happens on the node: the message alone must get there, well within the window's one second plus slack.
    let end = Instant::now() + Duration::from_secs(2);
    while posted().is_empty() {
        assert!(
            Instant::now() < end,
            "the message between agents on one machine did not reach the hub within 2 s"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        posted(),
        [("alpha".into(), "a1".into(), "a2".into())],
        "one copy, from the sender's node"
    );
    // And the window has it, live, within the same deadline.
    let seen = hub.rt.block_on(async {
        loop {
            let left = end.saturating_duration_since(Instant::now());
            let Ok(Ok(Some(f))) = tokio::time::timeout(left, window.recv()).await else {
                break false;
            };
            if (f.stream_id, f.kind) != (STREAM_EVENTS, kinds::POSITIONED_EVENT) {
                continue;
            }
            let Ok(d) = agora_protocol::v1::PositionedEvent::decode(f.payload.as_slice()) else {
                continue;
            };
            if matches!(d.event.and_then(|e| e.body), Some(Body::MessagePosted(m)) if m.body == "hello next door") {
                break true;
            }
        }
    });
    assert!(seen, "the window did not get the message live within 2 s");
}

/// #1020, ADR-0103: a message sent before a window opens is in the window's snapshot. Agents message each other with no window open;
/// a window that opens afterwards gets the stored message, with its position, producer and time, in `Snapshot.recent_messages`.
#[test]
fn a_window_that_opens_after_agents_messaged_each_other_gets_the_message_in_its_snapshot() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    use agora_hub::store::EventStore;
    use agora_mesh::broker::AgentInfo;
    use agora_protocol::frame::{kinds, STREAM_EVENTS};
    use agora_protocol::v1::event::Body;
    use prost::Message as _;
    let hub = hubside("alpha");
    let addr = hub.listen("127.0.0.1:0");
    let dir = tmp("history-message");
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = agora_node::daemon::NodeConfig {
        resume_all: false,
        node_id: "alpha".into(),
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
    let _up = spawn(Arc::clone(&node.events), link).unwrap();
    let register = |id: &str| {
        node.mesh
            .register(AgentInfo {
                id: AgentId::new(id),
                kind: "shell".into(),
                node: "alpha".into(),
                project: "p".into(),
                label: String::new(),
                worktree: String::new(),
                state: "idle".into(),
            })
            .unwrap()
    };
    let a_sock = register("a1");
    let _b_sock = register("a2");
    wait("the node to be online", || {
        hub.state
            .hub
            .view()
            .0
            .nodes
            .get("alpha")
            .is_some_and(|x| x.online)
    });
    let mut conn = agora_platform::local_connect(&a_sock).unwrap();
    mcp(
        &mut conn,
        1,
        "initialize",
        serde_json::json!({"protocolVersion": "2025-03-26"}),
    );
    let sent = mcp(
        &mut conn,
        2,
        "tools/call",
        serde_json::json!({"name": "send_message", "arguments": {"to": "a2", "body": "before you came"}}),
    );
    assert_ne!(sent["isError"], true, "{sent}");
    // Stored by the hub before any window is open.
    let stored = || {
        hub.state
            .hub
            .store()
            .events_after(0, 10_000)
            .unwrap()
            .into_iter()
            .find_map(|e| {
                let ev = agora_protocol::v1::Event::decode(e.payload.as_slice()).ok()?;
                matches!(&ev.body, Some(Body::MessagePosted(m)) if m.body == "before you came")
                    .then_some(e.pos)
            })
    };
    wait("the hub to store the message", || stored().is_some());
    let at = stored().unwrap();
    // Now a window opens.
    let mut window = hub.client(addr);
    let snapshot = hub.rt.block_on(async {
        window
            .send(&agora_protocol::Frame {
                stream_id: STREAM_EVENTS,
                kind: kinds::SUBSCRIBE,
                seq: None,
                payload: agora_protocol::v1::Subscribe::default().encode_to_vec(),
            })
            .await
            .unwrap();
        loop {
            let f = next_frame(&mut window).await;
            if f.kind == kinds::SNAPSHOT {
                break agora_protocol::v1::Snapshot::decode(f.payload.as_slice()).unwrap();
            }
        }
    });
    let got: Vec<_> = snapshot
        .recent_messages
        .iter()
        .filter_map(|p| {
            let e = p.event.as_ref()?;
            match &e.body {
                Some(Body::MessagePosted(m)) => Some((
                    p.pos,
                    e.producer_id.as_str(),
                    m.from_agent_id.as_str(),
                    m.to.as_str(),
                    m.body.as_str(),
                    e.at_ms > 0,
                )),
                _ => None,
            }
        })
        .collect();
    assert_eq!(
        got,
        [(at, "alpha", "a1", "a2", "before you came", true)],
        "the window's snapshot holds the message sent before it opened"
    );
    assert!(at <= snapshot.pos);
}

/// ADR-0106: a hub from before the role was told sends the snapshot first, not a `Welcome`. The terminal client's `connect` reads that
/// first frame while it waits for a `Welcome`, so it must keep it: the first `next` is that snapshot, and the role stays unspecified (the
/// client then learns it as before, #1107).
#[test]
fn the_terminal_client_keeps_an_older_hubs_first_snapshot_and_knows_no_role() {
    use agora_protocol::frame::{kinds, STREAM_EVENTS};
    use agora_protocol::Frame;
    use agora_tui::client::{Hub as ConsoleHub, Session, Update};
    use prost::Message;
    let ca = Ca::generate("older hub ca").unwrap();
    let (cert, key) = ca.issue_server(&["localhost".to_string()]).unwrap();
    let acceptor = tls::acceptor_with(&cert, &key, ca.cert_pem(), true).unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let listener = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let addr = listener.local_addr().unwrap();
    // The older hub: takes the subscription and answers with its snapshot, nothing before it. The client tries the address's names in
    // turn (the certificate names `localhost`), so a connection whose handshake fails is let go and the next one is taken.
    rt.spawn(async move {
        loop {
            let (tcp, _) = listener.accept().await.unwrap();
            let Ok(tls) = acceptor.accept(tcp).await else {
                continue;
            };
            let (mut conn, token) = agora_protocol::ws::accept_with_token(tls).await.unwrap();
            assert_eq!(token.as_deref(), Some("client-token"));
            let sub = conn.recv().await.unwrap().unwrap();
            assert_eq!((sub.stream_id, sub.kind), (STREAM_EVENTS, kinds::SUBSCRIBE));
            let snapshot = agora_protocol::v1::Snapshot {
                pos: 7,
                nodes: vec![agora_protocol::v1::NodeSnapshot {
                    node_id: "older".into(),
                    online: true,
                    ..Default::default()
                }],
                ..Default::default()
            };
            conn.send(&Frame {
                stream_id: STREAM_EVENTS,
                kind: kinds::SNAPSHOT,
                seq: None,
                payload: snapshot.encode_to_vec(),
            })
            .await
            .unwrap();
            // Keep the connection open while the client reads.
            let _ = conn.recv().await;
            return;
        }
    });
    let console = ConsoleHub {
        addr: addr.to_string(),
        ca_pem: ca.cert_pem().to_string(),
    };
    let mut session = Session::connect(&console, "client-token").unwrap();
    assert_eq!(
        session.person_role(),
        agora_protocol::v1::PersonRole::Unspecified
    );
    match session.next(Duration::from_secs(2)) {
        Update::Snapshot(s) => {
            assert_eq!((s.pos, s.nodes[0].node_id.as_str()), (7, "older"));
        }
        _ => panic!("the older hub's first snapshot was lost"),
    }
}

/// ADR-0092 (the maintainer's answer on #1133, "startable, but always signed"): a registry kind that starts through a package runner
/// (`npx`, `uvx`) fetches and runs code when it starts, so the node accepts a start of one only with a person's device signature, whatever
/// its `signed_commands` policy says. Here, with no policy file at all: the hub's unsigned `AgentStart` of such a kind is refused with
/// `node.policy_denied` (rule `registry_signed_start`); the same start signed by a trusted device (`AgentLaunch`) starts it. Decided by
/// origin (review F5): hand-made kinds with the very same command are not affected, also one named `registry-*`. A `registry-*` kind
/// the node has not read yet is held to it until it has (review F6). The "npx" here is a local stub that runs the fake ACP agent: no
/// network, no real npx; the registry kind is registered as the node registers the converted ones.
#[cfg(unix)]
#[test]
fn a_registry_kind_that_fetches_starts_only_when_a_device_signed_its_start() {
    use agora_protocol::v1;
    use agora_tui::client::{Hub as ConsoleHub, Session, Update};
    use std::os::unix::fs::PermissionsExt;
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    // The node runs in this process: its ACP bridge is the `agora` binary, not this test (as in `approval_always.rs`).
    std::env::set_var("AGORA_ACP_BRIDGE", env!("CARGO_BIN_EXE_agora"));
    let hub = hubside("laptop");
    let addr = hub.listen("127.0.0.1:0");
    let dir = tmp("registry-signed-start");
    let repo = dir.join("myproject");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let mut fake = std::path::PathBuf::from(env!("CARGO_BIN_EXE_agora"));
    fake.pop();
    fake.push(format!("agora-fake-agent{}", std::env::consts::EXE_SUFFIX));
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
    // A stub named `npx`: whatever package it is asked for, it runs the fake ACP agent.
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let npx = bin.join("npx");
    std::fs::write(
        &npx,
        format!("#!/bin/sh\nexec {:?} --acp\n", fake.to_string_lossy()),
    )
    .unwrap();
    std::fs::set_permissions(&npx, std::fs::Permissions::from_mode(0o755)).unwrap();
    let manifests = dir.join("manifests");
    std::fs::create_dir_all(&manifests).unwrap();
    let acp = |kind: &str| {
        format!(
            "kind = \"{kind}\"\ndisplay_name = \"Stub\"\nversion = \"1.0.0\"\nmin_engine = 1\n\n[structured]\nacp = {{ argv = [{:?}, \"--yes\", \"stub-agent@1.0.0\"] }}\n",
            npx.to_string_lossy()
        )
    };
    std::fs::write(manifests.join("by-hand.toml"), acp("stub-by-hand")).unwrap();
    std::fs::write(manifests.join("lookalike.toml"), acp("registry-lookalike")).unwrap();
    let pick = tmp("registry-signed-start-pick");
    std::fs::create_dir_all(pick.join("app")).unwrap();
    git(&pick.join("app"), &["init", "-q", "-b", "main"]);
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
        resume_all: false,
        node_id: "laptop".into(),
        socket: dir.join("c.sock"),
        data_dir: data,
        worktree_root: dir.join("wt"),
        manifests_dir: manifests,
        projects: vec![repo],
        limits: Default::default(),
    };
    let node = agora_node::daemon::Node::start(&cfg).unwrap();
    node.supervisor
        .lock()
        .unwrap()
        .register_from_registry(agora_adapters::Manifest::parse(&acp("registry-stub")).unwrap());
    let mut link = hub.link(addr);
    link.commands = Some(cfg.socket.clone());
    link.grants = Some(Arc::clone(&node.grants));
    let _uplink = spawn(Arc::clone(&node.events), link).unwrap();
    let console = ConsoleHub {
        addr: addr.to_string(),
        ca_pem: hub.ca_pem.clone(),
    };
    let end = Instant::now() + Duration::from_secs(20);
    let (mut session, project) = loop {
        let mut session = Session::connect(&console, "client-token").unwrap();
        if let Update::Snapshot(sn) = session.next(Duration::from_secs(2)) {
            if let Some(c) = sn.checkouts.first() {
                break (session, c.project_id.clone());
            }
        }
        assert!(Instant::now() < end, "no project");
        std::thread::sleep(Duration::from_millis(100));
    };
    // The hub alone (an unsigned start), with no `signed_commands` on this machine: refused, nothing started.
    let refused = session
        .start_agent("laptop", &project, "registry-stub")
        .unwrap_err();
    assert_eq!(refused.code, "node.policy_denied", "{refused:?}");
    let rule = refused.params.get("rule").and_then(|p| match &p.value {
        Some(v1::param_value::Value::Str(s)) => Some(s.clone()),
        _ => None,
    });
    assert_eq!(
        rule.as_deref(),
        Some("registry_signed_start"),
        "{refused:?}"
    );
    // Hand-made kinds with the same command are not affected, also one whose name looks like a registry kind's.
    for kind in ["stub-by-hand", "registry-lookalike"] {
        let by_hand = session.start_agent("laptop", &project, kind).unwrap();
        assert!(!by_hand.is_empty(), "{kind}");
    }
    // A `registry-*` description added after the node read its folder: unknown where the start is judged, so held to a signature;
    // once the node has read it (here: its agent list was asked for) it is a hand-made kind like the others.
    std::fs::write(cfg.manifests_dir.join("late.toml"), acp("registry-late")).unwrap();
    let late = session
        .start_agent("laptop", &project, "registry-late")
        .unwrap_err();
    assert_eq!(late.code, "node.policy_denied", "{late:?}");
    let _ = session.agent_kinds_all("laptop").unwrap();
    assert!(!session
        .start_agent("laptop", &project, "registry-late")
        .unwrap()
        .is_empty());
    // Signed by a trusted device: it starts.
    let now = || {
        u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap()
    };
    let app = pick.join("app").to_string_lossy().into_owned();
    let inspect = device
        .authorize("laptop", "inspect", &app, "", "", false, false, now())
        .unwrap();
    let info = session.inspect_folder("laptop", &app, inspect).unwrap();
    let auth = device
        .authorize_launch(
            "laptop",
            &info.path,
            "registry-stub",
            &info.project_id,
            false,
            false,
            &[],
            &[],
            now(),
        )
        .unwrap();
    let started = session
        .launch_in(
            "laptop",
            v1::AgentLaunch {
                path: info.path.clone(),
                project_id: info.project_id.clone(),
                kind: "registry-stub".into(),
                authorization: Some(auth),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(started.starts_with("laptop/"), "{started}");
    drop(node);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&pick);
}

/// ADR-0115 end to end: two machines with clones of one GitHub repository whose `origin` differs only in case (`Owner/Repo` and
/// `owner/repo`, the same root commit) discover one project id. Over mTLS to a real hub, one sends that id and the other the id an
/// older node makes of the same clone (`github.com/Owner/Repo#…`), and the hub holds them as one project shared by both.
#[test]
fn two_machines_with_clones_of_one_github_repository_in_two_cases_are_one_shared_project() {
    let hub = hubside("laptop");
    let addr = hub.listen("127.0.0.1:0");
    let dir = tmp("case-fold");
    let first = dir.join("Repo");
    std::fs::create_dir_all(&first).unwrap();
    git(&first, &["init", "-q", "-b", "main"]);
    git(&first, &["commit", "-q", "--allow-empty", "-m", "root"]);
    let second = dir.join("repo-clone");
    git(
        &dir,
        &[
            "clone",
            "-q",
            first.to_str().unwrap(),
            second.to_str().unwrap(),
        ],
    );
    git(
        &first,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/Owner/Repo.git",
        ],
    );
    git(
        &second,
        &[
            "remote",
            "set-url",
            "origin",
            "git@github.com:owner/repo.git",
        ],
    );
    let ids: Vec<String> = [&first, &second]
        .iter()
        .map(|r| {
            agora_node::git::discover_checkout(r)
                .unwrap()
                .project
                .to_string()
        })
        .collect();
    assert_eq!(ids[0], ids[1]);
    assert!(ids[0].starts_with("github.com/owner/repo#"), "{}", ids[0]);

    let older = ids[0].replace("github.com/owner/repo#", "github.com/Owner/Repo#");
    assert_ne!(older, ids[0]);
    let (desk_cert, desk_key) = hub.enroll("desk");
    let mut links = Vec::new();
    for (node, repo, id, cert) in [
        ("laptop", &first, &ids[0], None),
        ("desk", &second, &older, Some((desk_cert, desk_key))),
    ] {
        let sink = Arc::new(EventSink::open(&dir.join(format!("{node}-data")), node).unwrap());
        sink.node_online(node);
        sink.checkout_discovered(node, id, repo);
        let mut link = hub.link(addr);
        if let Some((cert_pem, key_pem)) = cert {
            link.cert_pem = cert_pem;
            link.key_pem = key_pem;
        }
        links.push((sink.clone(), spawn(sink, link).unwrap()));
    }
    wait("both machines' checkouts", || {
        hub.state.hub.view().0.checkouts.len() == 2
    });
    let (view, _) = hub.state.hub.view();
    assert!(view.checkouts.values().all(|(_, p, _)| p == &ids[0]));
    assert_eq!(
        agora_hub::board::nodes_with(&view, &ids[0]),
        ["desk", "laptop"]
    );
    // A command about the project reaches the older machine in its own spelling.
    assert_eq!(view.spelled_for("desk", &ids[0]), older);
    assert_eq!(view.spelled_for("laptop", &ids[0]), ids[0]);
    drop(links);
    let _ = std::fs::remove_dir_all(&dir);
}

/// ADR-0039 on a real hub: the holder's node renews its agent's board lease, so it outlives its TTL while the agent runs (the other
/// machine's agent is refused all along). When the hub sees the agent end while the node still has it on the mesh (as while the
/// node's supervisor restarts it), the hub gives the lease back, the node's next renewal is answered `agent.not_found`, and the node
/// stops renewing it; the other machine then gets the path.
#[test]
fn a_shared_lease_is_renewed_by_its_node_and_given_back_when_the_hub_sees_its_agent_end() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    use agora_mesh::broker::AgentInfo;
    use agora_node::uplink_client::MeshOut;
    use serde_json::{json, Value};
    let hub = hubside("alpha");
    let addr = hub.listen("127.0.0.1:0");
    let start = |name: &str, agent: &str, cert: String, key: String| {
        let dir = tmp(&format!("renew-{name}"));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = agora_node::daemon::NodeConfig {
            resume_all: false,
            node_id: name.into(),
            socket: dir.join("c.sock"),
            data_dir: dir.join("data"),
            worktree_root: dir.join("wt"),
            manifests_dir: dir.join("manifests"),
            projects: vec![],
            limits: Default::default(),
        };
        let node = agora_node::daemon::Node::start(&cfg).unwrap();
        // The node's renewal schedule, shortened so the test takes seconds: a 2 s TTL is renewed every second.
        node.mesh
            .set_lease_renewal_policy(agora_node::lease_renewal::Policy {
                min_ttl_ms: 2_000,
                min_gap_ms: 1_000,
                ..Default::default()
            });
        let out = MeshOut::new();
        node.mesh.set_remote(out.clone());
        let mut link = hub.link(addr);
        (link.cert_pem, link.key_pem) = (cert, key);
        link.commands = Some(cfg.socket.clone());
        link.mesh_out = Some(out);
        let up = spawn(Arc::clone(&node.events), link).unwrap();
        node.events
            .checkout_discovered(name, "p", Path::new("/work/p"));
        let id = AgentId::new(agent);
        node.events.agent_launched(
            &id,
            "p",
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
                project: "p".into(),
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
    let (a, _ua, a1, mut ac) = start("alpha", "a1", ca.0, ca.1);
    let (_b, _ub, _b1, mut bc) = start("beta", "b1", cb.0, cb.1);
    for (node, agent) in [("alpha", "alpha/a1"), ("beta", "beta/b1")] {
        wait(&format!("{node} and {agent} to reach the hub"), || {
            let (v, _) = hub.state.hub.view();
            v.nodes.get(node).is_some_and(|n| n.online) && v.agents.contains_key(agent)
        });
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
    let ttl_s = 3;
    let (l, err) = call(
        &mut ac,
        "lease_acquire",
        json!({"paths": ["src/x.rs"], "ttl_s": ttl_s}),
    );
    assert!(!err, "{l}");
    let probe = |bc: &mut agora_platform::LocalStream| {
        call(
            bc,
            "lease_acquire",
            json!({"paths": ["src/x.rs"], "ttl_s": 60}),
        )
    };
    let granted = Instant::now();
    while granted.elapsed() < Duration::from_secs(ttl_s * 3) {
        let (held, err) = probe(&mut bc);
        assert!(
            err,
            "the lease ended at {:?} while its agent runs",
            granted.elapsed()
        );
        assert_eq!(held["error"]["code"], "lease.held", "{held}");
        assert_eq!(held["error"]["params"]["holder_agent_id"], "alpha/a1");
        std::thread::sleep(Duration::from_millis(500));
    }
    assert_eq!(a.mesh.renewing(&a1), 1);

    // The hub sees the agent end; the node keeps it on the mesh (a restart is coming), so it would renew.
    a.events
        .record_bodies(vec![agora_protocol::v1::event::Body::AgentExited(
            agora_protocol::v1::AgentExited {
                agent_id: "a1".into(),
                exit_code: 1,
            },
        )]);
    wait("the hub to see alpha/a1 end", || {
        let (v, _) = hub.state.hub.view();
        v.agents.get("alpha/a1").is_some_and(|x| !x.running)
    });
    // What the hub answers that agent's lease call now, as it answers the node's renewal (the same call, as the same agent).
    let (gone, err) = call(
        &mut ac,
        "lease_acquire",
        json!({"paths": ["src/other.rs"], "ttl_s": ttl_s}),
    );
    assert!(err, "{gone}");
    assert_eq!(gone["error"]["code"], "agent.not_found", "{gone}");
    wait("the node to stop renewing after agent.not_found", || {
        a.mesh.renewing(&a1) == 0
    });
    let (got, err) = probe(&mut bc);
    assert!(
        !err,
        "the lease of an agent the hub saw end is still held: {got}"
    );
}

/// ADR-0119, end to end through a real hub: an agent on alpha asks beta for a helper with `agent_start {node: "beta"}`; beta raises an
/// `agent.start.remote` card; the person's v4 decision there starts it; the hub carries the result back and the agent's call answers
/// with the helper as `beta/<agent>`, whose `AgentStarted` names its parent on alpha.
#[test]
fn an_agent_starts_a_helper_on_another_machine_with_the_persons_decision_there() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    use agora_node::uplink_client::MeshOut;
    use agora_protocol::v1::{self, command_ok::Result as Ok_, command_request::Command as Cmd};
    let mut fake = std::path::PathBuf::from(env!("CARGO_BIN_EXE_agora"));
    fake.pop();
    fake.push(format!("agora-fake-agent{}", std::env::consts::EXE_SUFFIX));
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
    let hub = hubside("alpha");
    let addr = hub.listen("127.0.0.1:0");
    let (device, _) = agora_tui::device::Device::generate().unwrap();
    let start = |name: &str, cert: String, key: String| {
        let dir = tmp(&format!("remote-{name}"));
        let manifests = dir.join("manifests");
        std::fs::create_dir_all(&manifests).unwrap();
        std::fs::create_dir_all(dir.join("work")).unwrap();
        let launch = serde_json::to_string(&vec![
            fake.to_string_lossy().into_owned(),
            "--probe-directory".into(),
            dir.join("probe-{pid}.json").to_string_lossy().into_owned(),
        ])
        .unwrap();
        std::fs::write(
            manifests.join("probe.toml"),
            format!("kind = \"probe\"\ndisplay_name = \"Probe\"\nversion = \"1\"\nmin_engine = 3\n[process]\nlaunch = {launch}\n[mcp]\nmethod = \"cli_flag\"\npath = \".mcp.json\"\nflag = [\"--mcp-config\", \"{{config_file}}\"]\n"),
        )
        .unwrap();
        let data = dir.join("data");
        std::fs::create_dir_all(&data).unwrap();
        agora_node::approvals::trust_device(
            &data.join(agora_node::approvals::TRUSTED_DEVICES_FILE),
            &device.id,
            &device.public_key_hex(),
        )
        .unwrap();
        let cfg = agora_node::daemon::NodeConfig {
            resume_all: false,
            node_id: name.into(),
            socket: dir.join("c.sock"),
            data_dir: data,
            worktree_root: dir.join("wt"),
            manifests_dir: manifests,
            projects: vec![],
            limits: Default::default(),
        };
        let node = agora_node::daemon::Node::start(&cfg).unwrap();
        let out = MeshOut::new();
        node.mesh.set_remote(out.clone());
        node.spawner.set_link(out.clone());
        let mut link = hub.link(addr);
        (link.cert_pem, link.key_pem) = (cert, key);
        link.commands = Some(cfg.socket.clone());
        link.mesh_out = Some(out);
        link.spawner = Some(Arc::clone(&node.spawner));
        let up = spawn(Arc::clone(&node.events), link).unwrap();
        (node, up, cfg.socket, dir)
    };
    let (ca, cb) = (hub.enroll("alpha"), hub.enroll("beta"));
    let (alpha, _ua, alpha_sock, alpha_dir) = start("alpha", ca.0, ca.1);
    let (beta, _ub, beta_sock, beta_dir) = start("beta", cb.0, cb.1);
    wait("both machines to agree on mesh.spawn.remote", || {
        let agreed = hub
            .state
            .hub
            .router()
            .nodes_with(agora_protocol::handshake::MESH_SPAWN_REMOTE);
        agreed.iter().any(|n| n == "alpha") && agreed.iter().any(|n| n == "beta")
    });
    // The person's agent on alpha.
    let mut c = agora_node::control::Client::connect(&alpha_sock).unwrap();
    let work = agora_platform::canonicalize_plain(&alpha_dir.join("work")).unwrap();
    let info = match c
        .call(Cmd::DirectoryInspect(v1::DirectoryInspect {
            path: work.to_string_lossy().into_owned(),
            create_project_id: false,
            authorization: None,
        }))
        .unwrap()
        .unwrap()
        .result
    {
        Some(Ok_::DirectoryInfo(i)) => i,
        other => panic!("{other:?}"),
    };
    let asker = match c
        .call(Cmd::AgentLaunch(v1::AgentLaunch {
            path: info.path.clone(),
            project_id: info.project_id.clone(),
            kind: "probe".into(),
            ..Default::default()
        }))
        .unwrap()
        .unwrap()
        .result
    {
        Some(Ok_::AgentLaunch(r)) => r.agent_id,
        other => panic!("{other:?}"),
    };
    let sock = alpha
        .mesh
        .socket_path(&agora_core::AgentId::new(asker.clone()));
    wait("the asker's mesh socket", || sock.exists());
    let target_folder = agora_platform::canonicalize_plain(&beta_dir.join("work"))
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let call = std::thread::spawn({
        let sock = sock.clone();
        let target_folder = target_folder.clone();
        move || {
            let mut conn = agora_platform::local_connect(&sock).unwrap();
            mcp(
                &mut conn,
                1,
                "initialize",
                serde_json::json!({"protocolVersion": "2025-03-26"}),
            );
            mcp(
                &mut conn,
                2,
                "tools/call",
                serde_json::json!({"name": "agent_start", "arguments": {"kind": "probe", "node": "beta", "folder": target_folder}}),
            )
        }
    });
    // Beta's card, decided over v4 by the person's device.
    let mut card = None;
    wait("beta's agent.start.remote card", || {
        if call.is_finished() {
            panic!("the agent's call ended before any card");
        }
        card = beta
            .approvals
            .waiting()
            .into_iter()
            .find(|p| p.action == agora_node::approvals::REMOTE_START_ACTION);
        card.is_some()
    });
    let p = card.unwrap();
    assert_eq!(p.remote.as_ref().unwrap().origin_node, "alpha");
    let grant = v1::RemoteGrant::default();
    let msg = agora_node::approvals::start_signed_message_v4(
        "beta",
        p.remote.as_ref().unwrap(),
        &p.id,
        true,
        &agora_node::approvals::detail_settings(&p.detail),
        &agora_node::approvals::detail_launch_options(&p.detail),
        &grant,
        &p.detail,
    );
    let mut cb = agora_node::control::Client::connect(&beta_sock).unwrap();
    cb.call(Cmd::ApprovalResolve(v1::ApprovalResolve {
        approval_id: p.id.clone(),
        approved: true,
        signer_device_id: device.id.clone(),
        signature: device.sign(&msg).unwrap(),
        remote_grant: Some(grant),
        ..Default::default()
    }))
    .unwrap()
    .unwrap();
    let answer = call.join().unwrap();
    let text = answer["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    let helper = v["agent_id"].as_str().unwrap_or_default().to_string();
    assert!(helper.starts_with("beta/agent-"), "{answer}");
    let local = helper.trim_start_matches("beta/").to_string();
    let started = beta
        .events
        .events_after(0)
        .into_iter()
        .find_map(|e| match e.body {
            Some(v1::event::Body::AgentStarted(s)) if s.agent_id == local => Some(s),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        (
            started.parent_node_id.as_str(),
            started.parent_node_agent_id.as_str(),
            started.parent_agent_id.as_str()
        ),
        ("alpha", asker.as_str(), "")
    );
    // Following it from alpha (ADR-0119 decision 7): its screen through the hub, and a wait for a state it reaches.
    let mut conn = agora_platform::local_connect(&sock).unwrap();
    mcp(
        &mut conn,
        1,
        "initialize",
        serde_json::json!({"protocolVersion": "2025-03-26"}),
    );
    let out = mcp(
        &mut conn,
        3,
        "tools/call",
        serde_json::json!({"name": "agent_output", "arguments": {"agent_id": helper, "lines": 5}}),
    );
    assert_ne!(out["isError"], true, "{out}");
    let waited = mcp(
        &mut conn,
        4,
        "tools/call",
        serde_json::json!({"name": "agent_wait", "arguments": {"agent_id": helper, "state": "exited", "timeout_s": 3}}),
    );
    // It runs on: the wait ends at its timeout, and says so; the point is that it was asked of beta (not refused as unknown).
    let text = waited["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(!text.contains("agent.not_found"), "{waited}");
}
