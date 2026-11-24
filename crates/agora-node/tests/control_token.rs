//! ADR-0082 (issue #440, threat model T16): a confined agent cannot use the node's control socket on Linux. A real node, a real
//! Landlock-confined process, the control token it cannot read. The "agent" is this test binary itself, run again with
//! `AGORA_PROBE_SOCKET` set (`confined_probe_child`), so no other program is needed.
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "private_home/mod.rs"]
mod private_home;
#[path = "short_socket/mod.rs"]
mod short_socket;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use agora_node::control::Client;
use agora_node::control_token::{read_token, token_path};
use agora_node::daemon::{Node, NodeConfig};
use agora_node::supervisor::NodeLimits;
use agora_protocol::v1::{self, command_ok::Result as Ok_, command_request::Command as Cmd};

fn git(dir: &Path, args: &[&str]) {
    assert!(Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .status()
        .unwrap()
        .success());
}

struct Env {
    root: PathBuf,
    node: Option<Node>,
    /// Dropped after the node (`drop` takes the node first).
    _dir: Option<short_socket::SocketDir>,
}

impl Env {
    /// A node under `base` (here a folder a confined agent is granted) whose policy file says `policy`.
    fn in_base(base: &Path, name: &str, policy: Option<&str>) -> Self {
        private_home::init();
        Self::at(
            base.join(format!("ct-{}-{name}", std::process::id())),
            policy,
            None,
        )
    }

    /// A node whose home is `root`, kept for as long as `dir` (when the home is that folder).
    fn at(root: PathBuf, policy: Option<&str>, dir: Option<short_socket::SocketDir>) -> Self {
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("manifests")).unwrap();
        std::fs::create_dir_all(root.join("data")).unwrap();
        let repo = root.join("proj");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "x").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        if let Some(policy) = policy {
            std::fs::write(root.join("data/policy"), policy).unwrap();
        }
        let exe = std::env::current_exe().unwrap();
        let launch = serde_json::to_string(&vec![
            exe.to_string_lossy().into_owned(),
            "--exact".to_string(),
            "confined_probe_child".to_string(),
            "--nocapture".to_string(),
        ])
        .unwrap();
        let socket = root.join("node.sock");
        std::fs::write(
            root.join("manifests/probe.toml"),
            format!(
                "kind = \"probe\"\ndisplay_name = \"Probe\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = {launch}\n[process.env]\nAGORA_PROBE_SOCKET = {:?}\n",
                socket.to_string_lossy()
            ),
        )
        .unwrap();
        let node = Node::start(&config(&root)).unwrap();
        Self {
            root,
            node: Some(node),
            _dir: dir,
        }
    }

    /// A node in a folder no confined agent is granted: #926, its home (and the socket, token and mesh sockets in it) short enough
    /// for a unix socket and out of the agent's reach, whatever the checkout's depth and `TMPDIR`.
    fn new(name: &str, policy: Option<&str>) -> Self {
        private_home::init();
        let preferred = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("ct-{}-{name}", std::process::id()));
        let dir = short_socket::SocketDir::new(&preferred, &format!("ct-{name}"));
        Self::at(dir.path().to_path_buf(), policy, Some(dir))
    }

    fn socket(&self) -> PathBuf {
        self.node.as_ref().unwrap().socket().unwrap().to_path_buf()
    }

    fn start_probe(&self) -> Result<v1::AgentStartResult, v1::ErrorInfo> {
        let mut c = Client::connect(&self.socket()).unwrap();
        c.call(Cmd::AgentStart(v1::AgentStart {
            kind: "probe".into(),
            ..Default::default()
        }))
        .unwrap()
        .map(|ok| match ok.result {
            Some(Ok_::AgentStart(s)) => s,
            other => panic!("{other:?}"),
        })
    }

    fn agents(&self) -> usize {
        let mut c = Client::connect(&self.socket()).unwrap();
        match c
            .call(Cmd::AgentList(v1::AgentList::default()))
            .unwrap()
            .unwrap()
            .result
        {
            Some(Ok_::AgentList(l)) => l.agents.len(),
            other => panic!("{other:?}"),
        }
    }
}

