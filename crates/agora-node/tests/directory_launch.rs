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
    static BUILD: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    BUILD.get_or_init(|| {
        // Even when present: a binary left from an older checkout may lack newer flags.
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
            "building fake fixture failed"
        );
    });
    assert!(p.exists(), "fake fixture is absent after build");
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
            stop_helpers: false,
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
    let created_root = Path::new(&a.path).parent().unwrap();
    assert!(!git(created_root, &["status", "--porcelain"]).contains(".mcp.json"));
    git(created_root, &["add", "."]);
    assert!(!git(created_root, &["diff", "--cached", "--name-only"]).contains(".mcp.json"));
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
            stop_helpers: false,
        }))
        .unwrap()
        .unwrap();
        if expected == 1 {
            let mut sup = node.supervisor.lock().unwrap();
            let events = sup
                .resume(&agora_core::AgentId::new(&a.agent_id), Instant::now())
                .unwrap();
            node.events.supervisor_events(&events, &sup.list());
            drop(sup);
            let log: Vec<_> = node
                .events
                .payloads_after(0)
                .unwrap()
                .iter()
                .map(|b| v1::Event::decode(b.as_slice()).unwrap())
                .collect();
            let exited=log.iter().rposition(|e| matches!(&e.body,Some(v1::event::Body::AgentExited(x)) if x.agent_id==a.agent_id)).unwrap();
            let started=log.iter().rposition(|e| matches!(&e.body,Some(v1::event::Body::AgentStarted(x)) if x.agent_id==a.agent_id)).unwrap();
            assert!(
                started > exited,
                "resume must restore running in hub/client projections"
            );
            assert!(log[started+1..].iter().any(|e| matches!(&e.body,Some(v1::event::Body::AgentStateChanged(x)) if x.agent_id==a.agent_id && x.state==v1::AgentState::Unspecified as i32)));
            assert!(log[started+1..].iter().any(|e| matches!(&e.body,Some(v1::event::Body::AgentSessionReported(x)) if x.agent_id==a.agent_id)));
            assert!(!log[started+1..].iter().any(|e| matches!(&e.body,Some(v1::event::Body::TerminalOpened(x)) if x.agent_id==a.agent_id)));
        }
    }
}

#[test]
fn isolated_launch_recreates_an_untracked_selected_subdirectory() {
    if !supported() {
        return;
    }
    let e = Env::new("untracked-subdir");
    let repo = e.root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "--allow-empty", "-qm", "initial"]);
    let selected = repo.join("untracked/nested folder");
    std::fs::create_dir_all(&selected).unwrap();
    std::fs::write(selected.join("keep"), "original").unwrap();
    let mut c = e.client();
    let info = inspect(&mut c, &selected, false);
    let a = launch(&mut c, &info, "probe", true);
    assert!(Path::new(&a.path).ends_with("untracked/nested folder"));
    assert!(!Path::new(&a.path).join("keep").exists());
    assert_eq!(
        std::fs::read_to_string(selected.join("keep")).unwrap(),
        "original"
    );
    e.reports(1);
}

#[test]
fn isolated_launch_rejects_a_head_symlink_and_never_creates_directories_through_it() {
    if !supported() {
        return;
    }
    let e = Env::new("head-link");
    let repo = e.root.join("repo");
    let outside = e.root.join("outside");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    let blob_file = e.root.join("link-target");
    std::fs::write(&blob_file, outside.to_string_lossy().as_bytes()).unwrap();
    let blob = git(&repo, &["hash-object", "-w", blob_file.to_str().unwrap()]);
    git(
        &repo,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("120000,{},nested", blob.trim()),
        ],
    );
    git(&repo, &["commit", "-qm", "tracked symlink"]);
    let selected = repo.join("nested");
    std::fs::create_dir_all(&selected).unwrap();
    let before = git(&repo, &["worktree", "list", "--porcelain"]);
    let mut c = e.client();
    let info = inspect(&mut c, &selected, false);
    let result = c
        .call(Cmd::AgentLaunch(v1::AgentLaunch {
            path: info.path,
            project_id: info.project_id,
            kind: "probe".into(),
            separate_worktree: true,
            ..Default::default()
        }))
        .unwrap();
    assert!(
        result.is_err(),
        "a restored symlink must not select another folder"
    );
    assert!(
        !outside.join("missing").exists(),
        "directory creation followed a symlink outside the worktree"
    );
    assert_eq!(git(&repo, &["worktree", "list", "--porcelain"]), before);
    assert!(selected.is_dir());
}

