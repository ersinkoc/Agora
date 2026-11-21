//! ADR-0093 (Unix): a structured agent (an adapter on pipes) outlives its node when the holder runs it, and the next node takes it up with
//! what it wrote meanwhile and the request that was waiting for the person. A real node, a real `agora hold` and the fake adapter
//! (`agora-fake-agent --adapter`); never a real agent.
//!
//! The "node" is this test process (the role lock names it, as in `hold_node.rs`). A node that stops is [`Node`] dropped, which lets
//! a held adapter go and leaves it running: to the holder this is a link that closed, as when a node is killed.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use agora_core::{AgentId, AgentState};
use agora_node::control::Client;
use agora_node::daemon::{Node, NodeConfig};
use agora_node::holder::HolderSetup;
use agora_node::supervisor::NodeLimits;
use agora_protocol::v1::{self, command_ok::Result as Ok_, command_request::Command as Cmd};

const AGORA: &str = env!("CARGO_BIN_EXE_agora");

fn serial() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn fake_agent() -> String {
    let mut p = PathBuf::from(AGORA);
    p.pop();
    let target = p.parent().map(Path::to_path_buf).unwrap_or_default();
    p.push(format!("agora-fake-agent{}", std::env::consts::EXE_SUFFIX));
    static FRESH: std::sync::Once = std::sync::Once::new();
    FRESH.call_once(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        assert!(
            Command::new(cargo)
                .args(["build", "-q", "-p", "agora-fake-agent", "--target-dir"])
                .arg(&target)
                .status()
                .expect("run cargo")
                .success(),
            "building agora-fake-agent failed"
        );
    });
    p.to_string_lossy().into_owned()
}

fn alive(pid: u32) -> bool {
    agora_platform::process_start_time(pid).is_some()
}

fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(30);
    while !f() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// A home with one agent kind, `k`, hosted by the fake adapter.
struct Rig {
    root: PathBuf,
    folder: PathBuf,
}

impl Rig {
    fn new(tag: &str) -> Rig {
        Self::with_args(tag, &[])
    }

