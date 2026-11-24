//! ADR-0093 (Unix): a structured agent (an adapter on pipes) outlives its node when the holder runs it, and the next node takes it up with
//! what it wrote meanwhile and the request that was waiting for the person. A real node, a real `agora hold` and the fake adapter
//! (`agora-fake-agent --adapter`); never a real agent.
//!
//! The "node" is this test process (the role lock names it, as in `hold_node.rs`). A node that stops is [`Node`] dropped, which lets
//! a held adapter go and leaves it running: to the holder this is a link that closed, as when a node is killed. Since ADR-0104 O2 the
//! drop is a clean stop: it records what the node had read and not recorded and writes what it held, before it lets go.
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

    /// A home whose kind `k` is the fake ACP agent behind `agora acp-bridge`, an agent that loads its session on resume and replays its
    /// conversation as it does (`--load-session`).
    fn acp(tag: &str) -> Rig {
        let rig = Self::new(tag);
        let argv = serde_json::to_string(&[fake_agent(), "--acp".into(), "--load-session".into()])
            .expect("argv");
        std::fs::write(
            rig.root.join("manifests/k.toml"),
            format!("kind = \"k\"\ndisplay_name = \"K\"\nversion = \"1\"\nmin_engine = 1\n[structured]\nacp = {{ argv = {argv} }}\n"),
        )
        .expect("manifest");
        rig
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

/// ADR-0101, Codex review of PR #1007: an adapter the holder runs is unconfined by design. A node that confines agents and allows
/// "always" only for confined ones (`skip_permissions = confined_only`) still takes it up, and never counts it as confined: the
/// card offers no "always" and an always answer is refused. (The node's sandbox setting alone, which the check read before, says
/// confined.)
#[test]
fn a_kept_structured_agent_is_not_counted_confined_by_a_node_that_now_confines() {
    use agora_node::supervisor::Confinement;
    if !agora_platform::sandbox_supported() {
        return;
    }
    let _serial = serial();
    let rig = Rig::new("confine");
    let node = rig.node();
    let id = started(&rig, &node);
    let agent = AgentId::new(id.clone());
    assert_eq!(
        node.supervisor.lock().unwrap().confinement(&agent),
        Some(Confinement::Unconfined)
    );
    let pid = pid_of(&node, &id);
    drop(node);
    std::thread::sleep(Duration::from_millis(500));
    // The owner turns confinement on and allows "always" only for confined agents; the next node reads that at its start.
    std::fs::write(
        rig.root.join("data/policy"),
        "sandbox = worktree\nskip_permissions = confined_only\n",
    )
    .expect("policy");
    let node = rig.node();
    assert!(held(&node, &id), "the agent was not taken up again");
    assert_eq!(pid_of(&node, &id), pid, "a new process stands for it");
    // Read under the lock, checked after it is let go (a failed check must not leave the lock poisoned for the clean-up).
    let (level, enforceable, confinement, offered, decided) = {
        let sup = node.supervisor.lock().unwrap();
        (
            sup.sandbox_policy().level,
            sup.sandbox_policy().enforceable(),
            sup.confinement(&agent),
            sup.approval_raise_terms(&agent)
                .map(|(_, permitted)| permitted),
            sup.approval_decide(&agent).always,
        )
    };
    assert_eq!(
        level,
        agora_node::sandbox::Level::Worktree,
        "the new node confines agents"
    );
    assert!(enforceable);
    assert_eq!(confinement, Some(Confinement::Unconfined));
    assert_eq!(offered, Some(false), "no \"always\" is offered for it");
    assert!(!decided, "an always answer is refused");
}

/// #1009: the same holder-run (unconfined) agent under a node that now confines with `skip_permissions = confined_only` is refused a
/// switch to a mode or an option value that makes it act without asking (`agent.option_forbidden`, `policy: confined_only`), on the
/// owner's own control socket: no approval is raised for it. (The node's sandbox setting alone, which the switch checks read before,
/// says confined, and the switch became an `agent.approval_needed` card.)
#[test]
fn a_kept_structured_agent_under_a_node_that_now_confines_is_refused_a_mode_that_skips_permissions()
{
    use agora_node::supervisor::Confinement;
    if !agora_platform::sandbox_supported() {
        return;
    }
    let _serial = serial();
    let rig = Rig::with_args("confmode", &["--modes"]);
    let node = rig.node();
    let id = started(&rig, &node);
    let agent = AgentId::new(id.clone());
    let pid = pid_of(&node, &id);
    // The adapter reports its modes and config options once, after it says it is idle, and never again by itself. The node is stopped
    // without waiting for them: the next node has them whether this one recorded them (the kept session, #932), read them and did not
    // record them (the holder's `seen` lines, ADR-0104), or never read them (`fresh`). Until #1080 the middle case lost them.
    drop(node);
    std::thread::sleep(Duration::from_millis(500));
    std::fs::write(
        rig.root.join("data/policy"),
        "sandbox = worktree\nskip_permissions = confined_only\n",
    )
    .expect("policy");
    let node = rig.node();
    assert!(held(&node, &id), "the agent was not taken up again");
    assert_eq!(pid_of(&node, &id), pid, "a new process stands for it");
    let (level, enforceable, confinement) = {
        let sup = node.supervisor.lock().unwrap();
        (
            sup.sandbox_policy().level,
            sup.sandbox_policy().enforceable(),
            sup.confinement(&agent),
        )
    };
    assert_eq!(level, agora_node::sandbox::Level::Worktree);
    assert!(enforceable, "the new node confines agents");
    assert_eq!(confinement, Some(Confinement::Unconfined));

    let mut c = Client::connect(node.socket().unwrap()).expect("control");
    let set_mode = |c: &mut Client, mode: &str| {
        c.call(Cmd::AgentSetMode(v1::AgentSetMode {
            agent_id: id.clone(),
            mode: mode.into(),
        }))
        .unwrap()
    };
    // The new node knows the agent's modes and config options from the kept session (without them a switch is `agent.option_unknown`,
    // nothing applied): checked with the supervisor's own checks of a mode and a value that keep asking, which prepare a call and
    // apply nothing.
    wait_for("the agent's modes and config options", || {
        use agora_adapters::host::OptionKind;
        use agora_node::supervisor::Prepared;
        let sup = node.supervisor.lock().unwrap();
        matches!(
            sup.prepare_set_option(&agent, OptionKind::Mode, "default"),
            Ok(Prepared::Call(_))
        ) && matches!(
            sup.prepare_set_config(&agent, "auto_approve", "false"),
            Ok(Prepared::Call(_))
        )
    });
    let policy = |e: &v1::ErrorInfo| {
        e.params
            .get("policy")
            .and_then(|p| p.value.clone())
            .map(|v| match v {
                v1::param_value::Value::Str(s) => s,
                other => format!("{other:?}"),
            })
    };
    let refused = set_mode(&mut c, "bypassPermissions").expect_err("a mode that skips was applied");
    assert_eq!(refused.code, "agent.option_forbidden", "{refused:?}");
    assert_eq!(policy(&refused).as_deref(), Some("confined_only"));
    let refused = c
        .call(Cmd::AgentSetConfig(v1::AgentSetConfig {
            agent_id: id.clone(),
            config_id: "auto_approve".into(),
            value: "true".into(),
        }))
        .unwrap()
        .expect_err("an option value that skips was applied");
    assert_eq!(refused.code, "agent.option_forbidden", "{refused:?}");
    assert_eq!(policy(&refused).as_deref(), Some("confined_only"));
    assert!(
        !node
            .supervisor
            .lock()
            .unwrap()
            .skips_permissions_now(&agent),
        "the agent acts without asking"
    );
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
    // What was kept is judged again when it is put back, as a report is: written here without the adapter's own skip flags (as an
    // older node, or one without a rule added since, would have kept it), the mode's words still make it one that skips.
    let file = rig.root.join(format!("data/sessions/kept-{id}.pb"));
    let mut bytes: &[u8] = &std::fs::read(&file).expect("the kept session file");
    let mut rewritten = Vec::new();
    while !bytes.is_empty() {
        use prost::Message;
        let mut ev = v1::Event::decode_length_delimited(&mut bytes).expect("a kept report");
        if let Some(v1::event::Body::AgentSessionOptions(o)) = &mut ev.body {
            for m in &mut o.modes {
                m.skips_permissions = false;
            }
        }
        rewritten.extend(ev.encode_length_delimited_to_vec());
    }
    std::fs::write(&file, rewritten).expect("rewrite");

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
    // A resync announcement (the hub lost this node's start) says the agent up again with `AgentStarted`, which leaves the hub with no
    // session options: what the agent reported follows it there too.
    node.events.announce();
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
        .expect("a resync lost the kept agent's session options");
    assert_eq!(options.current_mode, "bypassPermissions");
    // Codex review of #928: the same for an announcement that leaves the node's start in place (`Announce::Present`).
    let views = node.supervisor.lock().unwrap().list();
    node.events
        .announce_present(&views, &[], agora_node::daemon::Announce::Present);
    let mut after = agora_hub::projection::View::default();
    for ev in node.events.events_after(0) {
        after.apply(&ev);
    }
    let after = &after.agents[&agora_hub::projection::qualified("hold-node", &id)];
    assert_eq!(
        after
            .session
            .options
            .as_ref()
            .map(|options| options.current_mode.as_str()),
        Some("bypassPermissions"),
        "a hub resync lost the restored mode"
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

/// The current mode and the value of `auto_approve` in the kept session file (`kept-<id>.pb`), as the node wrote it.
fn kept_on_disk(rig: &Rig, id: &str) -> (Option<String>, Option<String>) {
    use prost::Message;
    let (mut mode, mut auto) = (None, None);
    let Ok(bytes) = std::fs::read(rig.root.join(format!("data/sessions/kept-{id}.pb"))) else {
        return (mode, auto);
    };
    let mut bytes: &[u8] = &bytes;
    while let Ok(ev) = v1::Event::decode_length_delimited(&mut bytes) {
        match ev.body {
            Some(v1::event::Body::AgentSessionOptions(o)) => mode = Some(o.current_mode),
            Some(v1::event::Body::AgentSessionConfig(c)) => {
                auto = c
                    .options
                    .into_iter()
                    .find(|o| o.id == "auto_approve")
                    .map(|o| o.current);
            }
            _ => {}
        }
    }
    (mode, auto)
}

/// What the supervisor holds of an adapter agent's session: its current mode and the value of `auto_approve`.
fn reported(sup: &agora_node::supervisor::Supervisor, agent: &AgentId) -> (String, String) {
    use agora_adapters::host::AdapterEvent;
    use agora_node::supervisor::SupervisorEvent;
    let (mut mode, mut auto) = (String::new(), String::new());
    for e in sup.session_events(agent) {
        match e {
            SupervisorEvent::Adapter {
                event: AdapterEvent::Options { options, .. },
                ..
            } => mode = options.current_mode,
            SupervisorEvent::Adapter {
                event: AdapterEvent::Config { options, .. },
                ..
            } => {
                if let Some(o) = options.into_iter().find(|o| o.id == "auto_approve") {
                    auto = o.current;
                }
            }
            _ => {}
        }
    }
    (mode, auto)
}

/// #1080, ADR-0104 decision 1: the agent switches itself to a mode that skips permissions and turns on an option, and its node reads
/// both reports and stops before it records them. The holder counted both lines as given (`seen`) when it handed them over, so they
/// never come back as new, and the kept file still has the start's. The next node takes the newest of each from `seen` over the file.
///
/// The race of the issue (1 run in 15), made deterministic: the test holds the supervisor, so the node's own ticker cannot take the
/// adapter's events, and runs the tick itself and drops what it returns, as a node killed between reading a line and recording its
/// event loses it.
#[test]
fn reports_a_node_read_and_did_not_record_before_it_stopped_reach_the_next_node() {
    let _serial = serial();
    let rig = Rig::with_args("unrecorded", &["--modes"]);
    let node = rig.node();
    let id = started(&rig, &node);
    let agent = AgentId::new(id.clone());
    let pid = pid_of(&node, &id);
    wait_for("the start's modes and config options on disk", || {
        kept_on_disk(&rig, &id) == (Some("default".into()), Some("false".into()))
    });
    {
        let mut sup = node.supervisor.lock().unwrap();
        assert!(!sup.skips_permissions_now(&agent));
        sup.adapter_prompt(&agent, "mode:bypassPermissions", false)
            .expect("prompt");
        sup.adapter_prompt(&agent, "config:auto_approve=true", false)
            .expect("prompt");
        // Read (the node had both lines and made its events), never recorded: the events are dropped here.
        wait_for("the node to have read both reports", || {
            drop(sup.tick(Instant::now()));
            reported(&sup, &agent) == ("bypassPermissions".into(), "true".into())
        });
        assert!(sup.skips_permissions_now(&agent));
    }
    node.mesh.unregister(&agent);
    drop(node);
    assert!(alive(pid), "the adapter ended with its node");
    assert_eq!(
        kept_on_disk(&rig, &id),
        (Some("default".into()), Some("false".into())),
        "the node recorded the reports after all: the test no longer shows #1080"
    );

    let node = rig.node();
    assert!(held(&node, &id), "the agent was not taken up again");
    assert_eq!(pid_of(&node, &id), pid, "a new process stands for it");
    {
        let sup = node.supervisor.lock().unwrap();
        assert_eq!(
            reported(&sup, &agent),
            ("bypassPermissions".into(), "true".into()),
            "the next node took the stale kept file over the reports its holder had"
        );
        assert!(
            sup.skips_permissions_now(&agent),
            "the new node checks the kept agent as if it asks"
        );
    }
    // Said to the hub as such, and recorded, so a later node has them from the file too.
    let mut view = agora_hub::projection::View::default();
    for ev in node.events.events_after(0) {
        view.apply(&ev);
    }
    let shown = &view.agents[&agora_hub::projection::qualified("hold-node", &id)];
    assert_eq!(
        shown
            .session
            .options
            .as_ref()
            .map(|o| o.current_mode.as_str()),
        Some("bypassPermissions")
    );
    assert!(shown.session.config.as_ref().is_some_and(|c| c
        .options
        .iter()
        .any(|o| o.id == "auto_approve" && o.current == "true")));
    wait_for("the newer reports in the kept file", || {
        kept_on_disk(&rig, &id) == (Some("bypassPermissions".into()), Some("true".into()))
    });
}

/// Codex review of #928: a node with no holder to attach to takes nothing up, so every kept session left on disk is an earlier
/// process's and goes, rather than waiting for an agent that reuses the id.
#[test]
fn a_node_without_a_holder_keeps_no_earlier_session() {
    let _serial = serial();
    let rig = Rig::new("nokeep");
    let dir = rig.root.join("data/sessions");
    std::fs::create_dir_all(&dir).expect("sessions");
    let file = dir.join("kept-agent-7.pb");
    std::fs::write(&file, b"old").expect("kept file");
    let _node = rig.node_without_holder();
    assert!(!file.exists(), "an earlier process's session was kept");
}

/// #951: a held adapter that crashes is back after exactly one restart (the holder that ended with its only agent, released at the
/// crash, is replaced, never counted as a failed restart), the restarted one is held again, and a crash loop gives up after the
/// configured number of restarts with the exit code of the last real crash (the fake adapter's `crash` exits 3).
#[test]
fn a_crashed_held_adapter_is_back_after_one_restart_and_a_crash_loop_gives_up_with_its_exit_code() {
    use agora_node::supervisor::{
        RestartPolicy, StartRequest, Supervisor, SupervisorConfig, SupervisorEvent,
    };
    let _serial = serial();
    let rig = Rig::new("crash");
    let mut s = Supervisor::new(SupervisorConfig {
        node: agora_core::NodeId::new("hold-node"),
        restart: RestartPolicy {
            max_restarts: 2,
            base_delay: Duration::from_millis(50),
            max_delay: Duration::from_millis(100),
        },
        terminal: agora_node::terminal::TerminalOptions::default(),
        limits: NodeLimits::default(),
    });
    let manifest = std::fs::read_to_string(rig.root.join("manifests/k.toml")).expect("manifest");
    s.register(agora_adapters::Manifest::parse(&manifest).expect("parse"));
    s.set_holder(std::sync::Arc::new(agora_node::holder::HolderHub::new(
        rig.setup(),
    )));
    let path = rig.folder.to_string_lossy().into_owned();
    let id = s
        .start_in_directory(
            &StartRequest {
                kind: "k".into(),
                checkout: rig.folder.clone(),
                worktree_root: rig.root.join("worktrees"),
                name: "crash".into(),
                base: None,
            },
            agora_core::ProjectId::directory("hold-node", &path),
            Instant::now(),
        )
        .expect("start");
    let is_held = |s: &Supervisor| s.held_agents().0.contains(&id);
    assert!(
        is_held(&s),
        "the adapter was not started through the holder"
    );
    let mut seen: Vec<SupervisorEvent> = Vec::new();
    fn tick_until(
        s: &mut Supervisor,
        seen: &mut Vec<SupervisorEvent>,
        what: &str,
        done: &dyn Fn(&Supervisor, &[SupervisorEvent]) -> bool,
    ) {
        let end = Instant::now() + Duration::from_secs(30);
        let mark = seen.len();
        loop {
            assert!(
                Instant::now() < end,
                "timed out waiting for {what}: {seen:#?}"
            );
            seen.extend(s.tick(Instant::now()));
            if done(s, &seen[mark..]) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    let started = |_: &Supervisor, evs: &[SupervisorEvent]| {
        evs.iter()
            .any(|e| matches!(e, SupervisorEvent::Started { .. }))
    };
    let idle = |s: &Supervisor, id: &AgentId| {
        s.list()
            .iter()
            .any(|v| &v.id == id && v.running && v.state == AgentState::Idle)
    };
    let id2 = id.clone();
    tick_until(&mut s, &mut seen, "the agent to be idle", &|s, _| {
        idle(s, &id2)
    });

    // One crash: one restart, and it is back, held, and answers.
    s.adapter_prompt(&id, "crash", false).expect("prompt");
    tick_until(&mut s, &mut seen, "the restart", &started);
    let restarting = |seen: &[SupervisorEvent]| {
        seen.iter()
            .filter(|e| matches!(e, SupervisorEvent::Restarting { .. }))
            .count()
    };
    assert_eq!(
        restarting(&seen),
        1,
        "one crash cost more than one restart: {seen:#?}"
    );
    assert!(s.list().iter().any(|v| v.id == id && v.restarts == 1));
    assert!(is_held(&s), "the restarted adapter is not the holder's");
    tick_until(
        &mut s,
        &mut seen,
        "the restarted agent to be idle",
        &|s, _| idle(s, &id2),
    );

    // A second crash takes the last restart; the third ends it with the crash's own exit code.
    s.adapter_prompt(&id, "crash", false).expect("prompt");
    tick_until(&mut s, &mut seen, "the second restart", &started);
    tick_until(&mut s, &mut seen, "the agent to be idle again", &|s, _| {
        idle(s, &id2)
    });
    s.adapter_prompt(&id, "crash", false).expect("prompt");
    tick_until(&mut s, &mut seen, "the give-up", &|_, evs| {
        evs.iter()
            .any(|e| matches!(e, SupervisorEvent::GaveUp { .. }))
    });
    assert_eq!(restarting(&seen), 2, "{seen:#?}");
    assert!(
        seen.contains(&SupervisorEvent::GaveUp {
            agent: id.clone(),
            exit_code: Some(3),
        }),
        "{seen:#?}"
    );
}

/// The usage a node logged for `id`: (input, output) summed over its `AgentUsageReported` events.
fn logged_usage(node: &Node, id: &str) -> (u64, u64) {
    node.events
        .events_after(0)
        .iter()
        .filter_map(|e| match &e.body {
            Some(v1::event::Body::AgentUsageReported(u)) if u.agent_id == id => {
                Some((u.input_tokens, u.output_tokens))
            }
            _ => None,
        })
        .fold((0, 0), |(i, o), (a, b)| (i + a, o + b))
}

/// ADR-0104 decision 2 (#930): a kept agent reports usage, its node restarts and takes it up (the same process), and the hub's view
/// folded from the node's log still has the tokens and the model it reported before, then adds what it uses after. The window and
/// the terminal client start from that view's snapshot, so the terminal client's header still counts them. Before, the take-up's
/// `AgentStarted` started them again from 0.
#[test]
fn a_kept_agents_usage_counters_survive_its_node_restarting() {
    let _serial = serial();
    let rig = Rig::new("usage");
    let node = rig.node();
    let id = started(&rig, &node);
    let pid = pid_of(&node, &id);
    prompt(&node, &id, "hello");
    wait_for("the answer", || said(&node, &id, "hello"));
    // The node gathers usage for a few seconds before it logs it (`usage_merge`); a killed node loses a report still gathering (a clean
    // stop logs it, tested below), so this waits until it is logged and the take-up is what this test is about.
    wait_for("the usage in the node's log", || {
        logged_usage(&node, &id) == (10, 5)
    });
    drop(node);
    assert!(alive(pid), "the adapter ended with its node");

    let node = rig.node();
    assert!(held(&node, &id), "the agent was not taken up again");
    assert_eq!(pid_of(&node, &id), pid);
    let events = node.events.events_after(0);
    // The order the hub folds: the usage, the second start's `NodeOnline`, then the take-up's `AgentStarted` saying the same process.
    let position = |f: &dyn Fn(&v1::event::Body) -> bool| {
        events
            .iter()
            .rposition(|e| e.body.as_ref().is_some_and(f))
            .expect("the event")
    };
    let usage =
        position(&|b| matches!(b, v1::event::Body::AgentUsageReported(u) if u.agent_id == id));
    let online = position(&|b| matches!(b, v1::event::Body::NodeOnline(_)));
    let taken_up = position(&|b| matches!(b, v1::event::Body::AgentStarted(s) if s.agent_id == id));
    assert!(
        usage < online && online < taken_up,
        "{usage} {online} {taken_up}"
    );
    let Some(v1::event::Body::AgentStarted(s)) = &events[taken_up].body else {
        unreachable!()
    };
    assert!(s.same_process, "the take-up says it is the same process");
    let first = events
        .iter()
        .find_map(|e| match &e.body {
            Some(v1::event::Body::AgentStarted(s)) if s.agent_id == id => Some(s.same_process),
            _ => None,
        })
        .expect("the first start");
    assert!(!first, "the first start is a new process");

    let key = agora_hub::projection::qualified("hold-node", &id);
    let fold = |events: &[v1::Event]| {
        let mut view = agora_hub::projection::View::default();
        for ev in events {
            view.apply(ev);
        }
        view
    };
    let view = fold(&events);
    let agent = &view.agents[&key];
    assert!(agent.running, "the hub shows the kept agent finished");
    assert_eq!(
        (
            agent.input_tokens,
            agent.output_tokens,
            agent.model.as_str()
        ),
        (10, 5, "fake/model"),
        "the counters started again after the node's restart"
    );
    // The terminal client draws the hub's snapshot: its header still counts what the agent used before the restart.
    let snapshot = agora_hub::server::snapshot_of(&view, 0);
    let catalog = agora_i18n::Catalog::english();
    let text = |id: &str, args: &[(&str, agora_i18n::Arg)]| {
        catalog.render(id, args).unwrap_or_else(|| id.to_string())
    };
    let shown = agora_tui::render::render(&snapshot, false, false, "", "", 0, 0, &text);
    let header_tokens = |s: &v1::Snapshot| {
        s.agents
            .iter()
            .find(|a| a.agent_id == key)
            .map(|a| (a.input_tokens, a.output_tokens))
    };
    assert_eq!(header_tokens(&snapshot), Some((10, 5)));
    let usage_line = catalog
        .render(
            "tui-usage",
            &[
                ("input", agora_i18n::Arg::Num(10)),
                ("output", agora_i18n::Arg::Num(5)),
            ],
        )
        .expect("the usage line");
    assert!(shown.iter().any(|l| l.contains(&usage_line)), "{shown:#?}");

    // What it uses after the take-up adds to what it used before.
    prompt(&node, &id, "hello again");
    wait_for("the answer", || said(&node, &id, "hello again"));
    wait_for("the new usage in the node's log", || {
        logged_usage(&node, &id) == (20, 16)
    });
    let view = fold(&node.events.events_after(0));
    let agent = &view.agents[&key];
    assert_eq!((agent.input_tokens, agent.output_tokens), (20, 16));
}

/// ADR-0089: the agent's transcript on the node's disk.
fn transcript_file(rig: &Rig, id: &str) -> PathBuf {
    rig.root
        .join("data/transcripts")
        .join(format!("{id}.jsonl"))
}

fn on_disk(rig: &Rig, id: &str, needle: &str) -> bool {
    std::fs::read_to_string(transcript_file(rig, id)).is_ok_and(|t| t.contains(needle))
}

fn chat(node: &Node, id: &str) -> Vec<v1::ChatEntry> {
    node.chat.snapshot(&AgentId::new(id))
}

/// ADR-0104 C1 (#931): a kept agent's conversation is written to its node's disk, and the next node, which takes the agent up from the
/// holder, shows the earlier turns with the same ids and numbers what comes after past them. The test waits until the writer (once a
/// second) has written both turns before the node stops, so they are in the file as after a kill (a clean stop's flush is tested below).
#[test]
fn a_kept_agents_conversation_and_its_ids_survive_its_node_restarting() {
    let _serial = serial();
    let rig = Rig::new("convo");
    let node = rig.node();
    let id = started(&rig, &node);
    let pid = pid_of(&node, &id);
    prompt(&node, &id, "hello-1");
    wait_for("the first answer", || said(&node, &id, "hello-1"));
    prompt(&node, &id, "hello-2");
    wait_for("the second answer", || said(&node, &id, "hello-2"));
    wait_for("both turns on disk", || {
        on_disk(&rig, &id, "hello-1") && on_disk(&rig, &id, "hello-2")
    });
    let before = chat(&node, &id);
    let high = before.iter().map(|e| e.entry_id).max().expect("entries");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&transcript_file(&rig, &id)), 0o600);
        assert_eq!(mode(&rig.root.join("data/transcripts")), 0o700);
    }
    drop(node);
    assert!(alive(pid), "the adapter ended with its node");

    let node = rig.node();
    assert!(held(&node, &id), "the agent was not taken up again");
    // Shown at once: the ring was filled from the file before the node read anything new.
    let seeded = chat(&node, &id);
    let texts = |c: &[v1::ChatEntry]| {
        c.iter()
            .filter(|e| e.text.starts_with("hello"))
            .map(|e| (e.entry_id, e.text.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        texts(&seeded),
        texts(&before),
        "the earlier turns, with their ids"
    );
    prompt(&node, &id, "after");
    wait_for("the answer after the restart", || said(&node, &id, "after"));
    let after = chat(&node, &id);
    let new = after
        .iter()
        .find(|e| e.text == "after")
        .expect("the new turn");
    assert!(
        new.entry_id > high,
        "id {} reuses one up to {high}",
        new.entry_id
    );
    let mut ids: Vec<u64> = after.iter().map(|e| e.entry_id).collect();
    ids.dedup();
    assert_eq!(ids.len(), after.len(), "one entry per id");
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "ids increase: {ids:?}");
}

/// ADR-0104 C1: a tool call that started before the node restarted and finishes after it is ONE entry, updated in place: the next
/// node rebuilt the call from the file, so an update that carries only the status and the output changes that entry, whose text still
/// names the tool and its arguments.
#[test]
fn a_tool_call_that_finishes_after_its_node_restarts_is_one_entry_updated_in_place() {
    let _serial = serial();
    let rig = Rig::new("tool");
    let node = rig.node();
    let id = started(&rig, &node);
    prompt(&node, &id, "later-tool");
    let running_call = |node: &Node| {
        chat(node, &id)
            .into_iter()
            .find(|e| e.call_id == "t1" && e.status == v1::ToolStatus::InProgress as i32)
    };
    wait_for("the call to run", || running_call(&node).is_some());
    wait_for("the call on disk", || on_disk(&rig, &id, "\"t1\""));
    let call = running_call(&node).expect("the call");
    drop(node);

    let node = rig.node();
    let seeded = running_call(&node).expect("the running call, seeded");
    assert_eq!(seeded.entry_id, call.entry_id);
    std::fs::write(rig.folder.join("go.txt"), "go").expect("go");
    wait_for("the adapter to finish", || {
        rig.folder.join("later.txt").exists()
    });
    wait_for("the rest of the turn", || said(&node, &id, "tool-done"));
    wait_for("the call to finish", || {
        chat(&node, &id)
            .iter()
            .any(|e| e.call_id == "t1" && e.status == v1::ToolStatus::Completed as i32)
    });
    let calls: Vec<v1::ChatEntry> = chat(&node, &id)
        .into_iter()
        .filter(|e| e.call_id == "t1")
        .collect();
    assert_eq!(calls.len(), 1, "{calls:#?}");
    assert_eq!(calls[0].entry_id, call.entry_id);
    assert_eq!(calls[0].output, "done");
    assert!(
        calls[0].text.contains("\"tool\":\"read\"") && calls[0].text.contains("a.rs"),
        "{}",
        calls[0].text
    );
    // The finished call reaches the file too.
    wait_for("the finished call on disk", || {
        on_disk(&rig, &id, "\"output\":\"done\"")
    });
}

/// ADR-0089 decision 1: a node that cannot write any history (here a file where its transcripts folder would be, which refuses every
/// write as a full disk does) runs its agents as before, keeps their conversation in memory, and says once in it that it is not saved.
#[test]
fn a_node_that_cannot_write_history_runs_its_agent_and_says_so_once() {
    let _serial = serial();
    let rig = Rig::new("nodisk");
    std::fs::write(rig.root.join("data/transcripts"), "not a folder").expect("block");
    let node = rig.node();
    let id = started(&rig, &node);
    let notices = |node: &Node| {
        chat(node, &id)
            .iter()
            .filter(|e| e.code == "chat.history_not_saved")
            .count()
    };
    for word in ["one", "two", "three"] {
        prompt(&node, &id, word);
        wait_for("the answer", || said(&node, &id, word));
        wait_for("the notice", || notices(&node) >= 1);
        // A batch after this answer was taken (the writer runs once a second).
        std::thread::sleep(Duration::from_millis(1300));
    }
    assert_eq!(notices(&node), 1);
    assert_eq!(state_of(&node, &id), Some(AgentState::Idle));
}

/// Whether the agent said `word` (its own entry, not the person's).
fn agent_said(node: &Node, id: &str, word: &str) -> bool {
    chat(node, id)
        .iter()
        .any(|e| e.text == word && e.role == v1::ChatRole::Agent as i32)
}

/// The process id of the holder the rig's node started (a process of this test).
fn holder_pid(rig: &Rig) -> u32 {
    std::fs::read_to_string(rig.root.join("run/hold.pid"))
        .expect("the holder's file")
        .split_whitespace()
        .next()
        .and_then(|p| p.parse().ok())
        .expect("the holder's pid")
}

/// Sends `SIGSTOP` to a process this test started and `SIGCONT` when dropped, however the test ends.
struct Paused(u32);

impl Paused {
    fn new(pid: u32) -> Paused {
        assert!(Command::new("kill")
            .args(["-STOP", &pid.to_string()])
            .status()
            .unwrap()
            .success());
        Paused(pid)
    }
}

impl Drop for Paused {
    fn drop(&mut self) {
        let _ = Command::new("kill")
            .args(["-CONT", &self.0.to_string()])
            .status();
    }
}

/// ADR-0104 O2: the node had read a turn (the agent's answer, its usage and its states) from the holder and not recorded it when it
/// stopped. The holder counted those lines as given, so no later node reads them again; the clean stop records them before it lets
/// the adapter go, and the next node has the answer in the conversation and the usage in its log.
///
/// Deterministic: the test holds the supervisor, so the supervision loop cannot take the events, waits until the adapter's reader has
/// handed all four over, and asks the node to stop before it lets go (the loop then never ticks again).
#[test]
fn a_turn_the_node_read_and_had_not_recorded_when_it_stopped_cleanly_reaches_the_next_node() {
    let _serial = serial();
    let rig = Rig::new("drain");
    let node = rig.node();
    let id = started(&rig, &node);
    let agent = AgentId::new(id.clone());
    let pid = pid_of(&node, &id);
    let sup = std::sync::Arc::clone(&node.supervisor);
    {
        let guard = sup.lock().unwrap();
        let before = guard.adapter_events_read(&agent);
        guard
            .adapter_prompt(&agent, "drained", false)
            .expect("prompt");
        // Working, the answer, the usage, idle.
        wait_for("the reader to hand over the turn", || {
            guard.adapter_events_read(&agent) >= before + 4
        });
        node.begin_stop();
    }
    drop(sup);
    assert!(!agent_said(&node, &id, "drained"), "the loop took the turn");
    let stopping = Instant::now();
    drop(node);
    assert!(stopping.elapsed() < Duration::from_secs(3));
    assert!(alive(pid), "the adapter ended with its node");

    let node = rig.node();
    assert!(held(&node, &id), "the agent was not taken up again");
    // The stopped node let go of the agent's mesh endpoint, so this one serves it (in a process that goes on, too).
    assert!(
        node.mesh.is_registered(&agent),
        "the agent is not on the mesh"
    );
    assert!(
        agent_said(&node, &id, "drained"),
        "the answer the old node read is not in the conversation"
    );
    assert_eq!(logged_usage(&node, &id), (10, 7), "the usage of the turn");
}

/// ADR-0104 O2: a clean stop writes what the conversation's writer and the usage merge (5 s) still held: words the agent said in a turn
/// still open (held back, as the node holds a secret value: #1103's writer releases them at the turn's end or a stop) are in the file
/// after it, and the usage of the turn before, still being gathered, is in the log.
#[test]
fn a_clean_stop_writes_the_last_answer_and_the_usage_still_being_gathered() {
    let _serial = serial();
    let rig = Rig::new("flush");
    agora_node::credentials::SecretStore::in_data(&rig.root.join("data"))
        .set(
            "TOKEN",
            &agora_node::credentials::Secret::new("SECRETVALUE-ABCDEFGH-XYZ").unwrap(),
        )
        .unwrap();
    let node = rig.node();
    let id = started(&rig, &node);
    prompt(&node, &id, "counted");
    wait_for("the answer", || agent_said(&node, &id, "counted"));
    // The turn's usage is gathered for 5 s before it is logged: what follows is well within that.
    let gathered = logged_usage(&node, &id);
    assert_eq!(gathered, (0, 0), "the usage was logged before the stop");
    prompt(&node, &id, "hold:the last words");
    wait_for("the last words", || {
        agent_said(&node, &id, "the last words")
    });
    wait_for("the agent to work", || {
        state_of(&node, &id) == Some(AgentState::Working)
    });
    assert!(
        !on_disk(&rig, &id, "the last words"),
        "written while the turn was open: the test no longer shows the stop's flush"
    );
    drop(node);
    assert!(
        on_disk(&rig, &id, "the last words"),
        "the last words are not in the file"
    );
    let node = rig.node();
    assert_eq!(
        logged_usage(&node, &id),
        (10, 7),
        "the usage still being gathered was lost"
    );
}

/// Review of #1108 (#1119 item 2): a start or a resume launches its adapter under the supervisor's lock (an `initialize` may take 30 s).
/// A stop that comes meanwhile still writes, within its deadline, what does not need that lock: the agent's words held back in its open
/// turn and the usage of the turn before, still being gathered. Then, once the lock is free, it lets the adapter go, which keeps running.
///
/// The test holds the supervisor as such a launch does, until the stop has written both (or for 10 s: before, the stop waited for the
/// lock and wrote nothing until then). The node holds a secret and the words end inside it: this stop writes twice (once while the lock
/// is busy, again after the let-go), and no piece of the value is ever on disk, after the stop nor once the next node wrote the rest.
#[test]
fn a_stop_while_a_launch_holds_the_supervisor_still_writes_the_last_answer_and_the_usage() {
    let _serial = serial();
    let rig = Rig::new("busy");
    let value = "SECRETVALUE-ABCDEFGH-XYZ";
    agora_node::credentials::SecretStore::in_data(&rig.root.join("data"))
        .set(
            "TOKEN",
            &agora_node::credentials::Secret::new(value).unwrap(),
        )
        .unwrap();
    let pieces: Vec<&str> = (0..=value.len() - 4).map(|i| &value[i..i + 4]).collect();
    let leaked = |body: &str| -> Vec<String> {
        pieces
            .iter()
            .filter(|p| body.contains(**p))
            .map(|p| (*p).to_string())
            .collect()
    };
    let node = rig.node();
    let id = started(&rig, &node);
    let pid = pid_of(&node, &id);
    prompt(&node, &id, "counted");
    wait_for("the answer", || agent_said(&node, &id, "counted"));
    // The turn's usage is gathered for 5 s before it is logged: what follows is well within that.
    assert_eq!(
        logged_usage(&node, &id),
        (0, 0),
        "the usage was logged before the stop"
    );
    prompt(&node, &id, "hold:my token is SECRETVA|LUE-ABCDEFGH-XYZ ok");
    wait_for("the first words", || {
        chat(&node, &id)
            .iter()
            .any(|e| e.role == v1::ChatRole::Agent as i32 && e.text.ends_with("SECRETVA"))
    });
    wait_for("the agent to work", || {
        state_of(&node, &id) == Some(AgentState::Working)
    });
    assert!(
        !on_disk(&rig, &id, "my token is"),
        "written while the turn was open: the test no longer shows the stop's flush"
    );
    let (sup, events, file) = (
        std::sync::Arc::clone(&node.supervisor),
        std::sync::Arc::clone(&node.events),
        transcript_file(&rig, &id),
    );
    let agent = id.clone();
    let (locked, is_locked) = std::sync::mpsc::channel();
    let launch = std::thread::spawn(move || {
        let guard = sup.lock().unwrap();
        locked.send(()).unwrap();
        let usage = || {
            events
                .events_after(0)
                .iter()
                .filter_map(|e| match &e.body {
                    Some(v1::event::Body::AgentUsageReported(u)) if u.agent_id == agent => {
                        Some(u.input_tokens)
                    }
                    _ => None,
                })
                .sum::<u64>()
        };
        let written = || {
            std::fs::read_to_string(&file).is_ok_and(|t| t.contains("my token is")) && usage() > 0
        };
        let held_since = Instant::now();
        while !written() && held_since.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(20));
        }
        let took = held_since.elapsed();
        drop(guard);
        (written(), took)
    });
    is_locked.recv().unwrap();
    drop(node);
    let (written, took) = launch.join().unwrap();
    assert!(
        written,
        "the stop wrote nothing while the supervisor was held ({took:?})"
    );
    assert!(took < Duration::from_secs(4), "it took {took:?}");
    assert!(alive(pid), "the adapter ended with its node");
    let body = std::fs::read_to_string(transcript_file(&rig, &id)).unwrap();
    assert!(
        leaked(&body).is_empty(),
        "{:?} on disk after the stop: {body}",
        leaked(&body)
    );

    let node = rig.node();
    assert!(held(&node, &id), "the agent was not taken up again");
    std::fs::write(rig.folder.join("go.txt"), "go").expect("go");
    wait_for("the agent to be idle", || {
        state_of(&node, &id) == Some(AgentState::Idle)
    });
    wait_for("the rest on disk", || on_disk(&rig, &id, " ok"));
    let body = std::fs::read_to_string(transcript_file(&rig, &id)).unwrap();
    assert!(
        leaked(&body).is_empty(),
        "{:?} on disk after the rest: {body}",
        leaked(&body)
    );
    // The first turn's, which the stop wrote (the second turn's is still being gathered, or logged too on a slow runner).
    let logged = logged_usage(&node, &id);
    assert!(logged.0 >= 10, "{logged:?}");
}

/// ADR-0104 O2: a holder that does not answer (stopped here) leaves the adapter's reader waiting for the link's frame time (5 s);
/// the node's stop waits for it at most `DRAIN_WAIT` and still stops in time. The agent keeps running, and once its holder answers
/// again the next node takes it up.
#[test]
fn a_node_stops_in_time_while_its_holder_does_not_answer() {
    let _serial = serial();
    let rig = Rig::new("hung");
    let node = rig.node();
    let id = started(&rig, &node);
    let pid = pid_of(&node, &id);
    let paused = Paused::new(holder_pid(&rig));
    // The reader asks the holder every few milliseconds; let it be in a read that does not come back.
    std::thread::sleep(Duration::from_millis(300));
    let stopping = Instant::now();
    drop(node);
    let took = stopping.elapsed();
    assert!(
        took < agora_node::daemon::DRAIN_WAIT + Duration::from_secs(1),
        "the stop waited {took:?} for a holder that does not answer"
    );
    drop(paused);
    assert!(alive(pid), "the adapter ended with its node");
    let node = rig.node();
    assert!(held(&node, &id), "the agent was not taken up again");
    prompt(&node, &id, "after");
    wait_for("the answer after the restart", || {
        agent_said(&node, &id, "after")
    });
}

/// A command through the node's control socket at `socket`, as a window's would come (`Err` only when the socket failed).
fn command(socket: &Path, cmd: Cmd) -> std::io::Result<Result<v1::CommandOk, v1::ErrorInfo>> {
    Client::connect(socket)?.call(cmd)
}

fn prompt_command(id: &str, text: &str) -> Cmd {
    Cmd::AgentPrompt(v1::AgentPrompt {
        agent_id: id.to_string(),
        text: text.to_string(),
    })
}

/// The person's own entry `text` in the conversation, if there is one.
fn user_entry(node: &Node, id: &str, text: &str) -> Option<v1::ChatEntry> {
    chat(node, id)
        .into_iter()
        .find(|e| e.text == text && e.role == v1::ChatRole::User as i32)
}

/// ADR-0104 O2, review of #1108: a prompt the node had sent and whose answer it was still waiting for when it stopped was given to the
/// agent. The command is answered as sent and the prompt stays in the conversation (it was taken back, as if never sent, while the
/// agent worked on it).
#[test]
fn a_prompt_waiting_for_its_answer_when_the_node_stops_stays_in_the_conversation() {
    let _serial = serial();
    let rig = Rig::new("given-prompt");
    let node = rig.node();
    let id = started(&rig, &node);
    let socket = node.socket().unwrap().to_path_buf();
    // `hang`: the adapter takes the prompt and answers only after 30 s.
    let asking = {
        let (socket, id) = (socket.clone(), id.clone());
        std::thread::spawn(move || command(&socket, prompt_command(&id, "hang")))
    };
    wait_for("the person's entry", || {
        user_entry(&node, &id, "hang").is_some()
    });
    // #1150: the adapter read the prompt (a fixed wait here did not prove that on a stalled runner).
    wait_for("the adapter to read the prompt", || {
        rig.folder.join("hang.txt").exists()
    });
    drop(node);
    let answer = asking.join().unwrap();
    let node = rig.node();
    assert!(held(&node, &id), "the agent was not taken up again");
    assert!(
        user_entry(&node, &id, "hang").is_some(),
        "the prompt the agent was given is gone from the conversation"
    );
    assert!(matches!(answer, Ok(Ok(_))), "{answer:?}");
}

/// ADR-0104 O2, review of #1108: a queued prompt that had become the turn and was being sent when the node stopped was given. It is
/// shown as sent, never dropped as if it had not gone out.
#[test]
fn a_queued_prompt_being_sent_when_the_node_stops_is_shown_sent() {
    let _serial = serial();
    let rig = Rig::with_args("given-queued", &["--turns"]);
    let node = rig.node();
    let id = started(&rig, &node);
    let socket = node.socket().unwrap().to_path_buf();
    // A turn that stays open until `go.txt` appears (its prompt is answered at once), and a prompt queued behind it that is then sent
    // and not answered for 30 s. #1150: the first prompt's answer is in before the next is queued (a turn whose answer came only
    // after the next was queued, 300 ms after its words, was not answered within the prompt's 10 s on a stalled runner).
    let first = command(&socket, prompt_command(&id, "hold:one")).unwrap();
    assert!(first.is_ok(), "{first:?}");
    wait_for("the first turn's words", || agent_said(&node, &id, "one"));
    let queued = command(&socket, prompt_command(&id, "hang")).unwrap();
    assert!(
        matches!(
            &queued,
            Ok(v1::CommandOk {
                result: Some(Ok_::AgentPrompt(v1::AgentPromptResult { queued: true, .. })),
            })
        ),
        "{queued:?}"
    );
    // The first turn ends; the queued prompt becomes the turn and its send waits (it begins as the node takes in the turn's end).
    std::fs::write(rig.folder.join("go.txt"), "go").expect("go");
    wait_for("the first turn to end", || {
        rig.folder.join("later.txt").exists() && state_of(&node, &id) == Some(AgentState::Idle)
    });
    // #1150: the queued prompt was written to the adapter, which read it: a call not yet written when the node lets go is answered
    // as never given (a fixed 500 ms wait here did not prove that on a stalled runner).
    wait_for("the adapter to read the queued prompt", || {
        rig.folder.join("hang.txt").exists()
    });
    assert!(user_entry(&node, &id, "hang").is_some_and(|e| e.queued));
    drop(node);
    let node = rig.node();
    let entry = user_entry(&node, &id, "hang");
    assert!(
        entry.as_ref().is_some_and(|e| !e.queued),
        "the prompt the agent was given is dropped or still waiting: {entry:?}"
    );
    assert!(
        !chat(&node, &id)
            .iter()
            .any(|e| e.code == "adapter.queue_dropped"),
        "{:#?}",
        chat(&node, &id)
    );
}

/// ADR-0104 O2, review of #1108: a decision the person signed, which the node verified and was delivering when it stopped, was given to
/// the agent. It is recorded as decided, never as a request that ended unanswered.
#[test]
fn a_decision_being_delivered_when_the_node_stops_is_recorded_as_decided() {
    use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
    let _serial = serial();
    let rig = Rig::new("given-decision");
    let rng = ring::rand::SystemRandom::new();
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
    let key =
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng).unwrap();
    let hex: String = key
        .public_key()
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    agora_node::approvals::trust_device(&rig.root.join("data/trusted-devices"), "phone", &hex)
        .unwrap();
    let node = rig.node();
    let id = started(&rig, &node);
    prompt(&node, &id, "ask");
    let requested = |node: &Node| {
        node.events
            .events_after(0)
            .into_iter()
            .find_map(|e| match e.body {
                Some(v1::event::Body::ApprovalRequested(r)) if r.agent_id == id => Some(r),
                _ => None,
            })
    };
    wait_for("the request", || requested(&node).is_some());
    let approval = requested(&node).unwrap().approval_id;
    // The adapter cannot answer while it is stopped: the decision's delivery waits for it.
    let adapter = pid_of(&node, &id);
    let paused = Paused::new(adapter);
    let socket = node.socket().unwrap().to_path_buf();
    let signature = key
        .sign(
            &rng,
            &agora_node::approvals::signed_message("hold-node", &approval, true),
        )
        .unwrap()
        .as_ref()
        .to_vec();
    let deciding = {
        let approval = approval.clone();
        std::thread::spawn(move || {
            command(
                &socket,
                Cmd::ApprovalResolve(v1::ApprovalResolve {
                    approval_id: approval,
                    approved: true,
                    signer_device_id: "phone".into(),
                    signature,
                    ..Default::default()
                }),
            )
        })
    };
    // The decision was verified and left the queue; what follows is in memory (the call is queued for the adapter at once). The stop
    // must come after that: a decision that only reaches the adapter after the node let it go was not delivered, and is rightly
    // recorded so (a fixed 500 ms wait here failed once on a macOS runner; the cause was not established).
    wait_for("the decision to leave the queue", || {
        node.approvals.pending_count() == 0
    });
    std::thread::sleep(Duration::from_millis(300));
    drop(node);
    // A stopped agent has not ended: the holder keeps it.
    assert!(alive(adapter), "the stopped adapter was ended");
    drop(paused);
    let answer = deciding.join().unwrap();
    let node = rig.node();
    let resolved: Vec<v1::ApprovalResolved> = node
        .events
        .events_after(0)
        .into_iter()
        .filter_map(|e| match e.body {
            Some(v1::event::Body::ApprovalResolved(r)) if r.approval_id == approval => Some(r),
            _ => None,
        })
        .collect();
    assert_eq!(resolved.len(), 1, "{resolved:#?}");
    assert!(
        resolved[0].approved && resolved[0].signer_device_id == "phone",
        "the delivered decision is recorded as not approved: {resolved:#?}"
    );
    assert!(matches!(answer, Ok(Ok(_))), "{answer:?}");
}

