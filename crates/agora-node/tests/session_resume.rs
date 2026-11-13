//! ADR-0060: a real node restart. Agents are started with `agora-fake-agent` (never a real agent), the node object is dropped (its
//! agents die with it, as when a machine stops), a new node starts on the same data folder, the agents are interrupted, and Resume
//! brings them back under the same id with the same session id.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use agora_node::{
    control::Client,
    daemon::{Node, NodeConfig},
    supervisor::NodeLimits,
};
use agora_protocol::v1::{self, command_ok::Result as Ok_, command_request::Command as Cmd};
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

fn fake() -> String {
    let p = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(format!("agora-fake-agent{}", std::env::consts::EXE_SUFFIX));
    static BUILD: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    BUILD.get_or_init(|| {
        // Even when present: a binary left from an older checkout may lack newer behaviour.
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let mut command = Command::new(cargo);
        command
            .args(["build", "-q", "-p", "agora-fake-agent", "--target-dir"])
            .arg(p.parent().unwrap().parent().unwrap());
        if p.parent().unwrap().ends_with("release") {
            command.arg("--release");
        }
        assert!(
            command.status().unwrap().success(),
            "building the fake failed"
        );
    });
    p.to_string_lossy().into_owned()
}

fn supported() -> bool {
    agora_platform::PTY_SUPPORTED && agora_platform::LOCAL_SOCKETS_SUPPORTED
}

struct Env {
    root: PathBuf,
    node: Option<Node>,
}

impl Env {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("resume-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("manifests")).unwrap();
        std::fs::create_dir_all(root.join("work/a")).unwrap();
        let q = |v: Vec<String>| serde_json::to_string(&v).unwrap();
        let probe = |file: &str| {
            vec![
                fake(),
                "--probe-directory".into(),
                root.join(file).to_string_lossy().into_owned(),
            ]
        };
        let head = |kind: &str| {
            format!(
                "kind = \"{kind}\"\ndisplay_name = \"{kind}\"\nversion = \"1\"\nmin_engine = 1\n"
            )
        };
        // A terminal agent whose session id the node chooses, and resumes with it.
        let mut resume = probe("resume-{pid}.json");
        resume.extend(["--resume".to_string(), "{session_id}".to_string()]);
        std::fs::write(
            root.join("manifests/probe.toml"),
            format!(
                "{}[process]\nlaunch = {}\n[resume]\nargv = {}\n[session]\nid_args = [\"--session-id\", \"{{session_id}}\"]\n",
                head("probe"),
                q(probe("start-{pid}.json")),
                q(resume)
            ),
        )
        .unwrap();
        // A terminal agent the node learns no session of: only Start again.
        std::fs::write(
            root.join("manifests/plain.toml"),
            format!(
                "{}[process]\nlaunch = {}\n",
                head("plain"),
                q(probe("plain-{pid}.json"))
            ),
        )
        .unwrap();
        // An adapter agent: its session comes from the adapter (`session/reported`).
        std::fs::write(
            root.join("manifests/adapter.toml"),
            format!(
                "{}[adapter]\ncommand = {}\n",
                head("adapter"),
                q(vec![fake(), "--adapter".into()])
            ),
        )
        .unwrap();
        let mut e = Self { root, node: None };
        e.start();
        e
    }

    fn start(&mut self) {
        self.node = None;
        self.node = Some(
            Node::start(&NodeConfig {
                node_id: "node-resume".into(),
                socket: self.root.join("n.sock"),
                data_dir: self.root.join("data"),
                worktree_root: self.root.join("worktrees"),
                manifests_dir: self.root.join("manifests"),
                projects: vec![],
                limits: NodeLimits::default(),
            })
            .unwrap(),
        );
    }

    fn node(&self) -> &Node {
        self.node.as_ref().unwrap()
    }

    fn client(&self) -> Client {
        Client::connect(self.node().socket().unwrap()).unwrap()
    }

    fn folder(&self) -> PathBuf {
        self.root.join("work/a")
    }

