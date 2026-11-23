//! ADR-0107 (#1121): `agent.list` lists the agents a node found interrupted (ADR-0060) in `AgentListResult.interrupted`, each the
//! `AgentInterrupted` the node announces for it, and never in `agents`; an agent that is resumed or started again leaves that list and
//! is listed from the supervisor. Agents are `agora-fake-agent` (never a real agent); the node is dropped (its agents end with it, as
//! when a machine stops) or crashed (killed without stopping its agents, which outlive it) and started again on the same data folder.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use agora_node::{
    control::Client,
    daemon::{Node, NodeConfig},
    supervisor::NodeLimits,
};
use agora_protocol::v1::{self, command_ok::Result as Ok_, command_request::Command as Cmd};
use std::{
    path::PathBuf,
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
    /// Processes a crashed node left running, ended when the test ends whatever happened (`Node::crash` leaves them for good).
    survivors: Vec<agora_platform::ProcessIdentity>,
}

impl Env {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("interrupted-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("manifests")).unwrap();
        std::fs::create_dir_all(root.join("work")).unwrap();
        let q = |v: Vec<String>| serde_json::to_string(&v).unwrap();
        let probe = |file: &str| {
            vec![
                fake(),
                "--probe-directory".into(),
                root.join(file).to_string_lossy().into_owned(),
            ]
        };
        // A terminal agent whose session id the node chooses, and which it can resume: Resume is offered.
        let mut resume = probe("resume-{pid}.json");
        resume.extend(["--resume".to_string(), "{session_id}".to_string()]);
        std::fs::write(
            root.join("manifests/probe.toml"),
            format!(
                "kind = \"probe\"\ndisplay_name = \"probe\"\nversion = \"1\"\nmin_engine = 2\n[process]\nlaunch = {}\n[resume]\nargv = {}\n[session]\nid_args = [\"--session-id\", \"{{session_id}}\"]\n",
                q(probe("start-{pid}.json")),
                q(resume)
            ),
        )
        .unwrap();
        // A terminal agent the node learns no session of: only Start again.
        std::fs::write(
            root.join("manifests/plain.toml"),
            format!(
                "kind = \"plain\"\ndisplay_name = \"plain\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = {}\n",
                q(probe("plain-{pid}.json"))
            ),
        )
        .unwrap();
        let mut e = Self {
            root,
            node: None,
            survivors: Vec::new(),
        };
        e.start();
        e
    }