/// Review of #1103 (B1): an ACP agent that loads its session when it is resumed replays its conversation (`session/load`), and the
/// bridge passes the replay on as chat. The node already shows that conversation from its file, so the replay is not shown or written a
/// second time: every turn is there once after each resume, in the window and on disk.
#[test]
fn a_resumed_agent_that_replays_its_session_shows_its_conversation_once() {
    let _serial = serial();
    // The bridge is this build's `agora` (the test binary is not).
    std::env::set_var("AGORA_ACP_BRIDGE", AGORA);
    let rig = Rig::acp("replay");
    let node = rig.node_without_holder();
    let id = rig.launch(&node);
    wait_for("the agent to be idle", || {
        state_of(&node, &id) == Some(AgentState::Idle)
    });
    prompt(&node, &id, "hello-1");
    wait_for("the first answer", || said(&node, &id, "hello-1"));
    prompt(&node, &id, "hello-2");
    wait_for("the second answer", || said(&node, &id, "hello-2"));
    wait_for("both turns on disk", || {
        on_disk(&rig, &id, "hello-1") && on_disk(&rig, &id, "hello-2")
    });
    // Contained, not equal: replayed pieces may arrive joined into one entry.
    let count = |node: &Node, text: &str| {
        chat(node, &id)
            .iter()
            .filter(|e| e.text.contains(text))
            .count()
    };
    let on_disk_count = |text: &str| {
        std::fs::read_to_string(transcript_file(&rig, &id))
            .unwrap_or_default()
            .matches(text)
            .count()
    };
    let texts = |node: &Node| {
        chat(node, &id)
            .iter()
            .filter(|e| e.kind == v1::ChatKind::Text as i32)
            .count()
    };
    let before = texts(&node);
    drop(node);
    for round in 0..2 {
        let node = rig.node_without_holder();
        wait_for("the agent to be offered for resume", || {
            node.ledger.get(&id).is_some_and(|e| e.interrupted())
        });
        let mut c = Client::connect(node.socket().unwrap()).expect("control");
        let r = c
            .call(Cmd::AgentResume(v1::AgentResume {
                agent_id: id.clone(),
                new_session: false,
                ..Default::default()
            }))
            .unwrap();
        assert!(r.is_ok(), "{r:?}");
        wait_for("the resumed agent to be idle", || {
            state_of(&node, &id) == Some(AgentState::Idle)
        });
        // Nothing of the replay was added: as many of the agent's texts as before the restart (and one per round since).
        std::thread::sleep(Duration::from_millis(500));
        assert_eq!(texts(&node), before + round, "{:#?}", chat(&node, &id));
        let word = format!("after-{round}");
        prompt(&node, &id, &word);
        wait_for("the answer after the resume", || said(&node, &id, &word));
        // The replay came before the session's answer, so it was taken in before the answer to this prompt.
        for text in ["hello-1", "hello-2"] {
            assert_eq!(
                count(&node, text),
                1,
                "{text} in round {round}: {:#?}",
                chat(&node, &id)
            );
        }
        wait_for("the new turn on disk", || on_disk(&rig, &id, &word));
        for text in ["hello-1", "hello-2"] {
            assert_eq!(on_disk_count(text), 1, "{text} on disk in round {round}");
        }
        drop(node);
    }
}