#[test]
fn directory_diff_cannot_show_tracked_or_untracked_changes_in_the_parent_checkout() {
    if !supported() {
        return;
    }
    let e = Env::new("diff-root");
    let repo = e.root.join("repo");
    let selected = repo.join("sub");
    std::fs::create_dir_all(&selected).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("outside.txt"), "old").unwrap();
    std::fs::write(selected.join("inside.txt"), "old").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "initial"]);
    std::fs::write(repo.join("outside.txt"), "parent-private-change").unwrap();
    std::fs::write(repo.join("parent-new.txt"), "parent-new-change").unwrap();
    std::fs::write(selected.join("inside.txt"), "selected-change").unwrap();
    std::fs::write(selected.join("new.txt"), "selected-new").unwrap();
    let mut c = e.client();
    let info = inspect(&mut c, &selected, false);
    let a = launch(&mut c, &info, "probe", false);
    let ok = c
        .call(Cmd::WorkspaceQuery(v1::WorkspaceQuery {
            workspace_id: a.workspace_id,
            operation: "diff".into(),
            ..Default::default()
        }))
        .unwrap()
        .unwrap();
    let Some(Ok_::GitDiff(diff)) = ok.result else {
        panic!("missing diff");
    };
    assert!(diff.diff.contains("selected-change") && diff.diff.contains("selected-new"));
    assert!(
        !diff.diff.contains("parent-private-change") && !diff.diff.contains("parent-new-change"),
        "{}",
        diff.diff
    );
}

#[test]
fn confined_selected_subfolders_cannot_write_to_the_parent_checkout() {
    if !supported() || !agora_platform::sandbox_supported() {
        return;
    }
    let e = Env::new("subfolder-confinement");
    // Outside the shared system temp folder: that folder is intentionally writable in the default profile.
    let repo = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("selected-boundary-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&repo);
    std::fs::create_dir_all(repo.join("sub")).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("sub/keep"), "original").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "initial"]);
    let forbidden = repo.join("parent-write");
    let script="printf inside > inside; if printf outside > \"$1\"; then printf escaped > result; else printf denied > result; fi; sleep 30";
    let argv = serde_json::to_string(&vec![
        "/bin/sh",
        "-c",
        script,
        "probe",
        forbidden.to_str().unwrap(),
    ])
    .unwrap();
    {
        let mut sup = e.node.as_ref().unwrap().supervisor.lock().unwrap();
        sup.set_sandbox(agora_node::sandbox::SandboxPolicy {
            level: agora_node::sandbox::Level::Worktree,
            ..Default::default()
        });
        sup.register(agora_adapters::Manifest::parse(&format!("kind=\"boundary\"\ndisplay_name=\"Boundary\"\nversion=\"1\"\nmin_engine=1\n[process]\nlaunch={argv}\n")).unwrap());
    }
    let mut c = e.client();
    let info = inspect(&mut c, &repo.join("sub"), false);
    for separate in [false, true] {
        let a = launch(&mut c, &info, "boundary", separate);
        let cwd = Path::new(&a.path);
        let until = Instant::now() + Duration::from_secs(10);
        while !cwd.join("result").exists() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            std::fs::read_to_string(cwd.join("result")).unwrap(),
            "denied"
        );
        assert_eq!(
            std::fs::read_to_string(cwd.join("inside")).unwrap(),
            "inside"
        );
        assert!(!forbidden.exists());
        c.call(Cmd::AgentStop(v1::AgentStop {
            agent_id: a.agent_id,
            stop_helpers: false,
        }))
        .unwrap()
        .unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(repo.join("sub/keep")).unwrap(),
        "original"
    );
    drop(c);
    drop(e);
    let _ = std::fs::remove_dir_all(repo);
}

