//! Agents driven through an L3 adapter (docs/specs/adapter-protocol.md): the supervisor starts the adapter in the
//! agent's own worktree, applies what it reports, and relays prompts and decisions. The adapter is
//! `agora-fake-agent --adapter`; never a real agent. Adapters use pipes, not terminals, so this runs on every OS.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use agora_adapters::host::{AdapterEvent, ChatItem};
use agora_adapters::Manifest;
use agora_core::{AgentId, AgentState, NodeId};
use agora_node::supervisor::{
    NodeLimits, RestartPolicy, StartRequest, Supervisor, SupervisorConfig, SupervisorEvent,
};
use agora_node::terminal::TerminalOptions;

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

struct Env {
    dir: PathBuf,
    repo: PathBuf,
}

fn env(name: &str) -> Env {
    let dir = std::env::temp_dir().join(format!("agora-adp-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    Env { dir, repo }
}

fn supervisor() -> Supervisor {
    // Adapters that die stay dead unless a test asks for restarts.
    supervisor_with(RestartPolicy {
        max_restarts: 0,
        ..RestartPolicy::default()
    })
}

fn supervisor_with(restart: RestartPolicy) -> Supervisor {
    let mut s = Supervisor::new(SupervisorConfig {
        node: NodeId::new("node-1"),
        restart,
        terminal: TerminalOptions::default(),
        limits: NodeLimits::default(),
    });
    s.register(
        Manifest::parse(&format!(
            "kind = \"fake-adapter\"\ndisplay_name = \"F\"\nversion = \"1\"\nmin_engine = 1\n[adapter]\ncommand = [{:?}, \"--adapter\"]\n",
            fake_agent()
        ))
        .unwrap(),
    );
    s
}

fn start(s: &mut Supervisor, e: &Env, name: &str) -> AgentId {
    s.start(
        &StartRequest {
            kind: "fake-adapter".into(),
            checkout: e.repo.clone(),
            worktree_root: e.dir.join("worktrees"),
            name: name.into(),
            base: None,
        },
        Instant::now(),
    )
    .expect("start")
}

/// Tick until `done` matches an event; returns everything seen.
fn until(
    s: &mut Supervisor,
    mut done: impl FnMut(&SupervisorEvent) -> bool,
) -> Vec<SupervisorEvent> {
    let end = Instant::now() + Duration::from_secs(15);
    let mut all = Vec::new();
    while Instant::now() < end {
        for ev in s.tick(Instant::now()) {
            let stop = done(&ev);
            all.push(ev);
            if stop {
                return all;
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out; events so far: {all:#?}");
}

/// Tick until the agent's listed state is `want` (events in the same batch as the one we stopped at are not lost).
fn settle(s: &mut Supervisor, id: &AgentId, want: AgentState) {
    let end = Instant::now() + Duration::from_secs(15);
    while Instant::now() < end {
        s.tick(Instant::now());
        if s.list().iter().any(|v| &v.id == id && v.state == want) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("agent never reached {want:?}");
}

fn state_is(want: AgentState) -> impl FnMut(&SupervisorEvent) -> bool {
    move |e| matches!(e, SupervisorEvent::StateChanged { state, .. } if *state == want)
}

#[test]
fn an_adapter_agent_gets_its_own_worktree_and_reports_its_state() {
    let e = env("basic");
    let mut s = supervisor();
    let id = start(&mut s, &e, "adp1");
    let v = s.list().into_iter().find(|v| v.id == id).unwrap();
    assert!(v.running && v.branch == "agent/adp1");
    assert!(v.worktree.join("a.txt").exists(), "own worktree");
    until(&mut s, state_is(AgentState::Idle));

    s.adapter_prompt(&id, "hello", false).unwrap();
    let ev = until(&mut s, |e| {
        matches!(
            e,
            SupervisorEvent::StateChanged {
                state: AgentState::Idle,
                ..
            }
        )
    });
    assert!(ev.iter().any(|e| matches!(
        e,
        SupervisorEvent::StateChanged {
            state: AgentState::Working,
            ..
        }
    )));
    // Chat and usage are passed on as adapter events, and the native session id is recorded.
    let all = ev;
    assert!(all.iter().any(|e| matches!(e, SupervisorEvent::Adapter { event: AdapterEvent::Chat { item: ChatItem::Text(t), .. }, .. } if t == "hello")), "{all:#?}");
    // ADR-0058: this adapter declared no mode or model switch, so neither is sent to it.
    for (kind, cap) in [
        (agora_adapters::host::OptionKind::Mode, "agent.set_mode"),
        (agora_adapters::host::OptionKind::Model, "agent.set_model"),
    ] {
        assert_eq!(
            s.adapter_set_option(&id, kind, "x"),
            Err(agora_node::supervisor::SupervisorError::OptionUnsupported(
                cap
            ))
        );
    }
}

#[test]
fn approvals_are_relayed_and_the_node_decides() {
    let e = env("approval");
    let mut s = supervisor();
    let id = start(&mut s, &e, "adp2");
    until(&mut s, state_is(AgentState::Idle));
    s.adapter_prompt(&id, "ask", false).unwrap();
    let ev = until(&mut s, |e| {
        matches!(
            e,
            SupervisorEvent::Adapter {
                event: AdapterEvent::ApprovalRequest { .. },
                ..
            }
        )
    });
    assert!(ev.iter().any(|e| matches!(
        e,
        SupervisorEvent::StateChanged {
            state: AgentState::Blocked,
            ..
        }
    )));
    assert!(
        matches!(ev.last(), Some(SupervisorEvent::Adapter { event: AdapterEvent::ApprovalRequest { approval_id, .. }, .. }) if approval_id == "ap-1")
    );
    s.adapter_resolve_approval(&id, "ap-1", true).unwrap();
    until(&mut s, state_is(AgentState::Idle));
}

#[test]
fn a_crashing_adapter_ends_its_agent_as_unknown_without_taking_the_node_down() {
    let e = env("crash");
    let mut s = supervisor();
    let id = start(&mut s, &e, "adp3");
    let other = start(&mut s, &e, "adp3b");
    until(&mut s, state_is(AgentState::Idle));
    let _ = s.adapter_prompt(&id, "crash", false);
    let ev = until(&mut s, |e| {
        matches!(e, SupervisorEvent::AdapterCrashed { .. })
    });
    assert!(ev.iter().any(|e| matches!(
        e,
        SupervisorEvent::StateChanged {
            state: AgentState::Unknown,
            ..
        }
    )));
    let list = s.list();
    assert!(!list.iter().find(|v| v.id == id).unwrap().running);
    assert!(
        list.iter().find(|v| v.id == other).unwrap().running,
        "the other agent is unaffected"
    );
    assert!(s.adapter_prompt(&id, "again", false).is_err());
}

#[test]
fn stopping_an_adapter_agent_ends_its_session() {
    let e = env("stop");
    let mut s = supervisor();
    let id = start(&mut s, &e, "adp4");
    until(&mut s, state_is(AgentState::Idle));
    let ev = s.stop(&id).unwrap();
    assert!(ev.contains(&SupervisorEvent::Stopped { agent: id.clone() }));
    assert!(!s.list().into_iter().find(|v| v.id == id).unwrap().running);
}

#[test]
fn stopping_an_adapter_agent_ends_its_process() {
    let e = env("stop-process");
    let mut s = supervisor();
    let id = start(&mut s, &e, "adp-stop");
    let pid = s.running_pids()[0];
    assert!(
        agora_platform::process_name(pid).is_ok(),
        "the adapter runs"
    );
    s.stop(&id).unwrap();
    // Like a terminal agent's process, the adapter process ends with the stop instead of running on unobserved.
    assert!(
        agora_platform::process_name(pid).is_err(),
        "the adapter process outlived its stop"
    );
    assert!(s.running_pids().is_empty());
    // A resume starts a fresh adapter, and stopping again ends that one too.
    s.resume(&id, Instant::now()).unwrap();
    let again = s.running_pids()[0];
    assert_ne!(again, pid);
    assert!(s.adapter_prompt(&id, "hello", false).is_ok());
    s.stop(&id).unwrap();
    assert!(agora_platform::process_name(again).is_err());
}

#[test]
fn a_manifest_needs_a_real_adapter_command() {
    let bad = Manifest::parse("kind = \"x\"\ndisplay_name = \"X\"\nversion = \"1\"\nmin_engine = 1\n[adapter]\ncommand = []\n");
    assert!(bad.is_err());
}

#[test]
fn a_crashed_adapter_is_restarted_with_backoff_and_then_given_up_on() {
    let e = env("restart");
    let mut s = supervisor_with(RestartPolicy {
        max_restarts: 2,
        base_delay: Duration::from_millis(50),
        max_delay: Duration::from_millis(100),
    });
    let id = start(&mut s, &e, "adp5");
    until(&mut s, state_is(AgentState::Idle));

    // First crash: it comes back by itself and can be prompted again.
    let _ = s.adapter_prompt(&id, "crash", false);
    let ev = until(&mut s, |e| matches!(e, SupervisorEvent::Started { .. }));
    assert!(ev
        .iter()
        .any(|e| matches!(e, SupervisorEvent::Restarting { attempt: 1, .. })));
    settle(&mut s, &id, AgentState::Idle);
    assert!(s
        .list()
        .iter()
        .any(|v| v.id == id && v.running && v.restarts == 1));
    assert!(s.adapter_prompt(&id, "hello", false).is_ok());

    // Crashing again and again exhausts the budget; the agent is left stopped.
    let _ = s.adapter_prompt(&id, "crash", false);
    until(&mut s, |e| matches!(e, SupervisorEvent::Started { .. }));
    settle(&mut s, &id, AgentState::Idle);
    let _ = s.adapter_prompt(&id, "crash", false);
    until(&mut s, |e| matches!(e, SupervisorEvent::GaveUp { .. }));
    assert!(!s.list().iter().find(|v| v.id == id).unwrap().running);
}

#[test]
fn a_stop_cancels_a_pending_adapter_restart() {
    let e = env("stoprestart");
    let mut s = supervisor_with(RestartPolicy {
        max_restarts: 3,
        base_delay: Duration::from_millis(300),
        max_delay: Duration::from_millis(300),
    });
    let id = start(&mut s, &e, "adp6");
    until(&mut s, state_is(AgentState::Idle));
    let _ = s.adapter_prompt(&id, "crash", false);
    until(&mut s, |e| matches!(e, SupervisorEvent::Restarting { .. }));
    s.stop(&id).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    let ev = s.tick(std::time::Instant::now());
    assert!(!ev
        .iter()
        .any(|e| matches!(e, SupervisorEvent::Started { .. })));
    assert!(!s.list().iter().find(|v| v.id == id).unwrap().running);
}
