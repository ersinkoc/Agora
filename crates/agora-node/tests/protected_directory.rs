//! With confinement on, a launch in a folder that would make it meaningless is refused before anything starts (ADR-0052, ADR-0047).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Instant;

use agora_core::{NodeId, ProjectId};
use agora_node::sandbox::{Level, SandboxPolicy};
use agora_node::supervisor::{
    NodeLimits, RestartPolicy, StartRequest, Supervisor, SupervisorConfig, SupervisorError,
};
use agora_node::terminal::TerminalOptions;

fn supervisor(level: Level) -> Supervisor {
    let mut sup = Supervisor::new(SupervisorConfig {
        node: NodeId::new("node-1"),
        restart: RestartPolicy::default(),
        terminal: TerminalOptions::default(),
        limits: NodeLimits::default(),
    });
    sup.set_sandbox(SandboxPolicy {
        level,
        ..Default::default()
    });
    sup
}

fn request(dir: &std::path::Path, worktrees: &std::path::Path) -> StartRequest {
    StartRequest {
        kind: "shell".into(),
        checkout: dir.to_path_buf(),
        worktree_root: worktrees.to_path_buf(),
        name: "a".into(),
        base: None,
    }
}

#[test]
fn a_confined_launch_in_the_filesystem_root_or_the_nodes_own_folder_is_refused() {
    let node_home = std::env::temp_dir().join(format!("agora-protected-{}", std::process::id()));
    let worktrees = node_home.join("worktrees");
    std::fs::create_dir_all(&worktrees).unwrap();
    let mut sup = supervisor(Level::Worktree);
    for dir in [std::path::Path::new("/"), node_home.as_path()] {
        let err = sup
            .start_selected(
                &request(dir, &worktrees),
                ProjectId::directory("node-1", "x"),
                std::path::Path::new(""),
                false,
                Instant::now(),
            )
            .unwrap_err();
        assert_eq!(err, SupervisorError::ProtectedDirectory, "{dir:?}");
    }
    // The same folders are the owner's choice when nothing is confined: the refusal is not what stops this one.
    let mut open = supervisor(Level::Off);
    let err = open
        .start_selected(
            &request(&node_home, &worktrees),
            ProjectId::directory("node-1", "x"),
            std::path::Path::new(""),
            false,
            Instant::now(),
        )
        .unwrap_err();
    assert_ne!(err, SupervisorError::ProtectedDirectory);
    let _ = std::fs::remove_dir_all(&node_home);
}