#[test]
fn a_launched_folders_changes_are_reported_for_its_workspace() {
    use v1::event::Body;
    if !supported() {
        return;
    }
    // The start answers without counting the folder's changes; the node's Git watcher, woken by the start, reports them.
    let e = Env::new("changes");
    let repo = e.root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "committed\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "initial"]);
    std::fs::write(repo.join("a.txt"), "edited\n").unwrap();
    std::fs::write(repo.join("b.txt"), "new\n").unwrap();
    let mut c = e.client();
    let info = inspect(&mut c, &repo, false);
    let a = launch(&mut c, &info, "probe", false);
    // Generous: one status is a few git processes, which take seconds each on a busy Windows machine.
    let end = Instant::now() + Duration::from_secs(60);
    loop {
        let latest = e
            .node
            .as_ref()
            .unwrap()
            .events
            .events_after(0)
            .into_iter()
            .filter_map(|ev| match ev.body {
                Some(Body::WorkspaceStatusChanged(s)) if s.workspace_id == a.workspace_id => {
                    Some(s)
                }
                _ => None,
            })
            .next_back();
        if latest.as_ref().is_some_and(|s| s.files_changed == 2) {
            break;
        }
        assert!(
            Instant::now() < end,
            "no report of the two changed files: {latest:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    c.call(Cmd::AgentStop(v1::AgentStop {
        agent_id: a.agent_id,
        stop_helpers: false,
    }))
    .unwrap()
    .unwrap();
}

