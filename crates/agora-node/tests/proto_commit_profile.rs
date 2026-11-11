//! An agent confined folder by folder (ADR-0050, the Windows profile) can commit in its own worktree: `git add` and `git commit` need
//! the worktree, its admin folder, `objects`, `refs` and the branch reflog under `logs/refs`, and nothing else of the main `.git`.
//! Only where the platform grants permissions folder by folder; elsewhere the test returns at once.
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
fn proto_commit_under_the_profile() {
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
    let script = "echo agent> b.txt && git add b.txt > nul 2> add-err.txt && \
                  git -c user.name=agent -c user.email=agent@example.com commit -q -m from-agent > nul 2> commit-err.txt && \
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
    let _ = std::fs::remove_dir_all(dir);
}