    fn launch(&self, kind: &str) -> String {
        let mut c = self.client();
        let info = match c
            .call(Cmd::DirectoryInspect(v1::DirectoryInspect {
                path: self.folder().to_string_lossy().into_owned(),
                ..Default::default()
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
                path: info.path,
                project_id: info.project_id,
                kind: kind.into(),
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

    fn resume(&self, id: &str, new_session: bool) -> Result<v1::AgentLaunchResult, String> {
        match self
            .client()
            .call(Cmd::AgentResume(v1::AgentResume {
                agent_id: id.into(),
                new_session,
                authorization: None,
            }))
            .unwrap()
        {
            Ok(v1::CommandOk {
                result: Some(Ok_::AgentLaunch(r)),
            }) => Ok(r),
            Ok(other) => panic!("{other:?}"),
            Err(e) => Err(e.code),
        }
    }

    fn running(&self, id: &str) -> bool {
        self.node()
            .supervisor
            .lock()
            .unwrap()
            .list()
            .iter()
            .any(|v| v.id.as_str() == id && v.running)
    }

    fn session(&self, id: &str) -> String {
        self.node()
            .ledger
            .get(id)
            .map(|e| e.session_id)
            .unwrap_or_default()
    }

    fn wait(&self, what: &str, ok: impl Fn(&Self) -> bool) {
        let end = Instant::now() + Duration::from_secs(20);
        while !ok(self) {
            assert!(Instant::now() < end, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// The probe reports (`<prefix>-<pid>.json`) written so far.
    fn reports(&self, prefix: &str) -> Vec<serde_json::Value> {
        std::fs::read_dir(&self.root)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(prefix))
            .filter_map(|e| serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok())
            .collect()
    }

    fn interrupted(&self) -> Vec<v1::AgentInterrupted> {
        self.node()
            .events
            .events_after(0)
            .into_iter()
            .filter_map(|e| match e.body {
                Some(v1::event::Body::AgentInterrupted(i)) => Some(i),
                _ => None,
            })
            .collect()
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        self.node = None;
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn args(report: &serde_json::Value) -> Vec<String> {
    report["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn agents_interrupted_by_a_node_restart_resume_under_the_same_id_with_the_same_session() {
    if !supported() {
        return;
    }
    let mut e = Env::new("restart");
    let adapter = e.launch("adapter");
    let probe = e.launch("probe");
    let plain = e.launch("plain");
    assert_eq!(
        (adapter.as_str(), probe.as_str(), plain.as_str()),
        ("agent-1", "agent-2", "agent-3")
    );
    // The adapter's own session, and the one the node chose for the probe (passed as `--session-id`).
    e.wait("the adapter's session", |e| {
        e.session(&adapter) == "native-s-1"
    });
    let chosen = e.session(&probe);
    assert_eq!(chosen.len(), 36, "{chosen}");
    e.wait("the probe's start", |e| !e.reports("start-").is_empty());
    let first = args(&e.reports("start-")[0]);
    assert!(
        first
            .windows(2)
            .any(|w| w[0] == "--session-id" && w[1] == chosen),
        "{first:?}"
    );
    assert!(e.session(&plain).is_empty());

    // The machine stops: the node goes, its agents die with it. A new node starts on the same data folder.
    e.start();
    let found = e.interrupted();
    let by = |id: &str| found.iter().find(|i| i.agent_id == id).cloned().unwrap();
    assert!(by(&adapter).resumable && by(&probe).resumable && !by(&plain).resumable);
    assert_eq!(
        std::fs::canonicalize(&by(&probe).folder).unwrap(),
        std::fs::canonicalize(e.folder()).unwrap()
    );
    assert!(!e.running(&adapter) && !e.running(&probe));

    // Resume: same id, same folder, same session.
    let r = e.resume(&adapter, false).unwrap();
    assert_eq!(r.agent_id, adapter);
    assert!(e.running(&adapter));
    e.wait("the adapter's session again", |e| {
        e.node()
            .ledger
            .get(&adapter)
            .is_some_and(|x| !x.interrupted())
    });
    assert_eq!(e.session(&adapter), "native-s-1");

    let r = e.resume(&probe, false).unwrap();
    assert_eq!(r.agent_id, probe);
    assert!(!r.terminal_id.is_empty());
    e.wait("the probe's resume", |e| !e.reports("resume-").is_empty());
    let report = &e.reports("resume-")[0];
    let resumed = args(report);
    assert!(
        resumed
            .windows(2)
            .any(|w| w[0] == "--resume" && w[1] == chosen),
        "{resumed:?}"
    );
    // A resume is not a first launch: no new session id.
    assert!(!resumed.iter().any(|a| a == "--session-id"), "{resumed:?}");
    assert_eq!(
        std::fs::canonicalize(report["cwd"].as_str().unwrap()).unwrap(),
        std::fs::canonicalize(e.folder()).unwrap()
    );
    assert_eq!(e.session(&probe), chosen);

    // Twice is refused; without a session only Start again.
    assert_eq!(
        e.resume(&probe, false).unwrap_err(),
        "agent.not_interrupted"
    );
    assert_eq!(
        e.resume(&plain, false).unwrap_err(),
        "agent.session_missing"
    );
    assert_eq!(
        e.resume("agent-77", false).unwrap_err(),
        "agent.not_interrupted"
    );
    let r = e.resume(&plain, true).unwrap();
    assert_eq!(r.agent_id, plain);
    assert!(e.running(&plain));

    // An id is never given twice: the next agent is agent-4.
    assert_eq!(e.launch("plain"), "agent-4");
}

#[test]
fn a_folder_that_is_gone_is_not_resumed_and_the_owner_can_resume_on_their_own() {
    if !supported() {
        return;
    }
    let mut e = Env::new("folder");
    let adapter = e.launch("adapter");
    e.wait("the adapter's session", |e| {
        e.session(&adapter) == "native-s-1"
    });
    let probe = e.launch("probe");
    e.start();
    // On Windows a folder cannot be removed while a process that is being ended still has it as its working folder: try for a while.
    let end = Instant::now() + Duration::from_secs(20);
    while let Err(err) = std::fs::remove_dir_all(e.folder()) {
        assert!(Instant::now() < end, "{err}");
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(e.resume(&probe, false).unwrap_err(), "agent.folder_changed");
    assert_eq!(e.resume(&probe, true).unwrap_err(), "agent.folder_changed");
    assert!(e.node().ledger.get(&probe).unwrap().interrupted());

    // `resume = auto` for the adapter kind in that folder (made again): the next start brings it back by itself, and not the probe.
    std::fs::create_dir_all(e.folder()).unwrap();
    std::fs::write(
        e.root.join("data/policy"),
        format!(
            "resume = auto\nresume.kinds = adapter\nresume.folders = {}\n",
            e.root.join("work").display()
        ),
    )
    .unwrap();
    e.start();
    assert!(e.running(&adapter));
    assert!(!e.running(&probe));
    assert!(e.node().ledger.get(&probe).unwrap().interrupted());
}

#[test]
fn a_helper_is_resumed_only_while_its_parent_runs() {
    if !supported() {
        return;
    }
    let mut e = Env::new("helper");
    let parent = e.launch("plain");
    let helper = e.launch("plain");
    // The ledger as it would be after an agent_start: the helper names its parent (written by the node's own events).
    e.node()
        .events
        .record_bodies(vec![v1::event::Body::AgentStarted(v1::AgentStarted {
            agent_id: helper.clone(),
            project_id: e.node().ledger.get(&helper).unwrap().project_id,
            node_id: "node-resume".into(),
            worktree_id: String::new(),
            kind: "plain".into(),
            level: 2,
            parent_agent_id: parent.clone(),
        })]);
    e.start();
    assert_eq!(
        e.resume(&helper, true).unwrap_err(),
        "agent.parent_not_running"
    );
    e.resume(&parent, true).unwrap();
    let r = e.resume(&helper, true).unwrap();
    assert_eq!(r.agent_id, helper);
    let views = e.node().supervisor.lock().unwrap().list();
    let v = views.iter().find(|v| v.id.as_str() == helper).unwrap();
    assert_eq!(v.parent.as_ref().map(|p| p.as_str()), Some(parent.as_str()));
    assert!(Path::new(&r.path).is_dir());
}

const BAD_IDS: &[&str] = &[
    "--dangerously-bypass-approvals-and-sandbox",
    "-x",
    "a b",
    "a;b",
    "ünïcode",
    "",
];

#[test]
fn start_again_gives_a_kind_that_chooses_its_ids_a_new_one_it_can_resume_later() {
    if !supported() {
        return;
    }
    let mut e = Env::new("again");
    let probe = e.launch("probe");
    let first = e.session(&probe);
    e.start();
    let r = e.resume(&probe, true).unwrap();
    assert_eq!(r.agent_id, probe);
    // A new id, passed as at a first launch, and kept as its session.
    e.wait("the new session", |e| {
        let s = e.session(&probe);
        !s.is_empty() && s != first
    });
    let fresh = e.session(&probe);
    assert_eq!(fresh.len(), 36);
    e.wait("the second start", |e| e.reports("start-").len() >= 2);
    assert!(e.reports("start-").iter().any(|r| {
        args(r)
            .windows(2)
            .any(|w| w[0] == "--session-id" && w[1] == fresh)
    }));
    // So after the next restart it can be resumed with it.
    e.start();
    assert!(e
        .interrupted()
        .iter()
        .any(|i| i.agent_id == probe && i.resumable));
    e.resume(&probe, false).unwrap();
    e.wait("the resume", |e| !e.reports("resume-").is_empty());
    assert!(args(&e.reports("resume-")[0])
        .windows(2)
        .any(|w| w[0] == "--resume" && w[1] == fresh));
}

#[test]
fn a_session_id_that_could_be_an_option_never_reaches_a_command_line() {
    if !supported() {
        return;
    }
    let long = "a".repeat(129);
    let mut bad: Vec<&str> = BAD_IDS.to_vec();
    bad.push(&long);
    let mut e = Env::new("inject");
    let probe = e.launch("probe");
    // (d) A hook or file reporting one for a terminal agent: refused.
    for id in &bad {
        let set = e
            .node()
            .supervisor
            .lock()
            .unwrap()
            .report_session(&agora_core::AgentId::new(&probe), id);
        assert!(set.is_err(), "{id:?}");
    }
    // Recorded as reported anyway (a forged event): the ledger does not keep it.
    for id in &bad {
        e.node()
            .events
            .record_bodies(vec![v1::event::Body::AgentSessionReported(
                v1::AgentSessionReported {
                    agent_id: probe.clone(),
                    session_id: id.to_string(),
                },
            )]);
        assert!(e.session(&probe).is_empty(), "{id:?}");
    }
    // (b) Written into the ledger file while the node is down: dropped as it loads, so (c) Resume is refused and nothing starts.
    e.node = None;
    let file = e.root.join("data/sessions/ledger.json");
    let mut ledger: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    for entry in ledger["entries"].as_array_mut().unwrap() {
        entry["session_id"] = serde_json::Value::from("--dangerously-bypass-approvals-and-sandbox");
    }
    std::fs::write(&file, serde_json::to_vec(&ledger).unwrap()).unwrap();
    e.start();
    assert!(e
        .interrupted()
        .iter()
        .any(|i| i.agent_id == probe && !i.resumable));
    assert_eq!(
        e.resume(&probe, false).unwrap_err(),
        "agent.session_missing"
    );
    assert!(e.reports("resume-").is_empty());
}

fn stop(e: &Env, id: &str) -> Result<(), String> {
    e.client()
        .call(Cmd::AgentStop(v1::AgentStop {
            agent_id: id.into(),
            stop_helpers: false,
        }))
        .unwrap()
        .map(|_| ())
        .map_err(|err| err.code)
}

#[test]
fn an_agent_that_outlived_a_crashed_node_is_stopped_before_it_is_resumed() {
    // Where an agent can outlive its node at all (on Windows its job object ends it with the node, and no identity is kept).
    if !supported() || agora_platform::process_identity(std::process::id()).is_err() {
        return;
    }
    let mut e = Env::new("crash");
    let probe = e.launch("probe");
    e.wait("the probe's process in the ledger", |e| {
        e.node()
            .ledger
            .get(&probe)
            .is_some_and(|x| x.process().is_some())
    });
    let old = e.node().ledger.get(&probe).unwrap().process().unwrap();
    // The node is killed without stopping its agents (kill -9): the probe keeps running.
    e.node.take().unwrap().crash();
    // A pid that was given to another process since: this test's own, with another start time.
    let file = e.root.join("data/sessions/ledger.json");
    let mut ledger: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    ledger["entries"].as_array_mut().unwrap().push(serde_json::json!({
        "agent_id": "agent-50", "kind": "plain", "project_id": "p", "folder": "/nowhere", "directory": true,
        "started_at_ms": 1, "pid": std::process::id(), "process_group": std::process::id(), "process_started": 1
    }));
    std::fs::write(&file, serde_json::to_vec(&ledger).unwrap()).unwrap();
    e.start();
    assert!(agora_platform::process_still_running(&old));
    let found = e.interrupted();
    assert!(found.iter().any(|i| i.agent_id == probe && i.still_running));
    assert!(found
        .iter()
        .any(|i| i.agent_id == "agent-50" && !i.still_running));
    // Two processes on one conversation are never started: Resume waits until it is stopped.
    assert_eq!(e.resume(&probe, false).unwrap_err(), "agent.still_running");
    assert_eq!(e.resume(&probe, true).unwrap_err(), "agent.still_running");
    // A reused pid is never touched: stopping that entry ends nothing (this test is still here to say so).
    assert!(stop(&e, "agent-50").is_err());
    // Stop ends exactly the old process; then Resume works.
    stop(&e, &probe).unwrap();
    e.wait("the old process to end", |_| {
        !agora_platform::process_still_running(&old)
    });
    assert!(e
        .interrupted()
        .iter()
        .any(|i| i.agent_id == probe && !i.still_running));
    let r = e.resume(&probe, false).unwrap();
    assert_eq!(r.agent_id, probe);
    assert!(e.running(&probe));
}
