//! ADR-0079 W2 step 2 (Unix): the node starts its agents through the terminal holder, so they outlive it, and runs them itself when
//! there is no holder. A real node, a real `agora hold` and `/bin/sh` one-liners or `agora-fake-agent`; never a real agent.
//!
//! The "node" is this test process (the role lock, `run/node.lock`, names it, as the front door's lock names the node's supervisor). A
//! node that stops is [`Node`] dropped, which lets its held agents go and leaves them running.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use agora_core::AgentId;
use agora_node::control::Client;
use agora_node::daemon::{Node, NodeConfig};
use agora_node::holder::HolderSetup;
use agora_node::supervisor::NodeLimits;
use agora_protocol::v1::{self, command_ok::Result as Ok_, command_request::Command as Cmd};

const AGORA: &str = env!("CARGO_BIN_EXE_agora");

/// The tests share this process (its environment, the role lock it stands for): one at a time.
fn serial() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn alive(pid: u32) -> bool {
    agora_platform::process_start_time(pid).is_some()
}

fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(20);
    while !f() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// A home with one agent kind, `k`, whose launch command is `launch`.
struct Rig {
    root: PathBuf,
    folder: PathBuf,
}

impl Rig {
    fn new(tag: &str, launch: &[String]) -> Rig {
        static N: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "agora-hn-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("manifests")).expect("manifests");
        std::fs::create_dir_all(root.join("data")).expect("data");
        let folder = root.join("work");
        std::fs::create_dir_all(&folder).expect("work");
        let launch = serde_json::to_string(launch).expect("launch");
        std::fs::write(
            root.join("manifests/k.toml"),
            format!("kind = \"k\"\ndisplay_name = \"K\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = {launch}\n"),
        )
        .expect("manifest");
        agora_platform::ensure_private_dir(&root.join("run")).expect("run folder");
        let me = std::process::id();
        let start = agora_platform::process_start_time(me).expect("own start time");
        std::fs::write(root.join("run/node.lock"), format!("{me} {start} test\n")).expect("lock");
        Rig { root, folder }
    }

    fn cfg(&self) -> NodeConfig {
        NodeConfig {
            resume_all: false,
            node_id: "hold-node".into(),
            socket: self.root.join("n.sock"),
            data_dir: self.root.join("data"),
            worktree_root: self.root.join("worktrees"),
            manifests_dir: self.root.join("manifests"),
            projects: vec![],
            limits: NodeLimits::default(),
        }
    }

    fn setup(&self) -> HolderSetup {
        HolderSetup {
            home: self.root.clone(),
            exe: AGORA.into(),
        }
    }

    /// A node that starts agents through the holder.
    fn node(&self) -> Node {
        Node::start_with_holder(&self.cfg(), Some(self.setup())).expect("node")
    }

    /// A user-started agent of kind `k` in the work folder; returns its id.
    fn launch(&self, node: &Node) -> String {
        self.launch_with(node, &[])
    }

