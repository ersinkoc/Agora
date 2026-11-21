//! Issue #877: the node's own folders (its data: event buffer, session ledger, `hub-link`; its agents' mesh sockets) are closed to
//! every other user at start, also when an older version left them open. On Unix a folder's mode closes everything below it, which
//! is what is checked here; the Windows ACL is `agora_platform::restrict_to_owner`'s own and is checked on a real Windows machine.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use agora_node::daemon::{Node, NodeConfig};
use agora_node::supervisor::NodeLimits;

fn mode(p: &Path) -> u32 {
    std::fs::metadata(p).unwrap().permissions().mode() & 0o777
}

fn start(dir: &Path) -> Node {
    let manifests = dir.join("manifests");
    std::fs::create_dir_all(&manifests).unwrap();
    Node::start(&NodeConfig {
        resume_all: false,
        node_id: "node-test".into(),
        // Short path: Unix socket paths are limited to about 100 bytes.
        socket: dir.join("c.sock"),
        data_dir: dir.join("data"),
        worktree_root: dir.join("worktrees"),
        manifests_dir: manifests,
        projects: vec![],
        limits: NodeLimits::default(),
    })
    .expect("start node")
}

fn fresh(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("agora-877-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::os::unix::fs::DirBuilderExt::mode(&mut std::fs::DirBuilder::new(), 0o700)
        .create(&dir)
        .unwrap();
    dir
}

#[test]
fn a_new_node_makes_its_data_and_mesh_folders_its_users_only() {
    let dir = fresh("new");
    let node = start(&dir);
    assert_eq!(mode(&dir.join("data")), 0o700);
    assert_eq!(mode(&dir.join("m")), 0o700);
    drop(node);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn folders_an_older_version_left_open_are_closed_at_the_next_start() {
    let dir = fresh("old");
    for sub in ["data", "data/events", "m"] {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
        std::fs::set_permissions(dir.join(sub), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::fs::write(dir.join("data/hub-link"), "up\n").unwrap();
    std::fs::set_permissions(
        dir.join("data/hub-link"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    let node = start(&dir);
    // The folder's mode is what keeps others out of everything below it (the files keep their own modes).
    assert_eq!(mode(&dir.join("data")), 0o700);
    assert_eq!(mode(&dir.join("m")), 0o700);
    drop(node);
    let _ = std::fs::remove_dir_all(&dir);
}
