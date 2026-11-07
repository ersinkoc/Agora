//! Footprint measurement (P2-15): a real node with many terminals, compared with the budgets in
//! `docs/design/performance.md` §1. The node runs in this process, so "base" includes the bench's own small
//! overhead (a slight overestimate). Children (the shells) are separate processes and are not counted: the
//! budget is the node's own memory.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use agora_adapters::Manifest;
use agora_core::{AgentId, NodeId};
use agora_node::supervisor::{
    NodeLimits, RestartPolicy, StartRequest, Supervisor, SupervisorConfig,
};
use agora_node::terminal::{Output, TerminalOptions};
use agora_protocol::v1::{command_request::Command as Cmd, ProjectList};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FootprintReport {
    pub commit: String,
    pub terminals: usize,
    /// `Node::start` until the control socket answers a command.
    pub cold_start_ms: f64,
    /// Node RSS with no terminals.
    pub base_rss_mb: f64,
    /// RSS added per idle terminal (fresh shell prompt).
    pub per_terminal_rss_mb: f64,
    /// RSS added per terminal after it printed 3000 lines (scrollback filling toward its 2000-line cap).
    pub per_terminal_busy_rss_mb: f64,
    /// CPU used by the node while all terminals are idle, as a percentage of one core.
    pub idle_cpu_pct: f64,
    /// Keystroke sent to the node until its echo is delivered to a subscriber (p95), with all terminals open.
    pub echo_p95_ms: f64,
}

/// One budget line.
#[derive(Debug, Clone, PartialEq)]
pub struct BudgetCheck {
    pub name: &'static str,
    pub value: f64,
    pub limit: f64,
    pub unit: &'static str,
}

impl BudgetCheck {
    pub fn ok(&self) -> bool {
        self.value <= self.limit
    }
}

/// The footprint budgets of `docs/design/performance.md` §1 that this measurement can check.
pub fn budgets(r: &FootprintReport) -> Vec<BudgetCheck> {
    vec![
        BudgetCheck {
            name: "node base memory",
            value: r.base_rss_mb,
            limit: 50.0,
            unit: "MB",
        },
        BudgetCheck {
            name: "memory per idle terminal",
            value: r.per_terminal_rss_mb,
            limit: 10.0,
            unit: "MB",
        },
        BudgetCheck {
            name: "memory per busy terminal",
            value: r.per_terminal_busy_rss_mb,
            limit: 10.0,
            unit: "MB",
        },
        BudgetCheck {
            name: "idle CPU",
            value: r.idle_cpu_pct,
            limit: 1.0,
            unit: "%",
        },
        BudgetCheck {
            name: "node cold start",
            value: r.cold_start_ms,
            limit: 200.0,
            unit: "ms",
        },
        BudgetCheck {
            name: "keystroke to echo (local)",
            value: r.echo_p95_ms,
            limit: 16.0,
            unit: "ms",
        },
    ]
}

const MB: f64 = 1024.0 * 1024.0;

fn git(dir: &Path, args: &[&str]) -> std::io::Result<()> {
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "bench")
        .env("GIT_AUTHOR_EMAIL", "bench@example.com")
        .env("GIT_COMMITTER_NAME", "bench")
        .env("GIT_COMMITTER_EMAIL", "bench@example.com")
        .status()?
        .success();
    ok.then_some(())
        .ok_or_else(|| std::io::Error::other(format!("git {args:?} failed")))
}

fn rss_mb() -> std::io::Result<f64> {
    Ok(agora_platform::process_stats(std::process::id())?.rss_bytes as f64 / MB)
}

fn cpu_s() -> std::io::Result<f64> {
    Ok(agora_platform::process_stats(std::process::id())?.cpu_seconds)
}

fn p95_ms(samples: &mut [Duration]) -> f64 {
    crate::p95_us(samples) / 1000.0
}