    /// [`Self::launch`] with these launch options, each confirmed (as the window does for one that skips permissions).
    fn launch_with(&self, node: &Node, options: &[&str]) -> String {
        let mut c = Client::connect(node.socket().unwrap()).expect("control");
        let info = match c
            .call(Cmd::DirectoryInspect(v1::DirectoryInspect {
                path: self.folder.to_string_lossy().into_owned(),
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
        match c
            .call(Cmd::AgentLaunch(v1::AgentLaunch {
                path: info.path.clone(),
                project_id: info.project_id.clone(),
                kind: "k".into(),
                launch_options: options.iter().map(|o| (*o).to_string()).collect(),
                confirmed_launch_options: options.iter().map(|o| (*o).to_string()).collect(),
                ..Default::default()
            }))
            .unwrap()
            .unwrap()
            .result
        {
            Some(Ok_::AgentLaunch(r)) => r.agent_id,
            other => panic!("{other:?}"),
        }
    }

    fn hold_pid_file(&self) -> PathBuf {
        self.root.join("run/hold.pid")
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        // What a test left running: the holder named in its file, ended with its agents (never by name or pattern).
        if let Ok(text) = std::fs::read_to_string(self.hold_pid_file()) {
            if let Some(pid) = text
                .split_whitespace()
                .next()
                .and_then(|p| p.parse::<u32>().ok())
            {
                let _ = agora_platform::terminate_process(pid);
            }
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn screen(node: &Node, id: &str) -> String {
    node.supervisor
        .lock()
        .unwrap()
        .terminal(&AgentId::new(id))
        .map(|t| t.snapshot().lines.join("\n"))
        .unwrap_or_default()
}

fn type_into(node: &Node, id: &str, text: &str) {
    node.supervisor
        .lock()
        .unwrap()
        .terminal(&AgentId::new(id))
        .expect("a terminal")
        .send_input(text.as_bytes().to_vec())
        .expect("input");
}

fn pid_of(node: &Node, id: &str) -> u32 {
    node.supervisor
        .lock()
        .unwrap()
        .agent_pids()
        .into_iter()
        .find(|(a, _)| a.as_str() == id)
        .map(|(_, p)| p)
        .expect("a running agent")
}

fn held(node: &Node, id: &str) -> bool {
    node.supervisor
        .lock()
        .unwrap()
        .held_agents()
        .0
        .iter()
        .any(|a| a.as_str() == id)
}

fn sh(script: &str) -> Vec<String> {
    vec!["/bin/sh".into(), "-c".into(), script.into()]
}

#[test]
fn an_agent_outlives_its_node_and_the_next_node_takes_it_up_with_the_output_written_meanwhile() {
    let _serial = serial();
    let rig = Rig::new(
        "outlive",
        &sh("echo ready; read a; echo got-$a; sleep 2; echo written-meanwhile; read b; echo got-$b; sleep 600"),
    );
    let node = rig.node();
    let id = rig.launch(&node);
    assert!(
        held(&node, &id),
        "the agent was not started through the holder"
    );
    wait_for("the first line", || screen(&node, &id).contains("ready"));
    type_into(&node, &id, "one\n");
    wait_for("the answer", || screen(&node, &id).contains("got-one"));
    let pid = pid_of(&node, &id);
    // The ledger records the holder (pid and start time) before the node goes away.
    wait_for("the holder in the ledger", || {
        node.ledger
            .get(&id)
            .is_some_and(|e| e.holder_pid != 0 && e.process().is_some())
    });
    drop(node);
    // The node is gone; the agent is not.
    std::thread::sleep(Duration::from_millis(500));
    assert!(alive(pid), "the agent ended with its node");
    std::thread::sleep(Duration::from_millis(2200));

    let node = rig.node();
    let views = node.supervisor.lock().unwrap().list();
    let view = views
        .iter()
        .find(|v| v.id.as_str() == id)
        .expect("the agent");
    assert!(
        view.running,
        "the agent is not running after the node came back"
    );
    assert!(held(&node, &id));
    assert_eq!(
        pid_of(&node, &id),
        pid,
        "a new process stands for the agent"
    );
    assert!(
        !node.ledger.get(&id).unwrap().interrupted(),
        "an agent the holder kept is not interrupted"
    );
    // What it wrote while no node was there arrives, and the terminal answers.
    wait_for("the output written meanwhile", || {
        screen(&node, &id).contains("written-meanwhile")
    });
    type_into(&node, &id, "two\n");
    wait_for("the answer after the reattach", || {
        screen(&node, &id).contains("got-two")
    });
    // The holder is the same process.
    let (_, holder) = node.supervisor.lock().unwrap().held_agents();
    assert!(holder.is_some());
}

/// A held agent that stays idle and silent while no node is there: its screen no longer shows what a rule reads (it cleared it), so
/// detection has nothing to speak from. The new node reports what the ledger knew (idle), not `unknown`.
#[test]
fn a_reattached_idle_agent_is_idle_not_unknown_even_when_its_screen_says_nothing() {
    let _serial = serial();
    // Two markers in the agent's working folder let the test order the clear between the two nodes.
    let rig = Rig::new(
        "idle",
        &sh("echo ready-prompt; while [ ! -e clear-screen ]; do sleep 0.1; done; printf '\\033[2J\\033[H' && : > screen-cleared; sleep 600"),
    );
    let manifest = rig.root.join("manifests/k.toml");
    let mut text = std::fs::read_to_string(&manifest).expect("manifest");
    text.push_str("[[rules]]\nid = \"prompt\"\nstate = \"idle\"\npriority = 100\nregion = \"bottom_non_empty_lines(12)\"\ncontains = \"ready-prompt\"\n");
    std::fs::write(&manifest, text).expect("manifest");
    let state_of = |node: &Node, id: &str| {
        node.supervisor
            .lock()
            .unwrap()
            .list()
            .into_iter()
            .find(|v| v.id.as_str() == id)
            .map(|v| v.state)
    };
    let node = rig.node();
    let id = rig.launch(&node);
    assert!(held(&node, &id), "the test agent must start in the holder");
    let holder = node
        .supervisor
        .lock()
        .unwrap()
        .held_agents()
        .1
        .expect("the holder's identity");
    wait_for("the idle state", || {
        state_of(&node, &id) == Some(agora_core::AgentState::Idle)
    });
    // The ledger can learn Idle before it records the holder. The next node needs both to restore the agent.
    let end = Instant::now() + Duration::from_secs(60);
    loop {
        let entry = node.ledger.get(&id);
        if entry
            .as_ref()
            .is_some_and(|e| e.last_state == "idle" && (e.holder_pid, e.holder_started) == holder)
        {
            break;
        }
        assert!(
            Instant::now() < end,
            "timed out waiting for idle and holder {holder:?} in the ledger; it recorded {:?}, supervisor state: {:?}",
            entry.map(|e| (e.last_state, e.holder_pid, e.holder_started)),
            state_of(&node, &id)
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    drop(node);
    std::fs::write(rig.folder.join("clear-screen"), b"").expect("request screen clear");
    let cleared = rig.folder.join("screen-cleared");
    wait_for(
        "the agent to clear its screen while the node is down",
        || cleared.is_file(),
    );

    let node = rig.node();
    assert!(
        held(&node, &id),
        "agent {id} did not reattach to holder {holder:?}; ledger: {:?}, supervisor state: {:?}",
        node.ledger.get(&id),
        state_of(&node, &id),
    );
    assert_eq!(
        state_of(&node, &id),
        Some(agora_core::AgentState::Idle),
        "a reattached idle agent lost its state"
    );
    // It stays so while the screen is blank (nothing replaces it without evidence).
    std::thread::sleep(Duration::from_millis(1500));
    assert!(!screen(&node, &id).contains("ready-prompt"));
    assert_eq!(
        state_of(&node, &id),
        Some(agora_core::AgentState::Idle),
        "the state was dropped while the agent stayed idle"
    );
}

#[test]
fn an_agent_started_by_a_node_without_a_holder_is_the_nodes_own_and_ends_with_it() {
    let _serial = serial();
    let rig = Rig::new("fallback", &sh("echo up; sleep 600"));
    // A holder that cannot start: the node runs its agents itself, exactly as it did before. Neither the node's start nor its first
    // launch may wait for a holder that is not there (a bound far above what they take, far below a hang).
    let began = Instant::now();
    let node = Node::start_with_holder(
        &rig.cfg(),
        Some(HolderSetup {
            home: rig.root.clone(),
            exe: rig.root.join("no-such-agora"),
        }),
    )
    .expect("node");
    assert!(
        began.elapsed() < Duration::from_secs(5),
        "the node's start waited for a holder: {:?}",
        began.elapsed()
    );
    let id = rig.launch(&node);
    assert!(
        began.elapsed() < Duration::from_secs(8),
        "the first launch waited for a holder: {:?}",
        began.elapsed()
    );
    assert!(
        !held(&node, &id),
        "started through a holder that was not there"
    );
    wait_for("the agent", || screen(&node, &id).contains("up"));
    let pid = pid_of(&node, &id);
    assert!(!rig.hold_pid_file().exists(), "a holder was left behind");
    drop(node);
    wait_for("the node's own agent to end with it", || !alive(pid));
}

#[test]
fn the_agent_gets_the_whole_environment_the_node_sent_and_none_of_the_holders_own() {
    let _serial = serial();
    let rig = Rig::new(
        "env",
        &sh("echo VAR=[$AGORA_HOLD_NODE_TEST_VAR] STALE=[$AGORA_HOLD_STALE_VAR]; sleep 600"),
    );
    let node = rig.node();
    // The first agent starts the holder, whose environment is then a snapshot of this process as it is now.
    std::env::set_var("AGORA_HOLD_STALE_VAR", "stale");
    let first = rig.launch(&node);
    assert!(held(&node, &first));
    // Changed after the holder began: only what the node sends with the start can reach the next agent.
    std::env::remove_var("AGORA_HOLD_STALE_VAR");
    std::env::set_var("AGORA_HOLD_NODE_TEST_VAR", "from-the-node");
    let second = rig.launch(&node);
    assert!(held(&node, &second));
    wait_for("the listing", || {
        screen(&node, &second).contains("VAR=[from-the-node]")
    });
    let seen = screen(&node, &second);
    assert!(
        seen.contains("STALE=[]"),
        "the holder's own environment reached the agent: {seen}"
    );
    std::env::remove_var("AGORA_HOLD_NODE_TEST_VAR");
}

#[test]
fn an_agent_that_ends_while_the_node_is_attached_is_collected_and_the_holder_ends_with_its_last() {
    let _serial = serial();
    let rig = Rig::new("ends", &sh("echo done-here; sleep 1; exit 0"));
    let node = rig.node();
    let id = rig.launch(&node);
    assert!(held(&node, &id));
    wait_for("the agent to end", || {
        node.supervisor
            .lock()
            .unwrap()
            .list()
            .iter()
            .any(|v| v.id.as_str() == id && !v.running)
    });
    // Its status was taken (`Release`), and a holder with nothing left ends.
    wait_for("the holder to end", || !rig.hold_pid_file().exists());
}

#[test]
fn stopping_a_held_agent_ends_its_process() {
    let _serial = serial();
    let rig = Rig::new("stop", &sh("echo up; sleep 600"));
    let node = rig.node();
    let id = rig.launch(&node);
    wait_for("the agent", || screen(&node, &id).contains("up"));
    let pid = pid_of(&node, &id);
    node.supervisor
        .lock()
        .unwrap()
        .stop(&AgentId::new(id.clone()))
        .expect("stop");
    wait_for("the process to end", || !alive(pid));
    wait_for("the holder to end", || !rig.hold_pid_file().exists());
}

fn agora(rig: &Rig, args: &[&str]) -> (bool, String) {
    let out = Command::new(AGORA)
        .args(args)
        .env("AGORA_HOME", &rig.root)
        .env("AGORA_LANG", "en")
        .output()
        .expect("run agora");
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

#[test]
fn down_keeps_the_agents_and_down_agents_stops_them() {
    let _serial = serial();
    let rig = Rig::new("down", &sh("echo up; sleep 600"));
    let node = rig.node();
    let id = rig.launch(&node);
    wait_for("the agent", || screen(&node, &id).contains("up"));
    let pid = pid_of(&node, &id);
    wait_for("the holder to say it keeps one", || {
        std::fs::read_to_string(rig.hold_pid_file()).is_ok_and(|t| t.trim_end().ends_with(" 1"))
    });
    drop(node);
    // No lock names a live node any more (the test process stands for it); `agora down` finds only the holder.
    std::fs::remove_file(rig.root.join("run/node.lock")).expect("lock");

    let (ok, text) = agora(&rig, &["down"]);
    assert!(ok, "{text}");
    assert!(text.contains("1 agent keeps running"), "{text}");
    assert!(alive(pid), "plain `agora down` ended an agent");

    let (ok, text) = agora(&rig, &["down", "--agents"]);
    assert!(ok, "{text}");
    assert!(text.contains("Stopped 1 agent"), "{text}");
    wait_for("the agent to end", || !alive(pid));
    assert!(!rig.hold_pid_file().exists(), "the holder was left running");
}

/// A node that started, ran an agent and stopped; then `change` is done to what the next node will find; then a node starts. Returns the
/// agent's pid.
fn next_node_finds(tag: &str, change: impl FnOnce(&Rig)) -> (Rig, Node, u32) {
    let rig = Rig::new(tag, &sh("echo up; sleep 600"));
    let node = rig.node();
    let id = rig.launch(&node);
    wait_for("the agent", || screen(&node, &id).contains("up"));
    let pid = pid_of(&node, &id);
    wait_for("the holder in the ledger", || {
        node.ledger.get(&id).is_some_and(|e| e.holder_pid != 0)
    });
    drop(node);
    std::thread::sleep(Duration::from_millis(300));
    assert!(alive(pid));
    change(&rig);
    let node = rig.node();
    (rig, node, pid)
}

/// An agent the holder kept that the next node does not take up (nothing in the ledger knows it, or its folder is gone) must not be
/// left attached and unread: the attach made the holder stop reading it, so it would block on a full terminal for good. It is ended.
#[test]
fn a_held_agent_the_ledger_does_not_know_is_ended_not_left_unread() {
    let _serial = serial();
    let (rig, node, pid) = next_node_finds("unknown", |rig| {
        std::fs::remove_file(rig.root.join("data/sessions/ledger.json")).expect("ledger");
    });
    wait_for("the unknown agent to be ended", || !alive(pid));
    wait_for("the holder to end", || !rig.hold_pid_file().exists());
    // The holder is usable for the next agent.
    let id = rig.launch(&node);
    assert!(held(&node, &id));
}

#[test]
fn a_held_agent_whose_folder_is_gone_is_ended_not_left_unread() {
    let _serial = serial();
    let (rig, node, pid) = next_node_finds("nofolder", |rig| {
        std::fs::remove_dir_all(&rig.folder).expect("folder");
    });
    wait_for("the agent to be ended", || !alive(pid));
    wait_for("the holder to end", || !rig.hold_pid_file().exists());
    assert!(node.supervisor.lock().unwrap().list().is_empty());
}

/// The holder starts the agent but the answer never reaches the node: the node must find that agent and end it before it starts again,
/// never run a second copy.
#[test]
fn a_start_whose_answer_is_lost_leaves_one_agent_not_two() {
    let _serial = serial();
    let rig = Rig::new("lost", &sh("echo $$ >> ../pids; echo up; sleep 600"));
    let node = rig.node();
    // The first agent starts the holder; the second one's answer is lost.
    let first = rig.launch(&node);
    wait_for("the first agent", || screen(&node, &first).contains("up"));
    node.supervisor
        .lock()
        .unwrap()
        .holder_hub()
        .expect("a holder")
        .lose_next_start_reply();
    let second = rig.launch(&node);
    wait_for("the second agent", || screen(&node, &second).contains("up"));
    let pids: Vec<u32> = std::fs::read_to_string(rig.root.join("pids"))
        .expect("pids")
        .lines()
        .map(|l| l.trim().parse().expect("a pid"))
        .collect();
    // The first agent's, the lost one's (only when it ran far enough to write it: it is killed at once, and a slow machine, macOS in CI,
    // can end it before its shell wrote anything) and the one that was started again. A second copy would be a live pid beyond two.
    assert!(pids.len() >= 2, "{pids:?}");
    let running: Vec<&u32> = pids.iter().filter(|p| alive(**p)).collect();
    assert_eq!(running.len(), 2, "{pids:?}");
    assert!(alive(pids[0]) && alive(*pids.last().unwrap()), "{pids:?}");
    assert!(held(&node, &second));
}

#[test]
fn an_environment_that_cannot_be_sent_makes_the_node_run_the_agent_itself() {
    let _serial = serial();
    use std::os::unix::ffi::OsStrExt;
    let rig = Rig::new("badenv", &sh("echo up; sleep 600"));
    let node = rig.node();
    std::env::set_var(
        "AGORA_HOLD_NOT_UTF8",
        std::ffi::OsStr::from_bytes(b"caf\xe9"),
    );
    let id = rig.launch(&node);
    std::env::remove_var("AGORA_HOLD_NOT_UTF8");
    assert!(
        !held(&node, &id),
        "started through the holder with a smaller environment than the node's own"
    );
    wait_for("the agent", || screen(&node, &id).contains("up"));
}

/// A hand-run `agora mesh stdio` bridge (the MCP server an agent's client starts) on an agent's mesh socket.
struct Bridge {
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
    lines: std::sync::mpsc::Receiver<serde_json::Value>,
    next: u64,
}

impl Bridge {
    fn start(socket: &std::path::Path) -> Bridge {
        use std::io::BufRead;
        use std::process::Stdio;
        let mut child = Command::new(AGORA)
            .args(["mesh", "stdio", "--socket"])
            .arg(socket)
            .env("AGORA_LANG", "en")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("run the bridge");
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("stdout");
        let (tx, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout).lines() {
                let Ok(line) = line else { return };
                let Ok(v) = serde_json::from_str(&line) else {
                    return;
                };
                if tx.send(v).is_err() {
                    return;
                }
            }
        });
        Bridge {
            child,
            stdin,
            lines,
            next: 0,
        }
    }

    fn send(&mut self, msg: &serde_json::Value) {
        use std::io::Write;
        let stdin = self.stdin.as_mut().expect("stdin");
        writeln!(stdin, "{msg}").expect("write");
        stdin.flush().expect("flush");
    }

    fn request(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        self.next += 1;
        let id = self.next;
        self.send(&serde_json::json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}));
        let a = self
            .lines
            .recv_timeout(Duration::from_secs(10))
            .unwrap_or_else(|_| panic!("no answer to {method}"));
        assert_eq!(a["id"], id, "{a}");
        a
    }

    fn initialize(&mut self) {
        let a = self.request(
            "initialize",
            serde_json::json!({"protocolVersion":"2025-03-26"}),
        );
        assert!(a["result"].is_object(), "{a}");
        self.send(&serde_json::json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    }

    fn workspace_status(&mut self) -> serde_json::Value {
        self.request(
            "tools/call",
            serde_json::json!({"name": "workspace_status", "arguments": {}}),
        )
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn mesh_unavailable(a: &serde_json::Value) -> bool {
    a["error"]["data"]["code"] == "mesh.unavailable"
}

/// What a hub folds from everything this node recorded, its earlier starts included.
fn hub_view(node: &Node) -> agora_hub::projection::View {
    let mut view = agora_hub::projection::View::default();
    for ev in node.events.events_after(0) {
        view.apply(&ev);
    }
    view
}

/// #906 and #907 (RM-30, RM-34): an agent the holder kept across a node restart is the next node's in full. Its mesh socket is served
/// again at the same path, so a bridge that was open across the restart reconnects and its next call answers, and a new bridge
/// connects; and the hub, which the new node's `NodeOnline` told that everything it ran is gone, shows it running with its terminal,
/// not finished, and still acting without asking (it was started with an option that skips permissions, and it is the same process).
#[test]
fn a_kept_agent_is_on_the_mesh_again_and_shown_running_after_its_node_restarts() {
    let _serial = serial();
    let rig = Rig::new("kept", &sh("echo up; sleep 600"));
    let manifest = rig.root.join("manifests/k.toml");
    // Launch options need engine 2 (agent-manifest.md).
    let mut text = std::fs::read_to_string(&manifest)
        .expect("manifest")
        .replace("min_engine = 1", "min_engine = 2");
    text.push_str("[[launch.option]]\nid = \"skip\"\nlabel = \"option-skip\"\nargs = [\"--yes\"]\nskips_permissions = true\n");
    std::fs::write(&manifest, text).expect("manifest");
    let node = rig.node();
    let id = rig.launch_with(&node, &["skip"]);
    assert!(held(&node, &id), "the test agent must start in the holder");
    wait_for("the agent", || screen(&node, &id).contains("up"));
    let pid = pid_of(&node, &id);
    wait_for("the holder in the ledger", || {
        node.ledger.get(&id).is_some_and(|e| e.holder_pid != 0)
    });
    let socket = rig.root.join("m").join(format!("{id}.sock"));
    let mut bridge = Bridge::start(&socket);
    bridge.initialize();
    let a = bridge.workspace_status();
    assert!(a["result"].is_object(), "before the restart: {a}");
    let before = hub_view(&node);
    let key = agora_hub::projection::qualified("hold-node", &id);
    assert!(before.agents[&key].running);
    assert!(
        before.agents[&key].skips_permissions,
        "the test agent must start skipping permissions"
    );

    // A node's process that ends closes its agents' mesh endpoints with it. This test process stands for the node and lives on (other
    // parts still hold the old node's mesh), so the endpoint is closed here, as the end of the process would.
    node.mesh.unregister(&AgentId::new(id.clone()));
    drop(node);
    assert!(alive(pid), "the agent ended with its node");
    let a = bridge.workspace_status();
    assert!(mesh_unavailable(&a), "with no node: {a}");

    let node = rig.node();
    assert_eq!(pid_of(&node, &id), pid, "the agent was not taken up");
    // The open bridge reconnects to the same path (its backoff is at most 2 s) and the call answers.
    let end = Instant::now() + Duration::from_secs(15);
    let a = loop {
        let a = bridge.workspace_status();
        if !mesh_unavailable(&a) {
            break a;
        }
        assert!(
            Instant::now() < end,
            "the bridge never reconnected: the kept agent's mesh socket is not served"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(a["result"].is_object(), "after the restart: {a}");
    assert!(a.get("error").is_none(), "after the restart: {a}");
    // A bridge started now connects too.
    let mut fresh = Bridge::start(&socket);
    fresh.initialize();
    let a = fresh.workspace_status();
    assert!(a["result"].is_object(), "a new bridge: {a}");

    // The hub's view: running, not finished, with its terminal.
    let view = hub_view(&node);
    let agent = &view.agents[&key];
    assert!(agent.running, "the hub shows the kept agent finished");
    assert_ne!(agent.state, v1::AgentState::Done);
    assert!(agent.interrupted.is_none());
    assert!(
        agent.skips_permissions,
        "the kept agent acts without asking but is shown as if it asks"
    );
    assert_eq!(agent.launch_options, vec!["skip".to_string()]);
    assert!(node
        .supervisor
        .lock()
        .unwrap()
        .skips_permissions_now(&AgentId::new(id.clone())));
    let terminal = agora_node::control::terminal_id_of(&AgentId::new(&id));
    assert!(
        view.terminals
            .contains_key(&agora_hub::projection::qualified("hold-node", &terminal)),
        "the hub has no terminal for the kept agent"
    );
}

impl Bridge {
    fn tool(&mut self, name: &str, arguments: serde_json::Value) -> serde_json::Value {
        self.request(
            "tools/call",
            serde_json::json!({"name": name, "arguments": arguments}),
        )
    }

    /// The JSON a tool answered with (its one text item), or the error it answered with.
    fn tool_json(&mut self, name: &str, arguments: serde_json::Value) -> serde_json::Value {
        let a = self.tool(name, arguments);
        let text = a["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("{name}: {a}"))
            .to_string();
        serde_json::from_str(&text).unwrap_or_else(|_| panic!("{name}: {a}"))
    }
}

fn inbox_bodies(inbox: &serde_json::Value) -> Vec<String> {
    inbox
        .as_array()
        .unwrap_or_else(|| panic!("{inbox}"))
        .iter()
        .map(|m| m["body"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// The `inbox` call an agent's client keeps making across the node's restart: messages after the last one it read.
fn read_after(bridge: &mut Bridge, cursor: &str) -> serde_json::Value {
    bridge.tool_json("inbox", serde_json::json!({"since": cursor}))
}

/// #965 (RM-30): an agent the holder kept across a node restart hears the mesh again, also through the cursor its client carried
/// over. Before the restart it read its inbox up to some `message_id`; the restarted node's broker is new, and its ids must not
/// start over below that cursor, or every message for the kept agent (from an agent on its machine, a presence notice, one from
/// another machine) is hidden from `inbox(since)` while the senders are told it was accepted.
#[test]
fn a_kept_agent_hears_the_mesh_after_its_node_restarts_through_the_cursor_it_carried() {
    let _serial = serial();
    let rig = Rig::new("inbox", &sh("echo up; sleep 600"));
    let socket = |id: &str| rig.root.join("m").join(format!("{id}.sock"));
    let node = rig.node();
    let kept = rig.launch(&node);
    let peer = rig.launch(&node);
    for id in [&kept, &peer] {
        assert!(held(&node, id), "the test agents must start in the holder");
        wait_for("the agent", || screen(&node, id).contains("up"));
        wait_for("the holder in the ledger", || {
            node.ledger.get(id).is_some_and(|e| e.holder_pid != 0)
        });
    }
    let mut bridge = Bridge::start(&socket(&kept));
    bridge.initialize();
    let mut pb = Bridge::start(&socket(&peer));
    pb.initialize();
    // Enough traffic before the restart that the kept agent's cursor is past the ids a new broker would hand out first.
    for n in 0..8 {
        let a = pb.tool_json(
            "send_message",
            serde_json::json!({"to": kept, "body": format!("before {n}")}),
        );
        assert!(a["message_id"].is_string(), "{a}");
    }
    let before = bridge.tool_json("inbox", serde_json::json!({}));
    assert_eq!(inbox_bodies(&before).len(), 9, "{before}");
    let cursor = before[8]["message_id"].as_str().expect("id").to_string();
    assert!(read_after(&mut bridge, &cursor)
        .as_array()
        .unwrap()
        .is_empty());

    // The node stops (its endpoints close, as its process ending would) and the next one takes both agents up.
    node.mesh.unregister(&AgentId::new(kept.clone()));
    node.mesh.unregister(&AgentId::new(peer.clone()));
    drop(node);
    let node = rig.node();
    assert!(node.mesh.is_registered(&AgentId::new(kept.clone())));
    let end = Instant::now() + Duration::from_secs(15);
    while mesh_unavailable(&bridge.workspace_status()) {
        assert!(Instant::now() < end, "the bridge never reconnected");
        std::thread::sleep(Duration::from_millis(100));
    }

    // (b) A fresh agent joins: the kept agent gets its presence notice.
    let fresh = rig.launch(&node);
    wait_for("the fresh agent", || screen(&node, &fresh).contains("up"));
    // (a) A message from an agent on the same machine.
    let mut fb = Bridge::start(&socket(&fresh));
    fb.initialize();
    let sent = fb.tool_json(
        "send_message",
        serde_json::json!({"to": kept, "body": "from the fresh agent"}),
    );
    assert!(sent["message_id"].is_string(), "{sent}");
    // (c) A message from an agent on another machine, as the hub hands it to this node (`MeshPost`).
    node.mesh
        .deliver_remote(
            "other-node",
            "agent-9",
            &AgentId::new(kept.clone()),
            "from another machine",
            &[],
        )
        .expect("the remote message is accepted");

    let after = read_after(&mut bridge, &cursor);
    let bodies = inbox_bodies(&after);
    assert!(
        bodies
            .iter()
            .any(|b| b.contains(&fresh) && b.contains("joined")),
        "no presence notice for the fresh agent after the cursor: {after}"
    );
    assert!(
        bodies.iter().any(|b| b == "from the fresh agent"),
        "the local message is hidden behind the carried cursor: {after}"
    );
    assert!(
        bodies.iter().any(|b| b == "from another machine"),
        "the cross-node message is hidden behind the carried cursor: {after}"
    );
    // Nothing from before the restart comes back as new.
    assert!(!bodies.iter().any(|b| b.starts_with("before ")), "{after}");
    // The fresh agent's own cursor works the same way: its next read after what it saw is empty.
    let seen = fb.tool_json("inbox", serde_json::json!({}));
    if let Some(last) = seen.as_array().and_then(|a| a.last()) {
        let id = last["message_id"].as_str().expect("id").to_string();
        assert!(read_after(&mut fb, &id).as_array().unwrap().is_empty());
    }
}

/// Plain git with an identity, none of a calling git's repository variables; its output.
fn plain_git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .env_remove("GIT_DIR")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .expect("git");
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// An agent started in a worktree the node made from a checkout, kept by the holder while no node runs, with `change` made to its
/// worktree meanwhile; the next node, the agent's id and its process.
fn next_node_finds_in_worktree(tag: &str, change: impl FnOnce(&Path)) -> (Rig, Node, String, u32) {
    let rig = Rig::new(tag, &sh("echo up; sleep 600"));
    let repo = rig.root.join("repo");
    std::fs::create_dir_all(&repo).expect("repo");
    plain_git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "one\n").expect("file");
    plain_git(&repo, &["add", "-A"]);
    plain_git(&repo, &["commit", "-q", "-m", "first"]);
    let cfg = NodeConfig {
        projects: vec![repo],
        ..rig.cfg()
    };
    let node = Node::start_with_holder(&cfg, Some(rig.setup())).expect("node");
    let mut c = Client::connect(node.socket().unwrap()).expect("control");
    let id = match c
        .call(Cmd::AgentStart(v1::AgentStart {
            kind: "k".into(),
            ..Default::default()
        }))
        .unwrap()
        .unwrap()
        .result
    {
        Some(Ok_::AgentStart(r)) => r.agent_id,
        other => panic!("{other:?}"),
    };
    wait_for("the agent", || screen(&node, &id).contains("up"));
    let pid = pid_of(&node, &id);
    wait_for("the holder in the ledger", || {
        node.ledger.get(&id).is_some_and(|e| e.holder_pid != 0)
    });
    let worktree = PathBuf::from(node.ledger.get(&id).expect("entry").folder);
    drop(node);
    std::thread::sleep(Duration::from_millis(300));
    assert!(alive(pid));
    // A node process that restarts has no record of the worktrees an earlier one made; this one is the same process.
    agora_node::git::forget_made_worktree(&worktree);
    change(&worktree);
    let node = Node::start_with_holder(&cfg, Some(rig.setup())).expect("node");
    (rig, node, id, pid)
}

#[test]
fn a_held_agent_in_a_node_made_worktree_is_taken_up_again() {
    let _serial = serial();
    let (_rig, node, id, pid) = next_node_finds_in_worktree("wt-same", |_| {});
    assert!(held(&node, &id), "not taken up");
    assert_eq!(pid_of(&node, &id), pid);
}

/// T8b: a node-made worktree whose branch was given a root commit of its own while no node ran (by its agent, another agent of the
/// repository, or an unrelated merge) does not make its running agent another project's, nor end it: it is taken up in the project the
/// ledger recorded when the node started it.
#[test]
fn a_held_agent_whose_worktree_branch_moved_is_taken_up_in_its_recorded_project() {
    let _serial = serial();
    let mut before = String::new();
    let (_rig, node, id, pid) = next_node_finds_in_worktree("wt-moved", |wt| {
        before = plain_git(wt, &["rev-list", "--max-parents=0", "HEAD"]);
        let tree = plain_git(wt, &["rev-parse", "HEAD^{tree}"]);
        let root = plain_git(wt, &["commit-tree", &tree, "-m", "another root"]);
        plain_git(wt, &["update-ref", "HEAD", &root]);
    });
    assert!(held(&node, &id), "not taken up");
    assert_eq!(pid_of(&node, &id), pid);
    assert!(alive(pid));
    let views = node.supervisor.lock().unwrap().list();
    let view = views
        .iter()
        .find(|v| v.id.as_str() == id)
        .expect("the agent");
    assert_eq!(
        view.project,
        format!("#{before}"),
        "taken up as another project's"
    );
}
