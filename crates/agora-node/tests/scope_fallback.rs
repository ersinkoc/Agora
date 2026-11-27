//! A user manager that stops answering after the node checked it: an agent whose start through `systemd-run` fails at once is started
//! again as before (outside a scope), never left failing. A test binary of its own, since it puts a stand-in `systemd-run` first on PATH.
#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use agora_adapters::Manifest;
use agora_core::NodeId;
use agora_node::supervisor::{
    NodeLimits, RestartPolicy, StartRequest, Supervisor, SupervisorConfig, SupervisorEvent,
};
use agora_node::terminal::TerminalOptions;

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
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

#[test]
fn a_start_systemd_run_refuses_runs_again_outside_a_scope() {
    let dir = std::env::temp_dir().join(format!("agora-scope-fallback-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    // The stand-in answers the node's first check, then fails as systemd-run does when the user manager is gone (before it runs the
    // program); every call is counted.
    let count = dir.join("calls");
    let fake = bin.join("systemd-run");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\nn=$(cat '{c}' 2>/dev/null || echo 0)\necho $((n+1)) > '{c}'\n[ \"$n\" = 0 ] && exit 0\nexit 1\n",
            c = count.display()
        ),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs = vec![bin.clone()];
    dirs.extend(std::env::split_paths(&path));
    // The only test in this binary: nothing else reads the environment while it changes.
    std::env::set_var("PATH", std::env::join_paths(dirs).unwrap());

    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);

    let mut sup = Supervisor::new(SupervisorConfig {
        node: NodeId::new("node-1"),
        restart: RestartPolicy {
            max_restarts: 3,
            base_delay: Duration::from_millis(30),
            max_delay: Duration::from_millis(60),
        },
        terminal: TerminalOptions::default(),
        limits: NodeLimits::default(),
    });
    let end = Instant::now() + Duration::from_secs(10);
    while !agora_platform::own_scopes() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        agora_platform::own_scopes(),
        "the stand-in's first answer is yes"
    );
    sup.register(
        Manifest::parse(
            "kind = \"fake\"\ndisplay_name = \"Fake\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"sh\", \"-c\", \"sleep 30\"]\n",
        )
        .unwrap(),
    );
    let id = sup
        .start(
            &StartRequest {
                kind: "fake".into(),
                checkout: repo.clone(),
                worktree_root: dir.join("worktrees"),
                name: "a".into(),
                base: None,
            },
            Instant::now(),
        )
        .unwrap();

    // The scoped start fails at once; the check runs again and says no; the restart runs the agent itself.
    let end = Instant::now() + Duration::from_secs(20);
    let mut events = Vec::new();
    let mut running_since = None;
    while Instant::now() < end {
        events.extend(sup.tick(Instant::now()));
        let running = sup.list().first().is_some_and(|a| a.running) && sup.list()[0].restarts >= 1;
        match (running, running_since) {
            (true, None) => running_since = Some(Instant::now()),
            (false, _) => running_since = None,
            _ => {}
        }
        if running_since.is_some_and(|t: Instant| t.elapsed() > Duration::from_secs(1)) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        running_since.is_some_and(|t| t.elapsed() > Duration::from_secs(1)),
        "the agent runs after its restart: {events:#?}"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, SupervisorEvent::GaveUp { .. })),
        "{events:#?}"
    );
    assert_eq!(sup.list()[0].restarts, 1, "one failed start, then it runs");
    assert!(!agora_platform::own_scopes(), "the second check said no");
    let calls: u32 = std::fs::read_to_string(&count)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_eq!(
        calls, 3,
        "the first check, the failed start, the second check"
    );
    sup.stop(&id).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}