    fn start(&mut self) {
        self.node = None;
        self.node = Some(
            Node::start(&NodeConfig {
                resume_all: false,
                node_id: "node-interrupted".into(),
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

    fn launch(&self, kind: &str) -> String {
        let mut c = self.client();
        let info = match c
            .call(Cmd::DirectoryInspect(v1::DirectoryInspect {
                path: self.root.join("work").to_string_lossy().into_owned(),
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

    fn stop(&self, id: &str) {
        self.client()
            .call(Cmd::AgentStop(v1::AgentStop {
                agent_id: id.into(),
                stop_helpers: false,
            }))
            .unwrap()
            .unwrap();
    }

    fn resume(&self, id: &str, new_session: bool) {
        match self
            .client()
            .call(Cmd::AgentResume(v1::AgentResume {
                agent_id: id.into(),
                new_session,
                authorization: None,
            }))
            .unwrap()
            .unwrap()
            .result
        {
            Some(Ok_::AgentLaunch(r)) => assert_eq!(r.agent_id, id),
            other => panic!("{other:?}"),
        }
    }

    fn list(&self) -> v1::AgentListResult {
        match self
            .client()
            .call(Cmd::AgentList(v1::AgentList::default()))
            .unwrap()
            .unwrap()
            .result
        {
            Some(Ok_::AgentList(l)) => {
                let n = |id: &str| {
                    id.strip_prefix("agent-")
                        .and_then(|d| d.parse::<u64>().ok())
                        .unwrap_or(u64::MAX)
                };
                assert!(
                    l.interrupted
                        .windows(2)
                        .all(|w| n(&w[0].agent_id) < n(&w[1].agent_id)),
                    "interrupted not in id order: {:?}",
                    l.interrupted
                );
                // Never both: an id is in `agents` or in `interrupted`.
                for i in &l.interrupted {
                    assert!(
                        !l.agents.iter().any(|a| a.agent_id == i.agent_id),
                        "{} listed twice: {l:?}",
                        i.agent_id
                    );
                }
                l
            }
            other => panic!("{other:?}"),
        }
    }

    /// The last `AgentInterrupted` the node announced for `id` (what the hub, the window and the terminal client show).
    fn announced(&self, id: &str) -> v1::AgentInterrupted {
        self.node()
            .events
            .events_after(0)
            .into_iter()
            .filter_map(|e| match e.body {
                Some(v1::event::Body::AgentInterrupted(i)) if i.agent_id == id => Some(i),
                _ => None,
            })
            .next_back()
            .unwrap_or_else(|| panic!("no AgentInterrupted for {id}"))
    }

    fn wait(&self, what: &str, ok: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while !ok(self) {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        self.node = None;
        for p in &self.survivors {
            let _ = agora_platform::kill_process_if_same(p);
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn ids(l: &[v1::AgentInterrupted]) -> Vec<&str> {
    l.iter().map(|i| i.agent_id.as_str()).collect()
}

fn running(l: &v1::AgentListResult, id: &str) -> bool {
    l.agents.iter().any(|a| a.agent_id == id && a.running)
}

#[test]
fn agents_interrupted_by_a_node_restart_are_listed_as_interrupted_until_resumed() {
    if !supported() {
        return;
    }
    let mut e = Env::new("restart");
    let ended = e.launch("plain");
    let probe = e.launch("probe");
    let plain = e.launch("plain");
    assert_eq!(
        (ended.as_str(), probe.as_str(), plain.as_str()),
        ("agent-1", "agent-2", "agent-3")
    );
    e.stop(&ended);
    e.wait("the first agent ended, and the probe's session", |e| {
        e.node().ledger.get(&ended).is_some_and(|x| x.ended)
            && e.node()
                .ledger
                .get(&probe)
                .is_some_and(|x| !x.session_id.is_empty())
    });
    let before = e.list();
    assert!(before.interrupted.is_empty(), "{before:?}");

    // The machine stops: the node goes, its agents end with it. A new node starts on the same data folder.
    e.start();
    let l = e.list();
    // The ended one stays where #1120 lists it; the two that ran are interrupted, not stopped rows.
    assert_eq!(
        l.agents
            .iter()
            .map(|a| (a.agent_id.as_str(), a.running))
            .collect::<Vec<_>>(),
        [(ended.as_str(), false)],
        "{l:?}"
    );
    assert_eq!(ids(&l.interrupted), [probe.as_str(), plain.as_str()]);
    let by = |id: &str| l.interrupted.iter().find(|i| i.agent_id == id).unwrap();
    assert!(by(&probe).resumable && !by(&probe).still_running);
    assert!(!by(&plain).resumable && !by(&plain).still_running);
    assert_eq!(by(&probe).kind, "probe");
    // What the list says is what the node announced: the two never differ.
    for i in &l.interrupted {
        assert_eq!(*i, e.announced(&i.agent_id));
    }

    // Resumed: it is the supervisor's again, listed in `agents` and no longer interrupted.
    e.resume(&probe, false);
    let l = e.list();
    assert!(running(&l, &probe), "{l:?}");
    assert_eq!(ids(&l.interrupted), [plain.as_str()]);
    // Started again (a new session in the same folder): the same.
    e.resume(&plain, true);
    let l = e.list();
    assert!(running(&l, &plain), "{l:?}");
    assert!(l.interrupted.is_empty(), "{l:?}");
}

#[test]
fn an_agent_that_outlived_a_crashed_node_is_listed_as_still_running_and_not_as_an_agent() {
    // Where an agent can outlive its node at all (on Windows its job object ends it with the node, and no identity is kept).
    if !supported() || agora_platform::process_identity(std::process::id()).is_err() {
        return;
    }
    let mut e = Env::new("crash");
    let probe = e.launch("probe");
    e.wait("the probe's process and session in the ledger", |e| {
        e.node()
            .ledger
            .get(&probe)
            .is_some_and(|x| x.process().is_some() && !x.session_id.is_empty())
    });
    let old = e.node().ledger.get(&probe).unwrap().process().unwrap();
    e.survivors.push(old.clone());
    // The node is killed without stopping its agents (kill -9): the probe keeps running.
    e.node.take().unwrap().crash();
    e.start();
    assert!(agora_platform::process_still_running(&old));
    let l = e.list();
    assert!(l.agents.is_empty(), "{l:?}");
    assert_eq!(ids(&l.interrupted), [probe.as_str()]);
    assert!(l.interrupted[0].still_running && l.interrupted[0].resumable);
    assert_eq!(l.interrupted[0], e.announced(&probe));

    // `agent.stop` ends exactly that process; it stays interrupted, now without a process, and can be resumed.
    e.stop(&probe);
    e.wait("the old process to end", |_| {
        !agora_platform::process_still_running(&old)
    });
    let l = e.list();
    assert_eq!(ids(&l.interrupted), [probe.as_str()]);
    assert!(!l.interrupted[0].still_running && l.interrupted[0].resumable);
    assert_eq!(l.interrupted[0], e.announced(&probe));
    e.resume(&probe, false);
    let l = e.list();
    assert!(running(&l, &probe), "{l:?}");
    assert!(l.interrupted.is_empty(), "{l:?}");
}
