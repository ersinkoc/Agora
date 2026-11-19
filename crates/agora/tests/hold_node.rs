//! ADR-0079 W2 step 2 (Unix): the node starts its agents through the terminal holder, so they outlive it, and runs them itself when
//! there is no holder. A real node, a real `agora hold` and `/bin/sh` one-liners or `agora-fake-agent`; never a real agent.
//!
//! The "node" is this test process (the role lock, `run/node.lock`, names it, as the front door's lock names the node's supervisor). A
//! node that stops is [`Node`] dropped, which lets its held agents go and leaves them running.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
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
