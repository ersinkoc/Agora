//! An adapter installed from a registry publisher that a newer registry index revokes is not started (ADR-0044); the refusal comes
//! before any worktree is made. This binary has this one test because it points `AGORA_HOME` at a scratch folder.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Instant;

use agora_adapters::Manifest;
use agora_core::NodeId;
use agora_node::supervisor::{
    NodeLimits, RestartPolicy, StartRequest, Supervisor, SupervisorConfig, SupervisorError,
};
use agora_node::terminal::TerminalOptions;
use agora_protocol::ErrorCode;

#[test]
fn an_adapter_from_a_revoked_publisher_is_refused_and_leaves_no_worktree() {
    let dir = std::env::temp_dir().join(format!("agora-revoked-node-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let home = dir.join("home");
    let pkg = home.join("adapters/demo/1");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(pkg.join("publisher"), "acme\n").unwrap();
    let payload = r#"{"version":2,"expires_ms":1,"publishers":[{"id":"acme","key":"00","revoked":true}],"packages":[]}"#;
    let envelope = format!("{{\"payload\":{payload:?},\"sig\":\"\"}}");
    std::fs::write(home.join("registry-index"), envelope).unwrap();
    std::env::set_var("AGORA_HOME", &home);

    let program = pkg.join("run");
    let manifest = Manifest::parse(&format!(
        "kind = \"demo\"\ndisplay_name = \"Demo\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"demo\"]\n[adapter]\ncommand = [{:?}]\n",
        program.to_string_lossy()
    ))
    .unwrap();
    let mut sup = Supervisor::new(SupervisorConfig {
        node: NodeId::new("node-1"),
        restart: RestartPolicy::default(),
        terminal: TerminalOptions::default(),
        limits: NodeLimits::default(),
    });
    sup.register(manifest);
    let worktrees = dir.join("worktrees");
    let err = sup
        .start(
            &StartRequest {
                kind: "demo".into(),
                checkout: dir.clone(),
                worktree_root: worktrees.clone(),
                name: "x".into(),
                base: None,
            },
            Instant::now(),
        )
        .unwrap_err();
    assert_eq!(
        err,
        SupervisorError::PublisherRevoked("demo".into(), "acme".into())
    );
    let coded = err.to_protocol();
    assert_eq!(coded.code, ErrorCode::AgentPublisherRevoked);
    assert!(!worktrees.exists());
    let _ = std::fs::remove_dir_all(dir);
}