fn config(root: &Path) -> NodeConfig {
    NodeConfig {
        resume_all: false,
        node_id: "node-ctok".into(),
        socket: root.join("node.sock"),
        data_dir: root.join("data"),
        worktree_root: root.join("worktrees"),
        manifests_dir: root.join("manifests"),
        projects: vec![root.join("proj")],
        limits: NodeLimits::default(),
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        self.node.take();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn code_of(r: std::io::Result<Result<v1::CommandOk, v1::ErrorInfo>>) -> String {
    match r {
        Ok(Ok(_)) => "ok".into(),
        Ok(Err(e)) => e.code,
        Err(e) => format!("io:{:?}", e.kind()),
    }
}

/// What a process in a node's agent sees of the control socket; written to `probe-result` in the process's own folder.
fn probe(socket: &Path) -> String {
    let mut out = String::new();
    let token = token_path(socket);
    out += &format!("token_read={}\n", std::fs::read_to_string(&token).is_ok());
    out += &format!("token_listed={}\n", token.exists());
    let attempt = || {
        Cmd::AgentStart(v1::AgentStart {
            kind: "nothing".into(),
            launch_options: vec!["skip-permissions".into()],
            ..Default::default()
        })
    };
    match Client::connect_unauthenticated(socket) {
        Ok(mut c) => {
            out += "raw_connect=true\n";
            out += &format!("raw_start={}\n", code_of(c.call(attempt())));
            out += &format!(
                "raw_list={}\n",
                code_of(c.call(Cmd::AgentList(v1::AgentList::default())))
            );
        }
        Err(e) => out += &format!("raw_connect={:?}\n", e.kind()),
    }
    // What `agora` does: the client library reads the token itself, when it can.
    match Client::connect(socket) {
        Ok(mut c) => {
            out += &format!("cli_start={}\n", code_of(c.call(attempt())));
            out += &format!(
                "cli_list={}\n",
                code_of(c.call(Cmd::AgentList(v1::AgentList::default())))
            );
        }
        Err(e) => out += &format!("cli_connect={:?}\n", e.kind()),
    }
    out
}

/// The process the node starts as its confined (or unconfined) "agent": does nothing in a normal test run.
#[test]
fn confined_probe_child() {
    let Some(socket) = std::env::var_os("AGORA_PROBE_SOCKET") else {
        return;
    };
    let result = probe(Path::new(&socket));
    std::fs::write("probe-result.tmp", &result).unwrap();
    std::fs::rename("probe-result.tmp", "probe-result").unwrap();
    std::thread::sleep(Duration::from_secs(30));
}

fn probe_result(env: &Env, agent: &v1::AgentStartResult) -> Vec<(String, String)> {
    let file = Path::new(&agent.worktree_id).join("probe-result");
    let end = Instant::now() + Duration::from_secs(30);
    while !file.exists() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = env;
    std::fs::read_to_string(&file)
        .unwrap_or_else(|_| panic!("the probe wrote nothing at {file:?}"))
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
}

fn get<'a>(r: &'a [(String, String)], k: &str) -> &'a str {
    r.iter()
        .find(|(a, _)| a == k)
        .map(|(_, v)| v.as_str())
        .unwrap_or_else(|| panic!("no {k} in {r:?}"))
}

fn confining() -> bool {
    agora_platform::CONTROL_TOKEN_REQUIRED
        && agora_platform::sandbox_supported()
        && agora_platform::PTY_SUPPORTED
}

