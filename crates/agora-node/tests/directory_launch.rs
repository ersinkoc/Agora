//! ADR-0052: real node and fake processes, with user-owned folders and no accounts or real coding agents.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use agora_node::{
    control::Client,
    daemon::{Node, NodeConfig},
    supervisor::NodeLimits,
};
use agora_protocol::v1::{self, command_ok::Result as Ok_, command_request::Command as Cmd};
use prost::Message;
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}
fn fake() -> String {
    let p = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(format!("agora-fake-agent{}", std::env::consts::EXE_SUFFIX));
    assert!(
        p.exists(),
        "build agora-fake-agent before integration tests"
    );
    p.to_string_lossy().into_owned()
}
struct Env {
    root: PathBuf,
    node: Option<Node>,
}
impl Env {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("dir-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("manifests")).unwrap();
        let launch = serde_json::to_string(&vec![
            fake(),
            "--probe-directory".into(),
            root.join("probe-{pid}.json").to_string_lossy().into_owned(),
        ])
        .unwrap();
        std::fs::write(root.join("manifests/probe.toml"), format!("kind = \"probe\"\ndisplay_name = \"Probe\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = {launch}\n[mcp]\nmethod = \"cli_flag\"\npath = \".mcp.json\"\nflag = [\"--mcp-config\", \"{{config_file}}\"]\n")).unwrap();
        std::fs::write(root.join("manifests/adapter.toml"), format!("kind = \"adapter\"\ndisplay_name = \"Adapter\"\nversion = \"1\"\nmin_engine = 1\n[adapter]\ncommand = [{}, \"--adapter\"]\n", serde_json::to_string(&fake()).unwrap())).unwrap();
        let node = Node::start(&NodeConfig {
            node_id: "node-directory".into(),
            socket: root.join("n.sock"),
            data_dir: root.join("data"),
            worktree_root: root.join("worktrees"),
            manifests_dir: root.join("manifests"),
            projects: vec![],
            limits: NodeLimits::default(),
        })
        .unwrap();
        Self {
            root,
            node: Some(node),
        }
    }
    fn client(&self) -> Client {
        Client::connect(self.node.as_ref().unwrap().socket().unwrap()).unwrap()
    }
    fn reports(&self, count: usize) -> Vec<serde_json::Value> {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            let reports: Vec<_> = std::fs::read_dir(&self.root)
                .unwrap()
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with("probe-"))
                .filter_map(|e| serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok())
                .collect();
            if reports.len() >= count {
                return reports;
            }
            assert!(Instant::now() < end, "missing fake cwd report");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
impl Drop for Env {
    fn drop(&mut self) {
        self.node.take();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn inspect(c: &mut Client, path: &Path, create: bool) -> v1::DirectoryInfo {
    let ok = c
        .call(Cmd::DirectoryInspect(v1::DirectoryInspect {
            path: path.to_string_lossy().into_owned(),
            create_project_id: create,
            authorization: None,
        }))
        .unwrap()
        .unwrap();
    match ok.result {
        Some(Ok_::DirectoryInfo(i)) => i,
        other => panic!("{other:?}"),
    }
}
fn launch(
    c: &mut Client,
    info: &v1::DirectoryInfo,
    kind: &str,
    separate: bool,
) -> v1::AgentLaunchResult {
    let ok = c
        .call(Cmd::AgentLaunch(v1::AgentLaunch {
            path: info.path.clone(),
            project_id: info.project_id.clone(),
            kind: kind.into(),
            separate_worktree: separate,
            ..Default::default()
        }))
        .unwrap()
        .unwrap();
    match ok.result {
        Some(Ok_::AgentLaunch(r)) => r,
        other => panic!("{other:?}"),
    }
}
fn supported() -> bool {
    agora_platform::PTY_SUPPORTED && agora_platform::LOCAL_SOCKETS_SUPPORTED
}
#[test]
fn two_sessions_use_the_selected_dirty_subfolder_and_preserve_user_configuration() {
    if !supported() {
        return;
    }
    let e = Env::new("shared");
    let repo = e.root.join("repo");
    let folder = repo.join("nested folder");
    std::fs::create_dir_all(&folder).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(folder.join("keep.txt"), "committed").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "initial"]);
    std::fs::write(folder.join("keep.txt"), "user edits").unwrap();
    std::fs::write(folder.join(".mcp.json"), "user config").unwrap();
    let before = git(&repo, &["worktree", "list", "--porcelain"]);
    let mut c = e.client();
    let info = inspect(&mut c, &folder, false);
    let a = launch(&mut c, &info, "probe", false);
    let b = launch(&mut c, &info, "probe", false);
    assert_eq!(a.workspace_id, b.workspace_id);
    assert_ne!(a.agent_id, b.agent_id);
    let reports = e.reports(2);
    let mut configs = Vec::new();
    for report in reports {
        assert_eq!(
            std::fs::canonicalize(report["cwd"].as_str().unwrap()).unwrap(),
            std::fs::canonicalize(&folder).unwrap()
        );
        let args = report["args"].as_array().unwrap();
        let at = args.iter().position(|v| v == "--mcp-config").unwrap();
        configs.push(args[at + 1].as_str().unwrap().to_string());
    }
    assert_ne!(configs[0], configs[1]);
    for cfg in configs {
        assert!(Path::new(&cfg).starts_with(e.root.join("worktrees/.sessions")));
        assert!(Path::new(&cfg).is_file());
    }
    assert_eq!(git(&repo, &["worktree", "list", "--porcelain"]), before);
    assert_eq!(git(&repo, &["branch", "--show-current"]).trim(), "main");
    for id in [&a.agent_id, &b.agent_id] {
        c.call(Cmd::AgentStop(v1::AgentStop {
            agent_id: id.to_string(),
        }))
        .unwrap()
        .unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(folder.join("keep.txt")).unwrap(),
        "user edits"
    );
    assert_eq!(
        std::fs::read_to_string(folder.join(".mcp.json")).unwrap(),
        "user config"
    );
    e.node
        .as_ref()
        .unwrap()
        .supervisor
        .lock()
        .unwrap()
        .resume(&agora_core::AgentId::new(&a.agent_id), Instant::now())
        .unwrap();
    for report in e.reports(3) {
        assert_eq!(
            std::fs::canonicalize(report["cwd"].as_str().unwrap()).unwrap(),
            std::fs::canonicalize(&folder).unwrap()
        );
    }
}
#[test]
fn a_non_git_folder_is_available_without_restart_and_failed_launches_leave_it_intact() {
    if !supported() {
        return;
    }
    let e = Env::new("nongit");
    let folder = e.root.join("new folder");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("keep"), "x").unwrap();
    let mut c = e.client();
    let info = inspect(&mut c, &folder, false);
    assert!(info.git_root.is_empty());
    assert!(info.project_id.starts_with("folder:"));
    assert!(!folder.join(".agent-project.json").exists());
    let bad = c
        .call(Cmd::AgentLaunch(v1::AgentLaunch {
            path: info.path.clone(),
            project_id: info.project_id.clone(),
            kind: "missing".into(),
            ..Default::default()
        }))
        .unwrap();
    assert!(bad.is_err());
    let a = launch(&mut c, &info, "probe", false);
    e.reports(1);
    let r = c
        .call(Cmd::WorkspaceQuery(v1::WorkspaceQuery {
            workspace_id: a.workspace_id,
            operation: "read".into(),
            path: "keep".into(),
            ..Default::default()
        }))
        .unwrap()
        .unwrap();
    assert!(matches!(r.result, Some(Ok_::FilesRead(_))));
    assert!(folder.join("keep").exists());
    assert!(!folder.join(".git").exists());
}
#[test]
fn an_explicit_worktree_uses_the_selected_subfolder_and_never_switches_the_original_branch() {
    if !supported() {
        return;
    }
    let e = Env::new("isolated");
    let repo = e.root.join("repo");
    let folder = repo.join("sub");
    std::fs::create_dir_all(&folder).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(folder.join("keep"), "x").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "initial"]);
    let mut c = e.client();
    let info = inspect(&mut c, &folder, false);
    let a = launch(&mut c, &info, "probe", true);
    e.reports(1);
    assert_ne!(a.path, info.path);
    assert!(Path::new(&a.path).ends_with("sub"));
    assert!(Path::new(&a.path).join("keep").exists());
    assert_eq!(git(&repo, &["branch", "--show-current"]).trim(), "main");
}

