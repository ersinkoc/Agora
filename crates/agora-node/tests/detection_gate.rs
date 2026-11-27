//! ADR-0117 decision 1: a real node and a scripted fake agent whose screen flickers between a working line and an empty input box every
//! 150 ms. Under a kind with `idle_confirm_ms = 300` the agent is said idle once, when the box stays; under the same kind without the
//! gate the flicker reads as idle again and again.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use agora_node::{
    control::Client,
    daemon::{Node, NodeConfig},
    supervisor::NodeLimits,
};
use agora_protocol::v1::{self, command_ok::Result as Ok_, command_request::Command as Cmd};
use std::{
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};

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
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let mut command = Command::new(cargo);
        command
            .args(["build", "-q", "-p", "agora-fake-agent", "--target-dir"])
            .arg(p.parent().unwrap().parent().unwrap());
        if p.parent().unwrap().ends_with("release") {
            command.arg("--release");
        }
        assert!(command.status().unwrap().success());
    });
    p.to_string_lossy().into_owned()
}

/// The states the node said for `agent`, in order.
fn states(node: &Node, agent: &str) -> Vec<i32> {
    node.events
        .events_after(0)
        .into_iter()
        .filter_map(|ev| match ev.body {
            Some(v1::event::Body::AgentStateChanged(s)) if s.agent_id == agent => Some(s.state),
            _ => None,
        })
        .collect()
}

#[test]
fn a_flickering_screen_is_said_idle_once_under_the_gate() {
    if !agora_platform::PTY_SUPPORTED || !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let root =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("gate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("manifests")).unwrap();
    std::fs::create_dir_all(root.join("data")).unwrap();
    // Seven flickers of 150 ms each way, then the box stays.
    let mut script = String::new();
    for _ in 0..7 {
        script.push_str("[[frame]]\ntext = \"FLICKER-WORKING\"\ndelay_ms = 150\n");
        script.push_str("[[frame]]\ntext = \"FLICKER-BOX\"\ndelay_ms = 150\n");
    }
    script.push_str("[[frame]]\ntext = \"FLICKER-WORKING\"\ndelay_ms = 150\n[[frame]]\ntext = \"FLICKER-BOX\"\ndelay_ms = 120000\n");
    std::fs::write(root.join("flicker.toml"), script).unwrap();
    let launch = serde_json::to_string(&vec![
        fake(),
        "--script".into(),
        root.join("flicker.toml").to_string_lossy().into_owned(),
    ])
    .unwrap();
    let rules = "[[rules]]\nid = \"working\"\nstate = \"working\"\npriority = 800\nregion = \"bottom_non_empty_lines(3)\"\ncontains = \"FLICKER-WORKING\"\n[[rules]]\nid = \"idle\"\nstate = \"idle\"\npriority = 100\nregion = \"bottom_non_empty_lines(3)\"\ncontains = \"FLICKER-BOX\"\n";
    for (kind, detection) in [
        ("gated", "[detection]\nidle_confirm_ms = 300\n"),
        ("raw", ""),
    ] {
        std::fs::write(
            root.join(format!("manifests/{kind}.toml")),
            format!("kind = \"{kind}\"\ndisplay_name = \"F\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = {launch}\n{detection}{rules}"),
        )
        .unwrap();
    }
    let node = Node::start(&NodeConfig {
        resume_all: false,
        node_id: "n".into(),
        socket: root.join("n.sock"),
        data_dir: root.join("data"),
        worktree_root: root.join("worktrees"),
        manifests_dir: root.join("manifests"),
        projects: vec![],
        limits: NodeLimits::default(),
    })
    .unwrap();
    let mut c = Client::connect(node.socket().unwrap()).unwrap();
    let folder = root.join("work");
    std::fs::create_dir_all(&folder).unwrap();
    let info = match c
        .call(Cmd::DirectoryInspect(v1::DirectoryInspect {
            path: folder.to_string_lossy().into_owned(),
            create_project_id: false,
            authorization: None,
        }))
        .unwrap()
        .unwrap()
        .result
    {
        Some(Ok_::DirectoryInfo(i)) => i,
        other => panic!("{other:?}"),
    };
    let mut start = |kind: &str| match c
        .call(Cmd::AgentLaunch(v1::AgentLaunch {
            path: info.path.clone(),
            project_id: info.project_id.clone(),
            kind: kind.into(),
            ..Default::default()
        }))
        .unwrap()
        .unwrap()
        .result
    {
        Some(Ok_::AgentLaunch(r)) => r.agent_id,
        other => panic!("{other:?}"),
    };
    let gated = start("gated");
    let raw = start("raw");
    let idle = v1::AgentState::Idle as i32;
    // The script runs about 2.4 s, then the box stays: the gated agent is idle once that settled.
    let until = Instant::now() + Duration::from_secs(20);
    while !(states(&node, &gated).last() == Some(&idle)
        && Instant::now() > until - Duration::from_secs(16))
    {
        assert!(
            Instant::now() < until,
            "never idle: {:?}",
            states(&node, &gated)
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::sleep(Duration::from_millis(500));
    let g = states(&node, &gated);
    let r = states(&node, &raw);
    assert_eq!(g.iter().filter(|s| **s == idle).count(), 1, "gated: {g:?}");
    assert!(
        r.iter().filter(|s| **s == idle).count() > 1,
        "without the gate the flicker shows: {r:?}"
    );
    drop(c);
    drop(node);
    let _ = std::fs::remove_dir_all(&root);
}
