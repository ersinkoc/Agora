//! ADR-0091 end to end: a running terminal agent's own usage file carries plan limits (Codex-shaped `rate_limits`); the node reads them
//! through the manifest's `[quota]`, and `AgentQuotaObserved` reaches the event log with the numbers and nothing else the line said.
//! A file of another directory never counts. The agent is a plain shell; the file is written by the test.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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

/// A Codex-shaped `token_count` line, with the account fields Codex also writes (made-up values).
fn line(cwd: &str, primary: f64, resets: u64, secondary: Option<f64>) -> String {
    serde_json::json!({
        "type": "event_msg",
        "cwd": cwd,
        "payload": {
            "type": "token_count",
            "info": null,
            "rate_limits": {
                "primary": {"used_percent": primary, "window_minutes": 300, "resets_at": resets},
                "secondary": secondary.map(|p| serde_json::json!({"used_percent": p, "window_minutes": 10080, "resets_at": resets + 86_400})),
                "credits": {"has_credits": true, "balance": "BALANCE-4242"},
                "plan_type": "PLAN-SECRET",
                "account_id": "ACCOUNT-SECRET",
                "token": "TOKEN-SECRET"
            }
        }
    })
    .to_string()
}

fn quota_events(n: &Node) -> Vec<v1::AgentQuotaObserved> {
    n.events
        .events_after(0)
        .into_iter()
        .filter_map(|e| match e.body {
            Some(Body::AgentQuotaObserved(q)) => Some(q),
            _ => None,
        })
        .collect()
}

#[test]
fn a_running_agents_plan_limits_reach_the_event_log_as_numbers_only() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED || !agora_platform::PTY_SUPPORTED {
        return;
    }
    let dir: PathBuf = std::env::temp_dir().join(format!("agora-quota-e2e-{}", std::process::id()));
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
        manifests.join("quotash.toml"),
        r#"
kind = "quotash"
display_name = "Quota shell"
version = "1"
min_engine = 5
[process]
launch = ["sh"]
[usage]
source = "file"
[usage.file]
glob = "{home}/logs/*/*.jsonl"
format = "jsonl"
select = { path = "payload.type", equals = "token_count" }
cwd = "cwd"
model = "payload.model"
input = "payload.info.total_token_usage.input_tokens"
output = "payload.info.total_token_usage.output_tokens"
mode = "total"
[quota]
[[quota.window]]
percent = "payload.rate_limits.primary.used_percent"
minutes = "payload.rate_limits.primary.window_minutes"
resets = "payload.rate_limits.primary.resets_at"
[[quota.window]]
percent = "payload.rate_limits.secondary.used_percent"
minutes = "payload.rate_limits.secondary.window_minutes"
resets = "payload.rate_limits.secondary.resets_at"
"#,
    )
    .unwrap();
    // The node reads the user's home from the environment when it starts; this binary has this one test.
    std::env::set_var("HOME", &home);
    let n = Node::start(&NodeConfig {
        resume_all: false,
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
            kind: "quotash".into(),
            ..Default::default()
        }))
        .unwrap()
        .unwrap();
    let Some(Ok_::AgentStart(s)) = r.result else {
        panic!("not started");
    };
    let worktree = std::fs::canonicalize(&s.worktree_id).unwrap();
    let agent = s.agent_id.clone();
    std::thread::sleep(Duration::from_millis(400));
    let mine = worktree.to_string_lossy().into_owned();
    let resets = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;
    let mut f = std::fs::File::create(home.join("logs/p/mine.jsonl")).unwrap();
    writeln!(f, "{}", line(&mine, 40.0, resets, Some(7.0))).unwrap();
    std::fs::write(
        home.join("logs/p/foreign.jsonl"),
        format!("{}\n", line("/somewhere/else", 99.0, resets, Some(99.0))),
    )
    .unwrap();

    let wait = |want: &dyn Fn(&[v1::AgentQuotaObserved]) -> bool| {
        let end = Instant::now() + Duration::from_secs(25);
        loop {
            let got = quota_events(&n);
            if want(&got) || Instant::now() > end {
                return got;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    };
    let got = wait(&|q| q.len() >= 2);
    assert_eq!(got.len(), 2, "{got:?}");
    let by = |w: &str| got.iter().find(|q| q.window == w).unwrap();
    let (five, seven) = (by("five_hour"), by("seven_day"));
    assert_eq!((five.used_percent, seven.used_percent), (40, 7));
    assert_eq!(five.resets_at_ms, resets * 1000);
    for q in &got {
        assert_eq!(
            (q.node_id.as_str(), q.tool.as_str()),
            ("node-test", "quotash")
        );
        assert_eq!(q.source, "file");
        assert!(q.observed_at_ms > 0);
    }
    // Same numbers again are not an event; a change is (once it is due, at most one per window every 5 seconds).
    writeln!(f, "{}", line(&mine, 40.0, resets, Some(7.0))).unwrap();
    writeln!(f, "{}", line(&mine, 41.0, resets, Some(7.0))).unwrap();
    let got = wait(&|q| q.iter().any(|q| q.used_percent == 41));
    let five: Vec<_> = got.iter().filter(|q| q.window == "five_hour").collect();
    assert_eq!(
        five.iter().map(|q| q.used_percent).collect::<Vec<_>>(),
        [40, 41]
    );
    assert_eq!(
        got.iter().filter(|q| q.window == "seven_day").count(),
        1,
        "an unchanged window says nothing again"
    );
    // The foreign file's numbers, and every other field of the line, never reach the log.
    let log = format!("{:?}", n.events.events_after(0));
    for never in [
        "BALANCE-4242",
        "PLAN-SECRET",
        "ACCOUNT-SECRET",
        "TOKEN-SECRET",
    ] {
        assert!(!log.contains(never), "{never} reached the event log");
    }
    assert!(got.iter().all(|q| q.used_percent != 99));
    let _ = c.call(Cmd::AgentStop(v1::AgentStop {
        agent_id: agent,
        stop_helpers: false,
    }));
    drop(n);
    let _ = std::fs::remove_dir_all(dir);
}