#[test]
fn a_confined_agent_connects_but_can_neither_read_the_token_nor_act() {
    if !confining() {
        eprintln!("skipped: the node does not require the control token here, or no Landlock");
        return;
    }
    let e = Env::new("conf", Some("sandbox = worktree\n"));
    let token = token_path(&e.socket());
    let secret = read_token(&token).expect("the node wrote its control token");
    assert_eq!(e.agents(), 0);
    let agent = e.start_probe().expect("the owner starts the probe agent");
    let r = probe_result(&e, &agent);
    // The premise of T16 holds: Landlock lets the confined process connect.
    assert_eq!(get(&r, "raw_connect"), "true");
    // The token is out of its reach (Landlock refuses the read) ...
    assert_eq!(get(&r, "token_read"), "false", "{r:?}");
    // ... so nothing it sends is answered: not the start with a launch option no signature covers, not the list, and not through the
    // client library either (which presents a token only if it can read one).
    assert_eq!(get(&r, "raw_start"), "auth.unauthenticated");
    assert_eq!(get(&r, "raw_list"), "auth.unauthenticated");
    assert_eq!(get(&r, "cli_start"), "auth.unauthenticated");
    assert_eq!(get(&r, "cli_list"), "auth.unauthenticated");
    // Nothing was started by it, and the token the owner holds did not change.
    assert_eq!(e.agents(), 1, "only the owner's probe agent exists");
    assert_eq!(read_token(&token).as_deref(), Some(secret.as_str()));
}

#[test]
fn the_owner_without_a_token_in_hand_is_served_by_the_client_and_refused_when_unauthenticated() {
    if !agora_platform::CONTROL_TOKEN_REQUIRED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let e = Env::new("owner", Some("sandbox = worktree\n"));
    let token = token_path(&e.socket());
    // The file is private to the owner and is 64 hex characters.
    let text = std::fs::read_to_string(&token).unwrap();
    assert_eq!(text.trim().len(), 64);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&token).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    // `agora`'s client: works, the contract does not change.
    assert_eq!(e.agents(), 0);
    // A peer that presents nothing, or a wrong token, gets `auth.unauthenticated`; the wrong token also ends the connection.
    let mut c = Client::connect_unauthenticated(&e.socket()).unwrap();
    assert_eq!(
        code_of(c.call(Cmd::AgentList(v1::AgentList::default()))),
        "auth.unauthenticated"
    );
    assert_eq!(
        code_of(c.call(Cmd::ControlAuth(v1::ControlAuth {
            token: "0".repeat(64)
        }))),
        "auth.unauthenticated"
    );
    assert!(
        c.call(Cmd::AgentList(v1::AgentList::default())).is_err(),
        "a wrong token ends the connection"
    );
    // A retry key does not hand an unauthenticated peer the stored answer of an authenticated one.
    let mut ok = Client::connect(&e.socket()).unwrap();
    assert_eq!(
        code_of(ok.call_with_key("k1", Cmd::AgentList(v1::AgentList::default()))),
        "ok"
    );
    let mut raw = Client::connect_unauthenticated(&e.socket()).unwrap();
    assert_eq!(
        code_of(raw.call_with_key("k1", Cmd::AgentList(v1::AgentList::default()))),
        "auth.unauthenticated"
    );
    // The right token, presented by hand, is accepted.
    let mut hand = Client::connect_unauthenticated(&e.socket()).unwrap();
    let t = read_token(&token).unwrap();
    assert_eq!(
        code_of(hand.call(Cmd::ControlAuth(v1::ControlAuth { token: t }))),
        "ok"
    );
    assert_eq!(
        code_of(hand.call(Cmd::AgentList(v1::AgentList::default()))),
        "ok"
    );
    // A token file that does not match (a stale one) is refused, not silently ignored.
    std::fs::write(&token, "a".repeat(64)).unwrap();
    let err = Client::connect(&e.socket()).err().expect("refused");
    assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
}