/// Review of #1103, round 3: on a node that holds a secret, the agent's newest words are held back while its turn runs, and reach the
/// file when it ends (no stop needed), with the node's value replaced.
#[test]
fn a_node_holding_a_secret_writes_an_answer_when_the_turn_ends_without_the_value() {
    let _serial = serial();
    let rig = Rig::new("secret");
    let value = "SECRETVALUE-ABCDEFGH-XYZ";
    agora_node::credentials::SecretStore::in_data(&rig.root.join("data"))
        .set(
            "TOKEN",
            &agora_node::credentials::Secret::new(value).unwrap(),
        )
        .unwrap();
    let node = rig.node();
    let id = started(&rig, &node);
    let words = format!("my token is {value} ok");
    prompt(&node, &id, &format!("hold:{words}"));
    wait_for("the words", || said(&node, &id, &words));
    wait_for("the agent to work", || {
        state_of(&node, &id) == Some(AgentState::Working)
    });
    // The writer ran (it does each second): the turn is open, so the words wait.
    std::thread::sleep(Duration::from_millis(2_500));
    assert!(
        !on_disk(&rig, &id, "my token is"),
        "written while the turn was open"
    );
    std::fs::write(rig.folder.join("go.txt"), "go").expect("go");
    wait_for("the agent to be idle", || {
        state_of(&node, &id) == Some(AgentState::Idle)
    });
    wait_for("the words on disk", || on_disk(&rig, &id, "my token is"));
    let body = std::fs::read_to_string(transcript_file(&rig, &id)).unwrap();
    assert!(!body.contains(value), "{body}");
    assert!(body.contains("my token is <redacted> ok"), "{body}");
}

