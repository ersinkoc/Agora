//! #1120: an agent that ended is still listed by `agent.list` after the node restarts, while the ledger keeps it (ADR-0060, 14 days),
//! whatever folder it ran in: a project worktree, a folder of its own (here a Git root on `main`, as on Windows and the Mac), a separate
//! worktree the node made for a folder, or a folder outside Git. Agents are `agora-fake-agent` (never a real agent); the node is
//! dropped and started again on the same data folder, as `agora down` and a new start do.
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

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .status()
        .expect("git")
        .success();
    assert!(ok, "git {args:?}");
}

struct Env {
    root: PathBuf,
    node: Option<Node>,
}

impl Env {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("ended-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // A Git root on `main`, which is both the registered project and the folder of the directory agents.
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "x").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        std::fs::create_dir_all(root.join("plain")).unwrap();
        std::fs::create_dir_all(root.join("manifests")).unwrap();
        let launch = serde_json::to_string(&vec![
            fake(),
            "--probe-directory".into(),
            root.join("probe-{pid}.json").to_string_lossy().into_owned(),
        ])
        .unwrap();
        std::fs::write(
            root.join("manifests/probe.toml"),
            format!(
                "kind = \"probe\"\ndisplay_name = \"probe\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = {launch}\n"
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
                resume_all: false,
                node_id: "node-ended".into(),
                socket: self.root.join("n.sock"),
                data_dir: self.root.join("data"),
                worktree_root: self.root.join("worktrees"),
                manifests_dir: self.root.join("manifests"),
                projects: vec![self.root.join("repo")],
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

    /// A worktree agent of the registered project (`agent.start`).
    fn start_in_project(&self) -> String {
        match self
            .client()
            .call(Cmd::AgentStart(v1::AgentStart {
                kind: "probe".into(),
                ..Default::default()
            }))
            .unwrap()
            .unwrap()
            .result
        {
            Some(Ok_::AgentStart(r)) => r.agent_id,
            other => panic!("{other:?}"),
        }
    }

    /// A directory agent (`agent.launch`) in `folder`, in a worktree the node makes when `separate`.
    fn launch_in(&self, folder: &Path, separate: bool) -> String {
        let mut c = self.client();
        let info = match c
            .call(Cmd::DirectoryInspect(v1::DirectoryInspect {
                path: folder.to_string_lossy().into_owned(),
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
                kind: "probe".into(),
                separate_worktree: separate,
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

    fn list(&self) -> Vec<v1::AgentSummary> {
        match self
            .client()
            .call(Cmd::AgentList(v1::AgentList::default()))
            .unwrap()
            .unwrap()
            .result
        {
            Some(Ok_::AgentList(l)) => {
                let mut v = l.agents;
                v.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));
                v
            }
            other => panic!("{other:?}"),
        }
    }

    fn wait(&self, what: &str, ok: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !ok(self) {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        self.node = None;
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// What `agent.list` says of one agent, as compared across the restart (the folder canonical: a node may say it either way).
fn row(a: &v1::AgentSummary) -> (String, String, i32, PathBuf, bool) {
    (
        a.agent_id.clone(),
        a.kind.clone(),
        a.state,
        std::fs::canonicalize(&a.worktree_id).unwrap_or_else(|_| PathBuf::from(&a.worktree_id)),
        a.running,
    )
}

#[test]
fn agents_that_ended_are_listed_after_a_node_restart_whatever_their_folder() {
    if !(agora_platform::PTY_SUPPORTED && agora_platform::LOCAL_SOCKETS_SUPPORTED) {
        return;
    }
    let mut e = Env::new("kinds");
    let repo = e.root.join("repo");
    let worktree = e.start_in_project();
    let directory = e.launch_in(&repo, false);
    let separate = e.launch_in(&repo, true);
    let plain = e.launch_in(&e.root.join("plain"), false);
    let ids = [&worktree, &directory, &separate, &plain];
    for id in ids {
        e.stop(id);
    }
    // Ended and done, so the list before the restart is compared with a ledger that has caught up with it.
    e.wait("every agent ended and done in the ledger", |e| {
        ids.iter().all(|id| {
            e.node()
                .ledger
                .get(id)
                .is_some_and(|x| x.ended && x.last_state == "done")
        })
    });
    // What each kind of folder is, as the ledger holds it (Windows' agent-53 and the Mac's agent-31 are `directory`, not managed).
    let entry = |id: &str| e.node().ledger.get(id).unwrap();
    assert!(!entry(&worktree).directory);
    assert!(entry(&directory).directory && !entry(&directory).managed_worktree);
    assert!(entry(&separate).directory && entry(&separate).managed_worktree);
    assert!(entry(&plain).directory && !entry(&plain).managed_worktree);
    let before: Vec<_> = e.list().iter().map(row).collect();
    assert_eq!(before.len(), 4, "{before:?}");
    assert!(before.iter().all(|r| !r.4), "{before:?}");

    e.start();
    let after: Vec<_> = e.list().iter().map(row).collect();
    assert_eq!(after, before, "the restarted node lists what it listed");
    // Nothing was started again, and none of them is offered to resume (they ended).
    assert!(e.node().supervisor.lock().unwrap().list().is_empty());
    assert!(e.node().ledger.interrupted().is_empty());
}