#[test]
fn without_confinement_nothing_changes_for_anyone() {
    // Negative control: the old behaviour (connect and act, no token) is still there when the node confines nothing.
    if !agora_platform::PTY_SUPPORTED || !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let e = Env::new("off", None);
    assert!(
        !token_path(&e.socket()).exists(),
        "no token file when no agent is confined"
    );
    let mut raw = Client::connect_unauthenticated(&e.socket()).unwrap();
    assert_eq!(
        code_of(raw.call(Cmd::AgentList(v1::AgentList::default()))),
        "ok"
    );
    // `ControlAuth` is accepted with any value (a client with a stale token file from an earlier confining run).
    assert_eq!(
        code_of(raw.call(Cmd::ControlAuth(v1::ControlAuth { token: "x".into() }))),
        "ok"
    );
    // The same probe an agent runs: the command is processed (the kind is unknown), not refused for want of a token.
    let r = probe_text_to_pairs(&probe(&e.socket()));
    assert_eq!(get(&r, "raw_start"), "agent.kind_unknown");
    assert_eq!(get(&r, "cli_start"), "agent.kind_unknown");
    assert_eq!(get(&r, "raw_list"), "ok");
}

fn probe_text_to_pairs(t: &str) -> Vec<(String, String)> {
    t.lines()
        .filter_map(|l| l.split_once('='))
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
}

#[test]
fn a_stale_token_file_is_replaced_at_start_and_a_second_node_leaves_it_alone() {
    if !agora_platform::CONTROL_TOKEN_REQUIRED || !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    private_home::init();
    let preferred =
        Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("ct-{}-stale", std::process::id()));
    let dir = short_socket::SocketDir::new(&preferred, "ct-stale");
    let root = dir.path().to_path_buf();
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("data")).unwrap();
    std::fs::create_dir_all(root.join("manifests")).unwrap();
    std::fs::create_dir_all(root.join("proj")).unwrap();
    git(&root.join("proj"), &["init", "-q", "-b", "main"]);
    std::fs::write(root.join("proj/a.txt"), "x").unwrap();
    git(&root.join("proj"), &["add", "."]);
    git(&root.join("proj"), &["commit", "-q", "-m", "init"]);
    std::fs::write(root.join("data/policy"), "sandbox = worktree\n").unwrap();
    // A file a crashed node left behind.
    let token = token_path(&root.join("node.sock"));
    std::fs::write(&token, "b".repeat(64)).unwrap();
    let first = Node::start(&config(&root)).unwrap();
    let fresh = read_token(&token).unwrap();
    assert_ne!(fresh, "b".repeat(64));
    // A second node on the same home fails to bind and does not touch the running node's token.
    assert!(Node::start(&config(&root)).is_err());
    assert_eq!(read_token(&token).as_deref(), Some(fresh.as_str()));
    assert!(Client::connect(first.socket().unwrap()).is_ok());
    // A clean stop removes the file; the next start writes a new one.
    drop(first);
    assert!(!token.exists());
    let again = Node::start(&config(&root)).unwrap();
    assert_ne!(read_token(&token).unwrap(), fresh);
    drop(again);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_node_home_inside_a_folder_agents_may_write_refuses_to_start_a_confined_agent() {
    if !confining() {
        return;
    }
    // `/tmp` is writable to every confined agent on Linux, so a node home in it would hand over the token.
    let e = Env::in_base(Path::new("/tmp"), "reach", Some("sandbox = worktree\n"));
    let err = e.start_probe().expect_err("refused");
    assert_eq!(err.code, "agent.sandbox_unavailable");
    assert!(
        err.params
            .iter()
            .any(|(k, v)| k == "reason" && format!("{v:?}").contains("control_token_reachable")),
        "{err:?}"
    );
    assert_eq!(e.agents(), 0);
    // With no confinement the same home is fine: nothing needs the token.
    let open = Env::in_base(Path::new("/tmp"), "reach-off", None);
    assert!(open.start_probe().is_ok());
}
