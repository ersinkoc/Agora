//! A terminal agent's usage, read from the file the agent writes itself (ADR-0042), through a real node: the manifest names the
//! file, the node follows it for the agent's own directory only, and the numbers reach the event log. The agent is a plain shell.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Write;
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

fn line(cwd: &str, id: &str, i: u64, o: u64) -> String {
    serde_json::json!({"type":"assistant","cwd":cwd,"model":"m-1","id":id,"in":i,"out":o})
        .to_string()
}

#[test]
fn a_running_terminal_agents_own_usage_file_reaches_the_event_log_and_a_foreign_one_does_not() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let dir: PathBuf = std::env::temp_dir().join(format!("agora-usage-e2e-{}", std::process::id()));
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
    std::fs::write(
        manifests.join("usagesh.toml"),
        r#"
kind = "usagesh"
display_name = "Usage shell"
version = "1"
min_engine = 1
[process]
launch = ["sh"]
[usage]
source = "file"
[usage.file]
glob = "{home}/logs/*/*.jsonl"
format = "jsonl"
select = { path = "type", equals = "assistant" }
cwd = "cwd"
model = "model"
input = "in"
output = "out"
dedupe = "id"
mode = "delta"
"#,
    )
    .unwrap();
    // The node reads the user's home from the environment when it starts; this binary has this one test.
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
            kind: "usagesh".into(),
            ..Default::default()
        }))
        .unwrap()
        .unwrap();
    let Some(Ok_::AgentStart(s)) = r.result else {
        panic!("not started");
    };
    let worktree = std::fs::canonicalize(&s.worktree_id).unwrap();
    let agent = s.agent_id.clone();
    // Let the node notice the agent, then the "agent" writes its usage; another session writes to the same directory of logs.
    std::thread::sleep(Duration::from_millis(400));
    let mine = worktree.to_string_lossy().into_owned();
    let mut f = std::fs::File::create(home.join("logs/p/mine.jsonl")).unwrap();
    writeln!(f, "{}", line(&mine, "a1", 10, 2)).unwrap();
    writeln!(f, "{}", line(&mine, "a1", 10, 2)).unwrap();
    writeln!(f, "{}", line(&mine, "a2", 5, 1)).unwrap();
    std::fs::write(
        home.join("logs/p/foreign.jsonl"),
        format!("{}\n", line("/somewhere/else", "z1", 999, 999)),
    )
    .unwrap();

    let end = Instant::now() + Duration::from_secs(20);
    let totals = loop {
        let (mut i, mut o, mut others) = (0, 0, 0);
        for e in n.events.events_after(0) {
            if let Some(Body::AgentUsageReported(u)) = e.body {
                if u.agent_id == agent && u.model == "m-1" {
                    i += u.input_tokens;
                    o += u.output_tokens;
                } else {
                    others += 1;
                }
            }
        }
        if (i, o) == (15, 3) || Instant::now() > end {
            break (i, o, others);
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert_eq!(
        totals,
        (15, 3, 0),
        "the repeated message counts once, the foreign file never"
    );
    // And it stays that way: nothing is reported twice.
    std::thread::sleep(Duration::from_millis(2500));
    let sum: u64 = n
        .events
        .events_after(0)
        .into_iter()
        .filter_map(|e| match e.body {
            Some(Body::AgentUsageReported(u)) => Some(u.input_tokens),
            _ => None,
        })
        .sum();
    assert_eq!(sum, 15);
    let _ = c.call(Cmd::AgentStop(v1::AgentStop { agent_id: agent }));
    drop(n);
    let _ = std::fs::remove_dir_all(dir);
}