#[test]
fn adapter_sessions_and_resume_keep_the_selected_non_git_directory() {
    if !supported() {
        return;
    }
    let e = Env::new("adapter");
    let folder = e.root.join("adapter folder");
    std::fs::create_dir_all(&folder).unwrap();
    let mut c = e.client();
    let info = inspect(&mut c, &folder, false);
    let a = launch(&mut c, &info, "adapter", false);
    let node = e.node.as_ref().unwrap();
    c.call_on(
        9,
        Cmd::ChatSubscribe(v1::ChatSubscribe {
            agent_id: a.agent_id.clone(),
        }),
    )
    .unwrap()
    .unwrap();
    let mut answers = Vec::new();
    for expected in 1..=2 {
        c.call(Cmd::AgentPrompt(v1::AgentPrompt {
            agent_id: a.agent_id.clone(),
            text: "cwd".into(),
        }))
        .unwrap()
        .unwrap();
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(frame) = c.read_frame(Some(Duration::from_millis(100))).unwrap() {
                let entries =
                    if frame.stream_id == 9 && frame.kind == agora_node::control::KIND_CHAT_ENTRY {
                        vec![v1::ChatEntry::decode(frame.payload.as_slice()).unwrap()]
                    } else if frame.stream_id == 9
                        && frame.kind == agora_node::control::KIND_CHAT_SNAPSHOT
                    {
                        v1::ChatSnapshot::decode(frame.payload.as_slice())
                            .unwrap()
                            .entries
                    } else {
                        vec![]
                    };
                answers.extend(
                    entries
                        .iter()
                        .filter_map(|entry| {
                            serde_json::from_str::<serde_json::Value>(&entry.text).ok()
                        })
                        .filter(|v| v.get("session_cwd").is_some()),
                );
            }
            if answers.len() >= expected {
                for v in &answers {
                    assert_eq!(
                        std::fs::canonicalize(v["cwd"].as_str().unwrap()).unwrap(),
                        std::fs::canonicalize(&folder).unwrap()
                    );
                    assert_eq!(
                        std::fs::canonicalize(v["session_cwd"].as_str().unwrap()).unwrap(),
                        std::fs::canonicalize(&folder).unwrap()
                    );
                }
                break;
            }
            assert!(Instant::now() < until, "adapter cwd answer missing");
            std::thread::sleep(Duration::from_millis(20));
        }
        c.call(Cmd::AgentStop(v1::AgentStop {
            agent_id: a.agent_id.clone(),
        }))
        .unwrap()
        .unwrap();
        if expected == 1 {
            node.supervisor
                .lock()
                .unwrap()
                .resume(&agora_core::AgentId::new(&a.agent_id), Instant::now())
                .unwrap();
        }
    }
}
