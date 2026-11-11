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
    let script = format!(
        "echo in > inside; echo out > {} 2>/dev/null; sleep 30",
        outside.display()
    );
    setup_with(name, &script, policy)
}

/// A repository, and a supervisor with one kind that runs `script` in `sh` in its worktree.
fn setup_with(
    name: &str,
    script: &str,
    policy: SandboxPolicy,
) -> (Supervisor, StartRequest, PathBuf) {
    let dir = std::env::temp_dir().join(format!("agora-sbx-agent-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
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
    // `worktree-offline` is the level some machines cannot enforce (no network rules); where it can be, see the next test.
    if agora_platform::sandbox_supported() && agora_platform::offline_supported() {
        eprintln!("skipped: this machine enforces worktree-offline");
        return;
    }
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

#[test]
fn a_confined_agent_has_a_writable_temp_folder_of_its_own_where_the_system_one_is_private() {
    if !agora_platform::sandbox_supported() || !agora_platform::PRIVATE_TEMP {
        eprintln!("skipped: no confinement, or the shared temp folder is the agent's");
        return;
    }
    let policy = SandboxPolicy {
        level: Level::Worktree,
        ..Default::default()
    };
    // Tools that honour `TMPDIR` (Python, Node, Rust, `mktemp "$TMPDIR/..."`) get a folder they can write and read back. (On macOS,
    // plain `mktemp`, `xcrun` and `cc` use the system's per-user temp folder whatever `TMPDIR` says; see the sandbox recipes.)
    let script = r#"echo "$TMPDIR" > tmpdir; { f=$(mktemp "$TMPDIR/t.XXXXXX") && echo ok > "$f" && cp "$f" inside; } 2> err; sleep 30"#;
    let (mut sup, req, dir) = setup_with("temp", script, policy);
    sup.start(&req, Instant::now()).unwrap();
    let inside = dir.join("worktrees/a/inside");
    let end = Instant::now() + Duration::from_secs(10);
    while !inside.exists() && Instant::now() < end {
        sup.tick(Instant::now());
        std::thread::sleep(Duration::from_millis(50));
    }
    let tmpdir = std::fs::read_to_string(dir.join("worktrees/a/tmpdir")).unwrap_or_default();
    let err = std::fs::read_to_string(dir.join("worktrees/a/err")).unwrap_or_default();
    assert!(
        inside.exists(),
        "the agent could not use its temp folder {tmpdir}: {err}"
    );
    assert!(tmpdir.trim().contains("agora-agent-"), "{tmpdir}");
    drop(sup);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_offline_agent_works_in_its_worktree_but_reaches_no_other_machine() {
    if !agora_platform::sandbox_supported() || !agora_platform::offline_supported() {
        eprintln!("skipped: this machine cannot keep an agent off the network");
        return;
    }
    let local = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = local.local_addr().unwrap().port();
    let policy = SandboxPolicy {
        level: Level::WorktreeOffline,
        ..Default::default()
    };
    // `nc` ships with macOS (the only system that enforces this level today).
    let script = format!(
        "echo in > inside; {{ nc -z -G 2 1.1.1.1 443 && echo internet-open || echo internet-closed; \
         nc -z -G 2 127.0.0.1 {port} && echo local-open || echo local-closed; }} > net 2>/dev/null; sleep 30"
    );
    let (mut sup, req, dir) = setup_with("offline-on", &script, policy);
    sup.start(&req, Instant::now()).unwrap();
    let net = dir.join("worktrees/a/net");
    let end = Instant::now() + Duration::from_secs(15);
    while std::fs::read_to_string(&net).map_or(0, |t| t.lines().count()) < 2 && Instant::now() < end
    {
        sup.tick(Instant::now());
        std::thread::sleep(Duration::from_millis(50));
    }
    let text = std::fs::read_to_string(&net).unwrap_or_default();
    assert!(
        dir.join("worktrees/a/inside").exists(),
        "the agent could not write in its worktree"
    );
    assert!(text.contains("internet-closed"), "{text}");
    assert!(text.contains("local-open"), "{text}");
    drop((sup, local));
    let _ = std::fs::remove_dir_all(dir);
}