/// ADR-0104 O2 with #1103's held-back words: the node stops while the agent's words end inside a secret value. The stop writes what it
/// held, the end of the open words taken as an edge, so the start of the value there is masked; the agent goes on under the holder and
/// the next node writes the rest. No piece of the value of 4 bytes or more is ever on disk.
#[test]
fn a_stop_in_the_middle_of_a_secret_value_leaves_no_piece_of_it_on_disk() {
    let _serial = serial();
    let rig = Rig::new("secret-stop");
    let value = "SECRETVALUE-ABCDEFGH-XYZ";
    agora_node::credentials::SecretStore::in_data(&rig.root.join("data"))
        .set(
            "TOKEN",
            &agora_node::credentials::Secret::new(value).unwrap(),
        )
        .unwrap();
    let pieces: Vec<&str> = (0..=value.len() - 4).map(|i| &value[i..i + 4]).collect();
    let leaked = |body: &str| -> Vec<String> {
        pieces
            .iter()
            .filter(|p| body.contains(**p))
            .map(|p| (*p).to_string())
            .collect()
    };
    let node = rig.node();
    let id = started(&rig, &node);
    let pid = pid_of(&node, &id);
    prompt(&node, &id, "hold:my token is SECRETVA|LUE-ABCDEFGH-XYZ ok");
    wait_for("the first words", || {
        chat(&node, &id)
            .iter()
            .any(|e| e.role == v1::ChatRole::Agent as i32 && e.text.ends_with("SECRETVA"))
    });
    wait_for("the agent to work", || {
        state_of(&node, &id) == Some(AgentState::Working)
    });
    drop(node);
    assert!(alive(pid), "the adapter ended with its node");
    let body = std::fs::read_to_string(transcript_file(&rig, &id)).unwrap_or_default();
    assert!(
        body.contains("my token is"),
        "the stop did not write the held words: {body}"
    );
    assert!(
        leaked(&body).is_empty(),
        "{:?} on disk after the stop: {body}",
        leaked(&body)
    );

    let node = rig.node();
    assert!(held(&node, &id), "the agent was not taken up again");
    std::fs::write(rig.folder.join("go.txt"), "go").expect("go");
    wait_for("the agent to be idle", || {
        state_of(&node, &id) == Some(AgentState::Idle)
    });
    wait_for("the rest on disk", || on_disk(&rig, &id, " ok"));
    let body = std::fs::read_to_string(transcript_file(&rig, &id)).unwrap();
    assert!(
        leaked(&body).is_empty(),
        "{:?} on disk after the rest: {body}",
        leaked(&body)
    );
}