/// Measure a node with `terminals` terminals, observing idle CPU for `idle_for`.
pub fn run(terminals: usize, idle_for: Duration, commit: &str) -> std::io::Result<FootprintReport> {
    let dir = std::env::temp_dir().join(format!("agora-footprint-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo)?;
    git(&repo, &["init", "-q", "-b", "main"])?;
    std::fs::write(repo.join("a.txt"), "x")?;
    git(&repo, &["add", "."])?;
    git(&repo, &["commit", "-q", "-m", "init"])?;
    let manifests = dir.join("manifests");
    std::fs::create_dir_all(&manifests)?;
    std::fs::write(
        manifests.join("shell.toml"),
        "kind = \"shell\"\ndisplay_name = \"Shell\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"sh\"]\n",
    )?;

    // Cold start: from nothing to a node that answers a command.
    let t0 = Instant::now();
    let node = agora_node::daemon::Node::start(&agora_node::daemon::NodeConfig {
        node_id: "bench".into(),
        socket: dir.join("n.sock"),
        data_dir: dir.join("data"),
        worktree_root: dir.join("worktrees"),
        manifests_dir: manifests.clone(),
        projects: vec![repo.clone()],
        limits: NodeLimits::default(),
    })
    .map_err(|e| std::io::Error::other(e.to_string()))?;
    let mut client = agora_node::control::Client::connect(&dir.join("n.sock"))?;
    client
        .call(Cmd::ProjectList(ProjectList {}))?
        .map_err(|e| std::io::Error::other(e.code))?;
    let cold_start_ms = t0.elapsed().as_secs_f64() * 1000.0;
    drop(client);
    drop(node);

    // The measured node: a bare supervisor (the control socket and ticker add little and are covered above).
    let mut sup = Supervisor::new(SupervisorConfig {
        node: NodeId::new("bench"),
        restart: RestartPolicy::default(),
        terminal: TerminalOptions::default(),
        limits: NodeLimits::default(),
    });
    let manifest = Manifest::parse(&std::fs::read_to_string(manifests.join("shell.toml"))?)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    sup.register(manifest);
    settle();
    let base = rss_mb()?;

    let mut ids: Vec<AgentId> = Vec::new();
    for i in 0..terminals {
        let id = sup
            .start(
                &StartRequest {
                    kind: "shell".into(),
                    checkout: repo.clone(),
                    worktree_root: dir.join("wt"),
                    name: format!("t{i}"),
                    base: None,
                },
                Instant::now(),
            )
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        ids.push(id);
    }
    std::thread::sleep(Duration::from_millis(1500));
    settle();
    let idle_rss = rss_mb()?;
    let per_terminal_rss_mb = (idle_rss - base) / terminals.max(1) as f64;

    // Fill every terminal's scrollback.
    for id in &ids {
        if let Some(t) = sup.terminal(id) {
            let _ = t.send_input(b"seq 1 3000\n".to_vec());
        }
    }
    let end = Instant::now() + Duration::from_secs(30);
    for id in &ids {
        while Instant::now() < end {
            let done = sup
                .terminal(id)
                .is_some_and(|t| t.snapshot().lines.iter().any(|l| l == "3000"));
            if done {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    settle();
    let busy_rss = rss_mb()?;
    let per_terminal_busy_rss_mb = (busy_rss - base) / terminals.max(1) as f64;

    // Idle CPU, with the supervisor loop ticking as the node does.
    std::thread::sleep(Duration::from_millis(500));
    let (c0, w0) = (cpu_s()?, Instant::now());
    while w0.elapsed() < idle_for {
        let _ = sup.tick(Instant::now());
        std::thread::sleep(Duration::from_millis(100));
    }
    let idle_cpu_pct = (cpu_s()? - c0) / w0.elapsed().as_secs_f64() * 100.0;

    // Keystroke to echo through the node, with all terminals open.
    let mut samples = Vec::new();
    if let Some(first) = ids.first() {
        if let Some(t) = sup.terminal(first) {
            let (_, sub) = t.subscribe();
            for i in 0..200 {
                // A character no earlier output contains, cycling through a small set.
                let ch = b"qwertyuiop"[i % 10];
                let sent = Instant::now();
                let _ = t.send_input(vec![ch]);
                let end = sent + Duration::from_secs(2);
                while Instant::now() < end {
                    if let Ok(Output::Data(d)) = sub.recv_timeout(Duration::from_millis(50)) {
                        if d.contains(&ch) {
                            samples.push(sent.elapsed());
                            break;
                        }
                    }
                }
                if i % 40 == 39 {
                    let _ = t.send_input(vec![0x03]); // Ctrl-C clears the typed line
                    std::thread::sleep(Duration::from_millis(30));
                }
            }
        }
    }
    let echo_p95_ms = p95_ms(&mut samples);

    drop(sup);
    let _ = std::fs::remove_dir_all(&dir);
    Ok(FootprintReport {
        commit: commit.to_string(),
        terminals,
        cold_start_ms,
        base_rss_mb: base,
        per_terminal_rss_mb,
        per_terminal_busy_rss_mb,
        idle_cpu_pct,
        echo_p95_ms,
    })
}

/// Let allocator and threads settle before reading memory.
fn settle() {
    std::thread::sleep(Duration::from_millis(300));
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn report(cpu: f64) -> FootprintReport {
        FootprintReport {
            commit: "x".into(),
            terminals: 50,
            cold_start_ms: 10.0,
            base_rss_mb: 4.0,
            per_terminal_rss_mb: 0.1,
            per_terminal_busy_rss_mb: 4.0,
            idle_cpu_pct: cpu,
            echo_p95_ms: 0.1,
        }
    }

    #[test]
    fn budgets_flag_only_what_is_over() {
        assert!(budgets(&report(0.4)).iter().all(BudgetCheck::ok));
        let over: Vec<_> = budgets(&report(1.5))
            .into_iter()
            .filter(|b| !b.ok())
            .collect();
        assert_eq!(over.len(), 1);
        assert_eq!(over[0].name, "idle CPU");
    }

    #[test]
    fn a_small_node_can_be_measured_end_to_end() {
        if !agora_platform::PTY_SUPPORTED
            || agora_platform::process_stats(std::process::id()).is_err()
        {
            return;
        }
        let r = run(3, Duration::from_millis(500), "test").unwrap();
        assert_eq!(r.terminals, 3);
        assert!(
            r.base_rss_mb > 0.5 && r.cold_start_ms > 0.0 && r.echo_p95_ms > 0.0,
            "{r:?}"
        );
    }
}
