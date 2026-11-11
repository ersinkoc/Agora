//! A confined agent (ADR-0050 Amendment B on Windows) commits in a checkout that lives under the user profile: git `stat`s every folder
//! up to the root, which the attributes-only entries on the folders above the grants make possible (decision 3). Like `confined_git_commit.rs`,
//! with the repository under the user temp folder. Only where the platform grants permissions folder by folder.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use agora_adapters::Manifest;
use agora_core::NodeId;
use agora_node::sandbox::{Level, SandboxPolicy};
use agora_node::supervisor::{
    NodeLimits, RestartPolicy, StartRequest, Supervisor, SupervisorConfig,
};
use agora_node::terminal::TerminalOptions;

/// Every file and folder below `root` (links not followed).
fn walk(root: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(p) = stack.pop() {
        let Ok(meta) = std::fs::symlink_metadata(&p) else {
            continue;
        };
        if meta.is_dir() && !meta.file_type().is_symlink() {
            if let Ok(entries) = std::fs::read_dir(&p) {
                stack.extend(entries.flatten().map(|e| e.path()));
            }
        }
        out.push(p);
    }
    out
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn a_confined_agent_commits_in_a_checkout_under_the_user_profile() {
    if !agora_platform::NARROW_GIT_WRITES {
        return;
    }
    // The confinement is off by default until the maintainer switches it on; this test switches it on for its own process.
    std::env::set_var("AGORA_WINDOWS_CONFINEMENT", "1");
    if !agora_platform::sandbox_supported() {
        eprintln!("skipped: agents cannot be confined on this machine");
        return;
    }
    // Not under the user's temp folder: git `stat`s every folder above a ref it locks, and folders of the user profile refuse that to a
    // confined agent, so a checkout inside the profile cannot commit yet (reported separately; this test covers the reflog only).
    let dir = std::env::temp_dir().join(format!("agora-confined-commit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);

    // The identity is given on the command line: the user's global git config is outside what the agent may read.
    let script = "echo agent> b.txt && git add b.txt 2> add-err.txt && \
                  git -c user.name=agent -c user.email=agent@example.com commit -q -m from-agent 2> commit-err.txt && \
                  echo ok> committed.txt";
    let manifest = Manifest::parse(&format!(
        "kind = \"committer\"\ndisplay_name = \"Committer\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"cmd.exe\", \"/c\", {script:?}]\n"
    ))
    .unwrap();
    let mut sup = Supervisor::new(SupervisorConfig {
        node: NodeId::new("node-1"),
        restart: RestartPolicy::default(),
        terminal: TerminalOptions::default(),
        limits: NodeLimits::default(),
    });
    sup.register(manifest);
    sup.set_sandbox(SandboxPolicy {
        level: Level::Worktree,
        ..Default::default()
    });
    let req = StartRequest {
        kind: "committer".into(),
        checkout: repo.clone(),
        worktree_root: dir.join("worktrees"),
        name: "a".into(),
        base: None,
    };
    sup.start(&req, Instant::now()).unwrap();

    let worktree = dir.join("worktrees").join("a");
    let done = worktree.join("committed.txt");
    let end = Instant::now() + Duration::from_secs(30);
    while !done.exists() && Instant::now() < end {
        sup.tick(Instant::now());
        std::thread::sleep(Duration::from_millis(100));
    }
    let errors = ["add-err.txt", "commit-err.txt"]
        .iter()
        .map(|f| std::fs::read_to_string(worktree.join(f)).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        done.exists(),
        "the confined agent could not commit: {errors}"
    );
    assert_eq!(
        git(&worktree, &["log", "-1", "--format=%s"]).trim(),
        "from-agent"
    );
    // The main checkout's own state is untouched: its branch still points at the first commit.
    assert_eq!(
        git(&repo, &["log", "-1", "--format=%s", "main"]).trim(),
        "init"
    );
    drop(sup);
    // ADR-0050 Amendment B: once the agent has ended, no Low label is left anywhere below (labels taken back, and the explicit ones
    // Windows gives what a Low process creates cleared too).
    let labelled: Vec<String> = walk(&dir)
        .into_iter()
        .filter(|p| {
            let out = Command::new("icacls").arg(p).output().unwrap();
            String::from_utf8_lossy(&out.stdout).contains("Mandatory Label")
        })
        .map(|p| p.display().to_string())
        .collect();
    assert!(labelled.is_empty(), "left labelled: {labelled:?}");
    let _ = std::fs::remove_dir_all(dir);
}