/// Waits until the node's event log holds an event that `pred` accepts.
fn wait_for_event(node: &Node, what: &str, pred: impl Fn(&v1::event::Body) -> bool) {
    let until = Instant::now() + Duration::from_secs(10);
    while !node
        .events
        .events_after(0)
        .iter()
        .any(|e| e.body.as_ref().is_some_and(&pred))
    {
        assert!(Instant::now() < until, "no {what} in the event log");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// ADR-0054 section 3: when the hub's record of this node cannot be continued and this process's start is lost to the hub, the
/// node says at the tail of its buffer who it is, its folders, every agent that runs now (with state, session and terminal) and
/// every approval still waiting, and nothing about agents that ended. ADR-0055: otherwise it says the same without `NodeOnline`,
/// and its ended agents as ended.
#[test]
fn a_resync_announces_the_nodes_present() {
    use v1::event::Body;
    if !supported() {
        return;
    }
    let e = Env::new("resync");
    let (shared, chat) = (e.root.join("shared"), e.root.join("chat"));
    std::fs::create_dir_all(&shared).unwrap();
    std::fs::create_dir_all(&chat).unwrap();
    let mut c = e.client();
    let info = inspect(&mut c, &shared, false);
    let kept = launch(&mut c, &info, "probe", false);
    let ended = launch(&mut c, &info, "probe", false);
    let chat_info = inspect(&mut c, &chat, false);
    let talker = launch(&mut c, &chat_info, "adapter", false);
    let node = e.node.as_ref().unwrap();
    e.reports(2);
    wait_for_event(
        node,
        "session",
        |b| matches!(b, Body::AgentSessionReported(s) if s.agent_id == talker.agent_id),
    );
    c.call(Cmd::AgentPrompt(v1::AgentPrompt {
        agent_id: talker.agent_id.clone(),
        text: "ask".into(),
    }))
    .unwrap()
    .unwrap();
    wait_for_event(
        node,
        "approval",
        |b| matches!(b, Body::ApprovalRequested(a) if a.agent_id == talker.agent_id),
    );
    c.call(Cmd::AgentStop(v1::AgentStop {
        agent_id: ended.agent_id.clone(),
        stop_helpers: false,
    }))
    .unwrap()
    .unwrap();

    let mark = node.events.next_seq() - 1;
    node.events.announce();
    let said: Vec<Body> = node
        .events
        .events_after(mark)
        .into_iter()
        .filter_map(|e| e.body)
        .collect();
    let at = |pred: &dyn Fn(&Body) -> bool| said.iter().position(pred);
    let started = |id: &str| at(&|b| matches!(b, Body::AgentStarted(s) if s.agent_id == id));
    assert!(matches!(said.first(), Some(Body::NodeOnline(n)) if n.node_id == "node-directory"));
    // Only what runs now, each after the folder it works in and bound to it.
    assert_eq!(
        started(&ended.agent_id),
        None,
        "an ended agent is not announced"
    );
    for (agent, workspace) in [(&kept, &kept.workspace_id), (&talker, &talker.workspace_id)] {
        let s = started(&agent.agent_id).expect("a running agent is announced");
        let registered =
            at(&|b| matches!(b, Body::WorkspaceRegistered(w) if &w.workspace_id == workspace))
                .unwrap();
        let bound = at(&|b| matches!(b, Body::AgentWorkspaceBound(w) if w.agent_id == agent.agent_id && &w.workspace_id == workspace)).unwrap();
        assert!(registered < bound && bound < s);
        assert!(said[s..]
            .iter()
            .any(|b| matches!(b, Body::AgentStateChanged(x) if x.agent_id == agent.agent_id)));
    }
    let talker_started = started(&talker.agent_id).unwrap();
    assert!(said[talker_started..].iter().any(|b| matches!(b, Body::AgentSessionReported(x) if x.agent_id == talker.agent_id && !x.session_id.is_empty())));
    // Only the terminal agent that runs has a terminal; the chat agent has none.
    let terminals: Vec<&str> = said
        .iter()
        .filter_map(|b| match b {
            Body::TerminalOpened(t) => Some(t.agent_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(terminals, [kept.agent_id.as_str()]);
    // The approval still waiting, after the agent it belongs to.
    let approval =
        at(&|b| matches!(b, Body::ApprovalRequested(a) if a.agent_id == talker.agent_id)).unwrap();
    assert!(approval > talker_started);
    assert!(!said.iter().any(|b| matches!(b, Body::AgentExited(_))));

    // ADR-0055: a resync after a start the hub already has (the start is older than what it lost) states facts instead: no
    // `NodeOnline`, the running agents as above, and the ended one as ended, with its terminal closed.
    let start = node.events.last_online();
    let mark = node.events.next_seq() - 1;
    node.events.resync(start + 1, node.events.next_seq());
    let said: Vec<Body> = node
        .events
        .events_after(mark)
        .into_iter()
        .filter_map(|e| e.body)
        .collect();
    let at = |pred: &dyn Fn(&Body) -> bool| said.iter().position(pred);
    assert!(!said.iter().any(|b| matches!(b, Body::NodeOnline(_))));
    assert_eq!(node.events.last_online(), start);
    for agent in [&kept, &talker] {
        assert!(
            at(&|b| matches!(b, Body::AgentStarted(s) if s.agent_id == agent.agent_id)).is_some()
        );
    }
    let exits: Vec<(&str, i32)> = said
        .iter()
        .filter_map(|b| match b {
            Body::AgentExited(x) => Some((x.agent_id.as_str(), x.exit_code)),
            _ => None,
        })
        .collect();
    assert_eq!(
        exits,
        [(ended.agent_id.as_str(), i32::MIN)],
        "stopped on request"
    );
    let terminal = format!("t-{}", ended.agent_id);
    assert!(at(&|b| matches!(b, Body::TerminalClosed(t) if t.terminal_id == terminal)).is_some());
    assert!(
        at(&|b| matches!(b, Body::ApprovalRequested(a) if a.agent_id == talker.agent_id)).is_some()
    );
}
