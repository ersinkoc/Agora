//! ADR-0064 end to end: a node records the worktrees it makes for its agents (its own and a separate one for a folder), keeps them
//! while their agents run, refuses a resume into a workspace a clean-up has claimed, and `agora workspaces clean`'s path removes them
//! once the agents ended. Agents are `agora-fake-agent`; homes are temporary.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use agora_node::{
    control::Client,
    daemon::{Node, NodeConfig},
    supervisor::NodeLimits,
    workspaces::{Action, Clean, State, Users, Workspaces},
};
use agora_protocol::v1::{self, command_ok::Result as Ok_, command_request::Command as Cmd};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::AtomicBool,
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
        let root = std::env::temp_dir().join(format!("ws-node-{}-{name}", std::process::id()));
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
                node_id: "node-ws".into(),
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

fn by_hand(e: &Env, ws: &Workspaces) -> Vec<Action> {
    let data = e.root.join("data");
    let never = AtomicBool::new(false);
    ws.clean(
        &|| Users::from_ledger(&agora_node::ledger::read_entries(&data).unwrap()),
        agora_node::grants::now_ms(),
        Clean {
            by_node: false,
            older_than_ms: None,
            dry_run: false,
        },
        &never,
    )
    .into_iter()
    .map(|o| o.action)
    .collect()
}

#[test]
fn the_node_records_its_worktrees_keeps_them_while_used_and_they_go_once_their_agents_ended() {
    if !(agora_platform::PTY_SUPPORTED && agora_platform::LOCAL_SOCKETS_SUPPORTED) {
        return;
    }
    let e = Env::new("e2e");
    let repo = e.root.join("repo");
    let own = e.start_in_project();
    let separate = e.launch_in(&repo, true);
    // A directory agent in the checkout itself: never recorded, never removed.
    let shared = e.launch_in(&repo, false);
    let ws = Workspaces::at(&e.root.join("data"), &e.root.join("worktrees"));
    assert_eq!(ws.records().len(), 2, "both worktrees the node made");
    e.wait("the agents in the ledger", |e| {
        [&own, &separate, &shared]
            .iter()
            .all(|id| e.node().ledger.get(id).is_some())
    });
    e.node().ledger.flush().unwrap();
    assert_eq!(
        by_hand(&e, &ws),
        vec![Action::Kept(State::InUse), Action::Kept(State::InUse)]
    );
    for id in [&own, &separate, &shared] {
        e.stop(id);
    }
    e.wait("every agent ended in the ledger", |e| {
        [&own, &separate, &shared]
            .iter()
            .all(|id| e.node().ledger.get(id).is_some_and(|x| x.ended))
    });
    e.node().ledger.flush().unwrap();

    // A clean-up claimed the agent's own worktree: an agent started in that folder meanwhile is refused.
    let dir = e.root.join("data").join("workspaces");
    let own_folder = PathBuf::from(e.node().ledger.get(&own).unwrap().folder);
    let own_folder = agora_platform::canonicalize_plain(&own_folder).unwrap();
    let records: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|x| x.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    let claimed = records
        .iter()
        .find(|p| {
            std::fs::read_to_string(p)
                .unwrap()
                .contains(own_folder.to_str().unwrap())
        })
        .unwrap()
        .clone();
    std::fs::rename(&claimed, claimed.with_extension("claim")).unwrap();
    let mut c = e.client();
    let info = match c
        .call(Cmd::DirectoryInspect(v1::DirectoryInspect {
            path: own_folder.to_string_lossy().into_owned(),
            ..Default::default()
        }))
        .unwrap()
        .unwrap()
        .result
    {
        Some(Ok_::DirectoryInfo(i)) => i,
        other => panic!("{other:?}"),
    };
    let refused = c
        .call(Cmd::AgentLaunch(v1::AgentLaunch {
            path: info.path,
            project_id: info.project_id,
            kind: "probe".into(),
            separate_worktree: false,
            ..Default::default()
        }))
        .unwrap()
        .unwrap_err();
    assert_eq!(refused.code, "protocol.invalid_message", "{refused:?}");
    assert!(
        format!("{refused:?}").contains("directory.path"),
        "{refused:?}"
    );
    std::fs::rename(claimed.with_extension("claim"), &claimed).unwrap();

    assert_eq!(by_hand(&e, &ws), vec![Action::Removed, Action::Removed]);
    assert!(!own_folder.exists());
    assert!(ws.records().is_empty());
    assert!(
        repo.join("a.txt").is_file(),
        "the checkout itself is never touched"
    );
}
