//! What an agent writes to its usage file in its last moments, then exits (ADR-0042), is still reported: the node reads the file
//! one last time after the agent ended. The agent is a plain shell that writes one line and quits at once.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use agora_node::control::Client;
use agora_node::daemon::{Node, NodeConfig};
use agora_node::supervisor::NodeLimits;
use agora_protocol::v1::{
    self, command_ok::Result as Ok_, command_request::Command as Cmd, event::Body,
};

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
fn what_an_agent_wrote_just_before_it_exited_is_still_reported() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    // The "agent" is a POSIX shell that writes its own `$PWD` into the file; where that shell reports paths in its own form (Git's
    // `sh` on Windows says `/d/a/...`), the directory it writes is not the worktree's native path, so the line cannot be matched.
    if std::path::MAIN_SEPARATOR == '\\' {
        return;
    }
    let dir: PathBuf = std::env::temp_dir().join(format!("agora-usage-end-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let repo = dir.join("myproject");
    let home = dir.join("home");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(home.join("logs/p")).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let manifests = dir.join("manifests");
    std::fs::create_dir_all(&manifests).unwrap();
    let out = home.join("logs/p/last.jsonl");
    std::fs::write(
        manifests.join("lastsh.toml"),
        format!(
            r#"
kind = "lastsh"
display_name = "Last words shell"
version = "1"
min_engine = 1
[process]
launch = ["sh", "-c", "printf '{{\"type\":\"assistant\",\"cwd\":\"%s\",\"model\":\"m-1\",\"in\":7,\"out\":3}}\n' \"$(pwd -P)\" > '{}'"]
[usage]
source = "file"
[usage.file]
glob = "{{home}}/logs/*/*.jsonl"
format = "jsonl"
select = {{ path = "type", equals = "assistant" }}
cwd = "cwd"
model = "model"
input = "in"
output = "out"
mode = "delta"
"#,
            out.to_string_lossy().replace('\\', "/")
        ),
    )
    .unwrap();
    std::env::set_var("HOME", &home);
    let n = Node::start(&NodeConfig {
        node_id: "node-test".into(),
        socket: dir.join("c.sock"),
        data_dir: dir.join("data"),
        worktree_root: dir.join("worktrees"),
        manifests_dir: manifests,
        projects: vec![repo],
        limits: NodeLimits::default(),
    })
    .unwrap();
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let r = c
        .call(Cmd::AgentStart(v1::AgentStart {
            kind: "lastsh".into(),
            ..Default::default()
        }))
        .unwrap()
        .unwrap();
    let Some(Ok_::AgentStart(_)) = r.result else {
        panic!("not started");
    };
    let end = Instant::now() + Duration::from_secs(20);
    let total = loop {
        let t: u64 = n
            .events
            .events_after(0)
            .into_iter()
            .filter_map(|e| match e.body {
                Some(Body::AgentUsageReported(u)) => Some(u.input_tokens),
                _ => None,
            })
            .sum();
        if t == 7 || Instant::now() > end {
            break t;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert_eq!(total, 7, "the line written right before exit is reported");
    drop(n);
    let _ = std::fs::remove_dir_all(dir);
}
