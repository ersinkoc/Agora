//! #897: where the platform cannot confine a plain process (Windows confines PTY processes only), an adapter or ACP agent under a
//! `sandbox` level is refused before anything is created, with `agent.sandbox_unavailable`, instead of failing at spawn after its
//! worktree was made. Its own test binary: it switches Windows confinement on for this process.
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "private_home/mod.rs"]
mod private_home;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

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

#[test]
fn an_adapter_agent_that_cannot_be_confined_here_starts_nothing_and_leaves_no_worktree() {
    // Windows confinement is off by default (ADR-0050); this binary switches it on for itself.
    std::env::set_var("AGORA_WINDOWS_CONFINEMENT", "1");
    if agora_platform::COMMAND_CONFINEMENT_SUPPORTED {
        eprintln!(
            "skipped: this platform confines plain processes, so adapter agents may start confined"
        );
        return;
    }
    if !agora_platform::sandbox_supported() {
        eprintln!(
            "skipped: this machine confines nothing, which every kind is refused for already"
        );
        return;
    }
    private_home::init();
    let dir = std::env::temp_dir().join(format!(
        "agora-structured-unconfinable-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);

    let mut sup = Supervisor::new(SupervisorConfig {
        node: NodeId::new("node-1"),
        restart: RestartPolicy::default(),
        terminal: TerminalOptions::default(),
        limits: NodeLimits::default(),
    });
    sup.register(
        Manifest::parse(
            "kind = \"structured\"\ndisplay_name = \"S\"\nversion = \"1\"\nmin_engine = 1\n[adapter]\ncommand = [\"agora-fake-agent\", \"--adapter\"]\n",
        )
        .unwrap(),
    );
    let policy = SandboxPolicy {
        level: Level::Worktree,
        ..Default::default()
    };
    // A terminal agent could be confined here; a plain process cannot.
    assert!(policy.enforceable() && !policy.enforceable_for_commands());
    sup.set_sandbox(policy);
    let req = StartRequest {
        kind: "structured".into(),
        checkout: repo.clone(),
        worktree_root: dir.join("worktrees"),
        name: "a".into(),
        base: None,
    };

    let err = sup.start(&req, Instant::now()).unwrap_err();
    assert_eq!(
        err,
        SupervisorError::SandboxUnavailable("structured".into(), "worktree")
    );
    assert_eq!(err.to_protocol().code, ErrorCode::AgentSandboxUnavailable);
    assert!(!dir.join("worktrees").exists(), "no worktree is made");

    // The same in the user's own folder.
    let project = agora_node::git::discover_checkout(&repo).unwrap().project;
    let err = sup
        .start_in_directory(&req, project, Instant::now())
        .unwrap_err();
    assert_eq!(
        err,
        SupervisorError::SandboxUnavailable("structured".into(), "worktree")
    );
    assert!(sup.list().is_empty());
    let _ = std::fs::remove_dir_all(dir);
}