    /// [`Self::new`] with more arguments for the fake adapter (`--modes`).
    fn with_args(tag: &str, args: &[&str]) -> Rig {
        static N: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "agora-hnp-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("manifests")).expect("manifests");
        std::fs::create_dir_all(root.join("data")).expect("data");
        let folder = root.join("work");
        std::fs::create_dir_all(&folder).expect("work");
        let mut command = vec![fake_agent(), "--adapter".to_string()];
        command.extend(args.iter().map(|a| (*a).to_string()));
        let command = serde_json::to_string(&command).expect("cmd");
        std::fs::write(
            root.join("manifests/k.toml"),
            format!("kind = \"k\"\ndisplay_name = \"K\"\nversion = \"1\"\nmin_engine = 1\n[adapter]\ncommand = {command}\n"),
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

    fn node(&self) -> Node {
        Node::start_with_holder(&self.cfg(), Some(self.setup())).expect("node")
    }

    /// A node with no holder: its adapters are its own.
    fn node_without_holder(&self) -> Node {
        Node::start_with_holder(&self.cfg(), None).expect("node")
    }

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
}

impl Drop for Rig {
    fn drop(&mut self) {
        // What a test left running: the holder named in its file, ended with its agents (never by name or pattern).
        if let Ok(text) = std::fs::read_to_string(self.root.join("run/hold.pid")) {
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

fn held(node: &Node, id: &str) -> bool {
    node.supervisor
        .lock()
        .unwrap()
        .held_agents()
        .0
        .iter()
        .any(|a| a.as_str() == id)
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

fn state_of(node: &Node, id: &str) -> Option<AgentState> {
    node.supervisor
        .lock()
        .unwrap()
        .list()
        .into_iter()
        .find(|v| v.id.as_str() == id)
        .map(|v| v.state)
}

fn said(node: &Node, id: &str, word: &str) -> bool {
    node.chat
        .snapshot(&AgentId::new(id))
        .iter()
        .any(|e| e.text == word)
}

fn prompt(node: &Node, id: &str, text: &str) {
    node.supervisor
        .lock()
        .unwrap()
        .adapter_prompt(&AgentId::new(id), text, false)
        .expect("prompt");
}

/// Starts an agent and waits until it is idle and its holder, session and capabilities are in the ledger (what the next node needs).
fn started(rig: &Rig, node: &Node) -> String {
    let id = rig.launch(node);
    assert!(
        held(node, &id),
        "the adapter was not started through the holder"
    );
    wait_for("the agent to be idle", || {
        state_of(node, &id) == Some(AgentState::Idle)
    });
    wait_for("the holder and the session in the ledger", || {
        node.ledger.get(&id).is_some_and(|e| {
            e.holder_pid != 0 && !e.adapter_session.is_empty() && !e.adapter_capabilities.is_empty()
        })
    });
    id
}

/// ADR-0093 decisions 1 and 3: a turn is running with a permission request waiting for the person when the node ends. The adapter keeps
/// running, the next node takes it up (the same process, not initialised again), the request is asked again and is not answered, and
/// the person's answer reaches the agent, which finishes the turn.
#[test]
fn a_structured_agent_mid_turn_outlives_its_node_and_its_waiting_request_is_asked_again() {
    let _serial = serial();
    let rig = Rig::new("ask");
    let node = rig.node();
    let id = started(&rig, &node);
    prompt(&node, &id, "ask");
    wait_for("the request to be waiting", || {
        node.approvals.pending_count() == 1
    });
    wait_for("the agent to be blocked", || {
        state_of(&node, &id) == Some(AgentState::Blocked)
    });
    let pid = pid_of(&node, &id);
    drop(node);
    // The node is gone; the adapter and the turn are not.
    std::thread::sleep(Duration::from_millis(500));
    assert!(alive(pid), "the adapter ended with its node");

    let node = rig.node();
    assert!(held(&node, &id), "the agent was not taken up again");
    assert_eq!(pid_of(&node, &id), pid, "a new process stands for it");
    assert!(
        !node.ledger.get(&id).unwrap().interrupted(),
        "an agent the holder kept is not interrupted"
    );
    // Rebuilt from the replayed line: the person is asked again.
    wait_for("the request to be asked again", || {
        node.approvals.pending_count() == 1
    });
    // Never answered by the node, and the agent is still waiting.
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(node.approvals.pending_count(), 1);
    assert!(!said(&node, &id, "approved") && !said(&node, &id, "denied"));
    assert_eq!(state_of(&node, &id), Some(AgentState::Blocked));
    // The answer reaches the same agent (the node writes to the process through the holder), which finishes its turn.
    node.supervisor
        .lock()
        .unwrap()
        .adapter_resolve_approval(&AgentId::new(id.clone()), "ap-1", true)
        .expect("resolve");
    wait_for("the turn to finish", || said(&node, &id, "approved"));
    wait_for("the agent to be idle", || {
        state_of(&node, &id) == Some(AgentState::Idle)
    });
    // Another prompt works: the live stream goes both ways.
    prompt(&node, &id, "after");
    wait_for("the next answer", || said(&node, &id, "after"));
}

/// The request comes while no node is there at all: the next node finds it among what nobody had read, and asks.
#[test]
fn a_request_made_while_no_node_ran_is_waiting_for_the_next_node() {
    let _serial = serial();
    let rig = Rig::new("later");
    let node = rig.node();
    let id = started(&rig, &node);
    prompt(&node, &id, "later-ask");
    let pid = pid_of(&node, &id);
    drop(node);
    // The adapter asks 1.5 s after the prompt, with the node gone: it says so by writing a file (no guess at how long that took).
    wait_for("the adapter to have asked", || {
        rig.folder.join("later.txt").exists()
    });
    assert!(alive(pid));

    let node = rig.node();
    wait_for("the request to be waiting", || {
        node.approvals.pending_count() == 1
    });
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(node.approvals.pending_count(), 1);
    assert!(!said(&node, &id, "approved") && !said(&node, &id, "denied"));
    node.supervisor
        .lock()
        .unwrap()
        .adapter_resolve_approval(&AgentId::new(id.clone()), "ap-1", false)
        .expect("resolve");
    wait_for("the turn to finish", || said(&node, &id, "denied"));
}

/// ADR-0093 decision 1: an agent that writes more than the holder keeps while no node reads has its newest output kept and a notice in
/// the conversation saying how much was lost.
#[test]
fn output_beyond_what_the_holder_keeps_shows_a_notice_with_the_dropped_bytes() {
    let _serial = serial();
    let rig = Rig::new("flood");
    let node = rig.node();
    let id = started(&rig, &node);
    prompt(&node, &id, "later-flood");
    drop(node);
    // 3 MB written 1.5 s after the prompt, with no node there; the adapter writes a file when it is done.
    wait_for("the adapter to have written it all", || {
        rig.folder.join("later.txt").exists()
    });

    let node = rig.node();
    wait_for("the newest output", || said(&node, &id, "flood-end"));
    wait_for("the notice", || {
        node.chat
            .snapshot(&AgentId::new(id.clone()))
            .iter()
            .any(|e| e.code == "adapter.output_dropped")
    });
    let chat = node.chat.snapshot(&AgentId::new(id.clone()));
    let notice = chat
        .iter()
        .find(|e| e.code == "adapter.output_dropped")
        .expect("a notice that output was dropped");
    let bytes: u64 = notice.params["bytes"].parse().expect("a count");
    assert!(bytes > 1_000_000, "dropped only {bytes} bytes");
    // What was kept is the end of it, and fewer than were written.
    let items = chat.iter().filter(|e| e.text.starts_with('f')).count();
    assert!(items < 3000 && items > 0, "{items} items kept");
    assert!(chat.iter().any(|e| e.text.starts_with("f2999:")));
}

/// Without a holder the adapter is the node's own, as before: it ends with the node and the person is offered resume (ADR-0060).
#[test]
fn without_a_holder_an_adapter_is_the_nodes_own_and_ends_with_it() {
    let _serial = serial();
    let rig = Rig::new("none");
    let node = rig.node_without_holder();
    let id = rig.launch(&node);
    assert!(!held(&node, &id));
    wait_for("the agent to be idle", || {
        state_of(&node, &id) == Some(AgentState::Idle)
    });
    let pid = pid_of(&node, &id);
    drop(node);
    wait_for("the adapter to end with its node", || !alive(pid));
}

/// Stopping a held adapter ends its process in the holder and collects it; the holder ends with its last agent.
#[test]
fn stopping_a_held_adapter_ends_its_process_and_the_holder() {
    let _serial = serial();
    let rig = Rig::new("stop");
    let node = rig.node();
    let id = started(&rig, &node);
    let pid = pid_of(&node, &id);
    node.supervisor
        .lock()
        .unwrap()
        .stop(&AgentId::new(id.clone()))
        .expect("stop");
    wait_for("the process to end", || !alive(pid));
    let hold_pid = std::fs::read_to_string(rig.root.join("run/hold.pid"))
        .ok()
        .and_then(|t| {
            t.split_whitespace()
                .next()
                .and_then(|p| p.parse::<u32>().ok())
        });
    if let Some(h) = hold_pid {
        wait_for("the holder to end with its last agent", || !alive(h));
    }
}

/// One MCP conversation on an agent's mesh socket: `initialize`, then `workspace_status`; the tool's answer, or `None` when nothing
/// serves the socket.
fn mesh_status(socket: &Path) -> Option<serde_json::Value> {
    use std::io::{BufRead, Write};
    let stream = agora_platform::local_connect(socket).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .ok()?;
    let mut out = stream.try_clone().ok()?;
    let mut lines = std::io::BufReader::new(stream).lines();
    let mut ask = |msg: serde_json::Value| -> Option<serde_json::Value> {
        writeln!(out, "{msg}").ok()?;
        out.flush().ok()?;
        if msg.get("id").is_none() {
            return Some(serde_json::Value::Null);
        }
        serde_json::from_str(&lines.next()?.ok()?).ok()
    };
    ask(
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}),
    )?;
    ask(serde_json::json!({"jsonrpc":"2.0","method":"notifications/initialized"}))?;
    ask(
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"workspace_status","arguments":{}}}),
    )
}

/// #906 and #907 (RM-47): a structured agent the holder kept across a node restart has its mesh socket served again at the same path,
/// and the hub, which the new node's `NodeOnline` told that everything it ran is gone, shows it running (not finished), so the window
/// lets the person send it a prompt; the prompt is answered.
#[test]
fn a_kept_structured_agent_is_on_the_mesh_again_and_shown_running_after_its_node_restarts() {
    let _serial = serial();
    let rig = Rig::new("kept");
    let node = rig.node();
    let id = started(&rig, &node);
    let pid = pid_of(&node, &id);
    let socket = rig.root.join("m").join(format!("{id}.sock"));
    let a = mesh_status(&socket).expect("the mesh socket before the restart");
    assert!(a["result"].is_object(), "{a}");
    // A node's process that ends closes its agents' mesh endpoints with it; this test process stands for the node and lives on.
    node.mesh.unregister(&AgentId::new(id.clone()));
    drop(node);
    assert!(alive(pid), "the adapter ended with its node");
    assert!(
        mesh_status(&socket).is_none(),
        "the old endpoint still answers"
    );

    let node = rig.node();
    assert!(held(&node, &id), "the agent was not taken up again");
    assert_eq!(pid_of(&node, &id), pid);
    let a = mesh_status(&socket).expect("the kept agent's mesh socket is not served");
    assert!(a["result"].is_object() && a.get("error").is_none(), "{a}");

    let mut view = agora_hub::projection::View::default();
    for ev in node.events.events_after(0) {
        view.apply(&ev);
    }
    let agent = &view.agents[&agora_hub::projection::qualified("hold-node", &id)];
    assert!(agent.running, "the hub shows the kept agent finished");
    assert_eq!(agent.state, v1::AgentState::Idle);
    assert!(agent.interrupted.is_none());
    prompt(&node, &id, "hello again");
    wait_for("the answer", || said(&node, &id, "hello again"));
}

/// #932: a structured agent the holder kept runs on in the mode it was in, and that mode makes it act without asking. After the node
/// restarts it is shown and checked so: the new node puts back what the agent last reported of its session (the adapter is not asked
/// again), so the hub's view has its modes with the skipping one in use, and `skips_permissions_now` says so.
#[test]
fn a_kept_structured_agent_in_a_mode_that_skips_permissions_is_still_shown_skipping_after_its_node_restarts(
) {
    let _serial = serial();
    let rig = Rig::with_args("skipmode", &["--modes"]);
    let node = rig.node();
    let id = started(&rig, &node);
    let agent = AgentId::new(id.clone());
    assert!(!node
        .supervisor
        .lock()
        .unwrap()
        .skips_permissions_now(&agent));
    // The agent switches itself to a mode that skips permissions (the fake reports its options again with it in use).
    prompt(&node, &id, "mode:bypassPermissions");
    wait_for("the agent to act without asking", || {
        node.supervisor
            .lock()
            .unwrap()
            .skips_permissions_now(&agent)
    });
    wait_for("the mode in the kept session", || {
        node.ledger
            .kept_session(&id)
            .options
            .is_some_and(|o| o.current_mode == "bypassPermissions")
    });
    let pid = pid_of(&node, &id);
    node.mesh.unregister(&agent);
    drop(node);
    assert!(alive(pid), "the adapter ended with its node");

    let node = rig.node();
    assert!(held(&node, &id), "the agent was not taken up again");
    assert_eq!(pid_of(&node, &id), pid);
    assert!(
        node.supervisor
            .lock()
            .unwrap()
            .skips_permissions_now(&agent),
        "the new node checks the kept agent as if it asks"
    );
    let mut view = agora_hub::projection::View::default();
    for ev in node.events.events_after(0) {
        view.apply(&ev);
    }
    let shown = &view.agents[&agora_hub::projection::qualified("hold-node", &id)];
    assert!(shown.running);
    let options = shown
        .session
        .options
        .as_ref()
        .expect("the hub has no session options for the kept agent");
    assert_eq!(options.current_mode, "bypassPermissions");
    assert!(
        options
            .modes
            .iter()
            .any(|m| m.id == options.current_mode && m.skips_permissions),
        "the hub shows the kept agent as if it asks"
    );
    assert!(
        shown.session.config.is_some(),
        "the config options were not shown again"
    );
    // It still answers, and a later report replaces what was put back.
    prompt(&node, &id, "mode:default");
    wait_for("the agent to ask again", || {
        !node
            .supervisor
            .lock()
            .unwrap()
            .skips_permissions_now(&agent)
    });
}
