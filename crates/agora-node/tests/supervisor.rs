//! Supervisor integration tests with `agora-fake-agent` (never a real agent), on every platform with a pseudo-terminal (ADR-0045).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use agora_adapters::Manifest;
use agora_core::{AgentId, AgentState, NodeId};
use agora_node::supervisor::{
    NodeLimits, RestartPolicy, StartRequest, Supervisor, SupervisorConfig, SupervisorError,
    SupervisorEvent,
};
use agora_node::terminal::TerminalOptions;
use agora_protocol::ErrorCode;

fn fake_agent() -> String {
    let mut p = std::env::current_exe().expect("exe");
    p.pop();
    p.pop();
    p.push(format!("agora-fake-agent{}", std::env::consts::EXE_SUFFIX));
    // Built once per test binary, even when present: a binary left from an older checkout may lack newer flags.
    static FRESH: std::sync::Once = std::sync::Once::new();
    FRESH.call_once(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let ok = Command::new(cargo)
            .args(["build", "-q", "-p", "agora-fake-agent"])
            .status()
            .expect("run cargo")
            .success();
        assert!(ok, "building agora-fake-agent failed");
    });
    p.to_string_lossy().into_owned()
}

fn script() -> String {
    format!(
        "{}/tests/fixtures/supervised.toml",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn manifest(launch: &str, resume: Option<&str>) -> Manifest {
    let resume = resume
        .map(|r| format!("[resume]\nargv = {r}\n"))
        .unwrap_or_default();
    Manifest::parse(&format!(
        r#"
kind = "fake"
display_name = "Fake"
version = "1"
min_engine = 1
[process]
names = ["fake"]
launch = {launch}
{resume}
[[rules]]
id = "blocked"
state = "blocked"
priority = 900
region = "bottom_non_empty_lines(8)"
contains = "Allow Bash"

[[rules]]
id = "done"
state = "done"
priority = 800
region = "bottom_non_empty_lines(8)"
regex = '^Done\.$'

[[rules]]
id = "working"
state = "working"
priority = 700
region = "bottom_non_empty_lines(8)"
contains = "Thinking"
"#
    ))
    .expect("manifest")
}

fn fake_launch() -> String {
    format!("[{:?}, \"--script\", {:?}]", fake_agent(), script())
}

struct Env {
    dir: PathBuf,
    repo: PathBuf,
}

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
        .expect("git")
        .success();
    assert!(ok, "git {args:?} failed");
}

fn env(name: &str) -> Env {
    let dir = std::env::temp_dir().join(format!("agora-sup-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    Env { dir, repo }
}

fn supervisor(restart: RestartPolicy) -> Supervisor {
    Supervisor::new(SupervisorConfig {
        node: NodeId::new("node-1"),
        restart,
        terminal: TerminalOptions::default(),
        limits: NodeLimits::default(),
    })
}

fn request(e: &Env, name: &str) -> StartRequest {
    StartRequest {
        kind: "fake".into(),
        checkout: e.repo.clone(),
        worktree_root: e.dir.join("worktrees"),
        name: name.into(),
        base: None,
    }
}

/// Tick until `done(events)` is true or the timeout passes; returns every event seen.
fn run_until(
    sup: &mut Supervisor,
    timeout: Duration,
    mut done: impl FnMut(&[SupervisorEvent]) -> bool,
) -> Vec<SupervisorEvent> {
    let end = Instant::now() + timeout;
    let mut all = Vec::new();
    while Instant::now() < end {
        all.extend(sup.tick(Instant::now()));
        if done(&all) {
            return all;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out; events so far: {all:#?}");
}

fn states(events: &[SupervisorEvent]) -> Vec<AgentState> {
    events
        .iter()
        .filter_map(|e| match e {
            SupervisorEvent::StateChanged { state, .. } => Some(*state),
            _ => None,
        })
        .collect()
}

fn exited(events: &[SupervisorEvent]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, SupervisorEvent::Exited { .. }))
        .count()
}

#[test]
fn start_in_own_worktree_report_states_stop_and_resume() {
    let e = env("lifecycle");
    let mut sup = supervisor(RestartPolicy::default());
    sup.register(manifest(&fake_launch(), None));

    let id = sup.start(&request(&e, "one"), Instant::now()).unwrap();
    let view = sup.list().remove(0);
    assert_eq!(view.branch, "agent/one");
    assert!(view.running);
    assert!(
        view.worktree.join("a.txt").exists(),
        "own worktree checked out"
    );
    assert!(view.worktree.starts_with(e.dir.join("worktrees")));

    // State changes are reported from the screen: working, then blocked, then done.
    let events = run_until(&mut sup, Duration::from_secs(20), |ev| exited(ev) == 1);
    assert_eq!(
        states(&events),
        vec![AgentState::Working, AgentState::Blocked, AgentState::Done]
    );
    assert!(!sup.list()[0].running);
    assert_eq!(sup.list()[0].restarts, 0, "a clean exit is not restarted");

    // Stop (already exited) and resume without a session id: layout-only fallback, fresh process.
    let events = sup.stop(&id).unwrap();
    assert!(events.contains(&SupervisorEvent::Stopped { agent: id.clone() }));
    let events = sup.resume(&id, Instant::now()).unwrap();
    assert!(events.contains(&SupervisorEvent::Resumed {
        agent: id.clone(),
        native: false
    }));
    assert!(sup.list()[0].running);
    // Screen detection works again for the new process.
    let events = run_until(&mut sup, Duration::from_secs(20), |ev| exited(ev) == 1);
    assert!(states(&events).contains(&AgentState::Working));

    sup.stop(&id).unwrap();
}

#[test]
fn resume_substitutes_the_session_id_into_the_manifest_command() {
    let e = env("native");
    let mut sup = supervisor(RestartPolicy::default());
    // The resume command prints its own argv, which proves the placeholder was substituted.
    sup.register(manifest(
        "[\"sh\", \"-c\", \"sleep 30\"]",
        Some("[\"sh\", \"-c\", \"echo resumed-{session_id}; sleep 30\"]"),
    ));
    let id = sup.start(&request(&e, "n"), Instant::now()).unwrap();
    sup.report_session(&id, "abc123").unwrap();
    sup.stop(&id).unwrap();
    sup.resume(&id, Instant::now()).unwrap();
    let t = sup.terminal(&id).unwrap();
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        if t.snapshot()
            .lines
            .iter()
            .any(|l| l.ends_with("resumed-abc123"))
        {
            break;
        }
        assert!(Instant::now() < end, "session id never reached the command");
        std::thread::sleep(Duration::from_millis(20));
    }
    sup.stop(&id).unwrap();
}

#[test]
fn reported_state_outranks_the_screen() {
    let e = env("reported");
    let mut sup = supervisor(RestartPolicy::default());
    sup.register(manifest(&fake_launch(), None));
    let id = sup.start(&request(&e, "r"), Instant::now()).unwrap();
    let ev = sup.report_state(&id, AgentState::Idle).unwrap();
    assert_eq!(states(&ev), vec![AgentState::Idle]);
    // The script now shows working/blocked screens; the reported idle state must hold.
    let events = run_until(&mut sup, Duration::from_secs(20), |ev| exited(ev) == 1);
    assert!(states(&events)
        .iter()
        .all(|s| *s != AgentState::Working && *s != AgentState::Blocked));
    sup.stop(&id).unwrap();
}

#[test]
fn crashing_agents_restart_with_backoff_then_give_up() {
    let e = env("crash");
    let policy = RestartPolicy {
        max_restarts: 2,
        base_delay: Duration::from_millis(30),
        max_delay: Duration::from_millis(60),
    };
    let mut sup = supervisor(policy);
    sup.register(manifest("[\"sh\", \"-c\", \"exit 3\"]", None));
    let id = sup.start(&request(&e, "c"), Instant::now()).unwrap();

    let events = run_until(&mut sup, Duration::from_secs(20), |ev| {
        ev.iter()
            .any(|e| matches!(e, SupervisorEvent::GaveUp { .. }))
    });
    let restarting: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            SupervisorEvent::Restarting { attempt, after, .. } => Some((*attempt, *after)),
            _ => None,
        })
        .collect();
    assert_eq!(
        restarting,
        vec![
            (1, Duration::from_millis(30)),
            (2, Duration::from_millis(60))
        ]
    );
    assert_eq!(exited(&events), 3, "original run plus two restarts");
    let view = sup.list().remove(0);
    assert_eq!(
        (view.restarts, view.running, view.state),
        (2, false, AgentState::Unknown)
    );

    // A given-up agent can be resumed by the user, which resets the counter.
    sup.resume(&id, Instant::now()).unwrap();
    assert_eq!(sup.list()[0].restarts, 0);
    sup.stop(&id).unwrap();
}

#[test]
fn stopping_cancels_a_pending_restart() {
    let e = env("cancel");
    let policy = RestartPolicy {
        max_restarts: 5,
        base_delay: Duration::from_millis(300),
        max_delay: Duration::from_secs(1),
    };
    let mut sup = supervisor(policy);
    sup.register(manifest("[\"sh\", \"-c\", \"exit 1\"]", None));
    let id = sup.start(&request(&e, "s"), Instant::now()).unwrap();
    run_until(&mut sup, Duration::from_secs(10), |ev| {
        ev.iter()
            .any(|e| matches!(e, SupervisorEvent::Restarting { .. }))
    });
    sup.stop(&id).unwrap();
    std::thread::sleep(Duration::from_millis(600));
    assert!(sup.tick(Instant::now()).is_empty());
    assert!(!sup.list()[0].running);
}

#[test]
fn errors_map_to_protocol_codes() {
    let e = env("errors");
    let mut sup = supervisor(RestartPolicy::default());
    sup.register(manifest("[\"sh\", \"-c\", \"sleep 30\"]", None));

    let mut req = request(&e, "x");
    req.kind = "nope".into();
    let err = sup.start(&req, Instant::now()).unwrap_err();
    assert_eq!(err, SupervisorError::UnknownKind("nope".into()));
    assert_eq!(err.to_protocol().code, ErrorCode::AgentKindUnknown);

    let id = sup.start(&request(&e, "x"), Instant::now()).unwrap();
    // Same name again: the branch exists.
    let err = sup.start(&request(&e, "x"), Instant::now()).unwrap_err();
    assert_eq!(err.to_protocol().code, ErrorCode::WorktreeExists);
    // Resuming a running agent is refused.
    let err = sup.resume(&id, Instant::now()).unwrap_err();
    assert_eq!(err, SupervisorError::StillRunning(id.clone()));
    let missing = AgentId::new("agent-999");
    assert_eq!(
        sup.stop(&missing).unwrap_err().to_protocol().code,
        ErrorCode::AgentNotFound
    );
    sup.stop(&id).unwrap();
}

#[test]
fn restart_delay_doubles_and_is_capped() {
    let p = RestartPolicy {
        max_restarts: 10,
        base_delay: Duration::from_secs(1),
        max_delay: Duration::from_secs(10),
    };
    let secs: Vec<u64> = (0..6).map(|a| p.delay(a).as_secs()).collect();
    assert_eq!(secs, vec![1, 2, 4, 8, 10, 10]);
    assert_eq!(p.delay(10_000), Duration::from_secs(10));
}

#[test]
fn agent_limit_returns_resource_limit_and_frees_up_on_stop() {
    let e = env("limit");
    let mut sup = Supervisor::new(SupervisorConfig {
        node: NodeId::new("node-1"),
        restart: RestartPolicy::default(),
        terminal: TerminalOptions::default(),
        limits: NodeLimits {
            max_agents: Some(2),
            ..NodeLimits::default()
        },
    });
    sup.register(manifest("[\"sh\", \"-c\", \"sleep 30\"]", None));
    let a = sup.start(&request(&e, "a"), Instant::now()).unwrap();
    let _b = sup.start(&request(&e, "b"), Instant::now()).unwrap();

    let err = sup.start(&request(&e, "c"), Instant::now()).unwrap_err();
    assert_eq!(
        err,
        SupervisorError::ResourceLimit {
            resource: "agents",
            limit: 2
        }
    );
    let p = err.to_protocol();
    assert_eq!(p.code, ErrorCode::NodeResourceLimit);
    assert_eq!(p.params.len(), 2);
    // A refused start leaves nothing behind.
    assert!(!e.dir.join("worktrees/c").exists());

    // Stopping one frees a slot; the stopped agent can only resume while there is room.
    sup.stop(&a).unwrap();
    let c = sup.start(&request(&e, "c"), Instant::now()).unwrap();
    assert!(matches!(
        sup.resume(&a, Instant::now()),
        Err(SupervisorError::ResourceLimit { .. })
    ));
    sup.stop(&c).unwrap();
    sup.resume(&a, Instant::now()).unwrap();
    sup.stop(&a).unwrap();
}

#[test]
fn terminal_limit_and_disk_usage() {
    let e = env("terminals");
    let mut sup = Supervisor::new(SupervisorConfig {
        node: NodeId::new("node-1"),
        restart: RestartPolicy::default(),
        terminal: TerminalOptions::default(),
        limits: NodeLimits {
            max_terminals: Some(1),
            ..NodeLimits::default()
        },
    });
    sup.register(manifest("[\"sh\", \"-c\", \"sleep 30\"]", None));
    let a = sup.start(&request(&e, "a"), Instant::now()).unwrap();
    let err = sup.start(&request(&e, "b"), Instant::now()).unwrap_err();
    assert_eq!(
        err,
        SupervisorError::ResourceLimit {
            resource: "terminals",
            limit: 1
        }
    );

    let usage = sup.disk_usage(&a).unwrap();
    assert!(usage.files >= 1 && usage.bytes >= 1, "{usage:?}");
    sup.stop(&a).unwrap();
}
