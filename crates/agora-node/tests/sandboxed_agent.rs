//! An agent started under the node's `sandbox` policy (ADR-0047) is confined, and one that cannot be confined is not started.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use agora_adapters::Manifest;
use agora_core::NodeId;
use agora_node::sandbox::{Level, SandboxPolicy};
use agora_node::supervisor::{
    NodeLimits, RestartPolicy, StartRequest, Supervisor, SupervisorConfig, SupervisorError,
};
use agora_node::terminal::TerminalOptions;
use agora_protocol::ErrorCode;

fn git(dir: &Path, args: &[&str]) {
    assert!(Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .status()
        .unwrap()
        .success());
}

/// A repository, and a supervisor with one kind that writes a file in its worktree and one outside it.
fn setup(name: &str, outside: &Path, policy: SandboxPolicy) -> (Supervisor, StartRequest, PathBuf) {
    let dir = std::env::temp_dir().join(format!("agora-sbx-agent-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let script = format!(
        "echo in > inside; echo out > {} 2>/dev/null; sleep 30",
        outside.display()
    );
    let manifest = Manifest::parse(&format!(
        "kind = \"shy\"\ndisplay_name = \"Shy\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"/bin/sh\", \"-c\", {script:?}]\n"
    ))
    .unwrap();
    let mut sup = Supervisor::new(SupervisorConfig {
        node: NodeId::new("node-1"),
        restart: RestartPolicy::default(),
        terminal: TerminalOptions::default(),
        limits: NodeLimits::default(),
    });
    sup.register(manifest);
    sup.set_sandbox(policy);
    let req = StartRequest {
        kind: "shy".into(),
        checkout: repo,
        worktree_root: dir.join("worktrees"),
        name: "a".into(),
        base: None,
    };
    (sup, req, dir)
}

#[test]
fn a_level_that_cannot_be_enforced_starts_nothing_and_leaves_no_worktree() {
    let outside = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("sbx-never");
    let policy = SandboxPolicy {
        level: Level::WorktreeOffline,
        ..Default::default()
    };
    let (mut sup, req, dir) = setup("offline", &outside, policy);
    let err = sup.start(&req, Instant::now()).unwrap_err();
    assert_eq!(
        err,
        SupervisorError::SandboxUnavailable("shy".into(), "worktree-offline")
    );
    assert_eq!(err.to_protocol().code, ErrorCode::AgentSandboxUnavailable);
    assert!(!dir.join("worktrees").exists() && !outside.exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_confined_agent_writes_in_its_worktree_and_not_outside_it() {
    if !agora_platform::sandbox_supported() {
        eprintln!("skipped: no Landlock on this machine");
        return;
    }
    let outside =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("sbx-out-{}", std::process::id()));
    let _ = std::fs::remove_file(&outside);
    let policy = SandboxPolicy {
        level: Level::Worktree,
        ..Default::default()
    };
    let (mut sup, req, dir) = setup("worktree", &outside, policy);
    sup.start(&req, Instant::now()).unwrap();
    let inside = dir.join("worktrees/a/inside");
    let end = Instant::now() + Duration::from_secs(10);
    while !inside.exists() && Instant::now() < end {
        sup.tick(Instant::now());
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(inside.exists(), "the agent could not write in its worktree");
    std::thread::sleep(Duration::from_millis(300));
    assert!(!outside.exists(), "the agent wrote outside its worktree");
    drop(sup);
    let _ = std::fs::remove_dir_all(dir);
}
